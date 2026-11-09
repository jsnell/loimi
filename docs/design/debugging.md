# Design note: debugging guest programs

How Loimi supports breakpoints, stepping and inspection in an
interpreter written with it: what the compiler generates, what
`std.debug` provides, and what each interpreter writes itself.

Status: **prototype**. Built:

- `on break` in a vm: generated breakpoint ops and
  `Vm.set_break` / `clear_break` / `rearm`.
- `#[unfused]` variants and `#[cfg(..)]` on vm items.
- `std.debug`: breakpoint sites with the step-over, line stepping, line
  starts, conditions, a command reader.
- A debugger for Mini: `loimi build --cfg debug examples/mini/main.lm`,
  then `mini --debug script.mini`.
- A pdb-like debugger for the Python subset: `python-loimi --debug
  prog.py` (section 8).
- For editors (section 9): Mini's stop-callback C API, conditional and
  hit-counted breakpoints, break on error, pause, expression
  evaluation, and a Debug Adapter Protocol (DAP) server.

## Summary

- **The instruction level is generic, and the compiler generates it.**
  This is the part with the subtle invariants: patching verified code,
  quickening and superinstructions under a breakpoint, inline caches,
  instruction sizes.
- **The guest level stays the interpreter's**: what a line, a frame or
  a local is. `std.debug` supplies the bookkeeping: breakpoint sites,
  the step-over, step/next/finish, command parsing.

Mini's debugger (`examples/mini/debugger.lm`) is about 780 lines plus
`#[cfg(debug)]` items in the vm, compiler, `main` and `embed`. Without
`--cfg debug`, Mini's IR is byte-identical to a build without the
debugger; with it, the benchmarks are within noise. A debugged program
runs at full speed between breakpoints, and at 3.3x the time while a
stepping command runs (section 6).

## 1. What it builds on

- **Instrumentation variants** (language.md §9): `variant NAME wraps
  all { before { .. } }` makes a second dispatch table of wrappers;
  `use_variant` switches tables.
- **Line tables**: compilers record a source position per instruction
  (`std.bc`'s `set_loc`, Mini's `Proto.lines`).
- **Rewriting verified code**: `become`, `Vm.optimize`/`Vm.deopt`, all
  keeping instruction boundaries, and `Vm.Resume`, a proof that an
  address is a boundary.
- **The generated disassembler**: `Vm.disasm_insn`, `Vm.op_name`.

With only these, a debugger would be a variant that looks up `pc` in a
breakpoint table on every instruction: slow, and blind to breakpoints
inside superinstructions.

## 2. Breakpoints

| Design | Example | Cost when not stepping |
|---|---|---|
| 1. Patch the opcode with a breakpoint op | gdb's `int3`, CPython 3.12 | none; one dispatch per hit |
| 2. A variant that checks a bitmap per instruction | CPython ≤ 3.11 line tracing | a lookup per instruction while attached |
| 3. A per-code-object flag that switches to the variant table | — | a check in CALL/RET of the default table, even when not debugging |

**Chosen: design 1**, with a variant for the instruction after a
breakpoint. Its difficulty is everything else that reads or writes code
words; the rest of this section is how that is handled.

### 2.1 The generated ops

`on break { .. }` in a vm declares the handler. The compiler adds one
breakpoint op per inline cache layout (`BREAK`, plus `BREAK__C` for each
cache `C`), so a breakpoint has the size of the instruction it replaces.
Every walk over the code (`resume_at`, `is_insn_boundary`, the
disassembler, `deopt`) then stays on boundaries with no side table. Cost:
`1 + #caches` opcodes (3 for Mini), only in builds that declare
`on break`.

Rejected: one `BREAK` op with the original opcode in a side table, as in
CPython; every generated walker would have to consult the table.

### 2.2 Patching verified code

`Vm.set_break(v: Verified, i) -> ?Vm.Break` is a rewrite of verified
code like `become`, and sound by construction:

- It refuses an `i` that is not an instruction boundary.
- It writes only the opcode field, with a breakpoint op of the same
  size.
- `Vm.Break` is a proof like `Verified`, with private fields (address,
  replaced word). `Vm.clear_break(b)` writes back exactly that word;
  `b.rearm()` patches the same boundary again.

Code with breakpoints does not verify (`BadOpcode`). Code is verified
when built or loaded, before a debugger sees it, so this does not matter
in practice.

### 2.3 Running the instruction under a breakpoint: the step-over

The handler has to execute the instruction it replaced.

- **Rejected: dispatch the original opcode directly**, as CPython does.
  It needs a new "dispatch as op X" control word, and the instruction
  runs with the breakpoint still in its word, so quickening would
  silently delete the breakpoint. Every rewrite would need a breakpoint
  check.
- **Chosen: step over it**, as gdb does. Put the instruction back
  (`clear_break`), switch to a variant, and dispatch the word again
  (`jump pc`). The variant's wrapper, before the next instruction, sets
  the breakpoint again over whatever the word holds now (`rearm`) and
  switches back to the default table. While the instruction runs it is
  the original, so `become` needs no check.

`std.debug.Breakpoints` implements this (`hit` in the handler, `tick` in
the wrapper) and handles two corner cases that a hand-written version
gets wrong:

- The first wrapper after `jump pc` is the restored instruction's own,
  so the breakpoint is set again at the second.
- If the run ends first (a return from the bottom frame, or an error to
  the host), the breakpoint stays pending. The next `hit` sets it again
  first, and Mini turns the variant on while one is pending (section
  3.2).

### 2.4 Interactions

- **Superinstructions.** A breakpoint on a later part of a fused
  sequence would never run. `set_break` splits that one super (its head
  becomes its first part alone), so inline caches stay warm. `optimize`
  never fuses over a breakpoint op.
- **Quickening.** A word under a breakpoint does not run, so nothing
  writes it. During the step-over the instruction is the original, and
  `rearm` patches whatever it became.
- **Inline caches.** `BREAK__C` keeps the cache words, and `Vm.deopt`
  leaves a breakpoint and its cache alone.
- **The copy-and-patch JIT.** `on break` with `--jit-stencils` is a
  compile-time error.

## 3. Stepping

### 3.1 Generic and specific parts

Line stepping needs, at each instruction, its line and the call depth.
From those, `std.debug.Stepper` decides when a command is done:

| Command | Stops at the first instruction with |
|---|---|
| `step` | another line, or another depth (a call or a return) |
| `next` | another line at the same depth, or a smaller depth |
| `finish` | a smaller depth |

Lines are compared within a frame, so a loop that jumps back to its own
line does not stop (gdb's behavior).

The interpreter supplies line and depth, because the vm declaration does
not know where the line table or the frame stack are. Mini's depth is
the frame index plus the frames of the coroutines' resumers, so `next`
over `coroutine.resume` steps over the coroutine. Rejected: a vm-level
`debug_info { line, depth }` construct; it would save two small
functions.

### 3.2 The hook

Stepping runs under the same variant as the step-over, so every
instruction's wrapper asks the stepper. Two compiler features make it
see every instruction:

- **`#[unfused]` on the variant**: under it a superinstruction runs its
  parts one at a time through the variant table, so a line that starts
  at a super's later part is seen. The default table still runs supers
  fused.
- **`use_variant` where runs start and continue.** A nested run (a
  `table.sort` comparator) starts in the default table. Mini's `call`
  and `resume` entries and its `reload` block turn the variant on when
  the debugger is stepping: three `#[cfg(debug)]` lines.

The variant turns itself off when nothing is pending, so `continue` runs
at full speed.

## 4. Inspection

- **Op level, generated**: the current instruction, disassembled
  (`Mini.disasm_insn`, `Vm.op_name`, `Vm.operand_kind`).
- **Frames, the interpreter's**: Mini walks its frame stack; a caller's
  line is that of its CALL. Under `--cfg debug`, `sync` saves `pc` for a
  frame calling a native.
- **Locals, the interpreter's**: under `--cfg debug`, Mini's compiler
  records each local's register and live range, like Lua's `locvars`.
- **Values** are printed without addresses, so transcripts are
  deterministic.

## 5. Embedding: a C API

Mini's stop-callback C API and the DAP adapter on it are in section 9.

## 6. Cost

**Not debugging.** The debugger is a build option, `--cfg debug`,
because a vm with a variant carries its dispatch table in a state
register, and `reload` adds a check after each re-entering native:
small but real costs in the hot loop. Without the option every example
compiles to byte-identical IR in all three dispatch modes. (`loimi
ir-diff` ignores source positions, so `#[cfg(debug)]` items go on lines
of their own.)

**Compiled in.** The 14 Mini benchmarks, best of 5, interleaved,
tail-call dispatch, x86-64, shared machine (noise a few percent):

| Program | default (s) | cfg-debug | attached | stepping |
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

- *cfg-debug*: the debug build, not debugging.
- *attached*: `--debug`, `continue` at the first stop (no breakpoints).
- *stepping*: `finish` at the first stop, so the whole program runs
  under the step variant.

Having the debugger costs nothing measurable. *stepping* is what design
2 would cost for a whole session; design 1 pays it only while a stepping
command runs.

## 7. Limitations

- Clear breakpoints (`Breakpoints.clear_all`) before serializing or
  verifying code. A code cache must not save breakpoint ops.
- No JIT with `on break`.
- Stack vms reject `on break`: a breakpoint op would need the stack
  effect of the instruction it replaces.

**Why generate the instruction level?** It is about 150 lines of Rust,
and it covers what a hand-written version gets wrong: patched
instruction sizes, supers, caches, verification. A hand-written `BREAK`
op cannot be sound, because it would have to forge proofs.

## 8. Second interpreter: Python

To test whether the generic part is generic, a debugger was added to
`examples/python` (`loimi build --cfg debug examples/python/main.lm`,
then `python-loimi --debug prog.py`). It behaves like pdb, with gdb's
`finish`. The interpreter differs from Mini where a debugger cares:

- generators are frames on the shared frame array;
- exceptions unwind through handler tables, with `try`/`finally`;
- compare-and-branch ops are `then`-fused with the following JMP;
- four inline cache layouts and four supers;
- some builtins are Python code (a prelude, and `io`);
- natives start nested runs (`sorted` keys, `next(g)`, `__repr__`).

`examples/python/debugger.lm` is 554 lines (Mini's was 333 before
section 9's additions), plus 25 `#[cfg(debug)]` items (Mini: 15). The
extra is display and locals, not stepping.

### 8.1 What fitted as it was

All of the instruction level and `std.debug`'s `Breakpoints`,
`Stepper`, `Input` and `parse_command` worked unchanged, including
breakpoints on cached and fused instructions. Generators needed nothing
special: a resumed generator is one frame above its resumer, so the
frame index is the right depth. Exceptions needed nothing either:
`on raise` jumps to the handler through the current table, so `next`
over a raising call stops at the `except` or `finally`.

### 8.2 What did not fit, and what changed

| Problem | Fix | Where |
|---|---|---|
| A frame leaving to a native | `Stepper.left(depth)` | `std.debug` |
| One stop per `for` loop instead of per iteration | `debug.line_sites` | `std.debug` |
| Breakpoints hide jump labels from code walks | `debug.unpatched` | `std.debug` |
| Instructions with no real line | line 0 means "no line" | `std.debug` |
| `#[cfg]` on a struct literal's field | `#[cfg(..)]` before a field initializer | compiler |
| Prelude code shown to the user | the interpreter skips it | Python |
| Locals not cleared on function entry | record the first store of each name | Python |
| `repr` may run guest code | the debugger's own value printer | Python |

1. **A frame leaving to a native.** `next` at the last line of a
   `sorted` key stopped in the key's next call: same depth, and depth
   alone cannot tell the frames apart. Mini had the same bug with
   `table.sort`. `Stepper.left` is called where a nested run ends.
   Rejected: a frame serial number, which costs a store per call.
2. **Line starts.** Python's `for` jumps back into the middle of its
   line's instructions, so a site at the line's first instruction
   stopped once per loop. `debug.line_sites` puts the site at the loop
   head (the target of a backward jump from another line), giving pdb's
   stops.
3. **Code walks.** A breakpoint op has the right size but not the
   original's label, so a breakpoint on a loop's jump back hid the loop
   head. `debug.unpatched` copies the code with the original words.
4. **No line.** Python's compiler gave a loop's jump back the line of
   the body's last statement, so `next` could stop on a line that did
   not run. `Stepper` never stops at line 0 (like CPython's PEP 626),
   and Python's debug build gives jumps back line 0.
5. The other four are local to the interpreter or the compiler, as the
   table says. In particular, locals are shown from their first store in
   code order, because clearing them on entry would cost every call.

Mini gained the `left` hook (one line); its default build's IR did not
change.

### 8.3 Cost

Without `--cfg debug`, the Python interpreter's IR is byte-identical.
With it, on the eleven pyperformance programs (best of 3, tail-call
dispatch): cfg-debug 1.00 and attached 1.00 in total (0.97–1.04 per
program), stepping 2.70 (1.90–3.98). Stepping costs less than in Mini
because Python's instructions do more work each.

### 8.4 Verdict

The instruction level carried over unchanged. Every misfit was at the
guest level, in what "a line starts here" and "this frame" mean; four
fixes were general enough to move into `std.debug`, and one was a
compiler gap.

## 9. Editors: a stop-callback API and a DAP adapter

Mini's C API adds what an editor needs: conditional and hit-counted
breakpoints, break on error, pause, expression evaluation, and a DAP
server (`examples/mini/dap/mini_dap.c`) for VS Code or any DAP client.

### 9.1 The API

```
loimi build --cfg debug examples/mini/embed.lm --lib --header mini.h
```

| Call | What it does |
|---|---|
| `mini_debug_attach(s, cb, ud)` | `cb(s, ud, reason, bp) -> MiniResume` runs at each stop |
| `mini_debug_load(s, name, src)` | compile a chunk named `name` and push it, for `mini_call` |
| `mini_debug_stop_on_entry(s)` | stop before the chunk loaded last starts |
| `mini_debug_break_on_error(s, on)` | stop at uncaught errors, before they unwind |
| `mini_debug_set_break(s, file, line) -> id` | a breakpoint, in code loaded now or later |
| `mini_debug_condition(s, id, cond, hit)` | a Mini expression and a hit condition (`N`, `>= N`, `% N`, ...) |
| `mini_debug_code_line(s, file, line)` | the first line at or after `line` with code |
| `mini_debug_break_sites`, `mini_debug_delete` | list and delete breakpoints |
| `mini_debug_pause(s)` | stop soon; **the one call another thread may make** |
| `mini_debug_message`, `_depth`, `_frame` | at a stop: the error or failed condition; frames |
| `mini_debug_scope`, `mini_debug_variables` | at a stop: locals and globals, expanded lazily through handles |
| `mini_debug_eval(level, expr, &out, &child)` | at a stop: an expression or statement in that frame |

The callback returns `MiniResume_Continue`, `_Step`, `_Next`, `_Finish`
or `_Keep`. `_Keep` continues as before the stop (a `next` in progress
goes on), for a host that stopped the program for its own work.

The callback runs on the interpreter's thread, inside the stopped
instruction: it is the command line's loop with C calls in place of
commands, and both share one core in `debugger.lm`.

### 9.2 Conditions: running guest code from inside a stop

A condition, like `mini_debug_eval`, evaluates a Mini expression in the
stopped frame. The program is stopped inside a handler, so this is a
nested run from there. Three problems:

1. **Compiling against the frame.** Mini has no `load` with an
   environment, so the expression is compiled as a function whose
   parameters are the visible locals, and called with their values:
   `return function(n, total, i) return (EXPR) end`. Locals are
   therefore read-only copies; globals are real, so `done = true` works.
2. **Running nested from a handler.** The hooks are `uses reenter`, so
   the compiler puts `sync` before and `reload` after them, as for a
   native that calls back. The vm needed no other change.
3. **Turning the debugger off meanwhile.** A breakpoint hit inside the
   evaluation must not stop, nor disturb the step-over in progress
   (it would loop forever on the stopped breakpoint).
   `Breakpoints.suspend` takes every site but the pending one out of the
   code, and `unsuspend` puts them back. Rejected: a flag to skip the
   stop, which would need two step-overs at once.

A condition that fails to evaluate stops with the error (VS Code's
behavior). Hit counts count the hits at which the condition held.

### 9.3 Pause

`mini_debug_pause` only stores a word (`std.debug.request_pause`, a
relaxed atomic store, like CPython's eval breaker). At a poll the
interpreter switches to the step variant, whose wrapper sees the flag
and stops.

Rejected:

- patching breakpoints in from the other thread: races with `become`
  and the step-over;
- a signal handler that patches: it may interrupt `hit` or `malloc`.

Polls are in debug builds only, on the back edges of loops (`JMP`,
`FORLOOP`, `NEXT`, `INEXT`) and on `CALL`. `repeat ... until` compiles
in debug builds to a conditional exit and a `JMP` back, so every loop
polls. Polling conditional jumps too cost 1.2% in total and 4–5% on
branchy programs; without them, 0.5%.

### 9.4 Break on error

Mini's `on raise` sees every error at the instruction that raised it,
with frames intact. The hook stops there if nothing will catch the
error. Mini has no `pcall`, so an error is uncaught unless some
coroutine between the running one and the main thread was started by
`coroutine.resume` (which returns `false, msg`).

### 9.5 The adapter

`mini_dap.c` (770 lines of C, including JSON) runs the script in the
same process, with three threads:

- **Main**: handles requests, runs the chunk, and at each stop sends
  `stopped` and handles requests until one resumes.
- **Reader**: queues requests. While the script runs, any request other
  than `threads` calls `mini_debug_pause`. For a `pause` request the
  user sees a stop; for others (VS Code sends `setBreakpoints` while
  running) the callback handles the queue and returns `_Keep`.
- **Pump**: turns the script's stdout (fd 1, redirected to a pipe) into
  `output` events, drained before each stop so output comes first.

It supports launch, breakpoints (conditional, hit-counted, moved to the
next line with code), the "Uncaught errors" exception filter, stack
traces, scopes and paged variables, evaluate, the step requests, pause
and terminate. Not supported: `attach`, `setVariable`, `restart`,
`loadedSources`, `breakpointLocations`. Coroutines show as one thread,
and there are no columns.

### 9.6 Tests

- `examples/mini/dap/test_dap.py` drives a whole adapter session and
  compares its transcript (golden test `mini_dap`, all three dispatch
  modes).
- `examples/embed/debug.c` tests the C API directly under GC stress
  (golden test `mini_debug_api`).
- `examples/mini/tests/debug` holds the command line's transcripts.

### 9.7 Cost

Mini's default build stays byte-identical. In the debug build, the
pause polls and the `repeat` change cost 1.005 (geometric mean over the
14 benchmarks, not debugging). fannkuch, whose inner loops are
`repeat`, was up to 4% slower. A conditional breakpoint costs a compile
and a nested run per hit.

### 9.8 What is generic

`std.debug` gained, for any interpreter: `Reason`, conditions and hit
conditions on `Bp`, `Breakpoints.suspend`/`unsuspend`, `Refs` (handles
for lazy expansion), `request_pause`/`take_pause`, and `break .. if` in
the command reader. What stays Mini's: compiling an expression against a
frame, deciding which errors are uncaught, showing values, and where the
polls go.
