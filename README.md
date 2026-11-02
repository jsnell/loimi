# Loimi

*Loimi* (Finnish: the warp threads on a loom) is a statically typed systems
language for **writing bytecode interpreters**. It compiles to LLVM bitcode and
is designed around a single goal: the code you write for an interpreter's inner
loop should compile to what an expert would write by hand in assembly, without
you having to write assembly, macros, or `__attribute__` soup.

Status: **early implementation**. The compiler, the dispatch strategies,
value representations, the GC integration, generated verifiers and
instrumentation work, and a Lua-like interpreter written in Loimi runs on
them. See [`docs/status.md`](docs/status.md) for what is built and how it
differs from the design.

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
public r7rs-benchmarks suite. By geometric mean it takes 0.39× of Gauche's
time and 0.62× of Guile 3's time with Guile's JIT off. Guile with its JIT
is faster: we take 1.39× of its time
([`bench/scheme/README.md`](bench/scheme/README.md)).

## Getting started

Requirements: Rust (stable) and clang/LLVM 19.

```sh
cargo build --release
./target/release/loimi run tests/run/hello.lm          # compile and run
./target/release/loimi build examples/mini/main.lm --release -o mini
./mini examples/mini/tests/functions.mini              # the Lua-like example
cargo test                                             # all tests
tests/fuzz/run.sh -t 600 vm mini                       # bytecode fuzzing under ASan
bench/run.sh                                           # Loimi VM vs hand-written C
bench/lua/install.sh && bench/mini/compare.py          # Mini vs PUC-Lua / LuaJIT
```

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
| Untrusted bytecode | Either trust it or bounds-check everything in the hot loop | Generated verifier; verified code runs without operand bounds checks (nothing yet forces `run` to take verified code) |
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
  NaN-boxed values, GC-managed strings and tables, generated verifier.
- [`examples/scheme/`](examples/scheme): an R7RS-small subset with proper
  tail calls, flat closures and a register VM.
- [`bench/`](bench): the dispatch benchmark against hand-written C, and
  [`bench/mini`](bench/mini): Mini vs PUC-Lua and LuaJIT.

## A taste

```loimi
vm Mini {
    context ctx: *Interp;

    state {
        pc:   Cursor<u32>,      // pinned in a register
        regs: Window<Value>,    // pinned in a register
        k:    *const Value,     // pinned in a register
    }

    operand reg   = regs[it]  verify it < proto.nregs;
    operand kst   = k[it]     verify it < proto.consts.len;
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
}
```

With `--dispatch=tailcall` (the default on x86-64 and AArch64), each `op`
becomes its own LLVM function using the `preserve_none` calling convention, and
every `next` becomes a `musttail` call through the dispatch table. `pc`,
`regs`, `k`, and `ctx` stay in machine registers across the whole interpreter.
The `(Int, Int)` check compiles to one xor/or/shift/compare. The `cold` block is
outlined behind a `preserve_most` call, so the hot path never spills.
