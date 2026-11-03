# Design note: debugging guest programs

Status: **prototype**. Built: `on break` (generated breakpoint ops,
`Vm.set_break`/`clear_break`/`rearm`), `#[unfused]` variants,
`#[cfg(..)]` on vm items, `std.debug` (breakpoint sites with the
step-over, line stepping, a command reader), and a debugger for Mini
(`loimi build --cfg debug examples/mini/main.lm`, then
`mini --debug script.mini`). Written 2026-11-03.

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
(section 6).

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

Exceptions: Mini's `on raise` ends the run or unwinds coroutines; a
"break on error" would be one more hook there (not built).

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

## 5. Embedding: a C API (not built)

A host drives the debugger the way the command loop does. What a DAP
adapter needs, as `export "C"` functions over the same pieces:

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
- **Conditional breakpoints, watchpoints, setting variables, break on
  error**: each a few lines over the same pieces (a condition evaluated
  at `hit`, a store hook in a variant wrapping the SET ops, a hook in
  `on raise`).
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
