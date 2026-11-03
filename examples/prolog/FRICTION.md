# Friction report: a Prolog in Loimi

A usability study: a Prolog interpreter written from the docs
(`README.md`, `docs/language.md`, `docs/stdlib.md`, `docs/stdlib/gc.md`,
the tutorial), `std/` and `loimi --help`, without reading the other
example interpreters. It stresses what earlier studies did not:
unification, backtracking with choicepoints, the trail and undoing
bindings, cut, first-argument indexing, `catch/throw`, and a term
representation in the GC heap. The supported subset is in
[SPEC.md](SPEC.md).

## Summary

| | |
|---|---|
| Files (lines) | `term.lm` 157 (terms, atoms, functors), `machine.lm` 405 (stacks, trail, unify, compare, copy, root scanner), `vm.lm` 579 (the WAM), `compile.lm` 1325 (clauses to WAM code, register allocation, indexing, call/N), `builtins.lm` 1245, `arith.lm` 365 (with big integers), `reader.lm` 730 (tokens, operator precedence parser), `write.lm` 325, `prelude.lm` 223 (library predicates in Prolog), `main.lm` 251, `check.sh` 93; 5,700 in all |
| Features | ISO core plus the common SWI-Prolog library (SPEC.md): WAM compilation with first-argument indexing, last-call optimization, argument-register allocation, inline arithmetic and type tests, unbounded integers on `std.bigint`, `catch/throw` with ISO error terms, cut through `;`/`->`, soft-cut, DCG, assert/retract with the logical update view, findall/bagof/setof, `format/2`, `op/3` |
| Tests | 11 programs in `tests/` (control, exceptions, terms, arith, bigint, write, atoms, database, lists, gc, basics), each printing exactly SWI-Prolog 10's output. `check.sh` runs them in the three dispatch modes, each normally, under GC stress and in GC verify mode (generational heap, stress, barrier verification), plus a `--gc-check-roots` build under stress: 110 runs, all pass. `cargo test` runs the quick form (`prolog_programs` in tests/golden.rs, 13 s) |
| Benchmarks | All 30 van Roy / Aquarius programs of SWI-Prolog's bench suite (bench/prolog): 0.81x SWI-Prolog's time and 0.20x Trealla's, geometric means (table below) |
| Time | first doc read 15:32:52; first commit of a working interpreter (reader, compiler, vm, prelude; a first test program) 16:01:17, 28 minutes later; 29 of 30 benchmarks running 16:09:40; all tests passing in every mode 16:24:24; 29 benchmarks at 0.83x SWI-Prolog 16:43; all 30 at 0.81x 16:55 (commit timestamps, same day) |

**Design.** Terms live in the `std.gc` heap (`object Var`, `Struct`,
`Cons`, and std.bigint's `Big`; a nanboxed `value Term` with atoms and
48-bit integers immediate), not in a WAM heap array. That gives up
instant reclamation on backtracking, but makes the term GC Loimi's
precise mark-sweep collector with compiler-rooted locals, instead of a
hand-written sliding collector for a heap the machine also uses as a
stack, and it exercises Loimi's GC integration, which a raw cell array
would bypass. Every variable is its own `Var` object (no stack
variables, so no unsafe variables, `put_unsafe_value` or environment
trimming); a binding is trailed only if the variable is older than the
newest choicepoint, compared by a creation stamp instead of heap
addresses. Environments and choicepoints are WAM stacks in `mem.alloc`'d
memory, found by a root scanner. Control constructs in clause bodies
compile to auxiliary predicates, with a level variable for cuts that cut
the parent clause; `catch/3` is a two-clause library predicate whose
choicepoint is the catch frame (marked inactive, through the trail, when
its goal exits leaving choicepoints).

## Friction items

Roughly by how much they cost (time, performance or risk), highest first.

### 1. Every allocating or binding handler had a stack frame on every execution

*What I tried.* Handlers in the obvious style: `let c = new(&ctx.heap,
Cons { .. })` to build a list cell in write mode, `v.val = t` (through
`Machine.bind`) to bind a variable.

*What happened.* `loimi build --release -W handler_frame` (an excellent
tool: it named the culprit calls) reported 35 handlers, among them every
hot one:

```
warning[handler_frame]: op `UNIFY_VAR_X` sets up a stack frame on its fast path (`pushq %rbp`), on every execution; calls in it (any of which can make LLVM set up the frame at entry): `std.gc.refill`
warning[handler_frame]: op `GET_ATOM` sets up a stack frame on its fast path (`pushq %rbp`), on every execution; calls in it (any of which can make LLVM set up the frame at entry): `machine.Machine.grow_trail`, `std.gc.remember`
warning[handler_frame]: op `FAIL` ... `std.gc.remember`
```

Three sources: (a) `new` inlines `std.gc.alloc`, whose free-list refill
is a call; (b) the write barrier the compiler emits after `v.val = t` is
`gc.barrier`, whose slow path calls `remember`; (c) the trail's
`#[cold]` grow function is still a call. The tutorial says "allocating a
value in a handler costs nothing extra unless the refill path actually
runs" (language.md §5.2); the refill does not run, but its call site
gives the handler a frame.

*What I did.* (a) Before each `new`, a vm function checks the heap's free
list for the size class (`(ctx.heap.free[2] as u64) != 0` for the
32-byte `Var`/`Cons` slots): LLVM then proves the refill call dead and
drops it, and the `else` is a `cold` block that ends in `next`. This
needs `Heap`'s internal layout (the `free` array, the size-class formula
`(16 + size + 15) >> 4`) from reading `std/gc.lm`. (b) `quick_bind(v)`:
if `v` is younger than the newest choicepoint (no trail entry) and
`!gc.needs_barrier(v)`, store with a raw pointer, `*(v as *Term) = t`,
which the compiler's barrier does not see; otherwise a cold block calls
the full `bind`. (c) follows from (b). Structure allocation (variable
size: `memset` is a call too) moved to `cold` entirely. Result: only
CALLB, METACALL and REINDEX (which exist to call) keep a frame.
Measured (interleaved A/B, best of 3, ten benchmarks), together with
integer fast paths for `//`, `mod` and friends in the same commit: 0.95
of the time by geometric mean, 0.69 for crypt (mostly the integer fast
paths), 0.92-1.03 for the rest. So a frame per execution was a few
percent here, not more; finding and removing them took an hour, most of
it working out the free-list check.

*Suggested fix.* std: `gc.has_room<T>(h, n)` (or a `try_new` returning
`?Gc<T>`) documented as the way to allocate in a handler without a
frame; or have the compiler split an op at an inlined `new` whose
refill may run, the way it outlines `cold` blocks (the refill and the
rest of the op in an outlined copy). The barrier's `remember` call in a
handler could be treated the same way. Docs: say plainly in
language.md §5.2 that `new` and reference stores in a handler give it a
frame today, and show the free-list check.

### 2. The compiler emits a write barrier for stores of payload-free variants

*What happened.* Undoing the trail stores `Term.Unb` (a payload-free
variant of the `value` type) into `Var.val`. The compiler emitted the
generational barrier for it, so every handler that can fail (all of
them, through `backtrack()`) called `std.gc.remember` on some path and
got a stack frame (`op FAIL ... std.gc.remember`).

*What I did.* `*(v as *Term) = Term.Unb`, a raw store.

*Suggested fix.* Skip the barrier when the stored value is statically a
variant without a `Gc` payload (a constant, or the result of a variant
constructor without references).

### 3. `#[inline(never)]` on a `-> never` vm function did not keep it out of line

*What I tried.* `#[inline(never)] fn backtrack() -> never { .. }` in the
vm, called from about thirty handlers, expecting the "one shared copy
with the handler signature" that language.md §3.5 describes for
`#[inline(never)]` ("makes every call in tail-call mode use the shared
copy").

*What happened.* No shared copy exists in the binary (`nm` lists the
ops, `on_raise`, `invalid` and the disassembler, nothing else of the
vm); its body (an untrail loop and an argument restore loop) is inlined
into every handler that fails, which is how item 2's barrier reached
them. The same holds for the repository's own `tests/run/vm_never_fn.lm`:
its `#[inline(never)] fn bump` has no symbol of its own in a release
build either (one caller there, so it proves less).

*What I did.* Nothing more (after item 2 the inlined copies are frameless).

*Suggested fix.* Check the attribute on `-> never` vm functions, or
document when it is ignored.

### 4. The parser and lexer libraries do not fit Prolog

*What I tried.* `std.parse.Pratt` for the operator precedence grammar
and `std.lex.Cursor` for tokens.

*What happened.* `Pratt` is keyed by a payload-free token-kind enum
(`K` "used as a table index via `k as u64`"), with the table filled at
startup; Prolog operators are atoms, added and removed at run time by
`op/3`, with priorities 0..1200, the seven types, prefix operators that
are atoms when followed by an infix operator or a closing bracket, the
"argument priority is 999 but SWI accepts more" rule, and `- 1` versus
`-1`. The lexer needs the end token (`.` followed by layout), `0'c`,
symbol-char atoms, `''` inside quotes and `\NNN\` octal escapes closed by
a backslash, none of which `Syntax` describes.

*What I did.* A hand-written tokenizer and precedence parser (reader.lm,
730 lines, half of it the parser).

*Suggested fix.* A `Pratt` keyed by `u64` (or a hash map of operator
definitions) that can change while parsing; for Prolog-family languages
that would have been most of the parser.

### 5. Environments and choicepoints mix raw pointers and terms; the GC has no way to say so

*What I tried.* WAM environments (`{ ce: *Frame, cp: *u32, n: u64 }`
followed by `n` terms) and choicepoints (nine bookkeeping words followed
by the saved argument registers) in `mem.alloc`'d stacks.

*What happened.* Loimi structs cannot have a trailing array (only
`object`s can), so the frames are a struct plus pointer arithmetic,
`((f + 1) as *Term)[i]`. For the root scanner, choicepoints are walked
precisely, but environments are not linked in a way the scanner can walk
cheaply, so it marks every word below the live top as a `Term`, which is
safe only because in this nanbox a raw pointer or a small integer reads
as a float (`Flt` is the unboxed variant: words below
`0xFFF9_0000_0000_0000`). A `repr lowtag` switch would silently break it.
Above the live top, the scanner zeroes words written since the last
collection so a stale reference is never exposed later.

*Suggested fix.* Trailing arrays in plain structs (`struct Frame { ..,
ys: [Term] }`, with `size_of` taking the length), and a documented
"`value` words that are safe to scan" guarantee or a `gc_mark_word`
that ignores non-references for a given value type.

### 6. Code pointers in try/retry/trust chains: cache words written by hand

*What I tried.* `TRY`, `RETRY` and `TRUST` each jump to a clause's code,
whose address must be in the instruction stream. A 24-bit clause number
indexing a table would cost a load per call and limit the number of
clauses.

*What happened.* An inline cache layout was the only way to put a
64-bit word after an op (`cache Target { code: u64 }`), but the
generated emitters zero cache words and there is no encoder that takes
cache contents, so the chain builder pushes the op word and then the two
halves of the pointer itself. `jump (cache.code as *u32)` is a raw jump,
so the vm cannot be `#[require_verified]` (every block is still verified
when built, by `Wam.verify` with the clause's permanent-variable count
as verify context; the proof is dropped).

*Suggested fix.* Emitters/encoders that take cache initial values, and a
cache field type holding a `Verified<Vm>` (or a verified jump table) so
indirect jumps can keep the proof.

### 7. Small paper cuts

- `(16 + 8 + 8 * n + 15) >> 4` with `n: u64`: "mismatched types:
  expected `i64`, found `u64`" at `n`. The all-literal subexpression on
  the left is typed `i64` before it meets `n`. Rewrote as `(n * 8 + 39)
  >> 4`.
- `let tbl: [(str, u32, B); 107] = [..]` must count the elements
  exactly ("expected `[([]u8, u32, B); 107]`, found `[..; 109]`"); the
  fix was to drop the annotation, which inference handles. An `[T; _]`
  would be nice.
- The Prolog library is Prolog source in a Loimi string: there are no
  raw strings or file inclusion, so every backslash is doubled
  (`\\+`), and syntax errors in it report lines of the string.
- `fn Reader.var(..)`: "expected identifier, found `var`, which is a
  reserved word (rename it, e.g. `var_`)": clear.
- Opcode constants (`Wam.OP_ADD`) have the code word's type (`u32`), so
  `?u8` tables of opcodes did not type-check; the encoders' field types
  are `u8`. Consistent once known.
- `std.io.print` is an unbuffered `write(2)`: a Prolog that prints a
  lot needs its own buffer (`Machine.out`, flushed at exit, at 64 KiB
  and before writing to stderr).
- That `Gc<T> as *T` points at the first field (the header is before
  it) is implied by "the trailing-array length, which compiled code reads
  at object - 8", not stated; `s = c as *Term` for a `Cons` relies on it.
- A bounds-checked `ctx.x[i]` in a loop over a run-time count put a
  panic call, and with it a frame, into `TRY`; `-W handler_frame` named
  it (`a panic (vm.lm:344:32: index out of bounds)`), and a raw pointer
  fixed it.
- A `cold { gc.collect(..); }` that falls through, inside `EXECUTE`,
  was outlined in the op but not in the superinstruction
  `DEALLOCATE; EXECUTE`, which got a frame; writing the cold block to end
  in its own `jump` fixed both.

## What worked well

- **Rooting.** Not one rooting bug: everything the machine keeps in
  registers is rooted by the compiler at `uses gc` calls, the root
  scanner is 40 lines, and every failure the stress, verify
  (generational, barrier checks) and `--gc-check-roots` runs found was a
  bug in my clause compiler (the chunk mask below, unbounded recursion on
  long lists), which collections at other moments exposed. Only the
  deliberately raw stores (items 1, 2) needed thought about barriers, and
  verify mode covers them.
- **`value` types.** `Term` with eight variants, `match` on `(a, b)`,
  `==` on bits for atom/integer comparison, nanboxed floats: the head
  unification fast paths are a few instructions (below).
- **The generated disassembler**, through a five-line `--disasm` switch,
  found the one serious compiler bug (permanent-variable classification
  used a 64-bit chunk mask, wrong past 63 calls in a clause); the
  verifier checked every block (`yr` operands against the clause's
  environment size).
- **`-W handler_frame`** and **`--vm-stats`** (op counts, cold-block
  counts, suggested supers) told me exactly where to look; the supers it
  suggested were one line each and gained 3%.
- **`inline for`** generated the op families (ADD/SUB/MUL with their
  immediate forms, nine integer ops, six comparisons, the X/Y variants of
  the unify instructions) from one template each.
- **`on raise`** with a `Term` payload made `throw/1` and builtin errors
  one path: `raise ball` from any handler or `?` on a builtin's
  `Result<bool, Term>`, unwinding in one handler.
- Circular module imports (vm ↔ compile ↔ builtins) just work.
- **std.bigint** made unbounded integers (needed by `perfect`) about 150
  lines: a `Big(Gc<Big>)` variant of `Term`, the `gc_*` operations on
  `Arg.Small`/`Arg.Ref`, and a `norm` back to the immediate form. The only
  Prolog-specific care was that `==` on a `value` compares bits, so two
  equal big integers needed value comparison in unification, standard
  order and the constant-matching ops (a cold path), and no
  first-argument index key.
- **Dispatch modes**: the same source in all three, with no differences
  in behaviour across 110 test runs. Speed (A/B, six benchmarks): goto
  1.00x tail-call, switch 1.04x.

## Generated code

`UNIFY_VAR_X` (write mode allocates a fresh variable; read mode copies
an argument), release build, tail-call dispatch: no frame; the
allocation is inline (free-list pop, header store, the `count` and
`allocated` statistics); dispatch is four instructions.

```
<vm.Wam.op.UNIFY_VAR_X>:
  mov    %esi,%ecx
  movzbl %ch,%eax                 ; the register operand
  test   $0x1,%dil                ; wm (write mode), a state register
  je     read
  mov    0x830(%r12),%rdx         ; heap.free[2]
  test   %rdx,%rdx
  je     cold7                    ; empty: allocate in the cold block
  mov    0xc78(%r12),%rcx         ; cstamp
  mov    0x8(%rdx),%rsi
  mov    %rsi,0x830(%r12)         ; pop the free list
  movl   $0x0,0x4(%rdx)           ; header
  movq   $0x0,0x8(%rdx)
  incq   0xac0(%r12)              ; heap.count
  addq   $0x20,0xad0(%r12)        ; heap.allocated
  lea    0x10(%rdx),%rsi
  movabs $0xfffe000000000000,%rdi ; Unb
  mov    %rdi,0x10(%rdx)          ; Var { val: Unb,
  mov    %rcx,0x18(%rdx)          ;       stamp }
  ...                             ; tag as Ref
  mov    %rdx,(%r15)              ; *s = v
  mov    %rdx,(%r12,%rax,8)       ; x[r] = v
  add    $0x8,%r15                ; s += 1
  mov    0x4(%r13),%esi           ; next
  add    $0x4,%r13
  movzbl %sil,%eax
  mov    $0x1,%edi
  jmp    *0x43a0c0(,%rax,8)
read:
  mov    (%r15),%rcx
  mov    %rcx,(%r12,%rax,8)
  add    $0x8,%r15
  ...                             ; next
```

## Would the other examples have helped?

Skimmed after this one worked. Mini's `value.lm` has the same hand-made
barrier pattern as item 1 (`gc.needs_barrier` inline, `remember` out of
line) and would have saved some of that hour; Scheme's `prelude.lm`
embeds its library as a string the same way (item 7); Scheme's reader
and Python's `big.lm` (bigint promotion) solve similar problems but
nothing Prolog-specific (operators, unification, choicepoints), so the
docs were the right source for the rest.

## Optimizations: what helped

Interleaved A/B runs, best of 3 (`.build/ab.py`, not committed), and the
full table (`bench/prolog/run.py`) at three points.

| Change | Effect |
|---|---|
| Cut-free `\+` goals and if-then-else conditions compiled into the auxiliary clause instead of through `call/1` (which compiles a clause per call) | derive, divide10, log10, ops8, times10: from over 20 s to under 1 s |
| Clause lists per first-argument key for `retract/1` and `clause/2` on dynamic predicates, with lazy iteration | sieve: over 20 s to 2.3 s |
| **Baseline** | **1.04x SWI-Prolog, 0.25x Trealla** (geomean, 29 programs) |
| Inline `is/2` and comparisons on registers (ADD/SUB/MUL with i48 overflow checks, immediate forms, integer fast paths) | tak 0.46, crypt 0.43 of the time |
| Head temporaries allocated to the argument register they are passed in (WAM register allocation) | nreverse: PUT_VAL_X from 17.8% of executed ops to 0 |
| Five superinstructions from `--vm-stats` (`GET_LIST; UNIFY_VAR_X; UNIFY_VAR_X`, `PUT_VAL_Y; PUT_VAL_Y`, `DEALLOCATE; EXECUTE`, ...) | 0.97 (geomean of seven) |
| Frameless fast paths (item 1) and integer fast paths for `//`, `mod`, `>>`, ... | 0.95 (ten); crypt 0.69 |
| **Optimized, without bigints** | **0.83x SWI-Prolog, 0.20x Trealla** (29 programs) |
| Inline `=/2` (a new variable on one side is just a register write), type tests, `arg/3`, the cut of auxiliary clauses | 0.92 (ten); unify 0.72, flatten 0.78 |
| Unbounded integers (perfect runs) | no change elsewhere |
| **Final** | **0.81x SWI-Prolog, 0.20x Trealla** (all 30) |

What Loimi made hard: item 1 (allocation and barriers in handlers),
item 5 (no way to describe mixed frames to the GC), item 6 (raw jumps
through cache words instead of verified code). What it made easy: trying
each change (an op family in a few lines with `inline for`, a super in
one line), and finding where time goes (`--vm-stats` op and cold-block
counts, `-W handler_frame`, every op its own symbol in `objdump`).

## Benchmarks

`bench/prolog/run.py -n 3`: wall-clock seconds, best of three
interleaved runs, startup included, on a shared 8-core x86-64 machine
(another agent was running builds and tests). SWI-Prolog 10.0.2 with `-O`,
Trealla 3.14.46, ours a release build with tail-call dispatch.

| benchmark | K | swipl s | trealla s | loimi s | loimi/swipl | loimi/trealla |
|---|--:|--:|--:|--:|--:|--:|
| boyer | 57 | 1.056 | 5.186 | 1.858 | 1.76 | 0.36 |
| browse | 41 | 1.044 | 6.592 | 0.871 | 0.83 | 0.13 |
| chat_parser | 160 | 1.049 | 3.784 | 0.963 | 0.92 | 0.25 |
| crypt | 7900 | 1.034 | 3.838 | 0.584 | 0.56 | 0.15 |
| derive | 290000 | 1.023 | 2.863 | 0.687 | 0.67 | 0.24 |
| divide10 | 630000 | 1.040 | 2.828 | 0.784 | 0.75 | 0.28 |
| eval | 11000 | 0.982 | 2.610 | 0.667 | 0.68 | 0.26 |
| fast_mu | 29000 | 1.028 | 4.019 | 0.857 | 0.83 | 0.21 |
| flatten | 48000 | 1.029 | 5.497 | 0.843 | 0.82 | 0.15 |
| log10 | 1300000 | 1.013 | 3.698 | 0.612 | 0.60 | 0.17 |
| meta_qsort | 4700 | 1.039 | 5.327 | 0.593 | 0.57 | 0.11 |
| mu | 30000 | 1.026 | 4.366 | 0.637 | 0.62 | 0.15 |
| nand | 1100 | 0.983 | 3.795 | 0.765 | 0.78 | 0.20 |
| nreverse | 83000 | 1.056 | 8.269 | 0.799 | 0.76 | 0.10 |
| ops8 | 920000 | 1.066 | 3.390 | 0.671 | 0.63 | 0.20 |
| perfect | 3900 | 1.026 | 5.075 | 0.880 | 0.86 | 0.17 |
| poly_10 | 510 | 1.023 | 2.691 | 0.912 | 0.89 | 0.34 |
| prover | 25000 | 1.017 | 3.710 | 0.592 | 0.58 | 0.16 |
| qsort | 47000 | 1.024 | 6.246 | 0.622 | 0.61 | 0.10 |
| queens_8 | 560 | 1.004 | 5.524 | 1.064 | 1.06 | 0.19 |
| query | 11000 | 1.047 | 4.784 | 1.051 | 1.00 | 0.22 |
| reducer | 640 | 1.020 | 4.761 | 0.966 | 0.95 | 0.20 |
| sendmore | 410 | 0.990 | 3.565 | 1.167 | 1.18 | 0.33 |
| serialise | 78000 | 1.006 | 5.359 | 0.697 | 0.69 | 0.13 |
| sieve | 61 | 1.008 | 2.542 | 1.938 | 1.92 | 0.76 |
| simple_analyzer | 1500 | 1.006 | 5.052 | 0.848 | 0.84 | 0.17 |
| tak | 160 | 1.037 | 3.630 | 0.784 | 0.76 | 0.22 |
| times10 | 670000 | 1.002 | 2.991 | 0.671 | 0.67 | 0.22 |
| unify | 13000 | 1.006 | 6.676 | 0.901 | 0.90 | 0.14 |
| zebra | 680 | 1.049 | 3.783 | 0.865 | 0.82 | 0.23 |
| geomean |  |  |  |  | 0.81 | 0.20 |

Ours is faster than SWI-Prolog on 25 of the 30 and slower on boyer
(1.76x: functor/3 and many permanent-variable moves per call),
sieve (1.92x: assert/retract, each `assertz` compiling and verifying a
clause, each `retract` copying a stored clause term), sendmore (1.18x),
queens_8 (1.06x) and query (1.00x). Trealla takes 4-10x longer than
ours on everything but sieve. Earlier points: the baseline (before the
optimizations above, 29 programs) was 1.04x SWI-Prolog and 0.25x
Trealla.
