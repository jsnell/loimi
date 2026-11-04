# Python benchmarks

Reference interpreters and benchmark programs for the Python-subset
interpreter (`examples/python`).

- `MANIFEST`, `install.sh` — pinned sources; `install.sh` builds CPython 3.13.18
  (plain `./configure`, CPython's default `-O3`, **no PGO, no LTO**, `make -j2`)
  and MicroPython 1.29.0 (stock unix port, standard variant) into
  `install/bin/{python3,micropython}`, and unpacks pyperformance 1.14.0 into
  `src/pyperformance/` (gitignored). About 3 minutes at `-j2`.
- `programs/<name>.py` — standalone copies of pyperformance benchmarks
  (mechanical edits only, listed below); `programs/<name>.expected` — CPython
  3.13.18's stdout.
- `compare.py` — runs every program under loimi, CPython and MicroPython,
  best of N (default 3), interleaved, checks output, prints a markdown table
  with ratios and geometric means.
  `bench/python/compare.py [--loimi CMD] [-n N] [--only a,b]`.
  MicroPython is run with `-X heapsize=256M` (its default fixed GC heap is
  too small for `float.py`).

## Provenance

All programs come from pyperformance 1.14.0
(https://github.com/python/pyperformance/archive/refs/tags/1.14.0.tar.gz,
sha256 `2eb90812…40ed`, see MANIFEST), file
`pyperformance/data-files/benchmarks/bm_<name>/run_benchmark.py`.

Edits common to all files:

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

Dev machine, best of 3, `compare.py`:

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

MicroPython runs all programs (none needed changes), but three outputs differ
from CPython's: chaos and go use `random` (MicroPython has a different
generator, so different seeds → different games), and nbody differs in the
last digits of the energies (float rounding differences). Its times are still
reported, flagged MISMATCH. Without `-X heapsize=…` `float.py` fails with
MemoryError.

## Library use per program

The loimi interpreter needs these (beyond builtins such as `range`, `len`,
`print`, `list`, `tuple`, `set`, `int`, `float`, `str`, `abs`, `min`, `max`,
`sum`, `isinstance`):

- chaos: `math.sqrt`; `random.seed(int)`, `random.randrange(n)` (one-arg).
- go: `math.sqrt`, `math.log`; `random.seed(int)`, `random.random()`,
  `random.randrange(n)` (one-arg; also with n = 2**63-1).
- float: `from math import sin, cos, sqrt`.
- raytrace: `math.sqrt`, `math.tan`, `math.pi`.
- hexiom: `io.StringIO()` (`.getvalue()`, written to via `print(..., file=s)`).
- others: no imports.

Matching the `.expected` output of chaos and go requires a `random` that
reproduces CPython's exactly (Mersenne Twister, `seed(int)` via
`init_by_array`, `randrange` via `getrandbits`-based `_randbelow`,
`random()` as 53-bit `(a*67108864+b)/2**53`).

## Results

`examples/python` (python-loimi, tail-call release build) against both
references, `compare.py -n 3`: 0.53x CPython 3.13.18's time and 0.20x
MicroPython's by geometric mean; every program prints CPython's output.
The per-program table, the optimization log and the dispatch-mode
comparison are in `examples/python/FRICTION.md`.

## Profiling notes (2026-11-04)

`perf` is not available on the shared machine; the second optimization
round (`examples/python/FRICTION.md`, "Optimizations, second round")
used an `LD_PRELOAD` timer sampler (SIGPROF, rip plus return addresses
found by scanning the stack), `--vm-stats` cold-block counts and
`PYLOIMI_COUNTS=1`. Under load from other jobs, wall times of identical
builds moved by up to ±7%; A/B decisions used CPU time (user + sys) of
interleaved runs, which moved by about ±3%. On the set itself: 6 of the
11 programs are dominated by instance attributes and method calls, and
none uses dicts or strings heavily (go and hexiom aside), so dict and
string work is barely measured here.
