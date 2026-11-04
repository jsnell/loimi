# Benchmark results

One run of every example interpreter against its reference
implementations, made with [`bench/run_all.py`](../bench/run_all.py).
This page replaces the ratios quoted elsewhere (README, `status.md`,
`roadmap.md`); the per-interpreter logs (`bench/*/RESULTS.md`,
`bench/scheme/README.md`, `bench/prolog/README.md`, the examples'
`FRICTION.md`) keep their dated history.

## Setup

| | |
|---|---|
| Date | 2026-11-04 |
| Commit | master 95a227e |
| CPU | Intel Core i7-6700 @ 3.40 GHz, 4 cores / 8 threads (2.7 GHz in `/proc/cpuinfo` during this run) |
| Kernel | Linux 6.12.113+deb13-amd64 |
| Compilers | clang 19.1.7 (Debian), rustc 1.97.1 |
| Other | Python 3.13.5, node v26.13.0, PCRE2 10.46 (system library) |

No other agent ran, but the host was loaded from outside this machine
image: the 1-minute load average never fell below 1.0 (each chunk waited
3 minutes for it, the first 10, then started; 2.96 at the lowest),
and it was 3.0–5.0 during the runs, median 4.3, our own benchmark
process included (the previous run's was 0.4–3.4). Every program,
references included, took about 1.3–1.4× the time of the previous run
(references 1.28–1.40× by suite geometric mean), so compare ratios, not
seconds, across runs. The suites ran as six `run_all.py --only`
invocations into one output directory (mini, mini-coro: 8 minutes;
minijs, lox: 11; scheme, regex: 8; wasm, wasm-simd, stack: 7; python: 4;
prolog: 17, over the 15 minutes aimed for, as its slow cases are run
once already), 55 minutes in all, then the PolyBench set (3 minutes).
Single benchmarks move by about ±3% between runs from code placement and
load, more on a loaded host.

**Method.**

- Every Loimi interpreter is built with `--release --align-functions=64`
  in each dispatch mode (`--dispatch=tailcall`, `goto`, `switch`).
  Mini, MiniJS, Scheme and Prolog are also built with `--jit-stencils`
  (tail-call dispatch) and run with `LOIMI_JIT=1` (column *jit*).
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
  milliseconds. Prolog's JIT build on `sieve` (58 s a run) ran once
  (`run.py --slow 10`).
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
| mini (14) | tailcall | 1 | lua 0.52, luajit-joff 0.82 |
| mini (14) | goto | 1.00 | lua 0.52, luajit-joff 0.83 |
| mini (14) | switch | 1.08 | lua 0.57, luajit-joff 0.89 |
| mini (14) | jit | 0.76 | lua 0.40, luajit-joff 0.62 |
| mini-coro (3) | tailcall | 1 | lua 0.37, luajit-joff 1.01 |
| mini-coro (3) | goto | 1.04 | lua 0.39, luajit-joff 1.05 |
| mini-coro (3) | switch | 1.09 | lua 0.40, luajit-joff 1.09 |
| mini-coro (3) | jit | 0.84 | lua 0.31, luajit-joff 0.85 |
| minijs (14) | tailcall | 1 | node-jitless 0.50, qjs 0.39 |
| minijs (14) | goto | 1.02 | node-jitless 0.51, qjs 0.40 |
| minijs (14) | switch | 1.12 | node-jitless 0.56, qjs 0.43 |
| minijs (14) | jit | 0.85 | node-jitless 0.43, qjs 0.33 |
| scheme (43) | tailcall | 1 | gauche 0.25, guile-nojit 0.45, guile 0.94 |
| scheme (43) | goto | 1.03 | gauche 0.25, guile-nojit 0.46, guile 0.96 |
| scheme (43) | switch | 1.07 | gauche 0.26, guile-nojit 0.48, guile 1.01 |
| scheme (43) | jit | 0.82 | gauche 0.20, guile-nojit 0.36, guile 0.76 |
| lox (10) | tailcall | 1 | clox 0.44 |
| lox (10) | goto | 1.08 | clox 0.48 |
| lox (10) | switch | 1.03 | clox 0.46 |
| regex (9) | tailcall | 1 | pcre2 0.44, python-re 0.41 |
| regex (9) | goto | 1.01 | pcre2 0.45, python-re 0.42 |
| regex (9) | switch | 1.07 | pcre2 0.47, python-re 0.44 |
| wasm (7) | tailcall | 1 | wasm3 0.87 |
| wasm (7) | goto | 1.05 | wasm3 0.91 |
| wasm (7) | switch | 1.27 | wasm3 1.10 |
| wasm-simd (3) | tailcall | 1 | mvp 0.98 |
| wasm-simd (3) | goto | 1.03 | mvp 1.01 |
| wasm-simd (3) | switch | 1.30 | mvp 1.28 |
| wasm-polybench (30) | tailcall | 1 | wasm3 0.90 |
| wasm-polybench (30) | goto | 1.02 | wasm3 0.92 |
| wasm-polybench (30) | switch | 1.23 | wasm3 1.11 |
| stack (5) | tailcall | 1 | — |
| stack (5) | goto | 1.03 | — |
| stack (5) | switch | 1.18 | — |
| python (11) | tailcall | 1 | cpython 0.37, micropython 0.13 |
| python (11) | goto | 1.05 | cpython 0.39, micropython 0.14 |
| python (11) | switch | 1.18 | cpython 0.44, micropython 0.15 |
| prolog (30) | tailcall | 1 | swipl 0.53, trealla 0.13 |
| prolog (30) | goto | 1.04 | swipl 0.55, trealla 0.13 |
| prolog (30) | switch | 1.10 | swipl 0.58, trealla 0.14 |
| prolog (30) | jit | 1.26 (1.11 without sieve) | swipl 0.66, trealla 0.16 (swipl 0.58, trealla 0.14 over 29, without sieve) |

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

## Notes

- Mini (tail-call) takes 0.25–0.76 of PUC-Lua's time and 0.49–1.04 of
  LuaJIT `-joff`'s: less on 13 of 14 (hash_strings 1.04; string_build,
  1.09 in the previous run, 0.94). With the JIT: 0.56–0.99 of the
  interpreter's time.
- Mini's coroutine benchmarks switch threads inside the dispatch loop
  (`examples/mini/COROUTINES.md`): 0.34–0.42 of PUC-Lua's time and
  0.95–1.11 of LuaJIT `-joff`'s (chameneos 1.11); goto and switch take
  1.02–1.10× tail-call.
- MiniJS takes 0.22–0.78 of `node --jitless`'s time and 0.20–0.65 of
  QuickJS's, less on all 14.
  **2026-11-04, second MiniJS pass** (after this run; tail-call only,
  `compare.py -n 3 --align 64` with the 573511e build as a column):
  0.939 of the time before by geometric mean, 0.48 of node --jitless
  and 0.37 of QuickJS (0.51 and 0.39 for the old build in the same
  run); bitops 0.83, raytrace 0.90, closures and spectral_norm 0.91,
  strings 0.92, binary_trees 0.93, others 0.94-0.99. Arithmetic slow
  paths inline, a DIV family, constant compares, frameless handlers,
  calls that do not clear registers
  ([bench/minijs/RESULTS.md](../bench/minijs/RESULTS.md#second-performance-pass-2026-11-04)).
  Other dispatch modes and the JIT build were not rerun.
- Scheme takes less time than Guile with its JIT on 14 of 43 programs
  (geometric mean 0.94); with Loimi's JIT, on 21 (0.76). The bignum
  programs: pi 1.30× Guile (GMP's subquadratic division and square root
  against std.bigint's), chudnovsky 1.10×.
- Lox takes 0.25–0.66 of clox's time. Switch mode is within 3% of
  tail-call except `equality` (1.17×); goto mode took 1.08× tail-call by
  geometric mean in this run (1.03 in the previous one; invocation, zoo
  and method_call 1.12–1.14).
- Regex: the backtracker takes less time than the PCRE2 interpreter and
  Python's `re` except on `literal` (1.26 and 1.29) and, against Python,
  `dot-star` (1.25). `pathological` is rejected by the required-byte check
  before matching (0.0 ms; Python 551 ms) and is left out of the
  geometric means.
- Wasm takes 0.75–0.94 of wasm3's time on the seven programs, less on
  all, and 0.75–1.01 on the 30 PolyBench kernels (0.90 by geometric mean;
  more only on gesummv, 1.01). Switch builds take 1.27× the tail-call
  time (1.23× on PolyBench). The SIMD build of nbody takes 0.95 of its
  MVP build's time; fannkuchredux and spectralnorm, whose vectorized code
  is a few instructions outside the hot loops, 0.98–1.01.
- Python (tail-call) takes 0.22–0.67 of CPython's time, less on all 11
  (nqueens 0.67, hexiom 0.54; richards 0.22, float 0.26). MicroPython's
  output differs on chaos and go (its `random` is another generator) and
  nbody (last float digits); their times are in the table and in the
  geometric means (`compare.py` leaves them out of its own). Switch takes
  1.07–1.29× the tail-call time.
- Prolog takes 0.20–0.93 of SWI-Prolog's time, less on all 30 (tak 0.20;
  sieve, whose assert/retract compile clauses, 0.93), and 0.05–0.36 of
  Trealla's. **The JIT build takes 1.11× the interpreter's time** over
  the 29 programs other than sieve (1.01× in the previous run): 0.92
  unify to 1.43 meta_qsort (times10 1.35, divide10 1.31, poly_10 1.29;
  meta_qsort and times10 confirmed in a rerun, best of 5: 1.43 and
  1.31). Normalized to SWI-Prolog the JIT build still took 0.75 of its
  previous time, but the interpreter took 0.67: the second and third
  Prolog rounds (inline head unification, guards, first-argument
  indexing in CALL, superinstructions) helped the interpreter more than
  the stencil build. Not investigated further here. On sieve the JIT
  build takes 48× the interpreter's time: every assert/retract makes code
  objects that are never freed and the JIT compiles each as it gets hot
  (`examples/prolog/FRICTION.md`, item 6): that cell is marked † and the
  summary gives the geometric means with and without it.
  *Note, 2026-11-04 (after this run):* both were the JIT's. Clauses
  entered from try chains (`jump` through a `Wam.Resume`) never counted
  toward compiling, so most clause code stayed interpreted, and the
  registry of code objects was a sorted array kept by `memmove`, which
  sieve's 700,000 asserted clauses made quadratic. With both fixed the
  JIT build takes 0.98 of the interpreter's time over the 29 (1.04 for
  the old build in the same A/B, CPU time, best of 3; unify and tak
  0.83, mu and prover 1.16) and 1.30 on sieve (46 for the old build).
  Not rerun here; see
  [design/copy-and-patch.md §14](design/copy-and-patch.md#14-prolog-2026-11-04).

## GC: mark bitmaps instead of sweeping (2026-11-04)

A dated A/B of one change, made before this run; its effect is in the
tables ("Changes since the previous run"). `std.gc`
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

## Changes since the previous run (a663b98)

The previous tables (commit a663b98, 2026-11-03, same machine and
method) against these, over the benchmarks both runs have. The host ran
this run's programs about 1.3–1.4× slower (*references' time*, new/old
geometric mean of every reference of the suite), so the mode columns
divide that out: each is the geometric mean over benchmarks of (ours /
first reference) new over old, below 1 when ours gained on the
reference. *ratio* is the tail-call (or JIT) build over the first
reference, old → new.

| suite | references' time | tailcall | goto | switch | jit | ratio old → new |
|---|---:|---:|---:|---:|---:|---|
| mini (14) | 1.38–1.39 | 0.96 | 0.94 | 0.97 | 0.97 | lua 0.54 → 0.52, jit 0.41 → 0.40 |
| mini-coro (3) | 1.34 | 0.91 | 0.94 | 1.02 | 0.97 | lua 0.41 → 0.37, jit 0.32 → 0.31 |
| minijs (14) | 1.35–1.36 | 0.97 | 0.97 | 1.00 | 0.96 | node-jitless 0.52 → 0.50, jit 0.44 → 0.43 |
| scheme (43) | 1.34–1.40 | 0.97 | 0.96 | 0.96 | 0.96 | guile 0.97 → 0.94, jit 0.80 → 0.76 |
| lox (10) | 1.35 | 0.98 | 1.04 | 0.97 | | clox 0.45 → 0.44 |
| regex (8) | 1.30 | 1.00 | 1.00 | 0.97 | | pcre2 0.44 → 0.44 |
| wasm (7) | 1.32 | 0.95 | 0.95 | 0.95 | | wasm3 0.91 → 0.87 |
| wasm-simd (3) | (mvp) | 0.99 | 0.99 | 1.01 | | mvp 0.99 → 0.98 |
| stack (5) | | 1.22 | 1.26 | 1.21 | | (own time new/old; no reference) |
| python (11) | 1.28–1.29 | 0.72 | 0.73 | 0.73 | | cpython 0.52 → 0.37 |
| prolog (30) | 1.36–1.38 | 0.67 | 0.69 | 0.68 | 0.75 (29) | swipl 0.79 → 0.53, jit (29) 0.77 → 0.58 |

New in this run: the PolyBench held-out set (0.90 of wasm3; 0.893 in
the wasm round-two measurement, `examples/wasm/FRICTION.md`). No suite
lost more than 4% against its references by geometric mean (Lox's goto
build, 1.04) and no single benchmark more than 25% (the largest: MiniJS
switch-mode awfy_micro 1.23, Lox goto-mode invocation 1.16, Mini JIT
sieve 1.15; the Prolog JIT build's meta_qsort 1.11, see the notes). What
changed, from `git log a663b98..95a227e`:

- **Prolog, second and third optimization rounds** (6187ede, 95a227e;
  `examples/prolog/FRICTION.md`, "Optimizations, second round" and
  "third round"): binding and trailing inline in the head ops, nested
  head structures built in place, CALL/EXECUTE indexing the first
  argument, guard chains, superinstructions from `--vm-stats`, inline
  arg/3, functor/3 and compare/3, first-clause commitment on a
  comparison, a 2 MiB minimum heap. 0.67 of the previous time relative
  to SWI-Prolog (tak 0.32, queens_8 0.40, query 0.42, sendmore 0.45;
  the least, qsort 0.86); the JIT build 0.75 (notes).
- **Python, second round and instance layout** (418a644, e578712;
  `examples/python/FRICTION.md`): a global method cache, operator-method
  cache, list and math fast paths, FORITER2, a POW fast path; instance
  attributes in inline slots behind per-class shapes, LOADMETHOD caching
  the method. 0.72 relative to CPython (float 0.51, richards 0.59,
  deltablue 0.64, spectral_norm 0.66; nbody 1.02).
- **std.gc without sweeping** (41db7df, then prefetching marks and
  fragmented chunks after the pool, ecb9073; the section above): the
  GC-heavy programs gained most: Mini binary_trees 0.70 and
  string_build 0.78 relative to Lua, Scheme ctak and diviter 0.84,
  deriv 0.92, MiniJS strings 0.90, Lox binary_trees 0.83; part of
  Prolog's and Python's gains too (boyer 0.47, sieve 0.47).
- **Wasm translator, rounds one and two** (76772e3, 6187ede:
  `examples/wasm/FRICTION.md`): zeroing only locals read before
  written, K-form loads and stores, compare-and-branch fusions, min/max
  selects, constant slots for operands without a K form, `local.tee`'d
  addresses: 0.95 relative to wasm3 in every mode.
- **Mini coroutines**: the 4–6% lost at a0efc73 (the previous run's
  "Regressions" section: the inline barrier slow path made `coro.enter`
  and `coro.suspend` too large to inline) is fixed by logging the thread
  switch's stores with an out-of-line `gc.remember` (e5a9b29):
  tail-call 0.91 relative to Lua, goto 0.94; switch 1.02.
- **No speed change intended**: lifetimes checking on by default
  (80439a2; compile-time only, the examples' IR unchanged apart from 7
  `unsafe { }` blocks), `#![safe]` modules and `#[raw]` (std.lex,
  std.parse, std.diag, std.text, std.hash, std.io, std.bc, std.map,
  std.dict), write barriers for stores through trailing-array slices
  (5b44eb0), the vm verifier's bitmap fix (9bc3ad8), the Mini and Python
  debuggers (`--cfg debug` only). Lox, regex, Stak and the switch-mode
  columns moved within ±4% relative to their references.

## Per suite

### mini

| benchmark | tailcall | goto | switch | jit | lua | luajit-joff | tailcall/lua | tailcall/luajit-joff | jit/lua | jit/luajit-joff |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| binary_trees | 1.034 | 1.085 | 1.209 | 0.871 | 4.204 | 1.927 | 0.25 | 0.54 | 0.21 | 0.45 |
| closures | 0.596 | 0.668 | 0.683 | 0.497 | 1.596 | 1.226 | 0.37 | 0.49 | 0.31 | 0.41 |
| fannkuch | 1.593 | 1.358 | 1.619 | 1.192 | 2.185 | 1.798 | 0.73 | 0.89 | 0.55 | 0.66 |
| fib | 1.243 | 1.334 | 1.555 | 0.852 | 2.034 | 1.435 | 0.61 | 0.87 | 0.42 | 0.59 |
| hash_strings | 1.409 | 1.338 | 1.456 | 1.038 | 2.081 | 1.352 | 0.68 | 1.04 | 0.50 | 0.77 |
| interp | 1.228 | 1.201 | 1.324 | 0.919 | 2.615 | 1.417 | 0.47 | 0.87 | 0.35 | 0.65 |
| mandelbrot | 0.811 | 0.808 | 0.832 | 0.457 | 1.518 | 0.995 | 0.53 | 0.82 | 0.30 | 0.46 |
| nbody | 0.595 | 0.611 | 0.657 | 0.454 | 1.610 | 0.672 | 0.37 | 0.89 | 0.28 | 0.68 |
| oo_methods | 0.929 | 1.045 | 1.082 | 0.660 | 1.620 | 1.217 | 0.57 | 0.76 | 0.41 | 0.54 |
| queens | 1.213 | 1.193 | 1.280 | 0.891 | 1.590 | 1.240 | 0.76 | 0.98 | 0.56 | 0.72 |
| sieve | 1.073 | 1.020 | 1.111 | 0.970 | 1.794 | 1.309 | 0.60 | 0.82 | 0.54 | 0.74 |
| sort | 1.148 | 1.072 | 1.135 | 0.835 | 1.563 | 1.195 | 0.73 | 0.96 | 0.53 | 0.70 |
| spectral_norm | 1.400 | 1.404 | 1.455 | 0.959 | 2.256 | 1.527 | 0.62 | 0.92 | 0.43 | 0.63 |
| string_build | 1.171 | 1.204 | 1.233 | 1.157 | 3.096 | 1.243 | 0.38 | 0.94 | 0.37 | 0.93 |
| **geomean** |  |  |  |  |  |  | **0.52** | **0.82** | **0.40** | **0.62** |

Times in seconds.

### mini-coro

| benchmark | tailcall | goto | switch | jit | lua | luajit-joff | tailcall/lua | tailcall/luajit-joff | jit/lua | jit/luajit-joff |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| chameneos | 0.546 | 0.582 | 0.595 | 0.471 | 1.288 | 0.490 | 0.42 | 1.11 | 0.37 | 0.96 |
| coroutine_ring | 0.457 | 0.472 | 0.502 | 0.362 | 1.272 | 0.481 | 0.36 | 0.95 | 0.28 | 0.75 |
| sieve | 0.492 | 0.504 | 0.526 | 0.430 | 1.454 | 0.509 | 0.34 | 0.97 | 0.30 | 0.84 |
| **geomean** |  |  |  |  |  |  | **0.37** | **1.01** | **0.31** | **0.85** |

Times in seconds.

### minijs

| benchmark | tailcall | goto | switch | jit | node-jitless | qjs | tailcall/node-jitless | tailcall/qjs | jit/node-jitless | jit/qjs |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| awfy_micro | 1.003 | 1.003 | 1.366 | 0.783 | 2.061 | 2.626 | 0.49 | 0.38 | 0.38 | 0.30 |
| binary_trees | 1.322 | 1.420 | 1.498 | 1.158 | 2.320 | 6.617 | 0.57 | 0.20 | 0.50 | 0.18 |
| bitops | 1.070 | 1.054 | 1.139 | 0.912 | 2.123 | 1.890 | 0.50 | 0.57 | 0.43 | 0.48 |
| closures | 1.279 | 1.404 | 1.530 | 0.961 | 1.965 | 3.356 | 0.65 | 0.38 | 0.49 | 0.29 |
| deltablue | 1.138 | 1.233 | 1.327 | 0.968 | 1.982 | 3.499 | 0.57 | 0.33 | 0.49 | 0.28 |
| fannkuch | 0.606 | 0.589 | 0.662 | 0.484 | 1.663 | 1.912 | 0.36 | 0.32 | 0.29 | 0.25 |
| json | 1.230 | 1.248 | 1.309 | 1.197 | 2.024 | 2.982 | 0.61 | 0.41 | 0.59 | 0.40 |
| nbody | 0.552 | 0.551 | 0.585 | 0.424 | 2.479 | 1.702 | 0.22 | 0.32 | 0.17 | 0.25 |
| properties | 1.109 | 1.130 | 1.180 | 0.947 | 2.171 | 2.198 | 0.51 | 0.50 | 0.44 | 0.43 |
| raytrace | 0.658 | 0.667 | 0.725 | 0.606 | 1.952 | 2.937 | 0.34 | 0.22 | 0.31 | 0.21 |
| richards | 0.979 | 1.064 | 1.143 | 0.908 | 1.923 | 2.632 | 0.51 | 0.37 | 0.47 | 0.34 |
| sort | 1.190 | 1.188 | 1.309 | 1.007 | 1.857 | 2.408 | 0.64 | 0.49 | 0.54 | 0.42 |
| spectral_norm | 1.194 | 1.209 | 1.275 | 0.986 | 2.156 | 1.827 | 0.55 | 0.65 | 0.46 | 0.54 |
| strings | 1.741 | 1.681 | 1.835 | 1.620 | 2.241 | 2.999 | 0.78 | 0.58 | 0.72 | 0.54 |
| **geomean** |  |  |  |  |  |  | **0.50** | **0.39** | **0.43** | **0.33** |

Times in seconds.

### scheme

| benchmark | tailcall | goto | switch | jit | gauche | guile-nojit | guile | tailcall/gauche | tailcall/guile-nojit | tailcall/guile | jit/gauche | jit/guile-nojit | jit/guile |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| ack | 0.598 | 0.659 | 0.674 | 0.466 | 4.141 | 1.207 | 0.287 | 0.14 | 0.50 | 2.08 | 0.11 | 0.39 | 1.62 |
| array1 | 0.455 | 0.487 | 0.485 | 0.326 | 1.155 | 1.315 | 0.369 | 0.39 | 0.35 | 1.23 | 0.28 | 0.25 | 0.88 |
| browse | 0.022 | 0.022 | 0.023 | 0.022 | 0.117 | 0.038 | 0.026 | 0.19 | 0.58 | 0.85 | 0.19 | 0.58 | 0.85 |
| chudnovsky | 0.215 | 0.213 | 0.218 | 0.223 | 1.353 | 0.249 | 0.195 | 0.16 | 0.86 | 1.10 | 0.16 | 0.90 | 1.14 |
| conform | 0.156 | 0.159 | 0.171 | 0.136 | 0.611 | 0.236 | 0.103 | 0.26 | 0.66 | 1.51 | 0.22 | 0.58 | 1.32 |
| cpstak | 0.345 | 0.350 | 0.359 | 0.258 | 1.767 | 0.568 | 0.272 | 0.20 | 0.61 | 1.27 | 0.15 | 0.45 | 0.95 |
| ctak | 0.033 | 0.032 | 0.032 | 0.030 | 0.381 | 0.715 | 0.684 | 0.09 | 0.05 | 0.05 | 0.08 | 0.04 | 0.04 |
| deriv | 0.292 | 0.303 | 0.313 | 0.277 | 1.039 | 0.529 | 0.252 | 0.28 | 0.55 | 1.16 | 0.27 | 0.52 | 1.10 |
| destruc | 0.167 | 0.168 | 0.172 | 0.128 | 0.638 | 0.298 | 0.121 | 0.26 | 0.56 | 1.38 | 0.20 | 0.43 | 1.06 |
| diviter | 0.196 | 0.194 | 0.206 | 0.161 | 0.673 | 0.464 | 0.226 | 0.29 | 0.42 | 0.87 | 0.24 | 0.35 | 0.71 |
| divrec | 0.267 | 0.279 | 0.294 | 0.238 | 0.885 | 0.584 | 0.294 | 0.30 | 0.46 | 0.91 | 0.27 | 0.41 | 0.81 |
| earley | 0.056 | 0.056 | 0.058 | 0.052 | 0.198 | 0.085 | 0.043 | 0.28 | 0.66 | 1.30 | 0.26 | 0.61 | 1.21 |
| fft | 0.073 | 0.072 | 0.074 | 0.053 | 0.436 | 0.258 | 0.221 | 0.17 | 0.28 | 0.33 | 0.12 | 0.21 | 0.24 |
| fib | 0.624 | 0.670 | 0.772 | 0.543 | 1.844 | 1.377 | 0.360 | 0.34 | 0.45 | 1.73 | 0.29 | 0.39 | 1.51 |
| fibc | 0.047 | 0.045 | 0.045 | 0.039 | 0.524 | 0.847 | 0.799 | 0.09 | 0.06 | 0.06 | 0.07 | 0.05 | 0.05 |
| fibfp | 0.238 | 0.254 | 0.276 | 0.177 | 0.682 | 1.084 | 0.876 | 0.35 | 0.22 | 0.27 | 0.26 | 0.16 | 0.20 |
| gcbench | 0.027 | 0.026 | 0.027 | 0.022 | 0.190 | 0.038 | 0.023 | 0.14 | 0.71 | 1.17 | 0.12 | 0.58 | 0.96 |
| graphs | 0.165 | 0.172 | 0.176 | 0.152 | 0.866 | 0.276 | 0.094 | 0.19 | 0.60 | 1.76 | 0.18 | 0.55 | 1.62 |
| lattice | 0.071 | 0.073 | 0.079 | 0.067 | 0.295 | 0.134 | 0.057 | 0.24 | 0.53 | 1.25 | 0.23 | 0.50 | 1.18 |
| matrix | 0.092 | 0.096 | 0.098 | 0.081 | 0.379 | 0.153 | 0.069 | 0.24 | 0.60 | 1.33 | 0.21 | 0.53 | 1.17 |
| maze | 0.049 | 0.051 | 0.057 | 0.041 | 0.270 | 0.097 | 0.051 | 0.18 | 0.51 | 0.96 | 0.15 | 0.42 | 0.80 |
| mazefun | 0.177 | 0.189 | 0.194 | 0.142 | 0.592 | 0.379 | 0.125 | 0.30 | 0.47 | 1.42 | 0.24 | 0.37 | 1.14 |
| mbrot | 0.067 | 0.066 | 0.071 | 0.049 | 0.276 | 0.399 | 0.339 | 0.24 | 0.17 | 0.20 | 0.18 | 0.12 | 0.14 |
| mperm | 0.185 | 0.193 | 0.201 | 0.141 | 0.541 | 0.252 | 0.080 | 0.34 | 0.73 | 2.31 | 0.26 | 0.56 | 1.76 |
| nboyer | 0.901 | 0.961 | 1.005 | 0.754 | 1.618 | 1.185 | 0.450 | 0.56 | 0.76 | 2.00 | 0.47 | 0.64 | 1.68 |
| nqueens | 0.408 | 0.402 | 0.437 | 0.320 | 0.913 | 0.653 | 0.232 | 0.45 | 0.62 | 1.76 | 0.35 | 0.49 | 1.38 |
| ntakl | 0.222 | 0.245 | 0.245 | 0.146 | 0.676 | 0.404 | 0.098 | 0.33 | 0.55 | 2.27 | 0.22 | 0.36 | 1.49 |
| nucleic | 0.092 | 0.090 | 0.095 | 0.088 | 0.544 | 0.262 | 0.196 | 0.17 | 0.35 | 0.47 | 0.16 | 0.34 | 0.45 |
| paraffins | 0.052 | 0.052 | 0.054 | 0.048 | 0.201 | 0.071 | 0.042 | 0.26 | 0.73 | 1.24 | 0.24 | 0.68 | 1.14 |
| peval | 0.130 | 0.129 | 0.143 | 0.098 | 0.549 | 0.265 | 0.104 | 0.24 | 0.49 | 1.25 | 0.18 | 0.37 | 0.94 |
| pi | 0.181 | 0.180 | 0.181 | 0.193 | 3.940 | 0.142 | 0.139 | 0.05 | 1.27 | 1.30 | 0.05 | 1.36 | 1.39 |
| pnpoly | 0.103 | 0.104 | 0.107 | 0.083 | 0.298 | 0.468 | 0.298 | 0.35 | 0.22 | 0.35 | 0.28 | 0.18 | 0.28 |
| primes | 0.180 | 0.195 | 0.204 | 0.151 | 0.783 | 0.419 | 0.275 | 0.23 | 0.43 | 0.65 | 0.19 | 0.36 | 0.55 |
| puzzle | 0.122 | 0.133 | 0.142 | 0.094 | 0.382 | 0.312 | 0.116 | 0.32 | 0.39 | 1.05 | 0.25 | 0.30 | 0.81 |
| quicksort | 0.141 | 0.145 | 0.149 | 0.110 | 0.403 | 0.255 | 0.100 | 0.35 | 0.55 | 1.41 | 0.27 | 0.43 | 1.10 |
| sboyer | 0.902 | 0.980 | 1.050 | 0.724 | 1.679 | 1.292 | 0.409 | 0.54 | 0.70 | 2.21 | 0.43 | 0.56 | 1.77 |
| simplex | 0.157 | 0.162 | 0.170 | 0.127 | 0.738 | 0.322 | 0.151 | 0.21 | 0.49 | 1.04 | 0.17 | 0.39 | 0.84 |
| string | 0.028 | 0.028 | 0.028 | 0.028 | 0.189 | 0.036 | 0.037 | 0.15 | 0.78 | 0.76 | 0.15 | 0.78 | 0.76 |
| sum | 0.432 | 0.481 | 0.474 | 0.247 | 1.079 | 0.816 | 0.217 | 0.40 | 0.53 | 1.99 | 0.23 | 0.30 | 1.14 |
| sumfp | 0.136 | 0.142 | 0.146 | 0.079 | 0.365 | 0.740 | 0.709 | 0.37 | 0.18 | 0.19 | 0.22 | 0.11 | 0.11 |
| tak | 0.636 | 0.616 | 0.672 | 0.463 | 1.599 | 1.225 | 0.301 | 0.40 | 0.52 | 2.11 | 0.29 | 0.38 | 1.54 |
| takl | 0.108 | 0.106 | 0.119 | 0.077 | 0.408 | 0.207 | 0.057 | 0.26 | 0.52 | 1.89 | 0.19 | 0.37 | 1.35 |
| triangl | 0.211 | 0.229 | 0.250 | 0.151 | 0.543 | 0.547 | 0.168 | 0.39 | 0.39 | 1.26 | 0.28 | 0.28 | 0.90 |
| **geomean** |  |  |  |  |  |  |  | **0.25** | **0.45** | **0.94** | **0.20** | **0.36** | **0.76** |

Times in seconds.

**Update 2026-11-04 (096e04e, tailcall only).** After a third round of
interpreter work (builtins as ops, allocation without a stack frame, no
register clearing on calls, aliases, loop-invariant free variables;
[bench/scheme/README.md](../bench/scheme/README.md#third-phase-calls-allocation-builtins-2026-11-04)),
`compare.py -n 3 --align 64` against the commit of this table (091245f)
in one interleaved run: ours/before 0.894; Gauche 0.251 → 0.224, Guile
without JIT 0.446 → 0.398, Guile with JIT 0.945 → 0.845 (host load 2-4,
so the absolute times are lower than above). goto, switch and jit
builds were not rerun.

### lox

| benchmark | tailcall | goto | switch | clox | tailcall/clox |
|---|---:|---:|---:|---:|---:|
| binary_trees | 0.912 | 0.977 | 0.940 | 3.609 | 0.25 |
| equality | 1.351 | 1.334 | 1.576 | 4.107 | 0.33 |
| fib | 0.509 | 0.557 | 0.512 | 1.255 | 0.41 |
| instantiation | 0.452 | 0.454 | 0.445 | 0.686 | 0.66 |
| invocation | 0.183 | 0.208 | 0.188 | 0.350 | 0.52 |
| method_call | 0.105 | 0.118 | 0.106 | 0.228 | 0.46 |
| properties | 0.238 | 0.258 | 0.243 | 0.535 | 0.44 |
| trees | 1.686 | 1.821 | 1.741 | 3.084 | 0.55 |
| zoo | 0.192 | 0.218 | 0.195 | 0.394 | 0.49 |
| zoo_batch | 1.172 | 1.308 | 1.185 | 2.468 | 0.47 |
| **geomean** |  |  |  |  | **0.44** |

Times in seconds.

### regex

| benchmark | tailcall | goto | switch | pcre2 | python-re | tailcall/pcre2 | tailcall/python-re |
|---|---:|---:|---:|---:|---:|---:|---:|
| alt-heavy | 30.2 | 29.8 | 30.8 | 63.5 | 41.0 | 0.48 | 0.74 |
| capture | 22.0 | 23.1 | 24.7 | 64.2 | 141.6 | 0.34 | 0.16 |
| class | 80.6 | 77.0 | 79.5 | 281.1 | 195.6 | 0.29 | 0.41 |
| dot-star | 12.9 | 12.3 | 12.9 | 27.2 | 10.3 | 0.47 | 1.25 |
| literal | 4.0 | 4.0 | 3.9 | 3.2 | 3.1 | 1.26 | 1.29 |
| literal-alt | 11.2 | 11.2 | 11.2 | 23.0 | 47.5 | 0.49 | 0.24 |
| patho-alt | 56.5 | 60.1 | 78.3 | 196.0 | 155.8 | 0.29 | 0.36 |
| pathological | 0.0 | 0.0 | 0.0 | 0.0 | 550.7 | — | — |
| words | 25.1 | 27.6 | 28.4 | 69.1 | 198.3 | 0.36 | 0.13 |
| **geomean** |  |  |  |  |  | **0.44** | **0.41** |

Times in milliseconds.

### wasm

| benchmark | tailcall | goto | switch | wasm3 | tailcall/wasm3 |
|---|---:|---:|---:|---:|---:|
| binarytrees | 1.725 | 1.872 | 2.072 | 1.865 | 0.92 |
| coremark | 2.371 | 2.496 | 2.967 | 2.629 | 0.90 |
| fannkuchredux | 1.780 | 1.857 | 2.219 | 2.101 | 0.85 |
| fasta | 1.409 | 1.515 | 1.711 | 1.496 | 0.94 |
| mandelbrot | 1.924 | 2.056 | 2.679 | 2.566 | 0.75 |
| nbody | 2.300 | 2.393 | 2.785 | 2.519 | 0.91 |
| spectralnorm | 2.081 | 2.111 | 2.848 | 2.576 | 0.81 |
| **geomean** |  |  |  |  | **0.87** |

Times in seconds.

### wasm-simd

| benchmark | tailcall | goto | switch | mvp | tailcall/mvp |
|---|---:|---:|---:|---:|---:|
| fannkuchredux_simd | 1.748 | 1.849 | 2.247 | 1.780 | 0.98 |
| nbody_simd | 2.196 | 2.247 | 2.771 | 2.300 | 0.95 |
| spectralnorm_simd | 2.103 | 2.123 | 2.877 | 2.081 | 1.01 |
| **geomean** |  |  |  |  | **0.98** |

Times in seconds.

### wasm-polybench

The 30 PolyBench/C 4.2.1 kernels, built like the `wasm` programs, each at
the largest standard dataset that runs in at most about 2 s under wasm3
(`bench/wasm/polybench.sh`); the interpreter was not tuned on them.
Geometric means of the times: wasm3 0.252 s, tail-call 0.226, goto
0.231, switch 0.279.

| benchmark | tailcall | goto | switch | wasm3 | tailcall/wasm3 |
|---|---:|---:|---:|---:|---:|
| 2mm | 0.154 | 0.153 | 0.189 | 0.165 | 0.93 |
| 3mm | 0.240 | 0.234 | 0.294 | 0.255 | 0.94 |
| adi | 0.277 | 0.302 | 0.346 | 0.323 | 0.86 |
| atax | 0.127 | 0.130 | 0.150 | 0.135 | 0.94 |
| bicg | 0.130 | 0.133 | 0.159 | 0.131 | 0.99 |
| cholesky | 0.859 | 0.868 | 1.019 | 0.963 | 0.89 |
| correlation | 0.076 | 0.081 | 0.094 | 0.082 | 0.93 |
| covariance | 0.079 | 0.080 | 0.093 | 0.082 | 0.96 |
| deriche | 1.095 | 1.150 | 1.296 | 1.454 | 0.75 |
| doitgen | 0.068 | 0.070 | 0.087 | 0.072 | 0.94 |
| durbin | 0.176 | 0.171 | 0.221 | 0.193 | 0.91 |
| fdtd-2d | 0.206 | 0.217 | 0.266 | 0.246 | 0.84 |
| floyd-warshall | 1.791 | 1.817 | 2.094 | 1.919 | 0.93 |
| gemm | 0.127 | 0.127 | 0.157 | 0.129 | 0.98 |
| gemver | 0.892 | 0.897 | 1.135 | 1.036 | 0.86 |
| gesummv | 0.346 | 0.349 | 0.417 | 0.341 | 1.01 |
| gramschmidt | 0.134 | 0.137 | 0.161 | 0.140 | 0.96 |
| heat-3d | 0.300 | 0.326 | 0.412 | 0.372 | 0.81 |
| jacobi-1d | 0.093 | 0.097 | 0.118 | 0.109 | 0.85 |
| jacobi-2d | 0.211 | 0.221 | 0.264 | 0.258 | 0.82 |
| lu | 0.961 | 1.006 | 1.169 | 1.102 | 0.87 |
| ludcmp | 0.900 | 0.941 | 1.116 | 1.083 | 0.83 |
| mvt | 0.556 | 0.564 | 0.686 | 0.628 | 0.89 |
| nussinov | 0.246 | 0.246 | 0.305 | 0.321 | 0.77 |
| seidel-2d | 0.379 | 0.418 | 0.484 | 0.447 | 0.85 |
| symm | 0.098 | 0.101 | 0.122 | 0.112 | 0.88 |
| syr2k | 0.131 | 0.135 | 0.174 | 0.136 | 0.96 |
| syrk | 0.071 | 0.069 | 0.087 | 0.078 | 0.91 |
| trisolv | 0.142 | 0.136 | 0.164 | 0.156 | 0.91 |
| trmm | 0.054 | 0.056 | 0.065 | 0.057 | 0.95 |
| **geomean** |  |  |  |  | **0.90** |

Times in seconds.

### stack

| benchmark | tailcall | goto | switch |
|---|---:|---:|---:|
| fib | 0.067 | 0.075 | 0.091 |
| loop | 0.932 | 0.910 | 0.984 |
| nbody | 0.191 | 0.204 | 0.271 |
| sieve | 0.118 | 0.124 | 0.134 |
| strings | 0.163 | 0.156 | 0.163 |
| **geomean** |  |  |  |

Times in seconds.

### python

| benchmark | tailcall | goto | switch | cpython | micropython | tailcall/cpython | tailcall/micropython |
|---|---:|---:|---:|---:|---:|---:|---:|
| chaos | 0.321 | 0.357 | 0.374 | 0.877 | 2.253 | 0.37 | 0.14 |
| deltablue | 0.465 | 0.509 | 0.566 | 1.234 | 5.818 | 0.38 | 0.08 |
| fannkuch | 0.426 | 0.447 | 0.480 | 1.109 | 1.717 | 0.38 | 0.25 |
| float | 0.287 | 0.288 | 0.347 | 1.098 | 2.652 | 0.26 | 0.11 |
| go | 0.408 | 0.442 | 0.483 | 1.056 | 2.433 | 0.39 | 0.17 |
| hexiom | 0.496 | 0.516 | 0.621 | 0.922 | 3.939 | 0.54 | 0.13 |
| nbody | 0.471 | 0.481 | 0.606 | 1.235 | 4.482 | 0.38 | 0.11 |
| nqueens | 0.702 | 0.738 | 0.822 | 1.055 | 2.186 | 0.67 | 0.32 |
| raytrace | 0.440 | 0.447 | 0.471 | 1.152 | 4.501 | 0.38 | 0.10 |
| richards | 0.237 | 0.261 | 0.295 | 1.095 | 4.163 | 0.22 | 0.06 |
| spectral_norm | 0.267 | 0.257 | 0.293 | 0.931 | 1.817 | 0.29 | 0.15 |
| **geomean** |  |  |  |  |  | **0.37** | **0.13** |

Times in seconds.

**Update 2026-11-04 (1b6c661, tailcall only).** After a third round of
interpreter work (class instantiation and default arguments without the
generic call path, generator expressions consumed by builtins run
inline, `super(..).m(..)` as one op, list slices with raw copies;
[examples/python/FRICTION.md](../examples/python/FRICTION.md#optimizations-third-round-calls-generator-expressions-slices-2026-11-04)),
`compare.py -n 5` against b9e179a in one interleaved run: ours/before
0.88 (nqueens 0.62, hexiom 0.78, fannkuch 0.81; nbody, richards,
spectral_norm unchanged); CPython 0.38 → 0.33, MicroPython 0.13 → 0.11
(host load 2-3, so the absolute times are lower than above). goto and
switch builds were not rerun.

### prolog

| benchmark | tailcall | goto | switch | jit | swipl | trealla | tailcall/swipl | tailcall/trealla | jit/swipl | jit/trealla |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| boyer | 0.708 | 0.767 | 0.795 | 0.724 | 1.384 | 6.622 | 0.51 | 0.11 | 0.52 | 0.11 |
| browse | 0.886 | 0.921 | 0.992 | 0.906 | 1.301 | 8.451 | 0.68 | 0.10 | 0.70 | 0.11 |
| chat_parser | 0.961 | 1.010 | 1.053 | 1.022 | 1.353 | 4.867 | 0.71 | 0.20 | 0.76 | 0.21 |
| crypt | 0.530 | 0.523 | 0.547 | 0.530 | 1.313 | 4.959 | 0.40 | 0.11 | 0.40 | 0.11 |
| derive | 0.651 | 0.723 | 0.715 | 0.809 | 1.327 | 3.691 | 0.49 | 0.18 | 0.61 | 0.22 |
| divide10 | 0.685 | 0.760 | 0.779 | 0.898 | 1.312 | 3.687 | 0.52 | 0.19 | 0.68 | 0.24 |
| eval | 0.644 | 0.646 | 0.644 | 0.626 | 1.269 | 3.357 | 0.51 | 0.19 | 0.49 | 0.19 |
| fast_mu | 0.722 | 0.733 | 0.793 | 0.897 | 1.341 | 5.273 | 0.54 | 0.14 | 0.67 | 0.17 |
| flatten | 0.665 | 0.689 | 0.762 | 0.721 | 1.342 | 7.160 | 0.50 | 0.09 | 0.54 | 0.10 |
| log10 | 0.615 | 0.708 | 0.690 | 0.655 | 1.312 | 4.711 | 0.47 | 0.13 | 0.50 | 0.14 |
| meta_qsort | 0.631 | 0.708 | 0.750 | 0.901 | 1.301 | 6.899 | 0.49 | 0.09 | 0.69 | 0.13 |
| mu | 0.577 | 0.633 | 0.691 | 0.726 | 1.280 | 5.533 | 0.45 | 0.10 | 0.57 | 0.13 |
| nand | 0.793 | 0.811 | 0.887 | 0.879 | 1.282 | 4.968 | 0.62 | 0.16 | 0.69 | 0.18 |
| nreverse | 0.883 | 0.903 | 0.921 | 0.891 | 1.319 | 10.853 | 0.67 | 0.08 | 0.68 | 0.08 |
| ops8 | 0.681 | 0.717 | 0.730 | 0.808 | 1.340 | 4.258 | 0.51 | 0.16 | 0.60 | 0.19 |
| perfect | 0.767 | 0.798 | 0.846 | 0.791 | 1.311 | 6.488 | 0.59 | 0.12 | 0.60 | 0.12 |
| poly_10 | 0.694 | 0.723 | 0.766 | 0.894 | 1.159 | 3.575 | 0.60 | 0.19 | 0.77 | 0.25 |
| prover | 0.629 | 0.675 | 0.744 | 0.821 | 1.340 | 4.803 | 0.47 | 0.13 | 0.61 | 0.17 |
| qsort | 0.702 | 0.692 | 0.743 | 0.738 | 1.322 | 8.015 | 0.53 | 0.09 | 0.56 | 0.09 |
| queens_8 | 0.549 | 0.519 | 0.552 | 0.540 | 1.329 | 7.091 | 0.41 | 0.08 | 0.41 | 0.08 |
| query | 0.590 | 0.543 | 0.585 | 0.649 | 1.342 | 6.256 | 0.44 | 0.09 | 0.48 | 0.10 |
| reducer | 1.024 | 1.058 | 1.201 | 1.061 | 1.350 | 6.281 | 0.76 | 0.16 | 0.79 | 0.17 |
| sendmore | 0.626 | 0.634 | 0.659 | 0.698 | 1.274 | 4.586 | 0.49 | 0.14 | 0.55 | 0.15 |
| serialise | 0.745 | 0.762 | 0.791 | 0.811 | 1.323 | 6.918 | 0.56 | 0.11 | 0.61 | 0.12 |
| sieve | 1.213 | 1.262 | 1.311 | 58.008† | 1.310 | 3.374 | 0.93 | 0.36 | 44.28 | 17.19 |
| simple_analyzer | 0.734 | 0.749 | 0.817 | 0.720 | 1.317 | 6.778 | 0.56 | 0.11 | 0.55 | 0.11 |
| tak | 0.266 | 0.263 | 0.274 | 0.255 | 1.310 | 4.908 | 0.20 | 0.05 | 0.19 | 0.05 |
| times10 | 0.610 | 0.710 | 0.728 | 0.824 | 1.290 | 3.877 | 0.47 | 0.16 | 0.64 | 0.21 |
| unify | 0.786 | 0.787 | 0.865 | 0.724 | 1.296 | 8.755 | 0.61 | 0.09 | 0.56 | 0.08 |
| zebra | 0.902 | 0.970 | 1.035 | 0.987 | 1.296 | 4.728 | 0.70 | 0.19 | 0.76 | 0.21 |
| **geomean** |  |  |  |  |  |  | **0.53** | **0.13** | **0.66** | **0.16** |

Times in seconds.

† jit on sieve: every assert/retract makes code objects that are never freed, and the JIT compiles each one as it gets hot.
