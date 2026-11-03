# Prolog benchmarks and reference systems

    bench/prolog/install.sh          # build references, generate programs/ (~2 min)
    bench/prolog/run.py              # time swipl and trealla, best of 3 (~9 min)
    bench/prolog/run.py --loimi PATH # ... and our Prolog, with ratios
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
