# Design note: the ergonomics audit (zero run-time cost)

The audit asked: where can interpreter authors get clearly better
ergonomics at no run-time cost, even if the compiler gets more complex?
This note lists each item, what was decided, and, for the built items,
how they work inside and what the comparison of the ported examples
showed. Item numbers are stable; other documents cite them.

All built items were built in the week of 2026-11-05. How to use them is
in `language.md`.

## Method

The audit read the open entries of the six friction logs
(`examples/*/FRICTION.md`) and the sources of the nine example
interpreters, counted recurring patterns with grep, and checked each
candidate against the docs. Counts below are of the sources at the time.

A feature qualified only if both hold:

- **Zero cost.** The generated code is the code users wrote by hand, so
  porting an example leaves its `--release` code identical in all three
  dispatch modes (compared with `loimi ir-diff`, item 4). Where the code
  is only provably no worse, the item says so.
- **A real gain.** It removes lines, a class of bugs, or something the
  author must keep in mind.

Decoding, object tracing, opcode tables, disassemblers and stack effects
were already generated. The remaining burden fell into four groups:

- idioms a handler spelled out to stay frame-free: GC polls with `redo`,
  allocation refills, `cold { raise .. }`;
- tables kept in step with declarations the compiler already had: name
  tables, op-to-op relations, accumulator lists;
- layouts declared twice: inline caches that copy a std struct, context
  fields that a root scanner marks again;
- the build itself: one error per build, and IR identity that depended
  on line numbers.

## 1. The items

| # | Feature | What it replaced | Verdict |
|---|---|---|---|
| 1 | `poll NAME { .. }`, `#[poll(NAME)]`, `poll NAME;`; `new` in a handler; the `redo` check | 46 pasted GC polls, 24 hand-written allocation refills, Mini's 15 GIL and pause polls | Built; same code |
| 2 | `#[name]`, `#[names]`, `#[data(key: T = default)]` on enum variants | ~650 rows of parallel name and data tables | Built; same handlers |
| 3 | `rel kform(A) = B;` and `data NAME(A) = V;` in vm declarations | 178 relation rows in Wasm's translator; op maps in Mini, Prolog, Python | Built; same handlers |
| 4 | `loimi ir-diff` | IR identity that broke when a line moved | Built |
| 5 | `cache X = S;`, `&cache` as a `*S` | 22 `&cache.sid as *obj.Site` casts | Built; same code, or one instruction fewer |
| 6 | Errors and deoptimization cold by default; `#[hot]` to opt out | 138 `cold { raise .. }` and `cold { become ..; redo }` wrappers | Built; ports same code, unchanged sources no worse |
| 7 | Every independent error of a build reported | One compile per error | Built; same code |
| 8 | Inferring effects of private functions | — | Rejected (section 3) |
| 9 | `#[trace] struct`, typed `add_scanner<T>` | Hand-written root scanners, 10 `as *u8` round trips | Built; same roots |
| 10 | `e is P`, `e as! V` | 36 `=> true, _ => false` predicates, 79 `let X(v) = e else { trap() }` | Built; same code |
| 11 | Op selectors (`A..B`, globs, `with KIND`) in `accumulate` and `wraps`; `supers { .. }` groups | Wasm's 140-name `accumulate` list, hand lists of supers, 54 `#[allow(super_overlap)]` | Built; same code |
| 12 | A typed host-side decoder | — | Rejected (section 3) |

User docs: items 1 and 6 in `language.md` section 4; items 2 and 10 in
section 1.2; item 3 in section 7.5; item 5 in section 7.1; item 9 in
section 8; item 11 in sections 7.3, 7.4 and 9; item 4 in `status.md`.

The notes below cover how each item is built. "Ports" compares the
unported and ported sources, both built by the new compiler, with
`loimi ir-diff --release --asm`. "Unchanged sources" compares the old and
new compiler on the same sources.

### 1. Declared safepoint polls, and handler-lowered allocation refills

**Problem.** A handler that may allocate checked the heap threshold and
collected in a `cold` block ending in `redo`, pasted into each op.
Allocation was `try_new .. else cold { make_room; redo }`. Both are
correct only if nothing before the `redo` had a side effect, and nothing
checked that.

**Polls.** A poll is declared once (`poll gc { .. }`). The parser expands
it where an op has `#[poll(gc)]` or writes `poll gc;`, so every later
pass sees the written-out form. Nothing is added: a poll ending in
`redo` and one whose cold block falls through are both what the examples
wrote. The statement form is for Mini's GIL and pause polls, which sit
in the taken branch of conditional jumps. A poll under a `#[cfg]` that is
off expands to nothing.

**The `redo` check** (`src/redocheck.rs`, on the typed tree) covers every
`redo`:

- An effect on a path to the `redo`, before the block that ends in it,
  is the error `redo_effect`.
- That last block is the fix-up and is exempt: collecting, `make_room`,
  `become` + `refuse`, growing a stack, and so on.
- Calls are summarized by a least fixpoint. Effects are a store through
  a pointer, a call through a value, or an extern taking a pointer.
  `std.gc` and `std.gil` count as bookkeeping.

One site needs `#[allow(redo_effect)]` (Python's `CALL_NEW`). The check
found no real bug in the examples.

**`new` in a handler** is lowered to the inline `try_new` path with a cold
refill and `redo`, when the size is a constant of at most 496 bytes and
the `new` is outside loops and cold blocks. The compiler computes the
refill size, so a wrong size passed to `make_room` (an endless loop)
can't be written. A variable or large size is a call anyway, and in a
loop a refill would restart the loop. These stay hand-written:

- variable-length refills (Scheme's `try_closure`/`try_vect`, MiniJS's
  `try_obj`);
- polls in vm functions called from cold blocks (Python's and Lox's
  `gc_point()`);
- Mini's barrier tails (in the middle of an op, after a store).

**Result.** Python, Scheme, MiniJS, Stack and Mini: the same code in
three modes (Mini also with `--cfg gil`, `--cfg debug` and
`--jit-stencils`). Prolog: the same except an equivalent compare in two
handlers (`cmpq $31; jbe` for `cmpq $32; jb`). On unchanged sources,
Python's `MAKECELL` lost its frame (47 assembly lines to 33).

### 2. Names and data on enum variants

**Problem.** Builtins, exception kinds and operators had parallel tables
kept in step by hand. Scheme had five copies of its 149 primitives,
Python 27 `E_*` constants and a name array, Mini three copies of 27
natives.

**Design.**

- `#[data(min: i32 = 1, max: i32 = 1)]` on the enum declares keys with
  types and defaults. A key's type can't be inferred from `-1` or `None`,
  and a default makes the common row empty.
- `E.min(v)` and the like are generated, and compiled only if called.
- `name` and `from_name` are opt-in (`#[name]` on a variant, or
  `#[names]`), so a hand-written `fn E.name` is an error rather than a
  silent override.
- Data may be of enum and option type, so a relation between two enums
  is data. Scheme's `Prim` carries `op: ?PO`; Python's `Exc` carries
  `parent: ?Exc`.

**Result** (tail-call). Scheme: 507 of 511 functions the same, every
handler among them. Lookups by name became a `switch` on length and word
compares instead of up to 41 `str.eq` calls. Python: 707 of 708 the same.
Timing within noise.

### 3. Op relations and per-op data

**Problem.** A bytecode compiler that quickens or fuses ops needs
relations between them (constant form, branch form, fused form). The vm
declares those ops in `inline for` tables, so each relation was typed
again in the compiler as rows searched linearly: 178 rows in Wasm, a
24-arm function in Mini, smaller ones in Prolog and Python.

**Design.**

- These are vm declarations, not attributes on ops. `rel kform(#name) =
  #(name ++ "K");` is the generated function written as an equation,
  inside the `inline for` body, so one line covers every row. `_` on
  either side declares nothing.
- Names resolve after expansion against every op of the vm, so a
  relation may cross tables. Wasm keeps its K ops, loads and fused
  branches in separate tables; merging them would renumber every opcode.
- `data commutative: bool = false;` declares a key with a type and a
  default, as in item 2.
- The lookups are generated as host code after the vm's other generated
  items, so a `rel` moves no line of the expansion.

**Result.** Wasm: 178 rows became 23 lines (compile.lm −127 lines). Mini
−44, Prolog −5, Python −6. Each relation was checked against the table it
replaced, opcode by opcode. Items 3 and 11 together:

- Unchanged sources: byte-identical `--emit-llvm` for all nine examples.
- Ports: every handler, super, resume function and outlined cold block
  the same in all nine. Only translator code differs, because it calls
  the generated lookups instead of searching tables (Wasm's `T.binop` 705
  → 366 assembly lines).
- Wasm's spec suite and the PolyBench kernels pass; timing geomean 1.001
  for Wasm and Mini.

### 4. Panic locations out of the IR identity check: `loimi ir-diff`

**Problem.** Panic messages carry `file:line:col`, so moving code by one
line changed the release IR. To keep the default build's code identical,
every `#[cfg(debug)]` addition was crammed onto an existing line: Python
had a 268-character line, Mini a 233-character one.

**Design.** `loimi ir-diff A B` compares two builds function by function,
ignoring source locations and names (described in `status.md`). Every IR
comparison in this note uses it. The crammed lines were then split, one
item per line, and `ir-diff --release` reports the same code in the
default and `--cfg debug` builds.

### 5. Cache layouts from structs

**Problem.** `std.obj`'s inline-cache slow paths take a `*obj.Site`. Lox
and MiniJS declared caches whose fields copied `obj.Site`'s and cast
`&cache.sid as *obj.Site` (22 casts), relying on the layouts staying
equal.

**Design.**

- `cache X = S;` is the cache `cache X { f: T, .. }` with `S`'s fields,
  filled in before expansion (`vmexpand::cache_structs`). Everything
  downstream sees the written-out cache.
- `&cache` is a `*S` only where the checker proves the layouts agree:
  every field at its word's byte offset, and `S` no more aligned than a
  code word and no larger than the words. Otherwise it is an error naming
  the field.
- It compiles to `&cache.f0 as *S` (`vmcheck::cache_addr`), the
  hand-written form, so the lifetimes, `objref` and `redo` checks see the
  same tree.
- Cache fields are scalars: a struct field of array type is not a cache
  field.

**Result.** Lox: all 332 functions the same. MiniJS: 654 of 657 the same,
the other three one instruction shorter. Python: 707 of 710 the same.
Caches that are not an `S` in memory (Python's `MethIC`/`SupIC`) keep
field addresses.

### 6. Terminal `raise`, `fail` and `become ..; redo` are cold by default

**Problem.** A `raise` in a handler is the slow path, yet authors wrapped
it in `cold { }` so its arguments were outlined and the fast path got no
frame: 119 `cold { raise`, 21 `cold { become X; redo }`.

**Which code becomes cold.**

- An `if`/`match` arm or `let .. else` block that ends in `raise`, `fail`
  or `become ..; redo`.
- The rest of a block that ends in one of those, after a statement that
  can leave the block: a control word outside a `cold` block, a call of a
  `-> never` vm function, `return`, `break` or `continue`. Most hand
  wrappers were the end of an op after its fast path left it.
- Not: a control word inside a `cold` block (what follows a poll is still
  the op's path), `redo` without `become`, `?`, or a `raise` the code
  always reaches.

**Design.**

- A tree rewrite before checking (`src/autocold.rs`, after polls are
  expanded) produces exactly what the hand-written `cold { .. }` parses
  to, so every later pass sees the hand-written form.
- `LOIMI_COLD_AUDIT=1` lists the branches made cold.
- `#[hot]` on the control word keeps a branch inline; an unused one is an
  error. Prolog uses it on the two arms through which a guest `throw/1`
  raises: cold, they measured 3.1% slower in switch mode.
- Raises and outlined cold blocks pass `poison` for the instruction word
  where nothing reads it (`vmgen::reads_insn`, `on_raise_reads_insn`).
  Without this, a cold raise in Prolog's CALLB kept the word alive across
  a call and cost the hot path an instruction.

**Result.** Ports: 138 wrappers removed across eight examples, every
function the same in all three modes and the examples' extra build
configurations. Two tails stay hand-written, because the rule would also
have moved type tests into the cold block.

On unchanged sources, 42 sites the authors had not written as cold
became cold. In tail-call mode 365 functions are shorter (Python
LOADCELL 77 → 31 lines), 20 the same length and seven longer, none on a
hot path. In goto and switch mode only LLVM's placement and register
allocation differ. Timing, as geomean new/old over each benchmark set
(the build against itself gives 0.99 to 1.01):

| | tail-call | goto | switch |
|---|---:|---:|---:|
| Lox | 1.000 | same code | same code |
| MiniJS | 1.002 | 1.001 | 1.005 |
| Python | 0.965 | 0.998 | 0.991 |
| Scheme | 1.002 | 1.002 | 1.008 |
| Mini | 0.998 | same code | same code |
| Prolog | 0.994 | 0.995 | same code with `#[hot]` |
| Wasm | 1.005, then 0.988 | 0.995 | 0.986 |
| Stack | 1.028, then 0.991 | same code | same code |

### 7. Report more than one error per build

**What continues past an error.** Function bodies, generic instances
(first error each), vm handlers, vm functions, entries, `on raise`,
variant `before` blocks, and each statement of a body.

**What stops at its first error.** Parsing, expansion, imports, type
definitions, signatures, impls and module-level `var`s.

**Avoiding follow-on errors.**

- A failed statement's bindings get type `never`. Errors in statements
  that read them, or that enclose the failed one, are dropped.
- After an error in an `if`/`while` condition or a `match` scrutinee, the
  branches are still checked.
- The `objref` and lifetimes passes report the first error of each
  function.
- Speculative checks (probes, a closure retried with effects) run
  without recovery, so a program that compiles keeps the same locals.

Output is in source order, one error per place, capped by
`--max-errors N` (default 20). The release IR of the examples and
`tests/run` was byte-identical to the old compiler's. Mutation trials on
eight examples gave one error per break and no cascades. Tests:
`tests/fail/multi_errors*.lm` (`// errors: N` checks the exact count).

### 9. Derived root scanning; a typed `add_scanner`

**Problem.** The interpreter's context struct was scanned by a
hand-written function, registered with `heap.add_scanner(&m as *u8,
scan_roots)`. A field added to the context and not to the scanner was a
silent missing root.

**Design.**

- `#[trace]` on a plain struct generates `S.trace(s: *S, h: *gc.Heap)`,
  compiled only if called. The checker expands it from the resolved
  field types into what a scanner writes by hand, in declaration order:
  `gc_mark` for an inline reference, a loop for a fixed array or a
  `mem.Vec`, the `next` loop for a `map.HashMap`, and recursion into
  nested plain structs. (`gc_mark(h, *s)` on the whole struct would load
  it as one LLVM aggregate and miss the containers.)
- A field holding references the generator can't reach is an error.
  `#[untraced]` says the hand-written scanner handles it.
- `add_scanner<T>` and `add_sweeper<T>` take `data: *T, scan: fn(*Heap,
  *T)`. Inside std they convert with `unsafe { scan as fn(*Heap, *u8) }`:
  in `unsafe` only, a function type converts to one that differs in raw
  pointer types. Old `as *u8` callers still compile (`T = u8`).
- `LOIMI_DUMP_VM_EXPANSION=1` lists what each `trace` marks.

**Result.** Python's `Ctx.trace` marks the same 15 roots as the hand
scanner, in the same order (707 of 708 functions the same). Prolog's
marks its 7 fixed roots (444 of 445 the same). Both pass their GC stress
and root checks. Scheme keeps its hand scanner, which on purpose skips
two tables that hold the same symbols as a third.

### 10. `is` patterns and checked unwrap

**Design.**

- `e is P` tests a pattern that binds nothing. A variant with fields is
  named alone (`t is Ref`). A name that isn't a variant or constant is an
  error, so `t is Reff` can't be always true. `is` has the precedence of
  `==` and is not a reserved word.
- `e is A | B` becomes one `=> true` arm per alternative, the form the
  examples used, so their code stays the same. (LLVM lays out
  `A(_) | B(_) => true` differently.)
- `let p = e as! V;` is checked as `let V(p) = e else { trap() };`, the
  hand-written form. In a debug build the failure is a panic with the
  location and variant.

**Result.** Python (86 sites), Prolog (18), MiniJS, Scheme and Regex: all
functions the same. Mini: two functions in its bytecode compiler one
instruction shorter.

### 11. Op selectors and `super` groups

**Problem.** Vm declarations listed ops by name, kept in step with the op
set by hand. Stack's budget variant named every super containing a jump,
so a new super was silently left out of the budget.

**Design.**

- A selector is a name, `A..B` (the ops declared from A to B), a glob
  (`BR_I32_*`) or `with KIND`. `accumulate` and `wraps [..]` take a list
  of selectors and use their union. A selector naming no op is an error.
- A wrapped op also wraps the supers containing it, and a wrapped super
  its `#[forward]` variants. The second fixed a real bug: a forward
  super's forwarding variant was left out (`tests/run/vm_rel_data.lm`).
- `supers { A, B; NAME = C, D; }`: attributes on the group apply to each
  entry, and an unnamed entry is named by its parts joined with `_`.
  Declaration order, and so every opcode, is unchanged.
- `LOIMI_DUMP_VM_EXPANSION=1` prints each resolved list.

**Result.** Wasm's `accumulate` list is 56 selectors in 14 lines, with the
same set as the hand list. 124 of its 137 supers are in groups, and
`#[allow(super_overlap)]` went from 54 to 15. Same code in all nine
examples (see item 3).

## 2. Diagnostics and smaller items

These were built alongside the numbered items. Each was ported where it
applied, and `loimi ir-diff --release` reported the same handler code.

| Item | Problem | What was built |
|---|---|---|
| Literal-only expressions | `(if x { 1 } else { 0 }) == y` with `y: i32` didn't type | An `if`, `match` or block whose results are all literals is typed from the other operand. The right operand is checked first with the type the left would have alone, so programs that compiled are typed as before. A `for` variable is typed from its range; using it as another type is an error suggesting `for j: u32 in ..`. |
| `-W handler_frame` names the panic | The warning only said "a panic" | Codegen keeps a table from panic message to its source ("the overflow check of `/` at vm.lm:512"), and `framelint.rs` names each panic call from it. Panic calls LLVM merged list all candidates. Generated code doesn't change. |
| Operand to field by name | `op DELGLOBAL(z: imm, name: kst) : ABx` needed a dummy operand | `name: kst @ bx` names the format field (`vmexpand::param_fields`). Ported in Mini, Scheme, Python, MiniJS and Wasm. |
| `v.index_of(pc)` on `Verified<Vm>` | 17 copies of `((pc as u64) - (start as u64)) / 4` | Generated with the other `Verified` methods in `vmexpand.rs`; uses the vm's word size. |
| `become match st { .. }` | Ladders of `if st is A { become X; } ..` | Parser sugar for the ladder. `x is A \| B` on a payload-less enum is checked as `x == A or x == B`, as the ladders had (a real `match` gave a differently ordered switch). One-arm tests stay plain `if`s. |

## 3. Considered and rejected

| Idea | Why not |
|---|---|
| Outlining panics in handlers implicitly | Measured: removed the frame of 1 of 218 handlers. Cold `raise` (item 6) is what pays. |
| `?`'s error path cold by default | 137 sites on hot calls; moving the arm changes layout in every handler using it, neither identical nor clearly no worse. |
| `new_with(h, T { .. }, n, \|i\| ..)` | Would remove 5 `unsafe` blocks; too few sites. |
| Parameterized operand kinds | `inline for` among vm items already generates them. |
| `comptime` / `const fn` | Items 2 and 3 cover the examples' tables without an interpreter in the compiler. |
| Automatic superinstruction and accumulator selection | Changes code by design: a speed feature, not zero-cost ergonomics. Item 11 removes the list-keeping. |
| Trailing arrays in plain structs | 4 helpers and one operand kind; too few sites. |
| A `sync` shorthand in `state { .. }` | `sync`/`reload` are one or two lines per vm. |
| Derived state registers (reload `regs` whenever `fp` is assigned) | A `-> never` vm function already shares the 17 pop sequences with the same code, and the derived form hides which registers are reloaded where, which authors tune. |
| Item 8: inferring effects of private functions | No friction log asked for it, the missing-effect error says what to add, and it needed the checker's passes reordered. |
| Item 12: a typed host decoder and range-checked encoders | Host side only (42 accessor calls); the ~300 `as u8` casts at encoder calls are cosmetic. |
| `extend vm` blocks (debugger hooks in another file) | Name-resolution questions across modules, for a gain `ir-diff` mostly removes. |
| Wasm's translator type columns as vm `data` | The types don't follow from operand kinds, so 329 rows would move into the vm rather than disappear. |
| `ops that jump` and `A - B` selectors | Which ops count is a judgement (Stack's budget leaves out the forward-only `DO`); four names state it. |

## 4. Verifying "zero cost"

Each item was checked the same way:

1. Port one or more examples.
2. Build them with `--release` in tail-call, goto and switch dispatch,
   and with `--jit-stencils` and `--cfg debug` where the example has
   them.
3. Compare with `loimi ir-diff` against the unported sources. Every
   function should be the same.

Item 6 is "no worse" rather than identical on code that wasn't already
in the hand-written form. There the check was `-W handler_frame` and a
benchmark run.
