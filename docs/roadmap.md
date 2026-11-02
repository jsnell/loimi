# Roadmap and open questions

## Implementation plan

Progress (2026-11-02):
- M0–M5 are done. The bytecode fuzzer (ASan, about 2 hours total) found
  and fixed verifier and Mini holes; `cargo test` runs a smoke version.
  `Verified<V>` is built: `verify` returns the proof (or a diagnosis),
  `#[require_verified]` entries accept only it, and the three
  interpreters keep it in their prototypes, so calls reach only verified
  code. Open: tying the proof to its verify context without a per-call
  load (Mini's CALL pays one, up to 4% on call-heavy code; a proof
  that is a typed view of the prototype, with the code at a fixed
  offset, would cost nothing), and checking handler-table targets.
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
  - closures (non-escaping `block fn`) and traits;
  - C embedding (`--lib`, `--shared`, `--header`) with re-entrant natives;
  - a generational, non-moving GC with a write barrier and a verify mode;
  - weak references and ephemerons;
  - a generated disassembler;
  - a tutorial (`docs/tutorial/`). See
  [`bench/mini/RESULTS.md`](../bench/mini/RESULTS.md).

See [`status.md`](status.md) for details.

The compiler starts in **Rust**, emitting LLVM IR through the LLVM C API
(`inkwell` or `llvm-sys`), pinned to one LLVM major version (≥ 19). Self-hosting
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
     turn it on, Scheme does not.** Final, against the old collector:
     Mini 0.996, MiniJS 0.995, Scheme 0.973.
   - *What limits it.* Every minor survivor is promoted (no aging), so
     objects in use at a minor collection fill the old generation until
     the next major; a small nursery made this worse than no generations,
     so the nursery is the full growth budget. Sweeping is eager. Minor
     collections sweep only the chunks allocated from, but that is still
     every chunk the nursery passed through. Majors pay an extra pass to
     clear the sticky marks.
   - *Left open*: aging (which needs remembered-set entries kept while
     they point at young survivors), card marking for large arrays
     (stored into, they are retraced whole by each minor collection),
     lazy sweeping, and epoch marks instead of the clearing pass.
3. **Variable-length encodings and superinstructions.** Reusing words for
   fusion (language.md §7.3) is clean for fixed widths. For byte streams it
   needs either padding or "fused only at build time". Which one?
4. **Threads.** The plan is a GIL-style "one thread in the VM per heap", with
   atomic opcode rewrites under `#[threads(shared_code)]`. Free-threaded guest
   languages need a story for inline caches, which CPython's free-threading
   work could inform.
5. **`preserve_none` coverage.** Confirmed on x86-64 and AArch64 (LLVM 19+).
   Windows x64 and RISC-V status need checking per LLVM version. The `goto`
   fallback should make this a performance question, not a correctness one.
6. **Copy-and-patch JIT.** Handlers are separate functions with a fixed
   register interface, which is close to what a copy-and-patch template
   compiler needs. Can `vm` declarations generate a baseline JIT with no
   additional source? This could be the most compelling post-1.0 feature.
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
