# `std.obj`: objects, shapes, and inline caches

This page explains how to use `std.obj` for a guest language's objects:
setting it up, reading and writing properties, and caching property
access in the vm. It is for interpreter authors, and assumes you know
the GC ([`gc.md`](gc.md)) and the vm's `cache` declarations and
families ([`../language.md`](../language.md)).

## When to use it

`std.obj` is for **record-like objects with symbol keys**: JavaScript
objects, Python instances, Ruby objects, Lua tables used as records.
It gives them hidden classes (*shapes*), so that a property access can
be cached as "this shape, this slot".

Containers keyed by arbitrary values (Lua tables used as maps, Python
dicts) need a hash table instead:

- `std.dict.Dict`: insertion-ordered, in the GC heap, with lookups whose
  equality may run guest code (examples/python's dicts); or
- one you write yourself (Mini, Scheme and MiniJS each have their own
  Lua-style hybrid table).

An interpreter typically uses both.

<a id="as-implemented"></a>

## Setting up

You need an object type, a slot-array type for properties that don't
fit inline, and an `obj.Model` that manages shapes and caches.

**The ready-made types.** `obj.Obj<V>` and `obj.Slots<V>` are generic
over the interpreter's value type:

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

**Your own types.** If your objects need other fields (a class pointer,
a hash, flags), declare the types yourself; the library is generic over
them. The object type needs the fields `shape`, `dict`, `overflow` and
a trailing `inline` array. `proto` is needed only if you use the
prototype functions.

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

`m.attach(&heap)` registers the model's root scanner and sweeper with
the heap. After that, `m` must not move.

### The model's fixed choices

| Choice | How it works |
|---|---|
| Keys | `u64`: symbol ids (from `text.Interner`) or other integers that stay meaningful while shapes use them. Keys are not traced; see [Keys for guest strings](#keys-for-guest-strings). |
| Attributes | A `u32` of bits per property, with a meaning you choose |
| Prototypes | Part of the shape: one root shape per (prototype, inline slot count). Objects without a prototype use `root(n)`. |
| Key order | Always insertion order, in both fast and dictionary mode |
| Inline slots | Chosen per allocation: the object's `inline.len`, which must match the root shape's count (`root(n)`, `root_for(proto, n)`, `template_root(&t)`) |
| Dictionary threshold | `m.dict_threshold`, default 64 |

## Keys for guest strings

A key must name the same property for as long as any shape, cache or
dictionary holds it. Keys are plain words that the collector does not
see. So the address of a GC string works as a key only while that
string lives: once it dies, the next object allocated at its address
would silently alias the property in every shape that has it.

Two safe ways to key properties by a guest language's strings:

- **Pinned interned strings** (as in `examples/lox`). Intern every
  string in a `gc.WeakInterner`, so equal strings are one object, and
  use `strings.key(s)` for each property name. This pins the string
  (keeps it alive, and in the table, for the table's lifetime) and
  returns its address, which then names those contents for good. Lox's
  compiler pins names when it makes property-name constants, so the
  handlers use the constant's address directly.
- **Host symbol ids** (as in `examples/minijs`). A `text.Interner` maps
  contents to small ids that are never reused. Each string object caches
  its id, computed the first time it is used as a key.

Either way, names used as keys are never freed. Memory grows with the
number of distinct property names, as in JavaScript engines' symbol
tables.

## Reading and writing properties

| Method | Meaning |
|---|---|
| `m.get(o, key) -> ?V`, `m.set(o, key, v)` | Read, and write (adding if absent) |
| `m.add(o, key, v)` | Add a property known to be absent |
| `m.delete(o, key) -> bool` | Delete |
| `m.has_own(o, key)`, `m.find(o, key) -> ?u32` | Test; find the slot index |
| `m.len(o)`, `m.keys(o, &vec)` | Count; copy the keys out in insertion order |
| `m.load(o, i)`, `m.store(o, i, v)` | By slot index |
| `m.load_ref(o, r)`, `m.store_ref(o, r, v)` | By slot reference (below), with no bounds checks |
| `m.load_inline(o, i)`, `m.load_out(o, i)` | For cache members that know where the slot is |

**Slots.** Property *i* (in insertion order) lives in slot *i*: in
`inline[i]` below the inline count, and in `overflow.data[i - inline_cap]`
above it. Overflow arrays grow by 1.5× through your `new_slots`
function. A cache stores a *slot reference*: the inline index, or the
overflow index with bit 31 (`obj.OUT`) set, as computed by
`obj.slot_ref(i, cap)`.

### Dictionary mode

Objects switch to dictionary mode when the shape approach stops paying:

- Deleting the last property added just steps back to the previous
  shape.
- Deleting any other property, or adding one past `dict_threshold`,
  switches the object to dictionary mode. It gets the shared dictionary
  shape (id `obj.DICT_ID`, never cached) and an `obj.Dict`, which maps
  keys to slot indices. The values stay in the object's slots.
- `m.optimize(o)` turns a dictionary-mode object with at most
  `dict_threshold` properties back into a fast one.
- `m.to_dict(o)` forces dictionary mode, and `m.is_dict(o)` tests for
  it.

## Attributes and key order

Every property has a `u32` of attribute bits whose meaning is yours
(MiniJS: bit 0 means "not enumerable"). They are part of the shape.

- `m.define(o, key, v, attrs)` adds or redefines a property with
  attributes.
- `m.set_attrs(o, key, attrs)` changes them. A fast object moves to the
  shape with the new attributes; its slots stay put.
- `m.attrs(o, key)` reads them.
- `m.keys_where(o, &out, skip)` lists the keys that have none of the
  `skip` bits.
- Write caches are not filled for properties with any of the bits in
  `m.ic.readonly`, so that read-only properties and accessors reach the
  interpreter's slow path, where it can check them.

Keys are kept in insertion order. JavaScript's order (integer indices
first, ascending, then the rest in insertion order) is
`obj.index_keys_first(&keys, data, index_of)` (or
`index_keys_first_by(&keys, index_of)` with a closure), given your
function that says whether a key is an array index. `std.obj` has no
element store of its own.

## Inline caches

The vm declares the cache words and does the check itself, so the hit
path stays in the handler. `std.obj` runs the cache state machine
(monomorphic, polymorphic, megamorphic) in the slow paths. Its state
lives in the six words of an `obj.Site`: `cache PropIC = obj.Site;`
declares them, and `&cache` is the `*obj.Site` that the functions take.

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

`tests/run/std_obj_sites.lm` has the complete version.

### Cache states

`obj.IcState` names the family members: `Generic`, `OwnIn`, `OwnOut`,
`Proto`, `Poly`, `Mega`, and `Add` for write sites.

| State | Hit check |
|---|---|
| `OwnIn`, `OwnOut` | Shape id compare, then the slot (inline or overflow) |
| `Proto` | Shape id compare, then `m.proto_load(o, slot, cell, depth)` |
| `Poly` | `m.poly_get(o, ent)`: up to eight hits, in `m.ic.poly` at index `site.ent` |
| `Mega` | `m.mega_get(o, key)`: a shared cache, `m.ic.mega` |

### Reads

`m.read_miss(site, state, o, key)` reads through the prototype chain and
returns the value and the next state:

- A monomorphic site that misses is refilled up to `m.ic.refills` times
  (default 8; many sites are monomorphic within a program phase). Then
  it becomes polymorphic, keeping its old hit.
- A `Poly` entry that fills up starts over up to `m.ic.resets` times
  (default 8). Then the site goes megamorphic.

The megamorphic cache `m.ic.mega` (an `obj.MegaCache`, 4096 entries) is
direct-mapped on (shape id, key) and holds both own and prototype hits.
It is never cleared. `m.get_mega(&c, o, key)` is an own-properties-only
variant for object types without a `proto` field.

A hit is an `obj.Hit { sid, slot, cell, depth }` of plain words. For own
properties `cell` and `depth` are 0. For prototype hits `cell` is the
validity cell of the receiver's prototype, and `depth` the number of
`proto` links to the object holding the property. `m.read(o, key, &hit)`
fills one, and `m.hit(o, &hit)` checks one.

### Where a value came from

Languages that bind methods (Lox, Python, Ruby) must tell a value found
on the receiver (a field, which may hold a function) from one found
through its class. In the monomorphic states, the member says so
(`Proto` versus `OwnIn`/`OwnOut`). For the others:

- `m.poly_get_at(o, ent)` and `m.mega_get_at(o, key)` return
  `?(V, u32)`: the value and its depth. Depth 0 is an own property,
  n >= 1 was found n prototype links up, and `obj.ABSENT` is a cached
  absent property.
- `m.read_miss_at(..)` returns `(?V, IcState, depth)`.

The depth is already in the cache entry, so this costs only a register:

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

### Writes

- `m.write_miss(site, o, key, v)` stores the value. It returns
  `OwnIn`/`OwnOut` for an existing property of a fast-mode object, or
  `Add` when the property was added into an inline slot. For `Add`, the
  cache holds the old shape id, the slot, and (in `site.ent`) the new
  shape. `m.add_hit(o, ent, slot, v)` is the hit after the shape check:
  a store and a shape change.
- `m.write_miss_in(site, state, o, key, v)` is `write_miss` for a site
  in a given state, with polymorphic stores. A monomorphic store site
  that misses `m.ic.refills` times becomes `Poly` (own-slot hits only;
  `m.poly_set(o, ent, v)` is the hit). A Poly entry that fills up
  `m.ic.resets` times makes it `Mega`, which stores through the shared
  cache (`m.mega_set(o, key, v)`, only while `m.ic.readonly` is 0) and
  never changes state again.

### Caching absent properties

With `m.ic.absent` set (off by default), reads that find nothing are
cached too, as hits with slot `obj.ABSENT` in Poly and Mega entries.
Monomorphic members cannot express them, so such a read makes a site
polymorphic at once. `poly_get`, `mega_get` and `hit` return your nil
value for them, so turn this on only for languages where reading an
absent property gives nil. `m.has_mega(o, key)` is `key in o` through
the shared cache.

### Simple monomorphic caches

`m.get_fill(o, key, &shape_word, &slot_word)` and `m.set_fill(..)` are
the slow paths for a vm that only wants monomorphic own-property caches
(`tests/run/std_obj_ic.lm`).

### Cost of a hit

A hit costs two dependent loads (`o.shape`, `shape.id`), a compare
against the cache word, and the slot load: 0.8 ns over a register move
on an i7-6700. `bench/obj/README.md` shows the machine code. The hit
helpers (`load_ref`, `proto_load`, `poly_get`, `poly_get_at`,
`mega_get`, `mega_get_at`) are `#[inline]`, so a handler's hit path
makes no call and needs no stack frame.

## Prototype chains and validity cells

- `m.lookup(o, key)` follows `proto` fields and returns `Lookup.Own(i)`,
  `Lookup.Proto(holder, i)` or `Lookup.Missing`.
- `m.get_chain(o, key)` returns the value.
- `m.set_proto(o, p)` changes an object's prototype.

A prototype hit is checked by `m.proto_load(o, slot, cell, depth)`
after the receiver's shape compare: it tests the cell, then follows
`depth` `proto` links and loads the slot. The cell is that of the
receiver's prototype, from `m.cell(p)`.

**Validity cells.** A cell records that an object's layout has not
changed since a cache used it.

- Adding, deleting or changing the attributes of a property of an
  object with a cell invalidates the cell. So do switching it to
  dictionary mode, compacting its dictionary, and changing its
  prototype.
- Invalidating a cell also invalidates the cells of objects further down
  the chain.
- Storing a new value into an existing property does not invalidate it.
- When an object dies, its cell is invalidated, so a new object at the
  same address cannot revive old hits.
- Cells are made only for prototypes that a cache reads through (and
  again after each invalidation). Each costs 17 bytes, and cell ids are
  never reused.

**Prototype objects get their own shapes.** An object that gets a cell,
and every object above it in the chain, moves to a separate tree of
shapes flagged `obj.PROTO`. Interpreters may also call `m.make_proto(o)`
when an object becomes a prototype; `m.is_proto(o)` tells. Because
these shape ids never equal those of ordinary objects, `add_hit` need
not check for cells.

## Allocation templates

A template sizes the inline slots of the objects made at one allocation
site:

- `obj.Template.new(proto_bits, inline)` creates one.
- `m.template_root(&t)` is the root shape for the site's next object,
  with `t.inline` inline slots.
- `m.observe(&t, o)` raises `t.inline` to the number of properties a
  finished object has, up to `max_inline` (32).

## Shapes and their ids

A shape (`obj.Shape`) is a node in a tree: each adds one property
(key and attributes) to its parent. Shapes are found through a global
transition table from (parent id, key, attributes) to the child.

**Ids are stable across collections.** A shape that dies is freed, but
its id is remembered. A shape rebuilt along the same path (the next
object built the same way after a collection) gets the same id back. So
an id names a sequence of property additions, which fixes the layout.
What this means for caches:

- A cache entry holding a shape id can be stale but never wrong.
- No id is ever given to a different path.
- Caches need no reset after a collection.

**The cost is memory.** Each remembered dead path costs a 32-byte entry,
but no shape object. `shapes.retain` (default 65536) bounds the number of
dead entries kept; past it, all are forgotten (counted in
`shapes.forgotten`), and those paths get new ids, which caches see as
new shapes. The default costs at most about 4 MB, and only for programs
that build many distinct shapes. `retain = 0` makes ids die with their
shapes, which can make sites megamorphic in programs that rebuild the
same object graph again and again. `shapes.reborn` counts the shapes that
got their old id back.

Ids come from a `u32` counter: 0 is an empty cache, and `obj.DICT_ID`
(`0xFFFFFFFF`) the dictionary shape. Four billion distinct paths abort
the process.

**Prototype identity.** The prototype is part of a root shape's key, as
an address. A prototype that dies and a new one at the same address
share shape ids. That is harmless: own properties have the same layout,
and prototype hits are protected by the old prototype's validity cell,
which died with it.

**Lookup.** For chains of up to 8 properties, a lookup walks the shape
chain. Longer chains build a key → (index, attributes) table on first
lookup and keep it on the shape.

## Effects

Nothing in `std.obj` collects. Every function is free of effects, so
handlers can call them from fast paths and cold blocks without syncing.

## Limitations

- Attributes are bits that the interpreter interprets; there is no error
  for a refused store.
- There is no integer-indexed element storage: arrays are the
  interpreter's.
- Global variables are the interpreter's: a property cell would have to
  hold the interpreter's `Value`. Mini's `GETGLOBAL` slot cache plays
  this role.
- `keys` copies the keys out; iteration does not track changes.
- Each shape has its own lookup table; tables are not shared along a
  chain.
- The cache state machine reads a `proto` field, so object types without
  one can use only the monomorphic helpers and `get_mega`.
