# Design note: lifetimes and pointer stability at compile time

Status: **on by default** (`src/lifetimes.rs`): every module is checked,
std's included, unless it says `#![lifetimes(off)]` (section 5.5).
Checking everything costs under 10% of the front end (section 8).
Written 2026-11-04.

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
are *inferred* over the whole call graph. Nothing is declared except
`#[may_alias(s)]` on `Vec.extend`, the one std function that handles
self-aliasing on purpose, and `#[shrinks(v)]` on std's eight functions
that remove elements. The check rejects every bug of the corpus below,
accepts the nine `#![safe]` std modules (`text`, `diag`, `map`, `dict`,
`lex`, `parse`, `hash`, `io`, `bc`) unchanged, found three latent dangling
pointers in the examples and tests (section 7, all fixed), and
leaves the release IR of every example byte-identical. It does not
follow a pointer once it has been stored in memory and loaded again
somewhere else, but it forbids storing a pointer into memory that moves
(a `Vec`'s buffer, a map's table) where it outlives the function
(section 5.4, error 5). Section 6 lists the remaining holes.

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
minimized test that the check rejects. "Compiler" bugs (missing
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

Two more found by the check itself (section 7): `minijs`'s
`Gen.top` stored `&fs` (a local) in `g.fs` and returned without
restoring it (latent: nothing read `g.fs` afterwards; fixed), and a
false start of this note's own accepted test (`tests/run/lt_accepted.lm`
used a slice after a loop of pushes).

## 3. Options considered

Judged on the corpus, on the annotation burden (counted on `std.text`,
`std.map`, `std.diag` and the Lox front end), on interpreter cores and
on run-time cost:

- **A. Rust-style borrow checking** (lifetime parameters, exclusive
  `&mut`): catches the corpus, but needs ≈20 types with lifetime
  parameters, ≈33 explicit signatures and ≈8 redesigns in those files
  alone, rejects the clox structure every front end uses (`p.fs = &fs`
  with parent links, `p.pratt.expr(p)`), cannot express register
  windows or frames, and is a new dimension of the type system
  (language.md section 11). Rejected.
- **B. Second-class references** (Hylo-style: passed down or returned
  as a projection, never stored): about 60 field positions change type
  (50 in the Lox front end), and catches 4 of 8 bugs alone. Rejected as
  the base; its one cheap rule, applied to growable memory only, is
  error 5.
- **C. Pointer stability with inferred summaries**: built (section 5).
  Catches all 8 with no annotations on the std modules counted and no
  signature changes.
- **D. Regions** (`*'a Node`, `Expr<'a>`): covers arena pointers stored
  in nodes, at the cost of a region parameter on every AST and IR type,
  i.e. option A for the front ends. Rejected; that case is hole 1, and
  ASan builds catch it at run time.
- **E. Generational references** (Vale): a word per pointer and a
  compare per dereference. Rejected by the zero-cost requirement.

## 4. Decision

C, with no lifetime syntax, plus B's rule for growable memory (error 5),
exclusivity checked only where a callee needs it (error 2), and
logical staleness through `#[shrinks]`.

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
  scope, and the collector does not move objects;
- in vm code, a call that may re-enter (`TVm::Synced` with a `reload`)
  restructures what the state variables assigned by `reload` pointed
  into: a pointer computed from `regs` before the call is stale after it;
- freeing a block that a parameter's (or a global's) place held before
  the function gave the place a new one (`HashMap.grow`: `let old =
  m.slots; m.slots = mem.alloc(..); ..; mem.release(old)`) restructures
  the block the caller knew, for the summary, without touching pointers
  into the new one (so `HashMap.insert` moves its table:
  `tests/fail/lt_hashmap_insert.lm`);
- a call of a `#[shrinks(v)]` function (`Vec.pop`, `truncate`, `clear`;
  `HashMap.remove`, `clear`; `OrderedMap.remove_at`, `clear`;
  `Ephemerons.remove`), or of one whose summary has it through its
  callees (`StrBuf.clear`, `OrderedMap.remove`, `pop_last`, `Code.clear`),
  *removes elements*: every pointer into the buffers of the container
  `v` points to ends, as for a restructure, though the memory stays.
  Conservatively all of them, not only those into the
  removed range.

**Growable memory.** A restructure *moves* a block if it reallocates it
(`realloc`, so `mem.grow`) or frees a place's old block for a new one
(above), or if one function frees the block of a struct field and gives
that field fresh memory (`OrderedMap.resize` frees the old tables, then
installs new ones). A struct field whose block some function in the
program may move holds *growable* memory: `Vec.ptr` (also inside a
`StrBuf`), `HashMap.slots`, `OrderedMap.idx` and `ents`, an
interpreter's own `realloc`'d buffers (regex's backtrack stack, Mini's
result buffer). Blocks that are only ever freed (an arena's chunks, a
list node, a GC object, Mini's coroutine stacks, which `grow` copies and
retires instead of freeing) are not growable. Fields are told apart by
their struct type: a path's last dereference is typed from its root
(`p.*.lexer.*.toks.ptr.*` hangs from `Vec<Tok>.ptr`), and which fields
grow is known when every summary is, so error 5 is judged at the end.

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
| pairs | (i, B): it uses a pointer into block B of argument j's memory (j's pointee, or a buffer it owns: at most two dereferences down) after restructuring through argument i | the old `push_str`: (b, `s.*`); `fn f(a, b) { let s = a.items(); b.push(1); s[0] }`: (b, `a.*.ptr.*`) |
| keeps | parameter j's pointee may be stored in parameter i's memory, or elsewhere | `Vec.push`: `x` into `v`; `SourceMap.add`: `name`, `text` into `sm` |
| grows, frees | which restructures move a block (section 5.2), which free one | `Vec.reserve` grows `v.*.ptr.*`; `Vec.free` frees it |
| shrinks | parameter paths whose elements it may remove | `Vec.clear`: `v.*`'s buffers; `StrBuf.clear`: `b.*.bytes`'s |

`Arena.alloc_bytes` shows why inference beats declarations here: it
reallocates its chunk *list* (a `Vec`) but never the chunks, so pointers
from `new(&a, ..)` survive allocation and die only at `a.free()`,
with nothing written down.

`#[may_alias(p)]` on a function says that argument `p` may point into
memory the function restructures through another argument: its
`pairs` with `p` are not checked or propagated. Only `Vec.extend` has it
(its cold path re-derives `s` inside the moved buffer).

`#[shrinks(v)]` on a function says that it may remove elements of the
container `v` points to (section 5.2): the summary's `shrinks` gets
`v.*`'s buffers. It is the one seed the inference cannot find (removing
elements is a store of a smaller length), so std declares it on its
eight shrinking functions; everything that calls them inherits it.

### 5.4 The errors

In a checked function, outside `unsafe { }`:

1. **Use after restructure**: reading a stale local (also through `&x`
   or a method call), or one pointing into a container whose elements a
   call may have removed (`#[shrinks]`: "may point to removed
   elements").
2. **Arguments aliasing what the callee moves** (exclusivity): a call
   whose summary has pair (i, B), when what it restructures through
   argument i is, in the caller's terms, block B of argument j: argument
   j points into it (`b.push_str(b.as_str())` with a `push_str` that
   grows first), or holds it (`f(&v, &v)` where `f` takes `a.items()`,
   grows `b`, then reads the slice; `f(&c, &c.v)` where `f` grows `c.v`
   through `c`), unless `#[may_alias(j)]`. Only what the callee uses
   after the restructure counts: `f(&v, &v)` is fine when `f` takes its
   pointers again (`tests/run/lt_stored.lm`), so the check costs no
   `&mut`-style exclusivity on calls that do not need it.
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
5. **Pointer into growable memory kept**: a pointer into the block of a
   growable field (section 5.2) stored in memory that outlives the
   function, directly or through a callee's `keeps` (`seen.push(v.ptr_at(0))`
   with `seen` a parameter's `Vec`), or captured by an escaping closure.
   **Memory that outlives the function** is what a parameter points to
   (and everything reached from it), a global, a GC object, memory of
   unknown origin the function does not return, and an escaping
   closure's environment; *not* the function's own locals and the
   buffers they own, where the check follows the pointer (a local
   struct holding `b.as_str()` is stale when `b` grows), and not fresh
   memory the function returns: then the result points into the block
   for the caller (`Lexer.new(&b)`'s result is stale when `b` grows:
   `tests/fail/lt_fresh_kept_interior.lm`). A place restored before the
   function returns is not kept, as for error 4; `main` gets no
   exemption (the block moves while the program runs). What may still
   be stored: arena allocations (`a.str(..)`, `new(&a, ..)`), malloc'd
   blocks, freed-only blocks, GC objects, statics, an owning value moved
   whole (`n.all = v`). By the types, a pointer to `T` cannot point
   into a block of `U` that cannot hold a `T` inline, and a byte buffer
   (`Vec<u8>`) holds no typed values: so `Ok(mi)` of a result whose
   error message points into a `StrBuf` does not point into it
   (`tests/fail/lt_store_interior*.lm`, `tests/run/lt_stored.lm`).

### 5.5 Where it applies

Every module, std's included, unless it opts out with
**`#![lifetimes(off)]`**. Every function with a body is checked, each
instance of a generic function (so a generic std function no build
instantiates is not), vm handlers and generated code included.
Summaries are computed for everything checked code calls, in any
module (an opted-out one included: `free` and `realloc` are reached
through std's unsafe core).

The escapes, finest first: `unsafe { }` silences the errors inside it
(a site safe by a protocol the check cannot see; the reason goes in a
comment); `#[may_alias(p)]` and `#[shrinks(v)]` correct a function's
summary for all its callers; `#![lifetimes(off)]` drops a whole module's
errors (its summaries are still inferred and used). A `#![safe]` module
cannot opt out (an error: it promises the checks), and nothing in std,
the examples, the tutorial or the benchmarks does; the ASan tests of
stale pointers (`tests/asan/*_stale.lm`) do, since the stale use is
what they test. The opt-out is an argument of the old opt-in attribute
rather than a new name: the attribute names the check, `off` is how
`#[inline(never)]` already reads, and a bare `#![lifetimes]`, the
opt-in that 77 example modules carried, stays accepted as the default
it now is (no warning: the examples build with `-W error`, and code
written for the opt-in should not break). The examples dropped it.

`LOIMI_LIFETIMES=off` skips the pass for a build;
`LOIMI_LIFETIMES=audit` checks every module, opted-out ones included,
and prints each site (`lifetime-site file:line:col message (function)`)
without failing: what an opt-out hides, and every site of a tree at
once rather than the first error.

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
error: this argument may alias argument 1: `append_first` may move or free memory through argument 1 while it still uses a pointer it took through this one
error: `s` may point to removed elements: the call of `Vec<u64>.clear` may have removed elements of the container it points into
error: stores a pointer into the block of `std.mem.Vec<u8>.ptr` (which `Vec<u8>.reserve` may move) in memory that outlives the function
error: passes a pointer into the block of `std.mem.Vec<u64>.ptr` (which `Vec<u64>.grow_one` may move) to `Vec<*u64>.push`, which may keep it beyond the call
```

<!-- snippet: fail "stores a pointer into the block of `std.mem.Vec<u8>.ptr`"
import std.text;
struct Lexer { text: str, pos: u64 }
{snippet}
fn main() {}
-->
```loimi
fn Lexer.init(l: *Lexer, src: *text.StrBuf) {
    l.text = src.as_str();
}
```

## 6. What it does not check (the holes)

1. **Pointers stored in memory.** A pointer loaded from a field is
   tracked as "the memory that field points to", not as what it was
   derived from. Into growable memory, the store itself is now an error
   where it outlives the function (error 5: `ctx.cur =
   ctx.frames.ptr_at(i)` with `ctx` a parameter). What remains: such a
   pointer kept in the function's own memory (a local `Vec<*T>` of
   element pointers) is not followed once stored there; pointers into
   blocks that are only freed (arena nodes used after `a.free()`
   elsewhere; a freed list node); a store through `unsafe
   { }` (the 7 protocol sites of section 7); a GC object's growable
   field reached as "some GC object" (its type is not known, so it is
   not judged); a growable block of bytes holding typed values through
   casts (taken to hold bytes); a pointer into a *local*'s growable
   buffer kept in fresh memory the function returns (the local is gone,
   so nothing follows its buffer).
2. **Parameters are assumed not to alias each other's buffers** inside
   a function body (`fn f(a: *Vec<u8>, b: *Vec<u8>)` taking `a.items()`
   and pushing to `b`); the caller is checked instead (error 2): `f(&v, &v)` and `f(&c, &c.v)` are rejected when `f` uses what it
   took through one after moving memory through the other. Not checked:
   blocks more than two dereferences below a parameter (reached through
   pointers stored in memory, hole 1), and two different GC objects or
   fresh blocks, which share a root and are taken not to alias.
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
   `from_vm`, which `call_value` passes false; section 7). The values
   of a pattern's parts (`Ok(x)`, a tuple's fields) are the whole
   value's paths, less those their type cannot point into.
   A closure's captured owning value (a `Vec`, a `StrBuf`) is the
   enclosing variable itself and starts with no paths, like a `let`
   owner, so growing it inside the closure does not make it stale
   (Mini's debugger: `tests/run/lt_inference.lm`); what a captured
   slice points into is not related to the captured owner (hole 3).
5. **The collector does not move objects**, so a slice of an object
   held by a `let` is accepted across a collection.
6. **Logical staleness**: a pointer kept
   across `v.clear()`, `truncate`, `pop` or a map's `remove` is an error
   (`#[shrinks]`), though the memory stays allocated. Not covered: a
   container shrunk by assigning its length (`v.len = n` in user code),
   std's GC dictionaries (`Dict`, `KeyDict`: their parameters are GC
   references, accessed by entry index and checked by `version`), and
   a slice of a rooted list's old array after guest code replaced it
   (CPython reads the current list; Python's fix does too).
7. **Trusted code**: `unsafe { }` and modules that opt out
   (`#![lifetimes(off)]`) are not reported (their summaries are still
   inferred and used); nor are generic functions no build instantiates.

## 7. Evidence

**Corpus**: `tests/fail/lt_*.lm` (12 programs over the 8 shapes, plus
`lt_may_alias_param.lm`), each rejected with the message above;
`tests/run/lt_accepted.lm` runs the fixed version of each (re-derive
after growth, `let`-held GC object, arena nodes freed last, moving a
`Vec`, a local's address stored and restored, self-append through
`Vec.extend`). Phase 2 added 12 more: `lt_alias_args`, `lt_alias_field`
(exclusivity); `lt_store_interior`, `_kept`, `_global` and
`lt_fresh_kept_interior` (stored pointers); `lt_clear_slice`,
`lt_pop_ptr`, `lt_strbuf_clear`, `lt_map_remove_ptr` (removed
elements); `lt_hashmap_insert` (a place's old block); `lt_shrinks_param`
(the attribute); `tests/run/lt_stored.lm` runs what they accept.

**std**: the nine `#![safe]` modules are checked in every build that
uses them (all example interpreters and tests): no error, no change but
the one attribute (phase 2: `#[shrinks]` on eight functions).

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

**Opting in** (phase 1): all 77 modules of Lox, Mini (with its
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

**Phase 2 rules** (errors 2 and 5, `#[shrinks]`), audited with the same
builds (every example under `--cfg debug`, `--cfg gil`, `--jit-stencils`,
`--shared`, the fuzzers, regex's stats driver, the tutorial; every
module, std's included):

| rule | sites, first version | refinements | sites left | resolution |
|---|---:|---|---:|---|
| exclusivity (error 2) | 103 | `pairs` name the block of j that is used, not just j (a pointer *value* stored, as `Vec.push(v, x)` does with `x`, is not a use of what it points to) | 0 | none needed |
| stored pointers (error 5) | 39 | growable means *moved*, not freed (Mini's coroutine stacks are copied and retired, a GIL's thread record is freed: 30 sites); types filter what a pattern's part and a stored value may point into (Wasm's `Ok(mi)` beside an error message in a `StrBuf`: 2) | 7 | `unsafe { }` with the reason, on the same line (below) |
| removed elements (`#[shrinks]`) | 0 | | 0 | |

The 7 sites of error 5, each a pointer into growable memory kept on
purpose:

| site | why it is safe |
|---|---|
| `examples/mini/embed.lm` `mini_debug_eval` (2): `*out = d.out.as_str()` | the C API lends the text until the next `mini_debug_*` call (documented above the API) |
| `examples/mini/threads.lm` `save`: `t.rbuf = c.rbuf` | the result buffer is handed to the saved thread; `load` gives the context the next thread's before anything grows it |
| `examples/regex/vm.lm` `grow`: `r.bt_end = r.bt_base + ncap` | the end of the block just moved, set by the function that moves it |
| `examples/wasm/vm.lm` `Machine.switch_to` (3): `m.funcs = i.funcs.ptr`, `globals`, `sigs` | an instance's vectors are complete (`instantiate`) before any switch to it, and the machine takes them again at every switch |

**Default on**: every module checked, std's unsafe core
(`mem`, `gc`, `obj`, `bigint`, `gil`, `num`, `os`, `rt`, `test`,
`prof`, `debug`) included, across every example in every configuration
above plus `--sanitize=address` builds (std has `#[cfg(sanitize_address)]`
code) and the tutorial's nine steps, the fuzzers of `tests/fuzz`, the
four benchmarks written in Loimi (`bench/`), the C-library tests
(`tests/embed`), the bigint calculator, the 209 programs of
`tests/run`, the 348 of `tests/fail` (each must still fail with its own
error), `loimi test` of `tests/test` and the examples' unit tests, and
every doc snippet. New sites, all outside the examples (whose modules
already opted in):

| site | verdict | resolution |
|---|---|---|
| `tests/run/std_obj.lm` `weak`: `w.heap.add_scanner(&keep as *u8, scan)` with `keep` a local of `weak` (error 4: the heap keeps the address) | real dangling pointer, latent: the scanner outlived `weak`, and `keep` was freed in it, but nothing collected after `weak` returned (it ran last) | fixed: `keep` is `main`'s, registered there and passed to `weak`; `tests/fail/lt_scanner_local.lm` |
| `tests/asan/*_stale.lm` (8: a pointer into a grown `Vec`, a grown map, a freed arena; a slice or pointer across `clear`, `pop`, `truncate`, `StrBuf.clear`, `OrderedMap.clear`) | the stale use is the point: ASan must report it at run time | `#![lifetimes(off)]` |

std itself, the tutorial (so its text needs no change), the benchmarks,
the fuzzers and every doc snippet: nothing; no inference change and no
`unsafe { }` was needed. std's unsafe core passes because the pass
already derived every summary from it, and its pointer arithmetic is
not something the rule follows (computed addresses are fresh or keep
their paths, section 5.1). `LOIMI_LIFETIMES=audit` over all of the
above reports only the 8 opted-out ASan sites. Tests of the default:
`tests/fail/lt_default.lm` (an imported module without attributes),
`lt_off_safe.lm` and `lt_off_arg.lm` (the attribute's misuses),
`tests/run/lt_off.lm` (an opted-out module that keeps a pointer across
a `push` after a `reserve`).

**IR**: release `--emit-llvm` of Lox, Mini, Mini's embedding, MiniJS,
Prolog, Python, regex (library and fuzzer), Scheme, Stack (and fuzzer),
Wasm and the tutorial, base compiler against the new one with the same
std (the attribute line blanked for the base): byte-identical. After
phase 1 (the attribute takes the place of a blank line, so no line
moves), against the tree before it: byte-identical for every example
(also Mini `--cfg debug`, `--cfg gil` and Python `--cfg debug`) except
the three intended code fixes: MiniJS (`Gen.top`), Mini and its
embedding (`coro.call`, the CALL handler's yield branch) and the Stack
fuzzer (the order of frees). Phase 2 (std's `#[shrinks]` lines take
the place of blank lines, the 7 `unsafe { }` are on their lines),
master's compiler and tree against the new ones: byte-identical for
every example and configuration above, the edited modules included.
Default on (the examples' `#![lifetimes]` lines blanked), master's
compiler and tree against the new ones, std at paths of the same
length: byte-identical for all 55 builds (every example in every
configuration above and with `--sanitize=address`, the tutorial's
steps, the benchmarks, the fuzzers, `tests/embed`, the bigint
calculator, the ASan tests that build without an opt-out).

## 8. Cost

- **Run time: none.** The pass only rejects programs; the IR is
  unchanged (section 7).
- **Compile time** (front end and IR, `--emit-llvm`, best of 3, against
  `LOIMI_LIFETIMES=off`). Checking *everything* (the audit, which is also
  what every build costs now that the check is on by default) cost, in
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

  **Phase 2** (errors 2 and 5, `#[shrinks]`): the pass's own time
  (`LOIMI_LIFETIMES_STATS=1`), before and after, and its share of the
  front end without it:

  | example | pass before | after | share of front end |
  |---|---:|---:|---:|
  | Lox | 15.6 ms | 17.6 ms | 6% |
  | Mini (embedding) | 26.2 (24.2) | 27.4 (28.2) | 5% (5%) |
  | MiniJS | 34.4 | 45.5 | 7% |
  | Prolog | 16.0 | 18.7 | 7% |
  | Python | 50.0 | 57.0 | 9% |
  | regex | 3.9 | 4.4 | 7% |
  | Scheme | 23.1 | 34.3 | 9% |
  | Stack | 8.9 | 10.3 | 7% |
  | Wasm | 52.3 | 54.0 | 1% |
  | tutorial step 9 | 3.3 | 3.6 | 2% |

  The first version of the exclusivity `pairs` named every block of
  argument j used after the restructure: a recursive tree walker
  (MiniJS's `Gen.stmt`) listed 1357, and MiniJS's pass took 489 ms.
  Naming the block rather than the pointer, and only the parameter's
  pointee and the buffers it owns (two dereferences, section 6 item 2),
  brought it to 7.

  **Default on**: the pass's time before (master: the
  examples' modules and std's `#![safe]` ones checked) and after (every
  module), and its share of the front end without it (`LOIMI_LIFETIMES=off`,
  `--emit-llvm`, best of 3; the machine was busier than for the tables
  above, so the absolute times are higher). The examples change little,
  since their modules were checked already and std's unsafe core was
  summarized anyway; programs that were not checked at all pay the
  full pass now:

  | build | front end without the pass | pass before | after | share |
  |---|---:|---:|---:|---:|
  | Lox | 367 ms | 21.3 ms | 22.6 ms | 6% |
  | Mini (embedding) | 610 (627) | 29.8 (30.4) | 30.7 (30.9) | 5% (5%) |
  | MiniJS | 747 | 51.9 | 52.4 | 7% |
  | Prolog | 326 | 20.6 | 25.6 | 8% |
  | Python | 706 | 66.0 | 67.9 | 10% |
  | regex | 84 | 5.5 | 5.8 | 7% |
  | Scheme | 458 | 35.9 | 34.9 | 8% |
  | Stack | 164 | 11.0 | 12.4 | 8% |
  | Wasm | 5729 | 55.4 | 56.5 | 1% |
  | tutorial step 9 (not checked before) | 205 | 4.2 | 14.2 | 7% |
  | `tests/run/std_obj.lm` (std's object model, GC) | 82 | 0.7 | 5.1 | 6% |
  | `tests/bigint/calc.lm` (`std.bigint`) | 93 | 1.1 | 6.0 | 6% |
  | `tests/fuzz/fuzzcombo.lm` (Mini's vm, fuzzed) | 440 | 1.1 | 13.7 | 3% |
  | `bench/obj/ic.lm` (inline caches) | 75 | 0.7 | 4.4 | 6% |
- **Implementation**: `src/lifetimes.rs` (~2450 lines, no dependency
  on codegen), `#![lifetimes]`, `#[may_alias]` and `#[shrinks]` in the
  parser and checker, a summary map in `tir::Program`.

## 9. History, and what stays raw

Built in three steps: phase 1 (errors 1, 3 and 4, opt-in per module;
all 77 example modules opted in, after the fixes of section 7), phase 2
(errors 2 and 5, `#[shrinks]`), then the default (every module checked,
`#![lifetimes(off)]` to opt out; the examples' `#![lifetimes]` lines
went).

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
