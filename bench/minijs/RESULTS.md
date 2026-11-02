# MiniJS results

`bench/minijs/compare.py -n 3 --align 64` on 2026-11-02, at the end of
the performance pass below, tailcall dispatch. Times are the minimum
wall-clock time over 3 runs, in seconds, and every output is checked.
The machine is a shared i7-6700 with other work running (load average
about 2-3 during this run), so expect about ±5% noise.

| Benchmark | MiniJS | node --jitless | node (JIT) | QuickJS | MiniJS / jitless | MiniJS / QuickJS |
|---|---:|---:|---:|---:|---:|---:|
| awfy_micro | 0.758 | 1.570 | 0.141 | 2.060 | 0.48 | 0.37 |
| binary_trees | 1.241 | 1.739 | 0.250 | 4.705 | 0.71 | 0.26 |
| bitops | 0.812 | 1.602 | 0.086 | 1.431 | 0.51 | 0.57 |
| closures | 1.034 | 1.483 | 0.360 | 2.590 | 0.70 | 0.40 |
| deltablue | 0.895 | 1.490 | 0.085 | 2.612 | 0.60 | 0.34 |
| fannkuch | 0.478 | 1.330 | 0.105 | 1.546 | 0.36 | 0.31 |
| json | 0.960 | 1.490 | 0.299 | 2.199 | 0.64 | 0.44 |
| nbody | 0.442 | 1.931 | 0.084 | 1.372 | 0.23 | 0.32 |
| properties | 0.903 | 1.599 | 0.613 | 1.664 | 0.56 | 0.54 |
| raytrace | 0.530 | 1.446 | 0.104 | 2.179 | 0.37 | 0.24 |
| richards | 0.761 | 1.672 | 0.097 | 2.168 | 0.46 | 0.35 |
| sort | 0.921 | 1.393 | 0.384 | 1.763 | 0.66 | 0.52 |
| spectral_norm | 0.978 | 1.607 | 0.084 | 1.421 | 0.61 | 0.69 |
| strings | 1.294 | 1.575 | 0.684 | 2.262 | 0.82 | 0.57 |

Geometric mean of MiniJS / node --jitless: **0.53** (0.61 before the
pass, same machine and script, earlier the same day); of MiniJS /
QuickJS: **0.40** (0.47 before). The weakest ratios to V8's interpreter
went from 0.97 (strings), 0.93 (json) and 0.87 (closures) to 0.82, 0.64
and 0.70. MiniJS is faster than V8's bytecode interpreter (Ignition) and
than QuickJS on all 14 clean-room benchmarks. V8 with its optimizing
JITs is still 1.5–12× faster. Node's times include about 40 ms of start-up.

### Before the pass (commit c9fa2de)

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

## Changelog: performance pass

Before → after is measured with interleaved runs of two binaries
(`ab.py`-style: each benchmark alternately under each binary, minimum
of 5 runs), both built with `--align-functions=64` so that unrelated
changes do not move handlers. The machine was shared with other agents
(load average 2-6), so changes under about 5% are reported as measured
but not claimed. Profiles come from a SIGPROF sampler loaded with
`LD_PRELOAD` (every handler is its own symbol).

Overall, the starting point (989d221 + master's alignment flag) against
the end of the pass, interleaved, minimum of 5 wall-clock runs:

| Benchmark | before | after | after/before |
|---|---:|---:|---:|
| awfy_micro | 0.799 | 0.769 | 0.96 |
| binary_trees | 1.326 | 1.216 | 0.92 |
| bitops | 0.902 | 0.825 | 0.92 |
| closures | 1.266 | 1.021 | 0.81 |
| deltablue | 1.032 | 0.891 | 0.86 |
| fannkuch | 0.483 | 0.466 | 0.96 |
| json | 1.354 | 0.957 | 0.71 |
| nbody | 0.584 | 0.425 | 0.73 |
| properties | 0.991 | 0.855 | 0.86 |
| raytrace | 0.609 | 0.516 | 0.85 |
| richards | 0.808 | 0.745 | 0.92 |
| sort | 0.944 | 0.855 | 0.91 |
| spectral_norm | 0.994 | 0.921 | 0.93 |
| strings | 1.496 | 1.288 | 0.86 |
| geometric mean | 0.923 | 0.801 | 0.87 |

1. **Bytecode cleanups** (2139b38). `switch` cases that are literals
   compare with `JEQK`; top-level `const`s initialized with literals are
   folded (SPEC.md forbids reading them before their declaration), so
   `lex.kind === T_STRING` is a `JEQK` and `case T_LBRACE:` a constant;
   `x++` whose value is unused drops its `TONUM`. And a fix: json's
   lexer text is a rope, so its `t.charCodeAt` sites thrashed between
   the generic member and `GETMETHOD_SP` (5M generic misses per run);
   the string member accepts ropes. json -9%.
2. **Natives inline in CALL** (51ce16c). `CALL` became a family: a site
   whose callee is `charCodeAt`, `charAt`, `push`, `Math.floor` or
   `Math.sqrt` with one argument becomes `CALL_CCA`, `CALL_PUSH`, ...,
   which compare the callee with the native's value (one load from the
   context) and run it inline; anything else takes the native call or
   the generic CALL. closures -11% (the prelude's `map`/`filter` push
   their results), sort -6%, json and strings -4%.
3. **Interner, string search, ropes in registers** (f7cdcaa, std.text).
   `text.Interner` was a `HashMap<str, u32>` at 3/4 load comparing
   strings with memcmp on every probe; it is now a half-full table of
   one word per slot (id and 32 hash bits). `indexOf`/`split`/
   `includes` used a memcmp call per position; they find the first byte
   with memchr. A string-method site with a rope receiver replaces it
   in its register with the flat string. strings -9%, json -15%,
   properties -6%.
4. **Loop tests at the bottom** (5ab9ec5). `while`, `for` and `for-of`
   jump to their test once and branch back from it: one branch per
   iteration instead of a test and a `JMP` (8% fewer instructions in
   fannkuch). Within noise by itself; it makes `ADDI; JLT` and
   `GETLEN; JLT` the loop tails that the supers in 7 fuse.
5. **Cheaper CALL and RET** (b6ac92e, 884beaf). A third of `CALL`'s
   samples in deltablue were in the register-clearing loop, which LLVM
   had vectorized with a long setup. It now clears the registers above
   the fixed parameters two at a time with a plain loop (the register
   after the window may be cleared too), missing arguments in a cold
   block, and natives are called out of line. The bottom frame is
   flagged, so `RET` tests one word for both `new` frames and the end
   of the run, and constructors return without a cold call.
   deltablue -13%, binary_trees -10%, bitops -8%, closures -5%.
6. **Float-first arithmetic** (9d0a10e), as in Mini: `ADD`/`SUB`/`MUL`
   families with `_F` (raw-double add, NaN check on the result) and
   `_IF`/`_FI` members, `JS.optimize` re-run after quickening, `DIV` on
   raw words first, and `%` by a positive Int of an integral double in
   i64 instead of `fmod`. nbody -22%, raytrace -5%.
7. **Fewer dispatches** (9b73506). A call used as an operand or returned
   is used in its call base instead of being copied out (`MOVE; RET`,
   `CALL; MOVE`); ten supers from `--vm-stats` pair counts summed over
   the suite (`GETLEN; JLT`, `ADDI; JLT`, `MOVE; CALL`,
   `GETMETHOD_P; CALL`, `MUL_F; ADD_F`, ...); `JLTI`/`JLEI`/`JGTI`/
   `JGEI` compare with an 8-bit constant. Geomean -3% (bitops,
   closures, deltablue, richards, spectral_norm -4..5%); the constant
   compares alone are within noise.
8. **Constant bit operations and `%`** (54271ee): `BANDK` ... `USHRK`,
   `MODK` save the `LOADI` of `h << 5`, `x | 0`, `i % 7`. 1-2% each on
   strings, bitops, closures, richards (within noise).
9. **Exact constructor sizes** (1833ded). `new` allocated at least 4
   inline slots; it now allocates the constructor's `this.x` count
   (objects that grow in the constructor still raise it). TreeNode
   drops from 96 to 80 bytes. binary_trees -6%.
10. **Jump tables for `switch`** (f6b0ce1). Int cases within a range of
    at most 8 per case + 32 compile to `SWITCHI` and one `JMP` per
    value; `JEQ`/`JEQK` decide two different Ints without the general
    comparison. json -3% and -4%.
11. **Absent properties and polymorphic stores** (143470f, std.obj).
    properties reads `o.z` on a dozen shapes that mostly lack it, asks
    `"kind" in o`, and stores `o.x` on a dozen shapes; all three went
    to the slow path (`find_word` was 16% of the run). std.obj now
    caches absent reads in Poly and Mega entries (opt-in,
    `ic.absent`: shape id, `ABSENT`, the prototype's validity cell),
    answers `in` through the shared MegaCache (`has_mega`), and gives
    store sites Poly and Mega states (`write_miss_in`, `poly_set`,
    `mega_set`). properties -11%.

Tried and dropped (within noise, not committed):

- Clearing eight registers unconditionally in `CALL` (no loop branch to
  mispredict) with a cold loop for bigger windows.
- An interner whose slots point at records holding the id, length and
  bytes (two cache misses per lookup instead of three; strings has
  31,000 keys, so its table does not fit in L2).
- `%` of an integral double through a double divide instead of an i64
  `idiv`.
- `#[inline(always)]` on std.obj's `poly_get` and `mega_get`.
- GC growth 400% / 16 MiB instead of 300% / 8 MiB: -2% on the
  allocation-heavy benchmarks for more memory.
- Super-instructions for the prelude's callback loop. The prelude itself
  costs little: `data.map(f)` over 6M elements takes 0.136 s, the same
  loop written by hand 0.149 s, and V8's native `map` under `--jitless`
  0.128 s; V8 runs the hand-written loop in 0.386 s.

Where the time goes now: `CALL`/`RET` (15-25% of call-heavy programs:
frame push, register clearing, the return's indirect jump), the cached
property loads (`GETPROP_IN` is a shape-id load chain), and in the
allocation-heavy programs the collector (17% of binary_trees, mostly the
sweep touching every slot header). In strings, the comparator of the
word-count sort (`counts["w_" + b]`) builds a key string, interns it
(the 31,000-key table misses the cache) and looks it up in a
dictionary-mode object: that phase alone takes 0.6 s against V8's
0.39 s, while MiniJS wins every other phase.
