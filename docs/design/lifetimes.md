# Design note: lifetimes and pointer stability at compile time

Status: **phase 1: design and prototype** (`src/lifetimes.rs`, opt-in:
`#![safe]` and `#![lifetimes]` modules), **migration step 2 done**: every
module of the example interpreters is `#![lifetimes]` (section 9), and
checking all of them costs under 10% of the front end (section 8). For
review before phase 2. Written 2026-11-04.

> Where real bugs keep appearing is lifetimes: a pointer or slice into a
> container used after the container grew, was freed, or was collected.
> Can the compiler reject these, with **zero run-time cost** and without
> the annotation burden of a borrow checker?

**Answer.** Mostly, yes, with an inferred *pointer-stability* rule
instead of a borrow checker. A pointer or slice is abstracted by *where it
points* (a path such as `v.ptr.*`: "into `v`'s buffer"); a call that may
move or free memory (it reaches `free`/`realloc`, or may collect) makes
every pointer into that memory stale; using a stale pointer is an error.
What each function may move or free, what its result points into, which
arguments it uses after moving another's memory, and which it may keep
are *inferred* over the whole call graph, so nothing is declared except
one attribute on the one std function that handles self-aliasing on
purpose (`#[may_alias(s)]` on `Vec.extend`). The prototype rejects every
bug of the corpus below (12 test programs), accepts all nine `#![safe]`
std modules (`text`, `diag`, `map`, `dict`, `lex`, `parse`, `hash`, `io`,
`bc`) unchanged, found 3 sites when *every* module of every example
interpreter was first checked (one a real dangling pointer, since
fixed), and leaves the release IR of every example byte-identical. The
examples' 77 modules now opt in (section 7). What it does not do is
follow a pointer once it has been stored in memory and loaded again
somewhere else: section 6 lists that and the other holes, and section 9
the questions for phase 2.

## 1. The problem

`docs/design/safety.md` made `*T` in a `#![safe]` module "a reference to
one live value", and left lifetimes out (its section 2, item 1): a `*T`
or slice obtained from a container dangles once the container grows,
shrinks or is freed, `&local` may outlive its function, a copied `Vec`
shares its buffer, and a slice of a GC object dangles once the object is
collected. ASan builds catch most of these at run time (safety.md section
7), but only on the paths a test runs, and not in the GC heap.

Loimi deliberately has no borrow checker (language.md section 11), and
its interpreters are built on pointers that a borrow checker cannot
express: register windows into a value stack, frames, inline caches, an
arena of AST nodes pointing at each other, structs holding pointers to
the heap, the context and the parent scope. Any solution has to leave
that code alone (or confine it to `unsafe`) and cost nothing at run time.

## 2. The bug corpus

Real bugs from this repository's history (a search of the 751 commits,
CHANGELOG, safety.md, COROUTINES.md and the FRICTION files), each with a
minimized test that the prototype rejects. "Compiler" bugs (missing
roots in the generated code: 17ef2e2, 538e067, 334d6cc, cc6b3d2,
b56a15c, 16bd0a9, 4ab59b0, 63a3b30, 4f232c5, 9b081ae) are bugs in the
compiler or collector, not in Loimi code a checker could see, and are
out of scope; so are stale *decoded* state bugs (0456845, 5a77ca9).

| # | bug | where, fixed by | shape | test (`tests/fail/`) |
|---|---|---|---|---|
| 1 | `b.push_str(b.as_str())` read freed memory: `reserve` moved the buffer under `s` | std.text, a008aad (`Vec.extend` re-derives) | an argument points into what the callee grows | `lt_push_own_text`, `lt_grow_while_borrowed` |
| 2 | Python `list.index`/`count`/`<` scanned `l.arr.items` while `__eq__` grew the list and collected the old array | examples/python, a06bb38 (re-read per step) | slice of an unrooted GC object across a collecting call | `lt_gc_slice_across_collect` |
| 3 | `OrderedMap.get_ptr`/`entry` pointer used after an insert that resizes | std.map API (documented hole) | pointer into a container across a restructure | `lt_map_ptr_insert` |
| 4 | `Arena.free` then a node used; `HashMap.free` then a slot pointer used | std.mem 238ca34, std.map 738c2e3 (the containers' own state; the users' pointers stay a hole) | pointer across a free | `lt_arena_free`, `lt_hashmap_free` |
| 5 | a `Vec` copied: two owners, one grows and frees the other's buffer | safety.md section 2 | copy aliasing | `lt_vec_copy`, `lt_vec_copy_field` |
| 6 | `&local` returned or stored; `rt.panic` kept a pointer to a message in a dead frame | 6116e20 (copies the message) | local escape | `lt_local_return`, `lt_local_store`, `lt_local_kept` |
| 7 | Mini's coroutine stacks move on growth: handler and native pointers into the old block (`call_value` switched to indices, `reload`, `VStack.prev`) | examples/mini (20ac4b2..3706bbf, COROUTINES.md item 8) | pointer derived from vm state across a `reenter` call | `lt_vm_reenter_stale` |
| 8 | objref's remaining hole: a read-only slice of an object held across a call that may collect | safety.md section 8.1 | same as 2 | `lt_gc_slice_across_collect` |

Two more found by the prototype itself (section 7): `minijs`'s
`Gen.top` stored `&fs` (a local) in `g.fs` and returned without
restoring it (latent: nothing read `g.fs` afterwards; fixed), and a
false start of this note's own accepted test (`tests/run/lt_accepted.lm`
used a slice after a loop of pushes).

## 3. Options

Each option was judged on the corpus, on the annotation burden (counted
on `std.text`, `std.map`, `std.diag` and the Lox front end:
`examples/lox/parser.lm`, `compile.lm`), on what it does to interpreter
cores, on error messages, on its implementation in this compiler, and on
compile time. Run-time cost is zero for all but E.

### A. Rust-style borrow checking with lifetime parameters

Every reference gets a lifetime; structs that store references take
lifetime parameters; `&mut` is exclusive.

Counted by reading the code (the Lox front end about 85% of
`parser.lm`, 45% of `compile.lm`, extrapolated), with Rust's elision
rules applied to `x: *Self` receivers:

| file | types needing `'a` | explicit signatures | exclusivity rejections (redesigns) |
|---|---:|---:|---:|
| std.text | 0 | 0 | 1 (1: `Interner.names` points into its own `arena`) |
| std.map | 0 | 1 | 3 (`HashMap.probe` is NLL problem case #3; `find_with`'s closure captures a map that guest code may change) |
| std.diag | 4 | 5 (+4 advisable) | 2 (lazy line index mutates in queries; `Diagnostics` is self-referential) |
| Lox parser + lexer | 13 (≈22 params) | 22 | ≈10 (4) |
| Lox compile | 2 (+ `Proto` spreads into the vm) | ≈5 | 10 (1) |
| total | ≈20 types | ≈33 | ≈26 sites, ≈8 redesigns |

- Corpus: all 8, by construction (exclusivity rejects 1 and 3 at the
  call, lifetimes reject 4, 5 and 6; 2, 7 and 8 need GC and vm state
  modeled as borrows, which Rust does not do either).
- The clox structure every front end here uses is rejected outright:
  stack-local `FnState`/`ClassState` hung off the parser (`p.fs = &fs`)
  with parent links, arena `Fun` nodes mutated after linking, a Pratt
  table called with its owner (`p.pratt.expr(p)`). Each becomes an index
  stack or a `Cell`.
- Interpreter cores: register windows, frames pointing into the stack
  of the same machine, open upvalues into the stack, inline-cache words
  in the code: none expressible; all stay raw.
- Implementation: a new dimension of the type system (lifetime
  parameters, variance, inference, two-phase borrows, NLL), which
  language.md section 11 rules out for good reason.

### B. Second-class references (Hylo/Val-style)

A reference may be passed down to a call, or returned as a projection
of a parameter, but never stored in a field, global, container or
closure. No lifetime syntax at all; everything that is stored today must
become something else.

Counted on the same files: field positions that store a pointer,
`str` or slice today and would have to change type:

| category | text | map | diag | Lox front end | total | would become |
|---|---:|---:|---:|---:|---:|---|
| strings borrowed from the source | | | 2 | 13 | 15 | spans or interned ids |
| message strings (mostly literals) | | | 3 | 1 | 4 | a `'static` kind |
| arena-owned strings | 1 | | | | 1 | an arena kind |
| arena AST/IR links | | | | 31 | 31 | an arena kind, or `NodeId` indices |
| owners, contexts, parents | | | | 10 | 10 | a program/heap-lifetime kind, or an index stack (`Parser.fs`, `enclosing`) |
| runtime objects | | | | 2 | 2 | `Gc` |
| owned buffers | | 3 | | | 3 | an owning kind |

- About 60 field positions in five files, 50 of them in one front end.
  B is workable only with two or three blessed storable kinds (arena
  pointers, program-lifetime pointers, spans), at which point it is a
  coarse region system (option D) without the proof.
- What B keeps is valuable: every std accessor (`get_ptr`, `next`,
  `entry`, `as_str`, `items`) is a projection of a parameter and stays
  as it is. That is the part option C builds on (question 1).
- Corpus: 3, 5, 6, 7 by forbidding the stores; 1 and 2 still need the
  "argument into what the callee moves" and GC rules of C.
- Interpreter cores: as A; nothing on the hot path is a downward-only
  reference.

### C. Pointer stability ("freeze") with inferred summaries (built)

A pointer or slice is valid until something moves or frees the memory
it points into; a call that may do so ends it. What a function may move
or free is a summary inferred over the call graph (generalizing objref's
"may store / may keep" fixpoint, section 8 of safety.md), and `uses gc`
counts as "may free any GC object no local holds". Nothing about stored
pointers changes: the rule is about *uses after a restructure*, and about
the one thing that never outlives its function, the address of a local.

- Corpus: all 8 (12 tests).
- Burden: std.text, std.map, std.diag, and every other safe std module:
  **0 annotations, 0 changes**; std.mem: 1 (`#[may_alias(s)]` on
  `Vec.extend`). Every example module checked (`LOIMI_LIFETIMES=audit`):
  3 sites (section 7), since resolved; the examples now opt in. No
  signature changes anywhere.
- Interpreter cores: unchanged; vm handlers are checked for pointers
  derived from the state across `reenter` calls (corpus 7), which is the
  `reload` contract of language.md section 5.1 made a check.
- Messages: name the stale variable, the call that may have moved its
  memory, and the fix (section 5.6).
- Implementation: one pass over the typed IR after objref, ~1400 lines,
  no change to the language beyond `#![lifetimes]` and `#[may_alias]`.
- Compile time: section 8.

Its limit is the price of no annotations: a pointer stored in a struct
and loaded elsewhere is tracked only as "the memory at that field", not
as "a pointer into that container" (section 6).

### D. Regions / scoped allocation

Tie pointers to a lexically scoped region (MLKit, Cyclone, Rust's
`'arena`), so a pointer cannot outlive its arena. This is what Loimi's
arenas already are informally, and C covers the part that matters at
the use site: `new(&a, ..)` points into `a`'s chunk, and `a.free()`
makes it stale (corpus 4). What C leaves open, a node *stored* in
another structure that outlives the arena, is exactly what region
*types* would add (`*'a Node`); the cost is a region parameter on every
AST and IR type (`Expr<'a>`), i.e. option A for the front ends.
Recommended only as a possible phase-2 refinement for arena pointers
(question 3), not as the base.

### E. Generational references (Vale) — contrast only

Each allocation carries a generation; each pointer remembers it; each
dereference compares. Catches everything at run time, but costs a word
per pointer and a load and compare per access: rejected by the zero-cost
requirement (and safety.md section 7 already rejected the debug-only
variant for changing representations).

### Comparison

| | A. borrow checker | B. second-class refs | C. pointer stability (built) | D. regions | E. generations |
|---|---|---|---|---|---|
| corpus caught | 8/8 (2, 7, 8 only with GC/vm state as borrows) | 4/8 alone (3, 5, 6, 7) | 8/8 | 4, 6 | 8/8 at run time |
| annotations, std.text+map+diag | 4 struct params, 6 signatures, 6 rewrites | 0 syntax; 9 fields change type | 0 | region params on stored pointers | 0 |
| Lox front end | ≈15 struct params, ≈27 signatures, ≈20 sites (5 redesigns) | ≈50 fields change type | 0 (all examples: 3 sites) | `Expr<'a>` everywhere | 0 |
| interpreter cores | `unsafe`/raw only | `unsafe`/raw only | checked where possible (reenter rule), raw elsewhere as today | raw | slower |
| stored pointers | checked | forbidden | not followed (hole) | checked for regions | checked |
| run-time cost | 0 | 0 (but copies/indices) | 0 | 0 | per access |
| compiler work | a new type system dimension | moderate (new handle types) | one inference pass | new types | runtime + codegen |

## 4. Recommendation

Adopt C as the base (built), keep the language without lifetime syntax,
and decide in phase 2 how far to go on stored pointers (question 1):
either accept the hole (ASan builds catch most of it), or add one cheap
rule: a pointer derived from a container's *growable* interior
(`ptr_at`, `items`, `get_ptr`, `entry`, `file`, `at`) may not be stored in
memory that outlives the function, only arena allocations and
long-lived owners may. That is B restricted to the one kind of pointer
that dangles on growth, and the audit says it would touch few sites.

## 5. The rules (as built)

### 5.1 Where pointers point

Each value that holds pointers (a `*T`, a slice, a struct holding them,
a `Gc`) is abstracted by a set of **paths**: a root, then field steps
`.f` and dereferences `*`:

- roots: a local or parameter of the function, a global, a GC object
  held by an immutable local (`let`) or parameter, or "some other GC
  object";
- `&x` is the path `x` (the local's own storage), `&p.f` is `(paths of
  p).f`, `p.f` read as a pointer is `(paths of p).f.*`, `o.items` is
  `o.items` in the object's block, a slice of a local array is the array;
- a struct or slice read whole adds `.*` through "some field" (`v.*`:
  the buffers a `Vec` value owns); pointer arithmetic, casts and
  sub-slices keep the paths; integers point nowhere;
- a call's result points where the callee's summary says (below), with
  the callee's parameters replaced by what the arguments point into;
- a callee without a body (an extern) or a function value: its result
  points into what its pointer arguments point into, **where the types
  allow it**: a `*R` or `[]R` points at `R` values, so an argument
  `*T` counts only if some `R` of the result type can be stored inline
  in a `T` (a parse hook given `*Parser` returns a `*Node` and a `str`
  error, neither of which a `Parser` holds inline); byte pointers,
  generic and non-pointer arguments count always
  (`tests/run/lt_fn_value.lm`, `tests/fail/lt_fn_value_kept.lm`);
- memory of unknown origin (`malloc`'s result, an extern's pointer
  from no pointer argument) is the root **fresh**: never moved or freed
  as far as the rule knows, and a store into it is judged by whether
  the function returns it (section 5.4); a constant points nowhere.

Paths are cut at 8 steps; long pointer chains (`f = f.parent` in a
loop) are approximated (section 6).

### 5.2 Restructuring

A **restructure** of path R means the block at R may be moved or freed.
It invalidates a path P when R is a prefix of P and the rest of P stays
in that block (no further dereference): freeing `v.ptr.*` invalidates
`v.items()` and `&v.items()[i].f`, but not a `str` *stored in* the
buffer (`v.ptr.*.name.*` lies in another block). Sources:

- `free(p)` and `realloc(p, n)` restructure what `p` points into;
- a call restructures what its summary says, through its arguments;
- a call that may collect (`uses gc` or `reenter`, also through a
  function value's type) restructures every GC object not held by a
  `let` local or parameter: the compiler roots those while they are in
  scope, and the collector does not move objects (question 4);
- in vm code, a call that may re-enter (`TVm::Synced` with a `reload`)
  restructures what the state variables assigned by `reload` pointed
  into: a pointer computed from `regs` before the call is stale after it.

A local whose paths are invalidated is **stale** until it is assigned
again; flow-sensitive, joined at branches and iterated to a fixpoint in
loops.

### 5.3 Summaries (inferred, whole program)

For every function reachable from a checked one, a least fixpoint over
the call graph (a worklist, callees first):

| summary | meaning | example |
|---|---|---|
| restructs | parameter (or global) paths it may move or free | `Vec.push`: `v.*.ptr.*`; `Arena.free`: `a.*.chunk.*`, ...; `Arena.alloc_bytes`: only `a.*.chunks.ptr.*` |
| ret | parameter paths its result may point into | `Vec.items`: `v.*.ptr.*`; `SourceMap.file`: `sm.*.files.ptr.*` |
| pairs | (i, j): it uses argument j after restructuring through argument i | the old `push_str`: (b, s) |
| keeps | parameter j's pointee may be stored in parameter i's memory, or elsewhere | `Vec.push`: `x` into `v`; `SourceMap.add`: `name`, `text` into `sm` |

`Arena.alloc_bytes` shows why inference beats declarations here: it
reallocates its chunk *list* (a `Vec`) but never the chunks, so pointers
from `new(&a, ..)` survive allocation and die only at `a.free()`,
with nothing written down.

`#[may_alias(p)]` on a function says that argument `p` may point into
memory the function restructures through another argument: its
`pairs` with `p` are not checked or propagated. Only `Vec.extend` has it
(its cold path re-derives `s` inside the moved buffer).

### 5.4 The errors

In a checked function, outside `unsafe { }`:

1. **Use after restructure**: reading a stale local (also through `&x`
   or a method call).
2. **Argument into what the callee moves**: a call whose summary has
   pair (i, j), passing in j a pointer into what it restructures
   through i (`b.push_str(b.as_str())` with a `push_str` that grows
   first), unless `#[may_alias(j)]`.
3. **Copy of an owning value**: a struct value holding pointers copied
   (`var w = v`) makes two owners of the same buffers; a restructure
   through either makes the other stale (a local), or the place it was
   copied from (`var s = c.stack; s.push(..); c.stack.at(0)`).
   Moving (the original not used again, or assigned first) is fine:
   `Interner.grow`'s `var old = it.slots; it.slots = ..; old.free()`.
4. **Local escape**: returning the address of a local (its storage, a
   slice of a local array), storing it in memory that outlives the
   function and is not restored before it returns (`g.fs = &fs; ..;
   g.fs = parent` is fine), passing it to a function whose summary keeps
   it outside the caller's own locals, or capturing it in an escaping
   closure. `main`'s locals live as long as the program and may escape.
   A store into memory of unknown origin (fresh, or reached through no
   known path) counts as outliving the function; but when the function
   returns that fresh memory, the summary says it keeps the pointer **in
   its result**, and the caller judges where the result goes: into one
   of its own locals or a field of one (`m.t = Table.new(&m.c)`, Lox's
   `m.strings = gc.WeakInterner.new(&m.heap, ..)`) the address stays in
   the frame; returned, it is kept in the caller's result in turn;
   stored anywhere else, or passed on, it escapes
   (`tests/run/lt_fresh_kept.lm`, `tests/fail/lt_fresh_kept.lm`).

### 5.5 Where it applies

Modules with `#![safe]` (implies it) or `#![lifetimes]` (the check
without safe's raw-pointer rule, so a module with raw pointers and a
`vm` can opt in). Summaries are computed for everything those modules
call, in any module (std's unsafe core included: that is where `free`
and `realloc` are). `unsafe { }` silences the errors inside it.
`LOIMI_LIFETIMES=audit` checks every module and prints each site
(`lifetime-site file:line:col message (function)`) without failing;
`LOIMI_LIFETIMES=off` skips the pass.

### 5.6 Messages

<!-- snippet: fail "may dangle"
#![safe]
import std.mem;
fn main() {
{snippet}
}
-->
```loimi
var v: mem.Vec<u64> = mem.Vec.new();
v.push(1);
let s = v.items();
v.push(2);
let x = s[0];
```

```
error: `s` may dangle: it points into memory that the call of `Vec<u64>.push` may have moved or freed
  --> t.lm:8:13
    8 |     let x = s[0];
      |             ^
note: the call of `Vec<u64>.push` may move or free it here; take the pointer or slice again after this call, or keep an index
  --> t.lm:7:5
    7 |     v.push(2);
      |     ^^^^^^^^^
```

```
error: this argument points into memory that `Buf.push_str` may move or free (through argument 1) while it still uses this one
error: `xs` may dangle: it points into memory that the call of `eq` (which may collect) may have moved or freed
error: `dst` may dangle: it points into memory that this `reenter` call (guest code may grow or move the vm's stacks; `reload` recomputes the state) may have moved or freed
error: stores the address of local `n` in memory that outlives the function
error: passes the address of local `buf` to `SourceMap.add`, which may keep it beyond the call
```

## 6. What it does not check (the holes)

1. **Pointers stored in memory.** A pointer loaded from a field is
   tracked as "the memory that field points to", not as what it was
   derived from. `ctx.cur = ctx.frames.ptr_at(i)` in one function and
   `ctx.frames.push(..); *ctx.cur` in another is not caught (in one
   function it is not either, once it went through memory). Arena
   pointers stored in nodes and used after `a.free()` elsewhere: same.
   This is the main hole; question 1.
2. **Parameters are assumed not to alias each other's buffers** inside
   a function body (`fn f(a: *Vec<u8>, b: *Vec<u8>)` taking `a.items()`
   and pushing to `b`); the `pairs` summary catches the caller passing a
   pointer into the same container (`b.push_str(b.as_str())`), but not
   two pointers to the same `Vec` (`f(&v, &v)`). Rust's `&mut`
   exclusivity is what closes this; question 2.
3. **Closures and function values.** A call through a function value
   restructures nothing (except by its `uses gc` effect), and the body
   of a closure passed to a call is not part of that call's summary.
4. **Approximations of the abstraction**: paths are cut at 8 steps,
   and two pointers both cut at the same depth are taken to be in
   different blocks (pointer-chasing loops); so is a restructure cut at
   the depth bound and a pointer that ends exactly there (`release(&t.inner)`
   with `t` 8 steps deep frees a block below `t`'s:
   `tests/run/lt_inference.lm`), and summaries drop restructures cut at
   the bound altogether (a recursive tree walker's summary otherwise
   lists every combination of child fields 8 steps down: MiniJS's
   `Res.stmt` had 758, 743 of them cut); a place assigned a new
   value detaches what pointed into its old buffers (`var s =
   p.items(); p.v = other; free through a copy` is missed); externs
   other than `free`/`realloc` are assumed not to free; `defer`s are
   evaluated at the end of their block only; a `?` or `never` call is
   not an exit for the escape rule; all fresh memory is one root, so a
   fresh block's stored pointers are never taken as overwritten.
   Summaries are path-insensitive: a store guarded by a flag argument
   counts for every caller (Mini's `coro.call` stored `args` only when
   `from_vm`, which `call_value` passes false; section 7).
   A closure's captured owning value (a `Vec`, a `StrBuf`) is the
   enclosing variable itself and starts with no paths, like a `let`
   owner, so growing it inside the closure does not make it stale
   (Mini's debugger: `tests/run/lt_inference.lm`); what a captured
   slice points into is not related to the captured owner (hole 3).
5. **The collector is assumed not to move objects.** A slice of an
   object held by a `let` is accepted across a collection; a moving
   collector would make every such slice stale (the rule would then be
   one line: no GC root is exempt).
6. **Logical staleness is not memory unsafety**: a slice kept across
   `v.clear()` or `truncate` still points into allocated memory and is
   accepted (ASan builds poison it, safety.md section 7); so is a slice
   of a rooted list's old array after guest code replaced it (CPython
   reads the current list; Python's fix does too).
7. **Trusted code**: `unsafe { }` and modules that do not opt in are
   not reported (their summaries are still inferred and used).

## 7. Evidence

**Corpus**: `tests/fail/lt_*.lm` (12 programs over the 8 shapes, plus
`lt_may_alias_param.lm`), each rejected with the message above;
`tests/run/lt_accepted.lm` runs the fixed version of each (re-derive
after growth, `let`-held GC object, arena nodes freed last, moving a
`Vec`, a local's address stored and restored, self-append through
`Vec.extend`).

**std**: the nine `#![safe]` modules are checked in every build that
uses them (all example interpreters and tests): no error, no change but
the one attribute.

**Audit of all examples** (`LOIMI_LIFETIMES=audit`, every module and
function of Lox, Mini, Mini's embedding, MiniJS, Prolog, Python, the
regex library, Scheme, Stack, Wasm and the tutorial's final step, unsafe
std included): the first version of the rule reported 39 sites; three
refinements (a place assigned a new value detaches its old buffer;
deep pointer chains; escapes judged at function exit and by
destination: into the caller's own locals is fine, `main`'s locals live
forever) removed the 36 false positives, leaving:

| site | verdict | resolution |
|---|---|---|
| `examples/minijs/compile.lm` `Gen.top`: `g.fs = &fs`, never restored before returning (`Gen.function` restores it) | real dangling pointer, latent (nothing read `g.fs` after `top`) | fixed: `top` restores `g.fs` before returning, as `function` does |
| `examples/mini/vm.lm` `sort_less`: `&ab[0]` (a local array) passed as `args` to `call_value`, whose summary keeps `args` in the context | no hazard today: the only store, `c.xfer = args` in `coro.call`'s yield, runs only when `from_vm`, and `call_value` passes false (a yield from host code is an error); the summary is path-insensitive (the embedding's `s.args = args` is on another path, not reached from `call_value`). But the store lent a pointer to the context through a native's arguments | fixed in the code: the CALL instruction sets `ctx.xfer` itself (the yield's values are its own arguments, `regs + base + 1`), so `coro.call` no longer stores `args`; IR changes in `coro.call` and the CALL handler's yield branch only |
| `examples/minijs/compile.lm` `compile`: `&p` (a local parser) passed to `Parser.program`, which "may keep it" | false positive: the Pratt hooks are function values, whose result was taken to point into every pointer argument, so `Parser.primary`'s `*Node` "pointed into" the parser and storing a node in a node kept the parser | inference: a function value's (or extern's) result points into an argument's block only where the types allow it (section 5.1) |

**Opting in** (migration step 2): all 77 modules of Lox, Mini (with its
embedding, coroutines, threads and debugger), MiniJS, Prolog, Python
(with its debugger), the regex library, Scheme, Stack and Wasm are
`#![lifetimes]` (the tutorial's steps are not: they follow the
tutorial's text). Built in every configuration the tests use (`--cfg
debug`, `--cfg gil`, `--jit-stencils`, `--shared`), the new findings
were:

| finding | verdict | resolution |
|---|---|---|
| `examples/stack/fuzz.lm` `run_one`: the machine `m` used (`m.heap.free()`, ..) after `prog.free()`, while `m.words` and `m.consts` point into the program (not in the original audit list: it missed the fuzzer) | latent: the machine's root scanner reads `m.consts`, but freeing the heap does not scan | fixed: the program is freed after the machine |
| (after fixing a hole: a store through a pointer of unknown origin, `*t = ..` with `t` from `malloc`, was taken as a store into a local) `examples/lox/driver.lm`: `gc.WeakInterner.new(&m.heap, ..)` keeps `&m.heap` in the table it allocates | false positive: the table is stored in `m.strings`, a field of the same local, and freed with it | inference: fresh memory and "kept in the result" (section 5.4) |
| `examples/mini/debugger.lm` (`--cfg debug`): the key sorter's closure grows the captured `StrBuf`s `a`, `b` and reads them again | false positive: a captured owner started as "pointing into its own buffers" | inference: captured owners start pathless (section 6, item 4) |
| `examples/mini/embed.lm` `mini_thread_detach` (`--cfg gil`): `t`, 8 steps deep, used after `threads.release(&t.core)` | false positive of the depth bound: the freed block is below `t`'s | inference: a restructure cut where an uncut pointer ends is another block (section 6, item 4) |

No `unsafe { }` was needed. None in the example interpreters' vm
handlers: they already recompute pointers after `reenter` calls. With
every example module opted in, `LOIMI_LIFETIMES=audit` reports nothing
on the examples.

**IR**: release `--emit-llvm` of Lox, Mini, Mini's embedding, MiniJS,
Prolog, Python, regex (library and fuzzer), Scheme, Stack (and fuzzer),
Wasm and the tutorial, base compiler against the new one with the same
std (the attribute line blanked for the base): byte-identical. After
step 2 (the attribute takes the place of a blank line, so no line
moves), against the tree before it: byte-identical for every example
(also Mini `--cfg debug`, `--cfg gil` and Python `--cfg debug`) except
the three intended code fixes: MiniJS (`Gen.top`), Mini and its
embedding (`coro.call`, the CALL handler's yield branch) and the Stack
fuzzer (the order of frees).

## 8. Cost

- **Run time: none.** The pass only rejects programs; the IR is
  unchanged (section 7).
- **Compile time** (front end and IR, `--emit-llvm`, best of 3, against
  `LOIMI_LIFETIMES=off`). Checking *everything* (the audit, which is also
  what every build costs now that all example modules opt in) cost, in
  the prototype: Lox +46%, Mini +88%, MiniJS +126%, Python +107% (+577
  ms), Scheme +62%, Wasm +280% (3.0 → 11.5 s), the rest +16-40%. The
  state was cloned at every branch: Wasm's generated verifier (5716
  locals, a loop over a match of every opcode) took 4.5 s alone,
  analyzed twice. Now:

  | example | off | all modules checked | |
  |---|---:|---:|---:|
  | Lox | 264 ms | 283 ms | +7% |
  | Mini (embedding) | 413 (430) | 448 (452) | +8% (+5%) |
  | MiniJS | 531 | 566 | +7% |
  | Prolog | 259 | 275 | +6% |
  | Python | 535 | 592 | +10% (the pass: 52 ms) |
  | regex (lib, fuzz, stats) | 68-70 | 72-73 | +4-7% |
  | Scheme | 336 | 357 | +6% |
  | Stack (fuzz) | 140 (140) | 148 (151) | +6% (+8%) |
  | Wasm | 3052 | 3065 | +0.4% |
  | tutorial step 9 | 165 | 171 | +4% |

  Against a whole release build (clang -O3) the pass is under 1%
  (Python: 52 ms of 11.8 s). What fixed it, in order of effect: locals
  leave the state when their scope ends and only parameters and
  unbound locals start with a value (the verifier: 4.5 s → 17 ms); the
  state's parts are shared (`Rc`) and copied only when a branch changes
  them, paths are inline arrays, and a join skips shared parts; each
  function's errors come from its last summary analysis instead of one
  more pass; one restructure scan per call for all of the callee's
  paths, and the summary takes a callee's paths once per argument path;
  the same reason at the same place has the same index (a loop's state
  reached its fixpoint only at the iteration cap when a store was in
  it); summaries drop restructures cut at the depth bound (section 6);
  a post-order worklist; a fast hasher for ids. `LOIMI_LIFETIMES_STATS=1`
  prints the pass's time and its slowest functions.
- **Implementation**: `src/lifetimes.rs` (~2000 lines, no dependency
  on codegen), `#![lifetimes]` and `#[may_alias]` in the parser and
  checker, a summary map in `tir::Program`.

## 9. Migration plan

1. **std** (done): every `#![safe]` module is checked; `Vec.extend` is
   `#[may_alias(s)]`. Next: `num`, `os`, `rt`, `test`, `prof` when they
   become safe; the unsafe core (`mem`, `gc`, `obj`, `bigint`, `gil`,
   `map`'s probes) is not checked but is what the summaries are inferred
   from.
2. **Examples** (done): every module of the example interpreters is
   `#![lifetimes]`, front ends and vm modules alike (77 modules; the
   tutorial's steps excepted). The 3 audit sites and the 4 new findings
   are resolved (section 7): three code fixes, four inference
   refinements, no `unsafe { }`.
3. **Default**: the examples opt in cleanly and the whole-program cost
   is fixed (section 8); making the check the default for every module
   (keeping `unsafe { }` and an opt-out) is question 5.

**What stays raw (`unsafe` or unchecked) and why**: the allocator, the
collector, inline caches and limb arithmetic (`mem`, `gc`, `obj`,
`bigint`, `gil`): their pointers are computed, not derived from places
the rule can name; vm handlers' operand accesses (`regs[a]`, verified
bytecode) stay unchecked for bounds as today, but are checked for
staleness across `reenter`; register windows and frames stored in the
context (`ctx.stack + base`) are hole 1; interpreter natives that take
`args: *V` into the value stack rely on the stack not being freed during
the call (Mini's `VStack.prev`), which the rule treats as the caller's
promise for every pointer parameter.

## 10. Open questions for the user

1. **Stored pointers** (hole 1): accept the hole, or forbid storing a
   pointer into a *growable* container's interior in memory that
   outlives the function (a second-class rule for `ptr_at`, `items`,
   `get_ptr`, `entry`, `SourceMap.file`, `Diagnostics.at` results), with
   arena allocations and owner pointers still storable? It needs a way
   to tell growable from stable memory: inferred (some function other
   than the owner's `free` restructures it) or declared on the field.
2. **Aliasing of pointer parameters** (hole 2): add an exclusivity
   check at call sites (two arguments pointing to the same container,
   one restructured), at the cost of rejecting `f(&v, &v)`-style calls
   that are fine today?
3. **Arena pointers stored in nodes** and `Arena.free`: region types
   (option D) for arenas only, or keep "free the arena last" a
   convention checked by ASan?
4. **A moving collector** (planned in language.md section 5): then no
   `let` exempts a slice of an object across a collection. Should the
   rule already be the strict one, to keep that option open?
5. **Default on**: make `#![lifetimes]` the default for every module
   (after the examples opt in), or keep it opt-in like `#![safe]`?
6. **Logical staleness**: should `clear`/`truncate`/`pop` also end
   pointers into the dropped elements (memory-safe today, but always a
   bug)? It is one more seed in the summaries.
