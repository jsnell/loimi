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
