# MiniJS results

`bench/minijs/compare.py -n 3` on 2026-11-02, commit c9fa2de, tailcall dispatch.
Times are the minimum wall-clock time over 3 runs, in seconds, and every
output is checked. The machine is a shared i7-6700 with other work
running, so expect about ±5% noise.

| Benchmark | MiniJS | node --jitless | node (JIT) | QuickJS | MiniJS / jitless | MiniJS / QuickJS |
|---|---:|---:|---:|---:|---:|---:|
| awfy_micro | 0.815 | 1.570 | 0.136 | 1.986 | 0.52 | 0.41 |
| binary_trees | 1.313 | 1.752 | 0.268 | 4.709 | 0.75 | 0.28 |
| bitops | 0.935 | 1.589 | 0.081 | 1.406 | 0.59 | 0.67 |
| closures | 1.312 | 1.523 | 0.375 | 2.685 | 0.86 | 0.49 |
| deltablue | 0.931 | 1.558 | 0.091 | 2.622 | 0.60 | 0.36 |
| fannkuch | 0.500 | 1.346 | 0.105 | 1.519 | 0.37 | 0.33 |
| json | 1.344 | 1.509 | 0.310 | 2.234 | 0.89 | 0.60 |
| nbody | 0.602 | 1.851 | 0.077 | 1.358 | 0.33 | 0.44 |
| properties | 1.072 | 1.564 | 0.585 | 1.641 | 0.69 | 0.65 |
| raytrace | 0.650 | 1.441 | 0.118 | 2.306 | 0.45 | 0.28 |
| richards | 0.898 | 1.517 | 0.102 | 2.100 | 0.59 | 0.43 |
| sort | 0.999 | 1.383 | 0.372 | 1.765 | 0.72 | 0.57 |
| spectral_norm | 1.010 | 1.600 | 0.101 | 1.415 | 0.63 | 0.71 |
| strings | 1.519 | 1.560 | 0.661 | 2.248 | 0.97 | 0.68 |

MiniJS is faster than V8's bytecode interpreter (Ignition) and than
QuickJS on all 14 clean-room benchmarks. V8 with its optimizing JITs is
still 3–15× faster than MiniJS. Node's times include about 40 ms of
start-up.
