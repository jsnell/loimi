# Design note: compile-time memory safety for the standard library

Status: **built** (`#![safe]` modules, `#[raw]` functions). Written
2026-11-03.

> Raw pointers (`*T`) need no `unsafe` anywhere (language.md section
> 1.3), and the standard library is built on them. Can the compiler
> check memory safety in std, module by module, and at what cost?

**Answer.** Mostly, with one deliberate hole. A module that starts with
`#![safe]` may still use `*T`, but only as a *reference to one live
value*: following it is allowed, making it point anywhere else is not.
Everything that can reach memory outside the one value (indexing,
arithmetic, `p.slice(n)`, casts to pointers, another module's buffer
pointers, C calls, functions marked `#[raw]`) needs `unsafe { }`. What
is not checked is *lifetime*: a `*T` or slice obtained from a container
dangles once the container grows, shrinks or is freed, and nothing
stops safe code from keeping it at compile time (section 2); built with
`--sanitize=address`, std's containers poison the memory they keep but
no longer use, so most such uses are reported at run time (section 7).
Eight std modules (`lex`,
`parse`, `diag`, `text`, `hash`, `io`, `bc`, `map`) are now safe;
`dict` is not (section 5). It is a check only: no example's IR changed
from adding `#![safe]` and `#[raw]`, and the migrations' own IR changes
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
requires a pointer from a particular allocator (`Malloc.free_bytes`) is
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
(that is the aliasing part of the lifetime hole, section 3). This holds
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
   earlier can dangle. Checking this needs ownership or a borrow
   checker, which Loimi deliberately leaves out (language.md section
   11); index- and callback-based APIs (`Vec.at(i)`, `OrderedMap`
   entry indices with `version`) are the safe alternative where they
   are cheap. Built with `--sanitize=address`, most stale uses are
   reported at run time (section 7).
2. **GC references in host memory.** A `Gc<T>` kept somewhere the
   collector does not scan (a `Vec<Gc<T>>` that is not a root) dangles
   after a collection. Unchanged by this work; `gc.Heap` roots and
   `--gc-check-roots` are the tools.
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

Section 3a lists the adversarial pass that closed the other bypasses
(struct-destructuring was on this list: Loimi has no struct patterns,
`let Vec { ptr, .. } = v` does not parse, but tuple fields, arrays of
pointers and enum payloads were real bypasses).

## 3. Audit of std (before this work)

Counts are distinct source sites the checker reports when every module
is checked as if `#![safe]` (`LOIMI_SAFETY_AUDIT=1 loimi build ..`
prints them per module and kind; `LOIMI_SAFETY_AUDIT=sites` lists each
site), over the instantiations used by the example interpreters. "Field
via `*T`" counts the allowed `p.f` uses, for scale.

Columns: `idx` pointer indexing and slicing, `arith` pointer
arithmetic, `slice` `p.slice(n)`, `cast` casts to pointers (and one
integer-to-`value` forge in `obj`), `fld` another module's pointer
fields or a slice's `.ptr` read, `wr` writes to another module's
invariant fields, `call` extern and `#[raw]` calls, `hint`
`assume`/`unreachable`; `p.f` the allowed field accesses through a
`*T`.

| module | lines | idx | arith | slice | cast | fld | wr | call | hint | total | `p.f` | what the raw uses are; safe replacement |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---|
| `lex` | 641 | | | | | | | | | **0** | 156 | already slice-based (`Cursor` over a `str`) |
| `parse` | 572 | | | | | | | | | **0** | 32 | — |
| `diag` | 381 | | | | | | | | | **0** | 70 | — (`SourceMap.file`, `Diagnostics.at` return pointers into a `Vec`: lifetime hole) |
| `hash` | 48 | | | | | 1 | | | | 1 | 0 | `str.eq` by `memcmp` on `.ptr` → `mem.equal_bytes` |
| `text` | 294 | 6 | 1 | | 1 | 2 | 1 | 5 | | 16 | 35 | `StrBuf.push_str` grew `bytes` by hand (`len +=`, `memcpy`) → `Vec.extend`; `Interner` slot table `*u64` + mask → `Vec<u64>`; `snprintf`/`strtod` → `unsafe` |
| `io` | 175 | 2 | 2 | 3 | 1 | 12 | | 13 | | 33 | 22 | `write(2)` on `.ptr`/`.len` → one helper over a slice; `Out`'s buffer `*u8`+len+cap → `Vec<u8>`; `read_file` is C stdio (`unsafe`) |
| `bc` | 780 | 7 | | | | | | 2 | 2 | 11 | 156 | `relax`'s position table `mem.alloc` + `[i]` → `Vec`; `unreachable` → `trap()` |
| `num` | 402 | 1 | | | 3 | 5 | | 2 | | 11 | 0 | `snprintf`/`strtod` buffers (C interop) |
| `os` | 21 | 1 | | 1 | | | | 3 | 1 | 6 | 0 | `argv` from C |
| `rt` | 71 | 1 | | 1 | | 1 | | 5 | 2 | 10 | 0 | C runtime hooks (panic message, setjmp-based protect) |
| `map` | 470 | 7 | 14 | | 2 | | | 4 | | 27 | 48 | open-addressing tables over `mem.alloc`ed arrays, probing by masked index: section 5 |
| `dict` | 571 | 8 | 8 | | 11 | 14 | | | | 41 | 163 | the same tables in GC objects (trailing arrays, `Gc` → `*` casts): section 5 |
| `mem` | 410 | 15 | 6 | 2 | 7 | 11 | | 27 | | 68 | 53 | the allocator and `Vec` themselves: the unsafe core |
| `prof` | 305 | 4 | | 2 | | | | 5 | | 11 | 47 | counter arrays from `mem.alloc`, clock calls |
| `test` | 377 | | 1 | | 7 | 4 | | 14 | | 26 | 13 | C stdio, `setjmp`-style panic catching, golden files |
| `debug` | 468 | | | | | | | 2 | | 2 | 37 | two `mem` calls (not touched: another agent's lane) |
| `gil` | 285 | | | | 15 | | | 19 | | 34 | 87 | pthreads, atomics on casted words |
| `obj` | 1592 | 14 | 7 | | 7 | 5 | | 6 | 8 | 48 | 429 | shapes, inline caches over `mem.alloc`ed arrays; `assume` hints in IC paths |
| `gc` | 1953 | 15 | 46 | | 41 | 5 | 1 | 39 | | 147 | 550 | the collector: headers, free lists, mark stacks, address arithmetic (plus its 3 `unsafe` blocks forging `Gc`s) |
| `bigint` | 1909 | 116 | 62 | 4 | 2 | 58 | 2 | 51 | | 295 | 13 | limb arrays as `*u64` + length everywhere: a natural slice candidate, but the limb loops are its hot paths |

So the std splits into a safe periphery that was already (nearly)
pointer-free (the front-end toolkit: `lex`, `parse`, `diag`; `hash`,
`text`, `io`, `bc` with a handful of sites each), and an unsafe core
(`mem`, `gc`, `obj`, `bigint`, `gil`, the tables) where pointers are the
point. Most `*T` uses everywhere are `p.f` through a pointer to a
struct, which the rule allows.

**Use-after-free hazards found.** `StrBuf.push_str(b.as_str())` (a
buffer appending its own text) read freed memory when the push grew
the buffer: `realloc` moved it under the source slice. Fixed by
`Vec.extend`, which re-derives a source inside the moving buffer.
`HashMap.free`/`OrderedMap.free` released the table but kept its
capacity, so a later lookup on the same map read freed memory (section
5). The rest are the lifetime hole of section 2: pointer-returning
accessors (`Vec.ptr_at`, `Vec.items`, `map.get_ptr`, `OrderedMap.entry`,
`next`, `diag.SourceMap.file`, `diag.Diagnostics.at`) valid until the
next growth, and `Arena.free`.

## 3a. Adversarial pass (closing bypasses)

Programs that try to reach raw memory in a `#![safe]` module without
`unsafe` (each is a `tests/fail/safe_*.lm` now, or was already
rejected). "Was" is the compiler before this pass.

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

None of the std safe modules needed a change, and no example's IR
changed (the checks only reject programs).

## 4. Migration

| module | `unsafe` blocks | `#[raw]` | lines changed (+/−) | what changed |
|---|---:|---|---:|---|
| `lex` | 0 | | +2 | `#![safe]` only |
| `parse` | 0 | | +2 | `#![safe]` only |
| `diag` | 0 | | +2 | `#![safe]` only |
| `hash` | 0 | | +3 −3 | `str.eq` is `mem.equal_bytes` (also no `memcmp` on a null pointer for empty strings) |
| `text` | 2 | | +23 −22 | `StrBuf.push_str` is `bytes.extend(s)`; `Interner`'s slot table is a `Vec<u64>` (bounds-checked `at`/`set`); `parse_f64` copies into its buffer by a checked loop; `snprintf`/`strtod` in `unsafe` |
| `io` | 4 | `free_file` | +67 −48 | one `write_some(fd, s: []u8)` helper and one `format_g14` (the two C calls); `Out` buffers in a `Vec<u8>` (`extend`, `push`); `read_file` (C stdio) is one `unsafe` block; `free_file` frees what `read_file` returned |
| `bc` | 0 | | +7 −4 | `relax`'s position table is a `Vec<u64>` slice; `unreachable` → `trap()` |
| `map` | 21 | 7 (`probe_*`) | +266 −126 | section 5 |
| `mem` | — | 10 functions | +39 | `#[raw]` on `alloc`, `try_alloc`, `grow`, `release`, `copy`, `sort_by`, `try_sort_by`, `Arena.alloc_bytes`, `Malloc.alloc_bytes`, `Malloc.free_bytes`; new `Vec.extend(s)` and `Vec.filled(n, x)` |

The `unsafe` blocks, each with its one-line justification:

```
text.lm  // unsafe: C calls on `buf`, whose size snprintf is given and which it NUL-terminates
text.lm  // unsafe: strtod reads the NUL-terminated copy in `buf` (s.len < 64) and points `end` into it
io.lm    // unsafe: write(2) reads the s.len bytes of `s`
io.lm    // unsafe: snprintf writes at most the 64 bytes it is told `buf` has
io.lm    // unsafe: C stdio on a NUL-terminated copy of `path`; fread fills at most the n bytes malloc'd
io.lm    // unsafe: the caller passes memory read_file malloc'd
```

All are C calls: the safe modules have no `unsafe` pointer arithmetic
of their own. No example call site changed (`io.Out`'s fields changed
type, but nothing outside `std.io` touched them).

**`Vec.extend(s)`** is the primitive that `push_str` and `Out.print`
needed and that was missing: append a slice. It is alias-safe: if `s`
lies inside the vector's own buffer and the buffer must grow, the cold
path re-derives `s`'s address in the new buffer before copying, so
`v.extend(v.items())` works. The fast path (room left) is a compare and
a `memcpy`, as `push_str` was.

**Not migrated**, and why: `mem`, `gc`, `obj`, `bigint` and `gil` are
the unsafe core (allocator, collector, inline caches, limb arithmetic,
threads), where nearly every function is pointer manipulation and the
public API's contracts are documented, with `#[raw]` on the functions
that safe callers cannot satisfy; `num`, `os`, `rt`, `test` and `prof` are
small and mostly C interop (each could be made safe with a few `unsafe`
blocks; not done for lack of time and of benefit: none is hot or
complicated); `debug` belongs to another work stream.

## 5. `std.map` and `std.dict`

**`std.map` is `#![safe]`**, with 21 `unsafe` blocks, all of them
over the tables' own invariants (`slots` holds `cap` slots, `cap` a
power of two; an `OrderedMap`'s `idx` holds `mask + 1` slots naming
entries below `used`, `used <= cap <=` the allocation), which only
`map`'s own code can break since other modules cannot write the fields
of a struct with raw pointers. `HashMap` goes through one private
accessor, `slot(i)`, that masks the index (`m.slots + (i & (m.cap -
1))`: in bounds by construction; LLVM folds the second mask), plus
`new`, `grow` and `free`. The CPython-style probing core shared with
`std.dict` (`probe_find`, `probe_slot_of`, `probe_lookup`, `probe_put`,
`probe_rebuild`, `probe_kill`, `probe_find_with`) takes a raw index
pointer, a mask and an entries pointer: it is `#[raw]`, one `unsafe`
body each, with the contract documented. `OrderedMap` has ten blocks
(allocation, resize, the probing methods, `clear`, `free`) and a
bounds-checked entry accessor.

Two soundness bugs in the API were fixed on the way:

- `OrderedMap.entry(e)`, `set_value(e)` and `remove_at(e)` took any `e`
  unchecked (out of bounds reads and writes; `remove_at` of a dead
  entry looped forever). They now panic ("not a live entry index")
  unless `e` is a live entry. No example uses `OrderedMap` on a hot path
  (the Python interpreter's entry-index calls are on `dict.Dict`).
- `HashMap.free` and `OrderedMap.free` released the table but kept its
  capacity, so a later call on the map used freed memory. `free` now
  points the map at a shared zeroed sentinel table (a fresh table for
  slots over 512 bytes), which `grow`, `resize`, `clear` and `free` never
  write or release, so the map stays usable and the next insert
  allocates (`tests/run/map_free_reuse.lm`).

**`std.dict` stays non-safe.** It is the same table in the GC heap: 41
raw-pointer sites (14 `.ptr` reads of trailing arrays, 11 casts, 8
pointer arithmetic, 8 pointer indexing), plus about 14 calls of the
now-`#[raw]` probe core. Every probe passes `d.index.s.ptr` and
`d.ents.e.ptr` (raw pointers into GC trailing arrays); write barriers
take `Gc` objects cast to `*u8`; `place()` stores without barriers on
purpose. A safe `dict` would need slice-taking probe functions (a
bounds check per probe on Python's hottest path), a `gc.barrier` that
takes a `Gc<T>`, and checked entry-index methods; without those it
would be 15–20 `unsafe` blocks for little gain. Its public API keeps the
`OrderedMap`-style unchecked entry indices, documented as such.

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

The lifetime hole (section 2, item 1) cannot be closed at compile time
without ownership. It can be made *detectable*: a pointer or slice
into a container should fail loudly once the container has moved,
shrunk or freed the memory it points to.

**Options considered.**

- *Checked handles.* Containers keep a generation counter, bumped on
  every reallocation, `clear` and `free`; in debug builds `Vec.items`,
  `ptr_at`, `get_ptr`, `entry`, ... return (pointer, container,
  generation) and every dereference compares generations. Rejected:
  `*T` and `[]T` are plain machine values everywhere (struct fields,
  `vm` state, C interop), so the check would need a second pointer
  representation that exists only in debug builds, a container pointer
  that itself dangles when the container is moved or freed, and a
  validation at every `*p` and `s[i]` in all code, not just std's.
  Debug and release builds would no longer run the same code.
- *Pattern-filling* freed and abandoned memory in debug builds (fill
  the old buffer with `0xdd` before `free`, force growth to move). Cheap
  and needs no tool, but a stale read gives garbage, not a report, and
  `malloc` hands the filled block out again soon, so the garbage is
  often valid data.
- *AddressSanitizer poisoning* (picked). `--sanitize=address` already
  reports use of freed memory precisely, with the stacks of the use, the
  free and the allocation, and it already covers most of the hole: a
  `Vec` or map that grows calls `realloc`, which under ASan always
  moves and frees (quarantines) the old buffer; `Vec.free`, map `free`
  and `Arena.free` call `free`. What ASan cannot see is memory a
  container *keeps* but no longer uses. ASan's manual poisoning API
  (`__asan_poison_memory_region`, `__asan_unpoison_memory_region`) marks
  that memory, so a stale access is reported as `use-after-poison`.
  The cost is confined to ASan builds: the calls are statements under
  `#[cfg(sanitize_address)]`, which `--sanitize=address` now sets, and
  the parser drops them in every other build.

**What std poisons** (`std.mem.poison`/`unpoison`, `#[raw]`, ASan
builds only):

| container | poisoned | caught stale use |
|---|---|---|
| `mem.Vec` (so `text.StrBuf`, `diag.SourceMap`, `diag.Diagnostics`, every `Vec`-backed table) | the spare capacity `[len, cap)`: on `with_capacity`, after growth, on `pop`, `truncate`, `clear`; `push`, `extend` and `filled` unpoison what they write | `ptr_at(i)`/`last` pointers and `items()` slices kept across `pop`, `truncate`, `clear`; reads of spare capacity through the raw buffer |
| `mem.Arena` | each chunk when it is allocated; `alloc_bytes` unpoisons what it hands out | reads and writes past an arena allocation (into the unused tail or alignment padding) |
| `map.OrderedMap` | entries `[used, cap)`: on creation, after `resize`, on `clear` | `entry`, `get_ptr`, `next` pointers kept across `clear` |
| freed memory (ASan itself) | — | anything kept across growth (`Vec`, `HashMap`, `OrderedMap`, `StrBuf`), `free` of any of them, `Arena.free` |

Not covered: a `HashMap` slot pointer kept across `remove` or `clear`
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
arena wrote into freed memory, and a second `free` freed it twice
(`tests/run/arena_free_reuse.lm`; the rule "a container's methods leave
it valid" now holds for arenas too: the next allocation starts a new
chunk). The change is the only one in release IR: `Arena.free` gained
two compares (no hot path calls it). No example interpreter tripped a
poisoned access or freed memory: every test program of Python (14,
plus the 11 benchmark programs), Prolog (11), Scheme (26), Lox (6),
MiniJS (66, with its conformance tests), Stak (13), Mini (46) and the
Wasm C tests (2), built with `--sanitize=address` as debug builds, and
Python, Prolog, Scheme, MiniJS and Mini again as release builds, normally
and with `--gc-stress` (about 600 runs), printed the expected output with
no report.

**Tests.** `tests/asan/*.lm` (run by `asan_stale_pointers` in
`tests/golden.rs`, without `ulimit -v`, which ASan cannot run under):
one program per row above that must be reported, and one that uses
every poisoned container correctly and must run clean. Release IR of
every example is unchanged apart from `Arena.free`.
