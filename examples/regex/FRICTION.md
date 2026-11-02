# Friction report: a regex engine as a Loimi C library

A usability study. I built a regex engine (backtracking vm + Pike VM)
shipped as a C library, reading only README.md, docs/, std/,
examples/tutorial, examples/embed and tests/run + tests/fail. I did not
read src/ or the forbidden examples, and never needed to (see item 17).

**Result:** both engines agree with Python's `re` on every case tried,
captures included: about 30,000 random patterns × subjects across seeds,
plus 80 hand-written cases. That held in all three dispatch modes, under
ASan, with a bytecode fuzzer and with a byte-pattern fuzzer. See SPEC.md.

## Summary: top items

| # | Item | Area | Fix kind |
|---|---|---|---|
| 1 | A panic in library code (bounds check, `unwrap`, OOM) aborts the C host | embedding | runtime + doc |
| 2 | `--sanitize=address` refuses `--lib`/`--shared` | embedding/tooling | tooling + doc |
| 3 | `--vm-stats --lib` fails (clang `-o` error; missing `loimi_stats_seq_report`) | tooling bug | bug |
| 4 | Header: no constants, no doc comments, `void *` without const, no C name prefix for opaque types | embedding | header generator |
| 5 | No generated op enum or decoder for host tools: a second engine over the same bytecode is an if-chain that silently ignores new ops | vm | std/generated API |
| 6 | `!x` is bitwise NOT on integers, but that isn't documented; `~x` gives a parse error with no hint | language/docs | doc + diagnostic |
| 7 | `jump (expr)` is parsed as a call to a function named `jump` | diagnostics | parser/diagnostic |
| 8 | `*T` can't be null and `Vec.new()` allocates, so a struct needs placeholder allocations, which leak when overwritten | std/language | std |
| 9 | `--vm-stats` suggests supers that overlap each other, so its own suggestions set off `super_overlap` warnings | tooling | tooling |
| 10 | `a ?? b` with `b: ?T` is a type error: there is no `or_else` for options | language | std/language |

Details follow, in priority order. Times are this agent's wall-clock
time. A human would take longer on each, though in the same proportions.

---

### 1. A panic inside the library kills the host process (embedding, high)

- **Trying to:** decide what a C caller sees if library code hits a
  bounds check, `unwrap()` on None, `trap()`, or `mem.alloc` running out
  of memory.
- **What happened:** a test library `export "C" fn other_oob(i: u64) -> u8 { a[..][i] }`
  called with `i = 9` printed `other.lm:16:5: index out of bounds` on
  stderr and the host died with SIGABRT (exit 134). `std.rt.protect`
  catches only `rt.throw()`, not panics. `mem.alloc` "aborts when out
  of memory" (stdlib.md).
- **Expected:** a library can turn a failure inside one call into an
  error return, as `rt.protect` already does for `throw`. At the least,
  §10.1 should say plainly that panics abort the host.
- **Cost:** about 5 min to establish. In the regex library I designed
  around it: every slice index is guarded or is in verified handlers,
  and the backtrack stack has an explicit limit with an error return
  (`backtrack limit exceeded`) instead of growing until `mem.alloc` aborts.
- **Suggested fix:** let `rt.protect` also catch panics (it already uses
  setjmp/longjmp and restores the shadow stack), or add
  `#[catch_panics(ret = -1)]` on `export "C"` functions, or a
  `loimi_set_panic_hook`. Add `mem.try_alloc -> ?*T`. Document the
  current behavior in language.md §10.1.

### 2. ASan for a library: `--sanitize=address builds executables only` (embedding/tooling, high)

- **Trying to:** run the C test host against the library under ASan.
- **What happened:** `loimi build lib.lm --lib --sanitize=address` gave
  `--sanitize=address builds executables only`. Workaround:
  `loimi build --lib --emit-llvm -o rx.ll`, then
  `clang-19 -fsanitize=address -c rx.ll` (with the
  `-Wno-override-module` warning), then link with **gcc**:
  `clang-19 -fsanitize=address` failed with
  `cannot find /usr/lib/llvm-19/lib/clang/19/lib/x86_64-pc-linux-gnu/libclang_rt.asan_static.a`,
  since only gcc's libasan is installed. The docs don't say which ASan
  runtime Loimi's own `--sanitize` uses.
- **Expected:** `--lib --sanitize=address` gives an instrumented archive,
  and the docs show how to link it.
- **Cost:** about 6 min. It found real leaks in my code (item 8).
- **Suggested fix:** allow `--sanitize=address` with `--lib`/`--shared`.
  Document the link line (`gcc -fsanitize=address host.c librx.a`), and
  document that `--emit-llvm` with `--lib` gives IR you can instrument
  yourself.

### 3. `--vm-stats` doesn't work for a library (tooling bug, high for this use)

- **Trying to:** profile op pairs while the real C host runs the
  benchmarks.
- **What happened:** `loimi build lib.lm --release --lib --vm-stats -o librx_stats.a`
  gave `clang-19: error: cannot specify -o when generating multiple output files`
  and `error: clang-19 failed on generated IR (kept at /tmp/loimi-2073955/lib.ll)`.
  With `--emit-llvm` the IR linked into the host failed with
  `undefined reference to 'loimi_stats_seq_report'`.
- **Expected:** a stats build of the library prints at exit (atexit), or
  exports a `loimi_vm_stats_report()` for the host to call.
- **Cost:** about 8 min, including writing `stats.lm`, a Loimi
  executable that repeats the C benchmark so that `--vm-stats` can run.
  That is a second copy of the benchmark to keep in sync.
- **Suggested fix:** fix the clang invocation for `--lib --vm-stats` and
  link the stats runtime into library builds.

### 4. The generated header is minimal (embedding, medium)

- **Trying to:** give C callers a usable `rx.h`.
- **What happened:**
  - **No constants.** `rx_exec` takes flags. A payload-free enum reaches
    the header only if an export's signature uses it, so `host.c` has
    its own `#define RX_SEARCH 1u` / `RX_PIKE 2u`, which can drift from
    `lib.lm`.
  - **No documentation.** The `///` comments on exports (ownership,
    return codes, lifetimes) are not copied into the header.
  - **No const.** `*u8` becomes `void *`, so C callers holding
    `const char *` need casts (`rx_compile((void *)pat, n)`). The Mini
    host does the same.
  - **No C name prefix.** The opaque type's C name is the Loimi struct
    name: `typedef struct Regex Regex;` can collide with a host's own
    `Regex`. I would have to rename the Loimi struct to `rx_regex` to
    get a C-style name.
- **Expected:** `export "C" const RX_SEARCH: u32 = 1;`, doc comments in
  the header, a way to mark read-only pointer parameters (`*const u8` →
  `const void *`), and `#[c_name("rx_regex")]` on structs.
- **Cost:** about 3 min. Low cost, but every library will hit it.
- **What worked well:** `str` ↔ `loimi_str`, `[]u8` → `loimi_str`,
  `?*T` → nullable `T *`, `bool`, and `int64_t *` for the spans
  out-parameter all mapped as expected. The archive exports exactly the
  8 `rx_*` symbols. A second Loimi library linked into the same host
  caused no symbol clashes. 8 threads, each with its own regex, ran
  correctly. `--shared` exports only `rx_*`.

### 5. No typed decoder for host tools over a vm's bytecode (vm, medium)

- **Trying to:** write the Pike VM, a second interpreter over the same
  bytecode. Handlers can only express the backtracking semantics, so
  the Pike VM is host code.
- **What happened:** the generated readers make it possible
  (`Rx.opcode(w)`, `Rx.A_a(w)`, `Rx.AJ_a(w)`, `Rx.label(w) -> ?i64`),
  but the dispatch is an if-chain over `Rx.OP_*` constants. When I
  later added `UNMARK` and `NOPROG` to the vm, the Pike VM's final
  `else` (consuming ops) took them silently, and only the differential
  tests caught it. `Rx.label(w)` returns an option even for an op whose
  format always has a label, so there is a `trap()` on None.
- **Expected:** a generated `Rx.Op` enum (or `Rx.decode(w) -> Rx.Insn`
  with typed operands) that a `match` can be exhaustive over. Then
  adding an op breaks host tools at compile time.
- **Cost:** about 4 min of debugging, plus the risk.
- **Suggested fix:** generate `enum Vm.Op { CHAR, ANY, .. }` and
  `Vm.op_of(w) -> ?Vm.Op` (None for invalid or super opcodes, which a
  tool can map with `base_op`).

### 6. Bitwise NOT is `!`, which isn't documented; `~` is a parse error (language/docs, medium)

- **Trying to:** invert a 256-bit class set (`w = ~w`).
- **What happened:** `error: expected expression, found '~'`. The
  language reference has no operator table. I used
  `w ^ 0xFFFFFFFFFFFFFFFF`. Later, testing by hand, I found that `!x`
  on a `u64` is bitwise NOT (`!5 as i64` prints `-6`), which no
  document says.
- **Cost:** 2 min (more for someone who doesn't think to try `!`).
- **Suggested fix:** add an operator table to language.md §1.2 (and say
  that `!` is bitwise on integers). On `~`, say "use `!x` for bitwise
  not".

### 7. `jump (expr)` is parsed as a call (diagnostics, medium)

- **Trying to:** resume a backtrack record: `jump (t as *u32)`.
- **What happened:** `error: unknown name 'jump'`, pointing at `jump`.
- **Expected:** control words take any expression, or the message says
  that `jump (...)` reads as a function call and suggests binding the
  target first.
- **Cost:** 1 min. I wrote `let resume = t as *u32; jump resume`.
  `exit (..)`, `raise (..)` and `fail (..)` probably behave the same
  way; I didn't check.

### 8. Non-null pointers and allocating `Vec.new()` make placeholder state leak (std, medium)

- **Trying to:** create the `Regex` object before compiling, so that
  `rx_compile` can always return one, carrying an error if compilation
  failed.
- **What happened:** every `*T` field needs a value (no null; `..zero`
  is not allowed for pointers or for `Vec`), and `mem.Vec.new()`
  allocates one element. So `empty_regex()` allocates placeholders, and
  assigning the real vectors over them leaked. LeakSanitizer reported
  `57 byte(s) leaked in 9 allocation(s)` in `rx_compile`. The fix was
  `free()` before each overwrite, plus a hack in `rx_free` to tell a
  placeholder `marks` pointer from a real one.
- **Expected:** `Vec.new()` doesn't allocate (as in Rust), so an empty
  `Vec` is free to create and to drop, or `..zero` works for `Vec`.
- **Cost:** about 5 min after ASan pointed at it.
- **Suggested fix (std):** make `Vec.new()` start with `cap: 0`, using a
  static sentinel pointer, and have `free()` skip it. Alternatively,
  document the pattern for objects with optional resources (use `?*T`).

### 9. `--vm-stats` suggestions overlap each other (tooling, low)

- **What happened:** I pasted five of its suggestions (`SAVE_SPLIT_CPLUS_LIT`,
  `CPLUS_SAVE`, `SAVE_CPLUS`, `SAVE_MATCH`, `SPLIT_LIT`) and got three
  `warning[super_overlap]`s, for example
  ``superinstruction `CPLUS_SAVE` overlaps `SAVE_SPLIT_CPLUS_LIT` ... `CPLUS_SAVE` cannot run there``.
  The warnings themselves are excellent: they name the sequence and say
  which super `optimize` keeps.
- **Expected:** the suggested set is overlap-free, or it marks which
  suggestions conflict.
- **Cost:** 2 min.

### 10. `??` takes only a plain default (language, low)

- `required(t, b) ?? required(t, a)` failed with
  `mismatched types: expected 'u8', found '?u8'`. I wrote
  `if let Some(x) = .. { Some(x) } else { .. }`. An `o.or(p)` /
  `or_else` for options, or `??` accepting `?T` on the right, would fix
  it. 1 min.

### 11. Smaller items (low)

- `var res = Ok(false);` → `cannot infer the type of 'Ok'`. This is
  documented ("inference is local"), but `var` followed by an
  assignment is a common pattern. Annotating fixed it.
- `in` is a keyword: `state { .., in: *u8 }` →
  `expected identifier, found 'in'`. A note ("`in` is reserved") would
  help.
- std gaps I filled with `extern "C"` or by hand: `memchr`/`memcmp` on
  raw pointers (`text.find` covers only `str`), and `HashMap.clear()`
  (I stored a generation number as the value instead).
- The generated encoder `Rx.CHAR(c: u32)` for a 24-bit field accepts
  300 and masks it. The verifier then rejects it with a precise message
  (``operand `c` (byte) fails `it < 256` ``), so this was fine.
- An operand of struct type (`cls = ctx.classes[it]`) reads as a value.
  Calling a method on it works, and passing it to a `*Set` parameter
  needs `&s`. Again fine: the error said `expected '*Set', found 'Set'`.

### 12. Docs coverage of the C path (finding, low)

- The Loimi half of the embedding example, `examples/mini/embed.lm`,
  lives in a directory this study excluded. `examples/embed/` has only
  the C side, and `tests/embed/api.lm` isn't linked from the docs.
  **I didn't need any of them:** language.md §10.1 (the type table, the
  `str` register rule, TLS and PIC, "no initialization call") was
  enough to write the API first time. What I missed was a 20-line
  "opaque handle + error string + out-array" example in the docs
  themselves, plus a statement of what happens on panic (item 1).
- No `.lm` ↔ C example shows returning memory the C side must free. I
  avoided the question: strings returned by the library live in the
  `Regex` until the next call.

## GC interaction

None. The library uses only `std.mem`, never `std.gc`, so there are no
roots, barriers or effects to manage, and nothing about the GC is
visible in the C API. That the vm, the verifier, std.bc and the
embedding path all work without the GC is a plus. A regex engine
doesn't need it.

## Numbers

**Lines of code** (`wc -l`, comments included):

| file | lines | |
|---|---:|---|
| syntax.lm | 310 | parser |
| compile.lm | 285 | compiler, first-byte set, required byte |
| vm.lm | 307 | the Rx vm (17 ops, 4 supers, 1 variant, 2 entries) |
| pike.lm | 217 | Pike VM |
| lib.lm | 250 | C API |
| fuzz.lm, verify_test.lm, stats.lm | 197 | bytecode fuzzer, verifier test, stats driver |
| host.c | 343 | C host: demo, tests, fuzz, benchmarks (with PCRE2) |
| tests/gen.py, tests/compare.py, bench.py, check.sh, dispatch.sh | 277 | |

That is 1,369 lines of Loimi for the engine and its API, and 2,186 in
total.

**Time per phase** (agent wall-clock, from the commit log):

| phase | time |
|---|---|
| reading the docs (README, language.md, tutorial ch. 3, stdlib.md, std/bc.lm, examples/embed) | 6 min |
| parser, vm, compiler, Pike VM, C API, host (first build after 3 compile errors) | 8 min |
| differential tests vs Python, first benchmarks | 5 min |
| ASan path, `--vm-stats` path, CSTAR/CPLUS, supers | 6 min |
| matching sre's empty-iteration rules exactly (backtracker and Pike VM) | 5 min |
| verifier test, bytecode fuzzer, step-limit variant, dispatch modes, `--shared`, threads | 7 min |
| SPEC.md, this report | 10 min |

Most of the debugging time went into regex semantics (sre's rules for
empty loop iterations, Pike VM deduplication), not into Loimi.

**Benchmarks.** 4 MiB of generated text (the same generator in `host.c`
and `bench.py`). Each benchmark counts the non-overlapping matches from
C through the library (`rx_exec` in a loop). Best of 3. i7-6700, Python
3.13.5, PCRE2 10.46 through its C API (interpreter and JIT). `grep -P`
was not used, since the PCRE2 library times the same engine without
grep's output.

| benchmark | pattern | matches | rx backtrack | rx Pike VM | Python `re` | PCRE2 | PCRE2 JIT |
|---|---|---:|---:|---:|---:|---:|---:|
| literal | `Sherlock Holmes` | 1,635 | 3.1 ms | 15.3 ms | 2.4 ms | 2.6 ms | 0.8 ms |
| literal-alt | `Sherlock\|Watson\|Holmes\|1887` | 145,514 | 7.9 ms | 74 ms | 36 ms | 17.7 ms | 9.4 ms |
| class | `[a-z]+ing\|\d{3,}` | 36,133 | 57 ms | 327 ms | 149 ms | 220 ms | 17.0 ms |
| words | `\w+` | 729,317 | 17.4 ms | 185 ms | 153 ms | 53 ms | 24.4 ms |
| capture | `(\w+)\s+(\w+)` | 364,658 | 15.8 ms | 309 ms | 106 ms | 49 ms | 17.0 ms |
| alt-heavy | `(?:fox\|dog\|cat\|bird\|lazy)\s(?:over\|jumps\|end)` | 16,335 | 22.8 ms | 135 ms | 31 ms | 48.6 ms | 11.7 ms |
| dot-star | `x9.*end` | 8,495 | 9.7 ms | 88 ms | 7.9 ms | 21.4 ms | 4.2 ms |
| pathological | `(a*)*b` on a²² | 0 | 0.0 ms¹ (138 ms²) | 0.0 ms | 417 ms | 0.0 ms¹ | 0.0 ms¹ |
| patho-alt | `(a\|aa)+$` on a²⁸b | 0 | 42 ms | 0.0 ms | 119 ms | 152 ms | 11.3 ms |

¹ The required-byte check (`b` never occurs) rejects the input before
matching, as in PCRE. ² `(a*)*[b]` hides the required byte, so the
backtracker does the exponential work. Python takes 418 ms on the same
input.

The backtracker beats Python's `re` and the PCRE2 interpreter on every
benchmark except `literal` and `dot-star`, where it is within 30% of
Python. It is within about 2× of PCRE2's JIT, and ahead of it on
`words` and `literal-alt`. The Pike VM is a plain host-code loop with
no fast paths: it is linear but 5-20× slower, and fixes the
pathological cases. The three dispatch modes give the same results.
`goto` and `switch` are 0-10% slower than `tailcall`, except on
`patho-alt`, where `switch` is 30-60% slower across runs (`dispatch.sh`). A step limit
(`rx_set_step_limit`, the `budget` variant) costs 3-15% when set and
nothing when not.

## What I liked

- **It compiled almost first time.** About 1,000 lines of new Loimi built
  after three one-line fixes (`in`, `~`, `jump (..)`), and the first run
  gave correct matches. Diagnostics point at the right token, with
  carets.
- **Declaring the vm paid for itself.** Encoders, the disassembler
  (`rx_disasm` is three lines), `Rx.verify` and `Rx.optimize` were free.
  The verifier's messages are precise: ``word 1 (LOOP): operand `m`
  (mark) fails `(it as u32) < lim.nmarks` ``. `Verified<Rx>`,
  `#[require_verified]` and `Rx.placeholder()` made "only verified code
  runs" a type-level fact with no ceremony. The bytecode fuzzer (about
  80 lines) ran 20,000 mutated programs under ASan cleanly.
- **The code is good.** The CLASS handler is 17 instructions: bounds
  test, bit test, `pc` advance, the next dispatch load, and an indirect
  jump. There are no spills, and the fail path is a direct `jae` to the
  shared `backtrack` function.
- **`-> never` vm functions** (`backtrack`, `class_star`, `grow_stack`)
  were exactly right for a backtracking matcher. Each is one shared
  copy, called from every op's failure path, that can `jump` to a saved
  pc, `exit`, `fail` or `redo`.
- **`std.bc`** labels and fixups made the compiler short: the
  alternation, star and repeat layouts are a few lines each.
- **`--vm-stats` → `super` → `Rx.optimize`** took two minutes, and the
  `super_overlap` lint explained exactly what each conflict meant.
- **Instrumentation variants:** a per-match step limit that costs nothing
  when off, added in 15 lines.
- **The embedding path:** no initialization call, a clean symbol table,
  `str` as `loimi_str`, both `--lib` and `--shared` work, there are no
  clashes between two Loimi libraries, and threads are safe. The
  documented rules (str argument registers, TLS models) matched what I
  saw.
- **Builds are fast:** 0.9 s for the release library.
