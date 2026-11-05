# Design note: an ergonomics audit (zero run-time cost)

Status: **proposal; items 1, 2, 4 (step 1), 5, 6, 7, 9 and 10, and part of section 2, built.** Written 2026-11-05
against master at 627c1ca.
- **Item 1** (declared polls, `new` in handlers, the `redo` check):
  built as `poll NAME { .. }`, `#[poll(NAME)]` and `poll NAME;`
  ([language.md §4](../language.md#4-slow-paths-cold-blocks)); six
  interpreters are ported (see the item).
- **Item 2** (names and data on enum variants): built as `#[name]`,
  `#[names]` and `#[data(key: Type = default)]`
  ([language.md §1.2](../language.md#variant-names-and-data)). Scheme's
  primitives and Python's exception kinds are ported; the section below
  records what changed from the proposal and what the IR diff showed.
- **Item 4, step 1** (`loimi ir-diff`): built
  ([status.md](../status.md#comparing-builds-loimi-ir-diff)). The
  examples' `#[cfg]` lines are not unpacked yet.
- **Item 5** (cache layouts from structs): built as `cache X = S;`, with
  `&cache` (and `&c` for a handle) a `*S` where the words hold an `S`
  ([language.md §7.1](../language.md#71-inline-caches)); Lox, MiniJS and
  Python are ported (see the item).
- **Item 6** (errors and deoptimization cold by default): built, for
  branches and for the rest of a block after a statement that leaves
  it, with `#[hot]` to opt out
  ([language.md §4](../language.md#4-slow-paths-cold-blocks)); 138
  wrappers removed in eight interpreters. Raises and outlined cold blocks
  no longer pass the instruction word where nothing reads it, which
  made 365 tail-call functions shorter (see the item).
- **Item 7** (every error of a build): built; see its row below and section 2.
- **Item 9** (derived root scanning, typed `add_scanner`): built as
  `#[trace] struct` and `add_scanner<T>`; Python and Prolog are ported
  (see the item for what changed and what the IR diff showed).
- **Item 10** (`is` and checked unwrap): built as `e is P` and `e as!
  V`; ports in six interpreters, release code unchanged (see the item).

> Where could we give the user meaningfully better ergonomics at no
> run-time cost, even if it makes the Loimi implementation itself more
> complex?

**Method.** This audit read the six friction logs (`examples/*/FRICTION.md`,
about 3,500 lines; only items still open or partly fixed in their
follow-up tables count) and the sources of the nine interpreters
(`examples/{mini,minijs,scheme,lox,python,prolog,wasm,regex,stack}`). It
counted recurring patterns with grep, read the hot files in full, and
checked each candidate against `language.md`, `status.md` and the design
notes, so that nothing proposed already exists. The counts are of source
lines or sites on master. Each one is reproducible with the grep given,
or with a file:line reference.

**What qualifies.** A proposal counts only if the code it generates is
the code users write today, so that porting an example leaves its
`--release --emit-llvm` IR byte-identical in all three dispatch modes.
That is the same test the debugger and the lifetimes work used
(`debugging.md` section 6, `lifetimes.md` section 7). Where the
generated code is only provably no worse, the item says so. The second
condition is that the feature removes lines, a class of bugs, or a
concept the author must hold in mind.

**The short answer.** Instruction decoding, tracing of objects, opcode
tables, disassemblers and stack effects are already generated: no
interpreter decodes words by hand anymore, and none has a hand-written
opcode-name table or trace function. The lifetimes checker is not where
the burden is either:
- the nine interpreters have 13 `unsafe { }` blocks, 5 of which are
  unzeroed `new` of `Gc` arrays;
- they have no `#[may_alias]` or `#[shrinks]`.

The remaining burden falls into four groups:
- **(a) idioms a handler must spell out** to stay frame-free: GC polls
  with `redo`, allocation refills, `cold { raise .. }`;
- **(b) tables the author keeps parallel to declarations** the compiler
  already has: builtin and exception name tables, op-to-op relations in
  the bytecode compilers, accumulator lists;
- **(c) layouts declared twice**: inline caches that mirror a std
  struct, context fields that a root scanner marks again;
- **(d) the build itself**: one error per build, and IR identity that
  depends on source line numbers, which makes authors cram `#[cfg]`
  code onto existing lines.

## 1. Ranked proposals

The ranking weighs the burden removed (lines, bug classes, number of
interpreters affected) against the compiler cost. Cost is S (days), M
(a week or two) or L (more).

| # | Proposal | Evidence | Cost |
|---|---|---|---|
| 1 | **Built.** Declared safepoint polls, and handler-lowered allocation refills | 73 lines testing the heap threshold in 7 interpreters (46 the full poll), plus 29 `gc_point()` calls; 24 `else cold { make_room; redo }`; Mini's 9 GIL/pause polls and 6 barrier tails | M |
| 2 | **Built.** Names and data on enum variants (`#[name]`, generated `name`/`from_name`, per-variant constants) | ~650 rows of parallel tables: Scheme ~260 (5 copies of its 149 primitives), Python ~150, Prolog 90, Mini 84 (3 copies of 27 natives); 27 `E_*` + `enames`; Trap messages; 4 operator tables | M |
| 3 | Op relations and per-op metadata in `inline for` rows | Wasm: 178 relation rows + 329 translator rows whose types repeat operand kinds; Mini's 24-arm `jump_op`/`negate_jump`; Python, Prolog, Lox op maps | M |
| 4 | Panic locations out of the IR identity check (`loimi ir-diff`: **built**), then `extend vm` blocks | 97 `#[cfg(..)]` lines in Mini (71) and Python (26) packed onto existing lines; Python/Mini `on break` and variants on the line closing `on raise` | S, then M |
| 5 | **Built.** Cache layouts from structs; cache handles for plain functions | 31 `&cache.f` sites; `PropIC` copies `obj.Site` field by field in Lox and MiniJS, plus 22 `as *obj.Site` casts | S |
| 6 | **Built.** Terminal `raise`, `fail` and `become ..; redo` are cold by default | 119 `cold { raise .. }` in 8 vms; 21 `cold { become X; redo }` | S–M |
| 7 | Report more than one error per build (**built**) | Python: ~15 fix-and-rebuild cycles for 10k lines (FRICTION 582) | S |
| 8 | Infer effects of private functions | 136 `uses` on signatures (73 non-`pub`), 18 on fn types; Python `ops.lm` alone 43 | M |
| 9 | **Built.** Derived root scanning for context structs; a typed `add_scanner` | 93 `gc_mark` calls in hand-written scanners, ~45 of them fixed fields or arrays; 10 `add_scanner(.. as *u8, ..)` with casts back | S–M |
| 10 | **Built.** `is` patterns and checked unwrap | 36 `=> true, _ => false` predicates; 79 `let X(v) = e else { trap() }` | S |
| 11 | Op selectors for `accumulate`, `variant .. wraps`; defaults and derived names for `super` groups | Wasm's 140-name `accumulate` list; Stack's hand list of jumping ops for `wraps`; ~140 supers, nearly all named as their parts joined, ~100 `#[forward]`, 54 `#[allow(super_overlap)]` | S |
| 12 | A typed host-side decoder `Vm.decode(w)`, and encoders that take any integer with checked ranges | 42 `Vm.FMT_field(w)` calls with the format chosen by hand; ~300 `as u8` casts at encoder calls; silent masking (regex FRICTION 241); 12 hand decodes in Mini and Prolog compilers | M |

**Build first: 1, 2 and 7.**
- **Item 1** removes the most frequently copied handler idiom, used in
  seven interpreters. It also closes a bug class the compiler does not
  check today: side effects before a `redo`.
- **Item 2** removes the largest number of hand-maintained lines, about
  650, in Scheme, Python, Prolog and Mini. Its zero-cost argument is the
  simplest: the generated functions are only compiled when they are
  called.
- **Item 7** is small, and every new interpreter pays for its absence.

Item 4's `ir-diff` half is also small and unblocks how the debugger
work is written, so it is a good fourth.

---

### 1. Declared safepoint polls, and handler-lowered allocation refills

**Pattern.** A handler that may allocate checks the heap threshold and
collects in a `cold` block that ends in `redo`. A fall-through cold block
inside a plain vm function is not outlined (`language.md` section 4,
"Not built as described"), so the poll cannot live in a helper without
giving every caller a frame. Authors therefore paste it into each op.
Allocation in handlers has a second form, `try_new` with an
`else cold { make_room; redo }`. In both forms, `redo` is only correct
if nothing before it in the op had a side effect. Nothing checks that
(`try_new`'s rule in `language.md` section 4 is prose only).

**Evidence.**

Lines testing `heap.allocated > ctx.heap.threshold`: 73 (Python 24,
MiniJS 20, Scheme 13, Mini 10, Prolog 3, Stack 2, Lox 1). Of these, the
full inline poll `if unlikely(ctx.heap.allocated > ctx.heap.threshold) {
cold { gc.collect(&ctx.heap); redo } }` appears 46 times; most of the
rest are the bare test inside a larger cold block
(`minijs/vm.lm:810`, `:826`). The full poll by interpreter:

| Interpreter | Sites |
|---|---|
| Python | 24 (e.g. `vm.lm:568`, `:905`, `:909`, `:915`, `:1209`–`:1323`) |
| Scheme | 9 (`vm.lm:205`, `:226`, `:389`, `:399`, ...) |
| Mini | 5 |
| MiniJS | 3 |
| Prolog | 3 (`vm.lm:486`–`:495`, `:828`; CALL and EXECUTE copy their tail into the cold block) |
| Stack | 2 (`vm.lm:409`, `:417`) |

The other occurrences:
- **`gc_point()` helpers:** Python (`vm.lm:481`) and Lox (`vm.lm:263`)
  wrap the poll in a function, without `redo`, called 29 times.
- **Allocation refills:** `let Some(x) = try_..(..) else cold {
  make_room..; redo };` appears 24 times: Prolog 13 (`vm.lm:185`,
  `:207`, `:240`, `:250`, ...), Scheme 7, MiniJS 3, Lox 1.
- **Prolog's `try_struct`** (`vm.lm:226`–`:232`) is a 4-way `if` only
  so that each `try_new` gets a constant length.
- **Write barriers:** Mini writes `if unlikely(gc.needs_barrier(tab as
  *u8)) { cold { value.log_table(tab); } }` 6 times (`vm.lm:675`, `:700`,
  `:701`, `:703`, `:721`, `:722`). It is the same shape, and blocked by
  the same limit: the cold block cannot live in a helper.
- **Other polls:** Mini puts `#[cfg(gil)] if unlikely(ctx.gil.waiting())
  { cold { ctx.gil.yield_now(); } } #[cfg(debug)] if unlikely(pause)
  { use_variant step; }` on 6 jump ops, one line each (`vm.lm:1040`,
  `:1113`, `:1123`, `:1138`, `:1153`, `:1188`), and the GIL poll alone
  on 3 more (`:979`, `:1043`, `:1048`).
- **Friction logs:** Python 469 (24 ops given `redo` by hand, still
  partly open) and Prolog 61 (partly open; the `try_new` boilerplate
  remains).

**Proposal.** A vm declares its polls once. An op opts in with an
attribute, or by a selector such as "every op with a label operand",
which reuses the selectors of item 11. The compiler expands the poll at
the top of the op, as the code written today. It also checks the `redo`
rule: no store to memory, state or operand before the poll or a
handler-level `new`, which is an error otherwise.

`new` in a handler, where that rule holds, lowers to the `try_new` form
by itself.

<!-- snippet: skip design, not built -->
```loimi
// before (Python, 24 copies)
op BUILD_LIST(dst: reg, base: reg, n: imm) : ABC {
    if unlikely(ctx.heap.allocated > ctx.heap.threshold) { cold { gc.collect(&ctx.heap); redo } }
    ..
}
// after
poll gc { if unlikely(ctx.heap.allocated > ctx.heap.threshold) { cold { gc.collect(&ctx.heap); } } }
#[poll(gc)] op BUILD_LIST(dst: reg, base: reg, n: imm) : ABC { .. }

// before (Prolog, 13 copies)
let Some(v) = ctx.try_var() else cold { ctx.make_room(); redo };
// after: `new` in a handler takes the inline path, refills in a cold block and redoes the op
let v = new(&ctx.heap, Var { .. });
```

A poll body that falls through gets `redo` appended. The fall-through
form is what the author means; the copy-the-rest alternative would
duplicate the op.

The general alternative is to outline fall-through cold blocks in
*inlined* plain vm functions, by giving them the copy of the rest of
the op that cold blocks directly in ops already get (`language.md`
section 4). That would make Lox's and Python's `gc_point()` and Mini's
barrier tails free as helpers, without a new declaration. It is the
bigger compiler change: the rest is known only after inlining. It also
does not give the `redo`-safety check, so it complements polls rather
than replacing them.

**Why zero-cost, and how to verify.** The expansion is token-for-token
what the examples write, at the same place. To verify, port Python,
Scheme and Prolog, then diff `--release --emit-llvm` IR in all three
dispatch modes. Panic locations move, so the diff needs item 4's
normalizer, or the port must keep line numbers.

The lowering of `new` to `try_new` changes code only where `new` was
written in a handler. There the hand-written form is the documented
best practice: it is the same code as the hand-written form, and
strictly better than a `new` left on the hot path.

**Cost: M.**
- The poll declaration and its expansion are a vm-item expansion
  (`vmexpand.rs`), much like `inline for` in a handler.
- The `redo`-safety check walks the TIR of the op up to the poll or
  allocation. It needs a side-effect predicate, which the verifier's
  "may continue" scan approximates today.
- **Risk:** a poll placed after work that must not repeat. The check
  turns that into an error instead of a silent double effect.

**Interactions.**
- **Lifetimes:** none. The expansion is ordinary code, checked as
  written today.
- **JIT stencils:** stencils are built from handlers after expansion,
  so they are unchanged.
- **Dispatch modes:** `redo` is already defined in all three.
- **Debugger:** a poll is the natural home for Mini's pause check. The
  debugger's `#[cfg(debug)]` poll becomes `#[cfg(debug)] poll pause
  { .. }` in one place instead of six.
- **Superinstructions:** a poll in a later part runs before that part,
  as the code does today.

**As built** (2026-11-05). Refinements of the proposal:
- **The poll is the author's text, with no `redo` appended.** A poll
  that ends in `redo` (Python, Scheme) and one whose cold block falls
  through and gets the rest of the op copied in (Mini, MiniJS, Stack,
  Prolog's METACALL) are both what the examples write, and each gives
  its own code; appending `redo` would have changed the second kind.
  Polls are expanded by the parser, so everything after (checking,
  lifetimes, cold outlining, stencils, `--vm-stats`) sees the written-out
  form.
- **A statement form, `poll NAME;`**, besides the attribute: Mini's GIL
  and pause polls sit in the taken branch of conditional jumps, not at
  the top. A poll under a `#[cfg]` that is off expands to nothing, so
  `#[cfg(gil)] poll gil { .. }` is the only `#[cfg]` needed. Item 11's
  selectors were not needed: an attribute per op is as short.
- **The check covers every `redo`, not only polls** (`src/redocheck.rs`,
  after type checking, on the typed tree): an effect on a path to the
  `redo` before the block that ends in it is the error `redo_effect`.
  The block that ends in the `redo` is its fix-up and is exempt: the
  examples redo after collecting, `make_room`, `become` + `refuse`,
  growing a stack (Mini, Scheme), closing upvalues, flattening a rope
  into the operand (MiniJS); with the fix-up counted, those were the
  only false positives. Function calls are summarized (a least fixpoint:
  a store through a pointer, a call through a value, an extern that
  takes a pointer); `std.gc` and `std.gil` are bookkeeping. One site in
  the examples needs `#[allow(redo_effect)]`: Python's `CALL_NEW`
  calls `ensure`, which stores only on the path that does not redo.
  The check found no real bug in the examples.
- **The other invariant**, that the refill makes room for the size
  `try_new` asked for (a wrong size to `make_room` loops forever), is
  what the `new` lowering makes impossible: it computes the size.
- **`new` is lowered only at a constant size of at most 496 bytes,
  outside loops and cold blocks.** With a variable trailing length or a
  large object `try_new` is a call too (`memset`, `alloc_large`), so the
  lowering bought nothing and would run the op twice on a refill; in a
  loop a refill would restart the loop. A literal trailing length is
  folded into one constant, as `size_of<T>() + size_of<E>()` is.

**Ports** (`loimi ir-diff --release --asm` of the unported sources
against the ported ones, both built by the new compiler):

| Interpreter | Ported | Code |
|---|---|---|
| Python | 23 `#[poll(gc)]` | same, 3 modes, also `--cfg debug` |
| Scheme | 9 polls (`gc`, `gc_fall`); CONS, MKBOX `new` | same, 3 modes |
| MiniJS | 5 polls and a `poll gc;`; BOXNEW, REBOX `new` | same, 3 modes |
| Stack | 2 polls | same, 3 modes |
| Mini | 5 GC polls, 9 GIL, 6 pause (`#[poll]` and `poll ..;`) | same, 3 modes, `--cfg gil`, `--cfg debug`, `--jit-stencils` |
| Prolog | 10 `try_cons`/`try_var` refills as `new`, METACALL's poll | handlers the same but an equivalent compare in PUT_LIST and PUT_VAR_X (`cmpq $31; jbe` for `cmpq $32; jb`); cold refills call `std.gc.make_room` directly; same with `--jit-stencils` |

The compiler's own effect on unported code (the base compiler against
the new one, same sources): one handler in the nine interpreters,
Python's `MAKECELL` (`r = Cell(new(..))`), whose `new` is now lowered:
its frame and inline refill call are gone (47 lines of assembly to 33).

**Not ported**: Python's and Lox's `gc_point()` (a fall-through poll
in a vm function, called from inside cold blocks, where a poll
statement would nest one cold block in another); Prolog's `try_struct`
(a constant length per case, in a vm function) and CALL/EXECUTE (their
cold block copies a different tail by hand); Scheme's
`try_closure`/`try_vect` and MiniJS's `try_obj` (variable lengths), and
Scheme's REVERSE/APPEND2 refills (they fall back to a builtin, not
`redo`); Mini's barrier tails (mid-op, after a store); the bare
`if .. { gc.collect }` lines inside larger cold blocks.

### 2. Names and data on enum variants

**Pattern.** An interpreter's builtins, exception kinds, operators and
traps are an `enum` or a run of integer constants. Each has one or more
parallel tables: name to variant for registration, variant to message,
and variant to dunder name. These are kept in step by hand. `Kind.COUNT`
exists (`language.md` section 1.2); names and per-variant data do not.

**Evidence.**
- **Python builtins:** `enum B` has about 190 variants
  (`builtins.lm:74`–`:108`).
  - Hand registration arrays at `:214`–`:260`: fns 38, str 34, list 11,
    dict 11, set 14, plus 8 `meth(..)`.
  - A math table of about 34 at `:2167`.
- **Python exceptions:** 27 `E_*` constants (`builtins.lm:45`–`:71`)
  parallel to `enames` (`:170`).
- **Python operators:**
  - `O_*`/`U_*`/`C_*` (`ops.lm:20`–`:36`, `:145`–`:150`) are parallel
    to the `op_sym`, `dunder`, `rdunder` and `cmp_dunder` tables
    (`:1280`–`:1306`).
  - Offsets computed by hand: `DN_RBASE = 13`, `DN_CMP = 26`, ...,
    `dn: [Value; 40]` (`ops.lm:39`–`:47`).
- **Scheme primitives:** five parallel copies.
  - `prim_info` has 149 arms `0 => ("+", 0, -1)`
    (`builtins.lm:114`–`:270`).
  - 15 `pub const P_..: i32` ids (`:97`–`:110`).
  - `call_native` dispatches on the raw ids (`:631`).
  - `compile.lm:40` `enum PO` has 46 variants, `inline_prim` maps names
    with 39 `if name.eq("car") { return Some((PO.Car, 1)); }`
    (`:254`–`:291`), and a ~40-arm `PO` to opcode `match`
    (`:2604`–`:2625`).
- **Mini natives:** three copies of the same 27 natives.
  - 27 `const M_..: i32` ids (`lib.lm:35`–`:61`).
  - 27 `set(h, m, "floor", V.Native(M_FLOOR))` registrations
    (`:74`–`:118`).
  - A 30-arm `match id` (`:120`) on raw numbers rather than the
    constants, so a renumbering breaks it silently.
- **Prolog builtins:** `enum B` has about 110 variants
  (`builtins.lm:26`–`:40`), with a 90-row `("name", arity, B.X)`
  register table (`:44`).
- **Prolog arithmetic:** codes 0..14 are passed from `vm.lm:505`–`:576`
  and index `OPNAMES: [str; 15]` in another file (`arith.lm:359`).
- **Wasm:** `Trap` has 12 variants and a hand-written `trap_message`
  match (`vm.lm:16`–`:46`).
- **Stack:** `type_name` matches each variant (`value.lm:71`). Stack's
  37-line word table became a string `match`; it was fixed, but is the
  same shape.
- **Python `type_code`** (`ops.lm:238`–`:270`) maps variants and kinds
  to 24 `T_*` constants by hand.

**Proposal.** Allow attributes with constant data on enum variants, and
generate functions from them. The generated functions are compiled only
if called, like the disassembler.

<!-- snippet: skip design, not built -->
```loimi
// before
const E_TYPE: u64 = 3; const E_VALUE: u64 = 4; ..
var enames: [str; 27] = ["BaseException", .., "TypeError", "ValueError", ..];
// after
enum Exc { #[name("TypeError")] Type, #[name("ValueError")] Value, .. }
// generated: Exc.name(e) -> str, Exc.from_name(s) -> ?Exc, Exc.COUNT (exists)

enum Op {
    #[data(sym = "+", dunder = "__add__", rdunder = "__radd__")] Add,
    ..
}
// generated: Op.sym(o) -> str, Op.dunder(o) -> str, ..: a `match`, or a constant array indexed by the discriminant
```

`#[name]` defaults to the variant's name, so most variants need no
attribute; `from_name` is a string `match`, which exists. `#[data]`
fields must be constants of one type per key. Scheme's `prim_info`
becomes `#[data(name = "+", min = 0, max = -1)] Add` on `enum Prim`,
and its five copies become one.

A natural follow-on, not proposed here, is typed native signatures that
generate argument unpacking and arity checks. Today these are 33
`arg(args, n, i)` calls in MiniJS, 36 `num_arg(..)?` checks in Scheme
and 18 `check_*(.., "name")?` in Mini, which repeat the registration
name. That is library design (`std.rt`), and each guest language checks
arguments differently.

**Why zero-cost, and how to verify.**
- The generated `name` is the `[str; N]` lookup or `match` the authors
  write today. `from_name` is the string `match` that Stack already
  uses.
- Nothing is generated into handlers. Hand-written tables used from
  handlers, such as Python's `dn` offsets, become generated constants
  of the same value.
- **To verify:** port the Python exception table and Wasm's
  `trap_message`, then diff the IR. Only the symbol names of the tables
  may differ, and the port can keep them.

**Cost: M.**
- Attribute parsing exists. The generation is a source expansion like
  the vm's (`vmexpand.rs` writes Loimi source today).
- The work is in the rules: keys, types, and defaults for variants
  without a key.
- **Risk: scope creep toward `comptime`** (section 2). Keep it to
  constant data on variants.

**Interactions.** None with handlers, lifetimes, stencils, dispatch or
the debugger. The debugger's value printers may use `name`.

**As built** (2026-11-05). Three refinements of the proposal:
- **Keys are declared on the enum, with types and defaults**:
  `#[data(min: i32 = 1, max: i32 = 1)]`. Inferring a key's type from
  the first value cannot type `-1` as `i32` or `None` as `?Op`, and a
  default makes the common row empty (most of Scheme's primitives take
  one argument). Variants give values by position or by key.
- **`name` is opt-in** (`#[name]` on some variant, or `#[names]`), so
  existing `fn E.name` methods keep working, and a hand-written method
  of a generated name is an error rather than a silent override.
- **Data may be of enum and option type**, so a relation between two
  enums is data rather than a third table: Scheme's `Prim` carries `op:
  ?PO`, the op its compiler inlines it as, which replaced
  `inline_prim`'s 41-line name chain; Python's `Exc` carries `parent:
  ?Exc`, which replaced the `eparents` array of numbers.

**Ports, and what `ir-diff` showed** (`--release --asm`, tail-call
dispatch, against the sources at 3fc9376 built by the same compiler):
- **Scheme** (`enum Prim: i32`, 149 variants; `enum PO` moved next to
  it with `arity` data): 507 of 511 functions the same, every vm handler
  among them. `inline_prim` is gone; `Comp.inl` and `Comp.build_inner`
  (compile time) now use `from_name` and `op()`: a `switch` on the
  length and word compares instead of up to 41 `str.eq` calls, so no
  worse; LLVM inlines `from_name` there (+418 assembly lines in all).
  `call_native` keeps its shape (a jump table of the arity rows, merged
  with the dispatch `switch`), with the `name`/`min`/`max` constants
  materialized in a different order: 3,286 instructions against 3,186,
  at most one more `mov` on a builtin's path. Timing (r7rs, 8 programs,
  user+sys, interleaved, `--align-functions=64`) was within the
  machine's noise: geomean base/new 0.98 and 1.07 in two runs.
- **Python** (`#[names] enum Exc: u64`, 27 variants): 707 of 708
  functions the same. The one that differs is `main.main`, where the
  start-up loop that creates the exception classes is inlined: it now
  reads `name()` and `parent()` instead of two stack arrays (+160
  assembly lines, run once). Every `exc(c, Exc.TypeError, ..)` site
  passes the same constant as before. Timing (8 pyperformance
  programs, user+sys, min of 5, interleaved): geomean new/base 1.00,
  with single programs within the machine's noise (load 6 to 8 on 8
  cores) either way.
- **Not ported**: Python's `enum B` registration arrays (about 150
  rows) would put one attribute on each of 190 variants now written
  fifteen to a line, so the file would grow. Mini's natives have
  explicit, gapped ids (0..5, 10..19, ..; `call` dispatches on them),
  so a registration loop over the variants needs a way to enumerate
  them (`i as E` is only valid for `0..E.COUNT` without explicit
  discriminants); a generated `E.ALL` would be the next step. Prolog's
  `B` and arithmetic codes are straightforward candidates.

### 3. Op relations and per-op metadata in `inline for` rows

**Pattern.** A bytecode compiler that quickens or fuses at translation
time needs relations between ops: the constant form of an op, its
branch form, its fused form. The vm declares those ops in `inline for`
tables, so every relation is typed a second time in the compiler as
`(W.OP_X, W.OP_XK)` rows, usually looked up by linear search. A second
pattern is that the translator's rows restate the operand types of
each op.

**Evidence.**
- **Wasm relation tables in `compile.lm`:** 178 rows in all.

  | Table | Rows | Lines |
  |---|---|---|
  | `kforms` | 38 | `:885`–`:907` |
  | `fusions` | 52 | `:961`–`:990` |
  | `selects` | 12 | `:993`–`:1002` |
  | `kmems` | 23 | `:765`–`:780` |
  | `rloads` | 8 | `:791`–`:798` |
  | `tkloads` | 8 | `:826`–`:833` |
  | `memop` | 37 | `:719`–`:761` |

- **Wasm `numeric()`/`simd_numeric()`:** 329 rows
  `0x6A => num(W.OP_I32_ADD, i, i, i)` (`compile.lm:432`–`:714`). Their
  value-type columns repeat each op's operand kinds (`ri`, `rl`, `rd`,
  ...), which `Vm.operand_kind(w, i)` already knows.
- **Wasm FRICTION** 400–434 and 505–522: the fusion is hand-written,
  and one fusion bug was caught only by PolyBench.
- **Mini:** `compile.lm:1105`–`:1137`, `jump_op` (8 arms) and
  `negate_jump` (16 arms), all `if op == Mini.OP_JLT { return
  Mini.OP_JLTN; }`. The vm generates exactly those pairs from one
  `inline for (opname, want) in [(name, true), (nname, false)]`
  (`vm.lm:997`). Lox has the same with `invert` (`compile.lm:606`).
- **Python:** `compile.lm:1001`–`:1003`, `:1109`–`:1110` and
  `:1606`–`:1612` (`branch_ops`) repeat the lists of `inline for` at
  `vm.lm:924`, `:1066` and `:1133`.
- **Prolog:**
  - `jump_form` maps `LT` to `JLT`, six times
    (`compile.lm:163`–`:166`).
  - An 8-op set is written twice (`:181`, `:202`).
  - The X/Y twin choice is written 15 times.
- **Stack:** `compile.lm:211`–`:249` has 39 rows of `"dup" =>
  Some(Stak.DUP())`.

**Proposal.** An `inline for` row may define more than one op and name
the relation between them. An op may carry constant tags. The compiler
generates host-side functions from both, as it does `Vm.op_name`.

<!-- snippet: skip design, not built -->
```loimi
// before: vm.lm declares I32_ADD and I32_ADDK in two tables; compile.lm repeats
//   (W.OP_I32_ADD, W.OP_I32_ADDK), .. and searches the list.
// after
inline for (name, k, expr) in [("I32_ADD", ri, a +% b), ("I32_SUB", ri, a -% b), ..] {
    #[tag(wasm = ..)] op #name(d: k, a: k, b: k) : RRR { d = expr; next }
    #[rel(kform = #name)] op #(name ++ "K")(d: k, a: k, b: simm) : RRK { d = expr; next }
}
// generated: W.kform(op) -> ?W (a `match` on the opcode), W.from_wasm(u64) -> ?W,
// and the operand kinds are already W.operand_kind(w, i)
```

**Why zero-cost, and how to verify.**
- Handlers are unchanged. The tags and relations exist only in
  generated host functions, which are compiled only if called.
- The translator's lookups go from linear searches to a `switch`, so
  they are not slower.
- **To verify:** the interpreter's handler IR is byte-identical. The
  translator's IR differs, which is expected, so check that
  `examples/wasm`'s spec tests and PolyBench outputs are the same.

**Cost: M.**
- Parsing tags on ops is easy.
- Generating `W.REL(op) -> ?W` is a `match_fn` in `vmexpand.rs`, like
  `stack_effect`.
- **The design risk** is the syntax for relations across separately
  declared ops. Restricting relations to ops defined in the same
  `inline for` body covers every case above.

**Interactions.** Relations name ops, so `Vm.deopt` and supers are
unaffected. The JIT, dispatch and the debugger do not see tags.

### 4. Panic locations out of the IR identity check, then `extend vm`

**Pattern.** Panic messages carry `file:line:col`, so moving code by
one line changes the release IR. The project's zero-cost discipline is
"the default build's IR is byte-identical". So every `#[cfg(debug)]` or
`#[cfg(gil)]` addition is written on an existing line, or at the end of
the vm (`debugging.md` sections 6 and 8.2 item 9, `language.md`
section 1.4).

**Evidence.**
- **Python:** 26 `#[cfg(debug)]`, all on existing lines (`vm.lm:1666`
  says so).
  - `vm.lm:1522` is one line holding the close of `on raise`, all of
    `on break`, and an `#[unfused] variant step`.
  - `vm.lm:259`–`:260`: `sync` and `reload` each carry a
    `#[cfg(debug)]` tail.
- **Mini:** 71 `#[cfg(..)]`, written the same way.
  - `vm.lm:1040`: one 233-character line holding JMP, two polls and its
    `jump`.
  - `vm.lm:72`: four context fields on one line.
- **Debugger design note:** `debugging.md` 8.2 item 9 calls this "a
  layout constraint, not a missing feature". It is a constraint that
  the tooling imposes, not the language.

**Proposal, in two steps.**
1. **`loimi ir-diff a.ll b.ll`** (or `--emit-llvm
   --normalize-locations`). It compares IR with each panic location
   string replaced by a stable placeholder, and all other text equal.
   The examples' IR identity checks then use it, and `#[cfg]` code can
   go on lines of its own.
2. **`extend vm Py { .. }`** in another module, under `#[cfg(debug)]`.
   It adds `on break`, variants, polls (item 1) and vm functions to a vm
   declared elsewhere, so the debugger's hooks live in `debugger.lm`.

**Why zero-cost.**
- Step 1 does not change the compiler output at all.
- Step 2 merges items into the vm before expansion. A build without
  the cfg drops the block in the parser, as `#[cfg]` does today, and so
  produces the same IR.
- **To verify step 2:** move Python's debugger items into an `extend`
  block, then run `ir-diff` against master in the three dispatch modes,
  with and without `--cfg debug`.

**Cost.**
- **Step 1: S.** The panic location format is fixed (`codegen.rs`
  emits it), so the normalizer is a text rewrite of those constants.
- **Step 2: M.** `vmparse.rs` collects vm items from one block. Merging
  items from several modules raises name-resolution questions: what an
  `extend` block may see, and its module's imports.
- **Risk:** an `extend` block that silently changes ops. Restrict it to
  adding items (`on break`, `variant`, `poll`, `fn`), never altering
  existing ones.

**Interactions.**
- **Lifetimes:** none.
- **Stencils:** variants are already an error with `--jit-stencils`.
- **Debugger:** this is the debugger's main beneficiary.

### 5. Cache layouts from structs; cache handles for plain functions

**Pattern.** `std.obj`'s inline-cache slow paths take a `*obj.Site`.
An interpreter therefore declares a `cache` whose fields copy
`obj.Site` one by one, and passes `&cache.sid as *obj.Site`, relying on
the two layouts staying equal. Other interpreters pass a cache field's
address (`&cache.idx`) to plain functions, because a `cache` handle can
only go to vm functions (`language.md` section 3.5).

**Evidence.**
- `cache PropIC { sid: u32, slot: u32, cell: u32, depth: u32, ent: u32,
  miss: u32 }` appears in both `lox/vm.lm:515` and `minijs/vm.lm:513`.
  `docs/stdlib/obj.md:221` gives the same declaration. It copies
  `std/obj.lm:582` `pub struct Site`.
- The casts `&cache.sid as *obj.Site`: Lox 12 (`vm.lm:538`, `:565`,
  ...) and MiniJS 10 (`vm.lm:524`, `:569`, ...).
- **Field addresses passed to plain functions:**
  - Python 6, e.g. `vm.lm:733` `ops.loadmethod(ctx, o, s, &cache.cls,
    &cache.epoch, &cache.idx, &f)`, with five `*u32`/`*u64` parameters
    at `ops.lm:750`.
  - Mini 3 (`vm.lm:666`, `:679`, `:754`).
- **Python's 4-way `MethIC`** (`vm.lm:253`) spells out `cls`/`fnv`,
  `cls2`/`fnv2`, `cls3`/`fnv3` and `cls4`/`fnv4`, with a manual shift
  at `:738`–`:749`.

**Proposal.**
- `cache PropIC = obj.Site;` declares a cache whose words are a
  struct's fields. The struct must contain only fields that a cache
  allows.
- In an op, `&cache` has type `*obj.Site` (and `*PropIC` for a declared
  layout) and may be passed to any function.
- Array fields in caches (`ents: [Ent; 4]`) follow from that.

<!-- snippet: skip design, not built -->
```loimi
// before
cache PropIC { sid: u32, slot: u32, cell: u32, depth: u32, ent: u32, miss: u32 }
let (r, st, d) = read_prop(ctx, &cache.sid as *obj.Site, obj.IcState.Generic, x, key(name));
// after
cache PropIC = obj.Site;
let (r, st, d) = read_prop(ctx, &cache, obj.IcState.Generic, x, key(name));
```

**Why zero-cost, and how to verify.**
- `&cache` is `pc + 1`, the same address `&cache.sid` computes today.
- The words, the cache zeroing in the emitters and the disassembler's
  `[cache ..]` output are unchanged. The one exception is that the
  disassembler prints struct fields by name, which it already does.
- **To verify:** port Lox and MiniJS, then diff the IR.

**Cost: S.**
- `CacheDecl` (`vmast.rs:34`) takes a type instead of a field list.
  Code-word counting (`Vm.CW_Cache`) follows from the struct's size,
  rounded to whole words.
- **Risk: alignment.** Cache words are code words, so a struct with an
  8-byte field may be read unaligned. This is already handled for wide
  cache fields ("unaligned access for wide fields", section 3.5), but
  the struct's own accessors must use it too. Either restrict cache
  structs to fields no wider than the code word, or mark them
  `#[packed]`.

**Interactions.**
- **Lifetimes:** `&cache` is a pointer into code, which never moves
  while it runs. Give it the "fresh" root (`lifetimes.md` section 5.1),
  so that storing it is error 4.
- **Stencils:** cache words are never enter words (`language.md`
  section 3.3), so they are unaffected.
- **Debugger:** `deopt` zeroes the cache as before.

**As built** (2026-11-05), as `cache X = S;` with `&cache` a `*S`
([language.md §7.1](../language.md#71-inline-caches)). Refinements of
the proposal:
- **The words stay one field per word.** `cache X = S;` is the cache
  `cache X { f: T, .. }` with `S`'s fields, filled in before the vm is
  expanded (`vmexpand::cache_structs`, which finds `S` in the vm's
  module, as `m.S` or through `import ..{S}`, and qualifies field types
  of `S`'s module), so `op_size`, the emitters, the verifier, `deopt`,
  the disassembler (fields by name) and `cache.f` are what the written-out
  cache gives. The alternative, a cache that holds an `S` as laid out in
  memory, would have changed the word layout of every cache with a
  narrow field, and the generated code that computes it.
- **`&cache` is a `*S` only where the two layouts agree**, which the
  checker proves from the field offsets: every field at its word's byte
  offset, `S` no more aligned than a code word, and no larger than the
  words. The alignment risk the proposal raised is handled by that rule
  (a `u64` or pointer field in a `u32` vm makes `&cache` an error that
  names the field), not by `#[packed]` structs, which Loimi does not
  have. `cache.f` works whatever the layout.
- **`&cache` compiles to `&cache.f0 as *S`** (`TK::Cast` of the first
  word's address, `vmcheck::cache_addr`), the form written by hand, so
  the lifetimes, `objref` and `redo` checks see the same tree. In a vm
  function, `&c` for a handle `c: cache X` is the handle's pointer cast
  the same way. A cache with a field list has no struct type, so `&cache`
  there is an error that says to take a field's address.
- **Not built: array fields** (`ents: [Ent; 4]`). A struct field of array
  type is still not a cache field; Python's four-way `MethIC` would need
  them and has `u64` fields at 4-byte offsets anyway (below).

**Ports, and what `ir-diff` showed** (`--release --asm`, the unported
sources against the ported ones, both built by the new compiler, in
tail-call, goto and switch dispatch):
- **Lox**: `cache PropIC = obj.Site;` and 12 `&cache.sid as *obj.Site`
  → `&cache`: 332 of 332 functions the same in every mode.
- **MiniJS**: the same declaration and 10 casts; `prim_get`, the slow
  path that caches an array or string method, takes the `*obj.Site`
  instead of two `*u32` (`site.sid = ..; site.slot = ..`). 654 of 657
  functions the same: `prim_get` (two instructions fewer: one argument
  register less to save) and its two callers, the cold generic members
  of GETPROP and GETMETHOD (one `leaq` fewer each).
- **Python**: GETGLOBAL's cache is `cache GlobalIC = ops.GlobalSite;`
  and `ops.get_global(ctx, func, s, &cache)` takes one pointer instead
  of `&cache.idx, &cache.ver, &cache.builtin`. 707 of 710 functions the
  same: `get_global` (322 assembly lines against 331, a pointer fewer
  to keep), GETGLOBAL's generic member (two `leaq` fewer) and LOADNAME
  (two fewer: one local struct for three locals).
- **Not ported**: Python's `MethIC` and `SupIC` (`u64` method words at
  4-byte offsets: not an `S` in memory; `loadmethod` keeps its five
  pointers), `AttrIC`/`SetIC`/`SetGIC` (one field each, which
  `&cache.idx` passes already), and Mini's one-field `FieldIC` (3 sites).
  The `std.obj` documentation and `tests/run/std_obj_sites.lm` use the
  new form.

### 6. Terminal `raise`, `fail` and `become ..; redo` are cold by default

**Pattern.** A `raise` in a handler is the slow path by nature. Authors
nonetheless wrap it in `cold { }` so that its arguments, often a call
that builds an error value, are outlined, and the fast path gets no
frame.

**Evidence.**
- `cold { raise` appears 119 times:

  | Interpreter | Sites |
  |---|---|
  | Wasm | 30 |
  | Lox | 27 |
  | Scheme | 16 |
  | Python | 14 |
  | Stack | 12 |
  | MiniJS | 9 |
  | Mini | 8 |
  | Prolog | 3 |

  Examples: `wasm/vm.lm` has 17 `if top > msize { cold { raise
  Trap.MemOOB } }`; Python has 18 copies of `raise ops.exc(ctx,
  builtins.E_RECURSION, ..)`.
- About 40 more `raise`s in the vms sit inside larger cold blocks.
- **Deoptimizing:** `cold { become X; redo }` appears 21 times: Lox 9,
  MiniJS 6, Python 4, Mini 1, plus `become X; refuse(..); redo` forms.
  This is a family member giving up on its guard, which is the slow
  path by construction.
- **Uncovered sites:** the ones that are not cold are mostly whole-op
  bodies (`op UNREACHABLE() .. { raise Trap.Unreachable }`,
  `wasm/vm.lm:623`), where coldness is moot.

**Proposal.** A block whose last statement is `raise e` or `fail e`,
or that is `become X; redo` in a family member, and which is the arm of an `if` or `match` or the `else` of a `let ..
else`, is treated as `cold { }` unless it is marked `#[hot]`. The rule
covers only the block that ends in the control word, so a `raise` in
the middle of a longer hot block is not affected. `?` is not included:
it is on the hot path of every `Result` call (section 2).

**Why zero-cost, and how to verify.**
- **Where authors already wrap every such `raise`:** removing the
  wrappers gives byte-identical IR, so port Wasm (30 sites) and Lox
  (27) and diff.
- **Where a terminal `raise` is not wrapped today:** it becomes cold,
  so its branch is weighted unlikely and outlined. That is the code the
  author would get by writing `cold`, and the fast path can only lose
  instructions.
- **To find those sites:** before turning it on, run a one-off
  `LOIMI_AUDIT=raise` listing. Mini, MiniJS and Scheme raise from hot
  arms of `match` in a few ops; check each with `-W handler_frame`.

**Cost: S–M.** The rule is syntactic and would run in `vmcheck.rs`,
wrapping the block in the existing cold node. The open question is
`#[hot]` for interpreters that use `raise` for control flow, such as
Prolog's `throw/1` or Python's `StopIteration` in loops. Prolog has
only 3 cold raises, so it would want `#[hot]` on few sites, if any.

**Interactions.**
- The same machinery as hand-written `cold`, so none beyond what
  `cold` already has.
- **`--vm-stats`** gains counters for the new cold blocks, which is
  useful.
- **Stencils:** cold blocks are outlined functions, which the stencils
  already handle.

**As built** (2026-11-05), as "errors and deoptimization are cold by
default" ([language.md §4](../language.md#4-slow-paths-cold-blocks)).
Refinements of the proposal:
- **Tails as well as branches.** Most of the hand-written wrappers were
  not branches but the end of an op after its fast path left it: `if let
  (Num(x), Num(y)) = (a, b) { dst = Num(x - y); next } cold { raise ..
  }` (Lox's arithmetic, Scheme's `car`, Mini's field ops, Stack's
  words). The rule therefore also makes cold the rest of any block that
  ends in the control word after a statement that can leave the block
  (one with a control word outside a `cold` block, a call of a `->
  never` vm function, `return`, `break` or `continue`). A control word
  in a `cold` block does not count: what follows a poll is still the
  op's path (Python's RAISE, whose poll ends in `redo`, would otherwise
  have had its whole body outlined). An early version did not count
  `redo` either, and the port diff caught it: MiniJS's string-method
  member redoes on a rope that is already flat on its fast path ("no
  cold call", says its comment), which would have gone cold. `?` does
  not count. A `raise` the code always reaches stays as written (MiniJS's
  THROW and THROWREF, Prolog's UNDEF keep their `cold`).
- **A tree rewrite before checking** (`src/autocold.rs`, run by the vm
  parser after polls are expanded): the rewritten tree is the one the
  hand-written `cold { .. }` parses to (`{ cold { .. } }` for an `if`
  block and a tail, `else cold { .. }` and `=> cold { .. }` for an `else`
  and an arm), so the `redo` check (the block that ends in the `redo` is
  still its fix-up), lifetimes, cold outlining, hot-handler ordering and
  the cold section, `--vm-stats` (which counts the new blocks, at the
  branch's location) and the debugger see the hand-written form.
  `LOIMI_COLD_AUDIT=1` lists the branches made cold.
- **`#[hot]` is on the control word** (`Err(ball) => { #[hot] raise ball
  }`), and an unused one is an error. Prolog uses it on the two arms
  through which a guest `throw/1` raises (CALLB, METACALL): in tail-call
  mode cold cost the throw one jump and the hot path nothing, but in
  switch mode the dispatch loop, one function, came out placed
  differently and measured 3.1% slower (7 programs, min of 9
  interleaved, where the build against itself gave 0.992); with
  `#[hot]` Prolog's goto and switch code is the base compiler's, and the
  switch build measures 1.001.
- **A codegen change the measurement asked for.** Making Prolog's CALLB
  raise cold first cost its hot path an instruction: an outlined cold
  block is passed the handler's instruction word, so the word had to
  survive the call to the builtin (`pushq %rbp; movl %esi, %ebp` for
  `pushq %rax`). Nothing in a raising cold block reads it, and `on
  raise` reads it in none of the examples, so raises and outlined cold
  blocks now pass `poison` where nothing reads the word
  (`vmgen::reads_insn`, `on_raise_reads_insn`; lowering.md §3). That is
  the change behind most of the differences below, in hand-written cold
  blocks as much as in new ones.

**Ports, and what `ir-diff` showed** (`--release --asm`, tail-call,
goto and switch dispatch).
- **The ports** (the sources before against those without the wrappers,
  both built by the new compiler): 138 wrappers removed, 114 `cold {
  raise/fail .. }` and 24 `cold { become ..; redo }`: Lox 36, Wasm 30,
  Python 17, Scheme 16, MiniJS 15, Stack 12, Mini 10, Prolog 2. Every
  function the same in all three modes for all eight, and for Mini with
  `--cfg debug`, `--cfg gil` and `--jit-stencils`, Prolog with
  `--jit-stencils`, Python with `--cfg debug` and Regex (unchanged; its
  one `fail` is in a variant's `before` block, which the rule does not
  cover). Left hand-written: unconditional raises (MiniJS THROW and
  THROWREF, Prolog UNDEF), one wrapper inside a larger cold block
  (Python), the budget variant's `fail` (Regex), and two tails where the
  rule would also have moved type tests into the cold block, so the
  code would not have been the same (Mini's CALL after its native and
  wrapped-coroutine cases, MiniJS's string-method member after the rope
  cases: there the hot path was 4 instructions shorter and the native
  path one test longer).
- **The compiler on unchanged sources** (the base compiler against the
  new one, the sources before the ports): the sites the hand had not
  written cold (`LOIMI_COLD_AUDIT=1`: 42, in Scheme's `call_slow` and
  `callcc`, MiniJS's and Python's and Wasm's `call_slow`, Wasm's ten
  table and bulk-memory ops, Python's DELGLOBAL, LOADCELL, LOADFREE,
  Prolog's CALLB and METACALL, Regex's `grow_stack`), and the word no
  longer passed. In tail-call mode 365 functions are shorter (Lox 83,
  Scheme 118, Wasm 98, Python 39, Prolog 14, Mini 8, Stack 4, MiniJS 1;
  e.g. Lox SUB 30 → 27 lines: `ja cold` instead of `ja .L0; .L0: movl
  %eax, %esi; jmp cold`; Python LOADCELL 77 → 31: its unbound-variable
  error is outlined; Scheme CALLCC 208 → 124), 20 the same length, and
  seven longer, none on a hot path: a `.section` directive after Prolog's
  CALLB (the same instructions on its fast path, `pushq %rax`/`popq
  %rax`; the throw path one jump longer) and Wasm's TABLE_INIT and
  MEM_INIT (their trap path is a jump to the cold function), Prolog's
  METACALL.cold#0, Wasm's `call_slow` and Regex's `grow_stack` (slow
  paths), Python's GETATTR_GETATTR_MUL.resume1 (a super's resume
  function, reached from slow paths only). New functions are the
  outlined blocks (Python 19, Wasm 10, Scheme 6, Prolog 3; Prolog's go
  away with its `#[hot]`, which leaves 14 functions shorter and none
  else different). In goto and switch mode Lox, Mini, Stack and (with
  `#[hot]`) Prolog are the same; in MiniJS, Scheme, Python, Wasm and
  Regex the one function holding the handlers
  differs: its front-end IR differs only by the `cold` marker on the new
  cold blocks (in MiniJS, 15 `call void @llvm.sideeffect() cold` lines in
  inlined copies of `call_slow`), and the rest is LLVM's block placement
  and register allocation of a function of 10,000 to 30,000 lines.
  Python's `--cfg debug` build has three functions longer (CALLKW +31
  lines of spills in a handler with a 256-byte frame), which is the
  debugger build.
- **Timing** (the base compiler against the new one on the unported
  sources, `--align-functions=64`, user+sys, minimum of 5 interleaved
  runs, geometric mean of new/base over each suite's programs; the
  machine was also running the test suite, so single programs moved by
  up to ±8% both ways, and the build against itself gave 0.99 to 1.01):

  | | tail-call | goto | switch |
  |---|---:|---:|---:|
  | Lox (10) | 1.000 | same code | same code |
  | MiniJS (14) | 1.002 | 1.001 | 1.005 |
  | Python (11) | 0.965 | 0.998 | 0.991 |
  | Scheme (r7rs) | 1.002 | 1.002 | 1.008 |
  | Mini (14) | 0.998 | same code | same code |
  | Prolog (30) | 0.994 | 0.995 | 1.036; with `#[hot]` same code (1.001) |
  | Wasm (7) | 1.005; 0.988 again (3, min of 9) | 0.995 (min of 7) | 0.986 (min of 7) |
  | Stack (5) | 1.028; 0.991 again (min of 9; itself 1.007) | same code | same code |

### 7. Report more than one error per build

**Status: built** (2026-11-05). Function bodies, generic instances
(first error each), vm handlers, vm functions, entries, `on raise`,
variant `before` blocks and the statements of every body are checked
past an error. A failed statement's bindings are poisoned: they have
type `never`, and errors in statements that read them are dropped, as
are errors of the statements enclosing the failed one. After an error in
an `if`/`while` condition or a `match` scrutinee, the branches are still
checked. The `objref` and lifetimes passes report the first error of
each function. Output is in source order, one error per place, capped by
`--max-errors N` (default 20). Parsing, expansion, imports, type
definitions, signatures, impls and module-level `var`s stay fatal, as
proposed. The speculative checks (probes, a local closure retried with
effects) run without recovery; an `if`/`match` without an expected type
is checked first exactly as before, and again with recovery only after
an error, so a program that compiles keeps the same locals. The release
IR of Lox, Python, Wasm, Stack and Prolog in the three dispatch modes,
of Mini with and without `--cfg debug`, and of the 216 programs of
`tests/run`, is byte-identical to master's. Tests:
`tests/fail/multi_errors*.lm` (the `// errors: N` directive checks the
exact count). Mutation trials on eight examples, 6 to 8 statements or
conditions broken in different functions, gave one error per break and
no cascades (the other trials broke a file outside the build, or a
line into one that did not parse).
`status.md`'s Compiler row lists what continues.

**Pattern.** The checker returns at the first `Diag`, so getting a new
interpreter to build is one compile per error. Python's author needed
about fifteen cycles for 10,000 lines (python FRICTION 582, open).

**Evidence.**
- Python FRICTION 582.
- Every other new-interpreter log shows the same loop, with the fixes
  done one at a time. The regex, Wasm and Stack follow-up tables list
  about 20 diagnostic improvements.
- In `check.rs:609`–`:612`, pass 6 checks bodies in a loop with `?`.
  By then every signature, type and vm is declared (passes 1–5), so
  bodies are independent.

**Proposal.**
- In pass 6, keep checking after a body's error, collect up to N (20)
  errors, and print them in source order.
- The lifetimes pass already collects `errs: Vec<Diag>`
  (`lifetimes.rs:458`), and so does `objref.rs`.
- Parse errors and errors in passes 1–5 stay fatal. They are the minority
  in the logs, and those passes depend on each other.

**Why zero-cost.** It changes no output for a program that compiles.

**Cost: S.**
- The pass-6 loop and the generic-instance queue (`pending`) need to
  tolerate a body that failed: record it and skip codegen.
- Vm handlers are checked in pass 5 (`check_vm`). Per-op recovery there
  is the same change one level down, and is worth it, since handlers
  are where new code is.
- **Risk:** cascading errors from a generic instance whose body failed.
  Report only the first error per instance.

**Interactions.** None.

### 8. Infer effects of private functions

**Pattern.** `uses gc`, `uses sync` and `uses reenter` must be written
on every function that has them (`status.md` "Differences": "Effects
must be declared; they are not inferred for private functions").
`language.md` section 5 promises inference. The missing-effect error
says which effect to add (`vmcheck.rs:2094`), so the cost is clutter
and rebuilds rather than bugs.

**Evidence.**
- 136 function signatures with `uses` in the examples, 73 of them
  not `pub`, plus 18 on function types.
- **By file:**

  | File | Signatures |
  |---|---|
  | `python/ops.lm` | 43 |
  | `python/builtins.lm` | 20 |
  | `mini/embed.lm` | 12 |
  | `python/fmt.lm` | 10 |
  | `mini/debugger.lm` | 8 |

- **Python `uses reenter`:** 85 occurrences.
- **No complaint in the friction logs.** Lox 288 says GC integration
  "is effortless", so this ranks below the items that remove bugs.

**Proposal.**
- Infer the effects of non-`pub` functions as the least fixpoint over
  the call graph, like the lifetimes summaries (`lifetimes.md` section
  5.3).
- `pub` functions and function types keep declared effects. They are
  API.
- A declared effect on a private function stays allowed, and is
  checked as an upper bound.

**Why zero-cost, and how to verify.**
- **Where the declared effect equals the inferred one,** sync points and
  rooting are the same.
- **Where a private function over-declares,** inference gives a
  smaller effect, so it can only remove syncs and roots. That code is
  provably no worse, but it is not identical.
- **To verify:**
  - Add `LOIMI_EFFECTS=audit`, which prints each declared effect that
    is larger than the inferred one.
  - Delete only the annotations where the two agree, then diff the IR.
  - Treat the over-declared ones separately: each is a small speedup.

**Cost: M.**
- Effects are read in pass 4 (`declare_fn`), and handlers use them
  while being checked (`wrap_effects`, `vmcheck.rs:2001`), before the
  bodies of plain functions are checked.
- Inference therefore needs either a pre-pass over untyped bodies, or a
  re-check. A pre-pass is hard, because method calls resolve by type.
  The re-check approach:
  1. Check plain bodies first, with effects as declared, or "pending".
  2. Compute the fixpoint.
  3. Check vms.
- This means reordering passes 5 and 6.
- **Risk: recursion through function values.** Those keep declared
  types, so the fixpoint only runs over direct calls.

**Interactions.**
- **Lifetimes:** effects feed restructuring ("a call that may collect
  restructures every GC object not held by a `let`", `lifetimes.md`
  section 5.2). Inferred effects are at most the declared ones, so
  lifetimes errors can only go away, never appear.
- **Stencils and dispatch:** not affected.

### 9. Derived root scanning for context structs; a typed `add_scanner`

**Pattern.** Objects are traced by generated code. The interpreter's
context, a plain struct holding `Gc` fields, arrays and `Vec`s of
values, is scanned by a hand-written function registered with
`heap.add_scanner(&m as *u8, scan_roots)` and cast back from `*u8`.
Array lengths in the scanner are copied from the field types. A field
added to the context and not to the scanner is a silent missing root,
which only `--gc-check-roots` or the rooting fuzzer finds.

**Evidence.**
- 93 `gc_mark(` calls in the examples: Mini 25, MiniJS 22, Python 20,
  Prolog 11, Scheme 6, Lox 5, Stack 4.
- **Python** `vm.lm:168`–`:199` marks 20 roots in 31 lines, among them
  `for i in 0..32 { gc_mark(h, c.excs[i]); }` (`:187`), `0..40`
  (`:196`) and `0..65` (`:197`). These repeat `[?Gc<Box>; 32]` etc. at
  `:115`–`:154`.
- **Prolog:** `machine.lm:416`–`:452`.
- **Stack:** `vm.lm:59`–`:68`.
- **Proportion:** about 15 of Python's 20 and 6 of Prolog's 11 are
  fixed fields, fixed arrays, `Vec`s or maps. The rest are stack
  ranges, which must stay hand-written.
- **The `*u8` round trip:** `add_scanner(.. as *u8, ..)` is used 10
  times, each with a `data as *Ctx` back (`python/main.lm:80`,
  `prolog/main.lm:230`, `stack/main.lm:98`, ...).

**Proposal.**
- `#[trace] struct Ctx { .. }` generates `Ctx.trace(h: *gc.Heap, c:
  *Ctx)`. It marks every field the object tracer would mark: `Gc`,
  `value` types, options, fixed arrays and std containers of them.
  `#[untraced]` on a field excludes it.
- The hand scanner calls `c.trace(h)` and then marks the dynamic
  ranges.
- `heap.add_scanner<T>(p: *T, f: fn(*gc.Heap, *T))` is generic, and
  does the cast inside std.

**Why zero-cost, and how to verify.**
- The generated function is the object tracer's code applied to a
  plain struct. Collection-time code is not on the interpreter's hot
  path anyway.
- The generic `add_scanner` monomorphizes to the same call.
- **To verify:** port Python's scanner and diff the IR. Field order
  decides marking order, so mark in declaration order, as hand
  scanners mostly do already.

**Cost: S–M.** `gcgen.rs` already generates trace functions from field
types. The work is allowing it for a non-`object` struct, plus
container support (`Vec<Value>`, `HashMap<_, Value>`). Std's
containers would expose a `trace` that the generator calls.

**Interactions.**
- **Lifetimes:** none.
- **GC fuzzing:** the rooting fuzzer (`status.md`) is unchanged.
- **Dispatch and stencils:** none.

**As built** (2026-11-05). `#[trace]` on a plain struct generates
`S.trace(s: *S, h: *gc.Heap)` ([language.md
§8](../language.md#8-gc-integration)). Refinements of the proposal:
- **Generated as checked source, by type.** The parser adds the method
  (`#[__lazy]`, so it is compiled only if called) with the body
  `__gc_trace_fields(h, s)`, and the checker expands that into the
  statements a scanner writes by hand, from the resolved field types:
  `gc_mark(h, s.f)` for a type that holds references inline, `for i:
  u64 in 0..N { gc_mark(h, s.f[i]); }` for a fixed array, `for i in
  0..s.f.len { gc_mark(h, s.f.ptr[i]); }` for a `mem.Vec`, the `next`
  loop for a `map.HashMap` (keys and values, whichever hold
  references), and recursion into nested plain structs. So it needs no
  `trace` method in the std containers, and the code is what the
  examples wrote. `gc_mark(h, *s)` on the whole struct would have
  marked the inline fields too, but loads the context (Python's is 4
  KiB) as one LLVM aggregate and misses the containers.
- **No silent miss.** A field whose type holds references that this
  cannot reach (another container, an array of `Vec`s, a raw pointer in
  a nested struct) is an error naming the field and why; `#[untraced]` says
  the scanner handles it. Raw pointers are not followed, as in object
  tracing: the scanner marks the stack ranges they delimit. Python's
  and Prolog's contexts needed no `#[untraced]`.
- **The order** is the declaration order, and a scanner calls
  `c.trace(h)` before or after its dynamic ranges. Marking order does
  not matter to the collector (marking pushes onto one mark stack).
- **Typed `add_scanner<T>`** (and `add_sweeper<T>`): `data: *T, scan:
  fn(*Heap, *T)`. Inside std, the function value is converted with
  `unsafe { scan as fn(*Heap, *u8) }`, a new cast: a function type
  converts, in `unsafe` only, to one that differs in the types of raw
  pointers. Existing `as *u8` callers still compile (`T = u8`).
- `LOIMI_DUMP_VM_EXPANSION=1` lists what each `trace` marks, which is
  how the ports were checked against the hand-written scanners.

**Ports, and what `ir-diff` showed** (`--release --asm`, tail-call
dispatch, against the unported sources built by the same compiler):
- **Python** (`vm.lm`): `Ctx.trace marks globals, builtins, modules,
  types[0..24], excs[0..32], strs (HashMap values), n_init, n_str,
  n_repr, pre_list, pre_via1, pre_via2, dn[0..40], shape_roots[0..65],
  shape_dict`, the 15 lines it replaced, in the same order. 707 of 708
  functions the same; `scan_roots` differs in the operand order of one
  commutative `and` (same instructions), and LLVM no longer infers
  `nocapture` on its context parameter (the function is only called
  through the scanner pointer, so nothing uses the attribute).
- **Prolog** (`machine.lm`): `Machine.trace marks x[0..256], ball, bags
  (Vec), consts (Vec), codes (Vec), globals (HashMap values), inits
  (Vec)`, the 7 roots the scanner marked among its stack walks. 444 of
  445 functions the same; `scan_roots` has the same 252 instructions
  (the same multiset of opcodes) in another order, since the fixed
  roots are now marked first.
- **Typed scanners everywhere**: every scanner and sweeper of the
  examples (Lox, Mini's four, MiniJS, Scheme, Stack, Python, Prolog)
  takes its own type, and the ten `as *u8` registrations are gone.
  `add_scanner<Ctx>` compiles to the same push.
- **GC checks**: Python's and Prolog's `check.sh` (every test normally,
  under `--gc-stress`, `--gc-verify` and a `--gc-check-roots` build
  with stress, in the three dispatch modes) and their golden tests
  under `--sanitize=address` with GC stress pass.
- **Not ported**: Scheme's scanner marks `syms.all` but not
  `syms.map` (the same symbols) or `gsyms`; `#[trace]` would mark both
  again, extra work at each collection, so it stays hand-written (an
  `#[untraced]` on a nested field would allow it). MiniJS, Lox, Mini
  and Stack mark mostly stacks, frames and a few fields, so the
  derived part would be a line or two.

### 10. `is` patterns and checked unwrap

**Pattern.** Testing a variant without binding it takes a whole `match`
or an `if let .. {}`. The doc marks `v is Nil` **Not built**
(`language.md` section 2). Unwrapping a variant that the compiler or
the verifier guarantees takes `let X(v) = e else { trap() };`.

**Evidence.**
- **Variant predicates:** 36 `=> true, _ => false` arms. Prolog has 18
  (`vm.lm:122`, `:124`, `:642`–`:645`, `arith.lm:44`, `:357`), then
  MiniJS 7, Python 6, Scheme 4.
- **Empty-arm idioms:** e.g. `python/ops.lm:1325` `if let Str(_) = b {}
  else if let Inst(_) = b {} else {`.
- **Unwrap-or-trap:** 79 `else { trap() }`: Python 78, Regex 1.
  Examples: `vm.lm:495` `let Str(s) = name else { trap() };` and
  `:585` `let Func(f) = fp.func else { trap() };`.

**Proposal.**
- `e is P`, a `bool` pattern test whose `P` may not bind.
- `e as! X`, which yields the payload of variant `X` or traps, the
  same trap as written today.

<!-- snippet: skip design, not built -->
```loimi
// before
fn is_ref(t: Term) -> bool { match t { Ref(_) => true, _ => false } }
let Str(s) = name else { trap() };
// after
t is Ref
let s = name as! Str;
```

**Why zero-cost, and how to verify.**
- `is` lowers to the pattern test `match` already emits: one shift and
  compare, or a merged tuple test.
- `as!` is `let .. else { trap() }`, which is the same code.
- **To verify:** port Prolog's predicates and Python's traps, then diff
  the IR.

**Cost: S.** Both are front-end desugarings into existing TIR (`TK`
match and let-else). `is` needs a precedence slot; it is a reserved
word candidate, so check for clashes in the examples.

**Interactions.** None.

**As built** (2026-11-05), as `e is P` and `e as! V` ([language.md
§1.2](../language.md#patterns-literals-and-match)). Refinements:
- **`is` names a variant with fields alone** (`t is Ref`, not `t is
  Ref(_)`), at the top of the pattern and of each alternative, and a
  name that is not a variant or constant is an error rather than a
  binding, so `t is Reff` cannot be always true. Its precedence is that
  of `==`; `is` is not a reserved word (it is only an operator after an
  operand), so no example needed renaming.
- **`e is A | B` is one `=> true` arm per alternative**, `match e {
  A(_) => true, B(_) => true, _ => false }`. The examples wrote both
  that and `A(_) | B(_) => true`; the two give different block layouts
  in LLVM, and the arm-per-alternative form is the one Prolog's ten
  multi-variant predicates (in handlers among them) used, so those stay
  identical. MiniJS's and Python's `|` sites also compile to
  the same code; Mini's one (`v is Int | Num` in its bytecode
  compiler) moves two compile-time functions, one instruction shorter.
- **`as!` yields all the fields**: one field is its value, several a
  tuple (`let (a, b) = p as! Pair;`); a variant without fields is an
  error (test it with `is`).
- **The trap.** `let p = e as! V;` is checked as `let V(p) = e else {
  trap() };`, the TIR of the hand-written form, so the release code is
  the same; elsewhere `as!` is a `match` with a `trap()` arm. In a
  `--release` build the failure is `trap()` (an illegal instruction in
  an executable, a panic in a library, as `trap()` is). In a debug
  build it is a panic with the source location and the variant,
  `main.lm:12:13: as! Str of another variant`, like `unwrap` of None,
  which a bare `trap()` gave no hint of.

**Ports, and what `ir-diff` showed** (`--release --asm`, tail-call
dispatch, against the unported sources built by the same compiler):
- **Python**: all 78 `let X(v) = e else { trap() }` (vm.lm 29,
  builtins.lm 24, ops.lm 18, fmt.lm 4, value.lm 3), 6 predicates and 2
  empty-arm `if let .. {} else` tests: 708 of 708 functions the same.
- **Prolog**: 18 predicates and one empty-arm test: 445 of 445 the
  same.
- **MiniJS** (7 predicates), **Scheme** (4), **Regex** (1 unwrap):
  all functions the same.
- **Mini** (1 predicate): 624 of 626 the same in assembly; the two
  bytecode-compiler functions above differ.
 for `accumulate`, `variant .. wraps`, and `super` groups

**Pattern.** Several vm declarations list ops by name, and the author
keeps the lists in step with the op set.

**Evidence.**
- **Wasm's `accumulate` list:** 140 names (`vm.lm:278`–`:300`), each
  repeating a row of an `inline for`. Wasm FRICTION 221–233 explains
  why marking every eligible op is not an option: about 660 variants
  blew build time and size.
- **Stack's budget variant** `variant budget wraps [JMP, JZ, LOOP,
  CALL, PUSHI_LT_JZ, LT_JZ, LE_JZ, GT_JZ, DUP_PUSHI_LT_JZ]`
  (`vm.lm:511`) has to name every super that contains a jumping op.
  Adding a super silently drops it from the budget.
- **`#[allow(super_overlap)]`:** 54 occurrences. Wasm has 18
  consecutive ones on `#[forward] super ..` (`vm.lm:1151`–`:1169`),
  Mini 17, Stack 7 (`:477`–`:490`), Scheme 7, MiniJS 4.
- **Supers:** ~140 in all (Scheme 35, Mini 24, Wasm 19, Stack 18,
  MiniJS 15, Prolog 12, Lox 6, Regex 4, Python 4), with ~100
  `#[forward]`.
  - In Mini, MiniJS, Scheme and Lox, all 80 names are the parts joined
    with `_` (`mini/vm.lm:1412` `#[forward] super LOADI_SUB = LOADI;
    SUB;`; `scheme/vm.lm:948`–`:966`, 19 lines of `#[forward] super
    X_TEST = X; TEST;`). Only Prolog names some differently.

**Proposal.**
- **Op selectors**, usable wherever an op list is. Each selector is a
  predicate over what the compiler knows about an op:
  - `ops with label`: has a `pc + it` operand;
  - `ops that jump`: uses `jump`;
  - `ops in inline for #k`: the ops a given `inline for` block defines;
  - glob patterns: `I32_*K`;
  - unions and differences of these.
- **A super group:** `supers #[forward] #[allow(super_overlap)] {
  LOADI; SUB;  X; TEST; .. }` applies the attributes to every super
  inside, and a super without a name is named by its parts joined with
  `_`. The `Vm.OP_LOADI_SUB` constants and the disassembler's names
  stay what they are today.
- **Membership of supers:** a super belongs to a selector if any of its
  parts does, so `wraps ops with label` picks up `PUSHI_LT_JZ`.

**Why zero-cost.**
- Selectors resolve at compile time to the list written today.
- **To verify:** check that the resolved list equals the hand list. A
  `--explain-selectors` output can be compared to the hand list, then
  the IR diffed.

**Cost: S.** Variants and the accumulator already take op lists
(`vmparse.rs`, `vmacc.rs`). A selector is a filter evaluated after all
ops are known.

**Interactions.**
- **Debugger:** `#[unfused]` variants and `wraps` already define how
  supers behave.
- **Stencils:** accumulator variants are ops, so a stencil is built per
  variant as today.

### 12. A typed host-side decoder and checked encoders

**Pattern.** Host code, the Pike VM, peepholes and fuzzers matches on
`Vm.op_of(w)` and then reads fields with `Vm.FMT_field(w)`. It must
name the right format by hand: `Rx.A_a(w)` against `Rx.AJ_a(w)`. A
wrong format reads the wrong bits with no error. Encoders mask
oversized arguments silently: `Rx.CHAR(300)` into a 24-bit field gives
a wrong instruction, which only the verifier later rejects (regex
FRICTION 241, open).

**Evidence.**
- 42 generated field-accessor calls in host code: Wasm 20, Regex 9,
  Prolog 8 and others (`regex/pike.lm:93`–`:118`, `lox/compile.lm:608`).
- **Hand decoding:**
  - Prolog `compile.lm:179` and `:200` decode `((x >> 8) & 255)` by
    hand, because there is no format-independent `Vm.operand(w, i)`.
  - Mini's compiler has 10 raw masks where `Vm.set_ABC_a` and
    `Vm.with_opcode` exist: `compile.lm:408` `(w & 0xFFFF00FF) | (dst
    << 8)`, `:972`, `:1005`, `:1094`, `:1168`, `:1176`.
- **Encoder casts:** about 300 `as u8` casts at encoder call sites:
  MiniJS 114, Scheme 72, Lox 57, Mini 57. Encoders take the field's
  exact width (`language.md` section 3.3), so the bytecode compiler
  casts every register index.
- Regex FRICTION 147–165 is partly fixed: `Vm.Op` and `op_of` give an
  exhaustive `match`, but typed decoding is not done, and `Vm.label(w)`
  returns an option even for an op that always has a label.

**Proposal.**
- `Vm.decode(w) -> Vm.Insn`, an enum with one variant per op whose
  fields are the operands, named and typed as in the op's declaration.
  Its `match` is exhaustive and names fields, not formats.
- `Vm.operand(w, i) -> i64`.
- Encoders accept any integer type for a field, and reject constant
  arguments that do not fit, as a compile-time error. The `as u8` casts
  go away. The masking they replace is what the encoder already does.
- An opt-in `--cfg loimi_checked_encoders` adds a range check to every
  encoder.

<!-- snippet: skip design, not built -->
```loimi
// before
match Rx.op_of(w) { .CHAR => { let c = Rx.A_a(w); .. }, .SPLIT => { let x = Rx.AJ_a(w); .. }, .. }
// after
match Rx.decode(w) { .CHAR { c } => { .. }, .SPLIT { x, y } => { .. }, .. }
```

**Why zero-cost.**
- `decode` is a `match` on the opcode followed by the same shifts the
  accessors do. It is host code, never called from handlers, and
  compiled only if called. LLVM folds `decode` followed by `match` back
  into one switch.
- The constant-argument check is compile time.
- The checked encoders are off by default.
- **To verify:** check that handler IR is unchanged and that Regex's
  test suite passes.

**Cost: M.**
- Generating the enum is straightforward (`vmexpand.rs` already
  generates `Vm.Op`).
- The work is in supers and caches. A super's head should decode as its
  first part, as `op_of` does. Cache words can be fields of the variant,
  or left out.
- **Risk:** the `Insn` enum of a big vm, such as Wasm's 600+ ops, is
  large to compile. It is generated only if `decode` is called.

**Interactions.**
- **Stencils:** `decode` maps through `Vm.real_word` like every other
  generated function.
- **Debugger:** breakpoint words decode as the breakpoint op, as
  `op_of` does; `debug.unpatched` stays the way to read through them.

## 2. Diagnostics worth fixing

Better errors are zero-cost by definition. These are the ones that
still cost authors time:

- **One error per build:** item 7. **Built.**
- **Literal-only expressions typed `i64` without context** (python
  FRICTION 577, partly fixed). `(if x { 1 } else { 0 }) == y` with
  `y: i32` fails. Infer the literal's type from the other operand of a
  comparison or arithmetic operator, as the Prolog fix (FRICTION 243)
  did for some forms. **Built for the operator case:** an `if`, `match`
  or block whose results are all literals is typed from the other
  operand. The right operand is checked first with the type the left
  would have had alone, so programs that compiled are typed as before.
  The second half of the friction entry, `for j in 0..3` used with a
  `u32`, needs inference from later uses and is not done (the error
  already has a note suggesting `for j: u32 in ..`).
- **`-W handler_frame` says *that* a frame appeared, not *why* it
  appeared.** Prolog FRICTION 453 (open): adding a `deref` stopped LLVM
  proving an `i48` division's overflow panic dead, and only the warning
  hinted at it.
  - The lint finds the calls in the assembly (`framelint.rs`). Mapping
    a panic call back to its source panic, such as "the overflow check
    of `/` at vm.lm:512", would answer this directly.
  - `hot_gc` does not see calls inside plain vm functions (Lox FRICTION
    219, partly fixed). The same mapping would cover them.
  - Not done: it needs a panic-site table from codegen to the
    assembly lint, outside the checker.
- **`--vm-stats` super suggestions that are really missing constant
  operands** (Wasm FRICTION 1166, open). `CONST32_I32_ADD_CONST32` is
  the symptom of a missing `K` form. When a suggested super's head is a
  constant load, the report could say "or a constant-operand form of
  ADD". Not done: the report is printed by the stats runtime
  (`vmgen.rs`), which would need to know which ops load constants.
- **Embedded guest-language source** (the Prolog prelude in a Loimi
  string, FRICTION 251, partly fixed): errors report lines of the
  string. A `include_str("prelude.pl")` that keeps a file name for host
  diagnostics would fix it; it is zero-cost because the bytes are the
  same constant. Not done: a new expression form in the parser and
  loader, which are another lane's.
- **Operands map to format fields by position**, so `op DELGLOBAL(z:
  imm, name: kst) : ABx` needs a dummy operand (Python FRICTION 579; 6
  sites in 4 vms, e.g. Mini's `unused: imm` at `vm.lm:997`). The message was fixed. A named binding (`name: kst @ bx`)
  removes the dummy, and is S. Not done: it changes the vm parser and
  the operand layout in `vmexpand.rs`.

## 2a. Smaller items

These are each small and real, but too narrow to rank:

- **`v.index_of(pc)` on `Verified<Vm>`.** It would replace 13 copies of
  `((pc as u64) - (start as u64)) / 4`, with the word size hard-coded
  each time (`mini/vm.lm:171`, `mini/debugger.lm:434`, `:508`, `:909`,
  `scheme/vm.lm:61`, `minijs/vm.lm:364`, `lox/vm.lm:216`,
  `python/vm.lm:202`, `:1433`, `:1505`, `stack/vm.lm:525`,
  `python/debugger.lm:373`, `:502`). It is the same arithmetic. S.
- **`become match st { OwnIn => GETPROP_IN, .. }`.** It would replace
  48 `if st == obj.IcState.X { become Y; }` ladders: Lox 25
  (`vm.lm:540`–`:543`, `:567`–`:571`) and MiniJS 23. The ladders map
  `std.obj`'s cache state to a family member, the same way at each
  site. The compiled code is the same `if` chain. S.
- **A lint for raw masks on instruction words** in host code where a
  generated accessor exists (Mini's 10 sites, item 12). S.

## 3. Considered and rejected

- **Implicit outlining of panics in handlers** (unwrap, bounds checks).
  This was measured: it removed the frame of 1 of 218 handlers
  (`language.md` section 4). Making `raise` cold (item 6) is the part
  that pays.
- **Making `?`'s error path cold by default** (an extension of item 6).
  `?` sits on hot calls everywhere, and moving its error arm changes
  code layout in every handler that uses it: there are 137 sites. That
  is not identical code, and not obviously no worse. It is better done
  per site.
- **Lifetimes-checker ergonomics.** The checker costs the examples 13
  `unsafe { }` blocks and no attributes. Of those, 5 are `unsafe { new(..,
  n) }` of `Gc` arrays filled right after (`lox/vm.lm:409`,
  `mini/vm.lm:619`, three `Protos` arrays). A `new_with(h, T { .. }, n,
  |i| ..)` would remove them, but it is not worth a feature at that
  count.
- **Parameterized operand kinds** (`operand r<T> = *((regs + it) as *T)
  ..`; Wasm has 16 kinds differing only in `T`, `vm.lm:234`–`:253`).
  `inline for` already works among vm items and can generate these.
  This is a usage gap, not a language gap, so a doc example is enough.
- **Generic `comptime` / `const fn`** (`language.md` section 1.4, not
  built). Items 2 and 3 cover the tables the examples actually keep,
  with much less machinery and no interpreter in the compiler.
- **Automatic superinstruction and accumulator selection.** This
  changes the code by design, so it is a performance feature, not an
  ergonomic one at zero cost. The selector syntax of item 11 removes
  the list-keeping part.
- **Trailing arrays in plain structs** (Prolog frames, FRICTION 170,
  partly fixed). The raw arithmetic is wrapped in 4 helpers
  (`machine.lm:95`–`:102`) and one operand kind. That is too few sites.
- **`bool as i32`** (Wasm FRICTION 1072, still listed as open). It
  works on master: `true as i64` compiles and prints 1. Wasm's `b2i`
  helper (`vm.lm:138`) and Python's 21 `if b { 1 } else { 0 }` can just
  use it. The Wasm log's follow-up table should be updated.
- **A `state { fp: *Frame synced(ctx.fp) }` shorthand for `sync`.** The
  `sync`/`reload` blocks are one or two lines per vm (8 vms). That is
  too little to remove.
- **Derived state registers** (`state { fp, regs = fp.base, k = fp.k
  }`, reloaded whenever `fp` is assigned). The pop sequence `fp = fp -
  1; regs = fp.base; k = fp.k; jump ..` is written 12 times in Mini,
  MiniJS, Scheme and Lox, and 5 more in Python. But a `-> never` vm
  function already factors it with identical code. The derived form also
  hides which registers are reloaded where, which the authors tune by
  hand.
- **Named sub-variants of a payload** (Scheme's `Imm(i32)` holding
  `'()`, `#f` and `#t`, with 74 `IMM_*` uses). Since the shared nanbox
  layout, nullary variants each take one word and no block
  (`language.md` section 2). So Scheme could declare `Nil`, `False` and
  `True` as variants today. Its comment (`value.lm:4`–`:9`) predates
  the shared layout. Whether that keeps the fixnum and pair tests is a
  layout question, which `--explain-values` answers. This is an
  adoption gap, not a language gap.
- **A sampling profiler** (Wasm FRICTION 368 and 1074, Prolog 547 and
  592, Python 617; three authors wrote an `LD_PRELOAD` sampler). This
  is real, zero-cost to the default build, and worth doing, but it is
  tooling rather than language ergonomics. It belongs in `std.prof`'s
  roadmap: sample `pc`, map it to op and source line through the line
  tables the interpreters already keep.

## 4. Verifying "zero cost" in general

Every item above is verified the same way:
1. Port one or more examples.
2. Build them with `--release --emit-llvm` in `tailcall`, `goto` and
   `switch`, with and without `--jit-stencils` and `--cfg debug` where
   the example has them.
3. Diff the IR against master.

The diff should be empty, apart from panic location strings when lines
moved. Item 4's `ir-diff` removes that exception, which is why it is
worth building early. Items 6 and 8 are no worse rather than identical
where existing code was not already in the hand-written form. For
those, the check is:
- an empty diff on the sites that were;
- a list of the sites that were not, each checked with `-W
  handler_frame` and a benchmark run.
