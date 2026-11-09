# Python benchmarks

Reference interpreters and benchmark programs for the Python-subset
interpreter (`examples/python`). The latest results are in
[`docs/results.md`](../../docs/results.md): on 2026-11-07 the tail-call
build took 0.31 of CPython 3.13.18's time and 0.11 of MicroPython's, by
geometric mean, and printed CPython's output on every program.

## Running

```sh
bench/python/install.sh       # once, about 3 minutes
bench/python/compare.py [--loimi CMD] [-n N] [--only a,b]
```

- `install.sh` downloads the sources pinned in `MANIFEST` and builds
  CPython 3.13.18 and MicroPython 1.29.0 into
  `install/bin/{python3,micropython}`. It also unpacks pyperformance
  1.14.0 into `src/pyperformance/` (not committed).
- CPython is built with plain `./configure`: its default `-O3`, **no PGO
  and no LTO**. MicroPython is the stock unix port, standard variant.
- `compare.py` runs every program under Loimi's interpreter, CPython and
  MicroPython, interleaved, and takes the best of N (default 3). It checks
  each output against `programs/<name>.expected` (CPython 3.13.18's
  output) and prints a markdown table with ratios and geometric means.
- MicroPython runs with `-X heapsize=256M`. With its default fixed heap,
  `float.py` fails with MemoryError.

## Provenance

All programs come from pyperformance 1.14.0
(https://github.com/python/pyperformance/archive/refs/tags/1.14.0.tar.gz,
sha256 `2eb90812…40ed`, see MANIFEST), file
`pyperformance/data-files/benchmarks/bm_<name>/run_benchmark.py`.

Each program is a standalone copy, with mechanical edits only. Edits
common to all files:

- removed `import pyperf`, the `pyperf.Runner` / `runner.metadata` /
  `argparser` / `add_cmdline_args` / `bench_func` / `bench_time_func` harness
  and the `pyperf.perf_counter()` timing lines;
- added a module-level `LOOPS = n` constant and a `__main__` block that calls
  the benchmark function directly with the benchmark's default size
  parameters (unchanged), `LOOPS` times, then prints a result.

Sizes were chosen so that CPython 3.13.18 (this build) takes about 0.7–1.1 s.

Per-file edits beyond that:

| program | LOOPS | other edits | printed result |
|---|---|---|---|
| chaos | 10 | removed `write_ppm()` (file output, needs `open`/`with`) and the `if filename: write_ppm(...)` call in `create_image_chaos`, which now ends with `return im`; `main(runner, args)` → `main()` using the `DEFAULT_*` constants | sum of all pixels |
| deltablue | 250 | — | `planner.current_mark` |
| fannkuch | 2 | — | `fannkuch(9)` (max flips) |
| float | 10 | — | the final `Point` (via its `__repr__`) |
| go | 5 | — | `versus_cpu()` result, `TIMESTAMP`, `MOVES` |
| hexiom | 100 | `main()` returns the (already checked) solution text instead of the elapsed time | solution board |
| nbody | 10 | `bench_nbody` returns the two `report_energy()` values (were discarded) | `(e_before, e_after)` of last loop |
| nqueens | 8 | `bench_n_queens` returns the list (was discarded) | count, first, last solution |
| raytrace | 3 | `array.array('B', [0] * n)` → `[0] * n` (values are already clamped to 0..255); removed `Canvas.write_ppm()` (file output: `open`, `with`, `bytes`) and the `import array`; `bench_raytrace` returns `sum(canvas.bytes)` | sum of pixel bytes |
| richards | 15 | — | `richard.run(1)` result, holdCount, qpktCount |
| spectral_norm | 6 | `bench_spectral_norm` returns `(vBv, vv)` (were discarded) | `(vBv, vv)` |

## Reference timings

Best of 3 with `compare.py`, on the development machine:

| benchmark | CPython 3.13.18 (s) | MicroPython 1.29.0 (s) |
|---|---|---|
| chaos | 0.730 | 2.154 (output differs) |
| deltablue | 1.081 | 5.300 |
| fannkuch | 0.959 | 1.496 |
| float | 0.969 | 2.289 |
| go | 0.857 | 1.901 (output differs) |
| hexiom | 0.729 | 3.038 |
| nbody | 1.026 | 3.580 (output differs) |
| nqueens | 0.875 | 1.814 |
| raytrace | 0.974 | 3.694 |
| richards | 0.904 | 3.473 |
| spectral_norm | 0.711 | 1.385 |

MicroPython runs every program unchanged, but three outputs differ from
CPython's:

- chaos and go use `random`, and MicroPython has a different generator.
- nbody differs in the last digits of the energies (float rounding).

`compare.py` still reports their times, flagged MISMATCH.

## Library use per program

Beyond builtins such as `range`, `len`, `print`, `list`, `tuple`, `set`,
`int`, `float`, `str`, `abs`, `min`, `max`, `sum` and `isinstance`, the
programs use:

| Program | Library use |
|---|---|
| chaos | `math.sqrt`; `random.seed(int)`, one-argument `random.randrange(n)` |
| go | `math.sqrt`, `math.log`; `random.seed(int)`, `random.random()`, one-argument `random.randrange(n)` (also with n = 2**63-1) |
| float | `from math import sin, cos, sqrt` |
| raytrace | `math.sqrt`, `math.tan`, `math.pi` |
| hexiom | `io.StringIO()`: `.getvalue()`, written to with `print(..., file=s)` |
| others | no imports |

To match the `.expected` output of chaos and go, `random` must reproduce
CPython's exactly: Mersenne Twister, `seed(int)` through `init_by_array`,
`randrange` through `getrandbits`-based `_randbelow`, and `random()` as
the 53-bit `(a*67108864+b)/2**53`.

## Measuring

- On a shared machine, wall times of identical builds moved by up to
  ±7%. Comparisons between builds use the CPU time (user + sys) of
  interleaved runs, which moved by about ±3%.
- 6 of the 11 programs are dominated by instance attributes and method
  calls. Apart from go and hexiom, none uses dicts or strings heavily, so
  dict and string work is barely measured by this set.
- The per-program optimization log and dispatch-mode comparison are in
  `examples/python/FRICTION.md`.
