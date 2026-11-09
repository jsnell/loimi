# `std.gc`: the garbage collector

This page explains how to use `std.gc`, what it guarantees and what it
costs. It is for interpreter authors. The API summary is in
[`../stdlib.md`](../stdlib.md#stdgc), and the language features it
relies on (`object`, `Gc<T>`, `new`, effects, tracing) are in
[`../language.md`](../language.md).

<a id="as-implemented"></a>

## Overview

`std.gc` is a mark collector written in Loimi:

- **Non-moving.** Objects never move, so interpreters can keep raw
  interior pointers.
- **Collection only where you call it.** Allocation never collects.
  Only `gc.collect` and `gc.maybe_collect` do, and they are `uses gc`, so
  the compiler knows which calls may collect.
- **No sweep.** Marking records live objects in per-chunk bitmaps, and
  the allocator reuses the gaps. Dead objects are never touched.
- **Optional generational mode**, switched on at run time.

A minimal program: a list kept alive by a root scanner, with collections
along the way.

<!-- snippet: run "1000" -->
```loimi
import std.gc;
import std.io;

object Pair { car: i64, cdr: ?Gc<Pair> }

/// Roots the compiler doesn't know about: here, one global list.
struct Roots { list: ?Gc<Pair> }

fn scan(h: *gc.Heap, r: *Roots) {
    gc_mark(h, r.list);
}

fn main() uses gc {
    var heap = gc.Heap.new();
    var roots = Roots { list: None };
    heap.add_scanner(&roots, scan);
    for i in 0..1000 {
        roots.list = Some(new(&heap, Pair { car: i, cdr: roots.list }));
        gc.maybe_collect(&heap);
    }
    gc.collect(&heap);
    var n: i64 = 0;
    var p = roots.list;
    while let Some(x) = p { n += 1; p = x.cdr; }
    io.print_int(n); io.println("");
    heap.free();
}
```

## Roots

A collection marks from two places:

- **The shadow stack**, which the compiler maintains. At each call that
  may collect (`uses gc`), compiled code links a frame listing the
  traced locals in scope and values pending across defers. In vm code
  it also lists a copy of each state variable of a traced type (a cached
  top of stack, a current closure), taken at the call, so the variable
  itself can stay in a register.
- **Root scanners** that you register with
  `heap.add_scanner(data, scan)`. A scanner marks everything else: VM
  stacks in memory (up to the top that `sync` published), frames,
  globals, constants.

So nothing that lives only in a handler's registers needs scanning.
`rt.protect` and `rt.catch_panic` restore the shadow-stack top when they
unwind.

## Collection and growth

`gc.maybe_collect(&heap)` collects when `heap.allocated >
heap.threshold`. Interpreters can also test that in their handlers and
call `gc.collect`.

After a collection, the heap may grow by `percent`% of the live bytes,
but by at least `min_heap`, before the next one:

| Setting | Default | Meaning |
|---|---|---|
| `set_growth(percent, min_heap)` | 100%, 1 MiB | The budget until the next collection. The default collects again after allocating as much as survived, so the heap peaks at about twice the live data. |
| `set_max_budget(bytes)` | 0 (no cap) | Caps the budget, so that a large growth factor bounds the heap at the live data plus `bytes`, rather than at a multiple of it, once the live data is large |

No setting was best for every interpreter, so the default stays at 100%.
A larger factor helps some programs and hurts others, and a large
minimum slows small allocation-heavy programs, as the heap stops fitting
in the CPU caches. The examples choose their own:

- MiniJS uses 300% with an 8 MiB minimum (see
  `examples/minijs/README.md`).
- Scheme uses 400%, a 2 MiB minimum and a 256 MiB cap (see
  `bench/scheme/README.md`).

## Heap layout

What an object costs:

- **The header is 16 bytes**: `{ size: u32, tid: u16, mark: u8, flags:
  u8, len: u64 }`, where `len` is the trailing array's length. Compiled
  code reads it at object - 8. A two-word object such as a Lisp pair
  takes 32 bytes.
- **Small objects**, up to 512 bytes including the header, are rounded
  up to a size class (16-byte steps). They live in 64 KiB chunks, each
  holding one size class.
- **Large objects** are `calloc`'d separately.

Chunks are carved out of 2 MiB segments, so each segment can be a
transparent huge page. Within a chunk, the objects start at an offset
that varies from chunk to chunk (0 to 4 KiB), so that objects in fresh
chunks don't share page offsets with other page-aligned memory such as
VM stacks; that sharing slowed some benchmarks by up to 20%.

**Mark bitmaps, no sweep.** Each chunk has a bitmap with a bit per
16-byte granule. Marking an object sets the bits of all its granules.
After a collection the clear bits are the free memory, so dead small
objects are freed without being touched. Each size class allocates by
bumping a pointer through a *run* of clear bits in one of its chunks;
when the run is used up, `refill` finds the next run in the bitmap.
Since every object in a chunk has the chunk's size class, runs start and
end at object boundaries and no memory is lost to small gaps.

What this means for the cost of a collection:

- Dead objects cost nothing to free. Garbage-heavy programs (Prolog's
  backtracking, for instance) benefit most.
- The remaining costs are marking and `refill`.
- A chunk with nothing marked goes to a pool that any size class can
  take, so an all-dead chunk is reused without a pass over its objects.
- Chunks so fragmented that their runs hold fewer than 8 objects on
  average are used only after the pool of empty chunks, since each run
  costs a `refill` call.
- In a major collection, marking prefetches object headers, so the
  cache misses of up to 16 objects overlap.

[`../results.md`](../results.md#gc-mark-bitmaps-instead-of-sweeping-2026-11-04)
has the measurements.

## Allocating in handlers

`new(h, T { .. })` may call `refill`, and in a tail-call handler that
call site gives the handler a stack frame on every execution, even
though the refill is rare. Two forms avoid this.

**`try_new(h, T { .. }) -> ?Gc<T>`** is `new` without the refill. It
bumps the size class's run or returns `None`, and makes no call.
`gc.make_room(h, size)` refills the size class for `size` bytes
(`size_of<T>()` plus the trailing array) and never collects. A handler
calls it in a cold block and `redo`es the op:

```text
let Some(v) = ctx.try_var() else cold { ctx.make_room(); redo };
```

Large objects are allocated as by `new`. The language reference covers
this pattern ("Allocating and storing references in a handler").

**Unzeroed allocation.** `new_uninit(h, T { .. }, n)` is `new` without
zeroing the trailing array. It is for strings and similar arrays whose
contents are copied in at once; zeroing large strings took about a
third of the time in Scheme's `string` benchmark.

- The array must be one the collector does not trace. The compiler
  rejects traced element types, also in `unsafe`.
- Outside `unsafe { }`, every bit pattern must be a valid element:
  numbers, vectors, plain structs of them.
- The fixed fields are stored from the literal, and the header is
  written as usual.
- The array holds zeros in fresh memory and stale bytes in reused
  memory. Verify mode and `--gc-check-roots` builds fill it with 0x01
  bytes instead, so a program that reads it before writing it reads the
  same bytes on every run in those modes, not zeros by luck.

## Generational mode: sticky mark bits, non-moving

`heap.set_generational(true)` turns it on; it is off by default. The
source does not change: the write barrier is always compiled in. Mini,
MiniJS and Scheme turn it on.

- *Young* objects were allocated since the last collection; *old* ones
  survived one (large objects: two).
- **Old objects keep their mark** (sticky marks). A minor collection
  marks from the roots and from the remembered set, and stops at marked
  (old) objects. A minor pause is marking only.
- **Small objects are promoted when they first survive.** Large objects
  stay young through one more minor collection.
- **A major collection** marks everything again. It runs once the old
  generation has grown by the growth budget since the last major
  collection.
- **The nursery** (the allocation budget between minor collections) is
  by default the same budget as between full collections, computed from
  the live bytes after the last major collection. Cap it with
  `heap.set_nursery(bytes)`. Small nurseries measured worse: they
  promote objects still in use, which fill the old generation and bring
  major collections sooner.

In stress mode with verify on, a generational heap collects after a
varying 0 to 4 KiB of allocation rather than at every `maybe_collect`,
so objects stay young across a few calls and stores into old objects
exercise the barrier. Every eighth collection is major.

## The write barrier: object logging

<!-- snippet: excerpt std/gc.lm -->
```loimi
#[inline]
pub fn barrier(obj: *u8) {
    let hd = (obj - HEADER) as *Header;
    if unlikely((hd.flags & LOG) != 0) { log_obj(obj); }
}
```

**What the compiler does.** It calls `gc.barrier(obj)` after every store
of a traced value into an object. In a non-generational heap no object
has the `LOG` flag, so the barrier costs a byte test and a branch that
is never taken. A store of a value that statically holds no references
(a variant without a `Gc` payload, such as `Int(n)` or `None`, or a
literal tuple or struct of those) gets no barrier.

**The slow path** clears `LOG` and adds the object to the remembered
set. It runs once per object between collections. It is inline and
makes no call, so it gives a tail-call handler no stack frame. When the
set is full it only counts the object, and the next collection is then
a major one.

Being inline, it makes functions with barriers larger, which can stop
LLVM from inlining them. For such a helper, store through raw pointers
and log with `needs_barrier` and `remember` (below).

**Slices of trailing arrays.**

- A store through `o.items[i]`, or a sub-slice of it, gets the barrier.
- Passing `o.items` to a function that may store references through it
  (such as `fill(o.items, v)`) puts the barrier on `o` after the call.
  That function must not collect or keep the slice.
- A slice of a traced trailing array cannot be kept where the compiler
  would lose track of stores through it (a `var`, a field, a return
  value), or be stored through from a `let`. Those are compile errors.
- `o.items.ptr` is a raw pointer: stores through it need
  `gc.write_barrier`.

**Stores through raw pointers** bypass the compiler's barrier. Call
`gc.write_barrier(h, obj)` after them, naming the object that contains
the slot (not an interior pointer). To log several objects together,
use `gc.needs_barrier(obj)` and `gc.remember(obj)`. Mini logs a table
and its array and hash parts together, so its table stores test only
the table's own header.

**Verify mode** (`heap.set_verify(true)`) finds missing barriers. At the
start of every collection it checks every old object outside the
remembered set. A reference to a young object (a missing barrier) or to
freed memory (a dangling reference) aborts with both type names:

```
gc verify: old object of type value.Arr points to a young object of type value.Str but is not in the remembered set: missing write barrier
```

Verify mode also fills free memory with 0x01 bytes after each
collection, which is what makes dangling references visible.

## Checking roots: `--gc-check-roots`

`loimi build --gc-check-roots` (also for `run` and `test`) builds a
program whose collector checks every reference that generated code
hands it, before reading its header. That covers shadow-stack roots,
`gc_mark` calls in scanners and `#[trace]` hooks, and traced fields.

- A reference must be null or the start of a live object of the heap
  being collected.
- Unassigned traced slots and the objects a collection frees are filled
  with 0x01 bytes. So a root listed too early or kept too long is caught
  the first time it is marked.
- A bad reference is reported with its frame, function, call site and
  slot.

The example interpreters' test scripts run their suites in this mode
(for example `examples/python/check.sh`).

## Weak references and ephemerons

**`Weak<T>`** is a library object, `object Weak<T> { addr: u64 }`.

- Make one with `heap.new_weak(x)`, read it with `w.get() -> ?Gc<T>`,
  change it with `w.set(..)`.
- After marking, the collector clears weak references whose target was
  not marked.
- There is no `weak` field qualifier: an object that needs a weak field
  holds a `Gc<Weak<T>>`.

**`Ephemerons<K, V>`** is a table from keys `Gc<K>` (by address) to
values of any traced type `V`, kept in host memory.

- Create it with `gc.Ephemerons.new(h)`; release it with `t.free()`.
  `get`, `set`, `remove`, `len`.
- A value is kept alive only while its key is. Entries whose key died
  are dropped.
- The table is not a GC object itself, so a guest `WeakMap` object frees
  it with a free hook (`h.on_free`, below).

**Weak intern tables for GC strings.** `gc.WeakInterner.new(&heap, make)`
returns a `*WeakInterner<X>` over your string object type `X`, which
needs fields `hash: u64` and `data: [u8]`. `make(h, s, hash)` allocates
one; or use the ready-made `gc.Str` and `gc.new_str`.

- `intern(s) -> Gc<X>` returns the one live string with those contents.
  It may allocate but never collects.
- `find(s)`, `len`, `free`.
- Entries are weak: dead strings leave the table at the next collection.
- `key(x) -> u64` pins `x` (keeps it alive and in the table for the
  table's lifetime) and returns its address. That address is a stable
  key for `std.obj` properties and other tables keyed by name. An
  unpinned string's address can be reused once it dies. `is_pinned(x)`
  tells.

**Sweepers.** For other weak tables, register a sweeper with
`heap.add_sweeper(data, sweep)`. It runs after marking and drops
references to dead objects, testing them with `h.is_marked(p)` (or the
slower `gc.is_marked(p)`, which finds the heap from the object). In a
minor collection old objects count as marked, and `heap.is_minor()`
tells the sweeper that only young objects can have died. This suits
tables keyed by content rather than by object, such as Mini's string
intern table.

The order after marking is: ephemerons, weak references, then sweepers,
with free hooks first.

## Trace hooks, free hooks and dirty pinning

Three hooks for objects whose layout or lifetime the generated code
cannot see. Mini's coroutines use all of them.

### `#[trace(f)]`: custom tracing

Use it when only part of a trailing array is live, or when an object
owns host memory that holds references.

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

- The generated trace function still traces the declared fields. Then,
  instead of the whole trailing array, it calls `f(h, obj)`, which marks
  what is live with `gc_mark`. Because the fields stay generated, a hook
  cannot forget one.
- `f` is a function of the object's module, `fn f(h: *gc.Heap, obj:
  Gc<Obj>)`, without effects (it runs inside a collection). Generic
  objects cannot have one.
- Minor collections, verify mode and `--gc-check-roots` all go through
  the trace function, so they handle hooked objects like any other.
- A store that changes what the hook marks (a young reference into the
  owned memory, or raising `top` over slots holding young references)
  needs `gc.write_barrier(h, obj)`, unless the compiler emits one (a
  store into the trailing array does).

### `h.on_free(obj, f)`: free hooks

Use them to release host memory that an object owns.

- `f(obj)`, a `fn(*u8)`, runs once, when a collection finds `obj`
  unreachable, before its memory can be reused. `Heap.free` runs the
  hooks of all objects still registered.
- `h.on_free_gc(obj, f)` is the same with `f: fn(Gc<T>)`, so the hook
  can read the object's fields without an `unsafe` cast.
- Hooks are registered per object, so objects that own nothing yet cost
  nothing.

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

**What a hook may do:** release host memory, close descriptors, update
host counters, and read the object's own non-reference fields (raw
pointers, integers).

**What it must not do:** allocate in the heap, collect, register hooks,
follow references (what they point to may be dead too), or store `obj`
anywhere. There is no resurrection.

**When it runs:** inside the collection, after marking, ephemerons and
weak references, before your sweepers. Objects that die together run
their hooks in no particular order.

**Why so narrow.** A general finalizer (Lua's `__gc`, Python's
`__del__`) runs guest code, which may allocate and resurrect objects, so
it must run outside the collector. Releasing native memory needs none
of that. You can build guest finalizers on top: the hook queues a
host-side record, and the interpreter runs the guest code at a safe
point.

**Cost.** Heaps without hooks pay nothing. A minor collection looks only
at the hooks of young objects.

### `h.pin_dirty(obj)`: objects written without barriers

For a GC object that the interpreter writes in a hot loop, such as a
register file or a value stack kept in the heap.

- While pinned, the object is a root, and every collection traces it,
  minor ones included, as if it were always in the remembered set.
- Stores into it through raw pointers need no `write_barrier`, and
  verify mode does not check it.
- `h.unpin_dirty(obj)` puts an old object into the remembered set, to
  cover the stores made since the last collection. From then on, stores
  need the barrier again.
- Pins nest. `h.is_pinned_dirty(obj)` tells whether an object is pinned.
- Each collection traces the whole object. Combine pinning with a
  `#[trace]` hook to trace only the live part.

A root scanner of your own is not a substitute: verify mode would still
check the object, and a scanner cannot put an object into the
remembered set.

## Threads sharing a heap: `std.gil`

Independent heaps share nothing, so threads with separate heaps need no
locks. For several threads sharing one heap (and the interpreter state
around it), `std.gil` provides a global interpreter lock per heap. A
program that does not import it is unchanged. Mini built with
`--cfg gil` is the worked example (`examples/mini/threads.lm`,
`examples/mini/embed.lm`, `examples/embed/shared.c`).

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

### The lock

| Call | Meaning |
|---|---|
| `Gil.new(h, data)` | Create the lock, held by the calling thread, which is attached with per-thread state `data` |
| `g.attach(data)` | Attach another thread; waits for the lock |
| `g.detach()` | Detach the calling thread, which must hold the lock |
| `g.unlock()`, `g.lock()` | Let go of the lock and take it back, around blocking code |
| `g.waiting()` | Whether threads are waiting (a relaxed atomic load) |
| `g.yield_now()` | Let the waiting threads run, then take the lock back (`uses gc`) |
| `g.mine()` | The calling thread's record (null if not attached) |
| `g.holder()` | The holder's record |
| `g.switches()` | How many times the lock has changed hands |
| `g.free()` | Release the lock, once the calling thread is the only one attached and holds it. Free the heap first. |

Waiting is first come, first served.

### Handing over at safepoints

The holder polls `g.waiting()` where its state is synced; Mini does so
at every taken jump and every call of a Lua function. When it is
non-zero, `g.yield_now()` hands over. Polls are plain code in the
handlers:

```text
#[cfg(gil)] if unlikely(ctx.gil.waiting()) { cold { ctx.gil.yield_now(); } }
```

A handler syncs and roots its state for the poll as for any call that
may collect.

### Per-thread state

The interpreter keeps one copy of the per-thread parts (stacks, the
running frame): the holder's. The hooks move them:

- `save(data, thread)` when a thread lets go of the lock;
- `load(data, thread)` when one takes it (a thread attached with fresh
  data gets its stacks there);
- `scan(h, data, thread)` during collections.

### Roots of other threads

`Gil.new` registers a root scanner. In each collection (run by the
holder) it marks, for every other attached thread, the shadow-stack
frames the thread had linked when it let go of the lock, and its
per-thread state (`scan`). `gc.mark_shadow_stack(h, top)` does the
marking of saved frames, and marks only this heap's objects. While a
thread has let go of the lock, it must not touch the heap.

Generational heaps need nothing more: stacks are scanned as roots in
every collection.

No other synchronization is needed. The collector, the write barrier,
allocation, quickening and inline caches run only on the holder, and
the mutex orders one holder's writes before the next one's reads.
`examples/embed/shared.c` is clean under ThreadSanitizer.

## Limitations

- The heap never moves objects.
- There is no incremental marking.
- There is no card marking: a large array that is stored into is
  retraced whole by each minor collection.
- Small objects are promoted when they first survive.
- Finalizers cannot run guest code; free hooks release host memory.
- Threads share a heap one at a time (`std.gil`); free threading is not
  supported.
- Inline caches in bytecode cannot hold GC references, because bytecode
  is not scanned.
- Unassigned slots and freed objects are poisoned only in
  `--gc-check-roots` builds.

## Statistics

`LOIMI_GC_STATS=1` prints a line per collection to stderr:

- the kind of collection;
- the live (old) bytes;
- the remembered set's size;
- the mark and sweep times (the sweep is only that of large objects);
- the time spent in `refill` finding runs so far (labelled "lazy
  sweep").

## Invariants of the bitmap allocator

This section is for people changing `std/gc.lm`. The collector relies
on these rules. `tests/run/gc_torture.lm` (a random mutator in six heap
configurations) and the interpreters' suites under stress, verify and
`--gc-check-roots` check them.

1. **One class per chunk, on a grid.** Every object in a chunk has the
   chunk's size class and starts at `start + k * class` granules. A
   chunk changes class only from the pool, that is, with no mark bit
   set, so a run of clear bits starts and ends at object boundaries.
2. **Clear bit = free, after every collection.** Marking sets the bits
   of every granule of each small object it reaches; allocation never
   sets bits. A minor collection keeps old objects' bits and promotes
   every small survivor, so after marking a clear bit is free memory or
   a dead object. Objects allocated since the last collection have
   clear bits.
3. **No granule is handed out twice in a cycle.** `refill` walks each
   class's chunk list forward and takes pool chunks out of the pool. The
   lists are rebuilt only by `relink_chunks`, after marking, which also
   resets every class's run (`cur`, `lim`).
4. **Mark byte and bits agree for live objects.** The header's mark
   byte holds the heap's epoch (1 or 2) and is the "already marked"
   test. A major collection flips the epoch and clears the bitmaps
   together; a minor one keeps both. Only `mark_now` sets a mark on a
   small object, and it sets the bits with it.
5. **Dead objects are never read.** Their headers keep stale contents,
   so nothing may reach a dead object. Weak references, ephemerons, free
   hooks and sweepers drop it in the collection that finds it dead,
   while its memory is intact. Whatever overwrites dead memory runs
   after them (verify mode's fill, `--gc-check-roots`'s
   `rootck_poison` as the last sweeper). In a generational heap an old
   object that died stays allocated until the next major collection.
6. **Only this heap's objects get this heap's marks.** A reference to
   another heap's object may appear only on the shadow stack (nested
   runs, `std.gil`'s saved frames). Those walks check the chunk's heap
   and mark at once, never through the prefetch ring.
7. **The prefetch ring is empty whenever marks are read.** It is
   flushed after the scanners, whenever the mark stack runs dry, and in
   each ephemeron pass. Minor collections and shadow-stack walks don't
   use it.
