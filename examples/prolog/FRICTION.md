# Friction report: a Prolog in Loimi

A usability study: a Prolog interpreter written from the docs
(`README.md`, `docs/language.md`, `docs/stdlib.md`, `docs/stdlib/gc.md`,
the tutorial), `std/` and `loimi --help`, without reading the other
example interpreters. It stresses what earlier studies did not:
unification, backtracking with choicepoints, the trail, cut, first-argument
indexing, exceptions (`catch/3`), and a term representation in the GC heap.
The supported subset is in [SPEC.md](SPEC.md).

## Summary

| | |
|---|---|
| Files | `term.lm` 154 (terms, atoms, functors), `machine.lm` 403 (stacks, trail, unify, compare, copy, root scanner), `vm.lm` 531 (the WAM), `compile.lm` 1221 (clauses to WAM code, indexing, call/N), `builtins.lm` 1228, `arith.lm` 272, `reader.lm` 717 (tokens, operator precedence parser), `write.lm` 322, `prelude.lm` 223 (library predicates in Prolog), `main.lm` 251, `check.sh` 93 |
| Features | ISO core plus the common SWI library: see SPEC.md. WAM compilation with first-argument indexing, last-call optimization, inline arithmetic, `catch/throw`, cut through `;`/`->`, soft-cut, DCG, assert/retract with the logical update view, findall/bagof/setof, `format/2` |
| Tests | 10 programs in `tests/` (control, exceptions, terms, arith, write, atoms, database, lists, gc, basics), each printing exactly SWI-Prolog 10's output; `check.sh` runs them in the three dispatch modes, each normally, under GC stress and in GC verify mode (generational, stress, barrier verification), plus a `--gc-check-roots` build under stress: 100 runs. `cargo test` runs the quick form (`prolog_programs` in tests/golden.rs, 13 s) |
| Benchmarks | 29 of the 30 van Roy / Aquarius programs from SWI-Prolog's bench suite run (`perfect` needs integers beyond 48 bits); see the table below |
| Time | first doc read 15:32:52; first commit of a working interpreter (prelude, tests, all benchmarks but four running) 16:01:17; every test passing in all modes 16:24:24 (commit timestamps, same day) |

The design: terms live in the `std.gc` heap (`object Var`, `Struct`,
`Cons`; a nanboxed `value Term` with atoms and 48-bit integers
immediate), not in a WAM heap array. That gives up instant reclamation on
backtracking, but makes the term GC Loimi's precise mark-sweep collector
with compiler-rooted locals instead of a hand-written sliding collector
for a heap the machine also uses as a stack, and it exercises Loimi's GC
integration, which a raw cell array would bypass. Variables are separate
`Var` objects (no unsafe variables, so `put_unsafe_value` and
environment trimming are not needed); a binding is trailed only if the
variable is older than the newest choicepoint, compared by a creation
stamp instead of heap addresses. Environments and choicepoints are
WAM-style stacks in `mem.alloc`'d memory, scanned by a root scanner.
Control constructs in clause bodies compile to auxiliary predicates, with
a level variable for cuts that must cut the parent clause; `catch/3` is
a two-clause library predicate whose choicepoint is the catch frame.

## Friction items

Ranked by how much they cost (time or performance), highest first.

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
CALLB, METACALL and REINDEX (which exist to call) keep a frame. With
integer fast paths for `//`, `mod` and friends in the same commit, the
benchmarks got 0.65-0.95 of the time (table below, "frameless").

*Suggested fix.* std: `gc.has_room<T>(h, n)` (or a `try_new` returning
`?Gc<T>`) documented as the way to allocate in a handler without a
frame; or have the compiler outline the refill path of an inlined `new`
in a handler the way it outlines `cold` blocks (a tail call to a stub
that allocates and returns to the handler's continuation is not possible,
but a `cold`-style split of the op at the allocation is). Language: the
barrier's `remember` call in a handler could go through the same
mechanism. Docs: say plainly in language.md §5.2 that `new` in a handler
gives it a frame today.

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

*What happened.* No shared copy exists in the binary (`objdump -d` has no
symbol for it); its body (an untrail loop and an argument restore loop)
is inlined into every handler that fails, which is how item 2's barrier
reached them.

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
717 lines, half of it the parser).

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

- **Rooting.** No GC bug of mine survived to a crash: everything the
  machine keeps in registers is rooted by the compiler at `uses gc`
  calls, the root scanner is short, and stress, verify (generational,
  barrier checks) and `--gc-check-roots` runs passed from the first full
  run. Only the deliberately raw stores (items 1, 2) needed thought
  about barriers, and verify mode covers them.
- **`value` types.** `Term` with seven variants, `match` on `(a, b)`,
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

## Benchmarks

See the table in [the results section](#results) (filled from
`bench/prolog/run.py`).
