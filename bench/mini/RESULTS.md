# Mini performance log

How fast Mini (the Lua 5.4 subset in `examples/mini`) is against
PUC-Lua and LuaJIT's interpreter, and which changes made it so. For
readers checking the claims; the full current run is in
[`docs/results.md`](../../docs/results.md).

## Current state

From the 2026-11-07 run at 9dce087 (Intel Core i7-6700, quiet machine,
minimum wall-clock time of 3 interleaved runs; method in
`docs/results.md`). Ratios are Mini's time over the reference's: below
1, Mini is faster.

| Build | vs PUC-Lua 5.4.9 | vs LuaJIT 2.1 `-joff` |
|---|---:|---:|
| tail-call, 14 programs | 0.49 (0.31–0.71) | 0.75 (0.46–0.93) |
| JIT (`--jit-stencils`), 14 programs | 0.37 | 0.56 |
| tail-call, 3 coroutine programs | 0.33 | 0.90 |
| JIT, 3 coroutine programs | 0.25 | 0.69 |

Mini takes less time than both references on all 14 programs. The
closest to LuaJIT `-joff` are queens (0.93) and spectral_norm (0.92).

## Progress

Geometric means of Mini (tail-call) over each reference, at the end of
the phases that measured the whole suite:

| When | Commit | vs PUC-Lua | vs LuaJIT `-joff` | Conditions |
|---|---|---:|---:|---|
| End of third phase, 2026-11-04 | b2fbc43 | 0.49 | 0.77 | user+sys CPU, min of 7 interleaved runs, load ~3 |
| End of fourth phase, 2026-11-05 | 02087de | 0.47 | 0.75 | same, load ~4 |
| Full run, 2026-11-07 | 9dce087 | 0.49 | 0.75 | wall clock, min of 3, quiet machine |

The references' own times moved with machine load, so compare ratios,
not seconds, between rows. At the start, Mini took longer than PUC-Lua
on four programs:

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

Times in seconds, on the shared 8-core development machine.

## How changes were measured

- **First phase:** minimum of 3 or more runs of
  `bench/mini/compare.py -n 3`, wall clock, on a shared machine. Runs
  vary by about ±5%, so smaller changes are not claimed.
- **Second phase:** interleaved A/B runs (`ab.py`, minimum of 5–11),
  on a machine often busy with other builds.
- **From the third phase (2026-11-04):** user+sys CPU time, minimum of
  5–11 interleaved runs, `--align-functions=64`, against the build just
  before. Load average was 1–6. Suspects were rerun at
  `--align-functions=128`, since moving the handlers by a few KB moved
  single programs by up to 4–5%.

Profiles come from a small SIGPROF sampler loaded with `LD_PRELOAD`
(`perf` is not allowed on this machine).

## Changes that mattered

Effects are new time over old for the named programs, or before → after
in seconds. Changes under about 5% were within noise in the earlier
phases.

### First phase

| Change | Commit | Effect |
|---|---|---|
| GC: size-class allocator (per-class free lists in 64 KiB chunks, instead of `calloc`/`free` per object) | 3542a39 | binary_trees 3.40 → 1.61–1.84, closures 0.77 → 0.48, string_build 1.35 → 1.00 |
| One allocation per table, raw pointers to array and hash storage | f77f8f7 | binary_trees 1.61–1.84 → 1.36 |
| Interned strings: equality is one compare | d9489cc | hash_strings 2.41 → 1.41, nbody 1.38 → 1.13; string_build 1.00 → 1.08 |
| No NaN canonicalization for floats the compiler proves NaN-safe | b67ccac | mandelbrot 1.37 → 1.22, spectral_norm 1.87 → 1.78 |
| Constant-operand ops (ADDK, EQK, GETINDEXI, …) | 26198ea | mandelbrot 1.22 → 1.14 |
| Inline caches for GETFIELD, SETFIELD, SELF | cec5384 | nbody 1.07 → 0.64, oo_methods 1.11 → 0.83 |
| SETINDEXK (store a constant directly) | 3326740 | sieve 0.88 → 0.80 |
| Conditions compile to jump chains | 77412e6 | queens 1.20 → 0.98, interp 1.42 → 1.34 |
| ADD/SUB/MUL quicken to a float-first member when a site sees floats | 9d79b21 | mandelbrot 1.13 → 0.96, nbody 0.65 → 0.57, int programs unchanged |
| Mixed int/float operands also quicken to float-first | c61b3d4 | spectral_norm 1.68–1.74 → 1.66 |
| String keys go straight to the hash part | c34c44d | hash_strings 1.40 → 1.13 |
| Compare-and-branch ops (JEQ/JLT/JLE) | b73ffa0 | interp 1.33 → 1.16, fib 1.10 → 1.01, sort 0.92 → 0.88 |
| Multi-operand CONCAT (no interned intermediates) | c6addb9 | string_build 1.06 → 0.81 |

### Second phase

| Change | Commit | Effect |
|---|---|---|
| Float fast paths add the raw words and test the result for NaN (ADD_F: 23 → 16 instructions) | ae91268, 96257bf | mandelbrot 0.91 → 0.79, nbody −4 to −8% |
| Exact mixed int/float members (ADD_IF, ADD_FI, …) | 3fe4638 | spectral_norm 1.59 → 1.43 |
| RSUBK, RDIVK (`k - x`, `k / x`) | 56578ee | spectral_norm 1.43 → 1.35 |
| Branch polarity in the opcode; 12 supers from `--vm-stats` | e6bd5f5 | interp 1.19 → 1.04, sort −6%, fannkuch −4% |
| Counted integer for loops, as in Lua 5.4 | 3c41239 | sieve −9%, queens −6%, spectral_norm and mandelbrot −4% |
| Ints compared and indexed on their raw words | 2558ac8 | −2 to −4% on fannkuch, sort, fib, queens (near noise) |
| Supers fused again after quickening, with float supers | cffb314 | mandelbrot −6%, nbody −5% |
| `#[forward]` supers, opt-in (on ADDI_ADDI) | 9ac1748 | spectral_norm −5 to −10% |
| Forwarding variants: one opcode per set of linked operands, no run-time test | — | Mini 0.982 (mandelbrot 0.93), JIT 0.973; MiniJS 0.989; Scheme 0.967 |

Commit fe6c642 added `verify` clauses for register counts (from the
fuzzer). The machine code is unchanged; 10 minutes of fuzzing under
ASan afterwards found nothing.

### Third phase (2026-11-04)

Aimed at the programs closest to LuaJIT `-joff`: string_build, queens,
sort, sieve, spectral_norm, hash_strings and the coroutine suite. The
suite went to 0.927 of its starting time (cdb5024 → b2fbc43).

| Change | Commit | Effect |
|---|---|---|
| String hash eight bytes per multiply | a553d62 | string_build 0.92 |
| One shared buffer for the string natives | 3a304d7 | string_build 0.93 |
| `%` by multiplication, divisor cached per site | 2646566, 1c3e20b | closures 0.96, oo_methods 0.88 |
| Rotated while loops, threaded jumps | 0394dac | sort 0.977, interp 0.971 |
| `t[#t + 1] = v` appends in place | b23667e | sieve 0.895 |
| Unequal non-numbers compare false inline | 73c2c01 | chameneos 0.97 |
| Supers for upvalue calls, counting and table tests | 73c2c01, 826586b, ae8f557 | fib 0.95, queens 0.97, hash_strings 0.98 |
| math.sqrt and math.floor without the native call | 367c61a, adc4f0d | nbody 0.95, chameneos 0.96 |
| Plain `%d` in string.format without snprintf | 6a78cd0 | string_build 0.95 |
| No stack frame in table and return handlers (rare calls moved to cold blocks) | 6e611d3, 11a6663 | sieve 0.92, sort 0.94, fannkuch 0.95, closures 0.95 |
| Constants in multiple assignment stored directly | 6f18646 | queens 0.96 |
| Minor collections sweep only young strings | b2fbc43 | string_build 0.92–0.93 |

### Fourth phase (2026-11-05)

Suite 0.984 of its starting time (5cb7048 → 02087de); only interp's
code changed.

| Change | Commit | Effect |
|---|---|---|
| Constant locals read as constants by nested functions | 2c8b317 | interp 0.92 |
| If/elseif chains of four or more integer keys compile to `SWITCH` | 02087de | interp 0.84 on top of the above |

### Later (2026-11-06)

| Change | Effect |
|---|---|
| `SWITCH` jumps straight to its arm's target | interp 0.977 (align 64), 0.947 (align 128); JIT unchanged |
| Call sites that switch coroutines quicken to CALL_RESUME, CALL_WRAP, CALL_YIELD | chameneos 0.925, coroutine_ring 0.961, sieve 0.944 |

## Tried and dropped

- **Value accumulator** (2026-11-06, removed 2026-11-08): mandelbrot
  0.91, suite 0.99–1.00, but it took Mini's IR from 293k to 374k lines
  and its build from 8 s to 14 s.
- **Folding the dispatch table load back into the `jmp`** (assembly
  pass, reverted): folded over unfolded 0.996–1.001 on Mini; wasm 1.022.
- **48-bit instead of 32-bit integers:** closures +8–9%, spectral_norm
  +4–7%, queens +3–5%, from overflow checks and int-to-float sign
  extension.
- **48-bit integers with `nanbox(int_high)`** (799eecc, reverted):
  integer programs 0.84–0.96, but mandelbrot 1.80, nbody 1.33, suite
  1.03.
- **Lazy register clearing** (three attempts, the last safe across
  coroutines): fib 0.955, but oo_methods and coroutine_ring 1.03–1.05;
  suite 0.995–1.003.
- **Superinstructions from pair counts, first try:** no change beyond
  noise. Later supers only spectral_norm uses: no effect.
- **Float-first ordered compares; DIV with direct FP loads:** within
  noise.
- **`#[forward]` on every super with a run-time test:** interp +8%,
  fib +4%.
- **Mixed int/float paths in the hot arithmetic handlers:**
  spectral_norm 1.72 → 2.05; moved to the cold block.
- **Larger intern-table shrink thresholds:** string_build 1.04–1.07.
- **Breaking `cvtsi2sd` dependencies with `xorps`** (2026-11-08):
  spectral_norm 1.010 and 0.997, within the A/A spread.
- **Closure caching `code`/`k`/`nregs`; a first-probe hash fast path;
  hashing interned strings by address; LEN without its frame:** within
  noise.
