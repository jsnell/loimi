# Design note: a speed audit (features for faster interpreters)

A record of which features were considered to make Loimi interpreters
faster, what each measured, and what was built. For contributors who
want to know why a speed feature exists, or why an obvious one doesn't.

**Status: done.** Started 2026-11-06. Every item was either built or
measured and dropped.

## What the audit asked

Which language, compiler, runtime or std features would let an author
write a faster interpreter than before? A feature qualified only if it
made interpreters faster, not just shorter.

Candidates came from:

- the bench and friction logs (`bench/*/RESULTS.md`, `bench/*/README.md`,
  `examples/*/FRICTION.md`);
- the cores of the reference implementations under `bench/*`: LuaJIT's
  `vm_x64.dasc`, wasm3's `m3_exec.h` and `m3_compile.c`, CPython 3.13's
  `pycore_code.h` and `pycore_backoff.h`, Guile's `vm-engine.c`,
  SWI-Prolog's `pl-vmi.c`, QuickJS and PUC-Lua.

A tuned Loimi handler has three kinds of cost left:

| Cost | Example | What addresses it |
|---|---|---|
| Dispatch and operand decoding | About 11 of the 14 instructions of a three-operand float op (wasm3: 5) | The JIT (0.76–0.83 of the interpreter's time) |
| Memory traffic through the register file | Store-to-load forwarding between dependent ops | Accumulators (item 1) |
| Runtime work | Calls and frames, object size, the collector, division, hashing | Items 3, 9, 11 |

No single feature in any of these is worth more than a few percent
across a whole suite.

## How timings were taken

Unless an item says otherwise:

- user+sys CPU time, the minimum of interleaved runs, with the order
  rotated each round;
- an A/A column: a byte-identical copy of the baseline in the same
  interleave, to show the noise;
- `--align-functions=64`, with suspects rechecked at 128 (code placement
  alone moves single benchmarks by 3–9%);
- an i7-6700 (Skylake), LLVM 19.

Ratios are new time / old time: below 1 is faster.

## Items

| # | Item | Verdict | Why |
|---|---|---|---|
| 1 | Accumulators for `value` types | Built; used by Mini and MiniJS | Mini mandelbrot 0.91, MiniJS nbody 0.93–0.97; suites within noise |
| 2 | Cheaper operand decoding and dispatch | Dropped | PolyBench 1.003 (pre-scaled fields), 1.008 (offset-threaded) |
| 3 | Division by a verified constant: `std.math.Divisor` | Built | Prolog crypt 0.855, MiniJS closures 0.970 |
| 4 | Load facts (`dereferenceable`, `nonnull`, `!invariant.load`) | Dropped | Changed 0 of 154 Mini and 0 of 165 MiniJS handlers |
| 5 | Headerless typed chunks (16-byte pairs) | Dropped | At best 0.971 / 0.993 on four Scheme benchmarks; too much to change |
| 6 | A sampling profiler: `--vm-prof` | Built | Agrees with a hand sampler within 1 point; costs 3% at 1000 Hz |
| 7 | Shifted-int nanbox: `repr nanbox(int_high)` | Built; used by Python and Prolog | Python suite 0.93, Prolog 0.92; float-heavy vms got slower |
| 8 | `refuse CODE;` and `supers from "file";` | Built | Lox suite 0.973, zoo 0.883 |
| 9 | `std.rt.ValueStack` | Built | Lox suite 0.970, fib 0.863 |
| 10 | Adaptive quickening (warmup and backoff counters) | Dropped | Later `become`s are at most 0.055% of dispatches |
| 11 | Coroutine switching | Built in Mini only; a vm primitive was dropped | chameneos 0.925 |
| 12 | Trailing immediate words | Dropped | The ops that would gain run 0 times on the wasm suite |
| 13 | JIT stencils: computed jumps to a target | Built | Mini interp 0.977 (interpreter) |
| 14 | An optimizing tier | Built as ahead-of-time traces, then removed | Faster only on the programs it was trained on |

Items 1, 3, 6, 7, 8, 9, 11 and 13 have sections below with what decided
the design. The dropped items' sections are their full record.

---

### 1. Accumulators for `value` types

Accumulators (`language.md` §7.4) took the wasm interpreter to 0.91 of
its time, but held only numbers, so no nanbox or low-tag vm could use
one. A value accumulator holds a `value` word in a register between two
adjacent ops, saving the store and reload through the register file.
The syntax and the rules that keep it correct are in `language.md` §7.4;
the register and codegen details in `lowering.md`.

**Decisions** (from a Mini prototype over 14 float and compare ops, GC
off):

| Build | mandelbrot | nbody | Suite (14) |
|---|---:|---:|---:|
| A/A | 1.000 | 0.998 | 1.001 |
| u64 accumulator, no supers over variants | 0.983 | 0.982 | 0.999 |
| u64 accumulator + supers | 0.921 | 0.978 | 0.993 |
| f64 accumulator + supers (not GC-safe) | 0.855 | 0.972 | 0.991 |
| f64 write-through + supers (as built) | 0.914 | 0.966 | 0.994 |

- **An `f64` register, not a general-purpose one.** A u64 variant pays
  two `movq` on the critical path, and the u64 builds were 1–5% slower
  on queens and sort, which use no variant.
- **Write-through.** A writer also stores the slot. This keeps GC,
  polls, `raise`, the debugger, `Vm.deopt` and the JIT unchanged, and
  lets locals chain, not only temporaries. It cost mandelbrot 6 points
  (0.914 against 0.855).
- **No writer variants.** With write-through, every writer leaves its
  result in the accumulator before each `next`, and only readers get
  variants. LLVM forwards the stored value, so the write is a register
  move. Mini's 20 ops give 28 variants (the prototype had 76 from 14
  ops), so 233 of 256 opcodes fit with the supers.
- **Supers over variants are required.** Without them, variants break up
  the `#[forward]` supers (0.983 against 0.921).
- **Only adjacent ops.** Every hot pair in Mini's loops was adjacent, so
  selecting past ops that leave the slot alone (wasm3's
  `PreserveRegisterIfOccupied`) had nothing to gain.

`Vm.deopt` needed one change: a value reader is a member of its op's
family (of the op itself, for a plain op), so deoptimizing a writer
also turns the reader after it back.

**The seventh argument.** An `f64` accumulator is a seventh register
argument to every tail-call handler. LLVM's x86 pattern for a tail call
through memory (`X86tcret_6regs`) refuses to fold the table load into
the jump past six register operands, of any class. So dispatch becomes
`mov T(,%rax,8),%rax; jmp *%rax` instead of `jmp *T(,%rax,8)`.

An assembly pass that folded the jumps back (Mini: 1043 of 1051) was
built, measured and reverted: folded against unfolded was within
0.995–1.005 on Mini, MiniJS and Scheme. A microbenchmark agreed: the
critical path is the two dependent loads, not the extra µop. What an
unselected accumulator does cost is `xmm0` live across C calls, and in
Scheme a `movq` on integer adds.

#### Ports

Median of 9–11 paired interleaved rounds, suite geomean against the
build without, A/A in brackets:

| Port | Float benchmark (64 / 128) | Suite, align 64 | Suite, align 128 | Kept |
|---|---|---:|---:|---|
| Mini (14) | mandelbrot 0.906 / 0.907 | 0.999 (0.998) | 0.993 (1.002) | yes |
| MiniJS (14) | nbody 0.974 / 0.931 | 0.990 (0.996) | 0.997 (1.003) | yes |
| Scheme (43) | mbrot 0.965 / 0.887 | 1.007 (1.006) | 0.999 (0.992) | no |

- **Mini**: 20 ops, 28 variants, ten supers over them. mandelbrot's
  loop is six dispatches.
- **MiniJS**: the float members, compare forms and SETPROP_IN. Four
  supers take nbody's `b.vx -= dx * b2.mass * mag` to one dispatch.
- **Scheme**: not kept. Over five suite runs it averaged 0.3% slower,
  with tak and fib slower every time (the `movq` on integer adds).
  mbrot's gain doesn't outweigh 42 mostly-integer benchmarks.
- **Python** (2026-11-08): not ported. A bound over 10 ops gave nbody
  0.91 but raytrace 1.18–1.27; suite 1.000 / 1.028.

### 2. Cheaper operand decoding and dispatch

Dropped (hand prototypes on `examples/wasm`).

- **Pre-scaled byte-offset slot fields**: PolyBench 1.003. The scale is
  free in x86-64 addressing, and the field extraction stays.
- **Handler offset in the word** (`lea` instead of the table load):
  PolyBench 1.008, slower on 22 of 30 kernels. The predicted jump
  already takes the table load off the critical path, and the `lea`
  takes an ALU slot.

### 3. Division by a verified constant

A 64-bit `idiv` takes 40–90 cycles on Skylake, against about 4 for a
multiply and shift. It was hot in three interpreters:

- Prolog crypt spent 27% of its samples in MOD and IDIV;
- MiniJS's `MODK` was 8% of closures;
- Mini had written Granlund–Montgomery division by hand.

A hand copy of Mini's method into Prolog gave crypt 0.846, so
`std.math.Divisor` was built. Its API is in
[`stdlib/math.md`](../stdlib/math.md). The design choices:

- **Divisors 1 <= d < 2^32, a 64-bit magic number.**
  floor(n / d) = hi(2n · m) >> l for every n < 2^63, so one form covers
  i32 and i48 payloads.
- **Four `u32` fields**, so `cache DivK = math.Divisor;` is a cache
  layout and `&cache` is a `*Divisor`. A constant-divisor op is one line:
  `d = Term.Int((&cache).rem_floor(x as i64) as i48)`.
- **`udiv31`/`urem31`**: one `imul`, for non-negative i32 values. In
  Mini a first version using the 128-bit product, with the cache filled
  inline, gave `MODK` a stack frame (closures 1.022). Filling the cache
  in a `cold` block and using `urem31` fixed it.

Ports, whole suites, minimum of 5 interleaved runs:

| vm | Ops | Result (A/A) |
|---|---|---|
| Prolog | `IDIVK`, `MODK`, `DIVK`, `REMK`; cache filled by the clause compiler | crypt 0.855, suite 0.9935 (0.9994) |
| Mini | `MODK`, `MOD`; cache filled on first run | suite 1.0002 (1.0013): it already had a magic number |
| MiniJS | `MODK`; cache filled by the emitter | closures 0.970, suite 0.9944 (0.9979) |

Prolog's results match SWI-Prolog on 98,016 lines of `//`, `mod`, `rem`
and `div` over 65 constants, negative constants, zero, the i48 limits
and bignums. Python divides only by run-time steps, so it has no
constant divisor.

### 4. Load facts: dereferenceable, nonnull, invariant

Dropped. `!nonnull` and `dereferenceable` on every `Gc` load, and
`!invariant.load` on proto and shape fields, changed 0 of 154 Mini
handlers, 0 of 165 MiniJS handlers and 3 Python ones, and timings stayed
inside the noise. A handler's loads form one dependent chain and each
field is loaded once, so there is nothing to hoist or reuse.

### 5. Smaller objects: headerless typed chunks

Dropped. Headerless 16-byte Scheme pairs instead of 32-byte ones gave
nboyer, sboyer, mperm and earley 0.971 (align 64) and 0.993 (128), even
prefaulted and with no collection. 24-byte pairs and Prolog's terms
gained nothing. Halving the memory used doesn't pay for changing the
allocator, barrier, generational mode, verifier and debugger.

### 6. A sampling profiler in std

Every tuning round had written its own `LD_PRELOAD` `SIGPROF` sampler:
`perf` is not available here, and `--vm-stats` counts dispatches, not
time. `loimi build --vm-prof` (`src/prof_rt.c`) replaces them. How it
samples and names code is in
[language.md §7.3](../language.md#sampling---vm-prof).

Decisions not covered there:

- **Goto and switch modes store the op number.** LLVM merges and copies
  blocks between handlers of one function, so marks at handler starts
  misplaced code: switch-mode Mini showed `LOADBOOL` at 8% on
  `binary_trees`. With the stored op, the three modes agree.
- **`-z now`.** On AArch64, a sample in the first call of `write`
  crashed libgcc's unwinder in one run of three of
  `tests/run/vm_prof.lm` (under qemu at 4 kHz). The cause is wrong
  unwind info in glibc's lazy-binding trampoline; binding at load time
  means it never runs. x86-64's trampoline is correct.

**Checked against a hand sampler** on the same binaries:

| Program | Top 5 | Largest difference |
|---|---|---|
| Mini `binary_trees` | Same five, same order | 0.6 points |
| Python `richards` | Same entries | 1 point |
| Python `raytrace` | Same entries | 0.8 points |

**Overhead**, minimum of 5 runs under 4 stack placements:

| Build | Instrumentation only (`LOIMI_PROF=0`) | Sampling at 1000 Hz |
|---|---:|---:|
| Mini, tail-call (8 programs) | 1.017 | 1.032 |
| Python, tail-call (6 programs) | 1.016 | 1.031 |
| Mini, goto (6 programs) | 1.078 | 1.086 |

The tail-call instrumentation cost is all the `pc` store. Goto mode
stores two words per op inside one large function (2–17%, `nbody`
most).

### 7. A shifted-int nanbox option

Python moved to `Int(i48)` so that sums past 2³¹ don't allocate, and
paid 8–9% on spectral_norm: the overflow check became
`shl; shl; add; jo; shr` instead of `add; jo`. Mini measured i48 at
+3–9% and kept i32.

`nanbox(int_high)` stores `Int` payloads in the high 48 bits with a zero
low tag, so two Ints add as `add; jo` on the words, and floats pay a
`ror` and `xor` to decode. Integer code gets faster and float code
slower, so it is opt-in. The layout and codegen are in `lowering.md`
("`nanbox(int_high)`").

It had to be a layout in the compiler, not a trick in one handler: on
plain nanbox words, no `add; jo` sequence gives a 48-bit overflow flag.

**Python** (the port that justified building it), minimum of 11
interleaved runs:

| | align 64 (A/A) | align 128 (A/A) |
|---|---:|---:|
| fannkuch | 0.865 (0.999) | 0.869 (0.999) |
| chaos | 0.906 (0.997) | 0.909 (0.996) |
| spectral_norm | 0.920 (1.004) | 1.001 (0.994) |
| nbody | 0.990 (0.999) | 1.012 (0.997) |
| **suite (11)** | **0.932** (1.000) | **0.942** (0.999) |

Two results were unexpected:

- **Float benchmarks gained too** (chaos, raytrace 9%). Every pointer
  payload became `shr $16` instead of an `and` with a 48-bit mask that
  needs a `movabs`. Python's handlers have 5% fewer instructions overall.
- **Float paths cost little.** `ADD`'s float path grew from 31 to 32
  instructions, because the int test in front got shorter. Only TRUEDIV
  (43 → 50) and mixed operations grew. nbody, nearly all floats, is the
  one slower benchmark, at align 128 only.

#### Ports

The other nanbox vms, minimum of 7–9 interleaved runs, suite geomeans,
A/A in brackets:

| vm | Change | align 64 | align 128 | Worst | Kept |
|---|---|---:|---:|---|---|
| Prolog (30) | `Term` was already `Int(i48)` | 0.917 (1.003) | 0.920 (1.008) | log10 0.97–1.06 | yes |
| Scheme (43) | fixnums `i32` → `i48` | 0.979 (0.994) | 0.978 (0.998) | sumfp 1.24–1.27 | no |
| MiniJS (14) | `Int(i32)` → `i48` | 0.941 (0.993) | 0.958 (1.049) | nbody 1.25–1.47 | no |
| Mini (14) | `Int(i32)` → `i48` | 1.034 (1.003) | 1.030 (1.001) | mandelbrot 1.80 | no |
| Lox (9) | no integers; `Native(u32) block 48` as the zero-tag variant | 1.005 (0.987) | 1.030 (1.007) | equality 1.06–1.13 | no |

Integer benchmarks gained everywhere (Prolog queens_8 0.78–0.80, MiniJS
bitops 0.74–0.76, Mini fannkuch 0.84–0.89). Float kernels lost: in a
tight float loop each operand read becomes a load, `xor`, `rol` and
`movq` instead of one `addsd` from memory, all on the loop's dependency
chain. So the layout pays for a vm whose values are rarely floats
(Python, Prolog), not for one with float kernels in its benchmarks.

What the ports added:

- **`V.Num.xor`** (`language.md` §2): the constant a float-first fast
  path needs. Under `int_high`, an f64 with bits `b` is
  `rotl(b, 16) ^ V.Num.xor`.
- **Prolog**: `#[scan_raw_words]` is refused under the layout, so its
  environment frames store their three header words as `Int` words.
  Index keys of Ints set bit 15, because `Int(0)`'s word is the no-key 0.

### 8. Re-fusion on `become`; supers from a profile file

A super whose parts are quickened ops can only form if `Vm.optimize`
runs again after `become`. Mini and MiniJS did this by hand (Mini
mandelbrot −6%, nbody −5%). `--vm-stats` showed hot pairs of two
quickened ops in other vms too: Lox's `GETGLOBAL_C; INVOKE_P` (31% of
invocation and properties), Python's `GETGLOBAL_C; GETATTR`, Scheme's
`GETGLOBAL_B; ...`. A hand prototype gave Lox 0.976–0.981 and Python
and Scheme nothing.

`refuse CODE;` is described in `language.md` §7.3 ("Fusing again after
`become`"). Why it re-runs `optimize` over the whole function: going
backward from `pc`, the word before may be a cache or operand word, so
only a walk from the start finds the previous instruction. Once the code
is walked, `optimize` costs about the same. The pairs that matter are
formed by the second `become`, which is the backward case.

Ports, min / median of interleaved runs:

| Suite | Change | Result | A/A |
|---|---|---:|---:|
| Lox (9) | `refuse fp.cl.proto.code;`, six supers of quickened ops | 0.973 / 0.986 | 1.013 / 0.997 |
| Mini (14) | hand `refuse` (5 calls) replaced by the item | 0.999 / 0.995 | 0.997 / 0.994 |
| MiniJS (6 object benchmarks) | hand `refuse` (6 calls) replaced | 1.001 / 1.008 | 1.001 / 0.996 |

Lox and Mini use the item (Lox zoo 0.883, invocation 0.91–0.98). MiniJS
keeps its hand `refuse` for arithmetic only: re-fusing after property
quickening cut deltablue's dispatches by 10%, yet it ran 1.006–1.06
slower.

`supers from "file";` reads the supers `--vm-stats` suggests in a report
file (`src/vmexpand.rs`). No example uses it: their supers are
hand-picked from several reports and measured one by one.

### 9. A value-stack and frame runtime

Calls cleared the callee's registers so the collector never marks a
stale value. MiniJS, Scheme and Python had each replaced that by hand
with a high-water mark (Scheme fib 0.496 → 0.400 of Guile's time).
`std.rt.ValueStack<T>` packages that; its API is in `stdlib.md`
(std.rt).

Frames are not part of it: all five interpreters keep frames in a
separate array, with layouts of 3 to 9 fields.

Two lessons from the ports:

- **Call `raise` from a `cold` block that ends in `redo`.** A first
  Scheme port that called `ensure` on the hot path gave every call
  handler a stack frame: fib 1.05, ntakl 1.08.
- **`raise` must not clear slots between the mark and `fresh`.** A
  first version did, and broke MiniJS under GC stress, because MiniJS
  writes argument slots above the mark before a call raises it.

Ports, minimum of 7–9 interleaved runs:

| vm | Before | align 64 (A/A) | align 128 |
|---|---|---|---|
| Lox | cleared at every call | 0.970, fib 0.863 (1.004) | 0.966 |
| Scheme | hand mark + `rt.grow` | 1.000 (1.000) | 0.995 |
| MiniJS | hand mark + `grow_stack` | 0.995 (0.998) | 1.002 |

Python never lowers its mark, so it has nothing to gain. Mini keeps
clearing: it relocates a stack per coroutine, and its third attempt at a
mark measured 0.995–1.003.

### 10. Adaptive quickening (warmup and backoff counters)

Dropped. Over the Mini, MiniJS, Lox, Python and Scheme suites, `become`s
after the first are at most 0.055% of dispatches. Float ops that keep
missing are bounded by a vm whose `ADD` never quickens: 0.982 on bitops.

### 11. Coroutine and generator switching

Mini's coroutine resume and yield went through CALL's native-call path:
31% of chameneos, the one Mini benchmark slower than LuaJIT `-joff`.

**Built in Mini, with no language change.** `CALL` is a family. A site
whose call switched threads quickens to `CALL_RESUME`, `CALL_WRAP` or
`CALL_YIELD`, which do the switch on the hot path and fall back with
`become CALL; redo`. Three supers keep the fused forms. The switch is
ordinary vm control flow (`jump` to a `Mini.Resume`), so the JIT needed
nothing new.

| Minimum of 11 runs | align 64 (A/A) | align 128 |
|---|---:|---:|
| chameneos | 0.925 (1.002) | 0.919 |
| coroutine_ring | 0.961 (1.002) | 0.971 |
| sieve | 0.944 (1.003) | 0.927 |
| suite (14) | 0.994 (1.001) | 0.993 |

chameneos is now 0.91 of LuaJIT `-joff`.

**Dropped: a `switch_state` vm primitive.** It would save only two
loads that are off the critical path. The rest of a switch (35–40
cycles) is the mispredicted jump and Mini's own bookkeeping, which a
primitive would have to do too.

### 12. Trailing immediate words

Dropped: the ops that check a cache-word index (CALL_INDIRECT_X, bulk
memory ops) never run on the wasm suite, and CONST64 and ADDR read
theirs unchecked.

### 13. JIT stencils: computed jumps to a target

Mini's SWITCH jumped to a JMP that jumps to the arm: one dispatch more
than needed. Decoding the target in SWITCH itself was 4% faster on
interp, but broke under the JIT, which had replaced that JMP's word with
an enter word. Mapping through `Vm.real_word` was then a C call, which
made interp 1.33× slower under the JIT.

As built, `Vm.real_word` is an inline load, and a computed jump in a
stencil goes straight to compiled code when it lands on an enter word.
Both are described in `copy-and-patch.md`. Test:
`tests/run/vm_jit_switch_target.lm`.

Mini's SWITCH decodes the JMP and jumps to its target. When the JMP is
compiled, it jumps to the JMP instead, whose code is one direct `jmp`
(decoding through `real_word` there measured 1.02).

| interp | align 64 (A/A) | align 128 (A/A) |
|---|---:|---:|
| interpreter | 0.977 (0.998) | 0.947 (1.012) |
| JIT (`LOIMI_JIT=1`) | 0.998 (0.991) | 1.002 |

Wasm's BR_TABLE measured the same either way and was left as it was.

### 14. An optimizing tier

Built as ahead-of-time traces (2026-11-06 to 11-09) and removed: hot loops of
profiled programs, compiled at build time from the JIT's stencils, took 0.76–0.87 of
the JIT's time only on the programs trained on (an interpreter's builder does not
know its scripts), at four interpreter builds per test.

## Reference techniques against Loimi

**N**: native; **H**: by hand only; **X**: not expressible.

| Technique | Used by | Loimi |
|---|---|---|
| State in fixed machine registers | LuaJIT, wasm3 | N: `state` under `preserve_none` (`lowering.md` §3) |
| Decode-ahead, replicated dispatch | LuaJIT `ins_NEXT` | N: decode-ahead; goto mode copies the dispatch (`lowering.md` §5) |
| Handler address in the code stream | wasm3 | X (item 2 measured the nearest form) |
| Operand slot as a byte offset | wasm3 | H: an operand-kind expression, if the encoder scales by hand (item 2) |
| NaN tagging, dual int/double | LuaJIT | N: `nanbox`, merged tuple tests; `nanbox(int_high)` (item 7) |
| Top of stack in a register | wasm3 `_r0`/`_fp0` | N: accumulators (item 1) |
| Stack caching with several states (Ertl) | Forth systems | H: Stak's `tos` state variable (10–15%) |
| Inline caches in the code stream | CPython | N: `cache`, `const` fields (`language.md` §7.1) |
| Quickening, families, deopt | CPython PEP 659 | N: `family`, `become`, `Vm.deopt`, `refuse` |
| Warmup and backoff counters | CPython | H: a cache field (item 10: not needed) |
| Static superinstructions | CPython, QuickJS | N: `supers`, `optimize`, `#[forward]`, suggestions (`language.md` §7.3) |
| Dynamic superinstructions (Piumarta & Riccardi) | — | N in effect: `--jit-stencils` copies stencils per instruction |
| Compare fused with the next jump | LuaJIT, Lua 5.4, wasm3 | N: `then OP`, `rel` (`language.md` §7.1a, §7.5) |
| Ops per operand kind | LuaJIT, Lua 5.4, SWI, QuickJS | H: `inline for` over kinds, `rel kform` |
| Lazy compile by self-patching | wasm3 | H: `Vm.Resume` caches, `become` |
| Unboxed typed slots | Guile | N: several operand-kind views of one slot (wasm) |
| Hot counters, tier-up | LuaJIT, Guile | N inside `--jit-stencils` |
| Zero-cost hooks when off | Lua, Guile | N: instrumentation variants swap tables (`language.md` §9) |
| Clause indexing | SWI-Prolog | Interpreter work (Prolog's first-argument indexing) |
| Trace JIT | LuaJIT | Not built (item 14) |

## Other ideas dropped

- **Guard pages instead of wasm bounds checks.** 4 GiB plus a guard per
  memory counts against `ulimit -v`, and the check is already a fused,
  predicted `add; cmp; ja`.
- **Merging wasm's local MOVs.** 0.997 and 0.998.
- **Guest registers in machine registers, in the JIT.** One cached
  register measured 1.015 (`copy-and-patch.md` §13).
- **Stencil hot/cold splitting, constant patching in the JIT.** No gain
  (`copy-and-patch.md` §10, §12).
- **Slow paths through inline-asm `preserve_all` stubs.** 0.995 over 102
  benchmarks, `.text` +8–30%, Stak goto 1.34× slower.
- **Pinning hot constants in tail-call mode.** A rematerialized constant
  is one `movabs` off the critical path.
- **Handler replication for branch prediction** (Ertl & Gregg).
  Tail-call and goto modes already give every handler its own jump.
- **Type-check elimination across ops.** Within a super LLVM already
  merges repeated tag tests; across ops it needs types per slot.
- **Interner layouts** (MiniJS inline key bytes, Mini shrink
  thresholds). Noise or worse.
- **A moving or bump-pointer nursery.** Python 0.982, Prolog 1.01, and
  moving breaks raw pointers in state registers and `Vm.Resume` caches.
- **Direct `call`/`ret` for guest calls.** Call cost is frame stores
  and checks, not mispredicts, and guest recursion would go on the C
  stack.
- **A balanced compare tree for if-chains.** Hot arms come first.
- **A "known `Num`" assume on float constructors.** Mini's interpreter
  1.044, MiniJS nbody 0.986 (A/A 0.990).
- **Retagging Mini's FORLOOP counter.** Interpreter 0.992 (A/A 0.990).

## LLVM, clang and linker features

Measured 2026-11-07. None pays enough to use.

Already in use: `preserve_none` and `musttail` handlers, outlined `cold`
blocks and `llvm.expect`, goto-mode tail-duplication options,
`-enable-tail-merge=false` for switch mode, JCC-erratum branch
alignment, `--align-functions`, `nsw`, `-O3`, `--cpu`, vectorize-width
metadata, and hot-handler ordering.

Each candidate was first a disassembly diff against the baseline; one
that changes no handler stopped there. The rest were timed on Mini (14
programs), Lox (9), Prolog (30) and MiniJS (14). Runs marked *noisy*
had A/A up to 1.05, so they only rule out large effects; *quiet* runs
had A/A within 0.999–1.002.

| Feature | Result (A/A in brackets) | Decision |
|---|---|---|
| `--cpu=x86-64-v3` | Noisy: Mini 0.996, Lox 1.075 [1.039], Prolog 1.016; quiet: MiniJS 1.008 [1.001], Mini JIT 1.000 [1.001] | No; the default stays portable |
| `--cpu=x86-64-v2` | 1.004, 1.000, 1.007 (noisy) | No |
| `-mtune=skylake` | 0 functions differ | No |
| `-mllvm -enable-ipra` | 0 Lox handlers differ; timings inside A/A | No: `preserve_none` tail calls leave it nothing |
| `-falign-loops=32` | 0.995, 0.972, 1.011 (noisy) | No: handlers have no loops |
| `-mllvm -enable-ext-tsp-block-placement` | Mini 1.001, Lox 1.008, Prolog 1.001 (quiet) | No |
| `-static` (no PLT) | 1.002, 1.016, 1.017; binaries 2.4–4× larger | No |
| `-fuse-ld=lld` | 0.991, 0.985, 1.012 (noisy) | No |
| `noalias` on every handler pointer parameter | Mostly slow paths change; Mini 0.977 [0.983], Lox 1.027 [1.002] | No, as item 4 |
| IR PGO, trained on the timed programs | Lox 0.972, MiniJS 0.970, Prolog 0.971–0.978; Mini 1.007 (mandelbrot 1.45) | No |
| IR PGO, held out (trained on half the suite) | Prolog 0.985–0.993, MiniJS 0.980–0.988 | No: under 2% |
| Runtime C linked into the program's IR | The collector and allocation are already IR in the program | Does not apply |
| Symbol ordering file | Hot handlers are already placed by section and profile order | Covered |
| Huge-page text | `.text` is under one 2 MB page | No |
| `!unpredictable` on dispatch | Dispatch is an indirect jump, not a switch | No |
| Machine outliner, `-fno-semantic-interposition` | Off on x86-64; executables are not PIC | No |

PGO was measured without a profile runtime: `pgo-instr-gen`, the
counters lowered by script to a plain array written at exit, then
`pgo-instr-use`. Mini's mandelbrot slowdown with the profile is code
placement only.
