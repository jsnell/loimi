# Design note: a speed audit (features for faster interpreters)

Status: **proposal; nothing in it is built.** Written 2026-11-06 against
master at 0813bc1.

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
| 2 | **Cheaper operand decoding and dispatch**: pre-scaled byte-offset fields, then offset-threaded dispatch (a handler offset in the word, no table load) | `F64_MUL` is 14 instructions: 6 decode, 3 work, 5 dispatch. wasm3's is 5 (`roadmap.md` Q11). Every interpreter pays this on every op | 2–6% on handler-bound code (est.), all VMs | M, then L |
| 3 | **Division by a verified constant**: `std.math.Divisor` (Granlund–Montgomery) and a verify-time hook that fills it | Prolog crypt: MOD/IDIV 27% of samples. MiniJS closures: MODK's `idiv` 8%. Mini built a magic-number IC by hand (`vm.lm:833`), and recomputing the magic on a miss cost 12%. Python slicing: `idiv` 14% (special-cased by hand) | 5–20% on 3–4 benchmarks; ~0.5% suite | S |
| 4 | **Load facts**: `dereferenceable`/`nonnull`/invariant loads on pointers and `const` operand kinds | Listed in Mini's "remaining compiler issues" (`RESULTS.md:579`). Dependent-load chains: Python GETATTR 14–26%, MiniJS GETPROP_IN. GETINDEX_GETINDEX (20% of interp) keeps a frame | unknown; 0–3% (est.) | S |
| 5 | **Smaller objects**: typed small-object chunks with no per-object header (BiBoP), or an 8-byte header for fixed-size types | Every object has a 16-byte header. A Scheme pair is 32 bytes against Guile's 16, and CAR alone is 12–15% of nboyer. Size sensitivity, measured: +8 bytes per object cost MiniJS binary_trees 6% and Python float 6% | 3–10% on allocation- and memory-bound benchmarks in 6 VMs (est.) | L |
| 6 | **A sampling profiler in std** (`std.prof`), mapped to handlers, cold blocks and guest pcs | Every tuning round wrote its own LD_PRELOAD SIGPROF sampler (wasm FRICTION 366, Python FRICTION 618, Mini RESULTS:9). The 64 KiB verifier memset (Prolog sieve 0.54) was found only by sampling | indirect; it finds the next items | S–M |
| 7 | **Shifted-int nanbox option** (Int payload in the high bits, so `add; jo` checks overflow) | Python's i48 overflow check is `shl; shl; add; jo; shr`, costing spectral_norm 5–8%. Mini measured i48 at +4–9% and kept i32 (`RESULTS.md:527`) | 3–8% on integer-heavy code in Python, and i48 for Mini (est.) | M |
| 8 | **Re-fusion on `become`**, and supers from a profile file | Mini and MiniJS call `refuse` (a whole-function `optimize`) by hand after quickening; doing so gave mandelbrot −6% and nbody −5%. Lox, Python and Scheme don't do it | 1–3% where quickening precedes fusion (est.) | S |
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
| Pre-scaled slot offsets in `examples/wasm` by hand (encoder `* 8`, operand kinds as byte offsets) | 2 | half a day | PolyBench and wasm geomean move < 2% |
| Offset-threaded dispatch by IR patch on the wasm tail-call build | 2 | 1–2 days | < 3% on top of the above |
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
