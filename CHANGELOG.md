# What's new

Features by topic, with links to where they are documented.
[`docs/status.md`](docs/status.md) has the complete state of the
implementation; `git log` has the details.

## Since 2026-11-02

### `vm` declarations and dispatch

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

- **Frameless allocation and barriers in handlers**: `try_new(h, T { .. })
  -> ?Gc<T>` (no refill call) with `gc.make_room` in a cold block that
  `redo`es; the write barrier's slow path makes no call (a full
  remembered set makes the next collection major); stores of values
  without references (`Term.Unb`, `None`) get no barrier. Framed
  handlers in the example interpreters 223 -> 200
  ([language.md §4](docs/language.md#4-slow-paths-cold-blocks),
  [stdlib/gc.md](docs/stdlib/gc.md), [lowering.md §3, §10](docs/lowering.md)).

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

- **`std.bigint`**: arbitrary-precision integers with Python's semantics
  ([stdlib.md](docs/stdlib.md#stdbigint)).
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
  ([roadmap question 7](docs/roadmap.md)).
- [`examples/wasm`](examples/wasm): a WebAssembly interpreter that
  passes every non-SIMD spec test file.
- [`examples/python`](examples/python): a Python 3 subset running 11
  pyperformance programs, on `std.bigint` and `std.dict`.
- Mini has Lua 5.4 coroutines that switch threads inside the vm run
  ([`examples/mini/COROUTINES.md`](examples/mini/COROUTINES.md)).
