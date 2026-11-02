# `std.obj`: objects, shapes, and inline caches

Design draft. It expands the `std.obj` section of [`../stdlib.md`](../stdlib.md).
It builds on [`gc.md`](gc.md), and on inline caches and families in
[`../language.md`](../language.md) §7.

`std.obj` implements **record-like objects with symbol keys**: JS objects,
Python instances, Ruby objects, Lua tables used as records. Hash-table-like
containers keyed by arbitrary values (Lua tables used as maps, Python dicts)
belong to `std.collections.Table` / `IndexMap`. An interpreter typically uses
both.

## Goals

1. Property reads at a monomorphic site cost **a shape compare plus a load**.
2. Objects with the same construction history share a **shape**
   (hidden class), so per-object memory is just the slots.
3. Guest-language features (prototypes, attributes, ordering, deletion) are
   **opt-in at compile time** through a model configuration. Features a
   language doesn't use cost nothing.
4. Inline caches are **safe across GC**: no dangling shape references and no
   false hits (gc.md §8).

---

## 1. Configuring the object model

Everything is generated from a compile-time configuration:

```loimi
const Model = std.obj.Model(.{
    .value          = Value,          // the interpreter's `value` type
    .key            = Symbol,         // property key type (Symbol, or a user key type)
    .attributes     = false,          // JS: writable/enumerable/configurable per property
    .prototypes     = .in_shape,      // .none | .in_shape | .in_object
    .ordered        = true,           // iteration in insertion order
    .inline_slots   = 4,              // default inline slot count for new objects
    .dict_threshold = 64,             // properties before switching to dictionary mode
    .elements       = true,           // integer-indexed element storage (arrays)
});

const Object = Model.Object;
const Shape  = Model.Shape;
```

Examples:

| Language | `.attributes` | `.prototypes` | `.elements` |
|---|---|---|---|
| JS | `true` | `.in_shape` | `true` |
| Python instances | `false` | `.none` (class lives in a slot) | `false` |
| Lua table as record | `false` | `.none` (metatables live in a slot) | `false` |
| Self, Io-style | `false` | `.in_shape` | `false` |

When the prototype is part of the shape (`.in_shape`), an object's prototype
identity is covered by the shape check. One compare then validates both the
own layout and the prototype for inline caches.

---

## 2. Shapes

```loimi
object Shape {
    id: ShapeId,                   // stable u32 index into the shape table
    parent: ?Gc<Shape>,            // previous shape in the transition chain
    key: Key,                      // property added by this transition
    attrs: Attrs,                  // zero-sized if `.attributes = false`
    slot: SlotRef,                 // where the new property lives
    count: u32,                    // number of properties
    proto: ?Gc<Object>,            // only if `.prototypes = .in_shape`
    transitions: Transitions,      // children; weak, see below
    table: ?Gc<PropertyTable>,     // lazy lookup index (key → slot, attrs)
    flags: ShapeFlags,             // DICTIONARY, PROTOTYPE_MAP, UNSTABLE, ...
}
```

### Transition tree

Starting from a root shape (one per prototype, or one global root), adding a
property follows or creates a transition:

```
root ──x──▶ {x} ──y──▶ {x,y} ──z──▶ {x,y,z}
                └──z──▶ {x,z}
```

- `Transitions` stores a single inline `(key, attrs, Weak<Shape>)` in the
  common case and a small hash map once a shape has more than one child.
- Transition edges are **weak**: unused shapes are collected. Shapes are kept
  alive by objects that use them and by their children (through `parent`).
- `ShapeId`s come from a **shape table** (`Vec<Weak<Shape>>` plus a free list).
  An id is reused only after its shape has died *and* a collection has reset
  every weak inline-cache entry that referred to it (gc.md §8). That makes a
  false cache hit impossible.

### Lookup

`shape.lookup(key)` walks the chain for short chains (8 properties or fewer:
cheap and cache-friendly). For longer ones it builds a `PropertyTable` (an
open-addressed `key → (slot, attrs)` map) on first use and caches it on the
shape. Tables are shared between a shape and its descendants when possible:
a child copies on first write, so a straight chain shares a single table.

---

## 3. Objects

```loimi
object Object {
    shape: Gc<Shape>,
    overflow: ?Gc<SlotArray>,      // out-of-line slots once inline slots run out
    elements: Elements,            // only with `.elements = true`
    inline: [Value],               // trailing inline slots, length fixed at allocation
}

bits SlotRef: u32 {
    index: 31,
    out_of_line: 1,                // 0 = inline[index], 1 = overflow[index]
}
```

- The number of inline slots is fixed when the object is allocated: the
  `ObjectTemplate` (below) or `.inline_slots`. Properties past that go to
  `overflow`, which grows by about 1.5×.
- An inline cache stores a `SlotRef`, so a cached load is
  `if ref.out_of_line { o.overflow[i] } else { o.inline[i] }`. When the cache
  is filled, the out-of-line bit is known and the family member can be chosen
  accordingly (`GETPROP_INLINE` / `GETPROP_OUTLINE`) to drop that branch.

### Allocation templates

```loimi
struct ObjectTemplate {
    shape: Gc<Shape>,
    inline_slots: u32,
}
fn Model.template(h: *Heap, root: Gc<Shape>) -> ObjectTemplate;
fn ObjectTemplate.observe(t: *ObjectTemplate, o: Gc<Object>);  // learn final size ("slack tracking")
fn Model.new_object(h: *Heap, t: *ObjectTemplate) -> Gc<Object> uses gc;
```

Interpreters keep one template per allocation site: a class, a constructor, or
an object-literal instruction. `observe` lets later allocations reserve exactly
as many inline slots as objects from that site eventually have. An
object-literal instruction can keep its template in its inline cache and start
objects directly at their final shape.

### Elements (optional)

With `.elements = true`, integer keys (`a[0]`) are stored in a separate
`Elements` store instead of as properties:

- `Dense(Gc<ValueArray>, len)`: contiguous with holes allowed, grows
  amortized;
- `Sparse(Gc<IndexMap<u32, Value>>)`: used when density falls below 1/4 or
  when an index jumps far past the end.

Arrays are objects with elements and a `length` property, as in JS.

---

## 4. Dictionary mode

An object switches to **dictionary mode** after `dict_threshold` properties or
on deletion (unless deleting the last-added property, which just steps back
to the parent shape). Its properties then live in a per-object
`Gc<PropertyDict>` (ordered if `.ordered`), and it gets a unique
`DICTIONARY` shape. That shape's id never matches a filled cache entry, so
every inline cache misses and falls back to the generic path. A megamorphic
cache (section 5) still makes dictionary lookups fast.

`Model.optimize(o)` turns a dictionary-mode object back into a fast object.
Interpreters call it for objects that become prototypes, since lookups go
through prototypes constantly.

---

## 5. Inline cache helpers

A cache in the code stream is a fixed number of words (language.md §7.1). The
library provides ready-made cache layouts and the logic for the usual cache
state machine:

```
          miss                miss (≤ N shapes)       miss (> N)
 EMPTY ──────────▶ MONO ─────────────────────▶ POLY ─────────────▶ MEGA
   ▲                │ hit                       │ hit                │ hit/miss
   └── GC reset ────┴───────────────────────────┘                    ▼
                                                            global MegaCache
```

### Cache layouts

```loimi
// Monomorphic: 2 words.
cache PropIC { shape: weak ShapeId, slot: SlotRef }

// Polymorphic: 1 word in the code; entries live in a side table on the Proto.
cache PolyRef { index: u32 }
object PolyEntries { n: u8, shapes: [weak ShapeId; 4], slots: [SlotRef; 4] }

// Prototype-chain hit: the receiver shape plus a validity cell (section 6).
cache ProtoIC { shape: weak ShapeId, entry: u32 }     // entry → ProtoEntry side table
object ProtoEntry { cell: Gc<ValidityCell>, holder: Gc<Object>, slot: SlotRef }
```

### The global megamorphic cache

```loimi
struct MegaCache {                       // direct-mapped, 4096 entries, per heap
    entries: [MegaEntry; 4096],
}
struct MegaEntry { shape: ShapeId, key: Key, slot: SlotRef }
fn MegaCache.get(c: *MegaCache, s: ShapeId, k: Key) -> ?SlotRef;     // inline: hash, one compare pair
fn MegaCache.put(c: *MegaCache, s: ShapeId, k: Key, r: SlotRef);
```

It is cleared on every GC. The heap does this, because `ShapeId`s may be
reused afterwards.

### A complete property-read family

```loimi
family GETPROP(dst: reg, obj: reg, name: kst) : ABC + PropIC {
    generic {
        let Some(o) = Model.as_object(obj) else cold { dst = get_primitive(ctx, obj, name)?; next };
        match Model.lookup(o, as_key(name)) {
            .Own(r) => {
                Model.ic.fill_mono(&cache, o.shape, r);
                dst = o.load(r);
                become (if r.out_of_line { GETPROP_OUT } else { GETPROP_IN });
                next
            }
            .Proto(holder, r) => { dst = holder.load(r); next }      // ProtoIC handled by GETPROP_P (omitted)
            .Missing => { dst = Nil; next }
        }
    }
    GETPROP_IN {
        if let Some(o) = Model.as_object(obj) and o.shape.id == cache.shape {
            dst = unsafe { o.inline.at_unchecked(cache.slot.index) }; next
        }
        cold { Model.ic.on_miss(&cache); become generic; redo }      // on_miss counts misses and picks poly/mega
    }
    GETPROP_OUT {
        if let Some(o) = Model.as_object(obj) and o.shape.id == cache.shape {
            dst = unsafe { o.overflow.unwrap_unchecked().at_unchecked(cache.slot.index) }; next
        }
        cold { Model.ic.on_miss(&cache); become generic; redo }
    }
}
```

`Model.ic.on_miss` keeps a small per-site miss counter in the side table.
After a configurable number of misses it rewrites the site to `GETPROP_POLY`,
and past that to `GETPROP_MEGA` (not shown). Each is a family member that uses
the matching helper.

---

## 6. Prototype chains and validity cells

A load that finds its property on a prototype is cached against **the receiver's
shape** plus a **validity cell** that represents "the prototype chain above
this shape is unchanged".

```loimi
object ValidityCell { valid: bool }

fn Model.validity_cell(h: *Heap, receiver_shape: Gc<Shape>) -> Gc<ValidityCell> uses gc;
```

- Objects used as prototypes are flagged `PROTOTYPE_MAP` and kept in fast
  mode where possible (`Model.optimize`).
- Each prototype object owns the cell for "my chain is unchanged". Adding,
  deleting, or changing attributes of a property on a prototype, or changing
  its prototype, invalidates its cell. It also invalidates the cells of
  prototypes that inherit from it, which are found through a registry of
  dependents on the prototype's info record.
- A cached prototype hit checks `receiver.shape.id == cache.shape &&
  entry.cell.valid`: two compares and a load, regardless of chain depth.
- Overwriting the *value* of a prototype property doesn't invalidate the cell,
  because caches store the holder and slot, not the value.

When `.prototypes = .none` this whole section compiles away.

---

## 7. Global variables: property cells

Globals are a hot path in most dynamic languages, and dictionary versioning
(invalidate everything when the global dict changes) tends to be too coarse.
`std.obj.Globals` stores each binding in its own cell:

```loimi
object PropertyCell { value: Value, state: CellState }   // .valid | .deleted
struct Globals { map: HashMap<Key, Gc<PropertyCell>> }

fn Globals.cell(g: *Globals, h: *Heap, k: Key) -> Gc<PropertyCell> uses gc;  // creates a "deleted" cell if absent
fn Globals.get(g: *Globals, k: Key) -> ?Value;
fn Globals.set(g: *Globals, h: *Heap, k: Key, v: Value) uses gc;
fn Globals.delete(g: *Globals, k: Key);
```

The global-access op caches the cell itself in a side-table entry, so a hit is
one load to reach the cell, a state check, and one load of the value.
`delete` sets the cell to `.deleted` so cached sites see the change
immediately. A binding that is redefined reuses its old cell, so cached sites
stay valid.

---

## 8. Iteration and reflection

```loimi
fn Model.keys(o: Gc<Object>) -> KeyIter;                 // insertion order if `.ordered`
fn Model.entries(o: Gc<Object>) -> EntryIter;
fn Model.len(o: Gc<Object>) -> u32;
fn Model.has_own(o: Gc<Object>, k: Key) -> bool;
```

Iterators snapshot the shape when they are created. If the object's shape
changes during iteration (a property added or removed), the iterator falls back
to a slower path that reproduces the guest language's rules for mutation during
iteration. The callback `Model.on_iter_mutation` selects JS-like (skip deleted,
don't visit added), Python-like (raise "dict changed size"), or Lua-like
(undefined, but memory-safe) semantics.

---

## 9. Core API summary

```loimi
enum Lookup { Own(SlotRef), Proto(Gc<Object>, SlotRef), Missing }

fn Model.root_shape(h: *Heap, proto: ?Gc<Object>) -> Gc<Shape> uses gc;
fn Model.new_object(h: *Heap, t: *ObjectTemplate) -> Gc<Object> uses gc;
fn Model.as_object(v: Value) -> ?Gc<Object>;            // via the `value` type's variant

fn Model.lookup(o: Gc<Object>, k: Key) -> Lookup;        // no effects, safe on fast paths
fn Model.get(o: Gc<Object>, k: Key) -> ?Value;
fn Model.set(h: *Heap, o: Gc<Object>, k: Key, v: Value) -> Result<(), SetError> uses gc;
fn Model.define(h: *Heap, o: Gc<Object>, k: Key, v: Value, a: Attrs) -> Result<(), SetError> uses gc;
fn Model.delete(h: *Heap, o: Gc<Object>, k: Key) -> bool uses gc;
fn Model.set_proto(h: *Heap, o: Gc<Object>, p: ?Gc<Object>) uses gc;
fn Model.optimize(h: *Heap, o: Gc<Object>) uses gc;

fn Object.load(o: Gc<Object>, r: SlotRef) -> Value;
fn Object.store(o: Gc<Object>, r: SlotRef, v: Value);  // barrier per heap policy

fn Model.ic.fill_mono(c: *PropIC, s: Gc<Shape>, r: SlotRef);
fn Model.ic.on_miss(c: *PropIC) -> IcState;
```

`SetError` covers read-only properties, non-extensible objects, and so on.
It is only produced when `.attributes = true`; otherwise it is `never` and the
`Result` disappears at compile time.

---

## As implemented

`std/obj.lm`, tested by `tests/run/std_obj.lm` (semantics),
`tests/run/std_obj_gc.lm` (random operations checked against a host-side
model, collecting after every operation in stress mode) and
`tests/run/std_obj_ic.lm` (a `GETPROP` family in a vm, all dispatch
modes) and `tests/run/std_obj_generic.lm` (the ready-made `obj.Obj<V>`
under GC stress); `bench/obj` measures a cache hit.

### No `Model` configuration; the object type is generic or the interpreter's

There is no comptime. The library declares a generic object type over
the interpreter's `Value`, with exactly the fields it needs:

```loimi
value Value repr nanbox { .., O(Gc<obj.Obj<Value>>) }

var m: obj.Model<Gc<obj.Obj<Value>>, Gc<obj.Slots<Value>>, Value> =
    obj.Model.new(obj.new_slots<Value>, Value.Nil);
m.attach(&heap);
let o = obj.new_obj(&heap, m.root(4), None, 4);   // shape, prototype, inline slots
```

An interpreter that wants other fields in its objects (a class pointer,
a hash, flags) declares the types itself, and the library is generic
over them:

```loimi
object Obj {
    shape: Gc<obj.Shape>,
    dict: ?Gc<obj.Dict>,
    overflow: ?Gc<Slots>,
    proto: ?Gc<Obj>,              // only if the prototype functions are used
    // ... any other fields ...
    inline: [Value],
}
object Slots { data: [Value] }
fn new_slots(h: *gc.Heap, n: u64) -> Gc<Slots> { new(h, Slots {}, n) }

var m: obj.Model<Gc<Obj>, Gc<Slots>, Value> = obj.Model.new(new_slots, Value.Nil);
m.attach(&heap);                  // scanner + sweeper; `m` must not move afterwards
let o = new(&heap, Obj { shape: m.root(4), dict: None, overflow: None, proto: None }, 4);
m.set(o, sym_x, Value.Int(1));
```

Generic bodies resolve field accesses per instance, so a function that
reads `o.proto` only needs the field when it is used. Choices the design
puts in the `Model` are fixed or dropped:

| Design option | As implemented |
|---|---|
| `.key` | `u64`: symbol ids (`text.Interner`) or other integers that stay meaningful while shapes use them. Keys are not traced. |
| `.attributes` | not supported (no attribute bits, no `SetError`) |
| `.prototypes` | `.in_shape`: one root shape per (prototype, inline count). Objects without prototypes use `root(n)`. |
| `.ordered` | always: `keys` returns insertion order in both modes |
| `.inline_slots` | per allocation: the object's `inline.len`, which must match the root shape's count (`root(n)`, `root_for(proto, n)`, `template_root(&t)`) |
| `.dict_threshold` | `m.dict_threshold` (default 64) |
| `.elements` | not built |

### Shapes

`Shape { id, count, inline_cap, key, proto, parent, ... }`. Property *i*
(in insertion order) lives in slot index *i*: `inline[i]` below the
inline count, `overflow.data[i - inline_cap]` above. A cache stores a
SlotRef: the index, or the overflow index with bit 31 (`obj.OUT`) set,
computed by `obj.slot_ref(i, cap)`.

- **Transitions** are weak. A shape's first child is stored inline
  (`child_key`, `child` as an address), further ones in an open-addressed
  `Edges` object. Children keep their parent alive through `parent`. A
  sweeper (`Heap.add_sweeper`) clears edges to shapes that were not
  marked, and drops dead entries from the table of root shapes; the
  references are turned back into `Gc<Shape>` through a private `value`
  type, the only way to make a reference from bits.
- **Ids are never reused**: a u32 counter (0 is an empty cache,
  `DICT_ID` = `0xFFFFFFFF` the dictionary shape). A stale id in a cache
  can never match a live object, so caches need no reset after a
  collection, and the weak-cache-reset machinery of gc.md §8 (not built
  in std.gc) is not needed. The cost is that 4 billion shape creations
  abort the process.
- **Pinning**: `m.shapes.pin(sh)` keeps a shape (and so its ancestors
  and its id) alive for good. Without it a shape dies with its last
  object, and the next object built the same way gets a new id, which a
  cache sees as a new shape; an interpreter pins the shapes its caches
  record so that sites stay monomorphic across collections (MiniJS's
  deltablue went megamorphic without it).
- **Lookup**: chains of up to 8 properties are walked; longer ones build
  a key → index `Table` on first lookup and keep it on the shape (one
  table per shape; tables are not shared along a chain).

### Objects and dictionary mode

`m.get`, `m.set`, `m.add`, `m.delete`, `m.has_own`, `m.find` (slot
index), `m.len`, `m.keys(o, &vec)`, `m.load`/`m.store` (by index),
`m.load_ref`/`m.store_ref` (by SlotRef, no bounds checks),
`m.load_inline`/`m.load_out` (for cache members that know where the slot
is). Overflow arrays grow by 1.5× through the `new_slots` function.

Deleting the last property added steps back to the parent shape.
Deleting any other property, or adding one past `dict_threshold`,
switches to dictionary mode: the object gets the shared dictionary shape
(id `DICT_ID`, never cached) and a `Dict` (an open-addressed index over
entries kept in insertion order). Values stay in the object's slots; the
dictionary maps keys to slot indices. When the entry array is full it is
rebuilt without deleted entries, moving values down to keep slots dense.
`m.optimize(o)` turns a dictionary-mode object with at most
`dict_threshold` properties back into a fast one; `m.to_dict(o)` forces
dictionary mode.

### Inline caches

The vm declares the cache words and does the check itself, so the hit
path stays in the handler:

```loimi
cache PropIC { shape: u32, slot: u32 }
family GETPROP(dst: reg, o: reg, key: kst) : ABC + PropIC {
    generic {
        dst = get_slow(ctx, o, key, &cache.shape, &cache.slot);   // calls m.get_fill
        if cache.shape != obj.EMPTY_ID {
            if cache.slot & obj.OUT == 0 { become GETPROP_IN; } else { become GETPROP_OUT; }
        }
        next
    }
    GETPROP_IN {
        if let O(x) = o and x.shape.id == cache.shape { dst = ctx.m.load_inline(x, cache.slot); next }
        cold { become GETPROP; redo }
    }
    GETPROP_OUT { .. ctx.m.load_out(x, cache.slot & !obj.OUT) .. }
}
```

- `m.get_fill(o, key, &shape_word, &slot_word)` and `m.set_fill(..)`: the
  slow paths, which fill the two words for an own property of a
  fast-mode object.
- `obj.Poly`: up to four (shape id, SlotRef) pairs with no GC references,
  for a side table or eight cache words.
- `obj.MegaCache`: direct-mapped (shape id, key) → SlotRef, shared by
  megamorphic sites (`m.get_mega(&mc, o, key)`). Not cleared on GC, since
  ids are not reused.
- The miss counting and the EMPTY → MONO → POLY → MEGA policy are left to
  the vm (a family member per state, as above); there is no
  `Model.ic.on_miss`.

A hit costs two dependent loads (`o.shape`, `shape.id`), a compare
against the cache word and the slot load: 0.8 ns over a register move on
an i7-6700 (`bench/obj/README.md`, which shows the machine code).

### Prototype chains and validity cells

`m.lookup(o, key)` returns `Lookup.Own(i)`, `Lookup.Proto(holder, i)` or
`Lookup.Missing`, following `proto` fields; `m.get_chain` returns the
value; `m.set_proto(o, p)` moves a fast object to the matching shape in
`p`'s tree.

A prototype hit is cached in a `ProtoEntry<O> { sid, slot, cell, holder }`
by `m.get_proto_fill(o, key, &entry)` and checked by `m.proto_hit(o,
&entry)`: receiver shape id equal, cell valid, then the holder's slot.
The cell is that of the receiver's prototype, from `m.cell(p)`:

- Cells live in a weak side table keyed by the object's address, so
  objects need no cell field and only objects some cache depends on have
  one.
- A new cell for `p` registers with the cell of `p.proto` (created if
  needed), so the cells of a chain form a tree; each cell keeps its
  parent cell alive.
- Adding or deleting a property of an object with a cell, switching it to
  or compacting its dictionary, or changing its prototype invalidates its
  cell and, recursively, the cells registered under it. Storing a new
  value into an existing property does not.

The entry holds GC references (the cell and the holder), so it must live
somewhere traced; `ProtoEntry.mark(h)` marks it from a scanner.

### Allocation templates

`obj.Template.new(proto_bits, inline)`; `m.template_root(&t)` is the root
shape for the next object from the site (with `t.inline` inline slots),
and `m.observe(&t, o)` raises `t.inline` to the number of properties a
finished object has (capped at `max_inline`, 32).

### Not built

- Attributes (writable/enumerable/configurable) and `SetError`.
- Elements (integer-indexed storage) and arrays.
- `Globals` with property cells: a cell object would have to hold the
  interpreter's `Value`, so it is the interpreter's type; Mini's
  `GETGLOBAL` slot cache plays this role today.
- Iteration with mutation semantics (`on_iter_mutation`): `keys` copies
  the keys out.
- Sharing lookup tables along a transition chain, and transition caching
  for property *adds* (a cache that goes from the old shape to the new
  one in one step).
- `Model.ic.on_miss` and per-site miss counters.

### Effects

Nothing in `std.obj` collects. Every function is free of effects, so
handlers can call them from fast paths and cold blocks without syncing,
and the generic code needs no shadow-stack frames.
