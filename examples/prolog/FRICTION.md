# Friction report: a Prolog in Loimi

A usability study: a Prolog interpreter written from the docs
(`README.md`, `docs/language.md`, `docs/stdlib.md`, `docs/stdlib/gc.md`,
the tutorial), `std/` and `loimi --help`, without reading the other
example interpreters. It stresses what earlier studies did not:
unification, backtracking with choicepoints, the trail and undoing
bindings, cut, first-argument indexing, `catch/throw`, and a term
representation in the GC heap. The supported subset is in
[SPEC.md](SPEC.md).

## Follow-up (what has been done since)

This report is kept as written. Since then:

| # | Status |
|---|---|
| 1 | Fixed. The write barrier's slow path (`std.gc.log_obj`) makes no call: it pushes onto the remembered set inline, and when the set is full it only counts the object, which makes the next collection major (the set then gets room for twice what was logged). Allocation in a handler: `try_new(h, T { .. }) -> ?Gc<T>` takes only the inline free-list pop (`None` where `new` would refill), and `gc.make_room(h, size)` refills in a cold block that `redo`es the op: `let Some(v) = ctx.try_var() else cold { ctx.make_room(); redo };` (`Machine.try_var`/`try_cons` wrap `try_new`). The vm no longer reads `Heap.free` or the size-class formula, and binds are plain `v.val = t` stores. `quick_bind` keeps `!gc.needs_barrier(v)` (a public test) so that LLVM drops the barrier's inline slow path from GET_VAL_X/Y, whose register pressure it would otherwise push into a spill. Framed handlers (`-W handler_frame`, tail-call, x86-64) in the nine example interpreters: 223 → 200 (Lox 26 → 22, Mini 40 → 38, MiniJS 67 → 59, Python 51 → 47, Scheme 12 → 7, Prolog 4 → 4 without the workaround, Stak 8, Wasm 15, regex 0). Compiling the barrier slow path as an implicit cold block was not built: a cold block is outlined only where the rest of its block ends in a control word, and most barriers sit in helpers (`table_set`, `bind`) that LLVM, not the compiler, inlines into handlers. language.md §4 ("Allocating and storing references in a handler"), stdlib/gc.md, lowering.md §3 and §10. |
| 2 | Fixed: a store of a value statically free of references (a variant without a `Gc` payload such as `Term.Unb` or `Int(n)`, `None`, a literal tuple/struct/array of such) gets no barrier (`gcgen::holds_no_refs`). `untrail` stores `v.val = Term.Unb` again. IR changes in the other interpreters: `value.new_arr` (Scheme, Stak, Mini), `vm.Scheme.fn.call_slow`, MiniJS `rt.Ctx.new_array`/`arr_reserve`, Python `value.new_list`/`new_tuple`/`list_grow`, Lox `vm.Lox.op.CLOSURE`, Mini `coro.*` and `vm.close_upvals` (stores of `None`/`Nil` into objects). |
| 3 | Fixed: `#[inline(never)]` on a `-> never` vm function makes its shared copy `noinline` (LLVM used to inline the `musttail` calls back); a copy that never reads the calling instruction's word (no `raise`, `?`, cold block, shared call) is passed `poison` for it, so cold callers need not keep the word live (without that, GET_VAL_X spilled `rbp`). `backtrack` is now one shared copy (`vm.Wam.fn.backtrack`); `tests/run/vm_never_fn.lm`'s `bump` has its symbol. The regex example's `#[inline(never)] fn backtrack` is affected the same way (benchmarks below). |
| 4 | Parser fixed, lexer not: `std.parse.Ops` is an operator table keyed by an atom id (u64) for op/3 (priorities, the seven types, coexisting prefix/infix/postfix definitions, removal at priority 0), with a driver (`OpHooks`, `op_parse`, `op_parse_arg`) that implements the prefix-operator-as-atom rules, priority-999 arguments (accepting infix operators above 999 except `,` and `|`, as SWI does), operators as atoms and `X = \+a`; `peek` decides functional notation and `-1` vs `- 1`. reader.lm uses it: 730 → 620 lines (operator table and parser 307 → 197), and `- =(a,b)` now parses as in SWI. The end token, `0'c`, symbol-char atoms and `\NNN\` escapes are still hand-written (stdlib.md, "Run-time operator tables"). |
| 5 | Made safe, not restructured: `#[scan_raw_words]` on a value type is a compiler-checked guarantee that words below 2^48 (raw pointers, small integers) read as variants without references (only `repr nanbox` with an unboxed float variant passes); `Term` carries it, so the environment scanner's word-by-word marking is checked, and switching to `lowtag` is a compile error instead of a silent hole (language.md §2). Trailing arrays in plain structs are not built. |
| 6 | Fixed: inline cache fields may have type `Vm.Resume` (a resume point: an instruction boundary of verified code), and `jump cache.code` is a verified jump. The verifier resets such fields to the placeholder (host-written words are not trusted); `v.fill_resumes(vals)` sets them on verified code in code order; `deopt` keeps them. `Vm.emit.OP_cached(code, at, operands.., fields..)` emits an op with its cache's integer, `bool` and pointer fields (language.md §7.1). Clause, chain and stub code are now `object Code { code: verified [u32] by Wam }` built by `Wam.build_verified` (kept alive by `Machine.codes`); `Pred.code` and the index tables hold `Verified<Wam>`; TRY/RETRY/TRUST jump through `Target { code: Wam.Resume }`; the vm is `#[require_verified]` (`run` takes `Verified<Wam>`). With `--jit-stencils` and `LOIMI_JIT=1` the JIT compiles the clauses (nreverse: 12 code objects, 129 instructions) and the 11 test programs pass with `LOIMI_JIT_HOT=2`. Speed, JIT (`LOIMI_JIT=1`, default threshold) against the interpreter build of the same source, best of 3 interleaved: 1.02 of the time by geometric mean over the 29 programs other than sieve (0.92 simple_analyzer, 0.93 nreverse to 1.13 queens_8), and sieve 21x slower: every assert/retract makes new code objects (never freed), which the JIT registers and compiles as they get hot. Stencils do not help a WAM whose ops mostly unify and allocate (the work is in the handlers' bodies, not in dispatch), and calls through `jump p.code` that land on chains enter compiled code only per code object. |
| 6a | Unchanged. |
| 7 | Mostly fixed: literal-only arithmetic takes the other operand's type (`(16 + 8 + 8 * n + 15) >> 4` with `n: u64`); `let t: [T; _] = [..]` takes the literal's length; raw string literals `r"..."`/`r#"..."#` (prelude.lm and the operator table no longer double backslashes); `std.io.Out`, a buffered writer (flushed when full, by `flush` and `free`; no flush at exit), is `Machine.out`. Unchanged: file inclusion (prelude syntax errors still report lines of the string), opcode constant types, the `Gc<T> as *T` statement in the docs. |

Benchmarks of these changes (`--align-functions=64`, tail-call, best of 3 interleaved, old build against new; noise about ±3%), geometric means of new/old time: Prolog 1.01 (30 programs; 0.97 to 1.07), Lox 0.995, Mini 0.994, MiniJS 0.998, Python 1.002, Scheme 1.004 (43), Stak 1.008, regex 0.998 (`rxhost bench`, both engines; its `#[inline(never)] fn backtrack` is now a shared copy). Wasm's IR is unchanged; in the others only the GC's functions and the stores that lost their barrier changed (item 2).

## Summary

| | |
|---|---|
| Files (lines) | `term.lm` 157 (terms, atoms, functors), `machine.lm` 405 (stacks, trail, unify, compare, copy, root scanner), `vm.lm` 579 (the WAM), `compile.lm` 1325 (clauses to WAM code, register allocation, indexing, call/N), `builtins.lm` 1245, `arith.lm` 365 (with big integers), `reader.lm` 730 (tokens, operator precedence parser), `write.lm` 325, `prelude.lm` 223 (library predicates in Prolog), `main.lm` 251, `check.sh` 93; 5,700 in all |
| Features | ISO core plus the common SWI-Prolog library (SPEC.md): WAM compilation with first-argument indexing, last-call optimization, argument-register allocation, inline arithmetic and type tests, unbounded integers on `std.bigint`, `catch/throw` with ISO error terms, cut through `;`/`->`, soft-cut, DCG, assert/retract with the logical update view, findall/bagof/setof, `format/2`, `op/3` |
| Tests | 11 programs in `tests/` (control, exceptions, terms, arith, bigint, write, atoms, database, lists, gc, basics), each printing exactly SWI-Prolog 10's output. `check.sh` runs them in the three dispatch modes, each normally, under GC stress and in GC verify mode (generational heap, stress, barrier verification), plus a `--gc-check-roots` build under stress: 110 runs, all pass. `cargo test` runs the quick form (`prolog_programs` in tests/golden.rs, 13 s) |
| Benchmarks | All 30 van Roy / Aquarius programs of SWI-Prolog's bench suite (bench/prolog): 0.79x SWI-Prolog's time and 0.19x Trealla's, geometric means (table below); after a second optimization round (2026-11-04), 0.63x and 0.15x |
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

A consequence found later: the copy-and-patch JIT (`--jit-stencils`)
builds, but only registers code made by `Vm.build_verified` (verified code
in a GC object), so with these raw blocks `LOIMI_JIT=1` compiles nothing.
Moving clause code into `verified [u32] by Wam` objects would need the
chain words to point at objects that a collection does not move (true of
std.gc today) and code reachable from the root scanner.

*Suggested fix.* Emitters/encoders that take cache initial values, and a
cache field type holding a `Verified<Vm>` (or a verified jump table) so
indirect jumps can keep the proof.

### 6a. A GC'd term heap pays for sweeping what backtracking would free

*What happened.* With terms in `std.gc`, every structure a failed or
completed branch built is garbage the collector must sweep; a WAM heap
would have reset its top on backtracking for free. `LOIMI_GC_STATS=1`
shows the sweep (eager, proportional to the slots allocated) at 15-21%
of the run for the allocation-heavy benchmarks whose live data is a few
KiB (nreverse 169 ms of 0.8 s, derive 120 of 0.69, qsort 97 of 0.62),
and marking at under 1%. The generational mode (lazy sweeping) was no
better here (nreverse 1.06x, boyer 0.82x, the rest within noise), and a
larger growth factor only helps when the live data is large: 400%
instead of the default 100% made boyer (4.5 MB live) 0.69 of the time
and is now the default.

*What I did.* Nothing more; it is the price of the design choice
(justified in the summary).

*Suggested fix.* std.gc: a bump-allocated nursery that is evacuated or
dropped wholesale (non-moving old space can stay), or an API to free a
region of recently allocated slots in bulk, which a Prolog could call on
backtracking past the choicepoint that started the region.

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
| **All 30, std.gc's default heap growth** | **0.81x SWI-Prolog, 0.20x Trealla** |
| Heap growth 400% (item 6a) | boyer 0.69, browse 0.93, the rest within noise |
| **Final** | **0.79x SWI-Prolog, 0.19x Trealla** (all 30) |

What Loimi made hard: item 1 (allocation and barriers in handlers),
item 5 (no way to describe mixed frames to the GC), item 6 (raw jumps
through cache words instead of verified code). What it made easy: trying
each change (an op family in a few lines with `inline for`, a super in
one line), and finding where time goes (`--vm-stats` op and cold-block
counts, `-W handler_frame`, every op its own symbol in `objdump`).

## Optimizations, second round (2026-11-04)

Where the time went first: a timer-signal sampler (`LD_PRELOAD`,
`SIGPROF` every 37 µs recording the instruction pointer and, for
library code, the first return address in the binary; `perf` is not
available on this machine) on every benchmark, then `--vm-stats` op and
cold-block counts and `objdump` of the hot handlers. Interleaved A/B
runs of the whole suite, best of 3 (5 or 7 for small differences),
`--align-functions=64`, on a machine shared with another agent (load
3-6 on 8 threads: single benchmarks move by ±3-5%, more under load).

| Change | Effect (time, new/old) |
|---|---|
| Arithmetic ops (ADD .. MAX, NEG, the six comparisons) dereference their operands: a register often holds a bound variable, not the number, which sent the op to the generic evaluator | queens_8 0.49, query 0.53, sendmore 0.53, crypt 0.75, tak 0.85 |
| The vm verifier (generated by the compiler) zeroed a fixed 64 KiB boundary bitmap per call; every `assertz` verifies its clause | sieve 0.54 (compiler change, below) |
| PUT_STRUCT and GET_STRUCT's write mode allocate structures of arity 1-4 inline (`try_new` with a constant length; they were cold blocks calling `new` and `memset`) | derive family 0.89-0.95 |
| `unify` handles identical arguments and a variable meeting a non-variable in its argument loop instead of a recursive call | zebra 0.93, serialise 0.94, mu 0.95 |
| Bindings that need a trail entry are inline in the head ops (they took the cold path into `unify`) | boyer 0.95, prover 0.94 |
| CALL and EXECUTE index the first argument themselves (an indexed predicate's code was an INDEX block: two dispatches per call) | 0.99 overall, nreverse 0.88 |
| Integer `+ - *` in the generic evaluator (expressions built at run time) without matching the functor name as a string | eval 0.82 |
| UNIFY_NESTED / GET_NESTED: a compound inside a head structure is built in place in write mode (it made a variable object to bind at once, and every later read went through it) | divide10, times10 0.83, derive 0.92, 0.98 overall |
| Head ops fail inline on two different atoms or small integers, and bind variable to variable inline (both went to `unify`) | mu 0.84, sendmore 0.89, browse 0.92, 0.975 overall |
| Seven supers from `--vm-stats` over the suite (`GET_VAR_Y; GET_VAR_Y`, `PUT_VAR_Y; CALL`, `ALLOCATE; GET_LEVEL`, ...) | 0.99 |
| Register allocation keeps head variables in their argument registers across inline guards (`X =< Y`, type tests, `=`, `is` on head variables): partition/4's first clause 15 instructions to 9 | tak 0.92, unify 0.93 |
| std.gc: a chunk empty since the last sweep and not allocated from is not swept again (the sweep followed the heap's high-water mark: boyer peaks at 4.7 MB live, then drops below 1 MB) | boyer 0.91, poly_10 0.85 |
| Guard chains: a first clause whose head binds nothing and whose comparisons precede its neck cut runs them without a choicepoint (JLT .. JNNE jump to the other clauses on failure) | flatten, log10 0.95, 0.98 on ten programs |
| UNIFY_LIST: a list cell as a head structure's last argument is matched next without a register | 0.996 (browse 0.96) |

Tried and dropped (no gain beyond noise): a cheaper hash for the
first-argument index table (low bits instead of `hash.mix`); indexing on
the second to fourth argument when no clause has a first-argument key
(1.01); inline allocation of structures of arity 5-6; a functor-lookup
cache for `functor/3` (boyer unchanged); `--gc-gen` (1.01); lazy
sweeping in the non-generational heap (each chunk swept when the
allocator takes it, to keep it in cache: no gain); a sweep loop writing
one 8-byte header word per free slot (no gain: the loop waits on
memory); a smaller or larger minimum heap (512 KiB 1.02, 2 MiB 1.03; 1
MiB stays). Heap growth 1600% instead of 400% makes boyer 0.84 and
poly_10 0.89, but triples boyer's peak memory (30 MB to 89 MB; SWI uses
11 MB), so it was not taken.

**Where the time goes now** (sampler, final build): the collector
(`std.gc.collect`'s sweep, marking, root scanning) takes 3-29% (nreverse
27%, boyer and poly_10 29%, tak 26%, browse 24%; sendmore 3%, query
7%). Every variable is a heap object and every structure built in write
mode is garbage after backtracking, so the sweep is proportional to what
is allocated (item 6a); the rest is mostly clause selection (TRY,
`backtrack`: 10-25% in derive, log10, tak, meta_qsort) and the head
ops. Per benchmark, besides the collector: zebra is 44% general
`unify` (structures against partial structures), eval 43% the generic
evaluator (a 1000-deep expression built at run time), sendmore 31%
`=\=`, crypt 19% `mod`, boyer `arg/3`, `functor/3` and unification.

*Update (later on 2026-11-04):* std.gc no longer sweeps (mark bitmaps
and bump allocation through free runs; `docs/results.md`, "GC: mark
bitmaps instead of sweeping"). Prolog then took 0.885 of this round's
time by geometric mean; the collector's share fell to 8% on nreverse
and 21% on boyer, where finding free runs (`refill`) is what remains.

Friction this round: (a) adding `deref` to IDIV/REM's operands gave
both a stack frame (`-W handler_frame`: the `i48` division's overflow
panic was no longer proved dead behind the `y != -1` guard); dividing
as `i64` fixed it, but nothing but the warning said why. (b) GET_NESTED
needs its register's index (for `ctx.pend[r]`) as well as its value;
an operand kind gives one or the other, so it takes `r: imm` and reads
`ctx.xregs()[r]` itself, and sharing GET_STRUCT's body took an `inline
for` over the differing expressions. (c) The 64 KiB verifier memset was
invisible from the source: only the sampler (with return addresses for
libc time) found it. (d) A guard chain jumps into the middle of a
clause's verified code: `resume_at(word)` gives that resume point when
the word is an instruction boundary of the optimized code, which made
this safe to build without trusting the word index.

Checks: `check.sh` (110 runs: three dispatch modes, GC stress and
verify, `--gc-check-roots`), `cargo test --release`, and an ASan build
running the 11 tests (normally and under GC stress) and eight
benchmarks with two iterations each: all pass.

Compiler and std changes, and their effect elsewhere: the verifier
change touches every vm's generated `Vm__verify` (now a wrapper around
`Vm__verify_in` with a 256-byte stack bitmap or a calloc'd one), which
runs only when code is built. The std.gc change touches only
`std.gc.collect` (checked on Mini's release IR: no other function
differs); Mini and MiniJS benchmarks within noise.

Result: **0.63x SWI-Prolog's time, 0.15x Trealla's** by geometric mean
over the 30, against 0.83x and 0.20x for the code before in the same
run (a loaded machine: 0.79x and 0.19x in the quieter run of the first
round); 0.76 of the time before by geometric mean, from 0.44 (queens_8),
0.45 (query), 0.50 (sendmore) and 0.52 (sieve) to 0.97 (qsort,
serialise). Ours now takes less time than SWI-Prolog on 29 of the 30
(boyer 1.05x; sieve 0.90x). Table in bench/prolog/README.md.

## Optimizations, third round (2026-11-04)

Same method as the second round (sampler, `--disasm`, `LOIMI_GC_STATS`),
on a machine loaded by other agents' builds (load 6-8 on 8 threads):
wall-clock minima of 5-9 interleaved runs moved by up to ±10% between
two columns of the same binary, CPU time (user+sys) by more, and running
the builds concurrently did not help. Only changes well outside that
noise, or seen directly in the profile, were kept.

| Change | Effect (time, new/old) |
|---|---|
| A first clause whose first goal compares head arguments (or integers), when every other clause of the chain starts with a comparison of the same operands that excludes it (`X =< Y` / `X > Y`), runs as a guard chain: no choicepoint. Without it tak left a choicepoint per call, which kept every environment alive and trailed every binding (1.4 MB live, root scanning 10% of the time) | tak 0.57 |
| arg/3, functor/3 and compare/3 whose output variable is new write its register (ARG_V, FUNCTOR_R, FUNCTOR_W, COMPARE_V): no variable object made to be bound at once, no `builtins.call`; compare/3 no longer interns its result atom by name | boyer 0.76 (arg/3), then 0.69 (functor/3): 0.52 together; flatten 0.87, unify 0.95 |
| IDIV, MOD, REM divide as `u32` when both operands are non-negative and below 2^32 (64-bit `idiv` is several times slower on this Skylake) | MOD 21% to 14% of crypt's samples; crypt, query 0.95-0.97 |
| A clause whose head is distinct variables runs its leading comparisons on the argument registers before ALLOCATE; when the exclusion above commits and the only other clause starts with the complement, failure jumps past it | tak 0.97 |
| Minimum heap 2 MiB (was 1 MiB): without sweeping, a collection costs mostly its roots, and every collection marks the ~1000 code objects of the library and program again | 0.98 over the suite (4 MiB: poly_10 0.91, reducer 0.89, but nreverse 1.06) |

Tried and dropped:

- **Shallow backtracking** for first clauses whose head binds before a
  neck cut (d/3 in the derive family, qsort's partition): SH_TRY saved
  the argument registers and the trail height, raised the newest
  choicepoint's stamp so that every binding of an older variable was
  trailed, and NECK_CUT committed (restoring the stamp and dropping the
  trail entries of new variables); `backtrack` undid the bindings and
  went to the other clauses. Correct (the test clauses.pl covers it),
  but no faster: derive 0.99, log10 1.09, ops8 1.02, times10 1.05. A
  choicepoint here is 12 stores; the shallow form still does about 8,
  plus a trail entry for the output argument and its removal at the
  cut, and the same dispatches.
- **An iterative evaluator** for left-nested expressions (eval's
  `((1+1)+2)+...+1000`): the left spine on a stack, no recursion. eval
  unchanged (46% of samples in the evaluator either way): each node is
  a structure and a bound variable (`add/2` makes `Expr+N` with `Expr` a
  variable bound by the next call), two dependent loads per node.
- Var-var binding inline in `unify`'s argument loop, and SET_VOID /
  UNIFY_VOID making several variables inline (zebra): no change.
- The structural fix for variable objects (a variable as the cell of
  the structure it is in, the WAM's representation) was not attempted:
  a register would then hold a pointer into a structure, which std.gc
  does not trace (no interior pointers), and every op reading a register
  would have to handle it. The inline builtins above remove the
  variables that were made only to be bound at once; the rest (output
  arguments, nreverse's list tails, eval's `Expr`) need that fix.

What remains (sampler, final build): clause selection (TRY,
`backtrack`: 10-17% in the derive family, browse, mu, qsort), zebra
`unify` 47% (structures against structures, mostly failing on an early
argument), eval's evaluator 47% (pointer chasing), sendmore's `=\=` on
permanent variables 34%, queens_8's `=\=` 14%, crypt's MOD/IDIV 27%
(division by constant 10 could multiply by a reciprocal, if an op knew
its divisor at verification time), the collector 2-10% (poly_10,
browse, reducer).

Friction: (a) `-W handler_frame` caught the 32-bit division at once
(the division-by-zero panic of `x / y` is not proved dead by a guard in
the caller; an explicit `yy != 0` test next to the division fixed it).
(b) `inline_builtin`'s new-variable outputs needed the clause compiler's
variable state (`seen`, `perm`, a temporary for a permanent variable):
three small helpers (`is_new`, `dest_begin`, `dest_end`) made them one
pattern for arg/3, functor/3 and compare/3. (c) Measuring was the hard
part: no performance counters (`perf_event_paranoid` 3, no valgrind),
and a loaded machine.

Checks: `check.sh` (120 runs: the new test `clauses.pl` covers the
exclusion commit, the early comparisons, arg/3, functor/3, compare/3
and division against SWI-Prolog's output), `cargo test --release`, and
an ASan build running the 12 tests (normally and under GC stress) and
eleven benchmarks.

Result: **0.53x SWI-Prolog's time, 0.13x Trealla's** by geometric mean
over the 30, against 0.57x and 0.14x for the code before (commit
a739a25, with the sweep-free std.gc) in the same run; 0.92 of the time
before by geometric mean: tak 0.55, boyer 0.53, simple_analyzer 0.81,
flatten 0.82, reducer 0.86, unify 0.87, the rest 0.94-1.01 (no change
beyond noise). Ours now takes less time than SWI-Prolog on all 30
(boyer 0.52x, sieve 0.92x, reducer 0.76x). Table in
bench/prolog/README.md.

## The JIT build (2026-11-04)

After the rounds above the `--jit-stencils` build (`LOIMI_JIT=1`) took
1.11x the interpreter's time over the 29 programs other than sieve, and
48x on sieve. Both causes were in the JIT, and Prolog's source did not
change (design/copy-and-patch.md section 14):

- The clauses of a try chain are entered by `jump cache.code` (a
  `Wam.Resume`), which the JIT took for a return: it never counted, so
  only the chains and the clauses called directly were compiled
  (times10: 21% of the time in compiled code). Such jumps now count like
  calls (91%).
- Every asserted clause is a verified code object the JIT registers;
  the registry was a sorted array kept by `memmove`, and sieve's 700,000
  clauses made that quadratic (85% of its time). It is a treap now.

JIT build against the interpreter, CPU time, best of 3: 0.98 over the
29 (was 1.04 in the same run; unify 0.83, tak 0.83, mu and prover
1.16), sieve 1.30 (was 46). The JIT gains less here than on Mini (0.78):
the ops do most of their work in their bodies, and `backtrack` and
PROCEED, whose indirect jumps reach a few handlers in the interpreter,
reach one address per clause in compiled code (mu, which backtracks
most, is the slowest). `check.sh` (full, and `-j`, which `cargo test`
runs as `prolog_jit_stencils`) now runs the tests under the JIT with
`LOIMI_JIT_HOT=2`, normally and under GC stress.

Friction: none in the language; finding the first cause took a
`LOIMI_JIT_DEBUG` listing (which code objects were compiled), as the
statistics count compiles, not where the time is.

## Benchmarks (first round)

`bench/prolog/run.py -n 3`: wall-clock seconds, best of three
interleaved runs, startup included, on a shared 8-core x86-64 machine
(another agent was running builds and tests). SWI-Prolog 10.0.2 with `-O`,
Trealla 3.14.46, ours a release build with tail-call dispatch.

| benchmark | K | swipl s | trealla s | loimi s | loimi/swipl | loimi/trealla |
|---|--:|--:|--:|--:|--:|--:|
| boyer | 57 | 1.144 | 5.269 | 1.312 | 1.15 | 0.25 |
| browse | 41 | 1.048 | 6.726 | 0.910 | 0.87 | 0.14 |
| chat_parser | 160 | 1.102 | 3.833 | 0.952 | 0.86 | 0.25 |
| crypt | 7900 | 1.028 | 3.897 | 0.568 | 0.55 | 0.15 |
| derive | 290000 | 1.007 | 2.859 | 0.702 | 0.70 | 0.25 |
| divide10 | 630000 | 1.016 | 2.835 | 0.787 | 0.77 | 0.28 |
| eval | 11000 | 0.990 | 2.611 | 0.683 | 0.69 | 0.26 |
| fast_mu | 29000 | 1.082 | 4.253 | 0.891 | 0.82 | 0.21 |
| flatten | 48000 | 1.076 | 5.773 | 0.813 | 0.76 | 0.14 |
| log10 | 1300000 | 1.065 | 3.881 | 0.654 | 0.61 | 0.17 |
| meta_qsort | 4700 | 1.083 | 5.490 | 0.633 | 0.58 | 0.12 |
| mu | 30000 | 1.084 | 4.453 | 0.631 | 0.58 | 0.14 |
| nand | 1100 | 0.992 | 3.745 | 0.788 | 0.79 | 0.21 |
| nreverse | 83000 | 1.030 | 8.226 | 0.802 | 0.78 | 0.10 |
| ops8 | 920000 | 1.029 | 3.309 | 0.659 | 0.64 | 0.20 |
| perfect | 3900 | 1.018 | 5.033 | 0.857 | 0.84 | 0.17 |
| poly_10 | 510 | 1.033 | 2.830 | 0.990 | 0.96 | 0.35 |
| prover | 25000 | 1.041 | 3.674 | 0.590 | 0.57 | 0.16 |
| qsort | 47000 | 1.085 | 6.240 | 0.628 | 0.58 | 0.10 |
| queens_8 | 560 | 1.023 | 5.609 | 1.085 | 1.06 | 0.19 |
| query | 11000 | 1.034 | 4.938 | 1.086 | 1.05 | 0.22 |
| reducer | 640 | 1.040 | 4.916 | 0.994 | 0.96 | 0.20 |
| sendmore | 410 | 1.033 | 3.491 | 1.139 | 1.10 | 0.33 |
| serialise | 78000 | 1.081 | 5.387 | 0.721 | 0.67 | 0.13 |
| sieve | 61 | 1.028 | 2.764 | 2.086 | 2.03 | 0.75 |
| simple_analyzer | 1500 | 1.038 | 5.040 | 0.884 | 0.85 | 0.18 |
| tak | 160 | 1.147 | 4.014 | 0.660 | 0.58 | 0.16 |
| times10 | 670000 | 1.055 | 2.968 | 0.660 | 0.63 | 0.22 |
| unify | 13000 | 0.989 | 6.611 | 0.927 | 0.94 | 0.14 |
| zebra | 680 | 1.029 | 3.666 | 0.854 | 0.83 | 0.23 |
| geomean |  |  |  |  | 0.79 | 0.19 |

Ours takes less time than SWI-Prolog on 25 of the 30; not on sieve (2.03x), boyer (1.15x), sendmore (1.10x), queens_8 (1.06x), query (1.05x). Boyer does many functor/3 calls and permanent-variable moves per call; sieve is assert/retract (each `assertz` compiles and verifies a clause, each `retract` copies a stored clause term). Trealla takes 3-10x longer than ours on everything but sieve. The baseline (before the optimizations above, 29 programs) was 1.04x SWI-Prolog and 0.25x Trealla.