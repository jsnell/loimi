# Design note: a trace tier stitched from stencils

The trace tier compiles the hot loops of a program, ahead of time, into
one LLVM function each, stitched from the vm's own `--jit-stencils`
stencils (`src/tracegen.rs`). Traces are made from a profile in three
plain steps (record, profile, rebuild: §11.1), and the JIT runtime
(`src/jit_rt.c`) enters a trace at its loop header. Nothing is compiled
at run time beyond what the copy-and-patch JIT does: code that was not
in the profile runs on the stencils.

- §11: the generator, the register window, guards, exits and
  invalidation (M1).
- §12: exits that copy only what they must, and calls inlined into
  traces (M4, in part).
- §13: side traces and loops inside loops (M2). Mini at 0.759 of the
  JIT's time by geometric mean, MiniJS 0.866 (all 14 programs;
  [results.md](../results.md#trace-tier-in-this-run)), no program more
  than 1% slower.

Limitations:

- Traces need `--jit-stencils`, so they have its limits: x86-64 Linux,
  one vm per program, tail-call dispatch.
- `--cfg debug` builds cannot use `--jit-stencils` (`on break` is not
  supported there), so there are no traces under the debugger.
- A profile names the build it came from (a hash of the stencils); a
  trace build takes profiles of the same compiler and vm only.

## The first experiment (2026-11-06)

Before §11, a Python generator (`tools/trace-exp/`, kept for reference,
not maintained) built traces of four Mini loops by hand-run steps, to see
whether stitched handler IR plus LLVM gets near the hand-written bound of
`copy-and-patch.md` §13 (0.56–0.68 of the JIT on mandelbrot's inner
loop). Whole-program times, minimum of 11 interleaved runs, user+sys
CPU, `--align-functions=64`:

| traced loop | trace / JIT | trace / interpreter |
|---|---:|---:|
| mandelbrot's inner loop | **0.63** | 0.36 |
| nbody's inner loop (calls `math.sqrt`) | **0.73** | 0.55 |
| fannkuch's swap loop | 0.82 | 0.64 |
| sieve's two loops (memory-bound) | 0.87 | 0.70 |
| mandelbrot with a guard that fails from row 650, blacklisted | 0.87 | |
| the same, without blacklisting | 1.08 | |

Variants of the mandelbrot trace: the loop compiled whole as a *region*
(all paths, no pruning: the "method JIT" shape) 0.95 of the JIT;
untaken code kept with exits at its continuations 0.82; guards 0.63;
peeled 0.64. **Pruning is what pays**: untaken paths that merge back
into every step keep their values alive. Aliasing of the register file
was never the problem: once `regs` is a local array, SROA turns it into
SSA values in every variant.

What LLVM (19, `-O3`, no extra passes) did with the stitched IR:
register-file promotion with stores sunk to the exits, operands,
constants and every `pc` computation folded (no dispatch left), `JGTKN`'s
two tag tests merged into one compare, loop rotation. The hot loop is 53
instructions against about 100 in the stencils. What stays:

- **The NaN test after each float op** (7 per iteration in mandelbrot)
  is semantic: `inf - inf` gives a NaN that Mini must canonicalize, so no
  compiler drops it. It is a cost of the value representation, not of
  the tier.
- **3 tag tests per iteration**: `JGTKN` on a value an `ADD_F` just made
  (LLVM derives no bit facts from `fcmp ord`) and FORLOOP's loop-carried
  counter and step. Removing them does not pay (`speed-audit.md` §3).
- **Inline-cache words** are plain memory, reloaded after every heap
  store (nbody's `GETFIELD`).

Compile time of one trace with `clang -c` is 30–400 ms (mandelbrot's
4,400 lines of IR 0.08 s at `-O3`, nbody's 19,500 lines 0.41 s), against
7–30 µs per function for the stencil JIT; this is why traces are built
ahead of time.

## 11. M1 as built: ahead-of-time traces from a profile

M1 turned the experiment into a compiler mode. The generator is
`src/tracegen.rs`, in Rust and in the compiler; the runtime side is in
`src/jit_rt.c`.

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
that path closes at the header at least `LOIMI_TRACE_MIN` times. Since
§13.1 the path goes on through the loops inside the loop.

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

**The gain over the JIT was 6.8% by geometric mean**, under the 10%
the experiment had set as the bar for going on; §12 and §13 are what
followed. The tier does what the experiment
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
  1.26 → 1.11). §12.1 removed the rest (0.76).

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

### 11.7 Other vms

- **MiniJS** works unchanged: nbody, fannkuch, spectral_norm, bitops
  and sort recorded, profiled and traced (21 traces; installed in
  nbody, fannkuch, bitops and sort); every output equal to the JIT's.
  Its handlers return `i64` (`%r = musttail call ..; ret i64 %r`), which
  the generator now follows.
- **Scheme and Prolog**: §13.7.

### 11.8 Build time

A trace build takes as long as a JIT build plus LLVM on the traces (35 s
for Mini with 21 traces). A *profiling* build is slow: 36 regions
(500,000 lines of handler IR) took 3.4 minutes (§13.3 made regions
smaller). Regions marked `optnone` were slower still (9.3 minutes): in
an `-O3` module such functions still get the optimizing register
allocator, superlinear in their thousands of locals. The tests bound
the regions' total size (`LOIMI_TRACE_MAXTOTAL`).

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

**Declining a trace.** The report gives iterations per entry. Declining
short-trip traces (`LOIMI_TRACE_MINRUN=x`, off by default) does not
pay: after the copies above every short-trip trace measured beats the
JIT (nbody's inner loop: 1.5 iterations per entry, 0.76). Traces are
declined where the cost is known instead (§12.4).

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

- **Recursive calls are exits**, and traces start only at loop headers.
  fib has no loop, and binary_trees' loops call recursive functions whose
  usual path (the leaves) is not what the loop's call site runs. A trace
  of recursion keeps the frame chain in memory and saves only dispatch
  between straight-line stencils, which the JIT already lays out.
- **`interp`'s dispatch loop** is an `if` chain over 13 opcodes whose
  hottest arm (PUSH) is 24% of its iterations, so a one-path trace fails
  its first guard three times in four. It takes side traces and a
  smaller region, which §13 added.

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

The goal was 0.85 by geometric mean; this step reached 0.858 (0.862 at
128). Calls pay where the callee is arithmetic on registers, which the window
keeps in SSA values across the call: spectral_norm went from 1.00 to
0.64. Elsewhere in this suite Mini's calls do heap work that a trace
does exactly as the stencils do: method lookups through inline caches
(oo_methods), closure creation and upvalue boxes (closures), allocation
(binary_trees), and an inlined frame adds the window's costs around it
(a writeback before each call that may write, a reload after, checks of
heap pointers). Without the four programs that get no trace at all the
geometric mean is 0.81.

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

## 13. Side traces and loops inside loops (M2)

§12 left four programs with no trace at all and `interp`'s dispatch loop
unprofiled. This section makes a trace a graph: a root path around its
loop, through the loops inside it, plus *side traces*, the hot paths
from its exits, each until it rejoins the trace. Everything is still one
LLVM function per loop, built ahead of time from the same three-step
workflow (§11.1); results in §13.6.

### 13.1 Shape

- **The root** is the hottest path from the header, as before, except
  that it no longer ends at an inner loop. Exits are now edges of the
  path search too: a region records each static exit with its block's
  count (no new counters, so profiles keep their format), and a dynamic
  jump whose site went to the same code object (Mini's SWITCH, §13.3)
  is an edge with the site's count. When the path comes back to a step
  it already took (a loop inside the loop), the loop is closed there
  and the path leaves it by the hottest edge out of it. Such steps are
  *heads*: iterations are counted at every head, so a trace's guard
  failures compare with all the iterations it runs.
- **Side traces.** Then, while the budget allows, the hottest edge from
  a step of the trace that leaves it (an exit that ran at least
  `LOIMI_TRACE_SIDE_MIN` times, by default `LOIMI_TRACE_MIN`, and in at least
  `LOIMI_TRACE_SIDE_FRAC` of the root's iterations, 1%) starts a walk
  that follows the hottest path from there until it reaches a step of
  the trace: the side trace rejoins it there, the header or any other
  step, rather than duplicating the tail to the header (its exits are
  candidates for more side traces). An edge into a step that is already
  in the trace just becomes internal. A walk that leaves the loop, or
  passes 1,000 steps, is dropped. Calls on side traces need not be
  monomorphic: each callee of a site that ran often enough is a frame of
  its own with its own binding slot. Side traces go into calls as roots
  do (oo_methods: `apply`'s two branches call `withdraw` and `deposit`,
  each inlined on a side trace).
- **Merging, not tail duplication.** A rejoin makes the step's state
  registers phis of both paths, as a region's are. The window, the
  dirty-slot flow and the per-step snapshots of §12.1 are already
  analyses over the function's whole CFG, so what is dirty at a guard of
  a side trace is exactly what is dirty on the paths that reach it: the
  "transfer" of window state to a side trace is SSA, not a copy.
- **Budget.** Side traces are added hottest first while the trace's
  steps (estimated as lines of stencil IR, cold blocks included) stay
  under `LOIMI_TRACE_SIDE_LINES` (60,000), at most `LOIMI_TRACE_SIDES`
  (32) per trace; if the generated function is still over
  `LOIMI_TRACE_MAXLINES`, the trace is generated again with half the
  side traces.
- `LOIMI_TRACE_SIDES=0` and `LOIMI_TRACE_NEST=0` turn the two parts off.

### 13.2 Accounting, blacklisting, invalidation

- A side trace is an entry of `loimi_traces` of its own (`tN.sK`, with
  `parent` the root's index; the runtime never installs it). Its guards
  count failures in its own state; `loimi_trace_failing` compares them
  with the root's iterations (one in 8, as for roots: a side trace that
  is entered rarely costs little even when it fails) and drops it
  alone: it sets `state[4]`, which the root loads where it would enter
  the side trace, and that edge is an exit again, as without side
  traces. The root runs on. Root guards count against the root.
- A root's word range covers the steps of its side traces (frame 0), so
  a word change anywhere in it uninstalls the whole tree; a dropped side
  trace stays dropped when the root is installed again. Inlined frames
  on side traces bind and unbind as in §12.3. Nothing changes for the
  collector: the frames are the vm's, and every step is a stencil.
- `LOIMI_JIT_STATS=1` prints `N linked, S side traces`, and a line per
  side trace whose guards failed.

### 13.3 What it took besides side traces

- **Regions were quadratic.** A dispatch through the table in a region
  (a cold block's `next`, a computed jump) switched over every
  instruction of the region: `interp`'s dispatch loop was 542,000 lines
  of IR, 23,000 of its 25,000 blocks that switch. A region now records
  the target (`loimi_trace_site`, now 16 targets per site: a jump table
  has a target per case) and leaves through the table, which enters the
  region again at the target's trampoline. The loop is 50,000 lines (all 14 programs: 159 regions, 1 over the budget, against 147 and 13);
  the profiling build of the 14 programs is faster.
- **Mini's SWITCH** jumps to a computed pc (the arm's JMP when it is
  compiled; commit 318fa82). Its site's targets in the same code are
  path edges, and the trace compares the computed pc with each
  successor's address, hottest first.
- **Superinstruction resumes.** A cold block's `next` in a part of a
  super before the last calls the super's resume function (the super
  from the next part on, `lowering.md` §3). The generator took it for an
  opaque exit, so Mini's ADDI_ADDI with a float operand left every trace
  that ran it. It is now an edge to the next part's instruction, which
  runs the same parts.
- **Branch weights.** Step branches get `llvm.expect` where the
  profile's block counts differ by 32 times or more, and guards always
  (toward the trace). Without it LLVM takes each guard for a coin toss,
  so a loop that passes ten guards seems to iterate once, and the
  register allocator gave the registers to the code around it: the
  trace of mandelbrot's whole nest spilled every value of its inner
  loop, 1.85 of the M4 trace; with the weights it is 0.87 of it.
- **Writeback at loop entries.** On edges into a loop inside the trace
  from outside it (the head does not dominate the source), the window is
  written back, so that the inner loop's guards and exits restore only
  what the inner loop changed. Alone it did not cure mandelbrot's spills
  (1.85 with it; the branch weights did); it is kept (not measured on
  its own since) because it bounds what an inner loop keeps live for
  its guards.

### 13.4 Tests

- `mini_traces` traces `examples/mini/tests/trace_sides.mini` with the
  rest (a jump-table dispatch loop whose arms are side traces, a side
  trace that resumes a superinstruction, arithmetic that changes type in
  a side trace, nested loops, a polymorphic call site) and asserts that
  the scripts and benchmarks make at least 10 side traces. Every script
  and conformance test runs as before in every GC mode with these
  traces, with failing guards and with blacklisting at every failure.
- `mini_traces_sides` traces that script alone and checks, with guards
  forced (`LOIMI_TRACE_MINFRAC=0.5`: the side trace's branch taken in 3
  of 7 of its runs is a guard), that a side trace is blacklisted while
  its root runs on.
- The tests' recording and profiling runs are retried when they leave no
  file: under the tests' 2 GB address-space cap the JIT cannot always
  map its code region (about 1 run in 40 of a JIT build compiles
  nothing, silently), and a program whose profile is missing makes no
  traces.
- `mini_traces_fuzz`: the generator makes jump tables and `elseif` chains
  whose arms run more or less often by the input (side traces whose
  guards fail with another input), and every program ends with a
  dispatch loop on `i % (M + 1)`; the test asserts that side traces were
  made. Programs are profiled with one input and run with three, with
  traces, forced guards, blacklisting at every failure and a collection
  at every safepoint.
- ASan (by hand, as in §11.6): Mini built with `--sanitize=address`
  and the trace of `interp`'s profile (its dispatch loop with 14 side
  traces) ran `interp` correct and without reports (29.7 million
  iterations in the trace), and Mini's scripts without reports. Only that
  profile: the build took 15 minutes. ASan builds with many traces do
  not finish in practice (five profiles: over 50 minutes, all 14: over
  60; a build without traces took 18 on a loaded machine), although each
  of the six largest trace functions compiles with ASan alone in seconds;
  the cause is unknown.

### 13.5 Other vms

MiniJS runs unchanged: nbody, fannkuch, spectral_norm, bitops and sort
recorded, profiled and traced (22 traces, 14 side traces), every output equal to the JIT's. Against its JIT
(5 interleaved runs, minimum, a loaded machine: JIT A/A 1.03): nbody
0.58, spectral_norm 0.53, fannkuch 0.79 (§12.8: its traces did not
install, 1.00), bitops 0.89, sort 0.95; 0.730 by geometric mean, no
program slower. Scheme and Prolog: §13.7.

### 13.6 Results

Machine and method as in §11.5 (the shared 8-core box; another agent
building and testing): user+sys CPU seconds, minimum of 7 interleaved
runs, every output checked, a JIT A/A pair and the trace binary with
`LOIMI_TRACE=0` as controls. All 14 programs recorded, region-profiled
and traced again by the final compiler (159 regions, 1 over the budget,
against 147 and 13 before; the whole workflow 4 minutes; 35 traces with
45 side traces). The columns *no sides* and *no nesting* are builds from
the same profiles with `LOIMI_TRACE_SIDES=0`, and with
`LOIMI_TRACE_SIDES=0 LOIMI_TRACE_NEST=0` (5 runs, a separate round);
*M4* is §12.6.

| program | JIT | trace | trace off | trace/JIT | (medians) | at 128 | no sides | no nesting | M4 |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| binary_trees | 0.650 | 0.648 | 0.642 | 1.00 | 1.01 | 1.04 | 1.01 | 1.04 | 1.00 |
| closures | 0.423 | 0.410 | 0.393 | 0.97 | 1.04 | 1.00 | 0.97 | 0.99 | 1.00 |
| fannkuch | 0.895 | 0.311 | 0.892 | **0.35** | 0.34 | 0.35 | 0.52 | 0.65 | 0.67 |
| fib | 0.672 | 0.661 | 0.666 | 0.98 | 1.04 | 1.01 | 1.00 | 1.00 | 0.99 |
| hash_strings | 0.910 | 0.780 | 0.901 | **0.86** | 0.87 | 0.88 | 0.86 | 0.87 | 0.89 |
| interp | 0.701 | 0.503 | 0.704 | **0.72** | 0.71 | 0.72 | 0.85 | 0.85 | 1.01 |
| mandelbrot | 0.430 | 0.228 | 0.433 | **0.53** | 0.54 | 0.53 | 0.60 | 0.61 | 0.64 |
| nbody | 0.365 | 0.248 | 0.365 | **0.68** | 0.67 | 0.64 | 0.66 | 0.76 | 0.76 |
| oo_methods | 0.531 | 0.513 | 0.515 | 0.97 | 0.95 | 0.97 | 0.99 | 0.99 | 0.99 |
| queens | 0.746 | 0.641 | 0.744 | **0.86** | 0.95 | 0.89 | 0.90 | 0.91 | 0.93 |
| sieve | 0.626 | 0.515 | 0.628 | **0.82** | 0.82 | 0.88 | 0.80 | 0.81 | 0.85 |
| sort | 0.756 | 0.510 | 0.751 | **0.67** | 0.69 | 0.71 | 0.79 | 0.81 | 0.81 |
| spectral_norm | 0.741 | 0.484 | 0.741 | **0.65** | 0.63 | 0.66 | 0.65 | 0.61 | 0.64 |
| string_build | 0.648 | 0.618 | 0.656 | 0.95 | 0.97 | 1.00 | 0.99 | 0.98 | 1.00 |

**Geometric mean: 0.759 of the JIT at 64, 0.773 at 128** (M4 0.858,
0.862). Controls: JIT A/A 0.989, trace-off/JIT 0.991 (1.004 and 1.007 at
128); single programs vary by up to 6% in the A/A pair (closures, nbody
at 64). In the separate round: 0.763 with everything, 0.812 without side
traces, 0.835 without side traces or nesting. So:

- **Side traces** took the suite from 0.812 to 0.763: `interp` 0.85 →
  0.71 (14 side traces, one per hot arm of its jump table: §13.6),
  fannkuch 0.52 → 0.35 (the exits from one loop of its nest into the
  next are side traces, so a trace runs the whole nest), sort 0.79 → 0.68,
  mandelbrot 0.60 → 0.54 (the escape exit of the inner loop rejoins the
  outer loops).
- **Loops inside loops** took it from 0.835 to 0.812: nbody 0.76 → 0.66
  (its inner loop runs 1.5 times per entry; the outer loop's trace runs
  it in place), fannkuch 0.65 → 0.52.
- **The rest** (branch weights, resumes, SWITCH edges, smaller regions)
  took it from 0.858 to 0.835: `interp` 1.01 → 0.85 with a root trace
  alone (its loop is now profiled), mandelbrot 0.64 → 0.61.

The targets for this step (`interp` under 0.90, the geometric mean
under 0.83) were met: `interp` is 0.72 (0.72 at 128), the geometric mean
0.759 (0.773). Still at 1.00: binary_trees and fib
(recursion outside loops), closures, oo_methods and string_build (heap
work, which a trace does as the stencils do; oo_methods' methods are
inlined on side traces, 0.97).

**What `interp` looks like.** The root runs the hottest arm (PUSH, 24%); the
SWITCH's dispatch compares the computed pc with each arm's JMP, hottest
first; 12 side traces run the other arms, each rejoining the back edge,
and 2 more the rarer way of a branch inside an arm. A side trace's entry
loads its `state[4]`; there are no counters on the way in. The trace has
73 steps (68 of them on side traces) and 158 guards; the run stays in it
from its first entry (29.7 million iterations, no guard failures).

### 13.7 Scheme and Prolog

**Scheme** (examples/scheme) runs through the workflow of §11.1 with one
generic fix: a dispatch into the same code object is a path edge only
if the step passes on its own register file (a jump table) or goes to
the next instruction (a native call's end); a static continuation that
passes another frame's registers is not one either. Before, recursive
procedures got traces through a return into their own code or a
`CALLSELF` to its word 0, which the register-file check left at every
iteration (ack 1.55 of the JIT, tak 1.23). Its 43 benchmarks (the
inputs of `bench/scheme/compare.py`) make 109 traces with 74 side
traces; the workflow takes about 3 minutes. User+sys CPU, minimum of 7
interleaved runs, every output checked, controls as in §11.5; the full
table is in `results.md`:

- **0.773 of the JIT by geometric mean at `--align-functions=64`, 0.767
  at 128**; trained on a quarter of each iteration count, 0.774. JIT
  A/A 0.978 and trace-off 0.963 (a loaded round; a quieter one of the
  same profile: 0.779, A/A 1.004, trace-off 1.003).
- sum 0.24, mbrot 0.39, pnpoly 0.40, takl 0.42, sumfp 0.44, ntakl 0.45,
  fft 0.50, array1 0.55, simplex 0.56, nqueens 0.57, puzzle 0.63,
  destruc 0.68. At 1.00 (±5%): recursion (fib, tak, cpstak, ctak,
  divrec), bignums (pi, chudnovsky), allocation (nboyer, sboyer, deriv,
  gcbench). No program is over 1.05 in two rounds but string (20 ms:
  1.18 in one, 0.85 in another).
- Tests: `scheme_traces` (tests/golden.rs) traces the Scheme test
  programs with `examples/scheme/tests/trace_loops.scm` (fixnums turning
  flonums, nested vector loops, a dispatch loop, recursion next to
  loops, allocation) and runs them all in every GC mode, with guards
  failing and with blacklisting at every failure.

**Prolog** has no backward jumps: its loops are recursion, so the
workflow makes no traces. Taking a dispatch to a code object's own word
0 (EXECUTE of its own predicate) as a loop gave 11 traces over the 30
programs and 1.001 of the JIT (A/A 1.002; queens_8 0.86, the rest
0.96–1.10, as the A/A pair): its registers are in memory, so a trace
saves only dispatch. Not kept.

**`promote` of inline-cache words (2026-11-08): measured, not built.**
Recorded cache values folded into every trace (with or without a guard
per load): MiniJS richards, deltablue, raytrace, properties, Mini
oo_methods, nbody within the A/A spread (0.94–1.15, loaded machine). The
bound is coverage: richards, deltablue, raytrace run 1.3%, 0.8%, 0.5% of
their instructions in traces; properties' traced sites are megamorphic.
