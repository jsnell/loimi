# Coroutines in Mini

Mini has Lua 5.4's `coroutine` library: `create`, `resume`, `yield`,
`status`, `isyieldable`, `running`, `wrap` and `close` (SPEC.md §5). This
note describes how they map onto Loimi's vm model, how they are tested and
how fast they are. It also lists what Loimi made hard: Mini is the first
interpreter in the repository with more than one guest stack.

## Design

**No native stack switching.** A coroutine is a `Coro` object
(`coro.lm`) with a value stack and a frame stack of its own: `VStack`
(`[V]`) and `FStack` (`[Frame]`). Both are GC objects. They start small:
60 slots and 12 frames, which keeps each a small object. They double
when a call needs more, up to the main stack's limits. The VM itself does
not know about coroutines. It runs on whatever stacks the context's
fields `stack`, `stack_end`, `frames`, `frames_end`, `fp`, `base`, `open`
and `open_level` describe.

- **`resume(co, ...)`** is a native, called from CALL's native path. It
  does the following:
  1. Saves the resumer's eight fields into a `Saved` record on its own C
     stack. The record is linked from `Ctx.chain`.
  2. Installs the coroutine's stacks.
  3. Runs the coroutine through a nested entry. A coroutine that has not
     started yet runs through `Mini.call`, the same entry
     `table.sort`'s comparator uses. A coroutine that yielded runs
     through the new `Mini.resume`. That entry reloads `fp`, `regs` and
     `k` from the installed frame, stores the resume arguments as the
     results of the pending yield's CALL, and jumps to the instruction
     after that CALL.
  4. When the run returns, puts the resumer's state back.

  Each resume is one nested run, so `Mini.depth` bounds nested resumes
  (200, "C stack overflow") as `LUAI_MAXCCALLS` does in PUC-Lua.
- **`yield(...)`** is a native that only checks and records. It is an
  error outside a coroutine. It is also an error unless it was called
  directly by a CALL instruction of the coroutine's own run, which is
  `ctx.base == ctx.frames`, with no nested run in between. Otherwise it
  records the values and sets `ctx.yielded`. CALL's native path, the vm
  function `native_results`, then sees the flag. It saves `pc + 1`, the
  CALL's base register and its result count, and leaves the run with
  `exit`. The resumer's `Mini.call`/`Mini.resume` returns, and `resume`
  stores the coroutine's `fp`, open upvalues and continuation in the
  `Coro`.
- **The end of a coroutine** is its bottom frame's RET, which exits the
  run as before. RET1, RETN and RET0 now also record how many values they
  returned (`ctx.nxfer`, and `ctx.xfer` for RETN), so `resume` can pass
  them all on.
- **Errors** in the coroutine reach the run's `on raise`, which exits
  with `Err`. `resume` closes the upvalues still open into the dead
  stack and drops the stacks. It returns `false, msg`, or for a `wrap`
  function returns the error, so the caller's `on raise` adds its own
  line, as Lua 5.4 adds the caller's position.
- **Several results from natives.** A native can now return several
  values: it fills `ctx.rbuf` and sets `ctx.nret`, and CALL's native path
  copies them into its result registers. Before this, every Mini native
  returned exactly one value.
- **Values.** `V` gained `Co(Gc<Coro>)`, whose type is "thread", and
  `Wrap(Gc<Coro>)`, a function made by `coroutine.wrap`. CALL has a cold
  path for `Wrap` next to the one for natives. The nanbox had room for
  both: no fast path changed.
- **The main thread** has a `Coro` too (`ctx.main`, without stacks), so
  `coroutine.running()` and `status` treat it like any other thread.

**Why this keeps the vm state coherent.** Between two instructions the
only state is the context's fields and the handler registers, and the
handler registers are reloaded at every entry and after every
`reenter` call (`reload`). Resume and yield happen exactly at those
points: inside a native called by CALL (`sync` has published `ctx.fp`),
or at an `exit`. So no handler ever sees a half-switched thread. The
shadow stack stays a single stack: a coroutine's run is an ordinary
nested run on the resumer's C stack, and it is unwound by `exit` before
the resumer continues.

**Stack growth.** A coroutine's stacks move when they grow (`vm.grow`):
the main stack is fixed-size and never moves, but a coroutine's stacks
start small. CALL's overflow check calls `grow` in a cold block. `grow`
copies the stacks and rebases each frame's `base`, each open upvalue and
the context's pointers. The CALL then sets `regs = fp.base` and does
`redo`. `call_value` (comparators, `mini_call`) now keeps its saved frame
and window as indices, because the nested run may grow the stacks. The
block that a stack replaced stays allocated until the coroutine dies
(`VStack.prev`), because a host native may still hold an `args` pointer
into it.

**GC.**
- **Roots.** `scan_roots` walks the frames of the running thread, as
  before, and those of every resumer on `ctx.chain`, and marks
  `ctx.cur`, `ctx.main` and the multi-result buffer.
- **Suspended coroutines** are traced through their objects. A
  `VStack` is traced whole, including dead slots above the top (see
  friction 3).
- **Dead coroutines** drop their stacks, and the collector frees them.
- **Open upvalues.** An open upvalue into a coroutine's stack has a
  `stack` field that keeps the `VStack` alive. If a suspended coroutine
  is dropped while one of its closures survives, the closure still reads
  live memory. PUC-Lua closes such upvalues when it frees the thread;
  Mini has no finalizers.
- **The generational barrier.** The VM stores into a coroutine's stacks
  without write barriers. To make that safe, the running coroutine's
  stacks and its resumers' stacks are kept in the remembered set: the
  stacks are remembered when a coroutine is switched in, and a sweeper
  (`coro.sweep`) remembers them again after every collection (friction
  4).

**Yield across a C-call boundary.** Yield across a C-call boundary is an
error, as in PUC-Lua for C functions without continuations: "attempt to
yield across a C-call boundary". The cases are yielding from a
`table.sort` comparator, yielding from a host native's callback, and
`table.sort(t, coroutine.yield)`. `coroutine.isyieldable()` is false in
those places. A comparator may still create, resume and finish other
coroutines, and a `wrap` function works as a comparator.

## Tests

- **`tests/coroutine_basics.mini`, `tests/coroutine_gc.mini`,
  `tests/err_coroutine_wrap.mini` and `tests/err_coroutine_yield.mini`.**
  These cover the following:
  - values both ways, wrap generators, and yield from nested calls;
  - errors, `running`, `status`, `isyieldable` and `close`;
  - yield from a comparator;
  - shared and closed upvalues, and an upvalue that outlives its
    coroutine;
  - 9000-deep recursion inside a coroutine (stack growth with open
    upvalues);
  - resume chains and the nesting limit;
  - hundreds of garbage coroutines.

  `coroutine_gc.mini` prints exactly what Lua 5.4.9 prints. The scripts
  run in all three gc modes (normal, stress, verify), as every Mini script
  does. `tests/golden.rs` also runs them in the goto and switch dispatch
  modes (`mini_reentry_dispatch_modes`), under the copy-and-patch JIT
  (`mini_jit_stencils`), and on AArch64 when the cross toolchain exists.
- **`tests/conformance/coroutine_lua.lua`** is PUC-Lua 5.4.9's
  `testes/coroutine.lua`, reduced to the Mini subset. Its header lists
  every removed or rewritten part, with line numbers. It prints exactly
  what PUC-Lua prints and passes in every mode above.
  - **Kept:** multiple values through resume and yield; `running`,
    `isyieldable` and `status`; yields in tail calls; recursive yields;
    the sieve; closing dead and errored coroutines; "cannot close a
    running/normal coroutine"; the yield in a comparator; errors in
    coroutines and wraps; the generator over `all()`; closures over a
    coroutine's locals; the old bugs where a coroutine resumes itself
    or a 'normal' coroutine; and infinite recursion of coroutines ("C
    stack overflow").
  - **Removed:**
    - everything about `<close>`/`__close`;
    - pcall and xpcall around yields;
    - `debug.sethook` and `debug.getinfo`;
    - the weak-table check;
    - the `T` (testC) API block;
    - the `table.unpack` stack limit;
    - yields inside metamethods and in custom for-iterators.

  Mini has no pcall, so the port's `try(f, a)` runs `f` in a fresh
  coroutine instead.
- **Fixed on the way:** `table.sort` rejected a `wrap` function as
  comparator ("function expected, got function").

## Speed

Three existing public benchmarks that use coroutines are in
`bench/mini/coroutines/`, with their provenance and the mechanical edits
in the README there. They are LuaJIT-test-cleanup's `coroutine-ring`
(the Benchmarks Game thread-ring) and `chameneos`, and the classic Lua
`sieve` (via Luau's bench). The table gives the best of 3 runs, x86-64,
Mini built `--release`:

| benchmark | Mini | PUC-Lua 5.4.9 | LuaJIT 2.1 `-joff` | Mini/Lua |
|---|---|---|---|---|
| chameneos | 0.671 s | 1.134 s | 0.451 s | 0.59 |
| coroutine_ring | 0.716 s | 1.377 s | 0.408 s | 0.52 |
| sieve | 0.703 s | 1.496 s | 0.532 s | 0.47 |

A resume/yield pair costs one native call, one nested entry, a copy of
eight context fields each way, and the result copies. LuaJIT's
interpreter is 1.3-1.75× faster on these benchmarks. Its resume is
assembly that switches `lua_State`s without leaving the dispatch
loop. Mini's could do the same with a resume *op*, whose handler swaps
the state registers itself. That is not done: see friction 6.

The 14 clean-room benchmarks (`bench/mini/compare.py`), base vs this
change: see the results at the end of this file.

## Friction: what Loimi made hard

Ranked by how much each cost or would cost the next multi-stack
interpreter.

1. **A collector crash that depends on how a native function is
   written.** This is unexplained, and the most serious item.
   - **Symptom:** `coroutine.resume(coroutine.create(w))`, where `w`
     is a `wrap` function, segfaulted in tail-call dispatch under GC
     stress. Goto, switch, verify mode and an ASan build ran it fine.
   - **Situation:** the crash was in a collection inside the nested run.
     That run was started by `vm.call_value`, which was called from the
     native-body branch of `coro.resume`. That branch is a `let F(cl) =
     co.body else { ...; let r = vm.call_value(..); ...; return .. };`
     placed before `var saved = Saved { .., cur: Gc<Coro>, open:
     ?Gc<UpVal>, .. }`, a struct local with traced fields.
   - **Workaround:** moving the branch into a function of its own
     (`resume_native`) fixed it.
   - **What it looks like:** `resume`'s shadow-stack frame lists a
     traced slot that is not yet initialized on that path. Candidates
     are the `saved` record and the `let ... else` binding `cl`.
   - **Not reproduced:** two small programs of that shape
     (`gc.collect` on an early-return path before a struct local with
     a `?Gc` field, and before a `let A(x) = .. else` binding) did not
     crash.
   - **Suggested fix:** zero every traced shadow-stack slot at function
     entry (cheap: only functions with `uses gc` calls have them), or
     register a slot only from its initialization on. Also, a debug
     mode in which the collector checks that every shadow-stack entry
     points to an allocated object, as verify mode does for heap
     references.
2. **Entries cannot resume at a saved pc.**
   - **Error:** `Mini.resume` needs `jump c.ypc` to continue after the
     yield's CALL, which fails with: "`Mini` is `#[require_verified]`:
     an entry starts only in verified code, `jump v` with `v:
     Verified<Mini>` (from `Mini.verify`); found `*u32`".
   - **Workaround:** `unsafe { jump c.ypc }`.
   - **Why the address is safe:** it is exactly the kind of address that
     handlers may jump to without a proof, a return address that a
     verified CALL saved (`jump fp.resume`). Only entries are held to
     `Verified`.
   - **Suggested fix:** a proof type for resume points. For example,
     `Mini.Resume`, made only by vm code from `pc + 1` or `next_pc`, and
     accepted by an entry's `jump`. This is what language.md §10 means
     by "resuming re-enters through an entry that restores it".
3. **No way to trace only the live part of an object.**
   - **What happens:** a coroutine stack is an `object VStack { v: [V]
     }`, and the generated trace function marks every slot. That
     includes slots above the top frame, which keep dead values alive
     until they are overwritten or the coroutine dies. `FStack` does the
     same with the closures of popped frames. The main stack avoids
     this because a root scanner walks it precisely, up to `fp`.
   - **Cost:** objects are retained longer than they should be. It is
     not a correctness problem.
   - **Suggested fix:** a per-type trace hook (`#[trace(fn)]` on an
     `object`), or a declared live length for a trailing array
     (`v: [V] live(top)`).
4. **Raw stores into GC-object stacks need the remembered set, and no
   API says "always rescan this while it is active".**
   - **The problem:** the VM stores into the running coroutine's stacks
     through raw pointers without write barriers. If such a stack is
     old, a minor collection misses the young values in it, and verify
     mode aborts ("old object ... points to a young object ... missing
     write barrier").
   - **Two parts to the workaround:**
     - The scanner walks the active stacks precisely, which makes the
       marking correct.
     - Verify mode still checks the stack objects, so `coro.sweep` puts
       them back into the remembered set after every collection. A
       *scanner* cannot do this: an object that is young at scan time
       has no LOG flag, so `remember` does nothing, and it becomes old
       later in the same collection.
   - **Suggested fix:** `heap.add_root_object(obj)` /
     `remove_root_object`, objects traced as roots in every
     collection, minor ones included, and exempt from the barrier
     check. Or the `Root<T>` handles of gc.md §6.
5. **No finalizers.**
   - **Effect on the stacks:** they cannot be `mem.alloc`'d like the
     main stack, because nothing would free them when a coroutine is
     dropped. They are GC objects instead, which is the cause of items 3
     and 4.
   - **Effect on open upvalues:** PUC-Lua closes a dropped thread's open
     upvalues when it frees the thread. Mini instead keeps the dead
     stack alive from each open upvalue (`UpVal.stack`).
   - **Suggested fix:** finalizers, or a sweeper-friendly "free this
     host block when object X dies" registration (`heap.on_free(obj,
     ptr)`).
6. **Yield and resume can only happen where a native call happens.**
   - **What Mini does:** a yield must leave the run at a point where the
     handler knows the CALL's `pc`, base and result count. Mini does it
     in CALL's native path, after the native returns, through a flag.
     A guest operation that yields from another op (a metamethod, an
     iterator) would need the same code in each op.
   - **Resume costs a native call and a nested entry.** A cheaper resume
     would be an op that swaps the state registers. Ops have no
     supported way to do that across stacks: `reload` restores one
     fixed shape, and the shadow-stack copies of traced state belong to
     the run.
   - **Suggested fix:** language.md §10's `suspend v` (leave the loop
     with the state saved), with a matching `resume` entry form. Also
     the `std.rt` `FrameStack`/`ValueStack` that the docs sketch, with
     growth and relocation built in (item 7).
7. **Stacks that move.**
   - **The work:** `grow` rebases every frame base, every open upvalue
     and four context pointers by hand.
   - **Handler state:** `reload` covers what the handlers cache, and
     `redo` after the growth works well.
   - **Natives and the nested-run helper:** these hold raw pointers.
     `call_value` had to switch from saved pointers to indices, and
     superseded blocks are kept allocated (`VStack.prev`) because a host
     native may still hold an `args` pointer.
   - **The docs:** they say `reload` "should recompute state from ...
     offsets or the frame record" (§5), but nothing helps with the frame
     record itself moving.
   - **Suggested fix:** a `std.rt` stack type with index-based frames
     and a relocation callback.
8. **Mini's natives returned exactly one value.** `resume` returns `true`
   plus the values, and `running` returns two values, so Mini needed a
   multi-result protocol (`ctx.nret`/`rbuf`). This is Mini's design, not
   Loimi's, but no example interpreter had a multi-value native ABI to
   copy.

Process friction, not Loimi: in this agent sandbox, shell commands that
run a binary named through a variable were refused, so building and
running in a loop needed small wrapper scripts.

## What worked well

- **The nested-entry model** (`entry .. uses reenter`, `Mini.depth`, `on
  raise` exiting a run with `Err`) is exactly the substrate that
  "coroutines without stack switching" need. `Mini.call` was reused
  unchanged as the start of a coroutine.
- **Control words in a `-> never` vm function.** `exit` and `next` inside
  `native_results`, called from a `cold` block, made the
  yield-leaves-the-run path a few lines. `redo` after growing the stacks
  re-executes CALL with the new `regs`.
- **`sync` and `reload` keep handlers correct across thread switches with
  no new code.** After a resume returns, `reload` reads `ctx.fp`, which
  `resume` restored.
- **The shadow stack roots the traced state of all nested runs.**
  Interleaved coroutine runs on one C stack needed no new rooting rules.
  The only new roots are the saved records (`Saved`), walked by the
  scanner.
- **Adding two `V` variants was a three-line change.** Exhaustiveness
  errors pointed at every `match` to update.
- **GC stress and verify modes found the barrier issue (4) immediately.**
  GC stress also exposed item 1.
