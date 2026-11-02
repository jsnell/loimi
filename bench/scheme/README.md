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
```

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
interesting size, takes 4.5 s).

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
| nucleic | 1 | (suite) | | | | |
| pnpoly | 10000 | (suite) | | | | |
| simplex | 15000 | (suite) | | | | |

## Results

`compare.py -n 3` and `compare.py -n 3 --inner`, minimum of 3 runs,
seconds; ours is `--dispatch=tailcall`. Intel i7-6700, gcc 14 / clang 19,
2026-11-02, run one benchmark at a time on an otherwise lightly loaded
machine. Every program passes on every implementation with identical
output.

| geometric mean of ours / reference | gauche | guile-nojit | guile (JIT) | chibi |
|---|---|---|---|---|
| wall (whole process) | **0.39** | **0.62** | **1.39** | 0.10 |
| inner (timed loop only) | **0.49** | **0.67** | **1.57** | 0.12 |

That is, ours is about 2–2.5× faster than Gauche, 1.5–1.6× faster than
Guile's bytecode VM, and 1.4–1.6× slower than Guile with its JIT.
Startup (an almost empty program, best of 5): ours 1 ms, Guile 7 ms,
Gauche 42 ms (60 ms with the postlude's SLIB), chibi 313 ms; "inner"
removes it. The inputs were sized for chibi, so some times are short
(under 30 ms for Guile on browse, string, earley, paraffins, maze,
gcbench), and those ratios are coarse.

### Wall time (whole process)

| benchmark | ours | gauche | guile-nojit | guile | chibi | /gauche | /guile-nojit | /guile | /chibi |
|---|---|---|---|---|---|---|---|---|---|
| browse | 0.022 | 0.082 | 0.027 | 0.019 | 2.127 | 0.27 | 0.81 | 1.17 | 0.01 |
| deriv | 0.336 | 0.716 | 0.379 | 0.186 | 1.996 | 0.47 | 0.89 | 1.80 | 0.17 |
| destruc | 0.170 | 0.470 | 0.230 | 0.092 | 1.566 | 0.36 | 0.74 | 1.84 | 0.11 |
| diviter | 0.189 | 0.539 | 0.350 | 0.170 | 1.498 | 0.35 | 0.54 | 1.11 | 0.13 |
| divrec | 0.275 | 0.635 | 0.446 | 0.217 | 1.460 | 0.43 | 0.62 | 1.27 | 0.19 |
| puzzle | 0.130 | 0.285 | 0.237 | 0.085 | 1.991 | 0.46 | 0.55 | 1.53 | 0.07 |
| triangl | 0.193 | 0.414 | 0.448 | 0.132 | 1.516 | 0.47 | 0.43 | 1.46 | 0.13 |
| tak | 0.625 | 1.326 | 1.003 | 0.243 | 2.752 | 0.47 | 0.62 | 2.57 | 0.23 |
| takl | 0.139 | 0.336 | 0.174 | 0.046 | 1.856 | 0.41 | 0.80 | 3.04 | 0.07 |
| ntakl | 0.280 | 0.527 | 0.321 | 0.078 | 1.039 | 0.53 | 0.87 | 3.57 | 0.27 |
| cpstak | 0.261 | 1.349 | 0.431 | 0.206 | 2.019 | 0.19 | 0.61 | 1.27 | 0.13 |
| ctak | 0.026 | 0.251 | 0.475 | 0.475 | 1.686 | 0.11 | 0.06 | 0.06 | 0.02 |
| fib | 0.629 | 1.446 | 1.101 | 0.289 | 2.101 | 0.44 | 0.57 | 2.18 | 0.30 |
| fibc | 0.062 | 0.348 | 0.596 | 0.547 | 1.758 | 0.18 | 0.10 | 0.11 | 0.04 |
| fibfp | 0.212 | 0.519 | 0.838 | 0.671 | 1.656 | 0.41 | 0.25 | 0.32 | 0.13 |
| sum | 0.363 | 0.824 | 0.647 | 0.171 | 1.836 | 0.44 | 0.56 | 2.12 | 0.20 |
| sumfp | 0.110 | 0.279 | 0.556 | 0.527 | 1.486 | 0.39 | 0.20 | 0.21 | 0.07 |
| fft | 0.077 | 0.306 | 0.178 | 0.150 | 1.651 | 0.25 | 0.44 | 0.52 | 0.05 |
| mbrot | 0.063 | 0.202 | 0.294 | 0.253 | 1.740 | 0.31 | 0.22 | 0.25 | 0.04 |
| nucleic | 0.129 | 0.396 | 0.190 | 0.143 | 1.453 | 0.33 | 0.68 | 0.91 | 0.09 |
| pnpoly | 0.105 | 0.216 | 0.360 | 0.225 | 1.951 | 0.49 | 0.29 | 0.47 | 0.05 |
| simplex | 0.222 | 0.602 | 0.258 | 0.119 | 2.273 | 0.37 | 0.86 | 1.87 | 0.10 |
| ack | 0.719 | 3.874 | 0.992 | 0.232 | 2.095 | 0.19 | 0.72 | 3.09 | 0.34 |
| array1 | 0.436 | 0.977 | 1.225 | 0.329 | 2.053 | 0.45 | 0.36 | 1.33 | 0.21 |
| string | 0.044 | 0.131 | 0.026 | 0.026 | 1.482 | 0.34 | 1.67 | 1.68 | 0.03 |
| conform | 0.205 | 0.464 | 0.183 | 0.076 | 1.404 | 0.44 | 1.12 | 2.71 | 0.15 |
| earley | 0.118 | 0.140 | 0.065 | 0.033 | 1.452 | 0.84 | 1.81 | 3.62 | 0.08 |
| graphs | 0.226 | 0.628 | 0.210 | 0.070 | 4.003 | 0.36 | 1.08 | 3.23 | 0.06 |
| lattice | 0.085 | 0.227 | 0.101 | 0.042 | 1.469 | 0.37 | 0.84 | 2.03 | 0.06 |
| matrix | 0.115 | 0.296 | 0.117 | 0.052 | 1.579 | 0.39 | 0.98 | 2.22 | 0.07 |
| maze | 0.089 | 0.207 | 0.076 | 0.039 | 0.843 | 0.43 | 1.17 | 2.28 | 0.11 |
| mazefun | 0.212 | 0.449 | 0.294 | 0.095 | 1.527 | 0.47 | 0.72 | 2.23 | 0.14 |
| nqueens | 0.373 | 0.720 | 0.523 | 0.180 | 2.160 | 0.52 | 0.71 | 2.07 | 0.17 |
| paraffins | 0.056 | 0.167 | 0.053 | 0.031 | 1.370 | 0.34 | 1.06 | 1.81 | 0.04 |
| peval | 0.249 | 0.404 | 0.205 | 0.077 | 1.631 | 0.62 | 1.22 | 3.24 | 0.15 |
| primes | 0.183 | 0.616 | 0.323 | 0.215 | 0.974 | 0.30 | 0.57 | 0.85 | 0.19 |
| quicksort | 0.127 | 0.300 | 0.196 | 0.077 | 1.884 | 0.42 | 0.65 | 1.65 | 0.07 |
| nboyer | 1.126 | 1.253 | 0.930 | 0.334 | 4.213 | 0.90 | 1.21 | 3.37 | 0.27 |
| sboyer | 0.938 | 1.314 | 1.033 | 0.317 | 3.904 | 0.71 | 0.91 | 2.96 | 0.24 |
| gcbench | 0.036 | 0.175 | 0.051 | 0.019 | 1.167 | 0.20 | 0.70 | 1.87 | 0.03 |
| mperm | 0.286 | 0.407 | 0.199 | 0.059 | 2.187 | 0.70 | 1.44 | 4.81 | 0.13 |

### Inner time (the benchmark's own "Elapsed time")

| benchmark | ours | gauche | guile-nojit | guile | chibi | /gauche | /guile-nojit | /guile | /chibi |
|---|---|---|---|---|---|---|---|---|---|
| browse | 0.020 | 0.023 | 0.016 | 0.011 | 1.849 | 0.88 | 1.26 | 1.88 | 0.01 |
| deriv | 0.371 | 0.662 | 0.369 | 0.179 | 1.702 | 0.56 | 1.01 | 2.08 | 0.22 |
| destruc | 0.168 | 0.411 | 0.219 | 0.081 | 1.230 | 0.41 | 0.77 | 2.08 | 0.14 |
| diviter | 0.187 | 0.429 | 0.339 | 0.155 | 1.179 | 0.44 | 0.55 | 1.21 | 0.16 |
| divrec | 0.274 | 0.586 | 0.435 | 0.205 | 1.141 | 0.47 | 0.63 | 1.33 | 0.24 |
| puzzle | 0.128 | 0.225 | 0.226 | 0.074 | 1.629 | 0.57 | 0.57 | 1.74 | 0.08 |
| triangl | 0.192 | 0.355 | 0.413 | 0.122 | 1.179 | 0.54 | 0.46 | 1.58 | 0.16 |
| tak | 0.602 | 1.281 | 0.963 | 0.228 | 2.377 | 0.47 | 0.62 | 2.64 | 0.25 |
| takl | 0.133 | 0.250 | 0.154 | 0.033 | 1.369 | 0.53 | 0.87 | 3.99 | 0.10 |
| ntakl | 0.269 | 0.465 | 0.308 | 0.067 | 0.663 | 0.58 | 0.87 | 4.03 | 0.41 |
| cpstak | 0.260 | 1.138 | 0.414 | 0.189 | 1.716 | 0.23 | 0.63 | 1.38 | 0.15 |
| ctak | 0.025 | 0.186 | 0.467 | 0.461 | 1.346 | 0.13 | 0.05 | 0.05 | 0.02 |
| fib | 0.626 | 1.380 | 1.086 | 0.278 | 1.762 | 0.45 | 0.58 | 2.25 | 0.36 |
| fibc | 0.060 | 0.338 | 0.642 | 0.572 | 1.490 | 0.18 | 0.09 | 0.11 | 0.04 |
| fibfp | 0.211 | 0.492 | 0.846 | 0.680 | 1.409 | 0.43 | 0.25 | 0.31 | 0.15 |
| sum | 0.364 | 0.791 | 0.665 | 0.162 | 1.512 | 0.46 | 0.55 | 2.25 | 0.24 |
| sumfp | 0.108 | 0.223 | 0.552 | 0.547 | 1.169 | 0.49 | 0.20 | 0.20 | 0.09 |
| fft | 0.077 | 0.248 | 0.170 | 0.145 | 1.339 | 0.31 | 0.45 | 0.53 | 0.06 |
| mbrot | 0.061 | 0.143 | 0.285 | 0.246 | 1.476 | 0.43 | 0.21 | 0.25 | 0.04 |
| nucleic | 0.125 | 0.328 | 0.197 | 0.141 | 1.171 | 0.38 | 0.64 | 0.89 | 0.11 |
| pnpoly | 0.107 | 0.159 | 0.345 | 0.214 | 1.755 | 0.67 | 0.31 | 0.50 | 0.06 |
| simplex | 0.218 | 0.476 | 0.236 | 0.103 | 2.030 | 0.46 | 0.92 | 2.12 | 0.11 |
| ack | 0.672 | 2.804 | 0.920 | 0.204 | 1.701 | 0.24 | 0.73 | 3.29 | 0.39 |
| array1 | 0.419 | 0.804 | 0.993 | 0.263 | 1.578 | 0.52 | 0.42 | 1.59 | 0.27 |
| string | 0.040 | 0.061 | 0.016 | 0.015 | 1.143 | 0.66 | 2.56 | 2.61 | 0.03 |
| conform | 0.201 | 0.381 | 0.167 | 0.064 | 1.053 | 0.53 | 1.20 | 3.17 | 0.19 |
| earley | 0.097 | 0.082 | 0.055 | 0.022 | 1.111 | 1.18 | 1.76 | 4.39 | 0.09 |
| graphs | 0.238 | 0.542 | 0.196 | 0.061 | 3.574 | 0.44 | 1.21 | 3.88 | 0.07 |
| lattice | 0.085 | 0.209 | 0.142 | 0.055 | 1.229 | 0.40 | 0.60 | 1.55 | 0.07 |
| matrix | 0.122 | 0.212 | 0.104 | 0.041 | 1.275 | 0.57 | 1.17 | 2.99 | 0.10 |
| maze | 0.084 | 0.125 | 0.061 | 0.026 | 0.466 | 0.68 | 1.38 | 3.22 | 0.18 |
| mazefun | 0.200 | 0.365 | 0.272 | 0.082 | 1.164 | 0.55 | 0.74 | 2.44 | 0.17 |
| nqueens | 0.360 | 0.612 | 0.481 | 0.161 | 1.765 | 0.59 | 0.75 | 2.24 | 0.20 |
| paraffins | 0.051 | 0.081 | 0.042 | 0.021 | 0.962 | 0.62 | 1.21 | 2.45 | 0.05 |
| peval | 0.243 | 0.319 | 0.193 | 0.065 | 1.236 | 0.76 | 1.26 | 3.75 | 0.20 |
| primes | 0.181 | 0.512 | 0.301 | 0.195 | 0.607 | 0.35 | 0.60 | 0.93 | 0.30 |
| quicksort | 0.118 | 0.229 | 0.179 | 0.061 | 1.499 | 0.52 | 0.66 | 1.93 | 0.08 |
| nboyer | 1.076 | 1.120 | 0.879 | 0.309 | 3.635 | 0.96 | 1.22 | 3.49 | 0.30 |
| sboyer | 0.906 | 1.179 | 0.938 | 0.288 | 3.351 | 0.77 | 0.97 | 3.14 | 0.27 |
| gcbench | 0.031 | 0.080 | 0.018 | 0.007 | 0.755 | 0.38 | 1.72 | 4.23 | 0.04 |
| mperm | 0.157 | 0.199 | 0.120 | 0.033 | 1.030 | 0.79 | 1.31 | 4.75 | 0.15 |

### Reading the results

The explanations of the other implementations' behaviour are educated
guesses backed by the experiments noted, not profiles (there is no
`perf` in this container).

**Gauche.** Ours is faster on every program in wall time and on all but
earley in inner time (1.18). The widest gaps are ack (0.24 inner; Gauche
takes 2.8 s, probably spilling its VM stack to the heap on the deep,
non-tail recursion of `(ack 3 9)`), cpstak, ctak and fibc (closures and
continuations; ours are escape-only, see below), and flonum code. The
narrowest are the allocation- and list-heavy programs: earley 1.18,
nboyer 0.96, browse 0.88, mperm 0.79, sboyer 0.77, peval 0.76, where
Gauche's compiler (which inlines and closes over locals) and its Boehm GC
hold up best.

**Guile without the JIT** is the closest like-for-like reference: a
register-style bytecode VM in C, like ours. Ours is faster on 32 of 41
programs in wall time (28 inner), and on all the call- and arithmetic-bound ones (tak,
fib, ack, sum, diviter, divrec, triangl: 0.46–0.73). Ours is slower on
string (2.56 inner), earley (1.76), gcbench (1.72), maze (1.38), mperm
(1.31), peval, browse (1.26), nboyer, graphs, paraffins (1.21), conform
(1.20), matrix (1.17) and, barely, deriv (1.01). Guile's advantage there is mostly its
**compiler**, not its VM: recompiling the programs for Guile at lower
optimization levels (`guild compile -O1`, which keeps only basic
optimizations, versus the suite's `-O3`), still without the JIT, gives
these inner times:

| benchmark | ours | guile-nojit -O0 | -O1 | -O2 | -O3 |
|---|---|---|---|---|---|
| earley | 0.097 | 0.251 | 0.092 | 0.053 | 0.052 |
| peval | 0.243 | 0.719 | 0.270 | 0.193 | 0.189 |
| mperm | 0.157 | 0.735 | 0.216 | 0.116 | 0.119 |
| nboyer | 1.076 | 4.822 | 1.093 | 0.882 | 0.865 |
| conform | 0.201 | 0.688 | 0.442 | 0.216 | 0.167 |
| graphs | 0.238 | 0.829 | 0.309 | 0.246 | 0.194 |
| maze | 0.084 | 0.306 | 0.157 | 0.064 | 0.062 |
| gcbench | 0.031 | 0.088 | 0.025 | 0.018 | 0.018 |
| string | 0.040 | 0.016 | 0.015 | 0.015 | 0.015 |
| tak | 0.602 | 4.464 | 1.257 | 0.949 | 0.943 |
| fib | 0.626 | 4.864 | 1.588 | 1.094 | 1.091 |

At `-O1` ours is within 5% of Guile's VM or faster on all of these except gcbench
and string; Guile's `-O2` passes (inlining of small procedures,
contification of loops and local functions into jumps, type inference
that removes checks, CSE, unboxing, closure conversion that avoids
allocating closures) are what put it ahead, by 1.2–2× on the list- and
closure-heavy programs. Our compiler does none of these. The other two
losses are runtime and memory costs rather than code quality:

- **string** (2.56) is `string-append` and `substring` on strings of up
  to 0.5 MB. Ours builds a `string-append` result in a scratch buffer and
  then copies it into a new heap string (two copies), and every grown
  string is a fresh large allocation for our collector; Guile does one
  copy and runs with the 100 MB initial heap the suite gives it
  (`GC_INITIAL_HEAP_SIZE`), so it barely collects.
- **gcbench** (1.72) and the allocation in mperm, earley and nboyer point
  at allocation and collection cost. The same 100 MB initial heap
  favours Guile here. mperm also spends 0.13 s of its 0.29 s wall time
  outside the timed loop under ours (Guile: 0.03 s), which is worth a
  look.

**Guile with the JIT** is faster than ours on 32 of 41 programs, by 2–4×
on the call-bound and list-heavy ones (takl/ntakl 4.0, mperm 4.75,
earley 4.39, gcbench 4.23, graphs 3.88, peval 3.75): template JIT code
removes dispatch and operand decoding, on top of the compiler above.
Ours still wins where Guile's value representation or continuations
cost it more than its JIT saves:

- **flonums** (fibfp 0.31, sumfp 0.20, mbrot 0.25, fft 0.53, pnpoly 0.50,
  nucleic 0.89): Guile heap-allocates every flonum that its unboxing pass
  cannot keep in a register across calls and loops; ours are NaN-boxed
  immediates.
- **continuations** (ctak 0.05, fibc 0.11): Guile captures the whole
  stack for `call/cc`; ours are escape-only, so this is not a
  like-for-like comparison (see `examples/scheme/README.md`).
- primes (0.85 wall, 0.93 inner).

**Caveats** that favour ours: 32-bit fixnums that overflow to flonums
instead of bignums (none of these programs overflows, but the fast paths
of the references check for bignum overflow), escape-only continuations,
and byte strings. One that favours Guile: the suite's 100 MB initial
heap.

**chibi-scheme** is about 8–10× slower than ours on every program
(heap-allocated flonums, slower calls, full continuations, and a collector
that struggles with large live heaps: gcbench at depth 12 takes 0.1 s,
at 14 4.9 s, at 15 8.5 s, at 16 over 120 s; ours: 0.18 s at 15, 2.3 s at
18), which makes it a weak reference; it is kept because it is the first
reference compare.py checks outputs against.

## Skipped benchmarks

| benchmark | why |
|---|---|
| cat, tail, wc, read1, sum1, ray, parsing, dynamic, slatex, scheme, compiler | file ports (`open-input-file`, `call-with-output-file`, `read-char`, `read-line`, `file-exists?`, `delete-file`); slatex and compiler also need `define-syntax`, `parameterize`, `case-lambda`, `delay` |
| equal | its DAG-with-sharing case (`(equal? x y)` on `make-test-tree1 100`) needs R7RS's sharing-aware `equal?`; our recursive `equal?` is exponential on it and times out |
| pi, chudnovsky | bignums |
| mbrotZ | complex numbers |
| bv2string | bytevectors, `string->utf8`/`utf8->string` |
| read0, alexpander | not run by the suite's `bench` script either |

The programs this directory used before (`examples/scheme/bench`) were
reproduced from memory by the interpreter's author and were removed.
