# `std.gc`: the garbage collector

This page describes what `std/gc.lm` implements. It expands the `std.gc` section of [`../stdlib.md`](../stdlib.md)
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
- **Fragmented chunks after the pool** (2026-11-04). After marking,
  `relink_chunks` counts each chunk's marked granules and free runs
  (popcounts over the bitmap). A chunk whose runs hold fewer than
  `POOR_RUN` (8) objects of its class on average goes on its class's
  second list, which `refill` takes only once the pool of empty chunks
  is used up: a run costs a `refill` call, and a heap whose survivors
  are scattered (Prolog's boyer: 30 MB, 2-5 MB live) has thousands of
  short runs. The other chunks of the class still come first, so memory
  stays as it was: taking the pool before every chunk of the class cut
  boyer's `refill` time to a third too, but made its peak memory 1.15
  times as large (a class used the pool while its own chunks' runs went
  unused, and another class then needed new chunks).
- **Marking prefetches** (2026-11-04). In a major collection `mark_ptr`
  does not read the object's header at once: it prefetches it
  (`p.prefetch()`) and queues the reference in a 16-entry ring, marking
  the reference it pushes out (and the rest when the mark stack runs
  dry), so the cache misses of up to 16 headers overlap. Boyer's mark
  time fell from 232 to 177 ms, tak's from 96 to 86 ms. Minor
  collections mark at once: there `young_ref` must belong to the object
  being traced. Reading the header early with a plain load instead of
  a prefetch was slower than not deferring (boyer 252 ms), and tracing
  popped objects through a FIFO, without prefetching, gained nothing.
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
- **Unzeroed allocation** (2026-11-04). `new_uninit(h, T { .. }, n)`
  (compiled to `gc.alloc_uninit`) is `new` without zeroing: small
  objects skip the `memset`, large ones come from `malloc` instead of
  `calloc`, and the fixed fields are stored from the literal. Only for a
  trailing array the collector does not trace (the compiler rejects
  traced element types, also in `unsafe`) and, outside `unsafe { }`,
  whose every bit pattern is a valid element (numbers, vectors, plain
  structs of them): strings whose bytes are copied in at once. The
  invariants above are untouched: the header (size, type id, mark 0,
  flags 0 or `LARGE`, length) and a large object's `ChunkHead` are all
  written as by `alloc`, nothing reads the array's old bytes (a dead
  object's, which no sweeper reads any more after its collection,
  invariant 5), and the tracer never looks at them. What the array
  holds is zero in fresh memory and stale bytes in reused memory;
  under verify mode (`set_verify`) `alloc_uninit` fills it with 0x01
  bytes, as `poison_free` fills dead memory, and a `--gc-check-roots`
  build allocates with `rootck_alloc_uninit` (`rootck_alloc`, which
  records the object's start, then a 0x01 fill), so a program that
  reads such an array before writing it reads the same bytes in every
  run of those modes instead of zeros by luck. Scheme's `string` benchmark
  (string-append, substring of strings of thousands of characters)
  went from 0.019 to 0.015 s; it spent about a third of its time
  zeroing them. Tested by `tests/run/gc_new_uninit.lm` and
  `gc_torture.lm` (half of its objects).
- **Collection triggers**: `growth` (percent of the live bytes),
  `min_heap` and `max_budget` (`set_growth`, `set_max_budget`).
  Interpreters test `heap.allocated > heap.threshold` in their handlers
  and call `gc.collect`.

### Invariants of the bitmap allocator

What the collector relies on (checked by `tests/run/gc_torture.lm`, a
random mutator in six heap configurations, and by the interpreters'
suites under stress, verify and `--gc-check-roots`):

1. **One class per chunk, on a grid.** Every object in a chunk has the
   chunk's size class and starts at `start + k * class` granules. A chunk
   changes class only from the pool, that is with no mark bit set, so a
   run of clear bits starts and ends at object boundaries of the class.
2. **Clear bit = free, after every collection.** Marking sets the bits
   of every granule of each small object it reaches; allocation never
   sets bits. A minor collection keeps the bits of old objects and
   promotes every small survivor (only large objects age), so after
   marking in either kind of collection a clear bit is free memory or
   a dead object. Between collections, objects allocated since the last
   one have clear bits.
3. **No granule is handed out twice in a cycle.** `refill` walks each
   class's chunk list forward (`scan_g`) and takes a pool chunk out of
   the pool; the lists are rebuilt only by `relink_chunks`, after
   marking, which also resets every class's run (`cur`, `lim`).
4. **Mark byte and bits agree for live objects.** The mark byte is the
   "already marked" test. A live small object whose mark is the epoch
   has its bits set: a major collection flips the epoch and clears the
   bitmaps together, a minor one keeps both. Only `mark_now` sets a mark
   on a small object, and it sets the bits with it.
5. **Dead objects are never read.** Their headers keep whatever they
   held (a stale mark may equal the epoch again two major collections
   later), so nothing may reach a dead object: weak references,
   ephemerons, free hooks and weak tables (sweepers, `is_marked`) drop
   it in the collection that finds it dead, while its memory is intact.
   What overwrites dead memory runs after them: verify mode's 0x01 fill
   at the end of the collection, `--gc-check-roots`'s `rootck_poison` as
   the last sweeper. In a generational heap an old object that died stays
   allocated (its bits set) until the next major collection, so what an
   old object references stays valid until then.
6. **Only this heap's objects get this heap's marks.** A reference to
   another heap's object may appear only on the shadow stack (nested
   runs, `std.gil`'s saved frames); those walks check the chunk's heap
   (`OWN_ONLY`) and mark at once, never through the prefetch ring
   (a deferred mark would be made after the check is switched off).
7. **The prefetch ring is empty whenever marks are read**: it is
   flushed after the scanners, whenever the mark stack runs dry, and in
   each ephemeron pass; minor collections and the shadow-stack walks do
   not use it.

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
  entries keyed by it are gone), as the first sweeper, before the
  interpreter's sweepers (and `--gc-check-roots`'s `rootck_poison`,
  always the last one: it overwrites dead objects, headers included).
  Nothing is swept: the memory is reused only by allocations after the
  collection.
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
  for one more cycle. Releasing native
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

### Limitations

- The heap never moves objects (see "Generational mode" above):
  interpreters keep raw interior pointers.
- Generational mode is a run-time switch; the barrier is always
  compiled in. There is no incremental marking.
- There is no card marking: a large array stored into is retraced whole
  by each minor collection. Small objects are promoted when they first
  survive (aging them measured worse, above).
- Finalizers cannot run guest code; free hooks release host memory
  (above).
- Threads share a heap one at a time (`std.gil`, above); free
  threading is not supported.
- Inline caches in bytecode cannot hold GC references: bytecode is not
  scanned (language.md §7.1).
- Poisoning of unassigned slots and freed objects is done only in
  `--gc-check-roots` builds (above).

`LOIMI_GC_STATS=1` prints a line per collection to stderr: kind, live
(old) bytes, the remembered set's size, the mark and sweep times (the
sweep is now only that of large objects), and the time spent in
`refill` finding runs so far (still labelled "lazy sweep").
