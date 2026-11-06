# Design note: a trace tier stitched from stencils (experiment, 2026-11-06)

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
