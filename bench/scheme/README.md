# examples/scheme vs chibi-scheme on r7rs-benchmarks

**Provenance.** The programs are the public
[r7rs-benchmarks](https://github.com/ecraven/r7rs-benchmarks) suite
(Larceny's R7RS benchmarks by W. D. Clinger et al., drawn from the Gabriel,
Kernighan–Van Wyk and Gambit benchmark sets, maintained by ecraven), pinned
to a commit in [`MANIFEST`](MANIFEST). They are downloaded by `fetch.sh` and
are never committed: the repository has no license file (see `MANIFEST`).
**The program text is used unmodified.** Like the suite's own `bench`
script, `compare.py` builds each runnable program by concatenating

    <impl>-prelude.scm  src/NAME.scm  src/common.scm  <impl>-postlude.scm  src/common-postlude.scm

Chibi gets the suite's own `Chibi-prelude.scm`/`Chibi-postlude.scm`. Our
Scheme gets [`Loimi-prelude.scm`](Loimi-prelude.scm) (empty: our Scheme
accepts and ignores `(import ...)`, and the `(scheme base/read/write/time)`
procedures the suite uses are builtins) and
[`Loimi-postlude.scm`](Loimi-postlude.scm) (defines
`this-scheme-implementation-name`), which is how every implementation in
the suite is set up. The programs read their parameters from standard
input, from the suite's `inputs/NAME.input`.

The reference is **chibi-scheme 0.12** (a bytecode VM interpreter, so the
fair comparison), built from source by `install.sh` with its stock
makefile (`-O3`). Guile was not added: building it from source needs
libgc, GMP, libunistring and libffi first, and then a long bootstrap
compile of its own Scheme code.

```sh
bench/scheme/fetch.sh        # once: downloads r7rs-benchmarks at the pinned commit
bench/scheme/install.sh      # once: builds chibi-scheme into bench/scheme/install/
bench/scheme/compare.py                      # all, 3 runs each, process wall time
bench/scheme/compare.py --inner fib tak      # the benchmarks' own "Elapsed time"
bench/scheme/compare.py --check --count 1    # quick correctness pass, failures in full
bench/scheme/compare.py --dispatch tailcall goto switch
```

A run passes when the benchmark's own result check passes (no `ERROR:`
line) and its output, minus the timing lines, is identical to chibi's.

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

`compare.py -n 3`, minimum of 3 runs, seconds. "wall" is the whole
process (chibi spends about 0.3 s starting up and loading its libraries;
ours about 2 ms, compilation included); "inner" is the `Elapsed time` the
benchmark prints (wall-clock jiffies around the timed loop). Ours is
`--dispatch=tailcall`. Intel i7-6700, clang 19, 2026-11-02.

| benchmark | ours wall | chibi wall | ratio | ours inner | chibi inner | ratio |
|---|---|---|---|---|---|---|
| browse | 0.024 | 2.240 | 0.01 | 0.021 | 1.887 | 0.01 |
| deriv | 0.350 | 2.064 | 0.17 | 0.355 | 1.779 | 0.20 |
| destruc | 0.176 | 1.583 | 0.11 | 0.178 | 1.259 | 0.14 |
| diviter | 0.189 | 1.518 | 0.12 | 0.188 | 1.194 | 0.16 |
| divrec | 0.281 | 1.475 | 0.19 | 0.281 | 1.155 | 0.24 |
| puzzle | 0.131 | 1.982 | 0.07 | 0.129 | 1.650 | 0.08 |
| triangl | 0.202 | 1.522 | 0.13 | 0.201 | 1.200 | 0.17 |
| tak | 0.640 | 2.675 | 0.24 | 0.637 | 2.350 | 0.27 |
| takl | 0.140 | 1.689 | 0.08 | 0.139 | 1.366 | 0.10 |
| ntakl | 0.282 | 0.982 | 0.29 | 0.280 | 0.658 | 0.43 |
| cpstak | 0.282 | 2.046 | 0.14 | 0.281 | 1.725 | 0.16 |
| ctak | 0.027 | 1.690 | 0.02 | 0.025 | 1.369 | 0.02 |
| fib | 0.715 | 2.105 | 0.34 | 0.710 | 1.765 | 0.40 |
| fibc | 0.067 | 1.779 | 0.04 | 0.065 | 1.465 | 0.04 |
| fibfp | 0.236 | 1.656 | 0.14 | 0.235 | 1.334 | 0.18 |
| sum | 0.395 | 1.814 | 0.22 | 0.393 | 1.487 | 0.26 |
| sumfp | 0.111 | 1.505 | 0.07 | 0.110 | 1.182 | 0.09 |
| fft | 0.079 | 1.684 | 0.05 | 0.078 | 1.340 | 0.06 |
| mbrot | 0.064 | 1.770 | 0.04 | 0.062 | 1.428 | 0.04 |
| nucleic | 0.137 | 1.485 | 0.09 | 0.131 | 1.105 | 0.12 |
| pnpoly | 0.106 | 1.963 | 0.05 | 0.104 | 1.648 | 0.06 |
| simplex | 0.220 | 2.292 | 0.10 | 0.217 | 1.943 | 0.11 |
| ack | 0.756 | 2.019 | 0.37 | 0.757 | 1.700 | 0.45 |
| array1 | 0.435 | 1.892 | 0.23 | 0.433 | 1.575 | 0.28 |
| string | 0.042 | 1.491 | 0.03 | 0.040 | 1.150 | 0.03 |
| conform | 0.229 | 1.447 | 0.16 | 0.210 | 1.089 | 0.19 |
| earley | 0.120 | 1.504 | 0.08 | 0.107 | 1.068 | 0.10 |
| graphs | 0.275 | 4.485 | 0.06 | 0.223 | 3.538 | 0.06 |
| lattice | 0.083 | 1.569 | 0.05 | 0.081 | 1.090 | 0.07 |
| matrix | 0.116 | 1.619 | 0.07 | 0.112 | 1.210 | 0.09 |
| maze | 0.087 | 0.946 | 0.09 | 0.084 | 0.459 | 0.18 |
| mazefun | 0.265 | 1.531 | 0.17 | 0.211 | 1.262 | 0.17 |
| nqueens | 0.375 | 2.101 | 0.18 | 0.384 | 1.786 | 0.21 |
| paraffins | 0.057 | 1.359 | 0.04 | 0.054 | 0.985 | 0.05 |
| peval | 0.232 | 1.613 | 0.14 | 0.229 | 1.246 | 0.18 |
| primes | 0.185 | 0.949 | 0.20 | 0.192 | 0.649 | 0.30 |
| quicksort | 0.130 | 1.895 | 0.07 | 0.131 | 1.600 | 0.08 |
| nboyer | 1.139 | 4.058 | 0.28 | 1.127 | 3.684 | 0.31 |
| sboyer | 0.940 | 3.779 | 0.25 | 0.939 | 3.427 | 0.27 |
| gcbench | 0.032 | 1.078 | 0.03 | 0.031 | 0.762 | 0.04 |
| mperm | 0.269 | 2.200 | 0.12 | 0.155 | 1.046 | 0.15 |

Geometric mean of the ratios: 0.10 (wall) and 0.12 (inner), i.e. ours is
roughly 8–10× faster than chibi on this set, and faster on every program.
Reading the spread (the explanations of chibi's side are educated guesses,
not profiles):

- The narrowest gaps are the call-bound programs (ack 0.45, ntakl 0.43,
  fib 0.40, tak 0.27, nboyer 0.31): one call or return per handful of
  arithmetic ops is where chibi's VM is relatively efficient, and where
  our frames (a separate frame stack, register clearing on entry) cost
  most.
- The widest gaps come from things chibi does slowly rather than from
  our dispatch: flonum arithmetic (chibi heap-allocates flonums, ours are
  NaN-boxed: mbrot, fft, sumfp, pnpoly), full re-entrant continuations
  versus our O(1) escape continuations (ctak, fibc), strings and symbols
  (string, browse) and chibi's collector on large live heaps (gcbench on
  chibi: 0.1 s at depth 12, 4.9 s at 14, 8.5 s at 15, over 120 s at 16;
  ours: 0.18 s at 15, 2.3 s at 18).
- Ours runs with 32-bit fixnums that overflow to flonums and no bignums.
  None of the programs above overflows, but it is one reason their
  arithmetic fast paths are short.

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
