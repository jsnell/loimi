# MiniJS results

How fast MiniJS is against V8's interpreter and QuickJS, and which
changes made it so. The newest full run, with every dispatch mode and
the JIT, is in [`docs/results.md`](../../docs/results.md): on
2026-11-07 MiniJS (tail-call) took 0.48 of `node --jitless`'s time and
0.37 of QuickJS's by geometric mean.

## Current state (2026-11-04)

Measured at the end of the second performance pass (commit b71ab20),
against its starting point (573511e):

- Command: `compare.py -n 3 --align 64 --exe <573511e build> --refs node-jitless,qjs`.
- Times: minimum wall-clock seconds over 3 interleaved runs, outputs
  checked, tail-call dispatch.
- Machine: Intel Core i7-6700, shared with other agents (load average 3–6).

| Benchmark | MiniJS | before | node --jitless | QuickJS | after/before | MiniJS/jitless | MiniJS/QuickJS |
|---|---:|---:|---:|---:|---:|---:|---:|
| awfy_micro | 0.744 | 0.752 | 1.540 | 1.974 | 0.99 | 0.48 | 0.38 |
| binary_trees | 0.894 | 0.963 | 1.715 | 4.646 | 0.93 | 0.52 | 0.19 |
| bitops | 0.666 | 0.801 | 1.550 | 1.387 | 0.83 | 0.43 | 0.48 |
| closures | 0.865 | 0.955 | 1.445 | 2.510 | 0.91 | 0.60 | 0.34 |
| deltablue | 0.796 | 0.838 | 1.454 | 2.548 | 0.95 | 0.55 | 0.31 |
| fannkuch | 0.442 | 0.459 | 1.303 | 1.477 | 0.96 | 0.34 | 0.30 |
| json | 0.857 | 0.885 | 1.457 | 2.165 | 0.97 | 0.59 | 0.40 |
| nbody | 0.411 | 0.415 | 1.788 | 1.288 | 0.99 | 0.23 | 0.32 |
| properties | 0.797 | 0.819 | 1.511 | 1.589 | 0.97 | 0.53 | 0.50 |
| raytrace | 0.439 | 0.487 | 1.394 | 2.118 | 0.90 | 0.31 | 0.21 |
| richards | 0.721 | 0.735 | 1.405 | 1.948 | 0.98 | 0.51 | 0.37 |
| sort | 0.784 | 0.830 | 1.297 | 1.680 | 0.94 | 0.60 | 0.47 |
| spectral_norm | 0.818 | 0.903 | 1.551 | 1.361 | 0.91 | 0.53 | 0.60 |
| strings | 1.105 | 1.197 | 1.494 | 2.163 | 0.92 | 0.74 | 0.51 |
| **geometric mean** | | | | | **0.939** | **0.48** | **0.37** |

MiniJS takes less time than V8's bytecode interpreter (Ignition) and
QuickJS on all 14 clean-room benchmarks. V8 with its optimizing JITs was
1.5–12× faster than MiniJS in the 2026-11-02 run; node's times include
about 40 ms of start-up.

Where the time goes now:

- strings: 13% in interning `"w_" + b` in the sort comparator (31,000
  keys) and 7% in string concatenation.
- Elsewhere: calls and returns, and cached property loads (each loads
  the object's shape id first).
- closures: 8% in `%` by a constant that is not a power of two (an
  integer divide).

## Progress

Geometric mean of MiniJS's time over the reference's, tail-call
dispatch, same machine and script throughout:

| Date | Point | / node --jitless | / QuickJS |
|---|---|---:|---:|
| 2026-11-02 | before the first pass (c9fa2de) | 0.61 | 0.47 |
| 2026-11-02 | after the first pass | 0.53 | 0.40 |
| 2026-11-04 | before the second pass (573511e) | 0.51 | 0.39 |
| 2026-11-04 | after the second pass (b71ab20) | 0.48 | 0.37 |

The first pass measured 0.87 of its starting time by geometric mean
(interleaved, minimum of 5 runs): json 0.71, nbody 0.73, closures 0.81.
In it, the weakest ratios to `node --jitless` went from 0.97 (strings),
0.93 (json) and 0.87 (closures) to 0.82, 0.64 and 0.70.

## Changes that mattered

Each effect is new time over old, from interleaved runs of two builds
with `--align-functions=64`, on a machine shared with other agents
(load 2–6). Changes under about 5% are within noise.

| Change | Pass, commit | Effect |
|---|---|---|
| Bytecode cleanups: literal `switch` cases and folded top-level `const`s compare with `JEQK`; unused `x++` drops `TONUM`; string methods accept ropes | 1st (to 2026-11-02), 2139b38 | json 0.91 |
| Common natives (`charCodeAt`, `charAt`, `push`, `Math.floor`, `Math.sqrt`) run inline in specialized `CALL` instructions | 1st (to 2026-11-02), 51ce16c | closures 0.89, sort 0.94, json and strings 0.96 |
| Faster interner (one-word slots), `memchr` in string search, ropes flattened in their register | 1st (to 2026-11-02), f7cdcaa | json 0.85, strings 0.91, properties 0.94 |
| Cheaper `CALL` and `RET`: a plain register-clearing loop, missing arguments and natives out of line, one test for the bottom frame | 1st (to 2026-11-02), b6ac92e, 884beaf | deltablue 0.87, binary_trees 0.90, bitops 0.92, closures 0.95 |
| Float-first arithmetic (`ADD_F` etc.), `%` of integral doubles in i64 | 1st (to 2026-11-02), 9d0a10e | nbody 0.78, raytrace 0.95 |
| Fewer dispatches: calls used in place, ten super-instructions, compares with 8-bit constants | 1st (to 2026-11-02), 9b73506 | suite 0.97 |
| Constructors allocate exactly their `this.x` slot count | 1st (to 2026-11-02), 1833ded | binary_trees 0.94 |
| Jump tables for Int `switch` (`SWITCHI`) | 1st (to 2026-11-02), f6b0ce1 | json 0.97 and 0.96 |
| Cached absent-property reads, `in` through the shared cache, polymorphic store caches (std.obj) | 1st (to 2026-11-02), 143470f | properties 0.89 |
| Int overflow and `DIV` handled inline; compare with a pool constant | 2nd (2026-11-04), c9e11ba | part of the next row |
| No stack frames on fast paths (59 framed handlers down to 17) | 2nd (2026-11-04), c9e11ba, deb3c5d | binary_trees 0.87, bitops 0.82, closures 0.80–0.97, fannkuch, sort and spectral_norm 0.84–0.93 (user+sys CPU) |
| Calls no longer clear registers; the collector clears the stack up to a high-water mark instead | 2nd (2026-11-04), a802af8, 4d33b2f | deltablue, richards, raytrace, awfy_micro 0.94–0.97 |
| Value accumulator: an `f64` register for float ops, with four float super-instructions | later (2026-11-06), 6d40423 | nbody 0.974 / 0.931, suite 0.990 / 0.997 (align 64 / 128; user+sys CPU, median of 9 rounds) |

Loop tests at the bottom (5ab9ec5), constant bit operations (54271ee)
and more super-instructions (adfaaee, b71ab20) each measured within
±2%.

The value accumulator was first measured at suite 1.004 / 1.007 and
reverted, then measured again with more rounds and kept. The first
result was within the noise of the same method.

## Tried and dropped

All within noise unless a number is given.

- Clearing eight registers in `CALL` with no loop branch.
- Interner slots pointing at records with id, length and bytes; or
  holding a key's bytes in 2–4 words: strings 0.994.
- `%` of an integral double through a double divide.
- `#[inline(always)]` on std.obj's `poly_get` and `mega_get`.
- GC growth 400% / 16 MiB instead of 300% / 8 MiB: 0.98 on
  allocation-heavy programs, for more memory.
- Super-instructions for the prelude's callback loop: `data.map(f)`
  over 6M elements already takes 0.136 s, against 0.128 s for V8's
  native `map` under `--jitless`.
- The shape id copied into every object: 0.998 overall, binary_trees 1.06.
- Nine more `RET`/`CALL` super-instructions: 0.996 together.
- `% 2^k` as a mask and inline one-byte string compares: 1.005.
- `ADD_F`/`SUB_F`/`MUL_F` handling Int pairs inline: nbody 5% slower.
- `nanbox(int_high)` with 48-bit Ints (2026-11-06, 980c67c): suite
  0.941 / 0.958 (align 64 / 128), bitops 0.76, but nbody 1.25 / 1.47,
  because float operands now need decoding. Reverted.
