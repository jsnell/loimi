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
the interpreter's time); an ahead-of-time optimizing tier for the second
was built and removed (item 14).

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
| 12 | Trailing immediate words | no-go | the checked cache-word ops run 0 times on the wasm suite |
| 13 | JIT stencils: computed jumps to a target | built | Mini interp 0.977 (interpreter) |
| 14 | An optimizing tier | built as ahead-of-time traces, then removed | Mini 0.76 of the JIT, on its training programs |

---

### 1. Accumulators for `value` types

**Why.** Accumulators (`language.md` §7.4) took the wasm interpreter to
0.91 of its time (0.79 with supers over the variants), but held numbers
only, so none of the nanbox or low-tag VMs could declare one. A value
accumulator holds a `value` word between two adjacent ops, saving the
store-to-load round trip through the register file.

#### Prototype results (2026-11-06)

An uncommitted patch (`Vm.accumulate` on family members, an
`accumulator vacc: u64|f64 for reg;` in Mini over 14 float and
compare-and-branch ops, 76 variants, ten `#[forward]` supers over them)
measured, user+sys CPU, minimum of 5 interleaved runs, with collection
switched off in every build:

| build | mandelbrot | nbody | spectral_norm | suite geomean (14) |
|---|---:|---:|---:|---:|
| A/A | 1.000 | 0.998 | 0.999 | 1.001 |
| u64 accumulator, no supers on variants | 0.983 | 0.982 | 1.017 | 0.999 |
| u64 accumulator + supers | 0.921 | 0.978 | 1.005 | 0.993 |
| f64 accumulator + supers (untraced bound) | 0.855 | 0.972 | 1.017 | 0.991 |
| f64 write-through + supers (GC-safe) | 0.914 | 0.966 | 1.027 | 0.994 |

At `--align-functions=128` mandelbrot held (0.852, 0.910) and nbody's
gain went (1.00): only mandelbrot, 73% of whose dispatches take a
variant, gains for real. What decided the design:
- **f64, not a GPR.** A u64 variant pays two `movq` on the critical
  path, and the u64 builds were 1–5% slower on queens and sort, which
  run no variant.
- **Write-through, not spill-at-sync.** Storing the slot too keeps GC,
  polls, `raise`, the debugger, `Vm.deopt` and the JIT unchanged, and
  lets locals chain (`first_temp` 0) where only temporaries could
  otherwise. It cost mandelbrot 6 points (0.855 against 0.914); §3.
- **Supers over variants are required:** without them variants break
  up the `#[forward]` supers (0.983 against 0.921).
- **Selection stays adjacent.** Every hot pair in Mini's loops was
  adjacent, so selection past ops that leave the slot alone (wasm3's
  `PreserveRegisterIfOccupied`) had nothing to take.
- **Quickening.** The `_F` members appear only after `become`, so
  selection runs again from `refuse`, which needs `Vm.accumulate` to be
  idempotent.

#### As built (2026-11-06)

Built as the prototype said, with one change of design: **there are no
writer variants.** With write-through, a variant that writes the accumulator
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

**Python: not ported (2026-11-08).** A bound with its dunder calls cut
out of 10 ops (51 variants, a `u64` accumulator): nbody 0.91/0.90 at
align 64/128, but raytrace 1.18/1.27; suite 1.000/1.028 (A/A 1.011/0.982).

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

**Why.** Python went to `Int(i48)` so that sums past 2³¹ do not
allocate (84 → 18 ms), and paid for it: spectral_norm 8–9% slower,
because the overflow check is `shl; shl; add; jo; shr` instead of
`add; jo` (FRICTION 449–467). Mini measured i48 at +3–9% and kept i32,
as LuaJIT's DUALNUM mode does. The option: store `Int` payloads in the
high 48 bits with a zero low tag and NaN-tag the other variants so that
their low bits are never zero; then two Ints add as `add; jo` on the
words, and floats pay a `ror` or `xor` to decode. Integer code gets
faster and float code slower, so it is opt-in.

#### Prototype (2026-11-06)

A hand prototype on Python's `ADD` could not work. On the plain words an
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
  the float gets none (its test is not a mask test of the stored word)
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
- **JIT.** `optimize_words` calls
  `loimi_jit_invalidate`/`loimi_jit_rewritten`, which re-patch the
  stencils; the test runs under the JIT matrix.
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
jump), so the JIT needs nothing new.

**Rejected: a `switch_state` vm primitive** (2026-11-06). A state
snapshot saves only Mini's two dependent loads `fp.base` and `fp.k`,
which are off the critical path; the rest of a switch (about 35–40
cycles) is the mispredicted jump and Mini's own bookkeeping, which a
primitive would have to store too. Python generators already switch on
the hot path (FORITER 10.9% and YIELD 1.9% of nqueens samples), and
their register copy is the design.

### 12. Trailing immediate words

No-go (2026-11-08), bound 0%: the ops that check a cache-word index
(CALL_INDIRECT_X, the bulk ops) never run on the wasm suite (40 modules,
one table, no bulk op); CONST64 and ADDR read theirs unchecked.

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

Built as ahead-of-time traces (2026-11-06 to 11-09) and removed: hot loops of
profiled programs, compiled at build time from the JIT's stencils, took 0.76–0.87 of
the JIT's time only on the programs trained on (an interpreter's builder does not
know its scripts), at four interpreter builds per test.

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
| Inline caches in the code stream | CPython `_Py*Cache` | N: `cache`, `cache X = S`, `const` fields (`language.md` §7.1) |
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
| Trace JIT | LuaJIT | not built (item 14 tried traces ahead of time) |

## 3. Tried and rejected

Items 2, 4, 5, 10 and 12 and the `switch_state` primitive of item 11
are above. The rest:

- **Guard pages instead of wasm bounds checks.** 4 GiB plus a guard per
  memory counts against `ulimit -v`; the check is already a fused,
  predicted `add; cmp; ja`.
- **Loop-carried MOV elimination.** wasm's local MOVs (7–10% of
  dispatches, 20% in seidel-2d) rotate values live at once; MOV2 merging
  measured 0.997 and 0.998.
- **Guest registers in machine registers, in the JIT.** One cached
  register measured 1.015 (`copy-and-patch.md` §13).
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
  repeated tag tests; across ops it needs types per slot (item 14).
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
- **A "known `Num`" assume on float constructors** (2026-11-08): Mini's
  mandelbrot trace 0.987/0.997 at align 64/128 (A/A 0.980/0.985), its
  interpreter 1.044/1.048; MiniJS nbody 0.986 (A/A 0.990).
- **Retagging Mini's FORLOOP counter** (`Int(c -% 1)`, 2026-11-08): the
  loop-carried test stays without peeling. Suite at 64: traces 0.983
  (A/A 0.987), interpreter 0.992 (0.990), an extra `or`.

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
