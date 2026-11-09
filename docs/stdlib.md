# Standard library

This is the reference for Loimi's standard library: what each module
offers and how to use it. It is for interpreter authors looking
something up; the [tutorial](tutorial/) shows the modules in use.

The library covers the parts of an interpreter around the inner loop:
getting from source text to bytecode, and the runtime structures that
dynamic languages need. Each module lives in `std/NAME.lm` and is
imported with `import std.NAME;`.

| Module | What it's for |
|---|---|
| [`std.core`](#stdcore-built-in-types-and-operations) | Built-in types and their methods: integers, floats, vectors, options, pointers. Needs no import. |
| [`std.mem`](#stdmem) | Allocation, `Vec`, sorting, byte search, the `Arena` and `Malloc` allocators |
| [`std.hash`, `std.map`](#stdhash-and-stdmap) | The `Hash` trait, `HashMap`, the insertion-ordered `OrderedMap` |
| [`std.dict`](#ordered-maps-maporderedmap-and-dictdict) | `Dict` and `KeyDict`: insertion-ordered hash tables in the GC heap, for guest dicts |
| [`std.text`](#stdtext) | `StrBuf` with `{}` formatting, `Interner`, small string helpers |
| [`std.num`](#stdnum) | Exact number parsing and formatting, floor division |
| [`std.math`](stdlib/math.md) | `Divisor`: fast division by a constant known ahead of time |
| [`std.bigint`](#stdbigint) | Arbitrary-precision integers with Python's semantics |
| [`std.io`, `std.os`](#stdio-and-stdos) | Printing, buffered output, reading files, program arguments |
| [`std.lex`](#stdlex) | Building blocks for lexers |
| [`std.parse`](#stdparse) | Pratt parsers, and operator tables defined at run time (Prolog) |
| [`std.diag`](#stddiag) | Source spans and error messages with source snippets |
| [`std.debug`](#stddebug-guest-debuggers) | Breakpoints, stepping and commands for a source-level debugger of the guest language |
| [`std.bc`](#stdbc-bytecode-construction) | Emitting bytecode: labels, jumps, constant pools, register allocation |
| [`std.gc`](#stdgc) | The garbage collector ([full guide](stdlib/gc.md)) |
| [`std.gil`](stdlib/gc.md#threads-sharing-a-heap-stdgil) | A global interpreter lock, for threads that share a heap |
| [`std.obj`](#stdobj-objects-and-shapes) | Objects with hidden classes (shapes) and inline caches ([full guide](stdlib/obj.md)) |
| [`std.rt`](#stdrt-runtime-support) | Error unwinding (`protect`/`throw`), catching panics, value stacks |
| [`std.prof`](#stdprof) | Timers, benchmarks, counters, `--vm-stats` counts |
| [`std.test`](#stdtest) | `#[test]` functions, assertions, captured output, golden files |

## Conventions

These rules hold across the library.

**Freeing memory.** Every type that holds heap memory has a `free()`
method:

- `Vec`, `Arena`, `HashMap`, `OrderedMap`
- `StrBuf`, `Interner`, `Out`
- `OpTable`, `Keywords`, `Pratt`, `Parser`, `Ops`, `SourceMap`, `Diag`,
  `Diagnostics`
- `Code`, `Pool`
- `Heap`, `Shapes`, `MegaCache`, `Ics`, `Model`, `Ephemerons`,
  `WeakInterner`, `Gil`
- `BigInt`, `ValueStack`, `Bench`, `Counters`

`Malloc` frees each allocation with `free_bytes`, and the result of
`io.read_file` is released with `io.free_file`. `free()` leaves a
container empty and usable. Freeing a container does not free what its
elements point to.

**Pointers into containers.** Pointers and slices you get from a
container (`items()`, `ptr_at`, `get_ptr`, `entry`) stay valid until
the container next grows or is freed. The compiler does not check this.

**Safe modules.** `std.lex`, `std.parse`, `std.diag`, `std.text`,
`std.hash`, `std.io`, `std.bc`, `std.map` and `std.dict` are `#![safe]`
modules: they use no raw-pointer indexing, arithmetic or casts outside a
few commented `unsafe { }` blocks around C calls.

Functions that take a pointer and a length, free memory, or return raw
memory are marked `#[raw]`. A `#![safe]` module can call them only
inside `unsafe { }`; elsewhere they are ordinary functions. Examples:

- `mem.alloc`, `grow`, `release`, `copy`, `sort_by`, `try_sort_by`
- `Arena.alloc_bytes`, `Malloc.alloc_bytes`, `Malloc.free_bytes`
- `io.free_file`, `map.probe_*`
- `std.obj`'s inline-cache readers and writers, which trust their
  cache words (`Model.load_ref`, `hit`, `poly_get`, `MegaCache.put`, ...)

**AddressSanitizer.** Under `--sanitize=address`, `Vec`, `Arena` and
`OrderedMap` poison the memory they keep but no longer use, so ASan
reports a stale pointer into it just as it reports one into freed
memory. Your own containers can do the same with `mem.poison(p, n)` and
`mem.unpoison(p, n)` (`#[raw]`, and defined only in such builds: guard
them with `#[cfg(sanitize_address)]`).

The safety audit of every module is in
[`design/safety.md`](design/safety.md).

---

## `std.core`: built-in types and operations

These are built into the compiler. There is no `std/core.lm` and no
import.

### Types

- Integers: `i8`..`i64`, `u8`..`u64`, `isize`, `usize`, and any width
  `iN`/`uN` from 1 to 64.
- `f32`, `f64`, `bool`.
- `str`, which is a `[]u8` slice.
- Slices `[]T` (`.len`, `.ptr`, `s[a..b]`), arrays `[T; N]`, pointers
  `*T`.
- `?T`, `Result<T, E>`, tuples.
- 128-bit vectors `i8x16` .. `f64x2`.

### Integer arithmetic

| Operation | Meaning |
|---|---|
| `+ - *` | Trap on overflow in debug builds, wrap in release builds |
| `+% -% *%` | Always wrap |
| `a.add_checked(b)`, `a.sub_checked(b)`, `a.mul_checked(b)` | Return `?T`: `None` on overflow |
| `x.clz()`, `x.ctz()`, `x.popcount()` | Bit counts, on any integer type |
| `<<`, `>>` | Shifts |

For the `(T, bool)` overflow forms, see `num.add_overflow` and friends
in [`std.num`](#stdnum) (`i64` only).

### Wide arithmetic

On `u64`, for multi-word integers such as [`std.bigint`](#stdbigint):

| Method | Result |
|---|---|
| `a.mul_wide(b) -> (u64, u64)` | The full 128-bit product, as (low, high) |
| `hi.div_wide(lo, d) -> (u64, u64)` | `hi * 2^64 + lo` divided by `d`, as (quotient, remainder) |
| `a.mul_add_wide(b, c, d) -> (u64, u64)` | `a * b + c + d` as (low, high). It always fits, so it is the step of a multiply-and-accumulate loop. |

For `div_wide` the caller must make sure `hi < d`, so the quotient fits
(and `d` is nonzero), as with the x86-64 `div` instruction. `mul_wide`
compiles to one `mul` instruction; `div_wide` calls the runtime's
`__udivti3`.

### Carry chains

On `u8`, `u16`, `u32` and `u64`:

- `a.add_carry(b, c) -> (T, bool)` is `a + b + c`, with the `bool`
  carry-in counting as 1. It returns (sum modulo 2^bits, carry out).
- `a.sub_borrow(b, c) -> (T, bool)` is `a - b - c`, returning
  (difference modulo 2^bits, borrow out).

A loop that feeds each carry out into the next carry in compiles to a
chain of `adc`/`sbb` instructions on x86-64 (`adcs`/`sbcs` on AArch64),
with the carry kept in the flags register when nothing in between sets
flags. Use `c as u64` to turn a carry into a limb.

### Floats

These work on `f32` and `f64` alike, and each compiles to one LLVM
intrinsic.

| Method | Meaning |
|---|---|
| `x.to_bits()`, `f32.from_bits(u32)`, `f64.from_bits(u64)` | Reinterpret as bits (`u32`/`u64`) and back |
| `x.sqrt()`, `x.abs()` | Square root, absolute value |
| `x.floor()`, `x.ceil()`, `x.trunc()` | Round to an integral value |
| `x.nearest()` | Round to nearest, ties to even: 2.5 → 2.0, -0.5 → -0.0 (wasm `nearest`, C `roundeven`) |
| `x.copysign(y)` | `x`'s magnitude with `y`'s sign bit |
| `x.is_nan()` | NaN test |
| `x.min(y)`, `x.max(y)` | C `fmin`/`fmax` (IEEE 754-2008 minNum): a NaN operand is ignored |
| `x.minimum(y)`, `x.maximum(y)` | IEEE 754-2019 `minimum`/`maximum`: NaN propagates, `-0.0 < 0.0` |

The two kinds of min/max differ on NaN and on zeros:

- `min`/`max` return the other operand if one is NaN, and NaN only if
  both are. What a signaling NaN gives is unspecified. `min(-0.0, 0.0)`
  may return either zero. These are the cheaper choice when NaN cannot
  occur.
- `minimum`/`maximum` return a quiet NaN if either operand is NaN (a
  signaling one is quieted, as wasm's "arithmetic NaN" requires), and
  treat `-0.0` as less than `0.0`. This is what wasm's `f32.min`/`f64.max`
  and JavaScript's `Math.min` specify: use these to implement a guest's
  IEEE semantics.

On x86-64, `sqrt`, `abs`, `copysign` and both kinds of min/max compile
to SSE2 instructions. The rounding functions need SSE4.1, which the
default x86-64 target lacks, so there they call libm (`floor`,
`roundeven`, ...). AArch64 has rounding instructions in the base ISA.

`as` converts between integers and floats, and between `f32` and
`f64`. For floor division and the remainder, see [`std.num`](#stdnum).

### Vectors

The 128-bit vector types `i8x16` .. `f64x2` have:

- Construction: `T.splat(x)`, `T.new(..)`, `T.from_bits(v)`,
  `T.narrow_sat(a, b)`.
- Lanes: `v.lane(i)`, `v.with_lane(i, x)`, with a constant `i`.
- Lane-wise operators. Comparisons give masks.
- Methods: `abs`, `min`, `max`, `add_sat`, `sub_sat`, `avgr`, `clz`,
  `ctz`, `popcount`, `select`, `bitmask`, `widen_low`, `widen_high`,
  `shuffle`, `swizzle`.
- The float methods above, lane by lane (`sqrt` through
  `minimum`/`maximum`, but not `is_nan` or `to_bits`).
- `as` between vectors with the same lane count.

### Options and results

- `o.is_some()`, `o.is_none()`, `o.unwrap()` (traps on `None`),
  `o.unwrap_or(d)`.
- `o ?? d` gives the value of `o`, or `d` if `o` is `None`.
- `o ?? p`, with `p: ?T`, is a `?T`: `o` if it is `Some`, else `p`.
  `p` is evaluated only when needed. Chains work: `a ?? b ?? c`.
- `?` returns early on `None` or `Err`.

`Result` has no methods: use `match`, `if let` or `?`.

### Pointers

- `p.offset(n)` (in elements, `n: i64`), `p + n`, `p[i]`, `*p`.
- `p.slice(n) -> []T`.
- `p.read_unaligned()` and `p.write_unaligned(v)`, for addresses that
  may not be aligned for `T`.
- `p.prefetch()`, a cache hint that never faults.

### Other built-ins

- `size_of<T>() -> u64`.
- `likely(c)`, `unlikely(c)`, `assume(c)`.
- `trap() -> never` and `unreachable`.
- With `std.gc` imported: `new(heap, Obj { .. }, n)` and
  `gc_mark(heap, v)`.

String equality is `a.eq(b)`, defined in `std.hash` (`import
std.hash;`). `==` does not compare slices.

## `std.mem`

Memory allocation, vectors and sorting.

### Allocation

| Function | Meaning |
|---|---|
| `mem.alloc<T>(n) -> *T` | `n` zeroed elements. Out of memory is a panic, which aborts unless caught (see [`std.rt`](#stdrt-runtime-support)). |
| `mem.try_alloc<T>(n) -> ?*T` | The same, but `None` when out of memory |
| `mem.grow<T>(p, n) -> *T` | Resize |
| `mem.release(p)` | Free |
| `mem.copy(dst, src, n)` | Copy `n` elements; the ranges may overlap |

### Byte strings

These work on any `[]u8`, so on `str` too, and use libc underneath. For
a raw pointer, call `p.slice(n)` first.

| Function | Meaning |
|---|---|
| `mem.find_byte(s, c) -> ?u64` | First position of byte `c` (`memchr`) |
| `mem.rfind_byte(s, c) -> ?u64` | Last position (`memrchr`) |
| `mem.find_bytes(s, needle) -> ?u64` | First position of `needle` (`memmem`) |
| `mem.compare_bytes(a, b) -> i32` | -1, 0 or 1; a prefix sorts first |
| `mem.equal_bytes(a, b) -> bool` | Equality |

### `Vec<T>`

A growable array, `Vec<T> { ptr, len, cap }`:

- Create: `Vec.new()`, `Vec.with_capacity(n)`, `Vec.filled(n, x)`
  (`n` copies of `x`).
- Add and remove: `push`, `pop -> ?T`, `extend(s)` (append a slice,
  which may be the vector's own items), `truncate(n)`, `clear`.
- Access: `get(i) -> ?T`, `at(i)`, `ptr_at(i)`, `set(i, x)`,
  `last -> ?T`, `items -> []T`.
- Capacity: `reserve(extra)`.
- Sort: `sort_by(cmp)`, `try_sort_by(less)` (below).
- `free`: leaves the `Vec` empty and usable. Freeing twice is fine.

**An empty `Vec` allocates nothing.** `Vec.new()` and
`with_capacity(0)` have capacity 0 and a static placeholder pointer;
the first push allocates. `mem.release` and `mem.grow` accept the
placeholder pointer, so code that takes over `v.items().ptr` works for
empty vectors too. Don't pass it to C's `free`.

**Objects with optional parts.** A `*T` field cannot be null, so an
object built before its parts exist (say, a compiled regex that may
hold an error instead) needs a value for each part. Make each optional
part either:

- a `Vec<T>`: an empty one costs nothing to create, overwrite or drop;
  or
- a `?*T`: `None` until allocated, released with
  `if let Some(p) = x.p { mem.release(p); }`.

Never use a `mem.alloc(1)` dummy: it leaks when overwritten.

### Sorting

`mem.sort_by(p, n, cmp)` is a stable merge sort. `cmp` is a
`block fn(*T, *T) -> i32`: a closure capturing locals, or a plain
function.

`mem.try_sort_by(p, n, less) -> Result<(), E>` (and
`Vec.try_sort_by(less)`) is a stable sort for comparators that can fail
and run guest code, such as a dynamic language's `<`, which may raise
an error or call a user's `__lt__`.

- `less` is a `block fn(*T, *T) -> Result<bool, E> uses reenter`
  saying whether `a` goes before `b`. The sort itself is `uses reenter`.
- The first `Err` stops the sort and is returned. The elements are then
  unchanged.
- The comparator gets pointers into `p`, and `p` is not changed while it
  runs: the sort orders indices, then permutes `p` once, with no calls.
  So elements that `p` keeps reachable stay reachable if the comparator
  collects.
- `p` must stay valid throughout. If guest code could change it, sort a
  copy.

Python's `sorted` and `list.sort` use it (`examples/python/builtins.lm`).

### Allocators

**`Arena`** hands out memory that is freed all at once:

- `Arena.new()`, `alloc_bytes(n, align) -> *u8`, and `str(s) -> str`,
  which copies a string into the arena.
- `free()` releases everything. The arena stays usable: the next
  allocation starts a new chunk.
- Memory comes in 64 KiB chunks; larger requests get their own chunk.
- `free_bytes(p)` does nothing. It exists so that `c.free(&arena)` on an
  escaping closure, and code generic over the allocator, work.

**`Malloc`** allocates on the C heap, and frees allocations one by one:

- `Malloc.new()`, `alloc_bytes(n, align) -> *u8` (alignment up to 16),
  `free_bytes(p)`.
- `live` is the number of allocations not yet freed.
- Use it for escaping closures freed one at a time (`new(&m, |..| ..)`,
  `c.free(&m)`) and for cells (`new(&m, v)`, `m.free_bytes(p as *u8)`).

## `std.hash` and `std.map`

Hashing and host-memory hash tables.

**`hash.Hash`** is a trait with `fn hash(k: Self) -> u64` and
`fn eq(a: Self, b: Self) -> bool`. It is implemented for `u64`, `i64`,
`u32`, `i32` and `str`. For your own key type, define both methods and
declare `impl hash.Hash for Key;`. The building blocks are `hash.mix(x)`
(a 64-bit mixer) and `hash.bytes(s)`.

**`map.HashMap<K: hash.Hash, V>`** uses linear probing and is at most
3/4 full:

- `HashMap.new()`, `free`.
- `insert(k, v) -> bool` (true if the key was new), `get(k) -> ?V`,
  `get_ptr(k) -> ?*V`, `contains`, `remove -> bool`.
- `len` (a field), `clear()` (keeps the table's memory).
- Iteration:
  `var i: u64 = 0; while let Some(s) = m.next(&i) { .. s.key, s.value .. }`.

**`map.OrderedMap<K: hash.Hash, V>`** is a hash map that iterates in
insertion order. It lives in host memory; `std.dict.Dict` (below) is the
same table in the GC heap.

### Ordered maps: `map.OrderedMap` and `dict.Dict`

Both use CPython's compact dict layout:

- An index table of `u32` slots (a power of two in size). Each slot is
  empty, deleted, or an entry number.
- A dense array of entries `map.Entry<K, V> { hash, key, value }` in
  insertion order, at most 2/3 of the index size.

What this means in use:

- **Removal** marks the entry dead and its index slot deleted. A dead
  entry's hash becomes `map.DEAD` and its key and value bytes are
  zeroed, so a traced table no longer keeps them alive. The next resize
  compacts the entries.
- **Stored hashes.** Entries keep their hashes, so resizing and copying
  never call `hash` or `eq`. An interpreter whose hashing runs guest
  code (`__hash__`) calls it once per key.
- Probing is CPython's (`i = 5i + 1 + perturb`, `perturb >>= 5`).

#### The usual API

Through `hash.Hash` on `K`:

- `OrderedMap.new()`, `with_capacity(n)`, `free`.
- `insert(k, v) -> bool`: true if the key was new. A replaced key keeps
  its place.
- `get -> ?V`, `get_ptr -> ?*V`, `contains`, `remove(k) -> ?V`.
- `pop_last() -> ?(K, V)`. It reclaims dead entries at the end, so a
  stack-like use stays compact.
- `clear()` (keeps the memory), `len` (a field).
- Iteration in insertion order:
  `var i: u64 = 0; while let Some(e) = m.next(&i) { .. e.key, e.value .. }`.

#### Supplying the hash yourself

For interpreters that have a cached hash for a string, or want an
identity test before the general equality:

| Method | Meaning |
|---|---|
| `find_by(hash, eq: block fn(K) -> bool) -> ?u64` | Look up; gives an *entry index* |
| `put_by(hash, k, v, eq) -> (u64, bool)` | Insert or replace, in one probe |
| `insert_new(hash, k, v)` | Append a key the caller knows is absent |
| `entry(e) -> *Entry`, `set_value(e, v)`, `is_live(e)`, `remove_at(e)` | Work on an entry index |

`eq` gets a copy of each candidate key whose hash matches. For
`OrderedMap`, passing an `e` that is not a live entry is a panic.
`dict.Dict` checks only `remove_at` and bounds; a dead entry reads as
zeros.

**Entry indices and `version`.** An entry index stays valid until the
table is compacted. `version` changes on every insertion of a new key,
on removal, on compaction and on `clear`, but not when a value is
updated. So an inline cache can hold an entry index and check the key
there (Python's attribute caches), or hold the version (a cached
global).

#### Lookups that can fail and run guest code

`find_with<E>(hash, eq: block fn(K) -> Result<bool, E> uses reenter)
-> Result<?u64, E> uses reenter`:

- An `Err` from `eq` stops the lookup and is returned.
- If `eq` changes the map (inserts, removes or resizes, so `version`
  changes), the probe starts over on the current table. The answer is
  therefore about the map as it is when `find_with` returns. This is
  CPython's rule.
- An `eq` that changes the map on every call makes the lookup loop
  forever, as in CPython.
- To insert with such an equality, call `find_with` and then
  `set_value` or `insert_new`. Nothing runs between them.

<!-- snippet: run "1 3 2 | 20 negative key" -->
```loimi
import std.io;
import std.hash;
import std.map.{OrderedMap};

/// Keys are equal when equal mod 10; the equality can fail.
fn h10(k: i64) -> u64 { hash.mix((k % 10) as u64) }
fn eq10(a: i64, b: i64) -> Result<bool, str> {
    if a < 0 { return Err("negative key"); }
    Ok(a % 10 == b % 10)
}

fn main() uses reenter {
    var m: OrderedMap<i64, i64> = OrderedMap.new();
    m.insert_new(h10(1), 1, 10);
    m.insert_new(h10(3), 3, 30);
    m.insert_new(h10(2), 2, 20);
    var i: u64 = 0;
    while let Some(e) = m.next(&i) { io.print_int(e.key); io.print(" "); }
    io.print("| ");
    let hit: Result<?u64, str> = m.find_with(h10(12), |k| eq10(k, 12));
    if let Ok(Some(e)) = hit { io.print_int(m.entry(e).value); }
    let bad: Result<?u64, str> = m.find_with(h10(2), |k| eq10(-k, 2));
    if let Err(msg) = bad { io.print(" "); io.print(msg); }
    io.println("");
    m.free();
}
```

#### `dict.Dict`: the GC version

`dict.Dict<K: hash.Hash, V>` (in `std/dict.lm`) is an `object` whose
index and entries are GC objects, so keys and values may be `Value`s or
`Gc<T>`s. A guest dict can be a `Gc<Dict<Value, Value>>`, as in
examples/python.

It has the same API, with these differences:

- The dict is a `Gc` receiver, and calls that may allocate take the
  heap: `Dict.new(h, n)`, `insert(h, k, v)`, `put_by(h, ..)`,
  `insert_new(h, ..)`, `copy(h)`.
- Like any `new`, these never collect, so they can run in a handler's
  fast path.
- `try_insert_new(hash, k, v) -> ?u64` appends without resizing.
- `has_room()` and `try_add_by(hash, k, v, eq) -> ?u64` insert without
  resizing or allocating. They return `None` if the key is present or
  the dict is full. Python's attribute-store cache uses this to append
  a new attribute.
- Iteration is by entry index: `next(&i) -> ?u64`, then `key(e)`,
  `value(e)`, `hash_at(e)`. This stays usable across allocation and
  guest code; compare `version` to detect "changed during iteration".
- `find_with` re-reads the dict's tables after each `eq` call.

The stored hash makes each entry 24 bytes rather than 16 for `Value`
keys. For instance attributes, `KeyDict` avoids that.

#### `dict.KeyDict`: keys that carry their hash

`dict.KeyDict<K: dict.CachedHash, V>` is `Dict` without stored hashes,
for keys whose hash is cached in the key, such as interned attribute
names (CPython's str-keys dict). Its entries are
`dict.KeyEntry<K, V> { key, value }`, 16 bytes for `Value` keys.

The key type provides:

- `fn K.cached_hash(k) -> u64`: the hash callers pass for that key. It
  must not fail or run guest code. Computing and caching it on first
  use is fine.
- `fn K.eq(a, b) -> bool`.
- `impl dict.CachedHash for K;`.

Everything else is `Dict`'s: the index and probing, the whole API, entry
indices stable until a compaction, `version`, tracing, and no collection
on allocation. The differences:

- Resizing, removal and `copy` ask the key for its hash.
- A removed entry is zeroed and recognized by its all-zero key bytes,
  so a live key must never be all zero bits. (A string `Value`, being a
  tagged pointer, never is.)
- With no stored hash to filter by, `find_by`'s `eq` sees every live key
  on the probe path. Test identity first, then the candidate's cached
  hash, then the bytes (as `str_eq` in examples/python does).

examples/python keeps instance attributes in a `KeyDict<Value, Value>`;
`__dict__` gives a copy.

## `std.text`

Strings and formatting.

### `StrBuf`

A growable byte string:

- `StrBuf.new()`, `clear()`, `free()`, `len()`.
- `push(byte)`, `push_str(s)`, `push_int(i64)`, `push_u64(u64)`.
- `push_f64(f64)`: the shortest `%g` form that reads back as the same
  number (`num.write_g_shortest`). [`std.num`](#stdnum) has more float
  formats.
- `as_str()`: valid until the next push.

**Formatting.** `b.fmt2("expected {} values, got {}", n, m)` appends the
format string with each `{}` replaced by the next argument:

- `fmt` takes one argument; `fmt2` to `fmt4` take two to four.
- `{{` and `}}` are literal braces.
- `b.add(x)` appends one value.
- Both return the buffer, so calls chain:
  `` b.fmt("unknown word `{}`", name).add(" at line ").add(line).as_str() ``.

The format string is not checked at compile time. A `{}` without an
argument stays `{}`, and an argument without a `{}` is appended at the
end.

Arguments can be any type that implements the trait `text.Fmt`
(`fn fmt(x: Self, b: *StrBuf)`). Built in: `str`, all integer widths up
to 64 bits, `f64` (as `push_f64`) and `bool`. A `u8` prints as a number;
use `push` to append a byte. To make your own type formattable, write
`fn T.fmt(x: T, b: *text.StrBuf) { .. }` and `impl text.Fmt for T;`.

### `Interner`

- `Interner.new()`, `free()`.
- `intern(s) -> u32`: ids are 0, 1, 2, ...; the string is copied into
  the interner's arena.
- `find(s) -> ?u32`, `name(id) -> str`.

### Helpers

- `starts_with(s, prefix)`, `find(s, byte) -> ?u64` (`memchr`).
- `is_digit`, `is_alpha` (includes `_`), `is_space`.
- `parse_int(s) -> ?i64` (decimal), `parse_f64(s) -> ?f64` (C syntax,
  via strtod). [`std.num`](#stdnum) has exact parsers that report what
  went wrong.

## `std.num`

Number parsing and formatting for guest languages, and guest
arithmetic.

### Parsing

Parsers return `Result<_, NumError>`, which keeps `Invalid` and
`Overflow` apart. `num.error_message(e)` describes the error.

| Function | Parses |
|---|---|
| `parse_int(s)` | Sign, decimal or `0x` hex, exact `i64` range |
| `parse_i64(s, radix)`, `parse_u64(digits, radix)` | Radix 2 to 36 |
| `parse_u64_wrapping` | Wrapping hex, as in Lua hex literals |
| `parse_f64(s)` | Strict decimal syntax, then correctly rounded by strtod |
| `parse_number(s) -> Number` | `Number.Int(i64)` or `Number.Float(f64)`; decimal overflow becomes a float, as in Lua |

Helpers for lexers: `decimal_len(s)` (the length of a numeral),
`digit_value(c)`, `is_hex_digit(c)`.

### Formatting

These append to a `text.StrBuf`:

| Function | Output |
|---|---|
| `write_g(b, x, 14)` | Lua's `%.14g` (via snprintf) |
| `write_shortest(b, x)` | Shortest round-trip, Python `repr` notation without the `.0` |
| `write_g_shortest(b, x)` | `%.{p}g` with the fewest digits `p` that read back: `1e+02` for 100 |
| `write_js(b, x)` | ECMAScript `Number::toString` |
| `write_hex(b, x)` | A `u64` in lowercase hexadecimal, without a prefix |
| `mark_float(b, start)` | Appends `.0` if the text since `start` looks like an integer, so floats print as floats (`3.0`) |

`shortest(x)` gives the digits and exponent without formatting.

The shortest forms use the Schubfach algorithm. Their output is the same
as searching `%.{1..17}e` with snprintf and strtod for the fewest digits
that read back. Where several decimals are equally short, it takes the
closest to `x`. At exact powers of two it matches C's `%.{p}e` digits,
which need not be the shortest decimal (`2^-24` prints as
`5.9604644775390625e-08`). `write_shortest` takes about 80 ns per number.

### Guest arithmetic

- `floor_div`, `floor_mod`, `trunc_div`, `trunc_rem`: return `?i64`,
  `None` on a zero divisor. `MIN / -1` wraps.
- `floor_div_f64`, `floor_mod_f64`.
- `add_overflow`, `sub_overflow`, `mul_overflow`: return `(i64, bool)`.
- `to_i64_exact(f64) -> ?i64`.

## `std.bigint`

Arbitrary-precision integers with Python's semantics:

- floor and truncating division;
- infinite two's complement for bitwise operations on negative numbers;
- correctly rounded conversion to `f64`;
- CPython's `hash` of an int.

**The small case stays yours.** An interpreter keeps small integers
unboxed in its value type, and overflow checks (`num.add_overflow`,
`x.mul_checked(y)`) send it to `std.bigint`, which handles only the big
case.

### Three layers

All three share one representation: a sign and a magnitude of 64-bit
limbs, least significant first, with no zero limb at the top (zero has
no limbs).

**`Num`, a view.** `struct Num { neg: bool, d: []u64 }` is a number
whose limbs someone else owns. Operations read `Num`s and write the
result into a buffer you provide, sized by a matching `_size` function.
They return the result as a `Num` over the start of that buffer.

| Operation | Buffer size |
|---|---|
| `add_into(out, a, b)`, `sub_into` | `add_size(a, b)` |
| `mul_into` | `mul_size` |
| `divmod_into(q, r, a, b, floor)`: `None` for a zero divisor | `quot_size`, `rem_size` |
| `shl_into`, `shr_into` (arithmetic: `-1 >> n` is -1) | |
| `and_into`, `or_into`, `xor_into`, `not_into` | `bitop_size` |
| `pow_into` | `pow_size -> ?u64` (`None` when the size overflows) |
| `gcd_into(out, a, b)`: never negative | `gcd_size` |
| `isqrt_into(out, a) -> ?Num`: floor of the root, `None` for negative `a` | `isqrt_size` |
| `from_f64_into`: truncates, `None` for inf and NaN | `f64_size` |
| `parse_into(out, digits, radix)` | `parse_size` |
| `from_i64`, `from_u64` | one limb |

With `floor` true, `divmod_into` rounds the quotient down and gives the
remainder the divisor's sign (Python's `//` and `%`, Scheme's
`floor/`). Otherwise it truncates (C, Scheme's `truncate/`).

These need no buffer: `neg`, `abs`, `cmp`, `cmp_abs`, `eq`, `sign`,
`is_zero`, `is_odd`, `bit_length`, `to_i64` / `to_u64 -> ?..`,
`to_u64_wrapping`, `to_f64` (nearest, ties to even; ±inf beyond the
range), `hash_py` and `hash_i64` (CPython's int hash), and
`of_limbs(p, n, neg)` (a normalized view of limbs from elsewhere).

Nothing allocates except scratch space inside the algorithms, which is
freed before returning. So a view of a GC object's trailing array is a
valid operand, and its limbs can be the output.

**`BigInt`, owned.** `struct BigInt { neg: bool, d: mem.Vec<u64> }`
lives on the C heap.

- `bigint.add(a, b) -> BigInt`, and likewise `sub`, `mul`, `pow`,
  `shl`, `shr`, `bit_and`, `bit_or`, `bit_xor`, `bit_not`, `gcd`, `copy`.
- `divmod(a, b, floor) -> ?(BigInt, BigInt)`, `isqrt -> ?BigInt`.
- `of_i64`, `of_u64`, `of_f64`, `parse(s, radix) -> ?BigInt`.
- `x.num()` is the number's view; `x.free()` releases it.

They take views, so owned and GC numbers mix. Each call returns a new
`BigInt`.

**`Big`, in the GC heap.** `object Big { neg: bool, len: u64, d: [u64] }`;
the first `len` limbs of `d` are the number (results are allocated at
their size bound).

- `gc_add(h, a, b) -> Gc<Big>`, and likewise `gc_sub`, `gc_mul`,
  `gc_divmod`, `gc_pow`, `gc_shl`, `gc_shr`, `gc_and`, `gc_or`,
  `gc_xor`, `gc_not`, `gc_neg`, `gc_gcd`, `gc_copy`, `gc_of_i64`,
  `gc_from_f64`, `gc_parse`.
- `gc_isqrt -> ?Gc<Big>`.
- `b.num()` is the number's view.

They take `Arg`s, `enum Arg { Small(i64), Ref(Gc<Big>) }`, so a small
operand needs no object. Allocating does not collect, so these have no
effects. An interpreter can store a `Gc<Big>` in its own objects or as a
`value` variant's payload.

### Formatting and parsing

- `bigint.write(b, x, radix)` appends to a `text.StrBuf`: radix 2 to
  36, lowercase, `-` for negatives, no prefix.
- `parse_into` takes digits only: no sign, prefix, spaces or
  underscores. The guest language's syntax is the caller's job.

### Examples

100! with owned numbers:

<!-- snippet: run "93326215443944152681699238856266700490715968264381621468592963895217599993229915608941463976156518286253697920827223758251185210916864000000000000000000000000" "-ff" -->
```loimi
import std.io;
import std.text;
import std.bigint;
import std.bigint.{BigInt};

fn main() {
    var f = bigint.of_i64(1);
    var one: [u64; 1] = [0; 1];
    for i in 2..101 {
        // A small factor as a view of one limb on the stack.
        let g = bigint.mul(f.num(), bigint.from_i64(one[0..1], i));
        f.free();
        f = g;
    }
    var b = text.StrBuf.new();
    bigint.write(&b, f.num(), 10);          // 100!
    b.push(' ');
    var m = bigint.of_i64(-255);
    bigint.write(&b, m.num(), 16);
    io.println(b.as_str());
    f.free(); m.free(); b.free();
}
```

In an interpreter, the overflow of a small-int fast path goes to the GC
layer with the operands as `Arg`s, and a result that fits a small int
goes back to it (examples/python/big.lm works this way):

<!-- snippet: check
import std.gc;
import std.bigint;
import std.bigint.{Arg, Big};
value Value repr nanbox { Num(f64) unboxed, Int(i32), Big(Gc<Big>), Nil }
{snippet}
fn main() {}
-->
```loimi
fn arg(v: Value) -> Arg {
    match v { Int(i) => Arg.Small(i as i64), Big(b) => Arg.Ref(b), _ => Arg.Small(0) }
}

fn add(h: *gc.Heap, a: Value, b: Value) -> Value {
    if let (Int(x), Int(y)) = (a, b) {
        if let Some(r) = x.add_checked(y) { return Value.Int(r); }
    }
    let r = bigint.gc_add(h, arg(a), arg(b));
    if let Some(x) = bigint.to_i64(r.num()) {
        if x >= -2147483648 and x <= 2147483647 { return Value.Int(x as i32); }
    }
    Value.Big(r)
}
```

### Algorithms

Every operation except `gcd` is O(M(n) log n) or better, where M(n) is
the cost of Karatsuba or Toom-3 multiplication. The base cases are carry
chains (`add_carry`, `sub_borrow`, `mul_add_wide`). The thresholds are
`pub const`s, in limbs, tuned on one machine.

| Operation | Algorithm | From |
|---|---|---|
| Product | Schoolbook, two rows per pass | |
| | Karatsuba | `KARATSUBA` = 22 |
| | Toom-3 | `TOOM3` = 250 |
| | Unbalanced operands: cut into balanced pieces | |
| Square | Schoolbook, each cross product once | |
| | Karatsuba squaring | `KARATSUBA_SQR` = 40 |
| | Toom-3 squaring | `TOOM3_SQR` = 500 |
| Division | Schoolbook with Möller–Granlund 3/2 quotient limbs (Knuth's algorithm D for two-limb divisors) | |
| | Divide and conquer (Burnikel–Ziegler) | `DIV_DC` = 40 |
| One-limb divisor | Möller–Granlund, one reciprocal for the whole number | |
| Square root | Zimmermann's Karatsuba square root | |
| Power | Left-to-right binary | |
| Formatting, radix 2^k | Bit extraction | |
| Formatting, other radixes | A limb's worth of digits at a time | |
| | Divide and conquer | `DC_LIMBS` = 16 |
| Parsing | A limb's worth of digits at a time | |
| | Divide and conquer | `PARSE_DC` = 512 limbs' worth of digits |

There is no FFT multiplication and no subquadratic gcd.

### Performance

Against CPython 3.13 on the same work (`bench/bigint/bench.lm` and
`bench.py`, best of 5, one machine, 2026-11-03):

| | std.bigint | CPython 3.13 |
|---|---|---|
| factorial(5000), a running product | 1.75 ms | 4.8 ms |
| fib(100000) by fast doubling | 0.30 ms | 1.7 ms |
| `str` of a 99,722-digit number | 6.3 ms | 32 ms |
| `int` of it | 3.9 ms | 17 ms |
| product, 99,722 by 20,899 digits | 1.2 ms | 7.5 ms |
| `divmod`, 120,621 by 20,899 digits | 2.4 ms | 15 ms |

On larger numbers (7^e, best of 3), measured on an earlier version
that the current one beats by another 15–25%:

| Digits | | `str` | `int` | `divmod` by half size | `isqrt` |
|---|---|---|---|---|---|
| 299,165 | std.bigint | 44 ms | 25 ms | 15 ms | 10 ms |
| | CPython | 86 ms | 99 ms | 72 ms | 95 ms |
| 997,216 | std.bigint | 284 ms | 156 ms | 91 ms | 65 ms |
| | CPython | 365 ms | 666 ms | 478 ms | 635 ms |

Against GMP 6.3 (its `mpn`/`mpz` functions on the same operands, 4 to
1000 limbs), std.bigint takes:

- 1.2–1.8 times as long on sums, products and squares. GMP's x86-64
  assembly uses `mulx`, `adcx` and `adox`, which are beyond the baseline
  x86-64 that Loimi targets.
- 1.1–1.7 times as long on quotients and square roots.
- 1.6–1.8 times as long on decimal formatting.

## `std.io` and `std.os`

### `std.io`

Unbuffered output, one `write(2)` per call:

- `print(s)`, `println(s)`, `print_int(i64)`, `print_f64(f64)` (`%.14g`),
  `print_bool(b)`.
- The same on stderr: `eprint`, `eprintln`, `eprint_int`, `eprint_f64`.

Files: `read_file(path) -> ?str`, and `free_file(s)` to release the
result (`#[raw]`: `s` must come from `read_file`).

**Buffered output.** `io.Out` collects output and writes it in large
pieces. A program that prints a lot (an interpreter's `print` builtin)
makes one system call per 64 KiB instead of one per call.

- Create: `io.Out.stdout()` (a 64 KiB buffer) or `io.Out.new(fd, cap)`.
- Write: `o.print(s)`, `println`, `byte(c)`, `print_int`, `print_f64`,
  `print_bool`.
- The buffer is written out when it would overflow (a string larger than
  the buffer is written directly), and by `o.flush()` and `o.free()`
  (which flushes and releases).

There is no flush at exit. Flush before the program ends, and before
writing unbuffered to the same terminal (an `io.eprint` error message),
or the output comes out of order.

### `std.os`

- `arg_count() -> u64`, `arg(i) -> ?str` (argument 0 is the program).
- `quit(code) -> never`.

## `std.lex`

A toolkit for lexers built around a `Cursor`. A lexer is a short
function that tries the scanners in order:

<!-- snippet: check
import std.lex;
import std.num;
import std.text;
import std.diag;
enum T { Eof, Name, Number, String, Plus }
struct Token { kind: T, span: diag.Span }
struct Lexer { cur: lex.Cursor, ops: lex.OpTable<T>, keywords: lex.Keywords<T>, buf: text.StrBuf, num: num.Number }
fn Lexer.tok(lx: *Lexer, k: T, start: u64) -> Token { Token { kind: k, span: lx.cur.span_from(start) } }
{snippet}
-->
```loimi
fn Lexer.next(lx: *Lexer) -> Result<Token, lex.Error> {
    let c = &lx.cur;
    c.skip_trivia()?;
    let start = c.pos;
    if c.at_end() { return Ok(lx.tok(T.Eof, start)); }
    if let Some(name) = c.ident() { return Ok(lx.tok(lx.keywords.get(name) ?? T.Name, start)); }
    if c.at_number() { lx.num = c.number()?; return Ok(lx.tok(T.Number, start)); }
    if c.peek() == '"' { lx.buf.clear(); c.quoted(&lx.buf)?; return Ok(lx.tok(T.String, start)); }
    if let Some(k) = c.op(&lx.ops) { return Ok(lx.tok(k, start)); }
    Err(c.error_at(start, "unexpected character"))
}
```

A complete lexer for Mini's token set is about 50 lines
(`tests/run/lex_toy.lm`).

### `Syntax`

The conventions of one language. Presets: `lex.lua()`, `c_like()`,
`js()`, `none()`. The settings:

- Line comment prefix, and block comment delimiters (optionally
  nesting).
- Lua long-bracket comments.
- Octal or decimal `\ddd` escapes; whether backslash-newline keeps the
  newline; whether strings may contain raw newlines.
- Whether overflowing decimal or hex literals become floats, wrap, or
  are errors.
- Extra identifier bytes: `ident_extra` (allowed anywhere, such as `"$"`
  for JavaScript) and `ident_inner` (allowed after the first byte, such
  as `"?!"` or `"-"`).

### `Cursor`

`Cursor.new(src, file, syntax)` creates one.

- Reading: `peek`, `peek_at`, `bump`, `advance`, `eat`, `eat_str`,
  `starts_with`, `eat_while(fn(u8) -> bool)`, `at_end`.
- Position: `pos` and `line` (fields), `since`, `span_from`,
  `save`/`restore`.
- Errors: `error_at(start, msg)`, `error_here(msg)`.
- Identifier classes for the syntax: `is_ident_start(b)`,
  `is_ident_char(b)`.

ASCII character classes are free functions: `lex.is_digit`, `is_alpha`,
`is_ident_start`, `is_ident_char`, `is_hex_digit`, `is_space`.

### Scanners

| Scanner | Scans |
|---|---|
| `skip_trivia` | Whitespace and comments. Fails on an unterminated block or long comment. |
| `skip_space` | Whitespace and line comments only. Cannot fail; for syntaxes without block comments. |
| `ident -> ?str` | An identifier |
| `at_number`, `number` | A number, via `std.num`. `3abc` is reported as malformed. |
| `quoted(&buf)` | A string literal, with escapes decoded into a `StrBuf` (`\ddd`, `\xHH`, `\u{..}`, `\z`, line continuation). After a bad escape it still scans to the closing quote, so the lexer can continue. |
| `long_bracket_level`, `long_string` | Lua long strings |
| `op(&ops) -> ?K` | An operator from an `OpTable` |

`lex.push_utf8(buf, cp)` appends a code point. There is no "word"
scanner for whitespace-delimited languages: use `eat_while`.

### Tables

- `OpTable<K>`: `new`, `add(text, kind)`, `match_at(s, pos)`. Maps
  operator strings to token kinds, taking the longest match. Match with
  `cursor.op(&ops)` (or the older `ops.eat(&cursor)`).
- `Keywords<K>`: `new`, `add(word, kind)`, `get(word) -> ?K`. A wrapper
  over `HashMap<str, K>`.

### Errors

`lex.Error { span, msg }`, with a static message. `e.diag()` turns one
into a `diag.Diag`.

## `std.parse`

Expression parsers. `Pratt` and `Parser` are for operators fixed when
the parser is written; `Ops` is for operators defined while parsing.

### `Pratt`

`Pratt<P, K, E, X>` is a Pratt parser over:

- `P`, your parser state;
- `K`, a token-kind enum without payloads (used as a table index,
  `k as u64`);
- `E`, the expression result;
- `X`, the error type.

`Pratt.new(peek, advance, atom, unary, binary)` takes your hook
functions. The optional `before_rhs` hook is a field
(`pr.before_rhs = Some(f)`); a single-pass compiler uses it to put the
left operand in a register, or to emit the jump of `and`/`or`.

Fill the table at run time:

- `infix_left(k, bp)`, `infix_right(k, bp)`, `infix(k, l, r)`,
  `prefix(k, bp)`.
- `infix_of(k)` and `prefix_of(k)` read it back.

Then `parse(p, min_bp)` (or `expr(p)`) parses an expression.

**Operators that are not `lhs op rhs`** get a per-kind `led` hook:
`led(k, left, right, hook)`. When the operator binds, the parser
consumes it, and the hook parses the rest and returns the result. This
covers:

- mixfix operators (`c ? a : b`);
- postfix operators (`n!`);
- right-hand sides with their own grammar (assignment with a target
  check, `x => body`).

The hook parses operands with `pr.parse(p, pr.infix_of(k).right)`.
MiniJS parses assignment, `?:` and arrow functions this way. Postfix
chains such as calls and indexing can stay in `atom`. A hook that needs
to recurse (parentheses) reaches the table through a pointer in `P`.

### `Parser`

`Parser<P: Hooks<K, E, X>, K, E, X>` is the same table and parser, with
the hooks as methods of the parser state. The calls are direct, so they
inline. `P` declares `impl parse.Hooks<K, E, X> for P;` and defines:

- `fn P.peek`, `advance`, `atom`, `unary`, `binary`;
- optionally `before_rhs` (default: unchanged);
- optionally `led`: one method for all kinds registered with
  `pr.led(k, left, right)`, which dispatches on the kind. The default
  traps.

The hooks' types have no effects, so `parse` has none either and can be
called from any function. Hooks may allocate but not collect: collect
between parses.

### Run-time operator tables: `parse.Ops`

Prolog-family languages define operators while parsing
(`:- op(700, xfx, ===)`), name them by atoms, and give each a priority
(1 to 1200; larger binds looser) and one of seven types. `Ops` is that
table plus an operator-precedence parser for it. The Prolog reader
(`examples/prolog/reader.lm`) uses it; `tests/run/std_parse_ops.lm` is
a small Prolog-term reader.

**The table.**

- `Ops.new()`, `free()`.
- `add(name, prio, typ)`: `name: u64` is an interned atom id below 2^62,
  and `typ: parse.OpType` is one of `XFX`, `XFY`, `YFX`, `FY`, `FX`,
  `XF`, `YF` (`parse.op_type("xfx")` and `t.name()` convert). It
  replaces the definition of the same kind; priority 0 removes it, as
  `op/3` does.
- A name has at most one prefix, one infix and one postfix definition,
  independently (`-` is both `fy 200` and `yfx 500`).

**Lookups.**

- `prefix(name)`, `infix(name)`, `postfix(name)` return
  `?parse.Op { prio, typ, left, right }`. `left` and `right` are the
  highest priorities the arguments may have: `p - 1` on an `x` side,
  `p` on a `y` side.
- `is_op(name)`, `max_prio(name)`.
- `next(&at)` iterates, for `current_op/3`.

**The parser.** Your parser state implements `parse.OpHooks<E, X>`
(`impl parse.OpHooks<Term, Err> for Reader;`). Then:

- `p.op_parse(&ops, max)` parses a term of priority at most `max` and
  returns `(E, priority)`;
- `p.op_parse_arg(&ops)` parses an argument.

The hooks:

| Hook | Does |
|---|---|
| `peek(p, prefix) -> parse.Tok` | Classifies the current token |
| `advance` | Moves to the next token |
| `primary` | Parses any term that is not an operator application (numbers, variables, strings, compound terms, lists, brackets), recursing with `op_parse` |
| `atom(name)` | Builds an operator name used as an atom |
| `prefix`, `infix`, `postfix` | Build operator applications |

`Tok` is one of:

- `Name(u64)`: a name, which may be an operator;
- `Sep(u64)`: `,` or `|`, infix operators that also separate arguments;
- `Term`: starts a term that is not an operator application;
- `End`.

**What the parser handles for you.** Prolog's operator grammar is hard
to get right. The parser implements:

- Argument priorities by operator type.
- A prefix operator is an atom when the next token cannot start a term,
  or is an infix operator that is not also prefix (`- = x`, `f(-)`,
  `[-|T]`, `- , a`).
- An operator used as an atom has its own priority (0 above `max`, so
  `f(:-)` parses).
- A prefix operator above `max` is read at 999 (`X = \+a`).
- `op_parse_arg` uses priority 999 but accepts infix operators above 999
  other than the `Sep`s (`f(a :- b)`), as SWI-Prolog does. Use
  `op_parse(&ops, 999)` for ISO's strict rule.

**What stays in `peek`.** `peek` is told whether a term starts at the
token (`prefix`). There, a name followed directly by `(` (functional
notation, `-(1)`) and a sign followed directly by a number (`-1`, a
negative literal; `- 1` is `-(1)`) are `Tok.Term`. After a term, the
same tokens are infix operators (`X-1`, `a -(b)`).

`Ops.parse<P, E, X>(p, max)` and `parse_arg<P, E, X>(p)` are the same
parser called with explicit type arguments. The `OpHooks` methods exist
because `E` and `X` are not inferred from `P`'s `impl`.

## `std.diag`

Source locations and error messages. Messages render in the same format
as the Loimi compiler's own errors.

**`Span { file, lo, hi }`**: `diag.span(file, lo, hi)`, `a.to(b)`,
`len`.

**`SourceMap`** holds file names and text (borrowed, not copied):

- `new`, `free`, `add(name, text) -> u32`.
- `name(id)`, `text(id)`, `snippet(span)`, `line_count(id)`.
- `line_col(id, pos) -> LineCol`: a binary search in a table of line
  starts, built on first use.
- `line_text(id, line)`, `line_span(id, line)` (for errors that only
  know a line).
- `write_location(&buf, span)`, `render(&d, &buf)`, `eprint(&d)`.

**`Diag`** is one message:

- `Diag.new(severity, ?span, msg)`, `diag.error(span, msg)`,
  `diag.warning(span, msg)`.
- Add to it with `with_label`, `with_note`, `with_note_at`, `with_help`.
- `Severity` is `Error`, `Warning`, `Note` or `Help`.
- Rendering shows a snippet with carets. Tabs are preserved under the
  caret line, spans are clipped to their first line, and lines over 160
  bytes are cut to a window around the span.

**`Diagnostics`** collects messages. It copies each one into an arena,
so you can build them in a temporary `StrBuf`.

- `push(d)`, `error(span, msg)`, `warning(span, msg)`.
- `len`, `at(i)`, `error_count`, `warning_count`, `has_errors`.
- `summary(&buf)` (`1 warning and 2 errors generated.`),
  `render(&sm, &buf)`, `eprint(&sm)`.
- `clear`, `free`.

## `std.debug`: guest debuggers

The generic parts of a source-level debugger for a vm with `on break`.
The interpreter supplies frames, locals and the line of each
instruction; this module handles breakpoints, stepping and commands.
Mini's and the Python subset's `--cfg debug` builds
(`examples/mini/debugger.lm`, `examples/python/debugger.lm`) are built
on it. The design is in [`design/debugging.md`](design/debugging.md).

### Breakpoints

`Breakpoints<B>` manages breakpoints over the vm's `Vm.Break`.

- `Breakpoints.new(Vm.clear_break, rearm)` takes the vm's functions.
- `add(file, line, temp) -> id`: ids start at 1.
- `add_site(id, at, tok)`: records a word that `Vm.set_break` patched.
- `bp(id)`, `list()`, `has_site(at)`.
- `delete(id)` puts the original instructions back; `clear_all`, `free`.

A `Bp` has `id`, `file`, `line`, `nsites`, `hits` and `temp` (deleted at
its first hit).

**Stepping over a breakpoint.**

- `hit(at) -> id` is called from the vm's `on break` handler at `pc`. It
  puts the instruction back and remembers it.
- `tick() -> bool` is called from the wrapper of a variant that the
  handler switches to before running the instruction (`jump pc`). It
  sets the breakpoint again once execution has left it, and says
  whether the variant must stay on.

### Stepping

`Stepper` decides when a step command stops. `Mode` is `Run`, `Step`,
`Next` or `Finish`.

- `start(mode, line, depth)`, `active()`.
- `should_stop(line, depth)`:
  - step: stops at another line or another depth;
  - next: another line at this depth, or a return;
  - finish: a return.
- `left(depth)`: the frame at `depth` returned, yielded or raised to the
  native function that started its nested run. The next instruction may
  then be a new frame at the same depth (a sort key called again):
  `step` stops there, while `next` and `finish` wait for the caller.

The interpreter supplies each instruction's line and call depth. Line 0
means "no line", for instructions such as a loop's jump back, where no
command stops.

### Finding where lines start

- `line_sites(words, lines, line, Vm.base_size, Vm.label, &out)` gives
  the words in one code object where a line starts. That is the first
  word of each run of the line, or the run's loop head (the target of a
  backward jump from another line), which runs on every iteration.
- `unpatched(&bps, words, word, &out)` copies `words` with each
  breakpoint word replaced by the word it stands for (`word` is
  `Break.word`). Use it for walks that read labels or operands, which
  would otherwise see the breakpoint op.

### For embedded debuggers

These support front ends that embed the debugger, such as an editor
talking the Debug Adapter Protocol through a C API (Mini's
`mini_debug_*`):

- **Conditions.** `Bp.cond` is an expression the interpreter evaluates
  at each hit, and `Bp.hitc` a hit condition. Set both with
  `set_condition(id, cond, hitc)`. `counts(id) -> bool` counts a hit
  whose condition held and checks the hit condition. `HitCond` comes
  from `parse_hit_condition(..)`, which accepts `"N"`, `"== N"`,
  `">= N"`, `"> N"`, `"<= N"`, `"< N"`, `"% N"` or `""`.
- **Running guest code from a stop.** `suspend()`, `unsuspend()` and
  `suspended()` take the breakpoints out of the code while the debugger
  evaluates a condition or expression. A pending step-over is left
  alone, and `tick` does nothing meanwhile.
- **Stop reasons.** `Reason` is `Entry`, `Breakpoint`, `Step`, `Pause`
  or `Error`.
- **Handles.** `Refs<T>` hands out ids (from 1) for values a client
  expands lazily: `add(x, same)`, `get(id)`, `items()`, `clear()`.
- **Pausing.** `request_pause(&flag)` can be called from any thread (a
  relaxed atomic store). The interpreter polls with `take_pause(&flag)`
  at back-edges and calls.

### Commands and input

`parse_command(line) -> Command` parses a gdb-style command line, with
gdb's short forms. The commands:

- `Break(file, line)`, `TBreak`, `BreakIf(file, line, cond)` (for
  `break LINE if COND`), `Delete(id)`;
- `Continue`, `Step`, `Next`, `Finish`;
- `Backtrace`, `Locals`, `Print(name)`, `Op`, `Info`;
- `Quit`, `Help`, `Empty`, `Bad(msg)`.

Also `HELP` (the help text), `parse_location("[FILE:]LINE")` and
`basename(path)`.

`Input.stdin()` reads commands:

- `read_line() -> ?str` uses unbuffered `read(2)`, so a script of
  commands can follow the program's own input.
- `is_tty()`: a scripted session echoes its commands.
- `free`.

## `std.bc`: bytecode construction

A library over the encoders the compiler generates for each vm.
Everything is generic over the vm's code word `W`, through two functions
you pass in:

- `size: fn(W) -> u64`: the generated `Vm.op_size`.
- `patch: fn(W, i64) -> ?W`: sets a jump word's offset (`target - pc`),
  or returns `None` if it does not fit. Use the generated
  `Vm.set_label`, or a few lines over `Vm.opcode` and `bc.Field` for a
  custom encoding.

`tests/run/std_bc.lm` compiles a small language to a toy register vm
using all of it.

Nothing in `std.bc` collects, so GC references in a `Pool` are not
roots. Compile between collections, as Mini and Scheme do.

### `Code<W>`

- `Code.new(Vm.op_size, Vm.set_label)`, `clear`, `free`.
- `op(w)` emits an instruction and zeroes its inline-cache words;
  `word(w)` emits a raw word.
- `pc`, `len`, `at`, `set`, `words`, `copy_into`.

**Source positions.** `set_loc(loc)` tags the instructions that follow,
and returns the previous position. Positions are stored as runs:

- `loc_at(pc)` reads one;
- `locs_into(out)` expands them per word;
- `encode_locs(&bytes)` encodes them compactly (about 2 bytes per line
  change), and `bc.line_at(bytes, pc)` reads that back on error paths.

### Labels and jumps

- `label()`, `bind(l)`, `label_pc`.
- `here()`: a label placed at the current pc, for a loop head.
- `jump(w, l)`: emit a jump to a label.
- `fixup(at, l)`: for a jump word that is not the first word emitted.
- `patch(at, target)` and `jump_to(w, target)` for single jumps.
- Jump lists, for Lua-style compilers that pass pending exits around:
  plain `mem.Vec<u64>` of jump pcs, with `patch_list(&l, target)` and
  `patch_here(&l)`.

**Errors are sticky.** An offset that does not fit records `JumpRange`
with the pc and its source position. `check() -> ?Error` returns the
first error, which may also be `Unbound` (a label with pending jumps
that was never placed). `Error.message()` describes it. `fail(kind, pc)`
records one yourself. Compilers that stop at the first error can check
`patch`'s bool result instead.

### Optimizing

- **Peephole.** `last()` is the pc of the previous instruction if it may
  be rewritten or merged with the next one. It is blocked when a jump
  targets the current pc (binding a label or patching to here), as with
  Lua's `lasttarget`. `truncate(pc)` removes code, dropping pending
  fixups in it. `mark_target()`.
- **Jump threading.** `thread_jumps(target_of, is_goto)` retargets jumps
  that land on an unconditional jump, following chains up to 8 deep.
  Nothing moves.
- **Branch relaxation.** For conditional jumps whose short offset field
  may be too small:
  1. Turn it on with `set_relax(Vm.label, invert, Vm.JMP(0))`. A
     conditional jump that does not fit is then recorded instead of
     failing. `needs_relax()` tells whether any is waiting, and
     `check()` reports them as `JumpRange` until step 2.
  2. After binding the last label, call `relax()`. It rewrites each
     waiting jump into `invert(w)` (the opposite condition, jumping over
     the next instruction) followed by a long unconditional jump to the
     target.

  Code after a relaxed jump moves down. Jump offsets, the position
  table and placed labels are updated (and a jump that stops fitting is
  relaxed too), but pcs you saved earlier are stale. `invert` returns
  `None` for words without a long form, which stay errors. Lox
  (`examples/lox/compile.lm`) uses it.

### Fields, constants and registers

- **`bc.Field { shift, bits, signed }`**: range-checked access to an
  encoded field, with `fits`, `get`/`set` and `get32`/`set32`.
- **`Pool<K: hash.Hash>`**, a constant pool:
  - `Pool.new(limit)`, `clear`, `free`.
  - `add(k) -> ?u32`: deduplicated through a `HashMap`; `None` when
    full. `push` adds without deduplicating.
  - `find`, `get`, `len`, `items`.
  - For a nanboxed `Value`, define `fn V.hash` over the bits and
    `fn V.eq(a, b) { a == b }`, so that `0.0`, `-0.0` and `0` stay
    apart.
- **`Regs`**, a register allocator:
  - `Regs.new(limit)`.
  - `alloc(n)`: past the limit it sets `failed()` and still returns, so
    the error can be reported later. `try_alloc(n) -> ?u32` returns
    `None` instead.
  - `free(r)` frees the top register only (locals are ignored; false if
    out of order); `free2(a, b)`; `is_temp`.
  - `mark`/`reset` save and restore the first free register;
    `end_stmt`.
  - `activate(n)` turns registers into locals; `enter`/`leave` for
    blocks; `frame_size(min)`.

## `std.gc`

The garbage collector. This section lists the API;
[`stdlib/gc.md`](stdlib/gc.md) explains how to use it, what it
guarantees and what it costs. The language side (`object`, `Gc<T>`,
`new`, tracing, effects, write barriers) is in
[`language.md`](language.md).

In short: a non-moving mark collector with an optional generational
mode. Allocation never collects; collection happens only where you call
it.

| Task | API |
|---|---|
| Create and free a heap | `gc.Heap.new()`, `heap.free()` |
| Allocate | `new(&heap, Obj { .. })`, or with a trailing-array length. `gc.alloc` is the low-level call under it. |
| Allocate in a handler without a refill call | `try_new(h, T { .. }) -> ?Gc<T>`, with `gc.make_room(h, size)` in a cold block |
| Allocate without zeroing | `new_uninit(h, T { .. }, n)` |
| Collect | `gc.collect(&heap)`, or `gc.maybe_collect(&heap)` (collects if `heap.allocated > heap.threshold`, or in stress mode). Both `uses gc`. |
| Mark roots the compiler can't see | `heap.add_scanner(data, scan)` |
| Drop references to dead objects | `heap.add_sweeper(data, sweep)`, with `h.is_marked(p)` and `heap.is_minor()` |
| Tune | `set_growth(percent, min_heap)`, `set_max_budget(bytes)`, `set_generational(on)`, `set_nursery(bytes)` |
| Debug | `set_stress(on)`, `set_verify(on)`, `LOIMI_GC_STATS=1`, `loimi build --gc-check-roots` |
| Weak references | `heap.new_weak(x) -> Gc<gc.Weak<T>>`, `w.get() -> ?Gc<T>`, `w.set(..)` |
| Ephemeron tables | `gc.Ephemerons.new(&heap) -> *Ephemerons<K, V>`, `get`, `set`, `remove`, `len`, `free` |
| Weak string intern tables | `gc.WeakInterner.new(&heap, make)`, `intern`, `find`, `key`, `is_pinned`, `len`, `free` |
| Write barriers for raw-pointer stores | `gc.write_barrier(&heap, obj)`, `gc.needs_barrier(obj)`, `gc.remember(obj)` |
| Custom tracing | `#[trace(f)]` on an `object` |
| Release host memory when an object dies | `heap.on_free(obj, f)`, `heap.on_free_gc(obj, f)` |
| Objects written without barriers | `heap.pin_dirty(obj)`, `unpin_dirty`, `is_pinned_dirty` |
| Threads sharing a heap | `std.gil`, `gc.mark_shadow_stack(h, top)` |

**Roots.** The compiler roots traced locals at `uses gc` calls, and in
vm code copies of traced `state` variables. Everything else (globals,
VM stacks in memory, host tables) needs a root scanner:
`heap.add_scanner(data, scan)`, where `data: *T` is any pointer and
`scan: fn(*Heap, *T)` calls `gc_mark(h, v)` on each reference. Scanners
have no effects, so they cannot collect. A context struct declared
`#[trace]` gets a generated `ctx.trace(h)` that marks its fields.

**Memory cost.** Every object has a 16-byte header, so a two-word object
such as a Lisp pair takes 32 bytes.

**Limitations.** The heap never moves objects. There is no card marking
and no incremental marking. Small objects are promoted at their first
survival. Finalizers cannot run guest code. See
[Limitations](stdlib/gc.md#limitations) for details.

## `std.obj`: objects and shapes

Objects with symbol keys, as in JavaScript objects and Python instances,
with the machinery a fast property access needs. This section is a
summary; [`stdlib/obj.md`](stdlib/obj.md) is the full guide.

What it provides:

- **Shapes** (hidden classes) that stay valid across collections. A
  shape's id names the sequence of properties added, so a cached id is
  never wrong, and caches need no reset after a collection.
- **Slots**: inline in the object, then in an overflow array.
- **Dictionary mode** for objects with many or deleted properties.
- **Attribute bits** per property, with a meaning you choose.
- **Prototype chains**, with validity cells so that a prototype hit is
  checked with a few plain words.
- **Inline caches**: `obj.Site` and `read_miss`/`write_miss` run the
  state machine (refill, polymorphic, megamorphic, cached property
  adds). `poly_get_at`, `mega_get_at` and `read_miss_at` also say
  whether a value came from the receiver or a prototype, for method
  binding.
- **Allocation templates**, which size an allocation site's objects.

To use it, either take the ready-made generic `obj.Obj<V>` and
`obj.Slots<V>`, or declare your own object type (with fields `shape`,
`dict`, `overflow`, optionally `proto`, and a trailing `inline: [Value]`)
and slot-array type. Either way, create an `obj.Model<O, S, V>` with a
function that allocates slot arrays.

Keys are `u64`: symbol ids, or the addresses of pinned strings
(`gc.WeakInterner.key`). See
[Keys for guest strings](stdlib/obj.md#keys-for-guest-strings).

An inline-cache hit costs two loads, a compare and the slot load
(`bench/obj`).

Limitations: there is no integer-indexed element storage and no global
property cells; both are left to the interpreter.

## `std.rt`: runtime support

### Unwinding: `protect` and `throw`

`rt.protect(f: fn(*u8), data: *u8) -> bool` and `rt.throw() -> never`
are a setjmp/longjmp pair. They let a library return an error from deep
inside code that has no error path, instead of exiting the host process.
Mini's recursive-descent parser uses them (`compile.try_compile`).

- `throw` jumps to the innermost `protect` on the thread, and panics if
  there is none.
- It restores the shadow-stack top, so no GC roots are lost. `f`'s type
  has no effects, so it cannot collect.
- The frames in between are abandoned: their `defer`s do not run, and
  what they allocated with `mem.alloc` leaks.

### Panics

- `rt.catch_panic(f, data) -> bool` runs `f(data)` and returns false if
  it panicked.
- `rt.panic_message() -> str` is the message of the last panic caught on
  the thread, such as `"lib.lm:12:5: index out of bounds"`.
- `rt.panic(msg)` panics with your own message. The catch keeps a copy
  of its first 512 bytes, so the message may live in a buffer that the
  unwinding abandons.

A `throw` passes through a `catch_panic` to the enclosing `protect`,
and a panic passes through a `protect`. On an `export "C"` function, the
attribute `#[catch_panic(v)]` catches panics at the C boundary; see
[`language.md`](language.md) for its guarantees.

### VM stacks

A VM stack is memory you allocate yourself (`mem.alloc<Value>(n)`) and
keep in state registers. Frames are your own structs. The GC sees the
stack through a root scanner. The example interpreters
(`examples/mini`, `examples/stack`, and the tutorial from step 7) show
the pattern.

### Value stacks: `rt.ValueStack<T>`

A register vm's root scanner marks every register of every frame's
window. So a register the callee has not written yet must not hold a
pointer to a freed object. Clearing the window on every call ensures
that, but costs time (13% of Lox's fib). `rt.ValueStack<T>` avoids the
clearing with a *high-water mark*, `s.hw`, which splits the stack into
three regions:

| Region | Slots hold |
|---|---|
| Below `s.hw` | Values the scanner may read: live registers, or stale ones whose objects no collection has freed |
| `s.hw` up to `s.fresh` | Cleared values that refer to no freed object |
| From `s.fresh` up | Never written |

The operations:

| Operation | Meaning |
|---|---|
| `rt.ValueStack.new(n, fill)` | Allocate `n` slots. `fill` is what cleared slots hold: a value the collector ignores, such as nil. The memory is zeroed by `mem.alloc`, so untouched pages cost nothing. The stack never moves. |
| `s.fits(top) -> bool` | One compare, `top <= s.hw`. A call checks this instead of comparing against the stack's end. |
| `s.raise(top) -> bool` | `#[cold]`. False past the end. Otherwise clears the never-written slots below `top` (plus `rt.SLACK`, 256, more) and raises the mark to `s.fresh`. After a collection has lowered the mark, raising it again clears nothing. |
| `s.ensure(top) -> bool` | `fits`, else `raise`. For slow paths. |
| `s.lower(top)` | For the root scanner, after it has marked the live windows; `top` is their end. Clears the stale slots from there up to the mark (this collection may free what they point to) and lowers the mark. |
| `s.free()` | Release the stack |

In a handler's hot path, don't call `raise` directly: a call there gives
the handler a stack frame. Call it from a `cold` block that ends in
`redo`, as below.

What the interpreter must guarantee:

- It writes every register before reading it. A compiler that allocates
  registers to locals and temporaries does this.
- The scanner's `top` covers every live window. A callee's window can
  end below its caller's, so either take the highest end over all frames
  (Lox, Scheme), or have the compiler place a call's window above all of
  the caller's live registers (MiniJS).
- A value written above the mark (arguments set up before the call
  raises it) keeps its value through `raise`, but the mark must cover it
  before the next collection.
- A vm with coroutines needs one stack per coroutine, and the scanner
  must lower the mark of each. Nested runs must `ensure` their windows.

<!-- snippet: skip a fragment of examples/scheme/vm.lm and rt.lm -->
```loimi
pub struct Ctx { vs: ValueStack<V>, frames: *Frame, frames_end: *Frame, fp: *Frame, .. }

op CALLSELF(base: callbase, nargs: imm) : ABC {           // in the vm
    let callee = regs + base + 1;
    let n = fp.cl.proto.nregs as u64;
    if !ctx.vs.fits(callee + n + 1) or fp + 1 >= ctx.frames_end {
        cold {
            if fp + 1 >= ctx.frames_end or !ctx.vs.raise(callee + n + 1) { raise overflow(ctx) }
            redo
        }
    }
    ..                                                    // no clearing
}

pub fn scan_roots(h: *gc.Heap, c: *Ctx) {
    var top = c.vs.base;
    var f = c.frames;
    while f <= c.fp {
        let n = f.cl.proto.nregs as u64;
        for i in 0..n { gc_mark(h, f.base[i]); }
        if f.base + n > top { top = f.base + n; }
        f = f + 1;
    }
    c.vs.lower(top);
}
```

Lox, Scheme and MiniJS use it.

## `std.prof`

Timing and counting inside an interpreter or its benchmark driver.
Nothing in it runs unless called, and the `--vm-stats` access is
compiled in only when the program uses it.

### Clock

- `prof.now_ns() -> u64` (`CLOCK_MONOTONIC`).
- A stopwatch: `prof.Timer.start()`, then `t.ns()`, `t.ms() -> f64`,
  `t.restart()`.
- `prof.fmt_ns(&buf, ns)` appends a readable duration: "812 ns",
  "3.25 us", "41.7 ms".

### Benchmarks

1. `var b = prof.Bench.new(warmup, runs)`.
2. `b.add(name, f, data)` for each variant. `f` is a
   `fn(*u8) uses reenter`, so a variant may run an interpreter that
   collects; `run` then needs the effect too.
3. `b.run()` runs `warmup` untimed rounds, then `runs` timed rounds.
   Each round times every variant once, starting from a different one
   each round, so drift in the machine's speed hits all variants alike.

`run` prints the name, minimum, median, and minimum relative to the
first variant, on stderr. `b.report(&buf)` writes the same table to a
buffer; `b.min_ns(i)` and `b.median_ns(i)` give the numbers. `b.free()`.

### Counters

For events `--vm-stats` does not see, such as cache misses or slow paths
outside `cold` blocks (`--vm-stats` counts `cold` blocks itself):

- `prof.Counters.new()`, `c.free()`.
- `c.add(name) -> id`.
- `c.inc(id)`, `c.bump(id, n)` (inlined), `c.get(id)`.
- `c.report()` prints to stderr.

### Vm stats

In a `loimi build --vm-stats` build, `prof.op_counts("Vm") ->
?prof.OpCounts` gives the vm's counters. It is `None` without the flag
or for an unknown vm; `prof.vm_stats_enabled()` tells which.

- `c.len()`: the ops in declaration order, superinstructions included.
- `c.name(i)`, `c.count(i)`, `c.count_of("ADD")`, `c.total()`.
- `c.reset()` zeroes op counts, pairs and sequences: use it to measure
  one phase, or to skip a warmup. The report at exit then covers only
  what ran since.
- `c.report()` prints the exit report now.

The exit report also counts entries into each `cold` block, per op, with
its source location.

## `std.test`

Unit tests: `#[test]` functions, run by the `loimi test` command.

<!-- snippet: check
{snippet}
fn run_script(path: str) uses gc {}
-->
```loimi
import std.test;

#[track_caller]
fn squares(x: i64, want: i64) { test.eq(x * x, want); }

#[test]
fn arithmetic() {
    test.eq(2 + 2, 4);
    squares(3, 10);
}

#[test]
fn runs_a_program() uses gc { run_script("fib.lua"); }
```

```text
$ loimi test calc_test.lm            # or: .. arith  (names containing "arith")
test arithmetic ... FAILED
    calc_test.lm:9:5: test.eq failed
        got:  9
        want: 10
test runs_a_program ... ok

failures:
    arithmetic

test result: FAILED. 1 passed; 1 failed; 0 filtered out
```

### Running tests

`loimi test FILE.lm` replaces FILE.lm's `main` and runs each `#[test]`
function in source order, in FILE.lm's directory.

Arguments:

- filter words: a test runs if its name contains one of them;
- `--list`, `--update`;
- the usual build options (`--release`, `--dispatch=`,
  `--sanitize=address`, `-W error`, ...).

A test fails on any panic: an assertion, a bounds check, `unwrap` of
`None`. The frames the panic abandons leak, and the run goes on with the
next test. A crash that is not a panic (a segfault in unchecked code)
ends the run.

The generated `main` is plain Loimi over `test.Runner`: `Runner.new()`
reads the arguments, then `r.run(name, f)` and `r.finish() -> i32`. A
hand-written `main` can use it too.

### Assertions

Assertions report the location of their caller (`#[track_caller]`). A
helper of your own marked `#[track_caller]` passes the test's line
through, as `squares` does above.

| Assertion | Fails unless |
|---|---|
| `test.assert(c)`, `test.assert_msg(c, msg)` | `c` is true |
| `test.eq(got, want)`, `test.ne(a, b)` | equal / not equal |
| `test.some(opt) -> T`, `test.none(opt)` | `Some` / `None` |
| `test.ok(result) -> T`, `test.err(result) -> E` | `Ok` / `Err` |
| `test.has(text, part)` | `text` contains `part` |
| `test.panics(f, data) -> str` | `f` panics; returns the message |
| `test.fail(msg)` | always fails |

`test.eq` and `test.ne` work on types with `fn T.eq` and
`impl test.Eq for T;`, plus `text.Fmt` to print them. `str`, `bool`,
`f64` and the integers are built in. `test.caller()` gives the caller's
location inside a `#[track_caller]` function.

### Output and golden files

- `test.capture_start()` sends stdout and stderr (descriptors 1 and 2)
  to a temporary file, until `test.capture_end(&buf)` appends what was
  written. A test that fails while capturing shows the captured output.
- `test.golden(text, path)` compares `text` with the file at `path` and
  names the first line that differs. `loimi test --update` (or
  `LOIMI_TEST_UPDATE=1`) writes the file instead.

Conformance tests that run a built interpreter on script files stay
shell scripts (`examples/*/check.sh`). Bytecode fuzzers are ordinary
Loimi programs (see `tests/fuzz/`).
