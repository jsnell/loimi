# Prolog benchmarks

How to build the reference Prolog systems, where the 30 benchmark
programs come from, and how to time them against the Loimi Prolog
(`examples/prolog`). The latest full results table is in
[`docs/results.md`](../../docs/results.md#prolog).

## Running

```sh
bench/prolog/install.sh                    # build references, generate programs/ (~2 min)
bench/prolog/run.py                        # time swipl and trealla, best of 3 (~9 min)
bench/prolog/run.py --loimi PATH           # ... and our Prolog, with ratios
bench/prolog/run.py --exe tailcall=PATH --exe jit=WRAPPER   # several builds
bench/prolog/run.py --emit                 # only write out/NAME.pl
```

Other options:

- `-n RUNS`: number of runs per benchmark (default 3).
- `--slow S`: a benchmark that takes more than `S` seconds under a
  system is not run again under it.
- `--calibrate S`: rewrite `iterations.txt` so each benchmark takes about
  `S` seconds under SWI-Prolog.
- `NAME ...`: run only these benchmarks.

Times are the minimum wall-clock time of the whole process over the
runs, startup and loading included (about 0.02 s for SWI-Prolog, 0.03 s
for Trealla). Runs are interleaved round-robin over benchmarks and
systems. Ratio columns are our time over each reference's, with the
geometric mean over the benchmarks that every compared system passed.

## Reference systems

`install.sh` builds both from pinned sources into `install/`. Versions,
URLs and sha256 sums are in `MANIFEST`.

| System | Binary | Build |
|---|---|---|
| Trealla Prolog 3.14.46 | `install/trealla/bin/tpl` | Stock GNUmakefile (`-O3`) with `ISOCLINE=1 NOFFI=1 NOSSL=1`: bundled line editor instead of libedit, no FFI, no OpenSSL. About 30 s. |
| SWI-Prolog 10.0.2 | `install/swipl/bin/swipl` | CMake Release build with packages, docs and tests off; GMP from the system (`perfect` needs unbounded integers). About 45 s with `-j2`. |

CMake is Kitware's pinned 3.31.6 Linux binary, unpacked into `src/`,
because the host has no cmake.

Each system runs the same combined file `out/NAME.pl`:

```sh
install/swipl/bin/swipl -O -q -g halt out/NAME.pl
install/trealla/bin/tpl -q out/NAME.pl -g halt
PATH out/NAME.pl                                   # --loimi PATH
```

- SWI-Prolog runs with `-O`, its fast mode, which compiles arithmetic to
  VM instructions. The suite's own calibration did not use it.
- `-g halt` comes before the file: otherwise SWI prints an extra newline
  at exit.
- Trealla's default is already its optimizing mode.

## The programs

The programs come from the SWI-Prolog benchmark suite,
<https://github.com/SWI-Prolog/bench>, commit
`d74163e6d7560df26af1fbda9e4d0bfd54787009`, directory `programs/`. They
are the van Roy / Aquarius benchmarks plus SWI's additions:

```
boyer browse chat_parser crypt derive divide10 eval fast_mu flatten log10
meta_qsort mu nand nreverse ops8 perfect poly_10 prover qsort queens_8
query reducer sendmore serialise sieve simple_analyzer tak times10 unify
zebra
```

Left out: fib, pingpong and moded_path (need tabling), queens_clpfd
(needs clpfd) and det (needs SWI's `=>`).

**Not committed.** Licensing in the suite is mixed and mostly absent; its
README lists 21 files without clear redistribution terms. So
`install.sh programs` downloads the pinned tarball and `prepare.py`
writes `programs/NAME.pl` (gitignored), each with a two-line comment
naming its origin.

**Edits.** `prepare.py` deletes four DEC-10 `mode/1` declarations, which
are not ISO and do not change meaning. It fails if an edit does not
match exactly once.

| File | Deleted line |
|---|---|
| eval.pl | `:- mode(add(+,-)).` |
| log10.pl | `:- mode(d(+,?,-)).` |
| mu.pl | `:- mode(theorem(+,+,-)).` |
| nand.pl | `:- mode(init_state(+,-,-,-)).` |

`builtins.pl` lists the built-in predicates each program calls:
`swipl builtins.pl -- programs/*.pl`.

## The driver

`run.py` writes `out/NAME.pl` as the concatenation of:

1. `programs/NAME.pl`
2. `iterations(K).`, with K from `iterations.txt`
3. `driver.pl`
4. `:- initialization(main).`

So loading the file runs the benchmark. The driver:

```prolog
bench(N) :- N =< 0, !.
bench(N) :- \+ \+ top, N1 is N - 1, bench(N1).

main :- iterations(K), ( bench(K) -> write(done) ; write(failed) ), nl.
```

`\+ \+ top` undoes each run's bindings, as the suite's own harness does.
Unlike that harness, a failing `top` fails the run. A run passes if
stdout is exactly `done\n`; no benchmark prints anything itself.

K was calibrated with `run.py --calibrate 1.0` to about 1 s per
benchmark under `swipl -O` on the benchmark machine, rounded to two
significant digits.

## Results

Latest run, 2026-11-07 at commit 9dce087, Intel Core i7-6700, quiet
machine (method and per-benchmark times in
[`docs/results.md`](../../docs/results.md)). Geometric means of our time
over the reference's:

| Loimi build | vs SWI-Prolog | vs Trealla |
|---|---:|---:|
| tail-call | 0.47 | 0.11 |
| goto | 0.51 | 0.12 |
| switch | 0.57 | 0.14 |
| JIT | 0.45 | 0.11 |

The tail-call build takes 0.22 (tak) to 0.82 (sieve) of SWI-Prolog's
time, less on all 30. Over the 2026-11-03 run below, Trealla took 4.13×
SWI-Prolog's time by geometric mean.

### Reference times (2026-11-03)

`run.py -n 3`, wall-clock seconds, best of 3. All programs print `done`
on both systems.

| benchmark | K | swipl s | trealla s | trealla/swipl |
|---|--:|--:|--:|--:|
| boyer | 57 | 1.033 | 4.967 | 4.81 |
| browse | 41 | 1.012 | 6.502 | 6.42 |
| chat_parser | 160 | 1.043 | 3.734 | 3.58 |
| crypt | 7900 | 1.019 | 3.866 | 3.79 |
| derive | 290000 | 1.027 | 2.854 | 2.78 |
| divide10 | 630000 | 1.024 | 2.848 | 2.78 |
| eval | 11000 | 0.964 | 2.589 | 2.68 |
| fast_mu | 29000 | 1.028 | 3.983 | 3.88 |
| flatten | 48000 | 1.007 | 5.423 | 5.39 |
| log10 | 1300000 | 1.013 | 3.677 | 3.63 |
| meta_qsort | 4700 | 1.006 | 5.267 | 5.23 |
| mu | 30000 | 0.988 | 4.292 | 4.35 |
| nand | 1100 | 0.967 | 3.713 | 3.84 |
| nreverse | 83000 | 1.011 | 8.234 | 8.14 |
| ops8 | 920000 | 1.018 | 3.284 | 3.23 |
| perfect | 3900 | 1.014 | 5.010 | 4.94 |
| poly_10 | 510 | 1.003 | 2.667 | 2.66 |
| prover | 25000 | 1.024 | 3.693 | 3.61 |
| qsort | 47000 | 1.014 | 6.187 | 6.10 |
| queens_8 | 560 | 1.004 | 5.522 | 5.50 |
| query | 11000 | 1.019 | 4.754 | 4.66 |
| reducer | 640 | 1.033 | 4.760 | 4.61 |
| sendmore | 410 | 0.977 | 3.474 | 3.56 |
| serialise | 78000 | 1.008 | 5.194 | 5.15 |
| sieve | 61 | 0.980 | 2.489 | 2.54 |
| simple_analyzer | 1500 | 1.002 | 5.000 | 4.99 |
| tak | 160 | 1.026 | 3.604 | 3.51 |
| times10 | 670000 | 1.009 | 2.947 | 2.92 |
| unify | 13000 | 0.968 | 6.540 | 6.76 |
| zebra | 680 | 1.010 | 3.638 | 3.60 |
| **geomean** |  |  |  | **4.13** |

### Changes that mattered

Release builds, tail-call dispatch. The 2026-11-04 rounds ran on a
loaded machine (SWI-Prolog took 1.2–1.4 s instead of 1.0 s), so compare
ratios only within one row. What each round changed is in
`examples/prolog/FRICTION.md`.

| Date | Change | Effect (geometric mean) |
|---|---|---|
| 2026-11-03 | First release build | 0.79× SWI-Prolog, 0.19× Trealla. |
| 2026-11-04 | Second optimization round (from 90e86d0) | 0.83× → 0.63× SWI-Prolog in the same run; sieve 1.73 → 0.90, queens_8 1.07 → 0.47. |
| 2026-11-04 | Third optimization round (from a739a25) | 0.57× → 0.53× SWI-Prolog; tak 0.38 → 0.21. |
| 2026-11-06 | `Term` on `repr nanbox(int_high)` (7a80f29) | 0.92 of the old user+sys CPU time over the suite, min of 7 interleaved runs (A/A 1.00–1.01); queens_8 0.78–0.80, query 0.81–0.85. |
