# Mini performance log

Times are the minimum of 3 or more runs (`bench/mini/compare.py -n 3`),
in seconds, on the shared 8-core development machine. Other work runs
on the same machine, so runs vary by about ±5% (more for a few
layout-sensitive benchmarks such as spectral_norm). Changes smaller than
that are not claimed as wins.

Profiles come from a small SIGPROF sampler loaded with `LD_PRELOAD`
(`perf` is not usable here: `perf_event_paranoid` is 3), which works
well with Loimi because every handler is its own symbol.

## Current results

Commit 87cd496. `--dispatch=tailcall` and `goto` are the same source.
This run was on a busier machine than earlier ones (PUC-Lua itself ran
3-5% slower than in the starting-point run).

| Benchmark | Mini tailcall | Mini goto | PUC-Lua 5.4 | LuaJIT -joff | LuaJIT | Mini/Lua | Mini/-joff |
|---|---:|---:|---:|---:|---:|---:|---:|
| binary_trees | 1.235 | 1.285 | 2.588 | 1.394 | 0.977 | 0.48 | 0.89 |
| closures | 0.495 | 0.531 | 1.231 | 0.942 | 0.790 | 0.40 | 0.53 |
| fannkuch | 1.351 | 1.146 | 1.707 | 1.427 | 0.316 | 0.79 | 0.95 |
| fib | 1.058 | 1.131 | 1.620 | 1.120 | 0.197 | 0.65 | 0.94 |
| hash_strings | 1.187 | 1.127 | 1.640 | 1.109 | 0.234 | 0.72 | 1.07 |
| interp | 1.298 | 1.207 | 2.105 | 1.113 | 0.381 | 0.62 | 1.17 |
| mandelbrot | 0.996 | 0.935 | 1.216 | 0.797 | 0.158 | 0.82 | 1.25 |
| nbody | 0.594 | 0.565 | 1.180 | 0.527 | 0.090 | 0.50 | 1.13 |
| oo_methods | 0.809 | 0.921 | 1.381 | 1.028 | 0.189 | 0.59 | 0.79 |
| queens | 1.017 | 1.004 | 1.260 | 0.974 | 0.524 | 0.81 | 1.04 |
| sieve | 0.821 | 0.775 | 1.273 | 0.873 | 0.295 | 0.64 | 0.94 |
| sort | 0.931 | 0.893 | 1.137 | 0.871 | 0.320 | 0.82 | 1.07 |
| spectral_norm | 1.720 | 1.668 | 1.770 | 1.190 | 0.073 | 0.97 | 1.45 |
| string_build | 0.853 | 0.831 | 1.915 | 0.803 | 0.450 | 0.45 | 1.06 |

In quieter runs spectral_norm measured 1.65-1.68 s (Mini/Lua 0.89-0.91)
and binary_trees 1.21-1.33 s.

Mini is faster than PUC-Lua on all 14 benchmarks, and faster than or
within about 10% of LuaJIT's assembly interpreter (-joff) on 9 of them.
The largest remaining gaps to -joff are spectral_norm (1.45x: per-op
cost in a call-heavy float loop), mandelbrot and nbody (1.13-1.25x:
float handlers are ~18 instructions where LuaJIT's are ~10), and interp
(about 1.1x).

**Dispatch mode.** Neither mode wins consistently. In the last two full
runs `goto` was faster on fannkuch (by 10-15%), interp, mandelbrot and
sieve, and slower on closures, oo_methods (by 5-15%), fib and
binary_trees; most differences are within noise. The default
(`tailcall`) stays. Mini does not need to change it.

## Starting point

| Benchmark | Mini | PUC-Lua | Mini/Lua |
|---|---:|---:|---:|
| binary_trees | 3.398 | 2.480 | 1.37 |
| closures | 0.767 | 1.190 | 0.65 |
| fannkuch | 1.509 | 1.658 | 0.91 |
| fib | 1.153 | 1.552 | 0.74 |
| hash_strings | 2.501 | 1.566 | 1.60 |
| interp | 1.522 | 1.936 | 0.79 |
| mandelbrot | 1.367 | 1.160 | 1.18 |
| nbody | 1.454 | 1.120 | 1.30 |
| oo_methods | 1.335 | 1.338 | 1.00 |
| queens | 1.146 | 1.194 | 0.96 |
| sieve | 0.892 | 1.278 | 0.70 |
| sort | 1.015 | 1.066 | 0.95 |
| spectral_norm | 1.663 | 1.853 | 0.90 |
| string_build | 1.351 | 1.688 | 0.80 |

## Changelog

Before → after, Mini tailcall, seconds.

1. **std.gc: size-class allocator** (3542a39). Objects up to 512 bytes
   come from per-class free lists carved from 64 KiB chunks; sweep walks
   the chunks and rebuilds the lists. Was: one `calloc`/`free` per object
   and a linked list through every object.
   binary_trees 3.40 → 1.61-1.84, closures 0.77 → 0.48,
   string_build 1.35 → 1.00.
2. **One allocation per table** (f77f8f7). The constructor-sized array
   part lives in the table's trailing storage; `aptr`/`hptr` raw
   pointers to the array and hash storage save a dependent load per
   access. binary_trees 1.61-1.84 → 1.36.
3. **String interning** (d9489cc). Every string is interned in a weak
   table (a new `std.gc` sweeper hook turns dead entries into
   tombstones), so string equality and table key comparison are one
   compare. One word per slot (48-bit address + 16 hash bits), at most
   half full; the first version (two words per slot, 3/4 full) cost
   string_build 30%. hash_strings 2.41 → 1.41, nbody 1.38 → 1.13,
   oo_methods 1.28 → 1.11, string_build 1.00 → 1.08 (the cost of
   interning).
4. **No NaN canonicalization for provably NaN-safe floats** (b67ccac,
   compiler). `Num(e)` skips the `fcmp uno`/`select` when `e` is built
   from literals, int casts and values unboxed from the same nanbox with
   `+ - * /`, negation, sqrt or floor: hardware NaNs from canonical inputs
   are already valid encodings. mandelbrot 1.37 → 1.22,
   spectral_norm 1.87 → 1.78, nbody 1.13 → 1.09.
5. **Constant operands** (26198ea). ADDK SUBK MULK DIVK MODK,
   EQK LTK LEK GTK GEK (the compiler swaps `k * x`, `k < x`), GETINDEXI
   for `t[literal]`, and `if not x` / `~=` tested with TESTT.
   LT/LE also got (Num, Num) fast paths. mandelbrot 1.22 → 1.14
   (measured under heavy machine load; within noise elsewhere).
6. **Inline caches for GETFIELD, SETFIELD and SELF** (cec5384). Each
   caches the hash slot of its constant key; a hit is a range check and
   one identity compare. nbody 1.07 → 0.64, oo_methods 1.11 → 0.83.
7. **SETINDEXK, no GC check in CALL** (3326740). `t[i] = false` stores a
   constant directly: sieve 0.88 → 0.80. Removing the CALL GC check and
   adding mixed int/float fast paths to the hot arithmetic handlers were
   within noise; the mixed paths in the hot handler made spectral_norm
   18% *slower* (1.72 → 2.05), so they went to the cold block instead.
8. **Conditions compile to jump chains** (77412e6). `if`/`while`/`until`
   conditions jump straight to the false (or true) exit from each
   operand of an and/or chain; `not` flips a test. queens 1.20 → 0.98,
   interp 1.42 → 1.34.
9. **Superinstructions from `--vm-stats`** (not committed). Ten pairs
   seen in at least two benchmarks (MUL;ADD, MOVE;CALL,
   GETINDEX;GETINDEX, GETUPVAL;GETINDEX, ...) changed nothing beyond
   noise: dispatch is well predicted on this machine, so they were
   dropped.
10. **Float-first quickening for ADD/SUB/MUL** (9d79b21). Testing
    (Num, Num) before (Int, Int) everywhere made mandelbrot 1.13 → 0.99
    and nbody 0.65 → 0.59, but int code 3-5% slower (interp 1.34 → 1.40).
    So these ops became families: the generic member is int-first and
    becomes the float-first member the first time a site sees floats.
    Both are complete, so sites never switch back. mandelbrot 1.13 → 0.96,
    nbody 0.65 → 0.57, int benchmarks unchanged.
11. **GC: clear marks on free slots** (c2dac13). Robustness fix (a
    missed root could mark a free slot); cost within noise.
12. **Mixed int/float quickening** (c61b3d4). Mixed operands also
    become the float-first member, which handles them inline.
    spectral_norm 1.68-1.74 → 1.66.
13. **Closure caching `code`/`k`/`nregs`** (not committed). Saving the
    `proto` indirection in CALL was within noise.
14. **String-key paths in GETINDEX/SETINDEX** (c34c44d). String keys go
    straight to the hash part with the stored hash, and an existing key
    is overwritten in place. hash_strings 1.40 → 1.13. Tried and dropped
    (within noise): hash part at most half full instead of 3/4, a
    first-probe fast path, and hashing interned strings by address.
15. **NaN-safety through `let`; 64-bit register indexing**
    (dfcf270). The compiler's NaN-safe analysis follows immutable
    `let`s (FORLOOP's float path lost its canonicalization), and Mini's
    `regs[a + 1]` with an 8-bit operand no longer computes in u8. Both
    within noise.
16. **Compare-and-branch ops** (b73ffa0). JEQ/JLT/JLE and the K forms
    take a polarity and are followed by a JMP word with the offset.
    Conditions use them instead of compare + TEST, which stored a
    boolean, reloaded it and tested its truthiness.
    interp 1.33 → 1.16, fib 1.10 → 1.01, fannkuch 1.38 → 1.31,
    sort 0.92 → 0.88. Float-first quickening of the ordered compares
    (like item 10) was within noise and dropped.
17. **Multi-operand CONCAT** (c6addb9). `a .. b .. c` builds one
    string from consecutive registers, as in Lua, instead of creating
    and interning every intermediate. string_build 1.06 → 0.81.
18. **Field IC misses in cold blocks** (87cd496). Removes a stack frame
    from every GETFIELD hit (LLVM did not shrink-wrap the miss call).
    Within noise.
