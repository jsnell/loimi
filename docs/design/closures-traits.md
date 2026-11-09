# Design note: closures and static traits

How traits, closures and effects on function types are built, and why.
For what the language accepts, see `language.md` (section 1.2 for traits
and closures, section 5.2 for effects on function types).

All four features are built:

| Feature | What it is |
|---|---|
| Effects on function types | `fn(..) uses E`: a call through a function value is checked and rooted like a direct call |
| Static traits | `trait`, `impl Trait for Type;` and bounds `T: Trait` on generics |
| `block fn` | A closure that can't outlive the call it is passed to |
| `closure fn`, `Gc<closure fn>` | A closure that can be stored, with an explicitly allocated environment |

## Why

Before these features, Loimi had generics (code generated per instance)
and plain function pointers (`fn(A) -> R`). Two workarounds were common:

- **Callbacks with context.** `std.parse.Pratt<P, K, E, X>` stores five
  function pointers that all take a `*P` context. A sort comparator that
  needs a table goes through a global or a context struct.
- **Generics with unstated requirements.** `HashMap<K, V>` calls
  `k.hash()` and `k.eq(o)`. A missing method was reported inside
  `std/map.lm` when the instance was generated, not at the user's
  `HashMap<Sym, i64>`.

Every feature is additive: existing code compiles as before. The IR of
`examples/mini`, `minijs` and `scheme` was byte-identical before and after
escaping closures were added.

## 0. Effects on function types

`Ty::Fn` carries an effect mask.

- `coerce` lets a function value (or `?fn`) widen to a type with more
  effects, but never behind a pointer.
- `finish_call` takes the effect check and the handler `sync` wrapping
  from the callee's type, for every call.
- `gcgen::push_frame` also reads the effects from the callee's type. So a
  `uses gc` function called through a `fn(..)` value is checked against
  the caller, and the caller's traced locals are rooted.
- Generic inference ignores effects. They are checked when the argument
  is coerced to the instance's parameter type.

There are no effect variables: effects are written on function types.

## 1. Static traits

```loimi
trait Hash {
    fn hash(k: *Self) -> u64;
    fn eq(a: *Self, b: *Self) -> bool;
}

impl Hash for Sym;            // checks that fn Sym.hash and fn Sym.eq exist with these signatures

struct HashMap<K: Hash, V> { .. }
fn HashMap<K: Hash, V>.get(m: *HashMap<K, V>, key: K) -> ?V { .. key.hash() .. }
```

### Impls hold no methods

A trait lists method signatures over `Self`, with optional default
bodies. `pub trait` exports it.

`impl Trait for Type;` does not contain methods. It checks that the
ordinary methods `fn Type.name` exist, with:

- signatures that match the trait's, with `Self` = `Type`;
- effects that are a subset of the trait's.

It fills in defaults for missing methods. Because methods stay where they
are, a type can satisfy a trait it was not written for. Duplicate impls
are an error.

### Bounds

Bounds `T: A + B` can be put on type parameters of structs, enums,
functions and methods. Trait names may be module-qualified (`hash.Hash`).

- Bounds are checked when a concrete instance is created. The error is
  at the instantiating type or call: "`Sym` does not implement `Hash`
  (needed by `HashMap<Sym, i64>`)".
- A method of a generic type gets the type's bounds on its receiver's
  type parameters.
- `T.zero()` on a type parameter resolves through the bound.

### Generic bodies are checked against their bounds

Code is still generated per instance. But first the body is checked once
with each bounded parameter replaced by `Ty::Opaque(i)`, a type that has
only the methods of its bounds. Using a field, an operator or a coercion
to a concrete type on it fails.

Unbounded parameters keep their concrete argument in this pass, so code
that relies on them having some method still works. The pass runs once
per combination of the unbounded parameters' arguments, and its output is
discarded. Errors point into the generic code with an "instantiated here"
note.

### Default methods

Each trait method is registered as a generic function over
`[trait params.., Self, method params..]`, with `Self` bounded by the
trait.

- Its signature is what a call through a bound is checked against.
- Its body, if any, is the default, instantiated with `Self` = the
  implementing type.
- Method lookup on a concrete type tries the type's own methods first,
  then a default from a trait the type implements. Two traits providing
  the same default is an ambiguity error.

### Generic traits and generic impls

- Traits can have type parameters (`trait Hooks<K, E, X>`). Bounds and
  impls give the arguments: `P: Hooks<K, E, X>`, `impl Conv<i64> for Sym;`.
  The impl table is keyed by (trait, arguments, type).
- Trait methods can be generic (`fn fold<U>(..)`). The impl check
  instantiates both signatures with fresh opaque types for `U`.
- Impls can be generic: `impl<T: Hash> Hash for Vec<T>;`. The type and
  trait arguments are patterns over the impl's parameters.
  `implements(ty, trait, args)` unifies them with the instance and checks
  the impl's bounds on the bindings, recursively. The impl's methods are
  checked once, with its parameters as opaque types that have the impl's
  bounds.

### Overlap

`register_impls` rejects two impls of one trait whose (type, trait
arguments) unify. This covers a plain impl against a generic one, and
two generic impls (unifying over both impls' parameters). Bounds are
ignored, as in Rust.

There is no orphan rule. The whole program is checked at once, and an
impl carries no code, so where it sits does not matter.

### Cost

Calls through a bound are ordinary direct calls. They inline and cost
nothing at run time, and codegen has no trait-specific code.

An impl's method may have fewer effects than the trait declares, never
more. So a handler that calls `k.hash()` through a bound is checked
against the trait's declaration, and that check holds for every instance.

### In std

- `std.hash` has `trait Hash { hash, eq }`, with impls for the integer
  types and `str`. `std.map.HashMap<K: hash.Hash, V>` uses it.
- `std.parse` has `Parser<P: Hooks<K, E, X>, K, E, X>`. The Pratt hooks
  are a trait, so hook calls are direct and can inline. `before_rhs` and
  `led` have defaults. `led` is one method that dispatches on the
  operator kind, instead of a table of function pointers. `Pratt` stays
  for its existing users.

### Limits

- No `dyn`: traits are static. Interpreters use `value` types and tagged
  enums for run-time polymorphism.
- No associated types or constants, and no supertraits.
- Default bodies are checked when an instance uses them, not at the
  trait.

Tests: `tests/run/traits_full.lm`, `std_parse_hooks.lm`,
`tests/fail/trait_*.lm` (overlap: `trait_impl_overlap{,_generic}.lm`).

## 2. Non-escaping closures: `block fn`

For callbacks that run during the call that receives them: comparators,
predicates, per-element callbacks, visitor hooks.

```loimi
fn Vec<T>.sort_by(v: *Vec<T>, cmp: block fn(*T, *T) -> i32) { .. }

keys.sort_by(|a, b| if *a < *b { -1 } else if *a > *b { 1 } else { 0 });
```

### The escape rule

A `block fn` type is allowed only as the whole type of a parameter (of a
function, a function type or another `block fn`). It is never a type
argument.

- A `block fn` local can only be called or passed to a `block fn`
  parameter.
- A closure literal can only be passed to a `block fn` parameter.
- Storing it, returning it, `&f`, casts and passing it as a `T` are
  errors.

The rule is syntactic, so no lifetime analysis is needed.

### Captures and checking

Captures are by reference to the enclosing frame's locals. Changing a
captured `var` (`count += 1`) is visible to the caller.

The body is checked with the enclosing function's checker state (`FnCx`),
but with the closure's own return type, loops and effects:

- `return` and `?` leave the closure;
- `break` cannot reach an outer loop;
- calls need effects that the `block fn` type allows.

The closure becomes its own `FnDef` that shares the enclosing function's
local table. Its captures are the locals it uses that were declared
before it (`tirwalk::locals`, which also sees nested closures'
captures). In a generic call, closure arguments are checked last, against
the partly known parameter type.

### Lowering

The `{ code, env }` pair is passed as two scalar arguments. A literal's
code pointer is a constant, and LLVM's function specialization clones a
callee only for constant scalar arguments. With one aggregate argument, a
sort called from several sites kept an indirect comparator call and ran
1.5x slower than the hand-written sort (`bench/sort`). With two scalars,
`mem.sort_by` with a closure runs as fast as the hand-written sort,
without `#[inline(always)]`.

- **Small immutable captures are copied** into the environment: a `let`
  of at most 8 bytes with no GC reference. That is one load per use
  instead of two.
- A closure call that stays indirect costs the same as a function pointer
  call.
- **Plain functions** convert to `block fn` through one trampoline per
  signature, `(env, args..) -> env(args..)`, with the function pointer as
  the environment. That costs one extra jump when the call stays
  indirect.

### GC

Captured traced locals stay in the enclosing frame's slots and are never
copied. So the shadow-stack frame of the call that received the closure
roots them, and the closure sees their current values. A call through a
`block fn` that `uses gc` roots the caller's traced locals like any call.
`tests/run/gc_closures.lm` runs under GC stress and fails if those calls
are not rooted.

### Limits

- Closures cannot be created in op handlers, vm functions, entries or
  `on raise`. A handler calls a plain function that creates the closure,
  or passes a named function.
- vm functions and C functions cannot take `block fn` parameters.

So `musttail`, cold outlining and state variables in vm code are
unaffected.

In std: `Vec<T>.sort_by(cmp)` and `mem.sort_by(p, n, cmp)` (stable merge
sort), `obj.index_keys_first_by(keys, index_of)`.

Tests: `tests/run/closures.lm`, `gc_closures.lm`, `sort_by.lm`,
`tests/fail/{closure,block}_*.lm`.

## Escaping closures

Interpreter and front-end code also has callbacks that are stored:

- builtin tables whose entries need context;
- hooks registered with a parser, REPL or host;
- generators in front-end code;
- a comparator kept in a configuration struct.

Without closures, each is a `fn(*Ctx, ..)` plus a `*Ctx` stored next to
it (the C, Zig and Odin idiom), and a generic container cannot hold "a
callable" without knowing the context type.

Other languages don't fit Loimi's constraints. Rust's closures are safe
and allocate explicitly, but rely on a borrow checker and destructors.
Zig, Odin and C3 have no capturing closures. Languages with a GC
allocate environments implicitly, which Loimi rules out (no hidden
allocation).

### Types

| Type | Representation | Notes |
|---|---|---|
| `fn(A) -> R uses E` | Code pointer | |
| `block fn(A) -> R uses E` | `{ code, env }`, two scalar arguments | Can't escape (above) |
| `closure fn(A) -> R uses E` | `{ code, env }`, 16 bytes, LLVM `{ ptr, ptr }` | A value type; not traced |
| `Gc<closure fn(A) -> R uses E>` | One traced pointer to a GC object `{ code, captures.. }` | Rooted, traced, write-barriered and freed like any `Gc<T>` |

A `closure fn` can be copied, stored in fields, locals, globals, `Vec`s
and `?` options, returned, and used as a type argument. It is passed as
one aggregate, not split like `block fn`, because it is stored far more
often than it is called at a call site LLVM could specialize. The code
takes the environment as a hidden first argument, as for `block fn`. Its
environment is host memory, so it needs no rooting or barriers.

A separate `closure` keyword keeps plain function pointers one word, and
leaves every existing signature, the C ABI and the IR unchanged.

### Creation is an explicit allocation

```loimi
let add = new(&arena, |x: i64| x + k);      // closure fn(i64) -> i64, env in the arena
let hook = new(heap, |v: Value| log(v, tag)); // Gc<closure fn(Value)>, env in the GC heap
```

- `new(a, |..| ..)` gives a `closure fn`, for any `a` that has
  `alloc_bytes(n: u64, align: u64) -> *u8`: `*mem.Arena`, `*mem.Malloc`
  or a user's allocator. The checker checks `a.alloc_bytes(size, align)`
  as an ordinary method call, so its effects count.
- A closure that captures nothing allocates nothing: its environment is
  null.
- `new(heap, |..| ..)` with `heap: *gc.Heap` gives a `Gc<closure fn>`,
  allocated like `new(heap, Obj { .. })`. Allocation does not collect.
- Parameter and result types come from the expected type or from
  annotations on the literal.
- `new(a, |..| ..)` is recognized before the GC-only `new`, so it works
  without `import std.gc`.

### Captures are by value

The environment holds a copy of each captured local, made at creation,
so nothing in it points into the creating frame. Capturing a pointer is
allowed. `&local` inside the body refers to the environment's copy.

`check_escaping_captures` (`src/closures.rs`) rejects:

- assigning a captured local in the body, including `x += 1`, `x.f`,
  `x[i]` on an array, and from a nested `block fn`. The error says
  "captured by value: the closure has its own copy; keep shared state
  behind a pointer";
- capturing a `block fn` parameter;
- a traced capture (`Gc<T>`, a `value` type with references, or a struct
  holding one) in an untraced environment. The error points to
  `new(heap, ..)`;
- in a GC closure, `&` of a traced capture, so that no store into the
  environment can miss the write barrier.

### State lists

A state list gives a closure its own mutable variables:
`new(a, [var i = lo, var n] |..| body)`. The syntax is C++'s and Swift's
capture list; `[var` cannot start an array literal.

- The initializers are evaluated in the enclosing scope.
- The state variables are declared in the closure's scope before its
  capture boundary, so they become ordinary environment fields.
- `new_closure` wraps the result in `{ let n = init; ..; NewClosure }`,
  so the enclosing frame roots the initial values until they are copied.
- State may be assigned. In a GC closure, an assignment to a traced
  environment field is followed by `std.gc.barrier(%env)`
  (`gcgen::env_barrier`), and assigning a traced state variable from a
  nested `block fn` is an error.
- A `block fn` literal with a state list is an error.

### Shared state

Shared state stays explicit: a cell made with `new(a, v)` and captured as
a pointer. When `a` is not a `*gc.Heap`, this is
`a.alloc_bytes(size_of T, align_of T) as *T` followed by a store. Values
with GC references are rejected there, as in arena closures.

### Environments, GC and effects

- The environment is a `StructDef` generated per literal, named
  `<symbol> env`. For a `closure fn` it is `{ caps.. }`. For a GC closure
  it is an `object` `{ code: *u8, caps.. }` with its own type id, traced
  by `loimi_trace_object` like any object.
- `TK::NewClosure(fn, caps, alloc)` creates it: it calls the allocator
  (or `std.gc.alloc`), stores the code pointer (GC only) and copies the
  captures.
- The closure function takes `ptr %env` first and uses the environment's
  fields as the captured locals' slots.
- A GC closure keeps `%env` in a hidden traced local for the whole body.
  Its captures stay reachable across the body's own `gc` calls even when
  the caller has dropped its reference.
- The types are `Ty::Closure(params, ret, effects)` and
  `Ty::Gc(Ty::Closure)`. `callee_sig` gives the signature a value is
  called with. `finish_call`, field calls (`t.hook(x)`),
  `gcgen::push_frame` and `vmcheck::wrap_effects` all use it, so calls are
  checked, synced and rooted in one place, with the effects of section 0.
- Generic inference unifies `closure fn` types like `fn` types, as in
  `fn map<T, U>(xs: []T, f: closure fn(T) -> U)`. `Vec<closure fn(i64) ->
  i64>` is a valid type. A `new(a, |..| ..)` argument for a partly known
  `closure fn` parameter is checked last.

### Conversions

| From | To | How |
|---|---|---|
| Plain `fn` | `closure fn` | No allocation: the code is the signature's `block fn` trampoline, the environment is the function pointer. So a builtin table can mix functions and closures. |
| `closure fn` | `block fn` | Free (same representation). A stored comparator can be passed to `sort_by`. |
| `Gc<closure fn>` | — | Converts to neither. An untraced copy would not keep the object alive. Only `new(heap, ..)` makes one. |

### vm code and C

- Creating a closure in vm code is forbidden, as for `block fn`.
- Calling a stored `closure fn` or `Gc<closure fn>` from a handler or vm
  function is allowed. It is an indirect call like a `fn` field, synced
  and rooted according to its effects. vm functions may take `closure fn`
  parameters.
- Neither type crosses into C: `export "C"`, `extern` and the header
  generator reject them. C callbacks use a plain `fn` and a context
  pointer.

Nothing allocates unless the source says `new`.

### Freeing

An arena frees all its closures with `arena.free()`. GC closures are
collected.

One closure is freed with `c.free(a)`, which lowers to
`{ let c' = c; if let Some(p) = closure_env(c') { a.free_bytes(p) } }`.

- `closure_env` gives the environment, unless it is null (no captures)
  or the code is the signature's trampoline (a converted plain function).
- Trampolines are keyed by LLVM signature, which effects don't change, so
  a closure widened to more effects is still recognized.
- `a` is evaluated only when there is something to free.
- `Arena.free_bytes` does nothing. `std.mem.Malloc` (C heap, with a `live`
  count) frees. Both are emitted only where used (`LAZY_STD`,
  `LAZY_STD_TYPES`).

### Limits

There is no ownership tracking. A copy of a freed closure dangles, as
with `Vec.free`, and a closure used after its arena was freed is a
use-after-free. `--sanitize=address` reports both; the compiler does not.

### Tests

- Run tests: `tests/run/closures_escaping.lm`; `gc_closures_escaping.lm`
  and `_gen.lm` (GC stress, generational verify mode);
  `vm_closure_call.lm` (handlers and vm functions calling arena and GC
  closures, all dispatch modes); `closures_state.lm`;
  `gc_closures_state.lm` (generational, verify, `--gc-check-roots`);
  `closures_free.lm`; `closures_shared_cell.lm`.
- Failure tests: `tests/fail/escaping_*.lm`, `closure_state_*.lm`,
  `closure_free_*.lm`, `new_cell_*.lm`.
- The rooting fuzzer (`tests/fuzz/rootgen.rs`) generates GC closures with
  traced state assigned around collections.
- Under `--sanitize=address` with `detect_leaks=1`, the run tests report
  no leaks, and a call after `free` is reported as a heap use-after-free.

## Tried and rejected

- **One aggregate argument for `block fn`**: kept the comparator call
  indirect when a sort had several callers (1.5x the hand-written sort,
  `bench/sort`). Replaced by two scalar arguments.
- **A type per closure literal, with the callee instantiated per closure**
  (Rust's `impl Fn`): not needed. LLVM inlines or specializes the common
  case, and the indirect case costs a function pointer call (`bench/sort`).
- **Checking generic bodies with every parameter opaque**: breaks code
  that relies on unbounded parameters having some method. Only bounded
  parameters are made opaque.
- **Closures in vm handlers, capturing state variables by value**: no
  interpreter in the repository needed them.
- **Assignable captures in escaping closures**: capture by reference
  needs the variable boxed on the heap (Swift, C#, Go), a hidden
  allocation. Writing to the closure's copy (Rust `move`) makes
  `count += 1` silently not count. State lists replace both.
