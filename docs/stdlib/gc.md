# `std.gc`: the garbage collector

What `std/gc.lm` implements comes first ("As implemented"); the original
design follows, and much of it is not built (see "Not implemented").
It expands the `std.gc` section of [`../stdlib.md`](../stdlib.md)
and relies on the language features in [`../language.md`](../language.md)
§5 (effects) and §8 (GC integration).

## As implemented

`std/gc.lm`, tested by `tests/run/gc_*.lm` (among them
`gc_generational.lm`, `gc_verify_barrier.lm`, `gc_weak.lm` and
`gc_hooks.lm`) and by
every interpreter test run in three modes: normal, `stress`, and
`verify` (generational heap, stress, and verification).

### Heap layout

- **Chunks.** Small objects (header
  included, up to 512 bytes; classes step by 16 bytes) live in 64 KiB
  chunks aligned to 64 KiB, carved out of 2 MiB segments mapped aligned
  to 2 MiB (`mmap`, with `MADV_HUGEPAGE`), so that each can be a
  transparent huge page. With 1 MiB segments from `posix_memalign`, which
  the kernel backs with 4 KiB pages, MiniJS's `richards` ran 7% slower
  than with the old allocator's chunks in the brk heap (which got huge
  pages); with 2 MiB segments it is back within 1%. The start of a chunk holds a 16-byte `ChunkHead`
  pointing at the heap's remembered set, so the write barrier finds it
  from an object's address alone; the mark bitmap follows it (below).
  The objects then start at an offset that
  varies from chunk to chunk, from 0 to 4 KiB (cache coloring). Without
  it, objects at the start of fresh chunks shared their page offset with
  the VM stack and frame arrays (also allocated page-aligned), and loads
  from them waited on unrelated stores (4K aliasing): Mini's
  `spectral_norm` ran 20% slower.
- **The header is 16 bytes**, `{ size: u32, tid: u16, mark: u8, flags:
  u8, len: u64 }`. `len` is the trailing array's length. Flags: `LOG`
  (old and not in the remembered set), `LARGE`, and in generational mode
  `AGED` and `INRS` (below). The mark byte holds the heap's *epoch* (1 or
  2) when the object is marked; every major collection (in both modes)
  flips the epoch, which unmarks every object at once.
- **Mark bitmaps, no sweep** (2026-11-04). After the `ChunkHead`, each
  chunk has a 512-byte bitmap with a bit per 16-byte granule of the
  chunk; marking a small object sets the bits of all its granules (one or
  two word ORs; the header's mark byte stays the "already marked" test).
  The clear bits are the free memory, so a collection frees dead small
  objects without touching them: no sweep. Each size class (16-byte steps
  up to 512 bytes with the header) bump-allocates through a *run* of
  clear bits in one of its chunks (`h.cur[c]` .. `h.lim[c]`); when the
  run is used up, `refill` finds the class's next run from the bitmap
  (word-at-a-time, `ctz`), in chunk order. Every object of a chunk has
  the chunk's class, so runs start and end at object boundaries and no
  memory is lost to holes too small for the class. After each collection
  the chunks are relinked per class in index order, and a chunk with no
  marked bit goes to a pool that any class takes from (before adding new
  chunks): an all-dead chunk is reused without a per-object pass.
  A major collection clears the bitmaps (512 bytes per chunk) before
  marking; a minor one keeps them (old objects stay marked).
  The sweep it replaced read and wrote every slot of every chunk
  allocated from (the heap's whole allocation budget) at each
  collection: 3-29% of Prolog's run time, and as lazy sweeping 13-23% of
  Scheme's allocation-heavy programs (`deriv`, `diviter`, `cpstak`).
  Without it Prolog takes 0.885 of its time before, Scheme 0.955,
  Python 0.94 (geometric means), at the same peak memory; the
  collector's remaining cost is marking (the bitmap writes make it
  1.1-1.25 times as long) and `refill`. Measurements:
  [`../results.md`](../results.md#gc-mark-bitmaps-instead-of-sweeping-2026-11-04).
  Allocating all sizes from one run (Immix-like, with each object's
  granules marked) was as fast for Prolog but made MiniJS's `sort` 4%
  slower: records allocated between strings and arrays were spread over
  more cache lines than in a chunk of their own class.
- **`try_new(h, T { .. })`** (compiled to `gc.try_alloc`) is `new`
  without the refill: it bumps the class's run or gives `None`, and makes
  no call, so a tail-call handler that allocates with it has no stack
  frame (`new`'s refill call site gives it one on every execution, even
  though the refill is rare). `gc.make_room(h, size)` refills the size
  class of `size` bytes (`size_of<T>()`, plus the trailing array) and
  never collects; a handler does that in a cold block and `redo`es the
  op (language.md §4, "Allocating and storing references in a
  handler"). Large objects are allocated as by `new`.
- **Large objects** are `calloc`'d with the remembered-set pointer in 16
  bytes before the header, and kept in a vector.
- **Collection triggers**: `growth` (percent of the live bytes),
  `min_heap` and `max_budget` (`set_growth`, `set_max_budget`).
  Interpreters test `heap.allocated > heap.threshold` in their handlers
  and call `gc.collect`.

### Generational mode: sticky mark bits, non-moving

`heap.set_generational(true)`, off by default (the decision and the
measurements are in [`../roadmap.md`](../roadmap.md), open question 2).

- *Young* means allocated since the last collection, *old* means it
  survived one (a large object: two). Old objects keep their mark
  (sticky marks).
- **Mark epochs.** The mark byte holds the heap's epoch (1 or 2) when
  set. A major collection flips the epoch, which unmarks every object at
  once, instead of clearing every mark first (a pass over all headers:
  12 ms of Scheme's `nboyer` 750 ms run went to it).
- A **minor collection** marks from the roots and from the remembered set
  and stops at marked (old) objects. Marking an object sets its LOG flag
  and counts its bytes; the collection sweeps nothing but the young
  large objects. A young small object that survives gets its mark bits;
  the dead ones' granules stay clear, and the allocator reuses them (see
  "Mark bitmaps" above). Minor pauses are marking only.
- **Aging, for large objects.** A large object that survives its first
  minor collection stays young until the next one: it is marked with
  that collection's *young mark* (3 to 254, a new one each minor
  collection, so the next one sees it unmarked) and flagged `AGED`. An
  old object that points to a young survivor stays in the remembered set
  (flag `INRS`), with LOG set again so that a store into it still calls
  `remember` (an interpreter that logs an object's parts with it,
  below, relies on that); an object promoted while it points to one goes
  into the set. Small objects are promoted when they first survive:
  aging them too traced the survivors of programs with growing live
  data twice (Scheme `nboyer`, `earley` and `mperm` 5-8% slower).
- A **major collection** runs a full mark (bitmaps cleared, epoch
  flipped) whose survivors stay marked. It runs once the old generation has grown by the growth budget
  since the last major collection.
- **The nursery** is the same allocation budget as between full
  collections, computed from the live bytes after the last major
  collection (not from the old generation, which also holds promoted
  objects that have died since: budgets computed from it grew the heap,
  and Scheme's `string` and Mini's `string_build` ran 10-40% slower), and
  can be capped with `heap.set_nursery(bytes)`. Small nurseries measured
  worse: they promote the objects in use at each minor collection, which
  then fill the old generation and bring major collections sooner (Mini
  `binary_trees` with a 4 MiB nursery: 358 minor and 15 major
  collections, more GC time than without generations; with the full
  budget: 30 and 4, half the GC time).
- Stress mode with verify on and a generational heap collects after 0
  to 4 KiB of allocation (varying), not at every safepoint, so that objects stay
  young across a few safepoints and stores into old objects exercise the
  barrier. Every eighth collection is major.

### The write barrier: object logging

<!-- snippet: excerpt std/gc.lm -->
```loimi
#[inline]
pub fn barrier(obj: *u8) {
    let hd = (obj - HEADER) as *Header;
    if unlikely((hd.flags & LOG) != 0) { log_obj(obj); }
}
```

- The compiler calls `gc.barrier(obj)` after every store of a traced
  value into an object (see [`../lowering.md`](../lowering.md) §10). In a
  non-generational heap no object has `LOG`, so the barrier costs a byte
  test and a not-taken branch.
- `log_obj`, the slow path, clears `LOG` and appends the object to the
  remembered set (unless it is still there, `INRS`). It runs once per
  object between collections. It is inline and makes no call (a call
  site, even a rare one, gives a tail-call handler a stack frame on every
  execution): when the set is full it only counts the object as
  `missed`, and the next collection is then a major one, which needs no
  remembered set; after each collection the set gets room for twice
  what was logged since the last (at least 256 entries). Being inline,
  it makes functions with barriers larger, which can stop LLVM from
  inlining them (lowering.md section 3: Mini's thread switch lost 6%
  this way). For such a helper, store through raw pointers and log with
  `needs_barrier` and `remember` (below). `remember` is
  the same with a growing push, out of line, for interpreters that log
  several objects together. A minor collection traces the remembered objects and
  sets `LOG` again; those that point to young survivors stay in the set.
- A store of a value statically free of references (a variant without
  a `Gc` payload, such as `Term.Unb` or `Int(n)`, `None`, or a literal
  tuple or struct of such) gets no barrier.
- **Slices of trailing arrays.** A store through `o.items[i]` or a
  sub-slice of it gets the barrier. Passed to a function that may store
  references through it (`fill(o.items, v)`, a summary the compiler
  computes per function and parameter), `o.items` gets the barrier on
  `o` after the call; that function must not collect, nor keep the
  slice. A slice of a traced trailing array cannot be kept anywhere a
  store through it would escape the compiler (a `var`, a field, a return
  value) or be stored through from a `let`: those are compile errors
  ([`../design/safety.md`](../design/safety.md) section 8).
  `o.items.ptr` is a raw pointer: stores through it need
  `gc.write_barrier`.
- **Stores through raw pointers** bypass the compiler's barrier:
  `gc.write_barrier(h, obj)` after them, naming the object that contains
  the slot (not an interior pointer). `gc.needs_barrier(obj)` and
  `gc.remember(obj)` let an interpreter log several objects together.
  Mini logs a table and its `arr` and `hash` parts together, so its
  table stores test only the table's own header (the parts are never
  older than the table).
- **Verify mode** (`heap.set_verify(true)`): at the start of every
  collection, every old object outside the remembered set (and every
  young survivor of the last minor collection counts as young) is traced
  without marking, and a reference to a young object (a missing barrier)
  or to freed memory (a dangling reference; verify mode fills free
  memory with 0x01 bytes after each collection) aborts with both type names:

  ```
  gc verify: old object of type value.Arr points to a young object of type value.Str but is not in the remembered set: missing write barrier
  ```

### Roots

A collection marks from the root scanners (`heap.add_scanner`), then
from the shadow stack: the frames compiled code links at each call
that may collect (`uses gc`), listing the traced locals in scope, the
values pending across defers, and, in vm code, a copy of each state
variable of a traced type (a cached top of stack, a current closure),
taken at the call so that the variable itself stays in a register
(language.md §8). An interpreter's scanner therefore marks memory only:
its stacks up to the top that `sync` published, frames, globals,
constants. Nothing that lives only in a handler's registers needs
scanning. `rt.protect`/`rt.catch_panic` restore the shadow-stack top
when they unwind.

### Checking roots: `--gc-check-roots`

`loimi build --gc-check-roots` (also for `run` and `test`) builds a
program whose collector checks every reference that generated code
hands it before reading its header: shadow-stack roots (traced locals,
values kept for defers, the copies of vm state), `gc_mark` calls in
scanners and `#[trace]` hooks, and traced fields. A reference must be
null or the start of a live object of the heap being collected. It also
fills unassigned traced slots and the objects a collection frees with
0x01 bytes, so a root listed too early or kept too long is caught the
first time it is marked, and reports a bad reference with its frame,
function, call site and slot. The checks live in std.gc (`rootck_*`);
how the compiler wires them is in [`../lowering.md`](../lowering.md)
§9 ("Checked roots"). Interpreter test scripts run their suites in this
mode (`examples/python/check.sh`, the GC tests); the rooting fuzzer
(`tests/fuzz/rootgen.rs`, [`../status.md`](../status.md)) builds every
program with it.

### Weak references and ephemerons

- **`Weak<T>`** is a library object, `object Weak<T> { addr: u64 }`, made
  with `heap.new_weak(x)` (which registers it), read with `w.get() ->
  ?Gc<T>` and changed with `w.set(..)`. The address is not traced. After
  marking, the collector clears registered weak references whose target
  is unmarked and drops dead ones from its registry. No `weak` field
  qualifier exists: an object that needs one holds a `Gc<Weak<T>>`.
- **`Ephemerons<K, V>`** is a table in host memory from keys `Gc<K>` (by
  address) to values of any traced type V, created with
  `gc.Ephemerons.new(h)` (which registers it with the heap) and released
  with `t.free()`. After the roots are marked, the collector marks the
  values of entries with marked keys, drains, and repeats until nothing
  new is marked. It then drops the entries whose key died. The table is
  not itself a GC object, so a guest `WeakMap` object frees it with a
  free hook (`h.on_free`, below).
- Order after marking: ephemerons to a fixpoint, ephemeron sweeping,
  weak references, then the sweepers (`add_sweeper`), the free hooks
  (`on_free`, below) first. In a minor
  collection old objects count as marked (`h.is_marked(p)`, or the
  slower `gc.is_marked(p)`, which finds the heap from the object's
  chunk), so sweepers and weak references stay correct;
  `heap.is_minor()` tells a sweeper that only young objects can have
  died.
- **The sweeper-based weak tables stay.** Mini's string intern table
  and std.obj's transition table are host-side hash tables with weak
  entries keyed by content (string bytes; parent shape, key and
  attributes), and std.obj keeps the ids of dead shapes. Neither is a
  map from objects, so neither `Weak` (an extra object per entry) nor
  `Ephemerons` (keyed by identity) fits better than the sweeper.

### Trace hooks, free hooks and dirty pinning

Three hooks for objects whose layout or lifetime the generated code
cannot see. Mini's coroutines (examples/mini/COROUTINES.md) needed all
of them; `tests/run/gc_hooks.lm` tests each in a non-generational heap,
normal and under stress, and in a generational one under stress with
verify, also built with `--gc-check-roots`.

**`#[trace(f)]` on an `object`: a custom trace for the part the fields
do not describe.**

<!-- snippet: check -->
```loimi
import std.gc;

object Node { v: i64 }

#[trace(trace_stack)]
object Stack { top: u64, slots: [?Gc<Node>] }

/// Only the slots below `top` are live.
fn trace_stack(h: *gc.Heap, s: Gc<Stack>) {
    for i in 0..s.top { gc_mark(h, s.slots[i]); }
}

fn main() {}
```

- The generated trace function still traces the declared fields; then,
  instead of the whole trailing array, it calls `f(h, obj)`. `f` marks
  what is live (`gc_mark`): a prefix of the trailing array, or
  references held in memory the object owns outside the heap (Mini's
  coroutine stacks are `mem.alloc`'d, and their hook walks the frames).
  Keeping the fields generated means a hook cannot forget one.
- `f` is a function of the object's module, `fn f(h: *gc.Heap, obj:
  Gc<Obj>)`, without effects (it runs inside a collection), checked by
  the compiler. Generic objects cannot have one.
- **Everything else follows from going through the trace function.** A
  minor collection traces remembered objects with it, verify mode checks
  what it marks (a reference to a young object from an old one outside
  the remembered set aborts as for any field), and in a
  `--gc-check-roots` build its `gc_mark` calls are checked and reported
  as such. The barrier contract is the usual one: a store that changes
  what the hook marks (a young reference into the owned memory, or
  raising `top` over slots holding young references) needs
  `gc.write_barrier(h, obj)` unless the compiler emits it (a store into
  the trailing array does).
- Why a hook and not a declared live length (`slots: [V] live(top)`):
  the length form covers only a prefix of a trailing array, and Mini's
  stacks needed neither (their live part is the frames' register
  windows, and they are not in the heap). A hook covers both, at the
  cost of writing the loop.

**`h.on_free(obj, f)`: free hooks, for memory an object owns.**

- `f(obj)` (a `fn(*u8)`) runs once, when a collection finds `obj`
  unreachable, before the object's slot can be reused; `Heap.free` runs
  the hooks of all objects still registered. Registered per object, so
  objects that own nothing yet (a coroutine that has not started) cost
  nothing.
- `h.on_free_gc(obj, f)` is the same for an object of a known type,
  with `f` a `fn(Gc<T>)`, so the hook reads the object's fields without
  an `unsafe` cast from `*u8`:

<!-- snippet: run "1" -->
```loimi
import std.gc;
import std.io;
import std.mem;

object Buf { n: u64, data: *u8 }

var freed: i64 = 0;

fn free_buf(b: Gc<Buf>) {
    mem.release(b.data);
    freed += 1;
}

fn make(h: *gc.Heap) {
    let b = new(h, Buf { n: 64, data: mem.alloc(64) });
    h.on_free_gc(b, free_buf);
}

fn main() uses gc {
    var h = gc.Heap.new();
    make(&h);
    gc.collect(&h);
    io.print_int(freed); io.println("");
}
```

- **When.** Inside the collection, after marking, ephemerons and weak
  references (so no `Weak` reaches the object any more and ephemeron
  entries keyed by it are gone), as the first sweeper, before
  `rootck_poison` and the interpreter's sweepers. Not lazily: with lazy
  sweeping the slot is freed later, but the hook has already run.
  Objects that die together run their hooks in no particular order.
- **What it may do.** Release host memory, close descriptors, update
  host counters. It reads the object's own non-reference fields (raw
  pointers, integers); the object is not yet swept. It must not
  allocate in the heap, collect, register hooks, follow references
  (what they point to may be dead too, and their hooks may have run),
  or store `obj` anywhere: there is no resurrection.
- **Why so narrow.** A general finalizer (Lua's `__gc`, Python's
  `__del__`) runs guest code, which may allocate and resurrect, so it
  must run outside the collector, after the object has been kept alive
  for one more cycle (the original design below). Releasing native
  memory needs none of that, and a hook that cannot reach the heap
  cannot break it. Guest finalizers can be built on top: an
  interpreter's hook queues a host-side record, and the interpreter
  runs the guest code at a safe point.
- **Cost.** The registry is a host vector held as the data of a sweeper
  that `on_free` installs on first use, so heaps without hooks pay
  nothing, and the collector's mark and sweep loops are unchanged. In a
  minor collection only the entries of young objects are looked at
  (entries are split old/young like the large objects).

**`h.pin_dirty(obj)` / `h.unpin_dirty(obj)`: objects written without
barriers.**

- While pinned, the object is a root, and every collection traces it,
  minor ones included, as if it were always in the remembered set. Stores
  into it through raw pointers need no `write_barrier`, and verify mode
  does not check it (it carries `INRS`, which also keeps the barrier from
  logging it). This is for a GC object that the mutator writes in a hot
  loop: a register file or a value stack kept in the heap.
- `unpin_dirty` puts an old object into the remembered set, for the
  stores made since the last collection; from then on stores need the
  barrier again. Pins nest; `h.is_pinned_dirty(obj)` tells.
- The pinned objects are a scanner's data: in each collection the
  scanner marks them, traces the old ones in a minor collection (marking
  stops at marked objects), and sets their flags back to "remembered"
  (the remembered set's processing, a major collection or promotion may
  have changed them).
- Why not a scanner of the interpreter's own: a scanner can mark from
  the object, but verify mode would still check the object, and a
  scanner cannot put an object into the remembered set (at scan time a
  young object has no `LOG` flag, so `remember` does nothing, and it
  becomes old later in the same collection). Mini did it with a sweeper
  that re-remembered the stacks after each collection. Mini no longer
  needs either: its stacks left the heap (`on_free`), and a running
  coroutine's stacks are scanned as roots.
- Each collection traces the whole object; combine it with a
  `#[trace]` hook to trace only the live part.

The hook functions in std.gc (`on_free`, `pin_dirty` and their helpers)
are emitted only where used (src/codegen.rs, `is_lazy`): `Heap.free`
refers to the two registries' functions, and that is the only change in
the IR of programs that do not use them (with four type declarations).

### Threads sharing a heap: `std.gil`

`std/gil.lm`: a global interpreter lock per heap, so that several threads
share one heap (and the interpreter state around it), one at a time.
Opt-in: a program that does not import it is unchanged. Mini built with
`--cfg gil` is the worked example (`examples/mini/threads.lm`,
`examples/mini/embed.lm`, `examples/embed/shared.c`; language.md §10.1,
Threads).

<!-- snippet: check
import std.gc;
import std.gil;
struct Interp { heap: gc.Heap, gil: *gil.Gil }
struct PerThread { top: u64 }
fn save(d: *u8, t: *u8) {}
fn load(d: *u8, t: *u8) {}
fn scan(h: *gc.Heap, d: *u8, t: *u8) {}
fn blocking_io() {}
fn setup(it: *Interp, me: *PerThread) uses gc {
    {snippet}
}
fn main() {}
-->
```loimi
it.gil = gil.Gil.new(&it.heap, me as *u8);   // held by this thread
it.gil.set_hooks(gil.Hooks { data: it as *u8, save: save, load: load, scan: scan });
it.gil.unlock();                             // around code that blocks
blocking_io();
it.gil.lock();
```

- **The lock.** `Gil.new(h, data)` creates it held by the calling
  thread, attached with per-thread state `data`. Another thread calls
  `g.attach(data)` (waits for the lock) and, holding it, `g.detach()`.
  `g.unlock()` / `g.lock()` let go and take it back around blocking
  code. Waiting is first come, first served (tickets under a pthread
  mutex and condition variable). `g.mine()` is the calling thread's
  record (null if not attached), `g.holder()` the holder's,
  `g.switches()` counts hand-overs. `g.free()` releases the lock once
  the calling thread is the only one attached and holds it, after the
  heap's last collection (free the heap first).
- **Hand-over at safepoints.** `g.waiting()` is a relaxed atomic load of
  the number of waiting threads, which the holder polls where its state
  is synced: Mini at every taken jump and every call of a Lua function.
  When it is non-zero, `g.yield_now()` (`uses gc`) lets the waiters run
  and takes the lock back. Polls are plain code in the handlers,
  `#[cfg(gil)] if unlikely(ctx.gil.waiting()) { cold { ctx.gil.yield_now(); } }`,
  so a handler syncs and roots its state for them as for any call that
  may collect.
- **Per-thread state moves.** The interpreter's state has one copy of
  the per-thread parts (stacks, the running frame), the holder's. The
  hooks move them: `save(data, thread)` when a thread lets go,
  `load(data, thread)` when one takes the lock (a thread attached with
  fresh data gets its stacks there), `scan(h, data, thread)` in
  collections.
- **Roots of the other threads.** `Gil.new` registers a scanner. In
  each collection (run by the holder, which marks its own shadow stack
  as always) it walks every other attached thread: the shadow-stack
  frames it had linked when it let go of the lock, saved then
  (`gc.mark_shadow_stack`, which marks only this heap's objects, like
  the collector's own walk), and its per-thread state (`scan`). The
  frames stay valid until it takes the lock back: they belong to calls
  that have not returned, and `unlock`, `lock` and `yield_now` are
  `uses gc`, so their callers link their own traced locals. While a
  thread has let go, it must not touch the heap.
- **Generational heaps** need nothing more: stacks are scanned as roots
  in every collection, minor ones included, whichever thread's they are.
- **No other synchronization.** The collector, the write barrier,
  allocation, quickening and inline caches run only on the holder, and
  the mutex orders one holder's writes before the next one's reads.
  `examples/embed/shared.c` is clean under ThreadSanitizer.

### Not implemented

Policies as a compile-time parameter (`#![heap = ..]`; generational is a
run-time switch, and the barrier is always compiled in), incremental
marking, card marking (a large array stored into is retraced whole by each minor
collection), aging of small objects (measured, see above), finalizers that run guest
code (free hooks for host memory are built, above), `Root<T>`/`GcVec<T>`, heap snapshots, poisoning (except in
`--gc-check-roots` builds, above),
and the `weak` field qualifier (also for inline caches, language.md
§7.1).

`LOIMI_GC_STATS=1` prints a line per collection to stderr: kind, live
(old) bytes, the remembered set's size, the mark and sweep times (the
sweep is now only that of large objects), and the time spent in
`refill` finding runs so far (still labelled "lazy sweep").

---

## Design (not built as written)

The original design draft. **Not built** unless "As implemented" above
says so: in particular the policy parameter, `Incremental`, `HeapConfig`,
`Root<T>`, finalizers (only free hooks are built), weak inline caches
and heap snapshots.

### Goals

1. **Precise.** The collector never guesses whether a word is a pointer. Every
   root and every field is known exactly, from generated code.
2. **Allocation is a handful of instructions** when inlined into a handler,
   with no effect on the fast path (see effects below).
3. **No barrier cost unless the policy needs one.** The default collector has
   no barrier at all.
4. **Non-moving by default.** Interpreters keep raw pointers into the VM stack,
   into objects, and into code. A non-moving heap makes all of these safe
   without handles.
5. **Policies are swappable at compile time** without changing interpreter
   source: `#![heap = std.gc.MarkSweep]`, `Incremental`, or `Generational`.
6. **Debuggable.** Stress mode, heap verification, and heap snapshots are
   part of the library from the start, not added later.

Non-goals for 1.0: concurrent marking on a separate thread, multiple mutator
threads sharing one heap, and a compacting collector. The policy interface
(section 10) leaves room for all three later.

---

### 1. Heap layout

```
Segment (4 MiB, 4 MiB-aligned)
├── SegmentHeader     page table, free-page bitmap, owning Heap
├── Page 0 (64 KiB)   size class 3 (48-byte cells)    ← one size class per page
├── Page 1 (64 KiB)   size class 7 (128-byte cells)
├── ...
└── Page 63           large-object run (spans pages 61–63)
```

- **Segments** come from `mmap`, are aligned to their size, and are
  decommitted (`madvise(DONTNEED)`) after staying empty for N cycles. Masking
  an object pointer with `~(4 MiB - 1)` gives its segment header in one
  instruction, and the page index follows from the offset.
- **Pages** hold cells of a single size class. Classes step by 16 bytes up to
  128, then by about 12.5% up to 8 KiB (around 40 classes). Larger objects get
  a run of whole pages.
- **Mark bits live in a side bitmap** per page (one bit per 16-byte granule),
  not in object headers. Clearing marks is a `memset` over the bitmaps.
  Sweeping reads the bitmaps without touching object memory. Copy-on-write
  memory after `fork` stays shared.

#### Object header

Every `object` gets an 8-byte header, generated by the compiler:

<!-- snippet: skip design, not built: the real header is 16 bytes, see "Heap layout" above -->
```loimi
bits Header: u64 {
    type_id: 32,        // index into the type table
    flags: 16,          // HAS_FINALIZER, LARGE, LOGGED (generational), HASHED, PINNED_FOREIGN
    extra: 16,          // per-type use, e.g. a small length or a cached hash
}
```

#### Type table

The compiler emits one `TypeInfo` per `object` declaration:

<!-- snippet: skip design, not built: the compiler generates a trace function per type instead -->
```loimi
struct TypeInfo {
    name: str,
    size: SizeKind,          // Fixed(bytes) | Trailing { base: u32, elem: u32, len_field: u32 }
    ptr_map: u64,            // bit i set ⇒ word i is Gc<_> or Value (fixed part, first 64 words)
    elem_kind: ElemKind,     // trailing array: None | Gc | Value | Custom
    trace: ?fn(Gc<Any>, *Tracer),   // only for layouts the bitmap can't describe
    finalize: ?fn(Gc<Any>),
}
```

Most objects are traced by a **generic loop over `ptr_map`**, with no
indirect call per object. `Value` words are decoded with the value type's
generated `trace_value`, which checks whether the word holds a pointer and
extracts it.

---

### 2. Allocation

Each heap has one `Allocator` (one mutator per heap, section 9). For each size
class it keeps a **free list of cells** in the current page, as mimalloc does.

<!-- snippet: skip design, not built: the real allocator is `gc.alloc` in std/gc.lm -->
```loimi
// What `ctx.heap.new(Object { ... })` inlines to. `class` and `size` are
// compile-time constants for fixed-size objects.
#[inline(always)]
fn alloc_cell(h: *Heap, class: u8, size: u32) -> *mut u8 {
    let p = h.alloc.free[class];
    if unlikely(p.is_null()) {
        return cold { h.refill(class, size) };     // the only part with `uses gc`
    }
    h.alloc.free[class] = unsafe { *(p as **mut u8) };
    h.alloc.debt += size;
    p
}
```

The fast path is about five instructions with no stores outside the allocator
and no effects. Only `refill` is `uses gc`, so only the refill path syncs VM
state (language.md §5). The compiler then writes the header and fields.

`refill(class)`:

1. Sweep the next unswept page of this class (**lazy sweeping**). If it has
   free cells, thread them into the free list and return.
2. Otherwise, if `debt > budget`, run a collection (or, in incremental mode,
   a marking slice) and retry.
3. Otherwise take a fresh page, which is built as a free list on first use.
4. If the heap limit is exhausted: run a full GC. If that fails, return a guest
   out-of-memory error (`HeapError.OutOfMemory`), which the caller turns into a
   `raise`.

Large objects (> 8 KiB) bypass size classes: `alloc_large` is
`#[cold] uses gc` and is accounted the same way.

#### Variable-size objects

Objects with a trailing array (`slots: [Value]`) take the length at
allocation: `heap.new_with_len(Object { shape }, n)`. The size class is then
computed at run time through a lookup table indexed by `(size + 15) >> 4`
below 1 KiB. Trailing elements are initialized to the element type's default
(`Nil` for `Value`) so the GC never sees uninitialized memory. (As built:
`new(h, Obj { .. }, n)` zero-fills them, which is allowed only where zero
is a valid element, else `new(h, Obj { .. }, n, v)` fills them with `v`:
language.md section 8.)

---

### 3. Collection triggers

- `budget = max(min_heap, live_after_last_gc * (growth - 1))`, with
  `growth = 2.0` by default (like Go's `GOGC = 100`).
- `debt` counts bytes allocated since the last cycle. Crossing `budget`
  triggers a collection at the next refill.
- `heap.collect(kind)` forces one. Tuning is through `HeapConfig`:
  `min_heap`, `growth`, `max_heap`, and `incremental_step`.

---

### 4. MarkSweep (default policy)

```
collect():
  1. clear mark bitmaps of all pages                  (memset)
  2. mark roots (section 6) → push onto mark stack
  3. drain the mark stack, tracing through TypeInfo
  4. process ephemerons until no new marks             (section 7)
  5. clear weak references to unmarked objects; reset weak inline-cache entries
  6. unmarked objects with finalizers → mark (resurrect), queue for finalization
  7. drain again (finalizer-reachable objects survive this cycle)
  8. pages: mark as "unswept" (actual sweeping is lazy, in refill)
  9. live_after_last_gc = marked bytes; budget recomputed
```

- The mark stack is a growable host `Vec<Gc<Any>>`. If it can't grow, the
  collector falls back to rescanning marked pages for unscanned objects.
- **No write barrier.** `Policy.barrier` is empty and inlines away.
- Pause time is proportional to the live heap. That suits scripts and batch
  workloads. Interactive hosts should use `Incremental`.

---

### 5. Incremental and Generational policies

#### `Incremental`: tri-color, snapshot-at-the-beginning

- Marking happens in slices driven by allocation. Each refill does
  `incremental_step × bytes_allocated` worth of marking.
- **Barrier (SATB):** while marking is active, a store into a traced field
  first grays the *old* value. This is the only barrier that needs the
  overwritten value; the policy declares `const needs_old_value = true;` so the
  compiler emits a load before the store.

  <!-- snippet: skip design, not built: no incremental policy -->
  ```loimi
  #[inline(always)]
  fn barrier(h: *Heap, old: Value, new: Value) {
      if unlikely(h.marking) { cold { h.satb_push(old) } }   // one load and a branch when idle
  }
  ```
- Objects allocated during marking are allocated **black** (pre-marked in the
  bitmap), so they can't be lost.
- VM stacks aren't barriered (that would mean a barrier on every register
  write). They are rescanned in a short final pause, together with the other
  roots. The final pause is bounded by stack depth plus the SATB buffer size.

#### `Generational`: sticky mark bits, non-moving

- "Young" means unmarked, i.e. allocated since the last collection. A **minor
  GC** marks from the roots plus the remembered set and treats already-marked
  (old) objects as done. It does not clear mark bits, so survivors become old
  automatically. A **major GC** clears the bitmaps and runs a full MarkSweep.
- **Barrier (object logging):** when a store puts a pointer into an old object
  that hasn't been logged yet, log the object into the remembered set.

  <!-- snippet: skip design, not built: the real barrier is `gc.barrier` above -->
  ```loimi
  #[inline(always)]
  fn barrier(h: *Heap, obj: Gc<Any>, new: Value) {
      if unlikely(obj.header.flags & OLD_UNLOGGED != 0) and new.is_gc() {
          cold { h.remember(obj) }        // sets LOGGED, appends to remembered set
      }
  }
  ```
- Minor GCs are triggered by young-generation debt, majors by old-generation
  growth. Defaults: young budget 8 MiB, major when the old generation has
  doubled.

The VM stack is always scanned fully as a root, so stack writes need no
barrier in either policy.

---

### 6. Roots

The collector asks for roots through one generated function per heap
configuration:

<!-- snippet: skip design, not built: roots are the shadow stack plus `heap.add_scanner` -->
```loimi
fn scan_roots(h: *Heap, t: *Tracer)
```

It visits:

1. **VM stacks**, generated from each `vm`'s state kinds. A `ValueStack` is
   scanned from its base to `ctx.sync.top`. Effects guarantee `sync.top` is
   current whenever a collection can happen (language.md §5).
2. **Frame stacks:** `Gc`/`Value` fields of every `Frame` (generated from the
   struct's field types).
3. **The temporary root area** where handlers spill live locals at `gc` calls.
4. **Shadow-stack frames** of ordinary functions (`ctx.roots` chain).
5. **Handles** (`Root<T>`) and **root scanners** registered by the host.
6. **Code**: inline caches inside verified bytecode (section 8).

#### Host-side roots API

<!-- snippet: skip design, not built: no `Root<T>`, `GcVec<T>` or `Tracer` -->
```loimi
// A strong handle that keeps an object alive while host code holds it.
struct Root<T> { ... }                       // 4-byte index into the handle table
fn Heap.root<T>(h: *Heap, x: Gc<T>) -> Root<T>;
fn Root<T>.get(r: Root<T>, h: *Heap) -> Gc<T>;
fn Root<T>.release(r: Root<T>, h: *Heap);    // use with `defer`

// A host vector whose elements are roots. Typical use: a compiler collecting
// constants before the Proto object exists.
struct GcVec<T> { ... }                      // registers itself as a root scanner
fn GcVec<T>.new(h: *Heap) -> GcVec<T>;
fn GcVec<T>.push(v: *GcVec<T>, x: T);         // T: Gc<_> or Value
fn GcVec<T>.free(v: *GcVec<T>, h: *Heap);

// Arbitrary host structures.
fn Heap.add_root_scanner(h: *Heap, data: *mut u8, f: fn(*mut u8, *Tracer)) -> ScannerId;
fn Heap.remove_root_scanner(h: *Heap, id: ScannerId);

fn Tracer.visit(t: *Tracer, x: Gc<Any>);
fn Tracer.visit_value<V>(t: *Tracer, v: V);  // V: any `value` type
```

`defer` runs a statement at scope exit:

<!-- snippet: skip design, not built: no `Heap.root` -->
```loimi
let r = ctx.heap.root(s);
defer r.release(&ctx.heap);
```

---

### 7. Weak references, ephemerons, finalizers

<!-- snippet: skip design, not built: see "Weak references and ephemerons" above for the real API -->
```loimi
// Weak reference: does not keep the target alive; cleared after the target dies.
object Weak<T> { target: weak Gc<T> }
fn Heap.new_weak<T>(h: *Heap, x: Gc<T>) -> Gc<Weak<T>> uses gc;
fn Weak<T>.get(w: Gc<Weak<T>>) -> ?Gc<T>;

// Ephemeron table: value is reachable only through (table AND key).
// The basis for JS WeakMap and Lua tables with __mode = "k".
object EphemeronTable<V> { ... }
fn Heap.new_ephemeron_table<V>(h: *Heap) -> Gc<EphemeronTable<V>> uses gc;
fn EphemeronTable<V>.get(t: Gc<Self>, k: Gc<Any>) -> ?V;
fn EphemeronTable<V>.set(t: Gc<Self>, h: *Heap, k: Gc<Any>, v: V) uses gc;
fn EphemeronTable<V>.delete(t: Gc<Self>, k: Gc<Any>);

// Finalizers: run outside the collector, at a point the host chooses.
fn Heap.set_finalizer<T>(h: *Heap, x: Gc<T>);   // uses TypeInfo.finalize for T
fn Heap.pending_finalizers(h: *Heap) -> bool;
fn Heap.run_finalizers(h: *Heap, ctx: *mut u8, f: fn(*mut u8, Gc<Any>) -> Result<(), Value> uses reenter) uses reenter;
```

- **`weak` is a field qualifier** that the compiler understands. Weak fields
  are left out of the type's `ptr_map`, recorded in a separate weak map, and
  cleared in step 5 of the collection. Lua's weak-valued tables use this.
- **Ephemerons** are processed to a fixpoint. Each pass marks values whose
  keys are marked, and it repeats until nothing changes. Entries with dead keys
  are removed in step 5.
- **Finalizers never run inside the collector.** Guest finalizers
  (`__gc`, `__del__`) can run guest code (`reenter`), which isn't allowed
  in the middle of a collection. Objects are resurrected and queued, and the
  interpreter calls `run_finalizers` at a safe point. The usual choice is at a
  backward jump or call, via a cheap `heap.pending_finalizers()` check in a
  `cold` path. Objects survive until the cycle after their finalizer runs.

---

### 8. Code as a root: weak inline caches

Inline caches live inside bytecode (language.md §7.1). A `ShapeId` cached in
code would dangle if the shape died and its id were reused. Cache
declarations can therefore mark fields weak:

<!-- snippet: skip design, not built: no `weak` cache fields -->
```loimi
cache PropIC { shape: weak ShapeId, slot: SlotRef }
```

When a `Proto` is traced, its generated tracer walks the code with the vm's
generated decoder. That is cheap, because the decoder knows exactly where cache
words are. Weak cache fields don't keep anything alive. In step 5, any entry
whose target died is **reset to its zero (empty) state**. Because family
members reset to an empty cache look like a miss, the `cold` miss path
re-specializes (`become generic`) the next time the instruction runs. This
avoids both leaking shapes through stale caches and false cache hits.

A weak field has a type that is a `Gc<T>` or an id type that declares a
liveness check (`ShapeId` implements `WeakId` through the shape table).

---

### 9. Heaps and threads

- **One mutator thread per heap.** A process can create any number of heaps
  (isolates), each with its own `Interp`. This is the model of Lua states, V8
  isolates, and CPython subinterpreters.
- **As implemented.** Heaps on different threads run at the same time with
  no locks: `std.gc` has no process-wide state. Everything a collection
  touches hangs off its `Heap` (chunks, segments from its own `mmap`
  calls, free lists, large objects, the remembered set, scanners,
  sweepers, ephemerons, weak tables, interners); `gc.is_marked(p)` and
  the write barrier find the heap's data through the chunk header of `p`
  (`ChunkHead.rs`), never through a global table. The roots the compiler
  registers live on the shadow stack, whose top (`@loimi.shadow_top`) is
  thread-local, so a collection only sees the frames of its own thread.
  A heap may move between threads (a host hands a state to another
  worker) as long as one thread uses it at a time and the hand-over is
  synchronized (a mutex, a queue, `pthread_join`). `LOIMI_GC_STATS` is
  read with `getenv` in `Heap.new`: do not `setenv` concurrently.
  `examples/embed/threads.c` runs four Mini states on four threads (two
  under GC stress), clean under ThreadSanitizer.
- **Several threads in one heap, one at a time** (`std.gil`, above,
  "Threads sharing a heap"): the threads register with the heap's
  lock, and the holder's collections also walk the shadow stacks the
  others saved when they let go of it. Several threads in one heap at
  the same time (free threading) is not built.
- **Several heaps on one thread.** The shadow stack is per thread, not
  per heap: when a native of state B runs code of state A (a nested run
  into another state), A's collections see B's frames too. The root walk
  therefore marks only objects of the heap being collected (a check of
  the chunk header's heap, made only while walking the shadow stack):
  marking B's objects with A's mark would make B's next collection take
  them for already marked and free what they reference.
- `Gc<T>` doesn't record which heap it belongs to, so cross-heap pointers are
  a bug. Debug builds check the segment header's owning heap at every barrier
  and every handle creation.
- Sending data between heaps is done by copying (`std.gc.transfer` deep-copies
  through `TypeInfo`) or through host-owned buffers.

---

### 10. The policy interface

Heap policies implement a compile-time interface. The compiler lowers object
field stores through `barrier`, and allocation through `alloc_fast`:

<!-- snippet: skip design, not built: no heap policy interface -->
```loimi
interface HeapPolicy {
    const moving: bool;               // true ⇒ compiler reloads spilled refs after `gc` calls
    const needs_old_value: bool;      // true ⇒ compiler loads the old field value before stores
    const barrier_on_value_stores: bool;

    fn barrier(h: *Heap, obj: Gc<Any>, old: Value, new: Value);   // #[inline(always)]
    fn alloc_fast(h: *Heap, class: u8, size: u32) -> *mut u8;    // #[inline(always)]
    fn refill(h: *Heap, class: u8, size: u32) -> *mut u8 uses gc;
    fn alloc_large(h: *Heap, size: usize) -> *mut u8 uses gc;
    fn collect(h: *Heap, kind: CollectKind) uses gc;
    fn on_allocated_during_marking(h: *Heap, p: *mut u8);         // for black allocation
}
```

The built-in policies share the segment, page, allocator, tracer, and root
machinery. Only the cycle logic and barriers differ.

---

### 11. Debugging and observability

| Feature | How to enable | What it does |
|---|---|---|
| **Stress** | `HeapConfig.stress = true` (as built: `heap.set_stress(true)`; there is no environment variable) | collect at every refill; with `stress_every_alloc`, at every allocation (slow) |
| **Verify** | `HeapConfig.verify = .before_and_after` | after marking, check that every pointer in every marked object refers to a marked, valid object of a known type. Catches missing roots and missing barriers |
| **Poison** | debug builds | freed cells are filled with `0xDEAD...`; use-after-free shows up as an obviously invalid value |
| **Snapshot** | `heap.snapshot(writer)` | writes the object graph (nodes, edges, sizes, type names) as JSON in a Chrome-heap-snapshot-compatible subset, for finding leaks |
| **Stats** | `heap.stats()` | cycles, pause time histogram, bytes per type, fragmentation per size class |
| **Events** | `heap.on_event(f)` | collection start and end and phase timing, for embedder telemetry |

Effects (language.md §5) prevent missing *roots in handlers* statically.
Stress plus Verify catch what effects can't, namely missing barriers in
`unsafe` code and bugs in custom `trace` functions. The test matrix in
`std.test` runs suites under `stress` by default.

---

### 12. API summary

<!-- snippet: skip design, not built: `Heap.new()` takes no config; see "As implemented" -->
```loimi
struct HeapConfig {
    min_heap: usize = 4 << 20,
    max_heap: ?usize = None,
    growth: f64 = 2.0,
    incremental_step: f64 = 2.0,          // Incremental only
    young_budget: usize = 8 << 20,        // Generational only
    stress: bool = false,
    verify: VerifyMode = .off,
}

fn Heap.new(cfg: HeapConfig) -> Heap;
fn Heap.free(h: *Heap);                    // releases all segments; runs no finalizers

fn Heap.new<T>(h: *Heap, init: T) -> Gc<T> uses gc;               // fast path inlined
fn Heap.new_with_len<T>(h: *Heap, init: T, len: usize) -> Gc<T> uses gc;
fn Heap.collect(h: *Heap, kind: CollectKind) uses gc;             // .minor | .major | .full
fn Heap.stats(h: *Heap) -> HeapStats;

fn Gc<T>.ptr_eq(a: Gc<T>, b: Gc<T>) -> bool;
fn Gc<T>.addr(x: Gc<T>) -> usize;          // stable, because the heap doesn't move objects
fn Gc<T>.identity_hash(x: Gc<T>) -> u64;   // address-based; still stable with a moving
                                           // policy (uses the HASHED flag + side table)
```
