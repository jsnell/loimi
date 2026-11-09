# Loimi

Loimi is a statically typed systems language for **writing bytecode
interpreters**. It compiles to LLVM. Its goal is that the inner loop of
your interpreter compiles to the code an expert would write by hand in
assembly, without you writing assembly, macros or compiler-specific
attributes.

You describe the interpreter: its state, instruction formats, value
representation, ops, slow paths and inline caches. The compiler then
generates the dispatch loop, using the strategy you pick at build time.
It also generates the tools that must agree with that loop:

- instruction encoders and a disassembler
- a bytecode verifier
- a peephole pass that forms superinstructions
- GC tracing code
- a copy-and-patch JIT

**AI disclosure:** All the code, and 99% of this README, was written by
AI. There was at best minimal human oversight.

**Status:** working and extensively tested, but I don't expect anyone
else to use this. (Just vibe-code your own if you like the concept.)

## Why a new language?

Every serious interpreter written in C or C++ ends up rebuilding the
same machinery out of macros, compiler extensions and convention.
Loimi makes each piece part of the language.

| Problem | What people do today | What Loimi does |
|---|---|---|
| Fast dispatch | `switch`, computed goto (a GCC extension), `musttail` + `preserve_none` (CPython 3.14), hand-written assembly (LuaJIT), a custom DSL (JSC's offlineasm) | `vm` declarations. The dispatch strategy (tail calls, goto or switch) is a build flag over the same source |
| Keeping `pc`, `sp` and the frame in registers | Hope the register allocator copes with one giant function | Declared `state`, kept in registers by the calling convention |
| Slow paths slowing down hot paths | `UNLIKELY()`, manual outlining, `noinline` | `cold { }` blocks, moved out so the hot path makes no call and keeps its registers |
| "Save `pc` before anything that can fail or collect garbage" | Lua's `Protect()`, CPython's frame sync. Easy to forget, and forgetting causes bugs that are hard to reproduce | **Effects** (`sync`, `gc`, `reenter`). The compiler saves state where needed and rejects calls that would miss it |
| Instruction encoding | Hand-written bit shifts, and a separate disassembler that drifts out of date | Declared formats. Encoders, decoders, disassembler and verifier are generated |
| Value representation | Hand-written NaN-boxing or pointer-tagging macros | `value` types with a declared representation. `match` compiles to minimal bit tests, also on two values at once |
| Quickening and inline caches | Ad-hoc bytecode rewriting | `family` ops with `become`, and typed inline `cache` words |
| Superinstructions | Rarely done, because it's painful | `super A = X; Y;`. `--vm-stats` counts op pairs and suggests `super` declarations |
| Untrusted bytecode | Trust it, or bounds-check everything in the hot loop | A generated verifier. Verified code runs without operand bounds checks |
| GC roots | Manual root pushing or handle scopes, or conservative stack scanning | Collection happens only at calls marked `uses gc`, where the compiler roots locals and vm state. `--gc-check-roots` checks every root |
| Tracing and debugger hooks | A check in every handler, or a second copy of the interpreter | Instrumentation `variant`s: alternate dispatch tables that cost nothing when off |

## A taste

Part of a small register-based vm: an add with fast paths for integers
and floats and a slow path for everything else, a compare-and-branch,
and a return.

<!-- snippet: check
value Value repr nanbox { Num(f64) unboxed, Int(i32), Nil, Bool(bool) }
enum Arith { Add, Sub }
struct Proto { nregs: u32, nconsts: u64 }
struct Interp { proto: *Proto }
fn arith(c: *Interp, op: Arith, a: Value, b: Value) -> Result<Value, str> { Err("bad operands") }
fn less_than(a: Value, b: Value) -> Result<bool, str> { Ok(false) }
fn is_insn_boundary(p: *u32) -> bool { true }
{snippet}
-->
```loimi
vm Mini {
    context ctx: *Interp;

    state {
        pc:   *u32,             // pinned in a register
        regs: *Value,           // pinned in a register
        k:    *Value,           // pinned in a register
    }

    verify_context proto: *Proto;      // what the generated verifier checks against
    operand reg   = regs[it]  verify (it as u32) < proto.nregs;
    operand kst   = k[it]     verify it < proto.nconsts;
    operand label = pc + it   verify is_insn_boundary(pc + it);

    encoding fixed(u32) {
        format ABC  { op: 8, a: 8, b: 8, c: 8 }
        format ABsC { op: 8, a: 8, b: 8, sc: s8 }
    }

    op ADD(dst: reg, a: reg, b: reg) : ABC {
        match (a, b) {
            (Int(x), Int(y)) => if let Some(r) = x.add_checked(y) { dst = Int(r); next },
            (Num(x), Num(y)) => { dst = Num(x + y); next },
            _ => {},
        }
        cold { dst = arith(ctx, .Add, a, b)?; next }
    }

    op JLT(a: reg, b: reg, target: label) : ABsC {
        if less_than(a, b)? { jump target } else { next }
    }

    op RET(a: reg) : ABC { exit Ok(a) }

    entry run(c: *Interp, code: *u32, r: *Value, kp: *Value) -> Result<Value, str> {
        ctx = c; regs = r; k = kp;
        jump code
    }

    on raise(msg: str) { exit Err(msg) }   // where `?` on an Err lands
}
```

With `--dispatch=tailcall`, the default on x86-64 and AArch64:

- Each `op` becomes its own LLVM function with the `preserve_none`
  calling convention, and each `next` becomes a `musttail` call through
  the dispatch table.
- `pc`, `regs`, `k` and `ctx` stay in machine registers across the
  whole interpreter.
- The `(Int, Int)` check compiles to one xor, or, shift and compare.
- The `cold` block becomes a separate function reached by a tail call,
  so the hot path makes no call and needs no stack frame.

## Example interpreters

The repository has nine example interpreters, each with its own tests.
The numbers are run time divided by the reference implementation's run
time (lower is better), as a geometric mean over each benchmark suite,
for tail-call builds. All results, for every dispatch mode and the JIT,
are in [`docs/results.md`](docs/results.md).

| Example | What it is | Performance |
|---|---|---|
| [`mini`](examples/mini) | A Lua 5.4 subset ([spec](examples/mini/SPEC.md)) with coroutines, also usable as a C library ([`examples/embed`](examples/embed)) | 0.49× PUC-Lua 5.4 and 0.75× LuaJIT's assembly interpreter (`-joff`) on 14 clean-room benchmarks; coroutines 0.33× PUC-Lua ([log](bench/mini/RESULTS.md)) |
| [`minijs`](examples/minijs) | A JavaScript subset with hidden classes and inline caches | 0.48× V8's interpreter (`node --jitless`), 0.37× QuickJS |
| [`scheme`](examples/scheme) | An R7RS-small subset with proper tail calls, flat closures and a generational heap | On 43 unmodified r7rs-benchmarks programs: 0.22× Gauche, 0.38× Guile 3 without JIT, 0.80× Guile with its JIT ([notes](bench/scheme/README.md)) |
| [`lox`](examples/lox) | Lox from *Crafting Interpreters* | Passes all 246 tests of the book's suite; 0.42× clox |
| [`regex`](examples/regex) | A backtracking regex engine, shipped as a C library | 0.44× the PCRE2 interpreter, 0.41× Python's `re` |
| [`stack`](examples/stack) | Stak, a Forth with statically checked stack effects | (no reference) |
| [`wasm`](examples/wasm) | A WebAssembly interpreter: MVP plus reference types, bulk memory, multi-value and SIMD | Passes every spec test file (SIMD's 57 included); 0.85× wasm3 on seven programs, 0.82× on 30 held-out PolyBench/C kernels |
| [`python`](examples/python) | A Python 3 subset with classes, generators, exceptions, bigints and dicts ([spec](examples/python/SPEC.md)) | On 11 pyperformance programs: 0.31× CPython 3.13, 0.11× MicroPython |
| [`prolog`](examples/prolog) | An ISO Prolog subset on a WAM ([spec](examples/prolog/SPEC.md)) | On the 30 van Roy / Aquarius programs from SWI-Prolog's suite: 0.47× SWI-Prolog 10 (`-O`), 0.11× Trealla |

Lox, regex, stack, wasm, python and prolog were written by AI agents
that read only the documentation, as usability studies. Each has a
`FRICTION.md` report on what was hard.

`loimi build --jit-stencils` turns the same `vm` source into a
copy-and-patch JIT. For Mini, MiniJS and Scheme it runs in 0.76–0.83 of
the interpreter's time.

## What is in it

**Interpreter features**

- `vm` declarations: state kept in registers, operands with `verify`
  rules, fixed-width instruction formats, ops, entry points, and resume
  points for coroutines and generators.
- Generated from each `vm`: encoders, a disassembler, and a verifier
  whose `Verified<Vm>` type proves code was checked.
- Specialization: inline caches, quickening (`family` and `become`),
  superinstructions (`super`), accumulator registers and stack effects
  for stack machines.
- Instrumentation `variant`s, such as tracing or step limits, at no
  cost when off.
- Breakpoints patched over verified code, with `std.debug` for
  stepping. Mini's debug build includes `mini-dap`, a Debug Adapter
  Protocol server for editors.

**Code generation**

- Three dispatch strategies over the same source: `tailcall` (the
  default), `goto` and `switch`.
- `cold` blocks moved out of the hot path, and effects (`uses sync`,
  `gc`, `reenter`) that make the compiler save state before calls that
  need it.
- Handlers kept free of stack frames; `-W handler_frame` reports those
  that still set one up.
- Targets: x86-64 and AArch64 Linux, for a baseline or newer CPU
  (`--cpu=x86-64-v3`, `native`).
- A copy-and-patch JIT built from the same source (`--jit-stencils`).

**Values and memory**

- `value` types with a declared representation: `nanbox`, `lowtag` or
  `boxed`. `--explain-values` prints the chosen layout.
- `object` types and `Gc<T>`, with tracing code generated by the
  compiler.
- A mark-sweep collector written in Loimi (`std.gc`), with an optional
  generational mode and compiler-generated write barriers.
- Weak references, ephemerons, and trace and free hooks.
- GC testing aids: stress mode, write-barrier checking,
  `--gc-check-roots` and a rooting fuzzer.

**Embedding**

- Static or shared C libraries (`--lib`, `--shared`) with `export "C"`
  functions and a generated C header. Panics are caught at the
  boundary.
- Independent interpreters on separate threads, or threads sharing one
  heap behind a lock (`std.gil`).

**Core language**

Structs, enums, generics with static traits, integers of any width,
pattern matching, closures, `defer`, `inline for`, raw pointers, and
built-ins for floats and wide integers.

**Standard library**

| Area | Modules |
|---|---|
| Front end | `std.lex` (lexing), `std.parse` (Pratt parsing), `std.diag` (error messages), `std.num` (exact number parsing and printing) |
| Back end | `std.bc` (building bytecode), `std.obj` (hidden classes and inline caches) |
| Memory | `std.gc`, `std.gil`, `std.mem` (collections and allocators) |
| Data | `std.bigint`, `std.map`, `std.dict` (insertion-ordered), `std.text` |
| Other | `std.rt` (non-local exits), `std.test`, `std.prof` (timing) |

**Tools**

- `loimi test` runs `#[test]` functions.
- `--vm-stats` counts ops, common op sequences (printed as suggested
  `super` declarations) and how often cold blocks run.
- `--sanitize=address` and `--sanitize=thread`.
- A bytecode fuzzer for verified code.

## Getting started

You need Rust (stable) and clang/LLVM 19, on x86-64 Linux.

```sh
cargo build --release
./target/release/loimi run tests/run/hello.lm          # compile and run
./target/release/loimi --help                          # every build option
```

Build and run Mini, the Lua-like example:

```sh
./target/release/loimi build examples/mini/main.lm --release -o mini
./mini examples/mini/tests/functions.mini
```

Build Mini as a C library and embed it in a C program:

```sh
./target/release/loimi build examples/mini/embed.lm --lib -o libmini.a --header mini.h
clang-19 -I. examples/embed/host.c libmini.a -lm -o host && ./host
```

Run a file's `#[test]` functions:

```sh
./target/release/loimi test examples/lox/lox_test.lm
```

### Cross-compiling for AArch64

AArch64 Linux is a cross target. It links with `ld.lld` and is tested
under qemu user mode. The install script builds qemu from source and
extracts a Debian sysroot; it needs no root access.

```sh
bench/cross/install.sh                                 # qemu-aarch64 + sysroot
S=$PWD/bench/cross/sysroot-aarch64
./target/release/loimi build examples/mini/main.lm --release --target=aarch64-linux-gnu --sysroot=$S -o mini-arm
bench/cross/install/bin/qemu-aarch64 -L $S ./mini-arm examples/mini/tests/functions.mini
LOIMI_RUNNER="$PWD/bench/cross/install/bin/qemu-aarch64 -L $S" \
  ./target/release/loimi run tests/run/hello.lm --target=aarch64-linux-gnu --sysroot=$S
```

## Where to read next

- [The tutorial](docs/tutorial/README.md): **start here**. It builds a
  small language step by step, from a calculator to a fast,
  garbage-collected bytecode interpreter, with working code for every
  step.
- [The language reference](docs/language.md): every language feature.
  The core is short; most of it covers the interpreter features.
- [The standard library](docs/stdlib.md).
- [Benchmark results](docs/results.md).
- [How features compile to LLVM IR](docs/lowering.md), and what they
  cost. Everything uses stock LLVM.
- [What is built](docs/status.md) and [what changed](CHANGELOG.md).

## Developing Loimi

This section is for people working on the compiler itself.

### Running the tests

The tests come in three tiers. Times are for `RUST_TEST_THREADS=2` on an
8-core machine.

| Tier | Command | Time | When to run it |
|---|---|---|---|
| Fast | `LOIMI_TEST_FAST=1 cargo test --release` | about 2 minutes | While iterating |
| Default | `cargo test --release` | about 5 minutes | Before every merge |
| Heavy | `cargo test --release -- --include-ignored` | longer | Before merging changes to code generation, dispatch, the JIT, the GC, the C ABI, or anything target-specific |

- **Fast** runs the compiler's own tests (`tests/run`, `tests/fail`,
  `tests/test`, IR and C-library tests), Lox, Mini's scripts and
  conformance tests, the fuzz smoke runs and the doc snippets. Tests
  that build the other interpreters are skipped, with a note on stderr.
- **Default** adds every example in each dispatch mode, the GC modes,
  the JIT and debugger builds, and the C hosts.
- **Heavy** adds the tests marked `#[ignore = "heavy ..."]`: AArch64
  under qemu, the debugger sessions in the goto and switch builds, the
  Mini C hosts against every library build, ThreadSanitizer, and two
  minutes of GC rooting fuzzing. ASan and TSan tests run only without
  `ulimit -v`.

To run the whole suite cross-compiled for AArch64, set `LOIMI_TARGET`,
`LOIMI_SYSROOT` and `LOIMI_RUNNER` (see `cross()` in `tests/golden.rs`).

### Test build settings

| Setting | Effect |
|---|---|
| `LOIMI_TEST_BRANCH_ALIGN=1` | Keep x86-64 branch padding in every test build. By default, test builds of interpreters drop it (`--no-branch-align`), because it takes most of clang's time on a large vm and only moves code. Mini's scripts and conformance tests and `tests/run` always keep it. |
| `LOIMI_TEST_JOBS=N` | How many builds and programs one test runs at once (default 3). Builds are shared between tests within a run. |
| `LOIMI_TEST_TIMES=FILE` | Log the time of each build to `FILE`. |

To time individual tests:

```sh
RUSTC_BOOTSTRAP=1 cargo test --release --test golden -- -Z unstable-options --report-time NAME
```

### Fuzzing and benchmarks

```sh
tests/fuzz/run.sh -t 600 vm mini                       # bytecode fuzzing under ASan
bench/run.sh                                           # Loimi VM vs hand-written C
bench/lua/install.sh && bench/mini/compare.py          # Mini vs PUC-Lua and LuaJIT
bench/run_all.py --quick                               # every example vs its references
```
