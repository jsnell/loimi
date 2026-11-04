# Loimi

*Loimi* (Finnish: the warp threads on a loom) is a statically typed systems
language for **writing bytecode interpreters**. It compiles to LLVM bitcode and
is designed around a single goal: the code you write for an interpreter's inner
loop should compile to what an expert would write by hand in assembly, without
you having to write assembly, macros, or `__attribute__` soup.

You declare the interpreter (its state, instruction formats, value
representation, ops, slow paths and inline caches) and the compiler
generates the dispatch loop in the strategy you pick at build time, plus
the tools that must agree with it: encoders, a disassembler, a bytecode
verifier, a superinstruction peephole pass, GC tracing and even a
copy-and-patch JIT.

Status: **working, pre-1.0**. Eight example interpreters, from a Lua
subset to Python and WebAssembly, are written in Loimi, and most take
less time than the established interpreters they are compared with. See
[`docs/status.md`](docs/status.md) for what is built and how it
differs from the design, and [`CHANGELOG.md`](CHANGELOG.md) for what is new.

## Why a new language?

Every serious interpreter written in C or C++ ends up rebuilding the same
infrastructure out of macros, compiler extensions, and convention:

| Problem | What people do today | What Loimi does |
|---|---|---|
| Fast dispatch | `switch`, computed goto (GCC extension), `musttail` + `preserve_none` (CPython 3.14), hand-written asm (LuaJIT), custom DSL (JSC's offlineasm) | `vm` declarations; dispatch strategy is a build flag (tail-call, goto, switch) over the same source |
| Keep `pc`/`sp`/frame in registers | Hope the register allocator cooperates in one giant function | Declared `state`, pinned to registers via the calling convention |
| Slow paths ruining hot paths | `UNLIKELY()`, manual outlining, `noinline` | `cold { }` blocks, outlined so that the hot path makes no call and keeps its registers |
| "Save pc before anything that can error or GC" | Lua's `Protect()`/`savepc`, CPython's frame sync. Easy to forget, and forgetting causes heisenbugs | **Effects** (`sync`, `gc`, `reenter`); the compiler inserts state syncs and rejects calls that would miss one |
| Instruction encoding | Hand-written bit shifts, plus a separate disassembler that drifts out of sync | Declared formats; encoders, decoders, emitters, disassembler and verifier are generated (`std.bc` builds code over them) |
| Value representation | Hand-rolled NaN-boxing / pointer tagging macros | `value` types with a declared `repr`; `match` compiles to optimal bit tests, including multi-value checks |
| Quickening and inline caches | Ad-hoc bytecode rewriting | `family` ops with `become`, typed inline `cache` words |
| Superinstructions | Rarely done, because it's painful | `super A = X; Y;`, plus `--vm-stats`, which counts op pairs and suggests `super` declarations |
| Untrusted bytecode | Either trust it or bounds-check everything in the hot loop | Generated verifier; verified code runs without operand bounds checks, and `#[require_verified]` makes the entries accept only verified code |
| GC roots | Manual `PUSH_ROOT`/handle scopes, conservative stack scanning | Collection only at calls marked `uses gc`; the compiler roots traced locals and vm state there, and `--gc-check-roots` checks every root |
| Tracing and debugger hooks | A check in every handler, or a second copy of the interpreter | Instrumentation `variant`s: alternate dispatch tables, zero cost when off |

## A taste

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

With `--dispatch=tailcall` (the default on x86-64 and AArch64), each `op`
becomes its own LLVM function using the `preserve_none` calling convention, and
every `next` becomes a `musttail` call through the dispatch table. `pc`,
`regs`, `k`, and `ctx` stay in machine registers across the whole interpreter.
The `(Int, Int)` check compiles to one xor/or/shift/compare. The `cold` block is
moved to its own function and reached with a tail call, so the hot path
makes no call and needs no stack frame.

## Example interpreters

Each has its own tests, run by `cargo test`. Ratios are run time over the reference's, geometric means over
each suite, tail-call builds; every number, for every dispatch mode and
the JIT, is in [`docs/results.md`](docs/results.md) (made with
`bench/run_all.py`).

| Example | What it is | Headline |
|---|---|---|
| [`mini`](examples/mini) | A Lua 5.4 subset ([spec](examples/mini/SPEC.md)) with coroutines that switch inside the dispatch loop ([notes](examples/mini/COROUTINES.md)); also a C library ([`examples/embed`](examples/embed)) | 0.54× PUC-Lua 5.4, 0.85× LuaJIT's assembly interpreter (`-joff`), on 14 clean-room benchmarks; coroutines 0.41× PUC-Lua ([log](bench/mini/RESULTS.md)) |
| [`minijs`](examples/minijs) | A JavaScript subset on `std.obj` shapes and inline caches | 0.52× V8's interpreter (`node --jitless`), 0.40× QuickJS |
| [`scheme`](examples/scheme) | An R7RS-small subset with proper tail calls, flat closures and a generational heap | 43 unmodified r7rs-benchmarks programs: 0.26× Gauche, 0.46× Guile 3 without JIT, 0.97× Guile with its JIT ([notes](bench/scheme/README.md)) |
| [`lox`](examples/lox) | Lox from *Crafting Interpreters* | All 246 tests of the book's suite; 0.45× clox |
| [`regex`](examples/regex) | A backtracking regex engine shipped as a C library | 0.44× the PCRE2 interpreter, 0.41× Python's `re` |
| [`stack`](examples/stack) | Stak, a Forth with statically checked stack effects | (no reference) |
| [`wasm`](examples/wasm) | A WebAssembly interpreter (MVP plus reference types, bulk memory, multi-value, SIMD) | Every spec test file passes (SIMD's 57 included); 0.91× wasm3 |
| [`python`](examples/python) | A Python 3 subset: classes, generators, exceptions, bigints, dicts ([spec](examples/python/SPEC.md)) | 11 pyperformance programs with CPython's output: 0.52× CPython 3.13, 0.18× MicroPython ([report](examples/python/FRICTION.md)) |
| [`prolog`](examples/prolog) | An ISO Prolog subset on a WAM ([spec](examples/prolog/SPEC.md)) | The 30 van Roy / Aquarius programs of SWI-Prolog's bench suite: 0.63× SWI-Prolog 10 (`-O`), 0.15× Trealla ([report](examples/prolog/FRICTION.md)) |

Lox, regex, stack, wasm, python and prolog were written by agents that read only
the docs, as usability studies; each has a `FRICTION.md` report.
`loimi build --jit-stencils` turns the same `vm` source into a
copy-and-patch JIT: 0.75–0.86 of the interpreter's time for Mini, MiniJS
and Scheme ([design](docs/design/copy-and-patch.md)).

## What is in it

**`vm` declarations** ([language.md §3](docs/language.md#3-the-vm-declaration), [§7](docs/language.md#7-specialization-quickening-inline-caches-superinstructions)):
`state` pinned in registers, operand kinds with `verify` clauses,
fixed-width formats, ops ending in control words (`next`, `jump`,
`exit`, `raise`), vm functions (`next_pc`, `then_next_pc`), entries,
nested entries for natives that call guest code, and resume points
(`Vm.Resume`) for coroutines and generators. Generated from them:
encoders, emitters, per-format readers, setters and encoders, a
disassembler, the verifier and its `Verified<Vm>` proof type.
Specialization: inline `cache`s, quickening with `family`/`become`,
`then OP`, `super` instructions (with `#[forward]` register forwarding),
accumulator registers (`#[accumulate]`), stack effects for stack VMs,
and instrumentation `variant`s.

**Dispatch and code generation** ([§3.7](docs/language.md#37-dispatch-strategies), [lowering.md](docs/lowering.md)):
`--dispatch=tailcall` (`preserve_none` + `musttail`, the default),
`goto` (one shared dispatch block copied into each handler) or `switch`
over the same source. `cold` blocks are outlined; effects (`uses sync`,
`gc`, `reenter`) make the compiler sync state before calls that need
it. Handlers are kept free of stack frames (no `memcpy` formation or loop
vectorization in them), and `-W handler_frame` reports those that still
set one up. Targets: x86-64 and AArch64 Linux (`--target`), for the
baseline CPU or a newer one (`--cpu=x86-64-v3`, `native`).

**Values** ([§2](docs/language.md#2-value-types-declared-representations)):
`value` types with a declared `repr` (`nanbox`, `lowtag`, `boxed`);
`match`, also on tuples of values, compiles to a few bit tests.
Payloads up to 48 bits, and up to 56 `Gc` variants in a nanbox by
sharing NaN blocks. `--explain-values` prints the chosen layout,
`block N` pins it, and the `value_layout` warning says when adding a
variant would change it.

**GC** ([§8](docs/language.md#8-gc-integration), [stdlib/gc.md](docs/stdlib/gc.md)):
`object` types and `Gc<T>`, tracing generated by the compiler, a
mark-sweep collector written in Loimi (`std.gc`) with an optional
generational mode (sticky marks, mark bitmaps and no sweep, compiler-emitted write
barriers). Collection happens only in calls marked `uses gc`; the
compiler roots traced locals and vm state there. Weak references,
ephemerons, trace and free hooks. Testing aids: GC stress, a verify mode
for write barriers, `--gc-check-roots`, and a rooting fuzzer.

**Embedding and threads** ([§10.1](docs/language.md#101-embedding-in-a-c-host-implemented)):
`--lib`/`--shared` with `export "C"` functions and a generated C header,
panics caught at the boundary, independent interpreter states on
separate threads (runtime state is thread-local; `--sanitize=thread`),
and threads sharing one heap behind a lock (`std.gil`). `--cfg NAME`
with `#[cfg(..)]` and pointer atomics.

**Debugging guest programs** ([§9.1](docs/language.md#91-breakpoints-on-break), [design](docs/design/debugging.md)):
generated breakpoint ops patched over verified code, at no cost when not
debugging, with `std.debug` for stepping and breakpoints. Mini's debug
build is debuggable from editors: a stop-callback C API and `mini-dap`,
a Debug Adapter Protocol server (`examples/mini/dap`), with conditional
breakpoints, break on error, pause and evaluation.

**Core language** ([§1](docs/language.md#1-core-language)): structs,
enums, generics with static traits, `iN`/`uN` of any width, pattern
matching (with constants as patterns), non-escaping and escaping
closures, `defer`, `inline for`, float and wide-integer built-ins
(`mul_wide`, `div_wide`), unaligned loads and stores, raw pointers.

**Standard library** ([stdlib.md](docs/stdlib.md)): lexing (`std.lex`),
Pratt parsing (`std.parse`), diagnostics (`std.diag`), exact number
parsing and formatting (`std.num`), bytecode building (`std.bc`),
hidden classes and inline caches (`std.obj`), the GC (`std.gc`,
`std.gil`), arbitrary-precision integers (`std.bigint`), hash and
insertion-ordered maps (`std.map`, `std.dict`), strings (`std.text`),
collections and allocators (`std.mem`), non-local exits (`std.rt`),
tests (`std.test`) and timing (`std.prof`).

**Tooling**: `loimi test` runs `#[test]` functions; `--vm-stats` counts
ops, op sequences (printed as suggested `super` declarations) and
entries into cold blocks; `--sanitize=address`; a bytecode fuzzer for
verified code (`tests/fuzz/run.sh`); `bench/run_all.py` runs every
example against its references.

## Getting started

Requirements: Rust (stable) and clang/LLVM 19, on x86-64 Linux (AArch64
Linux is a cross target, below).

```sh
cargo build --release
./target/release/loimi run tests/run/hello.lm          # compile and run
./target/release/loimi build examples/mini/main.lm --release -o mini
./mini examples/mini/tests/functions.mini              # the Lua-like example
./target/release/loimi build examples/mini/embed.lm --lib -o libmini.a --header mini.h
clang-19 -I. examples/embed/host.c libmini.a -lm -o host && ./host   # Mini embedded in C
./target/release/loimi test examples/lox/lox_test.lm   # #[test] functions
./target/release/loimi --help                          # every build option
```

Testing and benchmarking:

```sh
cargo test --release                                   # everything, including the doc snippets
tests/fuzz/run.sh -t 600 vm mini                       # bytecode fuzzing under ASan
bench/run.sh                                           # Loimi VM vs hand-written C
bench/lua/install.sh && bench/mini/compare.py          # Mini vs PUC-Lua / LuaJIT
bench/run_all.py --quick                               # every example vs its references
```

Cross-compiling for AArch64 Linux (links with `ld.lld`; tested under qemu
user-mode, built from source with an extracted Debian sysroot, no root):

```sh
bench/cross/install.sh                                 # qemu-aarch64 + sysroot
S=$PWD/bench/cross/sysroot-aarch64
./target/release/loimi build examples/mini/main.lm --release --target=aarch64-linux-gnu --sysroot=$S -o mini-arm
bench/cross/install/bin/qemu-aarch64 -L $S ./mini-arm examples/mini/tests/functions.mini
LOIMI_RUNNER="$PWD/bench/cross/install/bin/qemu-aarch64 -L $S" \
  ./target/release/loimi run tests/run/hello.lm --target=aarch64-linux-gnu --sysroot=$S
```

`cargo test` runs a subset under qemu when the toolchain is installed
(`aarch64_qemu`); `LOIMI_TARGET`, `LOIMI_SYSROOT` and `LOIMI_RUNNER` run the
whole suite cross-compiled (see `cross()` in `tests/golden.rs`).

## Where to read next

- [`docs/tutorial/`](docs/tutorial/README.md): **start here**. Builds a
  small language from a calculator to a fast, garbage-collected, fuzzed
  bytecode interpreter, with working code for every step in
  [`examples/tutorial/`](examples/tutorial).
- [`docs/language.md`](docs/language.md): the language. The core is
  short; most of it covers the interpreter-specific features.
- [`docs/stdlib.md`](docs/stdlib.md): the standard library, with
  [`stdlib/gc.md`](docs/stdlib/gc.md) and [`stdlib/obj.md`](docs/stdlib/obj.md).
- [`docs/lowering.md`](docs/lowering.md): how each feature lowers to LLVM
  IR, and what it costs. Everything uses stock LLVM.
- [`docs/status.md`](docs/status.md): what is built and how it differs
  from the design; [`CHANGELOG.md`](CHANGELOG.md): what is new.
- [`docs/results.md`](docs/results.md): benchmark results;
  [`docs/roadmap.md`](docs/roadmap.md): plan and open questions.
- The examples above, and their `FRICTION.md` usability reports.
