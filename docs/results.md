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
| Date | 2026-11-03 |
| Commit | master a663b98 (plus the `bench/` script changes of this run: the prolog and wasm-simd suites) |
| CPU | Intel Core i7-6700 @ 3.40 GHz, 4 cores / 8 threads |
| Kernel | Linux 6.12.113+deb13-amd64 |
| Compilers | clang 19.1.7 (Debian), rustc 1.97.1 |
| Other | Python 3.13.5, node v26.13.0, PCRE2 10.46 (system library) |

The machine was shared with another agent's builds (load average 0.4–3.4
during the runs, median 1.6; the previous run's was 2–4.7). The suites
ran as four `run_all.py --only` invocations of 8–14 minutes each (mini,
mini-coro, minijs; scheme, lox, regex; wasm, wasm-simd, stack, python;
prolog), 42 minutes in all. Single benchmarks move by about ±3% between
runs from code placement and load, more for some references (LuaJIT's
`sieve` and `string_build` took 25% less time than in the previous run).

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
  Prolog: `done`).
- Times are the minimum wall-clock time of the whole process over 3 runs,
  in seconds, with the implementations interleaved within each benchmark
  (`bench.py` does not interleave, so it runs in 3 rounds of one run).
  The regex times are measured inside the process, each the best of 3
  matches over 4 MiB of text, and the minimum over 3 processes, in
  milliseconds. Prolog's JIT build on `sieve` (38 s a run) ran once
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

To reproduce (about 42 minutes; `--quick` runs one benchmark per suite,
once, mostly builds):

```sh
bench/lua/install.sh; bench/js/install.sh; bench/scheme/fetch.sh; bench/scheme/install.sh
bench/lox/install.sh; bench/wasm/install.sh; bench/wasm/programs.sh; bench/wasm/programs-simd.sh
bench/python/install.sh; bench/prolog/install.sh
bench/run_all.py [--quick] [--only mini,scheme] [--out DIR]
bench/run_all.py --report DIR/results.json     # the tables below, again
```

## Summary: geometric mean of ours / reference

| interpreter | build | vs tailcall | references (ours / ref) |
|---|---|---:|---|
| mini (14) | tailcall | 1 | lua 0.54, luajit-joff 0.85 |
| mini (14) | goto | 1.03 | lua 0.56, luajit-joff 0.87 |
| mini (14) | switch | 1.07 | lua 0.58, luajit-joff 0.91 |
| mini (14) | jit | 0.75 | lua 0.41, luajit-joff 0.64 |
| mini-coro (3) | tailcall | 1 | lua 0.41, luajit-joff 1.11 |
| mini-coro (3) | goto | 1.01 | lua 0.41, luajit-joff 1.12 |
| mini-coro (3) | switch | 0.97 | lua 0.40, luajit-joff 1.08 |
| mini-coro (3) | jit | 0.79 | lua 0.32, luajit-joff 0.88 |
| minijs (14) | tailcall | 1 | node-jitless 0.52, qjs 0.40 |
| minijs (14) | goto | 1.02 | node-jitless 0.53, qjs 0.40 |
| minijs (14) | switch | 1.09 | node-jitless 0.56, qjs 0.43 |
| minijs (14) | jit | 0.86 | node-jitless 0.44, qjs 0.34 |
| scheme (43) | tailcall | 1 | gauche 0.26, guile-nojit 0.46, guile 0.97 |
| scheme (43) | goto | 1.03 | gauche 0.27, guile-nojit 0.47, guile 1.00 |
| scheme (43) | switch | 1.09 | gauche 0.28, guile-nojit 0.50, guile 1.05 |
| scheme (43) | jit | 0.82 | gauche 0.22, guile-nojit 0.38, guile 0.80 |
| lox (10) | tailcall | 1 | clox 0.45 |
| lox (10) | goto | 1.03 | clox 0.46 |
| lox (10) | switch | 1.04 | clox 0.47 |
| regex (9) | tailcall | 1 | pcre2 0.44, python-re 0.41 |
| regex (9) | goto | 1.02 | pcre2 0.45, python-re 0.42 |
| regex (9) | switch | 1.10 | pcre2 0.49, python-re 0.45 |
| wasm (7) | tailcall | 1 | wasm3 0.91 |
| wasm (7) | goto | 1.05 | wasm3 0.96 |
| wasm (7) | switch | 1.27 | wasm3 1.16 |
| wasm-simd (3) | tailcall | 1 | mvp 0.99 |
| wasm-simd (3) | goto | 1.03 | mvp 1.02 |
| wasm-simd (3) | switch | 1.27 | mvp 1.27 |
| stack (5) | tailcall | 1 | — |
| stack (5) | goto | 1.00 | — |
| stack (5) | switch | 1.19 | — |
| python (11) | tailcall | 1 | cpython 0.52, micropython 0.18 |
| python (11) | goto | 1.03 | cpython 0.53, micropython 0.19 |
| python (11) | switch | 1.15 | cpython 0.59, micropython 0.21 |
| prolog (30) | tailcall | 1 | swipl 0.79, trealla 0.19 |
| prolog (30) | goto | 1.01 | swipl 0.80, trealla 0.19 |
| prolog (30) | switch | 1.08 | swipl 0.85, trealla 0.21 |
| prolog (30) | jit | 1.11 (1.01 without sieve) | swipl 0.88, trealla 0.21 (swipl 0.77, trealla 0.18 over 29, without sieve) |

(n) is the number of benchmarks. *vs tailcall* is the geometric mean of
that build's times over the tail-call build's. Suites: `mini` (Mini, a
Lua 5.4 subset, 14 clean-room benchmarks), `mini-coro` (Mini's three
coroutine benchmarks), `minijs` (14 clean-room benchmarks), `scheme` (43
r7rs-benchmarks programs: the 41 of the previous run plus the bignum
programs pi and chudnovsky), `lox` (the book's benchmarks), `regex` (the
backtracking engine), `wasm` (C programs compiled to MVP wasm32-wasi),
`wasm-simd` (three of them compiled with `-msimd128`; *mvp* is the
tail-call time of the MVP build in every row), `stack` (Stak),
`python` (the Python subset, 11 pyperformance programs), `prolog` (the
30 van Roy / Aquarius programs of SWI-Prolog's bench suite).

## Notes

- Mini (tail-call) takes 0.35–0.74 of PUC-Lua's time and 0.51–1.09 of
  LuaJIT `-joff`'s: less on 13 of 14 (string_build 1.09; LuaJIT's own
  time on it was 25% less than in the previous run, ours 15%). With the
  JIT: 0.57–0.98 of the interpreter's time.
- Mini's coroutine benchmarks switch threads inside the dispatch loop
  (`examples/mini/COROUTINES.md`): 0.38–0.45 of PUC-Lua's time and
  1.06–1.19 of LuaJIT `-joff`'s; goto and switch now within ±6% of
  tail-call (they were 1.20–1.23× when each resume entered a nested run).
  See "Regressions" for the 4% these lost at a0efc73.
- MiniJS takes 0.23–0.87 of `node --jitless`'s time and 0.22–0.66 of
  QuickJS's, less on all 14.
- Scheme takes less time than Guile with its JIT on 12 of 43 programs
  (geometric mean 0.97; 0.96 over the previous run's 41); with Loimi's
  JIT, 0.80. The bignum programs: pi 1.31× Guile (GMP's subquadratic
  division and square root against std.bigint's), chudnovsky 1.15×.
- Lox takes 0.30–0.67 of clox's time. Switch mode is within 5% of
  tail-call except `equality` (1.33×; 1.8× before the tail-merge fix).
- Regex: the backtracker takes less time than the PCRE2 interpreter and
  Python's `re` except on `literal` (1.26 and 1.23) and, against Python,
  `dot-star` (1.27). `pathological` is rejected by the required-byte check
  before matching (0.0 ms; Python 432 ms) and is left out of the
  geometric means.
- Wasm takes 0.78–1.01 of wasm3's time. Switch builds take 1.27× the
  tail-call time (1.67× in the previous run). The SIMD builds of nbody
  take 0.94 of its MVP build's time; fannkuchredux and spectralnorm,
  whose vectorized code is a few instructions outside the hot loops, 1.02–1.03.
- Python (tail-call) takes 0.36–0.91 of CPython's time, less on all 11
  (nqueens 0.91, hexiom 0.73; richards and nbody 0.36–0.37).
  MicroPython's output differs on chaos and go (its `random` is another
  generator) and nbody (last float digits); their times are in the table
  and in the geometric means (`compare.py` leaves them out of its own).
  Switch takes 1.42× the tail-call time on nbody.
- Prolog takes 0.56–1.10 of SWI-Prolog's time (more on boyer, query,
  queens_8, sendmore and, 1.96, sieve, whose assert/retract compile
  clauses) and 0.10–0.31 of Trealla's (sieve 0.79). The JIT build takes
  1.01 of the interpreter's time over the 29 programs other than sieve
  (0.89 nreverse to 1.14 queens_8) and 21× on sieve, where every
  assert/retract makes code objects that are never freed and the JIT
  compiles each as it gets hot (`examples/prolog/FRICTION.md`, item 6):
  that cell is marked † and the summary gives the geometric means with
  and without it. Stencils do not help a WAM whose ops mostly unify and
  allocate (the work is in the handlers' bodies, not in dispatch).

## Changes since the previous run (68012bd)

The previous tables (commit 68012bd, same machine and method, load
average 2–4.7) against these, geometric means over the benchmarks both
runs have. *time* is new/old of our own times (the references also took
1–9% less time this run, so the machine accounts for most of the
difference); *ratio* is the tail-call (or JIT) build over the first
reference, old → new.

| suite | tailcall time | goto time | switch time | jit time | references' time | ratio old → new |
|---|---:|---:|---:|---:|---:|---|
| mini (14) | 0.95 | 0.98 | 0.90 | 0.95 | 0.92–0.93 | lua 0.53 → 0.54, jit 0.40 → 0.41 |
| mini-coro (3) | 0.70 | 0.58 | 0.55 | 0.60 | 0.94 | lua 0.55 → 0.41, jit 0.51 → 0.32 |
| minijs (14) | 0.95 | 0.95 | 0.89 | 0.95 | 0.95 | node-jitless 0.52 → 0.52, jit 0.44 → 0.44 |
| scheme (41) | 0.92 | 0.94 | 0.92 | 0.92 | 0.91–0.93 | gauche 0.27 → 0.28, guile 0.96 → 0.96, jit 0.78 → 0.78 |
| lox (10) | 0.93 | 0.93 | 0.90 | | 0.93 | clox 0.45 → 0.45 |
| regex (8) | 0.96 | 0.97 | 0.98 | | 0.97–0.98 | pcre2 0.44 → 0.44 |
| wasm (7) | 0.98 | 0.97 | 0.75 | | 0.98 | wasm3 0.91 → 0.91 |
| stack (5) | 1.01 | 1.01 | 0.99 | | | |
| python (11) | 0.97 | 0.96 | 1.00 | | 0.99 | cpython 0.53 → 0.52 |

New in this run: the `prolog` suite (with its JIT build), the
`wasm-simd` suite, and Scheme's pi and chudnovsky. Relative to the
references nothing moved by more than 0.02 except where code changed:
switch mode (all suites, most in wasm) and Mini's coroutines. What
changed, from the follow-ups recorded on this page since the previous
run:

- **Switch mode without LLVM tail merging** (031a492). In the switch
  builds (one function, one shared `switch`), branch folding kept a
  single copy of the handlers' common tails (store the result, `pc +=
  1`, branch to the switch), so handlers reached the dispatch block
  through two to four unconditional jumps (Lox's EQK through four); no
  spills were involved. The driver now passes `-enable-tail-merge=false`
  when a program has a switch-mode vm (`lowering.md` §6, §12). Switch
  over tail-call: wasm 1.67 → 1.27, Lox `equality` 1.8 → 1.33, Mini 1.14
  → 1.07, MiniJS 1.15 → 1.09.
- **Mini coroutines switch inside the run** (59087f2, 8c60ac9, 5f79f1e).
  Before, every `resume` made a nested run of the vm; in goto and switch
  mode that entry is the function holding every handler (six pushes and
  pops, a 1.3 KB frame, the state set-up per resume/yield pair: 11% of
  `coroutine_ring` before the first dispatch), so those modes were
  1.2× tail-call. Now the CALL handler saves the running thread's state
  in its `Coro`, installs the other's stacks and registers and jumps to
  its saved resume point: no native call, no nested entry, no C stack
  per coroutine. Times 0.55–0.70 of the previous run's in every mode.
- **Regex's 3.5% since b88a8ce is `#[catch_panic(-2)]` on `rx_exec`**
  (ea536af, found by bisection): installing the handler, the frame
  pointer the builtin setjmp needs and the setjmp itself, about 1% each,
  per call. Kept: it is the price of catching internal errors per call.
- **Stak's sieve and strings** were not slower than at its `FRICTION.md`
  table; the difference was the machine or session (a8ccba2, b88a8ce
  and master all within 0.091–0.093 s and 0.146–0.157 s).
- **Python**: `/` and `%` on `i48` fold their division checks, instance
  attributes on `std.dict.KeyDict` (cc599a9), ints on the shared nanbox
  layout (fbd062e, 85da353), live instance `__dict__` (cedd033):
  each within ±3% (spectral_norm 0.98).
- **Scheme exact bignums** (c299e07): fixnum overflow promotes instead of
  producing a flonum, so pi and chudnovsky run (the other 41 within
  1.5%). **Subquadratic std.bigint** (cc4e91d: Toom-3, Burnikel-Ziegler
  division, Karatsuba square root, divide-and-conquer radix conversion,
  reciprocal division) made pi's arithmetic 1.8× faster, and
  **carry-chain intrinsics** (ee9e432 `add_carry`/`sub_borrow`/
  `mul_add_wide`, used in 6626272 and 68c495d) another 10%: pi went from
  2.39× Guile's time to 1.31× (0.262 → 0.134 s), chudnovsky from 1.37×
  to 1.15×. The remaining gap is GMP's base cases and its subquadratic
  algorithms at pi's sizes (`docs/stdlib.md`, "std.bigint").
- **Wasm SIMD** (0a27f64; `lane_at`/`with_lane_at` and `--cpu` in
  a663b98): non-SIMD programs unchanged; the `-msimd128` builds are in
  `wasm-simd`. `--cpu=x86-64-v2/v3` was within noise on these programs
  (`examples/wasm/FRICTION.md`), so the builds here use the baseline.
- **Prolog follow-ups** (9c491cc): frameless allocation (`try_new`), a
  call-free write-barrier slow path, no barrier for reference-free
  stores; geometric means of the other suites within ±1% then
  (`examples/prolog/FRICTION.md`), but see below.

## Regressions

Against each suite's last recorded number (the previous run, or the
follow-up measurements above), no suite is more than 5% slower by
geometric mean except Mini's coroutine benchmarks in tail-call mode:
0.442/0.383/0.401 s here against 0.417/0.356/0.390 s in the coroutine
follow-up (1.054; the references took 3–5% less time this run). Rerun
(best of 7, interleaved, `--align-functions=64`): master takes 1.045×
the time of 8294238 (the follow-up's commit). Bisected over the 39
first-parent commits and then inside the 9c491cc merge: the whole step
is a0efc73 ("Frameless allocation and barriers in handlers: ...
call-free barrier slow path; no barrier for reference-free stores"),
1.037–1.044× its parent 0492cab; chameneos and coroutine_ring 6%,
sieve unchanged; every earlier commit within 1%. The 14 clean-room Mini
benchmarks did not move (0.994 in that commit's own A/B).

**Cause and fix.** Not the remembered set: chameneos and coroutine_ring
run no collection at all (`LOIMI_GC_STATS`), and sieve, which does,
did not move. The barrier's slow path, now inline at each barrier site
instead of one call, made `coro.enter` (two barriers) and
`coro.suspend` (two) too large for LLVM to inline: in the tail-call
binary they became out-of-line functions (363 and 315 bytes) called
from the CALL handlers' cold blocks (`switch_in`) and `switch_out`; no
other Mini function lost its inlining. `#[inline]` on the two restored
tail-call mode (1.004× 0492cab) but made switch mode 7-10% slower, so
instead `enter` and `suspend` store their references (`open`, `prev`)
through raw pointers and log the objects themselves: a
`gc.needs_barrier` test per object and an out-of-line `gc.remember`,
at the end of `enter` so that the rare call is a tail call (in the
middle it gave `enter`, out of line in goto and switch mode, a frame
of four pushes: 2% in goto mode). Best of 7, interleaved,
`--align-functions=64`, 0492cab / master / fix, tail-call: chameneos
0.405 / 0.427 / 0.413 s, coroutine_ring 0.344 / 0.365 / 0.339, sieve
0.371 / 0.371 / 0.361 (fix 0.992× 0492cab, 0.957× master). Goto, fix
against master: 0.99–1.02; switch 0.995–0.997; JIT (best of 5)
0.92–1.00. The 14 Mini benchmarks: 0.997 against 0492cab, as master.
`-W handler_frame` counts unchanged (Mini 38; the nine interpreters
201 at master, 223 at 0492cab). std.gc and the compiler are unchanged,
so the IR of every example other than Mini is identical.

## Per suite

### mini

| benchmark | tailcall | goto | switch | jit | lua | luajit-joff | tailcall/lua | tailcall/luajit-joff | jit/lua | jit/luajit-joff |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| binary_trees | 0.854 | 0.925 | 0.942 | 0.709 | 2.430 | 1.378 | 0.35 | 0.62 | 0.29 | 0.51 |
| closures | 0.464 | 0.508 | 0.533 | 0.390 | 1.198 | 0.908 | 0.39 | 0.51 | 0.33 | 0.43 |
| fannkuch | 1.197 | 1.055 | 1.211 | 0.905 | 1.672 | 1.393 | 0.72 | 0.86 | 0.54 | 0.65 |
| fib | 0.951 | 1.101 | 1.197 | 0.639 | 1.557 | 1.096 | 0.61 | 0.87 | 0.41 | 0.58 |
| hash_strings | 1.023 | 1.000 | 1.079 | 0.774 | 1.575 | 1.069 | 0.65 | 0.96 | 0.49 | 0.72 |
| interp | 0.939 | 0.969 | 1.008 | 0.695 | 1.937 | 1.060 | 0.48 | 0.89 | 0.36 | 0.66 |
| mandelbrot | 0.622 | 0.641 | 0.645 | 0.352 | 1.162 | 0.760 | 0.54 | 0.82 | 0.30 | 0.46 |
| nbody | 0.457 | 0.480 | 0.513 | 0.349 | 1.197 | 0.518 | 0.38 | 0.88 | 0.29 | 0.67 |
| oo_methods | 0.742 | 0.815 | 0.786 | 0.500 | 1.319 | 0.985 | 0.56 | 0.75 | 0.38 | 0.51 |
| queens | 0.891 | 0.896 | 0.938 | 0.662 | 1.199 | 0.914 | 0.74 | 0.97 | 0.55 | 0.72 |
| sieve | 0.701 | 0.715 | 0.733 | 0.593 | 1.260 | 0.755 | 0.56 | 0.93 | 0.47 | 0.79 |
| sort | 0.794 | 0.769 | 0.808 | 0.600 | 1.068 | 0.831 | 0.74 | 0.96 | 0.56 | 0.72 |
| spectral_norm | 1.060 | 1.070 | 1.101 | 0.725 | 1.678 | 1.125 | 0.63 | 0.94 | 0.43 | 0.64 |
| string_build | 0.806 | 0.816 | 0.828 | 0.787 | 1.672 | 0.741 | 0.48 | 1.09 | 0.47 | 1.06 |
| **geomean** |  |  |  |  |  |  | **0.54** | **0.85** | **0.41** | **0.64** |

Times in seconds.

### mini-coro

| benchmark | tailcall | goto | switch | jit | lua | luajit-joff | tailcall/lua | tailcall/luajit-joff | jit/lua | jit/luajit-joff |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| chameneos | 0.442 | 0.469 | 0.450 | 0.368 | 0.978 | 0.372 | 0.45 | 1.19 | 0.38 | 0.99 |
| coroutine_ring | 0.383 | 0.396 | 0.365 | 0.287 | 0.976 | 0.361 | 0.39 | 1.06 | 0.29 | 0.80 |
| sieve | 0.401 | 0.374 | 0.380 | 0.317 | 1.047 | 0.369 | 0.38 | 1.09 | 0.30 | 0.86 |
| **geomean** |  |  |  |  |  |  | **0.41** | **1.11** | **0.32** | **0.88** |

Times in seconds.

### minijs

| benchmark | tailcall | goto | switch | jit | node-jitless | qjs | tailcall/node-jitless | tailcall/qjs | jit/node-jitless | jit/qjs |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| awfy_micro | 0.757 | 0.764 | 0.831 | 0.602 | 1.539 | 1.988 | 0.49 | 0.38 | 0.39 | 0.30 |
| binary_trees | 1.038 | 1.122 | 1.179 | 0.913 | 1.735 | 4.637 | 0.60 | 0.22 | 0.53 | 0.20 |
| bitops | 0.827 | 0.819 | 0.858 | 0.678 | 1.551 | 1.383 | 0.53 | 0.60 | 0.44 | 0.49 |
| closures | 0.963 | 1.051 | 1.130 | 0.736 | 1.441 | 2.549 | 0.67 | 0.38 | 0.51 | 0.29 |
| deltablue | 0.863 | 0.907 | 1.002 | 0.721 | 1.457 | 2.569 | 0.59 | 0.34 | 0.49 | 0.28 |
| fannkuch | 0.464 | 0.435 | 0.497 | 0.391 | 1.319 | 1.481 | 0.35 | 0.31 | 0.30 | 0.26 |
| json | 0.909 | 0.916 | 0.954 | 0.885 | 1.461 | 2.180 | 0.62 | 0.42 | 0.61 | 0.41 |
| nbody | 0.415 | 0.422 | 0.443 | 0.318 | 1.818 | 1.290 | 0.23 | 0.32 | 0.17 | 0.25 |
| properties | 0.833 | 0.845 | 0.881 | 0.716 | 1.507 | 1.586 | 0.55 | 0.53 | 0.48 | 0.45 |
| raytrace | 0.497 | 0.523 | 0.552 | 0.468 | 1.398 | 2.112 | 0.36 | 0.24 | 0.33 | 0.22 |
| richards | 0.755 | 0.815 | 0.863 | 0.688 | 1.459 | 2.060 | 0.52 | 0.37 | 0.47 | 0.33 |
| sort | 0.862 | 0.844 | 0.911 | 0.819 | 1.442 | 1.743 | 0.60 | 0.49 | 0.57 | 0.47 |
| spectral_norm | 0.932 | 0.940 | 0.990 | 0.775 | 1.626 | 1.406 | 0.57 | 0.66 | 0.48 | 0.55 |
| strings | 1.283 | 1.234 | 1.300 | 1.204 | 1.483 | 2.165 | 0.87 | 0.59 | 0.81 | 0.56 |
| **geomean** |  |  |  |  |  |  | **0.52** | **0.40** | **0.44** | **0.34** |

Times in seconds.

### scheme

| benchmark | tailcall | goto | switch | jit | gauche | guile-nojit | guile | tailcall/gauche | tailcall/guile-nojit | tailcall/guile | jit/gauche | jit/guile-nojit | jit/guile |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| ack | 0.452 | 0.498 | 0.511 | 0.344 | 2.908 | 0.911 | 0.208 | 0.16 | 0.50 | 2.17 | 0.12 | 0.38 | 1.65 |
| array1 | 0.334 | 0.361 | 0.361 | 0.237 | 0.849 | 0.980 | 0.272 | 0.39 | 0.34 | 1.23 | 0.28 | 0.24 | 0.87 |
| browse | 0.017 | 0.017 | 0.018 | 0.017 | 0.081 | 0.028 | 0.019 | 0.21 | 0.61 | 0.89 | 0.21 | 0.61 | 0.89 |
| chudnovsky | 0.164 | 0.167 | 0.167 | 0.173 | 0.969 | 0.180 | 0.142 | 0.17 | 0.91 | 1.15 | 0.18 | 0.96 | 1.22 |
| conform | 0.113 | 0.114 | 0.124 | 0.100 | 0.443 | 0.174 | 0.074 | 0.26 | 0.65 | 1.53 | 0.23 | 0.57 | 1.35 |
| cpstak | 0.255 | 0.265 | 0.270 | 0.199 | 1.104 | 0.416 | 0.196 | 0.23 | 0.61 | 1.30 | 0.18 | 0.48 | 1.02 |
| ctak | 0.026 | 0.025 | 0.026 | 0.022 | 0.233 | 0.468 | 0.451 | 0.11 | 0.06 | 0.06 | 0.09 | 0.05 | 0.05 |
| deriv | 0.230 | 0.231 | 0.244 | 0.216 | 0.682 | 0.377 | 0.183 | 0.34 | 0.61 | 1.26 | 0.32 | 0.57 | 1.18 |
| destruc | 0.134 | 0.137 | 0.146 | 0.107 | 0.485 | 0.230 | 0.092 | 0.28 | 0.58 | 1.46 | 0.22 | 0.47 | 1.16 |
| diviter | 0.172 | 0.173 | 0.180 | 0.143 | 0.502 | 0.352 | 0.167 | 0.34 | 0.49 | 1.03 | 0.28 | 0.41 | 0.86 |
| divrec | 0.209 | 0.211 | 0.234 | 0.189 | 0.650 | 0.447 | 0.217 | 0.32 | 0.47 | 0.96 | 0.29 | 0.42 | 0.87 |
| earley | 0.038 | 0.039 | 0.041 | 0.036 | 0.133 | 0.063 | 0.031 | 0.29 | 0.60 | 1.23 | 0.27 | 0.57 | 1.16 |
| fft | 0.056 | 0.056 | 0.058 | 0.042 | 0.313 | 0.178 | 0.148 | 0.18 | 0.31 | 0.38 | 0.13 | 0.24 | 0.28 |
| fib | 0.485 | 0.511 | 0.597 | 0.418 | 1.412 | 1.078 | 0.281 | 0.34 | 0.45 | 1.73 | 0.30 | 0.39 | 1.49 |
| fibc | 0.037 | 0.037 | 0.037 | 0.031 | 0.341 | 0.583 | 0.558 | 0.11 | 0.06 | 0.07 | 0.09 | 0.05 | 0.06 |
| fibfp | 0.187 | 0.197 | 0.212 | 0.140 | 0.521 | 0.825 | 0.657 | 0.36 | 0.23 | 0.28 | 0.27 | 0.17 | 0.21 |
| gcbench | 0.020 | 0.020 | 0.021 | 0.017 | 0.134 | 0.028 | 0.017 | 0.15 | 0.71 | 1.18 | 0.13 | 0.61 | 1.00 |
| graphs | 0.123 | 0.126 | 0.134 | 0.115 | 0.583 | 0.203 | 0.069 | 0.21 | 0.61 | 1.78 | 0.20 | 0.57 | 1.67 |
| lattice | 0.055 | 0.056 | 0.062 | 0.051 | 0.214 | 0.100 | 0.042 | 0.26 | 0.55 | 1.31 | 0.24 | 0.51 | 1.21 |
| matrix | 0.069 | 0.070 | 0.075 | 0.063 | 0.270 | 0.115 | 0.051 | 0.26 | 0.60 | 1.35 | 0.23 | 0.55 | 1.24 |
| maze | 0.037 | 0.038 | 0.042 | 0.031 | 0.188 | 0.072 | 0.037 | 0.20 | 0.51 | 1.00 | 0.16 | 0.43 | 0.84 |
| mazefun | 0.134 | 0.143 | 0.151 | 0.114 | 0.423 | 0.279 | 0.093 | 0.32 | 0.48 | 1.44 | 0.27 | 0.41 | 1.23 |
| mbrot | 0.052 | 0.051 | 0.054 | 0.038 | 0.200 | 0.298 | 0.254 | 0.26 | 0.17 | 0.20 | 0.19 | 0.13 | 0.15 |
| mperm | 0.134 | 0.140 | 0.145 | 0.101 | 0.388 | 0.188 | 0.059 | 0.35 | 0.71 | 2.27 | 0.26 | 0.54 | 1.71 |
| nboyer | 0.668 | 0.710 | 0.772 | 0.560 | 1.166 | 0.871 | 0.320 | 0.57 | 0.77 | 2.09 | 0.48 | 0.64 | 1.75 |
| nqueens | 0.315 | 0.315 | 0.333 | 0.249 | 0.664 | 0.482 | 0.171 | 0.47 | 0.65 | 1.84 | 0.38 | 0.52 | 1.46 |
| ntakl | 0.175 | 0.189 | 0.187 | 0.113 | 0.511 | 0.310 | 0.075 | 0.34 | 0.56 | 2.33 | 0.22 | 0.36 | 1.51 |
| nucleic | 0.069 | 0.069 | 0.073 | 0.067 | 0.393 | 0.192 | 0.144 | 0.18 | 0.36 | 0.48 | 0.17 | 0.35 | 0.47 |
| paraffins | 0.040 | 0.041 | 0.042 | 0.037 | 0.133 | 0.053 | 0.031 | 0.30 | 0.75 | 1.29 | 0.28 | 0.70 | 1.19 |
| peval | 0.098 | 0.096 | 0.107 | 0.076 | 0.383 | 0.200 | 0.076 | 0.26 | 0.49 | 1.29 | 0.20 | 0.38 | 1.00 |
| pi | 0.134 | 0.135 | 0.135 | 0.139 | 2.883 | 0.104 | 0.102 | 0.05 | 1.29 | 1.31 | 0.05 | 1.34 | 1.36 |
| pnpoly | 0.078 | 0.080 | 0.083 | 0.063 | 0.215 | 0.353 | 0.220 | 0.36 | 0.22 | 0.35 | 0.29 | 0.18 | 0.29 |
| primes | 0.142 | 0.152 | 0.159 | 0.120 | 0.567 | 0.312 | 0.206 | 0.25 | 0.46 | 0.69 | 0.21 | 0.38 | 0.58 |
| puzzle | 0.099 | 0.103 | 0.110 | 0.073 | 0.286 | 0.237 | 0.084 | 0.35 | 0.42 | 1.18 | 0.26 | 0.31 | 0.87 |
| quicksort | 0.108 | 0.112 | 0.117 | 0.087 | 0.291 | 0.194 | 0.076 | 0.37 | 0.56 | 1.42 | 0.30 | 0.45 | 1.14 |
| sboyer | 0.647 | 0.707 | 0.777 | 0.524 | 1.216 | 0.945 | 0.297 | 0.53 | 0.68 | 2.18 | 0.43 | 0.55 | 1.76 |
| simplex | 0.122 | 0.122 | 0.130 | 0.094 | 0.522 | 0.240 | 0.112 | 0.23 | 0.51 | 1.09 | 0.18 | 0.39 | 0.84 |
| string | 0.019 | 0.019 | 0.019 | 0.020 | 0.123 | 0.026 | 0.026 | 0.15 | 0.73 | 0.73 | 0.16 | 0.77 | 0.77 |
| sum | 0.338 | 0.372 | 0.368 | 0.194 | 0.829 | 0.646 | 0.172 | 0.41 | 0.52 | 1.97 | 0.23 | 0.30 | 1.13 |
| sumfp | 0.106 | 0.111 | 0.114 | 0.062 | 0.277 | 0.556 | 0.527 | 0.38 | 0.19 | 0.20 | 0.22 | 0.11 | 0.12 |
| tak | 0.451 | 0.468 | 0.562 | 0.367 | 1.244 | 0.962 | 0.235 | 0.36 | 0.47 | 1.92 | 0.30 | 0.38 | 1.56 |
| takl | 0.085 | 0.083 | 0.093 | 0.059 | 0.303 | 0.161 | 0.043 | 0.28 | 0.53 | 1.98 | 0.19 | 0.37 | 1.37 |
| triangl | 0.166 | 0.179 | 0.193 | 0.121 | 0.415 | 0.423 | 0.132 | 0.40 | 0.39 | 1.26 | 0.29 | 0.29 | 0.92 |
| **geomean** |  |  |  |  |  |  |  | **0.26** | **0.46** | **0.97** | **0.22** | **0.38** | **0.80** |

Times in seconds.

### lox

| benchmark | tailcall | goto | switch | clox | tailcall/clox |
|---|---:|---:|---:|---:|---:|
| binary_trees | 0.661 | 0.692 | 0.677 | 2.169 | 0.30 |
| equality | 0.973 | 0.973 | 1.292 | 3.167 | 0.31 |
| fib | 0.390 | 0.416 | 0.409 | 0.959 | 0.41 |
| instantiation | 0.352 | 0.359 | 0.358 | 0.522 | 0.67 |
| invocation | 0.137 | 0.138 | 0.138 | 0.269 | 0.51 |
| method_call | 0.079 | 0.083 | 0.081 | 0.170 | 0.46 |
| properties | 0.181 | 0.181 | 0.181 | 0.405 | 0.45 |
| trees | 1.270 | 1.300 | 1.327 | 2.287 | 0.56 |
| zoo | 0.149 | 0.155 | 0.148 | 0.305 | 0.49 |
| zoo_batch | 0.906 | 0.930 | 0.884 | 1.878 | 0.48 |
| **geomean** |  |  |  |  | **0.45** |

Times in seconds.

### regex

| benchmark | tailcall | goto | switch | pcre2 | python-re | tailcall/pcre2 | tailcall/python-re |
|---|---:|---:|---:|---:|---:|---:|---:|
| alt-heavy | 23.3 | 22.9 | 25.4 | 48.4 | 31.1 | 0.48 | 0.75 |
| capture | 16.7 | 17.8 | 19.5 | 49.9 | 107.1 | 0.33 | 0.16 |
| class | 61.8 | 59.4 | 60.3 | 213.7 | 148.8 | 0.29 | 0.42 |
| dot-star | 10.0 | 9.6 | 10.3 | 20.8 | 7.8 | 0.48 | 1.27 |
| literal | 3.1 | 3.1 | 3.1 | 2.5 | 2.5 | 1.26 | 1.23 |
| literal-alt | 8.6 | 8.7 | 8.7 | 17.6 | 36.5 | 0.49 | 0.24 |
| patho-alt | 44.0 | 46.3 | 62.0 | 150.7 | 125.0 | 0.29 | 0.35 |
| pathological | 0.0 | 0.0 | 0.0 | 0.0 | 432.0 | — | — |
| words | 19.2 | 21.1 | 22.4 | 53.3 | 150.7 | 0.36 | 0.13 |
| **geomean** |  |  |  |  |  | **0.44** | **0.41** |

Times in milliseconds.

### wasm

| benchmark | tailcall | goto | switch | wasm3 | tailcall/wasm3 |
|---|---:|---:|---:|---:|---:|
| binarytrees | 1.399 | 1.508 | 1.861 | 1.408 | 0.99 |
| coremark | 1.859 | 1.997 | 2.407 | 1.984 | 0.94 |
| fannkuchredux | 1.447 | 1.511 | 1.895 | 1.638 | 0.88 |
| fasta | 1.150 | 1.261 | 1.316 | 1.136 | 1.01 |
| mandelbrot | 1.517 | 1.568 | 1.990 | 1.957 | 0.78 |
| nbody | 1.906 | 1.930 | 2.295 | 1.959 | 0.97 |
| spectralnorm | 1.579 | 1.603 | 2.062 | 1.880 | 0.84 |
| **geomean** |  |  |  |  | **0.91** |

Times in seconds.

### wasm-simd

| benchmark | tailcall | goto | switch | mvp | tailcall/mvp |
|---|---:|---:|---:|---:|---:|
| fannkuchredux_simd | 1.486 | 1.548 | 1.921 | 1.447 | 1.03 |
| nbody_simd | 1.783 | 1.811 | 2.171 | 1.906 | 0.94 |
| spectralnorm_simd | 1.612 | 1.670 | 2.117 | 1.579 | 1.02 |
| **geomean** |  |  |  |  | **0.99** |

Times in seconds.

### stack

| benchmark | tailcall | goto | switch |
|---|---:|---:|---:|
| fib | 0.052 | 0.058 | 0.072 |
| loop | 0.749 | 0.711 | 0.790 |
| nbody | 0.155 | 0.163 | 0.214 |
| sieve | 0.091 | 0.090 | 0.103 |
| strings | 0.152 | 0.141 | 0.160 |
| **geomean** |  |  |  |

Times in seconds.

### python

| benchmark | tailcall | goto | switch | cpython | micropython | tailcall/cpython | tailcall/micropython |
|---|---:|---:|---:|---:|---:|---:|---:|
| chaos | 0.326 | 0.334 | 0.370 | 0.700 | 1.855 | 0.47 | 0.18 |
| deltablue | 0.578 | 0.604 | 0.650 | 0.977 | 4.788 | 0.59 | 0.12 |
| fannkuch | 0.469 | 0.491 | 0.541 | 0.909 | 1.405 | 0.52 | 0.33 |
| float | 0.441 | 0.433 | 0.478 | 0.863 | 2.138 | 0.51 | 0.21 |
| go | 0.372 | 0.375 | 0.412 | 0.834 | 1.877 | 0.45 | 0.20 |
| hexiom | 0.510 | 0.546 | 0.625 | 0.703 | 3.034 | 0.73 | 0.17 |
| nbody | 0.359 | 0.384 | 0.511 | 0.958 | 3.411 | 0.37 | 0.11 |
| nqueens | 0.725 | 0.750 | 0.822 | 0.798 | 1.654 | 0.91 | 0.44 |
| raytrace | 0.476 | 0.474 | 0.517 | 0.878 | 3.426 | 0.54 | 0.14 |
| richards | 0.301 | 0.315 | 0.351 | 0.825 | 3.148 | 0.36 | 0.10 |
| spectral_norm | 0.310 | 0.305 | 0.332 | 0.711 | 1.370 | 0.44 | 0.23 |
| **geomean** |  |  |  |  |  | **0.52** | **0.18** |

Times in seconds.

### prolog

| benchmark | tailcall | goto | switch | jit | swipl | trealla | tailcall/swipl | tailcall/trealla | jit/swipl | jit/trealla |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| boyer | 1.076 | 1.073 | 1.145 | 1.088 | 0.981 | 4.774 | 1.10 | 0.23 | 1.11 | 0.23 |
| browse | 0.804 | 0.803 | 0.886 | 0.850 | 0.984 | 6.200 | 0.82 | 0.13 | 0.86 | 0.14 |
| chat_parser | 0.912 | 0.881 | 0.927 | 0.895 | 0.992 | 3.580 | 0.92 | 0.25 | 0.90 | 0.25 |
| crypt | 0.547 | 0.531 | 0.543 | 0.521 | 0.972 | 3.652 | 0.56 | 0.15 | 0.54 | 0.14 |
| derive | 0.674 | 0.687 | 0.718 | 0.676 | 0.983 | 2.724 | 0.69 | 0.25 | 0.69 | 0.25 |
| divide10 | 0.772 | 0.781 | 0.809 | 0.771 | 0.985 | 2.691 | 0.78 | 0.29 | 0.78 | 0.29 |
| eval | 0.630 | 0.628 | 0.653 | 0.597 | 0.932 | 2.496 | 0.68 | 0.25 | 0.64 | 0.24 |
| fast_mu | 0.781 | 0.794 | 0.861 | 0.820 | 0.987 | 3.783 | 0.79 | 0.21 | 0.83 | 0.22 |
| flatten | 0.766 | 0.781 | 0.864 | 0.729 | 0.956 | 5.145 | 0.80 | 0.15 | 0.76 | 0.14 |
| log10 | 0.586 | 0.593 | 0.639 | 0.586 | 0.988 | 3.480 | 0.59 | 0.17 | 0.59 | 0.17 |
| meta_qsort | 0.580 | 0.611 | 0.668 | 0.602 | 0.962 | 5.028 | 0.60 | 0.12 | 0.63 | 0.12 |
| mu | 0.584 | 0.603 | 0.680 | 0.607 | 0.950 | 4.093 | 0.61 | 0.14 | 0.64 | 0.15 |
| nand | 0.724 | 0.724 | 0.828 | 0.747 | 0.932 | 3.555 | 0.78 | 0.20 | 0.80 | 0.21 |
| nreverse | 0.778 | 0.807 | 0.779 | 0.694 | 0.982 | 7.846 | 0.79 | 0.10 | 0.71 | 0.09 |
| ops8 | 0.633 | 0.647 | 0.700 | 0.657 | 0.984 | 3.169 | 0.64 | 0.20 | 0.67 | 0.21 |
| perfect | 0.813 | 0.821 | 0.874 | 0.822 | 0.961 | 4.769 | 0.85 | 0.17 | 0.86 | 0.17 |
| poly_10 | 0.741 | 0.730 | 0.807 | 0.755 | 0.969 | 2.484 | 0.76 | 0.30 | 0.78 | 0.30 |
| prover | 0.553 | 0.563 | 0.640 | 0.597 | 0.975 | 3.521 | 0.57 | 0.16 | 0.61 | 0.17 |
| qsort | 0.607 | 0.637 | 0.669 | 0.594 | 0.982 | 5.938 | 0.62 | 0.10 | 0.60 | 0.10 |
| queens_8 | 0.988 | 0.986 | 0.998 | 1.126 | 0.964 | 5.231 | 1.02 | 0.19 | 1.17 | 0.22 |
| query | 1.026 | 0.939 | 1.014 | 1.027 | 0.980 | 4.554 | 1.05 | 0.23 | 1.05 | 0.23 |
| reducer | 0.946 | 0.947 | 1.050 | 0.934 | 0.982 | 4.511 | 0.96 | 0.21 | 0.95 | 0.21 |
| sendmore | 1.030 | 1.063 | 1.099 | 1.131 | 0.945 | 3.331 | 1.09 | 0.31 | 1.20 | 0.34 |
| serialise | 0.667 | 0.669 | 0.758 | 0.688 | 0.965 | 4.965 | 0.69 | 0.13 | 0.71 | 0.14 |
| sieve | 1.818 | 1.834 | 1.844 | 37.818† | 0.928 | 2.315 | 1.96 | 0.79 | 40.75 | 16.34 |
| simple_analyzer | 0.821 | 0.806 | 0.889 | 0.742 | 0.968 | 4.813 | 0.85 | 0.17 | 0.77 | 0.15 |
| tak | 0.546 | 0.547 | 0.575 | 0.555 | 0.857 | 3.550 | 0.64 | 0.15 | 0.65 | 0.16 |
| times10 | 0.640 | 0.650 | 0.683 | 0.635 | 0.971 | 2.837 | 0.66 | 0.23 | 0.65 | 0.22 |
| unify | 0.851 | 0.870 | 0.978 | 0.833 | 0.929 | 6.233 | 0.92 | 0.14 | 0.90 | 0.13 |
| zebra | 0.799 | 0.801 | 0.837 | 0.809 | 0.967 | 3.490 | 0.83 | 0.23 | 0.84 | 0.23 |
| **geomean** |  |  |  |  |  |  | **0.79** | **0.19** | **0.88** | **0.21** |

Times in seconds.

† jit on sieve: every assert/retract makes code objects that are never freed, and the JIT compiles each one as it gets hot.

*2026-11-04:* after a second optimization round (examples/prolog/FRICTION.md,
"Optimizations, second round"; a vm-verifier and a std.gc change among
them), the tail-call build takes 0.63x SWI-Prolog's time and 0.15x
Trealla's by geometric mean, against 0.83x and 0.20x for the build above
in the same (loaded) run; tak 0.41x, queens_8 0.47x, sieve 0.90x, boyer
1.05x of SWI-Prolog (`bench/prolog/README.md`). The other columns of this
table were not rerun.
