# Prolog benchmarks and reference systems

    bench/prolog/install.sh          # build references, generate programs/ (~2 min)
    bench/prolog/run.py              # time swipl and trealla, best of 3 (~9 min)
    bench/prolog/run.py --loimi PATH # ... and our Prolog, with ratios
    bench/prolog/run.py --exe tailcall=PATH --exe jit=WRAPPER   # several builds
    bench/prolog/run.py --emit       # only write out/NAME.pl

## Reference systems

Built from pinned sources by `install.sh` (versions, URLs and sha256 in
`MANIFEST`) into `install/`:

- **Trealla Prolog 3.14.46**, `install/trealla/bin/tpl`. Stock GNUmakefile
  (`-O3`) with `ISOCLINE=1 NOFFI=1 NOSSL=1`: the bundled line editor instead
  of libedit (no headers on this machine), no FFI, no OpenSSL. ~30 s.
- **SWI-Prolog 10.0.2** (latest stable), `install/swipl/bin/swipl`. CMake
  `-DCMAKE_BUILD_TYPE=Release -DSWIPL_PACKAGES=OFF -DINSTALL_DOCUMENTATION=OFF
  -DBUILD_TESTING=OFF`, Unix Makefiles, GMP from the system (perfect needs
  unbounded integers). CMake is Kitware's pinned 3.31.6 Linux binary release,
  unpacked into `src/` (no cmake on the host, and no python3-venv for pip's).
  ~45 s with `-j2`.

Commands (run.py, on the combined file `out/NAME.pl` described below):

    install/swipl/bin/swipl -O -q -g halt out/NAME.pl
    install/trealla/bin/tpl -q out/NAME.pl -g halt
    PATH out/NAME.pl                                   # --loimi PATH

SWI-Prolog runs with `-O` (arithmetic compiled to VM instructions; SWI's
fast mode; the suite's own calibration did not use it). `-g halt` comes
before the file: with `-t halt`, or with `-g halt` after the file, SWI
prints an extra newline at exit. Trealla's default is its optimizing mode.

## Benchmarks

Source: the SWI-Prolog benchmark suite, <https://github.com/SWI-Prolog/bench>,
commit `d74163e6d7560df26af1fbda9e4d0bfd54787009`, directory `programs/`:
the van Roy / Aquarius benchmarks (boyer, browse, chat_parser, crypt,
fast_mu, flatten, meta_qsort, mu, nreverse, poly_10, prover, qsort,
queens_8, query, reducer, sendmore, serialise, simple_analyzer, tak, zebra,
the derive family ops8/log10/divide10/times10, ...) plus SWI's additions.

License: mixed and mostly absent. chat_parser.pl (Warren & Pereira, MIT-style
permission notice), unify.pl and simple_analyzer.pl (UC Regents, "may be freely
used and modified"), nreverse.pl ("Public domain"); the suite's README lists 21
files (boyer, browse, crypt, fast_mu, flatten, meta_qsort, mu, nand, perfect,
poly_10, prover, queens_8, reducer, sendmore, tak, zebra, ...) as lacking
clear redistribution terms. So, as for bench/scheme, nothing from the suite is
committed: `install.sh programs` downloads the pinned tarball (sha256 in
MANIFEST) and `prepare.py` writes `programs/NAME.pl` (gitignored).

### Programs and edits

`prepare.py` copies these 30 programs, each prefixed by a two-line `%`
comment naming its origin:

    boyer browse chat_parser crypt derive divide10 eval fast_mu flatten log10
    meta_qsort mu nand nreverse ops8 perfect poly_10 prover qsort queens_8
    query reducer sendmore serialise sieve simple_analyzer tak times10 unify
    zebra

The only edits to the sources (applied by prepare.py, which fails if one does
not match exactly once):

| file | edit |
|---|---|
| eval.pl | deleted `:- mode(add(+,-)).` |
| log10.pl | deleted `:- mode(d(+,?,-)).` |
| mu.pl | deleted `:- mode(theorem(+,+,-)).` |
| nand.pl | deleted `:- mode(init_state(+,-,-,-)).` |

(`mode/1` is a DEC-10 declaration, not ISO, with no effect on meaning.) All
other files are unchanged: none has `:- module`/`:- use_module`, none uses
strings, format/2,3, assoc, global variables or tabling. Left out of the
suite: fib, pingpong, moded_path (tabling), queens_clpfd (clpfd), det (SWI's
`=>` single-sided unification).

### Driver

`run.py` writes `out/NAME.pl` as the concatenation of `programs/NAME.pl`,
`iterations(K).` with K from `iterations.txt`, `driver.pl`, and
`:- initialization(main).`, so loading the file runs the benchmark. The
driver (`driver.pl`):

```prolog
bench(N) :- N =< 0, !.
bench(N) :- \+ \+ top, N1 is N - 1, bench(N1).

main :- iterations(K), ( bench(K) -> write(done) ; write(failed) ), nl.
```

`\+ \+ top` undoes each run's bindings, as the suite's harness does
(`not_not_top/1` in its run.pl); unlike the harness, a failing `top` makes
the run fail. A run passes if stdout is exactly `done\n`. No benchmark
writes output itself (from top/0).

K (`iterations.txt`) was calibrated with `run.py --calibrate 1.0` to about
1 s per benchmark under `swipl -O` on this machine, rounded to 2 significant
digits.

### What each benchmark needs

From `swipl builtins.pl -- programs/*.pl`: predicates called but not defined,
in the clauses reachable from `top/0` and in directives (control constructs
and arithmetic functions separately). Every program also needs `,`/2 and
clause indexing basics; the driver adds `\+`/1, `->`/2, `;`/2, `is`/2,
`=<`/2, `-`/2, `write/1`, `nl/0` and the `initialization/1` directive.

| benchmark | builtins | control | arithmetic | other |
|---|---|---|---|---|
| boyer | `=`/2 arg/3 atomic/1 functor/3 is/2 | ! -> ; fail true | - | |
| browse | `=`/2 `=:=`/2 arg/3 atom/1 functor/3 is/2 nonvar/1 var/1 | ! -> ; fail true | * + - mod | |
| chat_parser | `>`/2 | ! fail | | large (1200 lines) |
| crypt | is/2 | ! | * + // mod | |
| derive | integer/1 is/2 | ! | - | |
| divide10 | integer/1 is/2 | ! | - | |
| eval | `>`/2 integer/1 is/2 | ! ; fail true | - (and + in an expression built at run time) | |
| fast_mu | `<` `=:=` `>` `>=` is/2 | ! -> ; | + - `/\` `>>` | |
| flatten | `=` `=..` `=<` `==` `>` `@<` `@>` arg/3 atom_codes/2 atomic/1 functor/3 is/2 nonvar/1 number_codes/2 sort/2 var/1 | ! true | + | DCG |
| log10 | integer/1 is/2 | ! | - | |
| meta_qsort | `=` `=<` nonvar/1 number/1 var/1 | ! -> ; fail true | | |
| mu | `>` is/2 | ! | - | |
| nand | `<` `=` `>` `>=` asserta/1 retract/1 is/2 | ! -> ; \+ fail true | + | `:- dynamic` |
| nreverse | - | | | |
| ops8 | integer/1 is/2 | ! | - | |
| perfect | `<` `=:=` `=<` `>` is/2 | ! \+ findall/3 | * + - mod | unbounded integers (up to ~10^61) |
| poly_10 | `<` is/2 | ! | * + - << >> | `:- op(700,xfx,less_than)` |
| prover | `=` | ! fail | | `:- op` for `#` `&` (xfy) and prefix `+` `-` (fx 500) |
| qsort | `=<` | ! | | |
| queens_8 | `<` `=\=` is/2 | ! fail | + - | |
| query | `<` `>` is/2 | fail | * // | |
| reducer | `<` `=` `=:=` `=<` `==` `=\=` `>` `>=` `\==` arg/3 atom/1 atomic/1 compare/3 functor/3 is/2 number/1 var/1 | ! -> ; | * + -(unary) - // mod | DCG |
| sendmore | `<` `=` `=\=` is/2 | -> ; fail | + - | |
| serialise | `<` atom_codes/2 is/2 | ! | + | |
| sieve | `<` `=<` assertz/1 retract/1 retractall/1 is/2 | ! -> ; \+ true | * + | `:- dynamic(...)` |
| simple_analyzer | `=` `=<` `==` `>` `\==` arg/3 compare/3 functor/3 is/2 keysort/2 nonvar/1 sort/2 var/1 | ! \+ true | + - | DCG |
| tak | `=` `=<` `>` is/2 | | - | |
| times10 | integer/1 is/2 | ! | - | |
| unify | `<` `=` `=<` `>` arg/3 atomic/1 compare/3 functor/3 is/2 nonvar/1 var/1 | ! \+ | + -(unary) | DCG |
| zebra | `=` | | | |

Also loaded but not called from top/0 (only need to parse): chat_parser's
`go/0` (statistics/2, write/1), eval's `t/2` (time/1), zebra's
`print_houses/1` and nand's gate printer (write/1, nl/0).

## Results

`run.py -n 3` on this machine (2026-11-03; wall-clock seconds, best of 3,
startup included: ~0.02 s for swipl, ~0.03 s for tpl). All programs print
`done` on both systems.

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
| geomean |  |  |  | 4.13 |

## Results: the Loimi Prolog (examples/prolog)

### `nanbox(int_high)` (2026-11-06)

`Term` on `repr nanbox(int_high)` (`docs/language.md` §2; commit
7a80f29): `Int(i48)` is the integer variant, so `is`'s `+`/`-` on two
Ints are `add; jo` on the words, compares are word compares and every
pointer payload (`Ref`, `Str`, `Lst`, dereferencing) is `shr $16`
where it was an `and` with a 48-bit mask. `#[scan_raw_words]` is
refused under the layout, so environment frame headers are stored as
`Int` words and the root scanner still marks the stack word by word;
first-argument keys of Ints set bit 15, as `Int(0)`'s word is 0, the
no-key marker (`tests/index_keys.pl`). User+sys CPU, minimum of 7
interleaved runs per benchmark (`run.py`'s programs), against base
(master), A/A (a copy of base) in brackets:

| | align 64 | align 64, again | align 128 |
|---|---:|---:|---:|
| queens_8 | 0.785 (0.999) | 0.795 (0.995) | 0.777 (1.000) |
| query | 0.844 (0.999) | 0.846 (1.000) | 0.812 (1.000) |
| sendmore | 0.839 (0.992) | 0.845 (0.996) | 0.982 (1.000) |
| nreverse | 0.896 (1.013) | 0.864 (1.001) | 0.866 (1.002) |
| crypt | 0.914 (1.019) | 0.883 (1.013) | 0.884 (1.001) |
| eval | 0.840 (0.958) | 0.851 (1.006) | 0.872 (1.023) |
| log10 | 1.394 (1.301) | 1.059 (1.004) | 0.971 (0.977) |
| sieve | 0.959 (1.040) | 0.991 (1.028) | 1.026 (1.095) |
| suite (30) | 0.919 (1.013) | 0.917 (1.003) | 0.920 (1.008) |

Kept. The first align 64 round ran next to test builds (its A/A
spread is wide; log10's 1.39 is noise, 1.06 and 0.97 in the quieter
rounds). The Prolog benchmarks hardly use floats, so the decode cost
that sank Scheme, MiniJS and Mini (`docs/design/speed-audit.md`, item
7, "Ports") does not show.

### Third round (2026-11-04)

`run.py -n 3 --exe before=OLD --exe loimi=NEW`, release builds
(`--align-functions=64`, tail-call dispatch) of commit a739a25 (after the
second round and the sweep-free std.gc) and of the third round, best of
3 interleaved runs on a loaded machine (load 6-8 on 8 threads:
SWI-Prolog took 1.3-1.4 s where it took 1.0 s on 2026-11-03, so compare
ratios within a run). Geometric means: 0.53x SWI-Prolog, 0.13x Trealla;
the code before took 0.57x and 0.14x in the same run. What changed:
`examples/prolog/FRICTION.md`, "Optimizations, third round".

| benchmark | K | swipl s | trealla s | before s | loimi s | before/swipl | before/trealla | loimi/swipl | loimi/trealla |
|---|--:|--:|--:|--:|--:|--:|--:|--:|--:|
| boyer | 57 | 1.364 | 6.735 | 1.323 | 0.707 | 0.97 | 0.20 | 0.52 | 0.10 |
| browse | 41 | 1.328 | 8.637 | 0.935 | 0.904 | 0.70 | 0.11 | 0.68 | 0.10 |
| chat_parser | 160 | 1.402 | 4.987 | 1.048 | 1.009 | 0.75 | 0.21 | 0.72 | 0.20 |
| crypt | 7900 | 1.348 | 5.064 | 0.577 | 0.548 | 0.43 | 0.11 | 0.41 | 0.11 |
| derive | 290000 | 1.366 | 3.744 | 0.652 | 0.633 | 0.48 | 0.17 | 0.46 | 0.17 |
| divide10 | 630000 | 1.336 | 3.691 | 0.730 | 0.693 | 0.55 | 0.20 | 0.52 | 0.19 |
| eval | 11000 | 1.288 | 3.463 | 0.646 | 0.634 | 0.50 | 0.19 | 0.49 | 0.18 |
| fast_mu | 29000 | 1.351 | 5.290 | 0.736 | 0.724 | 0.54 | 0.14 | 0.54 | 0.14 |
| flatten | 48000 | 1.364 | 7.348 | 0.827 | 0.679 | 0.61 | 0.11 | 0.50 | 0.09 |
| log10 | 1300000 | 1.309 | 4.767 | 0.616 | 0.597 | 0.47 | 0.13 | 0.46 | 0.13 |
| meta_qsort | 4700 | 1.361 | 7.075 | 0.651 | 0.643 | 0.48 | 0.09 | 0.47 | 0.09 |
| mu | 30000 | 1.311 | 5.707 | 0.591 | 0.601 | 0.45 | 0.10 | 0.46 | 0.11 |
| nand | 1100 | 1.327 | 5.055 | 0.866 | 0.831 | 0.65 | 0.17 | 0.63 | 0.16 |
| nreverse | 83000 | 1.347 | 10.851 | 0.831 | 0.837 | 0.62 | 0.08 | 0.62 | 0.08 |
| ops8 | 920000 | 1.322 | 4.403 | 0.666 | 0.659 | 0.50 | 0.15 | 0.50 | 0.15 |
| perfect | 3900 | 1.361 | 6.594 | 0.795 | 0.782 | 0.58 | 0.12 | 0.57 | 0.12 |
| poly_10 | 510 | 1.347 | 3.667 | 0.755 | 0.752 | 0.56 | 0.21 | 0.56 | 0.21 |
| prover | 25000 | 1.353 | 4.952 | 0.639 | 0.632 | 0.47 | 0.13 | 0.47 | 0.13 |
| qsort | 47000 | 1.326 | 8.223 | 0.676 | 0.685 | 0.51 | 0.08 | 0.52 | 0.08 |
| queens_8 | 560 | 1.293 | 7.197 | 0.568 | 0.570 | 0.44 | 0.08 | 0.44 | 0.08 |
| query | 11000 | 1.340 | 6.347 | 0.628 | 0.594 | 0.47 | 0.10 | 0.44 | 0.09 |
| reducer | 640 | 1.368 | 6.482 | 1.203 | 1.036 | 0.88 | 0.19 | 0.76 | 0.16 |
| sendmore | 410 | 1.304 | 4.620 | 0.696 | 0.656 | 0.53 | 0.15 | 0.50 | 0.14 |
| serialise | 78000 | 1.350 | 6.970 | 0.814 | 0.821 | 0.60 | 0.12 | 0.61 | 0.12 |
| sieve | 61 | 1.432 | 3.454 | 1.320 | 1.312 | 0.92 | 0.38 | 0.92 | 0.38 |
| simple_analyzer | 1500 | 1.358 | 6.829 | 0.946 | 0.764 | 0.70 | 0.14 | 0.56 | 0.11 |
| tak | 160 | 1.316 | 4.867 | 0.503 | 0.278 | 0.38 | 0.10 | 0.21 | 0.06 |
| times10 | 670000 | 1.331 | 3.937 | 0.626 | 0.614 | 0.47 | 0.16 | 0.46 | 0.16 |
| unify | 13000 | 1.292 | 8.409 | 0.881 | 0.767 | 0.68 | 0.10 | 0.59 | 0.09 |
| zebra | 680 | 1.304 | 4.733 | 0.973 | 0.936 | 0.75 | 0.21 | 0.72 | 0.20 |
| geomean |  |  |  |  |  | 0.57 | 0.14 | 0.53 | 0.13 |

### Second round (2026-11-04)

`run.py -n 3 --exe before=OLD --exe loimi=NEW`, 2026-11-04, release
builds (`loimi build examples/prolog/main.lm --release
--align-functions=64 -o ...`, tail-call dispatch) of the code before
(commit 90e86d0) and after the second optimization round, best of 3
interleaved runs, on a machine shared with another agent (load 4-6 on
8 threads: SWI-Prolog took 1.2-1.4 s where it took 1.0 s on 2026-11-03,
so compare ratios within a run). Geometric means: 0.63x SWI-Prolog,
0.15x Trealla; the code before took 0.83x and 0.20x in the same run
(0.79x and 0.19x in the quieter run below). What changed:
`examples/prolog/FRICTION.md`, "Optimizations, second round".

| benchmark | K | swipl s | trealla s | before s | loimi s | before/swipl | loimi/swipl | loimi/trealla |
|---|--:|--:|--:|--:|--:|--:|--:|--:|
| boyer | 57 | 1.236 | 5.987 | 1.525 | 1.297 | 1.23 | 1.05 | 0.22 |
| browse | 41 | 1.218 | 7.628 | 1.092 | 0.914 | 0.90 | 0.75 | 0.12 |
| chat_parser | 160 | 1.265 | 4.317 | 1.175 | 0.961 | 0.93 | 0.76 | 0.22 |
| crypt | 7900 | 1.201 | 4.539 | 0.700 | 0.571 | 0.58 | 0.48 | 0.13 |
| derive | 290000 | 1.212 | 3.295 | 0.848 | 0.649 | 0.70 | 0.54 | 0.20 |
| divide10 | 630000 | 1.301 | 3.313 | 0.977 | 0.700 | 0.75 | 0.54 | 0.21 |
| eval | 11000 | 1.152 | 3.029 | 0.808 | 0.604 | 0.70 | 0.52 | 0.20 |
| fast_mu | 29000 | 1.194 | 4.688 | 0.999 | 0.730 | 0.84 | 0.61 | 0.16 |
| flatten | 48000 | 1.221 | 6.411 | 1.007 | 0.828 | 0.82 | 0.68 | 0.13 |
| log10 | 1300000 | 1.236 | 4.327 | 0.724 | 0.624 | 0.59 | 0.50 | 0.14 |
| meta_qsort | 4700 | 1.180 | 6.143 | 0.760 | 0.653 | 0.64 | 0.55 | 0.11 |
| mu | 30000 | 1.175 | 5.118 | 0.757 | 0.600 | 0.64 | 0.51 | 0.12 |
| nand | 1100 | 1.165 | 4.484 | 0.936 | 0.799 | 0.80 | 0.69 | 0.18 |
| nreverse | 83000 | 1.198 | 9.766 | 0.993 | 0.917 | 0.83 | 0.77 | 0.09 |
| ops8 | 920000 | 1.316 | 4.274 | 0.921 | 0.757 | 0.70 | 0.57 | 0.18 |
| perfect | 3900 | 1.315 | 6.486 | 1.159 | 0.874 | 0.88 | 0.66 | 0.13 |
| poly_10 | 510 | 1.199 | 3.634 | 1.250 | 0.792 | 1.04 | 0.66 | 0.22 |
| prover | 25000 | 1.308 | 4.856 | 0.806 | 0.724 | 0.62 | 0.55 | 0.15 |
| qsort | 47000 | 1.352 | 8.169 | 0.873 | 0.843 | 0.65 | 0.62 | 0.10 |
| queens_8 | 560 | 1.311 | 7.163 | 1.404 | 0.618 | 1.07 | 0.47 | 0.09 |
| query | 11000 | 1.343 | 6.379 | 1.409 | 0.639 | 1.05 | 0.48 | 0.10 |
| reducer | 640 | 1.343 | 6.406 | 1.432 | 1.211 | 1.07 | 0.90 | 0.19 |
| sendmore | 410 | 1.269 | 4.562 | 1.396 | 0.698 | 1.10 | 0.55 | 0.15 |
| serialise | 78000 | 1.318 | 6.893 | 0.976 | 0.947 | 0.74 | 0.72 | 0.14 |
| sieve | 61 | 1.421 | 3.409 | 2.464 | 1.274 | 1.73 | 0.90 | 0.37 |
| simple_analyzer | 1500 | 1.346 | 6.687 | 1.202 | 0.994 | 0.89 | 0.74 | 0.15 |
| tak | 160 | 1.332 | 4.903 | 0.810 | 0.549 | 0.61 | 0.41 | 0.11 |
| times10 | 670000 | 1.303 | 3.815 | 0.910 | 0.738 | 0.70 | 0.57 | 0.19 |
| unify | 13000 | 1.301 | 8.713 | 1.317 | 1.039 | 1.01 | 0.80 | 0.12 |
| zebra | 680 | 1.336 | 4.735 | 1.187 | 1.025 | 0.89 | 0.77 | 0.22 |
| geomean |  |  |  |  |  | 0.83 | 0.63 | 0.15 |

## Results before the second round (2026-11-03)

`run.py -n 3 --loimi examples/prolog/.build/pl` with a release build
(`loimi build examples/prolog/main.lm --release -o ...`, tail-call
dispatch), 2026-11-03, best of 3 interleaved runs, on a shared machine.
Geometric means: 0.79x SWI-Prolog, 0.19x Trealla. How it got there, and
what Loimi made easy or hard: `examples/prolog/FRICTION.md`.

| benchmark | K | swipl s | trealla s | loimi s | loimi/swipl | loimi/trealla |
|---|--:|--:|--:|--:|--:|--:|
| boyer | 57 | 1.144 | 5.269 | 1.312 | 1.15 | 0.25 |
| browse | 41 | 1.048 | 6.726 | 0.910 | 0.87 | 0.14 |
| chat_parser | 160 | 1.102 | 3.833 | 0.952 | 0.86 | 0.25 |
| crypt | 7900 | 1.028 | 3.897 | 0.568 | 0.55 | 0.15 |
| derive | 290000 | 1.007 | 2.859 | 0.702 | 0.70 | 0.25 |
| divide10 | 630000 | 1.016 | 2.835 | 0.787 | 0.77 | 0.28 |
| eval | 11000 | 0.990 | 2.611 | 0.683 | 0.69 | 0.26 |
| fast_mu | 29000 | 1.082 | 4.253 | 0.891 | 0.82 | 0.21 |
| flatten | 48000 | 1.076 | 5.773 | 0.813 | 0.76 | 0.14 |
| log10 | 1300000 | 1.065 | 3.881 | 0.654 | 0.61 | 0.17 |
| meta_qsort | 4700 | 1.083 | 5.490 | 0.633 | 0.58 | 0.12 |
| mu | 30000 | 1.084 | 4.453 | 0.631 | 0.58 | 0.14 |
| nand | 1100 | 0.992 | 3.745 | 0.788 | 0.79 | 0.21 |
| nreverse | 83000 | 1.030 | 8.226 | 0.802 | 0.78 | 0.10 |
| ops8 | 920000 | 1.029 | 3.309 | 0.659 | 0.64 | 0.20 |
| perfect | 3900 | 1.018 | 5.033 | 0.857 | 0.84 | 0.17 |
| poly_10 | 510 | 1.033 | 2.830 | 0.990 | 0.96 | 0.35 |
| prover | 25000 | 1.041 | 3.674 | 0.590 | 0.57 | 0.16 |
| qsort | 47000 | 1.085 | 6.240 | 0.628 | 0.58 | 0.10 |
| queens_8 | 560 | 1.023 | 5.609 | 1.085 | 1.06 | 0.19 |
| query | 11000 | 1.034 | 4.938 | 1.086 | 1.05 | 0.22 |
| reducer | 640 | 1.040 | 4.916 | 0.994 | 0.96 | 0.20 |
| sendmore | 410 | 1.033 | 3.491 | 1.139 | 1.10 | 0.33 |
| serialise | 78000 | 1.081 | 5.387 | 0.721 | 0.67 | 0.13 |
| sieve | 61 | 1.028 | 2.764 | 2.086 | 2.03 | 0.75 |
| simple_analyzer | 1500 | 1.038 | 5.040 | 0.884 | 0.85 | 0.18 |
| tak | 160 | 1.147 | 4.014 | 0.660 | 0.58 | 0.16 |
| times10 | 670000 | 1.055 | 2.968 | 0.660 | 0.63 | 0.22 |
| unify | 13000 | 0.989 | 6.611 | 0.927 | 0.94 | 0.14 |
| zebra | 680 | 1.029 | 3.666 | 0.854 | 0.83 | 0.23 |
| geomean |  |  |  |  | 0.79 | 0.19 |
