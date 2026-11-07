# Benchmark results

One run of every example interpreter against its reference
implementations, made with [`bench/run_all.py`](../bench/run_all.py).
This page replaces the ratios quoted elsewhere (README, `status.md`,
`roadmap.md`); the per-interpreter logs (`bench/*/RESULTS.md`,
`bench/scheme/README.md`, `bench/prolog/README.md`, the examples'
`FRICTION.md`) keep their dated history.

The tables below are the **2026-11-07 run at 9dce087**. They supersede
the 2026-11-05 run at c7e94ae, whose summary is in "Changes since the
previous run (c7e94ae)" below; the dated A/Bs made between the two runs
(2026-11-05 to 2026-11-07, further down) are kept as they were written,
and their changes are in this run's tables.

## Setup

| | |
|---|---|
| Date | 2026-11-07 |
| Commit | master 9dce087 |
| CPU | Intel Core i7-6700 @ 3.40 GHz, 4 cores / 8 threads |
| Kernel | Linux 6.12.113+deb13-amd64 |
| Compilers | clang 19.1.7 (Debian), rustc 1.97.1 |
| Other | Python 3.13.5, node v26.13.0, PCRE2 10.46 (system library) |

No other agent ran, and nothing ran beside the benchmarks. Each chunk
waited for the 1-minute load average to fall below 1.0, at most 3
minutes (0–10 s each time); it was 0.85–0.99 at the start of each chunk
and 1.0–1.15 at the end, our own benchmark process included (the
previous run's was 0.9–2.6 during the runs). The references took
0.93–0.97 of their previous run's time by suite geometric mean, so
compare ratios, not seconds, across runs; allocation-heavy programs
moved more under every implementation (Mini's `binary_trees`, `sieve`
and `string_build`: PUC-Lua 0.80, 0.86 and 0.66 of its previous time,
LuaJIT `-joff` 0.87, 0.73 and 0.75; LuaJIT `-joff` `hash_strings` 1.28).
The suites ran as six `run_all.py --only` invocations into one output
directory (mini, mini-coro: 6.5 minutes; minijs, lox: 9; scheme, regex:
6; wasm, wasm-simd, stack: 5.5; python: 3; prolog: 11), 41 minutes in
all, then the PolyBench set (2 minutes). Single benchmarks move by
about ±3% between runs from code placement and load.

**Method.**

- Every Loimi interpreter is built with `--release --align-functions=64`
  in each dispatch mode (`--dispatch=tailcall`, `goto`, `switch`).
  Mini, MiniJS, Scheme and Prolog are also built with `--jit-stencils`
  (tail-call dispatch) and run with `LOIMI_JIT=1` (column *jit*). Every
  x86-64 vm branch is kept off 32-byte boundaries (the JCC erratum,
  [lowering.md §12](lowering.md#12-driver-settings)), and the vms with a
  `#[profile]` lay out tail-call and goto builds hottest handler first
  (since this run's previous one).
- The suites' own scripts do the timing: `bench/mini/compare.py`,
  `bench/mini/coroutines/run.py`, `bench/minijs/compare.py`,
  `bench/scheme/compare.py`, `bench/lox/compare.py`,
  `bench/wasm/bench.py`, `examples/stack/bench/run.py`,
  `bench/python/compare.py`, `bench/prolog/run.py`, and for the regex
  library `rxhost bench` (`examples/regex/host.c`, built with PCRE2) plus
  `examples/regex/bench.py` (Python `re`). Every output is checked against
  the expected output (Lox: against clox's; Scheme: the benchmark's own
  check and identical output across implementations; Python: CPython's;
  Prolog: `done`; PolyBench: `polybench.sh --check`, the arrays of the
  `SMALL_DATASET` builds against wasm3's, all three builds).
- Times are the minimum wall-clock time of the whole process over 3 runs,
  in seconds, with the implementations interleaved within each benchmark
  (`bench.py` does not interleave, so it runs in 3 rounds of one run).
  The regex times are measured inside the process, each the best of 3
  matches over 4 MiB of text, and the minimum over 3 processes, in
  milliseconds. Prolog runs over 10 s are not repeated (`run.py --slow
  10`); none was, this time.
- **New in this run: a direct check against the previous run's code.**
  The ratios against references move with the host (above), so the
  tail-call and JIT builds of c7e94ae (built from a worktree with that
  commit's compiler and sources, same flags) ran interleaved with this
  run's, with an A/A copy of the old binary in the same interleave:
  user+sys CPU time of the process (`getrusage`), 7 runs (Prolog, wasm,
  PolyBench: 5), min and median, every output equal to the old build's.
  See "Changes since the previous run (c7e94ae)".
- Ratios are ours divided by the reference: below 1, ours takes less
  time. Geometric means are over every benchmark of the suite.
- References: PUC-Lua 5.4.9 and LuaJIT 2.1 (`-joff`, its assembly
  interpreter) from `bench/lua/install.sh`; `node --jitless` (V8's
  Ignition interpreter) and QuickJS 2026-06-04 (`bench/js/install.sh`);
  Gauche 0.9.15 and Guile 3.0.11 with its JIT off (`guile-nojit`,
  `GUILE_JIT_THRESHOLD=-1`) and on (`guile`) (`bench/scheme/install.sh`;
  chibi-scheme was not run, it is 10× slower than the others); clox
  (`bench/lox/install.sh`); wasm3 0.5.0 (`bench/wasm/install.sh`); PCRE2
  through its C API without JIT, and CPython 3.13's `re`; CPython 3.13.18
  (`-O3`, no PGO or LTO) and MicroPython 1.29.0 (unix port, `-X
  heapsize=256M`) (`bench/python/install.sh`); SWI-Prolog 10.0.2 (`-O`)
  and Trealla 3.14.46 (`bench/prolog/install.sh`). Stak has no
  reference: the CPython versions quoted in its `FRICTION.md` are not in
  the repository. The `wasm-simd` suite's reference (`mvp`) is our own
  tail-call time on the MVP build of the same program (wasm3 0.5 has no
  SIMD).
- The wasm PolyBench set (held out from the tuning of `examples/wasm`,
  [`bench/wasm/README.md`](../bench/wasm/README.md)) is not part of
  `run_all.py`: it ran as 3 rounds of `bench.py -n 1 --dir
  bench/wasm/polybench-out` with wasm3 and the three `run_all.py` wasm
  builds, keeping the minimum.

To reproduce (about an hour; `--quick` runs one benchmark per suite,
once, mostly builds):

```sh
bench/lua/install.sh; bench/js/install.sh; bench/scheme/fetch.sh; bench/scheme/install.sh
bench/lox/install.sh; bench/wasm/install.sh; bench/wasm/programs.sh; bench/wasm/programs-simd.sh
bench/wasm/polybench.sh; bench/python/install.sh; bench/prolog/install.sh
bench/run_all.py [--quick] [--only mini,scheme] [--out DIR]
bench/run_all.py --report DIR/results.json     # the tables below, again
bench/wasm/polybench.sh --check DIR/wasm-tailcall
bench/wasm/bench.py -n 1 --dir bench/wasm/polybench-out --interp="wasm3=bench/wasm/install/bin/wasm3 {wasm}" \
    --interp="tailcall=DIR/wasm-tailcall {wasm}" --interp="goto=DIR/wasm-goto {wasm}" --interp="switch=DIR/wasm-switch {wasm}"
```

## Summary: geometric mean of ours / reference

| interpreter | build | vs tailcall | references (ours / ref) |
|---|---|---:|---|
| mini (14) | tailcall | 1 | lua 0.49, luajit-joff 0.75 |
| mini (14) | goto | 1.02 | lua 0.49, luajit-joff 0.76 |
| mini (14) | switch | 1.12 | lua 0.55, luajit-joff 0.84 |
| mini (14) | jit | 0.76 | lua 0.37, luajit-joff 0.56 |
| mini-coro (3) | tailcall | 1 | lua 0.33, luajit-joff 0.90 |
| mini-coro (3) | goto | 1.06 | lua 0.35, luajit-joff 0.95 |
| mini-coro (3) | switch | 1.10 | lua 0.36, luajit-joff 0.99 |
| mini-coro (3) | jit | 0.77 | lua 0.25, luajit-joff 0.69 |
| minijs (14) | tailcall | 1 | node-jitless 0.48, qjs 0.37 |
| minijs (14) | goto | 1.03 | node-jitless 0.49, qjs 0.38 |
| minijs (14) | switch | 1.12 | node-jitless 0.53, qjs 0.41 |
| minijs (14) | jit | 0.83 | node-jitless 0.40, qjs 0.30 |
| scheme (43) | tailcall | 1 | gauche 0.22, guile-nojit 0.38, guile 0.80 |
| scheme (43) | goto | 1.04 | gauche 0.23, guile-nojit 0.39, guile 0.83 |
| scheme (43) | switch | 1.13 | gauche 0.25, guile-nojit 0.43, guile 0.90 |
| scheme (43) | jit | 0.78 | gauche 0.17, guile-nojit 0.30, guile 0.63 |
| lox (10) | tailcall | 1 | clox 0.42 |
| lox (10) | goto | 1.08 | clox 0.45 |
| lox (10) | switch | 1.06 | clox 0.44 |
| regex (9) | tailcall | 1 | pcre2 0.44, python-re 0.41 |
| regex (9) | goto | 1.04 | pcre2 0.46, python-re 0.43 |
| regex (9) | switch | 1.09 | pcre2 0.48, python-re 0.45 |
| wasm (7) | tailcall | 1 | wasm3 0.85 |
| wasm (7) | goto | 1.05 | wasm3 0.90 |
| wasm (7) | switch | 1.27 | wasm3 1.08 |
| wasm-simd (3) | tailcall | 1 | mvp 0.99 |
| wasm-simd (3) | goto | 1.04 | mvp 1.03 |
| wasm-simd (3) | switch | 1.30 | mvp 1.28 |
| wasm-polybench (30) | tailcall | 1 | wasm3 0.82 |
| wasm-polybench (30) | goto | 1.05 | wasm3 0.87 |
| wasm-polybench (30) | switch | 1.27 | wasm3 1.05 |
| stack (5) | tailcall | 1 | — |
| stack (5) | goto | 1.04 | — |
| stack (5) | switch | 1.10 | — |
| python (11) | tailcall | 1 | cpython 0.31, micropython 0.11 |
| python (11) | goto | 1.07 | cpython 0.33, micropython 0.11 |
| python (11) | switch | 1.16 | cpython 0.35, micropython 0.12 |
| prolog (30) | tailcall | 1 | swipl 0.47, trealla 0.11 |
| prolog (30) | goto | 1.08 | swipl 0.51, trealla 0.12 |
| prolog (30) | switch | 1.21 | swipl 0.57, trealla 0.14 |
| prolog (30) | jit | 0.94 (0.93 without sieve) | swipl 0.45, trealla 0.11 (swipl 0.43, trealla 0.10 over 29, without sieve) |
| mini (14) | jit + AOT traces ‡ | 0.764 of jit | luajit-joff 0.44, luajit (JIT on) 1.58 |
| minijs (14) | jit + AOT traces ‡ | 0.866 of jit | (not run against the references) |

(n) is the number of benchmarks. *vs tailcall* is the geometric mean of
that build's times over the tail-call build's. Suites: `mini` (Mini, a
Lua 5.4 subset, 14 clean-room benchmarks), `mini-coro` (Mini's three
coroutine benchmarks), `minijs` (14 clean-room benchmarks), `scheme` (43
r7rs-benchmarks programs), `lox` (the book's benchmarks), `regex` (the
backtracking engine), `wasm` (C programs compiled to MVP wasm32-wasi),
`wasm-simd` (three of them compiled with `-msimd128`; *mvp* is the
tail-call time of the MVP build in every row), `wasm-polybench` (the 30
PolyBench/C kernels, a held-out set; not run by `run_all.py`), `stack`
(Stak), `python` (the Python subset, 11 pyperformance programs),
`prolog` (the 30 van Roy / Aquarius programs of SWI-Prolog's bench
suite).

‡ Not from `run_all.py`: `--jit-stencils` builds with ahead-of-time
traces from a profile of the suite's own 14 programs, **trained on the
benchmark inputs themselves**, against the same run's JIT build, user+sys
CPU time; see "Trace tier in this run" below. Trained on quarter-size
inputs instead, Mini's is 0.765.

## Notes

- Mini (tail-call) takes 0.31–0.71 of PUC-Lua's time and 0.46–0.93 of
  LuaJIT `-joff`'s: less on all 14 (queens 0.93, spectral_norm 0.92;
  hash_strings 0.71, against 0.96 in the previous run, as LuaJIT's time
  on it rose). With the JIT: 0.59 (mandelbrot) to 0.96 (string_build) of
  the interpreter's time, 0.76 by geometric mean. With ahead-of-time
  traces on top, 0.44 of LuaJIT `-joff`'s time and 1.58 of LuaJIT's with
  its JIT (fannkuch 1.00; spectral_norm 6.6, fib 3.3).
- Mini's coroutine benchmarks switch threads inside the dispatch loop
  (`examples/mini/COROUTINES.md`): 0.31–0.35 of PUC-Lua's time and
  0.88–0.92 of LuaJIT `-joff`'s (chameneos 0.92, 1.06 in the previous
  run, now that call sites that switched quicken to CALL_RESUME,
  CALL_WRAP and CALL_YIELD); goto and switch take 1.05–1.11× tail-call.
- MiniJS takes 0.22–0.73 of `node --jitless`'s time and 0.20–0.59 of
  QuickJS's, less on all 14. Switch mode takes 1.12× the tail-call time
  (1.21× in the previous run; closures 1.30×).
- Scheme takes less time than Guile with its JIT on 21 of 43 programs
  (geometric mean 0.80); with Loimi's JIT, on 30 (0.63). The bignum
  programs: pi 1.12× Guile, chudnovsky 0.96×.
- Lox takes 0.25–0.62 of clox's time (0.42 by geometric mean, 0.45 in
  the previous run: `std.rt.ValueStack` and re-fusion). goto takes 1.08×
  tail-call by geometric mean (zoo 1.23×, zoo_batch 1.24×), switch 1.06×.
- Regex: the backtracker takes less time than the PCRE2 interpreter and
  Python's `re` except on `literal` (1.22 and 1.23) and, against Python,
  `dot-star` (1.24). `pathological` is rejected by the required-byte check
  before matching (0.0 ms; Python 432 ms) and is left out of the
  geometric means.
- Wasm takes 0.75–0.95 of wasm3's time on the seven programs, less on
  all, and 0.66–0.93 on the 30 PolyBench kernels (0.82 by geometric mean;
  less on all 30). Switch builds take 1.27× the tail-call time on both
  sets (PolyBench correlation 1.87×, gramschmidt 1.46×). goto-mode gemm
  is 1.08× tail-call (1.42× in the previous run: hot-handler ordering).
  The SIMD build of nbody takes 0.97 of its MVP build's time;
  fannkuchredux and spectralnorm 1.00.
- Python (tail-call) takes 0.21–0.39 of CPython's time, less on all 11
  (hexiom 0.39, nqueens 0.38; richards 0.21). MicroPython's output
  differs on chaos and go (its `random` is another generator) and nbody
  (last float digits); their times are in the table and in the geometric
  means (`compare.py` leaves them out of its own). goto takes 1.07× the
  tail-call time, switch 1.09–1.24×.
- Prolog takes 0.22–0.82 of SWI-Prolog's time, less on all 30 (tak 0.22;
  sieve, whose assert/retract compile clauses, 0.82), and 0.05–0.33 of
  Trealla's. The JIT build takes 0.94 of the interpreter's time (0.93
  over the 29 programs other than sieve; 0.99 in the previous run): 0.72
  tak to 1.06 sendmore, and 1.35 on sieve. goto and switch take 1.08× and
  1.21× tail-call (1.03× and 1.11× in the previous run): the
  `nanbox(int_high)` port made the tail-call build 0.90 of its old time
  and the other two less (0.95, 0.98 against SWI-Prolog). `run_all.py`
  still marks the JIT cell of sieve † (its `PATHOLOGICAL` table); the mark
  is left out here.

## Changes since the previous run (c7e94ae)

The previous run (2026-11-05, master c7e94ae, same machine and method)
against this one. Two views:

**Against the references** (as in earlier runs): over the benchmarks both
runs have, *references' time* is the new/old geometric mean of each
reference's times (the host's drift); each mode column is the geometric
mean over benchmarks of (ours / first reference) new over old, below 1
when ours gained on the reference. *ratio* is the tail-call (or JIT)
build over the first reference, old → new.

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

Mini's 1.03 against PUC-Lua is the host, not Mini: PUC-Lua took 0.66
and 0.80 of its old time on string_build and binary_trees, against 0.97
on most programs (above), and the direct check below has Mini at 0.993.

**Directly, old build against new** (user+sys CPU, interleaved, an A/A
copy of the old binary in the same interleave; geometric means, min /
median of the runs):

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

(Lox's zoo_batch runs for a fixed 10 s of `clock()`; wasm's coremark
prints its own timing, so its output differs from run to run; both are
left out here and are in the per-suite tables.) Regex, the wasm SIMD
builds and the goto and switch builds were not run this way.

**Regressions.** Three single-benchmark losses held when rerun (11
interleaved runs, A/A within 0.3%) and two were bisected over the
first-parent history in a separate worktree (`git bisect --first-parent`,
7 runs per step):

- **Scheme with the JIT, puzzle 1.11 and array1 1.07** (0.062 → 0.069 s,
  0.236 → 0.251 s); the interpreter is unchanged on both (1.00).
  First bad commit: 3e0de22 "errors and deoptimization are cold by
  default" (its parent f7688b7: puzzle 1.00; 3e0de22 against f7688b7:
  puzzle 1.11, array1 1.07). Investigated afterwards: this is where the
  JIT's code lands. The hot stencils are unchanged. A few copies that run
  outside the loops (`PRIMCALL`, `CALLK`) got 5 bytes shorter and moved
  the loops. Starting the JIT region 16 bytes later swaps the two
  builds' times, and placements that avoid the JCC erratum or align loop
  heads do not help over the suite
  ([design/copy-and-patch.md §15](design/copy-and-patch.md#15-placement-of-compiled-code-the-scheme-puzzlearray1-regression-2026-11-07)).
- **wasm tail-call binarytrees 1.06** (1.247 → 1.324 s). First bad
  commit: 5667a53 "examples: dispatch profiles for hot-handler ordering"
  (`#[profile]`; its parent 39c715f: 1.00). The suite is unchanged
  (1.003), and the hot-handler A/B measured wasm 1.001; a placement draw
  of ordering, like the two it fixed. Not investigated further.
- **MiniJS with the JIT, closures 1.03** (0.627 → 0.648 s; 1.034 again in
  a rerun of 11). Not bisected.

Switch-mode cells, not part of the direct check, moved most against the
references: PolyBench correlation 1.54 (1.87× the tail-call time),
Prolog tak 1.33, Mini string_build 1.30 (Lua's own drift above),
PolyBench gramschmidt 1.24.

What changed, from `git log c7e94ae..9dce087` (the dated sections below
measured each):

- **Hot-handler placement** (`#[profile]`, 2026-11-05): Lox properties
  and PolyBench goto-mode gemm (1.42× tail-call before, 1.08× now).
- **Value accumulators** in Mini and MiniJS (2026-11-06): Mini
  mandelbrot 0.90, MiniJS nbody 0.94.
- **`nanbox(int_high)`** in Python and Prolog (2026-11-06): Python 0.93,
  Prolog 0.90 (the JIT build 0.85).
- **`std.rt.ValueStack`** in Lox, Scheme and MiniJS and **re-fusion** in
  Lox (2026-11-06, 2026-11-07): Lox 0.93 (fib 0.87, properties and zoo
  0.85); Scheme and MiniJS unchanged.
- **Mini's direct SWITCH and quickened coroutine CALLs** (2026-11-06):
  interp 0.97, the coroutine suite 0.91 (the JIT build 0.89).
- **`Divisor`**, cold-by-default errors, `rel`/`data`, supers groups,
  `--vm-prof`: no suite-level change; the cold-by-default commit is the
  Scheme JIT regression above.
- **The trace tier** (M1, traces through calls, side traces): opt-in;
  the next section.

## Trace tier in this run

The ahead-of-time trace tier
([design/trace-tier.md §11–§13](design/trace-tier.md#11-m1-as-built-ahead-of-time-traces-from-a-profile))
by its workflow (§11.1): each of the suite's 14 programs recorded by this
run's `--jit-stencils` build (`LOIMI_TRACE_RECORD`), one profiling build
over the 14 recordings (`--trace-regions`), each program profiled by it
(`LOIMI_TRACE_PROFILE`), one `--traces` build from the 14 profiles. All
builds `--release --align-functions=64`. **The traces are trained on the
benchmark inputs themselves**; *trace (¼ input)* is a second `--traces`
build, recorded and profiled on copies of the programs whose size
constant `N` is a quarter (fib 32, queens 10, fannkuch 9, binary_trees
14), then run on the full programs. Workflow time: Mini 2.8 minutes (35
traces, 45 side traces; ¼ input: 33 and 45), MiniJS 3.2 minutes (63 and
27). User+sys CPU seconds, minimum of 7 interleaved runs (MiniJS 5),
every output equal to the JIT's; *JIT A/A* (a copy of the JIT binary)
and *trace off* (the trace binary with `LOIMI_TRACE=0`) are the
controls. LuaJIT ran in the same interleave.

| Mini | JIT | JIT A/A | trace | trace off | trace (¼ input) | luajit-joff | luajit | trace/JIT | ¼/JIT | trace/luajit-joff | trace/luajit |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| binary_trees | 0.628 | 0.623 | 0.625 | 0.623 | 0.628 | 1.358 | 0.940 | 0.99 | 1.00 | 0.46 | 0.66 |
| closures | 0.352 | 0.352 | 0.352 | 0.352 | 0.352 | 0.911 | 0.758 | 1.00 | 1.00 | 0.39 | 0.46 |
| fannkuch | 0.865 | 0.865 | 0.302 | 0.865 | 0.304 | 1.383 | 0.302 | **0.35** | 0.35 | 0.22 | 1.00 |
| fib | 0.641 | 0.641 | 0.634 | 0.634 | 0.634 | 1.088 | 0.195 | 0.99 | 0.99 | 0.58 | 3.26 |
| hash_strings | 0.729 | 0.728 | 0.634 | 0.731 | 0.638 | 0.988 | 0.224 | **0.87** | 0.88 | 0.64 | 2.83 |
| interp | 0.545 | 0.545 | 0.387 | 0.543 | 0.366 | 1.052 | 0.353 | **0.71** | 0.67 | 0.37 | 1.10 |
| mandelbrot | 0.331 | 0.331 | 0.179 | 0.331 | 0.196 | 0.758 | 0.152 | **0.54** | 0.59 | 0.24 | 1.18 |
| nbody | 0.320 | 0.318 | 0.207 | 0.319 | 0.206 | 0.500 | 0.087 | **0.65** | 0.64 | 0.41 | 2.38 |
| oo_methods | 0.451 | 0.451 | 0.450 | 0.450 | 0.441 | 0.974 | 0.182 | 1.00 | 0.98 | 0.46 | 2.47 |
| queens | 0.642 | 0.642 | 0.574 | 0.642 | 0.572 | 0.911 | 0.446 | **0.89** | 0.89 | 0.63 | 1.29 |
| sieve | 0.442 | 0.435 | 0.345 | 0.435 | 0.343 | 0.736 | 0.211 | **0.78** | 0.78 | 0.47 | 1.63 |
| sort | 0.571 | 0.570 | 0.390 | 0.571 | 0.389 | 0.827 | 0.305 | **0.68** | 0.68 | 0.47 | 1.28 |
| spectral_norm | 0.713 | 0.713 | 0.465 | 0.713 | 0.465 | 1.124 | 0.070 | **0.65** | 0.65 | 0.41 | 6.61 |
| string_build | 0.569 | 0.569 | 0.568 | 0.562 | 0.566 | 0.715 | 0.372 | 1.00 | 0.99 | 0.79 | 1.53 |
| **geometric mean** | | | | | | | | **0.764** | **0.765** | **0.442** | **1.58** |

Controls: JIT A/A 0.998, trace off / JIT 0.996; medians give 0.766 and
0.768. The JIT alone is 0.58 of LuaJIT `-joff`'s time and 2.07 of
LuaJIT's. Against §13.6 (a loaded machine, 0.759) the suite is the same.
Training on a quarter of the input changes nothing measurable except
interp (0.67, better) and mandelbrot (0.59, worse): the main chunk's
changed constant does not stop its traces from installing.

| MiniJS | JIT | JIT A/A | trace | trace off | trace/JIT | (medians) |
|---|---:|---:|---:|---:|---:|---:|
| awfy_micro | 0.622 | 0.622 | 0.488 | 0.627 | **0.79** | 0.79 |
| binary_trees | 0.738 | 0.739 | 0.742 | 0.744 | 1.01 | 1.00 |
| bitops | 0.523 | 0.522 | 0.496 | 0.522 | 0.95 | 0.95 |
| closures | 0.648 | 0.648 | 0.602 | 0.648 | 0.93 | 0.93 |
| deltablue | 0.646 | 0.647 | 0.651 | 0.647 | 1.01 | 1.01 |
| fannkuch | 0.364 | 0.364 | 0.275 | 0.364 | **0.76** | 0.76 |
| json | 0.822 | 0.823 | 0.753 | 0.830 | 0.92 | 0.92 |
| nbody | 0.293 | 0.293 | 0.174 | 0.293 | **0.59** | 0.59 |
| properties | 0.687 | 0.689 | 0.613 | 0.686 | 0.89 | 0.89 |
| raytrace | 0.398 | 0.400 | 0.399 | 0.401 | 1.00 | 1.00 |
| richards | 0.610 | 0.611 | 0.617 | 0.621 | 1.01 | 1.01 |
| sort | 0.662 | 0.666 | 0.617 | 0.664 | 0.93 | 0.93 |
| spectral_norm | 0.653 | 0.653 | 0.393 | 0.653 | **0.60** | 0.60 |
| strings | 0.974 | 0.974 | 0.893 | 0.971 | 0.92 | 0.92 |
| **geometric mean** | | | | | **0.866** | 0.866 |

Controls: JIT A/A 1.001, trace off / JIT 1.003. No MiniJS program is
more than 1% slower than the JIT; §13.5's five programs (nbody,
spectral_norm, fannkuch, bitops, sort) are 0.59–0.95 here, against
0.53–0.95 there (nbody 0.58, spectral_norm 0.53, fannkuch 0.79, bitops
0.89, sort 0.95; a loaded machine).

## GC: mark bitmaps instead of sweeping (2026-11-04)

A dated A/B of one change, made before the 2026-11-04 run (95a227e); its
effect is in the "Changes from 95a227e to c7e94ae" section. `std.gc`
stopped sweeping: marking sets a bit per 16-byte granule in each chunk's
bitmap, and each size class bump-allocates through the runs of clear
bits ([stdlib/gc.md](stdlib/gc.md#heap-layout)).

**Where the time went before.** With `LOIMI_GC_STATS=1` (per-collection
mark and sweep times), the sweep was most of the collector's time
everywhere: Prolog (non-generational, 1 MiB budget, 40-1500 KB live)
spent 3-29% of its run time in the collector, 80-90% of that in the
sweep (nreverse: 2434 collections, 247 ms sweep, 35 ms mark, of 1.07
s); Python (non-generational) 2-11% (nqueens: 111 ms sweep, 3 ms mark,
of 1.0 s); Scheme, Mini and MiniJS (generational) spent it in lazy
sweeping, 13-23% of Scheme's deriv, diviter, divrec, cpstak and primes
and 9-14% of Mini's closures and binary_trees, against 0-2% in minor
marking. The sweep read and wrote every slot of every chunk allocated
from since the last collection: work in proportion to the allocation
budget, almost all of it on dead objects.

**Result.** Interleaved A/B of the committed std against the one before
(same compiler), `--release --align-functions=64`, tail-call dispatch,
minimum of 3 runs, on a machine shared with another agent (load average
4.5-6 on 8 threads). Time new/old, geometric means; the noise is the
geometric mean of two columns built from the same code in the same run.

| Suite | new/old | range | noise |
|---|---|---|---|
| Prolog (29, not sieve) | 0.885 (0.876, 0.885 in two other runs) | 0.79 qsort, 0.80 nreverse, 0.82 derive .. 0.97 reducer | |
| Scheme (43) | 0.955 (0.949, 0.942) | 0.77 diviter, 0.83 deriv, 0.84 cpstak, 0.86 peval, gcbench .. 1.10 earley (0.91 noise) | 0.996 |
| Python (11) | 0.943 (0.914) | 0.88 nqueens, 0.89 fannkuch, 0.90 chaos .. 1.01 | |
| Mini (14) | 0.979 (0.977) | 0.82 binary_trees, 0.88 closures, 0.93 string_build; others ±3%, spectral_norm 1.07 (no GC; 0.95 noise) | 0.998 |
| MiniJS (14) | 0.975 (0.989) | 0.83 closures, 0.92 binary_trees, 0.95 strings; others ±5% | 0.989 |

Peak RSS is unchanged within 1% (Prolog boyer 30.1 to 30.2 MB, sieve
203 to 205 MB, tak 22.7 to 20.8 MB; Scheme nboyer 154.5 to 154.4 MB,
mperm 94.1 to 93.9 MB; Mini binary_trees 139.7 to 139.5 MB; MiniJS
binary_trees 90.6 to 90.7 MB, sort 76.5 to 76.4 MB; Python float 107.6
to 107.5 MB): the heap-growth policy is unchanged and the per-class
chunks leave no holes. What is left of the collector is marking (now
with the bitmap writes, 1.1-1.25 times the old mark time on Prolog)
and `refill` finding runs: nreverse 39 ms mark and 20 ms refill, 8% of
its time (from 26%); boyer 255 ms and 69 ms, 21% (from 28%), its 30 MB
heap leaving many short runs between survivors.

Tried and dropped: allocating every size from one bump run (bits set
over each marked object's granules, so any size fits any hole) was as
fast on Prolog (0.885) but made MiniJS's `sort` 4% slower: its records,
allocated between strings and array storage, spread over more cache
lines (`GETPROP_IN` 6% to 13% of samples). Per-class chunks keep them
together; an all-dead chunk still goes back to a pool shared by the
classes.

**Follow-up (2026-11-04): prefetching marks, fragmented chunks last.**
Marking in a major collection prefetches each header and marks the
reference 16 `mark_ptr` calls later (a new intrinsic, `p.prefetch()`);
chunks whose free runs average under 8 objects are allocated from after
the pool of empty chunks ([stdlib/gc.md](stdlib/gc.md#heap-layout)).
`LOIMI_GC_STATS`, min of 5, mark / refill ms before and after: boyer
224 / 59 to 168 / 45, poly_10 108 / 39 to 113 / 9, tak 94 / 2 to 84 /
2, nreverse 39 / 20 to 36 / 21. Interleaved A/B, user+sys CPU time
(the machine was at load 6-7 on 8 threads; single programs moved by up
to ±10% between runs), new/old geometric means: Prolog (29, not sieve,
min of 5) 0.994, with boyer 0.94, poly_10 0.94, times10 0.94, tak 0.95,
others within noise; Mini 0.995, MiniJS 1.007, Python 0.992, Scheme
1.012 (min of 3; Scheme's tak, which never collects and runs the same
handler code moved by 640 bytes, measured 1.05-1.10 in three runs: a
layout effect). Peak RSS unchanged within noise everywhere (boyer 29.6
MB both; taking the pool before *all* of a class's chunks cut boyer's
refill to 21 ms but grew its peak to 33.5-35 MB). Dropped: tracing
through a FIFO of popped objects without prefetching (no change), and a
plain load instead of the prefetch (boyer mark 252 ms, slower than
before).

## Changes from 95a227e to c7e94ae (the 2026-11-05 run; superseded)

Superseded by the 2026-11-07 run above: this section compared the
2026-11-05 run (c7e94ae), whose tables this page no longer shows (see
`git show 9dce087:docs/results.md`), with the run before it; "these",
"this run" and "the tables below" in it mean the 2026-11-05 run.

The previous tables (commit 573511e, a run at 95a227e, 2026-11-04, same
machine and method) against these, over the benchmarks both runs have.
The host ran this run's programs in about 0.75–0.8 of the time (*references'
time*, new/old geometric mean of every reference of the suite), so the
mode columns divide that out: each is the geometric mean over benchmarks
of (ours / first reference) new over old, below 1 when ours gained on
the reference. *ratio* is the tail-call (or JIT) build over the first
reference, old → new.

| suite | references' time | tailcall | goto | switch | jit | ratio old → new |
|---|---:|---:|---:|---:|---:|---|
| mini (14) | 0.78 | 0.91 | 0.93 | 0.91 | 0.91 | lua 0.52 → 0.47, jit 0.40 → 0.36 |
| mini-coro (3) | 0.77 | 0.98 | 0.96 | 0.97 | 0.92 | lua 0.37 → 0.36, jit 0.31 → 0.29 |
| minijs (14) | 0.76–0.77 | 0.96 | 0.97 | 1.04 | 0.93 | node-jitless 0.50 → 0.48, jit 0.43 → 0.40 |
| scheme (43) | 0.75–0.77 | 0.88 | 0.88 | 0.91 | 0.85 | guile 0.94 → 0.80, jit 0.76 → 0.63 |
| lox (10) | 0.79 | 1.01 | 0.94 | 1.04 | | clox 0.44 → 0.45 |
| regex (8) | 0.79 | 0.99 | 1.02 | 1.03 | | pcre2 0.44 → 0.44 |
| wasm (7) | 0.78 | 0.98 | 0.98 | 0.99 | | wasm3 0.87 → 0.85 |
| wasm-simd (3) | (mvp) | 1.00 | 1.00 | 0.99 | | mvp 0.98 → 0.98 |
| wasm-polybench (30) | 0.77 | 0.93 | 0.96 | 0.94 | | wasm3 0.90 → 0.83 |
| stack (5) | | 0.77 | 0.78 | 0.72 | | (own time new/old; no reference) |
| python (11) | 0.77–0.78 | 0.89 | 0.94 | 0.88 | | cpython 0.37 → 0.33 |
| prolog (30) | 0.76–0.77 | 0.99 | 0.98 | 1.00 | 0.79 (0.88 over 29) | swipl 0.53 → 0.52, jit 0.66 → 0.52 (29: 0.58 → 0.51) |

Stak's own times fell with the host's (0.77 against the references'
0.75–0.79): unchanged. No suite lost more than 4% against its references
by geometric mean (MiniJS and Lox switch builds, 1.04). The largest
single-benchmark losses: PolyBench goto-mode gemm 1.29, MiniJS
switch-mode nbody 1.22, Scheme goto-mode primes 1.18, Lox switch-mode zoo
1.17, Python goto-mode nbody 1.16, Scheme switch-mode takl 1.16, Lox
tail-call properties 1.15. What changed, from `git log 95a227e..c7e94ae`:

- **Mini, third and fourth passes** (`bench/mini/RESULTS.md`, third and
  fourth phases): a word-at-a-time `std.hash.bytes`, `%` by
  multiplication, rotated while loops, frameless table and return
  handlers, young-only sweeps of the string table, constant locals
  folded into nested functions, if/elseif chains on one local as a jump
  table. 0.91 relative to Lua (interp 0.77, sieve 0.72, string_build
  0.75, oo_methods 0.87, hash_strings 0.88; binary_trees 1.06, nbody
  1.07); the JIT build 0.91 (sieve 0.63). Coroutines 0.98.
- **MiniJS, second pass** (`bench/minijs/RESULTS.md`, second
  performance pass): arithmetic slow paths inline, a DIV family, constant
  compares, frameless handlers, calls that do not clear registers. 0.96
  relative to node --jitless (bitops 0.85, strings 0.88, closures 0.91;
  the pass's own A/B: 0.939), the JIT build 0.93; switch mode 1.04
  (nbody 1.22, fannkuch 1.10).
- **Scheme, third and fourth rounds, std.bigint**
  ([bench/scheme/README.md](../bench/scheme/README.md)): builtins as ops,
  allocation without a stack frame, no register clearing on calls,
  aliases, loop-invariant free variables, known calls as `CALLK`, record
  ops, `call/cc` as an op, strings with `std.gc`'s `new_uninit`; GMP-style
  division and Newton square roots in `std.bigint`. 0.88 relative to
  Gauche (gcbench 0.44, deriv 0.61, browse 0.70, paraffins 0.71, conform
  0.75; pi 0.86, chudnovsky 0.88; the rounds' own A/Bs: 0.894 and 0.941),
  the JIT build 0.85.
- **Python, third round**
  ([examples/python/FRICTION.md](../examples/python/FRICTION.md#optimizations-third-round-calls-generator-expressions-slices-2026-11-04)):
  class instantiation and default arguments without the generic call
  path, generator expressions consumed by builtins run inline,
  `super(..).m(..)` as one op, list slices with raw copies. 0.89 relative
  to CPython (nqueens 0.63, fannkuch 0.79, hexiom 0.80); goto 0.94 (nbody
  1.16).
- **The Prolog JIT** (a5db063, 691e424;
  [design/copy-and-patch.md §14](design/copy-and-patch.md#14-prolog-2026-11-04)):
  jumps to resume points count like calls, the code-object registry is a
  treap. The JIT build 0.88 relative to SWI-Prolog over the 29 programs
  other than sieve (times10 0.68, divide10 and poly_10 0.70, tak 0.73,
  meta_qsort 0.75), sieve 58 s → 1.3 s. The interpreter is unchanged
  (0.99).
- **Wasm translator, round three** (98074d4, 2e849e0;
  `examples/wasm/FRICTION.md`, "Round three"): bounds checks against the
  end of the access, array-element loads as one op, result variants for
  ops with several only-assigned operands. 0.98 relative to wasm3 on the
  seven programs, 0.93 on PolyBench (nussinov 0.86, cholesky 0.89; the
  round's own A/B: 0.838 against 0.932 for the old build, 0.90).
- **Branch alignment against the JCC erratum** (476f1a7; [lowering.md
  §12](lowering.md#12-driver-settings)): every x86-64 vm build pads
  branches, dispatch jumps included, off 32-byte boundaries, so every
  interpreter's machine code moved. On the build of the wasm round it
  fixed the 1.4× cholesky, lu and ludcmp took. Over a suite it is a wash:
  this run's binaries against the same code built with
  `--no-branch-align` (user+sys CPU time, MiniJS wall-clock, minimum
  of 2–5 interleaved runs),
  aligned/unaligned: Scheme tail-call 1.00, MiniJS 1.01, Lox 1.02, wasm
  tail-call on PolyBench 1.00, goto 1.01. It has two single-benchmark
  losses of its own, the two above that are not explained by the
  rounds: Lox `properties` 1.13 (0.210 s against 0.186) and PolyBench
  goto-mode gemm 1.37 (0.129 s against 0.094). Not investigated
  further here.
- **No speed change intended**: `--gc-check-roots` fixes, the GC
  allocator's documented invariants, `new_uninit` tests, Mini's and
  Python's `--cfg debug` builds. Regex, Stak, the wasm SIMD builds and
  the Prolog interpreter moved within ±3% relative to their references.

## Hot-handler ordering and the two alignment losses (2026-11-05)

A dated A/B after the 2026-11-05 run; its effect is in the 2026-11-07
run's tables above
([lowering.md §12](lowering.md#12-driver-settings), "What the two
alignment losses were" and "Handler order"). Both single-benchmark
losses that branch alignment brought (Lox `properties` tail-call 1.13,
PolyBench `gemm` goto 1.37) are placement draws rather than the
padding: `gemm` is fast again when the whole text moves by any of 63
other multiples of 64 bytes, and `properties` when its three hot
handlers sit together. Lox, Mini, MiniJS, Stak, Python, the Prolog
interpreter and the wasm interpreter now carry `#[profile]` (their
`*.vmprofile`, from `--vm-stats` over their suites; wasm's over the
seven programs, so PolyBench stays held out), which lays out tail-call
and goto builds hottest handler first in a section of their own. Same
machine, user+sys, minimum of two interleaved passes, ordered over
`--vm-profile=none`:

| suite | tailcall | goto |
|---|---:|---:|
| lox (9) | 0.982 | 0.996 |
| mini (14) | 1.001 | 0.996 |
| mini-coro (3) | 0.974 | 1.016 |
| minijs (14) | 0.999 | 0.996 |
| wasm (7) | 1.001 | 0.999 |
| wasm-polybench (30) | 1.001 | 0.988 |
| stack (5) | 1.003 | 0.999 |
| python (11) | 1.002 | 1.000 |
| prolog (30) | 0.999 | 0.997 |
| scheme (43, not opted in) | 1.008 | 1.000 |

Lox `properties` tail-call 0.212 s → 0.188 s, PolyBench `gemm` goto
0.131 s → 0.094 s (cholesky, lu and ludcmp unchanged, keeping the gains
alignment brought). Switch and `--jit-stencils` builds are not ordered.
In goto builds, `--align-functions=128` moved single benchmarks 1.22% on
average and up to 33% in declaration order, 0.47% and at most 3.1%
ordered (between identical binaries: 0.55%); tail-call builds were as
steady either way. The other two mode ratios that grew in this run are
not placement: unaligned (`--no-branch-align`) builds give MiniJS
switch/tail-call 1.198 against 1.215 aligned, Python goto/tail-call
1.091 against 1.104.

## Value accumulators (2026-11-06)

A dated A/B after the 2026-11-05 run; its effect is in the 2026-11-07
run's tables above
([language.md §7.4](language.md#74-accumulators), "Value accumulators";
[design/speed-audit.md](design/speed-audit.md), item 1, "As built").
Mini keeps a nanbox value's word in an `f64` accumulator between its
float ops, compare-and-branch forms and DIV, with supers over the
variants. User+sys CPU, median of 9 paired interleaved rounds against
the previous build (A/A in brackets; load 4 to 6 from other agents):

| suite | align 64 | align 128 | float benchmark |
|---|---:|---:|---|
| mini (14) | 0.994 (1.000) | 0.996 (0.996) | mandelbrot 0.912 / 0.902 |
| minijs (14), not kept | 1.004 (0.998) | 1.007 (0.982) | nbody 0.944 / 0.942 |
| scheme (43), not kept | 1.003 (0.998) | 1.002 (0.996) | mbrot 0.885 / 0.87 |

The accumulator is a seventh register argument of every handler's tail
call in these three, which stops LLVM folding the dispatch table's load
into the `jmp` ([lowering.md §3](lowering.md#3-handlers-in-tailcall-mode)).
Interpreters without a value accumulator compile to the same code
(`loimi ir-diff`, all three dispatch modes: Lox, Stak, Python, Prolog;
wasm differs only in `W.accumulate`, which now resets earlier variants
first, and the host-side `W.disasm_op`).

**Later the same day: MiniJS ported after all; the fold was not the
cost.** The fold is lost to LLVM's `X86tcret_6regs`, which counts the
tail call's register operands. A pass that folded the load back in the
assembly (commit 0c3bb46, reverted) measured no gain on Mini, MiniJS or
Scheme and lost 2% on the wasm interpreter, which it also folded
([lowering.md §3](lowering.md#3-handlers-in-tailcall-mode)). Remeasured
with more rounds (Mini 11 at align 64), a control build with the
accumulator declared and unselected is within the A/A spread of none
(`bench/mini/RESULTS.md`, item 44), so the "4% on fib" above was noise.
The ports against master, as built (unfolded):

| suite | align 64 | align 128 | float benchmark |
|---|---:|---:|---|
| mini (14), unchanged | 0.999 (0.998) | 0.993 (1.002) | mandelbrot 0.906 / 0.907 |
| minijs (14), **kept** | 0.990 (0.996) | 0.997 (1.003) | nbody 0.974 / 0.931 |
| scheme (43), not kept | 1.007 (1.006) | 0.999 (0.992) | mbrot 0.965 / 0.887 |

Scheme stays out: over five runs, folded or not, it averages 0.3%
slower, with tak and fib slower in every run (a `movq` to the
accumulator on ADD/SUB/MUL's integer paths).

## Side traces (2026-11-07)

Superseded by "Trace tier in this run" above (same build, a quiet
machine: 0.764). Not a run of the suites: the trace tier's Mini benchmarks with side
traces and loops inside loops
([design/trace-tier.md §13](design/trace-tier.md#13-side-traces-and-loops-inside-loops-m2)),
`--jit-stencils` with ahead-of-time traces from a profile of the 14
programs against the same binary's JIT, user+sys CPU seconds, minimum
of 7 interleaved runs.

| | M4 | now, `--align-functions=64` | 128 |
|---|---:|---:|---:|
| trace / JIT, geometric mean | 0.858 | **0.759** | 0.773 |
| JIT A/A | 0.996 | 0.989 | 1.004 |
| trace off / JIT | 1.003 | 0.991 | 1.007 |

Side traces took the suite from 0.812 to 0.763 in a separate round
(`interp` 0.85 → 0.71, fannkuch 0.52 → 0.35, sort 0.79 → 0.68), loops
inside loops from 0.835 to 0.812 (nbody 0.76 → 0.66), and branch weights
and the rest from 0.858 to 0.835. binary_trees, fib, closures,
oo_methods and string_build stay at 0.95–1.00.

## Traces through calls (2026-11-06)

Superseded by "Side traces" and "Trace tier in this run". Not a run
of the suites: the trace tier's Mini benchmarks
([design/trace-tier.md §12](design/trace-tier.md#12-cheaper-exits-and-traces-through-calls-m4-in-part)),
`--jit-stencils` with ahead-of-time traces from a profile of the 14
programs against the same binary's JIT, user+sys CPU seconds, minimum
of 7 interleaved runs.

| | M1 | now, `--align-functions=64` | 128 |
|---|---:|---:|---:|
| trace / JIT, geometric mean | 0.932 | **0.858** | 0.862 |
| JIT A/A | 0.998 | 0.996 | 1.000 |
| trace off / JIT | 1.004 | 1.003 | 1.004 |

Exits that write back only dirty slots took nbody from 1.11 to 0.76;
inlining monomorphic calls took spectral_norm from 1.00 to 0.64. No
program is slower than the JIT; binary_trees, fib, `interp`,
string_build, closures and oo_methods are at 0.99–1.01.

## Re-fusion on `become`; load facts (2026-11-06)

In the 2026-11-07 run's tables above. Not a run of the suites: speed-audit items 8 and 4
([design/speed-audit.md](design/speed-audit.md)), tail-call builds,
user+sys CPU, interleaved runs, an A/A copy of the baseline in the
same interleave (geometric means, min / median of the runs).

| Change | Suite | 64 | A/A | 128 | A/A |
|---|---|---:|---:|---:|---:|
| `refuse fp.cl.proto.code;` + six supers of quickened members | Lox (9) | 0.973 / 0.986 | 1.013 / 0.997 | 0.985 / 0.984 | 1.002 / 1.005 |
| `refuse` in place of the hand-written one | Mini (14) | 0.999 / 0.995 | 0.997 / 0.994 | 1.002 / 1.002 | 1.000 / 1.001 |
| same, reverted (64: the first cut, which also re-fused after misses) | MiniJS (14) | 1.010 / 1.013 | 1.004 / 1.004 | 1.003 / 1.005 | 0.997 / 1.001 |
| hand `refuse` + supers of quickened members, reverted | Python (11) | 0.998 / 0.994 | 1.007 / 0.990 | | |
| same, reverted | Scheme (11) | 0.999 / 1.002 | 0.997 / 1.005 | | |
| `Gc` facts + `!invariant.load` (prototype, not built) | Python deltablue, go, richards | 1.007 / 0.993 | 1.003 / 0.994 | | |
| same | MiniJS properties, deltablue, richards, raytrace | 1.003 / 0.999 | 0.999 / 0.997 | | |
| same | Mini interp | 1.007 / 1.002 | 1.005 / 0.997 | | |

Lox's zoo is 0.88 at both alignments. MiniJS's deltablue is 1.06
slower with the port: re-fusing after property quickening forms
`GETMETHOD_P_CALL` and two `GETPROP_IN` supers, with 10% fewer
dispatches. The load facts change no hot handler of Mini or MiniJS.

## SWITCH to the target; coroutine switch sites (2026-11-06)

In the 2026-11-07 run's tables above. Not a run of the suites: speed-audit items 13 and 11
([design/speed-audit.md](design/speed-audit.md)), Mini, tail-call,
user+sys CPU, minimum of 11 interleaved runs (the 14 programs: 7, the
JIT suite: 5), against the commit before each change, with an A/A copy
of the baseline in the same interleave.

| Change | Benchmark | 64 | A/A | 128 |
|---|---|---:|---:|---:|
| SWITCH straight to its target, `real_word` inline, computed jumps enter compiled code | interp | 0.977 | 0.998 | 0.947 |
| same, `LOIMI_JIT=1` (21 runs) | interp | 0.998 | 0.991 | 1.002 |
| same, `LOIMI_JIT=1` | 14 programs | 0.997 | 0.995 | |
| CALL_RESUME, CALL_WRAP, CALL_YIELD members and supers | chameneos | 0.925 | 1.002 | 0.919 |
| same | coroutine_ring | 0.961 | 1.002 | 0.971 |
| same | coroutine sieve | 0.944 | 1.003 | 0.927 |
| same | 14 programs | 0.994 | 1.001 | 0.993 |

Against LuaJIT `-joff` in the same interleave: chameneos 0.91 (it was
1.0), coroutine_ring 0.89, sieve 0.88.

## Shifted-int nanbox (2026-11-06)

In the 2026-11-07 run's tables above. Not a run of the suites: speed-audit item 7
([design/speed-audit.md](design/speed-audit.md)), Python's `Value` on
`repr nanbox(int_high)` against master's plain nanbox, tail-call
builds, user+sys CPU, 11 interleaved runs, an A/A copy of the baseline
in the same interleave (geometric means, min / median of the runs).

| Change | Suite | 64 | A/A | 128 | A/A |
|---|---|---:|---:|---:|---:|
| `nanbox(int_high)` | Python (11) | 0.932 / 0.928 | 1.000 / 0.997 | 0.942 / 0.952 | 0.999 / 1.002 |

A second round gave 0.934 / 0.939 and 0.951 / 0.957. Integer
benchmarks 0.87–0.97 (fannkuch 0.87, go, hexiom and nqueens 0.91–0.93);
chaos and raytrace 0.91 (pointer payloads are a shift, with no 48-bit
mask constant); float 1.00; nbody 0.99 at 64 and 1.01 / 1.06 at 128;
spectral_norm 0.92 at 64 and 1.00 / 1.07 at 128 (its int ops got
cheaper and its float accumulation dearer).

## Shifted-int nanbox on the other vms (2026-11-06)

In the 2026-11-07 run's tables above. Not a run of the suites: speed-audit item 7, "Ports"
([design/speed-audit.md](design/speed-audit.md)). Each vm's value type
on `repr nanbox(int_high)` against master's build, tail-call builds,
user+sys CPU, minimum of 7 (Scheme, Prolog) or 9 interleaved runs, an
A/A copy of the baseline in the same interleave (geometric means).

| Change | Suite | 64 | A/A | 128 | A/A | Kept |
|---|---|---:|---:|---:|---:|---|
| `Term` on `int_high` | Prolog (30) | 0.917 | 1.003 | 0.920 | 1.008 | yes |
| fixnums `i48` on `int_high` | Scheme (43) | 0.979 | 0.994 | 0.978 | 0.998 | no: sumfp 1.27, mbrot 1.15 |
| `Int(i48)` on `int_high` | MiniJS (14) | 0.941 | 0.993 | 0.958 | 1.049 | no: nbody 1.25–1.47 |
| `Int(i48)` on `int_high` | Mini (14) | 1.034 | 1.003 | 1.030 | 1.001 | no: mandelbrot 1.80, nbody 1.32 |
| `Native` as the zero-tag variant | Lox (9) | 1.005 | 0.987 | 1.030 | 1.007 | no: all numbers are floats |

Integer benchmarks gained 5–25% in every vm; float kernels lost 12–80%,
as each float operand from a register costs a decode on the loop's
dependency chain. Per benchmark: `bench/prolog/README.md`,
`bench/scheme/README.md`, `bench/minijs/RESULTS.md`,
`bench/mini/RESULTS.md`.

## Value stacks; quickening flip-flop (2026-11-07)

In the 2026-11-07 run's tables above. Not a run of the suites: speed-audit items 9 and 10
([design/speed-audit.md](design/speed-audit.md)). Tail-call builds
against master's, user+sys CPU, minimum of 7 (Scheme, MiniJS) or 9
(Lox) interleaved runs, with an A/A copy of the baseline in the same
interleave (geometric means). The 128 column ran with an A/A copy only for MiniJS (1.004).

| Change | Suite | 64 | A/A | 128 | Kept |
|---|---|---:|---:|---:|---|
| No register clearing at calls (`std.rt.ValueStack`) | Lox (9) | 0.970 | 1.004 | 0.966 | yes: fib 0.86, binary_trees 0.92–0.94 |
| Hand-written mark → `std.rt.ValueStack` | Scheme (43) | 1.000 | 1.000 | 0.995 | yes |
| Hand-written mark → `std.rt.ValueStack` | MiniJS (14) | 0.995 | 0.998 | 1.002 | yes |

Item 10: `--vm-stats` now counts `become`s per site. The later ones
(re-quickenings and deopts) are at most 0.055% of dispatches (MiniJS
deltablue), and are 0 in Mini and Scheme. An oracle build in which
MiniJS's ADD never quickens bounds warmup and backoff counters at
0.982–0.986 on bitops and 0.994–0.996 on strings, so they were not
built.

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

Times in seconds.

### wasm-polybench

The 30 PolyBench/C 4.2.1 kernels, built like the `wasm` programs, each at
the largest standard dataset that runs in at most about 2 s under wasm3
(`bench/wasm/polybench.sh`); the interpreter was not tuned on them.
Geometric means of the times: wasm3 0.186 s, tail-call 0.153, goto
0.162, switch 0.195.

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
| **geomean** |  |  |  |

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
