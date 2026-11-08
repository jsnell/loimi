# Design note: closures and static traits

Built 2026-11-02 and 2026-11-03. Language-level description in
`language.md` §1.2 ("Traits", "Closures", "Escaping closures") and §5
("Effects on function types"). This note records how the features are
built and why.

Before them, Loimi had monomorphized generics and plain function pointers
(`fn(A) -> R`). Two workarounds were common:

- **Callbacks with context.** `std.parse.Pratt<P, K, E, X>` stores five
  function pointers that all take a `*P` context, and a sort comparator
  that needs a table goes through a global or a context struct.
- **Duck-typed generics.** `HashMap<K, V>` calls `k.hash()` and `k.eq(o)`
  with nothing stating the requirement, so a missing method was reported
  inside `std/map.lm` at instantiation, not at the user's
  `HashMap<Sym, i64>`.

Every feature here is additive: `fn` values, `block fn` and existing code
compile as before, and the IR of `examples/mini`, `minijs` and `scheme`
was byte-identical before and after escaping closures.

## 0. Effects on function types

`Ty::Fn` has an effect mask. `coerce` lets a function value (or `?fn`)
widen to a type with more effects (never behind a pointer).
`finish_call` runs the effect check and the handler `sync` wrapping for
every call from the callee's type, and `gcgen::push_frame` reads the
effects from the callee's type, so a `uses gc` function called through a
`fn(..)` value is checked against the caller and its traced locals are
rooted. Generic inference ignores effects and checks them when the
argument is coerced to the instance. Effects are declared on function
types; there are no effect variables.

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

- A trait lists method signatures over `Self`, with optional default
  bodies. `pub trait` exports it.
- `impl Trait for Type;` **does not contain the methods**. It asserts
  that the ordinary methods (`fn Type.name`) exist with matching
  signatures (with `Self` = `Type`) and effects that are a subset of the
  trait's, and fills in defaults for the missing ones. Methods stay where
  they are, so a type can satisfy a trait it was not written for.
  Duplicate impls are an error.
- **Bounds** `T: A + B` (trait names may be module-qualified,
  `hash.Hash`) on type parameters of structs, enums, functions and
  methods are checked when a concrete instance is created, with the
  error at the instantiating type or call: "`Sym` does not implement
  `Hash` (needed by `HashMap<Sym, i64>`)". A method of a generic type has
  the type's bounds on its receiver parameters (implied bounds).
- **Default methods.** Every trait method is registered as a generic
  function over `[trait params.., Self, method params..]` with `Self`
  bounded by the trait. Its signature is what a call through a bound
  checks against; its body, if any, is the default, instantiated with
  `Self` = the implementing type. Method lookup on a concrete type tries
  the type's own methods first, then a default of a trait the type
  implements (two traits providing the same default is an ambiguity
  error).
- **Generic traits** (`trait Hooks<K, E, X>`), trait arguments in bounds
  and impls (`P: Hooks<K, E, X>`, `impl Conv<i64> for Sym;`), and
  **generic trait methods** (`fn fold<U>(..)`: the impl check instantiates
  both signatures with fresh opaque types for `U`). The impl table is
  keyed by (trait, arguments, type).
- **Generic impls** `impl<T: Hash> Hash for Vec<T>;`: type and trait
  arguments are patterns over the impl's parameters; `implements(ty,
  trait, args)` unifies them with the instance and checks the impl's
  bounds on the bindings, recursively. The impl's methods are checked
  once, with its parameters as opaque types that have the impl's bounds.
- **Generic bodies are checked against their bounds.** Code is still
  generated per instance, but before an instance is checked, the body is
  checked once with each *bounded* parameter replaced by `Ty::Opaque(i)`,
  a type that has only the methods of its bounds: fields, operators or
  coercions to a concrete type fail. Unbounded parameters keep their
  concrete argument in that pass, so they keep duck typing. The pass runs
  once per combination of the unbounded parameters' arguments, its output
  is discarded, and errors point into the generic code with an
  "instantiated here" note.
- **Static trait functions**: `T.zero()` on a type parameter resolves
  through the bound.
- Calls through a bound are ordinary direct calls, so they inline and
  cost nothing at run time; codegen has no trait-specific code. An
  impl's method may have fewer effects than the trait declares, never
  more, so a handler calling `k.hash()` through a bound is checked
  against the trait's declaration and is sound for every instance.
- **Overlap check** (`register_impls`): two impls of one trait whose
  (type, trait arguments) unify are an error, a plain impl against a
  generic one or two generic impls against each other (two-sided
  unification over both impls' parameters). Bounds are ignored, as in
  Rust. There is no orphan rule: the whole program is checked at once,
  and impls carry no code (the methods are the type's), so where an impl
  sits does not matter. Tests: `tests/fail/trait_impl_overlap{,_generic}.lm`.

Limitations: traits are static (no `dyn`; interpreters use `value` types
and tagged enums for run-time polymorphism), with no associated types or
constants and no supertraits. Default bodies are checked when an instance uses them, not
at the trait.

In std: `std.hash` has `trait Hash { hash, eq }` with impls for the
integer types and `str`, and `std.map.HashMap<K: hash.Hash, V>`.
`std.parse` has `Parser<P: Hooks<K, E, X>, K, E, X>`: the Pratt hooks as
a trait (`before_rhs` and `led` have defaults; `led` is one method
dispatching on the operator kind instead of a table of function
pointers), so hook calls are direct and inlinable. `Pratt` stays for its
existing users.

Tests: `tests/run/traits_full.lm`, `std_parse_hooks.lm`,
`tests/fail/trait_*.lm`.

## 2. Non-escaping closures: `block fn`

For callbacks that run during the call that receives them: comparators,
predicates, per-element callbacks, visitor hooks.

```loimi
fn Vec<T>.sort_by(v: *Vec<T>, cmp: block fn(*T, *T) -> i32) { .. }

keys.sort_by(|a, b| if *a < *b { -1 } else if *a > *b { 1 } else { 0 });
```

- **Escape rule.** `block fn` types are allowed only at the top of a
  parameter type (of a function, a function type or another `block fn`),
  never as type arguments. A `block fn` local can only be a callee or an
  argument for a `block fn` parameter, and a closure literal only such an
  argument. Storing, returning, `&f`, casts and passing it as a `T` are
  errors. The rule is syntactic, so no lifetime analysis is needed.
- **Captures are by reference** to the enclosing frame's locals, so
  mutation of a captured `var` (`count += 1`) is visible to the caller.
- **Checking.** The body is checked with the enclosing `FnCx`, with the
  closure's own return type, loops and effects: `return` and `?` leave
  the closure, `break` cannot reach an outer loop, and calls need effects
  the `block fn` type allows. The closure becomes its own `FnDef` sharing
  the enclosing function's local table; its captures are the locals it
  uses that were declared before it (`tirwalk::locals`, which also sees
  nested closures' captures). In a generic call, closure arguments are
  checked last against the partly known parameter type.
- **Lowering.** The `{ code, env }` pair is passed as **two scalar
  arguments**. A literal's code pointer is a constant and LLVM's function
  specialization clones a callee only for constant scalar arguments; with
  one aggregate, a sort called from several sites kept an indirect
  comparator call (1.5x the hand-written sort, `bench/sort`). Split,
  `mem.sort_by` with a closure runs as fast as the hand-written sort
  without `#[inline(always)]`. **Small immutable captures are copied**
  into the environment (a `let` of at most 8 bytes with no GC reference):
  one load per use instead of two. A closure call that stays indirect
  costs the same as a function pointer call.
- **Plain functions** convert to `block fn` through one trampoline per
  signature, `(env, args..) -> env(args..)`, with the function pointer as
  the environment: one extra jump when the call stays indirect.
- **GC.** Captured traced locals stay in the enclosing frame's slots and
  are never copied, so the shadow-stack frame of the call that received
  the closure roots them and the closure sees their current values. A
  call through a `block fn` that `uses gc` roots the caller's traced
  locals like any call. `tests/run/gc_closures.lm` runs under GC stress
  and fails if those calls are not rooted.

Limitations: closures cannot be created in op handlers, vm functions,
entries or `on raise`, and vm functions cannot take `block fn`
parameters, so `musttail`, cold outlining and state variables are
unaffected; a handler calls a plain function that creates the closure, or
passes a named function. C functions cannot take `block fn` parameters.

In std: `Vec<T>.sort_by(cmp)` and `mem.sort_by(p, n, cmp)` (stable merge
sort), `obj.index_keys_first_by(keys, index_of)`.

Tests: `tests/run/closures.lm`, `gc_closures.lm`, `sort_by.lm`,
`tests/fail/{closure,block}_*.lm`.

## Escaping closures

Interpreter and front-end code also has callbacks that are **stored**:
builtin tables whose entries close over context, hooks registered with a
parser, REPL or host, generators in front-end code, a comparator kept in
a configuration struct. Without closures each is a `fn(*Ctx, ..)` plus a
`*Ctx` stored next to it (the C/Zig/Odin idiom), and a generic container
cannot hold "a callable" without knowing the context type.

Rust is the only common language whose closures are both safe and
allocation-explicit, and it relies on a borrow checker and destructors;
Zig, Odin and C3 have no capturing closures, and GC'd languages allocate
environments implicitly, which Loimi rules out ("no hidden allocation").

### Types

- `fn(A) -> R uses E`: a thin code pointer.
- `block fn(A) -> R uses E`: the non-escaping pair above.
- **`closure fn(A) -> R uses E`**: an ordinary value type, a fat pointer
  `{ code, env }` (16 bytes; LLVM `{ ptr, ptr }`, passed as one aggregate
  since it is stored far more often than called through a specializable
  call site). Copyable, storable in fields, locals, globals, `Vec`s and
  `?` options, returnable, usable as a type argument. The code takes the
  environment as a hidden first argument, like `block fn`. **Not
  traced**: the environment is host memory.
- **`Gc<closure fn(A) -> R uses E>`**: a thin traced reference to a GC
  object `{ code, captures.. }`, rooted, traced and write-barriered by the
  existing `Gc<T>` machinery and freed by the collector.

The untraced kind costs no rooting or barriers; the GC kind is one word
the collector already understands. A separate `closure` keyword keeps
plain function pointers thin and every existing signature, C ABI and IR
unchanged.

### Creation is an explicit allocation

```loimi
let add = new(&arena, |x: i64| x + k);      // closure fn(i64) -> i64, env in the arena
let hook = new(heap, |v: Value| log(v, tag)); // Gc<closure fn(Value)>, env in the GC heap
```

- `new(a, |..| ..)`, with any `a` that has `alloc_bytes(n: u64, align:
  u64) -> *u8` (`*mem.Arena`, `*mem.Malloc`, a user's allocator), gives a
  `closure fn`; the checker type-checks the call `a.alloc_bytes(size,
  align)` as an ordinary method call, so its effects count. A closure
  that captures nothing allocates nothing (null environment).
- `new(heap, |..| ..)` with `heap: *gc.Heap` gives a `Gc<closure fn>`,
  allocated like `new(heap, Obj { .. })` (allocation does not collect).
- Parameter and result types come from the expected type or from
  annotations on the literal. `new(a, |..| ..)` is recognized before the
  GC-only `new`, so it works without `import std.gc`.

### Captures are by value

The environment holds a copy of each captured local, made at creation,
so nothing in it points into the creating frame. `check_escaping_captures`
(`src/closures.rs`) rejects:

- **assigning a captured local** in the body (also `x += 1`, `x.f`,
  `x[i]` on an array, and from a nested `block fn`): "captured by value:
  the closure has its own copy; keep shared state behind a pointer";
- **capturing a `block fn`** parameter;
- a **traced capture** (`Gc<T>`, a `value` type with references, a struct
  holding one) in an untraced environment, with a pointer to
  `new(heap, ..)`;
- in a GC closure, `&` of a traced capture (so no store into the
  environment can miss the write barrier).

Capturing a pointer is allowed; `&local` inside the body refers to the
environment's copy.

**State lists** give a closure its own mutable variables:
`new(a, [var i = lo, var n] |..| body)` (C++/Swift capture-list syntax;
`[var` cannot start an array literal). The initializers are evaluated in
the enclosing scope and the state variables are declared in the
closure's scope before its capture boundary, so they become ordinary
environment fields. `new_closure` wraps the result in `{ let n = init;
..; NewClosure }`, so the enclosing frame roots the initial values until
they are copied. State may be assigned; in a GC closure, a traced state
variable assigned from a nested `block fn` is an error, and an assignment
to a traced environment field is followed by `std.gc.barrier(%env)`
(`gcgen::env_barrier`). A `block fn` literal with a state list is an
error.

**Shared state** stays explicit: a cell from `new(a, v)` (`a` not a
`*gc.Heap`: `a.alloc_bytes(size_of T, align_of T) as *T`, then a store),
captured as a pointer. Values with GC references are rejected there, as
in arena closures.

### Environments, GC and effects

- The environment is a `StructDef` generated per literal (`<symbol>
  env`): `{ caps.. }`, or for a GC closure an `object` `{ code: *u8,
  caps.. }` with its own type id, traced by `loimi_trace_object` like any
  object. `TK::NewClosure(fn, caps, alloc)` carries it; creation calls the
  allocator (or `std.gc.alloc`), stores the code pointer (GC only) and
  copies the captures. The closure function gets `ptr %env` first and
  uses the environment's fields as the captured locals' slots.
- A GC closure stores `%env` in a hidden traced local live for the whole
  body, so its captures stay reachable across the body's own `gc` calls
  even when the caller dropped its reference.
- `Ty::Closure(params, ret, effects)`; `Gc<closure fn>` is
  `Ty::Gc(Ty::Closure)`. `callee_sig` gives the signature a value is
  called with, used by `finish_call`, field calls (`t.hook(x)`),
  `gcgen::push_frame` and `vmcheck::wrap_effects`, so calls are checked,
  synced and rooted from one place, with the effects of section 0.
- Generic inference unifies `closure fn` types like `fn` types
  (`fn map<T, U>(xs: []T, f: closure fn(T) -> U)`), `Vec<closure fn(i64)
  -> i64>` is a valid type, and a `new(a, |..| ..)` argument for a partly
  known `closure fn` parameter is checked last.

### Conversions

- A plain function converts to `closure fn` without allocating: the code
  is the per-signature `block fn` trampoline, the environment is the
  function pointer. A builtin table can mix functions and closures.
- A `closure fn` converts to `block fn` for free (same representation),
  so a stored comparator can be passed to `sort_by`.
- Only `new(heap, ..)` makes a `Gc<closure fn>`, and it converts to
  neither `closure fn` nor `block fn` (an untraced copy would not keep the
  object alive).

### vm code and C

Creating a closure in vm code is forbidden, as for `block fn`. Calling a
stored `closure fn` or `Gc<closure fn>` from a handler or vm function is
allowed: an indirect call like a `fn` field, synced and rooted per its
effects. vm functions may take `closure fn` parameters. Nothing
allocates unless the source says `new`. Neither type crosses into C
(`export "C"`, `extern` and the header generator reject them): C
callbacks use a plain `fn` and a context pointer.

### Freeing

An arena frees all its closures with `arena.free()`; GC closures are
collected. One closure is freed with `c.free(a)`, which lowers to
`{ let c' = c; if let Some(p) = closure_env(c') { a.free_bytes(p) } }`.
The intrinsic gives the environment unless it is null (no captures) or
the code is the signature's trampoline (a converted plain function);
trampolines are keyed by LLVM signature, which effects do not change, so
a closure widened to more effects is still recognized. `a` is evaluated
only when there is something to free. `Arena.free_bytes` is a no-op;
`std.mem.Malloc` (C heap, `live` count) frees. Both are emitted only
where used (`LAZY_STD`, `LAZY_STD_TYPES`).

Limitations: there is no ownership tracking. A copy of a freed closure
dangles, as with `Vec.free`, and a closure used after its arena was freed
is a use-after-free; `--sanitize=address` reports both, the compiler
does not.

### Tests

`tests/run/closures_escaping.lm`, `gc_closures_escaping.lm` and `_gen.lm`
(GC stress, generational verify mode), `vm_closure_call.lm` (handlers
and vm functions calling arena and GC closures, all dispatch modes),
`closures_state.lm`, `gc_closures_state.lm` (generational, verify,
`--gc-check-roots`), `closures_free.lm`, `closures_shared_cell.lm`;
`tests/fail/escaping_*.lm`, `closure_state_*.lm`, `closure_free_*.lm`,
`new_cell_*.lm`. The rooting fuzzer (`tests/fuzz/rootgen.rs`) generates
GC closures with traced state assigned around collections. Under
`--sanitize=address` with `detect_leaks=1` the run tests report no
leaks, and a call after `free` is reported as a heap use-after-free.

## Tried and rejected

- **One aggregate argument for `block fn`**: kept the comparator call
  indirect when a sort had several callers (1.5x the hand-written sort,
  `bench/sort`); replaced by two scalar arguments.
- **A type per closure literal, callee instantiated per closure** (Rust's
  `impl Fn`): not needed; LLVM inlines or specializes the common case and
  the indirect case costs a function pointer call (`bench/sort`).
- **Checking generic bodies once with every parameter opaque**: breaks
  code that relies on duck typing for unbounded parameters; only bounded
  parameters are made opaque.
- **Closures in vm handlers with by-value capture of state variables**:
  no interpreter in the repository needed them.
- **Assignable captures in escaping closures**: by reference needs the
  variable boxed on the heap (Swift, C#, Go), a hidden allocation; a
  write to the closure's copy (Rust `move`) lets `count += 1` silently
  not count. State lists replace both.
