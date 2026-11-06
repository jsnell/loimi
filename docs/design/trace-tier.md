# Design note: a trace tier stitched from stencils (experiment, 2026-11-06)

**Status: M1 built** (§11): `loimi build --jit-stencils
--trace-regions=REC` and `--traces=PROF` make ahead-of-time traces from a
profile, with the generator in the compiler (`src/tracegen.rs`). **Then
cheaper exits and traces through calls** (§12, M4 in part): Mini at 0.858
of the JIT by geometric mean (M1: 0.932), no program slower. §1–§10
describe the experiment as it was run; its hooks were replaced
(`LOIMI_TRACE_DUMP` is `LOIMI_TRACE_RECORD`, `LOIMI_TRACE_IR` is
`--traces`, `loimi_trace_blacklist` is `loimi_trace_failing`).

**Question.** Is there a "moral equivalent of a tracing JIT" for
Loimi's interpreters? That means compiling the common path through a
hot loop as one unit, with automatic fallback to the interpreter when a
guard fails. The strongest form is meta-tracing or partial evaluation
(PyPy's RPython, Truffle/Graal). The framework records a hot path
through the *interpreter* and compiles the handlers' code for that path
as one unit:

- bytecode operands become constants;
- every branch the trace did not take becomes a guard that exits to the
  interpreter;
- LLVM optimizes the whole unit.

`copy-and-patch.md` §13 measured the prize by hand. With guest
registers in host registers and the checks removed, mandelbrot's inner
loop runs at 0.56–0.68 of the copy-and-patch JIT's time. That section
ended: "start an optimizing tier over traces or loops, not more stencil
variants". Speed audit item 14 says the same.

**Answer: yes, and Loimi gets most of it almost for free.** This
experiment built the trace offline, with no runtime compiler. It
recorded a loop in the running program, generated one LLVM function
from the vm's own stencils, linked it back into the program, and
entered it at the loop header. LLVM did the rest. The results:

| traced loop | trace / JIT | trace / interpreter |
|---|---:|---:|
| mandelbrot's inner loop | **0.63** | 0.36 |
| nbody's inner loop (calls `math.sqrt`) | **0.73** | 0.55 |
| fannkuch's swap loop | 0.82 | 0.64 |
| sieve's two loops (memory-bound) | 0.87 | 0.70 |

These are whole-program times: minimum of 11 interleaved runs, user+sys
CPU, `--align-functions=64`; medians agree within 0.07 (§3). On
mandelbrot the stitched trace lands inside the hand-written bound
(0.56–0.68), although it keeps every NaN and tag check. Values stay in
xmm registers across the whole loop, which has 53 instructions and no
memory access. Stores to the register file are sunk to the exits.
Operands, constants and `pc` are folded, and dispatch is gone.

Every stitching step was mechanical: register-file promotion, guards,
exits, entry, and the inlining of an outlined cold block (nbody's
`sqrt`). The manual parts were choosing which loop to trace in three
of the five programs (nbody, sieve, fannkuch), and a build–run–build
cycle that a runtime tier would do by
itself (§5).

**Recommendation: go**, staged (§8). Build the first runtime tier on the
same idea: stitch handler IR along a recorded path, and keep the
shared-frame exits. Do not write a new optimizer.

## 1. What was built

```
 run with LOIMI_TRACE_DUMP   ->  recording: code words (quickened), ops, patched constants, hot loop pc
 tracegen.py --mode region   ->  the loop's instructions as one function, every block counted
 run with LOIMI_TRACE_PROFILE->  block and edge counts
 tracegen.py --mode trace    ->  the hottest path; untaken branches become guards
 loimi build (LOIMI_TRACE_IR)->  the trace appended to the program's IR, compiled with it (-O3)
 run (LOIMI_JIT=1)           ->  the JIT runtime points the loop header at the trace
```

Two flag-gated hooks are in master (this commit):

- **`src/jit_rt.c`.** With `LOIMI_TRACE_DUMP=file`, each compile writes
  the code object's words, the op of each instruction, the constants
  patched into its stencil, and the loop that triggered the compile
  (the recording). A weak table, `loimi_traces`, lists generated traces.
  Each entry gives a code object's length, a hash of the instruction
  words in the trace's range, and the loop header. After a compile of a
  matching object, the header's home gets a 5-byte `jmp` to the trace,
  and its enter word's entry points there too, so both compiled code
  and the interpreter enter it. With `LOIMI_TRACE=0` the hook installs
  nothing (the A/B switch). `LOIMI_TRACE_PROFILE=file` writes the
  trace's block counters at exit. `loimi_trace_blacklist(name)`, called
  by a trace whose guards have failed too often, restores the header's
  five bytes and its entry, and bans the trace.
- **`src/main.rs`.** With `--jit-stencils` and `LOIMI_TRACE_IR=file`,
  the file's IR is appended to the program's.

The generator (`tracegen.py`, about 850 lines of Python) and the driver
scripts are not committed; they are listed in §10. The generator reads:

- the stencil module (`LOIMI_KEEP_STENCILS`): every op handler, already
  in the form the JIT needs, with operand fields, words and constants as
  holes and `next`/label jumps as calls to `__jit.n.D`/`__jit.j.*`;
- the program IR, for the names it defines;
- the recording.

For each instruction (a *step*) of the trace, it copies the stencil's
body, renaming its values and blocks, and then:

- **Holes become literals.** Fields are decoded from the recorded word,
  and constants come from the recorded pool. A constant that is a heap
  pointer (a field name in nbody) is loaded from `k[i]` instead, since
  its address changes between runs.
- **Continuations.** A `next` or label jump to the trace's next step
  becomes a `br` to that step's head, with phis for `ctx`, `k` and `fp`.
  `pc` is `trace_pc + offset`, a constant, at every step. Any other
  continuation is an exit: it dispatches through the table at the
  target pc.
- **The register window.** `regs` becomes a local `[N x i64]` alloca
  (N: the largest register field + 4). It is copied in at entry, and
  written back before every exit and before every call that may see the
  register file or collect (anything but LLVM intrinsics and functions
  with no stores and no calls). It is reloaded after such a call.
  Pointers into the window passed to calls are rebased onto the real
  file. SROA turns the window into SSA values; the copy-in becomes loads
  at entry, and the writebacks become stores in the exit blocks.
- **Changes of `regs`.** The handler may switch to another frame
  (`CALL`), reload `regs = fp.base` after a native, or store a
  register-file pointer into a frame. Each case is handled at run time:
  - the real file maps back to the window;
  - a pointer into the window is rebased onto the real file, after a
    writeback;
  - any other frame makes the code continue on real memory, and exits
    pass it on.

  LLVM folds all of this away on the paths where `regs` does not change.
- **Outlined cold blocks are inlined once.** `Mini.CALL` handles
  natives in a `cold` block, an outlined function with the handler's
  signature, so `math.sqrt` in nbody is "cold". The generator inlines
  such calls. Their `next` is a dispatch through the table; it becomes a
  `switch` on `pc - trace_pc` that branches to the matching step.
- **Guards.** In trace mode, an on-trace block whose branch goes to a
  block the profile never ran gets a *guard*, if no effect lies on any
  on-trace path to it. The guard writes back the window as it was at the
  step's start, then dispatches at the step's pc, so the interpreter
  re-executes the whole instruction. Effects are stores outside the
  step's locals and the register file, atomics, and unsafe calls. This
  is the condition the `redo_effect` check already enforces for `redo`
  (`src/redocheck.rs`, `lowering.md` §10). Where an effect came first,
  the untaken code stays in the trace, but all its continuations exit,
  so it never merges back into the trace.
- **Peeling** (`--peel`, optional): the trace is emitted twice, as a
  preamble and then the loop, as LuaJIT's LOOP pass does.

The *region* mode is the profiling build. It covers every instruction
between the loop header and the last back edge, with all internal edges
and a counter per block and per dynamic edge. It is also the "method
JIT" baseline: the loop compiled as a whole, without pruning.

## 2. Correctness

- Every timed run printed the benchmark's expected output (the bench
  harness compares each run).
- **A failing guard.** `progs/mandg.lua` is mandelbrot with
  `if y >= SWITCH then ci = 0 end`. It was recorded and profiled with
  `SWITCH = 2000` (never true). The trace was then run on `mandg650.lua`,
  identical except `SWITCH = 650`: from row 650 on, `zi = 2.0*zr*zi + ci`
  adds an integer. The output matched the interpreter's (inside 1075833,
  checksum 288560). The guard in `MUL_F_ADD_F__f2` (the `(Num, Int)`
  arm, which the profile never saw) fired 38,062,700 times, once per
  iteration (in a build with counters and without blacklisting): the
  interpreter re-executes the super, the loop jumps back to the header,
  and the trace is entered again.
- **Blacklisting.** With no side traces, such a trace costs more than
  the JIT: mandg650 ran at 1.08 of the JIT's time. Each guard exit now
  counts its failures in the trace. At 1000 failures it calls
  `loimi_trace_blacklist`, which puts the stencil code back at the
  header and keeps the trace from being installed again. In that run,
  `LOIMI_JIT_DEBUG` showed `trace mgtr blacklisted`, the guard counter
  stopped at 1000, and the output was still right. mandg650 then ran at
  0.87 of the JIT's time: the trace for the first 650 rows, stencils
  after.
- The trace is installed only when its words match (a hash of the
  instruction words in its range). Rewrites through `become`/`deopt`
  recompile the code object, and the hook then re-checks. A family
  member rewritten *after* installation leaves the trace running the
  old member's code. That is correct for Mini, because every member
  handles every type (`vm.lm`: "both members handle every type"), but a
  real tier must invalidate (§7).

Not tested: GC stress (`mini main.lm script stress`) with a trace, the
golden suites with traces, and coroutines. The traced loops do not
allocate. The writeback-around-calls rule is what makes collection safe
(§7.1), but it was exercised only by nbody's off-trace paths.

## 3. Results

Machine: the shared 8-core x86-64 box, with two other agents running.
Load average 3.9–5.3. User+sys CPU seconds, minimum of 11 runs
interleaved per benchmark (7 for the guard programs), with the median
also shown. `--align-functions=64` throughout.

- *interp*: Mini's default build.
- *JIT*: `--jit-stencils` with `LOIMI_JIT=1`.
- *JIT A/A*: the same binary and settings again, as a noise control.
- *trace*: the same binary, with the trace hook on.

The JIT and the trace come from one binary, switched by `LOIMI_TRACE=0`.
Every run's output was checked.

| program (traced loop) | interp | JIT | JIT A/A | trace | trace/JIT | trace/JIT, medians |
|---|---:|---:|---:|---:|---:|---:|
| mandelbrot (inner `for i`) | 0.662 | 0.373 | 0.370 | 0.235 | **0.63** | 0.63 |
| nbody (inner `for j`, with `math.sqrt`) | 0.463 | 0.347 | 0.348 | 0.255 | **0.73** | 0.80 |
| sieve (marking loop and the `for i` loop, two traces) | 0.727 | 0.581 | 0.578 | 0.506 | 0.87 | 0.88 |
| fannkuch (the swap `repeat`, 1–4 iterations per entry) | 1.185 | 0.918 | 0.911 | 0.757 | 0.82 | 0.82 |
| mandg (mandelbrot with a dead `if`) | 0.648 | 0.377 | | 0.232 | 0.62 | 0.64 |
| mandg650 (guard fails from row 650) | 1.060 | 0.635 | | 0.552 | 0.87 | 0.89 |
| mandg650, without blacklisting | | | | 0.687 | 1.08 | 1.11 |

The A/A pairs differ by under 1%. An earlier batch under heavier load
(4.6–5.9) gave mandelbrot 0.54 and nbody 0.73, with a 2× spread
between runs. The table above is the steadier batch.

Variants of the mandelbrot trace (same batch):

| variant | seconds | / JIT |
|---|---:|---:|
| region: the loop's instructions as one function, all paths, cold blocks inlined, no pruning | 0.356 | 0.95 |
| trace, no guards: untaken code kept, its continuations exit | 0.304 | 0.82 |
| trace with guards | 0.235 | 0.63 |
| trace with guards, peeled | 0.238 | 0.64 |

Pruning is what pays. A loop compiled whole (the "method JIT" shape)
gains almost nothing over the stencils: the slow paths and inlined cold
blocks merge back into every step, and LLVM must keep their values
alive. Peeling removes one tag test per iteration but no time.

## 4. What LLVM did, and what it could not do

The hot loop of the mandelbrot trace is 53 instructions per iteration.
The stencils need about 9 per float op plus `JGTKN` and `FORLOOP`, about
100 in all (`copy-and-patch.md` §7).

```
loop:                                        ; zi = 2*zr*zi + ci; zr = zrq - ziq + cr; ...
  movapd %xmm0,%xmm10 ; addsd %xmm8,%xmm10   ; ADD_F
  ucomisd %xmm10,%xmm10 ; jp guard           ;   its NaN test (r == r)
  movapd %xmm6,%xmm7 ; subsd %xmm3,%xmm7     ; SUB_F ...
  ...                                        ; 9 float ops, 7 NaN tests
  movq %xmm5,%r11 ; cmp %rbx,%r11 ; jae x    ; JGTKN: not an Int (both tag tests in one compare)
  ucomisd %xmm12,%xmm5 ; ja escape           ;   zrq + ziq > 4.0 (the constant from .rodata)
  mov %rax,%rbp ; and %rsi,%rbp ; cmp %r8,%rbp ; jne x   ; FORLOOP: counter is an Int
  test %eax,%eax ; je done                   ;   count left
  ... and/cmp/jne                            ;   step is an Int
  dec %rax ; add %r9d,%ecx ; or %r8,%rcx     ;   count down, i += s, retag
  ... 4 movapd (phi copies) ; jnp loop
```

**What fired** (LLVM 19, `-O3`, no extra passes or hints):

- **SROA/mem2reg of the register window.** There is no load or store to
  `regs` in the loop. The writebacks are in the exit blocks, so stores
  are sunk to exits as a side effect of SSA construction. No LICM
  promotion was needed.
- **Constant folding of operands.** Field decoding, `k[c]` constants
  (`2.0`, `4.0`) and every `pc` computation folded. The hot loop has no
  `pc` arithmetic and no dispatch.
- **Instcombine and GVN on the tag tests.** `JGTKN`'s two tuple tests
  (both Int, then both Num) merged into one unsigned compare against
  the NaN-box boundary. The constant operand's tests vanished.
- **Loop rotation.** The first iteration's loads from the window are
  outside the loop.
- In nbody: the inlined cold `sqrt` path became one `sqrtsd`, after
  the native-id and tag tests. Inline-cache words are loaded from the
  code stream on every iteration (see below).

**What did not fire, and why:**

- **The NaN test after each float op (7 per iteration).** It is
  semantic, not a type test. `inf - inf` and `0 * inf` give NaN, which
  must be canonicalized, so no compiler can drop it. LuaJIT does not
  pay for it only because its NaN encoding needs no canonicalization
  after arithmetic. This is a cost of Mini's value representation, not
  of the tier.
- **JGTKN's tag test on `zrq + ziq`.** The value comes from `ADD_F`'s
  fast path, which already checked `r == r`, so it cannot be a boxed
  value. LLVM does not derive bit facts from `fcmp ord`. A Loimi-level
  fix: when a handler builds a `V` from an `f64` known not to be NaN
  (`dst = r.to_bits() as V` after `likely(r == r)`), codegen could emit
  `llvm.assume(bits < 0xfff8000000000000)`, i.e. "this is a `Num`".
  That is a change to how values lower, and it helps the interpreter
  too.
- **FORLOOP's tag tests (counter and step).** These are loop-carried.
  The phi merges the entry value (unknown, from memory) with the
  back-edge value (`(i + s) | 0xfff9 << 48`, whose tag *is* known).
  Peeling one iteration removes the test on the step and index. The
  counter test stays, because Mini decrements the raw word
  (`((regs[a + 1] as u64) - 1) as V`, "the payload is not zero, so
  decrementing the word decrements it"). LLVM cannot prove that the tag
  survives the subtraction. Writing `Int(c - 1)` would make it
  provable. "Retag, don't do raw arithmetic on boxed words" is a vm
  style note that costs the interpreter nothing.
- **Inline caches are not promoted.** nbody's `GETFIELD` reads its
  cached shape and slot from the code stream on every iteration. They
  are plain memory that any handler may write, so LLVM must reload them
  after every store to the heap. A tier needs RPython's `promote`:
  read the cache at record time, bake it in, and guard it. The guard is
  the same shape compare the handler already does, but against a
  constant. The slot then becomes an immediate.
- **Calls into Mini functions are not traced.** `CALL` of a closure
  pushes a frame and jumps to the callee. The trace exits there, with
  the window written back. Inlining calls into a trace needs frames that
  exist only on exit (LuaJIT's snapshots). That is the one piece of real
  deoptimization machinery the shared-frame design does not give for
  free.
- **Aliasing of the register file was never the problem.** Once `regs`
  is a local alloca, nothing aliases it. In the region build (no
  pruning) it was promoted as well. Pruning matters because untaken
  paths merge back: the `noguard` variant keeps the untaken code with
  exits only at continuations, and is 0.82 of the JIT where the guarded
  trace is 0.63.
- **GC polls** cost nothing in these loops: Mini's `FORLOOP` polls only
  `gil` and `pause`, which are empty without their cfgs. A gc poll in a
  traced loop would be a load, a compare and a branch to an exit or to
  an inlined collect with writeback and reload (§7.1).

**Compile latency.** Measured with `llvm-extract` of the trace function
alone, then `clang -c`, best of 3. Clang on an empty module takes
0.038 s, which is included below. Callees are declarations here, so
LLVM inlines less than in the program build.

| function | IR lines | `-O1` | `-O2` | `-O3` | code |
|---|---:|---:|---:|---:|---:|
| mandelbrot trace | 4,407 | 0.063 s | 0.072 s | 0.082 s | 5.6–7.2 KB |
| nbody trace | 19,513 | 0.24 s | 0.42 s | 0.41 s | 22 KB |
| mandelbrot region (no pruning) | 17,438 | 0.17 s | 0.26 s | 0.25 s | 16 KB |

That is 30–400 ms of real work per trace. It is far too slow for a JIT that compiles at the first loop (the
stencil JIT takes 7–30 µs per function), but acceptable for a tier
entered after millions of iterations. LuaJIT compiles a trace in tens of
microseconds; ORC with `-O2` on these functions would sit in between.

## 5. Automatic versus manual

| step | here |
|---|---|
| finding the hot loop | automatic: the JIT's hot counter names the loop header that triggered the compile (mandelbrot, mandg). Manual for nbody, sieve and fannkuch: the last trigger was a function entry or the wrong loop, so `--code`/`--header` were given by hand. |
| recording operands, quickened ops, constants | automatic (`LOIMI_TRACE_DUMP`) |
| choosing the path | automatic: block and edge counts of the region build, then the hottest successor at each step |
| stitching, holes, continuations, register window, writeback, calls, `regs` changes, cold-block inlining | automatic (the generator knows the stencil conventions, not Mini) |
| guards | automatic, from the profile and an effect analysis |
| linking and entry | automatic, through the JIT runtime hook |
| the cycle build, profile run, build | a script; a runtime tier does it in process |
| a failing guard | the input `mandg650.lua` was made by hand |

Mini-specific knowledge in the generator: the dispatch table's name
(`vm.Mini.table`), the stencil hole naming, and the NaN-box tag range
used to spot heap constants. All of it is available from the vm
declaration.

## 6. The gap to the bound

The hand-written bound (`copy-and-patch.md` §13) ran the loop alone on
a Skylake. These are whole-program times on a different, loaded
machine. Mandelbrot spends about 97% of its samples in this loop under
the JIT, so the two are close to comparable.

| | of the JIT's time |
|---|---:|
| guest registers in host registers, spilled at exits (bound) | 0.68 |
| registers, no type checks, `pc` updates or jumps (bound) | 0.56 |
| **stitched trace, mechanical** | **0.63** (0.54 in the noisier batch) |

The trace beats the bound's register-allocated row (0.68), which kept
the stencils' checks, `pc` updates and jumps. It falls short of the
best row (0.56) by about the cost of the checks it still does: 7 NaN
tests and 3 tag tests per iteration. The loop is a chain of dependent
float operations, so those tests partly hide in the shadow of the
`mulsd`/`addsd` latency. The two tag-test fixes in §4 (the `assume` and
the retagged counter) would close part of that gap. The NaN tests stay.
The remaining 1.5× to LuaJIT (`copy-and-patch.md` §6: 0.159 s against
the JIT's 0.410 s there) is the NaN canonicalization, the remaining
tests, and the 4 `movapd` phi copies per iteration that LLVM's register
allocator left in the rotated loop.

nbody's gap is the untraced outer loop (18% of samples in `GETFIELD`
and `SETFIELD` stencils of the `i` loop), inline caches read from
memory, and the field stores with their write-barrier tests.

## 7. Sketch of a real tier

A runtime tier on top of `--jit-stencils`, reusing everything this
experiment used:

1. **Trace selection.** The hot loop counters exist already (the
   JIT's `hotloop`). Loop headers are the targets of backward `label`
   operands; the vm declaration already marks label kinds, so no
   `jit_merge_point` hint is needed. A trace that reaches the header of
   an inner loop with its own trace links to it instead of failing
   (fannkuch's outer loop ran into this: "inner cycle at 40").
2. **Recording.** An instrumentation variant (`language.md`,
   instrumentation tables) whose handlers append their branch
   directions to a buffer for one or a few iterations. This is
   meta-tracing of the handler IR, which is exactly what the block
   counters gave here. The compiler numbers handler blocks once, for the
   variant and for the trace templates alike.
3. **Templates.** Per handler, the compiler ships its body as LLVM
   bitcode (the stencil module, in effect) with the hole conventions
   used here, plus per block an *effect-free-so-far* bit from the
   `redo_effect` analysis. That bit is what decides guard placement.
4. **Backend.** Stitch the bitcode at run time and run an `-O2` pipeline
   through LLVM ORC. That is the shortest path, and it keeps the property
   that the optimizer is not ours. The costs: linking LLVM (tens of MB)
   into every interpreter that enables the tier, and 10–100 ms per
   trace (§4). Option: compile traces on a background thread and patch
   the header's home when they are ready, as the JIT patches enter words
   today. A lighter backend (LuaJIT-style SSA plus a linear-scan
   allocator) is a far larger project: the instruction selection that
   LLVM does here for free would have to be rewritten for x86-64 and
   AArch64.
5. **Exits and deopt metadata.** No frame reconstruction, as long as
   traces do not inline calls. An exit is:
   - a writeback of the registers the trace holds (SSA values, known
     at compile time);
   - then a tail call: to the dispatch table at a pc (re-execution), or
     to the outlined cold function (the handler's own continuation).

   The metadata is the writeback set, which LLVM produces as code.
   Inlining calls adds frame snapshots: the callee frame is
   materialized only on exit.
6. **Side traces.** Count guard failures per guard. Past a threshold,
   record from the exit's pc and patch the exit's jump to the new trace.
   `mandg650` shows the cost without them: 1.08 of the JIT, or 0.87
   with the prototype's blacklisting (§2, §3). The blacklist stays as
   the backstop for traces that cannot be repaired.
7. **Invalidation.** A trace depends on the words in its range and, with
   `promote`, on inline-cache contents. Word rewrites already go through
   the JIT runtime (`rewrite`, `recompile`), so they can drop the traces
   that cover them. Cache writes are plain stores in handlers. Promoted
   caches need a guard per use (as here) or a write hook.
8. **Hints** worth adding to the language, none required for a first
   version:
   - `promote(e)`: treat `e` as a constant at record time and guard
     it, for inline caches and shapes;
   - "known `Num`" facts from value constructors (an `assume`);
   - a lint for raw arithmetic on boxed words.

### 7.1 GC: polls and roots in registers

- **Polls stay.** At a poll that may collect (`ctx.heap.allocated >
  threshold`), the trace exits, or calls the collector inline with a
  writeback before and a reload after, as the generator does for every
  unsafe call. Mini's collector scans frames through `ctx.fp` and
  `regs`, so the written-back window is the root set.
- **Handler-local temporaries.** These are values in a handler's own
  locals across a `uses gc` call, already rooted by the shadow-stack
  frames codegen emits per call site (`lowering.md` §9). Stitching
  copies those frames with the code.
- **The heap is non-moving.** Values held in SSA across a collection
  stay valid once rooted. A moving nursery would need every register
  and interior pointer (`aptr`) re-derived after each poll, which is one
  more reason to keep the non-moving design.
- **Write barriers.** They are in the handler code and come along
  (nbody's `SETFIELD`).

### 7.2 The lifetimes checker

Stitching works on IR, after checking, so the lifetimes rules
(`lifetimes.md`) are not in play: LLVM's own dependence analysis keeps
loads after the stores they depend on across steps. The risk is
metadata. If handlers start carrying per-handler aliasing facts
(`!invariant.load`, `noalias` scopes, speed audit item 4), those facts
are valid within one handler instance, so a stitcher must rename scopes
per step and drop `invariant.load` on memory that another step may
write. The stencils carry none today.

### 7.3 `redo`, `redo_effect` and polls

`redo` in a step is a continuation to the step's own pc, so it is an
exit, which is correct. The `redo_effect` analysis is the guard
analysis: a guard may re-execute an instruction exactly where the
handler could have `redo`ne. The generator computed this on IR. A real
tier should take it from `redocheck.rs`, which also exempts what IR
cannot see: `become`, cache stores and allocation of unreferenced
objects.

### 7.4 Accumulators (`src/vmacc.rs`, in progress)

An accumulator is a state register. In a trace it is one more SSA
value: the generator must pass it at exits like `ctx`/`k`/`fp`, and
pick the accumulator variant of a handler where the recording ran one.
Inside traces, register-window promotion subsumes what an accumulator
buys (every register is in a host register), so the two are
complementary: accumulators speed up the interpreter and the
copy-and-patch tier, and traces do not need them. Accumulator variants
multiply the templates to ship, but not the work per trace.

### 7.5 The debugger

The debugger switches tables (`use_variant`) and rewrites words for
breakpoints.

- **Breakpoints** are word rewrites, so they invalidate the traces that
  cover them (§7).
- **Pause requests.** The `pause` poll must stay in traces in
  `--cfg debug` builds. A set flag is an exit at an instruction
  boundary.
- **Stepping** never needs to see a trace: exits are at instruction
  boundaries or in handler cold paths, and the shared frame means the
  debugger sees the same state as in the interpreter.

### 7.6 Dispatch modes

The prototype needs tail-call dispatch: a trace has a handler's
signature, it is entered by a jump, and it leaves by tail calls, as
stencils do. `goto` and `switch` modes have one loop function, so a
trace would be called from the dispatch loop and return the exit pc,
after the writeback. That is possible, but it costs a call per entry.
The JIT is tail-call only today, and the tier would be too.

## 8. Costs, risks and recommendation

**Costs:**
- LLVM at run time (ORC), or an offline tier only (as here);
- 10–100 ms per trace;
- shipping handler bitcode with the program, about the stencil
  module's size;
- the recording variant;
- trace linking and side traces;
- invalidation;
- one more axis in the test matrix: GC stress, coroutines and the
  debugger, each with traces.

**Risks:**
- **Calls.** Traces stop at Mini calls. fib, spectral_norm, oo_methods
  and closures gain nothing until calls are inlined, and that needs
  snapshots.
- **Short trip counts.** fannkuch's swap loop runs 1–4 iterations per
  entry, but still gained (0.82). The writeback cost per exit is
  proportional to the window (NR slots), not to what changed. Computing
  the dirty set per exit is easy at the template level.
- **Code growth.** In the program build the mandelbrot trace is 5.8 KB
  (1.4 KB before cold blocks were inlined) and nbody's 26 KB. The
  unpruned region for mandelbrot is 16 KB. Most of it is exit code: one
  writeback of the whole window per exit. The hot loop is about 200
  bytes.
- **Correctness surface.** Exits, regs changes and calls were all
  handled mechanically here, but they were tested only on these loops.

**Recommendation: go**, in stages, each with a stop rule:

1. **M1: AOT traces from a profile (1–2 weeks).** Turn the generator
   into a compiler mode: `loimi build --trace-profile=file` reads a
   recording and emits traces into the binary. Do the stitching in Rust
   on the stencil IR. Run all Mini golden tests, GC stress and
   conformance with traces of every hot loop. *Measure the 14 Mini
   benchmarks.* Stop if the geometric mean gain over the JIT is under
   10% (expected: mandelbrot, nbody and spectral_norm's inner loops
   carry it).
2. **M2: tighter traces (1 week).**
   - the `Num` assume;
   - retagged FORLOOP;
   - `promote` for inline caches;
   - dirty-set writeback;
   - linking to inner-loop traces.

   Stop rule: nbody < 0.65 of the JIT.
3. **M3: a runtime tier (3–4 weeks).** Recording variant, ORC backend
   on a background thread, side traces, invalidation, the debugger's
   pause poll. Stop if compile latency or binary size is unacceptable
   for the embedding use case. Then the AOT form (M1) remains, which
   already serves benchmark-style and profile-guided builds.
4. **M4: calls (open).** Inlining Mini calls with snapshot frames. This
   is the only part that needs real deoptimization metadata, and the
   only route to LuaJIT on fib and spectral_norm.

Do not build more stencil variants, and do not build a separate
optimizer. Stitched handler IR plus LLVM is already within the
hand-written bound.

## 9. Things learned about Loimi itself

- Stencils are a good intermediate form for more than copy-and-patch.
  They hold each handler's semantics with every operand, constant and
  continuation named, so a tool that knows nothing about Mini could
  stitch them.
- Outlined `cold` blocks with the handler's signature double as
  ready-made exit points *and* as inlinable continuations.
- Shared frames make exits cheap: write back the window, then dispatch.
  No exit in this experiment needed state reconstruction.
- Value representation shows through once dispatch is gone: in
  mandelbrot's trace, the remaining cost is NaN canonicalization and
  tag tests on loop-carried raw words.

## 10. Reproducing

The scripts are in the session scratch directory `trace-exp-r7q3/` (not
committed): `tracegen.py` (the generator), `merge.py` (several traces in
one file), `gen.sh`, `genall.sh`, `buildmerged.sh`, `guard.sh`,
`guard2.sh`, `bench.py` (interleaved runs, user+sys CPU, output check),
`myprof.py` and `annot.py` (SIGPROF sampler, by symbol and by
instruction), `ctime.sh`, `measure.sh`. Outline:

```sh
# a --jit-stencils build that keeps its stencil module and IR
LOIMI_KEEP_STENCILS=$S/keep loimi build examples/mini/main.lm --release --align-functions=64 \
    --jit-stencils --keep-ll -o $S/mini-jit
# record
LOIMI_JIT=1 LOIMI_TRACE_DUMP=$S/mandelbrot.dump $S/mini-jit bench/mini/programs/mandelbrot.lua
# region build, profile, trace build
tracegen.py --stencils $S/keep/main.stencils.ll --main $S/mini-jit.ll --dump $S/mandelbrot.dump \
    --mode region --name mandreg --out $S/mandreg.ll --mapout $S/mandreg.json
LOIMI_TRACE_IR=$S/mandreg.ll loimi build ... --jit-stencils -o $S/mini-mandreg
LOIMI_JIT=1 LOIMI_TRACE_PROFILE=$S/mandreg.prof $S/mini-mandreg bench/mini/programs/mandelbrot.lua
tracegen.py ... --mode trace --profile $S/mandreg.prof --map $S/mandreg.json --name mandtr --out $S/mandtr.ll
LOIMI_TRACE_IR=$S/mandtr.ll loimi build ... --jit-stencils -o $S/mini-mandtr
# A/B: LOIMI_TRACE=0 turns the trace off in the same binary
LOIMI_JIT=1 $S/mini-mandtr bench/mini/programs/mandelbrot.lua
```

nbody: `--code 0 --header 23`; sieve: `--header 32` and `--header 23`,
merged; fannkuch: `--header 40`.

## 11. M1 as built: ahead-of-time traces from a profile

M1 turned the experiment into a compiler mode. The generator is
`src/tracegen.rs`, in Rust and in the compiler; the hooks in
`src/jit_rt.c` became a supported runtime; `tools/trace-exp/` stays as
the experiment's record.

### 11.1 Workflow

Any vm that builds with `--jit-stencils` (one tail-call vm, x86-64
Linux) can use it. Three steps, each a plain build or run:

```sh
# 1. record: any JIT build, run on the workload
loimi build vm.lm --release --jit-stencils -o vm-jit
LOIMI_JIT=1 LOIMI_TRACE_RECORD=prog.rec ./vm-jit prog
# 2. profile: every loop of the recording as a counted region
loimi build vm.lm --release --jit-stencils --trace-regions=prog.rec -o vm-prof
LOIMI_JIT=1 LOIMI_TRACE_PROFILE=prog.prof ./vm-prof prog
# 3. traces of the loops that ran often enough
loimi build vm.lm --release --jit-stencils --traces=prog.prof -o vm-fast
LOIMI_JIT=1 ./vm-fast prog
```

- **The recording** (`LOIMI_TRACE_RECORD`) lists each compiled code
  object: its real words, the op of each instruction, and the constants
  its stencils patch in. It is written at each compile and again at
  exit, so quickening that retargeted instead of recompiling is in it
  (the last entry per code object wins).
- **The profile** (`LOIMI_TRACE_PROFILE`) is self-contained: per region
  that ran, its fingerprint, the code's recording and the block counts.
  It names the build it came from (a hash of the stencils); a profile
  of another build of the vm or compiler is an error.
- Several recordings or profiles can be given (`--traces=a,b`), or
  concatenated: a region profiled by several runs sums its counts.
- Runtime switches: `LOIMI_TRACE=0` installs nothing (the A/B switch in
  one binary); `LOIMI_TRACE_FAILS=n` sets how many guard failures pass
  between blacklist checks; `LOIMI_JIT_STATS=1` adds a line per trace
  (iterations, guard failures, invalidations).
- Build-time knobs (environment): `LOIMI_TRACE_REPORT=1` lists the
  regions and traces made and why loops got none (`=2` also lists the
  guards); `LOIMI_TRACE_MIN=n` (default 1000) is the number of times the
  hot path must have closed; `LOIMI_TRACE_MAXLINES=n` (default 150,000)
  skips loops whose function would be larger; `LOIMI_TRACE_MINFRAC=x`
  (default 0) takes blocks run in under that fraction of their
  instruction's runs off the trace (testing: guards that fail);
  `LOIMI_TRACE_CHECK=1` checks every access the generator could not
  place (below); `LOIMI_TRACE_ONLY` / `LOIMI_TRACE_SKIP` restrict
  regions to some loops or leave instructions out (bisection);
  `LOIMI_TRACE_DEBUG=1` annotates accesses in the IR.

**Why three steps.** Branch directions inside handlers are only known
by running the handlers' blocks, and the stencils are machine code
without counters. The middle build compiles each loop as a region whose
every block counts, which is the experiment's method, made automatic.
A one-build alternative (counters in the stencils, with a hole for a
per-instruction counter base) would also need counters in the
interpreter's outlined cold blocks, which the stencils call; it is left
for M3, where a recording variant of the handlers replaces both.

**Why Rust.** The generator works on the compiler's own unoptimized
handler IR, which the compiler already has in memory (the program's IR
and the stencils), in a format it controls. In the compiler it needs no
files, no Python, and no `--keep-ll`; it knows the vm's state layout
(which arguments are `pc`, the register file, the constant pool, the
accumulators) and the encodings of immediate constants from the vm
declaration instead of Mini's NaN-box ranges. It parses IR lines with a
small hand-written parser (no regex crate), about 2,400 lines.

### 11.2 Loop selection

Loops are found in the code, not from the JIT's hot counter (which named
the wrong loop for nbody, sieve and fannkuch in the experiment): every
target of a backward continuation of a stencil (`next` or a label jump
with `target <= pc`) is a header, its loop runs to its last back edge.
A region is that range without the ranges of inner loops, whose headers
are exits (they have regions of their own). The trace is the hottest
path from the header by the region's edge counts; a loop gets a trace if
that path closes at the header at least `LOIMI_TRACE_MIN` times. A path
that leaves the loop first (an outer loop whose hot path goes into an
inner one) gets none: linking to inner traces is M2.

### 11.3 The register window

The experiment replaced `regs` with a local array everywhere. That was
wrong in ways its four loops did not show: a handler that compares
`regs + a` with a pointer from memory (Mini's `CLOSE`) compared an
address on the stack; after a native, `regs = fp.base` switched the
code to memory with the window still dirty; dynamic offsets
(`SETLIST`, call arguments) indexed past the array; and handlers pass
the address of their `regs` variable to vm functions (`for_count`).
M1 keeps `regs` the real register file and treats the window as a cache
of `regs[0..NR)`:

- A per-step analysis computes constants (fields, words, pool
  constants) and register-file offsets over the handler's locals,
  flow-sensitively (locals passed by reference to a call are reset by
  it; a local that ever holds `regs` holds a register-file pointer
  whatever is stored in it). A load or store at a constant offset in
  the window uses the window slot; other constant offsets use memory;
  unknown offsets choose at run time. `NR` is the largest constant slot
  any step accesses, plus one.
- Each block is in *window mode* (the window is the truth) or *real
  mode*. A call that may write memory (not a pure intrinsic, a
  read-only function, or one of the runtime's collector, lock or
  `become`) is preceded by a writeback and starts real mode: memory is
  the truth until the step's end, where the trace continues only if
  `regs` is still its register file and then reloads the window. A
  window-mode edge into a real-mode block writes back on the edge.
  Read-only calls given a pointer that reaches the register file (one
  into it, or to `ctx`, a frame, or a local holding one of those) are
  preceded by a writeback; read-only calls given heap objects are not
  (nbody's write-barrier tests, three per iteration, cost it 15% when
  they were). Read-only is decided before the collector's exemption. So the collector, natives and nested runs see
  the interpreter's state, and a stack that moves during a call (Mini's
  coroutine stacks grow) is never written back to.
- Assumptions: within a step, the stitched code reaches the window's
  slots only through `regs`, and functions reach the running frame's
  registers only through `regs`, the context or a frame (not through
  heap objects); other paths to the register file go through calls that
  may write, which are preceded by a writeback. `LOIMI_TRACE_CHECK=1` checks every access through a pointer the
  analysis cannot place (`loimi_trace_check` aborts if it is in the
  window); the Mini suite passes with it (§11.6).
- In traces, writebacks, reloads and guard snapshots copy the slots
  some emitted step names, one by one; SROA promotes each slot. A
  `memcpy` of the whole window was tried first: SROA left the slots no
  step names as memory slices and copied them at every step's snapshot
  (mandelbrot 1.73 of the JIT instead of 0.64). Regions (profiling
  builds) and traces with an access at an unknown offset (the
  window is memory there anyway) use one `memcpy` per copy, which
  shrank the regions' IR fivefold.

### 11.4 Guards, exits and invalidation

- **Guards** follow the `redo_effect` rules (`lowering.md` §10,
  `redocheck.rs`): a branch from an on-trace block to an off-trace one
  is a guard if nothing on an on-trace path from the step's start has an
  effect a re-execution would repeat: a store outside the step's locals,
  the window and its own instruction (cache words, `become`), an atomic,
  or a call that may store, with the collector, the lock and `become`
  exempt as in `redocheck.rs` (allocation is a collector call). Window
  stores are undone by the guard, which writes back the window as it
  was at the step's start (a snapshot SROA keeps in registers). After an
  effect, the untaken code stays and all its continuations exit with
  the effect done.
- A guard runs the instruction again in the interpreter through the
  dispatch table at the **real word's** op, not the word in memory: at
  the loop header that is the enter word, which leads back into the
  trace (the first version looped there until the blacklist ended it).
- **Blacklisting** is by rate: every `LOIMI_TRACE_FAILS` (1024) guard
  failures the trace calls the runtime, which drops it for good if more
  than one iteration in 8 failed (iterations are counted in a register
  and added up at exits). Loop exits are not guards: blocks that ran in
  the profile stay on the trace and leave by exits.
- **Installation** needs the code's length, a hash of the instruction
  words in the trace's range, and each baked-in pool constant to match
  (pool constants that are heap pointers are loaded, not baked; a
  constant is an immediate if its bits match an encoding of the
  constant pool's value type with no pointer payload). A trace is
  installed after each compile, after rewrites that kept the compiled
  code (`become` by retargeting, `Vm.optimize`), and per code object, so
  two code objects with the same words each get it.
- **Uninstalling**: a word change in a trace's range (`become`,
  `Vm.optimize`/`deopt` through `loimi_jit_invalidate`) uninstalls it
  first, then the rewrite proceeds on the restored header; a recompile
  or drop uninstalls it (the old code's header is restored too, for
  copies still running). Each uninstall bumps the trace's `gen`; a trace
  that made a call that may rewrite code checks `gen` before going on
  to the next step and exits if it changed.
- **Debugger and polls**: polls are in the handlers and come along
  (`poll gc` collects with a writeback; `poll gil` hands over the lock
  after one). `--cfg debug` builds cannot use `--jit-stencils` (`on
  break` is not supported there), so traces are off under the debugger.
- **Code not in the profile** (other programs, code loaded later, a
  changed script) matches no trace and runs on the stencils.
- **State**: the vm's handler arguments come from its declaration, so
  the accumulator of `docs/language.md` §7.4 (`vacc`, Mini) is carried
  through the trace in SSA like the context and frame registers, live
  into every step and out at every exit; accumulator variants
  (`ADD_F__RAR`, `JGTKN__A`) stitch like any op.

### 11.5 Results (Mini, 14 benchmarks)

Machine: the shared 8-core x86-64 box, load average about 4.5 (other
agents building and testing). User+sys CPU seconds, minimum of 7
interleaved runs, `--release --align-functions=64`, every output
checked. Master at 102aa44 (value accumulators) merged.

- *interp*: Mini's default build. *JIT*: `--jit-stencils`,
  `LOIMI_JIT=1`. *JIT A/A*: the same binary again (noise control).
- *trace*: `--traces` from one profile of all 14 programs (each
  recorded, region-profiled and traced by the workflow of §11.1), with
  `LOIMI_JIT=1`; *trace off*: the same binary with `LOIMI_TRACE=0`.

| program | interp | JIT | JIT A/A | trace | trace off | trace/JIT | (medians) | trace/interp | traces installed |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---|
| binary_trees | 0.837 | 0.689 | 0.688 | 0.691 | 0.710 | 1.00 | 1.08 | 0.83 | none (calls) |
| closures | 0.467 | 0.367 | 0.365 | 0.365 | 0.366 | 1.00 | 1.03 | 0.78 | none (calls) |
| fannkuch | 1.147 | 0.900 | 0.899 | 0.658 | 0.898 | **0.73** | 0.74 | 0.57 | 3 |
| fib | 0.947 | 0.679 | 0.660 | 0.666 | 0.672 | 0.98 | 0.96 | 0.70 | none (recursion) |
| hash_strings | 1.060 | 0.817 | 0.787 | 0.800 | 0.816 | 0.98 | 0.98 | 0.75 | 4 |
| interp | 0.765 | 0.560 | 0.580 | 0.594 | 0.585 | 1.06 | 1.03 | 0.78 | none (too large) |
| mandelbrot | 0.628 | 0.363 | 0.362 | 0.238 | 0.366 | **0.66** | 0.65 | 0.38 | 2 |
| nbody | 0.502 | 0.365 | 0.366 | 0.404 | 0.364 | 1.11 | 1.10 | 0.81 | 1 |
| oo_methods | 0.762 | 0.509 | 0.514 | 0.510 | 0.511 | 1.00 | 1.01 | 0.67 | none (calls) |
| queens | 0.958 | 0.731 | 0.726 | 0.674 | 0.729 | 0.92 | 0.93 | 0.70 | 1 |
| sieve | 0.727 | 0.579 | 0.575 | 0.508 | 0.574 | 0.88 | 0.87 | 0.70 | 3 |
| sort | 0.836 | 0.657 | 0.663 | 0.561 | 0.658 | 0.86 | 0.86 | 0.67 | 7 |
| spectral_norm | 1.170 | 0.804 | 0.802 | 0.803 | 0.804 | 1.00 | 1.00 | 0.69 | none (calls) |
| string_build | 0.764 | 0.743 | 0.744 | 0.738 | 0.739 | 0.99 | 1.01 | 0.97 | none |
| **geometric mean** | | | | | | **0.932** | | **0.700** | |

JIT/interp is 0.752; the A/A pair is 0.998 and trace-off/JIT 1.004, so
noise is about 1% here (single programs: up to 4%, `fib`, `interp`).

**Stop rule (§8, M1): the gain over the JIT is 6.8% by geometric mean,
under the 10% the rule asks for.** The tier does what the experiment
measured where it applies: mandelbrot 0.66 (experiment 0.63),
fannkuch 0.73 (0.82), sieve 0.88 (0.87), plus sort 0.86 and queens
0.92, which the experiment did not try. But it applies to 7 of the 14
programs, and loses on one:

- **Calls end traces.** binary_trees, closures, fib, oo_methods and
  spectral_norm spend their time in loops whose hot path calls a Mini
  function (an exit: the path "leaves the loop", §11.2) or in recursion.
  `interp`'s dispatch loop is too large (856,000 lines of IR as a
  region, over the 150,000 budget). These seven are 1.00 by design;
  without them the geometric mean is 0.86.
- **nbody is 1.11, where the experiment had 0.73.** Its trace runs 1–4
  iterations per entry, and the code is tight but spills: about 25 live
  window slots, a snapshot of them at each of 25 guarded steps, and a
  writeback of all of them at each exit. Two cheaper fixes were made
  and measured on the way (named-slot copies instead of a `memcpy`:
  nbody 1.38 → 1.26; read-only write-barrier tests without writeback:
  1.26 → 1.11). The rest is M2's dirty-set writeback and
  snapshots of only the slots a guard's step changes.

### 11.6 Tests

- `mini_traces` (tests/golden.rs): the benchmarks and scripts recorded,
  profiled (a bounded profiling build) and traced; then every script and
  conformance test, in every GC mode (normal, stress, generational
  verify), with the JIT at threshold 2 and the traces; the benchmarks'
  outputs; then all of it again with traces whose guards fail
  (`LOIMI_TRACE_MINFRAC=0.5`: a block run in under half of its
  instruction's runs is off the trace) and checked accesses
  (`LOIMI_TRACE_CHECK=1`), with blacklisting at the default rate and at
  every failure (`LOIMI_TRACE_FAILS=1`). The conformance tests' code is
  not profiled: they test that code without a profile runs without
  traces.
- `mini_traces_fuzz` (tests/golden.rs, generator tests/fuzz/traceprog.rs):
  10 random Mini programs (loops of arithmetic that changes type with the
  input, table reads and writes, string building, closures over loop
  variables, calls, `break`s, nested loops), profiled with one input and
  run with three, with traces, with failing guards and with blacklisting
  at every failure; the output must be the interpreter's. The input is a
  string constant, which traces load from the pool, so the same code
  object takes each input.
- ASan: Mini built with `--jit-stencils --sanitize=address --traces`
  (benchmark profiles; trace functions instrumented, stencils not) ran
  mandelbrot, nbody, sieve, fannkuch, sort, queens and hash_strings with
  their traces, correct and without reports, and the scripts without
  reports. Neither combination was rejected for a reason the code or
  history gives, so both are allowed now; ASan is not in `cargo test`
  (the ASan trace build takes 10 minutes on this machine).
- `--gc-check-roots` with `--jit-stencils`: Mini's scripts in every GC
  mode with the JIT at threshold 1 print what the same binary's
  interpreter prints (159 runs, stderr included). Traces were not run in
  this mode (its stencils differ, so it needs its own recording and
  profile).
- During the work the tests caught: dispatching a header guard's
  re-execution through the enter word (an endless re-entry), heap
  constants named by Mini's state order (`%k`), branches with `!prof`
  metadata, locals passed by reference (`for_count(&regs, ..)`), the
  experiment's window bugs (§11.3).

### 11.7 Other vms

- **MiniJS** works unchanged: nbody, fannkuch, spectral_norm, bitops
  and sort recorded, profiled and traced (21 traces; installed in
  nbody, fannkuch, bitops and sort); every output equal to the JIT's.
  Its handlers return `i64` (`%r = musttail call ..; ret i64 %r`), which
  the generator now follows.
- **Scheme and Prolog** were not run through the workflow in this
  milestone. Prolog has no `regs` state register: the generator then
  keeps no window (every access is memory), which is supported but
  untested.

### 11.8 Costs and what remains

- **Build time.** A trace build takes as long as a JIT build plus LLVM
  on the traces (35 s for Mini with 21 traces). A *profiling* build is
  slow: 36 regions (500,000 lines of handler IR) took 3.4 minutes. They
  were `optnone` first, which is worse: in an `-O3` module such
  functions still get the optimizing register allocator, superlinear
  in their thousands of locals (9.3 minutes; one region alone compiles
  in under 2 s). The tests bound it (`LOIMI_TRACE_MAXTOTAL`). Compiling
  regions in a module of their own at `-O0`, or counters in the
  stencils instead of regions, is for M2/M3.
- **Not done in M1**, for M2: linking traces to inner-loop traces (the
  outer loops of mandelbrot and nbody), side traces, `promote` for inline
  caches, the `Num` assume, retagged FORLOOP, dirty-set writeback and
  per-guard snapshots (nbody), traces through calls (M4). The last two
  are §12.

## 12. Cheaper exits, and traces through calls (M4, in part)

After M1 two things kept the tier from paying: nbody's trace was slower
than the JIT (1.11), and the seven programs whose loops call Mini
functions got no trace. This section is the work on both: (A) exits
and guards that copy only what they must, and a way to decline a trace;
(B) calls inlined into traces where the profile shows one callee.
Results in §12.6.

### 12.1 Exits and guards copy what they must (A)

Window copies are expanded once the whole function is emitted
(`expand_markers`), from markers the steps leave: `@@DIRTY j` after a
store to window slot `j`, `@@USE j` before a load, `@@SNAP` at a guarded
step's head, `@@GUARD` in a guard's exit, `@@COPY` for writebacks and
reloads. Two flow analyses over the function's blocks decide each
copy:

- **Dirty slots** (forward): the slots that may differ from memory, set
  by stores, cleared by writebacks and reloads. A writeback (before an
  exit or a call that may read the registers) copies only those. On a
  loop the set at the header is everything the loop body stores, so an
  exit writes back the slots the trace writes, not every slot it reads.
- **Per-step snapshots**: a step's snapshot holds the slots it stores
  to; a guard restores the slots dirty at the step's start and those the
  step stores to, as they were at its start. Before, every guarded step
  copied the whole window into the snapshot, so with 25 slots and 25
  guarded steps SROA kept 25 old values alive per step (nbody's spills).
- **Live slots** (backward): a reload (at the entry, and after a call
  that may write) loads only the slots read before they are written
  (`@@USE`, snapshots, guard restores, writebacks of dirty slots).

nbody's inner loop went from 1.11 of the JIT to 0.75 (0.76 in
§12.6); mandelbrot and fannkuch gained a few percent too.

**Declining a trace.** The report now gives iterations per entry (the
loop's runs over its entries, from the region's counts), and
`LOIMI_TRACE_MINRUN=x` declines traces under `x`. It is off by default:
after the copies above, the short-trip traces measured are all faster
than the JIT (nbody's inner loop runs 1.5 iterations per entry and is
0.76; fannkuch's 1.1 and wins too), so no threshold separates good from
bad. Two other rules decline traces where the cost is known (§12.4).

### 12.2 Profiles of calls

Inlining a call needs three things the M1 profile did not have.

- **Where dynamic jumps went.** A region counts, at each dispatch to a
  computed pc (the stencils' jumps through the table or to compiled
  code through `loimi_jit_entries`: calls, returns, `redo`), where it
  went: `loimi_trace_site` records up to four targets per site (code
  object by length and hash of its words, offset, count). The profile
  has them as `site`/`to` lines after the counts. The hashes are taken
  when the profile is written, not at the first visit: quickening
  rewrites words after the first calls, and the trace builder matches
  the targets against the recordings, written at exit too.
- **The callee's branches.** Each small code object gets an *entry
  region*: its instructions outside loops, from word 0, counted like a
  loop's (`LOIMI_TRACE_INLINE_SIZE` instructions, 64, and
  `LOIMI_TRACE_INLINE_LINES` lines of IR, 40,000, at most). Loop regions
  are made first; entry regions have a budget of their own
  (`LOIMI_TRACE_MAXTOTAL` each).
- **What happens after a return.** A return lands mid-loop, at the
  instruction after the call, which in M1 ran in compiled code until the
  loop came round: the region never saw it. Regions now have an entry
  at every instruction: a trampoline per instruction
  (`loimi.trace.rN.eX`) enters the region with its pc in the `insn`
  argument, and the region dispatches on it. A region is installed at
  every instruction's home, so any jump into its range is counted (this
  also profiles loops that are entered mid-body after an exit).

### 12.3 Inlined frames

The trace's path is chosen as in M1, the hottest successor at each
step, with two more kinds of successor: a **call**, to word 0 of a code
object with an entry region, if one target had at least 90% of the
site's calls, it is not on the frame stack already (recursion stays an
exit) and frames nest at most `LOIMI_TRACE_INLINE` (4) deep; and a
**return** to the calling frame, at the target after the call. Each
inlined call is a frame; a step is a (frame, pc) node. An edge from an
instruction to itself (`redo`, as Mini's returns do after closing
upvalues) goes round the step and is not on the path.

- **Frames are real.** The CALL step is the stencil, so it pushes the
  vm's frame (Mini: `fp.resume`, `cl`, `base`, `nres`, `k`) and the
  return pops it, as the JIT does. Exits, guards, the debugger and the
  collector see the frames they would see without the trace: an exit
  inside an inlined frame dispatches at that frame's pc with its
  registers, and nothing is materialized. LLVM forwards the stores of
  the push to the loads of the pop.
- **One window.** An inlined frame's registers are the trace's at an
  offset (`%fN.regs = %tr.regs + roff`), known from the call step's
  analysis (the `regs` it passes on); the window caches all frames'
  slots, so arguments and results stay in SSA values across calls.
- **The callee's identity.** The callee's code address is a heap
  address, different per run. Each inlined frame has a slot
  (`@loimi.trace.slot.NAME[k]`) with the address the trace last bound;
  a call compares the dispatch's target with it. On a miss the trace
  calls `loimi_trace_bind`, which binds the slot if the target is where
  the inlined code object is entered (its length, the hash of its words,
  its baked-in pool constants: a callee descriptor per frame) and
  leaves the trace either way, so no path into the inlined frame passes
  a call to the runtime and LLVM keeps its facts about memory. A word
  change, recompile or drop of the callee unbinds it (clears the slot,
  bumps the trace's `gen`). A different callee at the site leaves the
  trace through the dispatch it was going to make (with the frame
  pushed); it is not a guard failure.
- **Returns** compare the dispatch's target with the calling frame's
  pc after the call (the frame's code address is a phi at its first
  step); a different target leaves.
- **Dispatch code off the path.** The loads that only compute a
  dispatch's handler (the table, `loimi_jit_entries`) are emitted on the
  path that leaves; left in place, LLVM kept them on the path through
  every return (spectral_norm 0.74 instead of 0.63).

### 12.4 What a call costs in a trace

Two stencil patterns of Mini's calls do not suit the window, and both
are generic problems:

- **Stores at unknown offsets.** CALL clears the callee's registers
  past its arguments in a loop bounded by the prototype's register
  count, a load from the heap. M1 handled an access at an unknown offset
  by keeping the whole window in memory (a `select` between a window slot
  and memory). A switch over the window's slots instead keeps the window
  in SSA values but cost an indirect jump per cleared register and
  spilled every slot around the loop. Now such a store writes the dirty
  slots back and the rest of the step works on memory, as after a call
  that may write; the next step reloads the live slots. A load at an
  unknown offset writes the dirty slots back and reads memory.
- **Heap pointers into the window.** M1 assumed the running frame's
  registers are reached only through `regs`, the context or a frame.
  With inlining that is false: an inlined closure's upvalue may be open
  on a register of a frame below, which the window caches (a loop
  calling a closure over one of its locals read the stale register: a
  micro-benchmark caught it, `trace_calls.mini` and the fuzz programs
  cover it now). In inlined frames, every access of the window's
  value type through a pointer the analysis cannot place is checked
  against the window's range. Where nothing before it in the step has an
  effect, a hit is a guard (the interpreter runs the instruction; a
  trace whose callee keeps doing it is blacklisted, which is the right
  outcome: the JIT is faster there); after an effect, the access writes
  the dirty slots back, goes to memory and reloads. The check is a
  subtraction and a compare that is never taken; a switch over the
  slots there instead cost oo_methods 9% (every slot live at every
  table access).
- **Calls that may write, in callees.** A step that calls a function
  that may write (Mini's CLOSURE calls `find_upval`, a return that
  closes upvalues calls `close_upvals`, allocation of tables) writes the
  window back before and reloads after, and keeps the values live
  across the call in callee-saved registers or spill slots. A trace
  whose inlined frames do that on their usual path is declined
  (`LOIMI_TRACE_INLINE_WRITERS=1` keeps it): closures' main loop, with
  11 inlined calls that create closures, measured 1.45 of the JIT at
  first and 1.09 after the changes above. The fraction of such steps
  does not separate good traces from bad (sieve's best trace has one in
  two steps), which is why the rule is about inlined frames.

### 12.5 Recursion, and `interp`

- **Recursive calls are exits.** fib has no loop, and binary_trees'
  loops call recursive functions whose usual path (the leaves) is not
  what the loop's call site runs: a path through one level would fail at
  the first recursive call. The design's options (a trace from a
  function's entry that calls itself through its own entry, unrolling
  one level) keep the frame chain in memory and save only dispatch
  between straight-line stencils, which the JIT already lays out; they
  were not built.
- **`interp`'s dispatch loop** (856,000 lines of IR as a region) could
  be profiled in chunks under the budget, now that regions have an
  entry at every instruction. It was not, because the trace would not
  pay: the loop is an `if` chain over 13 opcodes and the hottest arm
  (PUSH) is 24% of its iterations, so a one-path trace fails its first
  guard three times in four and is blacklisted. It needs side traces
  (M3), not a smaller region.

### 12.6 Results

Machine and method as in §11.5: user+sys CPU seconds, minimum of 7
interleaved runs, every output checked, a JIT A/A pair and the trace
binary with `LOIMI_TRACE=0` as controls; all 14 programs recorded,
region-profiled and traced again by the final compiler (147 regions, 63
of them entry regions, 2.3 million lines of IR, 3.3 minutes to build;
30 traces, 8 of them through calls). `--align-functions=64`, then all
three binaries rebuilt at 128:

| program | JIT | trace | trace off | trace/JIT | (medians) | at 128 | M1 | traces |
|---|---:|---:|---:|---:|---:|---:|---:|---|
| binary_trees | 0.769 | 0.772 | 0.778 | 1.00 | 1.01 | 1.02 | 1.00 | none (recursion) |
| closures | 0.416 | 0.417 | 0.421 | 1.00 | 1.00 | 1.04 | 1.00 | 3 (2 inline map's and fold's callee; twice/bump inlined, blacklisted; main loop declined) |
| fannkuch | 1.006 | 0.671 | 1.006 | **0.67** | 0.67 | 0.69 | 0.73 | 3 |
| fib | 0.743 | 0.732 | 0.732 | 0.99 | 0.97 | 1.01 | 0.98 | none (recursion, no loop) |
| hash_strings | 0.881 | 0.783 | 0.880 | **0.89** | 0.90 | 0.87 | 0.98 | 4 |
| interp | 0.624 | 0.632 | 0.628 | 1.01 | 1.00 | 1.01 | 1.06 | none (too large; §12.5) |
| mandelbrot | 0.373 | 0.239 | 0.391 | **0.64** | 0.62 | 0.62 | 0.66 | 2 |
| nbody | 0.384 | 0.291 | 0.388 | **0.76** | 0.75 | 0.73 | 1.11 | 2 |
| oo_methods | 0.541 | 0.533 | 0.536 | 0.99 | 0.98 | 1.02 | 1.00 | 1 (inlines `apply` and `deposit`); the shapes loop's calls are polymorphic |
| queens | 0.753 | 0.697 | 0.756 | 0.93 | 0.91 | 0.94 | 0.92 | 1 |
| sieve | 0.590 | 0.503 | 0.582 | **0.85** | 0.87 | 0.85 | 0.88 | 3 |
| sort | 0.724 | 0.590 | 0.708 | **0.81** | 0.83 | 0.84 | 0.86 | 9 (2 inline a comparator, blacklisted: upvalue in the window) |
| spectral_norm | 0.837 | 0.535 | 0.845 | **0.64** | 0.63 | 0.62 | 1.00 | 2 (inline `A`) |
| string_build | 0.790 | 0.792 | 0.791 | 1.00 | 1.00 | 1.00 | 0.99 | none |

**Geometric mean: 0.858 of the JIT at 64, 0.862 at 128** (M1: 0.932).
The controls: JIT A/A 0.996, trace-off/JIT 1.003 (1.000 and 1.004 at
128). What each part did:

- **(A)** alone took the suite from 0.932 to 0.881 (nbody 1.11 → 0.75,
  mandelbrot 0.66 → 0.62, fannkuch 0.73 → 0.67) and made no trace slower
  than the JIT.
- **(B)** made spectral_norm 0.64 (its inner loops inline `A(i, j)`) and
  inlined comparators and small methods elsewhere. In closures,
  oo_methods and sort the inlined traces are neutral or blacklist
  themselves (an upvalue open on a window register), and closures' main
  loop is declined (§12.4); binary_trees, fib, `interp` and
  string_build get nothing (§12.5).

**Stop rule (the goal was 0.85 by geometric mean, stopping if a serious
attempt at calls could not get under 0.90): not met: 0.858 (0.862 at 128), between the goal and the line.**
Calls pay where the callee is arithmetic on registers, which the window
keeps in SSA values across the call: spectral_norm went from 1.00 to
0.64. Elsewhere in this suite Mini's calls do heap work that a trace
does exactly as the stencils do: method lookups through inline caches
(oo_methods), closure creation and upvalue boxes (closures), allocation
(binary_trees), and an inlined frame adds the window's costs around it
(a writeback before each call that may write, a reload after, checks of
heap pointers). Three programs need things this milestone does not
have: recursion that is not in a loop (fib, binary_trees), side traces
(`interp`, oo_methods' polymorphic shapes loop), and allocation removal
for closures that do not escape. Without the four programs that get no
trace at all the geometric mean is 0.81.

### 12.7 Tests

- `mini_traces` traces `examples/mini/tests/trace_calls.mini` with the
  rest: small and nested callees inlined, a callee that changes after
  the trace bound it (the identity check), a rare branch in a callee (an
  exit inside an inlined frame), a callee that allocates (collections
  with inlined frames on the stack, in the GC stress and verify modes),
  recursion inside a callee, several results and missing arguments, a
  method that reads an upvalue.
- `mini_traces_fuzz`: the generator makes callees that branch (rarely or
  often, by the input), allocate, and call each other, call sites whose
  callee is a different function one time in 5–17 (an identity miss
  past the 90% threshold), and a recursive function; each program also
  runs with a collection at every safepoint. The test asserts that some
  trace bound an inlined callee. Both tests build their traces with
  `LOIMI_TRACE_INLINE_WRITERS=1`, so callees that allocate are inlined
  there.
- The forced builds (`LOIMI_TRACE_MINFRAC=0.5`, `LOIMI_TRACE_CHECK=1`)
  put guards inside inlined frames too.
- ASan (by hand, as in §11.6): Mini built with `--sanitize=address`
  and traces of the 14 benchmarks' profiles, with
  `LOIMI_TRACE_INLINE_WRITERS=1` (31 traces: closures' main loop too,
  11 inlined calls that allocate). mandelbrot, nbody, sieve, fannkuch,
  sort, queens, hash_strings, spectral_norm, closures and oo_methods ran
  correct and without reports (closures bound 14 callees), and Mini's
  scripts without reports.
- Found on the way: the open-upvalue alias (§12.4), site hashes taken
  before quickening (§12.2), `redo` edges taken for the path (§12.3),
  and entry regions crowding loops out of a bounded profiling build.

### 12.8 Other vms

MiniJS runs unchanged: nbody, fannkuch, spectral_norm, bitops and sort
recorded, profiled and traced (20 traces), every output equal to the
JIT's. Its spectral_norm inlines its calls: 0.57 of its JIT (5 runs);
nbody 0.92, sort 0.96, bitops 0.97; fannkuch's traces did not install
(their words did not match the code at run time) and it runs at 1.00.

### 12.9 Knobs

Build time, in the environment: `LOIMI_TRACE_INLINE=n` (4; 0: calls are
exits and regions have no entry regions), `LOIMI_TRACE_INLINE_SIZE` (64
instructions) and `LOIMI_TRACE_INLINE_LINES` (40,000 lines) for entry
regions, `LOIMI_TRACE_MONO=x` (0.9: lower inlines polymorphic sites;
0.6 made oo_methods 1.14), `LOIMI_TRACE_INLINE_WRITERS=1`,
`LOIMI_TRACE_MINRUN=x` (off). `LOIMI_TRACE_ONLY` now also restricts
traces. `LOIMI_TRACE_REPORT=1` gives per trace its iterations per entry,
inlined calls and steps that call writers; `=2` also the path of loops
that got none. `LOIMI_JIT_STATS=1` counts callees bound.
