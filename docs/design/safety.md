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
stops safe code from keeping it (section 2). Seven std modules (`lex`,
`parse`, `diag`, `text`, `hash`, `io`, `bc`) are now safe; `map` and
`dict` are in section 5. It is a check only: no example's IR changed
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
| | `..zero` that makes a null `*T` (use `?*T`) |
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
pointers or build one with a literal. Copying a whole value is allowed
(that is the aliasing part of the lifetime hole, section 3). This holds
for structs of other *safe* modules too: their `unsafe` blocks rely on
their invariants.

**Exempt:** vm code (handlers, vm functions: their operand accesses are
unchecked because the verifier ran, section 6 of language.md), code
generated from a `vm` declaration, and every module without `#![safe]`.
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
   are cheap.
2. **GC references in host memory.** A `Gc<T>` kept somewhere the
   collector does not scan (a `Vec<Gc<T>>` that is not a root) dangles
   after a collection. Unchanged by this work; `gc.Heap` roots and
   `--gc-check-roots` are the tools.
3. **Trusted modules.** A `#![safe]` module is only as safe as the
   functions it calls in modules that are not: those must honor the
   `#[raw]` contract above. std's are audited (section 4); an example
   interpreter's are not.
4. **Uninitialized and zero values from elsewhere.** `..zero` cannot make
   a null `*T` in a safe module, but a non-safe module can hand one over
   (a struct it zeroed). A null `*T` faults deterministically on
   x86-64/AArch64 Linux, but LLVM treats the load as undefined.
5. **Data races.** Not addressed (`std.gil` serializes guest threads;
   atomics are allowed on any `*T`).
6. **Struct patterns.** `let Vec { ptr, .. } = v` reads `ptr` without
   going through the field rule (destructuring other modules' structs
   with raw pointers is rare; not checked).

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

MAPDICT

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

MAPCOST
