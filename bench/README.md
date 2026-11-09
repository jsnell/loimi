# Benchmarks

Benchmark programs, reference interpreters and timing scripts for Loimi.
The latest numbers for every example interpreter are in
[`docs/results.md`](../docs/results.md), along with the machine, the
method and how to reproduce them with `bench/run_all.py`.

## What is here

Each example interpreter has a suite of programs and a script that times
it against one or more established interpreters for the same language.

| Directory | Interpreter | Programs | References |
|---|---|---|---|
| [`mini/`](mini/README.md) | `examples/mini` (Lua subset) | 14 clean-room programs, 3 coroutine programs | PUC-Lua, LuaJIT (`bench/lua/`) |
| [`minijs/`](minijs/README.md) | `examples/minijs` (JavaScript subset) | 14 clean-room programs | `node --jitless`, QuickJS (`bench/js/`) |
| [`scheme/`](scheme/README.md) | `examples/scheme` | r7rs-benchmarks | Gauche, Guile |
| `lox/` | `examples/lox` | the *Crafting Interpreters* benchmarks | clox |
| [`wasm/`](wasm/README.md) | `examples/wasm` | C programs compiled to wasm, PolyBench/C | wasm3 |
| [`python/`](python/README.md) | `examples/python` | pyperformance programs | CPython, MicroPython |
| [`prolog/`](prolog/README.md) | `examples/prolog` | van Roy / Aquarius programs | SWI-Prolog, Trealla |

The regex and Stak suites live with their examples
(`examples/regex/bench.py`, `examples/stack/bench/run.py`).

The rest of this directory holds microbenchmarks of single features:

| Directory | Measures |
|---|---|
| `fib.lm`, `fib.c`, `run.sh` | Loimi against the same VM written by hand in C (below) |
| `sort/` | sort comparators as closures and function pointers (below) |
| [`obj/`](obj/README.md) | the cost of a `std.obj` inline-cache hit |
| `reenter/` | the cost of calling back into Mini from native code (results in `run.sh`) |
| `bigint/` | `std.bigint` against CPython's integers |
| `cross/` | not a benchmark: an AArch64 sysroot and qemu, for building and running aarch64 binaries (`install.sh`) |

## Loimi against hand-written C

`run.sh` builds `fib.lm` in each dispatch mode, and `fib.c`, the same VM
written by hand in C with computed goto, NaN-boxing and overflow
builtins. It reports the best of 7 runs.

```sh
bench/run.sh
```

The workload is recursive `fib(32)` as bytecode on a dynamically typed
register VM. That is about 7 million guest calls, with every arithmetic
op checking its operand types and taking an overflow-checked integer
path.

Results on an Intel i7-6700 with clang 19, 2026-11-01:

| Version | Time |
|---|---|
| Loimi, `--dispatch=goto` | 56 ms |
| C, computed goto | 62 ms |
| Loimi, `--dispatch=tailcall` | 67 ms |
| Loimi, `--dispatch=switch` | 72 ms |

Later the same day, after cold blocks were moved out of line, the same
benchmark measured tail-call 70 ms, goto 60 ms, C 67 ms and switch
75 ms. Expect about 5% drift between runs on this machine.

- The generated handlers are as tight as the C ones. `pc`, `regs`, the
  frame pointer and the context stay in registers, the `(Int, Int)` check
  is one xor/or/shift test, and slow paths sit out of line.
- On a VM this small, goto mode (one function with a replicated
  `indirectbr`) wins. Constants such as the NaN-box tag stay in registers
  for the whole loop, while each tail-call handler loads them again.

## Sort comparators: closures and function pointers

`sort/run.sh` sorts 2M `u64` values by `x ^ salt` with the merge sort in
`std.mem.sort_by`, passing the comparator in several ways (see
`sort/sort.lm`). The comparison is cheap and the memory accesses are
sequential, so call overhead shows.

```sh
bench/sort/run.sh
```

Best of 7, CPU time of the sort only, on an i7-6700 with clang 19,
2026-11-02, with other jobs running on the machine:

| Comparator | Time |
|---|---|
| comparison written into the sort (the floor) | 169 ms |
| `mem.sort_by` and a closure capturing `salt` | 169 ms |
| `mem.sort_by` and a plain fn (through a trampoline) | 175 ms |
| copy taking `fn(*u64, *u64) -> i32`, `salt` in a global | 175 ms |
| `#[inline(never)]` copy, closure, two call sites | 233 ms |
| `#[inline(never)]` copy, plain fn, two call sites | 251 ms |
| `#[inline(never)]` fn-pointer copy, two call sites | 234 ms |

- The first four rows are what a program gets by default. LLVM inlines
  the sort into its caller, or clones it for the constant code pointer,
  and then inlines the comparator.
- The last three rows keep the comparator call indirect. A closure call
  then costs the same as a function pointer call. A plain function passed
  as a `block fn` goes through a trampoline, one extra jump.
- Two lowering choices make the closure rows this fast. A `block fn`
  argument is passed as two scalars (code, env), so LLVM can specialize
  on a constant code pointer, and small immutable captures are copied
  into the environment, so a use is one load instead of two. Before
  them, the closure through `mem.sort_by` took 266 ms and the indirect
  closure 276 ms.
- Sorting indices by a 16 MB key table instead, every row takes about
  230 ms: the random loads hide the call.
