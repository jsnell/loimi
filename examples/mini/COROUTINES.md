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
up to the main stack's limits. The VM runs on whatever stacks the
context's fields `stack`, `stack_end`, `frames`, `frames_end`, `fp`,
`base`, `open` and `open_level` describe, with the vm state registers
(`pc`, `regs`, `k`, `fp`) pointing into them.

**A switch is ordinary vm control flow.** Each thread's state, while it
is not running, is in its own `Coro`: its frame pointer, run base, open
upvalues, and where the CALL that switched away continues (`ypc`, a
`Mini.Resume`) with where that CALL's results go (`ydst`, `ynres`).
The main thread has a `Coro` too (`ctx.main`), on the main stacks. A
running coroutine links to the thread that resumed it (`Coro.prev`).
CALL's cold native path recognizes the three calls that switch:

- **`coroutine.resume(co, ...)` and calling a wrap function**
  (`switch_in` in vm.lm): if `coro.can_switch` (suspended, under the
  nesting limit, a Mini function), save the running thread's state
  into its `Coro` with `ypc = next_resume` (`coro.enter`), install the
  coroutine's stacks in the context, and set the state registers from
  its saved frame. A started coroutine continues at its own `ypc`, with
  the resume's arguments stored as the results of its pending yield; a
  new one gets frame 0 and jumps to its body's code.
- **`coroutine.yield(...)`** in a coroutine resumed that way
  (`switch_out`): save its state the same way (`coro.suspend`, which
  also logs the `Coro` for the barrier, below), reinstall the resumer's
  (`coro.leave`), store `true` (not for a wrap function) and the values
  as the results of the resumer's CALL, and jump to its `ypc`.
- **The coroutine's last return** (RET1, RETN or RET0 from its bottom
  frame, a cold branch of the existing "bottom of the run" check) does
  the same with the returned values, then frees the coroutine's stacks
  (`finish`).
- **An error** in it reaches `on raise`, which prefixes the line as
  before and then, while the running thread is a coroutine resumed in
  this run, kills it and continues its resumer: with `false` and the
  message as the CALL's results, or, through a wrap function, by
  raising the message again at the resumer's CALL (with that line
  added, as Lua 5.4 does), which may kill the next coroutine in turn.

There is no native call, no nested entry and no C stack per active
coroutine: a resume/yield pair is two cold blocks of CALL and a jump
each way. The argument and result values move directly between the
two threads' registers.

**Resumes from natives** still nest: a wrap function used as a
`table.sort` comparator, or resumed by a host native, is resumed by
the native `coro.resume`. It saves the resumer's state the same way
(`coro.enter`, `coro.leave`) but runs the coroutine through a nested
entry: `Mini.call` the first time, and after a yield `Mini.resume`,
which reloads `fp`, `regs` and `k` from the installed frame, stores the
resume arguments as the results of the pending yield's CALL, and jumps
to the instruction after it. Such a coroutine has `inrun` false: its
yield records the values and sets `ctx.yielded`, and CALL's native path
calls `yield_exit`, which saves `next_resume`, the CALL's base register
and its result count, and leaves the run with `exit`; its last return
and its errors leave the run as well. A coroutine resumed this way can
resume others in its own run, and a coroutine resumed in a run can call
natives that start nested runs: each in-run link stays inside one run.

**Nesting limit.** Each nested run counts in `Mini.depth` and each
in-run resume in `ctx.nco`; together they are bounded by 200 ("C stack
overflow"), as `LUAI_MAXCCALLS` bounds PUC-Lua's. In-run resumes use no
C stack, so this is now Lua's limit rather than a need.

**Several results from natives.** A native can return several values:
it fills `ctx.rbuf` and sets `ctx.nret`, and CALL's native path copies
them into its result registers (`coroutine.running`, and `resume` when
it goes through the native).

**Values.** `V` has one variant for threads, `Co(Gc<Coro>)`. It is a
thread, or a function made by `coroutine.wrap`: a separate `Coro` whose
`wrap` field is the coroutine it resumes, so that it differs from the
thread that `coroutine.running()` returns inside. CALL has a cold path
for it next to the one for natives. Only one variant was added, not
two: friction 2 explains why.

**Why the vm state stays coherent.** A switch happens inside one
handler (a CALL, a RET, or `on raise`), which sets every state register
(`fp`, `regs`, `k`, and `pc` through its `jump`) from the thread it
installs, and stores the context's fields, so the next handler sees a
whole thread. `ctx.fp` is also stored at the switch; it is what `sync`
publishes before any call that may collect or re-enter. The state
registers are raw pointers, not traced values, so no rooting changes:
the collector finds the threads through the context (the running one)
and `Coro.prev` (the resumers). The suspended thread's continuation is
a `Mini.Resume` made by `next_resume`, the same proof the nested
`Mini.resume` entry jumps to.

**Yield across a C-call boundary.** `switch_out` is taken only when the
running coroutine was resumed in this run and no nested run is active
on its stacks (`ctx.base == ctx.frames`: the innermost run's bottom
frame is the coroutine's first). Otherwise the call goes to the native,
which yields through `yield_exit` (a coroutine resumed by a native,
same condition) or reports "attempt to yield across a C-call boundary",
as PUC-Lua does for C functions without continuations: yielding from a
`table.sort` comparator, from a host native's callback, and
`table.sort(t, coroutine.yield)`. `coroutine.isyieldable()` is false in
those places. A comparator may still create, resume and finish other
coroutines (in its own run), and a `wrap` function works as a
comparator.

**Stack growth.** A coroutine's stacks move when they grow (`vm.grow`):
the main stack is fixed-size and never moves, but a coroutine's stacks
start small. CALL's overflow check calls `grow` in a cold block. `grow`
copies the stacks and rebases each frame's `base`, each open upvalue and
the context's pointers. The CALL then sets `regs = fp.base` and does
`redo`. `call_value` (comparators, `mini_call`) keeps its saved frame
and window as indices, because the nested run may grow the stacks. The
block that a stack replaced stays allocated until the coroutine
suspends or dies (`Coro.retired`), because a native of its run may
still hold an `args` pointer into it; a coroutine can only suspend with
no native active on its stacks.

**GC.**
- **Roots.** `scan_roots` walks the frames of the running thread, and
  those of every resumer, following `Coro.prev` from `ctx.cur` up to
  each one's saved `fp`, and marks `ctx.cur`, `ctx.main` and the
  multi-result buffer.
- **Suspended coroutines** are traced through their objects: `Coro`
  has a `#[trace(trace_coro)]` hook (std.gc's trace hooks), which walks
  the frames of a suspended coroutine and marks each frame's closure
  and registers, as `scan_roots` does for the running thread. Slots
  above the top frame are not traced. A running or normal coroutine's
  hook marks nothing: its stacks are roots.
- **Dead coroutines** free their stacks at once (`die`). A suspended
  coroutine that is dropped frees them when the collector finds the
  `Coro` dead: the first resume registers a free hook (`heap.on_free_gc(co,
  release)`) when it allocates the stacks (`coro.prepare`). The
  stacks count towards the next collection (`heap.allocated`), as they
  did when they were heap objects.
- **Open upvalues.** An open upvalue into a coroutine's stack has a `co`
  field that keeps the `Coro`, and so its stacks, alive; closing it
  drops the reference. If a suspended coroutine is dropped while one of
  its closures survives, the coroutine stays alive with it, and the
  closure reads live memory. PUC-Lua closes such upvalues when it frees
  the thread instead (friction 6).
- **The generational barrier.** The VM stores into a coroutine's stacks
  without write barriers. While it runs (or is normal), the stacks are
  roots, which every collection scans, minor ones included. When it
  suspends, `coro.suspend` logs the `Coro` (`gc.write_barrier`), so the
  next minor collection traces it through the hook, and verify mode
  checks it from then on.
- **Checked** with GC stress (a collection at every safepoint), verify
  mode (the barrier check at every collection) and `--gc-check-roots`
  (the collector validates every reference it finds in a root), in all
  three dispatch modes and under the JIT.

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

- **`tests/coroutine_switch.mini`** covers the in-run switches and their
  mix with nested runs: results both ways with fewer and more values
  than wanted, a chain of resumes (main, a coroutine, a wrap function
  inside it), errors through `resume` and through two wrap functions
  (each adding its line), a comparator resuming coroutines in its run,
  a wrap function as the comparator (a nested run) that resumes another
  coroutine in its run and yields, an error in a coroutine inside a
  comparator, the yield-across-a-comparator error from a coroutine
  resumed by a CALL, upvalues open into suspended and finished
  coroutines, a chain of 150 live coroutines, the nesting limit, and
  300 dropped generators. Its expected output is what the nested-entry
  version printed, byte for byte, in every dispatch and GC mode.

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
`sieve` (via Luau's bench). With the switches inside the run
(2026-11-03; friction 7), best of 5, interleaved, x86-64, Mini built
`--release --align-functions=64`, times in seconds; *before* is the
commit before (each resume a native call and a nested entry):

| benchmark | mode | before | now | now/before | PUC-Lua 5.4.9 | LuaJIT 2.1 `-joff` | now/Lua | now/LuaJIT |
|---|---|---:|---:|---:|---:|---:|---:|---:|
| chameneos | tailcall | 0.593 | 0.417 | 0.70 | 1.009 | 0.380 | 0.41 | 1.10 |
| coroutine_ring | tailcall | 0.524 | 0.356 | 0.68 | 1.005 | 0.379 | 0.35 | 0.94 |
| sieve | tailcall | 0.576 | 0.390 | 0.68 | 1.104 | 0.388 | 0.35 | 1.01 |
| chameneos | goto | 0.604 | 0.445 | 0.74 | | | 0.44 | 1.17 |
| coroutine_ring | goto | 0.581 | 0.392 | 0.67 | | | 0.39 | 1.03 |
| sieve | goto | 0.655 | 0.413 | 0.63 | | | 0.37 | 1.06 |
| chameneos | switch | 0.626 | 0.480 | 0.77 | | | 0.48 | 1.26 |
| coroutine_ring | switch | 0.590 | 0.396 | 0.67 | | | 0.39 | 1.04 |
| sieve | switch | 0.655 | 0.405 | 0.62 | | | 0.37 | 1.04 |

By geometric mean, tail-call Mini takes 0.37 of PUC-Lua's time (0.54
before) and 1.01 of LuaJIT `-joff`'s (1.47 before); goto 1.09 and
switch 1.11 of LuaJIT's (1.60 and 1.63 before). A resume/yield pair is
now two cold blocks of CALL, about a dozen stores into the two `Coro`s
and the context each way, and the value copies; LuaJIT's resume is
assembly that switches `lua_State`s the same way, without leaving the
dispatch loop. The goto and switch builds no longer enter and leave
the function holding every handler per pair (docs/results.md,
"Follow-up (2026-11-03)"), so they are within 10% of tail-call here.
chameneos does more besides switching (upvalue reads, string
compares), where LuaJIT's interpreter stays ahead.

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

**The switches inside the run cost the other benchmarks nothing**
(2026-11-03): the 14 clean-room benchmarks, interleaved, best of 3,
`--align-functions=64`, now/before by geometric mean: tailcall 1.008,
goto 0.977, switch 0.971. In tail-call mode the handlers' hot paths are
the same instructions as before (only `Coro.wrap`'s field offset in
CALL's wrap test, and the raise and run-exit tails, differ); the
widest, `hash_strings`, is 1.03 in a best-of-7 rerun (placement). The
bottom-of-run check in RET calls `finish` as a shared copy
(`#[inline(never)]`): as a `cold` block it kept the instruction word
live and gave RET1 a larger frame on its hot path (fib 1.026). In goto
mode, where every handler is in one function, `fib` is 0.88 and
`interp` 1.04 (reproduced at best of 7), the rest within ±4%.

**No regression on the 14 clean-room benchmarks** (when coroutines
were added). These are
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
   and broke a fast path.** *Resolved* (2026-11-03; language.md §2,
   "Seeing and pinning the layout"):
   - `loimi build --explain-values` prints each value type's layout:
     every variant's block width, mask, tag and words, and which
     variants' tuple tests merge (same block width) and which do not.
   - `block N` pins a variant's block width. `V` now declares `Int(i32)
     block 48` and `Nil block 0`, the layout it already had: Int keeps
     its 48-bit block (and its merged tests with the pointers) however
     many variants are added.
   - The `value_layout` warning names every unpinned variant that a
     non-classic layout narrows, so adding a variant that changes the
     others' encoding is a warning, and an error under `-W error`, which
     the test suite builds Mini with. Lox and MiniJS, which already had
     packed layouts, pin their narrow variants at their current widths.
   - `V.Int.mask`, `V.Int.tag`, `V.Int.block` and `V.Num.limit` are the
     encoding as constants. The compare-and-branch fast path now tests
     `((x | y) & V.Int.mask) == 0` instead of a hand-written `< 2^32`
     (same instructions, the shift count follows the layout: `shr $48`
     instead of `shr $32`), and the string table makes a `V.S` from
     `V.S.tag` instead of a tag word it computed at run time (`stag`).
     No other example tests raw value words.

   As it was:
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
3. **Entries cannot resume at a saved pc.** *Resolved* (2026-11-03;
   language.md §6, "Resume points"): `Mini.Resume` is a proof that an
   address is an instruction boundary of verified code. Vm code makes
   one with `next_resume` (the address `next` continues at), a host with
   `v.resume_at(i)` on a `Verified<Mini>` (checked at run time), and
   an entry may `jump` to one under `#[require_verified]`. The yield
   path (`yield_exit`, called from CALL's native and wrap paths) saves
   `ctx.ypc = next_resume`, and `Mini.resume` does `jump c.ypc`, with
   no `unsafe`. Before the first yield, `ypc` holds
   `Mini.Resume.placeholder()`, which traps if jumped to. Its field is private to the vm's generated code, so it
   cannot be built from a raw pointer outside `unsafe`, and the verifier
   rejects code ending in an instruction that saves its successor (an
   existing hole for `next_pc` found on the way: ops that save `next_pc`
   without naming `pc` were not counted as continuing). Like a return
   address, a resume point does not keep its code alive: the suspended
   coroutine's frames do. The yield code moved out of `native_results`
   into `yield_exit`, so that the native-call path compiles as before
   (with the yield inside, LLVM stopped inlining the shared copy of
   `native_results`). The handlers' code is unchanged (with items 2 and
   3 together, the only hot-path difference is the shift count in the
   compare-and-branch ops), and so are the timings, against the commit
   before, interleaved, `--align-functions=64`: best of 10, fib 1.001,
   chameneos 1.007 to 1.009, coroutine_ring 0.948 to 0.996, sieve 1.018;
   best of 5 over 8 clean-room benchmarks and the 3 coroutine ones,
   geomean 1.009, all within the ±5% that this shared machine showed
   between repeated runs.
   A first version kept `ypc` as a `?Mini.Resume`, 8 bytes longer, which
   moved the `Ctx` fields after it: fib ran 3% slower with otherwise
   identical code.

   As it was:
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
7. **Resume cost a native call and a nested entry.** *Resolved in
   Mini, without a language feature* (2026-11-03): a CALL of
   `coroutine.resume`, of a wrap function or of `coroutine.yield`, and
   the coroutine's last return and its errors, switch threads inside
   the running vm run (Design, "A switch is ordinary vm control flow").
   A resume/yield pair takes 0.62–0.77 of the time it did, in every
   dispatch mode (Speed).
   - **Why no language feature was needed.** Everything a switch does
     is something handlers already do:
     - A handler may assign every state variable and `jump` to a code
       address that vm code saved: CALL and RET do it with each
       frame's return address (`jump fp.resume`). A switch is the same
       with another thread's frame: set `fp`, `regs` and `k` from the
       frame it installs, `jump` to its saved continuation.
       `next_resume` (item 3) already made that continuation a proof,
       `Mini.Resume`, so the saved thread state needs no raw pointer
       to code that was not made by vm code.
     - The shadow-stack copies of traced state do not belong to the
       run beyond a call: the compiler copies each traced state
       variable at each call that may collect, from its current value.
       A switch happens between calls, so nothing refers to the old
       thread's registers afterwards. Mini's state registers are raw
       pointers anyway; the collector finds the threads through Mini's
       own scanner (`ctx.cur` and `Coro.prev`). Under GC stress, verify
       mode and `--gc-check-roots`, in all three dispatch modes and
       under the JIT, the tests pass unchanged.
     - `reload` is for after a call that may have run guest code; a
       switch does not call anything that runs guest code, so it does
       not need it: it sets the registers itself.
     - `suspend v` (language.md §3.4, §10: leave the loop with the
       state saved) is the wrong primitive for this: it still leaves
       and re-enters the run at every switch, which is the cost this
       removes. It would help a host that drives guest generators from
       outside the loop; Mini's host-side resume (`coro.resume`, for
       comparators and host natives) already does that with a nested
       entry, `exit` and the `Mini.resume` entry.
   - **What is still the vm's job**, as for return addresses: that the
     state it installs (frame, registers, constants) belongs to the code
     it jumps to, and that the thread's context fields and the state
     registers are switched together. Mini keeps both in three small
     functions (`coro.enter`, `coro.leave`, `coro.suspend`) and three vm
     functions (`switch_in`, `switch_out`, `back`).
   - **Still true:** a yield is recognized at a CALL. A guest operation
     that yields from another op (a metamethod, an iterator) would need
     the same check in that op; Mini has neither.
   - **A tail-call detail:** `switch_in`, `switch_out` and `back` are
     `#[inline]`. Without it, each switch in tail-call mode went from
     CALL's outlined cold block through two shared copies of `-> never`
     vm functions (arguments in thread-local slots, a frame set-up
     each); inlined, tail-call mode is 10–17% faster on the three
     benchmarks, and goto and switch modes (which inline every call
     anyway) are unchanged.

   As it was:
   - **What Mini did:** a yield had to leave the run at a point where
     the handler knows the CALL's `pc`, base and result count. Mini did
     it in CALL's native path, after the native returned, through a
     flag. Each resume was a native call and a nested entry: in goto
     and switch mode the entry is the function holding every handler,
     entered and left once per resume/yield pair (docs/results.md,
     "Follow-up (2026-11-03)").
   - **Suggested fix (then):** language.md §10's `suspend v`, with a
     matching `resume` entry form, on the belief that ops had no
     supported way to swap the state registers across stacks.
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
  unchanged as the start of a coroutine resumed from a native.
- **Assigning the state in a handler.** Switching threads inside the
  run took no language support: a handler that sets `fp`, `regs` and
  `k` and jumps to a saved `Mini.Resume` is a RET to another stack.
  `#[inline]` and `#[inline(never)]` on the vm functions then decided
  where the code went (friction 7).
- **Control words in a `-> never` vm function.** `exit` and `next_resume`
  inside `yield_exit`, called from a `cold` block, made the
  yield-leaves-the-run path a few lines. `redo` after growing the stacks
  re-executes CALL with the new `regs`.
- **`sync` and `reload` keep handlers correct across nested resumes
  with no new code.** After a resume through a native returns, `reload`
  reads `ctx.fp`, which `resume` restored.
- **The shadow stack roots the traced state of all nested runs.**
  Interleaved coroutine runs on one C stack needed no new rooting rules.
  The only new roots are the resumers' saved frames, which the
  scanner walks through `Coro.prev`.
- **Adding a `V` variant was a small change** (but see friction 2).
  Exhaustiveness errors pointed at every `match` to update.
- **GC stress and verify modes found the barrier issue (5) immediately.**
  GC stress also exposed item 1, and the existing compare-and-branch test
  exposed item 2.
