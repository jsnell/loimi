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

Commit c2dac13 (before the last two entries of the changelog, which
are within noise on this table). `--dispatch=tailcall` and `goto` are
the same source.

| Benchmark | Mini tailcall | Mini goto | PUC-Lua 5.4 | LuaJIT -joff | LuaJIT | Mini/Lua | Mini/-joff |
|---|---:|---:|---:|---:|---:|---:|---:|
| binary_trees | 1.430 | 1.455 | 2.506 | 1.387 | 0.970 | 0.57 | 1.03 |
| closures | 0.487 | 0.529 | 1.218 | 0.924 | 0.763 | 0.40 | 0.53 |
| fannkuch | 1.380 | 1.277 | 1.662 | 1.382 | 0.304 | 0.83 | 1.00 |
| fib | 1.132 | 1.200 | 1.593 | 1.121 | 0.203 | 0.71 | 1.01 |
| hash_strings | 1.404 | 1.445 | 1.604 | 1.089 | 0.212 | 0.87 | 1.29 |
| interp | 1.329 | 1.260 | 1.937 | 1.062 | 0.369 | 0.69 | 1.25 |
| mandelbrot | 0.993 | 0.975 | 1.196 | 0.785 | 0.157 | 0.83 | 1.26 |
| nbody | 0.598 | 0.573 | 1.232 | 0.504 | 0.088 | 0.49 | 1.19 |
| oo_methods | 0.823 | 0.837 | 1.327 | 0.990 | 0.183 | 0.62 | 0.83 |
| queens | 0.974 | 0.999 | 1.193 | 0.915 | 0.469 | 0.82 | 1.06 |
| sieve | 0.814 | 0.807 | 1.316 | 0.758 | 0.223 | 0.62 | 1.07 |
| sort | 0.918 | 0.924 | 1.076 | 0.834 | 0.309 | 0.85 | 1.10 |
| spectral_norm | 1.742 | 1.673 | 1.691 | 1.130 | 0.072 | 1.03 | 1.54 |
| string_build | 1.065 | 1.049 | 1.758 | 0.752 | 0.387 | 0.61 | 1.42 |

binary_trees measured 1.21-1.37 s in other runs of the same build;
spectral_norm measured 1.66-1.74 s (Mini/Lua 0.90-1.03).

**Dispatch mode.** `goto` is within noise of `tailcall` on almost every
benchmark (fannkuch and interp a little faster with goto, closures and
fib a little slower). There is no consistent winner, so the default
(`tailcall`) stays.

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
