# Design note: debugging guest programs

Status: **prototype**. Built: `on break` (generated breakpoint ops,
`Vm.set_break`/`clear_break`/`rearm`), `#[unfused]` variants,
`#[cfg(..)]` on vm items, `std.debug` (breakpoint sites with the
step-over, line stepping, line starts, a command reader), and
debuggers for Mini (`loimi build --cfg debug examples/mini/main.lm`,
then `mini --debug script.mini`) and for the Python subset
(`python-loimi --debug prog.py`, pdb-like; section 8, which reports
what did not fit the first design). For editors (section 9): Mini's
stop-callback C API (`mini_debug_*`, `--cfg debug`), conditional and
hit-counted breakpoints, break on error, a pause from another thread,
expression evaluation, and a Debug Adapter Protocol server
(`examples/mini/dap/mini_dap.c`). Written 2026-11-03.

> Breakpoints, single-stepping and inspecting frames and locals in an
> interpreter built with Loimi: how much can be generated from the `vm`
> declaration, at no cost when not debugging? Today an interpreter
> author writes a line hook by hand (a `variant` wrapper) and keeps
> their own breakpoint table.

**Answer.** The instruction-level part is generic, and that part has
the subtle invariants: patching verified code, quickening and
superinstructions under a breakpoint, inline caches, instruction sizes.
The compiler now generates it. The guest-level part (what a line,
a frame or a local is) stays the interpreter's, with `std.debug`
supplying the bookkeeping (breakpoint sites, the step-over, the
step/next/finish logic, command parsing). For Mini, the debugger is
about 410 lines (`examples/mini/debugger.lm`) plus 14 `#[cfg(debug)]` items in
the vm, compiler and `main`. Without `--cfg debug`, Mini's IR is
byte-identical to before. With it but not debugging, the benchmarks
are within noise. A program being debugged runs at full speed between
breakpoints, and at 3.3x the time (1.5x to 4.9x) while a stepping command runs
(section 6). A second debugger, for the Python subset, needed no change
to the instruction-level part and four small additions to `std.debug`
(section 8).

## 1. What there was

- **Instrumentation variants** (language.md §9): `variant NAME wraps
  all { before { .. } }` makes a second dispatch table of wrappers, and
  `use_variant` switches tables from vm code. Off, they cost nothing
  but one state register (the table), and only in vms that declare
  variants.
- **Line tables**: compilers record a source position per instruction
  (`std.bc`'s `set_loc`/`locs_into`, Mini's `Proto.lines`), which error
  messages use.
- **Rewriting verified code**: `become` (one opcode),
  `Vm.optimize`/`Vm.deopt`, all keeping instruction boundaries, and
  `Vm.Resume`, a proof that an address is a boundary.
- **The generated disassembler** (`Vm.disasm_insn`, `Vm.op_name`,
  operand metadata).

A hand-written debugger is a variant whose wrapper looks up `pc` in a
breakpoint table on every instruction, plus a REPL. That works.
Debugging then runs every instruction through the hook, and a
breakpoint inside a superinstruction is never seen.

## 2. Breakpoints

Three designs:

1. **Patch the opcode** at the breakpoint with a breakpoint op, keeping
   the original elsewhere (CPython 3.12's `INSTRUMENTED_*` and `sys.monitoring`,
   gdb's `int3`). Code without breakpoints runs unchanged; the
   breakpoint costs one dispatch when hit. The difficulty is everything
   else that reads or writes code words.
2. **A debug variant checking a bitmap** per instruction (CPython ≤ 3.11's
   line tracing). Simple, nothing is patched, but every instruction pays a
   lookup while the debugger is attached, even with no breakpoint near.
3. **A per-code-object flag** that switches the code to the variant
   table. The switch has to happen when control enters or leaves the
   object (call, return, exception, coroutine switch, nested run). That
   is a check in the hot CALL/RET handlers of the default table, so it is
   not free when not debugging. CPython 3.12 does this per code object by
   rewriting all of its instructions to instrumented ones, which is design 1.

**Chosen: 1, with a variant for the instruction after a breakpoint.**

### 2.1 The generated ops

`on break { .. }` in a vm declares the handler. The compiler adds one
op per inline cache layout, `BREAK` and `BREAK__C` for each cache `C`,
so the breakpoint written over an instruction has that instruction's
size (`Vm.op_size`). Every walk over the code (`resume_at`,
`is_insn_boundary`, the disassembler, `deopt`, a debugger looking for
a line's first instruction) stays on boundaries with breakpoints set,
without a side table. That costs `1 + #caches` opcodes (3 for Mini),
only in builds that declare `on break`.

A side table would be the alternative: one BREAK op, with the
original opcode kept per code object, as CPython does. Then every
generated walker (verifier, `optimize`, `deopt`, resume walks, the
disassembler) would have to consult the table. With size-preserving
ops, none of them changed.

### 2.2 Patching verified code

`Verified<V>` code is not immutable: `become`, `optimize` and `deopt`
rewrite it, each in a way the verifier would accept again.
`Vm.set_break(v: Verified, i) -> ?Vm.Break` is the same kind of
rewrite, sound by construction:

- it walks `v` from its start and refuses an `i` that is not an
  instruction boundary (inside an inline cache, say);
- it writes only the opcode field, with a breakpoint op of the same
  size;
- the result, `Vm.Break`, is a proof like `Verified`. Its fields (the
  address and the replaced word) are private to the vm's expansion, so
  `Vm.clear_break(b)` writes back exactly the word that was there, and
  `b.rearm()` patches the same boundary again.

Code with breakpoints does not verify (`BadOpcode`): the verifier
cannot know what a breakpoint stands for. Code is verified when it is
built or loaded, before a debugger sees it, so this is not a
restriction in practice. A serializer must clear breakpoints first
(section 7).

### 2.3 Running the instruction under a breakpoint: the step-over

The breakpoint handler has to execute the instruction it replaced.
Two ways:

- **Dispatch the original opcode directly** (CPython looks up the
  original opcode and jumps to its handler). That needs a new control
  word, "dispatch as op X", in three dispatch modes. The handler then
  runs with the breakpoint still in its word, so a `become` would
  overwrite the breakpoint (quickening would silently delete it), and a
  `become` in a super's later part would rewrite the head. Every
  rewrite would then need a check for breakpoints, which costs in every
  vm that declares `on break`.
- **Step over it**, as gdb does: put the instruction back
  (`clear_break`), switch to a variant, and dispatch the word again
  (`jump pc`). The variant's wrapper runs before the next instruction,
  sets the breakpoint again over whatever the word holds now
  (`rearm`; it may have been quickened), and switches back to the
  default table. Nothing new in the dispatch, and `become` needs no
  check: while the instruction runs, it is the original.

**Chosen: step over.** `std.debug.Breakpoints` implements the protocol
(`hit` in the handler, `tick` in the wrapper). Two details:

- the first wrapper to run after `jump pc` is the restored
  instruction's own (`fresh`), so the breakpoint is set again at the
  second wrapper;
- if the run ends before that (the instruction returned from the bottom
  frame, or raised to the host), the breakpoint waits. The next `hit`
  sets it again first (it may be a hit in a nested run, whose table
  starts without the variant), and Mini's entries and `reload` turn the
  variant on while anything waits (section 3.2).

### 2.4 Interactions

- **Superinstructions.** A breakpoint on a later part of a fused
  sequence would never run: the head runs the parts. `set_break` splits
  the super that spans the word (its head becomes its first part alone,
  as when a part's `become` has no matching super). This is local to
  one sequence, so it is not a deopt of the code object, and the
  inline caches stay warm. A breakpoint op is not a part of any super,
  so `optimize` never fuses over it.
- **Quickening and `become`.** A word under a breakpoint is not an
  instruction that runs, so nothing writes it. `become` only rewrites
  the running instruction's own word and, from a super's part, the
  super's head, and a head under a breakpoint does not run the super.
  During the step-over the instruction is the original; `rearm` patches
  whatever it became.
- **Inline caches.** `BREAK__C` keeps the instruction's cache words.
  `Vm.deopt` keeps a breakpoint (it is its own family) and leaves its
  cache words alone, so the quickened instruction it stands for keeps
  the cache it filled.
- **Verification.** See 2.2. `resume_at` and `is_insn_boundary` work
  with breakpoints set; `verify` does not.
- **The copy-and-patch JIT** compiles code words to machine code and
  writes enter words over compiled instructions (copy-and-patch.md).
  Not built: `on break` with `--jit-stencils` is a compile-time error.
  The design: `set_break` calls the runtime's
  `loimi_jit_invalidate(code)` (as `optimize` does), the JIT treats
  breakpoint ops as uncompilable (they exit to the interpreter), and
  `rearm`/`clear_break` report the rewrite (`loimi_jit_rewritten`).
  Code with breakpoints then runs interpreted up to and through the
  breakpoint. Stepping runs in the variant, which the JIT does not
  compile, so that part needs nothing.
- **Threads** sharing code (`#[threads(shared_code)]`, not built) need
  the patch to be an atomic store, as for `become`.

## 3. Stepping

### 3.1 Generic and specific parts

Line-level stepping needs, at each instruction: its line, the call
depth, and whether the stepping command is done. `std.debug.Stepper`
does the last part from the first two:

| command | stops at the first instruction with |
|---|---|
| `step` | another line, or another depth (a call or a return) |
| `next` | another line at the same depth, or a smaller depth |
| `finish` | a smaller depth |

Lines are compared within a frame, so a loop that jumps back to its own
line does not stop (gdb's behavior; Lua's line hook stops there). The
interpreter supplies line and depth: Mini's are `Proto.lines[pc -
code]` and the frame index. Neither can be generated: the vm
declaration does not know where the current function's line table or
the frame stack are. A vm-level declaration for them (say
`debug_info { line: .., depth: .. }`) would save the interpreter two
small functions and is not worth a construct.

### 3.2 The hook

Stepping runs the vm under the same variant as the step-over, so the
wrapper of every instruction asks the stepper. Two compiler pieces
make that complete:

- **`#[unfused]`** on the variant: under it, a superinstruction's table
  entry is its first part's wrapper, which runs that part alone, and its
  `next` dispatches the next part through the variant table. So every
  instruction is seen, including a line that starts at a super's later
  part. The default table still runs the supers fused. (A `become` in
  the first part, run alone, splits the super; `optimize` would fuse it
  again.)
- **`use_variant` where runs start and continue**: a run's table is in
  a state register, so a nested run (a `table.sort` comparator) starts
  in the default table, and its caller continues in the table it had.
  Mini's `call` and `resume` entries and its `reload` block (which runs
  after every native that may re-enter) turn the variant on when the
  debugger is stepping. That is three `#[cfg(debug)]` lines, and none
  in a build without the debugger.

The variant turns itself off when neither a step-over nor a stepping
command is pending, so `continue` runs at full speed. A breakpoint hit
while stepping stops once: the wrapper skips the stepper on a
breakpoint word, and the breakpoint's handler stops.

Exceptions: Mini's `on raise` ends the run or unwinds coroutines;
"break on error" is one more hook there, before the error unwinds
(section 9.4).

## 4. Inspection

- **Op level, generated**: the current instruction, disassembled with
  its operands (`op` in Mini's debugger: `Mini.disasm_insn` on the
  code, after the step-over has put the instruction back), its name and
  operand kinds (`Vm.op_name`, `Vm.operand_kind`). A register-level dump
  could be generated from the operand kinds that index a register file,
  but it would mean little to a guest programmer; it is not built.
- **Guest level, the interpreter's**: frames and locals. Mini's backtrace
  walks its frame stack (each caller's line is that of its CALL,
  `fp.resume - 1`; for a frame calling a native, `--cfg debug` saves
  `pc` in `sync`). Locals need names. Mini's compiler records, under
  `--cfg debug`, each local's register and live range of code words
  (`open_local` where it declares one, `close_locals` where a block
  ends, `finish_proto`), which is what Lua keeps in `locvars`. `locals`
  and `print NAME` read the frame's registers through them. Values are
  shown as `tostring` would, except that strings are quoted and tables
  and functions are printed without their addresses (so that
  transcripts are deterministic).

## 5. Embedding: a C API (built: section 9)

A host drives the debugger the way the command loop does. The first
sketch of what a DAP adapter needs, as `export "C"` functions over the
same pieces (section 9.1 has the API as built):

```c
int  mini_debug_break(mini_State *s, const char *file, int line);  /* -> id, or 0 */
void mini_debug_delete(mini_State *s, int id);
/* Called at each stop; returns what to do next. */
typedef enum { MINI_CONTINUE, MINI_STEP, MINI_NEXT, MINI_FINISH } mini_Resume;
typedef mini_Resume (*mini_StopFn)(mini_State *s, void *ud, int bp_id);
void mini_debug_on_stop(mini_State *s, mini_StopFn f, void *ud);
/* Valid inside a stop callback: */
int  mini_debug_depth(mini_State *s);
int  mini_debug_frame(mini_State *s, int level, const char **func, int *line);
int  mini_debug_local(mini_State *s, int level, int n, const char **name);  /* pushes the value */
```

The stop callback runs on the interpreter's thread inside the
breakpoint's handler, as Mini's command loop does now. An adapter that
must answer the client asynchronously blocks in the callback on a
queue, as debugpy does. `stop()` in `debugger.lm` would become the
default callback.

## 6. Cost

**Not debugging.** The debugger is a build option, `--cfg debug`, not
always present. A vm with a variant carries the dispatch table in a
state register, which costs a register in every handler, and the
`reload` hook adds a check after each native that may re-enter. That is
a small but real cost in the hot loop. The option is what makes the
default build's IR byte-identical: every Mini example, in all three
dispatch modes, compiles to the same `--release --emit-llvm` as before
(with std paths in panic strings normalized), and so does every other
example. Mini's `#[cfg(debug)]` additions sit on existing lines, or at
the end of the vm, so that a build without the debugger keeps its
source positions in panic messages. (`#[cfg(gil)]` already did this.)

**Compiled in.** The 14 Mini benchmarks (`bench/mini/programs`), best of
5, interleaved, tail-call dispatch, x86-64, on a machine shared with
another benchmark run:

| program | default (s) | cfg-debug | attached | stepping |
|---|---:|---:|---:|---:|
| binary_trees | 0.859 | 0.98 | 0.97 | 2.45 |
| closures | 0.473 | 0.99 | 1.00 | 2.94 |
| fannkuch | 1.209 | 1.03 | 1.04 | 3.61 |
| fib | 0.962 | 0.99 | 0.98 | 3.45 |
| hash_strings | 1.031 | 0.98 | 0.97 | 2.78 |
| interp | 0.982 | 0.97 | 0.97 | 4.88 |
| mandelbrot | 0.630 | 0.99 | 1.00 | 4.56 |
| nbody | 0.457 | 1.01 | 1.01 | 4.53 |
| oo_methods | 0.768 | 0.95 | 0.96 | 2.56 |
| queens | 0.909 | 0.98 | 0.97 | 3.48 |
| sieve | 0.716 | 1.02 | 1.02 | 2.50 |
| sort | 0.795 | 0.99 | 1.00 | 3.39 |
| spectral_norm | 1.070 | 1.00 | 1.01 | 3.86 |
| string_build | 0.813 | 0.99 | 1.00 | 1.49 |
| **total** | 11.673 | 0.99 | 0.99 | 3.32 |

(Ratios to the default build; noise on this machine is a few percent either way.)

- *cfg-debug*: the `--cfg debug` build, run as `mini script`;
- *attached*: `mini --debug script` with `continue` at the first stop
  (the debugger attached, no breakpoints);
- *stepping*: `finish` at the first stop, so the whole program runs
  under the step variant, with the wrapper calling into the debugger
  before each instruction.

The first two are the cost of having the debugger, which is within
noise. The last is what design 2 (a hook on every instruction) would
cost for the whole session, and what design 1 costs only while a
stepping command runs. A breakpoint that is set but not reached costs
nothing (*attached*).

## 7. Limitations and next steps

- **Code made after a breakpoint is set** (Mini compiles a whole file
  at once; an interpreter with `load` or `eval`) needs the breakpoint
  kept by file and line and applied to new code objects as they are
  verified (gdb's pending breakpoints). The table knows file and line,
  so this is a hook where the interpreter finishes a code object.
- **Serialization, `optimize`, `verify` with breakpoints set**: clear
  them first (`Breakpoints.clear_all`). `optimize` and `deopt` are safe
  with breakpoints set, but a code cache must not save breakpoint ops.
- **Coroutines**: Mini's depth is the frame index in the running
  coroutine's stack, so `next` across a `coroutine.resume` treats the
  coroutine's frames as shallower or deeper by accident. A per-thread
  depth (the stack of resumers) would fix it.
- **Watchpoints, setting variables**: a store hook in a variant
  wrapping the SET ops; a store into the frame's register (not built).
  Conditional breakpoints and break on error: section 9.
- **JIT and threads**: section 2.4.
- **Stack vms**: `on break` is rejected there for now (the breakpoint ops
  would need the stack effect of the instruction they stand for).

**Does the generic part pay off?** The compiler part is small (about
150 lines of Rust, most of it the generated `set_break`/`rearm`). It
removes the parts a hand-written version gets wrong: the size of a
patched instruction, supers, caches, verification. A hand-written
BREAK op could not be made sound without the compiler, because it
would have to forge proofs. `std.debug` is ordinary library code that
an interpreter could also write; it is worth sharing because the
step-over protocol has two corner cases (the instruction's own wrapper
runs first; a pending breakpoint across nested runs), and those are
where a hand-written version breaks. The interpreter keeps what only
it knows: lines, depth, frames and locals.

## 8. Second interpreter: Python

Is the generic part really generic? The same debugger was added to
`examples/python` (`loimi build --cfg debug examples/python/main.lm`,
then `python-loimi --debug prog.py`; SPEC.md, "Debugger"), an
interpreter that differs from Mini where a debugger cares: generators
are frames on the same frame array whose registers are saved in the
generator object between resumes; exceptions unwind through per-prototype
handler tables in `on raise`, with `try`/`finally`; compare-and-branch
ops are `then`-fused with the JMP after them; four inline cache layouts
(globals, attributes with KeyDict entry numbers, `SETATTR`, two-class
method caches) and four supers; builtins written in Python (a prelude
that `list(g)` and `sum(g)` go through, `io` compiled at import); and
nested runs from natives (`sorted` keys, `next(g)`, `g.send`,
`__repr__`). The debugger behaves like pdb where that is natural
(locations, `(Pdb)`, `where`), with gdb's `finish`.

**Size.** `examples/python/debugger.lm` is 554 lines of code (Mini's: 333),
plus 25 `#[cfg(debug)]` items in the vm, compiler and `main`, on
existing lines but for two helpers at the end of `compile.lm` (Mini: 15). The difference is display and locals, not stepping:
showing values without running Python code (Python's `repr` may call
`__repr__`, so the debugger has its own, 107 lines; Mini reuses
`write_value`), and tracking where locals are bound (102 lines; Mini's
compiler has block-scoped locals with live ranges to report). The
breakpoint and stepping parts (`on break`, the variant, `stop`,
`set_line_break`) are the same shape as Mini's.

### 8.1 What fitted as it was

- **`on break`, the generated ops, `set_break`/`clear_break`/`rearm`,
  `#[unfused]`**, and `std.debug`'s `Breakpoints`, `Stepper`, `Input`
  and `parse_command`: unchanged. Breakpoints land on `GETATTR`,
  `LOADMETHOD` (`BREAK__AttrIC`, `BREAK__MethIC`) and the head of
  `LOADMETHOD_CALLMETHOD`, and quickened members are rearmed as such.
- **`then`-fused ops.** The JMP after a `LT`-style op is an instruction
  of its own, so it could carry a breakpoint, and the op reads its label
  through `then.target`. That stays right with a breakpoint on the JMP:
  `set_break` writes only the opcode field, so the label field the op
  decodes is the JMP's. (No line starts at such a JMP, so it does not
  arise in practice.)
- **Generators.** A resumed generator is a frame one above the
  resumer's, in the loop (`FORITER`) or in a nested run (`next(g)`), so
  the frame index is the depth with nothing generator-specific: `step`
  at a `for` over a generator stops at the line the generator resumes
  at, `next` over a `yield` stops in the resumer (pdb's behavior), and a
  breakpoint in a generator body is an ordinary site. (Mini's coroutines
  have stacks of their own, which is why its depth is wrong across a
  `resume`, section 7.)
- **Exceptions.** `on raise` finds the handler and `jump`s to it through
  the current table, so the variant sees the handler's first
  instruction: `next` over a call that raises, or `finish` from the
  raising function, stops at the `finally` or `except` that catches it,
  frames above or in the same function, with no exception-specific code.
- **Backtraces of frames calling natives.** As in Mini, `sync` saves
  `pc` (into a `#[cfg(debug)]` field of `Frame`), so a frame calling
  `sorted` shows its line.

### 8.2 What did not fit, and what changed

1. **A frame leaving to a native.** `next` at the last line of a `sorted`
   key stopped in the *next* call of the key: the native calls it again
   at the same depth, and the depth alone cannot tell the new frame from
   the old one. In the loop, a return passes through the caller (a
   smaller depth) before any new call, so this happens only when a
   nested run ends. Mini had the same problem with `table.sort`
   comparators (`step` at a one-line comparator skipped its later calls,
   as the same line at the same depth). *Generic fix:*
   `Stepper.left(depth)`, called where a nested run ends (Python's
   `invoke_at` and `gen_next`, Mini's `call_value`): `step` then stops at
   whatever runs next, and `next`/`finish` wait for the caller (Mini's
   new transcript `sort_next`). pdb gets this from frame identity; a frame serial number
   would cost a store per call.
2. **Line starts.** Mini's rule (a site at the first word of each run of
   a line) gave one stop per `for` loop instead of one per iteration:
   Python's `GETITER` and `FORITER` share the `for` line, and the jump
   back goes to `FORITER`, in the middle of the run. *Generic:*
   `debug.line_sites`, which puts the site at a run's *loop head* (the
   target of a backward jump from another line) when it has one. That
   runs on every iteration and on entry, so the count of stops is pdb's;
   the first stop comes after the iterable is evaluated. Jumps within a
   line (comprehensions, conditional expressions) add no site.
3. **Walking code with breakpoints set.** Finding loop heads reads jump
   labels, and `Vm.label` of a breakpoint word is the breakpoint op's
   (none): a breakpoint on a loop's jump back hid the loop head from a
   later `break` on the loop's line. Size-preserving breakpoint ops keep
   *sizes* right for every walker (section 2.1), but not labels, operand
   kinds or names. *Generic:* `debug.unpatched` copies the code with each
   site's original word (`Break.word`) for such walks. (A generated
   `Vm.label` cannot see through a breakpoint: the original opcode is in
   the `Vm.Break`, not in the code.)
4. **Instructions of no line.** Python's compiler gives a loop's jump
   back the line of the body's last statement, so `next` after a
   `finally` stopped on `out.append(-1)` in an `except` that did not
   run: the jump back follows it in the code. Mini's compiler happens not
   to do this. *Generic:* line 0 means "no line" and `Stepper.should_stop`
   never stops there (CPython 3.10 has the same notion for artificial
   instructions, PEP 626); Python's compiler gives the jumps back line 0
   under `--cfg debug` (a JMP never raises, so error lines do not change).
5. **`#[cfg]` on a struct literal's field.** The saved `pc` is a
   `#[cfg(debug)]` field of `Frame`, and `main` builds the bottom frame
   with a literal, which had no way to leave the field out of a default
   build. *Compiler fix:* `#[cfg(..)]` before a field initializer.
6. **Code that is not the program's.** The prelude (`__list`, `__via1`,
   through which `list(g)` and `sum(g)` run generators) and `io` are
   Python code in the same code objects and frames. `step` stopped in
   them, and `bt` showed them. The interpreter decides: its `on_step`
   does not ask the stepper in code outside the program's prototypes,
   and `bt` skips their frames (pdb's `skip`). Nothing generic was
   needed, since the stepper only sees what it is asked about; `next`
   and `finish` treat such frames as frames, so a generator consumed by
   `list(g)` is "called" from the prelude: `finish` in it runs to the
   end of `list(g)`.
7. **Locals.** Python's locals are function-wide registers that are not
   cleared on entry (reading a local before assignment is not detected),
   so a register may hold another call's value. Initializing them in
   debug builds would cost every call; instead the compiler reports where
   each name is first stored in code order (`store` on the existing lines
   that bind names), and `locals` shows a local from there on (a local
   assigned later in a loop than it is read is hidden on the first part
   of the second iteration). Comprehension targets have live ranges, as
   Mini's block locals. Cells and free variables are read through their
   cells.
8. **Showing values.** Python's `repr` may run `__repr__`, a nested run
   from inside the `on break` handler that would hit breakpoints and
   steps of its own (pdb turns tracing off while it evaluates). The
   debugger prints values without running Python code. Evaluating
   expressions at a stop needs that "nested run with the debugger
   suspended": not built.
9. **Source positions.** The vm is not at the end of `vm.lm` (host
   functions follow it), so `on break` and the variant had to go on the
   existing line that closes `on raise`, to keep the default build's
   panic messages, and IR, unchanged. A layout constraint, not a
   missing feature.

The debugger state is a module variable rather than a `Ctx` field (the
Python `Ctx` is built with a full literal in `main`), and the bottom of
the program's prototypes is recorded before compiling the file, to tell
the program's code from the prelude's.

Mini's debugger gained the `left` hook (one `#[cfg(debug)]` on an
existing line); its other transcripts are the same, and its default
build's IR too.

### 8.3 Cost

Without `--cfg debug`, the IR of the Python interpreter is
byte-identical to before in all three dispatch modes (and so is every
other example's). With it, the eleven pyperformance programs
(`bench/python/compare.py`), tail-call dispatch, best of 3, interleaved,
on a machine shared with another benchmark run:

| program | default (s) | cfg-debug | attached | stepping |
|---|---:|---:|---:|---:|
| chaos | 0.308 | 1.02 | 1.03 | 2.39 |
| deltablue | 0.575 | 0.98 | 0.97 | 3.59 |
| fannkuch | 0.449 | 1.02 | 1.04 | 1.98 |
| float | 0.428 | 1.01 | 1.01 | 2.01 |
| go | 0.362 | 1.01 | 1.01 | 2.67 |
| hexiom | 0.513 | 1.00 | 1.01 | 2.92 |
| nbody | 0.353 | 0.98 | 0.98 | 3.55 |
| nqueens | 0.716 | 1.03 | 1.02 | 1.90 |
| raytrace | 0.479 | 0.99 | 0.99 | 2.76 |
| richards | 0.302 | 1.00 | 1.00 | 3.98 |
| spectral_norm | 0.310 | 0.98 | 1.00 | 2.57 |
| **total** | 4.795 | 1.00 | 1.00 | 2.70 |

(Columns as in section 6: *attached* is `--debug` with `continue` at
the first stop, *stepping* is `finish` at the first stop, so the whole
program runs under the step variant.) A second run of the first two
columns, best of 5, gave 1.006 in total (0.98 to 1.02 per program).
Stepping costs less than in Mini (2.7x against 3.3x) because Python's
instructions do more work each; the hook is the same, plus the one-entry
cache that tells the program's code from the prelude's.

### 8.4 Verdict

The instruction-level part carried over without a change: generated
breakpoint ops, the step-over, unfused stepping and the variant hooks
worked on a vm with `then`-fused ops, four cache layouts, supers,
generators-as-frames and handler-table exceptions. Every misfit was at
the guest level, in what "a line starts here" and "this frame" mean,
and four of them (`left`, `line_sites`, `unpatched`, line 0) were
general enough to move into `std.debug`; one was a compiler gap
(`#[cfg]` on a literal's field). What stayed Python's is what only it
knows: which code is the program's, where locals are bound, and how to
show a value without running guest code.

## 9. Editors: a stop-callback API and a DAP adapter

Section 5 sketched a C API; this is it as built, for Mini, with what an
editor needs on top of the command line: conditional and hit-counted
breakpoints, break on error, a pause button, expressions, and a Debug
Adapter Protocol server (`examples/mini/dap/mini_dap.c`) that VS Code
or any DAP client can run as its debug adapter.

### 9.1 The API

Mini built with `--cfg debug` as a library (`loimi build --cfg debug
examples/mini/embed.lm --lib --header mini.h`) has, at the end of
`embed.lm`:

| call | what |
|---|---|
| `mini_debug_attach(s, cb, ud)` | debug state `s`; `cb(s, ud, reason, bp) -> MiniResume` runs at each stop |
| `mini_debug_load(s, name, src)` | compile a chunk named `name` (its file) and push it, for `mini_call` |
| `mini_debug_stop_on_entry(s)` | stop before the first instruction of the chunk loaded last |
| `mini_debug_break_on_error(s, on)` | stop at errors nothing catches, before they unwind |
| `mini_debug_set_break(s, file, line) -> id` | a breakpoint, in code loaded now or later (pending until then) |
| `mini_debug_condition(s, id, cond, hit)` | a Mini expression, and a hit condition (`N`, `>= N`, `% N`, ...) |
| `mini_debug_code_line(s, file, line)` | the first line at or after `line` with code (where an editor moves a breakpoint) |
| `mini_debug_break_sites`, `mini_debug_delete` | |
| `mini_debug_pause(s)` | stop soon; **the one call another thread may make** |
| at a stop: `mini_debug_message`, `_depth`, `_frame(level, &func, &file, &line)` | the error or failed condition; frames |
| at a stop: `mini_debug_scope(level, Locals or Globals) -> handle` | handles for lazy expansion |
| at a stop: `mini_debug_variables(handle, start, count, f, ud)` | `f(ud, name, value, child)` per variable; `child` is a table's handle |
| at a stop: `mini_debug_eval(level, expr, &out, &child)` | an expression or a statement in that frame |

The reasons are `MiniStop_Entry`, `_Breakpoint`, `_Step`, `_Pause`,
`_Error`; the resumptions `MiniResume_Continue`, `_Step`, `_Next`,
`_Finish`, and `_Keep`, which continues as before the stop (a `next`
in progress goes on), for a host that stopped the program for its own
work (section 9.5). The callback runs on the interpreter's thread,
inside the instruction where the program stopped, as the command line
does: it is the command line's loop with the commands replaced by C
calls. Both front ends share one core (`debugger.lm`: `Dbg`, its hooks,
frames, variables, evaluation); the command line gained `break LINE if
COND`, `print EXPR`, and a stop at uncaught errors.

Variables are shown as text (strings quoted, `table (3 entries)`,
`function 'name'`, no addresses, so transcripts are deterministic).
Handles (`std.debug.Refs`) are ids for a frame's locals, the globals
(without the library's, recorded when the debugger attaches) or a
table, valid until the program continues; the tables they name are GC
roots meanwhile. Locals include the closure's upvalues, whose names the
compiler now reports too (`compile.debug_upvals`, under `--cfg debug`).
Chunks loaded for debugging stay alive (their code holds breakpoints),
and their source is copied (local names point into it).

Strings cross the boundary as `loimi_str` (`{ptr, len}`), which takes two
argument registers; the header generator rejects an export whose `str`
would not fit in the first six. That shaped two calls: the condition is
set by a call of its own, and the variable callback has no `type`.

### 9.2 Conditions: running guest code from inside a stop

A condition is a Mini expression evaluated in the stopped frame at each
hit, before deciding to stop; `mini_debug_eval` and the command line's
`print EXPR` are the same thing at a stop. The program is stopped
*inside a handler* (`on break`, the step variant's wrapper, or `on
raise`), so evaluating means a nested run from there. Three questions:

1. **Compiling against the frame.** Mini's locals are registers, and
   Mini has no `load` with an environment. The expression is compiled as
   the body of a function whose parameters are the names visible at the
   stop (the live locals from the compiler's records, inner ones hiding
   outer ones, then the upvalues): `return function(n, total, i) return
   (EXPR) end`. The chunk's one closure is called with their values. So
   locals are read-only copies (assigning to one changes the copy),
   globals are the real ones (`done = true` works: a statement is tried
   when the text is not an expression), and upvalue cells are not
   shared. The compiler's local records are off meanwhile, so an
   evaluation leaves nothing behind but garbage. An error comes back
   without its `line 1:` (the expression's own line).
2. **Running nested from a handler.** The hooks are `uses reenter`, so
   the compiler puts the vm's `sync` before them and `reload` after
   (language.md §5.1), as for a native that calls back: `ctx.fp` is the
   stopped frame, the nested run (`vm.call_value`) starts above its
   registers, the collector sees every frame, and the handler reloads
   `fp`, `regs` and `k` afterwards (a coroutine's stack may have moved).
   That needed no change to the vm beyond the effect. The instruction
   under the breakpoint has already been put back for the step-over
   (section 2.3), and runs after the hook returns (`jump pc`). If the
   nested run executes and quickens that same instruction, `jump pc`
   runs the quickened one, which is fine, and `rearm` later patches
   whatever is there.
3. **The debugger off during the run.** A breakpoint hit inside the
   evaluation must not stop (or recurse) and, worse, must not disturb
   the step-over in progress: `hit` would set the pending site again
   first (it takes a pending site to mean "its instruction started, in a
   run a native interrupted"), and then `jump pc` would hit the stopped
   breakpoint forever. A step variant switched on in the nested run would
   call `tick`, which sets the pending site again too. So
   `Breakpoints.suspend` (std.debug) takes every site but the pending
   one out of the code, `tick` does nothing while suspended, and the
   interpreter's hooks (`on_step`, `stepping`, `left`, `on_error`)
   return at once; `unsuspend` sets the sites again over what the code
   holds then. That is one store per site, and an evaluation compiles
   and runs a chunk anyway. A flag that made the breakpoint handler skip
   the stop instead would have needed the step-over of each nested hit
   without touching the pending one: two step-overs at once, which
   `Breakpoints` does not represent.

A condition that fails to evaluate stops, with the error as the stop's
message (VS Code's behavior); a false one continues at once (`on_break`
returns without a stop; the handler runs the step-over either way).

**Hit counts** count the hits at which the condition held
(`Bp.count`), and the hit condition is checked on that count
(`HitCond`, parsed from DAP's strings: `N` or `== N`, `>= N`, `> N`,
`<= N`, `< N`, `% N`). Both are generic (`Breakpoints.set_condition`,
`counts`).

### 9.3 Pause

`mini_debug_pause` comes from another thread (the adapter's reader)
while the interpreter runs. What may it touch?

- **Patch breakpoints into the code from that thread** (a breakpoint at
  every line start): free when not pausing, but the patching races with
  the running code's own writes (`become` quickens the word being
  patched; the step-over clears and rearms sites) and with its use of
  the `Breakpoints` tables; making that safe needs atomic code writes
  and a lock the interpreter takes at each hit.
- **A signal** to the interpreter's thread, whose handler patches: no
  data race on the code, but the handler may interrupt the debugger
  itself (inside `hit` or `tick`) or `malloc`.
- **A flag the running code polls**, as CPython's eval breaker: the
  other thread only stores a word (`std.debug.request_pause`, a relaxed
  atomic store into `Ctx.pause`); the interpreter, at a poll, switches
  to the step variant (`use_variant step`), whose wrapper sees the flag
  at the next instruction (`take_pause`) and stops with reason Pause.
  Everything else happens on the interpreter's thread.

**Chosen: the poll**, in debug builds only, where every long
computation passes: the `JMP` (a `while` loop's back-edge), `FORLOOP`,
`NEXT`, `INEXT` (the other loops), and `CALL` (recursion without
loops); the places of `--cfg gil`'s hand-over poll, less the
conditional jumps. Those were most of the cost: polling them too (with
a `target <= pc` test so that forward branches skip the load) cost 1.2%
in total and 4 to 5% on branchy programs (interp, queens, mandelbrot);
without them, 0.5% (section 9.7). The one loop that ends in a
conditional jump, `repeat ... until`, compiles in debug builds to a
conditional exit and a `JMP` back (one more dispatch per iteration,
debug builds only), so that every loop polls. A pause asked for while
the program is stopped is dropped when it continues.

### 9.4 Break on error

Mini's `on raise` gets every error at the instruction that raised it,
with the frames intact. The hook (`debugger.on_error`, `#[cfg(debug)]`
on the existing line) runs there for an error raised in this run (not
one passed on from a nested run, whose own run had it first), and
stops if nothing will catch it. Mini has no `pcall`: an error is caught
only by `coroutine.resume` (which returns `false, msg`). So it is
uncaught if no coroutine between the running one and the main thread
was resumed by `coroutine.resume` (rather than through a `wrap`
function, which raises it again). The stop has the message and the
raising frame's stack; continuing lets the error unwind as before. The
command line stops there too.

### 9.5 The adapter

`mini_dap.c` (770 lines of C, a JSON reader and writer included) runs
the script in-process. Of its three threads, one calls into Mini:

- the **main thread** handles requests until `configurationDone`,
  then runs the chunk (`mini_call`); at each stop, the callback sends a
  `stopped` event and handles requests until one resumes;
- a **reader thread** frames and parses requests into a queue. While
  the script runs, it answers `threads` itself, and for anything else
  calls `mini_debug_pause`: after a `pause` request, a stop the user
  sees; after any other (VS Code sends `setBreakpoints` while the
  program runs), an internal one, where the callback handles the queue
  and returns `MiniResume_Keep`, so the program goes on as before;
- a **pump thread** turns the script's stdout (fd 1, which the adapter
  points at a pipe, keeping the protocol on a duplicate of the
  original) into `output` events. At each stop and at the end, the main
  thread flushes stdio and drains the pipe under the pump's lock before
  sending the event, so output comes before the stop that follows it.

Coverage: `initialize` (capabilities: configuration done, conditional
and hit-conditional breakpoints, hover evaluation, terminate, the
exception filter "Uncaught errors", on by default), `launch` (`program`,
`stopOnEntry`; a syntax error fails it), `setBreakpoints` (moved to the
next line with code; unverified where there is none or the hit
condition does not parse), `setExceptionBreakpoints`,
`configurationDone`, `threads` (one), `stackTrace` (`startFrame`,
`levels`), `scopes` (Locals, Globals), `variables` (`start`, `count`;
nested tables by handle), `evaluate` (any context; errors as failed
responses), `continue`, `next`, `stepIn`, `stepOut`, `pause`,
`disconnect`, `terminate`; events `initialized`, `stopped` (entry,
breakpoint with `hitBreakpointIds`, step, pause, exception with its
message), `output` (stdout, and the final error on stderr), `exited`,
`terminated`. Not built: `attach`, `setVariable`, `restart`,
`loadedSources`, `breakpointLocations`, coroutines as DAP threads,
columns (Mini records lines only).

### 9.6 Tests

- `examples/mini/dap/test_dap.py` drives the adapter through a session
  on `examples/mini/tests/dap/session.mini` and prints a transcript
  (requests, responses, events; sequence numbers dropped, consecutive
  output events joined, the directory as `$DIR`): breakpoints moved off
  a blank line, conditional (`n == 4` in a recursive function),
  hit-counted, refused (a bad hit condition, no code), set while the
  program runs; `stepIn`, `next`, `stepOut` across calls; stack traces;
  scopes and nested tables with paging; evaluation (in the frame,
  calling a function that passes a breakpoint, failing); a pause in a
  loop that only `done = true` from the debugger ends; break on error
  with its stack. The golden test (`mini_dap`) builds the library and
  the adapter in each dispatch mode; the transcript is the same in all
  three, and over repeated runs.
- `examples/embed/debug.c` tests the C API without DAP, under GC stress,
  with the library built four ways (`mini_debug_api`): pending
  breakpoints, a syntax error, conditions (true, false, failing), hit
  counts, frames, upvalues, globals without the library, evaluation at
  two levels with statements and errors, expansions from inside the
  variable callback, a pause from a second thread.
- The command line's transcripts (`examples/mini/tests/debug`) gained
  `cond` (conditions, `print EXPR`, the error stop).

### 9.7 Cost

Mini's default build is unchanged: `--release --emit-llvm` is
byte-identical in all three dispatch modes, for `main.lm` and for the
library `embed.lm`, and so is every other example's (the Python
subset's included: its debugger needed one line, for the new
`break .. if` command). The debug build gained the pause polls and the
`repeat` jump. The 14 Mini benchmarks, tail-call dispatch, best of 5,
interleaved, the debug build before and after (not debugging):

| program | before (s) | after | polls only |
|---|---:|---:|---:|
| binary_trees | 0.870 | 1.01 | 1.01 |
| closures | 0.490 | 1.01 | 1.00 |
| fannkuch | 1.226 | 1.04 | 1.01 |
| fib | 1.026 | 0.93 | 0.98 |
| hash_strings | 1.066 | 0.95 | 1.02 |
| interp | 0.968 | 1.02 | 1.02 |
| mandelbrot | 0.635 | 1.01 | 1.00 |
| nbody | 0.468 | 0.98 | 1.02 |
| oo_methods | 0.751 | 1.00 | 1.02 |
| queens | 0.965 | 1.08 | 1.00 |
| sieve | 0.833 | 0.99 | 0.99 |
| sort | 0.891 | 1.02 | 1.01 |
| spectral_norm | 1.186 | 1.01 | 1.01 |
| string_build | 1.138 | 1.04 | 0.99 |
| **geometric mean** | | 1.005 | 1.005 |

*after*: this build against the one before; *polls only*: an earlier
run of the polls without the `repeat` change. The machine was shared
with other benchmark runs (a third run, at load 5.7, scattered from
0.78 to 1.09 per program both ways, so it is not shown). fannkuch,
whose inner loops are `repeat`, was 4% slower here and 1% slower in the
noisy run, probably the extra `JMP`; queens' 8% was 1.00 with the same
polls before and 0.93 in the noisy run. The rest is within noise. A conditional breakpoint costs a compile and a nested
run per hit, about a one-line `mini_dostring`, like conditions in
debuggers that evaluate source text.

### 9.8 What is generic

`std.debug` gained what another interpreter's API would use unchanged:
`Reason`, conditions and hit conditions on `Bp` (`set_condition`,
`counts`, `parse_hit_condition`), `Breakpoints.suspend`/`unsuspend`/
`suspended` (no stops, and the step-over left alone, while the debugger
runs guest code), `Refs` (handles for lazy expansion),
`request_pause`/`take_pause` (the flag), and `break .. if` in the
command reader. What stayed Mini's is what section 8 found
interpreter-specific: how an expression is compiled against a frame
(the parameter trick needs the compiler's records of locals), which
errors are uncaught, how values are shown, and where the polls go. The
adapter is Mini's only by the C names and the two scopes.
