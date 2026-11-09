# Coroutines in Mini

Mini has Lua 5.4's `coroutine` library (`SPEC.md`, §5). This note is for
contributors and interpreter authors. It explains how the coroutines map
onto Loimi's vm model, how they are tested and what Loimi made hard.
Mini is the only example interpreter with more than one guest stack.

The short version:

- Each coroutine has its own value stack and frame stack, in host memory.
  There is no native stack switching.
- A switch between coroutines is ordinary vm control flow: a handler
  saves one thread's state, installs another's and jumps to its saved
  continuation. A resume/yield pair never leaves the dispatch loop.
- Only resumes from native code (a `table.sort` comparator, a host
  callback) start a nested run of the vm.

## Design

### Threads and their state

A coroutine is a `Coro` object (`coro.lm`). It owns a value stack and a
frame stack allocated with `mem.alloc`, like the main stack. They start
at 60 slots and 12 frames and double when a call needs more, up to the
main stack's limits.

The VM runs on whatever stacks these context fields describe: `stack`,
`stack_end`, `frames`, `frames_end`, `fp`, `base`, `open` and
`open_level`. The vm state registers (`pc`, `regs`, `k`, `fp`) point into
those stacks.

While a thread is not running, its state is in its `Coro`:

| Field | Holds |
|---|---|
| `fp`, `base` | Its frame pointer and run base. |
| `open`, `open_level` | Its open upvalues. |
| `ypc` | Where the CALL that switched away continues, as a `Mini.Resume`. |
| `ydst`, `ynres` | Where that CALL's results go, and how many it wants. |
| `prev` | For a running coroutine, the thread that resumed it. |

The main thread has a `Coro` too (`ctx.main`), on the main stacks.

**Values.** `V` has one variant for threads, `Co(Gc<Coro>)`. It holds
either a thread or a function made by `coroutine.wrap`. A wrap function
is a separate `Coro` whose `wrap` field is the coroutine it resumes, so
it differs from the thread that `coroutine.running()` returns inside.
CALL has a cold path for it next to the one for natives. There is one
variant rather than two because of the value layout (friction 2).

### Switching inside the run

CALL's cold native path recognizes the calls that switch threads:

- **`coroutine.resume(co, ...)` and calling a wrap function**
  (`switch_in` in `vm.lm`). If `coro.can_switch` holds (the coroutine is
  suspended, a Mini function, and under the nesting limit):
  1. save the running thread's state into its `Coro`, with
     `ypc = next_resume` (`coro.enter`);
  2. install the coroutine's stacks in the context;
  3. set the state registers from its saved frame.

  A started coroutine continues at its own `ypc`, with the resume's
  arguments stored as the results of its pending yield. A new one gets
  frame 0 and jumps to its body's code.
- **`coroutine.yield(...)`** in a coroutine resumed this way
  (`switch_out`). It saves its state the same way (`coro.suspend`, which
  also logs the `Coro` for the GC barrier), reinstalls the resumer's
  (`coro.leave`), stores `true` (not for a wrap function) and the values
  as the results of the resumer's CALL, and jumps to the resumer's `ypc`.
- **The coroutine's last return** (RET1, RETN or RET0 from its bottom
  frame) does the same with the returned values, then frees the
  coroutine's stacks (`finish`). This is a cold branch of RET's existing
  "bottom of the run" check.
- **An error** reaches `on raise`, which adds the line prefix. If the
  running thread is a coroutine resumed in this run, `on raise` kills it
  and continues its resumer:
  - through `resume`, with `false` and the message as the CALL's results;
  - through a wrap function, by raising the message again at the
    resumer's CALL, with that line added, as Lua 5.4 does. This may kill
    the next coroutine in turn.

So a resume/yield pair is two cold blocks of CALL and a jump each way.
There is no native call, no nested entry and no C stack per coroutine.
Arguments and results move directly between the two threads' registers.

`switch_in`, `switch_out` and `back` are `#[inline]`. Without it, each
switch in tail-call mode went from CALL's outlined cold block through two
shared copies of `-> never` vm functions; inlining made tail-call mode
10–17% faster on the coroutine benchmarks.

**Quickened switch sites.** A CALL that switched threads quickens to
CALL_RESUME, CALL_WRAP or CALL_YIELD (CALL is a family). These test for
the same call and switch on the hot path, and go back to plain CALL when
the test fails.

### Why the vm state stays coherent

- A switch happens inside one handler (a CALL, a RET or `on raise`). That
  handler sets every state register (`fp`, `regs`, `k`, and `pc` through
  its `jump`) from the thread it installs, and stores the context fields.
  The next handler sees a whole thread.
- `ctx.fp` is also stored at the switch. It is what `sync` publishes
  before any call that may collect or re-enter.
- The state registers are raw pointers, not traced values, so rooting
  does not change. The collector finds the running thread through the
  context and the resumers through `Coro.prev`.
- The suspended thread's continuation is a `Mini.Resume` made by
  `next_resume`. That is a proof that the address is an instruction
  boundary of verified code, so no raw code pointer is stored.
- What remains the vm's job, as with return addresses: the state it
  installs (frame, registers, constants) must belong to the code it jumps
  to, and the context fields and state registers must switch together.
  Mini keeps this in three `coro` functions (`enter`, `leave`, `suspend`)
  and three vm functions (`switch_in`, `switch_out`, `back`).

### Resumes from natives

A wrap function used as a `table.sort` comparator, or a coroutine resumed
by a host native, goes through the native `coro.resume`. It saves the
resumer's state the same way (`coro.enter`, `coro.leave`) but runs the
coroutine in a nested run:

- The first time, through the `Mini.call` entry.
- After a yield, through the `Mini.resume` entry. It reloads `fp`, `regs`
  and `k` from the installed frame, stores the resume arguments as the
  results of the pending yield's CALL, and jumps to the instruction after
  it (`jump c.ypc`).

Such a coroutine has `inrun` false. Its yield records the values and sets
`ctx.yielded`. CALL's native path then calls `yield_exit`, which saves
`next_resume`, the CALL's base register and its result count, and leaves
the run with `exit`. Its last return and its errors leave the run too.

The two kinds mix freely. A coroutine resumed from a native can resume
others inside its own run, and a coroutine resumed in a run can call
natives that start nested runs. Each in-run link stays inside one run.

### Yield across a C-call boundary

`switch_out` is taken only when the running coroutine was resumed in this
run and no nested run is active on its stacks (`ctx.base == ctx.frames`:
the innermost run's bottom frame is the coroutine's first). Otherwise the
call goes to the native. The native either yields through `yield_exit`
(a coroutine resumed by a native, same condition) or reports "attempt to
yield across a C-call boundary", as PUC-Lua does for C functions without
continuations. That error happens when yielding:

- from a `table.sort` comparator;
- from a host native's callback;
- in `table.sort(t, coroutine.yield)`.

`coroutine.isyieldable()` is false in those places. A comparator may
still create, resume and finish other coroutines in its own run, and a
wrap function works as a comparator.

A yield is recognized only at a CALL. A guest operation that yields from
another op, such as a metamethod or an iterator, would need the same
check in that op. Mini has neither.

### Nesting limit

Each nested run counts in `Mini.depth`, and each in-run resume in
`ctx.nco`. Together they are limited to 200 ("C stack overflow"), as
`LUAI_MAXCCALLS` limits PUC-Lua. In-run resumes use no C stack, so the
limit is there to match Lua, not out of need.

### Several results from natives

A native can return several values: it fills `ctx.rbuf` and sets
`ctx.nret`, and CALL's native path copies them into its result
registers. `coroutine.running` uses this, and so does `resume` when it
goes through the native.

### Stack growth

The main stack has a fixed size and never moves. A coroutine's stacks
start small, so they move when they grow (`vm.grow`):

- CALL's overflow check calls `grow` in a cold block.
- `grow` copies the stacks and rebases each frame's `base`, each open
  upvalue and the context's pointers.
- The CALL then sets `regs = fp.base` and does `redo`.
- `call_value` (used by comparators and `mini_call`) keeps its saved
  frame and window as indices, because the nested run may grow the
  stacks.
- A replaced block stays allocated until the coroutine suspends or dies
  (`Coro.retired`), because a native in its run may still hold an `args`
  pointer into it. A coroutine can only suspend with no native active on
  its stacks.

### GC

- **Roots.** `scan_roots` walks the frames of the running thread and of
  every resumer, following `Coro.prev` from `ctx.cur` up to each one's
  saved `fp`. It marks `ctx.cur`, `ctx.main` and the multi-result buffer.
- **Suspended coroutines** are traced through their objects. `Coro` has a
  `#[trace(trace_coro)]` hook, which walks a suspended coroutine's frames
  and marks each frame's closure and registers. Slots above the top frame
  are not traced. For a running or normal coroutine the hook marks
  nothing, because its stacks are roots.
- **Freeing stacks.** A dead coroutine frees its stacks at once (`die`).
  A dropped suspended coroutine frees them when the collector finds the
  `Coro` dead: the first resume registers a free hook
  (`heap.on_free_gc(co, release)`) when it allocates the stacks
  (`coro.prepare`). The stacks count towards the next collection
  (`heap.allocated`).
- **Open upvalues.** An open upvalue into a coroutine's stack has a `co`
  field that keeps the `Coro`, and so its stacks, alive. Closing the
  upvalue drops the reference. If a suspended coroutine is dropped while
  one of its closures survives, the coroutine stays alive with it, and
  the closure reads live memory. (PUC-Lua instead closes such upvalues
  when it frees the thread; see friction 6.)
- **The generational barrier.** The VM stores into a coroutine's stacks
  without write barriers. While the coroutine runs or is normal, its
  stacks are roots, which every collection scans, minor ones included.
  When it suspends, `coro.suspend` logs the `Coro` (`gc.remember`), so
  the next minor collection traces it through the hook.
- **Out-of-line barriers.** `coro.enter` and `coro.suspend` store their
  references (`open`, `prev`) through raw pointers and log the objects
  with `gc.remember` out of line. The compiler's barrier has an inline
  slow path that made these functions too large for LLVM to inline (6%
  on chameneos and coroutine_ring in tail-call mode).
- **Checked** with GC stress (a collection at every safepoint), verify
  mode (the barrier check at every collection) and `--gc-check-roots`
  (the collector validates every reference it finds in a root), in all
  three dispatch modes and under the JIT.

## Tests

All Mini scripts run in the three GC modes (normal, stress, verify).
`tests/golden.rs` also runs the coroutine scripts in the goto and switch
dispatch modes (`mini_reentry_dispatch_modes`), under the copy-and-patch
JIT (`mini_jit_stencils`), and on AArch64 when the cross toolchain
exists.

| Test | Covers |
|---|---|
| `tests/coroutine_basics.mini`, `coroutine_gc.mini`, `err_coroutine_wrap.mini`, `err_coroutine_yield.mini` | The library: values both ways, wrap generators, yield from nested calls, errors, `running`, `status`, `isyieldable`, `close`, yield from a comparator, shared and closed upvalues, an upvalue that outlives its coroutine, 9000-deep recursion inside a coroutine (stack growth with open upvalues), resume chains, the nesting limit, hundreds of garbage coroutines. `coroutine_gc.mini` prints exactly what Lua 5.4.9 prints. |
| `tests/coroutine_switch.mini` | In-run switches and how they mix with nested runs: results with fewer and more values than wanted, chains of resumes through wrap functions, errors through `resume` and through two wrap functions, comparators that resume coroutines or are wrap functions, the yield-across-a-comparator error, upvalues open into suspended and finished coroutines, 150 live coroutines, the nesting limit, 300 dropped generators. |
| `tests/conformance/coroutine_lua.lua` | PUC-Lua 5.4.9's `testes/coroutine.lua`, reduced to the Mini subset. Prints exactly what PUC-Lua prints. |

The conformance port's header lists every removed or rewritten part,
with line numbers. It removes:

- everything about `<close>`/`__close`;
- `pcall` and `xpcall` around yields;
- `debug.sethook` and `debug.getinfo`;
- the weak-table check;
- the `T` (testC) API block;
- the `table.unpack` stack limit;
- yields inside metamethods and in custom for-iterators.

Mini has no `pcall`, so the port's `try(f, a)` runs `f` in a fresh
coroutine instead.

## Speed

The benchmarks are three public programs in `bench/mini/coroutines/`:
LuaJIT-test-cleanup's `coroutine-ring` (the Benchmarks Game thread-ring)
and `chameneos`, and the classic Lua `sieve` (from Luau's benchmarks). The
README there gives their sources and the mechanical edits made.

In the run recorded in [`docs/results.md`](../../docs/results.md), Mini
took 0.31–0.35 of PUC-Lua 5.4's time on them and 0.88–0.92 of the time
of LuaJIT's assembly interpreter (`-joff`). The goto and switch dispatch
builds took 1.05–1.11 times as long as tail-call.

Measured design choices:

- **Switching inside the run** made a resume/yield pair take 0.62–0.77 of
  the time of the nested-entry version, in every dispatch mode. It cost
  the other 14 Mini benchmarks nothing measurable. RET's bottom-of-run
  check calls `finish` as a shared `#[inline(never)]` copy: as a `cold`
  block it gave RET1 a larger frame on its hot path (fib 1.026).
- **Stacks in host memory** instead of heap objects made no measurable
  difference in speed. Keeping heap stacks with a `#[trace]` hook was not
  built: it needs a pin and an unpin per switch, the hook needs the
  frames to know which slots are live, and growth would still allocate
  heap objects. Host memory needs one barrier per suspend and no sweeper.

## Friction: what Loimi made hard

The problems met while adding coroutines, ranked by how much each cost or
would cost the next interpreter with several stacks. Other documents
refer to these by number.

1. **A compiler bug in the rooting of `let ... else`.** *Fixed.* Under GC
   stress in tail-call dispatch, `coroutine.resume(coroutine.create(w))`
   with `w` a wrap function crashed, but only while a branch of
   `coro.resume` was written inline. The compiler rooted the bindings of
   `let PAT = e else { .. }` from the pattern test on, so calls inside
   the else block listed slots that were never assigned there and held
   stale stack values. The bindings are now rooted from the end of the
   pattern test (`src/codegen.rs`, `TStmt::LetPat`), tested by
   `tests/run/gc_roots_let_else.lm`. Lesson: a stale root is invisible
   until the garbage stops being harmless. ASan catches it if the slot
   happens to hold a stack address; verify mode does not check roots.
2. **Adding a value variant silently changed the others' encoding.**
   *Fixed.* With nine tagged variants, Mini's `V` moved from the classic
   NaN-box layout to the packed one, where `Int(i32)` gets a 32-bit block
   instead of a 48-bit one. A compare-and-branch fast path that tested
   raw words then also accepted Nil and Bool (a wrong checksum in
   `tests/compare_jumps.mini`), and tuple tests stopped merging
   (`hash_strings`, `interp`, `queens` 4–6% slower). Now:
   - `loimi build --explain-values` prints each value type's layout;
   - `block N` pins a variant's block width (`V` declares `Int(i32) block
     48` and `Nil block 0`);
   - the `value_layout` warning names unpinned variants that a packed
     layout narrows (an error under `-W error`, which the tests use for
     Mini);
   - `V.Int.mask`, `V.Int.tag`, `V.Int.block` and `V.Num.limit` give the
     encoding as constants, and the fast path tests
     `((x | y) & V.Int.mask) == 0`.

   Mini also merged its two new variants into one `Co`.
3. **Entries could not resume at a saved pc.** *Fixed.* `Mini.resume`
   needed `jump c.ypc`, which `#[require_verified]` refused, so it used
   `unsafe`. Now `Mini.Resume` is a proof that an address is an
   instruction boundary of verified code. Vm code makes one with
   `next_resume`, a host with `v.resume_at(i)` on a `Verified<Mini>`
   (checked at run time), and an entry may `jump` to one. Before the
   first yield, `ypc` holds `Mini.Resume.placeholder()`, which traps if
   jumped to. Like a return address, a resume point does not keep its
   code alive; the suspended coroutine's frames do.
4. **No way to trace only the live part of an object.** *Fixed* by
   std.gc's `#[trace(f)]` hooks. Before, the generated trace function of
   a stack object marked every slot, including dead ones above the top
   frame.
5. **Raw stores into stacks that are GC objects need the remembered
   set.** *Fixed* twice: std.gc has `heap.pin_dirty(obj)` and
   `unpin_dirty` (a root that every collection traces, exempt from the
   barrier check), and Mini no longer needs it, because its stacks left
   the heap (item 6).
6. **No finalizers.** *Fixed for the stacks* by std.gc's free hooks
   (`heap.on_free(obj, f)`: `f` runs once when `obj` is found dead; it may
   release host memory but not allocate, collect or follow references).
   *Not for open upvalues:* closing a dropped coroutine's open upvalues
   from a free hook would store unmarked values from the dead stack into
   live upvalues during a collection. PUC-Lua handles this by marking the
   values of open upvalues of unmarked threads (`remarkupvals`). Mini
   keeps the `Coro` alive from each open upvalue instead (`UpVal.co`).
7. **A resume cost a native call and a nested entry.** *Fixed in Mini,
   with no new language feature*: switches happen inside the run (see
   Design). Everything a switch does is something handlers already do.
   CALL and RET already set every state register and `jump` to a return
   address that vm code saved; a switch does the same with another
   thread's frame. A switch calls nothing that runs guest code, so it
   needs no `reload`. A `suspend` primitive that leaves the loop with the
   state saved would still leave and re-enter the run at every switch,
   which is the cost this removes.
8. **Stacks that move.** *Unchanged.* `grow` rebases every frame base,
   every open upvalue and four context pointers by hand. `reload` covers
   what the handlers cache, and `redo` after the growth works well. But
   natives and the nested-run helper hold raw pointers: `call_value` had
   to switch from saved pointers to indices, and replaced blocks must
   stay allocated while a native may hold an `args` pointer. The docs say
   `reload` should recompute state from offsets or the frame record, but
   nothing helps when the frame record itself moves. A possible fix: a
   `std.rt` stack type with index-based frames and a relocation callback.
9. **Mini's natives returned exactly one value.** `resume` returns `true`
   plus the values, and `running` returns two values, so Mini needed a
   multi-result protocol (`ctx.nret`, `ctx.rbuf`). This is Mini's design,
   not Loimi's, but no example interpreter had one to copy.

## What worked well

- **Nested entries** (`entry .. uses reenter`, `Mini.depth`, `on raise`
  leaving a run with `Err`) are what coroutines without stack switching
  need. `Mini.call` is reused unchanged to start a coroutine resumed from
  a native.
- **Setting the state in a handler.** Switching threads inside the run
  took no language support: a handler that sets `fp`, `regs` and `k` and
  jumps to a saved `Mini.Resume` is a RET to another stack.
- **Control words in a `-> never` vm function.** `exit` and
  `next_resume` inside `yield_exit`, called from a `cold` block, make the
  path where a yield leaves the run a few lines long. `redo` after growing
  the stacks re-executes CALL with the new `regs`.
- **`sync` and `reload`** keep handlers correct across nested resumes with
  no new code. After a resume through a native returns, `reload` reads
  `ctx.fp`, which `resume` restored.
- **The shadow stack** roots the traced state of all nested runs, so
  interleaved coroutine runs on one C stack needed no new rooting rules.
  The only new roots are the resumers' saved frames.
- **Adding a `V` variant** was a small change (apart from friction 2):
  exhaustiveness errors pointed at every `match` to update.
- **GC stress and verify modes** found the barrier problem (item 5) at
  once. GC stress also exposed item 1, and an existing test exposed
  item 2.
