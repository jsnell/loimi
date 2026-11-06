# Design note: a speed audit (features for faster interpreters)

Status: **proposal.** Written 2026-11-06 against master at 0813bc1.
Items 1, 3, 6 and 8 are built (their "As built" notes); items 2 and 4
were prototyped and dropped (their "Prototype results"); item 5's upper
bound was measured and it is not worth building for speed; item 14 has
an experiment and a staged plan (`trace-tier.md`).

> Have we exhausted the possibilities for language improvements that
> can be used to write faster interpreters or runtimes?

**No.** But the easy part is gone, and what is left is smaller than the
gains so far. Until now the speed work was mostly tuning rounds on one
interpreter at a time (`bench/mini/RESULTS.md`, `bench/minijs/RESULTS.md`,
`bench/scheme/README.md`, the examples' `FRICTION.md`). A language
feature was added only when a round needed one: accumulators,
`#[forward]` supers, `try_new`, polls, mark bitmaps and `#[profile]` all
came that way. This note is the systematic pass. It asks which
language, compiler, runtime or std features would let an author write
a faster interpreter than they can today, and it ranks them.

**Method.**
- **Logs.** I read every "Dropped", "Tried", "Not built" and "Next
  candidates" list in the bench logs (`bench/*/RESULTS.md`,
  `bench/*/README.md`), the friction logs (`examples/*/FRICTION.md`),
  `docs/results.md`, `lowering.md`, `status.md`, `roadmap.md`,
  `CHANGELOG.md` and `copy-and-patch.md`.
- **Reference sources.** I read the cores of the references' sources
  under `bench/*`:
  - LuaJIT's `vm_x64.dasc`;
  - wasm3's `m3_exec.h` and `m3_compile.c`;
  - CPython 3.13's `pycore_code.h` and `pycore_backoff.h` (only the
    headers are present);
  - Guile's `vm-engine.c`;
  - SWI-Prolog's `pl-vmi.c`;
  - QuickJS and PUC-Lua.
- **Existing features.** I checked every candidate against
  `language.md`, `lowering.md` and the design notes, so that nothing
  proposed already exists.

The profile data is what the logs already hold, from SIGPROF samplers
and `--vm-stats` counts. No new long runs were made: the logs are recent
(the newest is 2026-11-05) and agree with one another. Each number below
has a file reference. A number marked **(est.)** is my estimate, not a
measurement.

**What qualifies.** A proposal must make interpreters faster, not only
shorter. Where the gain is mostly ergonomic, the item says so and is
ranked by its speed effect alone.

**The short answer.** Three layers of cost are left in a tuned Loimi
handler:
1. **Dispatch and operand decoding.** About 11 of the 14 instructions
   of a three-operand float op, against wasm3's 5 (`roadmap.md` Q11).
   The JIT removes most of it (0.76–0.83 of the interpreter's time),
   but the interpreter still pays it.
2. **Memory traffic through the register file.** Store-to-load
   forwarding between dependent ops. Accumulators and `#[forward]`
   supers remove part of it, but only for numbers.
3. **Runtime work.** Calls and frames, object size, the collector,
   division, hashing.

The features that remain are in these layers, and none of them is
worth more than a few percent suite-wide alone. An optimizing tier
would buy more: LuaJIT's remaining 2–4× on numeric loops needs register
allocation across a loop (`copy-and-patch.md` §7, §13). That is a
project, not a feature (item 14).

## 1. Ranked proposals

The rank weighs the expected suite-wide gain (how many interpreters and
benchmarks, and how much) against the cost. Cost is S (days), M (a week
or two) or L (more). "Gain" is the measured upper bound where there is
one, else an estimate.

| # | Proposal | Evidence | Gain | Cost |
|---|---|---|---|---|
| 1 | **Accumulators for `value` types** (GC-safe, write-through or spilled at sync points), f32 accumulators, selection past non-interfering ops | Only wasm can use accumulators; it got 0.91, and 0.79 with supers. The five nanbox VMs (Mini, MiniJS, Lox, Python, Scheme) cannot (`vmacc.rs:267`). The latency probe puts an 8% bound on one cached register (`copy-and-patch.md` §13). Store-to-load chains appear in mandelbrot, nbody and spectral_norm | 2–5% on numeric benchmarks in 5 VMs (est.) | M |
| 2 | **Cheaper operand decoding and dispatch**: pre-scaled byte-offset fields, then offset-threaded dispatch (a handler offset in the word, no table load) | `F64_MUL` is 14 instructions: 6 decode, 3 work, 5 dispatch. wasm3's is 5 (`roadmap.md` Q11). Every interpreter pays this on every op. **Prototyped 2026-11-06: no gain on x86-64** (item 2, "Prototype results") | measured: 1.00 (step 1); 1.01, slower (step 2) | M, then L |
| 3 | **Division by a verified constant**: `std.math.Divisor` (Granlund–Montgomery) and a verify-time hook that fills it | Prolog crypt: MOD/IDIV 27% of samples. MiniJS closures: MODK's `idiv` 8%. Mini built a magic-number IC by hand (`vm.lm:833`), and recomputing the magic on a miss cost 12%. Python slicing: `idiv` 14% (special-cased by hand) | 5–20% on 3–4 benchmarks; ~0.5% suite | S |
| 4 | **Load facts**: `dereferenceable`/`nonnull`/invariant loads on pointers and `const` operand kinds | Listed in Mini's "remaining compiler issues" (`RESULTS.md:579`). Dependent-load chains: Python GETATTR 14–26%, MiniJS GETPROP_IN. GETINDEX_GETINDEX (20% of interp) keeps a frame. **Prototyped 2026-11-06: no handler of Mini or MiniJS changes; no gain** (item 4, "Prototype results") | measured: 1.00 | S |
| 5 | **Smaller objects**: typed small-object chunks with no per-object header (BiBoP), or an 8-byte header for fixed-size types | Every object has a 16-byte header. A Scheme pair is 32 bytes against Guile's 16, and CAR alone is 12–15% of nboyer. Size sensitivity, measured: +8 bytes per object cost MiniJS binary_trees 6% and Python float 6% | est. 3–10%; **upper bound measured 2026-11-06: 1–3%** on nboyer, sboyer, mperm, earley (16-byte pairs, no collection, faults excluded), 24-byte pairs and Prolog ≈0; RSS halved. No-go (item 5) | L |
| 6 | **A sampling profiler in std** (`std.prof`), mapped to handlers, cold blocks and guest pcs | Every tuning round wrote its own LD_PRELOAD SIGPROF sampler (wasm FRICTION 366, Python FRICTION 618, Mini RESULTS:9). The 64 KiB verifier memset (Prolog sieve 0.54) was found only by sampling | indirect; it finds the next items. **Built 2026-11-06** (`--vm-prof`, item 6): top entries match the hand sampler within 1 point; costs 3% (tail-call) at 1000 Hz | S–M |
| 7 | **Shifted-int nanbox option** (Int payload in the high bits, so `add; jo` checks overflow) | Python's i48 overflow check is `shl; shl; add; jo; shr`, costing spectral_norm 5–8%. Mini measured i48 at +4–9% and kept i32 (`RESULTS.md:527`) | 3–8% on integer-heavy code in Python, and i48 for Mini (est.) | M |
| 8 | **Re-fusion on `become`**, and supers from a profile file | Mini and MiniJS call `refuse` (a whole-function `optimize`) by hand after quickening; doing so gave mandelbrot −6% and nbody −5%. Lox, Python and Scheme don't do it. **Built 2026-11-06** as `refuse CODE;` (item 8, "As built"): Lox zoo 0.88, suite 0.973; Python and Scheme no gain | measured: Lox 0.97; others 1.00 | S |
| 9 | **A value-stack and frame runtime** (`std.rt.ValueStack`): high-water-mark clearing across coroutines, nested runs and the debugger | Built by hand in MiniJS, Scheme and Python; for Scheme it gave fib 0.81 and tak 0.85. Mini's three attempts failed: the unsafe upper bound was fib 0.92, the safe attempt 0.995–1.003. Python's frames do 8 stores | 1–2% for Mini and Lox; mostly ergonomic for the rest | M |
| 10 | **Adaptive quickening**: warmup and backoff counters on families (CPython's PEP 659) | No interpreter counts; all specialize on first execution. There is no measured flip-flop yet, so measure first | 0–2% (est.) | S |
| 11 | **Coroutine and generator switching** as a vm primitive (`suspend`, state snapshot and restore) | Mini's resume/yield cold blocks are 31% of chameneos, the one Mini benchmark slower than LuaJIT `-joff` (1.06). Python generators copy every register on YIELD and resume | 10–30% on coroutine benchmarks; niche | M |
| 12 | **Trailing immediate words in formats** (`roadmap.md` Q10) | wasm keeps CONST64 and table and segment indices in cache words, bounds-checked on every execution | ~1% for wasm; niche | M |
| 13 | **JIT stencils: computed jumps that exit to the target** | Mini's SWITCH straight to its target was 4% faster on interp (interpreter) but crashes `--jit-stencils` | 4% on one benchmark | S |
| 14 | **An optimizing tier** over loops or traces: register allocation, hoisted guards, deoptimization from shared frames | LuaJIT's remaining 2–4× (`copy-and-patch.md` §7). Bound by hand: 0.56–0.68 of the JIT's time on mandelbrot's loop | large on numeric loops | L+ |

**Prototype first: 2, 1 and 3** (section 3 has the experiments).
- **Item 2** touches every op of every interpreter, and an IR-level
  experiment answers it in a day.
- **Item 1** is the only way the five dynamic-language interpreters can
  get what wasm3 and our wasm interpreter already have. Its upper bound
  can be measured on Mini without the GC work.
- **Item 3** is small, certain and fixes measured hot spots in three
  interpreters.

**Item 5** has the largest possible gain of the runtime items, but it
is the most expensive: measure its upper bound (section 3) before
anyone designs it. **Item 6** is cheap and makes every later round
faster to do.

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

**Evidence.**
- **The instruction count** (`roadmap.md` Q11, wasm FRICTION 37–83).
  `F64_MUL d, a, b` is 14 instructions:
  - 6 decode three 16-bit fields (`mov`/`shr`/`movzwl`, and scaled
    addressing);
  - 3 do the work;
  - 5 dispatch: load the next word, advance `pc`, extract the opcode,
    load the handler, jump.

  wasm3's `op_f64_Multiply_rs` is 5: the slot offset is a byte offset
  in its own word, and the handler address is in the code stream.
- **Fewer, dearer dispatches.** wasm3 runs 40–60% more ops at about
  half the cost each.
- **LuaJIT** pre-decodes RA and RC in its `ins_NEXT` (`vm_x64.dasc:212`)
  but still indexes a table.
- **The JIT shows what this layer is worth.** It removes exactly this
  (dispatch and decode, 20–40% of handler-bound time), and gives 0.76
  (Mini), 0.83 (MiniJS) and 0.79 (Scheme).
- **What was tried and not done:**
  - narrow loads of fields from the code (1.044, slower: more loads
    against ALU ports);
  - folding the table load into the jump (1.007: no gain).

  Pre-scaled fields and direct threading were never tried (Q11, "not
  built").

**Proposal, in two steps.**
1. **Pre-scaled fields.** A format field may declare a scale,
   `a: 16 * 8`, which the encoder applies once. The operand kind
   `regs[it]` then becomes `*(regs as *u8 + it)`, with no index scaling
   and no `movzwl` where the field is the word's low half. The
   verifier checks that the offset is a multiple of the scale and in
   range.

   It is a change to the encoder and to operand-kind lowering. The
   handlers' source is unchanged. Disassembly and `Vm.decode` divide
   again.
2. **Offset-threaded dispatch** for `u64` words, opt-in
   (`#[dispatch(tailcall, threaded)]`). The op field holds
   `(handler - base) >> 6`, since handlers are 64-byte aligned
   (`--align-functions=64`). A 16-bit field then reaches 4 MiB of
   handlers. Dispatch becomes `lea base + field << 6; jmp`, with no
   table load and no dependency on a table in L1.
   - `become` writes the handler offset, not the opcode, and `Vm.deopt`
     and serialization map back.
   - Instrumentation variants need one copy of the handler block per
     variant, at the same offsets. Pad the handlers to a common stride
     per op, or keep table dispatch in variant builds.

<!-- snippet: skip design, not built -->
```loimi
// before
encoding fixed(u64) { format RRR { op: 16, a: 16, b: 16, c: 16 } }
operand rd = *((regs + it) as *f64);        // shr, movzwl, (,%r,8)
// after
encoding fixed(u64) { format RRR { op: 16, a: 16 * 8, b: 16 * 8, c: 16 * 8 } }
operand rd = *((regs as *u8 + it) as *f64); // shr, (%regs,%r)
#[dispatch(tailcall, threaded)] vm W { .. }
```

**Measure.**
- The wasm suite and PolyBench, which are the most handler-bound.
- Mini, Stak and regex.
- Instruction counts per handler from `objdump`, as in wasm FRICTION.

**Prototype.**
- **Step 1, in an afternoon.** Edit the encoder of one format in
  `examples/wasm/vm.lm` by hand: store `slot * 8`, change the operand
  kinds to byte offsets, and rebuild. No compiler change is needed,
  because operand kinds are expressions. If PolyBench does not move by
  2%, stop.
- **Step 2.** Patch the IR (`--keep-ll`) of the wasm tail-call build.
  Replace the `getelementptr @tbl` and its load with
  `inttoptr(base + (op << 6))`, and post-process the word stream with
  the handlers' offsets read from `nm`.

**Cost.** Step 1 is S–M (encoder scaling, verifier, `Vm.decode`).
Step 2 is L. Risks:
- **Step 1:** the field width shrinks by the scale's bits. With 16-bit
  fields and 8-byte slots that leaves 8 Ki slots, which is enough for
  wasm's frames but must be checked per vm.
- **Step 2:**
  - code words become build-specific: serialized code needs a
    relocation pass, which `Vm.deopt` already shows how to write;
  - u32 words have too few bits to spare;
  - goto and switch modes cannot use it;
  - the JIT's enter words are a second kind of word in the stream.

**Interactions.**
- **Step 1** is invisible to lifetimes, GC, `redo` and the debugger,
  which read `Vm.decode`.
- **Step 2:**
  - instrumentation variants need parallel handler blocks or table
    dispatch;
  - `#[profile]` ordering must keep the 4 MiB window;
  - the JIT's enter-word rewrite becomes a handler-offset rewrite;
  - accumulators and supers are just more handlers.

**General:** every interpreter for step 1, and those with u64 words for
step 2 (wasm, Stak; Mini and MiniJS would need wider words).

#### Prototype results (2026-11-06): no gain, no-go

Both steps were built by hand on `examples/wasm` and measured. **Neither
helps on x86-64.** Pre-scaling changes nothing, and offset threading is
1% slower. The stop rule (< 2% on PolyBench) applies to both, so step 3
(an estimate for other interpreters) was not run.

**Setup.**
- **Machine.** i7-6700 (Skylake), LLVM 19, tail-call dispatch,
  `--align-functions=64`.
- **Timing.** user+sys CPU, the minimum of interleaved runs: 7 rounds of
  the tuning set (the seven programs of `programs.sh`) and 11 of
  PolyBench (30 kernels). The order of the binaries rotates each round.
- **Control.** The A/A binary is a byte-identical copy of the baseline,
  run in the same interleave.
- **Load.** The load average was 2.4–4.3 on 8 cores, from two other
  benchmark jobs.
- **Correctness.** Each variant passed `polybench.sh --check` (30/30),
  the full spec suite (52,800 commands, 0 failed) and the ten programs'
  expected output.

**Results** (geometric means, ratio to the baseline):

| Build | Tuning set (7) | PolyBench (30) |
|---|---:|---:|
| A/A control | 0.997 | 1.000 |
| Step 1: pre-scaled slot fields | 0.998 | 1.003 |
| A/A control (second run) | 0.998 | 0.999 |
| Step 2: offset-threaded dispatch | 1.004 | 1.008 |
| A/A control, `--align-functions=128` | 1.001 | 1.000 |
| Step 2, `--align-functions=128` | 1.010 | 1.010 |

- **Noise.** Single benchmarks move ±1–2% in the A/A runs.
- **Step 1.** The per-program swings (binarytrees 0.94, fannkuchredux
  1.03) are placement effects, and they cancel in the means.
- **Step 2.** It is slower on 22 of the 30 kernels at both alignments.

**Step 1: pre-scaled slot fields.**

What was built:
- **Operand kinds.** The 16 slot kinds of `vm.lm` became byte offsets:
  `*(((regs as *u8) + it) as *f64)`, verified by `(it & 7) == 0` and
  `it < nslots * 8`.
- **Encoder.** A pass after `W.accumulate` in `compile.lm` multiplies
  every slot-kind field by 8. It finds the fields through
  `W.operand_kind`/`operand_bits`.
- **Limits.** `bulk` gets slot indices back with `>> 3`. The R4
  tee-fusions' 12-bit fields drop to 512 slots.

The asm is the evidence:

- **Totals.** 19,022 instructions over 1,094 handlers become 19,054.
  30 handlers lost one, 39 gained one, and `I8X16_SHUFFLE` gained 23.
  Every hot handler is unchanged: `F64_MUL` 14, `F64_ADD` 14,
  `F64_LOAD__AR` 15, `F64_LOAD` 18, `I32_ADDK` 13, `MOV` 11,
  `F64_LOAD_F64_MUL` 28.
- **Why: the scale is free on x86-64.** It is part of the addressing
  mode, so `(%r14,%rax,8)` becomes `(%r14,%rax,1)` at the same cost.
- **The decode does not shrink.** The "6 decode" instructions of
  `F64_MUL` are the field extraction:
  - field `a` (bits 16–31): `shr $16, %ecx`, a 32-bit shift that also
    clears the bits above;
  - field `b`: `mov; shr $32; movzwl`;
  - field `c` (the top 16 bits): `mov; shr $48`.

  A byte offset still has to be shifted out of the word and
  zero-extended, so it needs the same instructions.

wasm3 saves these instructions because its slot offset is a whole code
word, not because the offset is scaled. AArch64 also scales for free
(`ldr d0, [x, x, lsl #3]`). Some older cores charge a cycle for a
scaled register offset, but that was not measured.

**Step 2: offset-threaded dispatch.**

What was built:
- **The IR patch.** On the `--keep-ll` IR of the tail-call build, every
  `getelementptr @"vm.W.table", op` followed by `load ptr` (1,218 sites)
  became `getelementptr i64, @"vm.W.op.MOV", op`. The target is then
  `base + 8 * op`, and `MOV` is the lowest handler.
- **Rewriting the code.** `rt.lm` runs a pass after `W.optimize`, and
  over the HALT and RESTORE stubs. It rewrites each instruction word's
  16-bit opcode, and each super part's, to
  `(table[op] - base) / 8`. The pass is `loimi_proto_xlate`, defined in
  the patch, and it aborts if a handler is out of reach.
- **Reach.** The handlers span 0x403000–0x45ad80 (360 KiB), and the
  reach is 512 KiB.

The asm (`F64_MUL`, and every other handler):

```
before:  mov 0x8(%r13),%rcx; add $0x8,%r13; movzwl %cx,%eax; mov 0x47cd50(,%rax,8),%rax; jmp *%rax
after:   mov 0x8(%r13),%rcx; add $0x8,%r13; movzwl %cx,%eax; lea 0x403000(,%rax,8),%rax;  jmp *%rax
```

The instruction count is the same (19,022 in both builds, every handler
the same). The only change is that a load µop (ports 2/3) became an ALU
µop (ports 1/5 for `lea`).

**Why it gives nothing.**
- **The table load is off the critical path.** The indirect jump is
  predicted, so the next handler starts before the load resolves, and
  the load only confirms the prediction. Removing it does not shorten
  the path.
- **It costs an ALU slot.** These handlers are ALU-heavy (the shifts
  and moves of the decode), and the 2 load ports are not saturated
  (about 4 loads per 4–5 cycles). The extra ALU µop is a slight loss.
- **The earlier trade went the same way.** Narrow field loads (more
  loads, fewer ALU ops) measured 1.044, above. Both directions are worse
  than the current mix.

What a load-free and extraction-free dispatch would need is wasm3's
layout: the handler address as a whole code word, with operands in
their own words. That doubles the code words and moves a load from the
table to the stream, and it is not a field-format change. Not tried.
After this result it is not ranked either: the predicted indirect jump
is what makes our dispatch cheap, and wasm3's per-op advantage was
traced to its `fp0`/`r0` registers (now accumulators), not to its
dispatch (wasm FRICTION, "Float benchmarks against wasm3").

**What the JIT removes, then.** The JIT's 0.76–0.83 does not come from
the table load. It comes from the indirect jump itself (straight-line
stencils), the `pc` advance and the field extraction (operands are
patched in as constants). None of these is reachable by a change to the
interpreter's word format.

**Design sketch, kept for the record.** It is not recommended on this
evidence; revisit only on a core where the table load is measured to
matter (for example AArch64 hardware), with the same prototype.
- **Surface.** `#[dispatch(tailcall, threaded)]`, for `u64` encodings
  with an op field of at least 16 bits.
- **Encoding.** A final pass over verified, optimized code
  (`Verified<Vm>` keeps it a proof) maps each word's opcode to
  `(handler - base) >> k`, with `base` a symbol emitted first in
  `.text.hot.<vm>`. `k = 3` reaches 512 KiB. `k = 6` reaches 4 MiB but
  costs a `shl`, one instruction more than today.
- **Readers of opcodes.** `Vm.opcode`, `op_size`, `op_name`, the
  disassembler and `Vm.deopt` would decode through an inverse map
  (offset to opcode, a sorted array or a 64 Ki table). The verifier
  runs before the pass, so it is unchanged.
- **`become`.** It would write the handler offset, through a generated
  `Vm.OP_X` offset table.
- **Serialization.** Code words would need a relocation pass back to
  opcodes.
- **`#[profile]` ordering.** Every handler, hot or cold, must stay
  inside the window from `base`. A link-time check is needed.
- **Instrumentation variants.** These need handler blocks at identical
  offsets from a per-variant base, or table dispatch in variant builds.
- **`--jit-stencils`.** Its enter words, and the rewrite back to real
  words, would be offsets as well.
- **Other modes.** goto and switch modes are out of scope. So are u32
  vms: Mini, MiniJS, Lox, Python, Scheme, Prolog, regex and Stak all
  have 8-bit op fields, which have no room for an offset.
- **Cost.** L. **Measured gain on x86-64:** −1%.

**Prototype code.** It is not committed. It is a hand patch plus an IR
rewrite outside the compiler, so there is nothing clean to put behind a
flag.

### 3. Division by a verified constant

**Evidence.**
- **Prolog.** crypt spends 27% of its samples in MOD and IDIV. Its
  FRICTION (532–539) says the divisor could be multiplied away "if an
  op knew its divisor at verification time". The u32 division already
  gave MOD 21% → 14%.
- **MiniJS.** `MODK` by a non-power-of-two constant is an `idiv`, 8% of
  closures (`RESULTS.md:284-288`).
- **Mini** wrote Granlund and Montgomery by hand: `mod_magic` and a
  `ModIC` cache filled on first execution (`examples/mini/vm.lm:291`,
  `:833`). A version that recomputed the magic on each miss made the
  coroutine sieve 12% slower (item 31).
- **Python** special-cased slice steps ±1 to avoid an `idiv` (14% of
  fannkuch).
- **Skylake** takes 40–90 cycles for a 64-bit `idiv`, against about 4
  for a multiply and shift.

**Proposal.**
- **`std.math.Divisor<T>`**: `Divisor.new(d)` precomputes, then
  `x.div(dv)` and `x.rem(dv)`, for signed and unsigned 32- and 64-bit
  values, as libdivide does.
- **Filling it once.** A `cache` field may be filled by the emitter or
  at verification, `cache DivK { const dv: Divisor<i32> }` (the
  "immutable cache fields" option of `roadmap.md` Q10). So it is
  computed once per site, not on first execution.

<!-- snippet: skip design, not built -->
```loimi
// before (Mini, by hand: 25 lines, mod_magic, MAGIC_M, a first-run fill)
let q = ((x as u64) * (mg & MAGIC_M)) >> (mg >> 56);
// after
cache ModK { const dv: math.Divisor<i32> }      // filled by the emitter from k
op MODK(dst: reg, a: reg, b: kst) : ABC + ModK {
    if let Int(x) = a { dst = Int(x.rem_floor(cache.dv)); next }
    cold { .. }
}
```

**Measure.** Prolog crypt, MiniJS closures, Mini's coroutine sieve.

**Prototype.** Copy Mini's `mod_magic` into Prolog's MOD/IDIV with a
divisor operand that is a constant, then time crypt. That takes an
hour.

**Cost: S** for the std type, plus S–M for `const` cache fields, which
item 12 also wants. Risk: signed floor semantics differ per language
(Lua `%`, JS `%`, Prolog `mod` and `rem`), so the type needs both
truncating and flooring forms.

**Interactions.**
- **JIT.** With `const` fields the JIT may patch the magic in as an
  immediate.
- **`Vm.deopt`** must not zero `const` fields.
- None with the GC, `redo` or supers.

**General:** four interpreters, a few hot benchmarks each.

**Prototype results (2026-11-06).** Mini's magic-number method copied
by hand into Prolog: `MODK`/`IDIVK`/`REMK` ops with the magic in a cache
the clause compiler filled, for constants 1 <= k < 2^31. crypt took
0.846 of its time (min of 11 interleaved runs, user+sys,
`--align-functions=64`; A/A 1.000; 0.876 at `--align-functions=128`).
The other Prolog programs have no constant divisor: they moved within
placement noise (query 1.05 at 64-byte alignment, 1.004 at 128).
Results matched SWI-Prolog on 98,016 lines of `//`, `mod`, `rem` and
`div` over 65 constants, negative constants, zero, the i48 limits and
bignums. That is above the 5% stop line, so the feature was built.

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
- **Prolog.** `IDIVK`, `MODK`, `DIVK` and `REMK` (`div` is new), with
  the cache filled by the emitter.
- **Mini.** `MODK` and `MOD` fill the cache on first run, in a `cold`
  block (`mod_magic` is gone).
- **MiniJS.** `MODK`'s cache is filled by the emitter.
- **Python.** Not ported: its slice division is by a run-time step.

**Timing.** Ratios are new/old over whole suites, at the minimum of 5
interleaved runs. The load average was 2.9–3.0 (two other agents were
running).
- **Prolog:** crypt 0.855, geomean 0.9935 (A/A 0.9994).
- **Mini:** geomean 1.0002 (A/A 1.0013), closures 0.995 and the
  coroutine sieve 0.976. The old code was already a magic number, so
  nothing was expected beyond parity. A first version used the 128-bit
  product and filled the cache inline. That gave `MODK` a stack frame
  and made closures 1.022; the `cold` fill and `urem31` brought it to
  0.998 (min of 11).
- **MiniJS:** closures 0.970, geomean 0.9944 (A/A 0.9979). The 8%
  `idiv` share was a 32-bit `idiv` on non-negative Ints, so the gain is
  smaller than the share.

`loimi ir-diff` on Mini: 621 of 627 functions are unchanged. The
changes are `MOD`, `MODK`, their cold blocks (plus one new cold block
each, for the first-run fill) and the op-size tables (the caches grew
from 2 and 3 words to 4).

**Not built:** `const` cache fields and a verify-time fill. No
interpreter deoptimizes these ops, so an emitter-filled ordinary cache
was enough. Until `const` fields exist, `Vm.deopt` would zero the
cache.

### 4. Load facts: dereferenceable, nonnull, invariant

**Evidence.**
- **Mini's "Remaining compiler and codegen issues".** "No way to say
  'this pointer is dereferenceable', so loads cannot be hoisted or
  speculated" (`bench/mini/RESULTS.md:579`).
- **Dependent loads dominate the object ops.**
  - Python's GETATTR hit is four dependent loads, 14–26% of deltablue,
    go and richards (FRICTION 165, 388).
  - MiniJS's GETPROP_IN is a dependent load of the shape id.
  - Mini's GETINDEX_GETINDEX is 20% of interp and keeps a stack frame
    from register pressure.

  A load that LLVM may move above a type test starts its cache miss
  earlier. A load it knows is invariant (`!invariant.load`: a proto's
  `k`, `code`, `nregs`; a shape's id) may be CSE'd across calls and
  stores, which frees registers.
- **`const` operand kinds** (`language.md` §3.2) exist, but feed only
  the JIT.

**Proposal.**
- **Pointer facts.** `#[deref(N)]` and `#[nonnull]` on struct fields
  and parameters of pointer type, and on `Gc<T>` by default (a live
  `Gc` is dereferenceable for its type's size).
- **Invariant fields.** `const` struct fields, initialized once, read
  with `!invariant.load`.
- **Interpreter loads.** `const` operand kinds get `!invariant.load` in
  the interpreter too.

<!-- snippet: skip design, not built -->
```loimi
struct Proto { const code: *u32, const k: *V, const nregs: u32, .. }
struct Shape { const id: u32, .. }
// Gc<T> is dereferenceable(sizeof T) and nonnull: LLVM may load o.shape.id
// before the tag test that proves `o` is an object
```

**Measure.** Python deltablue, go and richards; MiniJS property
benchmarks; Mini interp.

**Prototype.** Take `--keep-ll` of Python's tail-call build. Add
`dereferenceable(16)` to the `Gc` loads in `GETATTR_*`, and
`!invariant.load` to the shape and proto loads, then rebuild from the
`.ll` and time it. A day.

**Cost: S**: metadata on loads the compiler already emits. Risks:
- **Soundness.** `dereferenceable` on a `Gc` that may be the nil
  encoding is wrong, so it applies only after a successful
  `let Obj(o)` match.
- **`const` and the collector.** The GC must not write `const` fields;
  it does not move objects, so it does not.

**Interactions.**
- **Lifetimes.** `const` fields are a new mutation rule, a simple one.
- **Write barrier.** `const` fields of reference type need one only at
  initialization.
- **Debugger.** It may not assign `const` fields.
- **Everything else.** None.

**General:** every interpreter, but the size of the gain is unknown.
That is why it ranks below items 1–3 despite its cost.

#### Prototype results (2026-11-06): no gain, no-go

**What was built.** Rather than editing the `.ll` by hand, a throwaway
switch in codegen (`LOIMI_PROTO_FACTS`, not kept) put the facts where
the compiler already knows the types, on every handler at once:
- **`gc`.** Every load of a `Gc<T>` struct field got `!nonnull` and
  `!dereferenceable(size_of T)`. Every `Gc` payload extracted after a
  successful match (`values::payload`, nanbox) got
  `llvm.assume(true) ["dereferenceable"(p, size_of T), "nonnull"(p)]`.
  The pointer is the one the match produced, so a nil encoding never
  gets the fact.
- **`inv=S.f,..`.** Loads of the named fields got `!invariant.load`:
  - Python: `Box.s`, `Box.kind`, `Inst.cls`, `Func.proto`,
    `Func.globals`, `Proto.nregs`, `Proto.simple`;
  - MiniJS: `Shape.id` (std.obj), `Fn.proto`, `Proto.nregs`, `fixed`,
    `nparams`, `flags`, `nfree`;
  - Mini: `Closure.proto`, `Proto.nregs`, `consts`, `nparams`,
    `protos`, `nupvals`.

The IR has them where intended. Python got 681 payload assumes, 251
field loads with `nonnull`/`dereferenceable` and 582 invariant loads;
MiniJS 399, 963 and 925; Mini 398, 321 and 677. MiniJS's GETPROP_IN,
for one, is `assume(deref 32) %o` → `load ptr %o.shape, !nonnull,
!dereferenceable 56` → `load i32 %shape.id, !invariant.load`.

**The machine code barely changes.** Comparing the disassembly
function by function (addresses and symbol names stripped):
- **Mini.** 0 of 154 handlers differ. Only 14 cold blocks of the call
  paths change, by 14–26 bytes.
- **MiniJS.** 0 of 165 handlers differ. One cold block changes (KEYS,
  −50 bytes).
- **Python.** 3 of the 11 attribute, global and call handlers differ
  (LOADMETHOD, LOADMETHOD_CALLMETHOD, CALLMETHODKW): a kind test folded
  away and blocks reordered (−32 bytes in LOADMETHOD). GETATTR,
  GETATTR_GETATTR, SETATTR and GETGLOBAL_C are identical.

**Why nothing changes.** A tail-call handler is one straight-line path
from the dispatch to the next dispatch:
- **Nothing to speculate above.** The loads of the chain `o → o.shape
  → shape.id` (or `o.keys → keys.s → keys.extra[i]`) depend on each
  other's results. The first one depends on the tag test, through the
  payload. `dereferenceable` lets LLVM hoist a load above a branch, but
  there is no earlier point that already has the pointer.
- **Nothing to reuse.** `!invariant.load` lets a load be reused across
  stores and calls. A handler loads each field once, so there is
  nothing to CSE; the next handler is another function.
- **Register pressure stays.** Mini's GETINDEX_GETINDEX frame comes
  from what is live across its two parts, not from reloads.

**Timing**, as an upper bound (user+sys CPU; interleaved;
`--align-functions=64`; A/A = a copy of the baseline in the same
interleave; load average 4–5 from other agents):

| Suite (benchmarks, runs) | `gc` | `gc` + `inv` | A/A |
|---|---:|---:|---:|
| Python: deltablue, go, richards (11) | 1.004 / 0.998 | 1.007 / 0.993 | 1.003 / 0.994 |
| MiniJS: properties, deltablue, richards, raytrace (7) | — | 1.003 / 0.999 | 0.999 / 0.997 |
| Mini: interp (9) | — | 1.007 / 1.002 | 1.005 / 0.997 |

(min / median, geometric means of the ratio to the baseline.) Every
column is inside the A/A range.

**Decision: not built.** `const` fields and pointer facts on `Gc`
would be sound (the prototype put them only after a match) and cheap,
but they buy nothing on these interpreters' handlers. They would matter
for code with loops over objects or calls between reuses: library code
in `std`, a trace, or an optimizing tier (item 14). That is where to
look again, not at the handlers.

### 5. Smaller objects: headerless typed chunks

**Evidence.**
- **Every object has a 16-byte header**,
  `{ size: u32, tid: u16, mark: u8, flags: u8, len: u64 }`
  (`stdlib/gc.md`), even when `len` is meaningless. The 16-byte
  granule then rounds a pair up to 32 bytes.
- **Scheme.** Its pairs are 32 bytes against Guile's 16. nboyer,
  sboyer and mperm are "memory-bound list walking", and CAR alone is
  12–15% of nboyer (`bench/scheme/README.md` 310).
- **Size moves time.** Measured:
  - MiniJS's `Obj.sid` (8 bytes more per object): binary_trees 1.06;
  - Python's cached entries pointer in `Inst` (24 bytes against 16):
    float 1.06;
  - the dropped single bump run (objects scattered across lines): sort
    4% slower.
- **Collector share.** Marking is 8–15% on large live heaps (Scheme
  phase 4). Prolog's collector is 2–21%, and boyer's 21% (FRICTION
  447).
- **Mark bitmaps already exist** (one bit per granule), so the
  per-object mark byte is redundant except as the "already marked"
  test.

**Proposal.** Big-bag-of-pages chunks for fixed-size types without
trailing arrays, opted in per type (`#[compact] struct Pair { .. }`):
- the chunk holds one type, and its `ChunkHead` holds the type id and
  the size;
- objects have no header; the bitmap is the mark;
- the granule is 8 bytes in such chunks, so a pair of two `V` words is
  16 bytes.

`tid` and `size` come from the chunk (`obj & ~(CHUNK-1)`), and the
barrier's `LOG` flag moves to a per-chunk side table, or to the bitmap
of a second plane. A lighter alternative is an 8-byte header without
`len` for types without trailing arrays: pairs would be 24 bytes, and
most small objects 8 bytes smaller.

<!-- snippet: skip design, not built -->
```loimi
// before: 16-byte header + 16 bytes = 32 per pair
struct Pair { car: V, cdr: V }
// after: 16 bytes per pair, header in the chunk
#[compact] struct Pair { car: V, cdr: V }
```

**Measure.**
- Scheme's 43 programs (nboyer, sboyer, mperm, gcbench, paraffins,
  earley).
- Prolog's 30, where terms and variables are small fixed objects.
- MiniJS binary_trees, Python float and raytrace.
- RSS as well as time.

**Prototype.** Allocate Scheme's pairs from a plain `mem.alloc` arena
of 16-byte cells, never freed, and run nboyer, sboyer, mperm and
earley with a heap budget large enough that no collection runs. Then
run the same with the GC-allocated 32-byte pairs and the same budget.
The difference bounds the layout gain, without touching the collector.
Two days.

**Cost: L.** The allocator, marking (no header to test),
`gc.is_young`/`LOG` for the barrier, the verifier mode, weak and
ephemeron handling, `#[trace]` derivation and the debugger's object
printer all read the header today. Risks:
- **Write barrier.** It tests the `LOG` flag with one `testb` at
  −9 bytes. Moving the flag to chunk metadata adds a mask and a load
  to every barrier (+0.4–0.6% today, `lowering.md` §10). It needs
  measuring.
- **Generational mode.** Its young marks live in the header's mark
  byte.

**Interactions.**
- **GC and polls:** the core change.
- **Lifetimes:** none.
- **JIT:** none, since stencils call the same inline paths.
- **Debugger:** its object printer reads `tid`, which it now gets from
  the chunk.

**General:** six interpreters (Scheme, Prolog, MiniJS, Python, Lox,
Mini closures and tables), mostly in allocation-heavy benchmarks.

#### Upper bound measured (2026-11-06): 1–3%, no-go

**Result.** On nboyer, sboyer, mperm and earley, headerless 16-byte
pairs are 1–3% faster in the mutator: 0.971 at
`--align-functions=64`, 0.993 at 128. That is under the 5% stop line.
24-byte pairs (the 8-byte header) gain nothing measurable, and
Prolog's terms and variables gain nothing. What 16-byte objects do
buy is memory: half the RSS on the four.

**The hack.** The patch is
[`speed-audit/item5-arena-pairs.patch`](speed-audit/item5-arena-pairs.patch);
it is not applied.
- **Scheme.** `V.P(Gc<Pair>)` is told apart by its NaN-box tag, so
  nothing reads a pair's header. Every pair allocation (CONS, REVERSE,
  APPEND2, `cons`, `list`, `append`, the reader) goes through
  `mk_pair`, which bumps a pointer through 64 KiB blocks of 2 MiB
  huge-page segments. That is the GC's own segment source.
- **Per block.** Each block starts with a zero "chunk head". Stores
  into pairs test that head's flag byte (a mask and a load), as the
  proposal's per-chunk `LOG` flag would.
- **Variants.** `CELL` = 16, 24 or 32 bytes. The 24-byte cells write
  an 8-byte header word. The 32-byte cells write the full header, so
  they control for the allocation path.
- **No collection.** Every build runs with the budget raised
  (`set_max_budget(0)`, growth 400% over a 1 TiB minimum). The stock
  build is included; `LOIMI_GC_STATS` showed 0 collections in every
  run.
- **Prolog.** `Var` and `Cons` (16-byte payloads, 32-byte objects)
  moved to the same arena, with iterations at a quarter of
  `iterations.txt` so that RSS stayed under 640 MB.

**Method.** User+sys CPU time and max RSS (`wait4`), minimum of 7–11
interleaved runs, `--align-functions=64` with a recheck at 128, an A/A
build, and inputs as `bench/scheme/compare.py`'s (mperm 8, earley 20,
gcbench 20, paraffins 100, browse 20, so that runs are long enough).

**Total CPU overstates the gain.** With nothing collected, every
allocated byte is a fresh page that the kernel faults in and zeroes,
so halving a pair halves that system time. Those numbers (first row
below) are an artifact of the no-collection setup: the stock build
*with* collection runs deriv in 0.140 s, against 0.18–0.21 s without.

The fair bound prefaults a pool sized to stock RSS plus 96 MB, so that
every variant has the same RSS and no faults in the timed loop. It then
times the benchmark's own elapsed time (rows marked "prefaulted").

| Geometric means against stock | four named: 16 / 24 / 32 | all 21 Scheme: 16 / 24 / 32 | A/A (four / all) |
|---|---|---|---|
| align 64, total CPU | 0.913 / 0.959 / 0.970 | 0.939 / 0.968 / 0.996 | 0.975 / 0.992 |
| align 128, total CPU | 0.966 / 0.988 / 1.017 | 0.958 / 0.979 / 1.013 | 1.000 / 0.999 |
| align 64, prefaulted | **0.971** / 0.996 / 0.999 | 0.983 / 0.997 / 1.010 | 0.997 / 1.001 |
| align 128, prefaulted | **0.993** / 1.004 / 1.014 | 0.988 / 0.993 / 1.013 | 0.996 / 1.000 |
| max RSS, no prefault | 0.50 / 0.73 / 0.97 | 0.63 / 0.82 / 0.99 | |

**Per benchmark.** Prefaulted, 16-byte cells, at align 64 / 128:

| benchmark | 64 | 128 |
|---|---|---|
| nboyer | 0.966 | 0.984 |
| sboyer | 0.968 | 0.999 |
| mperm | 0.976 | 0.975 |
| earley | 0.975 | 1.013 |
| divrec (the most) | 0.891 | 0.900 |
| paraffins | 0.947 | 0.943 |

16 against 32 bytes (the same allocation path, only the size
differs) gives the same bound: 0.972 at align 64, 0.979 at 128. A
same-binary A/A spreads ±1–3% per benchmark.

**Prolog.** At align 64 (30 programs):
- Total CPU: 0.962 / 0.982 / 1.012 for 16 / 24 / 32-byte cells, A/A
  0.995, RSS 0.63 / 0.81 / 0.98.
- Prefaulted (CPU minus a null run with the same pool): 1.006 / 1.005
  / 1.022, A/A 1.002. The layout gains nothing.

**Not measured: the collector.** With the stock budget, marking and
sweeping are a large share of the CPU time:

| benchmark | share | collections |
|---|---|---|
| nboyer | 16% | 14 |
| mperm | 23% | 6 |
| earley | 14% | 18 |
| paraffins | 24% | |
| sboyer | 4% | |

A chunk design might mark faster: no header to read, and a bitmap
that is the mark. Collections would not get much rarer, though: the
budget is a multiple of the live bytes, which shrinks along with the
pairs.

**Recommendation: no-go.** The 3–10% in the table was not real for
time: the layout is worth 1–3% on the benchmarks it was aimed at, and
not the L-sized change to the allocator, barrier, generational mode,
weak tables, verifier and debugger. Reopen it only for memory (half
the RSS on list-heavy programs), or if marking cost becomes the
target. Measure that first, as GC time saved on nboyer, mperm and
earley with collection on.

### 6. A sampling profiler in std

**Evidence.**
- **Every round wrote its own sampler.** `perf` is unavailable
  (`perf_event_paranoid=3`), so each round wrote or copied an
  LD_PRELOAD SIGPROF sampler:
  - wasm FRICTION 366–398 (it records `rip` and `r13`, the pc);
  - Python FRICTION 618–630;
  - Prolog FRICTION 453–465, where the verifier's 64 KiB memset was
    "only visible to a sampler";
  - Mini RESULTS:9;
  - the JIT's `LOIMI_JIT_MAP`.
- **What `--vm-stats` misses.** It counts dispatches, not time. Several
  rounds found that dispatch counts mislead: Python's suggested supers
  gave 0.996, and the cost of wasm's ops was per dispatch, not in the
  dispatch count.
- **`status.md`** lists a sampling profiler as not built.

**Proposal.** `loimi build --vm-prof`: a SIGPROF sampler linked into
the program. At exit it prints time per handler, per cold block (the
same names `--vm-stats` uses), per runtime function, and per guest pc
(from the vm's `pc` state register, which it knows), plus the JIT map
when the JIT is on.

<!-- snippet: skip design, not built -->
```loimi
// no source change: loimi build --release --vm-prof main.lm; ./main x.lua 2> prof.txt
// optional: a guest-level report through a vm hook
fn prof_location(code: *u32, pc: *u32) -> str { .. }   // Mini: "fib.lua:12"
```

**Measure.** It does not speed anything up by itself. Its worth is in
the rounds it makes shorter: compare a round's findings against the
hand sampler's.

**Prototype.** Generalize
`scratchpad/js-perf-q7k2/prof/sampler3.c` and `prof3.py`, which already
map samples to symbols, into a runtime C file linked under a flag, as
`stats_rt.c` is.

**Cost: S–M.** Risks:
- the signal handler must read the pc register, so it needs per-arch
  ucontext code (x86-64 and AArch64);
- under `--cfg gil`, threads need per-thread timers.

**Interactions.**
- **Dispatch modes.** In goto and switch modes handlers share one
  function, so the profiler attributes by address ranges from the
  handler labels.
- **Everything else.** None; it is an opt-in build like `--vm-stats`.

**General:** all interpreters, and every future tuning round.

#### As built (2026-11-06): `loimi build --vm-prof`

The flag links `src/prof_rt.c` into the program, as `--vm-stats`
links `stats_rt.c`; its use is described in
[language.md §7.3](../language.md#sampling---vm-prof).

**Sampling.** A `SIGPROF` timer runs at 1000 Hz (`LOIMI_PROF_HZ`). It is
a wall-clock high-resolution timer, because a CPU-time timer fires at
most once per kernel tick, which is 250 Hz here. Each sample records
the interrupted instruction from the `ucontext` (x86-64 and AArch64),
the vm's `pc` and running op, and up to 24 return addresses
(`backtrace`). The build adds `uwtable` to every function, so the stack
walk goes from a runtime function back to the handler that called it.

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

**Validation against the hand sampler.** The comparison ran
`sampler3.c`/`prof3.py` and `--vm-prof` on the same binaries, at
27 kHz and 10 kHz:

| Program | Top 5 | Largest difference |
|---|---|---|
| Mini `binary_trees` | same five, in the same order | 0.6 points |
| Python `richards` | same entries | 1 point |
| Python `raytrace` | same entries | 0.8 points |

The only other mismatch is a name: the hand sampler's `CALL.cold31` is
`CALL (cold block at vm.lm:795:13)` here. Goto- and switch-mode
profiles of Mini's `fannkuch` and `binary_trees` match the tail-call
profile, which the hand sampler cannot split, because goto and switch
modes are one function.

**Overhead.** User+sys time, minimum of 5 runs, each program under 4
environment sizes (stack placements), `--align-functions=64`:

| Build | Instrumentation only (`LOIMI_PROF=0`) | Sampling at 1000 Hz |
|---|---|---|
| Mini, tail-call (8 programs) | 1.017 | 1.032 |
| Python, tail-call (6 programs) | 1.016 | 1.031 |
| Mini, goto (6 programs) | 1.078 | 1.086 |

- **Instrumentation.** Removing one part at a time shows that the
  tail-call cost is all the `pc` store. Marks, unwind tables and the
  report at exit cost nothing measurable.
- **Mini's `sort`.** Its 12% is where the thread-local slot falls:
  moving the slot within a padded block gives 1.00 to 1.05. It is an
  address effect, not work.
- **Goto mode** stores two words per op (`pc` and the op) inside one
  large function, and that costs 2–17% (`nbody` most).
- **Sampling.** Faster sampling costs more: 10 kHz cost 3–9% on Mini,
  and that cost is the signal, not the stack walk.

**Not built:**
- the guest-level hook (`prof_location`), because the report runs in a
  destructor, after the interpreter may have freed its code;
- folded stacks;
- per-thread timers (with threads, a sample goes to the running
  thread).

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

### 8. Re-fusion on `become`; supers from a profile file

**Evidence.**
- **Re-fusion.** Supers whose parts are quickened members form only
  if `Vm.optimize` runs again after `become`. Mini and MiniJS do it by
  hand: `refuse(cl)` re-optimizes the whole function
  (`examples/mini/vm.lm:320`, `examples/minijs/vm.lm:63`). In Mini
  that gave mandelbrot −6% and nbody −5% (item 25). Lox (37 `become`),
  Python (8) and Scheme (6) don't.
- **Super selection is by hand.** About 140 supers across the
  examples, picked by hand from `--vm-stats` suggestions. Many tried
  sets measured as noise: MiniJS 0.996, Python 0.996, Scheme 0.993,
  Mini item 9. Wasm found that "supers that pay are per benchmark".

**Proposal.**
- **(a) Local re-fusion.** `become` looks at its neighbours' words and
  forms or splits the super for the new pair. The word and its
  neighbour rewrite in one place, with the same rules as
  `Vm.optimize` (`language.md` §7.3). This replaces whole-function
  `refuse`.
- **(b) Supers from a profile.** `supers from "vm.vmprofile";` takes
  the first list of the suggestions that `--vm-stats` prints, and
  `#[profile]` already reads that file format.

<!-- snippet: skip design, not built -->
```loimi
// before (Mini, MiniJS): after every quickening
become ADD_F; refuse(fp.cl); next
// after: the compiler re-fuses the two neighbouring words itself
become ADD_F; next
```

**Measure.** Lox, Python and Scheme, which do not re-fuse today, on
their numeric benchmarks.

**Prototype.** Add the `refuse` call that Mini has to Lox's and
Python's arithmetic families, then time them. An hour each.

**Cost: S.** Risks:
- a `become` inside a super must split it, which `Vm.deopt` already
  knows how to do;
- under `#[threads(shared_code)]` a two-word rewrite is not atomic.

**Interactions.**
- **`redo`.** A `become; redo` that re-fuses must redo the new super,
  or the part alone (today's rule).
- **JIT.** A rewritten word already invalidates compiled code
  (`copy-and-patch.md` §10).
- **`#[forward]` variants.** `become` picks the variant whose fields
  match.

**General:** five interpreters with families. The speed is in the
three that do not re-fuse today; (b) is ergonomic.

#### Prototype results (2026-11-06)

**The premise was wrong.** Lox, Python and Scheme have no arithmetic
families: their ADD and MUL test the types every time and never
`become` anything. What they quicken is globals, properties and calls
(Lox: GETGLOBAL, SETGLOBAL, GETPROP, SETPROP, INVOKE; Python: GETGLOBAL,
CALL; Scheme: GETGLOBAL, CHKGLOBAL). On their numeric benchmarks
(`--vm-stats`) quickened ops are 0% of dispatches (Python nbody,
fannkuch; Scheme fib, fibfp, sumfp, mbrot, ack, tak), so re-fusion has
nothing to act on there.

**Where it could act.** A `refuse` call adds nothing unless some super
names quickened members. The existing rules already cover a pair with
one quickened member: declare the generic pair and the quickened one,
and the `become` in the part re-targets the head. What only re-fusion
forms is a pair of two quickened members, since the second site to
quicken cannot see the word before it. From `--vm-stats` of every
benchmark of the three suites, the hot pairs of that kind are:

| Suite | Pair | Share of dispatches |
|---|---|---|
| Lox | `GETGLOBAL_C; INVOKE_P` | invocation, properties 31%; zoo 21% |
| Lox | `GETPROP_IN; INVOKE_POLY`, `GETPROP_POLY; JNEQK` | trees 15% |
| Lox | `SETGLOBAL_C; GETGLOBAL_C`, `SETPROP_ADD; SETPROP_ADD` | method_call 7%; binary_trees 5% |
| Python | `GETGLOBAL_C; GETATTR`, `GETATTR; GETGLOBAL_C` | deltablue 6–7%, go 3% |
| Scheme | `CHKGLOBAL_B; GETGLOBAL_B`, `GETGLOBAL_B; GETGLOBAL_B` | conform 8%, triangl 10% |
| Scheme | `GETGLOBAL_B; VREF`, `GETGLOBAL_B; PRIMCALL` | puzzle 14%, string 12% |

**What was measured.** Mini's pattern (`become X; refuse(cl); ..`,
`refuse` = `Vm.optimize` over the function) after every `become` in the
three suites, plus those pairs as supers: six in Lox, five in Python
(with the single-quickened pairs above), five in Scheme. Same machine
and method as item 2: user+sys CPU, interleaved runs, `--align-functions=64`,
an A/A copy of the baseline in the same interleave. The load average
was 4–5 on 8 cores, from two other agents' builds and benchmarks, so
the A/A column moved up to ±8% on single short benchmarks (medians);
the minimums are steadier.

| Suite (benchmarks, runs) | refuse + supers, min / median | A/A, min / median |
|---|---:|---:|
| Lox (9, 7) | 0.981 / 0.976 | 0.995 / 0.997 |
| Lox (9, 11), second run | 0.978 / 0.977 | 1.013 / 0.997 |
| Python (11, 7) | 0.998 / 0.994 | 1.007 / 0.990 |
| Scheme (11 of them, 7) | 0.999 / 1.002 | 0.997 / 1.005 |

- **Lox pays**, all of it on the benchmarks above: zoo **0.883–0.886**
  in both runs (min and median), invocation 0.94–0.97, instantiation
  0.94–0.99, properties 0.94–0.99; fib, equality and binary_trees
  stay within the A/A range.
- **Python and Scheme do not.** deltablue 0.97 (min) / 1.00 (median);
  conform 0.93 / 0.94 against an A/A of 0.99 / 0.94; puzzle, triangl,
  string within noise. These were reverted.

**Decision:** build (a) for Lox; keep Python and Scheme as they are.

#### As built (2026-11-06)

**(a) `refuse CODE;`** (`language.md` §7.3, "Fusing again after
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

- **Why not local.** The proposal asked for a local rewrite of the
  word and its neighbours. Forward is easy: the next instruction is at
  `pc + size(X)`. Backward is not: with inline caches and ops of several
  sizes, the word before `pc` may be a cache word or an operand word,
  and rewriting it as an opcode would corrupt the cache. Only a walk
  from the start of the code finds the previous instruction, and once
  the code is walked, running `optimize`'s pass over it costs about
  the same and gives exactly `optimize`'s result (the same rules, by
  construction). The pairs that matter (two quickened members) are
  formed by the *second* `become`, which is the backward case.
- **`redo`.** `become X; redo` dispatches the word as it is after the
  pass: the super `X` heads if one formed (`tests/run/vm_refuse.lm`,
  `NEG_F_RET`), else `X` alone. A `become` inside a super first
  re-targets or splits the head as before; the pass then sees the
  result.
- **JIT and traces.** `optimize_words` already calls
  `loimi_jit_invalidate`/`loimi_jit_rewritten`, which uninstall traces
  and re-patch the stencils; the test runs under the JIT matrix.
- **Cost.** One pass over the function per `become` (three `calloc`s
  and a DP over its words). Mini's hand `refuse` paid the same.

**Ports** (same method; base is master, with Mini's and MiniJS's hand
`refuse`):

| Suite | Change | min / median | A/A min / median |
|---|---|---:|---:|
| Lox (9, 11) | `refuse fp.cl.proto.code;`, six supers of quickened members | 0.973 / 0.986 | 1.013 / 0.997 |
| Lox, `--align-functions=128` (9, 9) | same | 0.985 / 0.984 | 1.002 / 1.005 |
| Mini (14, 9) | the hand `refuse` (5 calls) replaced by the item | 0.999 / 0.995 | 0.997 / 0.994 |
| Mini, 128 (14, 5) | same | 1.002 / 1.002 | 1.000 / 1.001 |
| MiniJS (14, 9), first cut | the hand `refuse` (6 calls) replaced; every `become` of a family in a super re-fused | 1.010 / 1.013 | 1.004 / 1.004 |
| MiniJS (6 object benchmarks, 9) | only `become`s of ops a super names re-fuse | 1.001 / 1.008 | 1.001 / 0.996 |
| MiniJS, 128 (14, 7) | same | 1.003 / 1.005 | 0.997 / 1.001 |

- **Lox** keeps it: zoo 0.883 (0.878 at 128), invocation 0.91–0.98,
  properties 0.98; trees moved 1.00 at 64 and 1.04 at 128.
- **Mini** keeps it. The hot code is the same: Mini's families in
  supers are the arithmetic ones that called `refuse` by hand.
- **MiniJS does not; its port was reverted.** Re-fusing after property
  quickening forms supers that its hand `refuse` (arithmetic only) never
  formed in deltablue: `GETMETHOD_P_CALL`, `GETPROP_IN_GETMETHOD_P` and
  `GETPROP_IN_GETPROP_IN`, 10% fewer dispatches (`--vm-stats`). Yet
  deltablue is 1.006–1.06 slower (1.057 at 128), and richards 1.02–1.03
  at 128. Those supers were picked when they formed only by accident
  (in functions where an arithmetic site also quickened). They are a
  candidate for re-measuring, not for this item.
- **The first cut** re-fused after every `become` of a family that some
  super named, misses and polymorphic members included. It was the
  slower one (MiniJS 1.010, deltablue 1.05). Now only a `become` to an
  op that a super names (or that has accumulator variants) calls
  `Vm.refuse`.

At 64 Lox gained zoo 0.883 and invocation 0.907 (min) / 0.962 (median).

**(b) `supers from "file";`** reads the first list of supers that
`--vm-stats` suggests in the `== vm stats: NAME ==` sections of the file
(or plain `super` lines), skips stale ones (parts the vm does not have)
with a warning and ones declared already silently, and accepts
`#[forward]` and `#[allow(super_overlap)]` on the item. Cheap (60
lines in `vmexpand.rs`), so built; no example uses it yet: the
examples' supers are hand-picked from several reports and measured one
by one.

### 9. A value-stack and frame runtime

**Evidence.**
- **Clearing registers at calls costs.** Calls clear the callee's
  registers so that the collector never marks a stale value. The fix,
  a high-water mark below which every slot may be marked, was built by
  hand three times:
  - MiniJS: deltablue, richards and raytrace 0.94–0.97;
  - Scheme: fib 0.496 → 0.400, tak 0.453 → 0.385, ack and fibfp −8%
    and −15%;
  - Python, which never cleared (`vm.lm:109`).
- **Mini failed three times.** The unsafe upper bound was fib 0.92.
  The safe version had to raise the mark across coroutine stacks,
  nested runs, `--cfg gil` threads and the debugger. It measured
  0.995–1.003 overall, with oo_methods 1.04–1.05 unexplained
  (`RESULTS.md:495-513`).
- **Calls cost.** CALL and RET are 15–25% of MiniJS's call-heavy
  programs. A Python method call is about 70 instructions with 8 frame
  stores. A Scheme call is 30–40 instructions. `std.rt.FrameStack` is
  designed but not built (`language.md` §10).

**Proposal.** `std.rt.ValueStack<V>`, the protocol the three
interpreters wrote:
- a high-water mark per stack;
- a root scanner that marks below it and clears above the top window;
- `grow` that relocates;
- `ensure(top)` folded into the stack-overflow compare.

Each coroutine owns one, and suspended stacks are logged for the next
collection, as Mini's attempt did. Pair it with `#[derived]` frame
fields that are recomputed from the function rather than stored
(Python's next candidate: "`k` and `proto` follow from the function").

<!-- snippet: skip design, not built -->
```loimi
context { stack: rt.ValueStack<V>, .. }
op CALL(f: reg, n: imm) : ABC {
    let top = regs + f + p.nregs;
    if unlikely(!ctx.stack.ensure(top)) { cold { .. } }  // one compare: limit and high-water mark
    ..
}
```

**Measure.** Mini (fib, closures, oo_methods, coroutine_ring) and Lox,
the two that still clear.

**Prototype.** Mini's attempt-3 patch is described in `RESULTS.md`.
Re-run it after items 1–2 change the handler layout, since its loss
was a placement-sensitive oo_methods.

**Cost: M.** Risks:
- the measured gain where it is not yet built is small;
- the oo_methods effect is unexplained.

**Interactions.**
- **GC:** the scanner is the core of it.
- **Debugger:** it must show only live ranges, as MiniJS's does.
- **Coroutines:** item 11.

**General:** the protocol is general, but its *speed* gain is mostly
taken already. It ranks low for speed, higher for ergonomics.

### 10. Adaptive quickening (warmup and backoff counters)

**Evidence.**
- **CPython.** PEP 659 specializes after a warmup counter, with a
  12-bit value and 4-bit exponential backoff on deopt
  (`pycore_backoff.h:20-45`, `pycore_code.h:540-571`).
- **Loimi.** Every Loimi family specializes on first execution, and
  none counts. No flip-flop between members has been measured yet. The
  closest evidence:
  - Mini's MOD recomputing its magic on every miss cost 12%;
  - Python's global method cache probed without filling the site cost
    deltablue 6%.

**Proposal.** `#[adaptive(warmup = 1, backoff)]` on a family adds a
16-bit counter to its cache. `generic` runs the specializer when the
counter reaches zero. A member's `become <generic>` on a guard failure
doubles the next wait, up to a cap.

<!-- snippet: skip design, not built -->
```loimi
#[adaptive(backoff)] family GETPROP(dst: reg, obj: reg, name: kst) : ABC + PropIC {
    generic { ..; become GETPROP_MONO; next }   // the compiler counts down first
    GETPROP_MONO { ..; cold { become GETPROP; redo } }
}
```

**Measure first.** Use `--vm-stats` cold-block counts on the deopt
blocks (`cold { become X; redo }`). A site that deopts more than a few
times per thousand executions is flip-flopping. If no benchmark shows
that, do not build this.

**Cost: S.** It fits in `cache`. Risk: none beyond the cache word.
**Interactions:**
- **`redo_effect`:** the counter decrement is an effect, so it must be
  generated after the last read;
- **JIT:** the counter decrement is a rewrite trigger;
- **`Vm.deopt`:** zeroes the counter.

**General:** six interpreters with families. Gain unknown.

### 11. Coroutine and generator switching

**Evidence.**
- **Mini.** The coroutine resume and yield cold blocks are 31% of
  chameneos (`RESULTS.md:524`), the one Mini benchmark slower than
  LuaJIT `-joff` (1.06, `results.md`). Mini switches threads inside the
  dispatch loop through CALL's cold native path: save the state
  registers, relink `Coro.prev`, and reinstall
  (`examples/mini/COROUTINES.md`).
- **Inlining.** The switch helpers outgrew LLVM's inlining threshold
  once barriers were inline: chameneos and coroutine_ring got 6%
  slower, and Mini now logs barriers by hand (`lowering.md` §3).
- **Python** generators copy every register on YIELD and resume
  (FRICTION 519–537, 399).
- **`suspend`** and `std.rt.Fiber` are designed and not built
  (`language.md` §10).

**Proposal.**
- **`switch_state(save: *S, load: *S)`**: a statement that stores
  every state variable into one struct of the vm's generated
  `Vm.State` type and loads another, in one basic block, from a
  handler.
- **Quickened call sites.** Calls of `coroutine.resume` and `yield`
  become ops of their own, members of the CALL family, so the switch
  is a hot path, not a cold block reached through native-call
  recognition.
- **Generators.** Python's generators get the same switch with their
  window left in place.

<!-- snippet: skip design, not built -->
```loimi
// CALL_RESUME, a CALL family member chosen when the callee is coroutine.resume
CALL_RESUME {
    let co = ..;
    switch_state(&ctx.cur.saved, &co.saved);   // all state registers, no sync/reload pair
    ..; next
}
```

**Measure.** Mini's coroutine suite (chameneos, coroutine_ring,
sieve), and a Python generator benchmark (nqueens).

**Prototype.** Make CALL a family in Mini, with a `CALL_RESUME`
member chosen when the callee is `coroutine.resume`, and move the
switch code out of the cold block. The language does not change. If
chameneos gains under 10%, stop here.

**Cost: M** for `switch_state` and the GC's view of saved states.
Risks:
- the saved state must be traced (it holds `Gc` registers);
- a `#[trace]`-derived `Vm.State` covers that.

**Interactions.**
- **GC:** saved states are roots.
- **Debugger:** must show the switched-to thread.
- **Nested runs:** `Vm.depth` is unchanged, since the switch happens
  inside one run.
- **JIT:** a computed transfer, as in item 13.

**Niche:** Mini and Python today.

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

**Evidence.**
- **Mini's SWITCH.** It jumps to a JMP that jumps to the arm, one
  dispatch more than needed. Decoding the target in SWITCH itself was
  4% faster on interp, but it crashes `--jit-stencils`, "whose runtime
  handles jumps to computed instructions, not to their targets" (item
  42).
- **Wasm's BR_TABLE** to its target measured nothing (1.796 against
  1.797 s), so the gain is Mini-specific.

**Proposal.** A computed `jump` to a code address in stencils exits
through the enter word of the target instruction (the runtime's
existing exit path), instead of assuming a jump target is a
`pc + it` label.

**Cost: S** in `jit_rt.c`. **Niche:** one benchmark, and the JIT line
is paused (`copy-and-patch.md` §13).

### 14. An optimizing tier

**Evidence.** `copy-and-patch.md` §7 and §13:
- dispatch and decoding were the part a template JIT could remove, and
  it removed them;
- what remains (register-file traffic, repeated type checks, calls)
  needs register allocation over a loop or trace, guards hoisted out of
  it, and arguments in registers;
- by hand, mandelbrot's loop runs at 0.56–0.68 of the JIT's time with
  registers allocated and checks removed.

LuaJIT's trace JIT, Guile's JIT (2–3.6× on call-bound Scheme) and V8's
tiers are where the references are faster than us.

**Proposal.** None here beyond naming it. It is the only route to
another 2× on numeric loops. The shared-frame design (interpreter and
compiled code share state) is what makes deoptimization cheap, and it
should be the first constraint of any design.

**Prototyped** (`trace-tier.md`). Traces stitched mechanically from
the stencils, with the register file as an SSA window and guards from a
profile, ran at:
- mandelbrot 0.63 of the JIT's time (inside the bound);
- nbody 0.73;
- fannkuch 0.82;
- sieve 0.87.

Recommendation: go, staged.

**Cost: L+.**

## 2. Reference techniques, against what Loimi offers

Status: **N** native, **H** by hand only, **X** not expressible.

| Technique | Used by | Loimi | Where a gap remains |
|---|---|---|---|
| State in fixed machine registers | LuaJIT (`vm_x64.dasc:19-56`), wasm3 | N: `state` under `preserve_none` (`lowering.md` §3) | none |
| Decode-ahead, replicated dispatch | LuaJIT `ins_NEXT` (`:212-224`) | N: decode-ahead; goto mode copies the dispatch (`lowering.md` §5) | LuaJIT also pre-decodes the next op's RA/RC; not worth it with three-field words |
| Handler address in the code stream | wasm3 | X | item 2 |
| Operand slot as a byte offset | wasm3 (slot offsets in their own code words) | H: an operand-kind expression can do it if the encoder scales by hand | item 2 |
| NaN tagging, dual int/double | LuaJIT | N: `nanbox`, merged tuple tests (`language.md` §2) | int-high layout, item 7 |
| Top of stack in a register | wasm3 `_r0`/`_fp0` (`m3_exec.h:119-146`) | N for numbers (accumulators); X for tagged values; adjacent pairs only | item 1 |
| Stack caching with several states (Ertl) | Forth systems | H: Stak's `tos` state variable by hand (10–15%, Stak FRICTION 358) | derived variants from `stack(..)` effects; niche (one stack VM), not ranked |
| Inline caches in the code stream | CPython `_Py*Cache` | N: `cache`, `cache X = S` (`language.md` §7.1) | weak cache fields not built |
| Quickening, families, deopt | CPython PEP 659 | N: `family`, `become`, `Vm.deopt` (§7.2) | counters, item 10; re-fusion, item 8 |
| Warmup and backoff counters | CPython `pycore_backoff.h` | H: a cache field, which nobody writes | item 10 |
| Static superinstructions | CPython (LOAD_FAST_LOAD_FAST), QuickJS | N, beyond the references: `supers`, DP `optimize`, `#[forward]`, suggestions (§7.3) | selection by profile, item 8(b) |
| Dynamic superinstructions, replication (Piumarta & Riccardi) | — | N in effect: `--jit-stencils` copies stencils per instruction | — |
| Compare fused with the next JMP | LuaJIT ISLT, Lua 5.4, wasm3 `BranchIf_r` | N: `then OP` (§7.1a), `rel` (§7.5) | none |
| Ops per operand kind (ADDVN, ADDK, H_SMALLINT, get_loc0) | LuaJIT, Lua 5.4, SWI, QuickJS | H: `inline for` over kinds, `rel kform` | ergonomic only |
| Lazy compile by self-patching | wasm3 `op_Compile` | H: `Vm.Resume` caches, `become` | none |
| Unboxed typed slots | Guile (`uadd`, `fadd`, `untag-fixnum`) | N: several operand-kind views of one slot (wasm) | the guest compiler's job |
| Hot counters, tier-up | LuaJIT `hotloop`, Guile `instrument-entry` | N only inside `--jit-stencils` | an `on hot` hook outside the JIT; not ranked (no speed evidence) |
| Zero-cost hooks when off | Lua `trap`, Guile `NEXT_HOOK` | N, better: instrumentation variants swap tables (§9) | none |
| Clause indexing | SWI-Prolog | runtime work in the interpreter (Prolog's first-argument CALL indexing) | not a language feature |
| Trace or method JIT | LuaJIT, Guile, V8 | X | item 14 |

The only reference techniques Loimi cannot express at all are the
handler address in the code stream (item 2), a tagged value in a
register across ops (item 1), and a real compiler tier (item 14). Every
other reference technique is native, or needs boilerplate only.

## 3. Prototype plan

Each item's experiment above is sized so that a negative result costs
little. In order:

| Experiment | Item | Effort | Stop if |
|---|---|---|---|
| Pre-scaled slot offsets in `examples/wasm` by hand (encoder `* 8`, operand kinds as byte offsets) | 2 | half a day | PolyBench and wasm geomean move < 2% (**done:** 1.003 and 0.998; stopped) |
| Offset-threaded dispatch by IR patch on the wasm tail-call build | 2 | 1–2 days | < 3% on top of the above (**done:** 1.008 and 1.004, slower; stopped) |
| Mini float members with a raw `u64` accumulator, no collection (heap budget raised) | 1 | 1 day | mandelbrot, nbody and spectral_norm all > 0.95 |
| Mini's `mod_magic` in Prolog's MOD/IDIV with constant divisors | 3 | 1 hour | crypt < 5% faster (unlikely) |
| `dereferenceable` and `!invariant.load` added to Python's GETATTR IR | 4 | 1 day | deltablue, go and richards < 2% |
| Scheme pairs from a 16-byte no-GC arena against 32-byte GC pairs, collection disabled for both | 5 | 2 days | nboyer, sboyer, mperm and earley < 5% |
| `refuse` in Lox and Python arithmetic families | 8 | 1 hour each | noise |
| `--vm-stats` deopt cold-block counts over all suites | 10 | 1 hour | no site deopts more than 1 in 1000 |
| Mini `CALL_RESUME` family member | 11 | 1 day | chameneos < 10% faster |

Run every timing as the logs do: `--align-functions=64`, interleaved,
minimum of 5 or more. Rerun suspects at `--align-functions=128`,
because placement alone moves single benchmarks by 3–9%
(`results.md`; Scheme tak ±9%).

## 4. Considered and rejected

- **Guard pages instead of wasm bounds checks.** 4 GiB plus a guard
  needs 8 GiB of address space per memory, and `PROT_NONE` mappings
  count against `ulimit -v` (wasm FRICTION 728). The check is already
  `add; cmp; ja`, fused and predicted. Hoisting checks per basic block
  saves one compare per access, at the cost of deferred, ordered traps.
- **Loop-carried MOV elimination.** Local-to-local MOVs are 7–10% of
  dispatches in four wasm programs and 20% in seidel-2d. They are
  rotations of values that are live at once, so liveness coalescing
  cannot remove them. Removing them is modulo variable expansion, that
  is, unrolling. MOV2 merging measured 0.997 and 0.998 (FRICTION 574).
  With item 1's selection past non-interfering ops, a MOV out of an
  accumulator becomes a store, which is the most a language feature can
  do.
- **More guest registers in machine registers, in templates.** The
  one-register JIT cache measured 1.015 (slower), and the bound needs
  loop register allocation (`copy-and-patch.md` §13). That is item 14,
  not a template feature.
- **Stencil hot/cold splitting, constant patching in the JIT.**
  Measured: no gain (`copy-and-patch.md` §10, §12).
- **Frame-free slow paths through inline-asm `preserve_all` stubs.**
  The 218 framed handlers were the motivation, but removing the frames
  measured 0.995 over 102 benchmarks, `.text` grew 8–30%, and Stak goto
  got 1.34× slower (`lowering.md` §3). Panic stubs removed 1 frame of
  218. The commonest causes (barrier, refill) are gone, and the logs
  put each remaining frame at "a few percent" on its own op.
- **Pinning hot constants (the nanbox tag) in tail-call mode.** goto
  wins fib by keeping them in registers (`bench/README.md`). But a
  rematerialized constant is one `movabs`, off the critical path, and
  goto is 1.03× tail-call over the Mini suite: the opposite direction.
  It would cost an argument register.
- **Handler replication for branch prediction** (Ertl & Gregg).
  Tail-call and goto modes already give every handler its own indirect
  jump, and the remaining placement effects are uop-cache and BTB
  aliasing, which `#[profile]` ordering addressed (stability, not
  speed).
- **Type-check elimination across ops.** Scheme estimated "one
  predicted branch, a few % at best" (README 423). Within a super, LLVM
  already merges repeated tag tests; across ops it needs types per
  slot, which is item 14.
- **Interner layouts.** MiniJS tried slots pointing at records and key
  bytes inline, and Mini tried shrink thresholds: all noise or worse.
  What remains (`Ctx.key` 13% of strings) is hashing and building
  31,000 keys, which is the interpreter's string representation, not
  the intern table.
- **A moving or bump-pointer nursery.** Python's generational mode
  measured 0.982 and was not kept. Prolog measured 1.01. Mark bitmaps
  already made allocation a bump through runs. A moving collector would
  break raw pointers in state registers and `Vm.Resume` caches, for a
  gain the logs do not show. Item 5 attacks object size, which the logs
  do show.
- **Int(i48) everywhere.** Measured +4–9% in Mini and kept i32. Item 7
  is the representation that would make it free.
- **Direct `call`/`ret` for guest calls**, to use the return-stack
  predictor. RET's dispatch is one indirect jump per return site.
  - **Against:** the logs attribute call cost to frame stores, clearing
    and checks, not mispredicts (MiniJS RESULTS 176, Python FRICTION
    388), and the target is usually one handler per site.
  - **Cost:** native returns would put guest recursion on the C stack
    and break coroutines, `raise` and the shared-frame JIT.
  - **To revisit:** with item 6's profiler, if RET shows up as
    mispredict-bound.
- **`--cpu=x86-64-v3` / `native`.** Noise for interpreters (wasm
  FRICTION 344, `lowering.md` §12).
- **A balanced compare tree for if-chains.** Hot arms come first
  (Mini, after item 42).
