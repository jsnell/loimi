# What's new

What changed in Loimi recently, grouped by topic, for users coming back
after a while. [`docs/status.md`](docs/status.md) has the full state of
the implementation, and `git log` has the details.

## Since 2026-11-02

### Build time

A large interpreter builds several times faster. A Mini build took
28 s on 2026-11-08; it now takes 4 s, or 6.5 s with `--release`.

- **Large modules compile in parallel.** The generated code is split
  into up to one part per core. Run time is unchanged. Mini: 10 s to
  4 s in development builds, 22 s to 6.5 s with `--release`.
  `LOIMI_SPLIT=N` sets the number of parts (`1` compiles in one piece),
  and `LOIMI_SPLIT_REPORT=1` times the parts.
- **Development builds no longer pad branches on x86-64.** The padding
  took more than half of a large vm's build time (Mini: 20 s of 28).
  `--release` builds still pad.
- **Mini dropped its value accumulator.** It made Mini's generated code
  27% larger and Mini's benchmarks at most 1% faster.

### Removed

- **The ahead-of-time trace tier** (`--trace-regions`, `--traces`,
  `LOIMI_TRACE_*`). It tuned the interpreter at build time to the
  programs it was trained on, and testing it took four builds per test.
  The run-time JIT is unchanged.

### Testing

- **Three test tiers.** The table below lists them; the README has
  details. Test builds skip branch padding (`LOIMI_BRANCH_ALIGN=0`, the
  same as `--no-branch-align`).

  | Tier | Command | Time |
  |---|---|---|
  | Fast | `LOIMI_TEST_FAST=1 cargo test --release` | about 2 minutes |
  | Default | `cargo test --release` | about 5 minutes |
  | Heavy | `cargo test --release -- --include-ignored` | longer |

- **Doc snippets are tested.** Every `loimi` code block in the README
  and `docs/` compiles, runs or fails as its marker says
  (`tests/docs.rs`).

### The `vm` declaration

All of these are in [language.md](docs/language.md).

- **`const` inline cache fields**, set by the emitter and only read by
  handlers. Their `verify` clauses run once, in `Vm.verify`, not on
  every use.
- **Warning `state_registers`**: a tail-call vm whose state needs more
  argument registers than the target has, so every handler would pass
  some on the stack.
- **Named operand fields**: `op DELGLOBAL(name: kst @ bx) : ABx` puts an
  operand in a named format field, so ops no longer need dummy operands.
- **`v.index_of(at)`** gives the word index of a pointer into verified
  code, for line tables and exception ranges.
- **`become match st { .. }`** picks a family member from an enum value.
- **Re-fusion after `become`**: `refuse code;` lets super instructions
  form over quickened ops, which no single `become` could do.
- **`supers from "vm.vmprofile";`** adds the super instructions a
  `--vm-stats` report suggests.
- **Value accumulators**: `accumulator vacc: f64 for reg;` keeps a
  NaN-boxed value's word in a floating-point register between ops.
  MiniJS's nbody takes 0.93–0.97 of the time with it.
- **`rel` and `data` on ops**: `rel kform(I32_ADD) = I32_ADDK;` and
  `data cost(MUL) = 3;` generate lookups by opcode, replacing
  hand-written tables of related ops.
- **Op selectors and `supers { .. }` groups**: `A..B` ranges, globs
  such as `BR_I32_*`, and `with KIND` in `accumulate` and `variant`
  lists.
- **Error paths are cold by default.** A `raise`, `fail` or `redo`
  reached on some paths only compiles as if inside `cold { }`. `#[hot]`
  opts out; `LOIMI_COLD_AUDIT=1` lists what was made cold.
- **Cache layouts from structs**: `cache PropIC = obj.Site;`, so `&cache`
  can be passed to plain functions as a `*obj.Site`.
- **Declared polls**: `poll gc { .. }` and `#[poll(gc)]` put a check at
  the start of an op.
- **`redo` is checked**: nothing before it on the path may store to
  memory (`error[redo_effect]`).
- **`new` in a handler** compiles to a form that keeps the slow refill
  call off the fast path.
- **`Vm.Resume` cache fields**: verified code addresses stored in the
  instruction stream, such as try chains and cached callees.
- **`#[inline(never)]` on a `-> never` vm function** now keeps it out
  of line.
- **Accumulators**: `accumulator acc: T for kinds;` keeps a temporary
  in a machine register between adjacent ops.
- **`next_pc` and `then_next_pc`**, for return addresses in vm
  functions.
- **Resume points** (`Vm.Resume`, `v.resume_at(i)`), so entries that
  require verified code can resume at a saved pc without `unsafe`.
- **Per-format setters and encoders**: `Vm.set_ABC_a(w, v)` and
  `Vm.encode.ABC(op, a, b, c)`.
- **Errors in `verify` clauses and other copied vm text** point at your
  source.

### Dispatch and generated code

- **Handlers without stack frames.** Tail-call handlers are compiled
  without the LLVM passes that tend to add frames.
  `-W handler_frame` reports handlers whose fast path still sets one up,
  and names the cause, such as "the overflow check of `/` at
  vm.lm:512".
- **Branch alignment on x86-64** in `--release` builds: no branch
  crosses or ends at a 32-byte boundary. One badly placed wasm handler
  had made three benchmarks take 1.4 times as long.
  `--no-branch-align` turns it off.
- **goto mode** builds in time nearly linear in the number of ops.
- **switch mode** builds with LLVM's tail merging off.
- **AArch64**: the dispatch table's address stays in a register.
- **The generated verifier** no longer zeroes 64 KiB per call. Prolog's
  `assertz`, which verifies each new clause, spent half its time there.
- **Measured and not built**: extra load facts for LLVM (no hot handler
  changed), and a coroutine-switch primitive (it could save only two
  loads per switch).

### JIT (`--jit-stencils`)

- **Handlers may read other instructions' words**, and jumps to a
  computed pc enter compiled code.
- **Jumps to a `Vm.Resume` count toward compiling**, and the code-object
  registry is a tree instead of a sorted array. Prolog's JIT build went
  from 1.04 to 0.98 of the interpreter's time.
- **`--jit-stencils` works with `--sanitize=address` and
  `--gc-check-roots`.**

### Debugging guest programs

A prototype; [design/debugging.md](docs/design/debugging.md) describes
it.

- **`on break { .. }`** generates breakpoint ops and `Vm.set_break`.
  `std.debug` has stepping and a command reader.
- **Mini** built with `--cfg debug` is a gdb-like debugger
  (`mini --debug script`). `next` now steps over `coroutine.resume`.
- **Python** built with `--cfg debug` is a pdb-like debugger
  (`python-loimi --debug prog.py`).
- **Debugging Mini from an editor**: a C API for the debugger, and
  `mini-dap`, a Debug Adapter Protocol server for VS Code or any DAP
  client.

### Values

- **`repr nanbox(int_high)`**, an opt-in layout that makes integer
  arithmetic cheaper and floats slightly dearer. Python's suite takes
  0.93 of the time on it, and Prolog's 0.92. Scheme, MiniJS, Mini and
  Lox tried it and kept the plain layout: their float code got slower.
- **`V.Num.xor`**, for float fast paths under `nanbox(int_high)`.
- **`#[scan_raw_words]`** on a value type guarantees that small words
  (raw pointers, small integers) read as variants without references.
- **Up to 56 `Gc` variants** in a nanbox: when the 48-bit blocks run
  out, up to 8 variants share one.
- **Layout visibility**: `loimi build --explain-values`, `block N` pins
  and the `value_layout` warning.
- **Division checks on `i48`** fold away after a `y > 0` guard.

### Core language

[language.md](docs/language.md) documents all of these.

- **`const` arrays indexed at run time are read in place.** They used to
  be copied to the stack on every access (1.2 µs a call before, 10 ns
  now, in one case).
- **Overlapping impls are an error**, instead of the first one silently
  winning.
- **`e is P`** tests a pattern without binding anything, and
  **`e as! V`** unwraps a variant or traps.
- **Names and data on enum variants**: `#[name("car")]`, `#[names]` and
  `#[data(..)]` generate `E.name(k)`, `E.from_name(s)` and accessors.
- **128-bit vectors**, `i8x16` to `f64x2`, with lane-wise operators,
  shuffles and saturating arithmetic. `lane_at` and `with_lane_at`
  choose a lane at run time.
- **Literals take the other operand's type**: `16 + 8 + n` with
  `n: u64`. So do an `if`, `match` or block whose results are all
  literals.
- **`let t: [T; _] = [..]`** takes the array literal's length.
- **Raw string literals**: `r"..."` and `r#"..."#`.
- **`#(a ++ "_X")`** in `inline for` builds one identifier from parts.
- **Constants in patterns**, or-patterns with bindings, and the
  `unreachable_arm` warning.
- **`for x in`** over arrays and slices.
- **Float built-ins** (`ceil`, `trunc`, `copysign`, `min`, `max` and
  more), **wide multiply and divide** on `u64`, and **carry chains**
  (`add_carry`, `sub_borrow`).
- **`p.read_unaligned()` and `p.write_unaligned(v)`.**
- **`..zero`** in struct literals also nulls raw pointer fields.
- **Closures**: escaping closures with state, cells shared between
  closures, and local closures (`let f = |x| ..;`).
- **`--cfg NAME`** with `#[cfg(NAME)]`, and pointer atomics.

### Memory safety

[design/safety.md](docs/design/safety.md) and
[design/lifetimes.md](docs/design/lifetimes.md) explain the rules.

- **Pointer stability is checked in every module by default.** A pointer
  into a container, used after a call that may move or free it, is a
  compile error. So is the address of a local that outlives its
  function. `#![lifetimes(off)]` opts a module out. The check found
  real bugs in MiniJS, Mini and the Stack fuzzer.
- **Pointers into growable memory** may not be stored where they outlive
  the function. `#[shrinks(v)]` marks functions such as `Vec.pop`.
- **`#![safe]` modules** need `unsafe { }` for pointer arithmetic,
  pointer casts and calls of `extern` functions.
- **GC objects are checked in every module.** A slice of an object's
  traced array gets the write barrier it needs, and trailing arrays
  start zero-filled only where zero is a valid element.
- **`--sanitize=address` catches stale pointers into `Vec` and
  `Arena`**, not just freed memory.
- **No `unsafe` left in the example interpreters**, apart from five
  allocations of `Gc` arrays that are filled right after.

### GC

[stdlib/gc.md](docs/stdlib/gc.md) documents all of these.

- **`#[trace] struct`** generates the code that marks a context
  struct's GC references. Scanners and sweepers are now typed.
- **No sweep.** Marking sets bits in a bitmap, and allocation reuses the
  clear runs directly. Prolog takes 0.885 of the time, Scheme 0.955,
  Python 0.94, at the same peak memory.
- **Allocation and write barriers in handlers without a stack frame**:
  `try_new` and `gc.make_room`.
- **`new_uninit`** allocates a trailing array without zeroing it.
- **`heap.on_free_gc(obj, f)`**, a typed free hook.
- **Generational mode**; Scheme runs on it.
- **Traced vm state is rooted automatically** at calls that may
  collect.
- **Trace hooks, free hooks and dirty pinning.**
- **`--gc-check-roots`** checks every reference the collector marks, and
  a GC rooting fuzzer runs in the tests.

### Embedding and threads

- **Threads**: independent states run on separate threads.
  `--sanitize=thread` works.
- **Threads sharing one heap** behind a lock: `std.gil`.
- **AArch64 Linux cross builds**: `--target=aarch64-linux-gnu
  --sysroot=DIR`, tested under qemu.

### Standard library

[stdlib.md](docs/stdlib.md) documents all of these.

- **Fast shortest float formatting** (Schubfach): about 80 ns per
  number instead of 10 µs, with the same output. Loops that convert
  floats to strings run 55 to 80 times faster.
- **`std.rt.ValueStack<T>`**: a value stack with a high-water mark, so
  calls don't clear registers for the collector. Lox's `fib` takes 0.86
  of the time.
- **`std.math.Divisor`**: division by a prepared constant, usable as an
  inline cache layout.
- **`std.hash.bytes`** hashes eight bytes per multiply.
- **Most std modules are `#![safe]`**, and several use-after-free and
  out-of-bounds bugs in `HashMap`, `OrderedMap`, `StrBuf` and `Dict`
  were fixed.
- **`std.bigint`**: arbitrary-precision integers with Python's
  semantics.
- **`std.map.OrderedMap`, `std.dict.Dict` and `std.dict.KeyDict`**:
  insertion-ordered maps in CPython's layout.
- **`mem.try_sort_by`**: a stable sort whose comparator may fail.
- **`std.io.Out`**: buffered output.
- **`std.parse.Ops`**: operator tables that change at run time, as
  Prolog's `op/3` needs.

### Tools

- **`loimi build --vm-prof`**: a sampling profiler built into the
  program. At exit it prints time per op handler, cold block and
  function. It costs about 3% in tail-call mode. It no longer crashes
  on AArch64.
- **`--vm-stats`** counts quickenings and entries into each `cold`
  block.
- **`loimi ir-diff A B`** compares the optimized code of two builds
  function by function, to show that a refactor costs nothing. It also
  works on `.ll` files from any compiler.
- **A build reports every error, not just the first** (up to
  `--max-errors N`, default 20).
- **`--cpu=NAME`** compiles for a newer CPU, such as `x86-64-v3` or
  `native`.
- **`bench/run_all.py`** runs every example against its references;
  the results are in [docs/results.md](docs/results.md).
- **`loimi --help`** prints the usage.

### Example interpreters

New:

- **[`examples/wasm`](examples/wasm)**: a WebAssembly interpreter that
  passes every spec test file.
- **[`examples/python`](examples/python)**: a Python 3 subset that runs
  11 pyperformance programs.
- **Mini** has Lua 5.4 coroutines.

Faster. Each figure is our interpreter's time divided by the reference's,
as a geometric mean over the example's benchmarks, from the 2026-11-07 run
in [docs/results.md](docs/results.md); *Before* is the first measurement.

| Example | Now | Before | Report |
|---|---|---|---|
| Python | 0.31× CPython 3.13 | 0.52× | [FRICTION.md](examples/python/FRICTION.md) |
| Prolog | 0.47× SWI-Prolog, 0.11× Trealla | 0.79×, 0.19× | [FRICTION.md](examples/prolog/FRICTION.md) |
| Scheme | 0.38× Guile's VM, 0.22× Gauche | 0.45×, 0.25× | [bench/scheme](bench/scheme/README.md) |
| Mini | 0.49× PUC-Lua, 0.75× LuaJIT `-joff` | | [bench/mini](bench/mini/RESULTS.md) |
| MiniJS | 0.48× node `--jitless`, 0.37× QuickJS | 0.939 of its own previous time | [bench/minijs](bench/minijs/RESULTS.md) |
| Wasm | 0.85× wasm3 on its own programs, 0.82× on PolyBench/C | 0.90× and 0.998× | [FRICTION.md](examples/wasm/FRICTION.md) |
