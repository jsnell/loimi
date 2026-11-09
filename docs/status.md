# Implementation status

What exists today, what its limits are, and where the implementation
differs from the design in [`language.md`](language.md). It is for
contributors and for anyone checking whether a feature is built.
Updated 2026-11-08; recent changes by topic are in
[`CHANGELOG.md`](../CHANGELOG.md).

## At a glance

| Area | State | Main limits |
|---|---|---|
| [Compiler](#compiler) | Rust, no dependencies; emits LLVM IR and drives `clang-19` | — |
| [Targets](#targets) | x86-64 Linux, AArch64 Linux | AArch64 tested under qemu only |
| [Language](#language) | As described in `language.md` | No `dyn` traits, no effect inference |
| [`vm` declarations](#vm-declarations) | All features, in all three dispatch modes | — |
| [Instrumentation](#instrumentation) | Variants, op counts, a sampling profiler | — |
| [Debugging guest programs](#debugging-guest-programs-prototype) | Prototype: breakpoints, stepping, a DAP server for Mini | Not under the JIT or in stack vms |
| [GC](#gc) | Mark-sweep in Loimi, optional generational mode | Non-moving; no card marking |
| [Threads](#threads) | Independent states on any thread; a shared heap one thread at a time | No free threading |
| [Libraries and embedding](#libraries-and-embedding) | Static and shared libraries with a generated C header | `Gc` and `value` types don't cross into C |
| [JIT](#jit) | Copy-and-patch, generated from the handlers | x86-64 executables, tail-call mode, one vm |
| [Standard library](#standard-library) | Modules from I/O and collections to lexers, bytecode builders and big integers | — |
| [Examples](#examples) | Nine interpreters | — |

## Compiler

- Written in Rust with no dependencies. It emits textual LLVM IR and
  drives `clang-19` to build it.
- A large module is split into parts that clang compiles in parallel
  (`src/splitir.rs`).
- `LOIMI_DUMP_VM_EXPANSION=1` prints the Loimi source generated for
  each `vm` (its expansion) to stderr.
- Errors and panic locations in user text copied into the expansion
  (`verify` and `const` clauses, `verify_context` types, `verify`
  blocks, stack effect counts, `stack_entry`) point at the vm
  declaration, with a note naming the generated context. An error in
  the rest of the expansion says `internal: generated code`.

### Commands and options

| Command or option | What it does |
|---|---|
| `loimi build`, `loimi run`, `loimi test` | Build; build and run; run `#[test]` functions |
| `loimi ir-diff A B` | Compare the code of two builds ([below](#comparing-builds-loimi-ir-diff)) |
| `--release` | Optimized build |
| `--emit-llvm`, `--keep-ll` | Write the IR instead of an executable; keep the IR next to the output |
| `--dispatch=tailcall\|goto\|switch` | Dispatch mode |
| `--sanitize=address`, `--sanitize=thread` | AddressSanitizer, ThreadSanitizer |
| `--align-functions=N` | Start every function on an N-byte boundary, so unrelated changes don't move vm handlers (steadier A/B benchmarks) |
| `--no-branch-align` | Drop the default x86-64 padding that keeps every branch of a vm program off 32-byte boundaries (Intel's JCC erratum; [lowering.md §12](lowering.md#12-driver-settings)) |
| `--max-errors N` | Report at most N errors (default 20, `0` for all) |
| `-W error`, `--deny-warnings` | Make warnings errors |
| `--target=`, `--sysroot=`, `--cpu=` | See [Targets](#targets) |
| `--lib`, `--shared`, `--header`, `--tls=` | See [Libraries and embedding](#libraries-and-embedding) |
| `--vm-stats`, `--vm-prof` | See [Instrumentation](#instrumentation) |
| `--explain-values` | Print every `value` type's encoding, then stop |
| `--gc-check-roots` | Check every GC root ([below](#gc-rooting-fuzzing)) |
| `--jit-stencils` | See [JIT](#jit) |

### Environment variables

| Variable | What it does |
|---|---|
| `LOIMI_STD` | Where the standard library is. Default: a `std/` directory next to the executable or up to three directories above it, else in the current directory |
| `LOIMI_CLANG`, `LOIMI_AR` | Override `clang-19` and `llvm-ar-19` |
| `LOIMI_LD`, `LOIMI_OBJCOPY` | Override `ld.lld` and `llvm-objcopy` (used to merge a static library's parts) |
| `LOIMI_SPLIT=N` | Number of parts a large module is split into; `1` compiles it whole |
| `LOIMI_SPLIT_REPORT=1` | Time the parts |
| `LOIMI_ASAN_RUNTIME`, `LOIMI_TSAN_RUNTIME` | A sanitizer runtime to link when clang has none |
| `LOIMI_BRANCH_ALIGN=0` | Same as `--no-branch-align` (for test suites) |
| `LOIMI_TARGET`, `LOIMI_SYSROOT` | Defaults for `--target` and `--sysroot` |
| `LOIMI_CPU` | Default for `--cpu` |
| `LOIMI_RUNNER` | Run `loimi run` and `loimi test` programs under this command (an emulator) |
| `LOIMI_LIFETIMES=off` | Turn off the pointer stability check for a build |
| `LOIMI_DUMP_VM_EXPANSION=1` | Print each vm's expansion |

### Errors

A build reports every independent error it finds, not only the first:
in source order, each place once, at most `--max-errors N`, then
`error: N errors`.

These passes go on past an error:

- Checking function bodies: each body, each generic instance (which
  reports only its first error) and each statement. This covers type
  errors, name resolution inside bodies, effects and unsafe operations.
- In a `vm`: each op handler, super, `-> never` and plain vm function,
  entry, `on raise` and variant `before` block.
- The write-barrier (`objref`) and lifetimes checks. They run only on a
  program that type-checked, and report the first error of each
  function.

A statement that fails is left out, and what it binds is *poisoned*
(type `never`). Errors in later statements that read it, or in an
enclosing statement, are not reported, so one bad expression gives one
error. An error in the condition of an `if` or `while`, or in the
scrutinee of a `match`, doesn't stop its branches from being checked.

These stop at their first error, because later passes depend on them:

- parsing and `vm` expansion;
- imports, type definitions, signatures, impls, module-level `var`
  initializers;
- a vm's declarations (state, operands, formats, caches).

The checks of a vm as a whole (constant operands written, `pc` writes,
`collects`, shared continuations) are skipped in a vm with an error in
a body.

### Warnings

Silence a warning with `#[allow(name)]` on an item, vm item or match
arm, or with `#![allow(name)]` for a file.

| Warning | On by default | Reports |
|---|---|---|
| `match_fallout` | yes | in vm code, an arm whose body is an `if` without `else` when a later arm could match |
| `super_overlap` | yes | declared supers where a proper suffix of one is a proper prefix of another |
| `unreachable_arm` | yes | a `match` arm after one that matches everything |
| `value_layout` | yes | an unpinned `value` variant that a non-classic layout narrows |
| `next_pc_then` | yes | `next_pc` in an op with `then` |
| `state_registers` | yes | a tail-call vm whose state needs more `preserve_none` argument registers than the target has |
| `hot_gc` | no (`-W hot_gc`) | a call that may collect on a handler's hot path |
| `handler_frame` | no (`-W handler_frame`) | a tail-call handler whose fast path sets up a stack frame, with the calls that cause it (read from the assembly) |
| `redo_effect` | an error unless allowed | an effect before a `redo` (`language.md` §4) |

The retired `state_root` is still accepted by `#[allow]`.

## Targets

- **x86-64 Linux**, native.
- **AArch64 Linux** with `--target=aarch64-linux-gnu [--sysroot=DIR]`:
  clang's `--target`, linked with `ld.lld`.

The IR is the same for both targets except for:

- a few vector operations (see [Vectors](#vectors));
- `std.rt`'s handlers: LLVM's builtin setjmp on x86-64, glibc's
  `_setjmp` on AArch64;
- tail-call vms: AArch64 keeps the dispatch table's address in a state
  register instead of an `adrp`+`add` per dispatch; x86-64 indexes the
  table as an absolute address ([lowering.md §3](lowering.md#3-handlers-in-tailcall-mode)).

**CPU level.** Code is for the target's baseline (x86-64: SSE2) unless
`--cpu=NAME` (or `LOIMI_CPU`) names a CPU. The name goes to clang as
`-march` (x86-64) or `-mcpu` (AArch64) for every clang run of the
build, stencils and C runtimes included. The CPU's extensions, read
from clang's predefined macros, choose Loimi's own vector lowering, so
`native` and any name clang knows work; an unknown name is an error.
Interpreters gain nothing measurable from `x86-64-v3` (within noise on
Stak, Lox, Scheme, Mini and the wasm benchmarks).

**Testing AArch64.** It runs under qemu user mode
(`bench/cross/install.sh` installs qemu and a Debian arm64 sysroot from
pinned sources):

- The whole golden suite passes cross-compiled (`LOIMI_TARGET`): all
  dispatch modes, GC stress and verify, C hosts, `rt` panics.
- The heavy test tier runs a subset under qemu (`aarch64_qemu`,
  `vm_prof_stress_aarch64`) when the toolchain is installed.

**Limits.**

- AArch64 has not been run on hardware or with 16K/64K pages.
- The JIT (x86-64 only) and `--sanitize` don't combine with `--target`.

## Language

`language.md` §1 describes the language as built; where it departs from
the original design is listed in
[Differences from the design](#differences-from-the-design). The notes
below are limits and implementation details that the reference doesn't
spell out.

### Core language

- Arbitrary-width integers `iN`/`uN` for N from 1 to 64. Overflow traps
  in debug builds; casts from integers to fieldless enums are checked in
  debug builds.
- `match` checks exhaustiveness. String literal patterns compile to a
  length switch, then word-sized compares.
- Float built-ins (`sqrt`, `floor`, `minimum`, ...) are each an LLVM
  intrinsic.
- `..zero` in a struct literal works only for fields where zero is
  valid: numbers, `bool`, options, slices and raw pointers. Not for
  `Gc<T>`, functions, structs, enums or values.
- The compiler gives hints for operators Loimi lacks (`++`, `**`, `?:`,
  `===`).

### Globals

- Module-level `var`s are plain LLVM globals. `#[thread_local]` gives
  one copy per thread (`thread_local(initialexec)`).
- The initializer must be a constant expression: literals, consts,
  `[x; N]`, struct, tuple and array literals, payload-less enum
  variants, `None`/`Some`/`Ok`/`Err` of constants, and functions.
- Globals are not GC roots. A global holding a `Gc` or `value` must be
  marked by a root scanner (`heap.add_scanner`).

### Generics and traits

- Generics are monomorphized; dispatch is static only.
- Bounds are checked at instantiation, with the error at the use site.
  Generic bodies are checked against their bounds; unbounded parameters
  stay duck-typed.
- Explicit type arguments on calls (`mem.alloc<u8>(n)`, `c.op<K>(&ops)`)
  can give only a method's own type parameters, not a generic type's:
  `Vec.new<T>` doesn't work.
- `f<T>(` is read as type arguments only when `<` directly follows the
  name and a balanced, type-shaped `<...>` is followed by `(`.
  Otherwise `<` is a comparison.

### Closures

- `block fn` closures are lowered to a separate function plus an
  environment of pointers to slots in the enclosing frame (small `let`s
  are copied in). Code and environment are passed as two arguments, so
  LLVM can specialize a callee for a closure literal. Plain functions
  convert through a trampoline.
- `block fn` closures can't be created in vm code. Escaping closures
  (`closure fn`, `Gc<closure fn>`) can be called in vm code but not
  created, and don't cross into C.
- `c.free(a)` frees nothing for closures that own no memory (it compares
  the code with the signature's trampoline). Use after free is not
  detected (ASan does detect it), and freeing doesn't free what captures
  point to.

### Vectors

- The 128-bit types are LLVM `<N x T>` vectors. Vector loads and stores
  assume no alignment.
- The baseline is SSE2 and NEON. The IR differs by target for
  `swizzle` (`tbl` on AArch64), signed `narrow_sat` (`sqxtn`/`sqxtun`),
  `f32x4` to integer conversions (`cvttps2dq` on x86-64) and float
  rounding (inline SSE2 on x86-64, no libm calls).
- With `--cpu`, `swizzle` becomes `pshufb` (SSSE3) and the rounding
  methods `roundps`/`roundpd` (SSE4.1). [lowering.md §12](lowering.md#12-driver-settings)
  has instruction counts per level.
- `--header` signatures don't take vector types.

### `value` types

- `nanbox`, `lowtag` and `boxed` representations, and `nanbox(int_high)`
  (words rotated left by 16; examples/python uses it, 0.93–0.94 of the
  plain nanbox's time).
- Integer payloads up to 48 bits (nanbox) or 61 bits (lowtag). Lowtag
  uses 3-bit primary tags.
- Tuple patterns over values merge into one test (xor/or/mask, `umax`).
- The layout is described in [lowering.md §2](lowering.md#2-values-nanbox-lowtag-boxed);
  `loimi build --explain-values` prints it for a program.

### Conditional compilation and atomics

- `#[cfg]` items are dropped by the parser: a build without the name
  has the same IR as one without the code.
- Atomics work on 8- to 64-bit integers and pointers (`atomic_add` not
  on pointers).

## `vm` declarations

`language.md` §3–§9 describes the vm features as built; all of them
work in all three dispatch modes. Some details:

- Operand fields decode to `u64` / `i64` in handlers and verify
  clauses, whatever their width.
- Inline cache fields wider than a code word span several words, with
  unaligned access. Caches can't hold GC references.
- `#[forward]` supers on all three larger interpreters: 0.97–0.99 of
  their time by geometric mean, up to 0.92 on float loops.
  Accumulators: the wasm interpreter runs in 0.91 of its time.
- Nested entries (`uses reenter`) keep a generated thread-local
  `Vm.depth` count.

### Dispatch

- **`tailcall`**: `preserve_none` and `musttail`, with the instruction
  word passed in a register. Handlers are compiled without
  memset/memcpy/memmove formation, and loops in tail-call vm code are
  not vectorized or unrolled, so no loop gives a handler a stack frame.
- **`goto`**: one function. A shared `indirectbr` block that LLVM copies
  into every handler after register allocation keeps compile time near
  linear in the op count ([lowering.md §5](lowering.md#5-goto-mode)).
- **`switch`**: one function with a `switch`.

### Generated per vm

The compiler generates these items for each vm:

- encoders (`Vm.OP(..)`), emitters (`Vm.emit.OP(..)`), opcode
  constants, `op_size`, `op_name`, `Vm.Op` and `Vm.op_of(w)`;
- the superinstruction peephole (`Vm.optimize`) and deoptimization
  (`Vm.deopt`);
- the verifier (`Vm.verify`), the proof type `Verified<Vm>`,
  `Vm.build_verified` and resume points (`Vm.Resume`);
- stack effect queries for stack vms (`v.max_stack()`,
  `Vm.stack_effect(w)`);
- label helpers, per-format field readers, setters and encoders, and
  per-op operand metadata;
- the disassembler (`Vm.disasm`, `Vm.disasm_insn`).

Setters and encoders are compiled only where called, and resume points
only for programs that name them.

The items are named `Vm__X` in the vm's module, so an op named like one
of them (`label`, `verify`) is an error, as is a module item named
`Vm__..`. Item and operand names starting with `__` are reserved.

## Instrumentation

- **Variants**: `variant NAME wraps all|[OPS] { before { .. } }`,
  switched from vm code with `use_variant`. `#[unfused]` variants run
  supers as their parts, so every instruction is wrapped.
- **Counts**: `loimi build --vm-stats` counts executed ops, adjacent op
  pairs, sequences of 3 and 4, entries into each `cold` block and every
  `become` per site. At exit it prints the counts, with frequent pairs
  and sequences as `super` declarations. `Vm.optimize` picks fusions by
  dynamic programming (most dispatches saved).
- **Time**: `loimi build --vm-prof` links a sampling profiler
  (`src/prof_rt.c`, a `SIGPROF` timer at 1000 Hz, `LOIMI_PROF_*`
  settings). At exit it prints samples per op handler and cold block,
  per function, by vm location on the stack and by guest `pc`. It
  works in all dispatch modes, under `--jit-stencils`, in libraries
  and on both targets, without external tools. Cost: 3% on Mini and
  Python in tail-call mode, 8% for Mini in goto mode
  (`language.md` §7.3).

## Debugging guest programs (prototype)

- **Breakpoints**: `on break { .. }` generates breakpoint ops, one per
  inline cache layout, so a breakpoint has the size of the instruction
  it replaces. `Vm.set_break` works on verified code and splits a super
  spanning the word. Code with breakpoints doesn't verify; `deopt` keeps
  them.
- **`std.debug`**: breakpoint sites with step-over, line stepping
  (`step`/`next`/`finish`), conditions, hit conditions, pause requests
  and a command reader.
- **Mini** built with `--cfg debug`: `mini --debug script < commands`,
  with break, step, next, finish, bt, locals, print and more.
- **Python subset** built with `--cfg debug`: `python-loimi --debug
  prog.py`, pdb-like.
- **For editors** (Mini, `--cfg debug`): a stop-callback C API
  (`mini_debug_*` in `embed.lm`) and a Debug Adapter Protocol server in
  C with no dependencies (`examples/mini/dap/mini_dap.c`).
- Default builds are unchanged (byte-identical IR). Debug builds run
  within noise of default ones when not debugging.
- Tests: golden transcripts of both debuggers and of a scripted DAP
  session, in all three dispatch modes; a C host tests the API under GC
  stress.

**Limits**: breakpoints work in interpreted code, not under
`--jit-stencils` or in stack vms. Design and protocol details are in
[`design/debugging.md`](design/debugging.md).

## GC

- `std.gc` is a mark-sweep collector written in Loimi, without a sweep:
  marking sets per-chunk bitmaps (a bit per 16-byte granule), and each
  size class bump-allocates through the runs of clear bits, so dead
  objects are never read. Chunks with nothing marked go to a shared
  pool.
- The compiler generates tracing, including pointers inside values.
- Roots are shadow-stack frames for traced locals, and for vm state
  variables of traced types, copied into the frame at each call that
  may collect. `sync` blocks and root scanners don't need to know them.
- Collection happens only in calls marked `uses gc` (see
  [Differences](#differences-from-the-design)).
- **Generational mode** (`heap.set_generational`, on in Mini, MiniJS
  and Scheme): non-moving, sticky mark bits with an epoch flip per major
  collection, aging for large objects. The compiler emits an
  object-logging write barrier after every store of a reference into an
  object (none for values statically free of references). Its slow
  path makes no call, so it gives handlers no stack frame.
- Also built: weak references, ephemeron tables, weak intern tables,
  `#[trace(f)]` hooks, free hooks, dirty pinning, and verify mode
  (checks the write barriers at every collection). See
  [`stdlib/gc.md`](stdlib/gc.md).

**Limits**: the heap never moves objects ([roadmap, question 2](roadmap.md)).
There is no card marking: a large array that is stored into is retraced
whole by each minor collection. Small objects are promoted at their
first survival. Free hooks can't run guest code or allocate.

## Threads

**Independent states** on different threads run concurrently with no
locks:

- The runtime's per-thread state (shadow-stack top, `Vm.depth`, vm
  scratch slots, `std.rt`'s handler chain and panic message, the
  `#[track_caller]` location) is initial-exec TLS (local-dynamic in
  `--shared` builds). This costs nothing measurable over plain globals.
- `std.gc` keeps everything in its `Heap`; it has no globals.
- A state may move between threads with a synchronized hand-over.
- Several states on one thread may nest (a native of B runs A). Each
  collector's shadow-stack walk marks only its own heap's objects.

**A shared heap, one thread at a time** (opt-in): `std.gil` is a lock
per heap, handed over first-come-first-served at points the interpreter
polls. Mini built with `--cfg gil` (`examples/mini/threads.lm`) runs one
Mini thread per OS thread over shared globals and code. Cost
single-threaded: 1.2–1.4% geometric mean on Mini (worst 4–5%); default
builds are unchanged.

**Tests**: `examples/embed/threads.c` (four states on four threads) and
`examples/embed/shared.c` (four threads on one heap) under GC stress,
both clean under ThreadSanitizer (`--sanitize=thread`, which links
GCC's `libtsan`).

**Limits**:

- No free threading (several threads running vm code in one heap at
  once; [roadmap, question 4](roadmap.md)).
- Not thread-safe: plain module-level `var`s, `--vm-stats` counters,
  the `--jit-stencils` JIT.

## Libraries and embedding

- `loimi build --lib` builds a static archive of `-fPIC` code;
  `--shared` builds a shared object. Neither has a `main`, and there is
  no init function. `export "C"` functions are the API.
- `--header out.h` writes a C header for them, `extern "C"`-guarded for
  C++. Types that don't map to C are rejected with the reason.
- `--tls=initial-exec|local-dynamic|global-dynamic` overrides the model
  of the runtime's thread-locals.
- `#[catch_panic(v)]` on an export turns panics into an error return
  instead of aborting the host. An `rt.panic` message is copied, up to
  512 bytes.
- `--sanitize=address` and `--vm-stats` work for libraries; several
  `--vm-stats` libraries can link into one host.
- `examples/mini/embed.lm` makes Mini a library and
  `examples/embed/host.c` uses it. C natives can call back into Mini as
  nested runs. A panic returns `MiniStatus_Internal` and poisons only
  that state.

Details are in `language.md` §10.1.

## JIT

`loimi build --jit-stencils` adds a copy-and-patch JIT generated from
the vm's handlers. It is turned on at run time with `LOIMI_JIT=1`.

- Stencils are read back from clang's object file (`src/elf.rs`); the
  runtime is `src/jit_rt.c`.
- It runs Mini, MiniJS, Scheme and Prolog, in 0.76, 0.83, 0.78 and 0.94
  of their interpreters' time by geometric mean (2026-11-07,
  [`results.md`](results.md)). With the JIT off, a JIT build runs as
  fast as a default one.
- Quickening re-patches compiled code instead of dropping it.
- A `jump` to a computed pc that is not a label enters the target's
  compiled code, or the interpreter there.
- `const` operands are patched into the code, with stencil variants per
  constant tag.
- Builds also take `--sanitize=address` and `--gc-check-roots`.

**Limits**: x86-64 Linux executables only, tail-call dispatch, one vm,
no variants, no breakpoints. There is no trace compiler; why the
template JIT stops here is in [roadmap, question 6](roadmap.md). Design:
[`design/copy-and-patch.md`](design/copy-and-patch.md).

## Standard library

Every module is described in [`stdlib.md`](stdlib.md).

| Module | Contents |
|---|---|
| `std.io`, `std.os` | Printing, buffered output (`Out`), reading files, program arguments |
| `std.mem` | `Vec` (with stable `sort_by`), byte search on slices, allocators `Arena` and `Malloc` |
| `std.hash`, `std.map` | Hashing; `HashMap` and the insertion-ordered `OrderedMap` |
| `std.dict` | Insertion-ordered maps in the GC heap, in CPython's compact layout (`Dict`, `KeyDict`) |
| `std.text` | `StrBuf` with `{}` formatting, `Interner`, number parsing and printing |
| `std.math` | `Divisor`: division by a prepared constant |
| `std.num` | Exact number parsing, shortest round-trip float formatting (Schubfach), checked and floor division |
| `std.bigint` | Arbitrary-precision integers with Python's semantics; 2.8–6.4x faster than CPython 3.13 on the stdlib.md benchmarks |
| `std.gc` | The collector ([GC](#gc)) |
| `std.gil` | A lock per heap ([Threads](#threads)) |
| `std.rt` | `protect`/`throw`, `catch_panic`, panics, `ValueStack` |
| `std.test`, `std.prof` | `loimi test` assertions and golden files; clock, benchmarking, in-program op counts |
| `std.diag` | Source maps and diagnostics rendered in the compiler's format |
| `std.lex` | A scanner toolkit (cursor, comment and escape presets, maximal-munch operators) |
| `std.parse` | Pratt parsers, and run-time operator tables for Prolog |
| `std.bc` | Bytecode construction over a vm's encoders: labels, fixups, constant pools, a register allocator |
| `std.obj` | Hidden-class objects with shapes, inline caches and prototype chains |
| `std.debug` | Breakpoints and stepping ([Debugging](#debugging-guest-programs-prototype)) |

Limits:

- `std.num` parses floats with strtod and formats `%.14g` with
  snprintf.
- `std.bigint` has no FFT multiplication or subquadratic gcd.
- `std.obj` has no elements, global property cells or `SetError`. Keys
  are u64 symbol ids. Shape ids are never reused, so inline caches need
  no reset after a collection ([`stdlib/obj.md`](stdlib/obj.md#as-implemented)).

## Examples

Nine interpreters, each tested by `cargo test`. The README has one
line on each; benchmarks are in [`results.md`](results.md).

| Directory | Language | Notes |
|---|---|---|
| `examples/mini/` | A Lua 5.4 subset | Coroutines switched inside the running vm (`COROUTINES.md`), threads with `--cfg gil`, embeddable in C, a debugger |
| `examples/scheme/` | An R7RS-small subset | Generational heap |
| `examples/minijs/` | A JavaScript subset | Built on `std.obj`: shapes, mono/poly/mega inline caches |
| `examples/lox/` | Lox | All 246 of the book's clox tests |
| `examples/regex/` | A backtracking regex engine | A C library |
| `examples/stack/` | Stak, a Forth | Declared stack effects |
| `examples/wasm/` | WebAssembly MVP plus SIMD and other extensions | Every spec test file passes; also benchmarked on PolyBench/C |
| `examples/python/` | A Python 3 subset | Ints on `std.bigint`, dicts on `std.dict`; the 11 pyperformance programs in `bench/python` print CPython's output |
| `examples/prolog/` | An ISO Prolog subset on a WAM | |

Lox, regex, Stak, wasm, Python and Prolog were written from the docs as
usability studies; each has a `FRICTION.md`.

## Tests

The three test tiers and how to run them are in the README
("Testing and benchmarking"). `cargo test` covers:

- golden programs (`tests/run/`, every vm test in all three dispatch
  modes), expected compile errors (`tests/fail/`) and `loimi test`
  files (`tests/test/`);
- every example's tests. Mini, Scheme and MiniJS also run with GC
  stress and in verify mode; Lox and Python with GC stress;
- the tutorial's steps against their chapters, and the code blocks in
  the README and `docs/` (`tests/docs.rs`);
- `std.bigint` against CPython;
- a few seconds each of [bytecode fuzzing](#bytecode-fuzzing) and
  [GC rooting fuzzing](#gc-rooting-fuzzing);
- C hosts against `--lib` and `--shared` builds and their headers
  (`tests/embed/`): caught panics, nested runs, ASan, threads and
  TSan;
- re-entry: `tests/run/vm_reenter.lm` (a stack that moves on every
  callback) and Mini's sort and coroutine scripts, in all three modes.

## Bytecode fuzzing

`tests/fuzz/run.sh` builds fuzzers that check that verified code
doesn't crash. It builds them with `--sanitize=address`, runs them for a
time or seed budget, and saves failing seeds. There are two:

- **`tests/fuzz/fuzzvm.lm`**: a test vm that uses every verifier feature
  (operand kinds of every sort, labels of three widths, verify contexts,
  inline caches, families, supers, `then`, `-> never` functions, cold
  blocks, variants), declared once per dispatch mode. Random programs
  (mostly well-formed, some mutated, some random words) are verified
  and, if they pass, run in all three modes with exactly-sized heap
  buffers and a step budget. The fuzzer checks that the modes agree,
  and that code still verifies after running (quickening) and after
  `Vm.optimize`.
- **`tests/fuzz/minifuzz.lm`**: random code against Mini's own `verify`
  clauses and `Mini.run`, on a stack exactly as large as the frame.

`cargo test` runs a few seconds of both without ASan. The longest run
so far was 2 h 10 min under ASan (2026-11-02): 149M test-vm programs
and 11.6M Mini programs. The bugs it found have regression tests in
`tests/run/vm_verify_*.lm`; the holes it found in Mini's verify clauses
are checked by `tests/fuzz/mini_holes.lm`.

**ASan notes.** ASan works in all three dispatch modes.

- `--sanitize=address` turns off use-after-return detection. Its
  fake-stack fallback is a dynamic alloca, which LLVM can't combine with
  stack realignment in `preserve_none` functions, so tail-call builds
  wouldn't compile.
- ASan executables don't report leaks, since programs leave their heaps
  to the OS. `ASAN_OPTIONS=detect_leaks=1` turns leak checking on.

## Comparing builds: `loimi ir-diff`

A refactor of an interpreter shouldn't change its code, but moving a
line changes the release IR, because panic messages carry
`file:line:col`. `loimi ir-diff A B` says whether two builds have the
same code, function by function:

    loimi ir-diff old/examples/scheme/main.lm examples/scheme/main.lm --release --asm
    loimi build main.lm --release --emit-llvm -o a.ll    # or two .ll files, from any compiler
    loimi ir-diff a.ll b.ll --release

**Inputs.** A and B are each a program (`.lm`, built with the build
options given) or a `.ll` file from `loimi build --emit-llvm`. Both go
through clang with the options `loimi build` uses, to optimized IR and,
with `--asm`, to assembly.

A `.ll` file names the LLVM options of its build on its first line
(`; loimi-llvm-args: ..`), so the outputs of two compilers compare
exactly in every dispatch mode. This also covers the interpreter of a
`--jit-stencils` build, which `.lm` inputs don't support. A `.ll`
without that line gets the goto options if it has an `indirectbr`, and
no others.

**What it ignores:**

- source locations in string constants (`dir/vm.lm:120:7:` becomes
  `vm.lm:L:C`, so the same file in two trees compares equal), and the
  message lengths passed next to them;
- names of private and internal constants (`@.str.12`, switch tables),
  which are renamed after their contents;
- local value and label names, which are renumbered in order of
  appearance;
- comments, `!dbg` attachments, and the numbering of attribute groups
  and metadata;
- constant pools (`.LCPI0_1`), which are inlined;
- the program-wide numbering of outlined cold blocks
  (`vm.Py.op.BOR.cold132` becomes `vm.Py.op.BOR.cold#0`, numbered per
  handler, so a cold block added to one op doesn't rename every later
  one);
- the order of functions.

Functions on one side only are paired when their normalized bodies are
equal (a rename), and calls to them compare equal.

**Output.** A summary per form, the names of the functions that
differ, and a unified diff of each. The exit status is 0 when every
function has the same code, 1 otherwise, 2 on errors. For example
(Scheme's `enum Prim` port: every vm handler is the same):

    IR: 511 functions in A, 510 in B: 507 same, 0 renamed, 3 differ, 1 only in A, 0 only in B
      only in A: compile.inline_prim (198 lines)
      differs: builtins.call_native (3581 lines in A, 3586 in B)
      differs: compile.Comp.build_inner (2455 lines in A, 2791 in B)
      differs: compile.Comp.inl (607 lines in A, 809 in B)

| Option | What it does |
|---|---|
| `--asm` | Compare the assembly too |
| `--context=N` | Lines of context in each diff |
| `--max-lines=N` | Lines per diff, `0` for all |
| `--keep=DIR` | Keep the optimized IR, the assembly and their normalized forms |

**Limits.**

- A location string that LLVM merges into a `phi` or a switch table of
  lengths is not recognized, and shows as a difference.
- Global data other than constants (initialized `var`s that are
  written) is compared by name only, through the code that uses it.

## GC rooting fuzzing

`loimi build --gc-check-roots` makes the collector check every
reference it marks before reading its header: shadow-stack roots,
copies of vm state, `gc_mark` in scanners and traced fields. It also
fills unassigned traced slots and freed objects with 0x01 bytes. A bad
root is reported with its frame, function, call site and slot
(lowering.md §9).

`tests/fuzz/rootgen.rs` generates random programs that mix traced
locals with every binding form, early exits, stores and calls that
collect, in plain functions and in vm code. Each program is built in
this mode and run three ways: without collections, under GC stress, and
under generational stress with verify. The fuzzer checks that:

- the outputs (digests that read through every reference) agree across
  runs and dispatch modes;
- objects reached only by out-of-scope locals are gone after a full
  collection.

It keeps to what the language promises: no traced temporary is held
across a call that may collect.

    cargo test --release --test fuzz fuzz_gc_roots       # smoke: 18 fixed seeds, ~20 s
    ROOTFUZZ_SECS=300 ROOTFUZZ_JOBS=2 ROOTFUZZ_FIRST=1000 \
      cargo test --release --test fuzz fuzz_gc_roots_long -- --ignored --nocapture
    ROOTFUZZ_SEEDS=17,42 ROOTFUZZ_KEEP=1 ...              # rerun seeds, keep target/rootfuzz/roots-SEED.lm

`ROOTFUZZ_PLAIN=1` builds without `--gc-check-roots`. The 18 smoke
seeds detect each of five seeded rooting bugs (let-else, match arms,
block scopes, `if let`, match guards).

## Differences from the design

- **Collection happens only at safepoints.** Allocation never collects;
  only functions with `uses gc` (such as `gc.collect`) do. So the only
  roots the compiler provides are traced locals in scope at those
  calls, and in vm code the traced state variables (copied at the
  call). Traced *temporaries* are not roots: bind a value to a local
  before a call that may collect. The exception is a value computed
  before defers run (a block's tail value, `return v`), which is rooted
  while they run.
- **`sync { ... }`** is a vm item not in the original design. It holds
  the code the compiler runs before an effectful call from a handler,
  for example publishing `fp` or the stack top for the GC's root
  scanner.
- **Cold blocks.** In tail-call mode, a `cold` block in an op handler
  that ends in control words moves to its own function with the
  handler's signature, reached with `musttail`. The hot path makes no
  call, so it needs no stack frame. Code after `if c { cold { .. } }`
  that ends in a control word is copied into the cold block so it can
  be outlined too. Other cold blocks, and all cold blocks in goto and
  switch mode, stay inline and are marked unlikely. How the outlined
  block gets the handler's values is in
  [lowering.md §3](lowering.md#3-handlers-in-tailcall-mode).
- **State variables** are plain types, stack and frame pointers
  included. Stack vms declare per-op stack effects (`language.md`
  §3.4a): the verifier checks depths and records the maximum, but the
  names are not bound in the handler, which moves its own stack
  pointer.
- **`unsafe`** is required for `p as Gc<T>` and `#[unsafe]` functions
  everywhere. Raw pointer dereference, indexing and arithmetic are
  allowed in modules without `#![safe]`. Pointer stability is checked in
  every module, std's included ([`design/lifetimes.md`](design/lifetimes.md));
  `#![lifetimes(off)]` opts a non-`#![safe]` module out, and no module
  in std, the examples, the tutorial or the benchmarks does. The full
  rules are in `language.md` §1.3.
- **Safe modules.** `std.lex`, `parse`, `diag`, `text`, `hash`, `io`,
  `bc`, `map` and `dict` are `#![safe]` ([`design/safety.md`](design/safety.md)).
  The example interpreters' only `unsafe` blocks are five zero-filled
  `Gc` arrays filled right after allocation. A checked register window
  was considered and not adopted ([roadmap, question 7](roadmap.md)).
- **`-> never` vm functions** are inlined into each call site. In
  tail-call mode, calls from `cold` blocks (and recursive calls)
  `musttail` to one shared copy that receives its arguments through
  thread-local slots. They can't use `become` or the caller's operands;
  the op's inline cache can be passed as a `c: cache Name` parameter.
- **Re-entry** (`language.md` §10): no whole `Vm.call` is generated.
  - The compiler provides the nested entry (declared `uses reenter`),
    the `reload` block after `reenter` calls in vm code, and `Vm.depth`.
  - The interpreter writes where the new frame goes, the sentinel that
    makes the callee's return leave the run, the depth limit (Mini:
    200, "C stack overflow"), and restoring its frame registers after
    the call, because only it knows its frame layout.
  - Derived pointers held across a `reenter` call (`let p = regs + a`)
    are not checked.
  - A nested run per callback costs about 20 ns on Mini. A native
    `table.sort` with a Lua comparator is 1.2x slower than the same sort
    written in Lua (`bench/reenter/`), so the Scheme and MiniJS preludes
    keep their higher-order builtins.
- **Effects** must be declared; they are not inferred for private
  functions. Handlers and plain vm functions are the exception: their
  effectful calls are synced in place. Entries declare what their vm
  code reaches (`uses gc` once any op may collect). There are no effect
  variables: a generic function can't take "a callback with any
  effects" and inherit them.
- **Verifier.** It checks opcodes, extents, operand `verify` clauses,
  superinstruction part words, stack depths for stack vms, and that code
  can't fall off its end. The fall-off scan is over tokens: an op may
  continue if its handler uses `next` or reads `pc`, so it errs toward
  "may continue". Mini, Scheme and MiniJS keep their code as a verified
  trailing array, so CALL reaches only verified code at no cost over
  raw code ([lowering.md §11](lowering.md#11-bounds-checks-and-verification)).
  Not covered:
  - the interpreter changing context fields after verification (they
    are ordinary fields);
  - return addresses (`jump fp.resume`), which are trusted to land on
    boundaries of verified code.
- **Instrumentation** tables are switched from vm code (`use_variant`).
  A debugger turns its variant on in the entries and `reload`.
- **`inline for`** is token-level expansion: values are token sequences,
  so operators like `+` work. In statement position the copies have no
  scope of their own. Inside a `vm`, a loop variable named like a state
  variable is an error. Errors in expanded code point at the source,
  but don't say which copy they come from.
- **Traits and closures** ([`design/closures-traits.md`](design/closures-traits.md)):
  - no `dyn`, associated types or constants, supertraits or orphan rule
    (overlapping impls are an error);
  - default bodies are checked when an instance uses them, not at the
    trait;
  - closures can't be created in vm code, and escaping closures copy
    their captures (assignable state goes in a state list).

## Limitations

- **Threads** share a heap one at a time (`std.gil`); there is no free
  threading.
- **The heap never moves objects** ([roadmap, question 2](roadmap.md)).
  There is no card marking, and small objects are promoted at their
  first survival.
- **The JIT** is x86-64 Linux only, for executables with one vm,
  tail-call dispatch and no variants. There is no trace compiler.
- **AArch64** is tested under qemu only, not on hardware.
- **Effects** are declared on every function, not inferred, and there
  is no effect polymorphism.
- **The verifier** doesn't cover context-field changes after
  verification or `jump fp.resume` targets.
