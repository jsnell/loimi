# Roadmap and open questions

## Implementation plan

Progress (2026-11-02):
- M0–M5 are done. M5's fuzzing criterion is in progress.
- M6 is done: std.num, std.diag, std.lex, std.parse, std.bc and std.obj
  exist, and the example interpreters use them. Mini's front end is
  std.lex + std.parse (Pratt driver, `led` hooks for `and`/`or`/`..`) +
  std.diag, its code buffer, labels, constant pool and registers std.bc;
  Scheme's reader is std.lex + std.num and its code generator std.bc;
  MiniJS uses all six. Mini and Scheme emit byte-identical bytecode to
  their hand-written versions. std.obj is used by MiniJS only: Lua tables
  and Scheme records have no shape-based objects to replace (see
  [`design/adoption/README.md`](design/adoption/README.md#stdobj)).
- M7 has two proof interpreters, `examples/mini` (a Lua 5.4 subset) and
  `examples/scheme` (an R7RS-small subset).
- **Speed criterion 1 is met.** Mini beats PUC-Lua 5.4 on all 14
  clean-room benchmarks, by 0.36–0.76× of Lua's time.
- **The stretch goal is met.** Mini is faster than LuaJIT's interpreter
  (`-joff`) on 11 of the 14, and within 8% on the rest.
- MiniJS beats `node --jitless` and QuickJS on all 14 of its clean-room
  benchmarks. See
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
   accessors (`mini_get_number`, `mini_arg_string`), as Lua's stack API
   does. No C++ wrapper: the header is `extern "C"`-guarded and usable
   from C++. Still open: value handles that the GC knows about (a handle
   table scanned as a root), `const` pointers (Loimi has none, so `*u8` is
   `void *` and C passes `const` data with a cast), generated
   `Vm.call`-style entry points, and re-entry from natives (below and
   `language.md` §10).
