# Design note: a speed audit (features for faster interpreters)

Status: **done.** Started 2026-11-06 against 0813bc1. Every item was
built, or measured and dropped; the table below gives each verdict. §4
(2026-11-07) surveys LLVM, clang and linker features: none pays enough
to use.

The audit asked which language, compiler, runtime or std features would
let an author write a faster interpreter than they could before. Until
then a feature was added only when a tuning round on one interpreter
needed it (accumulators, `#[forward]` supers, `try_new`, polls, mark
bitmaps and `#[profile]` all came that way). The candidates came from
the bench and friction logs (`bench/*/RESULTS.md`, `bench/*/README.md`,
`examples/*/FRICTION.md`) and from the cores of the references' sources
under `bench/*` (LuaJIT's `vm_x64.dasc`, wasm3's `m3_exec.h` and
`m3_compile.c`, CPython 3.13's `pycore_code.h` and `pycore_backoff.h`,
Guile's `vm-engine.c`, SWI-Prolog's `pl-vmi.c`, QuickJS and PUC-Lua).
A feature qualified only if it made interpreters faster, not only
shorter.

Three layers of cost are left in a tuned Loimi handler: dispatch and
operand decoding (about 11 of the 14 instructions of a three-operand
float op, against wasm3's 5), memory traffic through the register file
(store-to-load forwarding between dependent ops), and runtime work
(calls and frames, object size, the collector, division, hashing). None
of the features in these layers is worth more than a few percent
suite-wide alone. The JIT removes most of the first layer (0.76–0.83 of
the interpreter's time), and the trace tier (item 14) goes after the
second.

**Method of every timing below**, unless an item says otherwise: user+sys
CPU, the minimum of interleaved runs (order rotated each round), an A/A
column from a byte-identical copy of the baseline in the same
interleave, `--align-functions=64` with suspects rechecked at 128
(placement alone moves single benchmarks by 3–9%), on an i7-6700
(Skylake) with LLVM 19.

## 1. Items

| # | Item | Verdict | Measured |
|---|---|---|---|
| 1 | Accumulators for `value` types | built | see item 1 |
| 2 | Cheaper operand decoding and dispatch | no-go | PolyBench 1.003 (pre-scaled fields), 1.008 (offset-threaded) |
| 3 | Division by a verified constant: `std.math.Divisor` | built | Prolog crypt 0.855, MiniJS closures 0.970 |
| 4 | Load facts (`dereferenceable`, `nonnull`, `!invariant.load`) | no-go | 0 of 154 (Mini) and 165 (MiniJS) handlers change |
| 5 | Headerless typed chunks (16-byte pairs) | no-go | upper bound 0.971 / 0.993 on nboyer, sboyer, mperm, earley |
| 6 | A sampling profiler: `--vm-prof` | built | top entries within 1 point of a hand sampler; 3% at 1000 Hz |
| 7 | Shifted-int nanbox: `repr nanbox(int_high)` | built | see item 7 |
| 8 | `refuse CODE;` and `supers from "file";` | built | Lox suite 0.973, zoo 0.883 |
| 9 | `std.rt.ValueStack` | built | Lox suite 0.970, fib 0.863 |
| 10 | Adaptive quickening (warmup and backoff counters) | no-go | later `become`s ≤ 0.055% of dispatches; bound 0.982 on one benchmark |
| 11 | Coroutine switching | quickened call sites kept in Mini; the vm primitive is a no-go | chameneos 0.925 |
| 12 | Trailing immediate words | see item 12 | |
| 13 | JIT stencils: computed jumps to a target | built | Mini interp 0.977 (interpreter) |
| 14 | An optimizing tier | built as the trace tier | `trace-tier.md` |

---

### 1. Accumulators for `value` types

**Evidence.**
- **Wasm.** Accumulators (`language.md` §7.4) took the wasm
  interpreter to 0.91 of its time, and 0.79 with supers over the
  variants (`examples/wasm/FRICTION.md` 159–241). A variant is 7–11
  instructions where the slot form is 13–14.
- **The nanbox VMs.** "Accumulators hold numbers only: no GC
  references." `src/vmacc.rs:267` rejects any type but integers, f32 and
  f64. Mini, MiniJS, Lox, Python and Scheme all keep their registers as
  NaN-boxed or low-tagged `value` words, so none of them can declare
  one. Only wasm does.
- **What the bound says.** The JIT's latency probe projected at most
  8% by geometric mean over Mini's 14 benchmarks with one cached
  register (`copy-and-patch.md` §13). The register-file reads it slows
  are the ones a cache would serve: mandelbrot 39%, spectral_norm 43%,
  nbody 27%, hash_strings 23%, sieve 14%, queens 13%.
- **What forwarding already took.** `#[forward]` supers forward a
  value *within* a super: Mini 0.982, MiniJS 0.989, Scheme 0.967
  (`bench/mini/RESULTS.md` item 28). Accumulators would take the hops
  *between* handlers, which is what the hand-written one-slot loop got
  and the stencils did not.
- **Why the JIT result does not settle it.** The one-register cache
  built in the JIT was 1.015× (slower). Its causes were specific to
  stencils:
  - cache variants were exclusive with const variants;
  - the integer `movq` round trip through xmm15;
  - export and prologue costs on every stencil.

  The interpreter's accumulator is chosen statically by the translator,
  and lives in a GPR for integer and boxed classes.

**Proposal.**
- **Value accumulators.** Allow `accumulator acc: V for reg;` where `V`
  is a `value` type, or any traced type. The accumulator is a traced
  state variable, and those are already rooted automatically
  (`language.md` §8). Two lowering choices:
  - *write-through*: the variant also stores to the home slot, which
    keeps GC, `raise`, the debugger and `Vm.deopt` untouched. The store
    is off the critical path, and the next op's load is the one saved;
  - *spill at sync points*: store before polls, `uses gc` calls, cold
    blocks and exits.

  Start with write-through. On mandelbrot's loop, by hand, caching
  only the previous op's result with write-through measured 0.79 of the
  JIT's time, against 0.68 with every register cached and spilled at
  exits (`copy-and-patch.md` §13). Most of the gain survives
  write-through.
- **f32 accumulators**, which wasm lacks today (FRICTION 235–241).
- **Selection past non-interfering ops.** `Vm.accumulate` pairs only
  adjacent ops. wasm3's `PreserveRegisterIfOccupied`
  (`m3_compile.c:380-436`) keeps r0 live across ops that do not need
  it.

<!-- snippet: skip design, not built -->
```loimi
// before: Mini cannot declare an accumulator (V is a nanbox value)
op MUL(dst: reg, a: reg, b: reg) : ABC { .. }

// after
accumulator vacc: V for reg;            // traced; write-through by default
accumulate ADD..DIV, *_F, JLT*, FORLOOP;
// compile.lm: Mini.accumulate(code, fs.first_temp) after emitting each function
```

**Measure.**
- Mini's 14 benchmarks and MiniJS's numeric ones (nbody,
  spectral_norm, mandelbrot-like loops), A/B with
  `--align-functions=64`.
- `--vm-stats` to count how many dispatches take a variant.

**Prototype.** Mark Mini's float members (`ADD_F`, `MUL_F`, `SUB_F`,
`DIV_F`) `#[accumulate]` with the accumulator typed `u64`, and treat
`V` as its raw bits through a `reg64` operand view. Then call
`Mini.accumulate` from `compile.lm`, and run with a heap large enough
that no collection happens. This is unsafe only if a collection runs,
and measures the bound on mandelbrot, nbody and spectral_norm in about
a day. Build the traced version only if mandelbrot gets below 0.95.

**Cost: M.** It is mostly in `vmacc.rs` (the type check, write-through
stores in the variants) and the selector. Risks:
- **Code size and opcode count.** Marking the float and compare
  members gives up to 5 variants each. Wasm is the precedent: 287
  variants gave 669 opcodes instead of 382, and its goto build takes
  17 s and 220 MB since goto mode shares one dispatch block
  (`language.md` §7.4, `lowering.md` §5).
- **Fewer ops are eligible.** Type-checking ops branch to cold blocks
  that read the slot. With write-through that is still correct, but a
  variant's cold path must read the accumulator or the slot
  consistently.

**Interactions.**
- **Lifetimes, GC and polls.** A traced state variable is rooted at
  every sync, and write-through means the slot is never stale.
- **`redo_effect`.** A variant's write-through store is an effect. It
  is the same store the slot form makes, so `redo` stays correct only
  if it comes after the last read, as today.
- **JIT stencils.** Variants are ordinary ops, so they get stencils.
- **Dispatch modes.** Goto mode's single function grows with the
  variant count (`lowering.md` §5).
- **`#[profile]`.** Variants are counted and ordered like any other op.
- **Debugger.** Write-through keeps the register file truthful.
- **Supers.** Supers of variants form as in wasm, and `#[forward]`
  links stay valid.

**General:** five interpreters, numeric code first.

#### Prototype results (2026-11-06)

**Verdict: go.** Mandelbrot measured 0.85 with an untraced accumulator
and 0.91 with a GC-safe one (write-through), against a 0.95 threshold.
Both results held at both function alignments. nbody and spectral_norm
did not gain. The suite geomean moved less than 1%. Build the
write-through version first, with the accumulator held in an f64
register. Details follow.

**What was built.** The prototype is a patch, not committed:
`proto-value-acc.patch` in the session scratch directory
(`proto-acc-x7m3/`). It is about 140 lines, gated by
`LOIMI_PROTO_VALUE_ACC` and `LOIMI_PROTO_ACC_WT` at build time and by
`--cfg proto_acc*`. It changes the following.
- **`vmacc.rs`.**
  - A `value` kind may go to a `u64` accumulator, which holds the
    word's bits, or to an `f64` accumulator, which holds them as
    `f64.from_bits`.
  - `accumulate` may name family members. Their variants stay members of
    the family, so `become` still works.
  - With `_WT`, a variant that writes the accumulator also stores the
    slot.
- **`vmexpand.rs`.** `Vm.acc_base(op)` is new. `Vm.accumulate` first
  resets every variant to its base op and splits supers. This makes it
  idempotent, so it can run again after quickening.
- **Mini.**
  - `accumulator vacc: u64|f64 for reg;`.
  - The `accumulate` list: `ADD_F`, `SUB_F`, `MUL_F`, `ADDK_F`,
    `SUBK_F`, `MULK_F`, `DIV`, `DIVK`, and `JLT`, `JLTK`, `JGTK` with
    their `N` forms. That is 76 variants.
  - `refuse` calls `Mini.accumulate(code, proto.first_temp)` before
    `optimize`. `first_temp` is the most locals ever active at once,
    tracked in `add_local`.
  - Ten `#[forward]` supers over the variants. They mirror the existing
    pairs, so mandelbrot's loop keeps its six dispatches.
  - `MINI_PROTO_NOGC=1` sets the heap threshold to 2^62.
- **Allocation without GC.** mandelbrot, nbody and spectral_norm
  allocate nothing in their loops (8 MB RSS either way). The worst
  benchmark was binary_trees at 1.55 GB, within the 2.5 GB limit, so no
  input was shrunk. Every build matched all 14 `.expected` outputs.

**Quickening complicates selection.** The `_F` members exist only
after a site's first run (`become` in the generic member), so selection
has to run again inside the handler that quickens. Two hazards follow,
and any design must handle them:
1. **The next op may have just become a reader.** After `refuse`, the
   instruction after the quickening op may be a variant that reads the
   accumulator, but the quickening op wrote only its slot. The prototype
   also writes `vacc` in the generic member's `become` paths.
2. **`redo` may re-run a reader.** The `_IF`/`_FI` members `redo`
   after `become`, and the word they re-run may now be a variant that
   reads an accumulator that was never written. The prototype finishes
   the op in place instead. That also keeps `_IF`/`_FI` out of the
   list: a variant cannot `become` a member whose operand kinds differ.

**Timings.** Times are user+sys CPU, the minimum of 5 interleaved runs,
with `MINI_PROTO_NOGC=1` for every build including base, so collection
never runs. Load average was 2.7–4.1 on 8 cores, from two other agents.
Ratios are against base built from the same tree:

| build | mandelbrot | nbody | spectral_norm | suite geomean (14) | worst non-float |
|---|---:|---:|---:|---:|---|
| A/A (base vs base copy) | 1.000 | 0.998 | 0.999 | 1.001 | ±0.8% |
| u64 acc, no supers on variants | 0.983 | 0.982 | 1.017 | 0.999 | sort 1.045, queens 1.030 |
| u64 acc + supers | 0.921 | 0.978 | 1.005 | 0.993 | sort 1.046, queens 1.031 |
| **f64 acc + supers** (raw bound) | **0.855** | 0.972 | 1.017 | 0.991 | oo_methods 1.028 |
| u64 write-through + supers | 0.956 | 0.983 | 1.010 | 0.998 | queens 1.036 |
| **f64 write-through + supers** (GC-safe) | **0.914** | 0.966 | 1.027 | 0.994 | fannkuch 1.033 |
| f64 write-through, every slot (`first_temp` 0) | 0.904 | 0.976 | 1.028 | 0.996 | fannkuch 1.013 |

`--align-functions=128` rerun of the same configurations:
- f64 acc + supers: mandelbrot 0.852, nbody 1.004, spectral_norm 1.009.
- f64 write-through + supers: mandelbrot 0.910, nbody 0.996,
  spectral_norm 1.012.
- u64 acc + supers: mandelbrot 0.922, queens 1.037, sort 1.015.
- Every other benchmark stayed within ±1%, except binary_trees with
  write-through (1.025).

So:
- **mandelbrot gains for real.**
- **nbody's 2–3% is placement:** it is 1.00 at 128.
- **spectral_norm is 1–3% slower** in every build.
- **interp's 0.96–0.98 at 64 is placement too:** it is 0.996 at 128.

**`--vm-stats` hit rates** (share of dispatches that are a variant or a
super containing one, in the first_temp builds):
- **mandelbrot: 73.5%.** Five dispatches of the six-dispatch inner loop
  carry a variant: `MULK_F__AR_MUL_F__AAR`, `ADD_F__RAR_SUB_F__ARR`,
  `ADD_F__RAR_MUL_F`, `MUL_F_ADD_F__ARR` and `JGTKN__A`, at 14.7% each.
  Without supers on variants, the variants break up the existing
  `#[forward]` supers. Dispatches then rise from 6 to 9 per iteration,
  which is why "no supers" gains only 1.7%.
- **nbody: 25%.** `GETFIELD_MUL_F__ARR`, `MUL_F__ARR_SUB_F__RRA` and
  `ADD_F__RRA_SETFIELD` at 8.3% each. Most of its chains already ran
  inside `#[forward]` supers (`MUL_F_ADD_F__f2`, `GETFIELD_SUB_F__f2`).
- **spectral_norm: 8.3%.** `MUL_F__ARR_ADD_F__RRA` replaces
  `MUL_F_ADD_F__f2`, which already forwarded. Its hot ops are integer
  `ADD`/`MUL` and the `_IF`/`_FI` members, which are not in the list.
- **The other 11 benchmarks: 0%.** Their changes come from the extra
  state register alone.
- **Chaining locals too** (write-through makes this correct, `first_temp`
  0): mandelbrot's `zi*zi` feeds `zrq + ziq`
  (`MUL_F__ARR_ADD_F__ARA`). That is a further 1%.

**Assembly** (hot path, dispatch included):
- **`MUL_F`, slot form:** 18 instructions: decode 7, `mulsd`, NaN test
  2, store, dispatch 4.
- **`MUL_F__AAR`, u64 accumulator:** 14. It includes a `movq` from
  `%rsi` to `%xmm0` and a `movq` back, so a GPR accumulator costs two
  cross-domain moves of 1–3 cycles each on the critical path.
- **`MUL_F__AAR`, f64 accumulator:** 13, with the accumulator in
  `%xmm0` and no `movq`.
- **`MUL_F__AAR`, f64 write-through:** 15. It adds back one decode and
  the store, both off the critical path.
- **Supers:** `ADD_F__RAR_SUB_F__ARR` is 30 instructions against 33 for
  `ADD_F_SUB_F`.

**Every plain handler changed.** The tail-call build's jump through
the table was `jmp *T(,%rax,8)`. With the u64 accumulator it becomes
`mov T(,%rax,8),%rax; jmp *%rax`. One more argument register stops LLVM
from folding the load into the `jmp`, and the instruction word moves
from `%esi` to `%edx`. That is the likely cause of queens +3–4% and
sort +1–5% in the u64 builds, which run no variant at all. An f64
accumulator takes no GPR. Its builds keep queens and sort at 1.00, and
their plain-handler costs are within ±1% at 128. So the register class
matters as much as the variants.

**Design for the real feature** (traced, safe):
- **Register class.** `accumulator vacc: V for reg;` where `V` is a
  `value` type, held in an f64 register for nanbox values. Floats are
  already the raw word there, and the integer paths pay a `movq` only
  in variants. Low-tagged VMs, or a declared class (`.. in u64`), use a
  GPR. Measure that before choosing it, because of the dispatch fold
  above.
- **GC safety: write-through.** The collector is non-moving (sticky
  mark bits, `roadmap.md` Q2), so a slot written through roots the
  object, and the accumulator copy stays valid without tracing. Nothing
  in the GC, `raise`, the debugger, `Vm.deopt` or the JIT needs to know.
  It cost 6 points of mandelbrot (0.855 to 0.914) and stays under 0.95.
  - **Spill-at-sync** gets the rest. A reader variant stores the
    accumulator to its slot (the `__acc` field still names it) before
    any `poll`, `uses gc` call, `raise`, `cold` block, `become` or
    `redo`. A writer writes the accumulator only at `next`, and nothing
    syncs after that. It is safe on a non-moving heap because the value
    is only ever live between two adjacent ops. Keep it as a later
    option (`#[accumulate(spill)]`).
- **Write-through makes selection unconditional.** With the slot always
  current, a reader is correct whenever it follows its writer and is not
  a jump target. The "read once" promise and `first_temp` are no longer
  needed, and locals chain too (`first_temp` 0 was correct on all 14
  benchmarks and slightly faster). This matters for Scheme, whose
  compiler cannot give a `first_temp`: let-bound values share registers
  with temporaries.
- **Polls, `redo`, raises, the debugger.**
  - Polls and raises: with write-through, the register file is always
    truthful at op boundaries.
  - `redo`: the accumulator holds the reader's source while nothing
    overwrote it, so `redo` stays correct. One case needs handling:
    `redo` after `become` re-runs a different word. The generator
    should make `become` write the op's result (or the source it is
    about to re-read) to the accumulator. Simpler: re-selection inside
    a handler (`refuse`) leaves the current op and the next op
    unchanged until the next entry.
  - The debugger: `set_break` and `step` act on base ops. Breaking at a
    reader is fine with write-through, since the reader re-reads the
    accumulator, which still holds the value.
- **Idempotent selection.** Keep the prototype's `Vm.acc_base`, and the
  reset at the start of `Vm.accumulate`, so `refuse` can call it. Mini
  needs this because its float members appear only after quickening.
- **f32.** Wasm needs a second float accumulator (`sacc: f32 for rs`).
  An f64 one would convert at both ends (FRICTION 722). It is a small
  extension of `prim`'s class check, untested here.
- **Selection past ops that do not touch the slot.** wasm3's
  `PreserveRegisterIfOccupied` keeps the value across such ops. In
  Mini's loops the hot pairs were all adjacent: mandelbrot's chains
  are, and nbody's breaks are `GETFIELD` loads that need their own slot.
  So the prototype gives no evidence for it. Defer it.
- **Opcode budget.** Mini's 8-bit opcode field held 76 variants and 10
  supers. The first try, with 134 variants (all `_IF`/`_FI` members,
  every compare form), did not fit, and `Resume.placeholder` vanishes
  when every opcode is taken. Real lists must stay selective, or Mini
  must widen its opcode field.
- **Supers over variants are required.** Without them the variants
  break up the existing `#[forward]` supers and give back most of the
  gain: 0.983 against 0.921, u64. `--vm-stats` already suggests supers
  over variants. The default `#[profile]` flow should regenerate them.

**Which other interpreters gain.** These estimates come from `--vm-stats`
dispatch profiles. The stats and binaries are in
`proto-acc-x7m3/profiles/` in the scratch directory. "Share" is the
fraction of dispatches that would be a variant, counting both ends of a
single-use chain pair:

| VM | benchmark | share | notes |
|---|---|---:|---|
| MiniJS | nbody | ~35% (~47% with SETPROP readers) | `MUL_F_MUL_F__f1`→`SUB_F`/`ADD_F`, and `ADD_F`/`SUB_F`→`SETPROP_IN` (6% each). No `#[forward]` super covers these yet. Likely the best case after mandelbrot |
| MiniJS | raytrace, richards, fannkuch | 0–5% | chains broken by `GETPROP` |
| MiniJS | spectral_norm | ~0% (`_F` only) | the hot ops are generic integer `ADD`/`MUL`/`DIV` |
| Python | nbody | ~31% (~42% with SETINDEX) | `MUL`→`SUB`/`IADD`→`SETINDEX` temp chains. Python's arithmetic is generic (no `_F` members) and may allocate (Long), so it needs the traced, write-through form. `first_temp` is exact: locals are activated once at entry |
| Python | chaos, float, raytrace | 6–18% | |
| Scheme | mbrot | ~50% | 7 of 13 loop dispatches. Needs write-through selection, since there is no `first_temp` |
| Scheme | nucleic, fft | 12–24% | |
| Scheme | fibfp, sumfp, fib, tak | 0% | fibfp/sumfp want a float-constant `SUBK`/`LTK` instead: `LOADK` is 33% |
| Lox | whole suite | ≤5% | no float kernels; the results feed calls, returns and globals |

Mini's mandelbrot (73% share) gained 9–15%, and nbody (25% share, mostly
already forwarded) gained nothing. So expect gains only where the
chains are not already inside `#[forward]` supers:
- MiniJS nbody and Python nbody: about 3–6%.
- Scheme mbrot: 5–10%.
- Scheme nucleic and fft, and Python chaos: 1–3%.
- Lox: nothing.

These are estimates; the suite-wide effect stays below 1% per VM.

**Recommendation.** Build it: f64-class value accumulators,
write-through, with `Vm.accumulate` idempotent and run from `refuse`.
Two things come with it:
- `become` must leave the accumulator holding its result.
- `--vm-stats` must suggest supers over the variants.

Try MiniJS nbody first, then Python nbody, and Scheme mbrot with
unconditional selection. Spill-at-sync, selection past non-interfering
ops and GPR accumulators come later, each only if a measurement asks
for it.

#### As built (2026-11-06)

Built as recommended, with one change of design: **there are no writer
variants.** With write-through, a variant that writes the accumulator
also stores the slot, so it is the op itself plus one register write.
So every form of a *writer* (an op whose result is of a value
accumulator's kind) leaves the word in the accumulator before each
`next`, and only readers are variants. LLVM forwards the value just
stored, so that write is a register move, and none at all for a float
result. Mini's list is 20 ops and gives 28 variants (the prototype had
76 from 14 ops), so the compare-and-branch forms fit: 233 of 256
opcodes with the supers. `language.md` §7.4 has the syntax:
`accumulator vacc: f64 for reg;` over a nanbox `V`, and
`accumulate ADD_F, .., JGEKN;` naming family members.

**The hazards, by construction:**
1. *The next op may have just become a reader.* In an op that is not a
   writer, every `next` that may follow (in source order) a `become` of
   a writer stores the accumulator from the result slot (`vmacc.rs`,
   `after_become_block`). Mini's generic `ADD` keeps its integer path
   unchanged; only its quickening path pays a load.
2. *`redo` after `become`.* Nothing is needed beyond the first rule:
   the reader's writer ran just before and left the accumulator
   current, and `redo_effect` already forbids a store to a state
   register before a `redo`.
3. *`become` must not undo selection.* A writer may only `become` a
   writer of the same accumulator (an error otherwise). A variant's
   `become X` is to `X`'s variant with the same letters, or, for a
   value reader, to `X` itself (the slot it reads is current; `vmcheck`
   accepts the `__acc` field for that).
4. *Control that arrives other than from the op before.* Readers are
   never jump targets, entries zero the accumulator, and every `jump` to
   a computed target (not a label operand, `then.x` or `pc`) zeroes it:
   a return address or exception handler cannot read a stale reference,
   only the word 0, which `vmcheck` requires to hold none. A writer may
   not leave through a vm function that has `next` or `jump`, directly
   or through another (an error).
5. *Code from outside.* `Vm.verify` rejects reader variants and supers
   containing one (as an invalid opcode). Selection runs on verified
   code, where it sees every label.

**The claims, checked.** The collector, polls, `raise`, the debugger and
the JIT needed no change: GC stress, verify mode, the debugger and DAP
tests, `--jit-stencils` and TSan all pass on Mini. Two claims were wrong:
- **`Vm.deopt` needed a change.** It maps each family member to its
  generic op, and variants of plain ops (`DIV__RAR`) were in no family:
  a deoptimized writer (`ADD_F` to `ADD`) would have left a reader after
  it. A value reader is now a member of its op's family (of the op
  itself, for a plain op), so `deopt` turns it back.
- **The f64 accumulator does cost the dispatch fold.** Every tail-call
  handler's dispatch went from `jmp *T(,%rax,8)` to
  `mov T(,%rax,8),%rax; jmp *%rax`, as `lowering.md` §3 says for any
  seventh register argument, `xmm` included (the prototype blamed only
  the `u64` class). A control build of Mini with the accumulator
  declared and nothing selected measured fib 1.038, queens 1.022 and
  1.006 over eight benchmarks. That cost is what decided the ports.
  (Not so: remeasured, the control build's loss does not show, and
  folding the load back gains nothing; "The fold, recovered and not
  kept", below.)

**Ports.** User+sys CPU, median of 9 paired interleaved rounds
(min-of-runs moved ±10% between identical binaries: load 4 to 6 from
other agents), A/A in brackets; Scheme with `compare.py --cpu`, minimum
of 5:

| port | float benchmark, align 64 | align 128 | suite, align 64 | suite, align 128 | kept |
|---|---:|---:|---:|---:|---|
| Mini (14) | mandelbrot 0.912 (1.012), nbody 0.950 | mandelbrot 0.902 (0.996), nbody 0.973 | 0.994 (1.000) | 0.996 (0.996) | yes |
| MiniJS (14) | nbody 0.944 (0.998) | nbody 0.942 (0.994) | 1.004 (0.998) | 1.007 (0.982) | no |
| Scheme (43) | mbrot 0.885 | mbrot 0.87 | 1.003 (0.998) | 1.002 (0.996) | no |

- **Mini** (`bench/mini/RESULTS.md`, item 43): 20 ops, 28 variants,
  ten supers over them; mandelbrot's loop is six dispatches. queens,
  sort and spectral_norm were 1.025–1.033 at 64, 0.99–1.01 at 128;
  fib and sieve 1.04 at 128.
- **MiniJS** (`bench/minijs/RESULTS.md`): the float members, compare
  forms and SETPROP_IN; four supers take nbody's
  `b.vx -= dx * b2.mass * mag` to one dispatch (235M dispatches to
  157M). One float kernel among 14 does not pay for the fold. Reverted.
- **Scheme** (`bench/scheme/README.md`): ADD, SUB, MUL, DIV with no
  `first_temp`; mbrot's loop from 13 dispatches to 7. Integer ADD/SUB
  also pay a `movq`. Reverted.

**The fold, recovered and not kept (2026-11-06, later).** The cause is
LLVM's x86 isel pattern for a tail call through memory,
`X86tcret_6regs` (`X86InstrCompiler.td`): it counts the
`X86ISD::TC_RETURN` node's register operands and refuses to fold past
six, whatever their class or order, and whichever registers are free.
`-print-after=finalize-isel` on a six-`ptr` handler gives
`TCRETURNmi64` (folded); with a seventh `double`, `i64`, `float`, or
even a `poison` one, `TCRETURNri64` after a separate load. A
`<2 x i64>` counts once. No LLVM option changes the count, and in the
IR the only way under it is dropping or packing state: dropping the
instruction word (the callee reloads it from `pc`; tried on Mini, all
dispatches fold) moves the extra µop instead of saving it, and packing
the accumulator into a vector with another state value costs a
shuffle at every use of the other lane. Folding in the assembly works:
a pass (commit 0c3bb46) compiled x86-64 executables whose IR has a
`preserve_none` function with more than six parameters to `.s` and
rewrote `movq T(,%rI,8),%rD; ...; jmpq *%rD` to `jmpq *T(,%rI,8)` when
only plain instructions that leave `%rD` alone and do not write `%rI`
came between: Mini folded 1043 of its 1051 indirect jumps, MiniJS's
port 868 of 877, the wasm interpreter all but 18 of 1217, and every
test passed.

**It was not the cost, and the pass is reverted.** Folded over
unfolded, in paired rounds at align 64 and 128: Mini 0.996 and 1.001,
the control build (accumulator declared, nothing selected) 1.000 and
0.999, MiniJS's port 0.996 and 0.995, Scheme's 0.996 and 1.005, the
wasm interpreter 1.022 at 64 (spectralnorm 1.08 at both). The
2026-11-06 control result (fib 1.038, queens 1.022) does not come back
either: with 11 rounds at 64 and 9 at 128 the control is within the
A/A spread of no accumulator (`bench/mini/RESULTS.md`, item 44). A
dispatch microbenchmark (10^9 dispatches through four small handlers)
times 6 arguments, 7, and 7 folded within 2.5% of each other, 7
unfolded the fastest: the dispatch's critical path is the two
dependent loads, not the extra µop. What an unselected accumulator
does cost is `xmm0` being live across a hot path's C calls (GETGLOBAL
saves and restores it around `value.table_find`) and, in Scheme, the
`movq` on ADD/SUB/MUL's integer paths. The ports, re-measured as built
(unfolded), against master, A/A in brackets:

| port | float benchmark, align 64 | align 128 | suite, align 64 | suite, align 128 | kept |
|---|---:|---:|---:|---:|---|
| Mini (14) | mandelbrot 0.906, nbody 1.027 | mandelbrot 0.907, nbody 0.986 | 0.999 (0.998) | 0.993 (1.002) | yes (unchanged) |
| MiniJS (14) | nbody 0.974 | nbody 0.931 | 0.990 (0.996) | 0.997 (1.003) | **yes** |
| Scheme (43) | mbrot 0.965 | mbrot 0.887 | 1.007 (1.006) | 0.999 (0.992) | no |

User+sys CPU, median of 9 (Mini 11 at 64) paired interleaved rounds.
MiniJS is kept (6d40423 again). Scheme stays out: over five suite runs,
folded or not, it averages 0.3% slower where the A/A runs average 0.1%
faster, with tak and fib slower in every run (the `movq`); mbrot's
gain does not outweigh 42 benchmarks of mostly integer code.

**Python** is not ported. Its arithmetic ops pass `dst.raw` to `call2`
(a dunder method's frame, whose return writes the slot), so they read
their result operand and are not writers. A port would move the dunder
call out of the op first.

### 2. Cheaper operand decoding and dispatch

No-go (2026-11-06; hand prototypes on `examples/wasm`, not committed).
Pre-scaled byte-offset slot fields: PolyBench 1.003, tuning set 0.998
(A/A 1.000): the scale is free in x86-64 addressing, and the field
extraction (`shr`, `movzwl`) stays. Offset-threaded dispatch (handler
offset in the word, `lea` instead of the table load): PolyBench 1.008,
1.010 at align 128, slower on 22 of 30 kernels: the predicted indirect
jump puts the table load off the critical path, and the `lea` takes an
ALU slot.

### 3. Division by a verified constant

A 64-bit `idiv` takes 40–90 cycles on Skylake, against about 4 for a
multiply and shift, and it was hot in three interpreters: Prolog crypt
spent 27% of its samples in MOD and IDIV, MiniJS's `MODK` was 8% of
closures, and Mini had written Granlund–Montgomery by hand (`mod_magic`).
A hand copy of Mini's method into Prolog gave crypt 0.846 (A/A 1.000),
so the feature was built.

**As built.** `std.math.Divisor` ([`stdlib/math.md`](../stdlib/math.md)):
- **Range.** Divisors are 1 <= d < 2^32. The magic is 64 bits, and
  floor(n / d) = hi(2n · m) >> l for every n < 2^63, so one form covers
  i32 and i48 payloads.
- **Methods.** `div_trunc`/`rem_trunc`, `div_floor`/`rem_floor`,
  `udiv`/`urem`, and `udiv31`/`urem31` (one `imul`, for non-negative
  i32 values).
- **Layout.** Four `u32` fields, so `cache DivK = math.Divisor;` is a
  cache layout and `&cache` is a `*Divisor`.
- **The op.** A constant-divisor op is one line:
  `d = Term.Int((&cache).rem_floor(x as i64) as i48)`.

**Ports:**
- **Prolog.** `IDIVK`, `MODK`, `DIVK` and `REMK`, with the cache filled
  by the clause compiler. Results match SWI-Prolog on 98,016 lines of
  `//`, `mod`, `rem` and `div` over 65 constants, negative constants,
  zero, the i48 limits and bignums.
- **Mini.** `MODK` and `MOD` fill the cache on first run, in a `cold`
  block.
- **MiniJS.** `MODK`'s cache is filled by the emitter.
- **Python** divides slices by a run-time step, so it has no constant
  divisor.

**Timing** (new/old over whole suites, minimum of 5 interleaved runs):
- **Prolog:** crypt 0.855, geomean 0.9935 (A/A 0.9994).
- **Mini:** geomean 1.0002 (A/A 1.0013), closures 0.995 and the
  coroutine sieve 0.976. The old code was already a magic number. A
  first version used the 128-bit product and filled the cache inline;
  that gave `MODK` a stack frame and made closures 1.022, and the `cold`
  fill and `urem31` brought it to 0.998.
- **MiniJS:** closures 0.970, geomean 0.9944 (A/A 0.9979). The 8%
  `idiv` share was a 32-bit `idiv` on non-negative Ints, so the gain is
  smaller than the share.

### 4. Load facts: dereferenceable, nonnull, invariant

No-go (2026-11-06; a throwaway codegen switch, not kept). `!nonnull`
and `dereferenceable` on every `Gc` load and matched payload, and
`!invariant.load` on proto and shape fields, changed 0 of 154 Mini
handlers, 0 of 165 MiniJS handlers and 3 Python ones; timings were
inside the A/A range (Python 1.007 / 0.993, A/A 1.003 / 0.994). A
handler's loads form one dependent chain and each field is loaded once,
so there is nothing to hoist or reuse.

### 5. Smaller objects: headerless typed chunks

No-go (2026-11-06; a hand arena for Scheme pairs). Headerless 16-byte
pairs instead of 32-byte ones, prefaulted and with no collection, gave
nboyer, sboyer, mperm and earley 0.971 (align 64) and 0.993 (align 128);
24-byte pairs (an 8-byte header) and Prolog's terms gained nothing.
16-byte pairs halve the RSS, which does not pay for changing the
allocator, barrier, generational mode, verifier and debugger.

### 6. A sampling profiler in std

Every tuning round had written its own LD_PRELOAD `SIGPROF` sampler,
because `perf` is unavailable here and `--vm-stats` counts dispatches,
not time.

**As built (2026-11-06): `loimi build --vm-prof`.** The flag links
`src/prof_rt.c` into the program, as `--vm-stats` links `stats_rt.c`;
its use is described in
[language.md §7.3](../language.md#sampling---vm-prof).

**Sampling.** A `SIGPROF` timer runs at 1000 Hz (`LOIMI_PROF_HZ`). It is
a wall-clock high-resolution timer, because a CPU-time timer fires at
most once per kernel tick, which is 250 Hz here. Each sample records
the interrupted instruction from the `ucontext` (x86-64 and AArch64),
the vm's `pc` and running op, and up to 24 return addresses
(`backtrace`). The build adds `uwtable` to every function, so the stack
walk goes from a runtime function back to the handler that called it,
and links with `-z now`: glibc's AArch64 lazy-binding trampoline
(`_dl_runtime_resolve`) has wrong unwind info (its CFA leaves out the
16 bytes the PLT pushed), and a sample that landed in the first call
of `write` unwound into garbage and crashed libgcc's unwinder (one
run in three of `tests/run/vm_prof.lm` under qemu at 4 kHz; a C
reproducer with an IFUNC resolver crashes every time, and x86-64's
trampoline is right). Bound at load time, nothing runs it.

**Naming, in the process at exit.**
- **Functions** are named from the ELF symbol tables of the loaded
  objects (`dl_iterate_phdr`; `.symtab`, else `.dynsym`), so no Python
  or `nm` is needed.
- **Vm code** is named by marks the compiler records in a section,
  each an address and a key. Tail-call handlers and outlined cold
  blocks are marked by their function's address, which costs nothing.
  Inline cold blocks are marked at their start and at their exits by
  an empty inline asm (`.pushsection`), with no instructions.
- **Goto and switch.** Each handler also stores its op's number. LLVM
  merges and copies blocks between the handlers of the one function,
  so a mark at each handler's start misplaced code: switch-mode Mini
  showed `LOADBOOL` at 8% on `binary_trees`. With the stored op, the
  three modes agree.
- **JIT.** `jit_rt.c` reports every compiled instruction (start, end,
  op and the instruction's word) to a weak hook, the data
  `LOIMI_JIT_MAP` writes. JIT samples get the stencil's name and an
  exact guest pc.

**Validation against a hand sampler** (`sampler3.c`/`prof3.py`), on the
same binaries at 27 kHz and 10 kHz:

| Program | Top 5 | Largest difference |
|---|---|---|
| Mini `binary_trees` | same five, in the same order | 0.6 points |
| Python `richards` | same entries | 1 point |
| Python `raytrace` | same entries | 0.8 points |

Goto- and switch-mode profiles of Mini's `fannkuch` and `binary_trees`
match the tail-call profile, which the hand sampler cannot split,
because goto and switch modes are one function.

**Overhead.** User+sys time, minimum of 5 runs, each program under 4
environment sizes (stack placements):

| Build | Instrumentation only (`LOIMI_PROF=0`) | Sampling at 1000 Hz |
|---|---|---|
| Mini, tail-call (8 programs) | 1.017 | 1.032 |
| Python, tail-call (6 programs) | 1.016 | 1.031 |
| Mini, goto (6 programs) | 1.078 | 1.086 |

- **Instrumentation.** The tail-call cost is all the `pc` store. Marks,
  unwind tables and the report at exit cost nothing measurable.
- **Mini's `sort`.** Its 12% is where the thread-local slot falls:
  moving the slot within a padded block gives 1.00 to 1.05. It is an
  address effect, not work.
- **Goto mode** stores two words per op (`pc` and the op) inside one
  large function, and that costs 2–17% (`nbody` most).
- **Sampling.** 10 kHz costs 3–9% on Mini, and that cost is the signal,
  not the stack walk.

The profiler has no per-thread timers: with threads, a sample goes to
the thread that is running.

### 7. A shifted-int nanbox option

**Evidence.**
- **Python** went to `Int(i48)` so that sums past 2³¹ do not allocate
  (84 → 18 ms), and paid for it:
  - spectral_norm is 8–9% slower, because the overflow check is
    `shl; shl; add; jo; shr` instead of `add; jo`;
  - after the fix it is still 1.076 against i32's 1.029;
  - FRICTION 449–467: the remaining ~5% "need the tag in the low bits
    of a rotated word".
- **Mini** measured i48 at closures +8–9%, spectral_norm +4–7% and
  queens +3–5%, and kept i32 (`RESULTS.md:527-563`). Its analysis: 6
  instructions where i32 needs 4, plus sign extension before
  `cvtsi2sd`, plus the `%` guard.
- **LuaJIT** uses int32 in DUALNUM mode for the same reason.

**Proposal.** A `nanbox` layout option that stores `Int` payloads in
the high 48 bits with a zero low tag, and NaN-tags the other variants
so that they never have zero low bits. Then `a + b` of two Ints is
the tagged sum, `jo` checks i48 overflow directly, and compares are
plain word compares. Floats pay one `ror` or `xor` to decode. This is
an exchange: integer code gets faster, float code slower. So it is
opt-in, and `--explain-values` prints the costs.

<!-- snippet: skip design, not built -->
```loimi
value V repr nanbox(int_high) {      // Int(i48) as payload << 16
    Int(i48), Num(f64), Nil, Bool(bool), Obj(Gc<Obj>), ..
}
// `(Int(x), Int(y)) => Int(x + y)` lowers to `add; jo` on the words
```

**Measure.** Python's 11 programs and Mini's 14 with i48, against
their current builds.

**Prototype.** Take Python's `ADD`/`SUB`/`MUL` Int arms on the raw
words, by hand. Check the word tag with `w & 0xFFFF == INT_TAG`,
operate on `w - INT_TAG` shifted, and time spectral_norm. This is
wrong for floats, so run integer benchmarks only. Half a day.

**Cost: M**, in `values.rs` and the nanbox lowering (`lowering.md`
§2). Risk: float-heavy code (nbody, mandelbrot) pays an extra
instruction per decode. That is why it is opt-in, and it may be right
for Python and wrong for MiniJS.

**Interactions.**
- **GC.** The scanner's pointer test changes; `#[scan_raw_words]` must
  agree.
- **JIT.** Stencils follow the layout.
- **Everything else.** None.

**General:** Python and Mini (i48), possibly Prolog (`Int(i48)`).

#### Prototype (2026-11-06)

The hand prototype could not be done as written. On today's words an
`Int` is `0xFFF9 << 48 | x`, and no sequence of `add; jo` on two such
words gives the sum with an overflow flag at 48 bits: the int has to be
at the top of the word, so the whole layout has to change. And
spectral_norm is not an integer benchmark: `eval_A` divides `1.0` by
an int and the sums are floats, so a layout that is wrong for floats
cannot run it. So the prototype was the layout itself, in the
compiler: the plain nanbox's words rotated left by 16 bits and xored
with Int's block tag (below), every op on them generated by the
existing code paths (`values.rs`: `make`, `is_variant`, `payload`,
`mask_tag`, the tuple merge), and Python's `Value` switched to it.
That made Python's `ADD` int path `or %si, %bx; je; add; jo; store`,
18 instructions with operand decoding and dispatch against 28.

Its bound on Python, user+sys CPU, minimum / median of 11 interleaved
runs, against master's build, with an A/A copy of that build in the
same interleave (the quieter of two rounds; the other gave suite 0.934
and 0.951):

| | align 64 | A/A | align 128 | A/A |
|---|---:|---:|---:|---:|
| fannkuch | 0.865 / 0.870 | 0.999 / 1.003 | 0.869 / 0.872 | 0.999 / 0.998 |
| go | 0.920 / 0.918 | 1.001 / 1.000 | 0.934 / 0.934 | 0.999 / 1.001 |
| hexiom | 0.916 / 0.913 | 1.001 / 1.011 | 0.907 / 0.907 | 1.001 / 0.994 |
| nqueens | 0.914 / 0.920 | 1.000 / 0.992 | 0.923 / 0.917 | 1.007 / 0.999 |
| spectral_norm | 0.920 / 0.913 | 1.004 / 1.006 | 1.001 / 1.072 | 0.994 / 1.001 |
| deltablue | 0.958 / 0.953 | 1.001 / 1.003 | 0.958 / 0.951 | 1.002 / 1.002 |
| richards | 0.967 / 0.940 | 0.999 / 0.976 | 0.958 / 0.957 | 0.993 / 1.007 |
| chaos | 0.906 / 0.895 | 0.997 / 0.992 | 0.909 / 0.907 | 0.996 / 1.002 |
| raytrace | 0.911 / 0.921 | 1.001 / 1.006 | 0.911 / 0.914 | 1.001 / 1.014 |
| float | 0.995 / 0.996 | 0.999 / 1.001 | 0.996 / 1.000 | 1.001 / 1.002 |
| nbody | 0.990 / 0.980 | 0.999 / 0.975 | 1.012 / 1.058 | 0.997 / 1.002 |
| **suite (11)** | **0.932 / 0.928** | 1.000 / 0.997 | **0.942 / 0.952** | 0.999 / 1.002 |

Far above the 3% bar, so it was built. Two things were not expected.
The gain is not only in integer arithmetic: chaos and raytrace, float
benchmarks, gained 9%, most likely because every pointer payload
became `shr $16` in place of an `and` with a 48-bit mask that needs a `movabs` and a
register (Python's handlers have 5% fewer instructions in all: GETATTR
64 → 56, CALLEX 1091 → 1025, BUILDCLASS 520 → 469). And the float paths
cost less than feared: `ADD`'s float path went from 31 instructions to
32, because the int test in front of it got shorter; only TRUEDIV (43 →
50) and the float half of mixed operations grew. nbody, where nearly
every value is a float, is the one benchmark that is slower, at 128
only (1.01 / 1.06). spectral_norm moves with alignment (0.92 at 64,
1.00 / 1.07 at 128): `--vm-prof` (8 times the loops) shows the int
ops cheaper (ADD 279 → 189 samples, FORITER2 302 → 201) and the float ones dearer (TRUEDIV 288 → 345, IADD 127 →
194, the accumulation `partial_sum += ..`).

#### As built (2026-11-06)

`value V repr nanbox(int_high) { .. }` (`language.md` §2, `lowering.md`
§2, "`nanbox(int_high)`"):

- **The layout** is chosen exactly as for `nanbox` (classic, eight,
  packed or shared), and every word is then stored as `rotl(c, 16) ^ k`,
  where `c` is the plain word and `k` the 16-bit tag of the integer
  variant's block: the first variant with an integer payload and a
  whole 48-bit block (an error if there is none). That variant is
  `payload << 16`; every other variant's low 16 bits are its old tag
  xor `k`, nonzero; a shared block's 3-bit low tag sits at bits 16–18.
  It is a bijection of the plain layout, so canonicalized NaNs still
  cannot forge a pointer.
- **Codegen.** Variant tests are `(w & rotl(mask, 16)) == tag'`
  (`test %si, %si` for the int), payloads `w >> 16` (`ashr exact` for
  the int, which is what lets LLVM fold the checked arithmetic's
  shifts and the re-tagging away), constructors `payload << 16 | tag'`,
  the float `xor; ror` to decode, `rol; xor` to encode, and its test
  decodes before `< NUM`. Tuple tests merge as before.
- **GC.** The collector traces through the same payload extraction
  (`tests/run/gc_values_int_high.lm`: the shared layout under
  generational stress and verify, and `--gc-check-roots`).
  `#[scan_raw_words]` is refused: a raw pointer's low 16 bits can be any
  tag. The all-zero word is `Int(0)`.
- **JIT.** Stencil variants per constant tag rotate their mask and tag;
  the float gets none (its test is not a mask test of the stored word),
  and traces load float constants instead of baking them in
  (`tests/run/vm_jit_values_int_high.lm`, JIT on and off).
- **`--explain-values`** prints `repr nanbox(int_high)`, the rotation,
  the rotated masks and tags and the float's costs
  (`tests/run/values_explain_int_high.lm`). `V.Num.limit` is an error
  under it; `V.X.mask` and `V.X.tag` are the rotated ones.
- **Debugger.** Python's value printer matches on variants, so it
  needed no change (`python_debugger` transcripts pass).
- **Python** uses it (`examples/python/value.lm`); `id()` reads the
  payload above the low tag. The measurement above is the port.

**Mini: not tried when built** (it was later: "Ports", below). It is not cheap there: Mini's fast paths add two
slots as doubles before testing them (`f64.from_bits(a as u64)`, the
float-first form that gave mandelbrot and nbody 4–5%), and under
`int_high` those words are rotated, so every such path (and its
`Int(i32)` casts, for i48) would have to be rewritten first, in code
another lane is changing. What the layout would give it can be read
off the instruction counts: Mini's i32 add is `addl; jo; movl; or`, and
an `int_high` i48 add is `add; jo` with no re-tag, so i48 would no
longer cost Mini's integer benchmarks the measured 4–9%; its float
benchmarks would pay the decode, as nbody does here.

#### Ports (2026-11-06)

The other nanbox vms on `int_high`, each against master's build: user+sys
CPU, minimum of 7 (Scheme, Prolog) or 9 interleaved runs per benchmark,
an A/A copy of the baseline in the same interleave, align 64 and 128
(suite geomeans, A/A in brackets):

| vm | change | align 64 | align 128 | worst | kept |
|---|---|---:|---:|---|---|
| Prolog (30) | `Term` was already `Int(i48)` | 0.917 (1.003) | 0.920 (1.008) | sieve 0.99–1.03, log10 0.97–1.06 | yes |
| Scheme (43) | fixnums `i32` → `i48` | 0.979 (0.994) | 0.978 (0.998) | sumfp 1.24–1.27, mbrot 1.12–1.15 | no |
| MiniJS (14) | `Int(i32)` → `i48` | 0.941 (0.993) | 0.958 (1.049) | nbody 1.25–1.47, raytrace 1.05–1.07 | no |
| Mini (14) | `Int(i32)` → `i48` | 1.034 (1.003) | 1.030 (1.001) | mandelbrot 1.80, nbody 1.32, spectral_norm 1.18–1.20 | no |
| Lox (9) | no integers: `Native(u32) block 48` as the zero-tag variant | 1.005 (0.987) | 1.030 (1.007) | equality 1.06–1.13, fib 1.04–1.09 | no |

The integer side behaved as in Python everywhere: Prolog's queens_8
0.78–0.80, Scheme's tak 0.82–0.95, MiniJS's bitops 0.74–0.76 and sort
0.77–0.90, Mini's fannkuch 0.84–0.89 and queens 0.88–0.92; Mini's i48
no longer costs it the 4–9% of `bench/mini/RESULTS.md`. The float side
did not: in a tight float loop every operand read from a register is a
load, `xor`, `rol` and `movq` where it was one `addsd`/`mulsd` from
memory, and every result two more instructions before its store, all on
the loop's dependency chain (sumfp: 0.188 s to 0.259 s under
`--vm-prof`, Mini's mandelbrot 0.69 s to 1.12 s, spread over its float
supers). Python's float benchmarks pay the same per operation but spend
more of their time elsewhere. So the layout pays for a vm whose values
are rarely floats (Python, Prolog) and not for one with float kernels
among its benchmarks.

- **Compiler: `V.Num.xor`.** A float-first fast path (add two registers
  as doubles, test the result for NaN) needs the rotation's constant:
  under `int_high` an f64 with bits `b` is `rotl(b, 16) ^ V.Num.xor`
  (`language.md` §2; `tests/run/values_int_high_xor.lm`,
  `tests/fail/value_xor_not_int_high.lm`). MiniJS's and Mini's ports
  used it.
- **Prolog** (kept, `bench/prolog/README.md`): `#[scan_raw_words]` is
  refused under the layout, so its environment frames store their
  three header words as `Int` words and the scanner still reads the
  stack word by word; first-argument index keys of Ints set bit 15, as
  `Int(0)`'s word is the no-key 0.
- **Scheme, MiniJS, Mini** (`bench/scheme/README.md`,
  `bench/minijs/RESULTS.md`, `bench/mini/RESULTS.md`): each port is a
  commit followed by its revert. Mini's needed `int_index`, the integer
  compare-and-branch, FORLOOP and the intern table's slot words
  rewritten for rotated words; its value accumulator and traces were
  untouched.
- **Lox** has no integer variant; pinning `Native(u32) block 48` makes
  it the zero-tag one, so only the pointer payloads (cheaper) and the
  floats (dearer) change. Its numbers are all floats: not kept, no
  commit.

### 8. Re-fusion on `become`; supers from a profile file

A super whose parts are quickened members forms only if `Vm.optimize`
runs again after `become`. Mini and MiniJS did this by hand
(`refuse(cl)`, a whole-function `optimize`), which gave Mini mandelbrot
−6% and nbody −5%. Lox, Python and Scheme have no arithmetic families;
what they quicken is globals, properties and calls. The pairs that only
re-fusion forms are pairs of two quickened members (the second site to
quicken cannot see the word before it), and from `--vm-stats` the hot
ones were Lox's `GETGLOBAL_C; INVOKE_P` (31% of invocation and
properties), Python's `GETGLOBAL_C; GETATTR` (6–7% of deltablue) and
Scheme's `GETGLOBAL_B; ...` pairs (8–14% of conform, triangl, puzzle).
A hand prototype (re-fuse after every `become`, plus those pairs as
supers) gave Lox 0.976–0.981 (zoo 0.883) and Python and Scheme nothing,
so it was built and ported to Lox.

**`refuse CODE;`** (`language.md` §7.3, "Fusing again after
`become`"). A vm item naming the code the running instruction is in,
an expression of the handler's state of type `Verified<Vm>`:
`refuse fp.cl.proto.code;`. After every `become X` where `X` is a part
of some super (or has value accumulator variants), the handler calls
the generated `Vm.refuse(CODE)`:
`Vm.accumulate(code, 0)` first when every accumulator is a value
accumulator (so Mini's and MiniJS's variant selection comes along), then
`Vm.optimize_words`. Any other `become` stays one store: an op no super
names cannot form one, and a super it was a later part of keeps
running.

- **Why the whole function.** Forward, the next instruction is at
  `pc + size(X)`. Backward, with inline caches and ops of several sizes,
  the word before `pc` may be a cache word or an operand word, so only
  a walk from the start of the code finds the previous instruction;
  once the code is walked, `optimize`'s pass costs about the same and
  gives exactly `optimize`'s result. The pairs that matter are formed by
  the *second* `become`, which is the backward case.
- **`redo`.** `become X; redo` dispatches the word as it is after the
  pass: the super `X` heads if one formed (`tests/run/vm_refuse.lm`,
  `NEG_F_RET`), else `X` alone. A `become` inside a super first
  re-targets or splits the head as before; the pass then sees the
  result.
- **JIT and traces.** `optimize_words` calls
  `loimi_jit_invalidate`/`loimi_jit_rewritten`, which uninstall traces
  and re-patch the stencils; the test runs under the JIT matrix.
- **Cost.** One pass over the function per `become` (three `calloc`s
  and a DP over its words), as Mini's hand `refuse` paid.

**Ports** (base: Mini's and MiniJS's hand `refuse`):

| Suite | Change | min / median | A/A min / median |
|---|---|---:|---:|
| Lox (9, 11) | `refuse fp.cl.proto.code;`, six supers of quickened members | 0.973 / 0.986 | 1.013 / 0.997 |
| Lox, `--align-functions=128` (9, 9) | same | 0.985 / 0.984 | 1.002 / 1.005 |
| Mini (14, 9) | the hand `refuse` (5 calls) replaced by the item | 0.999 / 0.995 | 0.997 / 0.994 |
| Mini, 128 (14, 5) | same | 1.002 / 1.002 | 1.000 / 1.001 |
| MiniJS (6 object benchmarks, 9) | the hand `refuse` (6 calls) replaced | 1.001 / 1.008 | 1.001 / 0.996 |
| MiniJS, 128 (14, 7) | same | 1.003 / 1.005 | 0.997 / 1.001 |

Lox and Mini use the item: Lox gains zoo 0.883 (0.878 at 128) and
invocation 0.91–0.98. MiniJS keeps its hand `refuse` (arithmetic only):
re-fusing after property quickening formed `GETMETHOD_P_CALL`,
`GETPROP_IN_GETMETHOD_P` and `GETPROP_IN_GETPROP_IN` in deltablue, 10%
fewer dispatches, yet deltablue ran 1.006–1.06 slower. Re-fusing after
every `become` of a family in a super, misses included, was slower
still (MiniJS 1.010).

**`supers from "file";`** reads the first list of supers that
`--vm-stats` suggests in the `== vm stats: NAME ==` sections of the file
(or plain `super` lines), skips stale ones (parts the vm does not have)
with a warning and ones declared already silently, and accepts
`#[forward]` and `#[allow(super_overlap)]` on the item
(`src/vmexpand.rs`). The examples' supers are hand-picked from several
reports and measured one by one, so none of them uses it.

### 9. A value-stack and frame runtime

Calls clear the callee's registers so that the collector never marks a
stale value. MiniJS, Scheme and Python had each replaced that by hand
with a high-water mark below which every slot may be marked (Scheme:
fib 0.496 → 0.400 of Guile's time). Lox still cleared at every call;
removing the clearing (unsafe) bounded the gain at suite 0.968
(fib 0.87).

**As built (2026-11-07): `std.rt.ValueStack<T>`** (`stdlib.md`,
std.rt):
- `fits(top)`: one compare against the mark;
- `raise(top)`: `#[cold]`, clears only never-written slots (plus
  `SLACK`) and raises the mark to them;
- `ensure(top)`: `fits`, else `raise`;
- `lower(top)`: for the scanner.

A `fresh` pointer separates the slots the scanner has cleaned (between
the mark and `fresh`) from the never-written ones. So raising the mark
after a collection clears nothing, which is MiniJS's protocol, without
filling the stack when it is allocated. Frames are not part of it: all
five interpreters keep frames in a separate array with its own limit
compare, and their layouts differ (Lox 3 fields, Scheme 4, MiniJS and
Mini 5, Python 9).

Two lessons from the ports:
- **Hot paths call `raise` from a `cold` block that ends in `redo`.**
  A first Scheme port that called `ensure` on the hot path gave every
  call handler a stack frame (`push`/`sub rsp`): fib 1.05, ntakl 1.08.
- **`raise` must not clear slots between the mark and `fresh`.** A
  first version cleared from the mark up and broke MiniJS under GC
  stress, because MiniJS writes argument slots above the mark before a
  call raises it.

**Ports** (minimum of 7–9 interleaved runs):

| vm | change | align 64 | A/A | align 128 |
|---|---|---|---|---|
| Lox | clearing at calls → `ValueStack` | 0.970 (fib 0.863, binary_trees 0.939, trees 0.985) | 1.004 | 0.966 (fib 0.867, binary_trees 0.918, trees 0.961) |
| Scheme | `hwm` + `rt.grow` → `ValueStack` | 1.000 (fib 1.017, tak 0.990) | 1.000 | 0.995 |
| MiniJS | `hw` + `grow_stack` → `ValueStack` | 0.995 | 0.998 | 1.002 (A/A 1.004) |

Lox's tests pass with a poison value in every register it no longer
clears. Python never lowers its mark and clears nothing, so it has
nothing to gain. Mini relocates a stack per coroutine and would have to
lower the mark for suspended coroutines, resumers and `--cfg gil`
threads; its third hand attempt measured 0.995–1.003, and Mini still
clears.

### 10. Adaptive quickening (warmup and backoff counters)

No-go (2026-11-07). `--vm-stats` counts every `become` per site and
transition (`language.md` §7.3): over the Mini, MiniJS, Lox, Python and
Scheme suites, later `become`s are at most 0.055% of dispatches (MiniJS
deltablue). The other waste a counter could avoid, float members that
keep missing (MiniJS strings and bitops), is bounded by a vm whose `ADD`
never quickens: 0.982 on bitops, 0.994 on strings.

### 11. Coroutine and generator switching

Mini's coroutine resume and yield were cold blocks reached through
CALL's native-call path, 31% of chameneos, the one Mini benchmark slower
than LuaJIT `-joff` (1.06).

**Built in Mini (no language change).** `CALL` is a family: a site
whose call switched threads (`coroutine.resume`, a wrap function,
`coroutine.yield`) quickens to `CALL_RESUME`, `CALL_WRAP` or
`CALL_YIELD`, which repeat CALL's test and switch on the hot path, and
`become CALL; redo` when it fails. Three supers keep the fused forms
(`MOVE, MOVE, CALL_RESUME`, `MOVE, CALL_RESUME`, `GETUPVAL,
CALL_YIELD`). Against the commit before (A/A in brackets; minimum of 11
runs):

| | align 64 | align 128 |
|---|---:|---:|
| chameneos | 0.925 (1.002) | 0.919 |
| coroutine_ring | 0.961 (1.002) | 0.971 |
| sieve | 0.944 (1.003) | 0.927 |
| the 14 programs, geomean (7 runs) | 0.994 (1.001) | 0.993 |

chameneos is now 0.91 of LuaJIT `-joff` (0.357 s against 0.391 s),
coroutine_ring 0.89 and sieve 0.88. The switch stays ordinary vm control
flow (`jump` to a `Mini.Resume`, which compiled code takes as a resume
jump), so the JIT and traces need nothing new.

**Rejected: a `switch_state` vm primitive** (2026-11-06). A state
snapshot saves only Mini's two dependent loads `fp.base` and `fp.k`,
which are off the critical path; the rest of a switch (about 35–40
cycles) is the mispredicted jump and Mini's own bookkeeping, which a
primitive would have to store too. Python generators already switch on
the hot path (FORITER 10.9% and YIELD 1.9% of nqueens samples), and
their register copy is the design.

### 12. Trailing immediate words

**Evidence.**
- **Wasm keeps wide immediates in cache words.** `CONST64`, table
  indices and segment indices sit in cache words (`cache Imm64`),
  which the verifier cannot see, so they are bounds-checked on every
  execution (wasm FRICTION 957–979).
- **`roadmap.md` Q10** designs the fix and has not built it.

**Proposal.** As in Q10: `format RRRX { .. } + imm: u64`, checked once
by `Vm.verify`.

<!-- snippet: skip design, not built -->
```loimi
format RRX { op: 16, a: 16, b: 16, c: 16 } + imm: u64
op CONST64(dst: rl, v: imm) : RRX { dst = v as i64; next }
op TABLE_GET(dst: rl, i: ri, t: table) : RRX { .. }    // `t` verified once, unchecked here
```

**Measure** the wasm suite.

**Prototype.** Drop the run-time checks on the cache-word indices with
`unsafe` (the programs are valid) and time the wasm suite. That bounds
the gain at once.

**Cost: M.** **Interactions:**
- supers lay out each part's trailing words, as for caches;
- the JIT patches them as `const` holes.

**Niche:** wasm (and any VM with wide constants).

### 13. JIT stencils: computed jumps to a target

Mini's SWITCH jumped to a JMP that jumps to the arm, one dispatch more
than needed. Decoding the target in SWITCH itself was 4% faster on
interp, but crashed `--jit-stencils`: SWITCH *read* the JMP it picks,
and the JIT had replaced that compiled JMP's word with an enter word
(`ENTER | index << 8`), which SWITCH decoded as a jump offset. The
generated field getter mapped through `Vm.real_word`, but that was a C
call into the runtime (a treap lookup), and interp under the JIT took
1.33× as long with it.

**As built (2026-11-06)** (`jit_rt.c`, `vmexpand.rs`, `codegen.rs`,
`vmgen.rs`):
- **`Vm.real_word` is an inline load.** The runtime keeps
  `loimi_jit_words`, the real word per entry index (the enter word
  itself while the entry is not in the code), updated wherever it
  writes, rewrites (`become`) or restores an enter word. `real_word` is
  `if opcode(w) == ENTER { loimi_jit_words[w >> op_bits] } else { w }`,
  and the load is IR the compiler emits (`loimi_jit_real_word`), not a
  call. So a handler may read another instruction's word in the
  interpreter and in its stencil; host readers (disassembler, verifier)
  use the same table.
- **Computed jumps enter compiled code directly.** In a stencil, a
  `jump` whose target is computed from pc but is none of the op's label
  candidates tests the target's word: an enter word goes straight to
  its compiled code (one indirect jump instead of two), anything else
  to the interpreter at that pc. MiniJS's SWITCHI takes it too.
- **Test:** `tests/run/vm_jit_switch_target.lm` (`matrix: jit`), an op
  that decodes the JMP it picks and jumps to its target, compiled at
  the first backward jump, and host reads of the words afterwards.

**Mini's SWITCH** decodes a real JMP and jumps to its target. When the
JMP's word is an enter word (compiled), it jumps to the JMP instead,
whose code is one direct `jmp` to the target's: decoding through
`real_word` there measured 1.02 against the JMP. Minimum of 11 (JIT:
21) interleaved runs, A/A in brackets:

| interp | align 64 | align 128 |
|---|---:|---:|
| interpreter | 0.977 (0.998) | 0.947 (1.012) |
| JIT (`LOIMI_JIT=1`) | 0.998 (0.991) | 1.002 |
| JIT, 14 programs, geomean (5 runs) | 0.997 (0.995) | |

In compiled code the JMP was already a direct `jmp`, so the JIT has no
dispatch to save. Wasm's BR_TABLE to its target measured 1.796 against
1.797 s and was left as it was.

### 14. An optimizing tier

Built as the trace tier: [`trace-tier.md`](trace-tier.md).

## 2. Reference techniques, against what Loimi offers

**N** native, **H** by hand only, **X** not expressible.

| Technique | Used by | Loimi |
|---|---|---|
| State in fixed machine registers | LuaJIT (`vm_x64.dasc:19-56`), wasm3 | N: `state` under `preserve_none` (`lowering.md` §3) |
| Decode-ahead, replicated dispatch | LuaJIT `ins_NEXT` (`:212-224`) | N: decode-ahead; goto mode copies the dispatch (`lowering.md` §5) |
| Handler address in the code stream | wasm3 | X (item 2 measured the nearest form) |
| Operand slot as a byte offset | wasm3 (slot offsets in their own code words) | H: an operand-kind expression, if the encoder scales by hand (item 2) |
| NaN tagging, dual int/double | LuaJIT | N: `nanbox`, merged tuple tests (`language.md` §2); `nanbox(int_high)` (item 7) |
| Top of stack in a register | wasm3 `_r0`/`_fp0` (`m3_exec.h:119-146`) | N: accumulators (item 1) |
| Stack caching with several states (Ertl) | Forth systems | H: Stak's `tos` state variable (10–15%, Stak FRICTION) |
| Inline caches in the code stream | CPython `_Py*Cache` | N: `cache`, `cache X = S` (`language.md` §7.1) |
| Quickening, families, deopt | CPython PEP 659 | N: `family`, `become`, `Vm.deopt`, `refuse` (`language.md` §7.2–7.3) |
| Warmup and backoff counters | CPython `pycore_backoff.h` | H: a cache field (item 10: not needed) |
| Static superinstructions | CPython (LOAD_FAST_LOAD_FAST), QuickJS | N, beyond the references: `supers`, DP `optimize`, `#[forward]`, suggestions, `supers from` (`language.md` §7.3) |
| Dynamic superinstructions, replication (Piumarta & Riccardi) | — | N in effect: `--jit-stencils` copies stencils per instruction |
| Compare fused with the next JMP | LuaJIT ISLT, Lua 5.4, wasm3 `BranchIf_r` | N: `then OP` (`language.md` §7.1a), `rel` (`language.md` §7.5) |
| Ops per operand kind (ADDVN, ADDK, H_SMALLINT, get_loc0) | LuaJIT, Lua 5.4, SWI, QuickJS | H: `inline for` over kinds, `rel kform` |
| Lazy compile by self-patching | wasm3 `op_Compile` | H: `Vm.Resume` caches, `become` |
| Unboxed typed slots | Guile (`uadd`, `fadd`, `untag-fixnum`) | N: several operand-kind views of one slot (wasm) |
| Hot counters, tier-up | LuaJIT `hotloop`, Guile `instrument-entry` | N inside `--jit-stencils` |
| Zero-cost hooks when off | Lua `trap`, Guile `NEXT_HOOK` | N, better: instrumentation variants swap tables (`language.md` §9) |
| Clause indexing | SWI-Prolog | interpreter runtime work (Prolog's first-argument CALL indexing) |
| Trace JIT | LuaJIT | N: the trace tier (item 14) |

## 3. Tried and rejected

Items 2, 4, 5 and 10 and the `switch_state` primitive of item 11 are
above. The rest:

- **Guard pages instead of wasm bounds checks.** 4 GiB plus a guard per
  memory counts against `ulimit -v`; the check is already a fused,
  predicted `add; cmp; ja`.
- **Loop-carried MOV elimination.** wasm's local MOVs (7–10% of
  dispatches, 20% in seidel-2d) rotate values live at once; MOV2 merging
  measured 0.997 and 0.998.
- **Guest registers in machine registers, in the JIT.** One cached
  register measured 1.015 (`copy-and-patch.md` §13); the trace tier
  does this instead.
- **Stencil hot/cold splitting, constant patching in the JIT.** No gain
  (`copy-and-patch.md` §10, §12).
- **Frame-free slow paths through inline-asm `preserve_all` stubs.**
  0.995 over 102 benchmarks, `.text` +8–30%, Stak goto 1.34× slower
  (`lowering.md` §3).
- **Pinning hot constants (the nanbox tag) in tail-call mode.** A
  rematerialized constant is one `movabs` off the critical path, and
  goto mode, which keeps them in registers, is 1.03× tail-call on Mini.
- **Handler replication for branch prediction** (Ertl & Gregg).
  Tail-call and goto modes already give every handler its own indirect
  jump; what remains is placement, which `#[profile]` ordering
  addresses.
- **Type-check elimination across ops.** Within a super LLVM merges
  repeated tag tests; across ops it needs types per slot, which traces
  have.
- **Interner layouts.** MiniJS's records with inline key bytes and
  Mini's shrink thresholds measured noise or worse.
- **A moving or bump-pointer nursery.** Python's generational mode
  measured 0.982 and Prolog's 1.01, and moving breaks raw pointers in
  state registers and `Vm.Resume` caches.
- **Int(i48) in Mini.** +4–9%; Mini keeps i32 (item 7 is the
  representation that makes i48 cheap).
- **Direct `call`/`ret` for guest calls.** Call cost in the logs is
  frame stores, clearing and checks, not mispredicts, and native returns
  would put guest recursion on the C stack.
- **`--cpu=x86-64-v3` / `native`.** Noise (§4).
- **A balanced compare tree for if-chains.** Hot arms come first.
- **Spill-at-sync value accumulators.** Write-through cost Mini
  mandelbrot about 6 points (0.855 raw against 0.914), under 1%
  suite-wide.

## 4. LLVM, clang and linker features (2026-11-07)

Already in use: `preserve_none` and `musttail` handlers, outlined `cold`
blocks and `llvm.expect`, the goto-mode tail-duplication options,
`-enable-tail-merge=false` for switch mode, JCC-erratum branch
alignment, `--align-functions`, `nsw`, `-O3`, `--cpu`, vectorize-width
metadata, and hot-handler ordering (`.text.hot.vm.X` sections in profile
order).

**Method.** Each candidate was first a disassembly diff against the
baseline, function by function, with addresses stripped; one that
changes no handler stops there. The rest were timed (tail-call, min /
median of per-program ratios as geometric means). Mini has 14 programs,
Lox 9, Prolog 30 and MiniJS 14. The first screen ran at load average
4–6 (A/A up to 1.04 / 1.05 on Lox), so it only rules out large effects;
runs marked *quiet* had A/A within 0.999–1.002.

| Feature | What was tried | Result (A/A in brackets) | Decision |
|---|---|---|---|
| `--cpu=x86-64-v3` | Mini, Lox, Prolog (5 runs, noisy); MiniJS and the Mini JIT (`LOIMI_JIT=1`; 7 runs, quiet) | Mini 0.996 / 1.010 [0.995 / 1.007], Lox 1.075 / 0.998 [1.039 / 1.054], Prolog 1.016 / 1.015 [1.011 / 1.010]; quiet: MiniJS 1.008 [1.001], Mini JIT 1.000 [1.001] (mandelbrot 0.94) | no gain; the default stays the portable baseline |
| `--cpu=x86-64-v2` | same three suites (noisy) | 1.004, 1.000, 1.007; it changes little but `floor` calls into `roundsd` (14 Mini functions, 3 Lox) | no |
| `-mtune=skylake` | disassembly | 0 functions differ (Mini, Prolog) | stopped |
| `-mllvm -enable-ipra` | disassembly, timing (noisy) | 0 Lox handlers differ, 4–22 other functions per vm; 0.999, 1.030, 1.022, inside A/A | no: `preserve_none` tail calls leave it nothing |
| `-falign-loops=32` | timing (noisy) | 0.995, 0.972, 1.011 | no: handlers have no loops |
| `-mllvm -enable-ext-tsp-block-placement` | timing (7 runs, quiet) | Mini 1.001 [1.000], Lox 1.008 [0.999], Prolog 1.001 [1.001] | no |
| `-static` (no PLT; stands for `-fno-plt`) | timing (noisy) | 1.002, 1.016, 1.017; binaries 2.4–4× larger | no |
| `-fuse-ld=lld` | timing (noisy) | 0.991, 0.985, 1.012 | no |
| `noalias` on every handler pointer parameter (an upper bound for TBAA or scoped `noalias` there) | hand-edited `.ll` | 25 of 260 Lox, 75 of 635 Mini, 21 of 235 Prolog vm functions differ, mostly slow paths; Mini 0.977 [0.983], Lox 1.027 [1.002], Prolog 1.002 [1.000] (7 runs, noisy) | no, for item 4's reason: a straight-line handler has nothing to reorder |
| IR PGO, trained on the timed programs (upper bound) | below | quiet: Lox 0.972 [1.000], MiniJS 0.970 [1.001], Prolog 0.978 and 0.971 [1.001]; Mini 1.007 [0.999], of which mandelbrot 1.45 and the rest 0.92–1.02 | no |
| IR PGO, held out | trained on half of the suite, timed on the other half | Prolog 0.993 and 0.985, MiniJS 0.980 and 0.988 [1.001 each] | under 2% |
| Runtime C linked into the program's IR | inspection | the collector and allocation are already generated IR in the program's module; `jit_rt.c`, `prof_rt.c` and `stats_rt.c` are JIT compilation and profiling, not fast paths | does not apply |
| Symbol ordering file, `-z keep-text-section-prefix` | inspection | hot handlers are already placed by section and profile order (`hotorder`); GNU ld's default script already groups `.text.hot` and `.text.unlikely` | covered |
| Huge-page text | inspection | `.text` is 240–640 KB, under one 2 MB page, and file-backed text gets no transparent huge pages here | no |
| `!unpredictable` on dispatch | inspection | tail-call and goto dispatch are indirect jumps, not switches; on a switch it only steers select/branch formation | no |
| Machine outliner, `-fno-semantic-interposition` | inspection | the outliner is off on x86-64; executables are `-fno-pic -no-pie`, so nothing is interposable | no |

PGO was measured without a profile runtime (`libclang_rt.profile` is not
installed): `opt -passes=pgo-instr-gen`, the increments lowered by script
to a plain counter array written at exit, then `pgo-instr-use` on the
original `.ll`. Held out it gains 1–2%, and Mini's mandelbrot is 1.45×
slower with the profile (its hot handlers differ only by a moved dispatch
tail and padding: placement), so it is not used.
