# Lox in Loimi: a usability study

This is a report on writing a Lox bytecode VM in Loimi. Lox is the
language of Bob Nystrom's *Crafting Interpreters*. The report is for
people improving Loimi's usability, or curious how usable it is.

A first-time user (an AI agent) wrote the VM from Loimi's README,
`docs/`, `std/` and the tutorial. It was compared with the book's C
implementation, clox, as a black box: clox's tests and benchmarks were
used, but its C source was not read.

```sh
bench/lox/install.sh                         # fetch and build clox, its tests and benchmarks
loimi build examples/lox/main.lm --release -o lox
bench/lox/runtests.py ./lox                  # the book's test suite
bench/lox/compare.py -n 5 lox=./lox          # the book's benchmarks against clox
loimi test examples/lox/lox_test.lm          # unit tests
./lox FILE.lox [--disasm] [--disasm-after] [--gc-stress]
```

## Summary

- **It worked quickly.** About 2,000 lines written from the docs
  compiled with no errors and passed 241 of 246 tests on the first run.
  The whole job, report included, took about 35 minutes.
- **It is fast.** With no tuning, the first version ran the benchmarks
  in about half of clox's time. The final version takes 0.45× clox's
  time by geometric mean.
- **All 246 tests now pass**, also with `--gc-stress` and under
  AddressSanitizer.
- **The problems were mostly in `std.obj`**: telling a method from a
  field, choosing property keys, and helpers that weren't inlined. All
  of these have been fixed.

## Issues

The study listed these issues, most important first. The state is as
of the follow-up work after the study.

| # | Issue | State |
|---|---|---|
| 1 | `std.obj`'s polymorphic and megamorphic caches couldn't tell a hit on the object itself from a hit on its prototype. Lox needs this to tell a method (bound to the receiver) from a field holding a function. | Fixed. `poly_get_at`, `mega_get_at` and `read_miss_at` return the depth of the hit. Lox's extra `Method` value variant is gone. |
| 2 | No documented way to turn a property name (a guest string) into a `std.obj` key. Lox used the address of the interned string, which is only safe while the string stays alive. | Fixed. `gc.WeakInterner.key(s)` pins the string and returns its address; `stdlib/obj.md` has a section "Keys for guest strings". |
| 3 | `poly_get` and `mega_get` were called, not inlined, on a handler's fast path. | Fixed. The hit helpers are `#[inline]`. `trees` went from 0.58× to 0.55× of clox's time. |
| 4 | A nanbox held only seven pointer variants; Lox needed eight. | Fixed. A nanbox holds up to 56 `Gc` variants. |
| 5 | A weak intern table for guest strings had to be written by hand (60 lines). | Fixed. `gc.WeakInterner<X>` (or the ready-made `gc.Str`). |
| 6 | Calls that may allocate on a handler's fast path compile silently. | Fixed, opt-in: the lint `-W hot_gc`. It doesn't see calls inside plain vm functions. |
| 7 | `CLOSURE` reads its capture list from the prototype, so the verifier can't check it; the handler checks at run time. | Won't fix. By design, as tutorial chapter 9 explains. |
| 8 | No branch relaxation, so a conditional jump spans at most 32767 words. This failed `limit/loop_too_large.lox`. | Fixed. `std.bc` relaxes branches (`set_relax`, `relax()`). Lox checks clox's loop limit itself and prints clox's message. |
| 9 | `std.lex` fits Lox only in part. Numbers and strings were scanned by hand, and `skip_trivia()` returns a `Result` even when it can't fail. | Partly fixed. `Cursor.skip_space()` can't fail. Numbers and strings are still scanned by hand. |
| 10 | `std.parse` stops at the first syntax error; clox reports it and keeps parsing until a statement boundary. | Won't fix. Not a Loimi problem; a "panic" flag in the parser state was enough. |

Not Loimi problems: the sandbox refused shell loops and `cd`, which cost
more time than anything in Loimi.

## What worked well

- **The tutorial's code transfers.** The vm from tutorial step 9
  (compare-and-branch ops, `inline for` templates, register windows,
  `on raise` with a line table) carried over almost unchanged.
- **`std.obj`** gave fields and methods inline caches with polymorphic
  and megamorphic states in about 150 lines of handlers.
- **GC integration was effortless.** `--gc-stress` in every dispatch
  mode and AddressSanitizer found no bugs on the first try.
- **The generated tools helped**: the disassembler (`--disasm-after`
  shows quickening), `--vm-stats` with its suggestions for super
  instructions, and the verifier.
- **`loimi test`** with `test.capture_start()` made in-process tests of
  whole programs about a dozen lines.

## Comparison with clox

Benchmarks at the time of the study. Best of 5 runs, interleaved,
wall-clock seconds for the whole process, on a shared 8-core x86-64
machine. clox was built by its own makefile (`-O3 -flto`, NaN boxing
on).

| benchmark | clox (s) | Loimi Lox (s) | Loimi / clox |
|---|---:|---:|---:|
| binary_trees | 2.332 | 0.606 | 0.26 |
| equality | 3.110 | 0.952 | 0.31 |
| fib | 0.954 | 0.388 | 0.41 |
| instantiation | 0.520 | 0.360 | 0.69 |
| invocation | 0.277 | 0.141 | 0.51 |
| method_call | 0.175 | 0.081 | 0.46 |
| properties | 0.415 | 0.187 | 0.45 |
| trees | 2.350 | 1.478 | 0.63 |
| zoo | 0.308 | 0.155 | 0.50 |
| zoo_batch (s per 1000 batches) | 1.923 | 0.922 | 0.48 |
| **geomean** | | | **0.45** |

After the fixes above the geomean is unchanged at 0.45×. `trees` went
to 0.55× and `fib` to 0.40×; `instantiation` went to 0.71× for no
reason found.

Notes on the comparison:

- `string_equality` is left out. clox rejects it (`Too many constants in
  one chunk.`), and this VM copies clox's limit.
- Program outputs match clox's on every benchmark.
- Neither side gets an advantage the other couldn't have: there is no
  constant folding, and unused expression values are still computed.
- The speed comes from the usual differences between a register VM with
  inline caches and a stack VM with hash-table lookups: no push and pop
  traffic, compare-and-branch ops, constant operands, inline caches for
  fields and methods, and `std.gc`'s size-class allocator.

## Design in brief

- **Values**: a NaN-boxed `Value` with an unboxed `Num(f64)`, `Nil`,
  `Bool`, and pointer variants for strings, closures, classes,
  instances, bound methods and cells. Natives are `Native(u32)`, an
  index.
- **Registers**: local slot *i* is register *i*. A call's register
  window starts at the callee, so arguments are never copied.
- **Closures**: the parser resolves names and marks captured locals. A
  captured local lives in a heap cell from its declaration on, and
  closures copy cell references. This gives clox's semantics without an
  open-upvalue list.
- **Objects**: instances are `std.obj` objects whose prototype is the
  class's method table. Property reads, writes and method calls use
  `std.obj`'s inline caches.
- **Strings** are all interned in a weak table, so equality is a word
  compare.
- **Errors**: the parser reproduces clox's messages and recovery.

**Size**: 2,132 lines of Loimi, without comments and blank lines, plus
98 lines of unit tests. The largest files are `vm.lm` (636),
`compile.lm` (586) and `parser.lm` (576).

Lox is in the golden tests (`tests/golden.rs`): the unit tests and the
programs in `examples/lox/tests/` run normally and with `--gc-stress`.
