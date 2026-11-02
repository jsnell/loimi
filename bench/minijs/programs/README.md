# MiniJS benchmarks

Each `NAME.js` is a self-contained MiniJS program (see `../SPEC.md`). The problem
size is a constant near the top of the file. Every program is deterministic,
uses its own PRNG (Park-Miller `seed * 16807 % 2147483647` or xorshift32) and
prints a short checksum. `NAME.expected` is the exact stdout produced by `node`,
and `node --jitless` produces the same output.

Run: `node --jitless NAME.js | diff - NAME.expected`

| Benchmark | What it stresses | Origin | Size constant | `node --jitless` | `node` |
|---|---|---|---|---|---|
| richards | OO task scheduler; function/prototype-style methods, many small method calls, linked lists, bit flags | Octane/V8 Richards (checks queueCount/holdCount) | `ITERATIONS=600` | 1.49 s | 0.10 s |
| deltablue | Incremental constraint solver; class hierarchy with `extends`/`super(...)`/`super.m()`, static methods, polymorphic virtual calls | Octane DeltaBlue, rewritten with ES classes (chain + projection tests) | `ITERATIONS=350`, `CHAIN_SIZE=100` | 1.54 s | 0.08 s |
| raytrace | Object-heavy float math: vector/color objects allocated per operation, inheritance for shapes/materials, recursive reflection | In the style of Octane RayTrace (own scene) | `SIZE=300` | 1.47 s | 0.12 s |
| binary_trees | Allocation of many small class instances, recursion, GC | Computer Language Benchmarks Game binary-trees | `MAX_DEPTH=16` | 1.76 s | 0.26 s |
| fannkuch | Small integer arrays, tight loops, element swaps | CLBG fannkuch-redux | `N=9`, `REPEAT=3` | 1.37 s | 0.10 s |
| nbody | Float arithmetic, property reads/writes on a few objects, `Math.sqrt`, `toFixed` output | CLBG n-body | `STEPS=400000` | 1.90 s | 0.08 s |
| spectral_norm | Float loops, many small function calls, array element access | CLBG spectral-norm | `N=700` | 1.63 s | 0.08 s |
| strings | String building with `+=`, `join`/`split`/`repeat`/`slice`/`indexOf`/`lastIndexOf`/`trim`/case conversion, RLE encode/decode, word counts in an object, `sort` | Original | `ROUNDS=190` | 1.61 s | 0.67 s |
| properties | Polymorphic/megamorphic property reads and method calls over 14+ object shapes (constructors with different field orders, class chain, literals, `Object.create` chains, computed keys, `delete`), `in`, `hasOwnProperty`, dictionary-mode objects | Original | `ROUNDS=1600`, `N_OBJECTS=2400` | 1.57 s | 0.60 s |
| closures | Higher-order functions: `map`/`filter`/`reduce`/`forEach`/`some`/`every`, compose/curry, closure counters, per-iteration `let` captures | Original | `SIZE=2000`, `ROUNDS=1200` | 1.51 s | 0.37 s |
| json | Hand-written JSON-like tokenizer (`charCodeAt`) and recursive-descent parser plus serializer over generated text; dynamic keys, `for-in`, `Object.keys`, `Number()` | Original | `ROUNDS=25`, `DOC_ITEMS=1500` | 1.53 s | 0.32 s |
| bitops | Crypto-style int32 code: SHA-256 (verified against the known digest of "abc"), CRC-32, xorshift32; `\| 0`, `>>>`, `<<`, `^`, `&`, `~`, `toString(16)` | Standard algorithms, original harness | `BLOCKS=27000` | 1.64 s | 0.08 s |
| sort | `Array.prototype.sort` with numeric, string and record comparators (relies on stability), cross-checked against hand-written quicksort, merge sort and heap sort | Original | `SIZE=20000`, `ROUNDS=6` | 1.39 s | 0.37 s |
| awfy_micro | Sieve, Permute, Queens, Towers and List: class methods, recursion, boolean arrays, linked objects | "Are We Fast Yet" micro-benchmarks | `ITERATIONS=120` | 1.62 s | 0.13 s |

Times are wall-clock for the whole process (including node start-up of about
0.04 s), the best of 3 runs, measured with node v26.13.0 on Linux x86-64 under
`ulimit -v 4000000`.
