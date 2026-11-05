# What's new

Features by topic, with links to where they are documented.
[`docs/status.md`](docs/status.md) has the complete state of the
implementation; `git log` has the details.

## Since 2026-11-02

### `vm` declarations and dispatch

- **Debugging guest programs** (prototype): `on break { .. }` generates
  breakpoint ops and `Vm.set_break`/`clear_break`/`Break.rearm`, sound on
  verified code (size-preserving, supers split, caches kept);
  `#[unfused]` variants run supers as their parts; `#[cfg(..)]` on vm
  items ([language.md §9.1](docs/language.md#91-breakpoints-on-break)).
  `std.debug` has the step-over, line stepping and a command reader
  ([stdlib.md](docs/stdlib.md#stddebug-guest-debuggers)). Mini built with
  `--cfg debug` is a gdb-like debugger (`mini --debug script`); see
  [design/debugging.md](docs/design/debugging.md).
- **A second guest debugger, for the Python subset** (`--cfg debug`,
  `python-loimi --debug prog.py`, pdb-like: breakpoints in methods and
  generators, stepping across `yield` and into `sorted` keys, `finish`
  across exceptions; [SPEC.md](examples/python/SPEC.md#debugger---cfg-debug)).
  It needed no change to the generated part; `std.debug` gained
  `Stepper.left`, `line_sites`, `unpatched` and "line 0 is no line", and
  struct literals take `#[cfg(..)]` on field initializers
  ([design/debugging.md §8](docs/design/debugging.md#8-second-interpreter-python)).
- **Debugging Mini from an editor**: a stop-callback C API in the
  `--cfg debug` library (`mini_debug_*`: breakpoints by file and line,
  conditions and hit counts, break on uncaught errors, pause from any
  thread, frames, locals, globals, lazily expanded tables, evaluation),
  and `mini-dap`, a Debug Adapter Protocol server in C over it
  (`examples/mini/dap`; VS Code or any DAP client). Conditions run as
  nested runs from inside the stopped handler, with the breakpoints
  suspended (`std.debug`'s `Breakpoints.suspend`); a pause is a flag
  polled at back-edges and calls in debug builds (0.5% in total, within noise).
  The command line gained `break LINE if COND` and `print EXPR`
  ([design/debugging.md §9](docs/design/debugging.md#9-editors-a-stop-callback-api-and-a-dap-adapter),
  [SPEC.md §8](examples/mini/SPEC.md#8-debugger---cfg-debug)).
- **`Vm.Resume` inline cache fields**: verified code addresses in the
  instruction stream (try chains, cached callees); the verifier resets
  them to the placeholder, `v.fill_resumes(vals)` sets them, `jump
  cache.to` is allowed under `#[require_verified]`; and
  `Vm.emit.OP_cached(..)` emitters that take a cache's initial contents
  ([language.md §7.1](docs/language.md#71-inline-caches)).
- **`#[inline(never)]` on a `-> never` vm function** now keeps the shared
  copy out of line (it was inlined back by LLVM), and callers skip the
  instruction word when the copy never reads it
  ([§3.5](docs/language.md#35-vm-functions)).

- **Accumulators**: `accumulator acc: T for kinds;` state registers and
  `#[accumulate]` ops whose generated variants keep a temporary in a
  machine register between adjacent ops, picked by `Vm.accumulate`
  ([language.md §7.4](docs/language.md#74-accumulators)).
- **`next_pc`**, the address `next` continues at, for return addresses
  in vm functions; **`then_next_pc`**, the address past an op's `then`
  instruction (with a `next_pc_then` warning for `next_pc` there)
  ([§3.5](docs/language.md#35-vm-functions)).
- **Resume points**: `Vm.Resume`, `next_resume`, `v.resume_at(i)`, so
  `#[require_verified]` entries resume at a saved pc without `unsafe`
  ([§6](docs/language.md#6-verification-unchecked-by-proof)).
- **Per-format setters and encoders**: `Vm.set_ABC_a(w, v)`
  (range-checked) and `Vm.encode.ABC(op, a, b, c)`
  ([§3.3](docs/language.md#33-encodings-and-formats)).
- **`#[forward]` across slot views**: operand kinds that differ only in
  the type they cast to link like one kind
  ([§7.3](docs/language.md#73-superinstructions)).
- **Diagnostics in vm-expansion code**: errors in user text copied into
  a vm's expansion (`verify` clauses, contexts, stack effects) point at
  the user's source ([status.md](docs/status.md), Compiler row).
- **Handlers without stack frames**: tail-call handlers are compiled
  without `memset`/`memcpy` formation and without loop vectorization or
  unrolling ([lowering.md §3](docs/lowering.md#3-handlers-in-tailcall-mode));
  `-W handler_frame` reports handlers whose fast path still sets up a
  frame, with the calls that cause it
  ([language.md §4](docs/language.md#4-slow-paths-cold-blocks)).
- **goto mode** emits one shared dispatch block that LLVM copies into
  every handler after register allocation: build time near linear in the
  op count ([lowering.md §5](docs/lowering.md#5-goto-mode)).
- **switch mode** builds with LLVM's tail merging off
  ([lowering.md §6](docs/lowering.md#6-switch-mode)).
- **AArch64**: the tail-call dispatch table's address stays in a
  register ([lowering.md §3](docs/lowering.md#3-handlers-in-tailcall-mode)).
- **The generated verifier** sizes its instruction-boundary bitmap to
  the code (a 256-byte stack array up to 2048 words, else calloc'd)
  instead of zeroing 64 KiB per call: Prolog's `assertz`, which verifies
  each new clause, spent half its time there.
- **JIT (`--jit-stencils`)**: jumps to a `Vm.Resume` count toward
  compiling like calls (Prolog enters clauses from its try chains that
  way, and their code was never compiled), and the registry of code
  objects is a treap instead of a sorted array (Prolog's sieve, which
  asserts 700,000 clauses, spent 85% of its time in its inserts). Prolog's
  JIT build: 1.04 → 0.98 of the interpreter's time, sieve 46 → 1.3
  ([design/copy-and-patch.md §14](docs/design/copy-and-patch.md#14-prolog-2026-11-04)).

### Values

- **`#[scan_raw_words]`** on a value type: a checked guarantee that words
  below 2^48 (raw pointers, small integers) read as reference-free
  variants, for scanners of mixed frames
  ([language.md §2](docs/language.md#2-value-types-declared-representations)).

- **Shared nanbox blocks**: when the 48-bit blocks run out, up to 8 `Gc`
  variants share one, told apart by the pointer's low 3 bits (up to 56 in
  all) ([language.md §2](docs/language.md#2-value-types-declared-representations),
  [lowering.md §2](docs/lowering.md#2-values-nanbox-lowtag-boxed)).
- **Layout visibility**: `loimi build --explain-values`, `block N` pins,
  the `value_layout` warning, and `V.X.mask`/`tag`/`block` constants
  ([language.md §2](docs/language.md#2-value-types-declared-representations)).
- Division checks on odd-width integers (`i48`) fold after a `y > 0`
  guard ([lowering.md §2](docs/lowering.md#2-values-nanbox-lowtag-boxed),
  "Overflow-checked arithmetic on odd widths").

### Core language

- **Pointer stability** (prototype; `docs/design/lifetimes.md`): in
  `#![safe]` and `#![lifetimes]` modules, a pointer or slice into a
  container used after a call that may move or free it (`push`,
  `insert`, `free`, a collection, a `reenter` call in a vm handler), a
  copied `Vec` grown while the other copy is used, and the address of a
  local that outlives its function are compile errors. Inferred over
  the call graph: std needed one `#[may_alias(s)]` (`Vec.extend`). No
  IR change.
- **Every example interpreter checks pointer stability**: the 77
  modules of Lox, Mini, MiniJS, Prolog, Python, regex, Scheme, Stack and
  Wasm are `#![lifetimes]`. Found and fixed: MiniJS's `Gen.top` left
  `g.fs` pointing at a dead local; Mini's `coroutine.yield` lent a
  native's `args` to the context (the CALL instruction now sets
  `xfer`); the Stack fuzzer freed the program before the machine
  pointing into it. The inference got more precise (a function value's
  result points into an argument only where the types allow it; memory
  a function allocates and returns carries what it keeps to the
  caller's use of the result; captured owners in closures; the depth
  bound) and closed a hole (a store through a pointer of unknown origin
  counted as local). Whole-program checking got 5-50x cheaper (copy-on-write
  flow state, scoped locals, one analysis per function): under 10% of
  the front end on every example (`docs/design/lifetimes.md` sections
  7-9).
- **Pointer stability, phase 2** (`docs/design/lifetimes.md` section
  10, questions 1, 2 and 6): a pointer into *growable* memory (a struct
  field whose block some function moves: `Vec.ptr`, `HashMap.slots`,
  inferred) may not be stored where it outlives the function (a
  parameter's memory, a global, a GC object, an escaping closure; fresh
  memory the function returns hands it to the caller). A call passing
  two arguments that alias a block the callee uses after moving it
  through the other is rejected (`f(&v, &v)`, `f(&c, &c.v)`;
  `#[may_alias]` stays the escape hatch). New `#[shrinks(v)]`
  (`Vec.pop`/`truncate`/`clear`, the maps' `remove`/`clear`): pointers
  into the container end, also through callers (`StrBuf.clear`).
  `HashMap.insert` now counts as moving its table. In the examples: 7
  sites, safe by protocol, marked `unsafe { }` with the reason; no IR
  change.
- **Pointer stability is checked by default** in every module, std's
  unsafe core (`mem`, `gc`, `obj`, `bigint`, `gil`, ..) included
  (`docs/design/lifetimes.md` section 10, question 5).
  `#![lifetimes(off)]` opts a module out (not a `#![safe]` one);
  `#[may_alias]`, `#[shrinks]` and `unsafe { }` stay the finer escapes;
  a bare `#![lifetimes]` is accepted and does nothing, and the examples'
  77 modules dropped it. Outside the examples it found 9 sites: a test
  registered a dead local as a GC root scanner (`tests/run/std_obj.lm`,
  fixed; `tests/fail/lt_scanner_local.lm`), and the 8 ASan tests whose
  stale pointer is the point opt out. std, the tutorial, the benchmarks
  and the doc snippets needed nothing. No IR change.
- **`#![safe]` modules**: a module marked `#![safe]` treats `*T` as a
  reference to one value; pointer indexing, arithmetic, `p.slice(n)`,
  pointer casts, other modules' buffer pointers and invariant fields,
  `..zero` null pointers, `assume`/`unreachable` and calls of `extern`
  and `#[raw]` functions need `unsafe { }` there (a check only: the IR
  is unchanged). `std.mem` marks its pointer+length and freeing
  functions `#[raw]`. Lifetimes are not checked
  ([language.md §1.3](docs/language.md#13-memory),
  [design/safety.md](docs/design/safety.md)). Naming an `#[unsafe]`
  method through `x.m()` or explicit type arguments now needs `unsafe`
  too (it slipped through before). An adversarial pass closed bypasses
  ([design/safety.md §3a](docs/design/safety.md)): pointers in another
  module's tuple, array, option and function-typed fields (`let (p, _) =
  h.tup`, `gc.Heap.free[0]`), pointer payloads of another module's enums
  and `value` types (binding and building), writes to structs that hold
  pointers only through nested structs, `var p: *T;` (zero-initialized,
  null), `new(&a, v)` with an allocator whose `alloc_bytes` is not
  `#[raw]` (could return too few bytes), and `vm` declarations (whose
  handlers are exempt) are rejected in `#![safe]` modules.
- **Stale pointers into std containers are caught under
  `--sanitize=address`**, which now also sets `#[cfg(sanitize_address)]`:
  `Vec` poisons its spare capacity (after `pop`, `truncate`, `clear`,
  and beyond `len` generally) and `Arena` the unused tail of its chunk,
  so a pointer or slice kept across them is reported as
  "use-after-poison" (growth and `free` were already caught as
  use-after-free). Other builds are unchanged (cfg'd out at parse time)
  ([design/safety.md §7](docs/design/safety.md)). `Arena.free` now
  leaves the arena usable (it kept a pointer to its freed chunk, so the
  next allocation wrote to freed memory).
- **128-bit vectors**: `i8x16` .. `f64x2` with lane-wise operators,
  masks, lanes, shuffles, swizzles, widening and saturating narrowing,
  saturating and rounding arithmetic, IEEE min/max and lane-wise `as`;
  usable as fields, array elements and vm state registers, and through
  pointers at any alignment
  ([language.md §1.2, "Vectors"](docs/language.md#vectors)). The wasm
  interpreter uses them for the SIMD proposal.

- Literal-only arithmetic takes the other operand's type
  (`16 + 8 + n` with `n: u64`), and `let t: [T; _] = [..]` takes the
  array literal's length ([language.md §1](docs/language.md#1-core-language)).

- **Raw string literals** `r"..."` and `r#"..."#`: no escapes, may
  span lines ([§1.2](docs/language.md#lexical-structure)).

- **`lane_at`/`with_lane_at`**: vector lanes chosen at run time (the
  index masked to the lane count), `with_lane_at` as a compare and
  select in registers ([language.md §1.2, "Vectors"](docs/language.md#vectors)).
- **`#(a ++ "_X")`** in `inline for` bodies builds one identifier from
  several parts ([§1.4](docs/language.md#14-compile-time-evaluation)).

- **Constants in patterns** match their value; the `unreachable_arm`
  warning; or-patterns with bindings
  ([§1.2](docs/language.md#12-basics)).
- **`for x in`** over arrays and slices ([§1.2](docs/language.md#12-basics)).
- **Float built-ins** on `f32`/`f64` (`to_bits`/`from_bits`, `ceil`,
  `trunc`, `nearest`, `abs`, `copysign`, `is_nan`, `min`/`max`,
  `minimum`/`maximum`) and **`u64` `mul_wide`/`div_wide`**
  ([§1.2](docs/language.md#12-basics)); **carry chains**:
  `add_carry`/`sub_borrow` on `u8`..`u64` (`adc`/`sbb`, `adcs`/`sbcs`)
  and `u64` `mul_add_wide` ([stdlib.md](docs/stdlib.md), `std.core`).
- **`p.read_unaligned()` / `p.write_unaligned(v)`**
  ([§1.3](docs/language.md#13-memory)).
- **`..zero`** in struct literals also nulls raw pointer fields
  ([§1.2](docs/language.md#12-basics)).
- **Closures**: escaping closures with state lists
  (`new(a, [var n = 0] |..| ..)`), cells shared between closures
  (`new(a, v)`), per-closure `c.free(a)`, and local closures
  (`let f = |x| ..;`) ([§1.2](docs/language.md#12-basics), Closures).
- **Conditional compilation and atomics**: `--cfg NAME` with
  `#[cfg(NAME)]`, pointer atomics
  ([§1.4](docs/language.md#14-compile-time-evaluation),
  [§1.3](docs/language.md#13-memory)).
- Generic instances over same-named types from different modules get
  distinct names; enum-to-int casts in constants.

### GC

- **No sweep** ([stdlib/gc.md](docs/stdlib/gc.md#heap-layout)): marking
  sets a bit per 16-byte granule in each chunk's mark bitmap, and each
  size class bump-allocates through the runs of clear bits (found
  word-at-a-time when a run is used up); dead objects are never read,
  and chunks with nothing marked go to a pool any class takes from.
  Both modes flip the mark epoch at each major collection. Geometric
  means of new/old time: Prolog 0.885 (nreverse, qsort 0.80), Scheme
  0.955 (diviter 0.77, deriv 0.83), Python 0.94, Mini 0.98
  (binary_trees 0.82), MiniJS 0.975 (closures 0.83), at the same peak
  memory
  ([results.md](docs/results.md#gc-mark-bitmaps-instead-of-sweeping-2026-11-04)).
  Verify mode fills free memory with 0x01 bytes so that dangling
  references are still reported; `--gc-check-roots` keeps a start bitmap.
  Fixed after review: `gc.mark_shadow_stack` (std.gil's scan of the
  other threads' frames) marked another heap's objects in a major
  collection, through the new prefetch ring, which bypassed its
  this-heap-only check (`tests/run/gc_shadow_stack_other_heap.lm`);
  `--gc-check-roots` did not report a root to an object a generational
  heap had freed (the start bitmap was pruned only in non-generational
  heaps; `tests/run/gc_check_roots_gen_freed.lm`), and its sweeper,
  which now fills dead objects' headers too, ran before sweepers added
  later, which then saw dead objects as marked
  (`tests/run/gc_sweeper_after_check_roots.lm`);
  `tests/run/gc_torture.lm` is a random mutator over every size class,
  weak references, ephemerons and free hooks in six heap configurations;
  the allocator's invariants are listed in
  [stdlib/gc.md](docs/stdlib/gc.md#invariants-of-the-bitmap-allocator).

- **GC objects are checked in every module**
  ([design/safety.md §8](docs/design/safety.md),
  [language.md §8](docs/language.md#8-gc-integration)): a slice of an
  object's traced trailing array passed to a function that may store
  through it gets the write barrier after the call (it got none, and
  generational heaps could free a young object it stored; std.dict's
  hand-written barrier is gone); keeping such a slice (`var`, return,
  field) or storing through a `let` that holds one is an error, as is
  passing it to a function that may collect and store. Trailing arrays
  start zero-filled only where zero is a valid element (no null `Gc`, no
  word that is no variant of a `value` type); otherwise `new(h, X { .. },
  n, v)` fills them, or `unsafe { new(..) }`. The objects of a
  `#![safe]` module are built and written only by that module (Python's
  one write to a dict field is `KeyDict.use_up_room()` now), and in a
  `#![safe]` module another module's objects are treated like structs
  with raw pointers.
- **Frameless allocation and barriers in handlers**: `try_new(h, T { .. })
  -> ?Gc<T>` (no refill call) with `gc.make_room` in a cold block that
  `redo`es; the write barrier's slow path makes no call (a full
  remembered set makes the next collection major); stores of values
  without references (`Term.Unb`, `None`) get no barrier. Framed
  handlers in the example interpreters 223 -> 200
  ([language.md §4](docs/language.md#4-slow-paths-cold-blocks),
  [stdlib/gc.md](docs/stdlib/gc.md), [lowering.md §3, §10](docs/lowering.md)).
- **Unzeroed trailing arrays**: `new_uninit(h, T { .. }, n)` allocates
  without zeroing the trailing array (no `memset`; large objects from
  `malloc`), for element types the collector does not trace and of which
  every bit pattern is valid (any untraced type inside `unsafe`); verify
  mode and `--gc-check-roots` fill it with 0x01 bytes. Scheme's strings
  and symbols, MiniJS's and Python's strings use it where the bytes are
  copied in at once: Scheme `string` 0.019 -> 0.015 s
  ([language.md §8](docs/language.md#8-gc-integration),
  [stdlib/gc.md](docs/stdlib/gc.md)).

- **`heap.on_free_gc(obj, f)`**: a free hook typed `fn(Gc<T>)`, no
  `unsafe` cast from `*u8` ([stdlib/gc.md](docs/stdlib/gc.md)).
- **Generational mode** with epoch marks, lazy sweeping and large-object
  aging; Scheme runs on it
  ([stdlib/gc.md](docs/stdlib/gc.md#generational-mode-sticky-mark-bits-non-moving)).
- **Automatic rooting of traced vm state** at calls that may collect
  (the `state_root` lint is retired) ([language.md §8](docs/language.md#8-gc-integration)).
- **Trace hooks** (`#[trace(f)]`), **free hooks** (`heap.on_free`) and
  **dirty pinning** (`heap.pin_dirty`)
  ([stdlib/gc.md](docs/stdlib/gc.md#trace-hooks-free-hooks-and-dirty-pinning)).
- **`--gc-check-roots`**, a collector that checks every reference it
  marks, and a GC rooting fuzzer (`tests/fuzz/rootgen.rs`), which found a
  `match`-guard rooting bug
  ([stdlib/gc.md](docs/stdlib/gc.md#checking-roots---gc-check-roots),
  [status.md](docs/status.md#gc-rooting-fuzzing)).
- A non-generational collection no longer sweeps chunks that were empty
  after the last sweep and not allocated from since: the sweep followed
  the heap's high-water mark (Prolog's boyer 0.91, poly_10 0.85 of the
  time).

### Embedding and threads

- **Threads**: runtime state in thread-local storage, so independent
  states run on separate threads; `--sanitize=thread`
  ([language.md §10.1](docs/language.md#101-embedding-in-a-c-host-implemented)).
- **Threads sharing one heap** behind a lock: `std.gil`, Mini built with
  `--cfg gil` ([stdlib/gc.md](docs/stdlib/gc.md#threads-sharing-a-heap-stdgil)).
- **AArch64 Linux** cross builds: `--target=aarch64-linux-gnu
  --sysroot=DIR`, `LOIMI_RUNNER`, validated under qemu
  ([README](README.md#getting-started), [status.md](docs/status.md), Targets row).

### Standard library

- `std.hash.bytes` hashes eight bytes per multiply (was byte-at-a-time
  FNV-1a), the length in the seed: Mini's string_build 0.92 of the time
  before (Mini interns every string it makes); MiniJS and Python within
  ±2% (2026-11-04).
- **Memory-safe std modules**: `std.lex`, `std.parse`, `std.diag`,
  `std.text`, `std.hash`, `std.io`, `std.bc`, `std.map` and `std.dict` are
  `#![safe]`, with justified `unsafe` blocks around C calls and the
  hash tables' masked indexing. `OrderedMap.entry`/`set_value`/`remove_at`
  panic on an index that is not a live entry (they read out of bounds
  before), and `HashMap.free`/`OrderedMap.free` leave the map usable
  (a later call used freed memory); the shared probe core is `#[raw]`. `mem.Vec` gained
  `extend(s)` (safe when `s` is the vector's own items: fixes a
  use-after-free in `StrBuf.push_str(b.as_str())`) and `filled(n, x)`;
  `io.free_file` is `#[raw]` ([design/safety.md](docs/design/safety.md)).
  `std.obj`'s inline-cache API that trusts cache words without checking
  them (`Model.load_ref`, `hit`, `poly_get`, `MegaCache.put`, ..., 15
  functions) is `#[raw]` too, as are `gc.barrier` and `gc.write_barrier`
  (no IR change). `std.dict` probes its GC tables through slices (masked
  by the index length, so no slot bounds check) and does not trust its
  fields; `Dict.remove_at`/`KeyDict.remove_at` panic on a dead entry
  (they looped forever). Python benchmarks: geomean within ±0.5%,
  `richards` consistently about 1% slower.
- **`std.bigint`**: arbitrary-precision integers with Python's semantics
  ([stdlib.md](docs/stdlib.md#stdbigint)). Schoolbook division by 3/2
  quotient limbs (GMP's `sbpi1_div_qr`), reciprocals by table and Newton
  steps instead of `div`, `isqrt` without the top-level remainder:
  quotients 1.1-1.3x and square roots 1.4-1.6x faster at 4-100 limbs.
- **`std.map.OrderedMap`, `std.dict.Dict`, `std.dict.KeyDict`**:
  insertion-ordered maps in CPython's compact layout
  ([stdlib.md](docs/stdlib.md#ordered-maps-maporderedmap-and-dictdict)).
- **`mem.try_sort_by`**: a stable sort with a fallible, re-entrant
  comparator ([stdlib.md](docs/stdlib.md#stdmem)).
- **`std.io.Out`**: buffered output (`print`, `byte`, `print_int`,
  .., flushed when full and by `flush`)
  ([stdlib.md](docs/stdlib.md#stdio-and-stdos)).
- **`std.parse.Ops`**: operator tables changed at run time (Prolog's
  `op/3`: names, priorities, the seven types, prefix/infix/postfix
  definitions of one name) and an operator-precedence driver with its
  hooks as trait methods (`OpHooks`); the Prolog reader uses it
  ([stdlib.md](docs/stdlib.md#run-time-operator-tables-parseops)).

### Tooling

- **`--vm-stats`** counts entries into each `cold` block, per op, with
  its source location ([status.md](docs/status.md), Instrumentation row).
- **Doc-snippet test**: every `loimi` code block in the README and
  `docs/` compiles, runs or fails as its annotation says
  (`tests/docs.rs`); the tutorial is checked against
  `examples/tutorial` (`tutorial_steps`).
- **`bench/run_all.py`** runs every example against its references in
  every dispatch mode; the results are in [`docs/results.md`](docs/results.md).
- `loimi --help` prints the usage.
- **`--cpu=NAME`** (`x86-64-v2`, `x86-64-v3`, `native`, ..; `LOIMI_CPU`)
  compiles for a newer CPU: clang's `-march`/`-mcpu`, plus `pshufb` for
  `swizzle` and `roundps` for vector rounding; the default stays the
  portable baseline ([lowering.md §12](docs/lowering.md#12-driver-settings)).

### Examples

- No `unsafe` left in the example interpreters: `f64.from_bits(a as
  u64)` on a register operand compiles to a `double` load of the slot
  ([language.md](docs/language.md), [lowering.md §2](docs/lowering.md)),
  and a checked register window was measured and declined
  ([roadmap question 7](docs/roadmap.md)). (Since the zero-fill rule,
  five `unsafe { new(..) }` allocate `Gc` arrays that are filled right
  after: [design/safety.md §8](docs/design/safety.md).)
- [`examples/wasm`](examples/wasm): a WebAssembly interpreter that
  passes every non-SIMD spec test file.
- The wasm interpreter takes 0.976 of its previous time (0.87 of wasm3's,
  was 0.90 in the same run): calls zero only the locals a read can see
  unwritten (a forward pass over each body), `p + k` addresses fold into
  loads and stores at offset 0, bit tests and loads fuse with the branch
  after them, direct calls of the module's own functions skip the host
  test ([`FRICTION.md`](examples/wasm/FRICTION.md), "Where the time goes
  against wasm3", which also has a per-benchmark profile and what did
  not help).
- A held-out wasm benchmark set, the 30 PolyBench/C kernels
  (`bench/wasm/polybench.sh`, with an array check against wasm3): the
  round above took the interpreter from 0.998 to 0.919 of wasm3's time
  there. A second round (min/max selects as one op, constant slots for
  operands of ops without a K form, `local.tee`'d address plus load as
  one op, one MOV less for a tee over a pending read): 0.893 of wasm3 on
  PolyBench, 0.872 on the original seven
  ([`FRICTION.md`](examples/wasm/FRICTION.md), "A held-out set").
- The Scheme interpreter takes 0.894 of its previous time over the 43
  r7rs-benchmarks programs (0.40 of Guile's VM, was 0.45; 0.22 of
  Gauche, was 0.25): `list`, `vector`, record constructors, `memq` and
  `assq` as ops, allocation without a stack frame, no register clearing
  on calls (a stack high-water mark keeps stale slots safe for the
  collector), aliases of builtins and known procedures, free variables
  hoisted out of loops, constant-index vector ops; interpreter changes
  only ([bench/scheme/README.md](bench/scheme/README.md#third-phase-calls-allocation-builtins-2026-11-04)).
- The Scheme interpreter takes 0.941 of its previous time over the 43
  r7rs-benchmarks programs (0.37 of Guile's VM, was 0.40; 0.21 of
  Gauche, was 0.22): calls of known top-level procedures take their
  closure from the constant pool (`CALLK`), record accessors, `call/cc`,
  `reverse`, `append`, `length`, `odd?` and other builtins as ops,
  `symbol->string` cached, unzeroed strings (`new_uninit`), and the
  faster `std.bigint` division and square roots
  ([bench/scheme/README.md](bench/scheme/README.md#fourth-phase-known-calls-more-builtins-as-ops-strings-bigints-2026-11-04-later)).
- [`examples/python`](examples/python): a Python 3 subset running 11
  pyperformance programs, on `std.bigint` and `std.dict`.
- The Python subset takes 0.45 of CPython 3.13's time by geometric
  mean over its 11 pyperformance programs (was 0.52): `for a, b in ..`
  as one op (enumerate/zip without tuples), a global method cache
  behind the per-site caches, operator methods and `__eq__` presence
  cached per class, `list.pop/insert/append` and one-argument `math`
  functions without the call machinery, `reversed(range)` as a range
  iterator, int/str filtering in `x in list`, division-free unit-step
  slices; interpreter changes only
  ([FRICTION.md](examples/python/FRICTION.md#optimizations-second-round-2026-11-04)).
- The Python subset's instances are one object with inline attribute
  slots behind shapes (transition trees per class and slot count, slot
  counts from a compiler hint), with a dict only for `__dict__`,
  deletion and many attributes; attribute caches check the name at the
  cached slot, so subclass-polymorphic sites hit with one entry, and
  LOADMETHOD caches the method itself (four entries). 0.39 of CPython
  3.13's time by geometric mean (was 0.45): float 0.66, richards 0.68,
  deltablue 0.71, raytrace 0.75 of the time before; float's peak RSS
  105 → 36 MB
  ([FRICTION.md](examples/python/FRICTION.md#instance-layout-inline-slots-behind-shapes-2026-11-04)).
- The Python subset takes 0.33 of CPython 3.13's time by geometric
  mean (was 0.38 in the same run; 0.88 of its previous time): class
  instantiation and default/keyword arguments without `call_slow`'s
  loop (CALL_NEW, CALL_M1 family members), `tuple`/`set`/`sum`.. of a
  generator expression run inline while the name is the builtin,
  `super(..).m(..)` as one cached op, list slices with raw copies,
  per-class `__hash__`/`__bool__` flags; nqueens 0.62, hexiom 0.78,
  fannkuch 0.81; interpreter changes only
  ([FRICTION.md](examples/python/FRICTION.md#optimizations-third-round-calls-generator-expressions-slices-2026-11-04)).
- MiniJS takes 0.48 of node --jitless's time and 0.37 of QuickJS's
  (0.939 of the time before): Int overflow and string concatenation
  without cold calls, a DIV family, compare-and-branch with pool
  constants (`JLTK`..) and `EQK`/`NEK`, handlers without stack frames
  (`try_new` in NEW, slow paths in cold blocks; std.obj's Poly hits
  without bounds checks, which also removed frames in Lox), and calls
  that no longer clear the callee's registers (the collector clears the
  stack above the innermost window up to a high-water mark)
  ([bench/minijs/RESULTS.md](bench/minijs/RESULTS.md#second-performance-pass-2026-11-04)).
- Mini takes 0.49 of PUC-Lua's time and 0.77 of LuaJIT -joff's by
  geometric mean (0.927 of the time before; string_build 0.77, sieve
  0.83, oo_methods 0.86), less than -joff on all 14 programs, and 0.97
  of -joff's on the coroutine suite (2026-11-04, third pass):
  `%` by multiplication with the magic number in an inline cache,
  while loops that test again at the bottom and threaded jumps, appends
  in place, table and return handlers without stack frames (their rare
  calls in cold blocks), string natives on one kept buffer, a minor
  collection sweeping only the young strings of the intern table,
  math.sqrt/floor without the native call, constants in multiple
  assignment stored directly, four supers
  ([bench/mini/RESULTS.md](bench/mini/RESULTS.md#third-phase-2026-11-04)).
- Mini takes 0.47 of PUC-Lua's time and 0.75 of LuaJIT -joff's
  (0.984 of the time before; interp 0.82) (2026-11-05, fourth pass):
  never-assigned constant locals read as constants in nested functions
  (a first compiler pass finds the assigned ones), and if/elseif chains
  comparing a local with integer constants compile to a jump table
  (`SWITCH`). Lazy register clearing with a high-water mark was measured
  again and dropped (fib 0.955, oo_methods 1.05)
  ([bench/mini/RESULTS.md](bench/mini/RESULTS.md#fourth-phase-2026-11-05)).
- Mini has Lua 5.4 coroutines that switch threads inside the vm run
  ([`examples/mini/COROUTINES.md`](examples/mini/COROUTINES.md)).
- The Prolog takes 0.53x SWI-Prolog's time and 0.13x Trealla's (0.92
  of the time before; tak 0.55, boyer 0.53): a first clause whose first
  comparison excludes the other clauses' commits without a choicepoint;
  arg/3, functor/3 and compare/3 write a new output variable's register
  instead of binding a variable object; 32-bit division; comparisons
  of head arguments before the environment; a 2 MiB minimum heap
  ([FRICTION.md](examples/prolog/FRICTION.md#optimizations-third-round-2026-11-04)).
- The Prolog took 0.63x SWI-Prolog's time and 0.15x Trealla's by
  geometric mean over its 30 benchmarks (was 0.79x and 0.19x):
  dereferenced arithmetic operands, inline structure allocation,
  trailing and failure in the head ops, nested head structures built in
  place, calls that index the first argument themselves,
  choicepoint-free guard chains, register allocation across guards,
  supers
  ([FRICTION.md](examples/prolog/FRICTION.md#optimizations-second-round-2026-11-04)).
