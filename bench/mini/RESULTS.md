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

(See [`docs/results.md`](../../docs/results.md) for current numbers,
all dispatch modes and the JIT, measured 2026-11-03. The third phase
below, 2026-11-04, has a newer tail-call table: 0.49 of PUC-Lua's time
and 0.77 of LuaJIT -joff's by geometric mean, 0.927 of the time
before it.)

Commit fe6c642 (second phase). `--dispatch=tailcall` and `goto` are the
same source; "(before)" is Mini/-joff at the end of the first phase
(87cd496). Machine load was low for this run (load average below 2.5 at
its end); earlier in the phase other agents' builds made runs noisy.

| Benchmark | Mini tailcall | Mini goto | PUC-Lua 5.4 | LuaJIT -joff | LuaJIT | Mini/Lua | Mini/-joff | (before) |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| binary_trees | 1.250 | 1.254 | 2.538 | 1.378 | 0.953 | 0.49 | 0.91 | 0.89 |
| closures | 0.478 | 0.531 | 1.342 | 0.937 | 0.777 | 0.36 | 0.51 | 0.53 |
| fannkuch | 1.238 | 1.185 | 1.698 | 1.428 | 0.326 | 0.73 | 0.87 | 0.95 |
| fib | 1.048 | 1.063 | 1.604 | 1.137 | 0.212 | 0.65 | 0.92 | 0.94 |
| hash_strings | 1.115 | 1.168 | 1.595 | 1.066 | 0.212 | 0.70 | 1.05 | 1.07 |
| interp | 1.011 | 1.010 | 2.008 | 1.084 | 0.371 | 0.50 | 0.93 | 1.17 |
| mandelbrot | 0.696 | 0.730 | 1.190 | 0.783 | 0.157 | 0.58 | 0.89 | 1.25 |
| nbody | 0.451 | 0.521 | 1.155 | 0.524 | 0.090 | 0.39 | 0.86 | 1.13 |
| oo_methods | 0.774 | 0.836 | 1.354 | 1.008 | 0.186 | 0.57 | 0.77 | 0.79 |
| queens | 0.944 | 0.959 | 1.236 | 0.964 | 0.476 | 0.76 | 0.98 | 1.04 |
| sieve | 0.693 | 0.717 | 1.254 | 0.795 | 0.227 | 0.55 | 0.87 | 0.94 |
| sort | 0.809 | 0.816 | 1.080 | 0.834 | 0.311 | 0.75 | 0.97 | 1.07 |
| spectral_norm | 1.228 | 1.328 | 1.683 | 1.133 | 0.072 | 0.73 | 1.08 | 1.45 |
| string_build | 0.788 | 0.792 | 1.740 | 0.748 | 0.385 | 0.45 | 1.05 | 1.06 |

Mini is faster than PUC-Lua on all 14 benchmarks (0.36-0.76 of its
time) and faster than LuaJIT's assembly interpreter on 11 of them. The
three above it are within 8%: spectral_norm (1.08; was 1.45),
hash_strings and string_build (1.05; string hashing and interning, not
interpreter dispatch).

Float arithmetic is now 16 instructions per op in the hot path (ADD_F:
decode 6, two FP loads, `addsd`, `ucomisd; jp`, store, dispatch 4),
against ~10-12 for LuaJIT's ADDVV and 23 before.

`goto` is slower than `tailcall` on most benchmarks in this run
(closures, nbody, spectral_norm, oo_methods by 7-15%), faster only on
fannkuch. `tailcall` stays the default.

### Previous results (end of the first phase, commit 87cd496)

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
within 10% of LuaJIT's assembly interpreter (-joff) on 10 of them.
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

### Second phase

Measured with interleaved A/B runs (`ab.py`: each binary alternately,
minimum of 5-11 runs). The machine was shared with other agents' builds
for much of this phase, so effects under about 5% are reported as
measured but not claimed.

19. **Float fast paths test the result, not the operands** (ae91268,
    96257bf). A tagged word is a NaN as a double, so the float-first
    members add the raw words as doubles and keep the result if it is
    not NaN; otherwise they fall back to the typed match. Operands load
    straight into FP registers (`*((regs + a.raw) as *f64)`), and the
    result is stored as bits. ADD_F's hot path went from 23 to 16
    instructions. mandelbrot 0.91 → 0.79 (-14%), nbody -4..-8%.
    The same trick for compare-and-branch (`x < y` or `x >= y` holding
    means both are numbers) and for DIV was within noise.
20. **Exact mixed int/float members** (3fe4638). A site that first sees
    (Int, Num) or (Num, Int) becomes ADD_IF / ADD_FI (and SUB, MUL, and
    the K forms): one tag test on the integer, `cvtsi2sd`, the operation
    and the NaN test on the result. Anything else turns the site into the
    complete float-first member. spectral_norm 1.59 → 1.43 (-10%).
21. **RSUBK and RDIVK** (56578ee): `k - x` and `k / x` with a constant
    left operand, so `1.0 / x` needs no LOADK. spectral_norm 1.43 → 1.35.
22. **Polarity in the opcode; supers again** (e6bd5f5). JLT/JLTN etc.
    replace the polarity operand (saving a setcc and compare per branch).
    Twelve supers from `--vm-stats` pair counts summed over all
    benchmarks. interp 1.19 → 1.04 (-12%), sort -6%, fannkuch -4%,
    fib -3%. Supers alone (before the polarity split): interp -7%, sort
    -3%, others within noise.
23. **Counted integer for loops** (3c41239), as in Lua 5.4: FORPREP
    replaces the limit with the iteration count, so FORLOOP has no
    overflow check and no step-sign test. sieve -9%, queens -6%,
    spectral_norm -4%, mandelbrot -4%.
24. **Ints compared and indexed on their words** (2558ac8). `(a ^ B)`
    with B the Int tag word with the payload's sign bit set is below
    2^48 exactly for Ints and orders them as unsigned numbers, so the
    tag test and the compare share the xors; an array index is the key's
    word minus Int(1)'s word. These need no payload extraction, which is
    what made them matter for i48 (below). With i32: -2..-4% on
    fannkuch, sort, fib, queens (near noise).
25. **Fusing after quickening** (cffb314). `Mini.optimize` ran only at
    compile time, when arithmetic is generic, so supers of float members
    never formed. Each quickening transition of an arithmetic site now
    re-runs it on the function (bounded: a site changes at most twice).
    With supers for the frequent float pairs: mandelbrot -6%, nbody -5%.
26. **`#[forward]` supers** (compiler, 9ac1748). A later part's operand
    that may name the register the previous part wrote is compiled a
    second time with its field replaced by the earlier one under an
    equality test, and LLVM forwards the stored value. Applied to every
    super it cost interp +8% and fib +4% (an extra compare and branch
    in throughput-bound code), so it is opt-in. On ADDI_ADDI it gives
    spectral_norm -5..-10% (its A(i, j) is a latency chain of
    store-to-load round trips feeding a divide).

27. **Verify clauses for register counts** (fe6c642; from the
    fuzzer). CALL's argument and result counts, SETLIST's, RETN's and
    CONCAT's counts, GETINDEXI's index and SELF's base got operand kinds
    whose `verify` clauses check the covered range against the frame.
    The kinds decode to the plain field, so the handlers' machine code
    is unchanged. 10 minutes of minifuzz under ASan afterwards: 2.0M
    programs (421k verified and ran), no failures.
28. **Forwarding variants** (compiler). LLVM never forwarded a
    super's first result into its second part: the store to `regs[dst]`
    and the load of `regs[a]` use indices decoded from two words, which
    no alias metadata can relate (when they are equal the load must see
    the store). The test-and-branch of 26 was the only way, and it cost
    more than it saved outside latency chains (with the float supers
    marked: mandelbrot +2%, nbody +4%, JIT nbody +15%). A `#[forward]`
    super now gets an opcode per set of linked operands (`MUL_F_ADD_F__f2`),
    picked by `optimize` where the fields are equal, with no run-time
    test; every Mini, MiniJS and Scheme super is `#[forward]` (Mini +45
    opcodes, binary +22%). Best of 5, interleaved, `--align-functions=64`,
    against the same compiler without the variants: Mini 0.982 by
    geometric mean (mandelbrot 0.93, spectral_norm 0.96, fannkuch 0.97,
    none slower than 1.005), with the JIT 0.973 (mandelbrot 0.92,
    spectral_norm 0.93, queens 0.96). MiniJS 0.989 (nbody 0.95), JIT
    0.984 (bitops 0.93, json 0.93). Scheme (best of 3, 41 programs) 0.967
    (compare-and-`TEST` pairs branch on the compare's flags), JIT 1.00.
    The forwarded operand is a copy of the earlier field, not the same
    local: cold blocks decode fields again from the current word, which
    gave 26's ADDI_ADDI a wrong register on its overflow path.

Tried and dropped:

- **Lazy register clearing** (CALL clears only missing parameters;
  `scan_roots` clears stale slots between the top frame and a
  high-water mark). Twice measured: fib -5%, closures -3%,
  binary_trees +3%, spectral_norm +1%. Not worth the GC subtlety.
- **Float-first compares** (again) and DIV with direct FP loads: within
  noise.
- **Forwarding on all supers**: see 26.
- **Fixed four-slot register clearing in CALL** and **GETINDEX's
  generic path in a cold block** (removes the hot path's stack frame):
  both within noise in the noisy part of the phase; not committed.
- **Supers only spectral_norm uses** (MUL;MULK_IF, ADD_FI;RDIVK, ...):
  no measurable effect, removed.

### Third phase (2026-11-04)

Aimed at the benchmarks where LuaJIT's interpreter was closest:
string_build (1.06 of -joff at the start), queens (0.98), sort (0.96),
sieve (0.94), spectral_norm (0.93), hash_strings (0.89), and the
coroutine suite. A/B numbers are user+sys CPU time, minimum of 5-9
interleaved runs, `--align-functions=64`, against the build just before
(the host was loaded, 1-minute load average 2-5; wall-clock runs moved
by up to 10%).

At the end of the phase (b2fbc43; the reorder of item 36 came after),
user+sys CPU seconds, minimum of 7 interleaved runs, load average about
3; "before" is the phase's starting commit (cdb5024), built the same way:

| Benchmark | Mini | Mini (before) | LuaJIT -joff | PUC-Lua 5.4 | Mini/Lua | Mini/-joff | before/-joff | Mini/before |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| binary_trees | 0.765 | 0.758 | 1.369 | 2.489 | 0.31 | 0.56 | 0.55 | 1.009 |
| closures | 0.446 | 0.455 | 0.963 | 1.240 | 0.36 | 0.46 | 0.47 | 0.980 |
| fannkuch | 1.120 | 1.215 | 1.396 | 1.667 | 0.67 | 0.80 | 0.87 | 0.922 |
| fib | 0.970 | 0.978 | 1.122 | 1.601 | 0.61 | 0.86 | 0.87 | 0.992 |
| hash_strings | 0.978 | 1.069 | 1.019 | 1.643 | 0.60 | 0.96 | 1.05 | 0.915 |
| interp | 0.906 | 0.965 | 1.113 | 2.008 | 0.45 | 0.81 | 0.87 | 0.939 |
| mandelbrot | 0.637 | 0.642 | 0.781 | 1.205 | 0.53 | 0.82 | 0.82 | 0.992 |
| nbody | 0.443 | 0.464 | 0.517 | 1.152 | 0.38 | 0.86 | 0.90 | 0.955 |
| oo_methods | 0.683 | 0.795 | 1.025 | 1.382 | 0.49 | 0.67 | 0.78 | 0.859 |
| queens | 0.884 | 0.924 | 0.945 | 1.233 | 0.72 | 0.94 | 0.98 | 0.957 |
| sieve | 0.620 | 0.749 | 0.900 | 1.339 | 0.46 | 0.69 | 0.83 | 0.828 |
| sort | 0.752 | 0.830 | 0.871 | 1.148 | 0.66 | 0.86 | 0.95 | 0.906 |
| spectral_norm | 1.087 | 1.093 | 1.161 | 1.796 | 0.61 | 0.94 | 0.94 | 0.995 |
| string_build | 0.634 | 0.828 | 0.820 | 2.135 | 0.30 | 0.77 | 1.01 | 0.766 |
| **geomean** | | | | | **0.49** | **0.77** | 0.83 | **0.927** |
| coro chameneos | 0.392 | 0.442 | 0.392 | 1.029 | 0.38 | 1.00 | 1.13 | 0.887 |
| coro coroutine_ring | 0.357 | 0.347 | 0.374 | 1.007 | 0.35 | 0.95 | 0.93 | 1.029 |
| coro sieve | 0.363 | 0.368 | 0.381 | 1.062 | 0.34 | 0.95 | 0.97 | 0.986 |
| **coro geomean** | | | | | **0.36** | **0.97** | 1.00 | **0.966** |

Mini now takes less time than LuaJIT's interpreter on all 14 programs
(hash_strings 0.96, queens 0.94 and spectral_norm 0.94 are the closest;
string_build went from 1.01 to 0.77) and ties it on chameneos. In
other runs of the same builds binary_trees, fib and coroutine_ring were
0.98-1.01 of before: within noise.

29. **Word-at-a-time string hash** (a553d62, std). `std.hash.bytes` was
    FNV-1a, a dependent multiply per byte. It now takes eight bytes per
    multiply (overlapping reads for the tail and for strings under 8
    bytes, the length in the seed) and mixes as before. Mini hashes
    every string it makes, to intern it: string_build 0.92. MiniJS,
    Python: within ±2% (MiniJS strings 0.98).
30. **One buffer for the string natives** (3a304d7). string.char, rep,
    format, table.concat and tostring built each result in a new
    StrBuf (malloc, regrowth, free); they now borrow one kept in the
    intern table's `Strings` (a nested user gets a fresh one).
    string.char with one code uses a stack byte. The intern table's
    sweeper prefetches the string 16 slots ahead (~1%).
    string_build 0.93.
31. **`%` by multiplication** (2646566, 1c3e20b). MODK caches
    Granlund-Montgomery's m and s in an inline cache, so
    floor(x / k) = (x * m) >> s for 0 <= x < 2^31 (3 cycles instead of
    idiv's ~26). MOD caches them for the first divisor a site sees and
    gives up on a second one: recomputing at every miss (a 64-bit
    division) made the coroutine sieve, whose `n % p` sees every prime,
    12% slower. closures 0.96, oo_methods 0.88.
32. **Rotated while loops, threaded jumps** (0394dac, compiler). A
    while loop with a condition of at most 16 words (and no function
    literal) compiles the condition again after the body, re-lexed from
    the saved lexer state, as a test that jumps back while it holds:
    no JMP per iteration. `Parser.finish` threads jumps to JMPs
    (`std.bc` `thread_jumps`: the end of an if branch at the end of a
    loop body). Neither under `--cfg debug`. sort's JMPs went from 29M
    to 9M: sort 0.977, interp 0.971, hash_strings 0.982. (Found a latent
    bug: `jump_if` on `not x` emitted TESTT whatever the polarity.)
33. **Appends in place** (b23667e). `t[#t + 1] = v` with room in the
    array part and no hash part is stored by SETINDEX/SETINDEXK
    themselves. sieve 0.895.
34. **Unequal non-numbers compare false inline** (73c2c01; in EQ/EQK and the JEQ
    family, which went to `raw_equal` in a cold block whenever the
    words differed): chameneos 0.97.
35. **Supers** (73c2c01, 826586b, ae8f557): GETUPVAL;GETINDEX;TESTT (queens 0.97), GETINDEX;
    GETINDEX;TEST and ADDI;SETINDEX (the counting idiom of
    hash_strings: 0.98), GETUPVAL;ADDI;CALL (`f(n - 1)` with `f` an
    upvalue: fib 0.95, coroutine_ring 0.98).
36. **math.sqrt and math.floor without the native call** (367c61a), at the start
    of CALL's native cold block (floor by truncating, since libm's is a
    call): nbody 0.95. In the hot part of CALL the same code made
    closures 4-5% slower (code layout; the closure path's code was the
    same), so it stays cold, after the coroutine.resume/yield tests
    (in front of them it cost coroutine_ring ~3%; moving it, adc4f0d:
    chameneos 0.96).
37. **string.format's plain `%d`** (6a78cd0) with `push_int` instead of
    snprintf: string_build 0.95.
38. **Frameless table and return handlers** (6e611d3, 11a6663). Plain calls on rare paths
    gave whole handlers a stack frame (the "Remaining issues" item
    below): the table barrier's `log_table`, `table_set` and
    `table_get` in SETINDEX, SETINDEXK, SETFIELD, GETINDEX, GETINDEXI
    and supers containing them, and `close_upvals` in the returns. The
    barrier's slow path and the generic paths are now cold blocks. A
    cold block that falls through may stay inline with its call (the
    returns' did), so theirs now ends in `redo`, which runs the return
    again with the upvalues closed.
    sieve 0.92, fannkuch 0.95, sort 0.94, hash_strings 0.98; closures
    0.95 from the returns.
39. **Constants in multiple assignment** (6f18646) stay unloaded until stored:
    `cols[c], d1[a], d2[b] = true, true, true` is three SETINDEXK
    instead of three LOADBOOLs and SETINDEXes. queens 0.96 (fannkuch,
    whose code is unchanged, 1.02: layout).
40. **Minor collections sweep only young strings** (b2fbc43). The intern table's
    sweeper read the header of every live string in every collection
    (a cache miss each) and scanned the whole table (up to 8 MB in
    string_build). `Strings.young` now lists the strings made since the
    last sweep; in a minor collection only those can die (old objects
    count as marked), so the sweeper checks them and finds a dead one's
    slot from its hash (dead objects are intact until the sweepers have
    run). Strings over 400 bytes may be large objects, young for two
    collections, so survivors that long stay listed. sweep_strings 8.3%
    → 3.8% of string_build's samples; string_build 0.92-0.93 (two
    rounds of 11 runs).

Tried and dropped:

- **Supers** SETINDEXK;FORLOOP and GETINDEX;TESTT (sieve 2.5% slower),
  ADD;SUB (queens within 1%).
- **Intern table shrink thresholds**: shrinking when live * 32 < cap
  instead of live * 8 (fewer rehashes, a larger table): string_build
  1.07; when live * 4 < cap: 1.04, hash_strings 1.015.
- **LEN without its frame** (string fast path, `length` in a cold
  block): within 1%.
- **No register clearing in CALL**, as an upper bound (unsafe as is):
  fib 0.92, binary_trees and spectral_norm 0.98, others within 2%.
  Doing it safely needs MiniJS's high-water mark across coroutine
  stacks, nested runs and the debugger; not attempted.

### Fourth phase (2026-11-05)

From the third phase's next candidates. A/B numbers are user+sys CPU
time, minimum of 5-7 interleaved runs, `--align-functions=64`, against
the build before (5cb7048); the host was loaded (load average 1-5), and
moving the handlers by a few KB (a compiler change elsewhere in the
binary) moved single benchmarks by up to 4-5% either way, so suspects
were rerun with `--align-functions=128`.

At the end (02087de), minimum of 7 interleaved runs, load average about
4; "before" is 5cb7048:

| Benchmark | Mini | Mini (before) | LuaJIT -joff | PUC-Lua 5.4 | Mini/Lua | Mini/-joff | before/-joff | Mini/before |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| binary_trees | 0.771 | 0.763 | 1.444 | 2.981 | 0.26 | 0.53 | 0.53 | 1.010 |
| closures | 0.437 | 0.463 | 0.939 | 1.232 | 0.35 | 0.47 | 0.49 | 0.944 |
| fannkuch | 1.152 | 1.155 | 1.430 | 1.716 | 0.67 | 0.81 | 0.81 | 0.997 |
| fib | 0.938 | 0.934 | 1.122 | 1.602 | 0.59 | 0.84 | 0.83 | 1.004 |
| hash_strings | 0.986 | 0.983 | 1.029 | 1.619 | 0.61 | 0.96 | 0.96 | 1.003 |
| interp | 0.746 | 0.907 | 1.085 | 2.004 | 0.37 | 0.69 | 0.84 | 0.822 |
| mandelbrot | 0.641 | 0.641 | 0.783 | 1.200 | 0.53 | 0.82 | 0.82 | 1.000 |
| nbody | 0.447 | 0.446 | 0.520 | 1.168 | 0.38 | 0.86 | 0.86 | 1.002 |
| oo_methods | 0.691 | 0.691 | 1.035 | 1.397 | 0.49 | 0.67 | 0.67 | 1.000 |
| queens | 0.886 | 0.878 | 0.994 | 1.392 | 0.64 | 0.89 | 0.88 | 1.009 |
| sieve | 0.626 | 0.618 | 0.930 | 1.405 | 0.45 | 0.67 | 0.66 | 1.013 |
| sort | 0.755 | 0.753 | 0.870 | 1.133 | 0.67 | 0.87 | 0.87 | 1.003 |
| spectral_norm | 1.077 | 1.077 | 1.163 | 1.744 | 0.62 | 0.93 | 0.93 | 1.000 |
| string_build | 0.652 | 0.666 | 0.903 | 2.335 | 0.28 | 0.72 | 0.74 | 0.979 |
| **geomean** | | | | | **0.47** | **0.75** | 0.76 | **0.984** |
| coro chameneos | 0.376 | 0.380 | 0.383 | 1.016 | 0.37 | 0.98 | 0.99 | 0.989 |
| coro coroutine_ring | 0.363 | 0.348 | 0.381 | 1.008 | 0.36 | 0.95 | 0.91 | 1.043 |
| coro sieve | 0.362 | 0.366 | 0.390 | 1.094 | 0.33 | 0.93 | 0.94 | 0.989 |
| **coro geomean** | | | | | **0.35** | **0.95** | 0.95 | **1.007** |

Both changes are in the compiler; only interp's code changed among the
benchmarks (closures, coroutine_ring: noise; their bytecode is the same).
The ratios to the references moved from the third phase's table because
the references ran faster under this load (Mini 0.49 → 0.47 of Lua).

41. **Constant locals** (2c8b317, compile.lm). A local declared with a
    constant (`local PUSH, LOAD = 1, 2`) that no statement assigns is
    read as the constant by the functions nested in its scope: no
    upvalue, and `op == PUSH` is one JEQKN instead of GETUPVAL and JEQN.
    Whether a local is assigned is known only at the end of its scope,
    so `compile` runs the parser twice: the first pass records the
    assigned locals (`store_var` through a register or an upvalue
    chain, by declaration number) and its code is dropped. Not under
    `--cfg debug` (the debugger lists upvalues and can assign). interp
    0.92 at both alignments; no other benchmark reads such an upvalue
    in a loop.
42. **Jump tables for if/elseif chains** (02087de). `if x == K1 then ..
    elseif x == K2 ..` with at least four arms, `x` a local and the
    keys integer numerals or constant locals (41) spanning at most 64
    values in 0..255, compiles to `SWITCH` and n + 1 JMPs (MiniJS's
    SWITCHI): an Int, or a float of integer value, in range goes to its
    arm's JMP, anything else to the else block or out; a repeated key
    goes to the first arm, as the tests would. At `if`, the parser
    looks ahead over the tokens (block depth counted by
    `if/function/do/repeat` against `end/until`) for the conditions.
    interp's dispatch loop is such a chain of 15 arms (JEQKN was 35% of
    its samples after 41): 0.84 on top of 41. `SWITCH` jumps to the
    JMP, one dispatch more than jumping to its target: decoding the
    target in SWITCH was 4% faster on interp but crashes the
    `--jit-stencils` build, whose runtime handles jumps to computed
    instructions, not to their targets. Not under `--cfg debug`.

Tried and dropped:

- **Lazy register clearing, safely** (third attempt). `Ctx.hw` per
  thread (in `Coro` and `CtxThread` while a thread is not running,
  relocated by `grow`), raised by every window creation (CALL's
  overflow compare against `hw` instead of `stack_end`, its slow path
  out of line; `call_value`, coroutine starts, `run`); `scan_stack`
  marks the frames, clears from the highest window up to the mark and
  lowers it, for the running thread, its resumers, suspended coroutines
  (`trace_coro`: a suspend logs the coroutine, so the next collection
  traces it) and saved `--cfg gil` threads; CALL clears only the missing
  parameters. Running all tests and benchmarks with a poison value in
  the uncleared registers changed no output (the compiler writes every
  register before reading it; the debugger shows locals only in their
  live ranges). Result: fib 0.955, closures 0.95 (1.00 at
  `--align-functions=128`), but oo_methods 1.04-1.05 and coroutine_ring
  1.03-1.04 in every run, at both alignments and with the stack's data
  offset moved by 2 KB; overall 0.995-1.003. The same build clearing
  the whole window again had oo_methods at 1.00: the clearing stores
  themselves make its GETFIELD and SELF faster (their register loads;
  Skylake, not explained). Not committed (the patch is described here).
- **A balanced compare tree** for if chains, considered instead of 42:
  interp orders its hot opcodes first, so a tree's ~4 compares would
  not beat the chain's average.

## Value accumulators (2026-11-06)

43. **A value accumulator** (speed audit item 1, `language.md` §7.4):
    `accumulator vacc: f64 for reg;` over the float members, DIV/DIVK
    and the register and constant compare-and-branch forms (20 ops, 28
    reader variants), with ten `#[forward]` supers over the variants,
    selected again in `refuse` and at the end of `finish`
    (`first_temp` 0). 233 of 256 opcodes. `mini.vmprofile` regenerated.
    User+sys CPU, median of 9 paired interleaved rounds (load 4-6 from
    other agents; min-of-runs varied ±10% between identical binaries),
    against base (A/A in brackets):

    | | align 64 | align 128 |
    |---|---:|---:|
    | mandelbrot | 0.912 (1.012) | 0.902 (0.996) |
    | nbody | 0.950 (1.022) | 0.973 (0.977) |
    | queens | 1.033 (0.994) | 0.987 (1.000) |
    | sort | 1.025 (0.988) | 1.012 (0.994) |
    | sieve | 1.015 (0.992) | 1.043 (1.003) |
    | fib | 1.004 (1.013) | 1.043 (0.983) |
    | spectral_norm | 1.027 (1.004) | 0.986 (0.997) |
    | suite (14) | 0.994 (1.000) | 0.996 (0.996) |

    mandelbrot's loop is six dispatches, five of them variants or supers
    of them. The cost elsewhere is the dispatch: a seventh register
    argument (the `f64`, `xmm0`) stops LLVM folding the table load into
    the `jmp` (`lowering.md` §3), one more instruction in every
    handler. A control build with the accumulator declared but nothing
    selected (`accumulate DIVK;`, no `Mini.accumulate`) measured fib
    1.038, queens 1.022, sort 1.007, sieve 1.007 (geomean of eight
    benchmarks 1.006). Mini's suite still comes out ahead or even, so the
    port stays; MiniJS's did not (`bench/minijs/RESULTS.md`).

44. **The dispatch fold: recovered, measured, not kept; it was not the
    cost.** The cause of the lost fold is LLVM's x86 isel pattern
    `X86tcret_6regs`, which counts the tail call's register operands
    (`lowering.md` §3). A pass that folded the table load back into the
    `jmp` in the assembly (commit 0c3bb46, reverted) gave Mini's
    handlers 1043 `jmp *T(,%rax,8)` and 8 `jmp *%reg` (CALL supers whose
    index register is reassigned before the jump), against 44 and 1007;
    MOVE was `movzbl %sil,%eax; jmp *T(,%rax,8)` again, as without the
    accumulator. Measured with base = item 43's base (no accumulator),
    the control build (`accumulate DIVK;`, nothing selected) and the
    current Mini, each unfolded and folded; user+sys CPU, median of
    paired interleaved rounds (11 at align 64, 9 at 128), against base:

    | | A/A | Mini | Mini folded | control | control folded |
    |---|---:|---:|---:|---:|---:|
    | fib, 64 | 0.998 | 1.002 | 1.000 | 1.004 | 0.998 |
    | queens, 64 | 0.978 | 1.027 | 0.998 | 1.002 | 1.027 |
    | mandelbrot, 64 | 0.981 | 0.906 | 0.924 | 0.987 | 0.990 |
    | suite (14), 64 | 0.998 | 0.999 | 0.994 | 1.005 | 1.005 |
    | fib, 128 | 1.004 | 1.017 | 1.014 | 0.995 | 1.008 |
    | queens, 128 | 0.996 | 0.989 | 1.001 | 1.001 | 1.006 |
    | mandelbrot, 128 | 1.001 | 0.907 | 0.901 | 0.989 | 1.003 |
    | suite (14), 128 | 1.002 | 0.993 | 0.996 | 1.003 | 1.005 |

    Folded over unfolded: 0.996 and 1.001 (Mini), 1.000 and 0.999
    (control). Item 43's control result (fib 1.038, queens 1.022) does
    not come back, folded or not: the control is within half a percent
    of base, about the A/A spread, so that result was noise. A
    microbenchmark agrees (four small handlers, 10^9 dispatches: 6, 7
    and 7-folded arguments within 2.5% of each other, 7 unfolded the
    fastest): the dispatch waits on its two dependent loads, and the
    extra `mov` is a µop the front end has room for. The pass was not
    kept: no gain here, and on the wasm interpreter (nine arguments,
    which it folded too) 1.022 at align 64, spectralnorm 1.08 at both
    alignments. What the unselected accumulator does change is `xmm0`
    saved around the C calls of a few hot handlers (GETGLOBAL,
    NEWTABLE, CONCAT, POW) and a `xorps` on computed jumps.

## SWITCH to the target; quickened coroutine switches (2026-11-06)

Speed audit items 13 and 11 (`docs/design/speed-audit.md`). User+sys
CPU, minimum of 11 interleaved runs, against the build before; A/A in
brackets.

45. **SWITCH straight to its arm** (item 42's next candidate). SWITCH
    decodes the JMP it picks and jumps to its target: one dispatch
    less. The `--jit-stencils` crash was that decode reading a compiled
    JMP's enter word; `Vm.real_word` is now an inline table load and
    computed jumps enter compiled code directly (item 13). Where the
    JMP is compiled, SWITCH jumps to it (its code is one direct `jmp`).
    interp 0.977 (0.998) at align 64, 0.947 at 128; under the JIT 1.00.
46. **Coroutine switches on the hot path** (item 11). CALL is a family;
    a site that switched threads in the run quickens to CALL_RESUME,
    CALL_WRAP or CALL_YIELD, which switch without the native cold
    block, and fall back with `become CALL; redo`. Supers
    `MOVE, MOVE, CALL_RESUME`, `MOVE, CALL_RESUME`, `GETUPVAL,
    CALL_YIELD`. chameneos 0.925 (align 128: 0.919), coroutine_ring
    0.961 (0.971), sieve 0.944 (0.927); the 14 programs 0.994 (A/A
    1.001). chameneos is 0.91 of LuaJIT `-joff`, coroutine_ring 0.89,
    sieve 0.88.

Tried, as an upper bound (not correct in general): the switch's
bookkeeping cut to the minimum (no barriers, status, open upvalues or
retired stacks) on top of 46: chameneos 0.860, coroutine_ring 0.873,
sieve 0.902 of the build before 46.

## Int(i48): measured, not adopted

Mini keeps 32-bit integers (SPEC §2.1). Switching the payload to `i48`
(every `as i32` on Int payloads, range checks, NEG's minimum, FORPREP's
clamp, the lexer's integer range) passes all conformance tests; the
only expectation changes would be the i32 overflow-to-float lines in
basics.mini and lib_math.mini (where Lua then agrees). Cost against the
i32 build at the same commit, interleaved runs:

| | plain i48 | i48 + word tricks (24) + i64 `%` |
|---|---:|---:|
| closures | +8% | +9% |
| spectral_norm | +6..7% | +4% |
| queens | +4..5% | +3% |
| sieve | +3..7% | +2% |
| interp | +5..6% | +1% |
| fib | +2..6% | ±1% |
| others | ±3% | ±1% |

Where the cost comes from (x86-64, nanbox with the Int tag above bit 48):

- Overflow-checked `+ - *` shift both operands to the top of the word,
  add, `jo`, shift back and re-tag: 6 instructions where i32 needs 4
  (`addl; jo; movl; or`). This is inherent in a 48-bit payload; it is
  why LuaJIT's dual-number mode uses int32.
- int→float conversion must sign-extend the payload first (`shl; sar;
  cvtsi2sd`): +2 on every mixed operation (spectral_norm's A(i, j) has
  two per call).
- `x % y` on i48 is guarded for MIN % -1 in a way LLVM cannot fold even
  after `y > 0` is tested (the test is on the shifted payload). Doing
  it in i64 fixes that (+4 instead of +12 instructions in MODK) but
  costs the sign extensions.
- Wrapping adds (FORLOOP's index) need an extra mask (`and`).

Compares and array indexing cost nothing extra once done on words
(item 24). Because the remaining costs are per arithmetic instruction,
and two benchmarks stay above 4%, the switch was not made.

## Int(i48) on `nanbox(int_high)`: measured, not kept (2026-11-06)

The layout that was to remove the i48 costs above (speed audit item 7):
`value V repr nanbox(int_high)` with `Int(i48)`, commit 799eecc,
reverted by the next. Checked `+ -` on two Ints become `add; jo` on
the words, compares word compares, pointer payloads `shr $16`. The
float-first members (ADD_F and the like, DIV) read a register as
`rotr(w ^ V.Num.xor, 16)` and write `rotl(r, 16) ^ V.Num.xor`;
`int_index`, the integer compare-and-branch, FORLOOP and the intern
table's slot words were rewritten for the rotated words, and four test
lines that overflowed i32 into floats print integers (as Lua 5.4).
User+sys CPU, minimum of 9 interleaved runs per benchmark, against base
(master), A/A (a copy of base) in brackets:

| | align 64 | align 128 |
|---|---:|---:|
| fannkuch | 0.886 (0.998) | 0.843 (1.016) |
| queens | 0.883 (1.000) | 0.920 (1.001) |
| hash_strings | 0.922 (1.022) | 0.904 (0.966) |
| sort | 0.932 (0.982) | 0.926 (1.001) |
| sieve | 0.949 (0.999) | 0.938 (1.010) |
| closures | 0.959 (1.082) | 0.973 (1.016) |
| spectral_norm | 1.183 (0.968) | 1.196 (0.998) |
| nbody | 1.329 (1.012) | 1.315 (0.999) |
| mandelbrot | 1.799 (0.977) | 1.801 (1.000) |
| suite (14) | 1.034 (1.003) | 1.030 (1.001) |

Not kept. The integer benchmarks gain 3-16%, so i48 no longer costs
them anything (against the 4-9% above), but the float kernels lose
more: mandelbrot's float supers (`MULK_F; MUL_F__RAR`, `ADD_F__RAR;
SUB_F` and the others, `--vm-prof`: 0.69 s to 1.12 s) each read two or
three registers, and a register operand that was one `mulsd` from
memory is now a load, `xor`, `rol` and `movq`, with two more
instructions before each store.

## Remaining compiler and codegen issues

- Supers of quickened members only form if something re-runs
  `Vm.optimize` after `become` (Mini now does it by hand). Resolved:
  `refuse CODE;` and `Vm.refuse` fuse supers again over a `become`.
- `x % y` on `iN` wider than 32 bits keeps the MIN % -1 guard even when
  `y > 0` was tested on the same value (the test is on the shifted
  payload, the guard on the extended one). Resolved: the division
  checks now test the shifted operands (`lowering.md`), so the guard
  folds away.
- Hot handlers that make a plain call on some path get a stack frame
  (`push %rbp`) on every path; only `cold` keeps it off, reliably only
  for a cold block that ends in a control word (`next`, `redo`, ...):
  one that falls through may stay inline with its call (item 38).
- `cvtsi2sd` into a register with a stale upper half (no `xorps`
  break): a false dependency in the mixed int/float members.
- No way to say "this pointer is dereferenceable" (load facts were
  prototyped 2026-11-06, speed-audit item 4: no handler changed, no
  gain). Verify clauses that relate two operands decode the word
  themselves (`Mini.ABC_a(*pc)`).
