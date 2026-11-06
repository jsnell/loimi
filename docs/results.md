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
| Date | 2026-11-05 |
| Commit | master c7e94ae |
| CPU | Intel Core i7-6700 @ 3.40 GHz, 4 cores / 8 threads (3.1 GHz in `/proc/cpuinfo` during this run) |
| Kernel | Linux 6.12.113+deb13-amd64 |
| Compilers | clang 19.1.7 (Debian), rustc 1.97.1 |
| Other | Python 3.13.5, node v26.13.0, PCRE2 10.46 (system library) |

No other agent ran; the host was less loaded from outside than in the
previous run. Each chunk waited for the 1-minute load average to fall
below 1.0, at most 3 minutes (90–180 s; it started at 0.94–1.17, the
PolyBench set at 1.77), and it was 0.9–2.6 during the runs, median 2.0,
our own benchmark process included (the previous run's was 3.0–5.0).
Every program, references included, took about 0.75–0.8 of the time of
the previous run (references 0.75–0.79 by suite geometric mean; the
clock was 3.1 GHz against 2.7), so compare ratios, not seconds, across
runs. The suites ran as six `run_all.py --only` invocations into one
output directory (mini, mini-coro: 6 minutes; minijs, lox: 9; scheme,
regex: 6; wasm, wasm-simd, stack: 6; python: 3; prolog: 12), 44 minutes
in all, then the PolyBench set (2 minutes). Single benchmarks move by
about ±3% between runs from code placement and load.

**Method.**

- Every Loimi interpreter is built with `--release --align-functions=64`
  in each dispatch mode (`--dispatch=tailcall`, `goto`, `switch`).
  Mini, MiniJS, Scheme and Prolog are also built with `--jit-stencils`
  (tail-call dispatch) and run with `LOIMI_JIT=1` (column *jit*). Since
  this run's previous one the driver also keeps every x86-64 vm branch
  off 32-byte boundaries (the JCC erratum, [lowering.md
  §12](lowering.md#12-driver-settings)), in every build.
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
| mini (14) | tailcall | 1 | lua 0.47, luajit-joff 0.75 |
| mini (14) | goto | 1.03 | lua 0.49, luajit-joff 0.77 |
| mini (14) | switch | 1.09 | lua 0.52, luajit-joff 0.81 |
| mini (14) | jit | 0.76 | lua 0.36, luajit-joff 0.57 |
| mini-coro (3) | tailcall | 1 | lua 0.36, luajit-joff 0.98 |
| mini-coro (3) | goto | 1.02 | lua 0.37, luajit-joff 1.00 |
| mini-coro (3) | switch | 1.08 | lua 0.39, luajit-joff 1.06 |
| mini-coro (3) | jit | 0.80 | lua 0.29, luajit-joff 0.78 |
| minijs (14) | tailcall | 1 | node-jitless 0.48, qjs 0.37 |
| minijs (14) | goto | 1.04 | node-jitless 0.50, qjs 0.38 |
| minijs (14) | switch | 1.21 | node-jitless 0.58, qjs 0.45 |
| minijs (14) | jit | 0.83 | node-jitless 0.40, qjs 0.31 |
| scheme (43) | tailcall | 1 | gauche 0.22, guile-nojit 0.38, guile 0.80 |
| scheme (43) | goto | 1.03 | gauche 0.22, guile-nojit 0.39, guile 0.82 |
| scheme (43) | switch | 1.11 | gauche 0.24, guile-nojit 0.42, guile 0.89 |
| scheme (43) | jit | 0.79 | gauche 0.17, guile-nojit 0.30, guile 0.63 |
| lox (10) | tailcall | 1 | clox 0.45 |
| lox (10) | goto | 1.01 | clox 0.45 |
| lox (10) | switch | 1.07 | clox 0.48 |
| regex (9) | tailcall | 1 | pcre2 0.44, python-re 0.41 |
| regex (9) | goto | 1.03 | pcre2 0.45, python-re 0.42 |
| regex (9) | switch | 1.11 | pcre2 0.49, python-re 0.46 |
| wasm (7) | tailcall | 1 | wasm3 0.85 |
| wasm (7) | goto | 1.05 | wasm3 0.90 |
| wasm (7) | switch | 1.28 | wasm3 1.09 |
| wasm-simd (3) | tailcall | 1 | mvp 0.98 |
| wasm-simd (3) | goto | 1.03 | mvp 1.01 |
| wasm-simd (3) | switch | 1.29 | mvp 1.27 |
| wasm-polybench (30) | tailcall | 1 | wasm3 0.83 |
| wasm-polybench (30) | goto | 1.05 | wasm3 0.88 |
| wasm-polybench (30) | switch | 1.25 | wasm3 1.04 |
| stack (5) | tailcall | 1 | — |
| stack (5) | goto | 1.03 | — |
| stack (5) | switch | 1.11 | — |
| python (11) | tailcall | 1 | cpython 0.33, micropython 0.11 |
| python (11) | goto | 1.10 | cpython 0.36, micropython 0.13 |
| python (11) | switch | 1.17 | cpython 0.38, micropython 0.13 |
| prolog (30) | tailcall | 1 | swipl 0.52, trealla 0.13 |
| prolog (30) | goto | 1.03 | swipl 0.54, trealla 0.13 |
| prolog (30) | switch | 1.11 | swipl 0.58, trealla 0.14 |
| prolog (30) | jit | 1.00 (0.99 without sieve) | swipl 0.52, trealla 0.13 (swipl 0.51, trealla 0.12 over 29, without sieve) |

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

- Mini (tail-call) takes 0.26–0.71 of PUC-Lua's time and 0.47–0.96 of
  LuaJIT `-joff`'s: less on all 14 (hash_strings 0.96, 1.04 in the
  previous run; spectral_norm 0.93, queens 0.92). With the JIT: 0.56–0.97
  of the interpreter's time.
- Mini's coroutine benchmarks switch threads inside the dispatch loop
  (`examples/mini/COROUTINES.md`): 0.34–0.40 of PUC-Lua's time and
  0.92–1.06 of LuaJIT `-joff`'s (chameneos 1.06); goto and switch take
  1.01–1.12× tail-call.
- MiniJS takes 0.23–0.69 of `node --jitless`'s time and 0.19–0.60 of
  QuickJS's, less on all 14. Switch mode takes 1.21× the tail-call time
  (1.12× in the previous run; nbody 1.28×, closures 1.34×).
- Scheme takes less time than Guile with its JIT on 20 of 43 programs
  (geometric mean 0.80); with Loimi's JIT, on 29 (0.63). The bignum
  programs, after std.bigint's faster division and square root: pi 1.09×
  Guile (1.30 in the previous run), chudnovsky 0.94× (1.10).
- Lox takes 0.23–0.63 of clox's time. goto takes 1.01× tail-call by
  geometric mean (1.08 in the previous run), switch 1.07× (equality
  1.22×).
- Regex: the backtracker takes less time than the PCRE2 interpreter and
  Python's `re` except on `literal` (1.24 and 1.25) and, against Python,
  `dot-star` (1.25). `pathological` is rejected by the required-byte check
  before matching (0.0 ms; Python 432 ms) and is left out of the
  geometric means.
- Wasm takes 0.75–0.94 of wasm3's time on the seven programs, less on
  all, and 0.66–0.93 on the 30 PolyBench kernels (0.83 by geometric mean;
  less on all 30). Switch builds take 1.28× the tail-call time (1.25× on
  PolyBench). goto mode takes 1.42× the tail-call time on gemm, more than
  switch mode (see "Changes" below). The SIMD build of nbody takes 0.95
  of its MVP build's time; fannkuchredux and spectralnorm, whose
  vectorized code is a few instructions outside the hot loops, 1.00.
- Python (tail-call) takes 0.22–0.43 of CPython's time, less on all 11
  (hexiom 0.43, nqueens 0.42; richards 0.22, float 0.24). MicroPython's
  output differs on chaos and go (its `random` is another generator) and
  nbody (last float digits); their times are in the table and in the
  geometric means (`compare.py` leaves them out of its own). goto takes
  1.10× the tail-call time (1.05× in the previous run; nbody 1.22×),
  switch 1.06–1.30×.
- Prolog takes 0.18–0.99 of SWI-Prolog's time, less on all 30 (tak 0.18;
  sieve, whose assert/retract compile clauses, 0.99), and 0.05–0.37 of
  Trealla's. The JIT build takes the interpreter's time (1.00 by
  geometric mean, 0.99 over the 29 programs other than sieve; 1.11 over
  the 29 in the previous run): 0.77 tak to 1.15 query, and 1.35 on sieve
  (48 in the previous run). Clauses entered from try chains now count
  toward compiling, and the registry of code objects, which sieve's
  asserted clauses made quadratic, is a treap
  ([design/copy-and-patch.md §14](design/copy-and-patch.md#14-prolog-2026-11-04)).
  `run_all.py` still marks the JIT cell of sieve † (its `PATHOLOGICAL`
  table); the mark is left out here.

## GC: mark bitmaps instead of sweeping (2026-11-04)

A dated A/B of one change, made before the previous run (95a227e); its
effect is in that run's "Changes" section. `std.gc`
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

## Changes since the previous run (95a227e)

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

A dated A/B after this run, not reflected in the tables below
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

A dated A/B after this run, not reflected in the tables below
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

**Later the same day: the fold back, MiniJS ported.** The build now
folds the table load into the `jmp` in the assembly of executables
whose handlers pass more than six register arguments (`src/asmfold.rs`;
the cause is LLVM's `X86tcret_6regs`, which counts register operands).
Remeasured with more rounds (Mini 11 at align 64), the fold makes no
difference that shows, and neither did it cost the "4% on fib" above:
a control build with the accumulator declared and unselected is within
the A/A spread of no accumulator, folded or not
(`bench/mini/RESULTS.md`, item 44). The ports, folded, against master:

| suite | align 64 | align 128 | float benchmark |
|---|---:|---:|---|
| mini (14), unchanged | 0.994 (0.998) | 0.996 (1.002) | mandelbrot 0.924 / 0.901 |
| minijs (14), **kept** | 0.997 (0.996) | 0.997 (1.003) | nbody 0.932 / 0.921 |
| scheme (43), not kept | 0.998 (1.006) | 1.005 (0.992), 1.007 (0.999) | mbrot 0.857 / 0.876 |

Scheme stays out for its integer paths (a `movq` to the accumulator in
ADD/SUB/MUL: tak, fib, cpstak 1-2% slower). The wasm interpreter, which
passes nine arguments, now gets the folded dispatch too (1217 register
jumps to 18); IR and executables of Lox, Stak, Python and Prolog are
unchanged.

## Per suite

### mini

| benchmark | tailcall | goto | switch | jit | lua | luajit-joff | tailcall/lua | tailcall/luajit-joff | jit/lua | jit/luajit-joff |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| binary_trees | 0.800 | 0.862 | 0.901 | 0.661 | 3.073 | 1.560 | 0.26 | 0.51 | 0.22 | 0.42 |
| closures | 0.438 | 0.493 | 0.537 | 0.363 | 1.228 | 0.936 | 0.36 | 0.47 | 0.30 | 0.39 |
| fannkuch | 1.135 | 1.037 | 1.253 | 0.892 | 1.709 | 1.433 | 0.66 | 0.79 | 0.52 | 0.62 |
| fib | 0.934 | 1.105 | 1.057 | 0.660 | 1.611 | 1.121 | 0.58 | 0.83 | 0.41 | 0.59 |
| hash_strings | 0.979 | 0.991 | 1.014 | 0.771 | 1.639 | 1.025 | 0.60 | 0.96 | 0.47 | 0.75 |
| interp | 0.731 | 0.721 | 0.822 | 0.561 | 2.021 | 1.091 | 0.36 | 0.67 | 0.28 | 0.51 |
| mandelbrot | 0.644 | 0.647 | 0.682 | 0.360 | 1.203 | 0.782 | 0.54 | 0.82 | 0.30 | 0.46 |
| nbody | 0.458 | 0.452 | 0.511 | 0.337 | 1.156 | 0.518 | 0.40 | 0.88 | 0.29 | 0.65 |
| oo_methods | 0.685 | 0.775 | 0.750 | 0.471 | 1.372 | 1.049 | 0.50 | 0.65 | 0.34 | 0.45 |
| queens | 0.870 | 0.853 | 0.921 | 0.661 | 1.229 | 0.941 | 0.71 | 0.92 | 0.54 | 0.70 |
| sieve | 0.646 | 0.660 | 0.679 | 0.512 | 1.506 | 1.007 | 0.43 | 0.64 | 0.34 | 0.51 |
| sort | 0.759 | 0.743 | 0.805 | 0.592 | 1.150 | 0.871 | 0.66 | 0.87 | 0.51 | 0.68 |
| spectral_norm | 1.078 | 1.091 | 1.139 | 0.741 | 1.740 | 1.163 | 0.62 | 0.93 | 0.43 | 0.64 |
| string_build | 0.707 | 0.705 | 0.724 | 0.686 | 2.488 | 0.982 | 0.28 | 0.72 | 0.28 | 0.70 |
| **geomean** |  |  |  |  |  |  | **0.47** | **0.75** | **0.36** | **0.57** |

Times in seconds.

### mini-coro

| benchmark | tailcall | goto | switch | jit | lua | luajit-joff | tailcall/lua | tailcall/luajit-joff | jit/lua | jit/luajit-joff |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| chameneos | 0.407 | 0.411 | 0.456 | 0.308 | 1.008 | 0.383 | 0.40 | 1.06 | 0.31 | 0.80 |
| coroutine_ring | 0.346 | 0.363 | 0.379 | 0.284 | 1.003 | 0.377 | 0.34 | 0.92 | 0.28 | 0.75 |
| sieve | 0.369 | 0.371 | 0.382 | 0.300 | 1.074 | 0.383 | 0.34 | 0.96 | 0.28 | 0.78 |
| **geomean** |  |  |  |  |  |  | **0.36** | **0.98** | **0.29** | **0.78** |

Times in seconds.

### minijs

| benchmark | tailcall | goto | switch | jit | node-jitless | qjs | tailcall/node-jitless | tailcall/qjs | jit/node-jitless | jit/qjs |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| awfy_micro | 0.773 | 0.794 | 0.963 | 0.643 | 1.594 | 2.049 | 0.48 | 0.38 | 0.40 | 0.31 |
| binary_trees | 0.983 | 1.104 | 1.190 | 0.804 | 1.789 | 5.117 | 0.55 | 0.19 | 0.45 | 0.16 |
| bitops | 0.696 | 0.712 | 0.869 | 0.533 | 1.622 | 1.439 | 0.43 | 0.48 | 0.33 | 0.37 |
| closures | 0.897 | 0.990 | 1.200 | 0.649 | 1.509 | 2.631 | 0.59 | 0.34 | 0.43 | 0.25 |
| deltablue | 0.840 | 0.902 | 1.040 | 0.672 | 1.521 | 2.657 | 0.55 | 0.32 | 0.44 | 0.25 |
| fannkuch | 0.466 | 0.453 | 0.578 | 0.381 | 1.324 | 1.524 | 0.35 | 0.31 | 0.29 | 0.25 |
| json | 0.914 | 0.933 | 1.061 | 0.888 | 1.550 | 2.299 | 0.59 | 0.40 | 0.57 | 0.39 |
| nbody | 0.428 | 0.437 | 0.547 | 0.316 | 1.893 | 1.363 | 0.23 | 0.31 | 0.17 | 0.23 |
| properties | 0.850 | 0.860 | 0.927 | 0.725 | 1.564 | 1.628 | 0.54 | 0.52 | 0.46 | 0.45 |
| raytrace | 0.459 | 0.475 | 0.561 | 0.417 | 1.448 | 2.224 | 0.32 | 0.21 | 0.29 | 0.19 |
| richards | 0.748 | 0.829 | 0.928 | 0.667 | 1.465 | 2.023 | 0.51 | 0.37 | 0.46 | 0.33 |
| sort | 0.854 | 0.858 | 1.028 | 0.729 | 1.399 | 1.836 | 0.61 | 0.47 | 0.52 | 0.40 |
| spectral_norm | 0.846 | 0.857 | 0.992 | 0.656 | 1.612 | 1.415 | 0.52 | 0.60 | 0.41 | 0.46 |
| strings | 1.164 | 1.157 | 1.283 | 1.065 | 1.697 | 2.243 | 0.69 | 0.52 | 0.63 | 0.47 |
| **geomean** |  |  |  |  |  |  | **0.48** | **0.37** | **0.40** | **0.31** |

Times in seconds.

### scheme

| benchmark | tailcall | goto | switch | jit | gauche | guile-nojit | guile | tailcall/gauche | tailcall/guile-nojit | tailcall/guile | jit/gauche | jit/guile-nojit | jit/guile |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| ack | 0.433 | 0.464 | 0.488 | 0.292 | 2.960 | 0.941 | 0.218 | 0.15 | 0.46 | 1.99 | 0.10 | 0.31 | 1.34 |
| array1 | 0.360 | 0.383 | 0.388 | 0.256 | 0.882 | 1.008 | 0.289 | 0.41 | 0.36 | 1.25 | 0.29 | 0.25 | 0.89 |
| browse | 0.012 | 0.012 | 0.014 | 0.010 | 0.091 | 0.030 | 0.020 | 0.13 | 0.40 | 0.60 | 0.11 | 0.33 | 0.50 |
| chudnovsky | 0.140 | 0.140 | 0.143 | 0.141 | 0.997 | 0.189 | 0.149 | 0.14 | 0.74 | 0.94 | 0.14 | 0.75 | 0.95 |
| conform | 0.089 | 0.090 | 0.098 | 0.063 | 0.462 | 0.183 | 0.078 | 0.19 | 0.49 | 1.14 | 0.14 | 0.34 | 0.81 |
| cpstak | 0.229 | 0.245 | 0.248 | 0.182 | 1.211 | 0.436 | 0.211 | 0.19 | 0.53 | 1.09 | 0.15 | 0.42 | 0.86 |
| ctak | 0.019 | 0.019 | 0.019 | 0.017 | 0.253 | 0.530 | 0.514 | 0.08 | 0.04 | 0.04 | 0.07 | 0.03 | 0.03 |
| deriv | 0.125 | 0.125 | 0.143 | 0.107 | 0.732 | 0.383 | 0.195 | 0.17 | 0.33 | 0.64 | 0.15 | 0.28 | 0.55 |
| destruc | 0.118 | 0.124 | 0.131 | 0.090 | 0.477 | 0.232 | 0.094 | 0.25 | 0.51 | 1.26 | 0.19 | 0.39 | 0.96 |
| diviter | 0.141 | 0.143 | 0.155 | 0.121 | 0.507 | 0.356 | 0.174 | 0.28 | 0.40 | 0.81 | 0.24 | 0.34 | 0.70 |
| divrec | 0.172 | 0.174 | 0.200 | 0.153 | 0.658 | 0.452 | 0.224 | 0.26 | 0.38 | 0.77 | 0.23 | 0.34 | 0.68 |
| earley | 0.034 | 0.034 | 0.038 | 0.031 | 0.147 | 0.066 | 0.033 | 0.23 | 0.52 | 1.03 | 0.21 | 0.47 | 0.94 |
| fft | 0.058 | 0.058 | 0.060 | 0.043 | 0.321 | 0.201 | 0.169 | 0.18 | 0.29 | 0.34 | 0.13 | 0.21 | 0.25 |
| fib | 0.405 | 0.429 | 0.451 | 0.336 | 1.454 | 1.098 | 0.289 | 0.28 | 0.37 | 1.40 | 0.23 | 0.31 | 1.16 |
| fibc | 0.029 | 0.029 | 0.029 | 0.023 | 0.360 | 0.643 | 0.615 | 0.08 | 0.05 | 0.05 | 0.06 | 0.04 | 0.04 |
| fibfp | 0.161 | 0.167 | 0.182 | 0.122 | 0.528 | 0.851 | 0.675 | 0.30 | 0.19 | 0.24 | 0.23 | 0.14 | 0.18 |
| gcbench | 0.009 | 0.009 | 0.011 | 0.008 | 0.145 | 0.030 | 0.018 | 0.06 | 0.30 | 0.50 | 0.06 | 0.27 | 0.44 |
| graphs | 0.112 | 0.112 | 0.131 | 0.099 | 0.626 | 0.210 | 0.074 | 0.18 | 0.53 | 1.51 | 0.16 | 0.47 | 1.34 |
| lattice | 0.043 | 0.042 | 0.049 | 0.034 | 0.224 | 0.102 | 0.044 | 0.19 | 0.42 | 0.98 | 0.15 | 0.33 | 0.77 |
| matrix | 0.062 | 0.060 | 0.068 | 0.052 | 0.282 | 0.120 | 0.053 | 0.22 | 0.52 | 1.17 | 0.18 | 0.43 | 0.98 |
| maze | 0.029 | 0.030 | 0.034 | 0.022 | 0.200 | 0.075 | 0.039 | 0.14 | 0.39 | 0.74 | 0.11 | 0.29 | 0.56 |
| mazefun | 0.120 | 0.128 | 0.136 | 0.099 | 0.439 | 0.289 | 0.096 | 0.27 | 0.42 | 1.25 | 0.23 | 0.34 | 1.03 |
| mbrot | 0.052 | 0.054 | 0.054 | 0.037 | 0.208 | 0.303 | 0.260 | 0.25 | 0.17 | 0.20 | 0.18 | 0.12 | 0.14 |
| mperm | 0.141 | 0.147 | 0.153 | 0.107 | 0.412 | 0.194 | 0.063 | 0.34 | 0.73 | 2.24 | 0.26 | 0.55 | 1.70 |
| nboyer | 0.634 | 0.681 | 0.712 | 0.496 | 1.230 | 0.922 | 0.350 | 0.52 | 0.69 | 1.81 | 0.40 | 0.54 | 1.42 |
| nqueens | 0.275 | 0.278 | 0.306 | 0.211 | 0.686 | 0.497 | 0.176 | 0.40 | 0.55 | 1.56 | 0.31 | 0.42 | 1.20 |
| ntakl | 0.169 | 0.187 | 0.182 | 0.111 | 0.533 | 0.320 | 0.078 | 0.32 | 0.53 | 2.17 | 0.21 | 0.35 | 1.42 |
| nucleic | 0.055 | 0.057 | 0.065 | 0.051 | 0.405 | 0.201 | 0.151 | 0.14 | 0.27 | 0.36 | 0.13 | 0.25 | 0.34 |
| paraffins | 0.028 | 0.028 | 0.032 | 0.025 | 0.152 | 0.056 | 0.033 | 0.18 | 0.50 | 0.85 | 0.16 | 0.45 | 0.76 |
| peval | 0.080 | 0.081 | 0.096 | 0.056 | 0.406 | 0.207 | 0.079 | 0.20 | 0.39 | 1.01 | 0.14 | 0.27 | 0.71 |
| pi | 0.117 | 0.117 | 0.117 | 0.122 | 2.968 | 0.108 | 0.107 | 0.04 | 1.08 | 1.09 | 0.04 | 1.13 | 1.14 |
| pnpoly | 0.077 | 0.077 | 0.084 | 0.060 | 0.225 | 0.364 | 0.231 | 0.34 | 0.21 | 0.33 | 0.27 | 0.16 | 0.26 |
| primes | 0.131 | 0.170 | 0.148 | 0.106 | 0.578 | 0.319 | 0.213 | 0.23 | 0.41 | 0.62 | 0.18 | 0.33 | 0.50 |
| puzzle | 0.101 | 0.105 | 0.117 | 0.064 | 0.292 | 0.241 | 0.089 | 0.35 | 0.42 | 1.13 | 0.22 | 0.27 | 0.72 |
| quicksort | 0.106 | 0.111 | 0.116 | 0.080 | 0.302 | 0.199 | 0.078 | 0.35 | 0.53 | 1.36 | 0.26 | 0.40 | 1.03 |
| sboyer | 0.599 | 0.629 | 0.708 | 0.433 | 1.287 | 0.975 | 0.311 | 0.47 | 0.61 | 1.93 | 0.34 | 0.44 | 1.39 |
| simplex | 0.097 | 0.098 | 0.113 | 0.083 | 0.547 | 0.254 | 0.117 | 0.18 | 0.38 | 0.83 | 0.15 | 0.33 | 0.71 |
| string | 0.018 | 0.018 | 0.017 | 0.018 | 0.135 | 0.029 | 0.028 | 0.13 | 0.62 | 0.64 | 0.13 | 0.62 | 0.64 |
| sum | 0.337 | 0.369 | 0.367 | 0.194 | 0.835 | 0.648 | 0.173 | 0.40 | 0.52 | 1.95 | 0.23 | 0.30 | 1.12 |
| sumfp | 0.106 | 0.120 | 0.115 | 0.064 | 0.291 | 0.563 | 0.537 | 0.36 | 0.19 | 0.20 | 0.22 | 0.11 | 0.12 |
| tak | 0.391 | 0.368 | 0.445 | 0.283 | 1.266 | 0.975 | 0.237 | 0.31 | 0.40 | 1.65 | 0.22 | 0.29 | 1.19 |
| takl | 0.082 | 0.080 | 0.107 | 0.057 | 0.315 | 0.167 | 0.045 | 0.26 | 0.49 | 1.82 | 0.18 | 0.34 | 1.27 |
| triangl | 0.156 | 0.163 | 0.185 | 0.110 | 0.421 | 0.426 | 0.132 | 0.37 | 0.37 | 1.18 | 0.26 | 0.26 | 0.83 |
| **geomean** |  |  |  |  |  |  |  | **0.22** | **0.38** | **0.80** | **0.17** | **0.30** | **0.63** |

Times in seconds.

### lox

| benchmark | tailcall | goto | switch | clox | tailcall/clox |
|---|---:|---:|---:|---:|---:|
| binary_trees | 0.725 | 0.750 | 0.766 | 3.184 | 0.23 |
| equality | 0.981 | 1.001 | 1.200 | 3.117 | 0.31 |
| fib | 0.396 | 0.408 | 0.436 | 0.987 | 0.40 |
| instantiation | 0.339 | 0.337 | 0.346 | 0.536 | 0.63 |
| invocation | 0.142 | 0.142 | 0.150 | 0.276 | 0.51 |
| method_call | 0.082 | 0.088 | 0.088 | 0.174 | 0.47 |
| properties | 0.212 | 0.192 | 0.204 | 0.415 | 0.51 |
| trees | 1.346 | 1.373 | 1.362 | 2.408 | 0.56 |
| zoo | 0.158 | 0.161 | 0.179 | 0.309 | 0.51 |
| zoo_batch | 0.957 | 0.957 | 1.010 | 1.924 | 0.50 |
| **geomean** |  |  |  |  | **0.45** |

Times in seconds.

Note (2026-11-05): with `#[profile]` (hot-handler ordering), tail-call
`properties` takes 0.188 s instead of 0.212 s; see "Hot-handler ordering
and the two alignment losses" above.

### regex

| benchmark | tailcall | goto | switch | pcre2 | python-re | tailcall/pcre2 | tailcall/python-re |
|---|---:|---:|---:|---:|---:|---:|---:|
| alt-heavy | 23.3 | 23.8 | 25.9 | 49.9 | 32.0 | 0.47 | 0.73 |
| capture | 17.1 | 18.6 | 19.3 | 51.1 | 110.2 | 0.34 | 0.16 |
| class | 60.8 | 58.1 | 65.8 | 220.5 | 154.2 | 0.28 | 0.39 |
| dot-star | 10.1 | 10.3 | 10.4 | 21.4 | 8.1 | 0.47 | 1.25 |
| literal | 3.1 | 3.5 | 3.1 | 2.5 | 2.5 | 1.24 | 1.25 |
| literal-alt | 8.9 | 9.1 | 9.1 | 18.1 | 36.6 | 0.49 | 0.24 |
| patho-alt | 45.0 | 44.9 | 64.4 | 154.7 | 122.6 | 0.29 | 0.37 |
| pathological | 0.0 | 0.0 | 0.0 | 0.0 | 431.9 | — | — |
| words | 20.1 | 21.9 | 22.6 | 54.6 | 156.0 | 0.37 | 0.13 |
| **geomean** |  |  |  |  |  | **0.44** | **0.41** |

Times in milliseconds.

### wasm

| benchmark | tailcall | goto | switch | wasm3 | tailcall/wasm3 |
|---|---:|---:|---:|---:|---:|
| binarytrees | 1.296 | 1.415 | 1.633 | 1.457 | 0.89 |
| coremark | 1.792 | 1.908 | 2.280 | 2.017 | 0.89 |
| fannkuchredux | 1.317 | 1.394 | 1.720 | 1.657 | 0.79 |
| fasta | 1.105 | 1.137 | 1.321 | 1.172 | 0.94 |
| mandelbrot | 1.498 | 1.596 | 2.033 | 2.003 | 0.75 |
| nbody | 1.748 | 1.819 | 2.187 | 2.006 | 0.87 |
| spectralnorm | 1.613 | 1.634 | 2.162 | 1.913 | 0.84 |
| **geomean** |  |  |  |  | **0.85** |

Times in seconds.

### wasm-simd

| benchmark | tailcall | goto | switch | mvp | tailcall/mvp |
|---|---:|---:|---:|---:|---:|
| fannkuchredux_simd | 1.314 | 1.372 | 1.712 | 1.317 | 1.00 |
| nbody_simd | 1.666 | 1.727 | 2.061 | 1.748 | 0.95 |
| spectralnorm_simd | 1.617 | 1.632 | 2.164 | 1.613 | 1.00 |
| **geomean** |  |  |  |  | **0.98** |

Times in seconds.

### wasm-polybench

The 30 PolyBench/C 4.2.1 kernels, built like the `wasm` programs, each at
the largest standard dataset that runs in at most about 2 s under wasm3
(`bench/wasm/polybench.sh`); the interpreter was not tuned on them.
Geometric means of the times: wasm3 0.193 s, tail-call 0.161, goto
0.170, switch 0.201.

| benchmark | tailcall | goto | switch | wasm3 | tailcall/wasm3 |
|---|---:|---:|---:|---:|---:|
| 2mm | 0.103 | 0.105 | 0.127 | 0.122 | 0.84 |
| 3mm | 0.158 | 0.161 | 0.192 | 0.188 | 0.84 |
| adi | 0.207 | 0.224 | 0.259 | 0.242 | 0.86 |
| atax | 0.089 | 0.093 | 0.111 | 0.102 | 0.87 |
| bicg | 0.091 | 0.095 | 0.116 | 0.101 | 0.90 |
| cholesky | 0.569 | 0.628 | 0.706 | 0.718 | 0.79 |
| correlation | 0.054 | 0.056 | 0.066 | 0.063 | 0.86 |
| covariance | 0.051 | 0.053 | 0.062 | 0.060 | 0.85 |
| deriche | 0.811 | 0.848 | 0.984 | 1.129 | 0.72 |
| doitgen | 0.047 | 0.050 | 0.059 | 0.056 | 0.84 |
| durbin | 0.127 | 0.126 | 0.165 | 0.148 | 0.86 |
| fdtd-2d | 0.155 | 0.171 | 0.206 | 0.190 | 0.82 |
| floyd-warshall | 1.266 | 1.287 | 1.497 | 1.487 | 0.85 |
| gemm | 0.090 | 0.128 | 0.120 | 0.101 | 0.89 |
| gemver | 0.679 | 0.699 | 0.853 | 0.838 | 0.81 |
| gesummv | 0.245 | 0.253 | 0.302 | 0.265 | 0.92 |
| gramschmidt | 0.096 | 0.098 | 0.112 | 0.109 | 0.88 |
| heat-3d | 0.233 | 0.246 | 0.322 | 0.289 | 0.81 |
| jacobi-1d | 0.065 | 0.069 | 0.083 | 0.084 | 0.77 |
| jacobi-2d | 0.163 | 0.173 | 0.202 | 0.205 | 0.80 |
| lu | 0.644 | 0.709 | 0.795 | 0.806 | 0.80 |
| ludcmp | 0.616 | 0.675 | 0.763 | 0.811 | 0.76 |
| mvt | 0.443 | 0.455 | 0.530 | 0.526 | 0.84 |
| nussinov | 0.161 | 0.163 | 0.205 | 0.244 | 0.66 |
| seidel-2d | 0.296 | 0.324 | 0.370 | 0.350 | 0.85 |
| symm | 0.070 | 0.072 | 0.087 | 0.084 | 0.83 |
| syr2k | 0.084 | 0.090 | 0.107 | 0.102 | 0.82 |
| syrk | 0.051 | 0.051 | 0.067 | 0.058 | 0.88 |
| trisolv | 0.111 | 0.103 | 0.126 | 0.120 | 0.93 |
| trmm | 0.040 | 0.043 | 0.049 | 0.043 | 0.93 |
| **geomean** |  |  |  |  | **0.83** |

Times in seconds.

Note (2026-11-05): with `#[profile]` (hot-handler ordering), goto-mode
`gemm` takes 0.094 s instead of 0.131 s and the goto column's geometric
mean is 0.988 of this one's; see "Hot-handler ordering and the two
alignment losses" above.

### stack

| benchmark | tailcall | goto | switch |
|---|---:|---:|---:|
| fib | 0.052 | 0.059 | 0.066 |
| loop | 0.739 | 0.714 | 0.750 |
| nbody | 0.149 | 0.159 | 0.177 |
| sieve | 0.090 | 0.096 | 0.098 |
| strings | 0.124 | 0.118 | 0.123 |
| **geomean** |  |  |  |

Times in seconds.

### python

| benchmark | tailcall | goto | switch | cpython | micropython | tailcall/cpython | tailcall/micropython |
|---|---:|---:|---:|---:|---:|---:|---:|
| chaos | 0.248 | 0.279 | 0.280 | 0.706 | 1.842 | 0.35 | 0.13 |
| deltablue | 0.342 | 0.376 | 0.412 | 0.959 | 4.762 | 0.36 | 0.07 |
| fannkuch | 0.275 | 0.298 | 0.304 | 0.907 | 1.393 | 0.30 | 0.20 |
| float | 0.205 | 0.220 | 0.238 | 0.870 | 2.149 | 0.24 | 0.10 |
| go | 0.286 | 0.311 | 0.335 | 0.796 | 1.826 | 0.36 | 0.16 |
| hexiom | 0.295 | 0.324 | 0.356 | 0.682 | 2.973 | 0.43 | 0.10 |
| nbody | 0.353 | 0.432 | 0.457 | 0.958 | 3.387 | 0.37 | 0.10 |
| nqueens | 0.334 | 0.356 | 0.370 | 0.794 | 1.660 | 0.42 | 0.20 |
| raytrace | 0.300 | 0.335 | 0.339 | 0.867 | 3.394 | 0.35 | 0.09 |
| richards | 0.179 | 0.202 | 0.233 | 0.817 | 3.175 | 0.22 | 0.06 |
| spectral_norm | 0.200 | 0.208 | 0.212 | 0.699 | 1.365 | 0.29 | 0.15 |
| **geomean** |  |  |  |  |  | **0.33** | **0.11** |

Times in seconds.

### prolog

| benchmark | tailcall | goto | switch | jit | swipl | trealla | tailcall/swipl | tailcall/trealla | jit/swipl | jit/trealla |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| boyer | 0.555 | 0.568 | 0.615 | 0.506 | 1.032 | 4.969 | 0.54 | 0.11 | 0.49 | 0.10 |
| browse | 0.686 | 0.693 | 0.788 | 0.690 | 1.014 | 6.451 | 0.68 | 0.11 | 0.68 | 0.11 |
| chat_parser | 0.727 | 0.761 | 0.806 | 0.704 | 1.026 | 3.689 | 0.71 | 0.20 | 0.69 | 0.19 |
| crypt | 0.415 | 0.387 | 0.421 | 0.389 | 1.014 | 3.800 | 0.41 | 0.11 | 0.38 | 0.10 |
| derive | 0.468 | 0.530 | 0.557 | 0.509 | 1.016 | 2.823 | 0.46 | 0.17 | 0.50 | 0.18 |
| divide10 | 0.504 | 0.574 | 0.594 | 0.487 | 1.023 | 2.790 | 0.49 | 0.18 | 0.48 | 0.17 |
| eval | 0.470 | 0.484 | 0.497 | 0.454 | 0.968 | 2.587 | 0.49 | 0.18 | 0.47 | 0.18 |
| fast_mu | 0.541 | 0.553 | 0.608 | 0.582 | 1.019 | 3.964 | 0.53 | 0.14 | 0.57 | 0.15 |
| flatten | 0.517 | 0.524 | 0.588 | 0.483 | 1.001 | 5.391 | 0.52 | 0.10 | 0.48 | 0.09 |
| log10 | 0.439 | 0.506 | 0.537 | 0.448 | 1.008 | 3.614 | 0.44 | 0.12 | 0.44 | 0.12 |
| meta_qsort | 0.493 | 0.544 | 0.596 | 0.526 | 1.010 | 5.233 | 0.49 | 0.09 | 0.52 | 0.10 |
| mu | 0.445 | 0.483 | 0.536 | 0.503 | 0.984 | 4.257 | 0.45 | 0.10 | 0.51 | 0.12 |
| nand | 0.613 | 0.620 | 0.696 | 0.633 | 0.969 | 3.699 | 0.63 | 0.17 | 0.65 | 0.17 |
| nreverse | 0.612 | 0.589 | 0.609 | 0.642 | 1.011 | 8.149 | 0.61 | 0.08 | 0.64 | 0.08 |
| ops8 | 0.496 | 0.532 | 0.572 | 0.525 | 1.014 | 3.279 | 0.49 | 0.15 | 0.52 | 0.16 |
| perfect | 0.567 | 0.598 | 0.643 | 0.519 | 1.003 | 4.969 | 0.57 | 0.11 | 0.52 | 0.10 |
| poly_10 | 0.531 | 0.539 | 0.604 | 0.543 | 1.003 | 2.727 | 0.53 | 0.19 | 0.54 | 0.20 |
| prover | 0.486 | 0.518 | 0.577 | 0.515 | 1.012 | 3.675 | 0.48 | 0.13 | 0.51 | 0.14 |
| qsort | 0.534 | 0.512 | 0.541 | 0.528 | 1.010 | 6.165 | 0.53 | 0.09 | 0.52 | 0.09 |
| queens_8 | 0.426 | 0.395 | 0.425 | 0.411 | 0.998 | 5.418 | 0.43 | 0.08 | 0.41 | 0.08 |
| query | 0.464 | 0.430 | 0.449 | 0.533 | 1.023 | 4.737 | 0.45 | 0.10 | 0.52 | 0.11 |
| reducer | 0.759 | 0.788 | 0.911 | 0.676 | 1.014 | 4.791 | 0.75 | 0.16 | 0.67 | 0.14 |
| sendmore | 0.513 | 0.477 | 0.531 | 0.543 | 0.978 | 3.469 | 0.52 | 0.15 | 0.56 | 0.16 |
| serialise | 0.605 | 0.616 | 0.679 | 0.602 | 1.003 | 5.171 | 0.60 | 0.12 | 0.60 | 0.12 |
| sieve | 0.975 | 0.990 | 1.030 | 1.320 | 0.984 | 2.617 | 0.99 | 0.37 | 1.34 | 0.50 |
| simple_analyzer | 0.562 | 0.575 | 0.623 | 0.485 | 1.010 | 4.986 | 0.56 | 0.11 | 0.48 | 0.10 |
| tak | 0.206 | 0.198 | 0.209 | 0.159 | 1.118 | 3.821 | 0.18 | 0.05 | 0.14 | 0.04 |
| times10 | 0.463 | 0.526 | 0.545 | 0.444 | 1.017 | 2.942 | 0.46 | 0.16 | 0.44 | 0.15 |
| unify | 0.580 | 0.595 | 0.674 | 0.503 | 0.968 | 6.505 | 0.60 | 0.09 | 0.52 | 0.08 |
| zebra | 0.710 | 0.761 | 0.812 | 0.741 | 1.008 | 3.642 | 0.70 | 0.19 | 0.74 | 0.20 |
| **geomean** |  |  |  |  |  |  | **0.52** | **0.13** | **0.52** | **0.13** |

Times in seconds.
