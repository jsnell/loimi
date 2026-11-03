# Coroutines in Mini

Mini has Lua 5.4's `coroutine` library: `create`, `resume`, `yield`,
`status`, `isyieldable`, `running`, `wrap` and `close` (SPEC.md §5). This
note describes how they map onto Loimi's vm model, how they are tested and
how fast they are. It also lists what Loimi made hard: Mini is the first
interpreter in the repository with more than one guest stack.

## Design

**No native stack switching.** A coroutine is a `Coro` object
(`coro.lm`) with a value stack and a frame stack of its own, in host
memory (`mem.alloc`, like the main stack) that the object owns. They
start small, 60 slots and 12 frames, and double when a call needs more,
up to the main stack's limits. The VM itself does
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
- **Values.** `V` gained one variant, `Co(Gc<Coro>)`. It is a thread,
  or a function made by `coroutine.wrap`: a separate `Coro` whose `wrap`
  field is the coroutine it resumes, so that it differs from the thread
  that `coroutine.running()` returns inside. CALL has a cold path for it
  next to the one for natives. Only one variant was added, not two:
  friction 2 explains why.
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
block that a stack replaced stays allocated until the resume returns
(`Coro.retired`), because a native of the coroutine's run may still hold
an `args` pointer into it; once the run has yielded or ended, none can.

**GC.**
- **Roots.** `scan_roots` walks the frames of the running thread, as
  before, and those of every resumer on `ctx.chain`, and marks
  `ctx.cur`, `ctx.main` and the multi-result buffer.
- **Suspended coroutines** are traced through their objects: `Coro`
  has a `#[trace(trace_coro)]` hook (std.gc's trace hooks), which walks
  the frames of a suspended coroutine and marks each frame's closure
  and registers, as `scan_roots` does for the running thread. Slots
  above the top frame are not traced. A running or normal coroutine's
  hook marks nothing: its stacks are roots.
- **Dead coroutines** free their stacks at once (`die`). A suspended
  coroutine that is dropped frees them when the collector finds the
  `Coro` dead: `resume` registers a free hook (`heap.on_free(co,
  free_coro)`) when it allocates the stacks. The stacks count towards
  the next collection (`heap.allocated`), as they did when they were
  heap objects.
- **Open upvalues.** An open upvalue into a coroutine's stack has a `co`
  field that keeps the `Coro`, and so its stacks, alive; closing it
  drops the reference. If a suspended coroutine is dropped while one of
  its closures survives, the coroutine stays alive with it, and the
  closure reads live memory. PUC-Lua closes such upvalues when it frees
  the thread instead (friction 6).
- **The generational barrier.** The VM stores into a coroutine's stacks
  without write barriers. While it runs (or is normal), the stacks are
  roots, which every collection scans, minor ones included. When it
  suspends, `resume` logs the `Coro` (`gc.write_barrier`), so the next
  minor collection traces it through the hook, and verify mode checks
  it from then on.

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
| chameneos | 0.574 s | 1.010 s | 0.382 s | 0.57 |
| coroutine_ring | 0.529 s | 1.018 s | 0.381 s | 0.52 |
| sieve | 0.574 s | 1.116 s | 0.389 s | 0.51 |

A resume/yield pair costs one native call, one nested entry, a copy of
eight context fields each way, and the result copies. LuaJIT's
interpreter is 1.4-1.5× faster on these benchmarks. Its resume is
assembly that switches `lua_State`s without leaving the dispatch
loop. Mini's could do the same with a resume *op*, whose handler swaps
the state registers itself. That is not done: see friction 7.

**Stacks in host memory instead of the heap** (the GC hooks of items 4-6,
2026-11-03): best of 5, interleaved, both builds `--align-functions=64`,
new/old: chameneos 1.000, coroutine_ring 0.996, sieve 0.986. On the 14
clean-room benchmarks (best of 3) the geomean is 0.996, every benchmark
within ±3% except `oo_methods`, 1.03 in repeated runs, which runs no
coroutine and no collection and whose handlers differ only in field
offsets and type ids (placement). The other way, keeping the stacks as
heap objects with a `#[trace]` hook for their live part and
`pin_dirty` while active, was not built: it needs a pin and an unpin
per switch, the hook needs the frames (a separate object) to know
which slots are live, and growth would still allocate heap objects
that the old stack keeps alive. Host memory needs one barrier per
suspend and no sweeper.

**No regression on the 14 clean-room benchmarks.** These are
`bench/mini/compare.py`, run interleaved, best of 3, both builds with
`--align-functions=64`. Base is the commit before coroutines. The
geomean of new/base is 1.005, and every benchmark is within ±3%: 0.97
(fib) to 1.024 (queens). The first version, which added two `V`
variants, measured 1.007 by geomean but 1.04-1.06 on `hash_strings`,
`interp` and `queens` (friction 2).

## Friction: what Loimi made hard

Ranked by how much each cost or would cost the next multi-stack
interpreter.

1. **A collector crash that depended on how a native function was
   written: a compiler bug in the rooting of `let ... else`, now
   fixed.**
   - **Symptom:** `coroutine.resume(coroutine.create(w))`, where `w`
     is a `wrap` function, segfaulted in tail-call dispatch under GC
     stress. The crash was in a collection inside the nested run that
     `vm.call_value` started from the native-body branch of
     `coro.resume`, written inline as `let F(cl) = co.body else { ...;
     let r = vm.call_value(..); ...; return .. };`. Moving the branch
     into a function of its own (`resume_native`) made it go away.
   - **Cause:** the compiler added the bindings of `let PAT = e else {
     .. }` to the set of rooted locals when it generated the pattern
     test, before the else block. So every call that may collect in
     the else block listed the bindings' slots in its shadow-stack
     frame, although on that path they were never assigned: they held
     whatever the stack held before, or an earlier loop iteration's
     value. Here the slot of `cl` held a stale stack address; an ASan
     build of the inline version reports `mark_ptr` reading the
     "header" below a local array of `main`. Whether the garbage was
     harmless (null, or a live object) depended on what ran earlier on
     the stack, hence tail-call dispatch only and the dependence on
     how the code was split. The workaround did not remove the bug:
     the call `resume_native(..)` in the else block still rooted the
     unassigned `cl`, the garbage there was just benign.
   - **Fix:** the bindings are rooted from the end of the pattern
     test on (src/codegen.rs, `TStmt::LetPat`). The only other change
     in the IR of the six example interpreters is `coro.resume`, whose
     else-block call now has one root instead of two.
     `tests/run/gc_roots_let_else.lm` crashed before the fix in every
     dispatch mode (a function and a handler with `let .. else` whose
     else block collects, after a call that leaves pointer-like junk on
     the stack) and also checked that a loop's else block no longer
     keeps the previous iteration's object alive. Other pattern forms
     (`match` arms, `if let`, `while let`) already dropped their
     bindings before the failure path.
   - **Lesson for the collector:** an uninitialized or stale root is
     invisible until the garbage stops being benign. ASan catches it
     at the first scan if the slot happens to hold a stack address;
     verify mode does not check roots.
2. **Adding a value variant silently changed the encoding of the others,
   and broke a fast path.**
   - **What happened:** Mini's `V` had exactly 7 tagged variants. Adding
     `Co` and `Wrap` made 9, which switched the nanbox from the classic
     layout to the packed one. There, `Int(i32)` gets a 32-bit block
     instead of a 48-bit one.
   - **Two effects:**
     - *Correctness.* Mini's compare-and-branch fast path tests "both
       Ints" as `(a ^ bias) | (b ^ bias) < 2^48`. In the packed layout
       that also accepts Nil and Bool, so `tests/compare_jumps.mini`
       printed a wrong checksum. No compile-time diagnostic flagged it.
     - *Speed.* Tuple patterns such as `(T(tab), Int(_))` no longer merge
       into one test, because the elements have different block widths.
       `hash_strings`, `interp` and `queens` ran 4-6% slower. Measured
       with dummy variants on the base: the layout alone accounts for it.
   - **Fixes made:**
     - The fast path now tests `< 2^32`, which is right in both layouts.
     - Compiler (`src/values.rs`): with exactly eight tagged variants and
       one of them at most 47 bits wide, seven keep the classic blocks
       and the narrowest takes the 47-bit block. The layout is documented
       in lowering.md §2 and tested by `tests/run/values_nanbox_eight.lm`.
       No other value declaration in the repository has exactly 8
       variants.
     - Mini merged its two new variants into one `Co`.
   - **Suggested fixes:**
     - Pinned tags or a minimum block width per variant (`Int(i32)
       block 48`), which language.md lists as not built.
     - A way for code to ask for a variant's block width
       (`V.Int.width`), so that tricks like the int bias need not
       hard-code 48.
     - A note when a declaration crosses from the classic to the packed
       layout.
3. **Entries cannot resume at a saved pc.**
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
4. **No way to trace only the live part of an object.** *Resolved:*
   std.gc's `#[trace(f)]` object hooks (stdlib/gc.md, "Trace hooks, free
   hooks and dirty pinning"); `Coro`'s walks a suspended coroutine's
   frames. As it was:
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
5. **Raw stores into GC-object stacks need the remembered set, and no
   API says "always rescan this while it is active".** *Resolved* twice:
   std.gc has `heap.pin_dirty(obj)`/`unpin_dirty` (a root that every
   collection traces, minor ones included, exempt from the barrier and
   from verify mode), and Mini no longer needs it, because its stacks
   left the heap (item 6): an active coroutine's stacks are roots, and a
   suspended one's `Coro` is logged once when it suspends. `coro.sweep`
   is gone. As it was:
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
6. **No finalizers.** *Resolved for the stacks:* std.gc's free hooks
   (`heap.on_free(obj, f)`: `f` runs once when `obj` is found dead, and
   at `Heap.free`; it may release host memory but not allocate, collect
   or follow references). The stacks are `mem.alloc`'d and owned by the
   `Coro`. *Not for open upvalues:* closing a dropped coroutine's open
   upvalues from a free hook would store values from the dead stack,
   which no one marked, into live upvalues during a collection. A
   free hook is deliberately too weak for that; PUC-Lua does it by
   marking the values of open upvalues of unmarked threads
   (`remarkupvals`), which would take a trace hook on `UpVal` reading
   the stack, and a barrier story for a slot the VM writes freely. Open
   upvalues keep their `Coro` alive instead (`UpVal.co`). As it was:
   - **Effect on the stacks:** they cannot be `mem.alloc`'d like the
     main stack, because nothing would free them when a coroutine is
     dropped. They are GC objects instead, which is the cause of items 4
     and 5.
   - **Effect on open upvalues:** PUC-Lua closes a dropped thread's open
     upvalues when it frees the thread. Mini instead keeps the dead
     stack alive from each open upvalue (`UpVal.stack`).
   - **Suggested fix:** finalizers, or a sweeper-friendly "free this
     host block when object X dies" registration (`heap.on_free(obj,
     ptr)`).
7. **Yield and resume can only happen where a native call happens.**
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
     growth and relocation built in (item 8).
8. **Stacks that move.** *Unchanged,* except that a replaced block is
   now freed when the resume returns, not when the coroutine dies. As
   it was:
   - **The work:** `grow` rebases every frame base, every open upvalue
     and four context pointers by hand.
   - **Handler state:** `reload` covers what the handlers cache, and
     `redo` after the growth works well.
   - **Natives and the nested-run helper:** these hold raw pointers.
     `call_value` had to switch from saved pointers to indices, and
     superseded blocks were kept allocated until the coroutine died
     (`VStack.prev`) because a host native may still hold an `args`
     pointer.
   - **The docs:** they say `reload` "should recompute state from ...
     offsets or the frame record" (§5), but nothing helps with the frame
     record itself moving.
   - **Suggested fix:** a `std.rt` stack type with index-based frames
     and a relocation callback.
9. **Mini's natives returned exactly one value.** `resume` returns `true`
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
- **Adding a `V` variant was a small change** (but see friction 2).
  Exhaustiveness errors pointed at every `match` to update.
- **GC stress and verify modes found the barrier issue (5) immediately.**
  GC stress also exposed item 1, and the existing compare-and-branch test
  exposed item 2.
