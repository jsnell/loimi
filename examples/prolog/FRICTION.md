# Friction report: a Prolog in Loimi

A usability study. A Prolog interpreter was written from Loimi's docs
(`README.md`, `docs/language.md`, `docs/stdlib.md`, `docs/stdlib/gc.md`,
the tutorial), `std/` and `loimi --help`, without reading the other
example interpreters. This report lists what was hard, what has been
fixed since, and how the result compares with SWI-Prolog and Trealla.
It is for people improving Loimi's usability, or curious how usable it
is.

Prolog tests things the earlier studies did not: unification,
backtracking with choicepoints, the trail, cut, first-argument indexing,
`catch/throw`, and terms in the GC heap. The supported subset is in
[SPEC.md](SPEC.md).

## What was built

| | |
|---|---|
| Size | About 5,700 lines: reader 730, clause compiler 1,325, WAM vm 579, builtins 1,245, machine (stacks, trail, unify) 405, arithmetic 365, writer 325, a Prolog library 223, and smaller files |
| Features | ISO core plus common SWI-Prolog library predicates: WAM compilation with first-argument indexing and last-call optimization, unbounded integers on `std.bigint`, `catch/throw` with ISO error terms, DCG, assert/retract, findall/bagof/setof, `format/2`, `op/3` |
| Tests | 11 programs (12 after the third round) that print exactly SWI-Prolog 10's output. `check.sh` runs them in all three dispatch modes, under GC stress, in GC verify mode and with `--gc-check-roots`: 110 runs, all pass. `cargo test` runs a quick form (13 s) |
| Time taken | 28 minutes from first reading the docs to a working interpreter; about 1 hour 20 minutes to all 30 benchmarks running at 0.81x SWI-Prolog's time (same day) |

**Design.** Terms live in the `std.gc` heap, not in a WAM heap array.
`Term` is a nanboxed `value` type with atoms and 48-bit integers stored
inline. This gives up the WAM's free reclamation on backtracking, but
lets Loimi's collector and compiler-rooted locals manage the terms
instead of a hand-written collector.

Every variable is its own heap object, so there are no unsafe variables
or environment trimming. A binding is trailed only if the variable is
older than the newest choicepoint, compared by a creation stamp.
Environments and choicepoints live in manually allocated stacks found
by a root scanner. Control constructs in clause bodies compile to
auxiliary predicates; `catch/3` is a library predicate whose
choicepoint is the catch frame.

## Main findings

- **Allocation and write barriers in handlers were the main cost.**
  Allocating or binding gave every hot handler a stack frame, and
  avoiding it meant reading `std/gc.lm`'s internals. Fixed: `try_new`
  and `make_room` allocate without a frame.
- **Rooting just worked.** Not one rooting bug. Every failure the GC
  stress and verify runs found was a bug in the clause compiler.
- **The tools pointed straight at problems.** `-W handler_frame`,
  `--vm-stats` and the generated disassembler found the frames, the
  useful superinstructions, and the one serious compiler bug.
- **The parser library did not fit Prolog's run-time operators.** Since
  fixed for the parser with `std.parse.Ops`; the lexer is still
  hand-written.
- **Speed.** The first version took 0.79x SWI-Prolog's time. After three
  optimization rounds it took 0.53x, faster than SWI-Prolog on all 30
  benchmarks. Current numbers are in
  [bench/prolog/README.md](../../bench/prolog/README.md).

## Issues

Ordered by how much they cost at the time, highest first.

| # | Issue | State |
|---|---|---|
| 1 | Allocating (`new`) or binding (`v.val = t`) in a handler gave it a stack frame on every run, from the free-list refill call and the write barrier's `remember` call. The workaround read `Heap.free` and the size-class formula from `std/gc.lm` and stored through raw pointers. It took an hour and gained about 5%. | **Fixed.** `try_new` takes only the inline free-list pop; `make_room` refills in a cold block that `redo`es the op. The barrier's slow path makes no call. Binds are plain stores again. |
| 2 | Storing a variant with no `Gc` payload (such as `Term.Unb`) still got a write barrier, which put a frame into every handler that can fail. | **Fixed.** Stores of values that statically hold no references get no barrier. |
| 3 | `#[inline(never)]` on a `-> never` vm function (`backtrack`) was ignored: LLVM inlined it into every caller. | **Fixed.** It is now one shared copy. |
| 4 | `std.parse.Pratt` is keyed by a fixed token enum; Prolog operators are atoms that `op/3` changes at run time. `std.lex` doesn't describe Prolog's tokens (the end token, `0'c`, `\NNN\` escapes). Both were written by hand. | **Parser fixed, lexer open.** `std.parse.Ops` is a run-time operator table with a driver for Prolog's rules; the reader went from 730 to 620 lines. The lexer is still hand-written. |
| 5 | Frames mix raw pointers and terms. The root scanner marked every stack word as a `Term`, which was safe only because of how this nanbox encodes floats, and would silently break under another layout. | **Made safe.** `#[scan_raw_words]` is a compiler-checked guarantee for such scanning. Since `Term` moved to a layout that refuses it (2026-11-06), frame headers are stored as `Int` words, so the stack holds only terms. |
| 6 | Try/retry/trust chains jumped to clause code through cache words written by hand, so the vm couldn't be `#[require_verified]` and the JIT compiled nothing. | **Fixed.** Cache fields can hold a `Vm.Resume`, and `jump cache.code` is verified. Clause code is verified code objects; the JIT compiles it. |
| 6a | Terms in a GC heap must be swept after backtracking; a WAM heap frees them for free. Sweeping took 15–21% of allocation-heavy runs. | **Resolved (2026-11-04).** std.gc no longer sweeps (mark bitmaps). Prolog took 0.885 of its time before. |
| 7 | Small paper cuts, listed below. | **Mostly fixed.** |

The paper cuts:

| Paper cut | State |
|---|---|
| `(16 + 8 + 8 * n + 15) >> 4` with `n: u64` failed: the literal part was typed `i64` first | Fixed |
| `let t: [T; 107] = [..]` had to count elements exactly | Fixed: `[T; _]` |
| The Prolog library is a Loimi string, so every backslash was doubled | Fixed: raw strings `r"..."` |
| `std.io.print` is unbuffered | Fixed: `std.io.Out`, a buffered writer (no flush at exit) |
| No file inclusion, so syntax errors in the library report lines of the string | Open |
| Opcode constants have the code word's type (`u32`), not the encoders' `u8` | Open |
| The docs don't state that `Gc<T> as *T` points at the first field | Open |
| A bounds-checked index in a loop put a panic call, and so a frame, into `TRY` | Not a bug: `-W handler_frame` named it |
| A cold block that fell through was outlined in the op but not in a superinstruction containing it | Worked around by ending the block in a `jump` |

## What worked well

- **Rooting.** The compiler roots everything held in locals across GC
  calls. The root scanner is 40 lines.
- **`value` types.** `Term` with eight variants, `match` on pairs, and
  `==` on bits made head unification fast paths a few instructions.
- **The generated disassembler** found a bug in permanent-variable
  classification (a 64-bit mask, wrong past 63 calls in a clause). The
  generated verifier checked every code block.
- **`-W handler_frame` and `--vm-stats`** showed where to look. The
  superinstructions `--vm-stats` suggested took one line each and
  gained 3%.
- **`inline for`** generated each op family (arithmetic, comparisons,
  X/Y variants) from one template.
- **`on raise`** with a `Term` payload made `throw/1` and builtin errors
  one code path.
- **std.bigint** made unbounded integers about 150 lines.
- **Dispatch modes.** The same source runs in all three with identical
  behaviour. Goto took 1.00x and switch 1.04x tail-call's time.
- Circular module imports just work.

Reading the other examples afterwards: Mini has the same hand-made
barrier pattern as issue 1 and would have saved some time. For the
Prolog-specific parts, the docs were the right source.

## Performance

All 30 van Roy / Aquarius programs from SWI-Prolog's benchmark suite
(`bench/prolog`), against SWI-Prolog 10.0.2 with `-O` and Trealla
3.14.46. Times are geometric means of Loimi's time over the other's.
The machine was shared with other agents' builds, so single benchmarks
move by several percent.

| Point | vs SWI-Prolog | vs Trealla |
|---|--:|--:|
| Baseline (29 programs) | 1.04x | 0.25x |
| End of first round | 0.79x | 0.19x |
| End of second round | 0.63x | 0.15x |
| End of third round | 0.53x | 0.13x |

After the first round, Loimi was faster than SWI-Prolog on 25 of 30.
The slowest were sieve (2.03x: every `assertz` compiles and verifies a
clause) and boyer (1.15x).

### First round

The biggest gains:

| Change | Effect (time, new/old) |
|---|---|
| Compile `\+` and if-then-else conditions inline instead of through `call/1` | Five benchmarks from over 20 s to under 1 s |
| Index dynamic clauses by first argument for `retract/1` and `clause/2` | sieve from over 20 s to 2.3 s |
| Inline `is/2` and comparisons on registers | tak 0.46, crypt 0.43 |
| Allocate head temporaries to their argument registers | nreverse: register moves from 17.8% of ops to 0 |
| Superinstructions suggested by `--vm-stats` | 0.97 |
| Frameless fast paths (issue 1) and integer fast paths for `//`, `mod` | 0.95; crypt 0.69 |
| Inline `=/2`, type tests, `arg/3` | 0.92; unify 0.72 |
| Heap growth 400% instead of 100% (now std.gc's default) | boyer 0.69 |

## Optimizations, second round (2026-11-04)

Found with a timer-signal sampler (`perf` was not available),
`--vm-stats` and `objdump`. Result: 0.76 of the previous time overall,
from 0.44 (queens_8) to 0.97.

| Change | Effect (time, new/old) |
|---|---|
| Arithmetic ops dereference their operands instead of falling back to the generic evaluator | queens_8 0.49, query 0.53, sendmore 0.53 |
| The generated vm verifier no longer clears a fixed 64 KiB bitmap per call (a compiler change; every `assertz` verifies) | sieve 0.54 |
| Structures of arity 1–4 allocated inline | derive family 0.89–0.95 |
| Nested compounds in heads built in place | divide10, times10 0.83 |
| Head ops fail and bind variable-to-variable inline | mu 0.84, sendmore 0.89 |
| Guard chains: a first clause whose comparisons come before its cut runs without a choicepoint | flatten, log10 0.95 |
| std.gc skips chunks that have been empty since the last sweep | boyer 0.91, poly_10 0.85 |

Smaller gains came from indexing in CALL and EXECUTE, trailed bindings
inline in head ops, seven more superinstructions, and better register
allocation across guards.

Tried and dropped, no gain beyond noise: a cheaper index hash, indexing
on later arguments, `--gc-gen`, lazy sweeping, and other minimum heap
sizes. Heap growth 1600% helped boyer but tripled its peak memory.

Friction this round:

- Adding `deref` to the division ops gave them a stack frame, because
  LLVM no longer proved the overflow panic dead. Only the warning said
  anything; dividing as `i64` fixed it.
- An operand kind gives a register's index or its value, not both, so
  one op reads the register itself.
- The verifier's 64 KiB clear was invisible in the source; only the
  sampler found it.

Later the same day std.gc stopped sweeping altogether, and Prolog took
0.885 of this round's time.

## Optimizations, third round (2026-11-04)

The machine was heavily loaded (±10% between runs of the same binary),
so only large changes were kept. Result: 0.92 of the previous time
overall; tak 0.55, boyer 0.53.

| Change | Effect (time, new/old) |
|---|---|
| A first clause whose first goal is a comparison that every other clause excludes (`X =< Y` / `X > Y`) runs without a choicepoint | tak 0.57 |
| `arg/3`, `functor/3` and `compare/3` with a new output variable write the register directly | boyer 0.52, flatten 0.87 |
| Division by 32-bit unsigned when both operands fit | crypt, query 0.95–0.97 |
| Minimum heap 2 MiB instead of 1 MiB | 0.98 |

Tried and dropped:

- Shallow backtracking for clauses that bind before a neck cut: correct
  but no faster (derive 0.99, log10 1.09).
- An iterative evaluator for left-nested expressions: eval unchanged.
- Storing variables inside the structures that contain them (the WAM's
  layout) was not attempted: std.gc does not trace interior pointers.

What still takes the time: clause selection (10–17% in several
programs), general `unify` in zebra (47%), the arithmetic evaluator in
eval (47%), `=\=` in sendmore (34%), division in crypt (27%), and the
collector (2–10%).

Friction this round: `-W handler_frame` caught a division-by-zero panic
at once; the hard part was measuring on a loaded machine with no
performance counters.

## The JIT build (2026-11-04)

With `--jit-stencils` and `LOIMI_JIT=1`, the JIT build first took 1.11x
the interpreter's time, and 48x on sieve. Both causes were in the JIT;
the Prolog source did not change.

- Jumps into try-chain clauses through `jump cache.code` were not
  counted, so most clauses were never compiled. They now count like
  calls.
- Each asserted clause is a code object the JIT registers, and its
  registry was a sorted array. Sieve's 700,000 clauses made that
  quadratic. It is a tree now.

After the fixes the JIT build took 0.98 of the interpreter's time
(sieve 1.30). It gains less than on Mini (0.78) because the Prolog ops
do most of their work in their bodies, not in dispatch. `check.sh` runs
the tests under the JIT as well.
