# Design note: pointer stability (lifetimes at compile time)

How the compiler rejects a pointer or slice used after its container may
have moved or freed the memory, and why it works this way. For
contributors to the compiler.

**Status: on by default.** Every module is checked, std's included,
unless it says `#![lifetimes(off)]` (section 5.5). The pass is
`src/lifetimes.rs`. It has no run-time cost and takes under 10% of the
front end's time (section 8).

## Summary

The common memory bug in Loimi programs is a pointer or slice into a
container, used after the container grew, was freed, or was collected.
The check catches these without a borrow checker and without
annotations:

- A pointer is described by *where it points*, as a path such as
  `v.ptr.*` ("into `v`'s buffer").
- A call that may move or free memory makes every pointer into that
  memory stale. Using a stale pointer is an error.
- What each function may move or free, and where its result points, is
  inferred over the whole program.

Only two attributes exist, both on std functions:

- `#[may_alias(s)]` on `Vec.extend`, the one function that handles
  self-aliasing on purpose.
- `#[shrinks(v)]` on std's eight functions that remove elements.

The check rejects every bug in the corpus (section 2) and leaves the
release IR of every example byte-identical. It found three latent
dangling pointers in the examples and tests (section 7). It does not
follow a pointer once it is stored in memory and loaded again, but it
forbids storing a pointer into memory that moves where it outlives the
function (section 5.4, error 5). Section 6 lists what it misses.

## 1. The problem

[`safety.md`](safety.md) makes `*T` in a `#![safe]` module "a reference
to one live value", but leaves lifetimes out. In particular:

- a `*T` or slice from a container dangles once the container grows,
  shrinks or is freed;
- `&local` may outlive its function;
- a copied `Vec` shares its buffer with the original;
- a slice of a GC object dangles once the object is collected.

ASan builds catch most of these, but only on paths a test runs, and not
in the GC heap.

Loimi has no borrow checker on purpose. Interpreters are built on
pointers a borrow checker can't express: register windows into a value
stack, frames, inline caches, AST nodes in an arena pointing at each
other, structs holding pointers to the heap or a parent scope. A
solution has to accept that code (or confine it to `unsafe`) and cost
nothing at run time.

## 2. The bug corpus

Real bugs from the repository's history, each with a minimized test in
`tests/fail/` that the check rejects. Bugs in the compiler or collector
(missing GC roots in generated code) are out of scope, as is stale
decoded state.

| # | Bug | Fixed by | Shape | Test |
|---|---|---|---|---|
| 1 | `b.push_str(b.as_str())` read freed memory: `reserve` moved the buffer under `s` | std.text (`Vec.extend` re-derives) | an argument points into what the callee grows | `lt_push_own_text`, `lt_grow_while_borrowed` |
| 2 | Python `list.index`/`count`/`<` scanned `l.arr.items` while `__eq__` grew the list and the old array was collected | examples/python (re-read each step) | slice of an unrooted GC object across a collecting call | `lt_gc_slice_across_collect` |
| 3 | `OrderedMap.get_ptr`/`entry` pointer used after an insert that resizes | documented hole in std.map | pointer into a container across a resize | `lt_map_ptr_insert` |
| 4 | a node used after `Arena.free`; a slot pointer used after `HashMap.free` | std.mem, std.map (the containers' own state) | pointer across a free | `lt_arena_free`, `lt_hashmap_free` |
| 5 | a copied `Vec`: two owners, one grows and frees the other's buffer | [`safety.md`](safety.md) section 2 | copy aliasing | `lt_vec_copy`, `lt_vec_copy_field` |
| 6 | `&local` returned or stored; `rt.panic` kept a pointer to a message in a dead frame | copy the message | local escape | `lt_local_return`, `lt_local_store`, `lt_local_kept` |
| 7 | Mini's coroutine stacks move when they grow: handler and native pointers into the old block | examples/mini (indices, `reload`) | pointer from vm state across a `reenter` call | `lt_vm_reenter_stale` |
| 8 | objref's remaining hole: a read-only slice of an object held across a call that may collect | [`safety.md`](safety.md) section 8.1 | same as 2 | `lt_gc_slice_across_collect` |

## 3. Options considered

Judged on the corpus, on how many annotations `std.text`, `std.map`,
`std.diag` and the Lox front end would need, on interpreter cores, and
on run-time cost.

| Option | Verdict |
|---|---|
| **A. Rust-style borrow checking** (lifetime parameters, exclusive `&mut`) | Rejected. Catches the corpus, but those files alone would need about 20 types with lifetime parameters, 33 explicit signatures and 8 redesigns. Rejects the parser structure every front end uses (`p.fs = &fs` with parent links). Can't express register windows or frames. |
| **B. Second-class references** (Hylo-style: passed down or returned, never stored) | Rejected as the base: about 60 field types change, and it catches only 4 of 8 bugs. Its one cheap rule, applied to growable memory only, is error 5. |
| **C. Pointer stability with inferred summaries** | Built. Catches all 8 with no annotations in the files counted and no signature changes. |
| **D. Regions** (`*'a Node`, `Expr<'a>`) | Rejected: a region parameter on every AST and IR type, which is option A for the front ends. Arena pointers stored in nodes stay a hole (section 6, item 1); ASan catches them at run time. |
| **E. Generational references** (Vale) | Rejected: a word per pointer and a compare per dereference. |

## 4. Decision

Option C with no lifetime syntax, plus:

- B's rule, for growable memory only (error 5);
- exclusivity, checked only where a callee needs it (error 2);
- "removed elements" staleness through `#[shrinks]`.

## 5. The rules

### 5.1 Where pointers point

Each value that holds pointers (a `*T`, a slice, a struct holding them,
a `Gc`) is described by a set of **paths**: a root followed by field
steps `.f` and dereferences `*`.

Roots:

- a local or parameter of the function;
- a global;
- a GC object held by a `let` local or a parameter;
- "some other GC object";
- **fresh**: memory of unknown origin, such as `malloc`'s result or an
  extern's pointer that came from no pointer argument. Fresh memory is
  never moved or freed as far as the rule knows. A store into it is
  judged by whether the function returns it (section 5.4).

How expressions map to paths:

- `&x` is the path `x` (the local's own storage). `&p.f` is
  `(paths of p).f`.
- `p.f` read as a pointer is `(paths of p).f.*`. `o.items` is `o.items`
  in the object's block. A slice of a local array is the array.
- A struct or slice read whole adds `.*` through "some field": `v.*`
  is the buffers a `Vec` value owns.
- Pointer arithmetic, casts and sub-slices keep the paths. Integers and
  constants point nowhere.
- A call's result points where the callee's summary says (section
  5.3), with the callee's parameters replaced by what the arguments
  point into.
- A callee without a body (an extern), or a function value: the result
  points into what its pointer arguments point into, **where the types
  allow it**. A `*R` or `[]R` result counts an argument `*T` only if an
  `R` can be stored inline in a `T`. Byte pointers, generic and
  non-pointer arguments always count. Example: a parse hook given
  `*Parser` returns a `*Node` and a `str`, neither of which a `Parser`
  holds inline, so the result does not point into the parser
  (`tests/run/lt_fn_value.lm`, `tests/fail/lt_fn_value_kept.lm`).

Paths are cut at 8 steps, so long pointer chains (`f = f.parent` in a
loop) are approximated (section 6, item 4).

### 5.2 Restructuring

A **restructure** of path R means the block at R may be moved or freed.
It invalidates a path P when R is a prefix of P and the rest of P stays
in that block (no further dereference). Freeing `v.ptr.*` invalidates
`v.items()` and `&v.items()[i].f`, but not a `str` stored *in* the
buffer (`v.ptr.*.name.*` is another block).

What restructures:

- `free(p)` and `realloc(p, n)`: what `p` points into.
- A call: what its summary says, through its arguments.
- A call that may collect (`uses gc` or `reenter`, also through a
  function value's type): every GC object not held by a `let` local or
  parameter. The compiler roots those while they are in scope, and the
  collector does not move objects.
- In vm code, a call that may re-enter (`TVm::Synced` with a `reload`):
  what the state variables that `reload` assigns pointed into. A pointer
  computed from `regs` before the call is stale after it.
- Freeing the block that a parameter's (or global's) place held before
  the function gave the place a new one. `HashMap.grow` does
  `let old = m.slots; m.slots = mem.alloc(..); ..; mem.release(old)`.
  For the summary this restructures the block the caller knew, without
  touching pointers into the new one, so `HashMap.insert` moves its
  table (`tests/fail/lt_hashmap_insert.lm`).

**Removed elements.** A call of a `#[shrinks(v)]` function ends every
pointer into the buffers of the container `v` points to, as a
restructure does, though the memory stays allocated. All such pointers
end, not only those into the removed range. The functions:

- declared: `Vec.pop`, `truncate`, `clear`; `HashMap.remove`, `clear`;
  `OrderedMap.remove_at`, `clear`; `Ephemerons.remove`;
- inherited through callees, for example: `StrBuf.clear`,
  `OrderedMap.remove`, `pop_last`, `Code.clear`.

**Growable memory.** A restructure *moves* a block when:

- it reallocates it (`realloc`, so `mem.grow`);
- it frees a place's old block for a new one (above);
- one function frees the block of a struct field and gives that field
  fresh memory (`OrderedMap.resize`).

A struct field whose block some function in the program may move holds
*growable* memory. Examples: `Vec.ptr` (also inside a `StrBuf`),
`HashMap.slots`, `OrderedMap.idx` and `ents`, and an interpreter's own
`realloc`'d buffers. Blocks that are only freed, never moved, are not
growable: an arena's chunks, a list node, a GC object, Mini's coroutine
stacks (which `grow` copies and retires instead of freeing).

Fields are told apart by struct type: a path's last dereference is
typed from its root (`p.*.lexer.*.toks.ptr.*` belongs to
`Vec<Tok>.ptr`). Which fields grow is only known once every summary is,
so error 5 is judged at the end.

**Staleness.** A local whose paths are invalidated is **stale** until
it is assigned again. This is flow-sensitive: joined at branches and
iterated to a fixpoint in loops.

### 5.3 Summaries

Every function reachable from a checked one gets a summary, inferred as
a least fixpoint over the call graph (a worklist, callees first).

| Summary | Meaning | Example |
|---|---|---|
| restructs | parameter (or global) paths it may move or free | `Vec.push`: `v.*.ptr.*`; `Arena.alloc_bytes`: only `a.*.chunks.ptr.*` |
| ret | parameter paths its result may point into | `Vec.items`: `v.*.ptr.*`; `SourceMap.file`: `sm.*.files.ptr.*` |
| pairs | (i, B): after restructuring through argument i, it uses a pointer into block B of argument j's memory (j's pointee, or a buffer it owns: at most two dereferences down) | `fn f(a, b) { let s = a.items(); b.push(1); s[0] }`: (b, `a.*.ptr.*`) |
| keeps | parameter j's pointee may be stored in parameter i's memory, or elsewhere | `Vec.push`: `x` into `v`; `SourceMap.add`: `name`, `text` into `sm` |
| grows, frees | which restructures move a block, which free one | `Vec.reserve` grows `v.*.ptr.*`; `Vec.free` frees it |
| shrinks | parameter paths whose elements it may remove | `Vec.clear`: `v.*`'s buffers |

`Arena.alloc_bytes` shows why inference beats declarations. It
reallocates its list of chunks (a `Vec`) but never the chunks
themselves. So pointers from `new(&a, ..)` survive allocation and die
only at `a.free()`, with nothing written down.

Two attributes correct a summary:

- `#[may_alias(p)]`: argument `p` may point into memory the function
  restructures through another argument. Its `pairs` with `p` are not
  checked or propagated. Only `Vec.extend` has it: its slow path
  re-derives `s` inside the moved buffer.
- `#[shrinks(v)]`: the function may remove elements of the container
  `v` points to. Inference can't find this, because removing elements is
  just a store of a smaller length. Std declares it on its eight
  shrinking functions, and callers inherit it.

### 5.4 The errors

In a checked function, outside `unsafe { }`, these are errors:

**1. Use after restructure.** Reading a stale local (also through `&x`
or a method call), or one that points into a container whose elements a
call may have removed ("may point to removed elements").

**2. An argument aliases what the callee moves** (exclusivity). The
callee's summary has pair (i, B), and what it restructures through
argument i is, in the caller's terms, block B of argument j. Either
argument j points into it (`b.push_str(b.as_str())` with a `push_str`
that grows first), or holds it:

- `f(&v, &v)` where `f` takes `a.items()`, grows `b`, then reads the
  slice;
- `f(&c, &c.v)` where `f` grows `c.v` through `c`.

`#[may_alias(j)]` allows it. Only what the callee uses *after* the
restructure counts: `f(&v, &v)` is fine when `f` takes its pointers
again (`tests/run/lt_stored.lm`). So calls that don't need exclusivity
don't pay for it.

**3. Copy of an owning value.** Copying a struct value that holds
pointers (`var w = v`) makes two owners of the same buffers. A
restructure through either makes the other stale: the local, or the
place it was copied from (`var s = c.stack; s.push(..); c.stack.at(0)`).
Moving is fine (the original is not used again, or is assigned first),
as in `Interner.grow`'s `var old = it.slots; it.slots = ..; old.free()`.

**4. Local escape.** Any of:

- returning the address of a local (its storage, or a slice of a local
  array);
- storing it in memory that outlives the function, unless restored
  before the function returns (`g.fs = &fs; ..; g.fs = parent` is
  fine);
- passing it to a function whose summary keeps it outside the caller's
  own locals;
- capturing it in an escaping closure.

`main`'s locals live as long as the program and may escape.

A store into fresh memory counts as outliving the function, unless the
function returns that memory. Then the summary says the pointer is kept
**in its result**, and the caller judges where the result goes:

- into one of the caller's locals or a field of one, the address stays
  in the frame (`m.t = Table.new(&m.c)`; Lox's
  `m.strings = gc.WeakInterner.new(&m.heap, ..)`);
- returned, it is kept in the caller's result in turn;
- stored anywhere else, or passed on, it escapes.

Tests: `tests/run/lt_fresh_kept.lm`, `tests/fail/lt_fresh_kept.lm`.

**5. Pointer into growable memory kept.** A pointer into the block of a
growable field (section 5.2) is stored in memory that outlives the
function, either directly or through a callee's `keeps`
(`seen.push(v.ptr_at(0))` with `seen` a parameter's `Vec`), or is
captured by an escaping closure.

*Memory that outlives the function* is:

- what a parameter points to, and everything reached from it;
- a global, a GC object, an escaping closure's environment;
- fresh memory the function does not return.

It is *not* the function's own locals and the buffers they own. There
the check follows the pointer: a local struct holding `b.as_str()` is
stale when `b` grows. Nor is it fresh memory the function returns: the
result then points into the block for the caller, so `Lexer.new(&b)`'s
result is stale when `b` grows (`tests/fail/lt_fresh_kept_interior.lm`).
A place restored before return is not kept, as for error 4. `main` gets
no exemption, since the block moves while the program runs.

What may still be stored: arena allocations (`a.str(..)`,
`new(&a, ..)`), malloc'd blocks, freed-only blocks, GC objects, statics,
and an owning value moved whole (`n.all = v`).

Types narrow what a pointer can point into. A pointer to `T` can't point
into a block of `U` that can't hold a `T` inline, and a byte buffer
(`Vec<u8>`) holds no typed values. So `Ok(mi)`, in a result whose error
message points into a `StrBuf`, does not point into the `StrBuf`
(`tests/fail/lt_store_interior*.lm`, `tests/run/lt_stored.lm`).

### 5.5 Where it applies

Every module is checked, std's included, unless it opts out with
**`#![lifetimes(off)]`**. Every function with a body is checked,
including vm handlers and generated code. Generic functions are checked
per instance, so a generic std function that no build instantiates is
not checked. Summaries are computed for everything checked code calls,
in any module, opted-out ones included (`free` and `realloc` are reached
through std's unsafe core).

Ways out, finest first:

| Escape | Effect |
|---|---|
| `unsafe { }` | silences the errors inside it; give the reason in a comment |
| `#[may_alias(p)]`, `#[shrinks(v)]` | correct a function's summary for all callers |
| `#![lifetimes(off)]` | drops a whole module's errors; its summaries are still inferred and used |

A `#![safe]` module can't opt out: that is an error, since `#![safe]`
promises the checks. Nothing in std, the examples, the tutorial or the
benchmarks opts out. Only the ASan tests of stale pointers
(`tests/asan/*_stale.lm`) do, because the stale use is what they test.

The opt-out is an argument (`off`, as in `#[inline(never)]`) rather than
a new attribute name. A bare `#![lifetimes]`, the old opt-in, is still
accepted without a warning and means the default.

Environment variables:

| Variable | Effect |
|---|---|
| `LOIMI_LIFETIMES=off` | skips the pass for a build |
| `LOIMI_LIFETIMES=audit` | checks every module, opted-out ones included, and prints every site (`lifetime-site file:line:col message (function)`) without failing |
| `LOIMI_LIFETIMES_STATS=1` | prints the pass's time and its slowest functions |

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

Error 5, a pointer into a growable buffer stored through a parameter:

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

Other messages, one per kind:

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

## 6. What it does not check

1. **Pointers stored in memory.** A pointer loaded from a field is
   tracked as "the memory that field points to", not as what it was
   derived from. Storing a pointer into growable memory where it
   outlives the function is an error (error 5). Still missed:
   - a pointer kept in the function's own memory (a local `Vec<*T>` of
     element pointers) is not followed once stored;
   - pointers into blocks that are only freed (arena nodes used after
     `a.free()` elsewhere; a freed list node);
   - a store inside `unsafe { }` (the 7 sites in section 7);
   - a GC object's growable field reached as "some GC object" (its type
     is unknown, so it is not judged);
   - a growable byte block holding typed values through casts (taken
     to hold bytes);
   - a pointer into a *local*'s growable buffer kept in fresh memory the
     function returns (the local is gone, so nothing follows its buffer).
2. **Parameters are assumed not to alias each other's buffers** inside a
   function body (`fn f(a: *Vec<u8>, b: *Vec<u8>)` taking `a.items()`
   and pushing to `b`). The caller is checked instead (error 2). Not
   checked: blocks more than two dereferences below a parameter, and two
   different GC objects or fresh blocks, which share a root and are taken
   not to alias.
3. **Closures and function values.** A call through a function value
   restructures nothing (except by its `uses gc` effect). The body of a
   closure passed to a call is not part of that call's summary.
4. **Approximations.**
   - Paths are cut at 8 steps. Two pointers cut at the same depth are
     taken to be in different blocks, and so are a restructure cut at
     the bound and a pointer that ends exactly there
     (`release(&t.inner)` with `t` 8 steps deep frees a block below
     `t`'s: `tests/run/lt_inference.lm`).
   - Summaries drop restructures cut at the bound. Otherwise a recursive
     tree walker lists every combination of child fields 8 steps down
     (MiniJS's `Res.stmt` had 758, 743 of them cut).
   - Assigning a place a new value detaches what pointed into its old
     buffers (`var s = p.items(); p.v = other;` then freeing through a
     copy is missed).
   - Externs other than `free`/`realloc` are assumed not to free.
   - `defer`s are evaluated at the end of their block only.
   - A `?` or `never` call is not an exit for the escape rule.
   - All fresh memory is one root, so a fresh block's stored pointers
     are never taken as overwritten.
   - Summaries are path-insensitive: a store guarded by a flag argument
     counts for every caller.
   - A pattern's parts (`Ok(x)`, a tuple's fields) get the whole value's
     paths, less those their type can't point into.
   - A closure's captured owning value (a `Vec`, a `StrBuf`) starts with
     no paths, like a `let` owner, so growing it inside the closure does
     not make it stale (`tests/run/lt_inference.lm`). What a captured
     slice points into is not related to the captured owner.
5. **The collector does not move objects**, so a slice of an object held
   by a `let` is accepted across a collection.
6. **Removed elements, partly.** A pointer kept across `v.clear()`,
   `truncate`, `pop` or a map's `remove` is an error. Not covered:
   - a container shrunk by assigning its length (`v.len = n` in user
     code);
   - std's GC dictionaries (`Dict`, `KeyDict`), which are accessed by
     entry index and checked by `version`;
   - a slice of a rooted list's old array after guest code replaced it.
7. **Trusted code.** `unsafe { }` and opted-out modules are not
   reported (their summaries are still used), nor are generic functions
   no build instantiates.

## 7. Evidence

**Tests.**

- `tests/fail/lt_*.lm`: each corpus bug and each error kind, rejected
  with its message, plus the attributes' misuses (`lt_shrinks_param`,
  `lt_may_alias_param`, `lt_off_safe`, `lt_off_arg`) and the default
  (`lt_default`: an imported module without attributes).
- `tests/run/lt_accepted.lm`: the fixed version of each corpus bug
  (re-derive after growth, a `let`-held GC object, arena nodes freed
  last, moving a `Vec`, a local's address stored and restored,
  self-append through `Vec.extend`).
- `tests/run/lt_stored.lm`, `lt_inference.lm`, `lt_fresh_kept.lm`,
  `lt_fn_value.lm`: what the rules accept.
- `tests/run/lt_off.lm`: an opted-out module.

**Coverage.** Every module was audited (`LOIMI_LIFETIMES=audit`) across
every example in every test configuration (`--cfg debug`, `--cfg gil`,
`--jit-stencils`, `--shared`, `--sanitize=address`), the tutorial, the
fuzzers, the benchmarks, `tests/embed`, the bigint calculator, all of
`tests/run` and `tests/fail`, the unit tests and every doc snippet. The
only sites left are the 8 opted-out ASan tests. Std passes unchanged
apart from the eight `#[shrinks]` lines. Its unsafe core passes because
its pointer arithmetic gives computed addresses that are fresh or keep
their paths.

**Sites fixed in the code.** Three were latent dangling pointers; the
Mini one was not reachable but lent a pointer it should not have:

| Site | Bug | Fix |
|---|---|---|
| `examples/minijs/compile.lm` `Gen.top` | `g.fs = &fs` not restored before returning (nothing read it afterwards) | restore it, as `Gen.function` does |
| `examples/mini/vm.lm` `sort_less` | `&ab[0]`, a local array, passed as `args` to `call_value`, whose summary keeps `args` in the context. Only stored on a path `call_value` never takes, but it lent a pointer to the context | the CALL instruction sets `ctx.xfer` itself, so `coro.call` no longer stores `args` |
| `examples/stack/fuzz.lm` `run_one` | the machine used after `prog.free()`, while `m.words` and `m.consts` point into the program | free the program after the machine |
| `tests/run/std_obj.lm` `weak` | the heap kept the address of `weak`'s local `keep` as a scanner argument (error 4) | `keep` belongs to `main` (`tests/fail/lt_scanner_local.lm`) |

**False positives removed by refining inference.** The first rule gave
39 sites on the examples, 36 of them false. Error 2's first version gave
103 sites and error 5's gave 39. The refinements that removed them:

- a place assigned a new value detaches its old buffer;
- escapes are judged at function exit and by destination;
- a function value's result points into an argument only where the
  types allow it (section 5.1);
- fresh memory "kept in the result" (section 5.4);
- captured owners start with no paths (section 6, item 4);
- `pairs` name the block of j that is used, not just j: storing a
  pointer *value* (`Vec.push(v, x)`) is not a use of what it points to;
- "growable" means moved, not freed (30 of error 5's sites);
- types filter what a pattern's part may point into.

**The 7 `unsafe { }` sites** (error 5), each a pointer into growable
memory kept on purpose:

| Site | Why it is safe |
|---|---|
| `examples/mini/embed.lm` `mini_debug_eval` (2): `*out = d.out.as_str()` | the C API lends the text until the next `mini_debug_*` call (documented above the API) |
| `examples/mini/threads.lm` `save`: `t.rbuf = c.rbuf` | the result buffer is handed to the saved thread; `load` gives the context the next thread's before anything grows it |
| `examples/regex/vm.lm` `grow`: `r.bt_end = r.bt_base + ncap` | the end of the block just moved, set by the function that moves it |
| `examples/wasm/vm.lm` `Machine.switch_to` (3): `m.funcs = i.funcs.ptr`, `globals`, `sigs` | an instance's vectors are complete before any switch to it, and the machine takes them again at every switch |

The example interpreters' vm handlers needed no `unsafe`: they already
recompute pointers after `reenter` calls.

**IR.** Release `--emit-llvm` of every example in every configuration,
the tutorial, the benchmarks, the fuzzers, `tests/embed` and the bigint
calculator is byte-identical with the pass on and off. The only IR
changes came from the code fixes above.

## 8. Cost

**Run time: none.** The pass only rejects programs.

**Compile time.** The pass's time (`LOIMI_LIFETIMES_STATS=1`) and its
share of the front end measured without it (`--emit-llvm`, best of 3):

| Build | Front end without the pass | Pass | Share |
|---|---:|---:|---:|
| Lox | 367 ms | 22.6 ms | 6% |
| Mini (embedding) | 610 (627) | 30.7 (30.9) | 5% (5%) |
| MiniJS | 747 | 52.4 | 7% |
| Prolog | 326 | 25.6 | 8% |
| Python | 706 | 67.9 | 10% |
| regex | 84 | 5.8 | 7% |
| Scheme | 458 | 34.9 | 8% |
| Stack | 164 | 12.4 | 8% |
| Wasm | 5729 | 56.5 | 1% |
| tutorial step 9 | 205 | 14.2 | 7% |
| `tests/run/std_obj.lm` | 82 | 5.1 | 6% |
| `tests/bigint/calc.lm` | 93 | 6.0 | 6% |
| `tests/fuzz/fuzzcombo.lm` | 440 | 13.7 | 3% |
| `bench/obj/ic.lm` | 75 | 4.4 | 6% |

Against a whole release build (clang -O3) the pass is under 1%
(Python: 52 ms of 11.8 s).

The first prototype added 16–280% to the front end (Wasm: 3.0 s to
11.5 s), because it cloned the whole state at every branch. What fixed
it, largest effect first:

- Locals leave the state when their scope ends, and only parameters and
  unbound locals start with a value. Wasm's generated verifier (5716
  locals) went from 4.5 s to 17 ms.
- The state's parts are shared (`Rc`) and copied only when a branch
  changes them; paths are inline arrays; a join skips shared parts.
- Each function's errors come from its last summary analysis, not an
  extra pass.
- One restructure scan per call for all of the callee's paths.
- The same reason at the same place gets the same index, so loops reach
  their fixpoint before the iteration cap.
- Summaries drop restructures cut at the depth bound (section 6, item
  4).
- `pairs` name only the block used, at most two dereferences down. The
  first version listed 1357 pairs for MiniJS's `Gen.stmt`, and the pass
  took 489 ms on MiniJS.
- A post-order worklist and a fast hasher for ids.

**Implementation**: `src/lifetimes.rs` (~2460 lines, no dependency on
codegen); the attributes in the parser and checker; a summary map in
`tir::Program`.

## 9. What stays unchecked, and why

- **The allocator, collector, inline caches and limb arithmetic**
  (`mem`, `gc`, `obj`, `bigint`, `gil`): their pointers are computed,
  not derived from places the rule can name.
- **vm handlers' operand accesses** (`regs[a]` on verified bytecode):
  not bounds-checked, but checked for staleness across `reenter`.
- **Register windows and frames stored in the context**
  (`ctx.stack + base`): pointers stored in memory (section 6, item 1).
- **Natives that take `args: *V` into the value stack** rely on the
  stack not being freed during the call (Mini's `VStack.prev`). The rule
  treats this as the caller's promise, as for every pointer parameter.
