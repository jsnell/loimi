# WebAssembly interpreter: friction report

A usability study of Loimi, for anyone improving its usability or
curious how usable it is. On 2026-11-03 a WebAssembly interpreter was
written from the docs alone (`README.md`, `docs/`, `std/`,
`loimi --help`, `tests/run/*.lm`), without reading the other example
interpreters. The interpreter was then tuned against wasm3 over several
rounds on 2026-11-04.

This study stresses things the earlier ones (Stak, regex, Lox) did not:

- untagged typed values;
- structured control flow turned into jumps at load time;
- a wasm validator that the bytecode verifier relies on;
- linear memory with bounds checks;
- `call_indirect` with signature checks, and traps.

## Main findings

- **The worst problem was silent.** A `const` name in a `match` pattern
  bound a new variable instead of comparing, and every value took the
  first arm. Only the vm verifier caught the resulting bad code. This is
  now fixed.
- **Most friction was missing pieces, not wrong design.** `f32` methods,
  `next_pc`, setting one field of an instruction word, unaligned loads:
  all were added after the study.
- **The vm verifier earned its keep.** Each time it rejected the
  translator's output, the cause was a real translator bug, caught at
  load time instead of as a wild write.
- **Speed.** The first version took 1.10× wasm3's time (geometric mean
  of seven programs) and was about 25× faster than wabt's interpreter.
  After tuning it takes 0.84× wasm3's time, on the tuning set and on a
  held-out set of 30 PolyBench kernels alike.
- **Time to working code.** About 20 minutes from the first doc read to
  a C program compiled to wasm32 running correctly, and 30 minutes to
  every MVP spec file passing (by the container clock, which ran slow).

## Issues

| # | Issue | State | Notes |
|---|---|---|---|
| 1 | A `const` name in a `match` pattern silently bound a new variable | Fixed | A constant in scope in a pattern now matches its value. Arms after a catch-all draw the `unreachable_arm` warning. |
| 2 | `f32` lacked `to_bits`, `from_bits`, `sqrt`, `floor`; both float types lacked `ceil`, `trunc`, `nearest`, `abs`, `copysign`, `min`/`max` | Fixed | All exist on `f32` and `f64`, each one LLVM intrinsic. The libm externs and pointer punning are gone. |
| 3 | No way to name "the next instruction" in a vm function; a wrong return address after a cache word crashed with SIGILL | Fixed | `next_pc` in op handlers and the `-> never` vm functions they call. |
| 4 | Formats cannot hold a wide immediate next to three operands | Fixed | `const` cache fields, checked once by the verifier (`cache TableIx { const ti: u64 verify it < .. }`). |
| 5 | No way to set one field of an instruction word; hand-written masks | Fixed | `Vm.set_FMT_field(w, v) -> ?W` and `Vm.encode.FMT(op, ..)`. |
| 6 | LLVM turned a zeroing loop into a `memset` call, giving the CALL handler a stack frame | Fixed | Handlers are compiled without builtin `memset`/`memcpy`/`memmove`. |
| 7 | No unaligned load or store | Fixed | `p.read_unaligned()` and `p.write_unaligned(v)`. |
| 8 | Fixed-size array literals need their length spelled twice | Partly fixed | `for x in array` and `let t: [T; _] = [..]` work; a return type still spells the length. |
| 9 | ``error: `Vec` is not a value`` did not say what to write | Fixed | The message now suggests `var v: Vec<T> = Vec.new();`. |
| 10 | Smaller things: range patterns undocumented, no `bool as i32`, no handler profiler, no op enum or disassembler for host code | Fixed | Range patterns are documented; `bool as i32`, `--vm-prof`, `Vm.Op`, `Vm.op_of` and `Vm.disasm` exist. |
| 11 | WASI and libc buffering cost time | Not Loimi's | wasi-libc line-buffers stdout when told it is a terminal; the first stub always said so, making fasta 1.9× wasm3. The prebuilt wasi-sdk sysroot also needed rebuilding with `-mcpu=mvp` for wasm3. |

Item 1 was the costliest. The code was:

```loimi
const HOME: u8 = 0; const LOCAL: u8 = 1; const CONST: u8 = 2;
match v.loc { LOCAL => v.x, CONST => { .. }, _ => t.home(pos) }
```

No error or warning was given. The translator emitted register numbers
taken from constants' bits, and the only symptom was a verifier error
(`operand b (ri) fails it < (f.nslots as u64)`, with `b=r65532`). In a
less checked setting this would have been a silent miscompile.

## What worked well

- **Operand kinds as typed views of a slot.** One line per wasm type,
  and handlers read like the spec:
  `I32_SHR_U(d: ru, a: ru, b: ru) { d = a >> b }`.
- **`inline for`** generated about 300 ops from tables. Errors pointed
  into the table row.
- **The verifier as a test oracle for the translator**, plus `W.disasm`
  for free (`--disasm` in `main.lm` is 10 lines).
- **`then JMP`** for compare-and-branch, and `verify NAME { .. }` for
  the `br_table` invariant.
- **`std.bc`**: labels, `last()` for retargeting, `truncate` for fusion.
- **`cold`** kept every trap path out of the handlers.
- **Almost everything compiled first time**: 2,000 lines of `vm.lm`,
  `compile.lm` and `rt.lm` built after one fix (item 4).
- **Dispatch modes are free**: the same source passes the spec suite in
  tail-call, goto and switch mode.

## The interpreter

| File | Lines (at the study) | What |
|---|---:|---|
| `decode.lm` | 628 | Binary format: LEB128, sections, every instruction's syntax |
| `compile.lm` | 1142 | Validator and translator in one pass |
| `vm.lm` | 812 | The vm (`W`), runtime structures, table and bulk-memory helpers |
| `rt.lm` | 481 | Module validation, linking, instantiation, invocation |
| `host.lm` | 177 | `spectest`, `env` and a WASI preview 1 subset |
| `main.lm` | 265 | Command line and the spec-test script protocol |

**Features.** The wasm MVP plus sign extension, saturating float-to-int,
multi-value, reference types, bulk memory and fixed-width SIMD (added
later). Imports and exports of every kind across instances. Traps with
the spec's messages. Not supported: threads, GC, tail calls,
multi-memory, the text format.

**Spec tests** (WebAssembly/testsuite f3f04866, run by
`bench/wasm/spec_run.py`): all 147 files pass in all three dispatch
modes, the 57 SIMD files included. Skipped commands are text-format
modules, which a binary-only interpreter cannot read.

**Design.** Wasm is a stack machine, but after validation every stack
height is known, so each stack position gets a fixed home slot in the
frame and `i32.add` becomes `I32_ADD r5, r3, r4`, as in wasm3. A stack
entry can stay *pending* (a local or a constant), so
`local.get a; local.get b; i32.add; local.set c` is one instruction.
Calls use register windows: the callee's frame starts at the caller's
first argument slot.

Values are untagged. The wasm validator proves types; the generated
`W.verify` proves memory safety of the translated code (slot indices in
the frame, indices in range, jumps onto instructions). Type confusion
between slots cannot break memory safety, so the translator need not be
trusted.

## Speed at the time of the study

Best of 3, seconds. Programs are unmodified public sources (CoreMark,
and Benchmarks Game C programs), built with clang 19 `-O2 -mcpu=mvp`
against an MVP build of wasi-libc.

| benchmark | wasm3 0.5.0 | Loimi (tail calls) | Loimi / wasm3 | wabt wasm-interp |
|---|---:|---:|---:|---:|
| binarytrees | 1.462 | 1.540 | 1.05 | 32.9 |
| coremark | 2.027 | 2.172 | 1.07 | 55.6 |
| fannkuchredux | 1.684 | 1.722 | 1.02 | 50.5 |
| fasta | 1.165 | 1.369 | 1.18 | 26.3 |
| mandelbrot | 2.000 | 1.887 | 0.94 | 63.1 |
| nbody | 1.992 | 2.489 | 1.25 | 57.0 |
| spectralnorm | 1.909 | 2.314 | 1.21 | 60.7 |
| **geomean** | **1.718** | **1.889** | **1.10** | ~48 |

`--dispatch=goto` took 1.14× the time of tail calls on every program.

What helped during the study: constant-operand forms (`I32_ADDK d, a,
k16`) and compare-and-branch fusion took the geomean from about 1.6× to
1.21× wasm3. Fixing the WASI file type (item 11) took fasta from 2.5 s
to 1.5 s. `--vm-stats` was the main tool: its first report showed
`CONST32` at 12% of dispatches, which led to both changes.

The other examples would have saved only the debugging of item 3: Mini
and Lox pass the return address into their shared call function by
hand. None uses `f32`, untyped slots or wide immediates.

## Tuning against wasm3

All timings below are best of 5 or 7, interleaved, with
`--align-functions=64` and tail calls. The machine was shared with other
work, so ±3% per program is noise.

### Float benchmarks against wasm3

nbody, spectralnorm and fasta trailed wasm3 (1.25×, 1.21×, 1.18×).
wasm3's op counters showed it runs 40–60% more ops at about half the
time each. A Loimi three-operand op was 14 instructions where wasm3's
was 5, and on a chain like `x*y + z` every intermediate went through a
slot, while wasm3 keeps it in `xmm0`.

Changes:

- Loimi: `#[forward]` now links operand kinds that differ only in the
  type they cast to; loops in tail-call vm code are no longer vectorized
  or unrolled (that had given CALL a stack frame).
- wasm: 18 `#[forward]` supers for float chains, address arithmetic and
  loop counters; constant slots for constants used inside loops, filled
  by CALL.

Result: geomean 1.10× to 1.00× wasm3 (nbody 1.31 to 1.01, spectralnorm
1.22 to 0.97). Giving small integer constants slots too made
fannkuchredux 4–6% slower, so only some constants get them.

### Accumulators (wasm3's `r0` and `fp0`), generated

Loimi now derives accumulator variants of ops: `accumulator facc: f64
for rd` adds a state register (`xmm0` in every handler), and
`accumulate I32_ADD, F64_MUL, ..;` (90 ops) derives 287 variants such as
`F64_MUL__AAR` (`facc = facc * b`). The translator uses one when a
result goes to a slot that only the next op reads.

A variant is 7 to 11 instructions where the slot form is 13 or 14, and a
chain's values never touch memory. Accumulators alone took the geomean
to 0.91× of before (0.87× wasm3): as much as a set of 31
benchmark-tuned supers, without tuning, and the two add up.

### Fixed-width SIMD

Built on Loimi's vector types (`i8x16` .. `f64x2`): the whole SIMD
proposal, 236 instructions. All 57 `simd_*.wast` files pass (25,469
commands) in every dispatch mode, and on AArch64 under qemu.

- A v128 takes two consecutive 64-bit slots, so frames without v128
  values are laid out as before and no scalar op changed.
- Most ops are one line in an `inline for` table, e.g.
  `("I16X8_ADD_SAT_U", vuh, vuh, vuh, a.add_sat(b))`.
- Non-SIMD programs are unchanged: geomean 0.998 of the time before.
- nbody built with `-msimd128` runs in 1.818 s against 1.971 s for its
  MVP build (wabt's interpreter: 54.5 s; wasm3 0.5 has no SIMD).

The friction found here has been fixed:

- A lane index from the bytecode is not a compile-time constant, so
  `v.lane(i)` did not apply. Loimi gained `v.lane_at(i)` and
  `v.with_lane_at(i, x)`. A chain of replace/extract_lane runs in
  0.410 s against 0.551 s with the earlier memory-based form.
- An `inline for` name could not be built from another. Now
  `#(name ++ "_X")` works.

`loimi build --cpu=x86-64-v2|x86-64-v3|native` was added and measured.
It shrinks `swizzle` from 105 instructions to 6 and rounding from 28 to
4, but on these benchmarks it is noise (geomean 1.004 at v2, 0.997 at
v3), because their SIMD was already cheap on SSE2. The default stays
the portable baseline.

### Where the time goes against wasm3, and four translator changes (2026-11-04)

With no `perf` on the machine, a 40-line `LD_PRELOAD` sampler (a timer
signal recording `rip` and the vm's `pc`) gave time per handler and per
vm instruction. Four translator changes followed (no compiler changes):

1. CALL zeroes only the locals a read can see unwritten. dlmalloc had
   zeroed 10 of its 11 locals on every call. This was the largest gain.
2. `CALL_LOCAL` skips the host-function test for calls within the
   module.
3. Address plus constant folds into the load or store
   (`F64_LOADK d, p, 8`).
4. More compare-and-branch forms: `x & k; br_if` and `i32.load; br_if`.

Result: 0.976 of the time before, 0.87× wasm3.

Tried and dropped: reading operand fields from memory instead of
shifting them out of the instruction word (1.044× the time: it trades
shifts for loads, which are scarcer); supers from all seven
`--vm-stats` reports (of the top 70 pairs only five recur across
programs).

### A held-out set (PolyBench/C), and round two (2026-11-04)

All of the above was tuned on the same seven programs, so
`bench/wasm/polybench.sh` adds a set the interpreter was never tuned on:
the 30 PolyBench/C 4.2.1 kernels, unmodified. `polybench.sh --check`
compares their output with wasm3's.

Round one generalized: on PolyBench it went from 0.998× to 0.919×
wasm3. Round two added:

1. Min and max as one op (`SEL_I32_LT_S`): floyd-warshall 0.86,
   nussinov 0.80 of the time before.
2. Constant slots for operands without a constant form (`i % 4000`).
3. A better order for `local.tee` of a loaded value.
4. A load from a `local.tee`'d address as one op (`I32_LOAD_TK`). A bug
   in its first version was caught by the PolyBench check, not the spec
   tests; `tests/fusions.wat` now covers it.

| set | wasm3 | before round one | round one | round two |
|---|---:|---:|---:|---:|
| tuning set (7), geomean | 1.921 | | 1.683 (0.876) | 1.674 (0.872) |
| PolyBench (30), geomean | 0.214 | 0.214 (0.998) | 0.197 (0.919) | 0.191 (0.893) |

Ratios to wasm3 in parentheses. Tried and dropped: merging adjacent
MOVs (noise: the hot MOVs move values that are live at once); globals in
one array per instance (binarytrees 1.04–1.08× slower).

### Round three: handler placement, the bounds check, array loads (2026-11-04)

The same build measured worse in a later session: cholesky, lu and
ludcmp took 1.2–1.3× wasm3's time. A build that moved every handler by
128 bytes ran cholesky in 0.62 s instead of 0.92 s. The cause was one
handler (`F64_LOAD__AR`) whose dispatch jump ended exactly at a 64-byte
boundary. On this Skylake CPU, the microcode fix for Intel's JCC erratum
then runs that code from the slow legacy decoders.

1. **Branch alignment** (a compiler change): x86-64 release builds of vm
   programs pad so that no branch, indirect jumps included, crosses or
   ends at a 32-byte boundary. `--no-branch-align` drops it; development
   builds never pad. PolyBench 0.963 of the time before (cholesky 0.67),
   tuning set 1.003.
2. **The bounds check against the end of the access**: the offset field
   holds `offset + size`, saving two instructions per load or store.
3. **Array-element loads**: `F64_LOAD_R d, a, b` loads at `a + b`. An
   add followed by a load was 3–15% of dispatches in 25 of the 30
   kernels.
4. **Accumulator variants for loads with two results** (a compiler
   change to how variants are derived).

Results, user+sys seconds:

| benchmark | wasm3 | before | after | after / before | after / wasm3 |
|---|---:|---:|---:|---:|---:|
| binarytrees | 1.451 | 1.341 | 1.295 | 0.97 | 0.89 |
| coremark | 2.026 | 1.847 | 1.788 | 0.97 | 0.88 |
| fannkuchredux | 1.659 | 1.402 | 1.426 | 1.02 | 0.86 |
| fasta | 1.486 | 1.112 | 1.103 | 0.99 | 0.74 |
| mandelbrot | 2.050 | 1.552 | 1.538 | 0.99 | 0.75 |
| nbody | 2.094 | 1.937 | 1.855 | 0.96 | 0.89 |
| spectralnorm | 2.040 | 1.753 | 1.764 | 1.01 | 0.86 |
| **geomean (7)** | 1.809 | 1.538 | 1.515 | **0.985** | **0.837** |
| **PolyBench geomean (30)** | 0.202 | 0.188 | 0.169 | **0.899** | **0.838** |

Every PolyBench kernel is now faster than under wasm3; the slowest,
gesummv, takes 0.95× its time.

Evaluated and not built:

- Accumulator variants for the select and min/max ops: their operands
  are never single-use temporaries.
- `BR_TABLE` jumping straight to its target: no change on coremark.
- Guard pages instead of bounds checks: needs 8 GiB of address space
  per memory, more than the memory limit the builds and tests run
  under.
- Removing loop-carried MOVs: it amounts to unrolling the loop.

Since then the vm orders its handlers from a recorded profile
(`#[profile("wasm.vmprofile")]`), which guards against placement noise.
