# Roadmap and open questions

## Implementation plan

Progress (2026-11-02):
- M0–M5 are done. The bytecode fuzzer (ASan, about 2 hours total) found
  and fixed verifier and Mini holes; `cargo test` runs a smoke version.
  `Verified<V>` is built: `verify` returns the proof (or a diagnosis),
  `#[require_verified]` entries accept only it, and the three
  interpreters keep it in their prototypes, so calls reach only verified
  code. The code is a verified trailing array of the prototype
  (`code: verified [u32] by Mini`), built by `Vm.build_verified` with
  the prototype as its verify context, so the proof costs no load per
  call and is tied to the counts it was checked against; `verify`
  blocks check side tables (MiniJS's handler targets).
- M6 is done: std.num, std.diag, std.lex, std.parse, std.bc and std.obj
  exist, and the example interpreters use them. Mini's front end is
  std.lex + std.parse (Pratt driver, `led` hooks for `and`/`or`/`..`) +
  std.diag, its code buffer, labels, constant pool and registers std.bc;
  Scheme's reader is std.lex + std.num and its code generator std.bc;
  MiniJS uses all six. Mini and Scheme emit byte-identical bytecode to
  their hand-written versions. std.obj is used by MiniJS only: Lua tables
  and Scheme records have no shape-based objects to replace (see
  [`design/adoption/README.md`](design/adoption/README.md#stdobj)).
- M7 has three proof interpreters: `examples/mini` (a Lua 5.4 subset),
  `examples/scheme` (an R7RS-small subset) and `examples/minijs` (a
  JavaScript subset).
- **Speed criterion 1 is met.** Mini beats PUC-Lua 5.4 on all 14
  clean-room benchmarks, by 0.36–0.76× of Lua's time.
- **The stretch goal is met.** Mini is faster than LuaJIT's interpreter
  (`-joff`) on 11 of the 14, and within 8% on the rest.
- MiniJS beats `node --jitless` (0.53× by geometric mean) and QuickJS
  (0.40×) on all 14 of its clean-room benchmarks.
- Scheme runs 41 unmodified r7rs-benchmarks programs. By geometric mean
  it takes 0.29× of Gauche's time and 0.47× of Guile's with Guile's JIT
  off, and about the same time as Guile with its JIT on.
- Beyond the plan:
  - closures (non-escaping `block fn`, escaping `closure fn`) and traits;
  - C embedding (`--lib`, `--shared`, `--header`) with re-entrant natives;
  - a generational, non-moving GC with a write barrier and a verify mode;
  - weak references and ephemerons;
  - a generated disassembler;
  - a tutorial (`docs/tutorial/`). See
  [`bench/mini/RESULTS.md`](../bench/mini/RESULTS.md).

See [`status.md`](status.md) for details.

The compiler is written in **Rust** with no dependencies; it emits textual
LLVM IR and drives `clang-19`, pinned to one LLVM major version (≥ 19). Self-hosting
is a non-goal until the language has stabilized. One natural dogfooding target
is the compile-time evaluator itself: it is an interpreter, so it could
eventually be written as a Loimi `vm`.

Each milestone ends with something measurable.

| # | Milestone | Done when |
|---|---|---|
| M0 | Core language → LLVM: functions, structs, enums, ints/floats, pointers, slices, `match`, modules, C interop | `gcd`, a hash map, and an arena compile and pass tests |
| M1 | `vm` with `switch` dispatch, fixed encodings, operand kinds, generated encoders and disassembler | a toy stack VM (arithmetic + jumps) runs and disassembles |
| M2 | `tailcall` and `goto` dispatch, `cold` outlining with `preserve_most`, decode-ahead | same toy VM in all 3 modes; per-mode benchmark table; inspect asm for zero state spills in hot handlers |
| M3 | `value` types: `nanbox`, `lowtag`, `boxed`; tuple-pattern tag merging | microbenchmarks show `(Int, Int)` = one test; repr switch is a one-line change |
| M4 | `std.gc` MarkSweep, `object`, `Gc<T>`, effects + sync points, shadow-stack roots | GC-stress mode passes the test suite with no explicit rooting in user code; a deliberately missing effect is a compile error |
| M5 | Generated verifier, `Verified<V>`, families/`become`, inline caches, `super`, `--vm-stats`, variants | a bytecode fuzzer (ASan builds, a smoke run in `cargo test`, short campaigns after verifier changes) finds no crashes in verified code; instrumentation variants add 0% when off |
| M6 | Stdlib front-end pieces: `std.lex`, `std.parse`, `std.diag`, `std.bc`, `std.obj`, `std.num` | the example interpreters use them instead of hand-written equivalents |
| M7 | **Proof interpreters**: a Lua 5.4 subset and a small Scheme | see below |

## What success looks like

The language exists to make fast interpreters easier to write, so it is judged
on two numbers:

1. **Speed.** The Lua-subset interpreter should match or beat PUC-Lua 5.4
   (`-O2`, computed goto) on the standard small benchmarks (fib, binary-trees,
   n-body, spectral-norm, fannkuch, string building). Stretch goal: be within
   a small factor of LuaJIT's *interpreter* (`-joff`), which is hand-written
   assembly.
2. **Size and correctness.** The VM core should take noticeably less code
   than the equivalent C, with no `Protect()`-style manual syncing and no
   manual GC rooting.

Ablation benchmarks are designed in from the start, since each feature claims
a speedup: dispatch mode × value repr × cold outlining on/off × supers on/off ×
quickening on/off.

## Open questions

1. **Effect granularity.** Are `sync ⊂ gc ⊂ reenter` enough? Candidates:
   `alloc` (allocates host memory, may fail) and `throws-host` (for embedders).
   More effects mean more precision but noisier signatures.
2. **Moving GC.** Opt-in reloading after `gc` calls is designed but untested.
   Is a generational *moving* nursery worth the complexity compared with a
   non-moving sticky-mark-bits scheme? Benchmarks should decide.

   **Decided (2026-11-02): non-moving, sticky mark bits**, opt-in per heap
   (`heap.set_generational(true)`); see
   [`stdlib/gc.md`](stdlib/gc.md#generational-mode-sticky-mark-bits-non-moving)
   and [`lowering.md`](lowering.md#10-gc-barriers-and-allocation).
   - *Why not moving.* All three interpreters keep raw pointers into
     objects across safepoints: Mini's table parts (`aptr`, `hptr`) and
     upvalue pointers, MiniJS's `Arr.ptr`, and every interpreter's `k`
     pointer into its constant array. A copying nursery would need each
     of them re-derived after every `gc` call, in handlers too. The
     non-moving scheme needs only a write barrier.
   - *The barrier* is object logging: one header byte test after each
     store of a reference into an object, emitted by the compiler (raw
     pointer stores call `gc.write_barrier`). With generations off it
     costs within noise. Geomean against the commit before the work,
     best of 3, interleaved, `--align-functions=64` (±3% placement
     noise per benchmark): Mini 1.004, MiniJS 1.006. Its worst cases are
     MiniJS `richards` (+3%, property stores) and Mini `closures` (+2%).
     Getting there needed 2 MiB huge-page segments, cache coloring of
     chunk starts, and letting LLVM drop barriers on stores into
     just-allocated objects.
   - *Measured* (same setup, generational vs the old collector): Mini
     binary_trees −21%, MiniJS binary_trees −10%; Mini string_build +6%
     and sieve +3%. On the full suites, geomean with generations against
     without: Mini 0.989, MiniJS 0.996, Scheme 0.993. Scheme is mixed on
     its allocation-heavy programs: mperm −8%, graphs −10%, paraffins
     −12%, but nboyer +6%, earley +9%, string +27%. **Mini and MiniJS
     turn it on, Scheme does not** (until the changes under *Done*
     below). Final, against the old collector:
     Mini 0.996, MiniJS 0.995, Scheme 0.973.
   - *What limited it* (profiled 2026-11-03 with `LOIMI_GC_STATS`,
     which now also prints the remembered set's size and the time in
     deferred sweeps, and page-fault counts). Scheme stores few
     references into old objects (remembered sets of 0-9 objects), so the
     barrier was not the cost. `nboyer`'s live data grows steadily to 42
     MB: generations saved no marking, its last major collection spent
     12 ms of 29 ms clearing marks, and its minor collections swept the
     whole 46 MB nursery (GC time 146 ms against 101 ms without
     generations). `earley` is the same on a smaller scale. `string`
     spent no time in the collector at all: its loss was 4,000 more page
     faults per run (11 ms of system time against 3 ms), because each
     minor collection promoted the current large string, so the old
     generation filled with dead large strings that `free` returned to
     glibc a major collection later, and glibc trimmed and regrew its
     heap (with `GLIBC_TUNABLES=glibc.malloc.top_pad=4194304` the two
     modes ran alike). Mini's `string_build` promotes its `parts` tables
     in use at each minor collection.
   - *Done* (2026-11-03): **epoch marks** (a major collection flips the
     mark value between 1 and 2 instead of clearing every mark),
     **lazy sweeping** in generational heaps (collections leave the
     chunks to sweep pending, and the allocator sweeps each when it takes
     it; a major collection first sweeps only the chunks pending since
     the previous major, whose dead objects may carry the epoch that
     comes back), and **aging for large objects** (promoted at their
     second minor collection; old objects that point to young survivors
     stay in the remembered set with LOG set again, so that an
     interpreter's grouped logging still sees them). `nboyer`'s GC time
     went to 64 ms of marking plus 38 ms of deferred sweeps. Generations
     on against off, best of 3, interleaved, `--align-functions=64`:
     Scheme 0.986 (before 0.996): `string` 1.00 (was 1.21), `nboyer`
     1.02 (1.06), `earley` 1.05 (1.10), `paraffins` 0.84, `mperm` 0.91,
     `graphs` 0.94, `gcbench` 0.95. **Scheme turns generations on.**
     Mini 0.982 (before 0.988; `binary_trees` 0.76, `string_build`
     still 1.05: its `parts` tables are small objects), MiniJS 0.987
     (0.985). With generations on, against the previous generational
     collector: Mini 0.996, MiniJS 1.003, Scheme 0.998, all within
     noise except Mini `binary_trees` 0.94 and MiniJS `binary_trees`
     0.95 (best of 7). Without generations: Mini 1.002, MiniJS 1.000,
     Scheme 1.008 (non-allocating Scheme benchmarks moved up to 8% either
     way, placement).
   - *Measured and not kept: aging small objects.* Keeping every first
     survivor young (same mechanism, all sizes) fixed `string` too, but
     traced the survivors of programs with growing live data twice:
     `nboyer` 0.798 s against 0.762 s without aging,
     `earley` +9%, `mperm` +7%. Small objects are promoted at their
     first survival.
   - *Left open*: card marking for large arrays (stored into, they are
     retraced whole by each minor collection), lazy sweeping in
     non-generational heaps (it needs the epochs, and a flip at every
     collection), and aging that targets objects in use at a minor
     collection (by allocation time, for example) rather than all of
     them, for `string_build`-like programs.
3. **Variable-length encodings and superinstructions.** Reusing words for
   fusion (language.md §7.3) is clean for fixed widths. For byte streams it
   needs either padding or "fused only at build time". Which one?
4. **Threads.** The plan is a GIL-style "one thread in the VM per heap", with
   atomic opcode rewrites under `#[threads(shared_code)]`. Free-threaded guest
   languages need a story for inline caches, which CPython's free-threading
   work could inform.

   **First step done (2026-11-03): independent states on different threads.**
   An audit of every piece of process-global mutable state (language.md
   §10.1, Threads, has the table) found the runtime already per thread
   (initial-exec TLS for the shadow-stack top, `Vm.depth`, vm scratch
   slots, the panic handler chain and message) and `std.gc` free of
   globals. Making them plain globals instead measures as noise
   (geometric means 0.99–1.01 on Mini, MiniJS and Scheme), so there is no
   single-threaded build mode. `examples/embed/threads.c` runs four Mini
   states on four threads (GC stress, nested runs, panics caught on every
   thread, states handed between threads); `--sanitize=thread` was added
   and reports no race. The audit found one real bug, not about threads
   but about several states on one thread: the collector's walk of the
   (per-thread) shadow stack marked another heap's objects during a
   nested run into another state, so that heap's next collection could
   free live objects; the walk now marks only its own heap's objects.
   Still single-threaded by design: `--vm-stats` counters and the
   copy-and-patch JIT. What remains is everything shared: several
   threads in one heap (a GIL handing the heap over at safepoints, a
   per-thread shadow stack registered with the heap, allocation buffers
   per thread), code and inline caches shared between threads
   (`#[threads(shared_code)]`), and passing values between heaps without
   copying.
5. **`preserve_none` coverage.** Confirmed on x86-64 and AArch64 (LLVM 19+).
   AArch64 Linux is now run, not just compiled: `--target=aarch64-linux-gnu`
   builds pass the whole test suite under qemu user-mode (all dispatch
   modes, GC stress; the JIT is x86-64 only), and the tail-call handlers
   keep the vm state in registers with no spills on hot paths; the
   dispatch table's address is one of those registers, so a dispatch is
   `ldr`, `and`, `ldr`, `br` with no `adrp`+`add` (lowering.md §3). Not yet run
   on hardware or with 16K/64K pages (qemu-user uses the host's 4K).
   Windows x64 and RISC-V status need checking per LLVM version. The `goto`
   fallback should make this a performance question, not a correctness one.
6. **Copy-and-patch JIT.** Handlers are separate functions with a fixed
   register interface, which is close to what a copy-and-patch template
   compiler needs. Can `vm` declarations generate a baseline JIT with no
   additional source? This could be the most compelling post-1.0 feature.

   **Answered by a prototype (2026-11-02): yes, for a 22% gain.** See
   [`design/copy-and-patch.md`](design/copy-and-patch.md).
   - *What it does.* `loimi build --jit-stencils` (run with
     `LOIMI_JIT=1`; x86-64 Linux) emits every handler a second time as a
     stencil: operand fields, the instruction word and `next`/label
     targets become holes. It compiles the stencils with clang, reads
     the relocations back with a small ELF reader in the compiler, and
     embeds them as data. A C runtime copies and patches whole code
     objects when hot counters fire. It then rewrites their instruction
     words to an enter opcode, so the interpreter's own dispatch enters
     compiled code. Compiled code shares the interpreter's state
     registers, so exits (cold paths, ops without stencils, calls into
     uncompiled code) are ordinary dispatch, and need no deoptimization
     metadata. Mini's source is unchanged. All 113 ops and supers get
     stencils, and every Mini test passes with the JIT forced on.
   - *Measured.* Against Mini's interpreter: fib 0.68, sieve 0.89,
     mandelbrot 0.59, and 0.78 by geometric mean over the 14 benchmarks.
     It is faster than LuaJIT `-joff` on all 14 (0.66 by geometric
     mean), but 2.3× slower than LuaJIT with its JIT. It compiles at
     7–30 µs per function. Default builds are unchanged.
   - *Phase 2: all three interpreters.* MiniJS and Scheme build and pass
     all their tests with the JIT forced on, with no change to their
     source. JIT against interpreter, by geometric mean: Mini 0.77,
     MiniJS 0.85, Scheme 0.79 (Scheme with the JIT is 0.80 of Guile 3's
     time with its JIT). A JIT build with the JIT off now runs the
     default build's handlers (counting copies are swapped in only with
     `LOIMI_JIT=1`) and is within noise of a default build (was +5%).
     Stencil builds take a third less time.
   - *Where the time goes.* Dispatch and operand decoding were 20–40% of
     handler-bound time even after supers and quickening, and stencils
     remove most of it. Superinstructions needed stencils of their own:
     split into parts, nbody was 4% slower than the interpreter. What
     remains (values through the register file in memory, type checks,
     call overhead) needs register allocation and type specialization
     across instructions, not templates.
   - *Phase 3: quickening and stencil layout.* A rewrite of compiled
     code (`become`, `Vm.optimize`, `Vm.deopt`) no longer drops the code
     object. The runtime recompiles it at once, or points the rewritten
     instruction at code it has, through a 5-byte prefix that old code
     also passes. Members therefore need not be complete. Splitting
     stencils into hot and cold parts, so that fast paths fall through,
     is optional (`LOIMI_JIT_SPLIT=1`). Neither is faster: the
     invalidations cost little, and the new scheme costs about 1% (Mini
     0.78, MiniJS 0.86, Scheme 0.78).
   - *Phase 4: constant operands.* Operand kinds take a `const` clause
     (`operand kst = k[it] const proto.consts.data.ptr verify ..`): what
     they read cannot change after verification, which the checker
     enforces within the vm. The interpreter is unchanged. The JIT
     patches each constant in as a 64-bit immediate, and picks stencil
     variants per tag of the constant, generated from the handler with
     the tag known so LLVM folds its checks. All three interpreters opt
     in. No measurable gain (Mini 0.996 of the JIT's time before,
     MiniJS 0.998): constant loads and their type checks were never
     on the critical path. Phase 3's 1% was not measurable either.
   - *Guest registers in machine registers.* An upper bound by hand:
     mandelbrot's inner loop, copied from the JIT and rewritten to keep
     its values in host registers, takes 0.68 of the time (0.79 with
     only the previous result in a register). A latency probe
     (`LOIMI_JIT_REGDELAY`) projects at most 12% by geometric mean over
     the 14 benchmarks, much of it call arguments that a cache cannot
     hold. A one-register write-through cache with stencil variants
     (kept on branch `jit-regcache-experiment`, not merged) is correct but 1.5% slower by geometric mean
     (mandelbrot 0.91).
   - *Recommendation: stop the template-JIT line.* Keep the JIT as it
     is, an opt-in tier at about 0.8 of the interpreter's time, with no
     more speed phases. Phases 3, 4 and the register cache all gained
     nothing; what remains needs register allocation over loops,
     hoisted guards and a register calling convention, which is an
     optimizing tier with deoptimization. If it is ever made a supported
     feature, it needs freeing compiled code, several vms per program and
     variants, and AArch64 and PIC/Mach-O relocations.
7. **How unsafe is `Window`?** A register window is one pointer with no
   length. Operand access is verified, but computed indices
   (`regs.at(f.ret_reg)`) need `unsafe`. Is a "trusted index" type flowing
   from verified operands worth it?
8. **Syntax.** Everything is provisional. Control words (`next`, `jump`,
   `become`, `redo`) read well, but `inline for` generating items and `#name`
   splicing may want a proper macro design.
9. **Embedding API.** What the generated C header should look like (opaque
   `Interp*`, value handles, error codes), and whether a C++ wrapper is generated.

   *Decided (first version):* the API is whatever the interpreter writes as
   `export "C"` functions; `loimi build --lib`/`--shared` builds the library
   and `--header` declares them (`language.md` §10). Interpreter state is an
   opaque struct pointer (`MiniState *`), error codes are payload-less enums
   (a typedef plus `E_Variant` constants), strings are `loimi_str`
   (pointer and length). There are no value handles: `Gc<T>` and `value`
   types cannot cross into C, so hosts exchange numbers and strings through
   accessors (`mini_get_number`, `mini_arg_string`), and refer to other
   values by position on a rooted value stack (`mini_push_global`,
   `mini_call`), as Lua's stack API does. Natives may call back into the
   interpreter through nested entries (`uses reenter`, `language.md` §10).
   No C++ wrapper: the header is `extern "C"`-guarded and usable
   from C++. Still open: `const` pointers (Loimi has none, so `*u8` is
   `void *` and C passes `const` data with a cast), and generated
   C-ABI entry points.
10. **Immediates wider than a format's word.** The wasm study
    (`examples/wasm/FRICTION.md` item 4) needed a 64-bit constant, a
    32-bit memory offset next to two 16-bit slots, and table/segment
    indices next to three operands, in a `u64` word whose fields were
    already full. It used an inline cache layout (`cache Imm64 { v: u64 }`)
    as an immediate word, written after `emit` zeroed it. That works and
    disassembles, but the verifier cannot see cache words, so indices
    stored there are bounds-checked at run time in every execution, which
    is what operand kinds exist to avoid. Options:
    - *Trailing immediate words in formats*:
      `format RRRX { op: 16, a: 16, b: 16, c: 16 } + imm: u64` (or
      `+ imm: s32` packed into one word). The trailing fields map to
      operands by position like other fields, take operand kinds and
      `verify` clauses (checked once by `Vm.verify`), are encoder and
      emitter parameters, and print in the disassembly. `op_size` grows by
      the trailing words, before any cache words; `next`/`next_pc` skip
      them. Handlers read them `pc`-relative, as they read cache fields.
      This is the smallest change and covers every case in the study.
      Open: whether a trailing field may be a label (`pc + it`, relaxed by
      `std.bc`), and how supers whose parts have trailing words lay them
      out (as today for caches: each part keeps its words).
    - *Immutable cache fields*: `cache Imm64 { const v: u64 }`: the
      verifier may read and check `const` fields (they are set by the
      emitter, never by handlers), and `deopt` does not zero them. It
      reuses the cache machinery, but mixes two ideas (run-time state
      and instruction operands) in one construct, and the emitter would
      need parameters for them.
    - *`encoding bytes`* (variable-length instructions, not built) would
      make any field width possible, at the cost of byte-stream decoding
      and the questions in item 3.
    - *Wider words* (`encoding fixed(u128)`): no new concept, but every
      instruction pays for the widest one.

    The first is the likely answer; nothing is built yet.
11. **The cost of a three-operand op.** The wasm interpreter's float
    benchmarks (`examples/wasm/FRICTION.md`, "Float benchmarks against
    wasm3") run 30–40% fewer dispatches than wasm3 but trailed it by
    20–25%, because each dispatch costs more: `F64_MUL d, a, b` is 14
    instructions (6 to decode three 16-bit fields from the word, 3 for
    load, multiply and store, 5 to dispatch: load the next word, advance
    `pc`, extract the opcode, load the handler, jump), where wasm3's
    `op_f64_Multiply_rs` is 5 (the operand's slot offset is a word of its
    own, the other operand and the result are `xmm0`, and the next
    handler's address is in the code stream). `#[forward]` supers close
    most of the gap on float chains, since a forwarded value skips the
    store-to-load round trip. Directions:
    - *built*: accumulators (language.md §7.4): `accumulator facc: f64
      for rd;` is a state register (`xmm0`), `#[accumulate]` derives
      each op's "result to the accumulator" and "operand from it"
      variants, and `Vm.accumulate` selects them where a temporary goes
      from one op to the next. The wasm interpreter got 0.91 of its time
      (geometric mean of seven programs; 0.87 of wasm3's), and 0.79 with
      supers over the variants (`examples/wasm/FRICTION.md`). Open: the
      goto-mode build's memory grows faster than the op count (the wasm
      vm marks 90 ops, not all), and selection is a greedy pass over
      adjacent pairs;
    - not built: field decoding: three `mov`/`shr`/`movzwl` sequences per op; a
      format whose fields are pre-scaled byte offsets would drop the
      `movzwl` and the scaled addressing, and reading fields with narrow
      loads from `pc` (tried by reloading the word: LLVM does not narrow a
      load with several uses) would trade ALU work for loads;
    - not built: direct threading (handler addresses in the code words) for `u64`
      words, which saves the table load and the opcode extraction.
