# Design note: the ergonomics audit (zero run-time cost)

> Where can the user get meaningfully better ergonomics at no run-time
> cost, even if it makes the Loimi implementation itself more complex?

The audit was written 2026-11-05 against 627c1ca, and its items were
built the same week. This note records what each item is, the pattern
in the examples it replaced, and what the IR comparison of the ports
showed. Item numbers are stable (CHANGELOG and `language.md` cite them).

**Method.** The audit read the six friction logs (`examples/*/FRICTION.md`,
about 3,500 lines; only entries still open or partly fixed counted) and
the sources of the nine interpreters
(`examples/{mini,minijs,scheme,lox,python,prolog,wasm,regex,stack}`),
counted recurring patterns with grep, and checked each candidate against
`language.md`, `status.md` and the design notes. Counts below are of the
sources at the time of the audit.

**What qualifies.** A feature counts only if the code it generates is the
code users wrote by hand, so that porting an example leaves its
`--release` code identical in all three dispatch modes (compared with
`loimi ir-diff`, item 4). Where the generated code is only provably no
worse, the item says so. The second condition is that the feature
removes lines, a class of bugs, or a concept the author must hold in
mind.

**Where the burden was.** Instruction decoding, object tracing, opcode
tables, disassemblers and stack effects were already generated, and the
lifetimes checker cost the nine interpreters 13 `unsafe { }` blocks and
no attributes. The rest fell into four groups:
- **(a) idioms a handler spelled out** to stay frame-free: GC polls with
  `redo`, allocation refills, `cold { raise .. }`;
- **(b) tables kept parallel to declarations** the compiler already
  had: builtin and exception name tables, op-to-op relations in the
  bytecode compilers, accumulator lists;
- **(c) layouts declared twice**: inline caches that mirror a std
  struct, context fields that a root scanner marks again;
- **(d) the build itself**: one error per build, and IR identity that
  depended on source line numbers.

## 1. The items

| # | Feature | Replaced | Documented in |
|---|---|---|---|
| 1 | `poll NAME { .. }`, `#[poll(NAME)]`, `poll NAME;`; `new` in a handler lowered to the inline path; the `redo` check | 46 pasted GC polls, 24 `try_new .. else cold { make_room; redo }` refills, Mini's 9 GIL and 6 pause polls | [language.md §4](../language.md#4-slow-paths-cold-blocks) |
| 2 | `#[name]`, `#[names]`, `#[data(key: T = default)]` on enum variants | ~650 rows of parallel name and data tables | [language.md §1.2](../language.md#variant-names-and-data) |
| 3 | vm items `rel kform(A) = B;` and `data NAME(A) = V;` | 178 relation rows in Wasm's translator, op maps in Mini, Prolog, Python | [language.md §7.5](../language.md#75-relations-and-data-on-ops-rel-and-data) |
| 4 | `loimi ir-diff` | IR identity that broke when a line moved | [status.md](../status.md#comparing-builds-loimi-ir-diff) |
| 5 | `cache X = S;`, `&cache` a `*S` | 22 `&cache.sid as *obj.Site` casts, caches copying `obj.Site` field by field | [language.md §7.1](../language.md#71-inline-caches) |
| 6 | errors and deoptimization cold by default, `#[hot]` to opt out | 138 `cold { raise .. }` / `cold { become ..; redo }` wrappers | [language.md §4](../language.md#4-slow-paths-cold-blocks) |
| 7 | every independent error of a build reported | one compile per error (Python: ~15 cycles for 10k lines) | status.md, Compiler row |
| 9 | `#[trace] struct`, typed `add_scanner<T>` | hand-written root scanners, 10 `as *u8` round trips | [language.md §8](../language.md#8-gc-integration) |
| 10 | `e is P`, `e as! V` | 36 `=> true, _ => false` predicates, 79 `let X(v) = e else { trap() }` | [language.md §1.2](../language.md#patterns-literals-and-match) |
| 11 | op selectors (`A..B`, globs, `with KIND`) in `accumulate` and `wraps`; `supers { .. }` groups | Wasm's 140-name `accumulate` list, Stack's hand list of jumping supers, 54 `#[allow(super_overlap)]` | [language.md §7.3, §7.4, §9](../language.md#73-superinstructions) |

Items 8 (effect inference) and 12 (a typed host-side decoder) were
rejected: section 3.

Unless noted, "ports" below compares the unported sources against the
ported ones, both built by the new compiler, with `loimi ir-diff
--release --asm`; "the compiler on unchanged sources" compares the
base compiler against the new one on the same sources.

### 1. Declared safepoint polls, and handler-lowered allocation refills

**Pattern.** A handler that may allocate checked the heap threshold and
collected in a `cold` block ending in `redo`, pasted into each op,
since a fall-through cold block in a plain vm function is not outlined.
Allocation in handlers was `try_new` with `else cold { make_room; redo
}`. In both, `redo` is correct only if nothing before it had a side
effect, and nothing checked that.

**As built.**
- **A poll is the author's text**, declared once (`poll gc { .. }`) and
  expanded by the parser where an op has `#[poll(gc)]` or writes `poll
  gc;`, so checking, lifetimes, cold outlining, stencils and `--vm-stats`
  see the written-out form. Nothing is appended: a poll ending in `redo`
  (Python, Scheme) and one whose cold block falls through and gets the
  rest of the op copied in (Mini, MiniJS, Stack, Prolog's METACALL) are
  both what the examples wrote. The statement form exists for Mini's GIL
  and pause polls, which sit in the taken branch of conditional jumps. A
  poll under a `#[cfg]` that is off expands to nothing.
- **The `redo` check covers every `redo`** (`src/redocheck.rs`, on the
  typed tree): an effect on a path to the `redo`, before the block that
  ends in it, is the error `redo_effect`. That block is the fix-up and is
  exempt (collecting, `make_room`, `become` + `refuse`, growing a stack,
  closing upvalues, flattening a rope). Calls are summarized by a least
  fixpoint (a store through a pointer, a call through a value, an extern
  taking a pointer); `std.gc` and `std.gil` are bookkeeping. One site
  needs `#[allow(redo_effect)]` (Python's `CALL_NEW`); the check found no
  real bug in the examples.
- **`new` in a handler** is lowered to the inline `try_new` path with a
  cold refill and `redo`, at a constant size of at most 496 bytes,
  outside loops and cold blocks. It computes the refill size, so a wrong
  size to `make_room` (an endless loop) cannot be written. A variable or
  large size is a call anyway (`memset`, `alloc_large`), and in a loop a
  refill would restart the loop. Polls and the `new` lowering therefore
  cover constant-size refills only: variable-length refills (Scheme's
  `try_closure`/`try_vect`, MiniJS's `try_obj`), fall-through polls in vm
  functions called from cold blocks (Python's and Lox's `gc_point()`),
  and Mini's barrier tails (mid-op, after a store) stay hand-written.

**Ports.**

| Interpreter | Ported | Code |
|---|---|---|
| Python | 23 `#[poll(gc)]` | same, 3 modes, also `--cfg debug` |
| Scheme | 9 polls; CONS, MKBOX `new` | same, 3 modes |
| MiniJS | 5 polls and a `poll gc;`; BOXNEW, REBOX `new` | same, 3 modes |
| Stack | 2 polls | same, 3 modes |
| Mini | 5 GC polls, 9 GIL, 6 pause | same, 3 modes, `--cfg gil`, `--cfg debug`, `--jit-stencils` |
| Prolog | 10 `try_cons`/`try_var` refills as `new`, METACALL's poll | handlers the same but an equivalent compare in PUT_LIST and PUT_VAR_X (`cmpq $31; jbe` for `cmpq $32; jb`) |

On unchanged sources one handler changed: Python's `MAKECELL`, whose
`new` is now lowered, lost its frame (47 assembly lines to 33).

### 2. Names and data on enum variants

**Pattern.** Builtins, exception kinds, operators and traps were an
`enum` or integer constants, each with parallel tables (name to variant,
variant to message, variant to dunder name) kept in step by hand: Scheme
had five copies of its 149 primitives, Python 27 `E_*` constants and an
`enames` array, Mini three copies of 27 natives.

**As built.** `#[data(min: i32 = 1, max: i32 = 1)]` on the enum declares
keys with types and defaults (a key's type cannot be inferred from `-1`
or `None`, and a default makes the common row empty); variants give
values by position or key, and `E.min(v)` etc. are generated, compiled
only if called. `name` and `from_name` are opt-in (`#[name]` on a
variant, or `#[names]`), so a hand-written `fn E.name` is an error
rather than a silent override. Data may be of enum and option type, so
a relation between two enums is data: Scheme's `Prim` carries `op: ?PO`
(replacing `inline_prim`'s 41-line name chain), Python's `Exc` carries
`parent: ?Exc`.

**Ports** (`--release --asm`, tail-call, against the sources at 3fc9376):
- **Scheme** (`enum Prim: i32`, 149 variants): 507 of 511 functions the
  same, every handler among them. Compile-time lookups became a `switch`
  on length and word compares instead of up to 41 `str.eq` calls;
  `call_native` keeps its shape with constants materialized in another
  order (at most one more `mov` on a builtin's path). Timing within
  noise (geomean 0.98 and 1.07 in two runs).
- **Python** (`#[names] enum Exc: u64`, 27 variants): 707 of 708 the
  same; `main.main`'s start-up loop reads `name()`/`parent()` instead of
  two arrays. Timing geomean 1.00.

### 3. Op relations and per-op data

**Pattern.** A bytecode compiler that quickens or fuses needs relations
between ops (constant form, branch form, fused form). The vm declares
those ops in `inline for` tables, so each relation was typed again in
the compiler as `(W.OP_X, W.OP_XK)` rows, searched linearly: Wasm 178
rows in seven tables, Mini's 24-arm `jump_op`/`negate_jump`, Prolog's
`jump_form`, Python's `branch_ops`.

**As built.** Vm items, not attributes on ops: `rel kform(#name) =
#(name ++ "K");` is the generated function written as an equation, in
the `inline for` body next to the ops, so one line declares the
relation for every row; `_` on either side declares nothing. Names
resolve after expansion against every op of the vm, so a relation may
cross tables (Wasm's K ops, `_X` loads and fused branches are separate
tables; merging them would renumber every opcode). `data commutative:
bool = false;` declares a key with a type and default, as item 2. The
lookups are generated as host code after the vm's other generated items,
so a `rel` moves no line of the expansion.

**Ports.**

| Interpreter | Replaced | Now | Lines |
|---|---|---|---|
| Wasm | 178 rows (`kforms`, `fusions`, `selects`, `kmems`, `rloads`, `tkloads`, `memop`'s `_X` column) | 23 `rel`/`data` lines | compile.lm −127, vm.lm +17 |
| Mini | `jump_op` 8 arms, `negate_jump` 16 | 6 `rel` lines | −44 |
| Prolog | `jump_form` (6) | one `rel` line | −5 |
| Python | `branch_ops` (12) | one `rel` line | −6 |

Every relation was checked against the table it replaced, opcode by
opcode, from the expansion (`LOIMI_DUMP_VM_EXPANSION=1`).

**What `ir-diff` showed** (items 3 and 11 together, three dispatch
modes, `--jit-stencils` for Mini and Prolog). The compiler on unchanged
sources: byte-identical `--emit-llvm` for all nine examples. The ports:
every vm handler, super, resume function and outlined cold block the
same in all nine; what differs is translator code calling the generated
lookups instead of materializing tables and searching them (Wasm's
`T.binop` 705 → 366 assembly lines, `T.cond_jump` 821 → 346). The op
set, opcodes, supers and accumulator variants are unchanged, so the
checked-in `.vmprofile` files are too. Wasm's spec suite (52,800
commands, every mode) and the 30 PolyBench kernels pass; timing geomean
1.001 for Wasm and Mini.

### 4. Panic locations out of the IR identity check: `loimi ir-diff`

**Pattern.** Panic messages carry `file:line:col`, so moving code by one
line changed the release IR, and the zero-cost discipline is "the
default build's code is identical". So every `#[cfg(debug)]` or
`#[cfg(gil)]` addition was written on an existing line or at the end of
the vm: Python's 26 `#[cfg(debug)]` all sit on existing lines
(`vm.lm:1522` holds the close of `on raise`, all of `on break` and an
`#[unfused] variant step`), and Mini's 71 `#[cfg(..)]` the same way
(`vm.lm:1040`, one 233-character line).

**As built.** `loimi ir-diff A B` compares two builds function by
function, ignoring source locations and names
([status.md](../status.md#comparing-builds-loimi-ir-diff)); every IR
comparison in this note uses it. The crammed `#[cfg]` lines were then
unpacked: Mini's `Ctx` fields (`vm.lm:73`, 233 characters), Python's
`on break` and `variant step` (`vm.lm:1497`, 268 characters) and the
two longest debug-only lines of the compilers (Mini's loop back-jump,
Python's `for` target stores) are now one item per line; `ir-diff
--release` reports the same code, in the default and `--cfg debug`
builds.

### 5. Cache layouts from structs

**Pattern.** `std.obj`'s inline-cache slow paths take a `*obj.Site`, so
Lox and MiniJS declared a `cache PropIC` whose fields copied
`obj.Site` and passed `&cache.sid as *obj.Site` (22 casts), relying on
the layouts staying equal; Python and Mini passed single field
addresses to plain functions.

**As built.** `cache X = S;` is the cache `cache X { f: T, .. }` with
`S`'s fields, filled in before expansion (`vmexpand::cache_structs`), so
word layout, `op_size`, emitters, verifier, `deopt`, disassembler and
`cache.f` are what the written-out cache gives. `&cache` is a `*S` only
where the checker proves the layouts agree (every field at its word's
byte offset, `S` no more aligned than a code word and no larger than
the words); otherwise it is an error naming the field. It compiles to
`&cache.f0 as *S` (`vmcheck::cache_addr`), the hand-written form, so the
lifetimes, `objref` and `redo` checks see the same tree; in a vm
function `&c` for a handle is cast the same way. Cache fields are
scalars: a struct field of array type is not a cache field.

**Ports** (three dispatch modes):
- **Lox**: 12 casts → `&cache`: 332 of 332 functions the same.
- **MiniJS**: 10 casts, and `prim_get` takes the `*obj.Site` instead of
  two `*u32`: 654 of 657 the same (the three differ by an instruction
  fewer).
- **Python**: GETGLOBAL's `cache GlobalIC = ops.GlobalSite;`, one
  pointer instead of three: 707 of 710 the same (`get_global` 331 → 322
  assembly lines).

Python's `MethIC`/`SupIC` (`u64` words at 4-byte offsets, not an `S` in
memory) and the one-field caches keep field addresses.

### 6. Terminal `raise`, `fail` and `become ..; redo` are cold by default

**Pattern.** A `raise` in a handler is the slow path by nature, yet
authors wrapped it in `cold { }` so that its arguments were outlined and
the fast path got no frame: 119 `cold { raise`, 21 `cold { become X;
redo }`.

**As built** ([language.md §4](../language.md#4-slow-paths-cold-blocks)).
- **Branches and tails.** An `if`/`match` arm or `let .. else` block
  ending in `raise`, `fail` or `become ..; redo` is cold, and so is the
  rest of a block that ends in one of those after a statement that can
  leave the block (a control word outside a `cold` block, a call of a
  `-> never` vm function, `return`, `break`, `continue`): most hand
  wrappers were the end of an op after its fast path left it. A control
  word inside a `cold` block does not count (what follows a poll is
  still the op's path), nor does `redo` without `become` (MiniJS's
  string member redoes on its fast path), nor `?`. A `raise` the code
  always reaches stays as written.
- **A tree rewrite before checking** (`src/autocold.rs`, after polls are
  expanded) produces exactly what the hand-written `cold { .. }` parses
  to, so every later pass sees the hand-written form.
  `LOIMI_COLD_AUDIT=1` lists the branches made cold.
- **`#[hot]` on the control word** keeps a branch inline; an unused one
  is an error. Prolog uses it on the two arms through which a guest
  `throw/1` raises: in switch mode, making them cold re-placed the
  dispatch loop and measured 3.1% slower; with `#[hot]` the code is the
  base compiler's.
- **Raises and outlined cold blocks pass `poison`** for the instruction
  word where nothing reads it (`vmgen::reads_insn`,
  `on_raise_reads_insn`; lowering.md §3). Without it, a cold raise in
  Prolog's CALLB kept the word alive across the builtin call and cost the
  hot path an instruction.

**Ports.** 138 wrappers removed (Lox 36, Wasm 30, Python 17, Scheme 16,
MiniJS 15, Stack 12, Mini 10, Prolog 2): every function the same in all
three modes for all eight, and in Mini's `--cfg debug`, `--cfg gil` and
`--jit-stencils` builds, Prolog's `--jit-stencils` and Python's `--cfg
debug`. Two tails stay hand-written because the rule would also have
moved type tests into the cold block (Mini's CALL, MiniJS's string-method
member).

**The compiler on unchanged sources.** 42 sites the hand had not written
cold became cold, and the instruction word is no longer passed where
unread. In tail-call mode 365 functions are shorter (e.g. Lox SUB 30 → 27
lines; Python LOADCELL 77 → 31; Scheme CALLCC 208 → 124), 20 the same
length and seven longer, none on a hot path. In goto and switch mode the
one function holding the handlers differs only by LLVM's placement and
register allocation. Timing (geomean new/base, min of 5 interleaved; the
build against itself gave 0.99 to 1.01):

| | tail-call | goto | switch |
|---|---:|---:|---:|
| Lox (10) | 1.000 | same code | same code |
| MiniJS (14) | 1.002 | 1.001 | 1.005 |
| Python (11) | 0.965 | 0.998 | 0.991 |
| Scheme (r7rs) | 1.002 | 1.002 | 1.008 |
| Mini (14) | 0.998 | same code | same code |
| Prolog (30) | 0.994 | 0.995 | same code with `#[hot]` (1.001) |
| Wasm (7) | 1.005; 0.988 again | 0.995 | 0.986 |
| Stack (5) | 1.028; 0.991 again | same code | same code |

### 7. Report more than one error per build

Function bodies, generic instances (first error each), vm handlers, vm
functions, entries, `on raise`, variant `before` blocks and the
statements of every body are checked past an error. A failed
statement's bindings are poisoned (type `never`), and errors in
statements that read them, or that enclose the failed one, are dropped.
After an error in an `if`/`while` condition or a `match` scrutinee the
branches are still checked. The `objref` and lifetimes passes report the
first error of each function. Output is in source order, one error per
place, capped by `--max-errors N` (default 20). Parsing, expansion,
imports, type definitions, signatures, impls and module-level `var`s
stop at their first error. Speculative checks (probes, a local closure
retried with effects) run without recovery, so a program that compiles
keeps the same locals. The release IR of the examples and of
`tests/run` was byte-identical to the base compiler's. Tests:
`tests/fail/multi_errors*.lm` (`// errors: N` checks the exact count).
Mutation trials on eight examples gave one error per break and no
cascades.

### 9. Derived root scanning; a typed `add_scanner`

**Pattern.** The interpreter's context (a plain struct holding `Gc`
fields, arrays and `Vec`s of values) was scanned by a hand-written
function, registered with `heap.add_scanner(&m as *u8, scan_roots)` and
cast back. A field added to the context and not to the scanner was a
silent missing root.

**As built** ([language.md §8](../language.md#8-gc-integration)).
- `#[trace]` on a plain struct generates `S.trace(s: *S, h: *gc.Heap)`
  (compiled only if called), expanded by the checker from the resolved
  field types into the statements a scanner writes by hand: `gc_mark`
  for an inline reference, a loop for a fixed array or a `mem.Vec`, the
  `next` loop for a `map.HashMap`, recursion into nested plain structs,
  in declaration order. `gc_mark(h, *s)` on the whole struct would load
  the context as one LLVM aggregate and miss the containers.
- **No silent miss.** A field holding references the generator cannot
  reach (another container, an array of `Vec`s, a raw pointer in a
  nested struct) is an error; `#[untraced]` says the scanner handles it.
- **Typed `add_scanner<T>`** (and `add_sweeper<T>`): `data: *T, scan:
  fn(*Heap, *T)`, converted inside std with `unsafe { scan as fn(*Heap,
  *u8) }` (a function type converts, in `unsafe` only, to one differing
  in raw pointer types). `as *u8` callers still compile (`T = u8`).
- `LOIMI_DUMP_VM_EXPANSION=1` lists what each `trace` marks.

**Ports** (tail-call): Python's `Ctx.trace` marks the 15 roots the hand
scanner did, in the same order (707 of 708 functions the same;
`scan_roots` differs in operand order of one `and`). Prolog's
`Machine.trace` marks its 7 fixed roots (444 of 445 the same;
`scan_roots` has the same 252 instructions in another order). Every
scanner and sweeper of the examples takes its own type. Python's and
Prolog's `check.sh` (stress, `--gc-verify`, `--gc-check-roots`, three
modes) and golden tests under `--sanitize=address` pass. Scheme keeps its
hand scanner, which deliberately marks `syms.all` but not `syms.map` or
`gsyms` (the same symbols); MiniJS, Lox, Mini and Stack mark mostly
stacks and frames.

### 10. `is` patterns and checked unwrap

**As built** ([language.md §1.2](../language.md#patterns-literals-and-match)).
- `e is P` tests a pattern that binds nothing. A variant with fields is
  named alone (`t is Ref`), and a name that is not a variant or constant
  is an error, so `t is Reff` cannot be always true. Precedence is that
  of `==`; `is` is not reserved.
- `e is A | B` is one `=> true` arm per alternative, the form Prolog's
  multi-variant predicates used, so they stay identical (LLVM lays out
  `A(_) | B(_) => true` differently).
- `e as! V` yields the variant's fields (one field its value, several a
  tuple); a variant without fields is an error. `let p = e as! V;` is
  checked as `let V(p) = e else { trap() };`, the hand-written TIR. In
  `--release` the failure is `trap()`; in a debug build a panic with the
  location and variant (`main.lm:12:13: as! Str of another variant`).

**Ports** (tail-call): Python's 78 unwraps, 6 predicates and 2 empty-arm
tests: 708 of 708 functions the same. Prolog's 18 predicates: 445 of 445.
MiniJS (7), Scheme (4), Regex (1): all the same. Mini (1 predicate): 624
of 626, the two differing in its bytecode compiler, one instruction
shorter.

### 11. Op selectors and `super` groups

**Pattern.** Vm declarations listed ops by name, kept in step with the op
set by hand: Wasm's 140-name `accumulate` list, Stack's budget variant
naming every super containing a jump (a new super was silently dropped
from the budget), ~140 supers nearly all named as their parts joined,
~100 `#[forward]`, 54 `#[allow(super_overlap)]`.

**As built** ([language.md §7.3, §7.4, §9](../language.md#73-superinstructions)).
- **Selectors:** a name, `A..B` (the ops declared from A to B), a glob
  (`BR_I32_*`) and `with KIND`, in `accumulate` and `wraps [..]`; the
  list is their union. Ranges do most of the work: Wasm's list is runs
  of declaration order, not a predicate. A selector naming no op is an
  error; `LOIMI_DUMP_VM_EXPANSION=1` prints each resolved list.
- **A wrapped op wraps the supers containing it**, and a wrapped super
  its `#[forward]` variants (the second fixed a latent bug: a forward
  super's forwarding variant was left out; `tests/run/vm_rel_data.lm`).
- **Groups:** `supers { A, B; NAME = C, D; }`; attributes on the group
  apply to every entry, an unnamed entry is named by its parts joined
  with `_`, and declaration order (so every opcode) is unchanged.

**Ports.** Wasm's `accumulate` list is 56 selectors in 14 lines, resolving
to the hand list's set. 124 of 137 supers are in groups;
`#[allow(super_overlap)]` 54 → 15. Stack's budget is `wraps [JMP, JZ,
LOOP, CALL]`. Code and op set the same in all nine (see item 3).

## 2. Diagnostics

- **One error per build:** item 7.
- **Literal-only expressions** (python FRICTION 577): an `if`, `match`
  or block whose results are all literals is typed from the other
  operand of a binary operator, `(if x { 1 } else { 0 }) == y` with `y:
  i32`. The right operand is checked first with the type the left would
  have had alone, so programs that compiled are typed as before. A loop
  variable is typed from its range, not from later uses: `for j in
  0..3` used as a `u32` is an error whose note suggests `for j: u32 in
  ..`.
- **`-W handler_frame` names the panic behind a frame.** Prolog
  FRICTION 453: adding a `deref` stopped LLVM proving an `i48`
  division's overflow panic dead, and the warning only said "a panic".
  Built: codegen keeps a panic-site table (message text → "the overflow
  check of `/` at vm.lm:512", "the zero check of `%`", "the bounds check
  of an index", "an `.unwrap()` of `None`", ...), returned with the IR,
  and `framelint.rs` names each panic call from it; panic calls LLVM
  merged into one list the function's panic sites as candidates. The
  generated code does not change. `hot_gc` still does not see calls
  inside plain vm functions (Lox FRICTION 219).
- **Operands map to format fields by position**, so `op DELGLOBAL(z:
  imm, name: kst) : ABx` needed a dummy operand (Python FRICTION 579).
  Built: `name: kst @ bx` names the field (`language.md` §3, formats;
  `vmexpand::param_fields`), and the encoder-mismatch note mentions it.
  Ported: Mini's 4 compare-and-jump ops (`a: reg @ b, b: bkind @ c`),
  Scheme's CHKGLOBAL, Python's DELGLOBAL, MiniJS's THROWREF, and Wasm's
  trailing `z: imm` on the lane stores (just dropped). Python's
  SUPERMETHOD `z` is a real flag and stays. `loimi ir-diff --release`:
  the handlers have the same code; what changes is the disassembler,
  `operand_name` and verify-error text, which no longer know the dummy
  operands (and in Mini the name of an accumulator spill slot).

## 2a. Smaller items

- **`v.index_of(pc)` on `Verified<Vm>`.** Built (`language.md` §6,
  generated with the other `Verified` methods in `vmexpand.rs`): the
  word index of a pointer into the code, with the vm's word size. It
  replaced 17 copies of `((pc as u64) - (start as u64)) / 4` in Mini,
  Mini's debugger, Scheme, MiniJS, Lox, Python, Python's debugger, Stack
  and tutorial steps 6–9; `loimi ir-diff --release`: same code.
- **`become match st { OwnIn => GETPROP_IN, .. }`.** Built
  (`language.md` §7.2): sugar in the parser for the ladder `if st is
  OwnIn { become GETPROP_IN; } ..`, and `x is A | B` on a local of a
  payload-less enum is checked as `x == A or x == B`, the comparisons
  the ladders had (as a `match` it came out as a differently ordered
  switch in Lox's INVOKE_MISS). Lox's 6 and MiniJS's 5 multi-arm
  ladders (40 lines) were ported; `loimi ir-diff --release` reports the
  same code for both. One-arm tests stay plain `if`s.

## 3. Considered and rejected

- **Implicit outlining of panics in handlers** (unwrap, bounds checks):
  measured, it removed the frame of 1 of 218 handlers (`language.md`
  §4). Cold `raise` (item 6) is the part that pays.
- **Making `?`'s error path cold by default:** 137 sites on hot calls;
  moving the arm changes layout in every handler using it, neither
  identical nor obviously no worse.
- **`new_with(h, T { .. }, n, |i| ..)`** to remove the 5 `unsafe { new(..,
  n) }` of `Gc` arrays filled right after: too few sites for a feature.
- **Parameterized operand kinds** (Wasm's 16 kinds differing only in
  `T`): `inline for` among vm items already generates them.
- **`comptime` / `const fn`**: items 2 and 3 cover the tables the
  examples keep, without an interpreter in the compiler.
- **Automatic superinstruction and accumulator selection**: it changes
  code by design, so it is a speed feature, not zero-cost ergonomics;
  item 11 removes the list-keeping.
- **Trailing arrays in plain structs** (Prolog frames): 4 helpers and one
  operand kind; too few sites.
- **A `sync` shorthand in `state { .. }`**: `sync`/`reload` are one or
  two lines per vm.
- **Derived state registers** (`regs = fp.base` reloaded whenever `fp`
  is assigned): a `-> never` vm function already factors the 17 pop
  sequences with identical code, and the derived form hides which
  registers are reloaded where, which authors tune.
- **Item 8, inferring the effects of private functions** (136 `uses` on
  signatures): no friction log complained, the missing-effect error says
  what to add, and it needed reordering the checker's passes. Effects
  are declared, not inferred.
- **Item 12, a typed host decoder `Vm.decode(w)` and range-checked
  encoders**: host-side only (42 accessor calls); the ~300 `as u8`
  casts at encoder calls are cosmetic.
- **`extend vm` blocks** (item 4's second step, moving debugger hooks to
  `debugger.lm`): name-resolution questions across modules for a benefit
  that `ir-diff` mostly removes.
- **Wasm's translator type columns as vm `data`** (329 `numeric()`
  rows): the types do not follow from operand kinds (`F32_NEG` works on
  `ru`), so the rows would move into the vm rather than disappear, and
  need a generated inverse lookup.
- **`ops that jump` and `A - B` selectors**: Stack's budget wraps `JMP,
  JZ, LOOP, CALL` but not `DO`, which jumps forward only; the list is a
  judgement that four names state.

## 4. Verifying "zero cost"

Every item was verified the same way: port one or more examples, build
them with `--release` in `tailcall`, `goto` and `switch` dispatch, with
and without `--jit-stencils` and `--cfg debug` where the example has
them, and compare with `loimi ir-diff` against the unported sources. The
result should be every function the same. Item 6 is no worse rather than
identical where the existing code was not already in the hand-written
form; there the check is the same comparison on the sites that were,
`-W handler_frame` and a benchmark run on the sites that were not.
