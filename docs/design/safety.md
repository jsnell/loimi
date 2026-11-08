# Design note: compile-time memory safety for the standard library

Status: **built** (`#![safe]` modules, `#[raw]` functions, the GC-object
rules of section 8). Written 2026-11-03.

> Raw pointers (`*T`) need no `unsafe` anywhere (language.md section
> 1.3), and the standard library is built on them. Can the compiler
> check memory safety in std, module by module, and at what cost?

**Answer.** Mostly, with one deliberate hole. A module that starts with
`#![safe]` may still use `*T`, but only as a *reference to one live
value*: following it is allowed, making it point anywhere else is not.
Everything that can reach memory outside the one value (indexing,
arithmetic, `p.slice(n)`, casts to pointers, another module's buffer
pointers, C calls, functions marked `#[raw]`) needs `unsafe { }`. What
this rule does not check is *lifetime*: the pointer-stability check
(`lifetimes.md`) rejects a `*T` or slice used after its container may
have moved, within a function and through its calls, but a pointer
stored in memory and loaded elsewhere is unchecked (section 2); built
with `--sanitize=address`, std's containers poison the memory they keep
but no longer use, so most such uses are reported at run time (section
7). GC objects have three rules of their own, in every module (section
8): a slice of an object's traced trailing array cannot be kept where a
store through it would skip the write barrier, a trailing array starts
zero-filled only where zero is a valid element, and the objects of a
`#![safe]` module are built and written only by that module.
Nine std modules (`lex`, `parse`, `diag`, `text`, `hash`, `io`, `bc`,
`map`, `dict`) are safe (sections 4 and 5); the others (`mem`, `gc`,
`obj`, `bigint`, `gil`, `num`, `math`, `os`, `rt`, `test`, `prof`,
`debug`) are not. It is a check only: no example's IR changed from
adding `#![safe]` and `#[raw]`, and the migrations' own IR changes
(slices instead of pointer arithmetic, so bounds checks) are measured
in section 6.

## 1. The rule

In a module marked `#![safe]` (an attribute at the top of the file),
outside `unsafe { }`:

| allowed | needs `unsafe { }` |
|---|---|
| `*p`, `p.f`, `p.read_unaligned()`, `p.atomic_*()` on a `*T` | `p[i]`, `p[a..b]`, `p + n`, `p - n`, `p += n`, `p.offset(n)` |
| `&place` (a local, a field, an element: `&a[i]` is bounds-checked) | `p.slice(n)` (a slice of unchecked length) |
| `new(&arena, v)`, `Vec`, slices, `?*T` | casts to a pointer: from an integer (`0 as *T`), another pointer type, a function or a `Gc` |
| passing, storing, comparing pointers; `p as u64` | casts that forge a tag or a reference: integer to `enum` or `value` |
| methods of other modules' structs; reading their non-pointer fields | reading a raw-pointer field of *another module's* struct (`v.ptr`), or a slice's `.ptr` |
| | writing any field of another module's struct that has raw-pointer fields (`v.len = 0`), `&v.len`, mutating methods on such a field (`a.chunks.push(..)`), or building one with a literal |
| this module's enums with pointer payloads | binding a pointer payload of *another module's* enum or `value` type in a pattern (`lib.Buf.Bytes(p, _)`; `_` is fine), or building such a variant |
| | `..zero` or a `var` without initializer that makes a null `*T` (use `?*T`), also inside arrays, tuples and structs, and null `fn`s and `Gc`s |
| `new(&arena, v)` | `new(&a, v)` with an allocator whose `alloc_bytes` is not `#[raw]` (nothing promises it returns `size` bytes) |
| | declaring a `vm` (its handlers are exempt: put it in another module) |
| | `assume(c)` and `unreachable` (undefined behavior if wrong; `trap()` is fine) |
| | calling (or naming) an `extern "C"` function or a function marked `#[raw]` |
| | calling an `#[unsafe]` function (as everywhere) |

Each error says what to write instead:

```
error: indexing a raw pointer (`p[i]`) is a raw-pointer operation: not allowed in a `#![safe]` module outside `unsafe { }`
  --> lexer.lm:12:5
note: use a slice (`[]T`, bounds-checked), a `Vec` method, or wrap it in `unsafe { }` with a comment saying why the index is in bounds
```

**Why a `*T` stays usable.** Loimi has no separate reference type:
methods take `self: *T`, `&x` is a `*T`, `new(a, v)` returns one, and
`Vec.ptr_at(i)` hands out one. Banning `*T` would ban methods. What
makes a pointer dangerous is *where it points*, and safe code cannot
change that: every `*T` it holds came from `&place`, `new(..)`, or a
function, and pointer arithmetic, casts and slicing a pointer are the
ways to move it. So `*T` in a safe module means what `&mut T` means in
Rust, minus the borrow checker.

**`#[raw]`: the trust boundary for functions.** A function in a module
that is not `#![safe]` is trusted: safe code may call it with any
argument it can construct. That is sound only if the function is
sound for every such argument: pointers to one value. A function that
reads or writes beyond that (`mem.copy(dst, src, n)`), that frees
(`mem.release(p)`), that returns memory that is not one valid value
(`mem.alloc<T>(n)`: n zeroed `T`s, maybe not valid `T`s), or that
requires a pointer from a particular allocator (`Malloc.free_bytes`), or
that trusts an index or cache word without checking it (`obj`'s inline-cache
readers: `Model.load_ref(o, r)`) is
marked `#[raw]`. A `#[raw]` function is an ordinary function in modules
that are not safe (no change for the example interpreters), and needs
`unsafe { }` in a safe module, as `p[i]` does. Extern functions are
`#[raw]` implicitly. `new(a, v)` calls `a.alloc_bytes` (which is
`#[raw]`) itself, with the right size: allowed.

**Other modules' structs.** A struct with a raw-pointer field declared
in another module carries that module's invariants (`Vec`: `ptr` holds
`cap` values, `len <= cap`); its own `unsafe` blocks rely on them. A
safe module may read its non-pointer fields (`v.len`) and call its
methods, but not write its fields, take their address, read its
pointers or build one with a literal. "Raw-pointer field" means a field
whose type holds a `*T` by value (`*T`, `?*T`, and tuples, arrays and
options of them, and function types taking or returning one), and "a
struct with raw-pointer fields" is transitive through struct and enum
fields (a struct holding a `Vec` is one). Copying a whole value is allowed
(that is the aliasing part of the lifetime hole, section 2). This holds
for structs of other *safe* modules too: their `unsafe` blocks rely on
their invariants.

**Exempt:** vm code (handlers, vm functions: their operand accesses are
unchecked because the verifier ran, section 6 of language.md), code
generated from a `vm` declaration, and every module without `#![safe]`.
Since handler bodies are exempt, a `#![safe]` module cannot declare a
`vm` at all.
Generic functions are checked as their defining module says: a safe
module's generic body is checked; `mem.Vec.push` instantiated from a
safe module is not (it is `mem`'s code).

**`unsafe { }` style.** Each block gets a one-line comment saying why
it is in bounds or valid:

```loimi
// unsafe: write(2) reads the s.len bytes of `s`
unsafe { write(fd, s.ptr, s.len) }
```

## 2. What the rule does not check (the known holes)

1. **Lifetimes.** A `*T` or slice into a container is valid until the
   container changes: `Vec.items()`, `Vec.ptr_at(i)`, `HashMap.get_ptr`,
   `OrderedMap.entry(e)`/`next`, `diag.SourceMap.file(id)`,
   `diag.Diagnostics.at(i)` all dangle after a push, insert, resize or
   free; `&local` returned from its function dangles; `Arena.free`
   invalidates everything `new(&arena, ..)` made; a `Vec` is a plain
   struct, so a copy of one shares its buffer and dangles when the
   original grows. The rule for std's own APIs is: *a container's
   methods leave the container valid* (`Vec.free` leaves an empty
   `Vec`, `HashMap.free` an empty map), so only references obtained
   earlier can dangle. Uses within one function, through its calls, are
   compile errors in every module (pointer stability,
   `docs/design/lifetimes.md`, which also lists what that check does
   not cover); a pointer stored in memory and loaded elsewhere is not
   checked. Index- and callback-based APIs (`Vec.at(i)`, `OrderedMap`
   entry indices with `version`) are the safe alternative where they
   are cheap. Built with `--sanitize=address`, most stale uses are
   reported at run time (section 7).
2. **GC references in host memory.** A `Gc<T>` kept somewhere the
   collector does not scan (a `Vec<Gc<T>>` that is not a root) dangles
   after a collection. `gc.Heap` roots and `--gc-check-roots` are the
   tools.
3. **Trusted modules.** A `#![safe]` module is only as safe as the
   functions it calls in modules that are not: those must honor the
   `#[raw]` contract above. std's are audited (section 3); an example
   interpreter's are not.
4. **Uninitialized and zero values from elsewhere.** `..zero` cannot make
   a null `*T` in a safe module, but a non-safe module can hand one over
   (a struct it zeroed). A null `*T` faults deterministically on
   x86-64/AArch64 Linux, but LLVM treats the load as undefined.
5. **Data races.** Not addressed (`std.gil` serializes guest threads;
   atomics are allowed on any `*T`).
6. **Invariants without raw pointers.** The field rule protects structs
   that hold a raw pointer (directly or through a field of struct, enum,
   tuple, array or option type). Another module's struct whose fields
   are only slices and integers can be written field by field; a module
   whose `unsafe` code relies on an invariant between such fields (an
   index field below a slice's length, read through `.ptr`) must keep
   them private behind a raw pointer or check them. No std module does.
7. **Asserted contracts.** `#[raw]` on a safe module's own function is
   an unchecked claim, like Rust's `unsafe impl`: `new(&a, v)` trusts an
   `alloc_bytes` marked `#[raw]` to return `size` bytes, and its body is
   checked like any other but nothing checks that promise.
8. **Conservative cases.** A generic struct or enum of another module
   instantiated with the safe module's own pointer type (`lib.Box<*u8>`)
   counts as foreign: reading `b.v` is flagged though the pointer came
   from the safe module. A struct of the safe module that reaches it
   through another module's generic type (`lib.Lookup<Mine>` bound in a
   pattern) is not followed: its pointer fields are its own module's.
9. **GC objects** (section 8 covers stores through slices, zero-filled
   trailing arrays and who may write an object). A module that is not
   `#![safe]` can store through `&o.f` or `o.items.ptr` (raw pointers:
   `gc.write_barrier` is its job), and a slice of an object's trailing
   array that is read across a collection dangles if nothing else keeps
   that object alive (the lifetime hole, item 1, for the GC heap).

Section 3a lists the bypasses the rule closes (Loimi has no struct
patterns, so `let Vec { ptr, .. } = v` does not parse, but tuple fields,
arrays of pointers and enum payloads were real bypasses).

## 3. Audit of std

`LOIMI_SAFETY_AUDIT=1 loimi build ..` checks every module as if it were
`#![safe]` and prints the raw-pointer sites per module and kind
(`LOIMI_SAFETY_AUDIT=sites` lists each site). Run on std before the
migration, it split std into a periphery that was already (nearly)
pointer-free (`lex`, `parse`, `diag`: 0 sites; `hash` 1, `bc` 11,
`text` 16, `io` 33) and an unsafe core where pointers are the point
(`mem` 68, `gc` 147, `obj` 48, `bigint` 295, `gil` 34, `map` 27, `dict`
41). Most `*T` uses everywhere are `p.f` through a pointer to a struct,
which the rule allows.

Use-after-free bugs it found, all fixed: `StrBuf.push_str(b.as_str())`
(a buffer appending its own text) read freed memory when the push grew
the buffer (now `Vec.extend`, section 4); `HashMap.free` and
`OrderedMap.free` kept the capacity of the table they released (section
5).

## 3a. Adversarial pass (closing bypasses)

Programs that try to reach raw memory in a `#![safe]` module without
`unsafe`; each is rejected, and each is a `tests/fail/safe_*.lm`. "Was"
is the compiler before this pass.

| attempt | was | now |
|---|---|---|
| `let (p, _) = h.tup` / `h.tup.0` on another module's `(*u8, u64)` field | allowed | rejected: a field whose type holds a pointer through a tuple, array, option or function type is a pointer field |
| `h.arr[0]` on `[*u8; 2]` (e.g. `gc.Heap.free`) | allowed | rejected (same) |
| `let cb = h.cb; cb(&x)` on a `fn(*u8)` field (a callback expecting that module's pointers) | allowed | rejected (same) |
| `match b { lib.Buf.Bytes(p, _) => p }` (another module's enum or `value` type with a pointer payload) | allowed | rejected; `Bytes(_, n)` is fine |
| `lib.Buf.Bytes(&one_byte, 100)` (building such a variant) | allowed | rejected, like a struct literal |
| `o.inner = i` where `Outer { inner: Inner }`, `Inner { p: *u8 }` | allowed | rejected: "has raw-pointer fields" is transitive |
| `var p: *u8;` / `var s: S;` (zero-initialized: a null pointer) | allowed | rejected (also a null `fn`, `Gc`, through arrays, tuples, structs) |
| `S { ..zero }` with a `[*u8; 2]` or `(*u8, u8)` field | allowed | rejected (only a direct `*T` field was) |
| `new(&a, v)` / `new(&a, \|..\| ..)` with a safe module's allocator whose `alloc_bytes` returns `&one_byte` | allowed (heap overflow) | rejected unless `alloc_bytes` is `#[raw]` (`mem.Arena`, `mem.Malloc` are) |
| a `vm` declared in a `#![safe]` module (handler bodies are exempt) | allowed | rejected: declare it in a module of its own |
| `p[i]` in a closure, a `defer`, an `inline for` body, a generic function instantiated with `*u8` | rejected | rejected |
| `p as fn() -> u64`, `x as fn() -> u64`, `s as []u64` | rejected (no such casts) | — |
| `const P: *u8 = 0 as *u8` used; `var G: *u8 = 0 as *u8` | rejected | rejected (an unused constant is never checked, and makes no code) |
| calling `#[raw]` `alloc_bytes` through an unbounded generic (`fn al<X>(a: *X) { a.alloc_bytes(..) }`) | rejected | rejected (checked per instance) |
| `c.free(&malloc)` on a `closure fn` (frees through `Malloc.free_bytes`) | rejected | rejected |
| `x as V` for a `value` type | rejected | rejected |

None of the std safe modules needed a change for this pass, and no
example's IR changed (the checks only reject programs).

## 4. The safe modules and the `#[raw]` contracts

| module | `unsafe` blocks | `#[raw]` | what the migration changed |
|---|---:|---|---|
| `lex`, `parse`, `diag` | 0 | | `#![safe]` only |
| `hash` | 0 | | `str.eq` is `mem.equal_bytes` (also no `memcmp` on a null pointer for empty strings) |
| `text` | 2 | | `StrBuf.push_str` is `bytes.extend(s)`; `Interner`'s slot table is a `Vec<u64>` (bounds-checked `at`/`set`); `parse_f64` copies into its buffer by a checked loop; `snprintf`/`strtod` in `unsafe` |
| `io` | 4 | `free_file` | one `write_some(fd, s: []u8)` helper and one `format_g14` (the two C calls); `Out` buffers in a `Vec<u8>`; `read_file` (C stdio) is one `unsafe` block; `free_file` frees what `read_file` returned |
| `bc` | 0 | | `relax`'s position table is a `Vec<u64>` slice; `unreachable` → `trap()` |
| `map`, `dict` | section 5 | `map`'s `probe_*` core | section 5 |
| `mem` (not safe) | — | `alloc`, `try_alloc`, `grow`, `release`, `copy`, `sort_by`, `try_sort_by`, `Arena.alloc_bytes`, `Malloc.alloc_bytes`, `Malloc.free_bytes`, `poison`, `unpoison` | new `Vec.extend(s)` and `Vec.filled(n, x)` |
| `obj` (not safe) | — | the inline-cache API that trusts its cache words (slot refs, validity cells, Poly entry indices, `Hit`s): `Shapes.valid`, `Poly.add`, `MegaCache.new` (`n` a power of two), `put`, `free`, `Model.load_ref`, `store_ref`, `load_inline`, `load_out`, `proto_load`, `hit`, `poly_get`, `poly_get_at`, `poly_set`, `add_hit` | none: the rest of `obj`'s public API checks its indices (slices) or takes only values it made |
| `gc` (not safe) | — | `barrier`, `write_barrier` (they read the header before `obj`) | none |

The `unsafe` blocks of `text` and `io`, each with its one-line
justification:

```
text.lm  // unsafe: C calls on `buf`, whose size snprintf is given and which it NUL-terminates
text.lm  // unsafe: strtod reads the NUL-terminated copy in `buf` (s.len < 64) and points `end` into it
io.lm    // unsafe: write(2) reads the s.len bytes of `s`
io.lm    // unsafe: snprintf writes at most the 64 bytes it is told `buf` has
io.lm    // unsafe: C stdio on a NUL-terminated copy of `path`; fread fills at most the n bytes malloc'd
io.lm    // unsafe: the caller passes memory read_file malloc'd
```

All are C calls: these modules have no `unsafe` pointer arithmetic of
their own. No example call site changed.

**`Vec.extend(s)`** appends a slice. It is alias-safe: if `s` lies
inside the vector's own buffer and the buffer must grow, the cold path
re-derives `s`'s address in the new buffer before copying, so
`v.extend(v.items())` works. The fast path (room left) is a compare and
a `memcpy`.

**The modules that are not safe.** `mem`, `gc`, `obj`, `bigint` and
`gil` are the unsafe core (allocator, collector, inline caches, limb
arithmetic, threads), where nearly every function is pointer
manipulation; their public contracts are documented, with `#[raw]` on
the functions that safe callers cannot satisfy. `num`, `math`, `os`,
`rt`, `test`, `prof` and `debug` are small modules, mostly C interop.

## 5. `std.map` and `std.dict`

**`std.map` is `#![safe]`**; its `unsafe` blocks all rest on the
tables' own invariants (`slots` holds `cap` slots, `cap` a
power of two; an `OrderedMap`'s `idx` holds `mask + 1` slots naming
entries below `used`, `used <= cap <=` the allocation), which only
`map`'s own code can break since other modules cannot write the fields
of a struct with raw pointers. `HashMap` goes through one private
accessor, `slot(i)`, that masks the index (`m.slots + (i & (m.cap -
1))`: in bounds by construction; LLVM folds the second mask), plus
`new`, `grow` and `free`. The CPython-style probing core
(`probe_find`, `probe_slot_of`, `probe_lookup`, `probe_put`,
`probe_rebuild`, `probe_kill`, `probe_find_with`) takes a raw index
pointer, a mask and an entries pointer: it is `#[raw]`, one `unsafe`
body each, with the contract documented. `OrderedMap` has its
allocation, resize, probing methods, `clear` and `free` in `unsafe`
blocks and a bounds-checked entry accessor.

Two API contracts:

- `OrderedMap.entry(e)`, `set_value(e)` and `remove_at(e)` panic ("not
  a live entry index") unless `e` is a live entry (unchecked, they read
  and wrote out of bounds, and `remove_at` of a dead entry looped
  forever). No example uses `OrderedMap` on a hot path.
- `HashMap.free` and `OrderedMap.free` point the map at a shared zeroed
  sentinel table (a fresh table for slots over 512 bytes), which `grow`,
  `resize`, `clear` and `free` never write or release, so the map stays
  usable and the next insert allocates (`tests/run/map_free_reuse.lm`).

**`std.dict` is `#![safe]` too**, by a different argument. `Dict` and
`KeyDict` are GC objects holding `Gc`s and trailing arrays, not raw
pointers, so the field rule of section 1 does not protect them, and
dict's memory safety does not rest on its fields (`mask`, `used`, the
index slots):

- The probes are private safe functions over the trailing arrays as
  slices (`find_in`, `lookup_in`, `put_in`, `slot_of`, `rebuild_in`, and
  `key_*` ones for `KeyDict`), not map's `#[raw]` `probe_*` (which
  `OrderedMap` uses). Each takes its mask from the index's own length
  (`mask_of(idx)`: panics on an empty index, else length - 1), so LLVM
  removes the bounds check from the slot reads (the probe loop has no
  slot compare); entry reads keep one compare per candidate key.
  `d.mask` is maintained for the API but not trusted.
- Two `unsafe` blocks, no `#[raw]` function:
  `// unsafe: memset writes exactly the s.len * size_of<T>() bytes of \`s\`.`
  (`zero(s: []T)`), and
  `// unsafe: reads only the size_of<K>() bytes of the one K at \`k\` (as u64 words when the size is a multiple of 8).`
  (`key_live`). Stores through the trailing arrays passed as slices
  (`rebuild_in`, `zero`) get their barrier from the compiler after the
  call (section 8.1); `place()` stores a whole entry through
  `d.ents.e[e]`, which gets the barrier as any object store does.
- A bad field can cause a panic or an endless probe, never an access
  outside the tables. `remove_at(e)` panics unless `e` is a live entry;
  `key`, `value`, `hash_at`, `set_value` are not checked for liveness (a
  dead entry reads as zero bytes) but are bounds-checked.

Cost (Python, release, `--align-functions=64`, base and new interleaved,
best of 10, twice; outliers best of 15, twice): geomean 1.002 and 0.999
(range 0.978–1.017); reruns of deltablue, hexiom, richards, nqueens
1.006 and 1.003. `richards` is about 1% slower in every run (+0.3% to
+1.3%): below the noise threshold but consistent (the per-candidate
entry bounds check in attribute lookups that miss the inline caches).
Bounds-checking the slot reads as well cost `richards` 2.3–3% and was
dropped. No vm op handler's machine code changed; the IR changes are
confined to `std.dict` and its callers.

## 6. Cost

**The check itself costs nothing at run time:** adding `#![safe]` to
`lex`, `parse` and `diag` and `#[raw]` to `mem` left the release IR of
every example interpreter (Mini, MiniJS, Scheme, Lox, Python, Prolog,
Stack, Wasm, the Mini embedding, the bigint calculator) identical, up
to the source locations in panic messages.

**The migrations change IR** only in the migrated functions:
`hash.str.eq` (one more compare: the empty-string case),
`text.StrBuf.push_str` (now `Vec.extend`), `text.Interner.*`
(bounds-checked slot reads), `text.parse_f64` (a checked copy loop),
`io.print*`/`write_*` and `io.Out.*`, and `bc.Code.relax`. Measured on
the interpreters that call them at run time (release builds,
`--align-functions=64`, tail-call dispatch, base and new interleaved;
the machine was shared with other jobs, so single benchmarks move by
up to ±8% between runs; reruns of the outliers with 10 rounds came
back within 1.5%):

| suite | runs | new/base geomean | range |
|---|---:|---:|---|
| Python (11 programs, string-heavy `hexiom`, `chaos`) | best of 7 | 0.996 | 0.97–1.02 |
| Lox (10 programs; `zoo`, `zoo_batch`, `equality` rerun best of 10: 1.015, 0.998, 1.000) | best of 5 | 1.005 | 0.92–1.08 |
| Prolog (30 programs; output through `io.Out`) | best of 3 | 0.991 | 0.92–1.04 |
| Python front end: 20,000 function definitions (120k lines), parse + compile, never run | best of 7 | 1.008 | |

All within noise. Mini, MiniJS and Scheme use the changed functions
only at compile time or for printing (their IR differs in the same std
functions only), and `lex`/`parse` did not change, so their front ends
are unaffected.

**`std.map`.** Release machine code: `HashMap.insert` is about 5% smaller
in every interpreter (`probe` returns the slot pointer, so callers do
not index again); `get` is unchanged or ±1 instruction (Wasm's
`HashMap<[]u8, Extern>.get` 140 → 152 instructions); no op handler
changed size. Benchmarks (best of 3 to 5, interleaved): Python within
±3% (nqueens' 4% went away on a rerun), Lox geomean equal (0.46 vs 0.46
of clox, per-benchmark swings flipping sign between runs), Prolog
within ±3% except `mu` +4.8% (no handler changed: noise). No
measurable cost.

## 7. Catching dangling pointers at run time (ASan builds)

The pointers the lifetime check does not cover (section 2, item 1) are
*detectable* in ASan builds. `--sanitize=address` reports use of freed
memory with the stacks of the use, the free and the allocation, and
covers most of the hole by itself: a `Vec` or map that grows calls
`realloc`, which under ASan always moves and frees (quarantines) the
old buffer; `Vec.free`, map `free` and `Arena.free` call `free`. What
ASan cannot see is memory a container *keeps* but no longer uses; std
marks it with ASan's manual poisoning API
(`__asan_poison_memory_region`, `__asan_unpoison_memory_region`), so a
stale access is reported as `use-after-poison`. The calls are
statements under `#[cfg(sanitize_address)]`, which `--sanitize=address`
sets; the parser drops them in every other build.

**What std poisons** (`std.mem.poison`/`unpoison`, `#[raw]`, ASan
builds only):

| container | poisoned | caught stale use |
|---|---|---|
| `mem.Vec` (so `text.StrBuf`, `diag.SourceMap`, `diag.Diagnostics`, every `Vec`-backed table) | the spare capacity `[len, cap)`: on `with_capacity`, after growth, on `pop`, `truncate`, `clear`; `push`, `extend` and `filled` unpoison what they write | `ptr_at(i)`/`last` pointers and `items()` slices kept across `pop`, `truncate`, `clear`; reads of spare capacity through the raw buffer |
| `mem.Arena` | each chunk when it is allocated; `alloc_bytes` unpoisons what it hands out | reads and writes past an arena allocation (into the unused tail or alignment padding) |
| `map.OrderedMap` | entries `[used, cap)`: on creation, after `resize`, on `clear` | `entry`, `get_ptr`, `next` pointers kept across `clear` |
| freed memory (ASan itself) | — | anything kept across growth (`Vec`, `HashMap`, `OrderedMap`, `StrBuf`), `free` of any of them, `Arena.free` |

Not reported: a `HashMap` slot pointer kept across `remove` or `clear`
(the slots stay in the table, and probing reads them), an `OrderedMap`
entry kept across `remove_at` (dead entries stay below `used` until the
next resize), objects in the GC heap (hole 2: `--gc-check-roots` is the
tool there), and anything in an interpreter's own allocators.

Code outside `std.mem` that writes a `Vec`'s spare capacity before
raising `len` must unpoison it: `std.bigint` (limbs written through
`space(v)` and `room(n)`) and the collector's remembered set
(`rs.objs.ptr[n] = obj`) do, under the same `cfg`.

**Bugs found.** `Arena.free` released its chunks but kept the current
chunk's pointer, offset and size, so the next `alloc_bytes` on the same
arena wrote into freed memory, and a second `free` freed it twice. It
resets them, so "a container's methods leave it valid" holds for
arenas too: the next allocation starts a new chunk
(`tests/run/arena_free_reuse.lm`). That is the only release IR change:
two compares in `Arena.free`, which no hot path calls. No example
interpreter tripped a poisoned access or freed memory: every test
program of Python (14, plus the 11 benchmark programs), Prolog (11),
Scheme (26), Lox (6), MiniJS (66, with its conformance tests), Stak
(13), Mini (46) and the Wasm C tests (2), built with
`--sanitize=address` as debug builds, and Python, Prolog, Scheme,
MiniJS and Mini again as release builds, normally and with
`--gc-stress` (about 600 runs), printed the expected output with no
report.

**Tests.** `tests/asan/*.lm` (run by `asan_stale_pointers` in
`tests/golden.rs`, without `ulimit -v`, which ASan cannot run under):
one program per row above that must be reported, and one that uses
every poisoned container correctly and must run clean.

**Tried and rejected.** *Checked handles* (a generation counter per
container, compared at every dereference) need a second, debug-only
pointer representation and a check at every `*p` and `s[i]` in all
code. *Pattern-filling* freed memory gives garbage rather than a
report, and `malloc` soon hands the block out again.

## 8. GC objects

Three rules for GC objects, in every module (not only `#![safe]` ones:
a missing barrier is a collector soundness bug wherever it happens).
Written 2026-11-04.

### 8.1 Stores through slices of an object's trailing array

The compiler emits the write barrier after a store of a reference whose
place it can see is inside an object: `o.f = v`, `o.items[i] = v`,
`o.items[a..b][i] = v`. A slice is a plain (pointer, length) pair, so
without a rule `let s = o.items; s[i] = v` (or passing `o.items` to a
function that stores into it) stores a young object into an old one
with no barrier, and a minor collection frees it (verify mode reports
"missing write barrier": `tests/run/gc_barrier_slice_arg.lm`). The
compiler prevents it.

**The rule** (`src/objref.rs`, after checking, so every function's
effects are known). An *object slice* is `o.items`, a sub-slice of it,
or `o.arr[..]` of an array field, with elements of a traced type
(`Gc`, a `value` with references, a struct holding one). It may be

- indexed, sub-sliced, iterated, `.len`'d; stores `o.items[i] = v` get
  the barrier as before; `o.items.ptr` is a raw pointer (stores through
  it need `gc.write_barrier`, as through any raw pointer);
- bound by `let` to a local that is only read through (indexed, its
  fields read, passed where nothing stores);
- passed straight to a function. Each function's parameters of type
  `[]T` or `*T` with traced `T` are summarized twice: may the function
  store a reference through it (also through `.ptr`), and may it keep
  it beyond the call (in a `var`, a field, a return value, a capture)?
  Passing it on to a parameter that does counts as doing it (a fixpoint
  over the call graph; a function value or an extern counts as both). A
  call that may store through an object slice gets the barrier on the
  object after the call, and must not collect (`uses gc`): a minor
  collection between the store and that barrier would miss the young
  object. A call that may keep it is an error: the copy would outlive
  the barrier.

Everything else is an error, with what to write instead: keeping the
slice (`var`, return, field, capture), storing through a `let` that
holds one, passing a held one to a function that stores or keeps,
passing one to a function that keeps it, or to one that may collect and
store:

```
error: a slice of an object's traced trailing array is stored through (held in a local): a store through it would skip the write barrier
  --> tests/fail/gc_slice_kept.lm:11:5
note: store through the object (`o.items[i] = v` gets the write barrier); a slice of it may be bound by `let` and read, or passed to a function that cannot collect (the barrier follows the call)
```

A store of a value without references (`Nil`, an integer field of an
entry) needs no barrier and is not a store for the rule. The same rule
covers `&o.f` of a traced place in `#![safe]` modules, where taking an
address is not a raw-pointer operation; elsewhere `&o.f` is a raw
pointer, except that passing it (also implicitly, as a method's
receiver) to a call that may collect and store through it is an error
everywhere, since the compiler's barrier after the call would not cover
it. `unsafe { }` exempts both.

**What changed in the programs.** Python's `value.list_items(l)`
returned `l.arr.items[0..l.len]`; its 11 callers now write the slice
inline (they pass it to `tuple_of`, `list_of`, `seq_order`, ..., which
only read). `std.dict`'s `remove_at` zeroes through `d.ents.e[e..e +
1]` instead of a `let` copy, and its hand-written `barrier(ents)` after
`rebuild_in`/`key_rebuild_in` is replaced by the compiler's. Lox's `let c =
fp.cl.upvals` only reads, Mini's coroutine code stores through raw
`&co.open` pointers and logs the objects with `gc.remember` itself
(raw pointers in a module without `#![safe]`): both unchanged. No
example interpreter had a missing barrier: the only stores through
object slices in examples and std were std.dict's, which logged by hand.

**A use-after-free found on the way** (the lifetime hole, not the
barrier): Python's `list.index`, `list.count` and list/tuple ordering
(`<`) held a slice of the list's array while calling `__eq__`,
which may grow the list (moving its items to a new array) and collect.
The old array was then unreachable and freed, and the scan went on
reading it: `examples/python/tests/mutating_eq.py` printed "list.index(x):
x not in list" under `--gc-stress`. They read the list's current
items at each step, as CPython does (`value.seq_at`). The rule above
does not catch this (the slice is only read); `LOIMI_OBJREF_AUDIT=1`
lists object slices passed to, or held in a local across, calls that
may collect, for review: the remaining ones in the examples keep their
object alive (a rooted tuple, a list that the callee only reads before
allocating, which does not collect).

**Tried and rejected.** A slice type that remembers its object
(`ObjSlice<T>`, three words) changes every signature that takes
`[]Value` and makes host-memory slices pay; a barrier that finds the
object from an interior address costs every store through any traced
slice, register files included. The audit of the examples found six
such slices in three interpreters, all read-only but one.

### 8.2 Zero-filled trailing arrays

`new(h, X { .. }, n)` allocates zeroed memory. For `[Gc<T>]` that is n
null references, which outside `unsafe` cannot be made (`0 as Gc<T>`);
for a `value` type it is whatever the word 0 encodes. Zero-filling is
allowed only where zero is a valid element: numbers, `bool`,
options (None), slices, raw pointers (outside `#![safe]` modules, as for
`..zero`), and tuples, arrays and structs of those (no private `__`
fields); an enum's variant with discriminant 0 if its payload is; a
`value` type's variant that the word 0 encodes (a nanbox's unboxed
`0.0`; a low-tag or boxed variant with tag 0) unless its payload is a
reference. Generic objects are checked per instance. Elsewhere:

- `new(h, X { .. }, n, v)` fills the array with `v` (a loop of stores;
  no barrier: the object is young), or
- `?Gc<T>` elements, or
- `unsafe { new(h, X { .. }, n) }` for code that sets every element
  before anything reads it. A length that is the literal `0` needs
  none of these.

All value types of the example interpreters zero-fill to their unboxed
`0.0` (`Value`, `V`, `Term`), and std.dict's entries to all-zero keys
(dead entries; a dict over `Gc` keys or values does not compile).
Five places allocate `Gc` arrays and fill them right after:
Lox's and Mini's `CLOSURE` (upvalue cells) and the three compilers'
`Protos` tables (Mini, MiniJS, Scheme), plus the two fuzzers' copies.
They are `unsafe { new(..) }` with a comment, so the example
interpreters have five `unsafe` blocks; a fill value adds a loop to
`CLOSURE`, and `?Gc` elements a test to every upvalue access. The IR of
those five is unchanged.

### 8.3 The objects of a `#![safe]` module

A safe module's code may rely on its own structs' fields only if no
other module can write them; the field rule (section 1) protects
structs with raw pointers, and only from safe modules. Two extensions:

- **In every module**, an `object` declared in a `#![safe]` module is
  built (`new(h, M.X { .. })`) and written (`o.f = v`, `o.items[i] =
  v`, `&o.f`) only by that module's code, or in `unsafe { }`. Reading
  is unrestricted. This is what `std.dict` needed: Python wrote
  `m.popped = m.ents.e.len` to mark an attribute dict full; it calls
  `m.use_up_room()` (in `KeyDict` and `Dict`).
- **In a `#![safe]` module**, another module's `object` is treated as a
  struct with raw pointers: writing its fields or trailing array,
  taking their address and building one with a literal need `unsafe`.

Modules without `#![safe]` can still write a safe module's objects
through raw pointers (`&o.f` there is raw, as is `o.items.ptr`) and
through slices of their trailing arrays: they are trusted, as for any
`#[raw]` contract (section 2, item 3). Rejected: field visibility
(`pub(read)`); Loimi's only field privacy is the file-private `__`
prefix, and ownership by the safe module needs no new syntax.

### 8.4 Cost

Release IR of Lox, Mini, MiniJS, Prolog, Scheme, Stack, Wasm, the Mini
embedding, the regex library and the tutorial is unchanged (panic
locations aside). Python's changes are where intended:

| function | change |
|---|---|
| `std.dict.Dict/KeyDict.resize`, `Dict.copy` | the compiler's barrier after `rebuild_in` replaces dict's own call (same code after inlining) |
| `Dict/KeyDict.remove_at`, `Dict.clear` (inlined into `value.dict_clear`, `ops.delattr`) | one barrier (a header-byte test) after zeroing entries; `KeyDict.resize` also logs the old entries (it reads them through a pointer passed to dict's `unsafe` key test, counted as a store) |
| `builtins.call`, `construct`, `module_call`, `ops.setslice`, `vm.Py.op.TOTUPLE` | `list_items` inlined by hand (0-2 instructions difference) |
| `builtins.call_method_builtin` (`list.index`, `count`), `ops.order`, `ops.seq_order`, new `value.seq_at`/`seq_len` | the use-after-free fix: read the list at each step |
| `value.attrs_replace`, new `KeyDict.use_up_room` | the dict field write moved into std.dict |

Machine code (release, `--align-functions=64`) differs in those
functions only; no vm op handler changed. Python's 11 benchmarks, base
and new interleaved, best of 3 (`bench/python/compare.py --exe`):
geomean new/base 0.999 (range 0.976–1.019), within noise.
The other interpreters' IR did not change, so they were not rerun.
