# `std.obj`: objects, shapes, and inline caches

This page describes what `std/obj.lm` implements. It expands the `std.obj` section of [`../stdlib.md`](../stdlib.md).
It builds on [`gc.md`](gc.md), and on inline caches and families in
[`../language.md`](../language.md) §7.

`std.obj` implements **record-like objects with symbol keys**: JS objects,
Python instances, Ruby objects, Lua tables used as records. Hash-table-like
containers keyed by arbitrary values (Lua tables used as maps, Python dicts)
are a hash table: `std.dict.Dict` (insertion-ordered, in the GC heap, with
lookups whose equality may run guest code; examples/python's dicts), or
one the interpreter writes itself (Mini, Scheme and MiniJS each have
their own Lua-style hybrid table). An
interpreter typically uses both.

## As implemented

`std/obj.lm`, tested by `tests/run/std_obj.lm` (semantics),
`tests/run/std_obj_gc.lm` (random operations checked against a host-side
model, collecting after every operation in stress mode) and
`tests/run/std_obj_ic.lm` (a `GETPROP` family in a vm, all dispatch
modes), `tests/run/std_obj_sites.lm` (the cache state machine in a vm),
`tests/run/std_obj_stable.lm` (stable ids, dead prototypes),
`tests/run/std_obj_attrs.lm` (attributes, key order) and
`tests/run/std_obj_generic.lm` (the ready-made `obj.Obj<V>` under GC
stress); `bench/obj` measures a cache hit.

### The object type: generic or the interpreter's

The library declares a generic object type over
the interpreter's `Value`, with exactly the fields it needs:

<!-- snippet: run
import std.gc;
import std.obj;
{snippet}
-->
```loimi
value Value repr nanbox { Num(f64) unboxed, Nil, O(Gc<obj.Obj<Value>>) }

fn main() {
    var heap = gc.Heap.new();
    var m: obj.Model<Gc<obj.Obj<Value>>, Gc<obj.Slots<Value>>, Value> =
        obj.Model.new(obj.new_slots<Value>, Value.Nil);
    m.attach(&heap);
    let o = obj.new_obj<Value>(&heap, m.root(4), None, 4);   // shape, prototype, inline slots
    m.set(o, 1, Value.Num(2.5));                      // key 1: a symbol id
}
```

An interpreter that wants other fields in its objects (a class pointer,
a hash, flags) declares the types itself, and the library is generic
over them:

<!-- snippet: run
import std.gc;
import std.obj;
value Value repr nanbox { Num(f64) unboxed, Int(i32), Nil, O(Gc<Obj>) }
const sym_x: u64 = 1;
{snippet}
-->
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

fn main() {
    var heap = gc.Heap.new();
    var m: obj.Model<Gc<Obj>, Gc<Slots>, Value> = obj.Model.new(new_slots, Value.Nil);
    m.attach(&heap);              // scanner + sweeper; `m` must not move afterwards
    let o = new(&heap, Obj { shape: m.root(4), dict: None, overflow: None, proto: None }, 4);
    m.set(o, sym_x, Value.Int(1));
}
```

Generic bodies resolve field accesses per instance, so a function that
reads `o.proto` only needs the field when it is used. The object
model's choices are fixed:

| Choice | As implemented |
|---|---|
| keys | `u64`: symbol ids (`text.Interner`) or other integers that stay meaningful while shapes use them. Keys are not traced: see "Keys for guest strings" below. |
| attributes | a u32 of bits per property, meaning chosen by the interpreter |
| prototypes | in the shape: one root shape per (prototype, inline count). Objects without prototypes use `root(n)`. |
| key order | insertion order, always: `keys` returns insertion order in both modes |
| inline slots | per allocation: the object's `inline.len`, which must match the root shape's count (`root(n)`, `root_for(proto, n)`, `template_root(&t)`) |
| dictionary threshold | `m.dict_threshold` (default 64) |

### Keys for guest strings

A key must name the same property for as long as any shape, cache or
dictionary holds it. Keys are plain words that the collector does not
see, so the address of a GC string is a key only while that string
lives: once it dies, the next object allocated at its address would
silently alias the property in every shape that has it. Two safe ways
to key properties by a guest language's strings:

- **Pinned interned strings** (Lox, `examples/lox`): intern every
  string in a `gc.WeakInterner` (equal strings are one object) and use
  `strings.key(s)` for each property name. It pins the string (kept
  alive, and in the table, for the table's lifetime) and returns its
  address, which then names those contents for good. The compiler pins
  names when it makes property-name constants, so the handlers use the
  constant's address directly.
- **Host symbol ids** (MiniJS, `examples/minijs`): a `text.Interner`
  maps contents to small ids that are never reused; a string object
  caches its id (`ks.key`), computed on first use as a key.

Either way, names used as keys are never freed: memory grows with the
number of distinct property names, as in JS engines' symbol tables.

### Shapes

`Shape { id, count, inline_cap, flags, key, attrs, proto, parent, table }`.
Property *i* (in insertion order) lives in slot index *i*: `inline[i]`
below the inline count, `overflow.data[i - inline_cap]` above. A cache
stores a SlotRef: the index, or the overflow index with bit 31
(`obj.OUT`) set, computed by `obj.slot_ref(i, cap)`.

- **Transitions** are one global open-addressed table from (parent id,
  key, attributes) to (child id, child address); root shapes are entries
  with parent 0 and the key (prototype address, inline count, PROTO
  flag). The child address is weak: children keep their parent alive
  through `parent`, nothing keeps a child alive but its objects (and the
  targets of cached adds). A sweeper (`Heap.add_sweeper`) clears the
  address of every entry whose shape was not marked; references are
  turned back into `Gc<Shape>` through a private `value` type, the only
  way to make a reference from bits.
- **Stable ids.** The sweeper keeps the *id* of a dead shape's entry, and
  a shape rebuilt along the same path (the next object built the same
  way after a collection) gets the same id back (`shapes.reborn` counts
  these). An id therefore names a transition path, which fixes the
  layout, so a cache entry holding it can be stale but never wrong; no
  id is ever given to a different path, and caches need no reset after a
  collection (the weak-cache machinery of gc.md §8 is not needed). Ids
  come from a u32 counter (0 is an empty cache, `DICT_ID` =
  `0xFFFFFFFF` the dictionary shape); 4 billion distinct paths abort the
  process.
- **The trade-off: memory for cache stability.** Each remembered dead
  path costs a 32-byte entry (the table is at most half full), but no
  shape object: dead shapes are freed. `shapes.retain` (default 65536)
  bounds the dead entries; past it the sweeper forgets all of them
  (`shapes.forgotten`), and those paths get new ids, which caches see as
  new shapes. `retain = 0` is the old behaviour (ids die with their
  shapes; MiniJS's deltablue went megamorphic that way, as its
  constraint graph is rebuilt again and again); the default costs at
  most about 4 MB for programs that build many distinct shapes, and
  nothing for the others. The alternatives considered: keeping shapes
  alive while their parent lives (strong edges) costs whole shape objects
  (80 bytes plus lookup tables) and still loses the tree when its root
  dies; pinning (what MiniJS did) keeps every cached shape forever;
  an epoch scheme needs to know which caches still hold an id, which the
  hit path cannot afford to record.
- **The prototype's identity** is part of the root key, as an address. A
  prototype that dies and a new one at the same address share root ids
  and so all shape ids. That is harmless for own properties (same path,
  same layout) and for prototype hits, whose validity cell dies with the
  old prototype (below). `tests/run/std_obj_stable.lm` builds exactly
  this case.
- **Lookup**: chains of up to 8 properties are walked; longer ones build
  a key → (index, attributes) `Table` on first lookup and keep it on the
  shape (one table per shape; tables are not shared along a chain).

### Attributes and key order

Every property has a u32 of attribute bits whose meaning is the
interpreter's (MiniJS: bit 0 = not enumerable). They are part of the
transition key, so they are part of the shape: `m.define(o, key, v,
attrs)` adds or redefines, `m.set_attrs(o, key, attrs)` moves a fast
object to the shape with the new attributes (its slots stay put; in
dictionary mode the entry changes), `m.attrs(o, key)` reads them and
`m.keys_where(o, &out, skip)` lists the keys with none of the `skip`
bits. Write caches are not filled for properties with the bits in
`m.ic.readonly`, so read-only properties and accessors can be checked in
the interpreter's slow path.

Key order is insertion order in both modes. JavaScript's order (integer
indices first, ascending, then the rest in insertion order) is
`obj.index_keys_first(&keys, data, index_of)`, given the interpreter's
"is this key an array index" function (stable merge sort). Which keys
are indices is the interpreter's knowledge (MiniJS's keys are interned
strings); std.obj has no elements store.

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
entries kept in insertion order, each a key and slot | attributes << 32).
Values stay in the object's slots; the dictionary maps keys to slot
indices. When the entry array is full it is rebuilt without deleted
entries, moving values down to keep slots dense. `m.optimize(o)` turns a
dictionary-mode object with at most `dict_threshold` properties back
into a fast one; `m.to_dict(o)` forces dictionary mode.

### Inline caches

The vm declares the cache words and does the check itself, so the hit
path stays in the handler. std.obj runs the cache state machine
(monomorphic, polymorphic, megamorphic) in its slow paths, over the six
cache words of an `obj.Site`: `cache PropIC = obj.Site;` declares them (language.md §7.1), and
`&cache` is the `*obj.Site` its functions take:

<!-- snippet: skip excerpt of a vm with members elided; full version in tests/run/std_obj_sites.lm -->
```loimi
cache PropIC = obj.Site;
family GETPROP(dst: reg, o: reg, key: kst) : ABC + PropIC {
    generic {
        let (v, st) = ctx.m.read_miss(&cache, obj.IcState.Generic, x, k);
        ..   // become the member for `st`
    }
    GETPROP_IN {
        if let O(x) = o and x.shape.id == cache.sid { dst = x.inline.ptr[cache.slot]; next }
        cold { become GETPROP_MISS; redo }   // read_miss(.., IcState.OwnIn, ..)
    }
    GETPROP_OUT  { .. ctx.m.load_out(x, cache.slot & !obj.OUT) .. }
    GETPROP_P    { .. x.shape.id == cache.sid and let Some(v) = ctx.m.proto_load(x, cache.slot, cache.cell, cache.depth) .. }
    GETPROP_POLY { .. ctx.m.poly_get(x, cache.ent) .. }
    GETPROP_MEGA { .. ctx.m.mega_get(x, key) .. }
}
```

- `IcState` names the members: `Generic`, `OwnIn`, `OwnOut`, `Proto`,
  `Poly`, `Mega`, and `Add` for write sites.
- `m.read_miss(site, state, o, key)` reads through the chain and returns
  the value and the next state. A monomorphic site that misses is
  refilled `m.ic.refills` times (default 8: many sites are monomorphic
  per program phase), then becomes polymorphic, keeping its old hit. A
  `Poly` entry (eight hits, in `m.ic.poly`, indexed by `site.ent`) that
  fills up starts over `m.ic.resets` times (default 8), then the site
  goes megamorphic.
- A hit is an `obj.Hit { sid, slot, cell, depth }` of plain words: for
  own properties `cell` and `depth` are 0; for prototype hits `cell` is
  the validity cell of the receiver's prototype and `depth` the number
  of `proto` links to the holder. `m.read(o, key, &hit)` fills one (its
  `depth` says where the value was found even when there is nothing to
  cache), `m.hit(o, &hit)` checks one.
- **Where a value came from.** Languages that bind methods (Lox, Python,
  Ruby) must tell a value found on the receiver (a field, which may hold
  a function) from one found through its class. In the monomorphic
  states the member says it (`Proto` vs `OwnIn`/`OwnOut`). For the
  others, `m.poly_get_at(o, ent)` and `m.mega_get_at(o, key)` return
  `?(V, u32)`: the value and its depth (0 own, n >= 1 found n prototype
  links up, `obj.ABSENT` for an absent-property hit), and
  `m.read_miss_at(..)` returns `(?V, IcState, depth)`. The depth is
  already in the cache entry, so this costs a register:

  <!-- snippet: skip excerpt of a family member (Lox-style `Inst` variant), rest elided -->
  ```loimi
  GETPROP_POLY {
      if let Inst(x) = o and let Some((v, d)) = ctx.m.poly_get_at(x, cache.ent) {
          if d != 0 { cold { dst = bind(ctx, o, v); next } }   // a method
          dst = v; next
      }
      cold { .. }
  }
  ```
- The megamorphic cache `m.ic.mega` (an `obj.MegaCache`, 4096 entries)
  is direct-mapped on (shape id, key) and holds own and prototype hits.
  It is never cleared: ids name layouts, and prototype hits carry their
  cell. `m.get_mega(&c, o, key)` is the own-property-only variant for
  object types without a `proto` field.
- `m.write_miss(site, o, key, v)` stores and returns `OwnIn`/`OwnOut`
  for an existing property of a fast-mode object, or `Add` when the
  property was added into an inline slot: the cache holds the old shape
  id, the slot, and (in `site.ent`) the new shape, kept alive in a table
  the model marks. `m.add_hit(o, ent, slot, v)` is the hit after the
  shape check: a store and a shape change, with no check that the object
  needs its validity cell invalidated, because objects with cells have
  shapes of their own (below).
- `m.write_miss_in(site, state, o, key, v)` is `write_miss` for a site
  in a given state, with polymorphic stores: a monomorphic store site
  that misses `m.ic.refills` times becomes `Poly` (own-slot hits;
  `m.poly_set(o, ent, v)` is the hit), and a Poly entry that fills up
  `m.ic.resets` times makes it `Mega`, which stores through the shared
  cache (`m.mega_set(o, key, v)`, only while `m.ic.readonly` is 0) and
  fills it in its slow path but never changes state again.
- With `m.ic.absent` set (off by default), reads that find nothing are
  cached too, as hits with slot `obj.ABSENT` in Poly and Mega entries
  (monomorphic members cannot express them, so such a read makes a
  site polymorphic at once): the receiver's shape id and the validity
  cell of its prototype (0 without one). `poly_get`, `mega_get` and
  `hit` give `nil` for them, so turn this on only for languages where
  reading an absent property gives nil. `m.has_mega(o, key)` is `key in
  o` through the shared cache (`tests/run/std_obj_absent.lm`).
- `m.get_fill(o, key, &shape_word, &slot_word)` and `m.set_fill(..)` are
  the slow paths of a vm that only wants monomorphic own-property caches
  (`tests/run/std_obj_ic.lm`).

The hit helpers (`load_ref`, `proto_load`, `poly_get`, `poly_get_at`,
`mega_get`, `mega_get_at`) are `#[inline]`, so a handler's hit path has
no call and no stack frame (Lox's `trees`, whose sites are polymorphic,
ran 5% faster when they were inlined; MiniJS unchanged within noise).

A hit costs two dependent loads (`o.shape`, `shape.id`), a compare
against the cache word and the slot load: 0.8 ns over a register move on
an i7-6700 (`bench/obj/README.md`, which shows the machine code).

### Prototype chains and validity cells

`m.lookup(o, key)` returns `Lookup.Own(i)`, `Lookup.Proto(holder, i)` or
`Lookup.Missing`, following `proto` fields; `m.get_chain` returns the
value; `m.set_proto(o, p)` moves a fast object to the matching shape in
`p`'s tree.

A prototype hit is checked by `m.proto_load(o, slot, cell, depth)` after
the receiver's shape compare: the cell's byte, then `depth` loads of
`proto` and the slot load. The cell is that of the receiver's prototype,
from `m.cell(p)`:

- A cell is an id: a byte in `shapes.cvalid` (1 while valid) and links
  to its parent cell and its children. Ids are not reused (the table
  grows by 17 bytes per cell made; cells are only made for prototypes
  that a cache reads through, again after each invalidation). Cell 0 is
  never valid.
- Cells live in a side table keyed by the object's address, weak in the
  object: when the object dies the sweeper invalidates its cell, so a
  new object at the same address (which gets the same shape ids) cannot
  revive old hits.
- A new cell for `p` registers under the cell of `p.proto` (created if
  needed). Adding, deleting or changing the attributes of a property of
  an object with a cell, switching it to or compacting its dictionary,
  or changing its prototype invalidates its cell and, recursively, the
  cells under it. Storing a new value into an existing property does
  not.
- An object that gets a cell (and every object above it) moves to the
  **prototype tree**: the same keys and slots under a root flagged
  `obj.PROTO` (`m.make_proto(o)`, which interpreters may also call when
  an object becomes a prototype). Its shape ids therefore never equal
  those of ordinary objects, which is what makes `add_hit` safe, and
  `m.add` only looks up a cell for flagged objects. `m.is_proto(o)`
  tells.
- The holder is found by walking from the receiver, not stored: the
  cell's validity guarantees the chain is the one the hit was made on.

### Allocation templates

`obj.Template.new(proto_bits, inline)`; `m.template_root(&t)` is the root
shape for the next object from the site (with `t.inline` inline slots),
and `m.observe(&t, o)` raises `t.inline` to the number of properties a
finished object has (capped at `max_inline`, 32).

### Limitations

- Attributes are bits the interpreter interprets; there is no error type
  for a refused store.
- There is no integer-indexed element storage: arrays are the
  interpreter's.
- Global variables are the interpreter's: a property cell would have to
  hold the interpreter's `Value`. Mini's `GETGLOBAL` slot cache plays
  this role.
- `keys` copies the keys out; iteration does not track mutation.
- Each shape has its own lookup table; tables are not shared along a
  transition chain.
- The state machine reads a `proto` field, so object types without one
  can use the monomorphic helpers and `get_mega` only.

### Effects

Nothing in `std.obj` collects. Every function is free of effects, so
handlers can call them from fast paths and cold blocks without syncing,
and the generic code needs no shadow-stack frames.
