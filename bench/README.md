# Benchmarks

`run.sh` builds `fib.lm` in each dispatch mode and `fib.c` (the same VM,
written by hand in C with computed goto, NaN-boxing and overflow
builtins), then reports the best of 7 runs.

The workload is recursive `fib(32)` as bytecode on a dynamically typed
register VM: about 7 million guest calls, with every arithmetic op
type-checking its operands and taking an overflow-checked integer path.

Results (Intel i7-6700, clang 19, 2026-11-01):

| Version | Time |
|---|---|
| Loimi, `--dispatch=goto` | 56 ms |
| C, computed goto | 62 ms |
| Loimi, `--dispatch=tailcall` | 67 ms |
| Loimi, `--dispatch=switch` | 72 ms |

After cold-block outlining (later the same day), the same benchmark
measured tail-call 70 ms, goto 60 ms, C 67 ms, switch 75 ms. Expect about
5% run-to-run drift on this machine.

The Lua-like example interpreter (`examples/mini`), running recursive
`fib(30)` and a 20M-iteration numeric `for` loop:

| Mode | fib(30) | loop |
|---|---|---|
| tail-call | 50 ms | 214 ms |
| goto | 48 ms | 210 ms |
| switch | 52 ms | 233 ms |

(With the global-variable inline cache and compare-and-branch
superinstructions. Before them, fib(30) took 71–84 ms.)

Notes:

- The generated handlers are as tight as the C ones: `pc`, `regs`, the
  frame pointer and the context stay in registers, the `(Int, Int)` check
  is one xor/or/shift test, and slow paths sit out of line.
- On a VM this small, one function with replicated `indirectbr` wins:
  constants such as the NaN-box tag stay in registers for the whole loop,
  while each tail-call handler rematerializes them. Tail-call mode's
  advantage (independent register allocation per handler) is expected to
  show on larger interpreters, as it did for CPython.

## Other benchmark sets

- [`mini/`](mini/README.md): clean-room Lua-subset programs, Mini vs PUC-Lua and LuaJIT.
- [`scheme/`](scheme/README.md): the public r7rs-benchmarks suite, run unmodified,
  examples/scheme vs chibi-scheme.
