# Scheme benchmarks: examples/scheme vs Gauche, Guile and chibi-scheme

This directory runs the Scheme example (`examples/scheme`) on the
r7rs-benchmarks suite against three other Scheme implementations. It
covers how to install the references, where the programs come from, how
to run them, and a summary of the results.

## Results summary

From the 2026-11-07 run at commit 9dce087 (Intel Core i7-6700, minimum of
3 interleaved runs, wall time of the whole process). The full table,
machine and method are in [`docs/results.md`](../../docs/results.md).

Geometric mean over the 43 programs of our time divided by the
reference's (below 1: ours takes less time):

| our build | gauche | guile-nojit | guile (JIT on) |
|---|---:|---:|---:|
| tail-call dispatch | 0.22 | 0.38 | 0.80 |
| goto dispatch | 0.23 | 0.39 | 0.83 |
| switch dispatch | 0.25 | 0.43 | 0.90 |
| tail-call with Loimi's JIT (`--jit-stencils`) | 0.17 | 0.30 | 0.63 |

- Ours takes less time than Guile with its JIT on 21 of 43 programs; with
  Loimi's JIT, on 30.
- The two bignum programs: pi 1.12× Guile's time, chudnovsky 0.96×.
- chibi-scheme was not run; in earlier runs it was about 8–10× slower
  than ours on every program.

### Why the ratios look as they do

These explanations of the other implementations are educated guesses
backed by experiments, not profiles of them. The numbers in this section
are from older runs (2026-11-02 to 2026-11-04, on a machine shared with
other work), so they differ from the summary above.

- **Startup** (an almost empty program, best of 5): ours 1 ms, Guile
  7 ms, Gauche 42 ms (60 ms with the SLIB library the suite loads),
  chibi 313 ms. Some programs run under 30 ms on Guile, so their ratios
  are coarse.
- **Gauche** is slower on every program. The closest are nboyer and
  sboyer (0.46–0.59 across those runs): list walking limited by memory latency, where
  Gauche's 16-byte pairs help it. Our pairs are 32 bytes.
- **Guile without its JIT** is the closest like-for-like reference: a
  bytecode VM in C behind an optimizing compiler (`guild compile -O3`).
  The programs where it comes closest are allocation-bound. Guile runs
  with the suite's 100 MB initial heap and barely collects; ours starts
  at 2 MiB and collects after allocating 4× the live data.
- **Guile with its JIT** is 2–3.6× faster on call-bound programs (tak,
  fib, ack, takl, ntakl, sum) and list-heavy ones (nboyer, sboyer, graphs,
  nqueens, mperm): its template JIT removes dispatch and operand decoding.
- **Ours wins on flonums** (fibfp, sumfp, mbrot, fft, pnpoly, nucleic):
  Guile heap-allocates flonums it cannot unbox; ours are NaN-boxed.
- **Ours wins on continuations** (ctak, fibc: 0.04–0.07 of Guile's time): Guile copies
  the stack for `call/cc`; ours are escape-only. This is not a
  like-for-like comparison.
- **Bignums** (pi, chudnovsky): Guile uses GMP; ours use `std.bigint`.
  pi is dominated by square roots and divisions of 500–1000 digit
  numbers, where GMP's assembly base cases are faster (about 2 cycles per
  limb product against our 3 on this machine).

Caveats that favour ours: escape-only continuations and byte strings.
One that favours Guile: the 100 MB initial heap.

## Programs

The programs are the public
[r7rs-benchmarks](https://github.com/ecraven/r7rs-benchmarks) suite:
Larceny's R7RS benchmarks by W. D. Clinger et al., drawn from the
Gabriel, Kernighan–Van Wyk and Gambit benchmark sets, maintained by
ecraven. [`MANIFEST`](MANIFEST) pins the commit.

`fetch.sh` downloads them. They are never committed, because the
repository has no license file (details in `MANIFEST`).

**The program text is used unmodified.** Like the suite's own `bench`
script, `compare.py` builds each runnable program by concatenating:

```
<impl>-prelude.scm  src/NAME.scm  src/common.scm  <impl>-postlude.scm  src/common-postlude.scm
```

A missing prelude or postlude is skipped. Each reference gets the suite's
own files. Our Scheme gets:

- [`Loimi-prelude.scm`](Loimi-prelude.scm), which is empty: our Scheme
  accepts and ignores `(import ...)`, and the procedures the suite uses
  are builtins.
- [`Loimi-postlude.scm`](Loimi-postlude.scm), which defines
  `this-scheme-implementation-name`.

The programs read their parameters from standard input, from the suite's
`inputs/NAME.input`.

### Inputs

The suite's inputs are sized for native-code compilers, which means
minutes per program on chibi. `compare.py` (its `INPUTS` table) replaces
the leading data of each input file: always the iteration count, and for
some programs the problem size with its expected result.

- The smaller sizes come from the "old inputs" listed in the suite's
  input files where there are some (tak, cpstak, ctak, takl). The others
  were checked by running chibi on them.
- Every expected result is verified by the benchmark itself under chibi.
- Sizes were chosen so that chibi takes about 1–3 s (graphs, at its
  smallest interesting size, takes 4.5 s).
- pi and chudnovsky run the suite's own inputs. chibi is not run on them
  (`SKIP` in `compare.py`): its bignums take 21 s for one of pi's 100
  iterations, so the expected output is Gauche's.

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

### Skipped programs

| benchmark | why |
|---|---|
| cat, tail, wc, read1, sum1, ray, parsing, dynamic, slatex, scheme, compiler | need file ports (`open-input-file`, `read-line`, `delete-file` and similar); slatex and compiler also need `define-syntax`, `parameterize`, `case-lambda`, `delay` |
| equal | needs R7RS's sharing-aware `equal?`; ours is plain recursive, exponential on this input, and times out |
| mbrotZ | complex numbers |
| bv2string | bytevectors, `string->utf8`/`utf8->string` |
| read0, alexpander | not run by the suite's `bench` script either |

## References

`install.sh` builds all of them from source into `bench/scheme/install/`
(gitignored), from the git commit or sha256-checked release tarball
pinned in `MANIFEST`, with each project's stock compiler flags.

| column | what | version | suite files | command (as in the suite's `bench`) |
|---|---|---|---|---|
| gauche | Gauche, a compiler to a stack VM, in C (`-O2`) | 0.9.15 | `Gauche-postlude.scm` | `gosh -r7 -b prog.scm` |
| guile-nojit | Guile 3, compiled ahead of time to bytecode, VM only (`GUILE_JIT_THRESHOLD=-1`; `-O2 -flto`) | 3.0.11 | `Guile-prelude.scm` | `guild compile -O3 prog.scm` (not timed), then `GC_INITIAL_HEAP_SIZE=100000000 guile prog.scm` |
| guile | the same with Guile's template JIT on (its default) | 3.0.11 | same | same |
| chibi | chibi-scheme, an AST-to-bytecode VM interpreter (`-O3`, computed-goto dispatch) | 0.12 | `Chibi-prelude.scm`, `Chibi-postlude.scm` | `chibi-scheme -m scheme.load -e '(load "prog.scm")'` |

Build notes:

- **Gauche** bundles its own Boehm GC and needs nothing from the system.
  The suite's `Gauche-postlude.scm` imports
  [SLIB](https://people.csail.mit.edu/jaffer/SLIB.html), so `install.sh`
  also unpacks SLIB 3c3 and points Gauche at it. Build: about 4 minutes.
- **Guile** needs bdwgc 8.2.12 and libunistring 1.4.2, which `install.sh`
  builds, and GMP 6.3.0 and libffi 3.4.8 from the system (Debian packages
  with headers). The build bootstraps Guile's compiler. Build: about
  31 minutes at `make -j2` on an i7-6700.
- Guile's compiled `.go` files go to its compile cache under
  `$TMPDIR/loimi-scheme-bench/guile-cache`, where `guile prog.scm` picks
  them up. `compare.py` only rewrites a program file when its text
  changes, so the cache is never stale.

## Running

```sh
bench/scheme/fetch.sh                  # once: r7rs-benchmarks at the pinned commit
bench/scheme/install.sh                # once: chibi, gauche, guile (about 40 min)
bench/scheme/install.sh gauche         # or just some of them
bench/scheme/compare.py                # all programs, all installed references, 3 runs, wall time
bench/scheme/compare.py --inner fib tak            # the programs' own "Elapsed time"
bench/scheme/compare.py --refs gauche,guile-nojit  # a subset of the references ('' = none)
bench/scheme/compare.py --check --count 1          # quick correctness pass, failures in full
bench/scheme/compare.py --dispatch tailcall goto switch
bench/scheme/compare.py --align 128                # build ours with --align-functions=128
bench/scheme/compare.py --cpu fib tak              # user+system CPU time instead of wall time
bench/scheme/compare.py --refs '' --dispatch --exe old new -n 5   # A/B of two builds
```

How `compare.py` measures:

- **Interleaving.** Each repetition runs every implementation once, so
  changes in machine load affect all columns alike.
- **Checking.** A run passes when the program's own result check passes
  (no `ERROR:` line) and its output, minus the timing lines, matches that
  of the first reference that passed (chibi's when it is installed).
- **Ratios.** The `/ref` columns are ours divided by that reference. The
  last line is their geometric mean over the programs where both pass.
- **A/B runs.** With `--exe`, the last lines give the geometric mean of
  each extra binary's times over the first's. Build both binaries with
  the same `--align-functions`: code placement alone moves single
  programs by up to ±9% on this machine (tak), and so does where the
  bytecode lands in the heap.

## Changes that mattered

Each change was measured as an interleaved A/B of two builds (`--exe old
new`, 5 runs). Changes under about 5% are not claimed. Ratios are our
geometric mean over the reference's, wall time.

| phase | geometric mean before → after |
|---|---|
| Compiler passes (to commit 84da73f, measured 2026-11-02) | gauche 0.39 → 0.29, guile-nojit 0.62 → 0.47, guile 1.39 → 1.02 |
| Calls, allocation, builtins (091245f → 096e04e, 2026-11-04) | gauche 0.251 → 0.224, guile-nojit 0.446 → 0.398, guile 0.945 → 0.845 |
| Known calls, more builtins as ops, strings, bignums (5f50eb4 → after, 2026-11-04) | gauche 0.218 → 0.205, guile-nojit 0.398 → 0.374, guile 0.848 → 0.797 |

The individual changes, in seconds before → after:

| change | effect |
|---|---|
| Loops compiled inline: a `letrec` lambda only ever tail-called (named `let`, `do`) becomes jumps in the enclosing procedure, with no closure | earley 0.191 → 0.099, graphs 0.320 → 0.234, mperm 0.345 → 0.262 |
| Builtins the program never redefines are called directly (`PRIMCALL`); type predicates are ops | maze 0.102 → 0.062, peval 0.299 → 0.184 |
| Inlining small (≤ 12 AST nodes) non-recursive known procedures | nucleic 0.187 → 0.092, simplex 0.209 → 0.153 |
| GC growth policy: collect after allocating 4× the live data, from 2 MiB, growing by at most 256 MiB | string 0.037 → 0.020, nboyer 0.980 → 0.823 |
| 16-byte object headers in `std.gc` (a pair takes 32 bytes, not 48) | nboyer 0.818 → 0.724; earley at the suite's full size now fits in 2.5 GB (10.6 s) |
| Tail self-calls of top-level procedures become loops | takl 0.126 → 0.091, ntakl 0.273 → 0.200 |
| `CALLSELF`: a recursive call skips the global load and the type and arity checks | fib 0.626 → 0.526, tak 0.542 → 0.473 |
| Higher-order inlining: a call with a lambda argument may inline a body of up to 40 nodes; one-list `map` and `for-each` inline | lattice 0.071 → 0.058, deriv 0.315 → 0.264 |
| `list`, `vector`, record constructors, `memq`, `assq` as ops | deriv 0.228 → 0.176, paraffins 0.039 → 0.029 |
| Calls no longer clear the callee's registers; the collector clears stale stack slots instead | fib 0.496 → 0.400, tak 0.453 → 0.385 |
| Record accessors as ops (`RREF`, `RSET`, `RECP`) | gcbench 0.013 → 0.008 |
| `call/cc` as an op | ctak 0.021 → 0.018, fibc 0.032 → 0.028 |
| `length`, `odd?`, `reverse`, two-list `append` and others as ops | deriv −14%, maze −12%, matrix −10% |
| Strings filled right after allocation are not zeroed first (`new_uninit` in `std.gc`) | string 0.019 → 0.015 |
| Faster `std.bigint` division and square roots | pi 1.31 → 1.08 of guile-nojit, chudnovsky 0.84 → 0.71 |

Tried and not kept:

- `nanbox(int_high)` with 48-bit fixnums (2026-11-06): suite 0.98, but
  sumfp 1.24–1.27 and mbrot 1.12–1.15, since every float op pays a
  decode and encode on the loop's dependency chain.
- A floating-point value accumulator (2026-11-06, measured twice): mbrot
  0.86–0.89, but the suite 1.00–1.007 and tak and fib slower in every run.
- A larger minimum heap budget (8 or 16 MiB): paraffins −15%, but mperm
  +17–31% and string +40–60%; geometric mean 1.02.
- Calls of known procedures that skip the arity check (`CALLG`,
  `CALLF`), and inlining limits of 8 or 20 nodes: within noise.
- Hoisting free variables to procedure entry: simplex faster, peval 5%
  slower.
