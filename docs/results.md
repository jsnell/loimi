# Benchmark results

One run of every example interpreter against established interpreters
for the same language. This page is for anyone who wants to check
Loimi's performance claims: what was measured, on what machine, how,
and the numbers.

All numbers here come from the **2026-11-07 run at commit 9dce087**,
made with [`bench/run_all.py`](../bench/run_all.py). Ratios quoted
elsewhere in the repository refer to this page.

## Summary

Each cell is our time divided by the reference's time, as a geometric
mean over the suite. Below 1 means ours takes less time. *Interpreter*
is the tail-call build; *JIT* is the same interpreter built with
copy-and-patch JIT stencils (only some interpreters have one).

| Suite | Benchmarks | Reference | Interpreter | JIT |
|---|---:|---|---:|---:|
| mini (Lua subset) | 14 | PUC-Lua 5.4 | 0.49 | 0.37 |
| | | LuaJIT 2.1 `-joff` | 0.75 | 0.56 |
| mini-coro (Lua coroutines) | 3 | PUC-Lua 5.4 | 0.33 | 0.25 |
| | | LuaJIT 2.1 `-joff` | 0.90 | 0.69 |
| minijs (JavaScript subset) | 14 | `node --jitless` | 0.48 | 0.40 |
| | | QuickJS | 0.37 | 0.30 |
| scheme | 43 | Gauche | 0.22 | 0.17 |
| | | Guile, JIT off | 0.38 | 0.30 |
| | | Guile, JIT on | 0.80 | 0.63 |
| lox | 10 | clox | 0.42 | |
| regex | 9 | PCRE2 (no JIT) | 0.44 | |
| | | Python `re` | 0.41 | |
| wasm | 7 | wasm3 | 0.85 | |
| wasm-polybench | 30 | wasm3 | 0.82 | |
| wasm-simd | 3 | our own MVP build | 0.99 | |
| python (Python subset) | 11 | CPython 3.13 | 0.31 | |
| | | MicroPython | 0.11 | |
| prolog | 30 | SWI-Prolog | 0.47 | 0.45 |
| | | Trealla | 0.11 | 0.11 |
| stack (Stak) | 5 | none | | |

Over the 29 Prolog programs other than `sieve`, the JIT build's ratios
are 0.43 (SWI-Prolog) and 0.10 (Trealla).

### Dispatch modes

Every interpreter was also built in the other two dispatch modes. Each
cell is that build's time over the tail-call build's time (geometric
mean), so above 1 means slower than tail-call.

| Suite | goto | switch | JIT |
|---|---:|---:|---:|
| mini | 1.02 | 1.12 | 0.76 |
| mini-coro | 1.06 | 1.10 | 0.77 |
| minijs | 1.03 | 1.12 | 0.83 |
| scheme | 1.04 | 1.13 | 0.78 |
| lox | 1.08 | 1.06 | |
| regex | 1.04 | 1.09 | |
| wasm | 1.05 | 1.27 | |
| wasm-simd | 1.04 | 1.30 | |
| wasm-polybench | 1.05 | 1.27 | |
| stack | 1.04 | 1.10 | |
| python | 1.07 | 1.16 | |
| prolog | 1.08 | 1.21 | 0.94 (0.93 without `sieve`) |

The ratios against the references for every build are in the per-suite
tables below.

## Method

### Machine and software

| | |
|---|---|
| Date | 2026-11-07 |
| Commit | 9dce087 |
| CPU | Intel Core i7-6700 @ 3.40 GHz, 4 cores / 8 threads |
| Kernel | Linux 6.12.113+deb13-amd64 |
| Compilers | clang 19.1.7 (Debian), rustc 1.97.1 |
| Other | Python 3.13.5, node v26.13.0, PCRE2 10.46 (system library) |

### How the interpreters were built

- Every Loimi interpreter is built with `--release --align-functions=64`,
  once per dispatch mode: `--dispatch=tailcall`, `goto` and `switch`.
- Mini, MiniJS, Scheme and Prolog are also built with `--jit-stencils`
  (tail-call dispatch) and run with `LOIMI_JIT=1`. This is the *JIT*
  column.
- x86-64 dispatch branches are kept off 32-byte boundaries, to avoid
  Intel's JCC erratum ([lowering.md, Driver settings](lowering.md#12-driver-settings)).
- Interpreters with a `#[profile]` lay out their tail-call and goto
  builds hottest handler first.

### How runs were timed

- Each suite's own script does the timing (listed in the suites table
  below).
- A time is the minimum wall-clock time of the whole process over 3
  runs, in seconds. The implementations are interleaved within each
  benchmark. The exception is `bench/wasm/bench.py`, which does not
  interleave, so it ran as 3 rounds of one run each.
- Regex times are measured inside the process, in milliseconds: the
  best of 3 matches over 4 MiB of text, then the minimum over 3
  processes.
- Prolog runs longer than 10 s are not repeated (`run.py --slow 10`).
  None was, this time.
- Every output is checked against the expected output.
- Geometric means are over every benchmark of the suite.

### Conditions

- Nothing else ran on the machine. Before each chunk of suites, the
  script waited (at most 3 minutes) for the 1-minute load average to fall
  below 1.0. It was 0.85–0.99 at the start of each chunk and 1.0–1.15 at
  the end, the benchmark process itself included.
- The suites ran as six `run_all.py --only` invocations, 41 minutes in
  all, then the PolyBench set (2 minutes).
- Single benchmarks move by about ±3% between runs, from code placement
  and load.
- The reference interpreters' times also drift between runs. In this run
  they took 0.93–0.97 of their time in the previous run (2026-11-05), so
  compare ratios, not seconds, across runs.

### Suites and references

| Suite | Programs | References | Install | Timing script | Output checked against |
|---|---|---|---|---|---|
| mini | 14 clean-room Lua programs | PUC-Lua 5.4.9; LuaJIT 2.1 with `-joff` (its assembly interpreter) | `bench/lua/install.sh` | `bench/mini/compare.py` | expected output |
| mini-coro | 3 public Lua coroutine programs | as mini | as mini | `bench/mini/coroutines/run.py` | expected output |
| minijs | 14 clean-room JavaScript programs | `node --jitless` (V8's Ignition interpreter); QuickJS 2026-06-04 | `bench/js/install.sh` | `bench/minijs/compare.py` | expected output |
| scheme | 43 r7rs-benchmarks programs | Gauche 0.9.15; Guile 3.0.11 with its JIT off (`GUILE_JIT_THRESHOLD=-1`) and on | `bench/scheme/install.sh` | `bench/scheme/compare.py` | each program's own check, and identical output across implementations |
| lox | the *Crafting Interpreters* benchmarks | clox | `bench/lox/install.sh` | `bench/lox/compare.py` | clox's output |
| regex | 9 patterns | PCRE2 through its C API, without JIT; CPython 3.13's `re` | system PCRE2 | `rxhost bench` (`examples/regex/host.c`) and `examples/regex/bench.py` | expected output |
| wasm | 7 C programs compiled to MVP wasm32-wasi | wasm3 0.5.0 | `bench/wasm/install.sh` | `bench/wasm/bench.py` | expected output |
| wasm-simd | 3 of the wasm programs compiled with `-msimd128` | our tail-call time on the MVP build of the same program (wasm3 0.5 has no SIMD) | as wasm | `bench/wasm/bench.py` | expected output |
| wasm-polybench | 30 PolyBench/C kernels, held out from tuning | wasm3 0.5.0 | as wasm | `bench/wasm/bench.py` | `polybench.sh --check`: arrays of `SMALL_DATASET` builds against wasm3's |
| stack | 5 Stak programs | none | | `examples/stack/bench/run.py` | expected output |
| python | 11 pyperformance programs | CPython 3.13.18 (`-O3`, no PGO or LTO); MicroPython 1.29.0 (unix port, `-X heapsize=256M`) | `bench/python/install.sh` | `bench/python/compare.py` | CPython's output |
| prolog | the 30 van Roy / Aquarius programs from SWI-Prolog's benchmark suite | SWI-Prolog 10.0.2 (`-O`); Trealla 3.14.46 | `bench/prolog/install.sh` | `bench/prolog/run.py` | `done` |

Notes on the references:

- chibi-scheme was not run: it is about 10× slower than the other Scheme
  implementations.
- Stak has no reference. The CPython versions quoted in its
  `FRICTION.md` are not in the repository.
- The PolyBench set is not part of `run_all.py`. It ran as 3 rounds of
  `bench.py -n 1` with wasm3 and the three wasm builds, keeping the
  minimum. How it was chosen is in
  [`bench/wasm/README.md`](../bench/wasm/README.md).

### Reproducing

A full run takes about an hour. `--quick` runs one benchmark per suite,
once, and is mostly builds.

```sh
bench/lua/install.sh; bench/js/install.sh; bench/scheme/fetch.sh; bench/scheme/install.sh
bench/lox/install.sh; bench/wasm/install.sh; bench/wasm/programs.sh; bench/wasm/programs-simd.sh
bench/wasm/polybench.sh; bench/python/install.sh; bench/prolog/install.sh
bench/run_all.py [--quick] [--only mini,scheme] [--out DIR]
bench/run_all.py --report DIR/results.json     # print the tables again
```

The PolyBench set:

```sh
bench/wasm/polybench.sh --check DIR/wasm-tailcall
bench/wasm/bench.py -n 1 --dir bench/wasm/polybench-out --interp="wasm3=bench/wasm/install/bin/wasm3 {wasm}" \
    --interp="tailcall=DIR/wasm-tailcall {wasm}" --interp="goto=DIR/wasm-goto {wasm}" --interp="switch=DIR/wasm-switch {wasm}"
```

## Notes per interpreter

Ratios are for the tail-call build unless stated.

- **Mini** takes 0.31–0.71 of PUC-Lua's time and 0.46–0.93 of LuaJIT
  `-joff`'s, less on all 14. The closest are queens (0.93) and
  spectral_norm (0.92) against LuaJIT. The JIT build takes 0.59
  (mandelbrot) to 0.96 (string_build) of the interpreter's time.
- **Mini coroutines** switch threads inside the dispatch loop
  (`examples/mini/COROUTINES.md`). They take 0.31–0.35 of PUC-Lua's
  time and 0.88–0.92 of LuaJIT `-joff`'s.
- **MiniJS** takes 0.22–0.73 of `node --jitless`'s time and 0.20–0.59 of
  QuickJS's, less on all 14.
- **Scheme** takes less time than Guile with its JIT on in 21 of 43
  programs; with Loimi's JIT, in 30. On the bignum programs: pi 1.12×
  Guile, chudnovsky 0.96×.
- **Lox** takes 0.25–0.62 of clox's time. In goto mode, zoo and zoo_batch
  take 1.23× and 1.24× the tail-call time.
- **Regex** takes less time than PCRE2 and Python's `re` except on
  `literal` (1.22 and 1.23) and, against Python, `dot-star` (1.24).
  `pathological` is rejected before matching by the required-byte check
  (0.0 ms), so it is left out of the geometric means.
- **Wasm** takes 0.75–0.95 of wasm3's time on the seven programs and
  0.66–0.93 on the 30 PolyBench kernels, less on all. The largest switch
  costs are PolyBench correlation (1.87× tail-call) and gramschmidt
  (1.46×). The SIMD build of nbody takes 0.97 of its MVP build's time;
  fannkuchredux and spectralnorm 1.00.
- **Python** takes 0.21–0.39 of CPython's time, less on all 11.
  MicroPython's output differs on chaos and go (its `random` is a
  different generator) and on nbody (last float digits). Their times are
  still in the table and the geometric means; `compare.py` leaves them
  out of its own.
- **Prolog** takes 0.22–0.82 of SWI-Prolog's time, less on all 30, and
  0.05–0.33 of Trealla's. The slowest is sieve (0.82), whose
  assert/retract compile clauses. The JIT build takes 0.72 (tak) to 1.06
  (sendmore) of the interpreter's time, and 1.35 on sieve.
  `run_all.py` marks the JIT cell of sieve with † (its `PATHOLOGICAL`
  table); the mark is left out here.

## Per suite

### mini

| benchmark | tailcall | goto | switch | jit | lua | luajit-joff | tailcall/lua | tailcall/luajit-joff | jit/lua | jit/luajit-joff |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| binary_trees | 0.754 | 0.803 | 0.852 | 0.628 | 2.461 | 1.363 | 0.31 | 0.55 | 0.26 | 0.46 |
| closures | 0.425 | 0.479 | 0.533 | 0.353 | 1.190 | 0.917 | 0.36 | 0.46 | 0.30 | 0.38 |
| fannkuch | 1.110 | 1.022 | 1.310 | 0.865 | 1.657 | 1.383 | 0.67 | 0.80 | 0.52 | 0.63 |
| fib | 0.913 | 1.024 | 1.075 | 0.641 | 1.555 | 1.089 | 0.59 | 0.84 | 0.41 | 0.59 |
| hash_strings | 0.931 | 0.931 | 1.005 | 0.728 | 1.571 | 1.315 | 0.59 | 0.71 | 0.46 | 0.55 |
| interp | 0.680 | 0.679 | 0.784 | 0.546 | 1.951 | 1.056 | 0.35 | 0.64 | 0.28 | 0.52 |
| mandelbrot | 0.565 | 0.564 | 0.625 | 0.331 | 1.162 | 0.755 | 0.49 | 0.75 | 0.28 | 0.44 |
| nbody | 0.436 | 0.419 | 0.513 | 0.320 | 1.122 | 0.501 | 0.39 | 0.87 | 0.29 | 0.64 |
| oo_methods | 0.672 | 0.722 | 0.758 | 0.452 | 1.344 | 0.993 | 0.50 | 0.68 | 0.34 | 0.46 |
| queens | 0.849 | 0.833 | 0.914 | 0.642 | 1.190 | 0.913 | 0.71 | 0.93 | 0.54 | 0.70 |
| sieve | 0.586 | 0.589 | 0.614 | 0.436 | 1.291 | 0.735 | 0.45 | 0.80 | 0.34 | 0.59 |
| sort | 0.719 | 0.710 | 0.803 | 0.571 | 1.065 | 0.829 | 0.68 | 0.87 | 0.54 | 0.69 |
| spectral_norm | 1.037 | 1.062 | 1.119 | 0.713 | 1.675 | 1.124 | 0.62 | 0.92 | 0.43 | 0.63 |
| string_build | 0.594 | 0.587 | 0.621 | 0.571 | 1.646 | 0.735 | 0.36 | 0.81 | 0.35 | 0.78 |
| **geomean** |  |  |  |  |  |  | **0.49** | **0.75** | **0.37** | **0.56** |

Times in seconds.

### mini-coro

| benchmark | tailcall | goto | switch | jit | lua | luajit-joff | tailcall/lua | tailcall/luajit-joff | jit/lua | jit/luajit-joff |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| chameneos | 0.345 | 0.363 | 0.381 | 0.260 | 0.972 | 0.373 | 0.35 | 0.92 | 0.27 | 0.70 |
| coroutine_ring | 0.317 | 0.338 | 0.349 | 0.243 | 0.972 | 0.361 | 0.33 | 0.88 | 0.25 | 0.67 |
| sieve | 0.329 | 0.349 | 0.364 | 0.263 | 1.064 | 0.368 | 0.31 | 0.89 | 0.25 | 0.71 |
| **geomean** |  |  |  |  |  |  | **0.33** | **0.90** | **0.25** | **0.69** |

Times in seconds.

### minijs

| benchmark | tailcall | goto | switch | jit | node-jitless | qjs | tailcall/node-jitless | tailcall/qjs | jit/node-jitless | jit/qjs |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| awfy_micro | 0.748 | 0.760 | 0.823 | 0.622 | 1.548 | 1.970 | 0.48 | 0.38 | 0.40 | 0.32 |
| binary_trees | 0.903 | 0.992 | 1.009 | 0.740 | 1.707 | 4.598 | 0.53 | 0.20 | 0.43 | 0.16 |
| bitops | 0.665 | 0.693 | 0.748 | 0.522 | 1.550 | 1.379 | 0.43 | 0.48 | 0.34 | 0.38 |
| closures | 0.839 | 0.922 | 1.089 | 0.649 | 1.433 | 2.510 | 0.59 | 0.33 | 0.45 | 0.26 |
| deltablue | 0.804 | 0.853 | 0.971 | 0.646 | 1.453 | 2.560 | 0.55 | 0.31 | 0.44 | 0.25 |
| fannkuch | 0.445 | 0.444 | 0.489 | 0.364 | 1.312 | 1.476 | 0.34 | 0.30 | 0.28 | 0.25 |
| json | 0.866 | 0.884 | 0.924 | 0.820 | 1.451 | 2.158 | 0.60 | 0.40 | 0.57 | 0.38 |
| nbody | 0.389 | 0.392 | 0.409 | 0.293 | 1.786 | 1.283 | 0.22 | 0.30 | 0.16 | 0.23 |
| properties | 0.791 | 0.827 | 0.853 | 0.689 | 1.498 | 1.582 | 0.53 | 0.50 | 0.46 | 0.44 |
| raytrace | 0.452 | 0.458 | 0.524 | 0.400 | 1.393 | 2.120 | 0.32 | 0.21 | 0.29 | 0.19 |
| richards | 0.725 | 0.765 | 0.832 | 0.611 | 1.411 | 1.956 | 0.51 | 0.37 | 0.43 | 0.31 |
| sort | 0.789 | 0.792 | 0.861 | 0.663 | 1.299 | 1.676 | 0.61 | 0.47 | 0.51 | 0.40 |
| spectral_norm | 0.810 | 0.835 | 0.869 | 0.653 | 1.551 | 1.363 | 0.52 | 0.59 | 0.42 | 0.48 |
| strings | 1.068 | 1.059 | 1.117 | 0.975 | 1.468 | 2.151 | 0.73 | 0.50 | 0.66 | 0.45 |
| **geomean** |  |  |  |  |  |  | **0.48** | **0.37** | **0.40** | **0.30** |

Times in seconds.

### scheme

| benchmark | tailcall | goto | switch | jit | gauche | guile-nojit | guile | tailcall/gauche | tailcall/guile-nojit | tailcall/guile | jit/gauche | jit/guile-nojit | jit/guile |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| ack | 0.414 | 0.450 | 0.479 | 0.285 | 2.724 | 0.905 | 0.207 | 0.15 | 0.46 | 2.00 | 0.10 | 0.31 | 1.38 |
| array1 | 0.338 | 0.362 | 0.365 | 0.253 | 0.837 | 0.974 | 0.266 | 0.40 | 0.35 | 1.27 | 0.30 | 0.26 | 0.95 |
| browse | 0.011 | 0.012 | 0.013 | 0.010 | 0.080 | 0.027 | 0.019 | 0.14 | 0.41 | 0.58 | 0.12 | 0.37 | 0.53 |
| chudnovsky | 0.134 | 0.135 | 0.137 | 0.131 | 0.946 | 0.176 | 0.140 | 0.14 | 0.76 | 0.96 | 0.14 | 0.74 | 0.94 |
| conform | 0.080 | 0.082 | 0.090 | 0.058 | 0.433 | 0.174 | 0.073 | 0.18 | 0.46 | 1.10 | 0.13 | 0.33 | 0.79 |
| cpstak | 0.221 | 0.234 | 0.236 | 0.177 | 1.099 | 0.417 | 0.195 | 0.20 | 0.53 | 1.13 | 0.16 | 0.42 | 0.91 |
| ctak | 0.018 | 0.018 | 0.019 | 0.016 | 0.232 | 0.467 | 0.455 | 0.08 | 0.04 | 0.04 | 0.07 | 0.03 | 0.04 |
| deriv | 0.120 | 0.123 | 0.144 | 0.101 | 0.685 | 0.385 | 0.183 | 0.18 | 0.31 | 0.66 | 0.15 | 0.26 | 0.55 |
| destruc | 0.114 | 0.122 | 0.128 | 0.086 | 0.460 | 0.224 | 0.091 | 0.25 | 0.51 | 1.25 | 0.19 | 0.38 | 0.95 |
| diviter | 0.132 | 0.145 | 0.140 | 0.115 | 0.469 | 0.340 | 0.161 | 0.28 | 0.39 | 0.82 | 0.25 | 0.34 | 0.71 |
| divrec | 0.161 | 0.169 | 0.184 | 0.146 | 0.614 | 0.433 | 0.210 | 0.26 | 0.37 | 0.77 | 0.24 | 0.34 | 0.70 |
| earley | 0.030 | 0.031 | 0.034 | 0.026 | 0.132 | 0.062 | 0.031 | 0.23 | 0.48 | 0.97 | 0.20 | 0.42 | 0.84 |
| fft | 0.055 | 0.055 | 0.061 | 0.041 | 0.295 | 0.173 | 0.144 | 0.19 | 0.32 | 0.38 | 0.14 | 0.24 | 0.28 |
| fib | 0.398 | 0.402 | 0.454 | 0.326 | 1.405 | 1.067 | 0.281 | 0.28 | 0.37 | 1.42 | 0.23 | 0.31 | 1.16 |
| fibc | 0.027 | 0.027 | 0.028 | 0.021 | 0.328 | 0.570 | 0.541 | 0.08 | 0.05 | 0.05 | 0.06 | 0.04 | 0.04 |
| fibfp | 0.155 | 0.161 | 0.177 | 0.114 | 0.504 | 0.812 | 0.651 | 0.31 | 0.19 | 0.24 | 0.23 | 0.14 | 0.18 |
| gcbench | 0.008 | 0.008 | 0.010 | 0.007 | 0.131 | 0.028 | 0.017 | 0.06 | 0.29 | 0.47 | 0.05 | 0.25 | 0.41 |
| graphs | 0.105 | 0.106 | 0.123 | 0.093 | 0.572 | 0.201 | 0.069 | 0.18 | 0.52 | 1.52 | 0.16 | 0.46 | 1.35 |
| lattice | 0.041 | 0.042 | 0.048 | 0.032 | 0.208 | 0.097 | 0.042 | 0.20 | 0.42 | 0.98 | 0.15 | 0.33 | 0.76 |
| matrix | 0.058 | 0.057 | 0.066 | 0.048 | 0.264 | 0.113 | 0.050 | 0.22 | 0.51 | 1.16 | 0.18 | 0.42 | 0.96 |
| maze | 0.028 | 0.029 | 0.032 | 0.021 | 0.184 | 0.072 | 0.037 | 0.15 | 0.39 | 0.76 | 0.11 | 0.29 | 0.57 |
| mazefun | 0.116 | 0.125 | 0.133 | 0.089 | 0.412 | 0.276 | 0.091 | 0.28 | 0.42 | 1.27 | 0.22 | 0.32 | 0.98 |
| mbrot | 0.051 | 0.050 | 0.052 | 0.036 | 0.196 | 0.288 | 0.247 | 0.26 | 0.18 | 0.21 | 0.18 | 0.12 | 0.15 |
| mperm | 0.129 | 0.134 | 0.145 | 0.096 | 0.387 | 0.185 | 0.059 | 0.33 | 0.70 | 2.19 | 0.25 | 0.52 | 1.63 |
| nboyer | 0.568 | 0.610 | 0.649 | 0.446 | 1.146 | 0.852 | 0.317 | 0.50 | 0.67 | 1.79 | 0.39 | 0.52 | 1.41 |
| nqueens | 0.267 | 0.276 | 0.295 | 0.204 | 0.654 | 0.477 | 0.168 | 0.41 | 0.56 | 1.59 | 0.31 | 0.43 | 1.21 |
| ntakl | 0.164 | 0.190 | 0.189 | 0.107 | 0.509 | 0.309 | 0.075 | 0.32 | 0.53 | 2.19 | 0.21 | 0.35 | 1.43 |
| nucleic | 0.052 | 0.053 | 0.061 | 0.048 | 0.382 | 0.190 | 0.141 | 0.14 | 0.27 | 0.37 | 0.13 | 0.25 | 0.34 |
| paraffins | 0.025 | 0.026 | 0.028 | 0.022 | 0.132 | 0.053 | 0.031 | 0.19 | 0.47 | 0.81 | 0.17 | 0.42 | 0.71 |
| peval | 0.076 | 0.077 | 0.091 | 0.054 | 0.374 | 0.194 | 0.074 | 0.20 | 0.39 | 1.03 | 0.14 | 0.28 | 0.73 |
| pi | 0.112 | 0.112 | 0.113 | 0.116 | 2.833 | 0.103 | 0.100 | 0.04 | 1.09 | 1.12 | 0.04 | 1.13 | 1.16 |
| pnpoly | 0.074 | 0.077 | 0.084 | 0.058 | 0.212 | 0.349 | 0.220 | 0.35 | 0.21 | 0.34 | 0.27 | 0.17 | 0.26 |
| primes | 0.124 | 0.132 | 0.137 | 0.100 | 0.546 | 0.306 | 0.202 | 0.23 | 0.41 | 0.61 | 0.18 | 0.33 | 0.50 |
| puzzle | 0.096 | 0.103 | 0.113 | 0.069 | 0.277 | 0.231 | 0.083 | 0.35 | 0.42 | 1.16 | 0.25 | 0.30 | 0.83 |
| quicksort | 0.101 | 0.104 | 0.113 | 0.076 | 0.284 | 0.192 | 0.075 | 0.36 | 0.53 | 1.35 | 0.27 | 0.40 | 1.01 |
| sboyer | 0.549 | 0.597 | 0.656 | 0.399 | 1.202 | 0.937 | 0.290 | 0.46 | 0.59 | 1.89 | 0.33 | 0.43 | 1.38 |
| simplex | 0.094 | 0.094 | 0.110 | 0.079 | 0.518 | 0.238 | 0.109 | 0.18 | 0.39 | 0.86 | 0.15 | 0.33 | 0.72 |
| string | 0.015 | 0.015 | 0.015 | 0.014 | 0.122 | 0.026 | 0.025 | 0.12 | 0.58 | 0.60 | 0.11 | 0.54 | 0.56 |
| sum | 0.328 | 0.369 | 0.365 | 0.187 | 0.802 | 0.627 | 0.167 | 0.41 | 0.52 | 1.96 | 0.23 | 0.30 | 1.12 |
| sumfp | 0.103 | 0.108 | 0.116 | 0.062 | 0.271 | 0.540 | 0.513 | 0.38 | 0.19 | 0.20 | 0.23 | 0.11 | 0.12 |
| tak | 0.380 | 0.365 | 0.443 | 0.271 | 1.224 | 0.946 | 0.231 | 0.31 | 0.40 | 1.65 | 0.22 | 0.29 | 1.17 |
| takl | 0.079 | 0.079 | 0.102 | 0.055 | 0.301 | 0.160 | 0.043 | 0.26 | 0.49 | 1.84 | 0.18 | 0.34 | 1.28 |
| triangl | 0.151 | 0.160 | 0.177 | 0.103 | 0.402 | 0.411 | 0.128 | 0.38 | 0.37 | 1.18 | 0.26 | 0.25 | 0.80 |
| **geomean** |  |  |  |  |  |  |  | **0.22** | **0.38** | **0.80** | **0.17** | **0.30** | **0.63** |

Times in seconds.

### lox

| benchmark | tailcall | goto | switch | clox | tailcall/clox |
|---|---:|---:|---:|---:|---:|
| binary_trees | 0.576 | 0.603 | 0.626 | 2.344 | 0.25 |
| equality | 1.051 | 1.038 | 1.241 | 3.193 | 0.33 |
| fib | 0.347 | 0.369 | 0.353 | 0.983 | 0.35 |
| instantiation | 0.328 | 0.339 | 0.328 | 0.532 | 0.62 |
| invocation | 0.136 | 0.147 | 0.141 | 0.267 | 0.51 |
| method_call | 0.076 | 0.084 | 0.078 | 0.169 | 0.45 |
| properties | 0.173 | 0.184 | 0.180 | 0.401 | 0.43 |
| trees | 1.233 | 1.226 | 1.208 | 2.286 | 0.54 |
| zoo | 0.131 | 0.161 | 0.145 | 0.300 | 0.44 |
| zoo_batch | 0.774 | 0.959 | 0.864 | 1.860 | 0.42 |
| **geomean** |  |  |  |  | **0.42** |

Times in seconds.

### regex

| benchmark | tailcall | goto | switch | pcre2 | python-re | tailcall/pcre2 | tailcall/python-re |
|---|---:|---:|---:|---:|---:|---:|---:|
| alt-heavy | 22.6 | 24.1 | 24.9 | 48.4 | 31.1 | 0.47 | 0.73 |
| capture | 16.6 | 18.0 | 18.7 | 49.5 | 106.3 | 0.34 | 0.16 |
| class | 58.8 | 58.2 | 63.7 | 213.6 | 148.9 | 0.28 | 0.39 |
| dot-star | 9.8 | 10.0 | 10.1 | 20.7 | 7.9 | 0.47 | 1.24 |
| literal | 3.0 | 3.0 | 3.0 | 2.5 | 2.4 | 1.22 | 1.23 |
| literal-alt | 8.6 | 8.9 | 8.8 | 17.5 | 35.6 | 0.49 | 0.24 |
| patho-alt | 43.6 | 46.5 | 54.0 | 150.8 | 118.9 | 0.29 | 0.37 |
| pathological | 0.0 | 0.0 | 0.0 | 0.0 | 417.7 | — | — |
| words | 19.5 | 21.2 | 21.7 | 52.9 | 151.3 | 0.37 | 0.13 |
| **geomean** |  |  |  |  |  | **0.44** | **0.41** |

Times in milliseconds.

### wasm

| benchmark | tailcall | goto | switch | wasm3 | tailcall/wasm3 |
|---|---:|---:|---:|---:|---:|
| binarytrees | 1.322 | 1.330 | 1.564 | 1.390 | 0.95 |
| coremark | 1.711 | 1.850 | 2.188 | 1.946 | 0.88 |
| fannkuchredux | 1.257 | 1.361 | 1.658 | 1.637 | 0.77 |
| fasta | 1.052 | 1.087 | 1.216 | 1.112 | 0.95 |
| mandelbrot | 1.447 | 1.562 | 1.968 | 1.933 | 0.75 |
| nbody | 1.674 | 1.759 | 2.072 | 1.919 | 0.87 |
| spectralnorm | 1.551 | 1.580 | 2.082 | 1.842 | 0.84 |
| **geomean** |  |  |  |  | **0.85** |

Times in seconds.

### wasm-simd

| benchmark | tailcall | goto | switch | mvp | tailcall/mvp |
|---|---:|---:|---:|---:|---:|
| fannkuchredux_simd | 1.256 | 1.354 | 1.658 | 1.257 | 1.00 |
| nbody_simd | 1.619 | 1.668 | 1.996 | 1.674 | 0.97 |
| spectralnorm_simd | 1.554 | 1.581 | 2.085 | 1.551 | 1.00 |
| **geomean** |  |  |  |  | **0.99** |

Times in seconds. *mvp* is our tail-call time on the MVP build.

### wasm-polybench

The 30 PolyBench/C 4.2.1 kernels, built like the `wasm` programs. Each
uses the largest standard dataset that runs in at most about 2 s under
wasm3. The interpreter was not tuned on them. Geometric means of the
times: wasm3 0.186 s, tail-call 0.153, goto 0.162, switch 0.195.

| benchmark | tailcall | goto | switch | wasm3 | tailcall/wasm3 |
|---|---:|---:|---:|---:|---:|
| 2mm | 0.099 | 0.099 | 0.123 | 0.116 | 0.85 |
| 3mm | 0.151 | 0.152 | 0.182 | 0.180 | 0.84 |
| adi | 0.199 | 0.216 | 0.251 | 0.236 | 0.84 |
| atax | 0.085 | 0.089 | 0.106 | 0.098 | 0.87 |
| bicg | 0.088 | 0.092 | 0.109 | 0.097 | 0.91 |
| cholesky | 0.542 | 0.610 | 0.664 | 0.694 | 0.78 |
| correlation | 0.052 | 0.054 | 0.097 | 0.060 | 0.87 |
| covariance | 0.048 | 0.051 | 0.058 | 0.058 | 0.83 |
| deriche | 0.760 | 0.793 | 0.882 | 1.055 | 0.72 |
| doitgen | 0.045 | 0.048 | 0.058 | 0.054 | 0.83 |
| durbin | 0.119 | 0.122 | 0.154 | 0.142 | 0.84 |
| fdtd-2d | 0.148 | 0.159 | 0.191 | 0.185 | 0.80 |
| floyd-warshall | 1.203 | 1.211 | 1.386 | 1.421 | 0.85 |
| gemm | 0.084 | 0.091 | 0.113 | 0.098 | 0.86 |
| gemver | 0.624 | 0.655 | 0.822 | 0.797 | 0.78 |
| gesummv | 0.235 | 0.244 | 0.286 | 0.256 | 0.92 |
| gramschmidt | 0.091 | 0.094 | 0.133 | 0.104 | 0.88 |
| heat-3d | 0.223 | 0.238 | 0.303 | 0.281 | 0.79 |
| jacobi-1d | 0.062 | 0.067 | 0.080 | 0.082 | 0.76 |
| jacobi-2d | 0.156 | 0.167 | 0.193 | 0.199 | 0.78 |
| lu | 0.613 | 0.686 | 0.751 | 0.777 | 0.79 |
| ludcmp | 0.585 | 0.654 | 0.717 | 0.782 | 0.75 |
| mvt | 0.442 | 0.464 | 0.551 | 0.537 | 0.82 |
| nussinov | 0.154 | 0.166 | 0.187 | 0.232 | 0.66 |
| seidel-2d | 0.287 | 0.313 | 0.354 | 0.340 | 0.84 |
| symm | 0.067 | 0.068 | 0.082 | 0.081 | 0.83 |
| syr2k | 0.079 | 0.085 | 0.100 | 0.098 | 0.81 |
| syrk | 0.048 | 0.049 | 0.062 | 0.056 | 0.86 |
| trisolv | 0.105 | 0.100 | 0.122 | 0.113 | 0.93 |
| trmm | 0.037 | 0.040 | 0.046 | 0.042 | 0.88 |
| **geomean** |  |  |  |  | **0.82** |

Times in seconds.

### stack

| benchmark | tailcall | goto | switch |
|---|---:|---:|---:|
| fib | 0.050 | 0.057 | 0.063 |
| loop | 0.718 | 0.694 | 0.726 |
| nbody | 0.144 | 0.154 | 0.171 |
| sieve | 0.086 | 0.092 | 0.094 |
| strings | 0.119 | 0.116 | 0.118 |

Times in seconds.

### python

| benchmark | tailcall | goto | switch | cpython | micropython | tailcall/cpython | tailcall/micropython |
|---|---:|---:|---:|---:|---:|---:|---:|
| chaos | 0.214 | 0.229 | 0.248 | 0.670 | 1.781 | 0.32 | 0.12 |
| deltablue | 0.315 | 0.348 | 0.358 | 0.921 | 4.574 | 0.34 | 0.07 |
| fannkuch | 0.233 | 0.243 | 0.276 | 0.869 | 1.346 | 0.27 | 0.17 |
| float | 0.197 | 0.210 | 0.218 | 0.812 | 2.073 | 0.24 | 0.10 |
| go | 0.252 | 0.267 | 0.289 | 0.757 | 1.757 | 0.33 | 0.14 |
| hexiom | 0.260 | 0.273 | 0.302 | 0.659 | 2.862 | 0.39 | 0.09 |
| nbody | 0.341 | 0.375 | 0.422 | 0.910 | 3.272 | 0.37 | 0.10 |
| nqueens | 0.288 | 0.301 | 0.319 | 0.758 | 1.590 | 0.38 | 0.18 |
| raytrace | 0.262 | 0.282 | 0.309 | 0.828 | 3.255 | 0.32 | 0.08 |
| richards | 0.164 | 0.190 | 0.199 | 0.778 | 3.038 | 0.21 | 0.05 |
| spectral_norm | 0.175 | 0.184 | 0.190 | 0.674 | 1.313 | 0.26 | 0.13 |
| **geomean** |  |  |  |  |  | **0.31** | **0.11** |

Times in seconds.

### prolog

| benchmark | tailcall | goto | switch | jit | swipl | trealla | tailcall/swipl | tailcall/trealla | jit/swipl | jit/trealla |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| boyer | 0.465 | 0.493 | 0.591 | 0.416 | 0.976 | 4.755 | 0.48 | 0.10 | 0.43 | 0.09 |
| browse | 0.568 | 0.609 | 0.749 | 0.543 | 0.974 | 6.167 | 0.58 | 0.09 | 0.56 | 0.09 |
| chat_parser | 0.655 | 0.690 | 0.775 | 0.600 | 0.984 | 3.567 | 0.67 | 0.18 | 0.61 | 0.17 |
| crypt | 0.309 | 0.350 | 0.360 | 0.273 | 0.967 | 3.652 | 0.32 | 0.08 | 0.28 | 0.07 |
| derive | 0.433 | 0.509 | 0.527 | 0.448 | 0.981 | 2.712 | 0.44 | 0.16 | 0.46 | 0.17 |
| divide10 | 0.453 | 0.526 | 0.545 | 0.423 | 0.981 | 2.677 | 0.46 | 0.17 | 0.43 | 0.16 |
| eval | 0.387 | 0.400 | 0.445 | 0.362 | 0.929 | 2.484 | 0.42 | 0.16 | 0.39 | 0.15 |
| fast_mu | 0.462 | 0.487 | 0.557 | 0.455 | 0.980 | 3.778 | 0.47 | 0.12 | 0.46 | 0.12 |
| flatten | 0.452 | 0.478 | 0.577 | 0.420 | 0.959 | 5.164 | 0.47 | 0.09 | 0.44 | 0.08 |
| log10 | 0.437 | 0.508 | 0.505 | 0.396 | 0.973 | 3.474 | 0.45 | 0.13 | 0.41 | 0.11 |
| meta_qsort | 0.442 | 0.512 | 0.571 | 0.463 | 0.961 | 5.023 | 0.46 | 0.09 | 0.48 | 0.09 |
| mu | 0.374 | 0.425 | 0.477 | 0.393 | 0.945 | 4.083 | 0.40 | 0.09 | 0.42 | 0.10 |
| nand | 0.557 | 0.578 | 0.669 | 0.541 | 0.929 | 3.559 | 0.60 | 0.16 | 0.58 | 0.15 |
| nreverse | 0.505 | 0.550 | 0.561 | 0.421 | 0.973 | 7.825 | 0.52 | 0.06 | 0.43 | 0.05 |
| ops8 | 0.429 | 0.475 | 0.519 | 0.448 | 0.978 | 3.147 | 0.44 | 0.14 | 0.46 | 0.14 |
| perfect | 0.505 | 0.525 | 0.631 | 0.433 | 0.956 | 4.753 | 0.53 | 0.11 | 0.45 | 0.09 |
| poly_10 | 0.426 | 0.477 | 0.541 | 0.427 | 0.968 | 2.476 | 0.44 | 0.17 | 0.44 | 0.17 |
| prover | 0.432 | 0.467 | 0.542 | 0.449 | 0.967 | 3.506 | 0.45 | 0.12 | 0.46 | 0.13 |
| qsort | 0.453 | 0.497 | 0.516 | 0.402 | 0.976 | 5.897 | 0.46 | 0.08 | 0.41 | 0.07 |
| queens_8 | 0.324 | 0.352 | 0.419 | 0.300 | 0.963 | 5.211 | 0.34 | 0.06 | 0.31 | 0.06 |
| query | 0.373 | 0.449 | 0.425 | 0.384 | 0.986 | 4.556 | 0.38 | 0.08 | 0.39 | 0.08 |
| reducer | 0.685 | 0.730 | 0.868 | 0.578 | 0.982 | 4.504 | 0.70 | 0.15 | 0.59 | 0.13 |
| sendmore | 0.423 | 0.420 | 0.501 | 0.447 | 0.944 | 3.340 | 0.45 | 0.13 | 0.47 | 0.13 |
| serialise | 0.525 | 0.560 | 0.649 | 0.487 | 0.966 | 4.939 | 0.54 | 0.11 | 0.50 | 0.10 |
| sieve | 0.755 | 0.774 | 0.836 | 1.021 | 0.925 | 2.269 | 0.82 | 0.33 | 1.10 | 0.45 |
| simple_analyzer | 0.519 | 0.538 | 0.604 | 0.419 | 0.960 | 4.771 | 0.54 | 0.11 | 0.44 | 0.09 |
| tak | 0.183 | 0.181 | 0.211 | 0.132 | 0.850 | 3.505 | 0.22 | 0.05 | 0.16 | 0.04 |
| times10 | 0.417 | 0.476 | 0.506 | 0.380 | 0.971 | 2.831 | 0.43 | 0.15 | 0.39 | 0.13 |
| unify | 0.524 | 0.537 | 0.665 | 0.419 | 0.927 | 6.230 | 0.57 | 0.08 | 0.45 | 0.07 |
| zebra | 0.605 | 0.641 | 0.711 | 0.613 | 0.968 | 3.485 | 0.62 | 0.17 | 0.63 | 0.18 |
| **geomean** |  |  |  |  |  |  | **0.47** | **0.11** | **0.45** | **0.11** |

Times in seconds.

## Changes since the previous run (c7e94ae)

The previous run was on 2026-11-05 at c7e94ae, on the same machine with
the same method. Because the references drift between runs, this run
also timed the old and new builds directly against each other.

### Old build against new, directly

The tail-call and JIT builds of c7e94ae were built from a worktree with
that commit's compiler and sources, same flags. They ran interleaved
with this run's builds, together with a second copy of the old binary
(the *A/A* column, which shows the noise).

- Measure: user+sys CPU time of the process (`getrusage`).
- Runs: 7 per benchmark (Prolog, wasm and PolyBench: 5).
- Every output was equal to the old build's.
- Cells are geometric means, minimum / median of the runs.

| suite | build | new/old | A/A | largest gains | largest losses |
|---|---|---:|---:|---|---|
| mini (14) | tailcall | 0.993 / 0.996 | 0.999 / 1.001 | mandelbrot 0.90, interp 0.97 | sieve 1.02, oo_methods 1.02 |
| mini (14) | jit | 0.994 / 0.995 | 0.999 / 0.998 | mandelbrot 0.94 | binary_trees 1.02 |
| mini-coro (3) | tailcall | 0.913 / 0.910 | 0.993 / 1.007 | chameneos 0.88, sieve 0.93, coroutine_ring 0.93 | |
| mini-coro (3) | jit | 0.893 / 0.896 | 0.998 / 0.994 | chameneos 0.87, coroutine_ring 0.89 | |
| minijs (14) | tailcall | 0.993 / 0.994 | 1.000 / 1.001 | nbody 0.94, closures 0.97 | raytrace 1.03, json 1.01 |
| minijs (14) | jit | 0.994 / 0.993 | 1.000 / 0.999 | strings 0.94, richards 0.96 | **closures 1.03** |
| scheme (43) | tailcall | 0.999 / 0.999 | 1.001 / 0.998 | browse 0.98 | primes 1.02 |
| scheme (43) | jit | 0.995 / 0.997 | 1.003 / 1.005 | string 0.93, mazefun 0.93 | **puzzle 1.11, array1 1.07** |
| lox (9, not zoo_batch) | tailcall | 0.932 / 0.931 | 1.000 / 0.999 | properties 0.85, zoo 0.85, fib 0.87 | trees 1.01 |
| stack (5) | tailcall | 0.990 / 0.996 | 0.985 / 0.992 | | |
| python (11) | tailcall | 0.931 / 0.931 | 1.002 / 1.002 | fannkuch 0.87, chaos 0.90, raytrace 0.91 | nbody 1.00 |
| prolog (30) | tailcall | 0.903 / 0.910 | 1.000 / 1.005 | crypt 0.78, queens_8 0.79, nreverse 0.81 | log10 1.01 |
| prolog (30) | jit | 0.853 / 0.851 | 1.000 / 0.996 | nreverse 0.68, crypt 0.73, query 0.76 | sieve 0.96 |
| wasm (6, not coremark) | tailcall | 1.003 / 1.000 | 0.999 / 0.996 | fasta 0.98 | **binarytrees 1.06** |
| wasm-polybench (30) | tailcall | 0.995 / 0.990 | 1.000 / 0.999 | gemm 0.97 | (mvt 1.13 min, 1.01 median) |

Left out:

- Lox's zoo_batch runs for a fixed 10 s of `clock()`.
- wasm's coremark prints its own timing, so its output differs from run
  to run.
- Regex, the wasm SIMD builds, and the goto and switch builds were not
  run this way.

### Against the references

Over the benchmarks both runs have:

- *references' time* is the new/old geometric mean of each reference's
  times, which shows how the host drifted.
- Each mode column is the geometric mean of (ours / first reference),
  new over old. Below 1 means ours gained on the reference.
- *ratio* is the tail-call (or JIT) build over the first reference, old
  → new.

| suite | references' time | tailcall | goto | switch | jit | ratio old → new |
|---|---:|---:|---:|---:|---:|---|
| mini (14) | 0.92–0.94 | 1.03 | 1.01 | 1.06 | 1.03 | lua 0.47 → 0.49, jit 0.36 → 0.37 |
| mini-coro (3) | 0.96–0.97 | 0.91 | 0.94 | 0.93 | 0.88 | lua 0.36 → 0.33, jit 0.29 → 0.25 |
| minijs (14) | 0.95 | 1.00 | 0.99 | 0.92 | 1.00 | node-jitless 0.48 → 0.48, jit 0.40 → 0.40 |
| scheme (43) | 0.93–0.95 | 1.01 | 1.02 | 1.03 | 1.01 | guile 0.80 → 0.80, jit 0.63 → 0.63 |
| lox (10) | 0.95 | 0.94 | 1.01 | 0.93 | | clox 0.45 → 0.42 |
| regex (9) | 0.97 | 1.00 | 1.00 | 0.98 | | pcre2 0.44 → 0.44 |
| wasm (7) | 0.96 | 1.00 | 1.00 | 0.99 | | wasm3 0.85 → 0.85 |
| wasm-simd (3) | (mvp) | 1.01 | 1.02 | 1.01 | | mvp 0.98 → 0.99 |
| wasm-polybench (30) | 0.96 | 0.99 | 0.99 | 1.00 | | wasm3 0.83 → 0.82 |
| stack (5) | | 0.96 | 0.97 | 0.96 | | (own time new/old; no reference) |
| python (11) | 0.95–0.96 | 0.94 | 0.91 | 0.93 | | cpython 0.33 → 0.31 |
| prolog (30) | 0.95 | 0.90 | 0.95 | 0.98 | 0.85 | swipl 0.52 → 0.47, jit 0.52 → 0.45 |

Mini's 1.03 against PUC-Lua comes from the host, not from Mini.
Allocation-heavy programs sped up more under every reference than the
rest did:

| Program | PUC-Lua new/old | LuaJIT `-joff` new/old |
|---|---:|---:|
| binary_trees | 0.80 | 0.87 |
| sieve | 0.86 | 0.73 |
| string_build | 0.66 | 0.75 |
| hash_strings | | 1.28 |

Most other programs were at about 0.97, and the direct check has Mini at
0.993.

The switch-mode cells, which were not part of the direct check, moved
most against the references: PolyBench correlation 1.54 (1.87× the
tail-call time), Prolog tak 1.33, Mini string_build 1.30 (from Lua's
own drift above) and PolyBench gramschmidt 1.24.

### Regressions

Three single-benchmark losses held when rerun (11 interleaved runs, A/A
within 0.3%). Two were bisected over the first-parent history in a
separate worktree (`git bisect --first-parent`, 7 runs per step).

| Loss | Cause |
|---|---|
| Scheme with the JIT: puzzle 1.11 (0.062 → 0.069 s), array1 1.07 (0.236 → 0.251 s). The interpreter is unchanged on both. | First bad commit 3e0de22, "errors and deoptimization are cold by default". The hot JIT code is unchanged; a few copies outside the loops got 5 bytes shorter and moved the loops. Starting the JIT region 16 bytes later swaps the two builds' times ([copy-and-patch.md](design/copy-and-patch.md#15-placement-of-compiled-code-the-scheme-puzzlearray1-regression-2026-11-07)). |
| wasm tail-call binarytrees 1.06 (1.247 → 1.324 s) | First bad commit 5667a53, which added dispatch profiles for hot-handler ordering. The suite is unchanged (1.003); a placement effect. Not investigated further. |
| MiniJS with the JIT, closures 1.03 (0.627 → 0.648 s; 1.034 again in a rerun of 11) | Not bisected. |

### What changed

From `git log c7e94ae..9dce087`. Each change was also measured on its
own; those A/Bs are in [design/speed-audit.md](design/speed-audit.md)
unless noted.

| Change | Effect |
|---|---|
| Hot-handler placement (`#[profile]`, 2026-11-05; [lowering.md](lowering.md#12-driver-settings), "Handler order") | Lox properties; PolyBench goto-mode gemm went from 1.42× tail-call to 1.08× |
| Value accumulators in MiniJS (2026-11-06) | nbody 0.94. Mini's port (mandelbrot 0.90, suite 0.99–1.00) was removed on 2026-11-08: it added 27% to Mini's IR and its build time. |
| `nanbox(int_high)` in Python and Prolog (2026-11-06) | Python 0.93; Prolog 0.90, its JIT build 0.85. Prolog's goto and switch builds gained less (0.95, 0.98), so they are now 1.08× and 1.21× tail-call, from 1.03× and 1.11×. |
| `std.rt.ValueStack` in Lox, Scheme and MiniJS; re-fusion in Lox (2026-11-06, 2026-11-07) | Lox 0.93 (fib 0.87, properties and zoo 0.85); Scheme and MiniJS unchanged |
| Mini: direct SWITCH, and coroutine calls quickened to CALL_RESUME, CALL_WRAP and CALL_YIELD (2026-11-06) | interp 0.97; coroutine suite 0.91, its JIT build 0.89. chameneos against LuaJIT `-joff`: 1.06 → 0.92. |
| `Divisor`, cold-by-default errors, `rel`/`data`, supers groups, `--vm-prof` | No suite-level change. The cold-by-default commit caused the Scheme JIT regression above. |

## GC: mark bitmaps instead of sweeping (2026-11-04)

A dated A/B of one change to `std.gc`, made before the 2026-11-04 run
(95a227e); its effect is included in the later runs. The collector
stopped sweeping. Marking now sets a bit per 16-byte granule in each
chunk's bitmap, and each size class allocates by bumping through runs of
clear bits ([stdlib/gc.md](stdlib/gc.md#heap-layout)).

### Where the time went before

Measured with `LOIMI_GC_STATS=1`, which prints mark and sweep times per
collection. Sweeping was most of the collector's time everywhere,
because it touched every slot of every chunk allocated from since the
last collection, almost all of them dead.

| Interpreter | Collector | Time in the collector | Example |
|---|---|---|---|
| Prolog | non-generational, 1 MiB budget, 40–1500 KB live | 3–29% of run time, 80–90% of that sweeping | nreverse: 2434 collections, 247 ms sweep, 35 ms mark, of 1.07 s |
| Python | non-generational | 2–11% | nqueens: 111 ms sweep, 3 ms mark, of 1.0 s |
| Scheme | generational, lazy sweeping | 13–23% sweeping on deriv, diviter, divrec, cpstak and primes; 0–2% minor marking | |
| Mini | generational, lazy sweeping | 9–14% sweeping on closures and binary_trees | |
| MiniJS | generational, lazy sweeping | similar | |

### Result

Interleaved A/B of the new std against the old one, same compiler.
Built with `--release --align-functions=64`, tail-call dispatch, minimum
of 3 runs. The machine was shared with another agent (load average
4.5–6 on 8 threads). Cells are time new/old, geometric means. *Noise*
is the geometric mean of two columns built from the same code in the
same run.

| Suite | new/old | range | noise |
|---|---|---|---|
| Prolog (29, not sieve) | 0.885 (0.876, 0.885 in two other runs) | 0.79 qsort, 0.80 nreverse, 0.82 derive .. 0.97 reducer | |
| Scheme (43) | 0.955 (0.949, 0.942) | 0.77 diviter, 0.83 deriv, 0.84 cpstak, 0.86 peval, gcbench .. 1.10 earley (0.91 noise) | 0.996 |
| Python (11) | 0.943 (0.914) | 0.88 nqueens, 0.89 fannkuch, 0.90 chaos .. 1.01 | |
| Mini (14) | 0.979 (0.977) | 0.82 binary_trees, 0.88 closures, 0.93 string_build; others ±3%, spectral_norm 1.07 (no GC; 0.95 noise) | 0.998 |
| MiniJS (14) | 0.975 (0.989) | 0.83 closures, 0.92 binary_trees, 0.95 strings; others ±5% | 0.989 |

Peak RSS is unchanged within 1%, because the heap-growth policy is the
same and the per-class chunks leave no holes:

| Program | Peak RSS before → after |
|---|---|
| Prolog boyer | 30.1 → 30.2 MB |
| Prolog sieve | 203 → 205 MB |
| Prolog tak | 22.7 → 20.8 MB |
| Scheme nboyer | 154.5 → 154.4 MB |
| Scheme mperm | 94.1 → 93.9 MB |
| Mini binary_trees | 139.7 → 139.5 MB |
| MiniJS binary_trees | 90.6 → 90.7 MB |
| MiniJS sort | 76.5 → 76.4 MB |
| Python float | 107.6 → 107.5 MB |

What is left of the collector's time is marking and `refill` (finding
free runs). Marking now also writes the bitmap, and takes 1.1–1.25 times
the old mark time on Prolog.

- nreverse: 39 ms mark and 20 ms refill, 8% of its time (from 26%).
- boyer: 255 ms and 69 ms, 21% (from 28%). Its 30 MB heap leaves many
  short runs between live objects.

### Follow-up (2026-11-04): prefetching marks, fragmented chunks last

Two further changes:

- Marking in a major collection prefetches each object's header and
  marks it 16 `mark_ptr` calls later (a new intrinsic, `p.prefetch()`).
- Chunks whose free runs average under 8 objects are allocated from
  after the pool of empty chunks.

Mark / refill times in ms, `LOIMI_GC_STATS`, minimum of 5:

| Program | Before | After |
|---|---|---|
| boyer | 224 / 59 | 168 / 45 |
| poly_10 | 108 / 39 | 113 / 9 |
| tak | 94 / 2 | 84 / 2 |
| nreverse | 39 / 20 | 36 / 21 |

Interleaved A/B, user+sys CPU time, new/old geometric means. The machine
was at load 6–7 on 8 threads, and single programs moved by up to ±10%
between runs.

| Suite | new/old |
|---|---|
| Prolog (29, not sieve, min of 5) | 0.994 (boyer, poly_10, times10 0.94; tak 0.95; others within noise) |
| Mini | 0.995 |
| MiniJS | 1.007 |
| Python | 0.992 |
| Scheme (min of 3) | 1.012 |

Scheme's tak, which never collects, measured 1.05–1.10 in three runs: its
handler code moved by 640 bytes, a layout effect. Peak RSS was unchanged
within noise everywhere (boyer 29.6 MB both).

### Tried and dropped

- **One bump run for every size** (bits set over each marked object's
  granules, so any size fits any hole): as fast on Prolog (0.885), but
  MiniJS's `sort` was 4% slower because its records spread over more
  cache lines (`GETPROP_IN` 6% → 13% of samples).
- **Taking the empty-chunk pool before all of a class's chunks**: cut
  boyer's refill to 21 ms but grew its peak RSS to 33.5–35 MB.
- **Tracing through a FIFO of popped objects without prefetching**: no
  change.
- **A plain load instead of the prefetch**: boyer mark 252 ms, slower
  than before.
