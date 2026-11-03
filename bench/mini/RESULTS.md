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
all dispatch modes and the JIT, measured 2026-11-03.)

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

## Remaining compiler and codegen issues

- Supers of quickened members only form if something re-runs
  `Vm.optimize` after `become` (Mini now does it by hand). `become`
  could look up the super for the new neighbour pair itself.
- `x % y` on `iN` wider than 32 bits keeps the MIN % -1 guard even when
  `y > 0` was tested on the same value (the test is on the shifted
  payload, the guard on the extended one).
- Hot handlers that make a plain call on some path (GETINDEX's
  `table_get`, SETINDEX's `table_set`, CALL's clearing loop) get a
  stack frame (`push %rbp`) on every path; only `cold` keeps it off.
- `cvtsi2sd` into a register with a stale upper half (no `xorps`
  break): a false dependency in the mixed int/float members.
- No way to say "this pointer is dereferenceable", so loads cannot be
  hoisted or speculated; and verify clauses that relate two operands
  have to decode the word themselves (`Mini.ABC_a(*pc)`).
