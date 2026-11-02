# Usability report: writing Stak, a stack VM, in Loimi

An outside developer's notes from writing `examples/stack` (Stak, a
statically stack-checked Forth; see `SPEC.md`). I read the README,
`docs/` (tutorial, language, status, stdlib), `std/*.lm`,
`examples/tutorial/` and a few `tests/run` files. I did not read `src/`
or the other example interpreters; nothing forced me to (one lookup of
`Vm.opcode` in `std/bc.lm` and `tests/run/std_bc.lm`, item 9).

## Summary

| | |
|---|---|
| Interpreter | 1,407 lines of Loimi: `vm.lm` 588 (vm, runtime helpers, stack checker), `compile.lm` 586 (tokenizer and single-pass compiler with static stack-depth tracking), `value.lm` 106, `main.lm` 127; plus `fuzz.lm` 202 |
| Features | ints (i32, overflow to float), floats, strings, arrays, bools, nil; `: word ( in -- out ) ... ;` with recursion, locals `{ a b }`, globals, `if/else/then`, `begin/until`, `begin/while/repeat`, `do/loop` with `i`/`j`, `exit`, `[ ... ]` literals, 37 built-in words, compile errors with snippets, run-time errors with word and line |
| vm features used | state registers (including a `Value` as a cached top of stack), operand kinds with `verify` clauses and `display`, a fixed encoding, `cold` blocks, `inline for`, vm functions, `on raise`, `sync`, `#[require_verified]`, a `verify NAME { }` block, `super` (with `#[allow(super_overlap)]`), a `variant` for the fuzzer's step budget, `--vm-stats`, `std.gc` objects and a root scanner, `std.bc` code buffers, labels, line tables and constant pool, `std.lex` cursor, `std.diag` |
| Tests | 15 programs, run by `check.sh` in all three dispatch modes, normally and with `--gc-stress` (78 runs, all pass, built with `-W error`); a bytecode fuzzer (under ASan: 25,000 mutated programs, plus 3,000 with GC stress; no crashes after the two fixes in items 2 and 11) |
| Time | about 25 minutes of wall-clock time from the first doc read to a working, tested interpreter, and about 50 minutes in all, including optimization, the stack verifier, the fuzzer and this report (timestamps from the commits) |

Rough phases (wall clock): reading docs 6 min; spec + first complete
interpreter (compiled and ran the first test correctly on the first
build) 6 min; tests and `check.sh` 2 min; `--vm-stats` and supers 2 min;
top-of-stack caching 3 min; stack-depth `verify` block 2 min; longer
supers 2 min; fuzzer and its two findings 7 min; benchmarking and this
report the rest.

## Benchmarks

Release builds, best of 5, seconds, on a shared machine. `v1` is the
first working version, `final` is the committed one (supers, cached top
of stack). CPython 3.13 runs the same algorithms written idiomatically,
for scale only.

| benchmark | what | v1 | final (tailcall) | goto | switch | CPython 3.13 |
|---|---|---:|---:|---:|---:|---:|
| fib | `32 fib`, 7M calls | 0.085 | 0.052 | 0.058 | 0.062 | 0.35 |
| loop | 3 x 30M iterations of int/float `do` loops | 0.937 | 0.708 | 0.793 | 0.729 | (first third only: 1.95) |
| sieve | 30 x sieve(100000) | 0.117 | 0.087 | 0.098 | 0.095 | 0.49 |
| strings | string building, 3M short arrays | 0.145 | 0.135 | 0.135 | 0.132 | |
| nbody | 5M steps of float locals through an array | 0.185 | 0.153 | 0.181 | 0.171 | |

What paid: superinstructions from `--vm-stats` (8-20%), caching the top
of stack in a `Value` state register (another 10-15%), and 3- and 4-op
supers for stack idioms (`dup 2 < if`, `a b @`, `k i + to k`: 5-12% on
fib and sieve).

## Friction, most important first

### 1. A stack VM gets no stack verification; I wrote it by hand, three times over

*Trying to:* make Stak's handlers pop and push without underflow or
overflow checks, which is the point of a verified stack VM.

*What happened:* `language.md` §3.1 lists `StackTop<T>` with
`push`/`pop`/`peek(n)`, and §6 says ops declare `stack(a, b -- r)`
effects from which the verifier computes depths. `status.md` says both
are not built (`StackTop` is a plain pointer, stack effects are not
checked). So the generated verifier checks operand fields only: any
verified Stak program could `DROP` below the stack. I ended up with the
stack effect of every op written in three places: the compiler's
built-in table (`compile.lm`, `builtin`), a hand-written
`stack_effect(w)` in `vm.lm` (an `if` chain over `Stak.OP_*`) and the
handlers themselves, plus a 50-line abstract interpreter
(`check_stack`) run from a `verify stack { }` block. The fuzzer (item 3)
was what showed it was right.

*Expected:* `op ADD() : N stack(a, b -- r)`, with `a`/`b`/`r` bound in
the handler, the verifier computing per-function maximum depth, and a
generated `Vm.stack_effect(w)` for the compiler to use.

*Cost:* ~10 minutes, and the largest part of the code that is not the
language itself. For a register-VM author this item does not exist; for
a stack-VM author it is the main thing the "unchecked by proof" story
promises.

*Suggested fix:* implement `stack(..)` effects (language change), or at
least move the design-only rows (`StackTop`, stack effects) out of the
tables in `language.md` §3.1/§6 into a clearly marked "design" box, and
add a short "stack VMs" section to the tutorial or language doc showing
the hand-written pattern (a `verify` block plus a depth bound checked
once per call) as the current recommendation.

### 2. A `verify` block sees superinstruction heads, and that is undocumented

*Trying to:* re-verify optimized code in the fuzzer, as the tutorial's
fuzzer does (`code does not verify after Pip.optimize`).

*What happened:* my `verify stack` block decoded each word's opcode;
after `Stak.optimize` the first word of a fused sequence carries the
super's opcode (`PUSHI_LT_JZ`), which my effect table did not know, so
verification failed: `BUG: seed 1: code does not verify after
Stak.optimize`. The fix was to copy the code and run
`Stak.deopt_words` on the copy before analysing it. The same would
happen with quickened family members (`become`), which I did not use.

*Expected:* §6 ("Side tables") to say what `code` holds in a `verify`
block: possibly optimized and quickened words.

*Cost:* 5 minutes, thanks to the fuzzer; without it this would have
been a latent bug in any host that verifies after optimizing (e.g.
loading cached bytecode).

*Suggested fix:* doc note in §6; better, a generated
`Vm.base_op(w) -> u32` (the family-generic op of a super's first part)
or an option for `verify` blocks to see the deoptimized view.

### 3. `verify` blocks can only say "no"

*What happened:* a failing check reports ``error: bytecode of `main`
failed verification: check `stack` fails``, with no word index, while the
generated checks say ``word 4 (LOADK): operand `c` (kst) fails ...``.
When I deliberately broke the compiler's depth bookkeeping to test the
checker, I had to bisect by hand.

*Suggested fix:* allow a `verify` block to return `?u64` (failing word)
or a `Result<(), (u64, str)>`, and feed that into `VerifyError.at` and
`write`.

### 4. Greedy left-to-right super fusion fits register code better than stack code

*Trying to:* add the pairs `--vm-stats` suggested.

*What happened:* in stack code nearly every hot pair overlaps another
(`LGET PUSHI ADD`, `DUP PUSHI LT JZ`), and `optimize` takes the first
match left to right. Declaring the suggested pairs gave five
`super_overlap` warnings (which are excellent: they name the exact
sequence and which super loses). Removing the overlapping pairs and
adding the 3- and 4-op sequences explicitly with `#[allow(super_overlap)]`
was what worked.

*Suggested fix:* have `--vm-stats` also print frequent triples and
quadruples (the data is the same), and/or have `optimize` choose fusions
by dynamic programming over a basic block rather than greedily.

### 5. stdlib.md mixes design and implementation

*What happened:* each `stdlib.md` section describes the design first
(`std.collections.Table`, `std.rt.ValueStack`, `std.test`, `fmt`
format strings in `std.core`, `TokenStream`) and the "As implemented"
note after. Twice I planned around something that is design only
(`std.rt.ValueStack` for the data stack; `fmt` for error messages). The
`std.core` section has no "As implemented" note at all, although `fmt`
does not exist.

*Suggested fix:* lead with what exists (or mark design-only items
inline, e.g. *(design)*), and add the missing note to `std.core`.

### 6. Building messages is verbose: no formatting

*What happened:* every compile error with a number or a name in it is a
`StrBuf` dance (`clear`, `push_str`, `push_int`, `push_str`, copy into
the arena). I wrote two helpers (`Comp.fmt`, `Comp.fmt_n`) to keep it
readable; `vm.lm`'s `type_error` is the same. The tutorial's Pip does
the same in `undefined()`.

*Suggested fix:* a small `text.StrBuf.cat(..)`/`fmt` with `{}`
placeholders over str/int/f64 arguments (std addition); `inline for`
might already be enough to implement a fixed-arity version.

### 7. No string patterns in `match`

*What happened:* `match name { "dup" => .. }` gives `error: expected
pattern, found string literal`. The built-in word table became 37 lines
of `if name.eq("dup") { return Some(..); }`.

*Suggested fix:* allow string-literal patterns (lowered to length
switch + compares), or have the error suggest `lex.Keywords` /
`map.HashMap<str, T>`.

### 8. Struct literals need every field

*What happened:* `Machine` has 21 fields, and the literal appears in
`main.lm` and again in `fuzz.lm` (most fields zero or null, written as
`0 as *Value`, `0 as *mem.Vec<*Word>`). Adding a field (`tos`, `steps`)
meant editing both. An empty slice placeholder was
`mem.alloc<u32>(1).slice(0)` (I did not find an empty-slice literal in the
docs).

*Suggested fix:* field defaults or a `..zeroed()` struct update, and a
documented empty slice.

### 9. Small doc gaps in the generated API list

- `Vm.opcode(w)` exists (I found it in a comment in `std/bc.lm` and in
  `tests/run/std_bc.lm`) but is not in `language.md` §3.3's list of
  generated functions. A stack-effect table over opcodes needs it.
- Encoder parameter types are documented for 8-bit (`u8`) and `s16`
  (`i16`) fields; for 24-bit fields I guessed `i32`/`u32` (it worked).
- That an op may use a format with fields it ignores (`op DUP() : N`
  where `N` has a 24-bit `x`) I learned from Pip's `HALT() : ABC`.

### 10. Effects and roots around a cached top of stack: fine, but only by reasoning

*What happened:* moving the top of stack into a state register means
the GC must see it: I added `ctx.tos = tos` to `sync` and marked
`m.tos` in the scanner. That worked first time and `--gc-stress` agreed,
but nothing in the docs talks about traced values *in state registers*
(only traced handler locals, which are rooted automatically). A
forgotten `ctx.tos = tos` would be the classic stale-root bug the
effects system otherwise prevents.

*Suggested fix:* either root traced state variables automatically at
`gc` calls (the compiler knows their types), or warn when a state
variable of a traced type is not stored by `sync`.

### 11. Running the fuzzer can eat the machine

*What happened:* my first ASan fuzz run grew to 30 GB RSS before I
noticed: a mutant doubled a string in a loop (`dup concat`), which a
step budget on jumps does not stop. The fix was a length cap in
`concat` (as the tutorial's step 9 caps arrays). `ulimit -v` cannot be
used with ASan, so the tutorial's `fuzz` command line has no memory
bound.

*Suggested fix:* chapter 9 could recommend
`ASAN_OPTIONS=hard_rss_limit_mb=..` and mention unbounded growth of
strings, not only `array(2147483647)`.

### Minor

- `std.lex` is built for C-like tokens; for whitespace-delimited words I
  used only `skip_trivia` (with `line_comment = "\\"`) and `quoted`.
  That worked well; a `Cursor.word()` (run of non-space bytes) would make
  it a one-liner.
- `bc.Pool` has no way to take its items and free the index; I reached
  into `c.consts.index.free()`.
- An `entry` cannot share code with another entry except through a
  `-> never` vm function; that works (`fn start(..) -> never`), but the
  tutorial duplicates the entry body instead, so I wasn't sure it was
  allowed until I tried.

## What I liked

- **It worked the first time.** The first complete build of the vm, the
  compiler and the driver (~1,100 lines) compiled without an error and
  printed the right output for the first test. The tutorial's step 8 is
  close enough to a template that a different VM shape was mostly a
  matter of changing the ops.
- **`Value` as a state register.** Caching the top of stack was a
  mechanical edit of the ops (`*sp` becomes `tos`), and the generated `ADD` keeps it in `r15` with no
  spills: about 30 instructions, both fast paths and the dispatch included.
  The `PUSHI_ADD` super folds the constant's type check away.
- **`--vm-stats`** printing ready-to-paste `super` lines, and the
  `super_overlap` warning explaining exactly which sequence loses.
- **The verifier plus `verify` blocks plus `variant`** made a real
  bytecode fuzzer a 200-line file, and it found two real bugs (items 2
  and 11) in its first minute.
- **Diagnostics.** `std.diag` gave Stak compiler errors with snippets and
  carets for free; Loimi's own errors were clear the few times I saw
  them; debug builds trap integer overflow with a source location
  (`compile.lm:520:18: integer overflow` when I broke the depth
  bookkeeping on purpose).
- **Three dispatch modes and GC stress** from one flag each; `check.sh`
  runs the whole matrix in about 30 seconds.
