# Standard library

The language handles the inner loop. The standard library handles the rest of
an interpreter: getting from source text to verified bytecode, and providing
the runtime structures every dynamic language needs.

Each section below documents a module's API in `std/*.lm`; import it
with `import std.NAME;`.

| Module | Status | Source |
|---|---|---|
| [`std.core`](#stdcore-built-in-types-and-operations) (built-ins) | built in (formatting is in `std.text`, assertions in `std.test`) | the compiler |
| [`std.mem`](#stdmem) | built | `std/mem.lm` |
| [`std.hash`, `std.map`](#stdhash-and-stdmap), [`std.dict`](#ordered-maps-maporderedmap-and-dictdict) | built (`HashMap`, insertion-ordered `OrderedMap`; `Dict`, the same in the GC heap; `KeyDict`, without stored hashes, for keys that cache theirs) | `std/hash.lm`, `std/map.lm`, `std/dict.lm` |
| [`std.text`](#stdtext) | built (`StrBuf` with `{}` formatting, `Interner`, small helpers) | `std/text.lm` |
| [`std.num`](#stdnum) | built | `std/num.lm` |
| [`std.math`](stdlib/math.md) | built (`Divisor`: division by a prepared constant, as an inline cache) | `std/math.lm` |
| [`std.bigint`](#stdbigint) | built (Karatsuba, Toom-3, divide-and-conquer division, square root and radix conversion; no FFT) | `std/bigint.lm` |
| [`std.io`, `std.os`](#stdio-and-stdos) | built (minimal) | `std/io.lm`, `std/os.lm` |
| [`std.lex`](#stdlex) | built (a cursor toolkit) | `std/lex.lm` |
| [`std.parse`](#stdparse) | built (`Pratt`, `Parser`, `Ops`) | `std/parse.lm` |
| [`std.diag`](#stddiag) | built | `std/diag.lm` |
| [`std.debug`](#stddebug-guest-debuggers) | built (breakpoints with conditions and hit counts, stepping, pause requests, handles for lazy expansion, commands; frames and locals are the interpreter's) | `std/debug.lm` |
| [`std.bc`](#stdbc-bytecode-construction) | built | `std/bc.lm` |
| [`std.gc`](#stdgc), [details](stdlib/gc.md) | built (mark-sweep, optional generational mode) | `std/gc.lm` |
| [`std.gil`](stdlib/gc.md#threads-sharing-a-heap-stdgil) | built (a global interpreter lock per heap) | `std/gil.lm` |
| [`std.obj`](#stdobj-objects-and-shapes), [details](stdlib/obj.md) | built | `std/obj.lm` |
| [`std.rt`](#stdrt-runtime-support) | built (`protect`/`throw`, `catch_panic`/`panic`, `ValueStack`) | `std/rt.lm` |
| [`std.prof`](#stdprof) | built (clock, `Bench`, `Counters`, `--vm-stats` counts; sampling is `loimi build --vm-prof`) | `std/prof.lm` |
| [`std.test`](#stdtest) | built (`#[test]`, `loimi test`, assertions, capture, golden files) | `std/test.lm` |

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
  `OrderedMap`, `StrBuf`, `Interner`, `OpTable`, `Keywords`, `Pratt`, `Parser`, `Ops`,
  `SourceMap`, `Diag`, `Diagnostics`, `Code`, `Pool`, `Heap`, `Shapes`,
  `MegaCache`, `Ics`, `Model`, `Ephemerons`, `WeakInterner`, `Gil`, `Out`,
  `BigInt`, `Bench`, `Counters`; `Malloc` frees each allocation with
  `free_bytes`), and `io.read_file`'s result
  is released with `io.free_file`. Freeing a container does not free what
  its elements point to.
- **Checked where it can be.** `std.lex`, `std.parse`, `std.diag`,
  `std.text`, `std.hash`, `std.io`, `std.bc`, `std.map` and `std.dict` are
  `#![safe]` modules (`language.md` §1.3): no raw-pointer indexing, arithmetic or
  casts outside a few commented `unsafe { }` blocks around C calls.
  Functions that take a pointer and a length, free, or return raw
  memory are marked `#[raw]` (`mem.alloc`, `grow`, `release`, `copy`,
  `sort_by`, `try_sort_by`, `Arena.alloc_bytes`, `Malloc.alloc_bytes`,
  `Malloc.free_bytes`, `io.free_file`, `map.probe_*`, and `obj`'s
  inline-cache readers and writers that trust their cache words:
  `Model.load_ref`, `hit`, `poly_get`, `MegaCache.put`, ...): a `#![safe]` module calls them
  only in `unsafe { }`; elsewhere they are ordinary functions. A
  container's methods leave it valid (`free()` leaves it empty and
  usable); pointers and slices obtained from it (`items()`, `ptr_at`,
  `get_ptr`, `entry`) are valid until it next grows or is freed, which
  is not checked at compile time. Under `--sanitize=address`, `Vec`,
  `Arena` and `OrderedMap` poison the memory they keep but no longer use
  (`mem.poison(p, n)`/`mem.unpoison(p, n)`, `#[raw]`, defined only in
  such builds: `#[cfg(sanitize_address)]`), so ASan reports a stale
  pointer into it as it reports one into freed memory. See
  `design/safety.md` for the audit of every module and section 7 for
  the run-time check.

---

## `std.core`: built-in types and operations

There is no `std/core.lm`: these are built into the compiler and need no
import.

- **Types**: `i8`..`i64`, `u8`..`u64`, `isize`, `usize`, any-width
  `iN`/`uN` (1..=64), `f32`, `f64`, `bool`, `str` (a `[]u8` slice), slices
  `[]T` (`.len`, `.ptr`, `s[a..b]`), arrays `[T; N]`, pointers `*T`,
  `?T`, `Result<T, E>`, tuples, 128-bit vectors `i8x16` .. `f64x2`
  (language.md §1.1).
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
  `a.mul_add_wide(b, c, d) -> (u64, u64)` is `a * b + c + d` as (low,
  high): it always fits (at most 2^128 - 1), the step of a
  multiply-and-accumulate loop.
- **Carry chains** on `u8`, `u16`, `u32` and `u64`: `a.add_carry(b, c)
  -> (T, bool)` is `a + b + c` (the `bool` carry in counting 1) as (sum
  modulo 2^bits, carry out), and `a.sub_borrow(b, c) -> (T, bool)` is `a
  - b - c` as (difference modulo 2^bits, borrow out). A loop that feeds
  each carry out into the next carry in compiles to a chain of `adc`/`sbb`
  (x86-64) or `adcs`/`sbcs` (AArch64) instructions, the carry staying in
  the flags register when nothing between the steps sets flags: each
  lowers to two `llvm.uadd.with.overflow` (or `usub`) steps whose
  overflows are or'ed, the pattern LLVM selects as one add-with-carry.
  `c as u64` turns a carry into a limb.
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
- **Vectors** (`i8x16` .. `f64x2`, language.md §1.2, "Vectors"):
  `T.splat(x)`, `T.new(..)`, `T.from_bits(v)`, `T.narrow_sat(a, b)`;
  `v.lane(i)`, `v.with_lane(i, x)` (constant `i`); lane-wise operators
  (comparisons give masks); `abs`, `min`, `max`, `add_sat`, `sub_sat`,
  `avgr`, `clz`, `ctz`, `popcount`, `select`, `bitmask`, `widen_low`,
  `widen_high`, `shuffle`, `swizzle`, and the float methods above
  (`sqrt` .. `minimum`/`maximum`, without `is_nan` and `to_bits`)
  lane by lane; `as` between vectors with the same lane count.
- **Options**: `o.is_some()`, `o.is_none()`, `o.unwrap()` (traps on
  None), `o.unwrap_or(d)`, `o ?? d`, and `?` (also on `Result`).
  `o ?? p` with `p: ?T` is `?T`: `o` if it is `Some`, else `p`, which is
  evaluated only then (Rust's `o.or_else(|| p)`; chains:
  `a ?? b ?? c`).
  `Result` has no methods: use `match`, `if let` or `?`.
- **Pointers**: `p.offset(n)` (`i64` elements), `p.slice(n) -> []T`,
  `p + n`, `p[i]`, `*p`; `p.read_unaligned()` and `p.write_unaligned(v)`
  for addresses that may not be aligned for `T` (language.md §1.3);
  `p.prefetch()`, a cache hint that never faults.
- **Other built-ins**: `size_of<T>() -> u64`; `likely(c)`,
  `unlikely(c)`, `assume(c)`; `trap() -> never` and `unreachable`;
  `new(heap, Obj { .. }, n)` and `gc_mark(heap, v)` with `std.gc`
  imported (§8 of language.md).
- **String equality** is `a.eq(b)`, defined in `std.hash` (`import
  std.hash;`); `==` does not compare slices.

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
  `extend(s)` (append a slice; `s` may be the vector's own items),
  `Vec.filled(n, x)` (`n` copies of `x`),
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
  (copy a string into the arena), `free()` (the arena stays usable:
  the next allocation starts a new chunk). 64 KiB chunks; larger requests
  get their own chunk. `free_bytes(p)` does nothing (arena memory is
  freed all at once), so `c.free(&arena)` on an escaping closure and code
  generic over the allocator work.
- `Malloc`: an allocator on the C heap whose allocations are freed one
  by one: `Malloc.new()`, `alloc_bytes(n, align) -> *u8` (alignment up
  to 16), `free_bytes(p)`, and `live`, the number of allocations not
  freed yet. For escaping closures freed one at a time (`new(&m, |..|
  ..)`, `c.free(&m)`) and cells (`new(&m, v)`, `m.free_bytes(p as
  *u8)`).

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
  `set_value(e, v)`, `is_live(e)`, `remove_at(e)` (for `OrderedMap`,
  an `e` that is not a live entry is a panic; `dict.Dict` checks only `remove_at` and bounds: a dead entry reads as zeros). `eq` gets a copy of
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

## `std.text`

`std/text.lm`:

- `StrBuf`, a growable byte string: `StrBuf.new()`, `push(byte)`,
  `push_str(s)`, `push_int(i64)`, `push_f64(f64)` (shortest `%g` that
  reads back as the same number, `num.write_g_shortest`), `as_str()`
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
  `%.14g`, by snprintf), `write_shortest(b, x)` (shortest round-trip,
  Python `repr` notation without the `.0`), `write_g_shortest(b, x)`
  (`%.{p}g` with the fewest digits p that read back: `1e+02` for 100),
  `shortest(x)` (the digits and exponent) and `write_js(b, x)`
  (ECMAScript Number::toString), `mark_float(b, start)` (append `.0` to
  integer-looking output) and `write_hex(b, x)`.
- The shortest digits come from Schubfach (Giulietti 2020): one
  64x128-bit multiplication (`mul_wide`) per end of the rounding
  interval against a table of 617 128-bit powers of ten, no loop over
  precisions. Among equally short decimals it takes the closest to `x`,
  as C's correctly rounded `%.{p}e` does; at exact powers of two, where
  the interval is lopsided, it takes C's `%.{p}e` digits for the fewest p
  that read back (that need not be the shortest decimal in the interval:
  `2^-24` is `5.9604644775390625e-08`, not `5.960464477539063e-08`), so
  the output is the same as a search over `%.{1..17}e` with snprintf and
  strtod. `tests/run/std_num_shortest.lm` checks that against such a
  search (every power of two and power of ten with their neighbours,
  subnormals, extremes, random doubles). `write_shortest` takes about
  80 ns per number where the search took 10 us (a mix of short and
  17-digit numbers). Python, MiniJS and Scheme loops that convert
  floats to strings run 55 to 80 times faster, and loops that print them
  13 to 56 times.
- Guest arithmetic: `floor_div`, `floor_mod`, `trunc_div`, `trunc_rem`
  (`?i64`: None on a zero divisor; `MIN / -1` wraps), `floor_div_f64`,
  `floor_mod_f64`, `add_overflow` / `sub_overflow` / `mul_overflow`
  returning `(i64, bool)`, and `to_i64_exact(f64) -> ?i64`.

## `std.bigint`

`std/bigint.lm`: arbitrary-precision integers with Python's semantics
(floor and truncating division, infinite two's complement for bitwise
operations on negative numbers, correctly rounded conversion to `f64`,
Python's `hash`). Tested by `tests/test/bigint.lm` (also products, divisions, roots
and conversions at every threshold below) and by a differential
test against CPython (`tests/bigint/diff.py`, run by `cargo test`
as `bigint_vs_python`: 3000 random cases, and 1000 more collecting
between the steps of the GC operations; the cases include products,
squares, divisions with all-ones quotients, square roots and parsing
past every threshold; longer runs by hand, `--big` for thousands of
limbs).

**Representation.** A magnitude of 64-bit limbs, least significant
first, normalized (no zero limb at the top; zero has no limbs), and a
sign. Limbs are `u64` because `a.mul_wide(b)` and `hi.div_wide(lo, d)`
(`std.core`) give the 128-bit products and quotients that 64-bit limbs
need; that makes multiplication about four times the work per
instruction of 32-bit limbs (CPython's are 30 bits). The base cases are
carry chains on `add_carry`/`sub_borrow` and `mul_add_wide` (`std.core`,
"Carry chains"): `adc`/`sbb` on x86-64, `adcs`/`sbcs` on AArch64.

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
  `from_u64` (one limb), `gcd_into(out, a, b)` (`gcd_size`; never
  negative, Euclid's algorithm on limbs and then on machine words),
  `isqrt_into(out, a) -> ?Num` (`isqrt_size`; None for a negative a;
  the floor of the root),
  `from_f64_into` (`f64_size`, truncates; None
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
  `bit_or`, `bit_xor`, `bit_not`, `gcd`, `isqrt -> ?BigInt`, `copy`, `of_i64`, `of_u64`, `of_f64`,
  `parse(s, radix) -> ?BigInt`) take views, so owned and GC numbers mix;
  `x.num()` is an owned number's view, `x.free()` releases it.
- **`Big`, in the GC heap**: `object Big { neg: bool, len: u64, d: [u64] }`
  (`len` limbs of `d` are the number: results are allocated at their
  size bound). `gc_add(h, a, b) -> Gc<Big>` (and `gc_sub`, `gc_mul`,
  `gc_divmod`, `gc_pow`, `gc_shl`, `gc_shr`, `gc_and`, `gc_or`,
  `gc_xor`, `gc_not`, `gc_neg`, `gc_gcd`, `gc_isqrt -> ?Gc<Big>`, `gc_copy`, `gc_of_i64`, `gc_from_f64`,
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

**Algorithms.** The thresholds are `pub const`s, in limbs, measured on
the development machine (2026-11-03, best of 5 of each size on both
sides of the candidate; within 5% over a wide range in most cases, and
the values picked from the middle of it; `KARATSUBA`,
`KARATSUBA_SQR` and `DIV_DC` retuned after the carry-chain base cases,
best of 3 interleaved runs of 7 batches):

| operation | algorithm | from |
|---|---|---|
| product | schoolbook, row by row, two rows a pass (`mul_1`, then `addmul_2`: each pass loads and stores the result once for two limbs of the shorter operand; its loop two limbs a step, so the exit stays predicted). Faster than summing column by column (Comba, used before) at every size: 0-26% at 14-47 limbs | |
| | Karatsuba | `KARATSUBA` = 22 |
| | Toom-3 (evaluation at 0, 1, -1, -2, ∞; Bodrato's interpolation; on owned temporaries) | `TOOM3` = 250 |
| | unbalanced: cut into balanced pieces of the shorter operand's size | |
| square (both operands the same limbs: `mul(x, x)`, `pow`, and the algorithms below) | schoolbook with each cross product once, two rows a pass (`addmul_2` after the rows' corner product), then the doubling and the diagonal squares in one pass; 5-19% faster than Comba | |
| | Karatsuba squaring (2 a0 a1 = a0² + a1² − (a1 − a0)²) | `KARATSUBA_SQR` = 40 |
| | Toom-3 squaring | `TOOM3_SQR` = 500 |
| division | schoolbook, each quotient limb from the remainder's top three limbs by the divisor's top two (Möller and Granlund's 3/2 division by a precomputed reciprocal, GMP's `mpn_sbpi1_div_qr`): exact but for a rare borrow, so no correction loop, and the row subtracted (`submul_1`, four limbs a step) is two limbs shorter than the divisor; Knuth's algorithm D for two-limb divisors | |
| | recursive divide and conquer (Burnikel-Ziegler, with GMP's `mpn_dcpi1_div_qr` structure: the quotient's high half from the top halves, recursively, the product of that half by the divisor's low half subtracted, at most a few corrections, then the low half; a long quotient in blocks of the divisor's length): about two products of the same size | `DIV_DC` = 40 (divisor and quotient) |
| one-limb divisor | Möller-Granlund, one reciprocal for the whole number (the reciprocal from a 256-entry table and Newton steps, GMP's `invert_limb`: multiplications only; `div_wide` is a `div` instruction, 40-90 cycles on Skylake) | |
| square root | Zimmermann's Karatsuba square root (GMP's `mpn_dc_sqrtrem`): the root and remainder of the top half recursively, then one division of about n/2 by n/4 limbs and one square of n/4, one correction; on one limb a floating-point estimate and a Newton step (in floating point, on the exact 128-bit a - s²: no division). `isqrt` needs only the root: at the top level the correction's sign comes from the top limbs of u and q (all but about 2^-62 of the time) instead of the square and the remainder | |
| power | left-to-right binary (squarings) | |
| formatting, radix 2^k | bit extraction | |
| formatting, other radixes | dividing out the largest power of the radix that fits a limb, a limb's worth of digits at a time (constant division for radix 10) | |
| | divide and conquer: split by radix^(k·2^i) (the precomputed squares) with the fast division, halves recursively, the low ones zero-padded | `DC_LIMBS` = 16 |
| parsing | a limb's worth of digits at a time (one `mul_1` pass per chunk: quadratic but cheap per step) | |
| | divide and conquer: the high digits times radix^(k·2^i) plus the low k·2^i digits, recursively | `PARSE_DC` = 512 limbs' worth of digits |

With these, every operation but `gcd` is O(M(n) log n) or better, with
M(n) Karatsuba's or Toom-3's cost.

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
`bench/bigint/bench.py` on CPython 3.13, best of 5, one machine,
2026-11-03; "subquadratic": before the carry-chain base cases;
"schoolbook": before that, with schoolbook division and radix
conversion, Karatsuba from 32 limbs, no squaring routine):

| | std.bigint | subquadratic | schoolbook | CPython 3.13 |
|---|---|---|---|---|
| factorial(5000), a running product | 1.75 ms | 1.75 ms | 1.8 ms | 4.8 ms |
| fib(100000) by fast doubling | 0.30 ms | 0.41 ms | 0.58 ms | 1.7 ms |
| `str` of a 99,722-digit number | 6.3 ms | 8.0 ms | 19 ms | 32 ms |
| `int` of it | 3.9 ms | 5.0 ms | 9.5 ms | 17 ms |
| product, 99,722 by 20,899 digits | 1.2 ms | 1.65 ms | 2.0 ms | 7.5 ms |
| `divmod`, 120,621 by 20,899 digits | 2.4 ms | 3.2 ms | 6.4 ms | 15 ms |

Larger numbers (7^e, best of 3, before the carry-chain base cases,
which take another 15-25% off at these sizes; CPython 3.13 converts big
numbers with the subquadratic pure-Python `_pylong`, and divides big
numbers with it too):

| digits | | str | int | divmod by half size | isqrt |
|---|---|---|---|---|---|
| 99,722 | std.bigint | 8.1 ms | 5.1 ms | 2.6 ms | 1.8 ms |
| | before | 19 ms | 9.6 ms | 7.5 ms | 6.8 ms |
| | CPython | 32 ms | 17 ms | 12 ms | 16 ms |
| 299,165 | std.bigint | 44 ms | 25 ms | 15 ms | 10 ms |
| | before | 147 ms | 85 ms | 68 ms | 55 ms |
| | CPython | 86 ms | 99 ms | 72 ms | 95 ms |
| 997,216 | std.bigint | 284 ms | 156 ms | 91 ms | 65 ms |
| | before | 1566 ms | 963 ms | 764 ms | 588 ms |
| | CPython | 365 ms | 666 ms | 478 ms | 635 ms |

At the sizes of examples/scheme's pi benchmark (ns per operation,
n by n limbs, a quotient 2n by n, a root of 2n limbs; before → after):

| limbs | product | square | quotient | isqrt | `write` decimal |
|---|---|---|---|---|---|
| 8 | 85 → 87 | 84 → 75 | 372 → 225 | 1422 → 496 | 1843 → 821 |
| 26 | 760 → 651 | 911 → 408 | 1852 → 1091 | 4094 → 1379 | 11350 → 3534 |
| 52 | 2653 → 2254 | 2632 → 1505 | 4998 → 3761 | 9636 → 3249 | 20533 → 8050 |
| 100 | 8267 → 7105 | 8288 → 4610 | 14850 → 12108 | 24816 → 8260 | 41952 → 19066 |
| 1000 | 354 → 284 us | 358 → 203 us | 1152 → 573 us | 1171 → 374 us | 1040 → 603 us |

The carry-chain base cases (`add_carry`, `sub_borrow`, `mul_add_wide`;
rows two at a time instead of columns; thresholds retuned), ns per
operation through the `*_into` functions (into caller buffers, random
limbs), before → after, and GMP 6.3's `mpn` functions on the same
operands in parentheses (`mpn_add_n`, `mpn_mul_n`, `mpn_sqr`,
`mpn_tdiv_qr`, `mpn_sqrtrem`; best of 5 batches, best of 3 interleaved
runs, a 3.4-4 GHz Skylake):

| limbs | sum | product | square | quotient 2n by n | isqrt of 2n |
|---|---|---|---|---|---|
| 8 | 8 → 5 (4) | 76 → 60 (41) | 63 → 45 (31) | 202 → 201 (125) | 486 → 451 (194) |
| 26 | 31 → 13 (9) | 635 → 515 (425) | 388 → 316 (242) | 1100 → 1003 (682) | 1406 → 1277 (704) |
| 52 | 70 → 23 (17) | 2282 → 1714 (1346) | 1518 → 1055 (784) | 3724 → 3229 (2286) | 3091 → 2751 (1719) |
| 100 | 134 → 50 (30) | 7110 → 5205 (3476) | 4671 → 3195 (2283) | 11533 → 9630 (6753) | 8234 → 7130 (4662) |
| 1000 | 1354 → 475 (325) | 286 → 200 us (109 us) | 191 → 132 us (73 us) | 560 → 427 us (261 us) | 375 → 288 us (182 us) |

Sums are 2.7-3x faster (an `adc` chain four limbs a step: about 1.7
cycles a limb against GMP's 1.2), products and squares 1.2-1.45x (the
base case does a limb product in about 0.8 ns, 3 cycles; GMP's x86-64
assembly uses `mulx` and the two carry chains of `adcx`/`adox`, which
need BMI2 and ADX, beyond the baseline x86-64 that Loimi targets). Quotients of up to 16 limbs and decimal formatting are
unchanged: their time is the quotient-limb estimates and the per-call
work, not carries.

Division, square root and the GC layer, again (2026-11-04: the 3/2
quotient limbs, the table reciprocal, `submul_1` four limbs a step, the
root without the remainder, one allocation per division and per root,
`gc_bin` inlined): ns per operation through the `*_into` functions,
before → after, GMP 6.3's `mpz` functions in parentheses (`mpz_tdiv_qr`,
`mpz_sqrt`, `mpz_get_str`; best of 5 batches, best of 5 interleaved
runs):

| limbs | quotient 2n by n | isqrt of 2n | `write` decimal of n |
|---|---|---|---|
| 4 | 127 → 99 (90) | 295 → 183 (116) | 374 → 298 (182) |
| 8 | 209 → 166 (128) | 490 → 360 (247) | 785 → 698 (399) |
| 13 | 368 → 317 (251) | 727 → 528 (323) | 1341 → 1182 (751) |
| 26 | 1142 → 952 (800) | 1316 → 957 (574) | 3397 → 3050 (1862) |
| 52 | 3593 → 3089 (2594) | 3047 → 2046 (1312) | 7846 → 6837 (4382) |
| 104 | 10955 → 9976 (7906) | 8187 → 5583 (3331) | 19436 → 17449 (10499) |

Products and squares are unchanged (1.2-1.45x GMP's). In the Scheme
example, pi took 0.84 of its time and chudnovsky 0.89 (mostly
one-limb operands: the GC layer's per-call work); against Guile 3's VM
(GMP), pi 1.31x -> 1.10x wall and chudnovsky 0.84x -> 0.74x (best of 9,
interleaved, on a shared machine).

The base cases are carry chains in plain Loimi, 1.2-1.8x behind GMP's
assembly on sums, products and squares (table above); quotients of a
few limbs are 1.6-3.5x behind (the quotient-limb estimates and per-call
work). Each operation returns a new `BigInt`.

## `std.io` and `std.os`

`std/io.lm`: `print(s)`, `println(s)`, `print_int(i64)`, `print_f64(f64)`
(`%.14g`), `print_bool(b)`, their stderr forms `eprint`, `eprintln`,
`eprint_int`, `eprint_f64`, and `read_file(path) -> ?str` / `free_file(s)`
(`#[raw]`: `s` must come from `read_file`).
These write with one unbuffered `write(2)` per call.

`io.Out` is buffered output: `io.Out.stdout()` (64 KiB) or
`io.Out.new(fd, cap)`, then `o.print(s)`, `println`, `byte(c)`,
`print_int`, `print_f64`, `print_bool`; the buffer is written out with
one `write(2)` when it would overflow (a string larger than the buffer
is written directly after it), and by `o.flush()` and `o.free()` (flush
and release). There is no flush at exit: flush before the program ends,
and before writing to the same terminal unbuffered (an `io.eprint`
error message), or the output comes out of order. A program that
prints a lot (an interpreter's `print` builtin) gains one system call
per 64 KiB instead of one per call.

`std/os.lm`: `arg_count() -> u64`, `arg(i) -> ?str` (0 is the program),
`quit(code) -> never`.

## `std.lex`

`std/lex.lm`. The module is a `Cursor` toolkit, and a lexer is a short function that tries
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

### Run-time operator tables: `parse.Ops`

`Pratt` and `Parser` index their tables by a token-kind enum fixed
when the parser is built. Prolog-family languages define operators
while parsing (`:- op(700, xfx, ===)`), name them by atoms, and give each
a priority (1..1200, larger binds looser) and one of seven types. `Ops`
is that table and the operator-precedence driver for it:

- `Ops.new()`, `free()`; `add(name, prio, typ)` with `name: u64` (an
  interned atom id, below 2^62) and `typ: parse.OpType` (`XFX`, `XFY`,
  `YFX`, `FY`, `FX`, `XF`, `YF`; `parse.op_type("xfx")` and
  `t.name()` convert): replaces the definition of the same kind, and
  priority 0 removes it, as `op/3` does. A name has at most one prefix,
  one infix and one postfix definition, independently (`-` is `fy 200`
  and `yfx 500`).
- Lookups: `prefix(name)`, `infix(name)`, `postfix(name)` return
  `?parse.Op { prio, typ, left, right }`, where `left`/`right` are the
  highest priorities the arguments may have (`p - 1` on an `x` side, `p`
  on a `y` side); `is_op(name)`, `max_prio(name)`; `next(&at)` iterates
  (`current_op/3`).
- The driver: the parser state implements
  `parse.OpHooks<E, X>` (`impl parse.OpHooks<Term, Err> for Reader;`)
  and calls `p.op_parse(&ops, max)` for a term of priority at most
  `max` (returns `(E, priority)`) or `p.op_parse_arg(&ops)` for an
  argument. The hooks: `peek(p, prefix) -> parse.Tok` classifies the
  current token, `advance`, `primary` (any term that is not an operator
  application: numbers, variables, strings, compound terms, lists,
  brackets; it recurses with `op_parse`), `atom(name)` (an operator name
  as an atom), and `prefix`, `infix`, `postfix` to build applications.
  `Tok` is `Name(u64)` (may be an operator), `Sep(u64)` (`,` and `|`:
  infix operators that also separate arguments), `Term` (starts a term
  that is not an operator application) or `End`.
- The driver implements the rules that make Prolog's operator grammar
  hard to get right: argument priorities by type; a prefix operator is
  an atom when the next token cannot start a term or is an infix
  operator that is not also prefix (`- = x`, `f(-)`, `[-|T]`, `- , a`);
  an operator as an atom has its priority (0 above `max`, so `f(:-)`
  parses); a prefix operator above `max` is read at 999 (`X = \+a`);
  `op_parse_arg` is priority 999 but accepts infix operators above 999
  other than the `Sep`s (`f(a :- b)`), as SWI-Prolog does (use
  `op_parse(&ops, 999)` for ISO's strict rule).
- Token-level distinctions stay in `peek`, which is told whether a term
  starts at the token (`prefix`): there a name followed directly by `(`
  (functional notation `-(1)`) and a sign followed directly by a number
  (`-1`, a negative literal; `- 1` is `-(1)`) are `Tok.Term`, while after
  a term the same tokens are infix operators (`X-1`, `a -(b)`).
- `Ops.parse<P, E, X>(p, max)` and `parse_arg<P, E, X>(p)` are the same
  driver called with explicit type arguments (the `OpHooks` methods
  exist because `E` and `X` are not inferred from `P`'s `impl`).

The Prolog reader (`examples/prolog/reader.lm`) parses with it;
`tests/run/std_parse_ops.lm` is a small Prolog-term reader.

`Span` and `SourceMap` live in `std.diag`.

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

## `std.debug`: guest debuggers

`std/debug.lm`: the generic parts of a source-level debugger for a vm
with `on break` (language.md §9.1; design in
[`design/debugging.md`](design/debugging.md)). Mini's and the Python
subset's `--cfg debug` builds (`examples/mini/debugger.lm`,
`examples/python/debugger.lm`) are two on top of it.

- `Breakpoints<B>`, over the vm's `Vm.Break`:
  `Breakpoints.new(Vm.clear_break, rearm)` (the vm's functions, as
  `std.bc` takes `op_size`); `add(file, line, temp) -> id` (ids from 1),
  `add_site(id, at, tok)` (a word `Vm.set_break` patched), `bp(id)`,
  `list()`, `has_site(at)`, `delete(id)` (puts the instructions back),
  `clear_all`, `free`. `Bp` has `id`, `file`, `line`, `nsites`, `hits`,
  `temp` (deleted at its first hit).
- The step-over: `hit(at) -> id`, from the vm's `on break` handler at
  `pc`: puts the instruction back and remembers it; `tick() -> bool`,
  from the wrapper of a variant the handler switches to before running
  the instruction (`jump pc`): sets the breakpoint again once execution
  has left it, and says whether the variant must stay on.
- `Stepper`: `Mode` is `Run`, `Step`, `Next` or `Finish`;
  `start(mode, line, depth)`, `active()`, `should_stop(line, depth)`
  (step: another line or another depth; next: another line at this
  depth, or a return; finish: a return). The interpreter supplies each
  instruction's line and call depth; line 0 is "no line" (an
  instruction such as a loop's jump back, where no command stops).
  `left(depth)`: the frame at `depth` returned, yielded or raised to the
  native that started its nested run, so the next instruction may be a
  new frame at the same depth (a sort key called again): `step` stops
  there, `next` and `finish` wait for the caller.
- Line starts: `line_sites(words, lines, line, Vm.base_size, Vm.label,
  &out)`: the words where a line starts in one code object (the first
  word of each run of the line, or the run's *loop head*, the target of
  a backward jump from another line, which runs on every iteration).
  `unpatched(&bps, words, word, &out)`: a copy of `words` with each
  breakpoint word replaced by the word it stands for (`word` is
  `Break.word`), for walks that read labels or operands, which see a
  breakpoint word as the breakpoint op.
- For front ends that embed the debugger (an editor's, through a C API
  and the Debug Adapter Protocol; Mini's `mini_debug_*`,
  design/debugging.md §9): `Bp.cond` (an expression the interpreter
  evaluates at each hit) and `Bp.hitc`, set with
  `set_condition(id, cond, hitc)`; `counts(id) -> bool` counts a hit
  whose condition held and checks the hit condition (`HitCond`, from
  `parse_hit_condition("N" | "== N" | ">= N" | "> N" | "<= N" | "< N" |
  "% N" | "")`). `suspend()`/`unsuspend()`/`suspended()`: take the
  breakpoints out of the code while the debugger runs guest code from a
  stop (a condition, an expression), leaving the pending step-over
  alone; `tick` does nothing meanwhile. `Reason` (`Entry`,
  `Breakpoint`, `Step`, `Pause`, `Error`). `Refs<T>`: handles (from 1)
  for what a client expands lazily (`add(x, same)`, `get(id)`,
  `items()`, `clear()`). `request_pause(&flag)` (any thread, a relaxed
  atomic store) and `take_pause(&flag)`, for a pause flag the
  interpreter polls at back-edges and calls.
- Commands: `parse_command(line) -> Command` (`Break(file, line)`,
  `TBreak`, `BreakIf(file, line, cond)` for `break LINE if COND`,
  `Delete(id)`, `Continue`, `Step`, `Next`, `Finish`,
  `Backtrace`, `Locals`, `Print(name)`, `Op`, `Info`, `Quit`, `Help`,
  `Empty`, `Bad(msg)`, with gdb's short forms), `HELP`,
  `parse_location("[FILE:]LINE")`, `basename(path)`.
- `Input.stdin()`: `read_line() -> ?str` (unbuffered `read(2)`, so a
  script of commands can follow the program's own input), `is_tty()`
  (a scripted session echoes its commands), `free`.

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
  `data: *T` is any pointer and `scan: fn(*Heap, *T)` calls `gc_mark(h,
  v)` on each reference. Scanners have no effects: they cannot collect.
  A context struct declared `#[trace]` gets `ctx.trace(h)`, which marks
  its fields' references for the scanner (language.md section 8,
  "Root scanners and `#[trace] struct`").
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
  of host memory it owns), `heap.on_free(obj, f)` / `heap.on_free_gc(obj, f)` (release host memory
  when `obj` dies; no guest code, no resurrection), and
  `heap.pin_dirty(obj)` / `unpin_dirty` / `is_pinned_dirty` (an object
  written without barriers, traced at every collection).
- `loimi build --gc-check-roots` validates every reference the collector
  marks and poisons unassigned slots and freed objects
  ([details](stdlib/gc.md#checking-roots---gc-check-roots)).
- Threads: heaps have no shared state, so independent heaps run on
  different threads without locks ([`stdlib/gc.md`](stdlib/gc.md#threads-sharing-a-heap-stdgil)).
  Threads sharing a heap, one at a time: `std.gil` (`gil.Gil.new(&heap,
  data)`, `attach`, `detach`, `unlock`/`lock`, the poll `waiting()` and
  `yield_now()`, hooks that move per-thread interpreter state); see
  [`stdlib/gc.md`](stdlib/gc.md#threads-sharing-a-heap-stdgil).
  `gc.mark_shadow_stack(h, top)` marks another thread's saved roots.

Every object has a 16-byte header (size, type id, mark, flags, and the
trailing-array length, which compiled code reads at object - 8), so a
two-word object such as a Lisp pair takes 32 bytes. Small objects (up
to 512 bytes with the header) live in 64 KiB chunks of one size class
each; marking sets a bit per 16-byte granule in the chunk's bitmap, and
the allocator bump-allocates through the runs of clear bits, so nothing
is swept ([details](stdlib/gc.md#heap-layout)).

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

Limitations (details in [`stdlib/gc.md`](stdlib/gc.md#limitations)): the
heap never moves objects; there is no card marking (a large array stored
into is retraced whole by each minor collection) and no incremental
marking; small objects are promoted at their first survival; finalizers
cannot run guest code (free hooks release host memory).

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

Limitations (details in [`stdlib/obj.md`](stdlib/obj.md#limitations)):
no integer-indexed element storage and no global property cells (both
are the interpreter's).

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

### Value stacks: `rt.ValueStack<T>`

A register vm's root scanner marks every register of every frame's
window, so a register the callee has not written yet must not hold a
pointer to a freed object. Clearing the window on every call does that,
at a cost (Lox's fib: 13% of its time). `rt.ValueStack<T>` keeps a
*high-water mark* `s.hw` instead, with three regions:

- below `s.hw`, every slot holds a value the scanner may read: a live
  register, or a stale one whose object no collection has freed;
- from `s.hw` up to `s.fresh`, slots hold values that refer to no freed
  object (cleared ones);
- from `s.fresh` up, slots were never written.

The operations:

- `s.fits(top) -> bool`: one compare, `top <= s.hw`. A call checks it in
  place of the compare against the stack's end.
- `s.raise(top) -> bool` (`#[cold]`): false past the end; else it clears
  the never-written slots below `top` (and `rt.SLACK`, 256, more) and
  raises the mark to `s.fresh`. After a collection lowered the mark,
  raising it again clears nothing.
- `s.ensure(top) -> bool`: `fits`, else `raise`, for slow paths. In a
  handler's hot path, call `raise` from a `cold` block that ends in
  `redo` instead: a call to it on the hot path would give the handler a
  stack frame (Scheme's calls were 3–8% slower that way).
- `s.lower(top)`: for the root scanner, after it marked the live
  windows, with `top` the end of the live windows. It clears the stale
  slots from there to the mark, because this collection may free what
  they point to, and lowers the mark.

So calls clear nothing. The interpreter must write every register
before it reads it (a compiler that allocates registers to locals and
temporaries does). The scanner's `top` must cover every live window. A
callee's window can end below its caller's, so take the highest end
over all frames (Lox, Scheme), or have the compiler put a call's window
above all of the caller's live registers (MiniJS). A value written
above the mark (arguments set up before the call raises it) keeps its
value through `raise`, but the mark must cover it before the next
collection.

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

`rt.ValueStack.new(n, fill)` allocates `n` slots (`mem.alloc` zeroes
them, so untouched pages cost nothing). `fill` is what cleared slots
hold, a value the collector ignores (nil). The stack does not move.

Lox, Scheme and MiniJS use it; Python keeps its own variant, which
never lowers the mark and marks every slot below it. A vm with
coroutines needs one stack per coroutine, and the scanner must lower
the mark of each one; nested runs must `ensure` their windows. Mini's
hand-written attempt at all that measured no gain
([design/speed-audit.md](design/speed-audit.md), item 9).

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
