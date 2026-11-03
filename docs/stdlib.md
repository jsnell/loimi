# Standard library

The language handles the inner loop. The standard library handles the rest of
an interpreter: getting from source text to verified bytecode, and providing
the runtime structures every dynamic language needs.

Each module section below starts with **what exists** (the API in
`std/*.lm`, which you can import today), followed by a **Design (not
built)** part describing what the module was planned to grow into. Do not
plan around the design parts: nothing in them exists unless the built part
says so. Some modules are design only; their sections say so at the top.

| Module | Status | Source |
|---|---|---|
| [`std.core`](#stdcore-built-in-types-and-operations) (built-ins) | built in part (no `assert`, `rotl`, ...; formatting is in `std.text`) | the compiler |
| [`std.mem`](#stdmem) | built | `std/mem.lm` |
| [`std.hash`, `std.map`](#stdhash-and-stdmap), [`std.dict`](#ordered-maps-maporderedmap-and-dictdict) | built (`HashMap`, insertion-ordered `OrderedMap`; `Dict`, the same in the GC heap; `KeyDict`, without stored hashes, for keys that cache theirs) | `std/hash.lm`, `std/map.lm`, `std/dict.lm` |
| [`std.text`](#stdtext) | built (`StrBuf` with `{}` formatting, `Interner`, small helpers) | `std/text.lm` |
| [`std.num`](#stdnum) | built | `std/num.lm` |
| [`std.bigint`](#stdbigint) | built (no subquadratic division) | `std/bigint.lm` |
| [`std.io`, `std.os`](#stdio-and-stdos) | built (minimal) | `std/io.lm`, `std/os.lm` |
| [`std.lex`](#stdlex) | built (a cursor toolkit, no generator) | `std/lex.lm` |
| [`std.parse`](#stdparse) | built (`Pratt`, `Parser`; no `TokenStream`) | `std/parse.lm` |
| [`std.diag`](#stddiag) | built | `std/diag.lm` |
| [`std.bc`](#stdbc-bytecode-construction) | built (a library, no generated `Builder`) | `std/bc.lm` |
| [`std.gc`](#stdgc), [details](stdlib/gc.md) | built (mark-sweep, optional generational mode) | `std/gc.lm` |
| [`std.gil`](stdlib/gc.md#threads-sharing-a-heap-stdgil) | built (a global interpreter lock per heap) | `std/gil.lm` |
| [`std.obj`](#stdobj-objects-and-shapes), [details](stdlib/obj.md) | built (no comptime `Model`) | `std/obj.lm` |
| [`std.rt`](#stdrt-runtime-support) | built in part (`protect`/`throw`, `catch_panic`/`panic`; no `ValueStack`, `FrameStack`, `Fiber`) | `std/rt.lm` |
| [`std.collections`](#design-not-built-stdcollections) (beyond `HashMap`, `OrderedMap`, `Dict`) | **not built** | |
| [`std.ast`](#stdast) | **not built** | |
| [`std.prof`](#stdprof) | built (clock, `Bench`, `Counters`, `--vm-stats` counts; no sampling profiler) | `std/prof.lm` |
| [`std.ffi`](#design-not-built-stdio-stdos-stdffi) | **not built** (`extern "C"` is in the language) | |
| [`std.test`](#stdtest) | built (`#[test]`, `loimi test`, assertions, capture, golden files; no matrix or differential runner) | `std/test.lm` |

Design rules for the library:

- **Generated per VM where it helps.** Encoders, disassemblers, and verifiers
  come from `vm` declarations (`language.md` §3.3). The library supplies the
  generic machinery they plug into.
- **Arena-first.** Front-end data (tokens, AST, IR) lives in arenas and is
  freed in one go.
- **Fast paths inline, slow paths `cold`, effects declared.** The library
  follows the same discipline it asks of users.
- **Everything that owns memory can give it back.** Each std type that
  holds heap memory has a `free()` method (`Vec`, `Arena`, `HashMap`,
  `OrderedMap`, `StrBuf`, `Interner`, `OpTable`, `Keywords`, `Pratt`, `Parser`,
  `SourceMap`, `Diag`, `Diagnostics`, `Code`, `Pool`, `Heap`, `Shapes`,
  `MegaCache`, `Ics`, `Model`, `Ephemerons`, `WeakInterner`, `Gil`,
  `BigInt`, `Bench`, `Counters`; `Malloc` frees each allocation with
  `free_bytes`), and `io.read_file`'s result
  is released with `io.free_file`. Freeing a container does not free what
  its elements point to.

---

## `std.core`: built-in types and operations

There is no `std/core.lm`: these are built into the compiler and need no
import.

- **Types**: `i8`..`i64`, `u8`..`u64`, `isize`, `usize`, any-width
  `iN`/`uN` (1..=64), `f32`, `f64`, `bool`, `str` (a `[]u8` slice), slices
  `[]T` (`.len`, `.ptr`, `s[a..b]`), arrays `[T; N]`, pointers `*T`,
  `?T`, `Result<T, E>`, tuples (language.md §1.1).
- **Integer arithmetic**: `+ - *` trap on overflow in debug builds and
  wrap in release builds; `+% -% *%` always wrap;
  `a.add_checked(b)`, `a.sub_checked(b)`, `a.mul_checked(b)` return `?T`.
  The `(T, bool)` forms are functions in `std.num` (`num.add_overflow(a,
  b)` etc., `i64` only).
- **Bit operations**: `x.clz()`, `x.ctz()`, `x.popcount()` on any
  integer type; shifts `<< >>` (language.md §1.2).
- **Wide arithmetic** on `u64` (for multi-word integers, `std.bigint`):
  `a.mul_wide(b) -> (u64, u64)` is the full 128-bit product as (low,
  high); `hi.div_wide(lo, d) -> (u64, u64)` divides the 128-bit number
  `hi * 2^64 + lo` by `d` and gives (quotient, remainder). The caller
  guarantees `hi < d` (so the quotient fits; `d` is then nonzero), as for
  the x86-64 `div` instruction. They lower to LLVM `i128` operations: a
  `mul` instruction, and a call of the runtime's `__udivti3`.
- **Floats** (`f32` and `f64` alike; each lowers to one LLVM
  intrinsic): `x.to_bits()` (`u32`/`u64`) and `f32.from_bits(u32)`,
  `f64.from_bits(u64)`; `x.sqrt()`, `x.abs()`; rounding to an integral
  value `x.floor()`, `x.ceil()`, `x.trunc()` and `x.nearest()` (to
  nearest, ties to even: 2.5 rounds to 2.0 and -0.5 to -0.0; wasm
  `nearest`, C `roundeven`); `x.copysign(y)` (`x`'s
  magnitude, `y`'s sign bit); `x.is_nan()`. Two kinds of min/max:
  - `x.min(y)`, `x.max(y)`: C `fmin`/`fmax` (IEEE 754-2008 minNum): if
    one operand is NaN the other is returned; NaN only if both are
    (what a signaling NaN gives is unspecified). The result for
    `min(-0.0, 0.0)` may be either zero.
  - `x.minimum(y)`, `x.maximum(y)`: IEEE 754-2019 `minimum`/`maximum`,
    what wasm's `f32.min`/`f64.max` and JavaScript's `Math.min` specify:
    a NaN operand gives a quiet NaN (a signaling one is quieted, as
    wasm's "arithmetic NaN" requires), and `-0.0` is less than `0.0`.
    Use these to implement a guest's IEEE semantics; `min`/`max` are
    the cheaper choice when NaN cannot occur.

  `sqrt`, `abs`, `copysign` and both min/max kinds compile to SSE2
  instructions on x86-64 (`sqrtss`/`sqrtsd`, masks, `minsd` with a NaN
  fix-up; `minimum` adds a sign test). The rounding functions need
  SSE4.1's `roundss`/`roundsd`, which the default x86-64 target lacks:
  there they are calls to libm's `floor`, `roundeven`.. (AArch64 has
  `frintm`/`frintn`.. in the base ISA). `as` converts between integers
  and floats (and `f32 as f64` and back). For floor division and the
  rest, see `std.num`.
- **Options**: `o.is_some()`, `o.is_none()`, `o.unwrap()` (traps on
  None), `o.unwrap_or(d)`, `o ?? d`, and `?` (also on `Result`).
  `o ?? p` with `p: ?T` is `?T`: `o` if it is `Some`, else `p`, which is
  evaluated only then (Rust's `o.or_else(|| p)`; chains:
  `a ?? b ?? c`).
  `Result` has no methods: use `match`, `if let` or `?`.
- **Pointers**: `p.offset(n)` (`i64` elements), `p.slice(n) -> []T`,
  `p + n`, `p[i]`, `*p`; `p.read_unaligned()` and `p.write_unaligned(v)`
  for addresses that may not be aligned for `T` (language.md §1.3).
- **Other built-ins**: `size_of<T>() -> u64`; `likely(c)`,
  `unlikely(c)`, `assume(c)`; `trap() -> never` and `unreachable`;
  `new(heap, Obj { .. }, n)` and `gc_mark(heap, v)` with `std.gc`
  imported (§8 of language.md).
- **String equality** is `a.eq(b)`, defined in `std.hash` (`import
  std.hash;`); `==` does not compare slices.

### Design (not built)

Not built: `fmt` as compile-time checked format strings (`text.StrBuf`
has `fmt`/`add` with run-time `{}` placeholders, see `std.text`), `assert` / `debug_assert` (write `if !c { trap(); }`),
`rotl`, `bswap`, `bitcast` (use `to_bits`/`from_bits` and `as`), the
float functions `fma` and `nextafter`, `a.add_overflow(b)` as a method, `f64.to_i32_js()`, and
saturating conversions. The guest-arithmetic helpers that were planned
here (`floor_div`, `floor_mod`, `trunc_div`, `trunc_rem`,
`to_i64_exact`) are built, in `std.num`.

## `std.mem`

`std/mem.lm`:

- `mem.alloc<T>(n) -> *T` (zeroed; out of memory is a panic, which
  aborts unless caught, see `std.rt`), `mem.try_alloc<T>(n) -> ?*T`
  (None when out of memory), `mem.grow<T>(p, n) -> *T`, `mem.release(p)`,
  `mem.copy(dst, src, n)` (overlapping allowed).
- Byte strings (any `[]u8`, so `str` too; libc underneath):
  `mem.find_byte(s, c) -> ?u64` (`memchr`), `rfind_byte` (`memrchr`),
  `find_bytes(s, needle) -> ?u64` (`memmem`), `compare_bytes(a, b) -> i32`
  (-1, 0, 1; a prefix sorts first), `equal_bytes(a, b)`. For a raw
  pointer, `p.slice(n)` first.
- `Vec<T> { ptr, len, cap }`: `Vec.new()`, `with_capacity(n)`,
  `reserve(extra)`, `push`, `pop -> ?T`, `get(i) -> ?T`, `at(i)`,
  `ptr_at(i)`, `set(i, x)`, `last -> ?T`, `items -> []T`,
  `truncate(n)`, `clear`, `free`, `sort_by(cmp)`, `try_sort_by(less)`.
  **An empty `Vec` allocates nothing**: `Vec.new()` (and
  `with_capacity(0)`) has capacity 0 and a static sentinel pointer, and
  the first push allocates. `free()` leaves the `Vec` empty and usable
  (freeing twice is fine). `mem.release` and `mem.grow` accept the
  sentinel, so code that takes over `v.items().ptr` works for empty
  vectors too; do not pass it to C's `free`.
- **Objects with optional resources.** A `*T` field cannot be null and
  most structs need a value for every field, so an object built before
  its parts exist (a compiled regex that may carry an error instead)
  needs placeholders. Make each optional part a `Vec<T>` (an empty one
  is free to create, overwrite and drop) or a `?*T` (None until
  allocated, released with `if let Some(p) = x.p { mem.release(p); }`),
  never a `mem.alloc(1)` dummy, which leaks when overwritten.
- `mem.sort_by(p, n, cmp)`: a stable merge sort; `cmp` is a
  `block fn(*T, *T) -> i32`, a closure capturing locals or a plain
  function.
- `mem.try_sort_by(p, n, less) -> Result<(), E>` (and
  `Vec.try_sort_by(less)`): a stable sort for comparators that can fail
  and run guest code, such as a dynamic language's `<` (which may raise,
  or call a user `__lt__`): `less` is a `block fn(*T, *T) -> Result<bool,
  E> uses reenter` saying whether `a` goes before `b`, and the sort is
  `uses reenter`. The first `Err` stops it and is returned, with the
  elements unchanged. The comparator gets pointers into `p`, which stays
  untouched while it runs (the sort orders indices, then permutes `p`
  once, with no calls), so elements that `p` keeps reachable stay
  reachable when the comparator collects. `p` must stay valid: if guest
  code could change it, sort a copy. Python's `sorted`/`list.sort` use it
  (`examples/python/builtins.lm`).
- `Arena`: `Arena.new()`, `alloc_bytes(n, align) -> *u8`, `str(s) -> str`
  (copy a string into the arena), `free()`. 64 KiB chunks; larger requests
  get their own chunk. `free_bytes(p)` does nothing (arena memory is
  freed all at once), so `c.free(&arena)` on an escaping closure and code
  generic over the allocator work.
- `Malloc`: an allocator on the C heap whose allocations are freed one
  by one: `Malloc.new()`, `alloc_bytes(n, align) -> *u8` (alignment up
  to 16), `free_bytes(p)`, and `live`, the number of allocations not
  freed yet. For escaping closures freed one at a time (`new(&m, |..|
  ..)`, `c.free(&m)`) and cells (`new(&m, v)`, `m.free_bytes(p as
  *u8)`).

### Design (not built)

`Arena.reset`, scoped sub-arenas and typed arena allocation
(`arena.new(T)`, `arena.slice(T, n)`); `Pool<T>` (fixed-size free-list
allocator); `Id<T>` / `IndexVec<Id<T>, T>` typed indices; `SmallVec<T, N>`.

## `std.hash` and `std.map`

`std/hash.lm`, `std/map.lm`:

- `hash.Hash` is a trait: `fn hash(k: Self) -> u64` and
  `fn eq(a: Self, b: Self) -> bool`, implemented for `u64`, `i64`,
  `u32`, `i32` and `str`. A key type defines both methods and declares
  `impl hash.Hash for Key;`. `hash.mix(x)` (a 64-bit mixer) and
  `hash.bytes(s)` are the building blocks.
- `map.HashMap<K: hash.Hash, V>` (linear probing, at most 3/4 full):
  `HashMap.new()`, `insert(k, v) -> bool` (true if the key was new),
  `get(k) -> ?V`, `get_ptr(k) -> ?*V`, `contains`, `remove -> bool`,
  `len` (a field), `clear()` (keeps the table's memory), iteration with
  `var i: u64 = 0; while let Some(s) = m.next(&i) { .. s.key, s.value .. }`,
  and `free`.
- `map.OrderedMap<K: hash.Hash, V>`: a hash map that iterates in
  insertion order, in host memory. `std.dict.Dict` (below) is the same
  table in the GC heap, for guest dicts.

### Ordered maps: `map.OrderedMap` and `dict.Dict`

Both use CPython's compact dict layout: an index table of `u32` slots (a
power of two; empty, deleted, or an entry number) over a dense array of
`map.Entry<K, V> { hash, key, value }` in insertion order, at most 2/3 of
the index size. Removing a key marks its entry dead (its hash becomes
`map.DEAD`, its key and value bytes are zeroed, so a traced table no
longer keeps them alive) and its index slot deleted; the next resize
compacts the entries. Entries keep their hashes (with the top bit
cleared), so resizing and copying never call `hash` or `eq`: an
interpreter whose hashing runs guest code (`__hash__`) calls it once per
key. Probing is CPython's (`i = 5i + 1 + perturb`, `perturb >>= 5`).

- **The usual API**, through `hash.Hash` on K: `OrderedMap.new()`,
  `with_capacity(n)`, `insert(k, v) -> bool` (true if new; a replaced
  key keeps its place), `get -> ?V`, `get_ptr -> ?*V`, `contains`,
  `remove(k) -> ?V`, `pop_last() -> ?(K, V)` (reclaims trailing dead
  entries, so a stack-like use stays compact; their index slots stay
  deleted, so the reclaimed room counts as used until the next resize,
  else popping and inserting in turn would fill the index with deleted
  slots), `clear()` (keeps the
  memory), `len` (a field), iteration in insertion order with
  `var i: u64 = 0; while let Some(e) = m.next(&i) { .. e.key, e.value .. }`,
  and `free`.
- **Hash-supplied forms**, for interpreters (a string's cached hash, an
  identity test before the general equality): `find_by(hash, eq: block
  fn(K) -> bool) -> ?u64` gives an *entry index*; `put_by(hash, k, v,
  eq) -> (u64, bool)` inserts or replaces in one probe; `insert_new(hash,
  k, v)` appends a key the caller knows is absent; `entry(e) -> *Entry`,
  `set_value(e, v)`, `is_live(e)`, `remove_at(e)`. `eq` gets a copy of
  each candidate key whose hash matches.
- **Entry indices and `version`.** An entry index stays valid until the
  table is compacted. `version` changes on every insertion of a new key,
  removal, compaction and `clear`, not on value updates, so an inline
  cache can hold an entry index and check the key at it (Python's
  attribute caches), or hold the version (a cached global).
- **Lookups that can fail and run guest code**:
  `find_with<E>(hash, eq: block fn(K) -> Result<bool, E> uses reenter)
  -> Result<?u64, E> uses reenter`. An `Err` from `eq` stops the lookup.
  The rule for changes during the lookup is CPython's: after each `eq`
  call, if `version` changed (`eq` inserted, removed or resized), the
  probe starts over on the current table, so the answer is about the map
  as it is when `find_with` returns. An `eq` that changes the map on
  every call makes the lookup loop forever, as in CPython. Insertion with
  such an equality is `find_with` and then `set_value` or `insert_new`
  (nothing runs between them).

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

`std/dict.lm`: `dict.Dict<K: hash.Hash, V>` is an `object` whose index
(`DictIndex`) and entries (`DictEntries<K, V>`) are GC objects, so keys
and values may be `Value`s or `Gc<T>`s; a guest dict can be a
`Gc<Dict<Value, Value>>` (examples/python's is). It has the same API
with the dict as a `Gc` receiver and the heap on calls that may allocate
(`Dict.new(h, n)`, `insert(h, k, v)`, `put_by(h, ..)`, `insert_new(h,
..)`, `copy(h)`; `try_insert_new(hash, k, v) -> ?u64` appends without
resizing); like any `new`, they never collect, so they can run in
a handler's fast path. Iteration is by entry index (`next(&i) -> ?u64`,
then `key(e)`, `value(e)`, `hash_at(e)`), which stays usable across
allocation and guest code (compare `version` to detect "changed during
iteration"). `has_room()` and `try_add_by(hash, k, v, eq) -> ?u64`
insert without resizing or allocating (None if the key is present or the
dict is full): Python's attribute-store cache appends a new attribute
with it. `find_with` re-reads the dict's tables after each `eq` call.
Tested by `tests/test/ordered_map.lm` (randomized operations against a
`Vec` model, both maps; `find_with` restarting and failing) and
`tests/run/dict_gc.lm` (traced keys and values in a stress heap and a
generational verify heap, an `eq` that collects and inserts during
`find_with`, `--gc-check-roots`).

Measured in examples/python (which uses `Dict<Value, Value>` for every
dict, instance attributes, globals and class namespaces): the 11
benchmarks are unchanged (0.97x the time of its own dict, interleaved
best of 5). Entries of 24 bytes instead of 16 (the stored hash) make
`P(i, 2)` in a loop about 20% slower (more allocation per instance
dict); dict-heavy code with int and string keys is a few percent faster.
CPython avoids the hash in instance dicts with a str-keys-only entry
layout: `dict.KeyDict` (below).

#### `dict.KeyDict`: keys that carry their hash

`dict.KeyDict<K: dict.CachedHash, V>` is `Dict` without stored hashes:
entries are `dict.KeyEntry<K, V> { key, value }` (16 bytes for `Value`
keys instead of 24), for keys whose hash is cached in the key, such as
interned attribute names (CPython's str-keys dict). `K` provides
`fn K.cached_hash(k) -> u64` (the hash callers pass for that key; it must
not fail or run guest code, computing and caching it on first use is
fine) and `fn K.eq(a, b) -> bool`, and `impl dict.CachedHash for K;`.
Resizing, removal (`remove_at` finds the index slot by the key's hash)
and `copy` ask the key for its hash. A removed entry is zeroed and is
recognized by its all-zero key bytes, so a live key must never be all
zero bits (a string `Value` is a tagged pointer). Everything else is
`Dict`'s: the index and probing, the API (`new`, `find_by`, `find_with`
with its restart rule, `put_by`, `try_add_by`, `try_insert_new`,
`insert_new`, `has_room`, `key`/`value`/`set_value`, `hash_at` (from the
key), `is_live`, `remove_at`, `get`/`contains`/`insert`/`remove`,
`pop_last`, `next`, `clear`, `copy`), entry indices stable until a
compaction, `version`, tracing, no collection on allocation. With no
stored hash to filter by, `find_by`'s `eq` sees every live key on the
probe path: test identity first, then the candidate's cached hash
(`str_eq` in examples/python compares lengths and cached hashes before
bytes). Tested like `Dict`: `tests/test/ordered_map.lm`
(`keydict_differential` against the model in a stress heap with
colliding hashes, `keydict_find_with_and_room`, `pop_insert_alternating`)
and `tests/run/dict_gc.lm` (its traced-key checks on a KeyDict too).

examples/python keeps instance attributes in a `KeyDict<Value, Value>`
(`__dict__` gives a copy). Measured against `Dict` there (2026-11-03,
best of 3-9 interleaved): the 11 benchmarks 0.99 by geomean, each
within noise (raytrace 0.97 in one run, 1.02 in another); creating an
instance with 7 attributes in a loop 0.84-0.90 of the time; `P(i, 2, 3)`
in a loop unchanged (smaller allocations, but `eq` now loads colliding
keys on the probe path where the stored hash used to reject them).

### Design (not built): `std.collections`

Everything here except `HashMap`, `OrderedMap` and `Dict` is design only:

- `HashSet<T>`, SwissTable-style probing.
- `BitSet`, `Deque`, `BinaryHeap`.
- `std.collections.Table`: a **Lua-style hybrid table**. A dense array part
  plus a hash part, with array-part migration. Keys are `Value`s through a
  user-supplied hash/eq interface, so it works with any `value` type.
  (Mini, Scheme and MiniJS each write their own.)
- A per-process random hash seed (to resist hash flooding) and a
  streaming hash API.

## `std.text`

`std/text.lm`:

- `StrBuf`, a growable byte string: `StrBuf.new()`, `push(byte)`,
  `push_str(s)`, `push_int(i64)`, `push_f64(f64)` (shortest `%g` that
  reads back as the same number), `as_str()`
  (valid until the next push), `len()`, `clear()`, `free()`,
  `push_u64(u64)`. `std.num` has more float formats.
- Formatting, for messages: `b.fmt2("expected {} values, got {}", n, m)`
  appends the format string with each `{}` replaced by the next
  argument (`fmt` takes one argument, `fmt2` to `fmt4` two to four;
  `{{` and `}}` are literal braces). `b.add(x)` appends one value. Both
  return the buffer, so calls chain:
  `` b.fmt("unknown word `{}`", name).add(" at line ").add(line).as_str() ``.
  The format string is not checked at compile time: a `{}` without an
  argument stays `{}`, and an argument without a `{}` is appended at the
  end. Arguments are any type that implements the trait `text.Fmt`
  (`fn fmt(x: Self, b: *StrBuf)`): `str`, all the `i*`/`u*` widths up to
  64 bits (`u8` prints as a number; `push` appends a byte), `f64` (as
  `push_f64`) and `bool`. A type of your own joins with
  `fn T.fmt(x: T, b: *text.StrBuf) { .. }` and `impl text.Fmt for T;`.
- `Interner`: `Interner.new()`, `intern(s) -> u32` (ids 0, 1, 2, ...;
  the string is copied into the interner's arena), `find(s) -> ?u32`,
  `name(id) -> str`, `free()`.
- Helpers: `starts_with(s, prefix)`, `find(s, byte) -> ?u64` (`memchr`),
  `is_digit`, `is_alpha` (includes `_`), `is_space`, `parse_int(s) -> ?i64`
  (decimal), `parse_f64(s) -> ?f64` (C syntax, via strtod). `std.num` has
  the exact parsers with error kinds.

### Design (not built)

`String` (host-owned), `Gc<Str>` (a GC string object with cached hash,
byte length and code-point length; interpreters declare their own string
`object`), `Symbol` as a type and compile-time pre-seeded symbols
(`intern!("__init__")`), rope concatenation, UTF-8 and UTF-16 iteration
and validation, and case mapping.

## `std.num`

`std/num.lm`. Parsing returns `Result<_, NumError>` with `Invalid` and
`Overflow` kept apart (`num.error_message(e)`):

- `parse_int(s)` (sign, decimal or `0x` hex, exact `i64` range),
  `parse_i64(s, radix)` / `parse_u64(digits, radix)` (radix 2 to 36),
  `parse_u64_wrapping` (Lua hex literals), `parse_f64(s)` (strict decimal
  syntax checked first, then correctly rounded by strtod),
  `parse_number(s) -> Number` (`Number.Int(i64)` or `Number.Float(f64)`;
  decimal overflow becomes a float, as in Lua), `decimal_len(s)` (the
  extent of a numeral, for lexers), `digit_value(c)`, `is_hex_digit(c)`.
- Formatting appends to a `text.StrBuf`: `write_g(b, x, 14)` (Lua's
  `%.14g`), `write_shortest(b, x)` (shortest round-trip, Python `repr`
  notation, by trying `%.{1..17}e` with snprintf), `shortest(x)` (the
  digits and exponent) and `write_js(b, x)` (ECMAScript
  Number::toString), `mark_float(b, start)` (append `.0` to
  integer-looking output) and `write_hex(b, x)`.
- Guest arithmetic: `floor_div`, `floor_mod`, `trunc_div`, `trunc_rem`
  (`?i64`: None on a zero divisor; `MIN / -1` wraps), `floor_div_f64`,
  `floor_mod_f64`, `add_overflow` / `sub_overflow` / `mul_overflow`
  returning `(i64, bool)`, and `to_i64_exact(f64) -> ?i64`.

### Design (not built)

Ryu formatting and Eisel-Lemire parsing (snprintf and strtod are used
instead); binary literals, digit separators and per-language literal
configuration.

## `std.bigint`

`std/bigint.lm`: arbitrary-precision integers with Python's semantics
(floor and truncating division, infinite two's complement for bitwise
operations on negative numbers, correctly rounded conversion to `f64`,
Python's `hash`). Tested by `tests/test/bigint.lm` and by a differential
test against CPython (`tests/bigint/diff.py`, run by `cargo test`
as `bigint_vs_python`: 3000 random cases, and 1000 more collecting
between the steps of the GC operations; longer runs by hand).

**Representation.** A magnitude of 64-bit limbs, least significant
first, normalized (no zero limb at the top; zero has no limbs), and a
sign. Limbs are `u64` because `a.mul_wide(b)` and `hi.div_wide(lo, d)`
(`std.core`) give the 128-bit products and quotients that 64-bit limbs
need; that makes multiplication about four times the work per
instruction of 32-bit limbs (CPython's are 30 bits).

The small case is the interpreter's: an unboxed integer in its value
type, with overflow checks (`num.add_overflow`, `x.mul_checked(y)`)
sending it here. std.bigint handles only the big case, through three
layers over one representation:

- **`Num`, a view**: `struct Num { neg: bool, d: []u64 }`, limbs that
  someone else owns. Every operation reads `Num`s and writes its result
  into a buffer the caller provides, sized by a matching function, and
  returns the result as a `Num` over the start of that buffer:
  `add_into(out, a, b)` with `add_size(a, b)` limbs, `sub_into`,
  `mul_into` (`mul_size`), `divmod_into(q, r, a, b, floor)` (`quot_size`,
  `rem_size`; None for a zero divisor; `floor` rounds the quotient down
  and gives the remainder the divisor's sign, Python's `//` and `%` and
  Scheme's `floor/`, else it truncates, C's and Scheme's `truncate/`),
  `shl_into` / `shr_into` (arithmetic: `-1 >> n` is -1), `and_into`,
  `or_into`, `xor_into` (`bitop_size`), `not_into`, `pow_into`
  (`pow_size -> ?u64`, None when the size overflows), `from_i64`,
  `from_u64` (one limb), `from_f64_into` (`f64_size`, truncates; None
  for inf and nan), `parse_into(out, digits, radix)` (`parse_size`).
  Views need no buffer: `neg`, `abs`, `cmp`, `cmp_abs`, `eq`, `sign`,
  `is_zero`, `is_odd`, `bit_length`, `to_i64` / `to_u64 -> ?..`,
  `to_u64_wrapping`, `to_f64` (nearest, ties to even; ±inf beyond the
  range), `hash_py` and `hash_i64` (CPython's `hash` of an int: modulo
  2^61 - 1 with the sign, -1 becomes -2), `of_limbs(p, n, neg)` (a view
  of limbs from elsewhere, normalized). Nothing allocates except scratch
  space inside the algorithms, freed before returning, so a view of a GC
  object's trailing array is a valid operand, and its limbs can be the
  output.
- **`BigInt`, owned**: `struct BigInt { neg: bool, d: mem.Vec<u64> }` on
  the C heap. `bigint.add(a, b) -> BigInt` (and `sub`, `mul`, `divmod(a,
  b, floor) -> ?(BigInt, BigInt)`, `pow`, `shl`, `shr`, `bit_and`,
  `bit_or`, `bit_xor`, `bit_not`, `copy`, `of_i64`, `of_u64`, `of_f64`,
  `parse(s, radix) -> ?BigInt`) take views, so owned and GC numbers mix;
  `x.num()` is an owned number's view, `x.free()` releases it.
- **`Big`, in the GC heap**: `object Big { neg: bool, len: u64, d: [u64] }`
  (`len` limbs of `d` are the number: results are allocated at their
  size bound). `gc_add(h, a, b) -> Gc<Big>` (and `gc_sub`, `gc_mul`,
  `gc_divmod`, `gc_pow`, `gc_shl`, `gc_shr`, `gc_and`, `gc_or`,
  `gc_xor`, `gc_not`, `gc_neg`, `gc_copy`, `gc_of_i64`, `gc_from_f64`,
  `gc_parse`) take `Arg`s, `enum Arg { Small(i64), Ref(Gc<Big>) }`, so
  a small operand needs no object. `b.num()` is a `Big`'s view. Allocating
  does not collect (collections run at the interpreter's safepoints), so
  these have no effect. An interpreter can store `Gc<Big>` in its own
  objects (examples/python's `Long`, an object behind its own
  `value` variant, holds an optional `Gc<Big>`) or as a `value` variant's
  payload.

Formatting: `bigint.write(b, x, radix)` appends to a `text.StrBuf`
(radix 2 to 36, lowercase, `-` for negatives, no prefix);
`parse_into` takes digits only (no sign, prefix, spaces or underscores:
the language's syntax is the caller's).

Algorithms: schoolbook multiplication, Karatsuba from `KARATSUBA` (32)
limbs (an unbalanced product is cut into balanced pieces), Knuth's
algorithm D for division (one `div_wide` per quotient limb), left to
right binary powering, power-of-two radixes by bit extraction, other
radixes by dividing out the largest power of the radix that fits a limb
up to `DC_LIMBS` (30) limbs and above that divide and conquer (split by
radix^(k·2^i), the halves formatted recursively, the low ones padded with
zeros), parsing by multiplying in a limb's worth of digits at a time.

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
layer with the operands as `Arg`s, and a result that fits the small case
again goes back to it (examples/python/big.lm is this, with an i64
"medium" `Long` in between):

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

Performance (`bench/bigint/bench.lm` against the same work in
`bench/bigint/bench.py` on CPython 3.13, best of 5, one machine):

| | std.bigint | CPython 3.13 |
|---|---|---|
| factorial(5000), a running product | 1.8 ms | 4.9 ms |
| fib(100000) by fast doubling | 0.58 ms | 1.7 ms |
| `str` of a 99,722-digit number | 20 ms | 33 ms |
| `int` of it | 9.7 ms | 17 ms |
| product, 99,722 by 20,899 digits | 2.0 ms | 7.5 ms |
| `divmod`, 120,621 by 20,899 digits | 6.4 ms | 15 ms |

### Design (not built)

Subquadratic division (Burnikel-Ziegler or Newton) and with it
subquadratic radix conversion (formatting is divide and conquer over
schoolbook division, so still quadratic; CPython 3.12+ converts huge
numbers with `_pylong`, which is asymptotically faster), divide-and-conquer
parsing, a squaring routine, FFT multiplication, `gcd`, integer square
root, correctly rounded `a / b` to `f64`, and in-place operations on an
owned `BigInt` (each operation returns a new one).

## `std.io` and `std.os`

`std/io.lm`: `print(s)`, `println(s)`, `print_int(i64)`, `print_f64(f64)`
(`%.14g`), `print_bool(b)`, their stderr forms `eprint`, `eprintln`,
`eprint_int`, `eprint_f64`, and `read_file(path) -> ?str` / `free_file(s)`.

`std/os.lm`: `arg_count() -> u64`, `arg(i) -> ?str` (0 is the program),
`quit(code) -> never`.

### Design (not built): `std.io`, `std.os`, `std.ffi`

`std.ffi` does not exist (`extern "C" fn` and `export "C"` are in the
language, language.md §1.5). Not built: buffered or formatted output,
stdin, environment variables, clocks, `mmap`, dynamic loading.

## `std.lex`

`std/lex.lm`. There is no comptime, so there is no lexer generator: the
module is a `Cursor` toolkit, and a lexer is a short function that tries
the scanners in order:

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

- `Syntax` holds the per-language conventions: line comment prefix, block
  comment delimiters (optionally nesting), Lua long-bracket comments, octal
  or decimal `\ddd`, whether backslash-newline keeps the newline, raw
  newlines in strings, whether overflowing decimal/hex literals become
  floats/wrap or are errors, and extra identifier bytes: `ident_extra`
  (anywhere, `"$"` for JavaScript) and `ident_inner` (after the first
  byte: `"?!"`, `"-"`). Presets: `lex.lua()`, `c_like()`, `js()`, `none()`.
  The cursor turns the identifier classes into 256-bit sets
  (`cursor.is_ident_start(b)`, `cursor.is_ident_char(b)`).
- `Cursor.new(src, file, syntax)`: `peek`, `peek_at`, `bump`, `advance`,
  `eat`, `eat_str`, `starts_with`, `eat_while(fn(u8) -> bool)`, `since`,
  `span_from`, `save`/`restore`, `at_end`, `pos` and `line` (fields),
  `error_at(start, msg)`, `error_here(msg)`. Character classes are ASCII
  (`lex.is_digit`, `is_alpha`, `is_ident_start`, `is_ident_char`,
  `is_hex_digit`, `is_space`).
- Scanners: `skip_trivia` (fails on an unterminated block or long comment), `skip_space` (whitespace and line comments only: cannot fail; for syntaxes without block comments), `ident -> ?str`, `at_number`/`number` (via
  std.num, with `3abc` reported as malformed), `quoted(&buf)` (escapes
  decoded into a `StrBuf`, including `\ddd`, `\xHH`, `\u{..}`, `\z`, line
  continuation; after a bad escape it still scans to the closing quote so
  the lexer can continue), `long_bracket_level`/`long_string`,
  `lex.push_utf8(buf, cp)`. There is no "word" scanner for
  whitespace-delimited languages: use `eat_while`.
- `OpTable<K>`: `new`, `add(text, kind)`, `match_at(s, pos)`; operator
  strings to token kinds, maximal munch through per-first-byte chains
  sorted longest first. Matching is `cursor.op(&ops) -> ?K` (or the older
  `ops.eat(&cursor)`).
- `Keywords<K>`: `new`, `add(word, kind)`, `get(word) -> ?K`; a
  `HashMap<str, K>` wrapper.
- Errors are `lex.Error { span, msg }` with a static message;
  `e.diag()` turns one into a `diag.Diag`.

A complete lexer for Mini's token set is about 50 lines
(`tests/run/lex_toy.lm`).

### Design (not built)

A comptime lexer generator: a token specification turned into a DFA at
compile time, with longest match, priority by declaration order, and
keyword tables via perfect hashing:

<!-- snippet: skip design, not built (no comptime) -->
```loimi
const Tok = std.lex.define(.{
    .skip     = .{ "[ \t\r\n]+", "--[^\n]*" },
    .tokens   = .{ .Num = "[0-9]+(\\.[0-9]+)?([eE][+-]?[0-9]+)?",
                   .Name = "[A-Za-z_][A-Za-z0-9_]*",
                   .Str  = std.lex.quoted('"', escapes = .c) },
    .keywords = .{ .from = .Name, "if", "then", "else", "end" },
    .punct    = .{ "+", "-", "*", "/", "==", "~=", "<", "<=", "=", "(", ")" },
});
```

Also not built: UTF-8 aware character classes.

## `std.parse`

`std/parse.lm`:

- `Pratt<P, K, E, X>` over parser state `P`, a payload-free token-kind
  enum `K` (used as a table index via `k as u64`), expression result `E`
  and error type `X`. `Pratt.new(peek, advance, atom, unary, binary)`
  takes the hooks as function pointers; the optional `before_rhs` hook is
  a field (`pr.before_rhs = Some(f)`). The table is filled
  at run time (`infix_left(k, bp)`, `infix_right(k, bp)`,
  `infix(k, l, r)`, `prefix(k, bp)`), and `parse(p, min_bp)` (or
  `expr(p)`) calls the user's `peek`, `advance`, `atom`, `unary` and
  `binary` functions and `before_rhs` (where a single-pass
  compiler puts the left operand in a register or emits the jump of
  `and`/`or`). `infix_of(k)` and `prefix_of(k)` read the table.
- Operators that are not `lhs op rhs` get a per-kind `led` hook
  (`led(k, left, right, hook)`): when the operator binds, the driver
  consumes it and the hook parses the rest and returns the result. That
  covers mixfix (`c ? a : b`), postfix (`n!`) and right-hand sides with
  their own grammar (assignment with a target check, `x => body`); the
  hook parses operands with `pr.parse(p, pr.infix_of(k).right)`. MiniJS
  parses assignment, `?:` and arrows this way. Postfix chains such as
  calls and indexing can stay in `atom`. A hook that needs to recurse
  (parentheses) reaches the table through a pointer in `P`.
- `Parser<P: Hooks<K, E, X>, K, E, X>` is the same table and driver with
  the hooks as methods of the parser state: `P` declares
  `impl parse.Hooks<K, E, X> for P;` and defines `fn P.peek`, `advance`,
  `atom`, `unary` and `binary`, and optionally `before_rhs` (default:
  unchanged) and `led` (one method for all kinds registered with
  `pr.led(k, left, right)`, dispatching on the kind; the default traps).
  The calls are direct, so they inline.
- The hooks' types have no effects, so `parse` has none and runs from any
  function: hooks may allocate but not collect (collect between parses).

`Span` and `SourceMap` live in `std.diag`.

### Design (not built)

`TokenStream` (lookahead, `expect`, and error recovery to synchronization
tokens, `recover_to(.{ .Semi, .End })`); an operator table built at
compile time.

## `std.diag`

`std/diag.lm`:

- `Span { file, lo, hi }`, `diag.span(file, lo, hi)`, `a.to(b)`, `len`.
- `SourceMap` (borrowed names and text): `new`, `add(name, text) -> u32`,
  `name(id)`, `text(id)`, `snippet(span)`, `line_count(id)`,
  `line_col(id, pos) -> LineCol` (binary search in a line-start table
  built on first use), `line_text(id, line)`, `line_span(id, line)` (for
  errors that only know a line), `write_location(&buf, span)`,
  `render(&d, &buf)`, `eprint(&d)`, `free`.
- `Diag`: `Diag.new(severity, ?span, msg)`, `diag.error(span, msg)`,
  `diag.warning(span, msg)`, built up with `with_label`, `with_note`,
  `with_note_at`, `with_help`. `Severity` is `Error`, `Warning`, `Note`,
  `Help`. Rendered in the same format as the Loimi compiler's own errors:
  a snippet with carets, tabs preserved under the caret line, spans
  clipped to their first line, lines over 160 bytes cut to a window
  around the span.
- `Diagnostics`: a sink that copies the messages pushed into it into an
  arena (so they can be built in a temporary `StrBuf`): `push(d)`,
  `error(span, msg)`, `warning(span, msg)`, `len`, `at(i)`,
  `error_count`, `warning_count`, `has_errors`, `summary(&buf)`
  (`1 warning and 2 errors generated.`), `render(&sm, &buf)`,
  `eprint(&sm)`, `clear`, `free`.

### Design (not built)

Fix-it hints, ANSI color, multi-line spans, guest tracebacks.

## `std.bc`: bytecode construction

`std/bc.lm`, a library over the generated encoders (language.md §3.3).
Everything is generic over the vm's code word `W` through two functions
passed in: `size: fn(W) -> u64` (the generated `Vm.op_size`) and
`patch: fn(W, i64) -> ?W` (set a jump word's offset, `target - pc`, or
None if it does not fit: the generated `Vm.set_label`, or a few lines
over `Vm.opcode` and `bc.Field` for a custom encoding). The test
`tests/run/std_bc.lm` compiles a small language to a toy register vm
with all of it.

- `Code<W>`: `Code.new(Vm.op_size, Vm.set_label)`; `op(w)` emits an
  instruction and zeroes its inline-cache words, `word(w)` a raw word;
  `pc`, `len`, `at`, `set`, `words`, `copy_into`, `clear`, `free`. Source
  positions: `set_loc(loc)` (returns the previous one) tags the following
  instructions; positions are kept as runs, read with `loc_at(pc)`,
  expanded per word with `locs_into(out)`, or encoded with
  `encode_locs(&bytes)` (delta-encoded LEB128 runs, about 2 bytes per
  line change) and read back with `bc.line_at(bytes, pc)` on error paths.
- Labels: `label()`, `bind(l)`, `here()` (a placed loop head),
  `jump(w, l)`, `fixup(at, l)` (for a jump word that is not the first
  word emitted), `label_pc`. Jump lists for Lua-style compilers that pass
  pending exits around: plain `mem.Vec<u64>` of jump pcs with
  `patch_list(&l, target)` and `patch_here(&l)`; `patch(at, target)` and
  `jump_to(w, target)` for single jumps.
- Errors are sticky: an offset that does not fit records `JumpRange` with
  the pc and its source position, `check() -> ?Error` returns the first
  error (also `Unbound` for a label with pending jumps that was never
  placed), and `Error.message()` describes it. `fail(kind, pc)` records
  one. Compilers that stop at the first error can still check `patch`'s
  bool result.
- Peephole support: `last()` is the pc of the previous instruction if it
  may be rewritten or merged with the next one (no jump targets the
  current pc: binding a label or patching to here blocks it, as Lua's
  `lasttarget` does); `truncate(pc)` removes code (dropping pending
  fixups in it); `mark_target()`.
- `thread_jumps(target_of, is_goto)`: retargets jumps that land on an
  unconditional jump, following chains up to 8 deep; nothing moves.
- Branch relaxation: `set_relax(Vm.label, invert, Vm.JMP(0))` turns it
  on. A conditional jump whose offset does not fit its field is then
  recorded instead of failing, and `relax()`, called after the last label
  is bound, rewrites each into `invert(w)` (the opposite condition,
  jumping over the next instruction) followed by the long unconditional
  jump to the target. Later code moves down: every jump offset, the
  position table and placed labels are updated (and a jump that stops
  fitting is relaxed too); pcs kept from before are stale. `invert`
  returns None for words without a long form, which stay errors.
  `needs_relax()` tells if any jump waits; `check()` reports them as
  `JumpRange` until `relax()` runs. Lox (`examples/lox/compile.lm`) and
  `tests/run/std_bc_relax.lm` use it.
- `bc.Field { shift, bits, signed }` with `fits`, `get`/`set` (and
  `get32`/`set32`): range-checked access to an encoded field.
- `Pool<K: hash.Hash>`: `Pool.new(limit)`, `add(k) -> ?u32` (deduplicated
  through a `HashMap`; None when full), `push` (no dedup), `find`, `get`,
  `len`, `items`, `clear`, `free`. For a nanboxed `Value`, `fn V.hash`
  over the bits and `fn V.eq(a, b) { a == b }` keep `0.0`, `-0.0` and
  `0` apart. There is no call that hands over the items and frees only
  the index.
- `Regs`: `Regs.new(limit)`; `alloc(n)` (past the limit it sets `failed()`
  and still returns, so the error can be reported later) or
  `try_alloc(n) -> ?u32`; `free(r)` (top only, locals ignored, false if
  out of order), `free2(a, b)`, `is_temp`, `mark`/`reset` (save and
  restore the free register), `end_stmt`, `activate(n)` (registers
  become locals), `enter`/`leave` (blocks), `frame_size(min)`.

Nothing in `std.bc` collects, so GC references in a `Pool` are not
roots: compile between collections, as Mini and Scheme do.

### Design (not built)

The design had a generated `Builder` per vm that this library would plug
into. Not built:

- emitters as `b.ADD(r1, r2, r3)` methods (the generated `Vm.ADD(..)`
  encoders plus `code.op` cover it);
- an exception table (ranges to handler pc) for `on raise`;
- `Proto` assembly that calls the verifier and returns `Verified<V>` (each
  interpreter's `Proto` and verify context differ; the compiler calls
  `Vm.verify` and `Vm.optimize` itself);
- serialization of protos (bytecode cache files).

## `std.gc`

`std/gc.lm`; details and measurements in
[`stdlib/gc.md`](stdlib/gc.md#as-implemented). The language side
(`object`, `Gc<T>`, `new`, tracing, effects, write barriers) is in
`language.md` §5 and §8.

- `Heap.new()`, `heap.free()`. Allocation is `new(&heap, Obj { .. })`
  (or with a trailing-array length); `gc.alloc` is the low-level call
  under it.
- Collection happens only where you call it: `gc.collect(&heap)` or
  `gc.maybe_collect(&heap)` (collects if `heap.allocated >
  heap.threshold`, or in stress mode), both `uses gc`. Allocation never
  collects.
- Roots: traced locals, and in vm code copies of traced `state`
  variables, are rooted by the compiler at `uses gc` calls.
  Everything else (globals, VM stacks in memory, host tables) is
  marked by a root scanner, `heap.add_scanner(data, scan)`, where
  `scan: fn(*Heap, *u8)` calls `gc_mark(h, v)` on each reference.
  Scanners have no effects: they cannot collect.
- `heap.add_sweeper(data, sweep)`: runs after marking; with
  `h.is_marked(p)` (or the slower `gc.is_marked(p)`) it drops references
  to dead objects (weak tables).
  `heap.is_minor()` tells it only young objects can have died.
- Tuning: `set_growth(percent, min_heap)` (default 100% and 1 MiB),
  `set_max_budget(bytes)`, `set_generational(on)`, `set_nursery(bytes)`,
  `set_stress(on)` (collect at every `maybe_collect`), `set_verify(on)`
  (check write barriers at each collection). `LOIMI_GC_STATS=1` prints a
  line per collection.
- Weak references: `heap.new_weak(x) -> Gc<gc.Weak<T>>`, `w.get() ->
  ?Gc<T>`, `w.set(..)`. Ephemerons: `gc.Ephemerons.new(&heap) ->
  *Ephemerons<K, V>`, `get`, `set`, `remove`, `len`, `free`.
- Weak intern tables for GC strings: `gc.WeakInterner.new(&heap, make)
  -> *WeakInterner<X>` over the interpreter's string object X (fields
  `hash: u64` and `data: [u8]`; `make(h, s, hash)` allocates one, or use
  the ready-made `gc.Str` and `gc.new_str`). `intern(s) -> Gc<X>` (the
  one live string with those contents; allocates, never collects),
  `find(s)`, `len`, `free`. Entries are weak: dead strings leave the
  table at the next collection. `key(x) -> u64` pins `x` (alive and in
  the table for the table's lifetime) and returns its address, a stable
  key for `std.obj` properties and other name-keyed tables (an unpinned
  string's address can be reused once it dies); `is_pinned(x)`.
- Write barriers: the compiler emits `gc.barrier(obj)` after every store
  of a reference into an object. After a store through a raw pointer,
  call `gc.write_barrier(&heap, obj)`; `gc.needs_barrier(obj)` and
  `gc.remember(obj)` log several objects at once.
- Hooks for objects the generated code cannot fully see
  ([details](stdlib/gc.md#trace-hooks-free-hooks-and-dirty-pinning)):
  `#[trace(f)]` on an `object` (a custom trace of the trailing array or
  of host memory it owns), `heap.on_free(obj, f)` (release host memory
  when `obj` dies; no guest code, no resurrection), and
  `heap.pin_dirty(obj)` / `unpin_dirty` / `is_pinned_dirty` (an object
  written without barriers, traced at every collection).
- `loimi build --gc-check-roots` validates every reference the collector
  marks and poisons unassigned slots and freed objects
  ([details](stdlib/gc.md#checking-roots---gc-check-roots)).
- Threads: heaps have no shared state, so independent heaps run on
  different threads without locks ([`stdlib/gc.md`](stdlib/gc.md#9-heaps-and-threads)).
  Threads sharing a heap, one at a time: `std.gil` (`gil.Gil.new(&heap,
  data)`, `attach`, `detach`, `unlock`/`lock`, the poll `waiting()` and
  `yield_now()`, hooks that move per-thread interpreter state); see
  [`stdlib/gc.md`](stdlib/gc.md#threads-sharing-a-heap-stdgil).
  `gc.mark_shadow_stack(h, top)` marks another thread's saved roots.

Every object has a 16-byte header (slot size, type id, mark bits, and
the trailing-array length, which compiled code reads at object - 8), so
a two-word object such as a Lisp pair takes a 32-byte slot.

**Growth policy.** After a collection, `percent`% of the live bytes (at
least `min_heap`) may be allocated before the next one. The default,
100% and 1 MiB, collects again after allocating as much as survived (the
heap peaks at about twice the live data). MiniJS uses 300% with an 8 MiB
minimum (as fast as 100% / 64 MiB, a quarter of the memory on small
heaps; see `examples/minijs/README.md`). The default stays because no
setting was better for every interpreter (best of 3-7, wall clock,
loaded machine): Mini at 200% runs binary_trees 17% faster (1.21 to
1.01 s) and the rest within noise; Scheme at 200% runs nboyer, graphs
and mperm 4-10% faster but string 27% slower (41 to 52 ms) and primes
and array1 3-4% slower; an 8 MiB minimum makes Scheme's small
allocation-heavy programs (deriv, divrec, cpstak) 6-10% slower, as the
heap leaves the CPU caches. `set_max_budget(bytes)` caps the budget (0,
the default, is no cap), so that a large growth factor bounds the heap
at the live data plus `bytes` rather than at a multiple of it once the
live data is large. Scheme uses 400%, a 2 MiB minimum and a 256 MiB cap
(see `bench/scheme/README.md`).

### Design (not built)

Full design: [`stdlib/gc.md`](stdlib/gc.md). Not built:

- heap policies as a compile-time parameter (`#![heap = ...]`) and the
  `Incremental` policy (tri-color, snapshot-at-the-beginning barrier);
- lazy sweeping in a non-generational heap, card marking, aging of small
  objects before promotion, a moving collector;
- roots generated from vm stacks and frame structs (write a root
  scanner; traced `state` variables are rooted automatically),
  `Root<T>`, `GcVec<T>`;
- finalizers that run guest code (free hooks for host memory are
  built), heap snapshots, poisoning outside `--gc-check-roots` builds;
- inline caches in bytecode as weak roots (caches cannot hold GC
  references; language.md §7.1).

## `std.obj`: objects and shapes

`std/obj.lm`; the API is described in
[`stdlib/obj.md`](stdlib/obj.md#as-implemented). In short: shapes with
weak transitions and ids that stay stable across collections (an id
names a transition path), inline and overflow slots, dictionary mode,
per-property attribute bits, prototype chains with validity cells (plain
ids, so a prototype hit is a few plain words), the inline-cache state
machine (`obj.Site`, `read_miss`/`write_miss`: refill, polymorphic,
megamorphic, cached property adds; `poly_get_at`, `mega_get_at` and
`read_miss_at` also say whether a value came from the receiver or a
prototype, for method binding) and allocation templates. Keys are u64
symbol ids or pinned string addresses (`gc.WeakInterner.key`; see
"Keys for guest strings" in `stdlib/obj.md`). Use the ready-made generic `obj.Obj<V>` / `obj.Slots<V>`,
or declare your own object type (fields `shape`, `dict`, `overflow`,
optionally `proto`, and trailing `inline: [Value]`) and slot-array type;
either way, instantiate `obj.Model<O, S, V>` with a function that
allocates slot arrays. An inline-cache hit is two loads, a compare and
the slot load (`bench/obj`).

### Design (not built)

Full design: [`stdlib/obj.md`](stdlib/obj.md). Not built: a compile-time
`Model` configuration (attributes, prototypes, ordering, element storage
and thresholds chosen per guest language), elements (integer-indexed
storage), property cells for globals, `SetError`, and lookup tables shared
along a transition chain.

## `std.rt`: runtime support

`std/rt.lm` has `rt.protect(f: fn(*u8), data: *u8) -> bool` and
`rt.throw() -> never`, a setjmp/longjmp pair (the runtime half is
generated by `src/rtgen.rs`). They let a library return an error from
deep inside code that has no error path, such as Mini's recursive-descent
parser (`compile.try_compile`), instead of exiting the host process.
`throw` goes to the innermost `protect` on the thread (it panics if there
is none) and restores the shadow-stack top. The frames in between are
abandoned: their `defer`s do not run and what they `mem.alloc`'d leaks.
`f` has a function type without effects, so it cannot collect, and no GC
roots are lost.

Panics have the same machinery: `rt.catch_panic(f, data) -> bool` runs
`f(data)` and returns false if it panicked, and `rt.panic_message() -> str`
is the message of the last panic caught on the thread
(`"lib.lm:12:5: index out of bounds"`). `rt.panic(msg)` panics with your
own message (a catch keeps a copy of its first 512 bytes, so it may live
in a buffer the unwinding abandons). A `throw` passes through a
`catch_panic` to the enclosing `protect`, and a panic
passes through a `protect`. On an `export "C"` function the attribute
`#[catch_panic(v)]` does the same at the C boundary; the guarantees are
in `language.md` §10.1.

A VM stack is something you allocate yourself (`mem.alloc<Value>(n)`)
and keep in state registers; frames are your own structs; the GC sees
the stack through a root scanner. The example interpreters
(`examples/mini`, `examples/stack`, the tutorial's step 7 onwards) show
the pattern.

### Design (not built)

- `ValueStack<T>`: the VM stack. It would reserve virtual address space up
  front and commit pages on demand, so it never moves, and `StackTop`
  pointers stay valid. Overflow is a single compare against the limit,
  done once per call.
- `FrameStack<F>`: guest call frames, with entry markers for reentrant calls.
- `Upvalues`: open-upvalue list and closing (Lua-style), for closure capture by
  reference with stack-allocated locals.
- `Fiber`: stackful coroutines (separate host stack), for guest coroutines
  that must suspend across host frames.
- A C-stack depth guard for `reenter` (the compiler's `Vm.depth` counter
  is built; checking it is the interpreter's job).

## `std.ast`

**Not built.** Design: arena-allocated trees with `Id<T>` links, a
generic visitor via comptime reflection, and scope and resolver helpers:
`Scopes<Symbol, Binding>` with local, upvalue, and global classification,
which is the core of Lua-, JS-, and Python-style closure compilation.

## `std.prof`

`std/prof.lm`, for timing and counting inside an interpreter or its
benchmark driver. Nothing in it runs unless called, and the vm-stats
access is compiled in only when the program uses it.

- **Clock**: `prof.now_ns() -> u64` (`CLOCK_MONOTONIC`), and a stopwatch
  `prof.Timer.start()` with `t.ns()`, `t.ms() -> f64`, `t.restart()`.
  `prof.fmt_ns(&buf, ns)` appends "812 ns", "3.25 us", "41.7 ms".
- **Benchmarks**: `var b = prof.Bench.new(warmup, runs)`, then
  `b.add(name, f, data)` per variant (`f: fn(*u8) uses reenter`, so a
  variant may run an interpreter that collects; `run` then needs the
  effect too), and `b.run()`: `warmup` untimed rounds, then `runs`
  rounds that each time every variant once, starting from a different
  one each round, so drift in the machine's speed hits all variants
  alike. It prints name, minimum, median and minimum relative to the
  first variant on stderr (`b.report(&buf)` writes the table);
  `b.min_ns(i)` and `b.median_ns(i)` give the numbers. `b.free()`.
- **Counters**: `prof.Counters.new()`, `c.add(name) -> id`, `c.inc(id)`,
  `c.bump(id, n)` (inlined), `c.get(id)`, `c.report()` (stderr),
  `c.free()`: for events `--vm-stats` does not see (cache misses,
  slow paths outside `cold` blocks, which `--vm-stats` counts itself).
- **Vm stats**: in a `loimi build --vm-stats` build,
  `prof.op_counts("Vm") -> ?prof.OpCounts` gives the vm's counters
  (None without the flag or for an unknown vm; `prof.vm_stats_enabled()`).
  `c.len()` ops in declaration order (supers included), `c.name(i)`,
  `c.count(i)`, `c.count_of("ADD")`, `c.total()`, `c.reset()` (op counts,
  pairs and sequences: measure one phase, or skip a warmup; the report
  at exit covers what ran since) and `c.report()` (the exit report, now).
  The exit report (language.md §7.3) also counts entries into each
  `cold` block, per op, with its source location.

### Design (not built)

A sampling profiler that reads the synced `pc` from a timer signal
(made accurate by an instrumentation variant that syncs on backward
jumps and calls) and maps it through the line table to guest source
lines, with folded stacks for flame graphs; `suggest-supers` as a
library over the pair counts; confidence intervals in `Bench`.

## `std.test`

`std/test.lm` and the `loimi test` command (language.md §1.2).

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

- **Running.** `loimi test FILE.lm` replaces FILE.lm's `main` and runs
  each `#[test]` function of FILE.lm in source order, in FILE.lm's
  directory. Arguments: filter words (a test runs if its name contains
  one), `--list`, `--update`, and the build options (`--release`,
  `--dispatch=`, `--sanitize=address`, `-W error`, ..). A test fails on
  any panic: an assertion, a bounds check, `unwrap` of None; the frames
  it abandons leak (`language.md` §10.1), and the rest of the run goes
  on. A crash that is not a panic (a segfault in unchecked code) ends
  the run. The generated `main` is plain Loimi over `test.Runner`
  (`Runner.new()` reads the arguments, `r.run(name, f)`,
  `r.finish() -> i32`), which a hand-written `main` can use as well.
- **Assertions** fail the test with the location of their caller
  (`#[track_caller]`; a helper of your own marked `#[track_caller]`
  passes the test's line through): `test.assert(c)`,
  `test.assert_msg(c, msg)`, `test.eq(got, want)` and `test.ne(a, b)`
  (types with `fn T.eq` and `impl test.Eq for T;`: `str`, `bool`, `f64`
  and the integers are built in, and `text.Fmt` to print them),
  `test.some(opt) -> T`, `test.none(opt)`, `test.ok(result) -> T`,
  `test.err(result) -> E`, `test.has(text, part)`,
  `test.panics(f, data) -> str` (the panic's message; fails if `f`
  returns), `test.fail(msg)`. `test.caller()` is the location inside a
  `#[track_caller]` function.
- **Output and golden files.** `test.capture_start()` sends stdout and
  stderr (descriptors 1 and 2) to a temporary file until
  `test.capture_end(&buf)` appends what was written; a test that fails
  while capturing shows the captured output. `test.golden(text, path)`
  compares with the file at `path` and names the first differing line;
  `loimi test --update` (or `LOIMI_TEST_UPDATE=1`) writes the file
  instead.

Conformance tests that run a built interpreter on script files remain
shell scripts (`examples/*/check.sh`); bytecode fuzzers are ordinary
Loimi programs (tutorial chapter 9, `tests/fuzz/`).

### Design (not built)

- A golden-file runner over `tests/**/*.<ext>` scripts with `-- expect:`
  comments, `--update` and parallel runs (`check.sh` does this per
  example today).
- A mode matrix: each test under every dispatch strategy, value repr and
  GC policy asked for (`--matrix dispatch=*,gc=MarkSweep,stress`).
- Differential testing against a reference interpreter binary.
- Tests collected from imported modules, and tests in parallel or in
  separate processes (so that a crash fails one test, not the run).
