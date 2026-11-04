# WebAssembly interpreter: friction report

A usability study: a WebAssembly interpreter written from the docs
(`README.md`, `docs/language.md`, `docs/stdlib.md`, `docs/tutorial/`,
`std/`, `loimi --help`, `tests/run/*.lm`), without reading the other
example interpreters. It stresses what the earlier studies (stack
language, regex, Lox) did not: untagged typed values, structured control
flow resolved to jumps at load time, a validator the bytecode verifier
leans on, linear memory with bounds checks, `call_indirect` with
signature checks, and traps.

## Follow-up (what has been done since)

This report is kept as written. Since then:

| # | Status |
|---|---|
| 1 | Fixed: a name in a pattern that is a constant in scope (this module's, imported with `import m.{C}`, or qualified: `m.C`, `W.OP_ADD`) matches its value; a variant of the matched type still comes first, a local of the same name shadows, `var X` always binds, and a float constant is an error (language.md §1.2, "Constants in patterns"). Arms after one that matches everything draw the new warning `unreachable_arm`, whose note says when that arm is a binding: `match x { A => 10, B => 20, _ => 30 }` without constants now warns (an error under `-W error`, which `check.sh` uses). `Loc` stays an enum (exhaustive); its comment is updated. |
| 2 | Fixed: `f32` has what `f64` has (`to_bits -> u32`, `f32.from_bits`, `sqrt`, `floor`), and both have `ceil`, `trunc`, `nearest` (ties to even), `abs`, `copysign`, `is_nan`, `min`/`max` (C `fmin`/`fmax`: a NaN operand is ignored) and `minimum`/`maximum` (IEEE 754-2019 and wasm: NaN propagates, quieted; `-0 < +0`), each one LLVM intrinsic (stdlib.md, "Floats"). `sqrt`, `abs`, `copysign` and min/max are inline SSE2; the rounding functions stay libm calls on baseline x86-64, which lacks `roundsd` (SSE4.1). The libm externs, the pointer punning and the hand-written `fmin`/`fmax` are gone; `F64_MIN` is 32 instructions, `F32_NEAREST` a `roundevenf` call as `rintf` was. |
| 3 | Fixed: `next_pc`, the address `next` continues at (the calling op's `pc` plus its size, cache words included; a super part's own size), in op handlers and the `-> never` vm functions they call, shared copies included (language.md §3.5). `do_call` and `call_slow` lost their `ret` parameter and save `next_pc`; Lox's `call_closure`/`call_value` too (its `pc + 1 + Lox.CW_PropIC` call sites are gone; Lox benchmarks 0.996x by geometric mean, within noise). |
| 4 | Open (design note): docs/roadmap.md open question 10 compares trailing immediate words in formats (`format RRRX {..} + imm: u64`, with operand kinds and `verify`), `const` cache fields, `encoding bytes` and wider words. |
| 5 | Fixed: `Vm.set_FMT_field(w, v) -> ?W` (None if `v` does not fit the field) and `Vm.encode.FMT(op, a, b, ..)` (language.md §3.3). They are compiled only into programs that call them, so other vms' IR is unchanged. The masks, the `SELECT4` special case (now `match W.set_R4_a(w, x)`) and the `with_opcode` templates are gone. |
| 6 | Fixed: tail-call handlers (and their cold blocks and super resume functions) carry `no-builtin-memset`/`memcpy`/`memmove`, so LLVM keeps loops as loops (language.md §3.7). CALL zeroes the callee's locals with a plain loop again, without a frame (63 instructions with the loop, against 46 for the four-stores workaround; geomean 0.99x of the workaround's time). Mini, MiniJS and Scheme compile to byte-identical `.text`; Stak's HALT/RET store their zeroed result differently. |
| 7 | Fixed: `p.read_unaligned()` and `p.write_unaligned(v)` (`align 1`; language.md §1.3); every guest memory access in the handlers and the WASI host functions uses them, and `I32_LOAD` is unchanged (19 instructions in this build, the access one `mov`). |
| 8 | Partly: `for x in e` iterates over an array or slice (language.md §1.2), so the K-form and fusion tables are walked without repeating their lengths; the return types still spell them (`[T; _]` is not built). |
| 9 | Fixed: "`Vec` is a generic type, not a value; .. write `var v: Vec<T> = Vec.new();`". |
| 10 | Range patterns are documented (language.md §1.2, "Integer patterns"). `bool as` integer, a profiler and an op-table dump are unchanged. |
| 11 | Not Loimi's. |

Spec tests after these changes: all 90 non-SIMD files pass in all three
dispatch modes (`check.sh`). Benchmarks (best of 3, interleaved,
`--align-functions=64`, tail calls), this version against the study's:
binarytrees 0.97, coremark 0.97, fannkuchredux 0.97, fasta 1.00,
mandelbrot 0.99, nbody 1.00, spectralnorm 1.01; geomean 0.985 (1.881 s
against 1.909 s).

### Float benchmarks against wasm3

Why nbody, spectralnorm and fasta trailed wasm3 (1.25x, 1.21x, 1.18x)
while mandelbrot led it, and what changed.

**Evidence.** A wasm3 built with its op counters
(`-Dd_m3EnableOpProfiling=1`, same flags otherwise) against
`--vm-stats`:

| | wasm3 ops | Loimi ops (before) | wasm3 ns/op | Loimi ns/op |
|---|---:|---:|---:|---:|
| nbody | 2.62 G | 1.87 G | 0.76 | 1.35 |
| spectralnorm | 2.96 G | 2.16 G | 0.65 | 1.07 |
| fasta | 1.40 G | 0.87 G | 0.83 | 1.55 |
| mandelbrot | 2.94 G | 1.94 G | 0.68 | 0.97 |

So the cause is the cost per dispatch, not the number of dispatches:
wasm3 runs 40–60% more ops (on nbody 503 M `op_SetSlot_f64` and 253 M
`op_SetSlot_i32` alone, the stores of its register operands to slots)
at about half the time each. The handlers show why
(`objdump` of both builds):

- `F64_MUL d, a, b` is 14 instructions: 6 decode three 16-bit fields
  from the word in `rdx` (`mov; shr; movzwl` per field), 3 load,
  multiply and store, 5 dispatch (load the next word, advance `pc`,
  `movzwl` the opcode, load the handler from the table, `jmp`).
  wasm3's `op_f64_Multiply_rs` is 5: `movslq (%rdi),%rax; mulsd
  (%rsi,%rax,4),%xmm0`, then the next handler's address from the code
  stream and `jmp *%rax` (direct threading; one operand and the result
  are `fp0`, which is `xmm0` across ops).
- On a dependency chain (`x*y + z`, nbody's and spectralnorm's inner
  loops), every Loimi intermediate goes through a slot: a store and a
  load of the same address, about 5 cycles of store forwarding on top
  of the arithmetic. wasm3's chains stay in `xmm0`. So yes, wasm3's
  `fp0` (and its `_rs`/`_sr`/`_ss` variants) is what wins.

Checked and not the cause: NaN canonicalization (neither interpreter
canonicalizes in these handlers), `sqrt` (inline `sqrtsd` in both),
`f64.div` (both `divsd`), the call path (nbody makes 2.5 M calls,
spectralnorm 55), bounds checks (the same `lea; add; cmp; ja` in both),
`floor`/`ceil` (none executed in these four; wasm3 is built
`-O3 -fomit-frame-pointer`, baseline x86-64, and calls libm `floor`
through a stack frame too). A target-CPU option was measured by
compiling the same IR with `-march=x86-64-v2` and `-v3`: 0.998 and 0.989
of the baseline build's time over nbody, spectralnorm and fasta (noise):
LLVM 19 decodes the fields the same way with BMI2, and only the rounding
ops would change (`roundsd`). Not added.

**Loimi changes.**

- *Forwarding between views of a slot* (language.md §7.3,
  lowering.md §8): `#[forward]` links joined operands of the same kind
  only, so `I32_ADDK d: ri` could not forward into `F64_LOAD a: ru`, nor
  `F64_ADD d: rd` into `F64_STORE v: rq`. Kinds whose bodies differ only
  in the type they cast to now link (`tests/run/vm_super_forward_views.lm`).
  The six slot kinds here are such views (`rq` is now written
  `*((regs + it) as *u64)` like the others).
- *Loops in tail-call vm code are not vectorized or unrolled*
  (`!llvm.loop` metadata, lowering.md §3): copying the constant slots in
  CALL (below) made LLVM vectorize two loops, and the handler grew a stack
  frame (`push %rbp`) and spills: 154 instructions. Now 50, no frame.
  Mini, MiniJS and Scheme: their IR differs only in the metadata (their
  Nil-filling loops, mostly in cold paths, are no longer vectorized);
  benchmarks within noise (Scheme 0.982 by geometric mean; Mini and
  MiniJS per benchmark within ±3% on reruns).
- *Documented* (lowering.md §3): a float state variable stays in an SSE
  register across tail calls (`state { .., acc: f64 }`: `acc = acc * a`
  compiles to `mulsd (%r14,%rdx,8),%xmm0` plus the dispatch), so wasm3's
  `fp0` is expressible; and the dispatch's table load folds into the
  jump (`jmp *table(,%rax,8)`) only when the tail call passes at most six
  values in registers (this vm passes seven; LLVM's isel counts them),
  which turned out to cost nothing measurable (lowering.md §3). docs/roadmap.md open question 11 lists what could
  make a three-operand op cheaper.

**wasm changes.**

- 18 `#[forward]` supers for float chains (`F64_MUL; F64_ADD`,
  `F64_LOAD; F64_MUL; F64_ADD`, `F64_SUB; F64_STORE`, ..), address
  arithmetic (`I32_ADDK; F64_LOAD`), loop counters (`I32_ADDK; BRNZ`,
  `I32_ADDK; BR_I32_NE`) and `CONST64; F64_DIV`; `W.optimize` runs on
  each function after verification.
- Constant slots (wasm3's constant table): constants that appear inside
  loops (floats, integers that do not fit a K form's 16 bits, and 0) get
  a slot after the locals, which CALL fills from `Func.consts`, so ops
  read them in place instead of running CONST32/CONST64 every iteration
  (mandelbrot's `BR_F64_LE a, r24` against 4.0). At most 32 per function.

Dispatches, before and after both: nbody 1.87 G to 1.22 G, spectralnorm
2.16 G to 1.36 G, fasta 0.87 G to 0.75 G, mandelbrot 1.94 G to 1.36 G.
- `fd_write` makes one `writev` per call instead of one `write` per
  buffer (wasi-libc flushes with two): no measurable change on fasta.

**Negative results.** Giving every constant in a loop a slot, small
integers included, made fannkuchredux 4–6% slower (25 slots in its main
function, nearly all used only by K forms); hence the filter. Reading
the operand fields from memory instead of the word in a register
(reloading the word in each handler) left the code as it was: LLVM does
not narrow a load with three uses.

**Results** (best of 5, interleaved, `--align-functions=64`, tail
calls; the machine was shared with another agent's test runs, so
±3% per benchmark is placement and load noise):

| benchmark | wasm3 | before | after | before / wasm3 | after / wasm3 |
|---|---:|---:|---:|---:|---:|
| binarytrees | 1.469 | 1.595 | 1.643 | 1.09 | 1.12 |
| coremark | 2.044 | 2.169 | 2.127 | 1.06 | 1.04 |
| fannkuchredux | 1.726 | 1.650 | 1.625 | 0.96 | 0.94 |
| fasta | 1.153 | 1.349 | 1.291 | 1.17 | 1.12 |
| mandelbrot | 2.013 | 1.874 | 1.647 | 0.93 | 0.82 |
| nbody | 1.997 | 2.617 | 2.016 | 1.31 | 1.01 |
| spectralnorm | 1.921 | 2.335 | 1.868 | 1.22 | 0.97 |
| **geomean** | 1.729 | 1.896 | 1.725 | 1.10 | **1.00** |

Of that, the supers alone (an earlier run, best of 5): nbody 2.61 to
2.09, spectralnorm 2.32 to 1.91, mandelbrot 1.89 to 1.72; the constant
slots then mandelbrot 1.73 to 1.68 and fasta 1.37 to 1.32, others within
noise. binarytrees (call-heavy: malloc) moved between 0.99x and 1.04x of
its old time across runs. fasta still trails wasm3 by about 12%: its
inner loops are integer (`I32_REM_U` random numbers, a binary search
with two `SELECT4`s), which these changes barely touch.

### Accumulators (wasm3's `r0` and `fp0`), generated

The float study above found wasm3's ops cost about half of Loimi's
because one value lives in a machine register between ops. Loimi now
derives such ops (`language.md` §7.4): `accumulator iacc: u64 for ri,
ru, rl, rq; accumulator facc: f64 for rd;` adds two state registers
(`facc` is `xmm0` in every handler), the vm item `accumulate I32_ADD,
F64_MUL, F64_LOAD, BR_I32_LT_U, ..;` (90 ops) derives 287 variants
(`F64_MUL__ARR`: `facc = a * b`; `F64_MUL__AAR`: `facc = facc * b`;
`F64_STORE__RA`: store `facc`; ..), and `compile.lm` calls
`W.accumulate(words, nlocals)` before verification: an op whose result
goes to an operand stack home that the next op alone reads (and that is
not a jump target) writes the accumulator, and the next op reads it.

**Upper bound first** (hand-generated, not committed: a script wrote the
variants of every eligible op and a table-driven pass in `compile.lm`).
Best of 3 interleaved, `--align-functions=64`, seconds:

| | base | base + supers | acc | acc + supers | wasm3 |
|---|---:|---:|---:|---:|---:|
| geomean (7) | 1.767 | 1.648 (0.933) | 1.640 (0.928) | 1.416 (0.801) | 1.854 |

("supers": every super `--vm-stats` suggested on the seven benchmarks
for that build, 31 for base and 45 for acc, `#[forward]`.) Accumulators
alone gained 7%, and with the supers 14% over base with its supers: well
over the 5% bar, so the generic feature was built.

**Built** (the committed vm; best of 5 interleaved, `--align-functions=64`,
tail calls, seconds; the machine was shared with another agent, so ±3%
per benchmark is noise):

| benchmark | wasm3 | before | acc | acc / before | acc / wasm3 | acc + supers |
|---|---:|---:|---:|---:|---:|---:|
| binarytrees | 1.543 | 1.534 | 1.483 | 0.97 | 0.96 | 1.455 |
| coremark | 2.109 | 2.241 | 2.031 | 0.91 | 0.96 | 1.782 |
| fannkuchredux | 1.713 | 1.649 | 1.528 | 0.93 | 0.89 | 1.370 |
| fasta | 1.507 | 1.286 | 1.136 | 0.88 | 0.75 | 0.989 |
| mandelbrot | 2.031 | 1.677 | 1.533 | 0.91 | 0.75 | 1.224 |
| nbody | 2.064 | 2.075 | 1.959 | 0.94 | 0.95 | 1.665 |
| spectralnorm | 2.020 | 2.007 | 1.658 | 0.83 | 0.82 | 1.391 |
| **geomean** | 1.839 | 1.753 | **1.593** | **0.91** | **0.87** | 1.389 |

"before" with its own 31 suggested supers measured 1.600 in the same
run (0.91): accumulators are worth as much as a benchmark-tuned set of
supers, without being tuned, and the two add up ("acc + supers", the 46
supers `--vm-stats` suggests for the accumulator build: 0.79 of before,
0.87 of before-plus-its-supers). The supers are not committed: most are
four-op sequences of one benchmark's inner loop.

**Dispatches and instructions.** Executed ops (`--vm-stats`), before and
after: nbody 1.22 G to 1.80 G, spectralnorm 1.36 G to 1.76 G,
mandelbrot 1.36 G to 1.64 G, fasta 0.75 G to 0.79 G, the others
unchanged. More dispatches, because a variant breaks up the `#[forward]`
supers the plain ops formed (`F64_MUL_F64_ADD`), yet faster: 44–74% of
the executed ops are variants (29% on binarytrees), and a variant is
7–11 instructions where the slot form is 13–14 (`F64_MUL__AAR`:
`shr $0x30,%rcx; mulsd (%r14,%rcx,8),%xmm0` and the 5-instruction
dispatch, which is wasm3's `op_f64_Multiply_rs` plus a table load), and
on a chain no value is stored and loaded back. Plain handlers did not
change (the two extra state registers cost no spills in `CALL`, `RET1`,
`I32_ADD`, ..).

**What it took in the vm.** Two kind changes: `F64_STORE` reads its
value as `rd` (it was `rq`, an integer view, so a float chain could not
end in it) and `F64_NEG` is `-a` on `rd` (was `a ^ sign` on `rq`; LLVM's
`fneg` flips only the sign bit, as wasm requires). The `accumulate` list
names 90 ops: marking every op with an operand of an accumulator's kind
gave about 660 variants, and the goto-mode build (one function with
every handler) ran out of its 2.5 GB address space in instruction
selection; with 287 it took 1 GB and 3.5 minutes. Since goto mode emits
one shared dispatch block that LLVM copies into the handlers after
register allocation (`docs/lowering.md` §5), the build with 287 takes
17 s and 220 MB, and marking every eligible op (1170 dispatch targets)
39 s and 325 MB: the op count is no longer limited by goto mode. The
list stays at 90 ops for the tail-call build's size.

**Limits.** Selection is greedy left to right and per adjacent pair: in
`x = a / b; y = load; x * y` the division keeps its slot because the
load is in between. Values used twice (`local.tee`) and locals stay in
slots (wasm3 has `_rs` ops with a slot result and `SetSlot`; Loimi's
translator already writes results straight into locals). Stak, a stack
vm that caches the top of stack in a state register by hand, has no
slot operands for `accumulate` to replace: not applied.

### Fixed-width SIMD

Built on the vector types Loimi gained for it (`i8x16` .. `f64x2`,
language.md): the whole SIMD proposal (236 instructions), and
`check.sh` now runs every spec file. **All 57 `simd_*.wast` files pass
(25,469 commands, 0 failures; 510 skipped, all text-format
`assert_malformed`), and all 90 others still do (27,331), in each of the
three dispatch modes.**

**Values.** A v128 takes two consecutive 64-bit slots: locals, operand
stack homes and call windows are laid out by width (`Val.s` in
compile.lm is a value's home relative to the first one; `T.lslots` maps
a local index to its slot), so a frame without v128 values is laid out
exactly as before and no scalar op changed. Global cells are two words.
Vector operand kinds view the pair as each lane type
(`operand vi = *((regs + it) as *i32x4) verify it + 1 < f.nslots`);
Loimi loads and stores vectors without assuming alignment, which the
8-byte slots need. A separate v128 register file was the alternative;
two slots kept calls, branches with values, multi-value returns and
`select` working through the existing moves, with `MOVV`, `SELECTV`,
`RETV` and `GLOBAL_GETV`/`SETV` as the only v128 variants of control ops.

**Ops** (about 250, after the existing ones so that their opcodes do not
move): most are one line in an `inline for` table, e.g.
`("I16X8_ADD_SAT_U", vuh, vuh, vuh, a.add_sat(b))`,
`("F32X4_PMIN", vf, vf, vf, (b < a).select(b, a))`,
`("I32X4_TRUNC_SAT_F64X2_U_ZERO", vui, vd, u32x4.narrow_sat(a as u64x2, u64x2.splat(0)))`;
extadd_pairwise, dot and q15mulr_sat are small helpers from
`widen_low`/`widen_high`, `shuffle` and `narrow_sat`. The translator
needed no new paths for them: SIMD instructions without immediates go
through the same table-driven `numeric` path as the scalar ones (v128
as one more operand type), and SIMD loads and stores through `memop`.
`v128.const` and `i8x16.shuffle` carry 16 bytes in two cache words;
shuffle is `a.swizzle(idx) | b.swizzle(idx - 16)`, since its indices come
from the code, not from the handler.

**Friction.** Small, all of one kind: a lane index that comes from the
bytecode is not a compile-time constant, so `v.lane(i)` and
`v.with_lane(i, x)` do not apply; extract/replace_lane and the lane
loads and stores address the slot's bytes instead
(`*(((regs + a) as *i16) + l)`, with lane operand kinds `l8`, `l4`, ..
whose `verify it < 8` keeps that inside the frame), or mask a lane index
from the cache word. Each could have been 16 ops per type from an
`inline for` over lanes; the memory form is one. An `inline for` name
cannot be built from another (`#(name ++ "_X")`), so the load table
spells both names of each load. The compiler accepted the rest as
written; no bug was hit.

**Speed.** Non-SIMD programs are unchanged (best of 3, 5 for binarytrees and coremark,
`--align-functions=64`, against the previous commit: binarytrees 1.00,
coremark 0.98, the others within 1%). The CLBG programs built with
`-msimd128` (clang 19, the same sources: nbody gets 53 SIMD
instructions, fannkuchredux 13, spectralnorm 2; the others none) run
under Loimi in 1.818 s (nbody), 1.484 s (fannkuchredux) and 1.608 s
(spectralnorm), against 1.971, 1.510 and 1.649 s for their MVP builds,
and 54.5, 58.1 and 81.2 s under wabt's wasm-interp (wasm3 0.5 has no
SIMD).

Checked afterwards: the handlers of the non-SIMD ops are the same
instructions as before (objdump, addresses normalized), and the
translator emits the same code for the benchmarks (`--disasm` of
binarytrees and coremark: identical); best of 5, interleaved,
`--align-functions=64`, tail calls: geomean 0.998 of before (binarytrees
1.003, coremark 0.988, fannkuchredux 1.003, fasta 0.999, mandelbrot
0.999, nbody 1.005, spectralnorm 0.987). On AArch64 (cross-built, under
qemu) all 57 SIMD files pass in tail-call mode and all 147 files in goto
mode.

**Follow-up: the friction fixed, and a target CPU.**

- *Run-time lanes.* Loimi gained `v.lane_at(i)` and `v.with_lane_at(i, x)`
  (language.md §1.2, "Vectors": the index is masked to the lane count,
  no panic path). Extract/replace_lane and the lane loads and stores now
  use them on the typed vector operands (`d = a.with_lane_at(cache.v, x
  as i8)`); the `vslot` operand kind is gone. Handler instructions
  (x86-64 baseline): extract 14 → 15 (the mask: LLVM turns `lane_at`
  of a loaded vector into one scalar load but cannot see that the
  verifier bounded `l`), lane stores unchanged (21/23), replace 18 →
  23..28 and lane loads 25..27 → 32..36 on SSE2, 20 and 27..29 with
  AVX2. More instructions, but all in registers: `with_lane_at` is a
  compare-and-select, where the memory form stored the vector and then
  a lane into the same slot, so the next vector load of it failed store
  forwarding (and LLVM's own variable-index `insertelement` does the
  same on the stack). A dependent chain of replace/extract_lane
  (`lanes.c`, 20 M iterations) runs in 0.410 s against 0.551 s before
  (0.354 s against 0.582 s at `x86-64-v3`). All 147 spec files pass in
  every dispatch mode.
- *Names from names.* `#(name ++ "_X")` in an `inline for` body
  (language.md §1.4) joins loop variables and literals into one
  identifier; the v128 load table lost its second name column. (The
  scalar load/store tables stay two tables: merging them would change
  the order, and so the numbering, of the scalar ops.)
- *`--cpu`.* `loimi build --cpu=x86-64-v2|x86-64-v3|native|..` (clang's
  `-march`; lowering.md §12 has instruction counts per level): `swizzle`
  becomes `paddusb`+`pshufb` (105 → 6 instructions with loads and the
  store; `i8x16.shuffle` 209 → 13), rounding `roundps` (28 → 4),
  `minimum` 28 → 18 (12 with AVX), i64x2 compares 15 → 5. All 147 spec
  files pass at `x86-64-v2` and `x86-64-v3` in every dispatch mode.
  Speed, best of 5, `--align-functions=64`, tail calls, seconds
  (the `_simd` programs are the CLBG sources built with `-msimd128`):

  | benchmark | x86-64 | x86-64-v2 | x86-64-v3 |
  |---|---:|---:|---:|
  | binarytrees | 1.437 | 1.472 | 1.447 |
  | coremark | 1.979 | 1.995 | 1.982 |
  | fannkuchredux | 1.530 | 1.583 | 1.577 |
  | fannkuchredux_simd | 1.484 | 1.528 | 1.559 |
  | fasta | 1.231 | 1.234 | 1.175 |
  | mandelbrot | 1.507 | 1.504 | 1.575 |
  | nbody | 2.045 | 2.078 | 1.937 |
  | nbody_simd | 1.805 | 1.816 | 1.860 |
  | spectralnorm | 1.622 | 1.623 | 1.624 |
  | spectralnorm_simd | 1.631 | 1.624 | 1.611 |
  | lanes (microbenchmark) | 0.360 | 0.334 | 0.333 |
  | **geomean** | 1.405 | 1.410 (1.004) | 1.401 (0.997) |

  Noise, both ways: the SIMD the CLBG programs contain (f64x2
  arithmetic, splats, one shuffle in fannkuchredux's setup) was already
  one or two instructions on SSE2, and dispatch dominates. `--cpu` pays
  where a program runs the ops SSE2 lacks (`swizzle`/`shuffle`,
  rounding, `minimum`/`maximum`, 64-bit compares); none of these
  benchmarks does. The default stays the portable baseline.

### Where the time goes against wasm3, and four translator changes (2026-11-04)

**Profiling without `perf`.** `perf_event_paranoid` is 3 on this
machine, so no hardware counters. A 40-line `LD_PRELOAD` sampler (a
`CLOCK_MONOTONIC` POSIX timer every 37 µs whose `SIGPROF` handler records
`rip` and `r13`, the vm's `pc`) attributes time per handler (each op is
its own symbol) and per vm instruction (hot code shows as runs of `pc`
values 8 bytes apart), which with `--disasm` finds the hot loops. Not
committed (it is a scratch tool), but worth a `std.prof` feature: a
sampling profiler that maps samples to vm `pc`s needs nothing from the
kernel.

Per benchmark (handler profile, `--vm-stats`):

- **binarytrees** (malloc/free-bound): `CALL` alone is 14% of the time,
  most of it the loop that zeroes the callee's locals (a data-dependent
  trip count, so its exit branch mispredicts across callees); then
  `I32_LOAD`/`I32_STORE` (12.5/7.5%, dlmalloc's free lists), `MOV`
  (6.5%), `CONST32` (5.7%, constant arguments and local inits) and
  `GLOBAL_SET` of the stack pointer (3.9%: three dependent loads).
- **coremark**: spread out; 11% is one pointer-reversal loop (`MOV;
  I32_LOAD; I32_STORE; MOV; BRNZ`), which is load-latency-bound through
  slots, and the state machine's `BR_TABLE`.
- **fannkuchredux**: `I32_ADDK` 11%, `I32_LOAD` 12%, `MOV` 10%; address
  arithmetic (`I32_ADDK t, p, 28` then a store at offset 0: clang adds
  the constant where the offset cannot be used, since offsets do not
  wrap) and bit tests (`I32_ANDK; BRZ`, 3.4% of dispatches).
- **fasta**: the random-number loop is `I32_REM_U` and `F64_DIV`
  latency (17%), and the cumulative-probability search two `SELECT4`s
  (12%) on an `F64_LT` result; at parity with wasm3, as before.
- **nbody**, **spectralnorm**, **mandelbrot**: float chains, already
  ahead of wasm3; nbody's next items are `I32_ADDK__AR; F64_LOAD__AA`
  pairs (4.9% of dispatches).

**Changes** (examples/wasm only; the compiler is unchanged):

1. *Calls zero only the locals a read can see unwritten.* A forward pass
   over each body (`needs_zero` in compile.lm) tracks the locals written
   on every path: at a construct's `end`, what every way of reaching it
   wrote (each branch to it, the fall-through of its last instruction
   and of an `if`'s then arm; a loop's end is only reached by falling
   through; an `if` without `else` adds the empty path around it).
   Those locals get slots right after the parameters, and `Func.nzero`
   ends the zeroed range. clang hardly reads an unwritten local: one
   function of the seven benchmarks (in fasta) still zeroes any (dlmalloc zeroed
   10 of its 11 locals on each call). `tests/zero_locals.wat` (now run by
   `check.sh`, with wabt's `wat2wasm`) reads locals that one path leaves
   unwritten, after a call that leaves 99s in the same slots; a build that
   zeroes nothing prints the 99s.
2. *`CALL_LOCAL`*, the call of a function the module defines, skips the
   test for a host function or another instance's (two loads and two
   branches).
3. *Address plus constant.* `local.get p; i32.const 8; i32.add` (or a
   value in its home slot plus a 16-bit constant) stays pending, like a
   pending local (`Loc.AddK`), and a load or store at offset 0 takes it
   whole: `F64_LOADK d, p, 8` computes `(p + 8) mod 2^32`, as `i32.add`
   wraps (23 ops, 12 with accumulator variants). Anything else
   materializes it as the `I32_ADDK` it was. nbody 1.80 G to 1.68 G
   dispatches, fasta 0.79 to 0.75 G, binarytrees and coremark 3% fewer.
   In fannkuchredux the address is usually `local.tee`d first, so it
   stays two ops.
4. *More compare-and-branch*: `x & k; br_if` is `BR_I32_ANDK_NZ/Z`, and
   `i32.load` (also `i32.load; local.tee x`) followed by `br_if`/`if` is
   `BR_I32_LOAD_NZ/Z d, a, off`, which also writes the word to d
   (linked-list walks, `if (p->next)`). And a function whose end no
   branch reaches returns a pending local or constant directly (`RET1 x`
   instead of `MOV; RET1`), plus one generic super, `I32_ADDK;
   I32_ADDK` (two counters or pointers stepped together: in coremark,
   fannkuchredux and fasta).

**Results** (best of 7, interleaved, `--align-functions=64`, tail calls;
the machine was shared with another agent's builds, load average 4–5,
so ±3–4% per benchmark is noise):

| benchmark | wasm3 | before | after | after / before | before / wasm3 | after / wasm3 |
|---|---:|---:|---:|---:|---:|---:|
| binarytrees | 1.686 | 1.646 | 1.537 | 0.93 | 0.98 | 0.91 |
| coremark | 2.347 | 2.180 | 2.128 | 0.98 | 0.93 | 0.91 |
| fannkuchredux | 1.926 | 1.692 | 1.642 | 0.97 | 0.88 | 0.85 |
| fasta | 1.332 | 1.317 | 1.274 | 0.97 | 0.99 | 0.96 |
| mandelbrot | 2.246 | 1.716 | 1.688 | 0.98 | 0.76 | 0.75 |
| nbody | 2.325 | 2.170 | 2.126 | 0.98 | 0.93 | 0.91 |
| spectralnorm | 2.211 | 1.823 | 1.860 | 1.02 | 0.82 | 0.84 |
| **geomean** | 1.975 | 1.769 | 1.726 | **0.976** | 0.90 | **0.87** |

Of that, the zeroing is the largest part (a build that zeroed nothing
measured binarytrees 0.96, coremark 0.97, fannkuchredux 0.97 of before),
the K loads and stores gave nbody 0.95–0.96 in two runs, the rest is
within noise each.

**Tried and dropped.**

- *Operand fields loaded from the code instead of shifted out of the
  instruction word* (a compiler change: a 16-bit field at a byte offset
  becomes `movzwl 2(%r13),%eax`; `I32_ADD` 14 instructions to 11): 1.044
  of the time by geometric mean (mandelbrot 1.16, binarytrees 1.09, only
  coremark faster). The handlers trade ALU work (shifts, which have four
  ports) for loads, which have two and which the slot accesses, the next
  word and the table entry already use: `I32_ADD` goes from 4 loads to 7.
  Not kept.
- *`super`s from the union of the seven `--vm-stats` reports*: with the
  accumulator variants, the frequent pairs split into many variants
  (`I32_ADDK__AR; F64_LOAD__AA` in nbody, `I32_ADDK__AA; F64_LOAD__AA` in
  fasta), and of the top 70 pairs only five recur in two or more
  benchmarks. Supers that pay are per benchmark, so the translator
  changes above were made instead (3 and 4 cover the recurring ones),
  and only `I32_ADDK; I32_ADDK` was added.

**Not done, the next candidates.** Local-to-local `MOV`s are 7–10% of
dispatches in binarytrees, coremark, fannkuchredux and mandelbrot
(clang's own copies, often loop-carried: coalescing them needs liveness
over the wasm locals). `CONST32` into locals and as call arguments (12%
of binarytrees' dispatches). `GLOBAL_GET`/`GLOBAL_SET` of the stack
pointer read `ctx.globals[g]` and then the cell: globals of the instance
in one array would save a dependent load. An address that is
`local.tee`d (fannkuchredux) or used with a nonzero offset keeps its
`I32_ADDK`. A held-out benchmark set would show how much of this is
specific to these seven programs; all four changes are about how clang
compiles C, not about one program.

## Summary

| File | Lines | What |
|---|---:|---|
| `decode.lm` | 628 | binary format: LEB128, sections, UTF-8 names, syntax of every instruction (all "malformed" errors) |
| `compile.lm` | 1142 | validator and translator in one pass: control stack, typed operand stack, pending locals/constants, labels, K-forms, compare-and-branch fusion |
| `vm.lm` | 812 | the vm (`W`, about 340 ops, most from `inline for` tables), runtime structures, table/bulk helpers |
| `rt.lm` | 481 | module validation, linking (imports, `register`), instantiation, segments, start, invocation |
| `host.lm` | 177 | `spectest`, `env` (`print_i32`, `print_f64`, `putchar`, ..) and a WASI preview 1 subset |
| `main.lm` | 265 | CLI (`wasm prog.wasm`, `--invoke`, `--disasm`) and the spec-test script protocol |
| `check.sh`, `tests/` | 67 + 56 | every dispatch mode with `-W error`, C programs compiled to wasm32, the spec files |
| `bench/wasm/spec_run.py` | 234 | spec runner (wast2json JSON to a command script, one process per file) |
| `bench/wasm/*` (subagent) | ~430 | pinned testsuite, wabt, wasm3, cmake, wasi-libc (MVP build), CoreMark, CLBG; `bench.py` |

**Features.** The wasm MVP core, plus sign extension, saturating
float-to-int, multi-value (blocks with params and results, multiple
returns), reference types (funcref/externref, `ref.*`, `table.*`,
several tables, `call_indirect` through any table, typed `select`) and
bulk memory (`memory.copy/fill/init`, `data.drop`, `table.init/copy`,
`elem.drop`). Imports and exports of every kind across instances
(calls into another instance switch memory, globals and tables and
come back through a verified RESTORE stub). Traps with the spec's
messages. Not built: SIMD, threads, GC, tail calls, multi-memory, the
text format.

**Spec tests** (WebAssembly/testsuite f3f04866, the commit wabt 1.0.36
pins; `bench/wasm/spec_run.py`): **all 90 non-SIMD .wast files pass,
27,331 commands, 0 failures**, in all three dispatch modes. 581
commands are skipped: text-format modules (`assert_malformed` on `.wat`,
`module quote`), which a binary-only interpreter cannot read, and a few
actions whose arguments are non-null funcref constants. The 57 SIMD
files fail (not implemented: 121 commands pass by accident). Per file:
`python3 bench/wasm/spec_run.py --wasm WASM --skip simd`.

**Time.** First doc read 01:51 (by the clock in this container); first
commit with a working interpreter (a C program compiled to wasm32 ran
correctly) 02:10; every MVP spec file passing 02:21; reference types
and bulk memory 02:37. The commit timestamps are in `git log`. (The
container clock advanced much less than the work suggests; the
relative order is what it is.)

**Speed** (best of 3, seconds; wasm-interp from the infrastructure
run, best of 3, run two at a time):

| benchmark | wasm3 0.5.0 (-O3) | Loimi (tailcall) | Loimi / wasm3 | Loimi goto | wasm-interp 1.0.36 |
|---|---:|---:|---:|---:|---:|
| binarytrees (CLBG, depth 15) | 1.462 | 1.540 | 1.05 | 1.686 | 32.9 |
| coremark (5000 iterations) | 2.027 | 2.172 | 1.07 | 2.507 | 55.6 |
| fannkuchredux (CLBG, 10) | 1.684 | 1.722 | 1.02 | 1.871 | 50.5 |
| fasta (CLBG, 2.5M) | 1.165 | 1.369 | 1.18 | 1.562 | 26.3 |
| mandelbrot (CLBG, 2400) | 2.000 | 1.887 | 0.94 | 2.280 | 63.1 |
| nbody (CLBG, 2.5M steps) | 1.992 | 2.489 | 1.25 | 2.810 | 57.0 |
| spectralnorm (CLBG, 2000) | 1.909 | 2.314 | 1.21 | 2.652 | 60.7 |
| **geometric mean** | **1.718** | **1.889** | **1.10** | 2.146 | ~48 |

All programs: unmodified public sources (CoreMark at eembc/coremark
1f483d5 with a timer port; Computer Language Benchmarks Game C programs
at benchmarksgame 4029666), compiled by clang 19 `-O2 -mcpu=mvp`
against an MVP build of wasi-libc; every output checked (CoreMark's
CRCs). Loimi is within 10% of wasm3 by geometric mean, faster on
mandelbrot and slowest relative to it on float-heavy nbody and
spectralnorm (F64_MUL/F64_LOAD-bound: 45% of nbody's dispatches),
and about 25x faster than wabt's interpreter. `--dispatch=goto` is
1.14x slower than tail calls on every program.

## Design

**Register vm over untyped 64-bit slots.** Wasm is a stack machine, but
after validation every operand stack height is static, so each stack
position gets a fixed *home slot* in the frame (`nlocals + position`)
and `i32.add` becomes `I32_ADD r5, r3, r4` (the approach of wasm3 and
Wizard's fast path). The translator also does not materialize
`local.get` or constants: a stack entry can be *pending* (a local's
slot, or a constant), so `local.get a; local.get b; i32.add; local.set c`
is one instruction, `I32_ADD c, a, b` (the last instruction's
destination is retargeted to the local when no label intervenes, with
`bc.Code.last()`). Pending entries are flushed to their home slots at
block boundaries, before a local they read is assigned, and for calls.
A stack design would have spent dispatches on `local.get`/`local.set`
(about 40% of wasm instructions) and on stack-pointer updates.

Calls use register windows: arguments are the caller's top home slots,
the callee's frame starts at the first one, and results come back in
the same slots. Frames record the return pc and the caller's window.
Branches that carry values move them to the target's home slots first;
`br_table` is `BR_TABLE idx, n` followed by n + 1 JMP words (stubs after
the table do the moves for targets that need them).

**Typed views of one slot.** Values are untagged: the validator has
proved every type, so each op reads a slot as the type it needs, and
operand kinds express that directly:

```loimi
operand ri = *((regs + it) as *i32) verify it < (f.nslots as u64) display "r{}";
operand ru = *((regs + it) as *u32) verify it < (f.nslots as u64) display "r{}";
operand rd = *((regs + it) as *f64) verify it < (f.nslots as u64) display "r{}";
op I32_LT_U(d: ri, a: ru, b: ru) : RRR { d = b2i(a < b); next }
```

**What the vm verifier checks, and what the wasm validator checks.**
The wasm validator (compile.lm) proves types: no op reads a slot as a
type that was not written there. The generated `W.verify` proves memory
safety of the *translated* code: every slot index is below the frame
size, every function, global and type index is in range for the
instance (`verify_context f: *Func, inst: *Inst`), every jump lands on
an instruction, a JMP follows every compare-and-branch (`then JMP`), and
a `verify brtable { .. }` block checks that `BR_TABLE n` is followed by
n + 1 JMPs. Type confusion between slots cannot break memory safety
(every view stays inside its 8-byte slot), so the vm verifier does not
need types, and the translator is not trusted: a translator bug becomes
a verification error at load time, not a wild write. Bounds that exist
only at run time (memory and table size, call depth, stack room for the
callee's frame) are checked in the handlers, with `cold` slow paths.

This division worked very well, and the verifier earned its keep: it
rejected the translator's output during development, each time for a
real translator bug (item 1 is the first one).

## Friction, ranked by impact

### 1. A `const` name in a `match` pattern silently binds a new variable

**Tried.** Stack-entry locations as `u8` constants, matched by name:

```loimi
const HOME: u8 = 0; const LOCAL: u8 = 1; const CONST: u8 = 2;
match v.loc { LOCAL => v.x, CONST => { .. }, _ => t.home(pos) }
```

**Happened.** No error and no warning. `LOCAL` is a fresh binding that
matches everything, so every value took the first arm; the translator
emitted register numbers taken from constants' bits. The only symptom
was the vm verifier rejecting the code: `internal error: translated
code fails verification: word 10 (I32_AND): operand `b` (ri) fails
`it < (f.nslots as u64)`` (operand `b=r65532`, the constant -4). A
3-line probe confirmed it (`match x { A => 10, B => 20, _ => 30 }`
returns 10 for every x).

**Did instead.** An `enum Loc { Home, Local, Const }`.

**Suggested fix (language).** Resolve an identifier pattern that names
a `const` in scope as a comparison with it (Rust does), or make it an
error ("`LOCAL` is a constant; this pattern binds a new variable");
in any case warn about unreachable arms after an irrefutable one
(`B => 20` and `_ => 30` here can never match). This is the costliest
item of the study: in a less checked setting it is a silent
miscompile.

### 2. `f32` is a second-class type

**Tried.** `x.to_bits()`, `f32.from_bits(b)`, `x.sqrt()` on `f32`.

**Happened.** `error: no method `to_bits` on type `f32``; likewise
`sqrt`, `floor`. `docs/stdlib.md` lists them for floats without saying
"f64 only". No test in `tests/run` uses `f32` at all.

**Did instead.** Punning through a local
(`var y = x; *((&y) as *u32)`, which LLVM turns into a `movd`), and
libm through `extern "C"` (`sqrtf`, `ceilf`, `floorf`, `truncf`,
`rintf`). Wasm's f32 arithmetic itself (`a + b` on `f32`) works and
compiles to `addss`.

**Suggested fix (language/std).** The same built-ins on `f32` as on
`f64` (`to_bits -> u32`, `from_bits`, `sqrt`, `floor`), and the missing
float operations for both (`ceil`, `trunc`, `round_even`/`nearest`,
`abs`, `copysign`, `min`/`max` with a stated NaN rule), lowered to the
LLVM intrinsics so they become `roundsd`/`sqrtss` instead of libm calls.
A guest with IEEE semantics needs all of them.

### 3. No expression for "the next instruction" in a vm function

**Tried.** A shared `-> never` vm function `do_call(f, base)` that saves
the return address as `pc + 1` (as the tutorial's CALL does), called
from CALL (one word) and later from CALL_INDIRECT_X (one word plus an
inline-cache word holding a table index).

**Happened.** Calls through a table other than 0 crashed with SIGILL:
the callee returned to `pc + 1`, the cache word, whose value (2) ran as
opcode 2 (CONST64) and the stream went off the rails into the
placeholder's invalid opcode. Debug and release builds alike; no
diagnostic. Found with print statements.

**Did instead.** `do_call(f, base, ret)` with each op passing its own
size (`pc + 1`, `pc + 2`). (Afterwards I saw Lox does the same.)

**Suggested fix (language).** A control-word-like expression for the
address `next` would continue at, e.g. `next_pc`, valid in ops and in
`-> never` vm functions (where it already knows the calling op's size,
since `next` works there). Then "save a return address" cannot get an
op's size wrong.

### 4. Formats cannot hold a wide immediate next to three operands

**Tried.** `i64.const` (64-bit immediate), loads with a 32-bit offset
and two 16-bit slots, `ADDR d, a, offset32`.

**Happened.** `error: format `RU` has only 2 operand field(s)` for an op
listing three operands on a 16+16+32 format; a u64 word cannot hold
16+16+16+32. There is no way to declare a second instruction word as an
operand.

**Did instead.** An inline cache `cache Imm64 { v: u64 }` used as an
immediate, written after `code.op(..)` zeroed it
(`let pc = t.code.op(W.CONST64(s)); t.code.set(pc + 1, bits);`). It
works and the disassembler even prints it (`[cache v=4607182418800017408]`),
but the verifier cannot check it: the table and segment indices of the
bulk ops and of CALL_INDIRECT_X live in such a word and must be
bounds-checked at run time, which is exactly what operand kinds exist
to avoid.

**Suggested fix (language).** Trailing immediate words in formats
(`format RRRX { op: 16, a: 16, b: 16, c: 16 } + imm: u64`), with
operand kinds and `verify` clauses like any field, encoder parameters,
and disassembly. `encoding bytes` (not built) would also cover it.

### 5. Re-encoding one field of an instruction word is hand-written bit twiddling

**Tried.** Retargeting the destination of the last instruction
(`local.set` after an op), patching the table-index slot into a
`call_indirect`, turning a compare into a compare-and-branch.

**Happened.** The generated API reads fields (`W.RRR_b(w)`,
`W.RRK_k(w)`) and replaces opcodes (`W.with_opcode`), but cannot set a
field. Encoders exist per op only, so emitting "this format with that
opcode" needs a template op.

**Did instead.** Masks: `(w & 0xFFFFFFFF0000FFFF) | (x << 16)`, a
special case for the 12-bit field of `SELECT4`, and
`W.with_opcode(W.I32_ADD(a as u16, b as u16, c as u16), op)` as a
generic three-register encoder (the casts are needed: 16-bit fields take
`u16`).

**Suggested fix (generated API).** `W.set_RRR_a(w, v) -> ?W`
(range-checked like `set_label`) and a per-format encoder
`W.encode.RRR(op, a, b, c)`.

### 6. LLVM turns a zeroing loop into a `memset` call, and the handler gets a stack frame

**Tried.** In CALL: `while i < n { nr[i] = 0; i += 1; }` to zero the
callee's locals.

**Happened.** The loop became `call memset@plt`, so CALL's handler
pushed a frame and spilled on *every* path (57 instructions:
`push %rbp; sub $0x10,%rsp; .. mov %rdi,0x8(%rsp)`), though no call is
visible in the source. The `cold` discipline cannot help with calls
LLVM introduces itself.

**Did instead.** Four unconditional stores (slots past `nlocals` are
the callee's scratch, and the stack has slack) and a `cold` path that
calls an `#[inline(never)]` loop for longer runs: 46 instructions, no
frame.

**Suggested fix (compiler/docs).** Compile handlers with
`-fno-builtin`-like attributes (`"no-builtins"`) or at least warn in
`--vm-stats`/`-W` mode when a handler's hot path ends up with a frame;
mention the pitfall in tutorial chapter 8's checklist ("no stack frame").

### 7. No unaligned load/store primitive

**Tried.** Guest memory accesses at arbitrary addresses.

**Happened.** `*((mem + ea) as *i32)` compiles to `load i32` with the
type's natural alignment (4), which is undefined behavior for an
unaligned guest address. It works on x86-64 and AArch64, but LLVM is
entitled to assume alignment (vectorizing, combining). Cache fields get
`align 4` unaligned accesses, but there is no user-facing way to ask
for that.

**Did instead.** Kept the plain loads.

**Suggested fix (language/std).** `p.read_unaligned()` /
`p.write_unaligned(v)` (or `*align(1)` pointers), lowered to
`load ... align 1`.

### 8. Fixed-size array literals need their length spelled out twice

**Tried.** Lookup tables of tuples for K-forms and fusions returned
from a function.

**Happened.** The return type needs the exact length
(`-> [(u64, u64, u64, bool); 52]`) and the loop repeats it
(`for i in 0..52`); adding a row means editing two numbers, and there is
no `const` array of tuples of other consts' values in practice (no
comptime, `W.OP_X` constants). Also, matching on `W.OP_X` constants was
avoided after item 1.

**Suggested fix (language).** Infer `[T; _]`, and allow
`for x in array` iteration.

### 9. `mem.Vec<u64>.new()` is "not a value"

**Happened.** `error: `Vec` is not a value` at `mem.Vec<u64>.new()`.
`docs/language.md` explains it (explicit type arguments are not taken
on a generic type's static functions), but the message does not say
what to write.

**Did instead.** `var v: mem.Vec<u64> = mem.Vec.new();`

**Suggested fix (diagnostics).** "write `var v: mem.Vec<u64> =
mem.Vec.new();`" in the error.

### 10. Smaller things

- **Range patterns are undocumented.** `0x28..=0x3E =>` works in
  `match` (seen in `tests/run/control.lm`), but `language.md` never
  mentions it; I almost wrote 23 alternatives.
- **`bool` to integer.** No `c as i32`; a `b2i` helper with
  `#[inline(always)]` (it compiles to `setcc`, fine).
- **No `perf`/sampling profiler** on the machine and none in `std.prof`;
  `--vm-stats` was the only profile, and it does not see time spent in
  host functions. The fasta slowdown (item 11) was invisible to it.
- **`sh`-style probes**: compiling and running a one-off file needs a
  scratch directory and an `-o`; `loimi run file.lm` covers it, but
  there is no `--disasm`-style dump of a vm's generated op table
  (`W.NUM_OPS`, sizes) from the command line.

### 11. Not Loimi, but cost time: WASI and libc buffering

wasi-libc line-buffers stdout when `fd_fdstat_get` reports a character
device. My first stub always said "character device", so fasta made
one `write` per 61-byte line and took 1.9x wasm3's time; reporting a
regular file when `isatty` is false brought it to 1.3x. And the prebuilt
wasi-sdk sysroot is compiled with bulk memory and reference-types
encodings, which wasm3 0.5.0 cannot read, so the benchmark
infrastructure builds wasi-libc from source with `-mcpu=mvp`.

## What worked well

- **Operand kinds as typed views.** One line per wasm type, and every
  handler reads like the spec (`I32_SHR_U(d: ru, a: ru, b: ru) { d = a >> b }`).
  Shifts taken modulo the width, `+%`/`*%` wrapping, `clz`/`ctz`/
  `popcount` and signed/unsigned views made the integer ops one-liners.
- **`inline for`** generated about 300 of the ops from tables: all
  binary and unary ops, loads and stores (and their 64-bit-address
  variants), K-forms and 60 compare-and-branch ops. Errors pointed into
  the table row.
- **The verifier as a translator test oracle** (above), plus `W.disasm`
  for free, which made every translator bug a one-glance diagnosis
  (`--disasm` in `main.lm` is 10 lines).
- **`then JMP`** for compare-and-branch: the fused op and its target
  word, checked by the verifier, with 32-bit offsets and no branch
  relaxation needed.
- **`verify NAME { .. }`** for the `br_table` invariant.
- **`std.bc`**: labels, `last()` for the retargeting peephole,
  `truncate` for fusion, `check()`.
- **`cold`** kept every trap path out of the handlers, and **effects**
  never got in the way (no GC here).
- **Everything compiled the first time**, almost: `vm.lm`, `compile.lm`
  and `rt.lm` (2,000 lines) built after one fix (item 4); the remaining
  bugs were mine, and the spec suite found them.
- **Dispatch modes are free**: the same source passes the whole suite
  in tailcall, goto and switch mode.

## Generated code

x86-64, `--release`, tail-call dispatch. State: `pc` r13, `regs` r14,
`fp` r15, `mem` rdi, `msize` (memory size, a state register), ctx r12;
the instruction word arrives in a register. No handler below has a
stack frame or a spill.

`I32_ADD` (13 instructions + the jump):

```text
mov    %rsi,%rax
shr    $0x20,%rax
movzwl %ax,%eax            ; b
mov    %rsi,%rcx
shr    $0x30,%rcx           ; c
mov    (%r14,%rcx,8),%ecx
add    (%r14,%rax,8),%ecx
shr    $0x10,%esi           ; d (32-bit shift truncates)
mov    %ecx,(%r14,%rsi,8)
mov    0x8(%r13),%rsi       ; next word
add    $0x8,%r13
movzwl %si,%eax
jmp    *0x424110(,%rax,8)
```

`I32_LOAD` (18): the same decode, `lea (%rax,%rcx,1),%rdx; add $4,%rdx;
cmp <msize>,%rdx; ja cold` (the bounds check), `mov (%rdi,%rax,1),%eax`,
store, dispatch. With the memory size in `ctx` it was one `cmp
0x30(%r12),%rdx` instead; making it a state register measured as noise.
`I32_ADDK` 13, `F64_MUL` 14, `RET1` 10, `BR_I32_LT_S` 20 (it reads its
`then` word), `CALL` 46 (after item 6; 57 with a frame before).
Trap paths are a single `jmp vm.W.op.X.coldN`.

## Optimizations, and what helped

Measured with `bench/wasm/bench.py` (best of 3, interleaved).

| change | effect |
|---|---|
| pending locals and constants, destination retargeting (from the start) | most of the gap to a naive stack translation (not measured separately) |
| K-forms (`I32_ADDK d, a, k16`, 36 ops) and compare-and-branch fusion with `then JMP` (40 integer ops) | geomean 1.21x of wasm3 from about 1.6x: nbody 5.4 to 2.6 s, spectralnorm 5.9 to 2.5 s (single runs, before/after) |
| float compare-and-branch (20 ops, NaN-correct negated forms) | noise |
| memory size in a state register instead of `ctx` | noise |
| CALL without a stack frame (item 6) | noise to 10% on call-heavy code (spectralnorm 2.70 to 2.34) |
| `SELECT4 d, a, b, c` (12-bit fields) instead of MOV + SELECT + MOV | noise |
| WASI `fd_fdstat_get` filetype (item 11) | fasta 2.5 to 1.5 s |

`--vm-stats` was the main tool: its first report (fannkuch) showed
CONST32 at 12% and `cmp; BRNZ` pairs everywhere, which led directly to
the K-forms and fusion; its suggested supers (`CONST32_I32_ADD_CONST32`
and friends) were symptoms of missing constant operands rather than
good supers. I did not add `super`s: after the K-forms, no pair was
above a few percent across the suite, and the earlier studies found
supers within noise. Not tried for lack of time: folding constant
addresses into load offsets, `i32.add` + load fusion, `#[forward]`
supers for float chains (nbody is F64_MUL/F64_LOAD-bound), and the
copy-and-patch JIT (`--jit-stencils`).

Would the other examples have helped? Mini's and Lox's vms (skimmed
after the interpreter worked) show `then JMP` compare-and-branch and the
same "pass the return pc into the shared call function" workaround as
item 3, so they would have saved the debugging of item 3. None uses
`f32`, untyped slots or wide immediates, so items 1, 2, 4 and 5 would
have happened anyway.
