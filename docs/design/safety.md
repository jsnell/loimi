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
stops safe code from keeping it (section 3). Six std modules (`lex`,
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

AUDIT_TABLE

## 4. Migration

MIGRATION

## 5. `std.map` and `std.dict`

MAPDICT

## 6. Cost

COST
