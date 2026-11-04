# Implementation status

What exists today, and where the implementation differs from the design
in [`language.md`](language.md). Updated 2026-11-04. What changed
recently, by topic: [`CHANGELOG.md`](../CHANGELOG.md).

## Built

| Area | Status |
|---|---|
| Compiler | Rust, no dependencies. Emits textual LLVM IR and drives `clang-19` (`loimi build`, `loimi run`, `--emit-llvm`, `--release`, `--dispatch=`, `--sanitize=address`, `--sanitize=thread`, `--align-functions=N` to start every function on an N-byte boundary so that unrelated code changes do not move vm handlers, which makes A/B benchmarks steadier, `--keep-ll` to keep the IR next to the output). The standard library is found through `LOIMI_STD` (else a `std/` directory next to the executable or up to three directories above it, else in the current directory); `LOIMI_CLANG` and `LOIMI_AR` override `clang-19` and `llvm-ar-19`, and `LOIMI_ASAN_RUNTIME`/`LOIMI_TSAN_RUNTIME` name a sanitizer runtime to link when clang has none; `LOIMI_DUMP_VM_EXPANSION=1` prints the Loimi source generated for each `vm` (its expansion) to stderr. Errors (and panic locations) in user text copied into the expansion (`verify`/`const` clauses, `verify_context` types, `verify` blocks, stack effect counts, `stack_entry`) point at the vm declaration, with a note naming the generated context; an error in the rest of the expansion says `internal: generated code`. Named warnings (`match_fallout`, `super_overlap`, `unreachable_arm`, `value_layout`, `next_pc_then`; the retired `state_root` is still accepted by `#[allow]`; `hot_gc`, a call that may collect on a handler's hot path, is opt-in with `-W hot_gc`; so is `handler_frame`, `-W handler_frame`, tail-call handlers whose fast path sets up a stack frame, found in the assembly with the calls that cause it), silenced with `#[allow(name)]` on an item, vm item or match arm or `#![allow(name)]` per file; `-W error` / `--deny-warnings` makes them errors. |
| Libraries and embedding | `loimi build --lib` (static archive of `-fPIC` code) and `--shared` (shared object), with no `main`; `export "C"` functions are the API. `--header out.h` writes a C header for them (integers, floats, `bool`, pointers, opaque struct pointers, nullable `?*T`, C function pointers, `str` as `loimi_str { const char *ptr; size_t len; }`, payload-less enums as a typedef plus `E_Variant` constants; other types are rejected with the reason). No init function. `--tls=` overrides the model of the runtime's thread-locals (see Threads). `std.rt.protect`/`throw` recover from errors deep in code without an error path, and `#[catch_panic(v)]` on an export (or `rt.catch_panic`) turns panics into an error return instead of aborting the host (an `rt.panic` message is copied, up to 512 bytes). The header also carries `export "C" const`s, `///` comments (on functions, constants, structs and enums), `#[c_const]` parameters and `#[c_name]`/`--header-prefix` names. `--sanitize=address` and `--vm-stats` work for libraries (several `--vm-stats` libraries link into one host). `examples/mini/embed.lm` makes Mini a library (`mini_new`, `mini_dostring`, `mini_error`, globals, C natives, a rooted value stack, `mini_call`); `examples/embed/host.c` uses it. C natives may call back into Mini (`mini_call`, `mini_dostring`), as nested runs. An internal error (a panic) returns `MiniStatus_Internal` and poisons that state only. See `language.md` §10.1. |
| Threads | Independent states on different threads run concurrently with no locks: the runtime's per-thread state (shadow-stack top, `Vm.depth`, vm scratch slots, `std.rt` handler chain and panic message, `#[track_caller]` location) is initial-exec TLS (local-dynamic in `--shared` builds), and `std.gc` keeps everything in its `Heap` (no globals). A state may move between threads with a synchronized hand-over. TLS instead of plain globals costs nothing measurable (geometric means 0.99–1.01 on Mini, MiniJS, Scheme; `lowering.md` §4). Several states on one thread may nest (a native of B runs A): the collector's shadow-stack walk marks only its own heap's objects (fixed 2026-11-03; it used to mark the other heap's objects, which that heap's next collection then took as marked, freeing what they referenced). `--sanitize=thread` builds with ThreadSanitizer (host links GCC's `libtsan`). `examples/embed/threads.c`: four Mini states on four threads, GC stress, nested runs, caught panics on every thread, states handed between threads; clean under TSan. Not thread-safe: plain module-level `var`s, `--vm-stats` counters, the `--jit-stencils` JIT (executables). **Several threads sharing one heap and state, one at a time** (opt-in): `std.gil`, a lock per heap with first-come-first-served hand-over, attach/detach, `unlock`/`lock` around blocking code, a poll (`waiting()`, a relaxed atomic load) and `yield_now()`, hooks that move the interpreter's per-thread state in and out of its shared state, and a scanner that marks the other threads' saved shadow stacks and states. Mini built with `--cfg gil` (`examples/mini/threads.lm`): each OS thread runs its own Mini thread (stacks, main coroutine, API value stack) over shared globals and code, polls at every taken jump and Lua call, `mini_thread_attach`/`detach`, `mini_unlock`/`lock`; `examples/embed/shared.c` (a hand-over at a back-edge of a loop that only another thread can end; four threads on shared tables under GC stress, natives releasing the lock two runs deep) is clean under TSan. Cost single-threaded: 1.2–1.4% geomean on Mini (worst 4–5%); default builds unchanged (IR byte-identical). Not built: free threading (roadmap Q4). See `language.md` §10.1. |
| Core language | raw string literals (`r"..."`, `r#"..."#`, no escapes), type aliases (`type Name = T;`, `pub type`, generic `type Map<V> = HashMap<str, V>;`; transparent), `inline for` (module level, in `vm` blocks, and as a statement in any block), structs (literals may be module-qualified: `diag.Span { .. }`, and end in `..zero` to zero the fields not given, where zero is valid: numbers, `bool`, options (`?*T`, `?Gc<T>`, `?fn` start as None), slices, raw pointers `*T` (null); not `Gc<T>`, functions, structs, enums or values), `[]` as the empty slice, enums with payloads, `?T`, `Result` with `?`, tuples, arrays, slices, pointers and pointer arithmetic, methods (`fn Type.name`), modules and imports, `match` with exhaustiveness checking (integer literal and range patterns; a `const` in scope, imported or `mod.C`/`Vm.OP_X`, matches its value; arms after one that matches everything warn), string literal patterns on `str` (a length switch, then word-sized compares) (through nested patterns and tuples) and guards (`pat if cond =>`; in vm code, a warning for an arm whose body is an `if` without `else` when a later arm could match), `let ... else`, `if let` chains, loops with `break` values, `for i in 0..n` ranges typed by the non-literal bound or an annotation (`for i: u8 in 0..10`), `for x in e` over an array or slice, `defer`, consts, module-level `var`s (see below), casts (including integer to fieldless enum, checked in debug builds; `Kind.COUNT` gives the number of variants; `x as u64 < y` parses as a comparison), local inference across `if`/`match` branches (`if c { Ok(e) } else { Err(m) }` needs no annotation), checked/wrapping arithmetic, float built-ins on `f32` and `f64` (`to_bits`/`from_bits`, `sqrt`, `floor`, `ceil`, `trunc`, `nearest`, `abs`, `copysign`, `is_nan`, C-style `min`/`max` and IEEE 754-2019 `minimum`/`maximum`, each an LLVM intrinsic), unaligned loads and stores (`p.read_unaligned()`, `p.write_unaligned(v)`), `~x` (bitwise not; `!x` also works on integers), `a ?? b` with `b: T` or `b: ?T`, hints for operators Loimi lacks (`++`, `**`, `?:`, `===`), arbitrary-width integers `iN`/`uN` (1..=64), overflow and bounds checks (overflow traps in debug builds). |
| Vectors | 128-bit SIMD types `i8x16`, `u8x16`, `i16x8`, `u16x8`, `i32x4`, `u32x4`, `i64x2`, `u64x2`, `f32x4`, `f64x2` (LLVM `<N x T>`; language.md §1.2, "Vectors"): lane-wise operators (integer lanes wrap; shifts by a scalar modulo the lane width; comparisons give signed masks), `splat`, `new`, `from_bits`, `lane`/`with_lane` with constant lanes, `lane_at`/`with_lane_at` with a run-time lane (masked to the lane count), `shuffle` with constant indices, `swizzle`, `select`, `bitmask`, `widen_low`/`widen_high`, `narrow_sat`, saturating `add_sat`/`sub_sat`, `avgr`, integer and float `min`/`max`, `minimum`/`maximum`, rounding, `abs`, `popcount`/`clz`/`ctz`, `as` lane by lane (float to integer saturating). Values everywhere a scalar goes (fields, objects, enum payloads, arrays, `?T`, state registers in vector registers across tail calls); vector loads and stores assume no alignment. Baseline SSE2 and NEON; where the IR differs by target: `swizzle` (`tbl` on AArch64), signed `narrow_sat` (`sqxtn`/`sqxtun`), `f32x4` to integer conversions (`cvttps2dq` on x86-64) and float rounding (inline SSE2 on x86-64, no libm calls). `--cpu=NAME` (clang's `-march`/`-mcpu`; `x86-64-v2`, `x86-64-v3`, `native`, ..) builds for a newer CPU: `swizzle` becomes `pshufb` (SSSE3) and the rounding methods `roundps`/`roundpd` (SSE4.1); the default is the portable baseline (lowering.md §12 has instruction counts per level). Not built: vectors in `--header` signatures, 64- and 256-bit vectors. |
| Conditional compilation and atomics | `--cfg NAME` with `#[cfg(NAME)]`/`#[cfg(not(NAME))]` on items (imports included), struct fields and statements, dropped by the parser (a build without the name has the same IR as without the code). Pointer atomics: `atomic_load` (acquire), `atomic_load_relaxed`, `atomic_store` (release), `atomic_swap`, `atomic_cas`, `atomic_add` on 8–64-bit integers (and pointers, but `add`). `language.md` §1.4 and §1.3. |
| Globals | `var NAME: T = init;` at module level (with `pub`, readable and assignable from other modules as `mod.NAME` or after `import mod.{NAME}`). Plain LLVM globals; `#[thread_local]` makes one copy per thread (`thread_local(initialexec)`). The initializer must be a constant expression: literals, consts, `[x; N]`, struct/tuple/array literals, enum variants without payload, `None`/`Some(..)`/`Ok(..)`/`Err(..)` of constants, and functions (for hooks). Globals are not GC roots: a global holding `Gc`/`Value` must be marked by a root scanner (`heap.add_scanner`). |
| Generics | Monomorphized generic structs, enums, functions and methods; type arguments inferred from arguments and the expected type. Qualified variants of generic enums (`Lookup.Missing`, `Lookup.Own(i)`, also in patterns) take the instance from the expected type or the payload. Methods resolve per instance. Static traits (`trait Hash { fn hash(k: Self) -> u64; .. }`, `impl Hash for Sym;`) with default methods, type parameters (`trait Conv<T>`, bounds `P: Hooks<K, E>`), generic methods (`fn fold<U>(..)`), static functions (`T.zero()`) and generic impls (`impl<T: Show> Show for Vec<T>;`). Bounds (`HashMap<K: hash.Hash, V>`, `fn f<T: A + B>`; methods of a generic type inherit the type's) are checked at instantiation, with the error at the use site, and generic bodies are checked against them: a bounded parameter has only its bounds' methods (opaque in a second check of the body, once per combination of the unbounded parameters' arguments); unbounded parameters stay duck-typed. Static dispatch only. A generic function can be used as a function pointer where a function type is expected (`Pratt.new(peek, ...)` with `fn peek<E>(p: *P<E>) -> T`), or with explicit type arguments anywhere (`scan<Gc<Box>>`); passed to another generic function, its type arguments can come from the parameter types alone (`apply(double, 21)` with `fn apply<T, U>(f: fn(T) -> U, x: T)`). Methods can have their own type parameters after the method name, on plain receivers (`fn Cursor.op<K>(c: *Cursor, ops: *OpTable<K>) -> ?K`) and generic ones (`fn Box<T>.map<U>(b: *Box<T>, f: fn(T) -> U) -> Box<U>`); the receiver's arguments come from the receiver, the method's are inferred from the arguments and the expected type. Type arguments can also be given explicitly on calls: `mem.alloc<u8>(n)`, `id<i64>(x)`, `c.op<K>(&ops)`, `Cursor.cast<u8>(&c)` (a method's own parameters only; not on a generic type's static functions such as `Vec.new`). `f<T>(` is recognized when `<` directly follows the name and a balanced, type-shaped `<...>` is followed by `(`; otherwise `<` is a comparison. |
| `value` types | `nanbox`, `lowtag`, `boxed` representations; NaN canonicalization; tuple patterns merged into one test (xor/or/mask, umax). Integer payloads up to 48 bits (nanbox) or 61 bits (lowtag); more than 7 (nanbox) or 8 (lowtag) tagged variants when payloads are narrow, with tags sized to the payloads; in a nanbox, more pointer variants than 48-bit blocks: `Gc` variants share a block, up to 8, told apart by the pointer's low 3 bits (the shared layout, `block 45`; tests are `(v & mask) == tag`, the payload is the usual `and` with the low tag folded into load displacements). The layout is visible and can be pinned: `loimi build --explain-values` prints every value type's encoding and which tuple tests merge; `Int(i32) block 48` pins a variant's block width; the `value_layout` warning names every unpinned variant that a non-classic layout narrows (so a new variant cannot silently change the others' encoding; an error under `-W error`); `V.X.mask`, `V.X.tag`, `V.X.block` and `V.Num.limit` are the encoding as constants, for code that tests raw words. Tag pinning (`= tag 0b001`) and `lowtag(bits = N)` are not implemented: lowtag uses 3-bit primary tags. |
| `vm` declarations | state, context, operand kinds (`const` kinds: what they read cannot change after verification, checked within the vm), fixed-width formats, inline caches (fields of any integer, float, `bool` or pointer type, wider ones spanning several code words with unaligned access; no GC references; `Vm.Resume` fields for verified code addresses, reset by the verifier and set with `fill_resumes`; `emit.OP_cached` emitters with initial contents), `then OP` (typed operands of the following instruction), `family` + `become`, `super` (any number of parts, including family members; `become` in a part re-targets the super; `#[forward]` adds opcodes, forwarding variants, in which a part reads the previous part's result from a machine register instead of the register file; `optimize` picks them where the operand fields match; kinds that are views of one slot, differing only in the types they cast to, link like one kind; on every super of the three interpreters: 0.97–0.99 of their time by geometric mean, up to 0.92 on float loops, see `bench/mini/RESULTS.md` item 28), accumulators (`accumulator acc: T for kinds;` state registers and `#[accumulate]` variants of ops that keep a temporary in one between adjacent ops, picked by `Vm.accumulate`; the wasm interpreter: 0.91 of its time), vm functions (including `-> never` ones with control words and `next_pc`, the address `next` continues at (in an op with `then`, the `then` word's address, with a warning; `then_next_pc` is past it), and `c: cache Name` parameters that receive the calling op's cache), entries, `on raise`/`raise`, `cold`, `sync`, `reload` (run after each `reenter` call in vm code), nested entries (`entry call(..) uses reenter`: a run above an active one, for natives calling guest code; callers need the effect; a generated thread-local `Vm.depth` counts them), in all three dispatch modes. |
| Dispatch | `tailcall` (`preserve_none` + `musttail`, instruction word passed in a register; handlers are compiled without memset/memcpy/memmove formation, and loops in tail-call vm code are not vectorized or unrolled, so no loop gives them a stack frame), `goto` (one function; a shared `indirectbr` block that LLVM copies into every handler after register allocation, so compile time stays near linear in the op count: `lowering.md` §5), `switch`. |
| Targets | x86-64 Linux (native) and AArch64 Linux (`--target=aarch64-linux-gnu [--sysroot=DIR]`: clang `--target`, linked with `ld.lld`; `LOIMI_RUNNER` runs `loimi run`/`test` programs under an emulator). The IR is target-independent except a few vector operations ("Vectors" row) and `std.rt`'s handlers (LLVM's builtin setjmp on x86-64, glibc's `_setjmp` on AArch64) and tail-call vms, which on AArch64 keep the dispatch table's address in a state register instead of an `adrp`+`add` per dispatch (lowering.md §3; x86-64 indexes the table as an absolute address). AArch64 is validated under qemu user-mode (`bench/cross/install.sh`: qemu and a Debian arm64 sysroot from pinned sources): the whole golden suite passes cross-compiled (`LOIMI_TARGET`, all dispatch modes, GC stress and verify, C hosts, `rt` panics), and `cargo test` runs a subset under qemu (`aarch64_qemu`) when the toolchain is installed. Not yet run on AArch64 hardware or with 16K/64K pages. The JIT (x86-64 only) and `--sanitize` do not combine with `--target`. Code is for each target's baseline (x86-64: SSE2) unless `--cpu=NAME` (or `LOIMI_CPU`) names a CPU: clang's `-march` (x86-64) or `-mcpu` (AArch64) for every clang run of the build (stencils and C runtimes included), and the CPU's extensions, read from clang's predefined macros (so `native` and any name clang knows work; an unknown name is an error), choose Loimi's own vector lowering. Interpreters gain nothing measurable from `x86-64-v3` (run_all on Stak, Lox, Scheme, Mini and the wasm benchmarks: within noise; lowering.md §12). |
| Generated per vm | encoders (`Vm.OP(..)`), emitters (`Vm.emit.OP(..)`), opcode constants, `op_size`, `op_name`, `Vm.Op` and `Vm.op_of(w)` (an enum of the ops for exhaustive `match`es in host tools), the superinstruction peephole (`Vm.optimize`), deoptimization (`Vm.deopt`: every word back to its family's generic op, supers split, caches zeroed), the verifier (`Vm.verify -> Result<Verified<Vm>, Vm.VerifyError>`, with any number of `verify_context` parameters; the error says which word and which check, `e.write(&out)`), the proof type `Verified<Vm>` (opaque; `jump v`, `v.start()`, `v.words()`; `#[require_verified]` vms start entries only from one, or from a resume point `Vm.Resume` (an instruction boundary of verified code: `next_resume` in vm code, `v.resume_at(i) -> ?Vm.Resume`, `Vm.Resume.placeholder()` (traps), `unsafe { Vm.assume_resume(p) }`; generated only for programs that name them); `unsafe { Vm.assume_verified(code) }`, `Vm.placeholder()`), `Vm.build_verified(heap, Obj { .. }, words, ctx..)` for objects with a `code: verified [W] by Vm` trailing array (`owner` verify contexts; `unsafe { Vm.build_assume_verified(..) }`), `verify NAME { .. }` side-table checks (over the base view of the code; `reject(at, msg)` reports a word and a message), stack effects for stack VMs (`op X() : N stack(a b -- c)`, `stack_entry`: depth checks in the verifier, `v.max_stack()`, `Vm.stack_effect(w)`, `Vm.stack_effect_text(w)`), label helpers (`Vm.set_label(w, off) -> ?W`, range-checked; `Vm.label(w) -> ?i64`), per-format field readers (`Vm.ABC_a(w)`), setters (`Vm.set_ABC_a(w, v) -> ?W`, range-checked) and encoders (`Vm.encode.ABC(op, a, b, c)`; setters and encoders are compiled only where called) and per-op operand metadata (`Vm.operand_count(w)`, `Vm.operand_kind(w, i)`, `operand_bits`, `operand_signed`, `kind_name`, `kind_is_label`, `NUM_KINDS`), the disassembler (`Vm.disasm(code, &out)`, `Vm.disasm_insn(code, i, &out) -> size`, generic over the output buffer; operand kinds can declare `display "r{}"`). The items are named `Vm__X` in the vm's module: an op named like one of them (`label`, `verify`) is an error, as is a module item named `Vm__..`; item names starting with `__` (generated C declarations) and operand names starting with `__` are reserved. |
| Instrumentation | `variant NAME wraps all|[OPS] { before { .. } }`, switched with `use_variant`; `#[unfused]` variants run supers as their parts (every instruction is wrapped). `#[cfg(..)]` on vm items. |
| Debugging guest programs (prototype) | `on break { .. }`: generated breakpoint ops (one per inline cache layout, so a breakpoint has the size of the instruction it replaces), `Vm.set_break(v, i) -> ?Vm.Break` on verified code (splits a super spanning the word), `Vm.clear_break`, `b.rearm()`, `Vm.is_break`; code with breakpoints does not verify, `deopt` keeps them. `std.debug`: breakpoint sites with the step-over protocol, line stepping (`step`/`next`/`finish`), a command reader. `std.debug` also has `Stepper.left` (a nested run's frame left to its native), `line_sites` (line starts with loop heads), `unpatched` (code without breakpoints, for label walks) and line 0 as "no line". Mini built with `--cfg debug`: `mini --debug script < commands` (break, tbreak, delete, info, continue, step, next, finish, bt, locals, print, op, quit); the Python subset built with `--cfg debug`: `python-loimi --debug prog.py`, pdb-like (generators, exceptions, comprehensions, `sorted` keys); golden transcripts of both in all three dispatch modes; default builds unchanged (IR byte-identical), the debug builds not debugging within noise. **For editors** (Mini, `--cfg debug`): a stop-callback C API (`mini_debug_*` in `embed.lm`: breakpoints by file and line, pending until code loads; conditions, a Mini expression evaluated in the stopped frame by a nested run with the breakpoints suspended; hit conditions; break on uncaught errors before they unwind; a pause from any thread, a flag polled at back-edges and calls; frames, locals with upvalues, globals, tables expanded through handles; evaluation of expressions and statements), and a Debug Adapter Protocol server in C (`examples/mini/dap/mini_dap.c`, no dependencies: initialize, launch, setBreakpoints with conditions and hit conditions, setExceptionBreakpoints, configurationDone, threads, stackTrace, scopes, variables, evaluate, continue, next, stepIn, stepOut, pause, disconnect, terminate; events initialized, stopped, output, exited, terminated); a scripted DAP session's transcript is golden in all three dispatch modes, and a C host tests the API under GC stress. `std.debug` gained conditions, hit conditions, `suspend`, `Refs`, pause requests; the command line gained `break LINE if COND`, `print EXPR`, the error stop. Not built: `attach`, `setVariable`, coroutines as DAP threads. `#[cfg(..)]` on struct literal field initializers. Not with `--jit-stencils`, not in stack vms. See `design/debugging.md`. |
| Instrumentation (counts) | `loimi build --vm-stats` counts executed ops, adjacent op pairs and sequences of 3 and 4, and entries into each `cold` block (per op, with its source location), and prints the counts and frequent pairs and dominant longer sequences at exit as `super` declarations. `Vm.optimize` picks fusions by dynamic programming (the most dispatches saved; greedy on ties). |
| JIT (prototype) | `loimi build --jit-stencils`: a copy-and-patch JIT generated from the vm's handlers (stencils with holes, read back from clang's object file by `src/elf.rs`, a C runtime in `src/jit_rt.c`), turned on at run time with `LOIMI_JIT=1`. Runs Mini, MiniJS, Scheme and Prolog (0.75, 0.86, 0.82 and 1.01 of their interpreters' time by geometric mean, `results.md`; Prolog without `sieve`, whose assert/retract make code objects the JIT compiles and never frees: 21× there); with the JIT off, a JIT build runs as fast as a default one. Quickening re-patches compiled code instead of dropping it. Operand kinds marked `const` (all three interpreters) have their constants patched into compiled code, with stencil variants per constant tag; no measurable gain. Keeping guest registers in machine registers was measured too (a third at best on mandelbrot, at most 12% over the suite; a one-register cache, kept on branch `jit-regcache-experiment`, is 1.5% slower): the template-JIT line stops here. x86-64 Linux executables, tail-call dispatch, one vm, no variants. See `design/copy-and-patch.md`. |
| GC | `object` types (with trailing arrays), generic `object` types (`object Obj<V> { .., inline: [V] }`; one type id and trace function per instance), `Gc<T>` (also over a type parameter, checked per instance), `new(heap, ..)`, `try_new(heap, ..)` (no refill call: `None` where `new` would refill, for frameless handlers, with `gc.make_room` in a cold block), `gc_mark`; `std.gc` mark-sweep written in Loimi; compiler-generated tracing (including pointers inside values); shadow-stack frames for traced locals, and for vm state variables of traced types (a cached top of stack), copied into the frame at each call that may collect, so `sync` and root scanners need not know them; an optional generational mode (`heap.set_generational`, on in Mini, MiniJS and Scheme: non-moving, sticky mark bits with an epoch flip per major collection, lazy sweeping, aging for large objects, an object-logging write barrier the compiler emits after every store of a reference into an object (none for values statically free of references; its slow path makes no call, so it gives handlers no stack frame), `gc.write_barrier` for raw-pointer stores, a barrier after calls that may store through a slice of an object's trailing array, verify mode); weak references (`heap.new_weak`), ephemeron tables (`gc.Ephemerons`) and weak intern tables for GC strings (`gc.WeakInterner`, with pinned keys); `#[trace(f)]` hooks on objects (trace the live part of a trailing array, or references in memory the object owns), free hooks for host memory an object owns (`heap.on_free`, typed `heap.on_free_gc`, run when it is found dead and by `Heap.free`; no allocation, no resurrection) and dirty pinning for objects written without barriers (`heap.pin_dirty`: a root traced by every collection, minor ones included; `stdlib/gc.md`); `unsafe { p as Gc<T> }` from a pointer or address; collection only in calls marked `uses gc` (effects: Differences below); `loimi build --gc-check-roots` checks every root (below). |
| Closures | Non-escaping `block fn(A) -> R uses E` parameters and `|a, b| expr` literals, capturing by reference; only as call arguments, never stored or returned (checked). Effects of the body are checked against the type. Lowered to a separate function with an environment of slot pointers in the enclosing frame (small `let`s copied in); code and environment passed as two arguments, so LLVM specializes callees for a closure literal; plain functions convert through a trampoline. Not allowed in vm code. Escaping closures: `closure fn(A) -> R uses E` (`{ code, env }`, untraced) and `Gc<closure fn(..)>` (a traced closure object), created with `new(a, \|..\| ..)` where `a` is an allocator with `alloc_bytes` (`*mem.Arena`) or a `*gc.Heap`; captures copied in (assigning one, capturing a `block fn`, or a traced value in an arena environment are errors); generated trace function per GC closure literal, environment rooted by the body; plain `fn` converts to `closure fn`, `closure fn` to `block fn`; callable (not creatable) in vm code; not across C. State of a closure's own in a state list, `new(a, [var i = lo, var n] \|..\| ..)` (environment fields, assignable in the body, kept between calls; in a GC closure traced state is traced through the environment and assigned with the write barrier, not from a nested `block fn`); shared state in a cell from `new(a, v)` (a `*T` in allocator memory; untraced values only). `c.free(a)` frees one `closure fn`'s environment with `a.free_bytes(p)`, and nothing for closures that own no memory (no captures, converted plain functions: the code is compared with the signature's trampoline); `std.mem.Malloc` frees one by one, `Arena.free_bytes` does nothing. Use after free is not detected (ASan does); freeing does not free what captures point to. |
| Standard library | `std.io` (with `Out`, buffered output), `std.os`, `std.mem` (`Vec` with `sort_by` and `try_sort_by` (stable, with a fallible, re-entrant comparator), non-allocating when empty; `memchr`/`memmem`/`memcmp`-backed byte search and compare on slices; allocators `Arena` and `Malloc` (frees one by one)), `std.hash`, `std.map` (`HashMap`, with `clear`; `OrderedMap`, below), `std.text` (`StrBuf` with `fmt`/`add` and `{}` placeholders over the `Fmt` trait, `Interner`, number parsing/printing), `std.gc` (GC row), `std.gil` (Threads row), `std.rt` (`protect`/`throw`, `catch_panic`, `panic`, `panic_message`), and the modules below. Every module is described in [`stdlib.md`](stdlib.md). |
| `std.num` | Exact numeral parsing with overflow reporting (`parse_int`, `parse_i64`/`parse_u64` in radix 2 to 36, `parse_u64_wrapping`, `parse_f64` via strtod with strict syntax, `parse_number` for literals, `decimal_len`), float formatting (`write_g` for `%.14g`, `write_shortest` for shortest round-trip, `shortest` digits and `write_js` for ECMAScript's Number::toString, `mark_float` for Lua's `3.0`, `write_hex`), `add/sub/mul_overflow`, `floor_div`/`floor_mod`/`trunc_div`/`trunc_rem` with the `MIN / -1` case defined, float `floor_div_f64`/`floor_mod_f64`, `to_i64_exact`. |
| `std.map.OrderedMap`, `std.dict.Dict` | Insertion-ordered maps in CPython's compact layout (index table plus dense entries with stored hashes, tombstones, compaction): `OrderedMap` in host memory, `Dict` in the GC heap. Entry indices stay stable until a compaction and a `version` counter changes with the key set or layout (for inline caches); `find_with` is a lookup whose equality may fail and run guest code (`uses reenter`) and starts over when the map changed during it, as CPython's does. examples/python's dict is `std.dict.Dict<Value, Value>` (benchmarks 0.97x of its own dict's time). `std.dict.KeyDict` is the same without stored hashes, for keys that cache their own (CPython's str-keys layout; 16-byte entries for `Value` keys): examples/python's instance attributes (benchmarks unchanged within noise against `Dict`, an instance with 7 attributes 0.84-0.90x). |
| `std.bigint` | Arbitrary-precision integers with Python's semantics: 64-bit limbs (on the `u64` intrinsics `mul_wide` and `div_wide`), schoolbook (rows two at a time, on the carry-chain intrinsics `add_carry`, `sub_borrow`, `mul_add_wide`), Karatsuba and Toom-3 multiplication and squaring, Knuth and divide-and-conquer (Burnikel-Ziegler) division (floor and truncating), Zimmermann's Karatsuba square root, binary powering, shifts and two's-complement bit operations, correctly rounded `to_f64`, `from_f64`, CPython's int `hash`, parsing and formatting in radix 2 to 36 (divide and conquer above 16 and 512 limbs). Three layers: caller-buffer functions over `Num` views, an owned `BigInt`, and a GC object `Big` (`gc_add(h, Arg, Arg)`, ...). Differential test against CPython in `cargo test`; 2.8-6.4x faster than CPython 3.13 on the stdlib.md benchmarks, 1.3-10x on million-digit conversions and division. Used by examples/python and examples/scheme. Not built: FFT multiplication, subquadratic gcd. |
| `std.test`, `std.prof` | `loimi test FILE.lm [FILTER..]` runs the file's `#[test]` functions, each under a panic catch, with `#[track_caller]` assertions (`test.eq`, `ok`, `some`, `has`, `panics`, ..) that report the test's line and both values, stdout/stderr capture and golden files (`--update`); `examples/regex/verify_test.lm` and the tutorial's step 9 `unit_test.lm` use it. `std.prof`: monotonic clock and `Timer`, `Bench` (warmup, interleaved rounds, min/median), `Counters`, and the `--vm-stats` op counts read in-program (`prof.op_counts`, with `reset`). |
| `std.diag` | `Span`, `SourceMap` (borrowed files, lazily built line-start table, binary-searched `line_col`, `line_text`, `write_location`), `Diag` (severity, message, primary span with label, notes and help lines, built with `with_label`/`with_note`/`with_note_at`/`with_help`), `line_span` (for errors that only know a line), rendering with a snippet and carets in the compiler's format (lines over 160 bytes cut to a window around the span), and a `Diagnostics` sink that copies messages into an arena, counts errors and warnings, and prints a summary. |
| `std.lex` | A scanner toolkit rather than a generator: `Cursor` (peek, bump, line tracking, marks, spans, errors), `Syntax` presets (`lua()`, `c_like()`, `js()`, `none()`) for line, block, nested and Lua long-bracket comments, escape and overflow rules; `skip_trivia`, `ident`, `number` (via std.num), `quoted` (escapes into a `StrBuf`, including `\ddd`, `\xHH`, `\u{..}`, `\z`, line continuation), `long_string`, `OpTable<K>` (maximal munch), `Keywords<K>`. Errors are `lex.Error` (span and message), convertible to a `diag.Diag`. A complete lexer for Mini's token set is about 50 lines (`tests/run/lex_toy.lm`). |
| `std.parse` | `Pratt<P, K, E, X>`: binding-power table indexed by token kind (`infix_left`, `infix_right`, `infix`, `prefix`) and a driver that calls the user's `peek`, `advance`, `atom`, `unary`, `binary` and optional `before_rhs` hooks through function pointers. `Parser<P: Hooks<K, E, X>, K, E, X>`: the same table and driver with the hooks as methods of the parser state (trait `Hooks`, with defaults for `before_rhs` and `led`). `Ops`: operator definitions changed at run time (Prolog `op/3`: `u64` names, priorities, the seven types, prefix/infix/postfix per name) and a Prolog-style operator-precedence driver (`OpHooks`, `op_parse`, `op_parse_arg`); the Prolog reader uses it. |
| `std.bc` | Bytecode construction over any vm's generated encoders: `Code<W>` (given `Vm.op_size` and a jump-patch function: `op` reserves inline-cache words, labels with fixups, jump lists, range-checked patching with sticky errors, a run-length position table with per-word expansion and a delta-encoded form, `last`/`truncate` for peepholes, jump threading, branch relaxation into a long-jump form), `Field` (encoded bit fields), `Pool<K: hash.Hash>` (deduplicating constant pool with a limit) and `Regs` (Lua-style locals/temporaries register allocator with maximum tracking). |
| `std.obj` | Hidden-class objects over a generic `obj.Obj<V>` or the interpreter's own object type: shapes in a transition tree with weak edges (cleared by a sweeper) and never-reused ids, inline and overflow slots, dictionary mode, prototype chains with validity cells, monomorphic cache fill helpers (shape id + SlotRef in two cache words; a hit is two loads, a compare and the slot load), `Poly`, a global `MegaCache` (hits inlined into handlers, and reporting whether the value was own or inherited), allocation templates, per-property attribute bits (`define`, `set_attrs`, `attrs`, `keys_where`; `ic.readonly` keeps write caches off read-only properties). No elements, global property cells or `SetError`. |
| Examples | Eight interpreters, each tested by `cargo test` (see the README for one line each; [`results.md`](results.md) for benchmarks). `examples/mini/`: a Lua 5.4 subset (single-pass compiler on std.lex, std.parse, std.diag and std.bc; GC'd strings and tables; `table.sort` with Lua comparators through re-entry; coroutines with their own stacks in host memory, switched inside the running vm run, resumed from natives through a nested run and a `Mini.Resume`, `examples/mini/COROUTINES.md`; threads sharing one heap with `--cfg gil`), embeddable in C (`embed.lm`, natives that call back). `examples/scheme/`: an R7RS-small subset (reader on std.lex and std.num, code generator on std.bc, generational heap). `examples/minijs/`: a JavaScript subset on std.obj (shapes, mono/poly/mega inline caches, prototype validity cells). Written from the docs as usability studies, each with a `FRICTION.md`: `examples/lox/` (all 246 of the book's clox tests), `examples/regex/` (a backtracking regex engine as a C library), `examples/stack/` (Stak, a Forth with declared stack effects), `examples/wasm/` (MVP plus sign extension, saturating conversions, multi-value, reference types, bulk memory and fixed-width SIMD on Loimi's vector types; every spec test file passes, SIMD's 57 included; accumulators, `#[forward]` float supers, constant slots, calls that zero only locals read before written, `p + k` loads and stores, bit-test and load-and-branch fusion), `examples/python/` (a Python 3 subset: classes, generators, exceptions, ints on `std.bigint`, dicts on `std.dict`; the 11 pyperformance programs in `bench/python` print CPython's output). |
| Tests | `cargo test`: golden programs (`tests/run/`, every vm test in all three dispatch modes), expected compile errors (`tests/fail/`), `loimi test` (`tests/test/`), every example's tests (Mini, Scheme, MiniJS, Lox, Stak, wasm, Python, the regex library; Mini's, Scheme's and MiniJS's normally, with GC stress, and in verify mode, i.e. generational heap under stress, checking the write barriers at every collection; Lox's and Python's also with GC stress), the tutorial's steps against their chapters (`tutorial_steps`), the README's and `docs/`'s code blocks (`tests/docs.rs`: each compiles, runs or fails as its annotation says), `std.bigint` against CPython, a few seconds of bytecode fuzzing and of GC rooting fuzzing (below), and C hosts built against `--lib` and `--shared` libraries and their generated headers (`tests/embed/`: the C ABI, caught panics of every kind, also through nested vm runs and host callbacks, ASan builds, two `--vm-stats` libraries in one host; embedded Mini with two states, natives calling back, and GC stress; Mini on four threads and with a shared heap, also under TSan when GCC's `libtsan` is installed). Re-entry: `tests/run/vm_reenter.lm` (all three modes, a stack that moves on every callback), Mini's sort and coroutine scripts in all three modes. AArch64: `aarch64_qemu` (above, Targets). |

## Bytecode fuzzing (M5)

M5 asks for a bytecode fuzzer that finds no crashes in verified code. `tests/fuzz/run.sh` builds the fuzzers with `--sanitize=address`
and runs them for a time or seed budget, saving failing seeds:

- `tests/fuzz/fuzzvm.lm`: a test vm using every verifier feature
  (register, constant and context-indexed operands, labels of three
  widths, a call operand checked with `is_insn_boundary`, two verify
  contexts, inline caches, families with `become`/`redo`, supers with up to
  four parts and family-member parts, `then JMP`, `-> never` vm functions,
  cold blocks, variants), declared once per dispatch mode. Random programs
  (mostly well-formed, some mutated, some random words) are verified and,
  if they pass, run in all three modes with exactly-sized heap buffers and
  a step budget. Besides crashes it checks that the modes agree and that
  code still verifies after running (quickening) and after `Vm.optimize`.
- `tests/fuzz/minifuzz.lm`: random code against Mini's own `verify`
  clauses and `Mini.run`, on a stack exactly as large as the frame.

ASan works in all three dispatch modes. `--sanitize=address` turns off
use-after-return detection: its fake-stack fallback is a dynamic alloca,
which LLVM cannot combine with stack realignment in `preserve_none`
functions, so tail-call builds would not compile. `musttail` survives the
instrumentation. ASan executables do not report leaks (programs leave their heaps
to the OS); `ASAN_OPTIONS=detect_leaks=1` turns leak checking on.

**Run so far: 2 h 10 min** under ASan (2026-11-02), before and after the
n-ary supers / `then` changes, 20 minutes of it on the final verifier:
149M test-vm programs (55M verified and run in all three modes) and 11.6M
Mini programs (3.0M run), in debug and `--release` builds. `cargo test` runs a few seconds of both
without ASan.

Bugs found and fixed (regression tests in `tests/run/vm_verify_*.lm`):

- Verified code could run off its end: a last instruction using `next`
  read past the buffer, and empty code verified. The verifier now rejects
  both (see `language.md` §6). It also missed ops that save `next_pc`
  without naming `pc` or `next` (Lox's calls): code ending in one passed,
  and the callee's return would run past the end. Found while adding
  resume points (2026-11-03; `tests/run/vm_resume_point.lm`).
- Before the n-ary super rewrite: a super's later words were checked only
  as the declared part's operands, so after the first part quickened they
  ran as unverified instructions; and `Vm.optimize` produced overlapping
  supers that failed verification.
- Found by review while fuzzing: `verify` clauses are pasted into the
  generated verifier, whose locals shadowed verify contexts of the same
  name (`verify_context w: u32` compared with the instruction word).

Holes in Mini's verify clauses (`examples/mini/vm.lm`), now fixed:
CALL's `nres` let the callee's return write past the stack; SETLIST's,
RETN's and CONCAT's counts (and CALL's argument count) read past the
frame; CONCAT with fewer than 2 values and GETINDEXI with index 0
underflowed; SELF wrote `R[a+1]` with only `a < nregs` checked. Each
count now has its own operand kind whose `verify` clause reads the base
register from the instruction word (`Mini.ABC_a(*pc) + it <= nregs`);
the kinds decode to the plain field, so handlers are unchanged.
`tests/fuzz/mini_holes.lm` checks that all nine reproducers are
rejected (run by `cargo test`).

## GC rooting fuzzing

`loimi build --gc-check-roots` makes the collector check every
reference it marks (shadow-stack roots, copies of vm state, `gc_mark`
in scanners, traced fields) before reading its header, fills unassigned
traced slots and freed objects with 0x01 bytes, and reports a bad root
with its frame, function, call site and slot (lowering.md §9).
`tests/fuzz/rootgen.rs` generates random programs that mix traced
locals (`Gc`, options, a struct, a tuple, an enum, a nanbox `value`, an
array) with every binding form (`let`/`var`, `let .. else` (also `else
cold`), `if let` chains, `while let`, `match` with guards that collect
and fail, or-patterns and tuple patterns, `for`, `loop` with `break`
values, `defer`, closures), early exits, stores, and calls that collect,
in plain functions and in a vm's handlers, `cold` blocks, vm functions
and `on raise`. Each is built in this mode and run without collections,
under GC stress, and under generational stress with verify; the outputs
(digests that read through every reference) must agree across runs and
dispatch modes, and objects only out-of-scope locals reached must be
gone after a full collection (`Weak` probes). It keeps to what the
language promises: no traced temporary is held across a call that may
collect.

    cargo test --release --test fuzz fuzz_gc_roots       # smoke: 18 fixed seeds, ~20 s
    ROOTFUZZ_SECS=300 ROOTFUZZ_JOBS=2 ROOTFUZZ_FIRST=1000 \
      cargo test --release --test fuzz fuzz_gc_roots_long -- --ignored --nocapture
    ROOTFUZZ_SEEDS=17,42 ROOTFUZZ_KEEP=1 ...              # rerun seeds, keep target/rootfuzz/roots-SEED.lm

(`ROOTFUZZ_PLAIN=1` builds without `--gc-check-roots`.) Seeded bugs,
detected by the 18 smoke seeds with the checks (without):
let-else bindings rooted in the else block (the bug of commit 334d6cc)
18 (16); a match arm's bindings not rooted 10 (2); a block's locals
rooted after their scope 18 (18, as leaks); `if let` bindings rooted
in the else branch 14 (13); a match scrutinee not rooted during guards
(the bug below) 5 (0).

**Run so far: 14 minutes** (2026-11-03), 535 programs, each in
three dispatch modes and three GC settings. Found and fixed: a `match`
guard that collects ran with a temporary scrutinee (`match f() {..}`)
unrooted, so arms after a failing guard bound freed objects; in vm
code also with the earlier fix, since a call there is a vm node
(`tests/run/gc_roots_match_guard.lm`). Found: LLVM 19
crashed in `LoopSimplifyCFG` (at `-O1` and `-O3`) on the goto-mode run
loop of about 1% of the generated programs (the IR verified); gone since
goto mode emits one shared dispatch block (`lowering.md` §5,
`tests/run/goto_loop_simplifycfg.lm`), and the fuzzer no longer skips
those builds.

## Differences from the design

- **Collection happens only at safepoints.** Allocation never collects;
  only functions with `uses gc` (such as `gc.collect`) do. The effect rules
  are as designed, but "may collect" is now explicit and rare. As a result,
  the only roots the compiler has to provide are traced locals that are in
  scope at those calls, and in vm code the traced state variables (copied
  at the call), registered in shadow-stack frames. Traced
  *temporaries* are not roots: bind a value to a local before a call that
  may collect. (The exception: a value computed before defers run, such
  as a block's tail value or `return v`, is rooted while they run.)
- **`sync { ... }`** is a new vm item. It holds the code the compiler runs
  before an effectful call from a handler, for example publishing `fp` or
  the stack top for the GC's root scanner. The design's fixed `ctx.sync` area
  became user-defined code.
- **Cold blocks are outlined with `musttail`, not with `preserve_most`
  calls.** In tail-call mode, a `cold` block in an op handler that ends in
  control words moves to its own function with the handler's signature,
  and the handler reaches it with `musttail`. The hot path makes no call,
  so it needs no stack frame. The instruction word travels in its
  register, and the cold function decodes the operand fields it uses
  again; other handler locals it uses travel through thread-local slots
  (`initialexec`), written only on the cold path.
  `if c { cold { .. } }` followed by code ending in a control word gets that
  continuation copied into the cold block, so it can be outlined too. This
  includes superinstruction parts: an outlined `next` from a part other
  than the last dispatches the next part's word on its own. Other
  cold blocks, and all cold blocks in goto/switch mode, stay inline and are
  marked unlikely.
- **State kinds** (`Cursor`, `Window`, `StackTop`) are plain pointers.
  Stack VMs declare per-op stack effects instead (`op ADD() : N stack(a b
  -- c)`, `language.md` §3.4a): the verifier checks depths and records
  the maximum (`v.max_stack()`), but the names are not bound in the
  handler, which moves its own stack pointer.
- **`unsafe`** is required for `p as Gc<T>` and `#[unsafe]` functions
  everywhere. Raw pointer dereference, indexing and arithmetic are
  allowed in modules without `#![safe]` (the design draft required
  `unsafe` for them everywhere); see `language.md` §1.3 for why. A
  module marked `#![safe]` is checked: there a `*T` is a reference to
  one value, and indexing, arithmetic, pointer casts, other modules'
  buffer pointers and invariant fields, `extern` and `#[raw]` calls need
  `unsafe { }`; so do reading pointers held in another module's
  tuple/array/function fields or enum payloads, zero-initialized
  pointers, `new` with an allocator whose `alloc_bytes` is not `#[raw]`,
  and declaring a `vm`. Pointer stability is checked in `#![safe]` and
  `#![lifetimes]` modules (prototype, `design/lifetimes.md`): a pointer
  into a container used after a call that may move or free it or
  remove its elements (`#[shrinks]`), Vec copy aliasing, arguments
  aliasing what the callee moves, escaping local addresses, and a
  pointer into growable memory stored where it outlives the function
  are errors; other pointers stored in memory are not followed. Every
  module of the example
  interpreters opts in (`#![lifetimes]`). Under
  `--sanitize=address`, `Vec` and `Arena` poison memory they keep but no
  longer use, so stale pointers into it are reported at run time.
  `std.lex`, `parse`, `diag`, `text`, `hash`,
  `io`, `bc`, `map` and `dict` are safe (`design/safety.md`).
  GC objects are checked in every module (`design/safety.md` section
  8): slices of traced trailing arrays cannot be kept where a store
  through them would skip the write barrier (a call that may store
  through one gets the barrier after it), trailing arrays start
  zero-filled only where zero is a valid element (`new(h, X { .. }, n,
  v)` fills them otherwise), and a `#![safe]` module's objects are built
  and written only by that module.
  The example interpreters' only `unsafe` blocks are five zero-filled
  `Gc` arrays that are filled right after allocation (Lox's and Mini's
  closure upvalues, three `Protos` tables): float reads of a
  register use `f64.from_bits(a as u64)` (compiled to a `double` load
  of the slot), and Mini's coroutine free hook uses `heap.on_free_gc`.
  A checked register window was measured and not adopted (roadmap open
  question 7).
- **Operand fields** decode to `u64` / `i64` in handlers and verify
  clauses, whatever their width (the design left them at the field width).
- **`-> never` vm functions** are inlined into each call site, except
  that in tail-call mode, calls from `cold` blocks (and recursive calls)
  `musttail` to one shared copy that receives its arguments through
  thread-local slots. They cannot use `become` or the caller's operands;
  the op's inline cache can be passed as a `c: cache Name` parameter (also
  to plain vm functions), which a shared copy receives as an address.
- **Re-entry** (`language.md` §10): the design's `Mini.call` is not
  generated whole. The compiler provides the nested entry (an entry
  declared `uses reenter`, which callers must be allowed to make), the
  `reload` block after `reenter` calls in vm code, and `Vm.depth`; the
  interpreter writes where the new frame goes, the sentinel that makes
  the callee's return leave the run, the depth limit (Mini: 200, "C
  stack overflow"), and restoring its frame registers after the call
  (Mini's `vm.call_value`), because only it knows its frame layout.
  Derived pointers held across a `reenter` call (`let p = regs + a`)
  are not checked. Measured on Mini, a nested run per callback costs
  about 20 ns over a native comparison and makes a native `table.sort`
  with a Lua comparator 1.2x slower than the same sort written in Lua
  (`bench/reenter/`, `lowering.md` §4), so the Scheme and MiniJS
  preludes keep their higher-order builtins.
- **Effects** must be declared; they are not inferred for private functions.
  Handlers and plain vm functions are the exception: their effectful calls
  are synced in place (in a vm function, through the state pointers it
  sees), and callers of such a vm function root their traced locals there.
  Function types carry them (`fn(*Heap) uses gc`), so calls through function
  pointers are checked, synced and rooted like direct calls. Entries
  declare what their vm code reaches: `uses gc` once any op, vm function
  or `on raise` may collect, so an entry's function type says it may
  collect and cannot be converted to one that does not. There are no
  effect variables: a generic function cannot take "a callback with any
  effects" and inherit them.
- **Verifier**: opcodes, extents, operand `verify` clauses, superinstruction
  part words, and "cannot fall off the end" (an op may continue if its
  handler uses `next` or reads `pc`; the scan is over tokens, so it errs
  toward "may continue"), and stack depths for vms that declare stack
  effects. `Verified<V>` (`language.md` §6) ties the code that runs
  to a verification: `#[require_verified]` entries start only from one,
  and Mini, Scheme and MiniJS keep their code as a verified trailing
  array of the prototype (`code: verified [u32] by Mini`, created only
  by `Vm.build_verified`, which verifies the words in the object with
  the object as the `owner` verify context), so CALL reaches only
  verified code at no cost over raw code (`lowering.md` §11). `verify
  NAME { .. }` blocks check side tables: MiniJS's exception handler
  targets. Not covered: the interpreter must not change the context
  fields after verification (they are ordinary fields), and return
  addresses (`jump fp.resume`) are trusted to land on boundaries of
  verified code.
- **Instrumentation**: switching tables happens from vm code (`use_variant`),
  not through an external `set_dispatch`. A debugger turns its variant on
  in the entries and `reload` (Mini's `--cfg debug`, `design/debugging.md`).
- **Front-end stdlib** (`std.lex`, `std.parse`, `std.diag`, `std.num`) is
  smaller than the design: there is no comptime, so `std.lex` is a cursor
  toolkit instead of a DFA generator and the Pratt table is filled at run
  time; there are no `TokenStream` or error-recovery helpers, fix-it hints,
  ANSI color, or Ryu/Eisel-Lemire (formatting and parsing use
  snprintf and strtod). See the notes in [`stdlib.md`](stdlib.md).
- **Back-end stdlib** (`std.bc`, `std.obj`): `std.bc` is a library that
  takes the generated `Vm.op_size` and a jump-patch function rather than
  a generated `Builder`; it has no exception table,
  proto assembly or serialization. `std.obj` has no comptime `Model`:
  it offers a generic `Obj<V>` (or the interpreter declares its own
  object and slot-array types) and a generic `Model` over them; keys are u64 symbol ids; attributes are bits per
  property; elements and global property cells are not built. Shape ids are never
  reused, so inline caches need no reset after a collection. See
  [`stdlib.md`](stdlib.md) and
  [`stdlib/obj.md`](stdlib/obj.md#as-implemented).
- **`inline for`** is token-level expansion: values are token sequences
  (so operators like `+` work), and `#var` makes an identifier from a
  string or identifier value; `#(a ++ "_X" ++ 2)` joins loop variables
  and literals into one. In statement position the copies are
  statements of the enclosing block, with no scope of their own (see
  `language.md` §1.4). Inside a `vm`, a loop variable named like a state
  variable is an error. Errors in expanded code point at the body or the
  element in the source, but do not say which copy they come from.
- **Traits and closures** (see [`design/closures-traits.md`](design/closures-traits.md)):
  no `dyn`, associated types or constants, supertraits, orphan rule or
  overlap check between impls; default bodies are checked when an
  instance uses them, not at the trait. Closures differ from the design
  as listed in the Closures row above: not creatable in vm code, and
  escaping closures copy their captures (assignable state goes in a
  state list).
- **Not yet built**: a moving or incremental GC, card marking and
  aging of small objects in the generational mode, lazy sweeping in a
  non-generational heap, a `weak` field qualifier, finalizers that run
  guest code (free hooks for host memory are built), free threading
  (threads share a heap only one at a time, `std.gil`), `std.rt`'s
  `FrameStack` and `Fiber`, `suspend`, a sampling profiler and a
  test-matrix runner, variable-length encodings, and atomic `become`
  for shared code.
  Smaller items marked in `language.md`: `comptime` and `const fn`, `bits`
  types, the `is` operator (write `if let`), `a.add_overflow(b)` as a
  method (`std.num.add_overflow` is a function), the `pointer` value
  representation, and the warning when `state` + `context` exceed the
  target's argument registers.
- **Generated tooling not built** (described in `language.md` and
  `lowering.md`, marked there): a `Builder` type (emitters plus `std.bc`
  do its job), generated C-ABI entry points
  (entries are internal functions; embedders write `export "C"`
  wrappers), `loimi vm suggest-supers` (`--vm-stats` prints the
  suggestions itself), handler ordering from profiles, the
  `#![heap = ..]` policy parameter, and `Root<T>` handles.
