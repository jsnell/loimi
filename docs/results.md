# Benchmark results

One run of every example interpreter against its reference
implementations, made with [`bench/run_all.py`](../bench/run_all.py).
This page replaces the ratios quoted elsewhere (README, `status.md`,
`roadmap.md`); the per-interpreter logs (`bench/*/RESULTS.md`,
`bench/scheme/README.md`, the examples' `FRICTION.md`) keep their dated
history.

## Setup

| | |
|---|---|
| Date | 2026-11-03 |
| Commit | master 68012bd (plus `bench/run_all.py`) |
| CPU | Intel Core i7-6700 @ 3.40 GHz, 4 cores / 8 threads |
| Kernel | Linux 6.12.113+deb13-amd64 |
| Compilers | clang 19.1.7 (Debian), rustc 1.97.1 |
| Other | Python 3.13.5, node v26.13.0, PCRE2 10.46 (system library) |

The machine was shared with another agent's builds and tests (load
average 2–4.7 during the runs). Single benchmarks move by about ±3%
between runs from code placement and load; the geometric means moved by
at most 0.02 between the two runs below.

**Method.**

- Every Loimi interpreter is built with `--release --align-functions=64`
  in each dispatch mode (`--dispatch=tailcall`, `goto`, `switch`).
  Mini, MiniJS and Scheme are also built with `--jit-stencils` (tail-call
  dispatch) and run with `LOIMI_JIT=1` (column *jit*).
- The suites' own scripts do the timing: `bench/mini/compare.py`,
  `bench/mini/coroutines/run.py`, `bench/minijs/compare.py`,
  `bench/scheme/compare.py`, `bench/lox/compare.py`,
  `bench/wasm/bench.py`, `examples/stack/bench/run.py`, and for the regex
  library `rxhost bench` (`examples/regex/host.c`, built with PCRE2) plus
  `examples/regex/bench.py` (Python `re`). Every output is checked against
  the expected output (Lox: against clox's; Scheme: the benchmark's own
  check and identical output across implementations).
- Times are the minimum wall-clock time of the whole process over 3 runs,
  in seconds, with the implementations interleaved within each benchmark
  (`bench.py` does not interleave, so it runs in 3 rounds of one run).
  Mini, Lox and Wasm were run twice (the second run because the
  references' times were high in the first); the tables show the minimum
  over both, i.e. best of 6. The regex times are measured inside the
  process, each the best of 3 matches over 4 MiB of text, and the minimum
  over 3 processes, in milliseconds.
- Ratios are ours divided by the reference: below 1, ours takes less
  time. Geometric means are over every benchmark of the suite.
- References: PUC-Lua 5.4.9 and LuaJIT 2.1 (`-joff`, its assembly
  interpreter) from `bench/lua/install.sh`; `node --jitless` (V8's
  Ignition interpreter) and QuickJS 2026-06-04 (`bench/js/install.sh`);
  Gauche 0.9.15 and Guile 3.0.11 with its JIT off (`guile-nojit`,
  `GUILE_JIT_THRESHOLD=-1`) and on (`guile`) (`bench/scheme/install.sh`;
  chibi-scheme was not run, it is 10× slower than the others); clox
  (`bench/lox/install.sh`); wasm3 0.5.0 (`bench/wasm/install.sh`); PCRE2
  through its C API without JIT, and CPython 3.13's `re`. Stak has no
  reference: the CPython versions quoted in its `FRICTION.md` are not in
  the repository.

To reproduce (about 26 minutes; `--quick` runs one benchmark per suite,
once, in about 5 minutes, mostly builds):

```sh
bench/lua/install.sh; bench/js/install.sh; bench/scheme/fetch.sh; bench/scheme/install.sh
bench/lox/install.sh; bench/wasm/install.sh; bench/wasm/programs.sh
bench/run_all.py [--quick] [--only mini,scheme] [--out DIR]
bench/run_all.py --report DIR/results.json     # the tables below, again
```

## Summary: geometric mean of ours / reference

| interpreter | build | vs tailcall | references (ours / ref) |
|---|---|---:|---|
| mini (14) | tailcall | 1 | lua 0.53, luajit-joff 0.83 |
| mini (14) | goto | 1.00 | lua 0.52, luajit-joff 0.83 |
| mini (14) | switch | 1.14 | lua 0.60, luajit-joff 0.94 |
| mini (14) | jit | 0.75 | lua 0.40, luajit-joff 0.63 |
| mini-coro (3) | tailcall | 1 | lua 0.55, luajit-joff 1.51 |
| mini-coro (3) | goto | 1.20 | lua 0.67, luajit-joff 1.82 |
| mini-coro (3) | switch | 1.23 | lua 0.68, luajit-joff 1.85 |
| mini-coro (3) | jit | 0.92 | lua 0.51, luajit-joff 1.38 |
| minijs (14) | tailcall | 1 | node-jitless 0.52, qjs 0.40 |
| minijs (14) | goto | 1.02 | node-jitless 0.52, qjs 0.41 |
| minijs (14) | switch | 1.15 | node-jitless 0.60, qjs 0.46 |
| minijs (14) | jit | 0.86 | node-jitless 0.44, qjs 0.34 |
| scheme (41) | tailcall | 1 | gauche 0.27, guile-nojit 0.44, guile 0.96 |
| scheme (41) | goto | 1.01 | gauche 0.27, guile-nojit 0.45, guile 0.97 |
| scheme (41) | switch | 1.10 | gauche 0.30, guile-nojit 0.49, guile 1.05 |
| scheme (41) | jit | 0.81 | gauche 0.22, guile-nojit 0.36, guile 0.78 |
| lox (10) | tailcall | 1 | clox 0.45 |
| lox (10) | goto | 1.03 | clox 0.46 |
| lox (10) | switch | 1.07 | clox 0.49 |
| regex (9) | tailcall | 1 | pcre2 0.44, python-re 0.42 |
| regex (9) | goto | 1.01 | pcre2 0.45, python-re 0.42 |
| regex (9) | switch | 1.08 | pcre2 0.48, python-re 0.45 |
| wasm (7) | tailcall | 1 | wasm3 0.91 |
| wasm (7) | goto | 1.06 | wasm3 0.96 |
| wasm (7) | switch | 1.67 | wasm3 1.52 |
| stack (5) | tailcall | 1 | — |
| stack (5) | goto | 1.01 | — |
| stack (5) | switch | 1.22 | — |

(n) is the number of benchmarks. *vs tailcall* is the geometric mean of
that build's times over the tail-call build's. Suites: `mini` (Mini, a
Lua 5.4 subset, 14 clean-room benchmarks), `mini-coro` (Mini's three
coroutine benchmarks), `minijs` (14 clean-room benchmarks), `scheme` (41
r7rs-benchmarks programs), `lox` (the book's benchmarks), `regex` (the
backtracking engine), `wasm` (C programs compiled to wasm32-wasi), `stack`
(Stak).

## Notes

- Mini (tail-call) takes 0.29–0.76 of PUC-Lua's time and 0.53–1.00 of
  LuaJIT `-joff`'s: less on 13 benchmarks, the same on hash_strings
  (0.999). With the JIT: 0.57–0.99 of the interpreter's time.
- Mini's coroutine benchmarks take 0.53–0.59 of PUC-Lua's time and
  1.42–1.57 of LuaJIT `-joff`'s; goto and switch builds are 20% slower
  there (explained in "Follow-up" below).
- MiniJS takes 0.23–0.76 of `node --jitless`'s time and 0.23–0.67 of
  QuickJS's, less on all 14.
- Scheme takes less time than Guile with its JIT on 12 of 41 programs
  (geometric mean 0.96); with Loimi's JIT, 0.78.
- Lox takes 0.29–0.69 of clox's time. The switch build is 1.8× slower
  than tail-call on `equality` (reproduced in both runs) and within 5%
  elsewhere (fixed since: "Follow-up" below).
- Regex: the backtracker takes less time than the PCRE2 interpreter and
  Python's `re` except on `literal` (1.27–1.29) and, against Python,
  `dot-star` (1.29). `pathological` is rejected by the required-byte check
  before matching (0.0 ms; Python 430 ms) and is left out of the
  geometric means.
- Wasm takes 0.75–1.00 of wasm3's time. wasm3's own times vary between
  sessions: fasta took 1.51 s in the run recorded in
  `examples/wasm/FRICTION.md` and 1.15 s here, with our time unchanged
  (1.14–1.15 s), so the geometric mean moved from 0.87 to 0.91.
- The switch builds of the wasm interpreter take 1.67× the tail-call time
  (2× on mandelbrot and nbody; about 1.4× since the fix in "Follow-up"
  below).

## Against the last recorded numbers

No interpreter is more than 5% slower by geometric mean than in its last
recorded run.

- Mini, tail-call: 0.98 of the interpreter times in
  `design/copy-and-patch.md` (JIT: 0.95 of the JIT times there).
- MiniJS 0.52 of `node --jitless` (0.53 recorded), Scheme 0.27/0.44/0.96
  of Gauche/Guile without JIT/Guile with JIT (0.29/0.47/1.02), Lox 0.45
  of clox (0.45).
- Wasm: our times are within 2% of `examples/wasm/FRICTION.md`'s; the
  ratio changed with wasm3's (see above).
- Regex: the backtracker is 1.035× slower by geometric mean than at
  b88a8ce, the commit of the `FRICTION.md` table (both built here with
  `--align-functions=64`, best of 5 interleaved processes), concentrated
  in `literal-alt`, `words` and `capture` (1.08–1.11); the PCRE2 times
  are unchanged. The b88a8ce build itself is up to 10% slower here than
  the `FRICTION.md` table (`class` 63 against 57 ms), which was built
  without `--align-functions` at another time. The cause is in
  "Follow-up" below.
- Stak, tail-call: 1.04 of its `FRICTION.md` times (sieve and strings
  1.06–1.07, nbody 0.97).

## Follow-up (2026-11-03): the anomalies above

The tables on this page are from before these changes; the numbers here
are A/B runs on the same machine (best of 3, or 5 where noted,
interleaved, `--align-functions=64`; the machine was loaded, so absolute
times are higher than in the tables).

**Switch mode was slowed by LLVM's tail merging.** In the switch builds
(one function, one shared `switch`), branch folding kept a single copy
of the handlers' common tails (store the result, `pc += 1`, branch to
the switch), so handlers reached the dispatch block through two to four
unconditional jumps (Lox's EQK through four). The state was not the
problem: no spills in the hot handlers, and the wasm accumulators stay
in `xmm6` and `r15` across the loop. The driver now passes
`-enable-tail-merge=false` when a program has a switch-mode vm
(`lowering.md` §6, §12; tail-call and goto builds are unchanged). Switch
builds, new over old:

| suite | geomean | range |
|---|---:|---|
| wasm (7) | 0.79 | 0.73 (mandelbrot, nbody) – 0.88 (fasta) |
| lox `equality` | 0.71 | other Lox benchmarks 0.96–1.02 |
| mini (14) | 0.93 | 0.75 (closures) – 1.05 (sieve) |
| minijs (14) | 0.95 | 0.81 (bitops) – 1.01 |
| scheme (41) | 1.00 | 0.93 – 1.15 (puzzle; placement) |
| stack (5) | 0.98 | 0.92 – 1.02 |
| mini-coro (3) | 0.93 | |

Wasm switch is now about 1.35–1.4× its tail-call time (was 1.67×; 1.5–1.6×
on mandelbrot and nbody, whose handlers are three to five instructions
around the one shared indirect jump), Lox `equality` 1.24× (was 1.8×).

**Mini's coroutine benchmarks in goto and switch mode** pay for the
nested run each `resume` makes (`examples/mini/COROUTINES.md`). In
tail-call mode the entry (`Mini.resume`) is a small function that LLVM
inlines into the `resume` native; in goto and switch mode it is the
function holding every handler, entered and left once per resume/yield
pair: six pushes and pops, a 1.3 KB frame (24 shadow-stack frames of
different handlers, never overlapped since they carry no lifetime
markers), the state set-up. A sampling profile of `coroutine_ring` (goto)
puts 11% of the time in that entry block before the first dispatch, and
the yield path (MOVE_CALL plus the exit) 35% above tail-call's. The
other hot handlers cost about the same in both modes (within ±20% of
few samples each). At the current master
(after the resume-point commits) goto takes 1.14× the tail-call time on
`coroutine_ring` and `sieve` and 1.02× on `chameneos` (best of 5; it was
1.13–1.27× in the table above). Not changed: a resume op that switches
coroutines inside the dispatch loop (COROUTINES.md, "Performance") would
remove the nested entry in every mode.

**Regex: the 3.5% is `#[catch_panic(-2)]` on `rx_exec`** (ea536af, which
says so: "5-8% on the ones with a match every 25 ns"). Bisected over the
101 first-parent commits from b88a8ce (each commit's own compiler and
example, best of 5): 31fefd4 is at 0.997, ea536af at 1.049, and every
later commit 1.03–1.04. Master with the attribute removed from `rx_exec`
is 1.003. The cost is per call: installing the handler (two TLS loads and
stores) about 1.5%, the frame pointer that the builtin setjmp needs
about 1%, the setjmp about 1% (variants of master's IR: without the
setjmp 1.025, without setjmp and frame address 1.016). Not inlining the
body into the wrapper (the arguments then cross the setjmp in memory) is
slower, 1.044, as ea536af found; passing them through an opaque copy
after the setjmp gains nothing (1.042). Kept: it is the price of
catching internal errors per call.

**Stak's sieve and strings** are not slower than at the commit of its
`FRICTION.md` table: a8ccba2, b88a8ce and master built today all run
sieve in 0.091–0.093 s and strings in 0.146–0.157 s (best of 7). The
difference from the recorded 0.087/0.135 is the machine or session, not
a code change.

## Per suite

### mini

| benchmark | tailcall | goto | switch | jit | lua | luajit-joff | tailcall/lua | tailcall/luajit-joff | jit/lua | jit/luajit-joff |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| binary_trees | 0.888 | 0.924 | 1.037 | 0.747 | 3.019 | 1.470 | 0.29 | 0.60 | 0.25 | 0.51 |
| closures | 0.496 | 0.519 | 0.565 | 0.410 | 1.240 | 0.940 | 0.40 | 0.53 | 0.33 | 0.44 |
| fannkuch | 1.251 | 1.060 | 1.370 | 0.923 | 1.709 | 1.437 | 0.73 | 0.87 | 0.54 | 0.64 |
| fib | 0.981 | 1.100 | 1.217 | 0.680 | 1.603 | 1.136 | 0.61 | 0.86 | 0.42 | 0.60 |
| hash_strings | 1.063 | 1.044 | 1.137 | 0.801 | 1.650 | 1.064 | 0.64 | 1.00 | 0.49 | 0.75 |
| interp | 0.970 | 0.927 | 1.149 | 0.717 | 2.004 | 1.081 | 0.48 | 0.90 | 0.36 | 0.66 |
| mandelbrot | 0.641 | 0.660 | 0.726 | 0.364 | 1.198 | 0.782 | 0.54 | 0.82 | 0.30 | 0.47 |
| nbody | 0.478 | 0.486 | 0.588 | 0.357 | 1.249 | 0.531 | 0.38 | 0.90 | 0.29 | 0.67 |
| oo_methods | 0.781 | 0.818 | 0.913 | 0.530 | 1.383 | 1.049 | 0.56 | 0.74 | 0.38 | 0.51 |
| queens | 0.931 | 0.887 | 1.082 | 0.680 | 1.230 | 0.945 | 0.76 | 0.99 | 0.55 | 0.72 |
| sieve | 0.754 | 0.755 | 0.808 | 0.663 | 1.505 | 1.018 | 0.50 | 0.74 | 0.44 | 0.65 |
| sort | 0.844 | 0.785 | 0.908 | 0.640 | 1.167 | 0.883 | 0.72 | 0.96 | 0.55 | 0.72 |
| spectral_norm | 1.103 | 1.093 | 1.230 | 0.753 | 1.743 | 1.164 | 0.63 | 0.95 | 0.43 | 0.65 |
| string_build | 0.950 | 0.971 | 1.011 | 0.945 | 2.517 | 1.004 | 0.38 | 0.95 | 0.38 | 0.94 |
| **geomean** |  |  |  |  |  |  | **0.53** | **0.83** | **0.40** | **0.63** |

Times in seconds.

### mini-coro

| benchmark | tailcall | goto | switch | jit | lua | luajit-joff | tailcall/lua | tailcall/luajit-joff | jit/lua | jit/luajit-joff |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| chameneos | 0.600 | 0.676 | 0.750 | 0.570 | 1.022 | 0.387 | 0.59 | 1.55 | 0.56 | 1.47 |
| coroutine_ring | 0.552 | 0.669 | 0.694 | 0.486 | 1.035 | 0.390 | 0.53 | 1.42 | 0.47 | 1.25 |
| sieve | 0.611 | 0.779 | 0.719 | 0.563 | 1.125 | 0.390 | 0.54 | 1.57 | 0.50 | 1.44 |
| **geomean** |  |  |  |  |  |  | **0.55** | **1.51** | **0.51** | **1.38** |

Times in seconds.

### minijs

| benchmark | tailcall | goto | switch | jit | node-jitless | qjs | tailcall/node-jitless | tailcall/qjs | jit/node-jitless | jit/qjs |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| awfy_micro | 0.794 | 0.806 | 0.933 | 0.636 | 1.625 | 2.079 | 0.49 | 0.38 | 0.39 | 0.31 |
| binary_trees | 1.194 | 1.244 | 1.386 | 1.038 | 1.854 | 5.173 | 0.64 | 0.23 | 0.56 | 0.20 |
| bitops | 0.850 | 0.850 | 1.065 | 0.700 | 1.643 | 1.458 | 0.52 | 0.58 | 0.43 | 0.48 |
| closures | 1.011 | 1.097 | 1.220 | 0.779 | 1.506 | 2.640 | 0.67 | 0.38 | 0.52 | 0.30 |
| deltablue | 0.882 | 0.949 | 1.075 | 0.747 | 1.545 | 2.706 | 0.57 | 0.33 | 0.48 | 0.28 |
| fannkuch | 0.486 | 0.445 | 0.558 | 0.389 | 1.336 | 1.542 | 0.36 | 0.32 | 0.29 | 0.25 |
| json | 1.002 | 1.007 | 1.102 | 1.011 | 1.586 | 2.318 | 0.63 | 0.43 | 0.64 | 0.44 |
| nbody | 0.429 | 0.439 | 0.465 | 0.325 | 1.881 | 1.334 | 0.23 | 0.32 | 0.17 | 0.24 |
| properties | 0.887 | 0.884 | 0.970 | 0.769 | 1.611 | 1.630 | 0.55 | 0.54 | 0.48 | 0.47 |
| raytrace | 0.529 | 0.555 | 0.623 | 0.505 | 1.475 | 2.231 | 0.36 | 0.24 | 0.34 | 0.23 |
| richards | 0.774 | 0.815 | 0.927 | 0.693 | 1.476 | 2.042 | 0.52 | 0.38 | 0.47 | 0.34 |
| sort | 0.937 | 0.919 | 1.024 | 0.796 | 1.438 | 1.846 | 0.65 | 0.51 | 0.55 | 0.43 |
| spectral_norm | 0.939 | 0.954 | 1.049 | 0.778 | 1.624 | 1.409 | 0.58 | 0.67 | 0.48 | 0.55 |
| strings | 1.372 | 1.339 | 1.523 | 1.313 | 1.805 | 2.369 | 0.76 | 0.58 | 0.73 | 0.55 |
| **geomean** |  |  |  |  |  |  | **0.52** | **0.40** | **0.44** | **0.34** |

Times in seconds.

### scheme

| benchmark | tailcall | goto | switch | jit | gauche | guile-nojit | guile | tailcall/gauche | tailcall/guile-nojit | tailcall/guile | jit/gauche | jit/guile-nojit | jit/guile |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| ack | 0.467 | 0.509 | 0.543 | 0.359 | 3.035 | 0.946 | 0.220 | 0.15 | 0.49 | 2.12 | 0.12 | 0.38 | 1.63 |
| array1 | 0.359 | 0.376 | 0.399 | 0.251 | 0.899 | 1.016 | 0.292 | 0.40 | 0.35 | 1.23 | 0.28 | 0.25 | 0.86 |
| browse | 0.018 | 0.017 | 0.019 | 0.016 | 0.091 | 0.031 | 0.020 | 0.20 | 0.58 | 0.90 | 0.18 | 0.52 | 0.80 |
| conform | 0.128 | 0.128 | 0.144 | 0.112 | 0.471 | 0.184 | 0.079 | 0.27 | 0.70 | 1.62 | 0.24 | 0.61 | 1.42 |
| cpstak | 0.282 | 0.296 | 0.311 | 0.230 | 1.244 | 0.444 | 0.214 | 0.23 | 0.64 | 1.32 | 0.18 | 0.52 | 1.07 |
| ctak | 0.030 | 0.027 | 0.029 | 0.026 | 0.264 | 0.543 | 0.528 | 0.11 | 0.06 | 0.06 | 0.10 | 0.05 | 0.05 |
| deriv | 0.250 | 0.257 | 0.268 | 0.246 | 0.823 | 0.391 | 0.191 | 0.30 | 0.64 | 1.31 | 0.30 | 0.63 | 1.29 |
| destruc | 0.137 | 0.146 | 0.155 | 0.116 | 0.481 | 0.235 | 0.094 | 0.28 | 0.58 | 1.46 | 0.24 | 0.49 | 1.23 |
| diviter | 0.183 | 0.183 | 0.195 | 0.152 | 0.513 | 0.362 | 0.180 | 0.36 | 0.51 | 1.02 | 0.30 | 0.42 | 0.84 |
| divrec | 0.231 | 0.235 | 0.258 | 0.211 | 0.670 | 0.456 | 0.226 | 0.34 | 0.51 | 1.02 | 0.31 | 0.46 | 0.93 |
| earley | 0.045 | 0.045 | 0.052 | 0.046 | 0.148 | 0.067 | 0.034 | 0.30 | 0.67 | 1.32 | 0.31 | 0.69 | 1.35 |
| fft | 0.062 | 0.058 | 0.061 | 0.045 | 0.367 | 0.221 | 0.186 | 0.17 | 0.28 | 0.33 | 0.12 | 0.20 | 0.24 |
| fib | 0.498 | 0.533 | 0.626 | 0.433 | 1.460 | 1.102 | 0.290 | 0.34 | 0.45 | 1.72 | 0.30 | 0.39 | 1.49 |
| fibc | 0.041 | 0.038 | 0.039 | 0.033 | 0.357 | 0.656 | 0.624 | 0.11 | 0.06 | 0.07 | 0.09 | 0.05 | 0.05 |
| fibfp | 0.187 | 0.190 | 0.217 | 0.141 | 0.530 | 0.854 | 0.684 | 0.35 | 0.22 | 0.27 | 0.27 | 0.17 | 0.21 |
| gcbench | 0.024 | 0.023 | 0.026 | 0.021 | 0.169 | 0.033 | 0.020 | 0.14 | 0.73 | 1.20 | 0.12 | 0.64 | 1.05 |
| graphs | 0.131 | 0.136 | 0.149 | 0.120 | 0.653 | 0.208 | 0.071 | 0.20 | 0.63 | 1.85 | 0.18 | 0.58 | 1.69 |
| lattice | 0.061 | 0.058 | 0.067 | 0.053 | 0.242 | 0.111 | 0.045 | 0.25 | 0.55 | 1.36 | 0.22 | 0.48 | 1.18 |
| matrix | 0.084 | 0.083 | 0.096 | 0.077 | 0.371 | 0.133 | 0.061 | 0.23 | 0.63 | 1.38 | 0.21 | 0.58 | 1.26 |
| maze | 0.042 | 0.044 | 0.050 | 0.036 | 0.228 | 0.087 | 0.045 | 0.18 | 0.48 | 0.93 | 0.16 | 0.41 | 0.80 |
| mazefun | 0.143 | 0.152 | 0.161 | 0.124 | 0.439 | 0.293 | 0.098 | 0.33 | 0.49 | 1.46 | 0.28 | 0.42 | 1.27 |
| mbrot | 0.056 | 0.052 | 0.057 | 0.039 | 0.241 | 0.329 | 0.342 | 0.23 | 0.17 | 0.16 | 0.16 | 0.12 | 0.11 |
| mperm | 0.157 | 0.168 | 0.176 | 0.121 | 0.447 | 0.205 | 0.065 | 0.35 | 0.77 | 2.42 | 0.27 | 0.59 | 1.86 |
| nboyer | 0.779 | 0.808 | 0.862 | 0.650 | 1.246 | 0.930 | 0.348 | 0.63 | 0.84 | 2.24 | 0.52 | 0.70 | 1.87 |
| nqueens | 0.331 | 0.325 | 0.348 | 0.277 | 0.694 | 0.500 | 0.179 | 0.48 | 0.66 | 1.85 | 0.40 | 0.55 | 1.55 |
| ntakl | 0.180 | 0.195 | 0.199 | 0.116 | 0.533 | 0.321 | 0.080 | 0.34 | 0.56 | 2.25 | 0.22 | 0.36 | 1.45 |
| nucleic | 0.075 | 0.071 | 0.076 | 0.070 | 0.497 | 0.209 | 0.157 | 0.15 | 0.36 | 0.48 | 0.14 | 0.33 | 0.45 |
| paraffins | 0.045 | 0.045 | 0.047 | 0.042 | 0.149 | 0.056 | 0.034 | 0.30 | 0.80 | 1.32 | 0.28 | 0.75 | 1.24 |
| peval | 0.113 | 0.112 | 0.121 | 0.085 | 0.477 | 0.223 | 0.086 | 0.24 | 0.51 | 1.31 | 0.18 | 0.38 | 0.99 |
| pnpoly | 0.084 | 0.081 | 0.092 | 0.068 | 0.260 | 0.372 | 0.238 | 0.32 | 0.23 | 0.35 | 0.26 | 0.18 | 0.29 |
| primes | 0.158 | 0.175 | 0.177 | 0.144 | 0.667 | 0.343 | 0.221 | 0.24 | 0.46 | 0.71 | 0.22 | 0.42 | 0.65 |
| puzzle | 0.104 | 0.106 | 0.115 | 0.067 | 0.296 | 0.252 | 0.091 | 0.35 | 0.41 | 1.14 | 0.23 | 0.27 | 0.74 |
| quicksort | 0.114 | 0.113 | 0.122 | 0.088 | 0.315 | 0.201 | 0.080 | 0.36 | 0.57 | 1.43 | 0.28 | 0.44 | 1.10 |
| sboyer | 0.712 | 0.773 | 0.834 | 0.580 | 1.293 | 0.979 | 0.314 | 0.55 | 0.73 | 2.27 | 0.45 | 0.59 | 1.85 |
| simplex | 0.122 | 0.116 | 0.125 | 0.094 | 0.554 | 0.254 | 0.117 | 0.22 | 0.48 | 1.04 | 0.17 | 0.37 | 0.80 |
| string | 0.022 | 0.021 | 0.021 | 0.023 | 0.141 | 0.029 | 0.030 | 0.16 | 0.76 | 0.73 | 0.16 | 0.79 | 0.77 |
| sum | 0.341 | 0.375 | 0.366 | 0.195 | 0.840 | 0.650 | 0.173 | 0.41 | 0.52 | 1.97 | 0.23 | 0.30 | 1.13 |
| sumfp | 0.106 | 0.109 | 0.115 | 0.063 | 0.287 | 0.573 | 0.539 | 0.37 | 0.18 | 0.20 | 0.22 | 0.11 | 0.12 |
| tak | 0.460 | 0.467 | 0.582 | 0.369 | 1.276 | 0.977 | 0.238 | 0.36 | 0.47 | 1.93 | 0.29 | 0.38 | 1.55 |
| takl | 0.091 | 0.087 | 0.102 | 0.061 | 0.319 | 0.166 | 0.045 | 0.29 | 0.55 | 2.02 | 0.19 | 0.37 | 1.36 |
| triangl | 0.165 | 0.176 | 0.197 | 0.123 | 0.423 | 0.427 | 0.134 | 0.39 | 0.39 | 1.23 | 0.29 | 0.29 | 0.92 |
| **geomean** |  |  |  |  |  |  |  | **0.27** | **0.44** | **0.96** | **0.22** | **0.36** | **0.78** |

Times in seconds.

### lox

| benchmark | tailcall | goto | switch | clox | tailcall/clox |
|---|---:|---:|---:|---:|---:|
| binary_trees | 0.939 | 0.950 | 0.947 | 3.218 | 0.29 |
| equality | 0.983 | 0.969 | 1.761 | 3.132 | 0.31 |
| fib | 0.401 | 0.411 | 0.420 | 0.989 | 0.41 |
| instantiation | 0.370 | 0.373 | 0.379 | 0.537 | 0.69 |
| invocation | 0.141 | 0.141 | 0.140 | 0.276 | 0.51 |
| method_call | 0.083 | 0.091 | 0.087 | 0.174 | 0.48 |
| properties | 0.200 | 0.197 | 0.192 | 0.440 | 0.45 |
| trees | 1.340 | 1.473 | 1.421 | 2.445 | 0.55 |
| zoo | 0.152 | 0.159 | 0.153 | 0.313 | 0.49 |
| zoo_batch | 0.919 | 0.948 | 0.915 | 1.946 | 0.47 |
| **geomean** |  |  |  |  | **0.45** |

Times in seconds.

### regex

| benchmark | tailcall | goto | switch | pcre2 | python-re | tailcall/pcre2 | tailcall/python-re |
|---|---:|---:|---:|---:|---:|---:|---:|
| alt-heavy | 24.0 | 23.7 | 25.6 | 50.0 | 32.1 | 0.48 | 0.75 |
| capture | 17.2 | 18.2 | 19.3 | 51.3 | 110.3 | 0.34 | 0.16 |
| class | 63.3 | 62.4 | 63.6 | 220.9 | 154.1 | 0.29 | 0.41 |
| dot-star | 10.5 | 9.9 | 10.4 | 21.5 | 8.1 | 0.49 | 1.29 |
| literal | 3.2 | 3.2 | 3.2 | 2.6 | 2.5 | 1.27 | 1.29 |
| literal-alt | 9.1 | 9.1 | 9.3 | 18.1 | 36.7 | 0.50 | 0.25 |
| patho-alt | 45.3 | 47.7 | 61.1 | 155.2 | 125.9 | 0.29 | 0.36 |
| pathological | 0.0 | 0.0 | 0.0 | 0.0 | 431.7 | — | — |
| words | 19.9 | 21.8 | 22.4 | 54.6 | 155.2 | 0.37 | 0.13 |
| **geomean** |  |  |  |  |  | **0.44** | **0.42** |

Times in milliseconds.

### wasm

| benchmark | tailcall | goto | switch | wasm3 | tailcall/wasm3 |
|---|---:|---:|---:|---:|---:|
| binarytrees | 1.446 | 1.610 | 2.094 | 1.449 | 1.00 |
| coremark | 1.933 | 2.021 | 2.974 | 2.030 | 0.95 |
| fannkuchredux | 1.498 | 1.534 | 2.437 | 1.660 | 0.90 |
| fasta | 1.151 | 1.192 | 1.522 | 1.151 | 1.00 |
| mandelbrot | 1.506 | 1.614 | 3.050 | 2.005 | 0.75 |
| nbody | 1.918 | 2.084 | 3.894 | 2.007 | 0.96 |
| spectralnorm | 1.607 | 1.668 | 2.948 | 1.909 | 0.84 |
| **geomean** |  |  |  |  | **0.91** |

Times in seconds.

### stack

| benchmark | tailcall | goto | switch |
|---|---:|---:|---:|
| fib | 0.054 | 0.055 | 0.072 |
| loop | 0.740 | 0.710 | 0.851 |
| nbody | 0.149 | 0.162 | 0.209 |
| sieve | 0.092 | 0.094 | 0.110 |
| strings | 0.144 | 0.140 | 0.151 |
| **geomean** |  |  |  |

Times in seconds.

