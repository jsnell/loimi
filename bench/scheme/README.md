# examples/scheme vs Gauche, Guile and chibi-scheme on r7rs-benchmarks

**Provenance.** The programs are the public
[r7rs-benchmarks](https://github.com/ecraven/r7rs-benchmarks) suite
(Larceny's R7RS benchmarks by W. D. Clinger et al., drawn from the Gabriel,
Kernighan–Van Wyk and Gambit benchmark sets, maintained by ecraven), pinned
to a commit in [`MANIFEST`](MANIFEST). They are downloaded by `fetch.sh` and
are never committed: the repository has no license file (see `MANIFEST`).
**The program text is used unmodified.** Like the suite's own `bench`
script, `compare.py` builds each runnable program by concatenating

    <impl>-prelude.scm  src/NAME.scm  src/common.scm  <impl>-postlude.scm  src/common-postlude.scm

(a missing prelude or postlude is skipped, as the script does). Each
reference gets the suite's own files and is run with the suite's command
line; our Scheme gets [`Loimi-prelude.scm`](Loimi-prelude.scm) (empty:
our Scheme accepts and ignores `(import ...)`, and the `(scheme
base/read/write/time)` procedures the suite uses are builtins) and
[`Loimi-postlude.scm`](Loimi-postlude.scm) (defines
`this-scheme-implementation-name`), which is how every implementation in
the suite is set up. The programs read their parameters from standard
input, from the suite's `inputs/NAME.input`.

## References

All are built from source by `install.sh` into `bench/scheme/install/`
(gitignored), from the git commit or the sha256-checked release tarball
pinned in `MANIFEST`, with each project's stock compiler flags (Gauche
`-O2`; Guile `-O2 -flto`; chibi `-O3`, with computed-goto dispatch).

| column | what | version | suite files | command (as in the suite's `bench`) |
|---|---|---|---|---|
| gauche | Gauche, a compiler to a stack VM, written in C | 0.9.15 (latest release) | `Gauche-postlude.scm` (no prelude) | `gosh -r7 -b prog.scm` |
| guile-nojit | Guile 3, compiled ahead of time to bytecode; VM only (`GUILE_JIT_THRESHOLD=-1`) | 3.0.11 (latest release) | `Guile-prelude.scm` (no postlude) | `guild compile -O3 prog.scm` (untimed), then `GC_INITIAL_HEAP_SIZE=100000000 guile prog.scm` |
| guile | the same with Guile's template JIT on (its default) | 3.0.11 | same | same |
| chibi | chibi-scheme, an AST-to-bytecode VM interpreter | 0.12 (git commit) | `Chibi-prelude.scm`, `Chibi-postlude.scm` | `chibi-scheme -m scheme.load -e '(load "prog.scm")'` |

Build notes:

- **Gauche** bundles its own Boehm GC (built with the compiler's atomics,
  so no libatomic_ops) and needs nothing from the system; TLS, dbm, zlib
  and iconv support are configured out. The suite's
  `Gauche-postlude.scm` does `(import (slib))`, so `install.sh` also
  unpacks [SLIB](https://people.csail.mit.edu/jaffer/SLIB.html) 3c3
  (portable Scheme source) and points Gauche at it (`--with-slib`).
  Loading SLIB is most of Gauche's 60 ms startup here. Build: 4 minutes.
- **Guile** needs bdwgc (8.2.12, built with GCC atomics instead of
  libatomic_ops) and libunistring (1.4.2), both built from source by
  `install.sh`; GMP (6.3.0) and libffi (3.4.8) come from the system
  (Debian packages with headers). The release tarball only ships
  precompiled `eval`, `psyntax` and `boot-9`, so the build bootstraps its
  compiler: three passes over 343 modules. Build: 31 minutes of wall time
  at `make -j2` on an i7-6700.
- Guile's `.go` files go to its compile cache under
  `$TMPDIR/loimi-scheme-bench/guile-cache` (`XDG_CACHE_HOME`), where `guile
  prog.scm` picks them up, exactly as the suite relies on with the default
  cache. `compare.py` only rewrites a program file when its text changes,
  so the cached code is never stale.

```sh
bench/scheme/fetch.sh                  # once: r7rs-benchmarks at the pinned commit
bench/scheme/install.sh                # once: chibi, gauche, guile (about 40 min)
bench/scheme/install.sh gauche         # or just some of them
bench/scheme/compare.py                # all benchmarks, all installed references, 3 runs, wall time
bench/scheme/compare.py --inner fib tak            # the benchmarks' own "Elapsed time"
bench/scheme/compare.py --refs gauche,guile-nojit  # a subset of the references ('' = none)
bench/scheme/compare.py --check --count 1          # quick correctness pass, failures in full
bench/scheme/compare.py --dispatch tailcall goto switch
bench/scheme/compare.py --align 128                # build ours with --align-functions=128
bench/scheme/compare.py --refs '' --dispatch --exe old new -n 5   # A/B of two builds
```

Runs are interleaved: each repetition runs every implementation once, so
drifts in machine load affect all columns alike. With `--exe`, the last
lines give the geometric mean of each extra binary's times over the
first's. For A/B comparisons build both binaries with the same
`--align-functions` (128 here): code placement alone moves single
benchmarks by up to ±9% on this machine (tak), and so does the placement
of the bytecode in the heap (fib, with identical bytecode, after a
prelude change).

A run passes when the benchmark's own result check passes (no `ERROR:`
line) and its output, minus the timing lines, is identical to that of the
first reference that passed (chibi's when it is installed). The `/ref`
columns are ours divided by that reference (below 1: ours is faster), and
the last line is their geometric mean over the programs where both pass.

## Inputs

The suite's inputs are sized for native-code compilers (minutes per
program on chibi), so `compare.py` (`INPUTS`) replaces the leading data of
each input file: always the iteration count, and for some programs the
problem size with its expected result. Smaller sizes were taken from the
"old inputs" that the suite's input files list where there are some (tak,
cpstak, ctak, takl); the others were checked by running chibi on them. All
expected results are verified by the benchmark itself under chibi. Sizes
were chosen so that chibi takes about 1–3 s (graphs, at its smallest
interesting size, takes 4.5 s). The two bignum programs, pi and
chudnovsky, run the suite's own inputs (ours takes 0.2–0.3 s, Gauche
1.2–3.7 s), and chibi is not run on them (`SKIP` in compare.py): its
bignums take 21 s for one of pi's 100 iterations, so the expected
output is Gauche's.

| benchmark | count | parameters → expected | | benchmark | count | parameters → expected |
|---|---|---|---|---|---|---|
| browse | 3 | (suite) | | ack | 5 | 3 9 → 4093 |
| deriv | 250000 | (suite) | | array1 | 20 | (suite) |
| destruc | 80 | (suite) | | string | 7 | (suite) |
| diviter | 30000 | (suite) | | conform | 5 | (suite) |
| divrec | 30000 | (suite) | | earley | 3 | 10 → 4862 |
| puzzle | 10 | (suite) | | graphs | 1 | 6 → 10275 |
| triangl | 1 | (suite) | | lattice | 1000 | 33 → 10 |
| tak | 1 | 32 16 8 → 9 | | matrix | 20 | (suite) |
| takl | 20 | lists of 18 12 6 → 7 | | maze | 80 | (suite) |
| ntakl | 40 | lists of 18 12 6 → 7 | | mazefun | 150 | (suite) |
| cpstak | 150 | 18 12 6 → 7 | | nqueens | 25 | 10 → 724 |
| ctak | 7 | 18 12 6 → 7 | | paraffins | 25 | 17 → 24894 |
| fib | 20 | 30 → 832040 | | peval | 20 | (suite) |
| fibc | 2 | 25 → 75025 | | primes | 400 | (suite) |
| fibfp | 5 | 30.0 → 832040.0 | | quicksort | 20 | (suite) |
| sum | 5000 | (suite) | | nboyer | 1 | 3 → 5375678 |
| sumfp | 10 | (suite) | | sboyer | 1 | 3 → 5375678 |
| fft | 3 | (suite) | | gcbench | 1 | 13 |
| mbrot | 10 | (suite) | | mperm | — | M=1, N=9, K=2, L=1 |
| nucleic | 1 | (suite) | | pi | 100 | (suite: 50 to 500 digits) |
| pnpoly | 10000 | (suite) | | chudnovsky | 1000 | (suite: 50 to 500 digits) |
| simplex | 15000 | (suite) | | | | |

## Results

(See [`docs/results.md`](../../docs/results.md) for current numbers,
all dispatch modes and the JIT, measured 2026-11-03.)

`compare.py -n 3 --align 128` and the same with `--inner`, minimum of 3
interleaved runs, seconds; ours is `--dispatch=tailcall` at commit
84da73f. Intel i7-6700, gcc 14 / clang 19, 2026-11-02, one benchmark at a
time on a machine shared with other agents' work (load average about
3-5 of 8 cores). Every program passes on every implementation with
identical output. chibi was not rebuilt for this run; its column is from
the previous one.

| geometric mean of ours / reference | gauche | guile-nojit | guile (JIT) | chibi |
|---|---|---|---|---|
| wall (whole process) | **0.29** | **0.47** | **1.02** | |
| inner (timed loop only) | **0.36** | **0.50** | **1.18** | |
| wall, before this phase | 0.39 | 0.62 | 1.39 | 0.10 |
| inner, before this phase | 0.49 | 0.67 | 1.57 | 0.12 |

That is, ours is about 3× faster than Gauche, 2× faster than Guile's
bytecode VM, and level with Guile with its JIT in wall time (1.2× slower
in the timed loops). Startup (an almost empty program, best of 5): ours
1 ms, Guile 7 ms, Gauche 42 ms (60 ms with the postlude's SLIB), chibi
313 ms; "inner" removes it. The inputs were sized for chibi, so some
times are short (under 30 ms for Guile on browse, string, earley,
paraffins, maze, gcbench), and those ratios are coarse.

### Wall time (whole process)

| benchmark | ours | gauche | guile-nojit | guile | /gauche | /guile-nojit | /guile |
|---|---|---|---|---|---|---|---|
| browse | 0.019 | 0.083 | 0.028 | 0.019 | 0.23 | 0.67 | 1.01 |
| deriv | 0.258 | 0.714 | 0.377 | 0.189 | 0.36 | 0.68 | 1.36 |
| destruc | 0.147 | 0.484 | 0.232 | 0.092 | 0.30 | 0.63 | 1.60 |
| diviter | 0.180 | 0.515 | 0.355 | 0.167 | 0.35 | 0.50 | 1.07 |
| divrec | 0.237 | 0.665 | 0.450 | 0.218 | 0.36 | 0.53 | 1.09 |
| puzzle | 0.102 | 0.292 | 0.239 | 0.086 | 0.35 | 0.43 | 1.19 |
| triangl | 0.173 | 0.418 | 0.424 | 0.132 | 0.41 | 0.41 | 1.31 |
| tak | 0.488 | 1.263 | 0.973 | 0.237 | 0.39 | 0.50 | 2.06 |
| takl | 0.086 | 0.302 | 0.161 | 0.043 | 0.29 | 0.54 | 1.99 |
| ntakl | 0.193 | 0.530 | 0.319 | 0.077 | 0.36 | 0.61 | 2.50 |
| cpstak | 0.268 | 1.531 | 0.438 | 0.203 | 0.18 | 0.61 | 1.32 |
| ctak | 0.028 | 0.321 | 0.575 | 0.561 | 0.09 | 0.05 | 0.05 |
| fib | 0.597 | 1.454 | 1.100 | 0.290 | 0.41 | 0.54 | 2.06 |
| fibc | 0.038 | 0.381 | 0.619 | 0.569 | 0.10 | 0.06 | 0.07 |
| fibfp | 0.214 | 0.520 | 0.848 | 0.675 | 0.41 | 0.25 | 0.32 |
| sum | 0.364 | 0.832 | 0.645 | 0.171 | 0.44 | 0.56 | 2.12 |
| sumfp | 0.109 | 0.281 | 0.561 | 0.529 | 0.39 | 0.19 | 0.21 |
| fft | 0.056 | 0.308 | 0.177 | 0.147 | 0.18 | 0.31 | 0.38 |
| mbrot | 0.053 | 0.205 | 0.295 | 0.253 | 0.26 | 0.18 | 0.21 |
| nucleic | 0.071 | 0.391 | 0.192 | 0.143 | 0.18 | 0.37 | 0.50 |
| pnpoly | 0.082 | 0.214 | 0.352 | 0.223 | 0.38 | 0.23 | 0.37 |
| simplex | 0.121 | 0.525 | 0.240 | 0.113 | 0.23 | 0.50 | 1.07 |
| ack | 0.492 | 2.756 | 0.915 | 0.212 | 0.18 | 0.54 | 2.32 |
| array1 | 0.353 | 0.851 | 0.994 | 0.272 | 0.41 | 0.35 | 1.30 |
| string | 0.018 | 0.121 | 0.027 | 0.026 | 0.15 | 0.69 | 0.72 |
| conform | 0.121 | 0.446 | 0.176 | 0.074 | 0.27 | 0.69 | 1.62 |
| earley | 0.041 | 0.132 | 0.063 | 0.031 | 0.31 | 0.66 | 1.32 |
| graphs | 0.146 | 0.577 | 0.203 | 0.069 | 0.25 | 0.72 | 2.10 |
| lattice | 0.057 | 0.210 | 0.099 | 0.041 | 0.27 | 0.58 | 1.38 |
| matrix | 0.075 | 0.269 | 0.115 | 0.051 | 0.28 | 0.65 | 1.46 |
| maze | 0.039 | 0.192 | 0.073 | 0.038 | 0.20 | 0.53 | 1.02 |
| mazefun | 0.147 | 0.449 | 0.289 | 0.093 | 0.33 | 0.51 | 1.59 |
| nqueens | 0.344 | 0.722 | 0.493 | 0.172 | 0.48 | 0.70 | 2.00 |
| paraffins | 0.047 | 0.148 | 0.053 | 0.032 | 0.32 | 0.89 | 1.48 |
| peval | 0.106 | 0.410 | 0.203 | 0.077 | 0.26 | 0.52 | 1.37 |
| primes | 0.165 | 0.611 | 0.318 | 0.211 | 0.27 | 0.52 | 0.78 |
| quicksort | 0.115 | 0.296 | 0.196 | 0.077 | 0.39 | 0.59 | 1.50 |
| nboyer | 0.729 | 1.235 | 0.885 | 0.318 | 0.59 | 0.82 | 2.30 |
| sboyer | 0.717 | 1.220 | 0.944 | 0.299 | 0.59 | 0.76 | 2.40 |
| gcbench | 0.022 | 0.134 | 0.027 | 0.017 | 0.16 | 0.79 | 1.28 |
| mperm | 0.183 | 0.388 | 0.189 | 0.059 | 0.47 | 0.97 | 3.09 |

### Inner time (the benchmark's own "Elapsed time")

| benchmark | ours | gauche | guile-nojit | guile | /gauche | /guile-nojit | /guile |
|---|---|---|---|---|---|---|---|
| browse | 0.016 | 0.022 | 0.016 | 0.007 | 0.75 | 1.03 | 2.41 |
| deriv | 0.258 | 0.632 | 0.370 | 0.177 | 0.41 | 0.70 | 1.46 |
| destruc | 0.142 | 0.433 | 0.224 | 0.082 | 0.33 | 0.63 | 1.72 |
| diviter | 0.193 | 0.465 | 0.342 | 0.160 | 0.41 | 0.56 | 1.20 |
| divrec | 0.247 | 0.630 | 0.471 | 0.223 | 0.39 | 0.52 | 1.11 |
| puzzle | 0.111 | 0.234 | 0.241 | 0.087 | 0.47 | 0.46 | 1.27 |
| triangl | 0.173 | 0.385 | 0.425 | 0.123 | 0.45 | 0.41 | 1.41 |
| tak | 0.529 | 1.337 | 1.019 | 0.249 | 0.40 | 0.52 | 2.12 |
| takl | 0.085 | 0.256 | 0.155 | 0.034 | 0.33 | 0.55 | 2.54 |
| ntakl | 0.197 | 0.472 | 0.311 | 0.068 | 0.42 | 0.63 | 2.91 |
| cpstak | 0.273 | 1.533 | 0.428 | 0.202 | 0.18 | 0.64 | 1.35 |
| ctak | 0.026 | 0.212 | 0.498 | 0.499 | 0.12 | 0.05 | 0.05 |
| fib | 0.578 | 1.400 | 1.101 | 0.280 | 0.41 | 0.53 | 2.07 |
| fibc | 0.036 | 0.316 | 0.596 | 0.549 | 0.12 | 0.06 | 0.07 |
| fibfp | 0.213 | 0.464 | 0.831 | 0.663 | 0.46 | 0.26 | 0.32 |
| sum | 0.355 | 0.757 | 0.628 | 0.159 | 0.47 | 0.57 | 2.23 |
| sumfp | 0.107 | 0.217 | 0.543 | 0.516 | 0.49 | 0.20 | 0.21 |
| fft | 0.054 | 0.243 | 0.165 | 0.136 | 0.22 | 0.33 | 0.40 |
| mbrot | 0.052 | 0.143 | 0.285 | 0.246 | 0.37 | 0.18 | 0.21 |
| nucleic | 0.065 | 0.333 | 0.183 | 0.133 | 0.19 | 0.35 | 0.49 |
| pnpoly | 0.083 | 0.163 | 0.352 | 0.215 | 0.51 | 0.23 | 0.38 |
| simplex | 0.121 | 0.481 | 0.234 | 0.104 | 0.25 | 0.52 | 1.16 |
| ack | 0.504 | 3.117 | 0.934 | 0.207 | 0.16 | 0.54 | 2.44 |
| array1 | 0.354 | 0.798 | 1.009 | 0.266 | 0.44 | 0.35 | 1.33 |
| string | 0.018 | 0.062 | 0.016 | 0.016 | 0.29 | 1.13 | 1.10 |
| conform | 0.121 | 0.383 | 0.169 | 0.064 | 0.32 | 0.72 | 1.89 |
| earley | 0.039 | 0.070 | 0.053 | 0.021 | 0.56 | 0.74 | 1.82 |
| graphs | 0.146 | 0.542 | 0.195 | 0.060 | 0.27 | 0.75 | 2.44 |
| lattice | 0.057 | 0.154 | 0.087 | 0.032 | 0.37 | 0.65 | 1.76 |
| matrix | 0.074 | 0.208 | 0.105 | 0.040 | 0.35 | 0.70 | 1.82 |
| maze | 0.036 | 0.125 | 0.062 | 0.026 | 0.29 | 0.58 | 1.36 |
| mazefun | 0.148 | 0.368 | 0.275 | 0.082 | 0.40 | 0.54 | 1.80 |
| nqueens | 0.338 | 0.615 | 0.482 | 0.162 | 0.55 | 0.70 | 2.08 |
| paraffins | 0.044 | 0.089 | 0.042 | 0.021 | 0.50 | 1.04 | 2.12 |
| peval | 0.101 | 0.320 | 0.195 | 0.065 | 0.32 | 0.52 | 1.55 |
| primes | 0.162 | 0.537 | 0.305 | 0.197 | 0.30 | 0.53 | 0.82 |
| quicksort | 0.112 | 0.232 | 0.180 | 0.062 | 0.48 | 0.62 | 1.81 |
| nboyer | 0.731 | 1.141 | 0.860 | 0.308 | 0.64 | 0.85 | 2.37 |
| sboyer | 0.710 | 1.158 | 0.948 | 0.288 | 0.61 | 0.75 | 2.46 |
| gcbench | 0.020 | 0.075 | 0.017 | 0.007 | 0.26 | 1.12 | 2.65 |
| mperm | 0.115 | 0.196 | 0.118 | 0.032 | 0.59 | 0.98 | 3.58 |

### Reading the results

The explanations of the other implementations' behaviour are educated
guesses backed by the experiments noted, not profiles of them.

**Gauche.** Ours is faster on every program, in wall time (0.09-0.59)
and in inner time (0.12-0.75). The closest are nboyer and sboyer (0.6:
list traversal limited by memory latency, where Gauche's 16-byte pairs
help it) and browse and earley in inner time.

**Guile without the JIT** is the closest like-for-like reference: a
bytecode VM in C, like ours, but behind an optimizing compiler
(`guild compile -O3`). Before this phase ours was slower on 13 programs;
the compiler passes now in `examples/scheme/compile.lm` (see the
changelog) closed most of that, and ours is faster on all 41 in wall time
and on 37 in inner time. The four left are within 16 ms of Guile and all
allocation-bound: string (1.13 inner), gcbench (1.12), paraffins (1.04)
and browse (1.03); mperm is 0.98. There Guile runs with the suite's
100 MB initial heap (`GC_INITIAL_HEAP_SIZE`) and barely collects, and
its substrings share storage (string). Ours collects every 4× the live
data, starting from 2 MiB.

For reference, the inner times before this phase, and Guile's at each
optimization level (still without the JIT):

| benchmark | ours before | ours now | guile-nojit -O0 | -O1 | -O2 | -O3 |
|---|---|---|---|---|---|---|
| earley | 0.097 | 0.039 | 0.251 | 0.092 | 0.053 | 0.052 |
| peval | 0.243 | 0.101 | 0.719 | 0.270 | 0.193 | 0.189 |
| mperm | 0.157 | 0.115 | 0.735 | 0.216 | 0.116 | 0.119 |
| nboyer | 1.076 | 0.731 | 4.822 | 1.093 | 0.882 | 0.865 |
| conform | 0.201 | 0.121 | 0.688 | 0.442 | 0.216 | 0.167 |
| graphs | 0.238 | 0.146 | 0.829 | 0.309 | 0.246 | 0.194 |
| maze | 0.084 | 0.036 | 0.306 | 0.157 | 0.064 | 0.062 |
| gcbench | 0.031 | 0.020 | 0.088 | 0.025 | 0.018 | 0.018 |
| string | 0.040 | 0.018 | 0.016 | 0.015 | 0.015 | 0.015 |
| tak | 0.602 | 0.529 | 4.464 | 1.257 | 0.949 | 0.943 |
| fib | 0.626 | 0.578 | 4.864 | 1.588 | 1.094 | 1.091 |

**Guile with the JIT.** Even in wall time (1.02 overall) ours is faster
on 10 programs and within 10% on 5 more. Ours wins where Guile's value
representation or continuations cost more than its JIT saves: flonums
(fibfp 0.32, sumfp 0.21, mbrot 0.21, fft 0.38, pnpoly 0.37, nucleic 0.50:
Guile heap-allocates flonums it cannot unbox; ours are NaN-boxed),
continuations (ctak 0.05, fibc 0.07: Guile copies the stack for `call/cc`;
ours are escape-only, not a like-for-like comparison), primes and string.
Guile's JIT is 2-3.6× faster on the call-bound programs (tak, fib, ack,
takl, ntakl, sum) and on the list-heavy nboyer, sboyer, graphs, nqueens
and mperm: its template JIT removes dispatch and operand decoding, which
a bytecode interpreter cannot. Ours is at about 30-40 x86 instructions
per call plus a few per op (see `examples/scheme/README.md`).

**Remaining losses and why** (vs Guile without the JIT, inner time):

- string (1.13): `string-append`/`substring` of strings of up to 0.5 MB,
  where 80% of our time is in libc copying and zeroing new strings;
  Guile's substrings share the buffer (copy on write).
- gcbench (1.12), paraffins (1.04), browse (1.03): 16-20 ms programs
  dominated by allocation and collection, where Guile's 100 MB initial
  heap means it hardly collects at all.
- nboyer, sboyer, mperm (0.75-0.98, the narrowest wins): memory-bound
  list walking (`CAR` alone is 12-15% of nboyer's time) and collection of
  a growing heap (mperm keeps 65 MB live). Our pairs are 32 bytes
  (16-byte header), Guile's 16.

**Caveats** that favour ours: escape-only continuations and byte
strings. (Until 2026-11-03 fixnum overflow produced a flonum; none of the
41 programs overflows, and exact bignums, added then, changed only the
cold paths of the arithmetic ops; see "Bignums" below.) One that favours Guile: the suite's 100 MB initial
heap.

**chibi-scheme** is about 8–10× slower than ours on every program
(heap-allocated flonums, slower calls, full continuations, and a collector
that struggles with large live heaps: gcbench at depth 12 takes 0.1 s,
at 14 4.9 s, at 15 8.5 s, at 16 over 120 s; ours: 0.18 s at 15, 2.3 s at
18), which makes it a weak reference; it is kept because it is the first
reference compare.py checks outputs against.

**Full-size inputs.** earley at the suite's own size (15, 2674440 parses)
runs in 10.6 s and 1.7 GB under `ulimit -v 2500000` since the 16-byte
headers (it ran out of memory before: 1.9 GB of live data with 48-byte
pairs).

## Changelog

Second phase (compiler passes and runtime). Each step was measured with
interleaved A/B runs of aligned builds (`--exe old new -n 5`, wall time,
seconds); changes under about 5% are not claimed. The first steps were
measured while Guile was being built on the same machine, so their
absolute times are higher than in the tables above.

1. **Contification** (5c3e30e). A `letrec` lambda whose every use is a
   tail call from the letrec body or from another contified lambda of the
   same letrec (every named-let and `do` loop, mutually tail-recursive
   local procedures) is compiled inline in the enclosing procedure:
   parameters become registers, calls become moves and jumps, and its
   returns go to the letrec's continuation (return, a register and a join
   jump, or nothing). No closure is allocated per loop entry, and the
   loop's free variables are the procedure's registers instead of
   `GETFREE`s; an assigned variable used only by contified code is no
   longer boxed. earley 0.191 → 0.099, matrix 0.159 → 0.120,
   graphs 0.320 → 0.234, mperm 0.345 → 0.262, conform 0.316 → 0.247,
   array1 0.569 → 0.463, deriv 0.536 → 0.438.
2. **Direct builtin calls and type-predicate ops** (f141bd8). A call of a
   builtin the program never redefines is `PRIMCALL` (straight to the
   native) instead of `GETGLOBAL; CALL` → cold path → `call_slow`;
   `symbol? number? exact-integer? string? vector?` are ops (peval calls
   `symbol?` 3 million times). maze 0.102 → 0.062, peval 0.299 → 0.184,
   deriv 0.438 → 0.354, matrix 0.124 → 0.106, conform 0.283 → 0.244.
3. **Inlining** (f1c9031). Calls of small (≤ 12 AST nodes),
   non-recursive known procedures are replaced by their bodies:
   top-level procedures defined once and never `set!`, and unassigned
   `letrec` lambdas. Parameters are bound by a `let`, or replaced by a
   constant or unassigned-local argument. nucleic 0.187 → 0.092,
   simplex 0.209 → 0.153, conform 0.224 → 0.164, maze 0.055 → 0.045,
   sboyer 1.137 → 1.006, peval 0.202 → 0.176. To keep "unbound variable"
   errors for calls that run before the definition, an inlined top-level
   call is guarded by `CHKGLOBAL` (quickened to a no-op once bound);
   b685486 drops the guard where the definition must already have run
   (an earlier top-level form, or before the first form that can run
   program code): it was 20% of the executed ops in nucleic and fibc.
4. **`string-append` copies once** (ca7cf6c): sized first, then each part
   copied into the result, instead of building it in a scratch buffer and
   copying that. string 0.045 → 0.040.
5. **GC growth policy** (a6213d8). `set_growth(400, 2 MiB)` and a new
   `std.gc` `Heap.set_max_budget(256 MiB)`: collect after allocating 4×
   the live data, capped so that big heaps grow by at most 256 MiB.
   string 0.037 → 0.020, earley 0.054 → 0.043, nboyer 0.980 → 0.823,
   paraffins 0.065 → 0.055, graphs 0.159 → 0.143, mperm 0.210 → 0.193;
   nothing slower. (Sweep, best of 5: 200%/1 MiB, 300%/1-4 MiB and
   400%/4 MiB were each worse on one of these.)
6. **16-byte object headers in `std.gc`** (dc435d4). The header lost its
   `next` link (free slots link through the length word, large objects
   are kept in a vector), so a pair takes 32 bytes instead of 48.
   nboyer 0.818 → 0.724, sboyer 0.808 → 0.737, primes 0.191 → 0.174,
   mperm 0.193 → 0.179. earley at the suite's full size (live data about
   1.3 GB) now runs under the 2.5 GB `ulimit -v` (1.7 GB RSS, 10.6 s);
   before, its 1.9 GB of live data could not fit.
7. **Branches for `and`/`or`/`not`** (3471623). Conditions branch through
   `not`, `and` and `or` in either sense; in tail position, `and`
   operands and boolean-valued `or` operands jump to a `#f`/`#t` exit
   instead of materializing booleans. takl 0.135 → 0.126,
   puzzle 0.108 → 0.100, nqueens 0.351 → 0.334, peval 0.126 → 0.120.
8. **Tail self-calls of top-level procedures are loops** (fa31ba0), as
   letrec self-calls already were: a known top-level procedure's closure
   is the one running. takl 0.126 → 0.091, ntakl 0.273 → 0.200,
   mazefun 0.192 → 0.154, ack 0.700 → 0.575, diviter 0.202 → 0.183,
   tak 0.605 → 0.556.
9. **`CALLSELF`** (ff5969c): a non-tail recursive call with the right
   arity calls the running closure, with no global load, closure-type or
   arity check. fib 0.626 → 0.526, tak 0.542 → 0.473, ack 0.565 → 0.490,
   triangl 0.186 → 0.168, divrec 0.242 → 0.222, paraffins 0.052 → 0.048,
   fibfp 0.209 → 0.194.
10. **Higher-order inlining** (84da73f). The inliner copies lambdas and
    letrecs; a call with a lambda (or a known, certainly bound top-level
    procedure) as an argument may inline a body of up to 40 nodes, so the
    argument's calls in the body become known and are inlined in turn,
    and the body's loops are contified. `(map f l)` and `(for-each f l)`
    with one list go through inlinable one-list prelude versions;
    `let`-bound lambdas are known procedures, and a lambda binding left
    unreferenced makes no closure. lattice 0.071 → 0.058,
    deriv 0.315 → 0.264, peval 0.127 → 0.107, conform 0.134 → 0.125.

Tried and dropped (within noise):

- `CALLG`/`TAILCALLG` and `CALLF`/`TAILCALLF`: calls of known top-level
  procedures and of known letrec procedures in free slots that fetch the
  closure themselves and skip the arity check (one dispatch fewer per
  call): 2% on nboyer/sboyer, 1% overall.
- `let`-bound lambdas as known procedures on their own (gcbench is
  dominated by allocation); kept as part of step 10, where it matters.
- Inlining limits of 8 and 20 nodes instead of 12.

Not done, with reasons:

- **Constant folding and propagation**: these programs have almost no
  constant subexpressions in hot code (inlining substitutes constant
  arguments already).
- **Type-check elimination** (an unchecked `car` after `(pair? x)`): the
  tag check is one well-predicted compare and branch inside a handler
  that also decodes operands and dispatches; the estimated gain is a few
  percent at best on the list programs.
- **Lambda lifting**: after contification and inlining, `CLOSURE` is
  under 2% of the executed ops except in cpstak and ctak, where the
  closures are continuations that escape.

## Third phase: calls, allocation, builtins (2026-11-04)

Profiled with an `LD_PRELOAD` sampler (per handler), `--vm-stats`,
`-W handler_frame` and per-builtin call counts over the 43 programs,
then changed `examples/scheme` only (no compiler or std change). Each
step was an interleaved A/B (`--exe old new -n 5`, `--align 64`, wall
time); the whole: `compare.py -n 3 --align 64` with the commit before
(096e04e vs 091245f), host load 2-4.

| geometric mean, wall | gauche | guile-nojit | guile (JIT) |
|---|---|---|---|
| before (091245f) | 0.251 | 0.446 | 0.945 |
| after (096e04e) | **0.224** | **0.398** | **0.845** |
| inner, before | | 0.477 | |
| inner, after | | **0.426** | |

Ours/before: 0.894 wall, 0.892 inner. In inner time ours is slower than
Guile's VM only on pi and string (0.017 s vs 0.016). The largest changes (seconds, before → after, and
after / guile-nojit): deriv 0.225 → 0.144 (0.38), gcbench 0.020 →
0.013 (0.46), nucleic 0.069 → 0.053 (0.27), conform 0.119 → 0.094
(0.52), fib 0.497 → 0.405 (0.37), simplex 0.120 → 0.098 (0.40), lattice
0.055 → 0.044 (0.43), tak 0.458 → 0.386 (0.39), paraffins 0.037 →
0.031 (0.58), earley 0.041 → 0.034 (0.52), peval 0.098 → 0.083 (0.41),
nboyer 0.654 → 0.564 (0.64), sboyer 0.641 → 0.561 (0.59), cpstak 0.267
→ 0.228 (0.53), fibfp 0.186 → 0.157. Unchanged within 3%: the flonum
loops, puzzle, sum, array1, takl, ntakl, pi.

1. **Builtins as ops** (58de404). `(list ...)` is a chain of `CONS`;
   `(vector ...)` and record constructors are `VECTOR`/`RECORD`, which
   allocate from consecutive registers; `memq` and `assq` loop in their
   handler. Before, each was a `PRIMCALL` into the native dispatcher
   (whose `prim_info` arity lookup was an out-of-line call; now
   `#[inline]`, a table). deriv 0.228 → 0.176 (`list`: 2.75 million
   calls), paraffins 0.039 → 0.029 (`vector`), gcbench 0.019 → 0.015
   (records), earley 0.040 → 0.035, conform 0.117 → 0.106 (`memq`).
2. **No stack frame on allocation** (a59e5b4). `CONS`, `MKBOX`,
   `CLOSURE`, `VECTOR` allocate with `try_new` and refill in a cold
   block (`-W handler_frame` flagged them); closures and vectors of 1-4
   slots use constant lengths (no `memset` call), bigger ones a cold
   path. cpstak 0.254 → 0.242, graphs 0.125 → 0.119, ctak and fibc -5%.
3. **No register clearing on calls** (08b40eb). `CALL`, `CALLSELF` and
   `TAILCALL` set every callee register to unspecified so the collector
   never marked a stale value. Now every stack slot below a high-water
   mark is safe to mark: stale values inside live windows are marked
   (their objects stay allocated), the root scanner clears what lies
   above the top window and lowers the mark, and a call that passes the
   mark clears the new slots in the cold path that checks for stack
   overflow (`rt.grow`). fib 0.496 → 0.400, tak 0.453 → 0.385, fibfp
   0.187 → 0.158, ack 0.467 → 0.428, graphs 0.118 → 0.110.
4. **Aliases and loop-invariant free variables** (df74c9c). A global
   defined once and never `set!` as another name for a builtin or known
   procedure (`(define make-internal-node vector)`, `(define
   lattice->elements car)`, `(define operation blue-edge-operation)`)
   calls it directly (an op where there is one, or inlined); a builtin
   passed as an argument (`(map cdr l)`) counts as higher-order for
   inlining. Free variables used in a contified loop are read into
   registers once before the loop. conform 0.099 → 0.089, lattice 0.049
   → 0.045, simplex 0.120 → 0.105.
5. **Constant-index vector ops and supers** (096e04e). `VREFI`/`VSETI`
   (record fields, `(vector-ref v 2)`); `NUMEQ_TESTT`, `EQ_TESTT`,
   `PAIRP_TESTT`, `EQV_TEST`, `LOADK_RET`, `CAR_CAR`, `CDR_CAR`.
   nucleic 0.064 → 0.054, nboyer 0.620 → 0.589, nqueens 0.309 → 0.297.

Tried and dropped:

- Free variables loaded into registers on procedure entry when used
  twice or more (instead of before loops): simplex 0.101 (vs 0.105) but
  peval 5% slower, as the hoisted loads run on paths that do not use
  them (GETFREE count doubled).
- Supers `ADDI_CALLSELF`, `LOADK_SUB`, `GETGLOBAL_B_VREF`, `VREF_VSET`
  (suggested by `--vm-stats` for fib, fibfp, puzzle, array1): fib and
  fibfp -5%, but divrec +6% and triangl +3% (handler placement);
  0.993 overall.

Where we still lose most (wall, vs guile-nojit): pi 1.30 and chudnovsky
0.84 (std.bigint against GMP), string 0.71 (half the time in `memcpy`
and the zeroing `memset` of `alloc_large` for strings that are
overwritten right after: an uninitialized allocation for byte arrays in
`std.gc` would remove the latter), mperm 0.70, nboyer 0.64, sboyer 0.59
(list walking at memory latency and collection of large live heaps:
`std.gc` marking is 8-12% of nboyer; Guile runs with a 100 MB initial
heap), nqueens 0.59 (`append`, a native call), paraffins 0.58. Against
Gauche the closest are nboyer 0.48 and sboyer 0.46. Next candidates:
calls of known top-level procedures without `GETGLOBAL` (conform,
triangl, puzzle: 10-18% of ops are `GETGLOBAL_B`), the remaining hot
natives as ops or with a lighter call (`append`, `length`, `reverse`,
`odd?`, `positive?`, `symbol->string`, record accessors), and
continuation capture (`call_slow` is 25% of ctak and fibc).

## Bignums (2026-11-03)

Exact bignums (`examples/scheme/bignum.lm`, on `std.bigint`) made pi
and chudnovsky runnable: pi computes 50 to 500 digits of pi three ways
(Brent-Salamin, quadratic and quartic Borwein: `exact-integer-sqrt`,
`quotient` and products of numbers of up to 1000 digits), chudnovsky the
same digits by binary splitting. Both run the suite's inputs, pass their
own checks and print the same output as Gauche and Guile. Best of 3,
interleaved, `--align 64`, seconds:

| benchmark | ours tailcall | goto | switch | gauche | guile-nojit | guile | /gauche | /guile-nojit | /guile |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| pi (wall) | 0.262 | 0.257 | 0.269 | 3.027 | 0.112 | 0.110 | 0.09 | 2.34 | 2.39 |
| chudnovsky (wall) | 0.209 | 0.211 | 0.209 | 1.015 | 0.195 | 0.153 | 0.21 | 1.07 | 1.37 |
| pi (inner) | 0.247 | | | 2.987 | 0.094 | 0.091 | 0.08 | 2.63 | 2.70 |
| chudnovsky (inner) | 0.196 | | | 0.987 | 0.172 | 0.141 | 0.20 | 1.14 | 1.39 |

Ours is 4-11x faster than Gauche, level with Guile's VM on chudnovsky
and 2.3-2.7x slower than Guile on pi. Guile's integers are GMP's, and
pi is dominated by the operations where GMP is far ahead: integer square
roots and divisions of 500-1000 digit numbers (std.bigint has
schoolbook division and a Newton isqrt built on it; GMP has
subquadratic division and a dedicated square root). Against CPython 3.13
std.bigint is faster on the same sizes (1000 by 500 digits: product
2.3 us vs 6.3, quotient 2.0 vs 5.2, isqrt 5.7 vs 7.6).

The 41 other programs do not overflow, and the bignum support changed
only cold paths there (overflow and mixed operands already left the fast
paths): the A/B against the previous commit (best of 3, interleaved,
`--align-functions=64`, geometric mean new/old) was 1.015 tailcall,
1.023 goto, 1.026 switch, on a machine shared with another agent's
builds; the largest repeatable differences are 0.5-1 ms on the 20 ms
programs (string, gcbench), and the empty program starts 0.08 ms later
(six more prelude definitions). A first version that called
`gc.collect` directly in the arithmetic ops' cold paths cost goto
dispatch 4-12% on tak, diviter and ack; the collection now happens in a
`#[cold]` helper (`builtins.arith_gc`).

**Subquadratic std.bigint (2026-11-03, later).** Squaring, Toom-3,
divide-and-conquer division, Zimmermann's square root and division by
precomputed reciprocals (docs/stdlib.md, "std.bigint"; docs/results.md
has the table) took pi from 0.258 s to 0.158 s wall (2.37x Guile ->
1.45x; inner time 2.69x -> 1.69x) and chudnovsky from 0.209 s to 0.184 s
(1.36x -> 1.21x). What remains on pi is per-limb speed at 10-100 limbs:
GMP's assembly base cases do a limb product in 1-1.5 cycles, ours (plain
Loimi, no add-with-carry intrinsic) in about 3.

**Carry-chain base cases (2026-11-03, later).** With the
`add_carry`/`sub_borrow`/`mul_add_wide` intrinsics (`adc`/`sbb` chains),
std.bigint's sums are 2.7-3x faster and its products and squares at
pi's sizes 1.2-1.45x (rows two at a time replace the column sums;
Karatsuba from 22 limbs, docs/results.md has the details). pi: 0.154 s
-> 0.140 s wall (1.50x Guile -> 1.36x; inner 1.67x -> 1.52x);
chudnovsky, mostly a few limbs, 0.178 -> 0.176 s (1.22x). The base case
now does a limb product in about 3 cycles against GMP's `mulx`/`adx`
assembly at about 2 on this machine (GMP's whole 26-limb product is
1.2x faster, its 8-limb quotient 1.6x and 8-limb square root 2.3x).

## Skipped benchmarks

| benchmark | why |
|---|---|
| cat, tail, wc, read1, sum1, ray, parsing, dynamic, slatex, scheme, compiler | file ports (`open-input-file`, `call-with-output-file`, `read-char`, `read-line`, `file-exists?`, `delete-file`); slatex and compiler also need `define-syntax`, `parameterize`, `case-lambda`, `delay` |
| equal | its DAG-with-sharing case (`(equal? x y)` on `make-test-tree1 100`) needs R7RS's sharing-aware `equal?`; our recursive `equal?` is exponential on it and times out |
| mbrotZ | complex numbers |
| bv2string | bytevectors, `string->utf8`/`utf8->string` |
| read0, alexpander | not run by the suite's `bench` script either |

The programs this directory used before (`examples/scheme/bench`) were
reproduced from memory by the interpreter's author and were removed.
