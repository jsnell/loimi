# Loimi

*Loimi* (Finnish: the warp threads on a loom) is a statically typed systems
language for **writing bytecode interpreters**. It compiles to LLVM bitcode and
is designed around a single goal: the code you write for an interpreter's inner
loop should compile to what an expert would write by hand in assembly, without
you having to write assembly, macros, or `__attribute__` soup.

Status: **working, pre-1.0**. The compiler, the dispatch strategies,
value representations, the GC, generated verifiers, instrumentation, C
embedding, a test runner and an opt-in copy-and-patch JIT work, and six
example interpreters are written in Loimi. See
[`docs/status.md`](docs/status.md) for what is built and how it differs
from the design.

**Results so far.** `examples/mini` implements a Lua 5.4 subset
([spec](examples/mini/SPEC.md)). On 14 benchmarks written clean-room
from that spec, it is faster than PUC-Lua 5.4 on all of them, taking
0.36–0.76× of Lua's time. It is also faster than LuaJIT's hand-written
assembly interpreter (`luajit -joff`) on 11 of them, and within 8% on
the other three.
See [`bench/mini/RESULTS.md`](bench/mini/RESULTS.md).

[`examples/minijs`](examples/minijs) implements a JavaScript subset on
the standard library's shape and inline-cache support. On its 14
clean-room benchmarks it is faster than V8's bytecode interpreter
(`node --jitless`: 0.53× of its time by geometric mean, 0.23–0.82× per
benchmark) and faster than QuickJS (0.40×), on every one ([`bench/minijs/RESULTS.md`](bench/minijs/RESULTS.md)).
[`examples/scheme`](examples/scheme) runs 41 unmodified programs from the
public r7rs-benchmarks suite. By geometric mean it takes 0.29× of Gauche's
time and 0.47× of Guile 3's time with Guile's JIT off, and is on par with
Guile with its JIT (1.02×)
([`bench/scheme/README.md`](bench/scheme/README.md)).

Three more examples were written by agents that read only the docs, as
usability studies (each has a `FRICTION.md` report):
[`examples/lox`](examples/lox) implements Lox from *Crafting
Interpreters*. It passes all 246 tests of the book's official suite and
takes 0.45× of the time of the book's C interpreter, clox, by geometric
mean over its official benchmarks (faster on all 10).
[`examples/regex`](examples/regex) is a backtracking regex engine shipped
as a C library, checked against Python's `re`.
[`examples/stack`](examples/stack) is a Forth-like stack VM using
declared stack effects.

`loimi build --jit-stencils` (x86-64 Linux) generates a copy-and-patch
JIT from the same `vm` declarations, with no extra interpreter source:
0.77–0.86 of the interpreter's time for Mini, MiniJS and Scheme
([`docs/design/copy-and-patch.md`](docs/design/copy-and-patch.md)).

## Getting started

Requirements: Rust (stable) and clang/LLVM 19.

```sh
cargo build --release
./target/release/loimi run tests/run/hello.lm          # compile and run
./target/release/loimi build examples/mini/main.lm --release -o mini
./mini examples/mini/tests/functions.mini              # the Lua-like example
./target/release/loimi build examples/mini/embed.lm --lib -o libmini.a --header mini.h
clang-19 -I. examples/embed/host.c libmini.a -lm -o host && ./host   # Mini embedded in C
cargo test                                             # all tests
tests/fuzz/run.sh -t 600 vm mini                       # bytecode fuzzing under ASan
bench/run.sh                                           # Loimi VM vs hand-written C
bench/lua/install.sh && bench/mini/compare.py          # Mini vs PUC-Lua / LuaJIT
./target/release/loimi test examples/lox/lox_test.lm   # #[test] functions
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
| Tracing and debugger hooks | A check in every handler, or a second copy of the interpreter | Instrumentation `variant`s: alternate dispatch tables, zero cost when off |

## Documents

- [`docs/tutorial/`](docs/tutorial/README.md): **start here**. A
  step-by-step tutorial that builds a small language from a calculator to
  a fast, garbage-collected, fuzzed bytecode interpreter, with working
  code for every step in [`examples/tutorial/`](examples/tutorial).
- [`docs/language.md`](docs/language.md): the language. The core is short;
  most of the document covers the interpreter-specific features (`value`, `vm`,
  effects, quickening, verification, GC integration, instrumentation).
- [`docs/lowering.md`](docs/lowering.md): how each feature lowers to LLVM IR,
  with IR sketches. Everything uses stock LLVM; there is no custom backend.
- [`docs/stdlib.md`](docs/stdlib.md): the standard library, aimed at the
  rest of an interpreter: lexing, parsing, diagnostics, bytecode building,
  GC, objects and shapes, strings, numbers, runtime support, and testing.
- [`docs/roadmap.md`](docs/roadmap.md): implementation plan, benchmarks
  that define success, and open questions.
- [`docs/status.md`](docs/status.md): implementation status and deviations
  from the design.
- [`examples/mini/`](examples/mini): a working interpreter for a small
  Lua-like language: lexer, single-pass compiler, register VM with
  NaN-boxed values, GC-managed strings and tables, generated verifier;
  also built as a C library ([`examples/embed/`](examples/embed)).
- [`examples/scheme/`](examples/scheme): an R7RS-small subset with proper
  tail calls, flat closures and a register VM.
- [`examples/minijs/`](examples/minijs): a JavaScript subset on shapes and
  inline caches from `std.obj`.
- [`examples/lox/`](examples/lox), [`examples/regex/`](examples/regex),
  [`examples/stack/`](examples/stack): written from the docs alone, with
  usability reports.
- [`bench/`](bench): the dispatch benchmark against hand-written C, and
  [`bench/mini`](bench/mini): Mini vs PUC-Lua and LuaJIT.

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
