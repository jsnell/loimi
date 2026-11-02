# Design note: closures and static traits

Status: implemented. Section 0 first, then section 2 in the reduced scope
of "Step 1 scope", then the rest of section 2 and section 1 as described
in "Step 2 (implemented)" at the end, which also lists where the result
differs from the proposal. Written 2026-11-02.

Loimi has generics (monomorphized, with methods resolved per instance) and
plain function pointers (`fn(A) -> R`). It has no closures and no traits.
Library and interpreter authors work around both:

- **Callbacks with context.** `std.parse.Pratt<P, K, E, X>` stores five
  function pointers that all take a `*P` context, and the user writes five
  top-level functions that cast or dereference it. A sort comparator that
  needs a table (`sort(keys, |a, b| names[a] < names[b])`) has to go
  through a global or a context struct.
- **Duck-typed generics.** `HashMap<K, V>` calls `k.hash()` and `k.eq(o)`.
  This works because methods resolve per instance, but nothing states the
  requirement: a missing method is reported inside `std/map.lm` at
  instantiation, not at the user's `HashMap<Sym, i64>`.

This note sketches the smallest form of each feature that would serve
these uses, and how each would interact with monomorphization, the `vm`
dispatch modes and the GC shadow stack. Both are additive: neither changes
how existing code compiles.

## 0. Prerequisite: effects on function types

**Done** (2026-11-02): see "Effects on function types" in
`language.md`, section 5. As built: `Ty::Fn` has an effect mask, `coerce`
allows a function value (or `?fn`) to widen to a type with more effects
(never behind a pointer), `finish_call` runs the effect check and handler
`sync` wrapping for every call from the callee's type, and
`gcgen::push_frame` reads the effects from the callee's type. Generic
inference ignores effects and checks them when the argument is coerced to
the instance. There are no effect variables (`fn each<E>(f: fn(T) uses E)
uses E`); with only three ordered effects, the plan is to add them only
when a real callback API needs both plain and effectful callbacks. The
original text follows.

Function types carried no effects. `finish_call` checks effects (and
`gcgen::push_frame` builds shadow-stack frames) only for direct calls
(`TK::FnRef`). A `uses gc` function stored in a `fn(..)` value and called
through it is neither checked against the caller's effects nor rooted. Any
callback design inherits this gap, so it should be fixed first:

- `fn(A) -> R uses gc` is a distinct type. A function converts to a function
  type only if its effects are a subset of the type's.
- An indirect call through a `uses gc` type is treated like a direct call
  to a `uses gc` function: the caller needs the effect (or is a handler or
  cold block, where the `sync` block runs), and traced locals in scope get
  a shadow-stack frame.

This is a small change in `check.rs` (`Ty::Fn` gains an effect mask,
`coerce` checks subsets, `finish_call` wraps indirect calls) and in
`gcgen::push_frame` (look at the callee's type instead of requiring a
`FnRef`).

## 1. Non-escaping closures

### What the users need

Comparators for `sort`/`binary_search`, predicates for `eat_while` and
`find`, per-element callbacks for `Vec.each` and `HashMap.each`, and
visitor hooks in tree walkers. All of these are called **during** the call
that receives them and never stored. Escaping closures (stored in structs,
returned) need heap-allocated environments, which in an interpreter
toolkit means either a GC-managed object or an arena, and an ownership
story. That is not worth it now.

### Proposed form

```loimi
fn Vec<T>.sort_by(v: *Vec<T>, less: block fn(*T, *T) -> bool) { .. }

let names = ..;
keys.sort_by(|a, b| names[*a].lt(names[*b]));
```

- A closure literal `|a, b| expr` (or `|a: *u32| -> bool { .. }`) is only
  allowed where a `block fn(..)` parameter is expected: as a call argument.
- `block fn(A) -> R` is a new type: a pair `{ code: ptr, env: ptr }`. It
  can be called, passed on as an argument to another `block fn` parameter,
  and nothing else: it cannot be stored in a field, a local declared with
  `var`, a global, returned, or captured by an escaping construct. This
  rule is purely syntactic, so no lifetime analysis is needed.
- A plain function converts to `block fn` (with a null environment), so
  existing callers keep working.
- Captures are **by reference**: the environment is a struct of pointers
  to the captured locals' stack slots, built in the caller's frame. The
  closure body is lowered to a function `(env: ptr, a: A) -> R` that reads
  and writes locals through those pointers. Mutation of captured `var`s is
  allowed and visible to the caller, which is what loops like
  `count += 1` inside a callback expect.
- Effects follow section 0: `block fn(..) uses gc`.

Using `block fn` in `std.parse` would *not* replace `Pratt`'s stored
function pointers, since the table stores them. Pratt is the trait case
(section 2); closures serve the call-and-forget callbacks.

### Monomorphization

A `block fn` is an ordinary value type (two pointers), so a generic
function taking one is instantiated once per type arguments, not once per
closure. The call through it is indirect. In practice the callee is small
and generic (`sort_by` on `Vec<u32>`); marking such functions
`#[inline(always)]` lets LLVM see the constant code pointer and inline the
closure body, the same way C++ `std::sort` gets fast with a lambda. If that
proves unreliable, a later step can make each closure literal its own type
and instantiate the callee per closure (Rust's `impl Fn`), without changing
the surface syntax: the non-escaping rule already guarantees the closure's
type never needs to be named.

The closure body is checked in the enclosing function's scope (it sees the
same type parameters), and lowered as a separate `FnDef` created during
checking, with the captured locals turned into fields of the environment.
`tirwalk::locals` already computes "locals used in an expression", which is
what cold-block outlining uses to find captures; closures would reuse it.

### `vm` dispatch modes

- **Handlers can create closures** (for example to call `sort_by` from a
  `cold` block). Capturing **state variables** by reference would force
  them into memory, which is exactly what the `vm` design avoids. So
  state variables are captured by value (read-only inside the closure),
  and writing to them inside a closure is an error. Ordinary handler
  locals are captured by reference like elsewhere.
- **tailcall mode.** A handler ends in a `musttail` call, and LLVM forbids
  passing pointers to the caller's allocas to a `musttail` callee. The
  non-escaping rule guarantees the environment is dead before the
  handler's `next`/`jump`, because it is only reachable during the call
  that received it. The environment alloca therefore never reaches the
  tail call.
- **Cold outlining.** A closure literal inside a `cold` block is lowered in
  the outlined cold function, so its environment lives in that function's
  frame. Captured handler locals reach the cold function through the
  existing thread-local slots first, and the closure then refers to the
  cold function's copies. Writes to them are visible in the rest of the
  cold block, as today.
- **goto and switch modes** need nothing special: handlers are blocks in one
  function, and the environment is an alloca in it.
- A closure body cannot contain control words (`next`, `jump`, `raise`):
  it is a separate function and cannot transfer the interpreter's control
  flow. `?` inside a closure returns from the closure, not the handler.

### GC shadow stack

By-reference capture fits the current rooting scheme:

- Captured traced locals stay in the caller's stack slots. If the call
  that receives the closure `uses gc`, those locals are in scope at that
  call, so `push_frame` already registers their slots. The closure reads
  the same slots through the environment, so it sees updates. That also
  holds for a future moving collector, which would update the slots.
- The closure body is a function like any other: its own traced locals
  get frames at its own `gc` calls. Its parameters are rooted the same way
  as parameters of named functions.
- By-value capture of a `Gc`/`Value`, used only for vm state variables,
  needs a root for the copy in the environment. State values are already
  published by the vm's `sync` block before an effectful call, so a copy
  that is only read would be covered when the copy happens at the call.
  The simplest rule is to forbid by-value capture of traced state, and
  capture traced state by copying it into a local first.

### Cost

Parser: closure literals. Checker: the `block fn` type, the escape rule,
and closure bodies checked as nested functions with a capture list.
Codegen: environment structs and pair values. About the size of the cold
outlining code. Section 0 comes first.

## 2. Static traits (interfaces)

### What the users need

- `HashMap<K, V>` and `Interner` need `K: hash + eq`.
- `Pratt` needs a set of hooks over the parser state. Today that is five
  function pointers in a struct. With traits, `Pratt<P: PrattHooks<K, E, X>>`
  calls `p.peek()`, `p.atom()`, ... as direct, inlinable calls. No function
  pointers, no stored table of hooks, and `before_rhs` becomes a method with
  a default.
- Sorting needs `T: Ord`. Formatting or debug printing needs `T: Show`.
- A generic numeric helper needs `T: Int`, which can be provided for the
  primitive types.

All of these are static: the type is known at instantiation. Nobody needs
`dyn`, since interpreters use `value` types and tagged enums for runtime
polymorphism.

### Proposed form

```loimi
trait Hash {
    fn hash(k: *Self) -> u64;
    fn eq(a: *Self, b: *Self) -> bool;
}

impl Hash for Sym;            // checks that fn Sym.hash and fn Sym.eq exist with these signatures

struct HashMap<K: Hash, V> { .. }
fn HashMap<K: Hash, V>.get(m: *HashMap<K, V>, key: K) -> ?V { .. key.hash() .. }
```

- A trait lists method signatures over `Self`, and may give default bodies.
- `impl Trait for Type;` **does not contain the methods**. It asserts that
  the ordinary methods (`fn Type.name`) exist with matching signatures, and
  fills in defaults for the missing ones. Methods stay where they are today,
  so existing code keeps working and a type can satisfy a trait it was not
  written for (as long as the `impl` is in the type's or the trait's
  module).
- Bounds `T: A + B` on type parameters of structs, enums, functions and
  methods are **checked at instantiation**, against the `impl` list, with
  the error at the use site: "`Sym` does not implement `Hash` (needed by
  `HashMap<Sym, i64>`)".
- Inside a generic body, calls on a bounded `T` are resolved exactly as
  today, per instance. A later step could type-check generic bodies once,
  against the bounds, instead of per instance; that would give errors in
  the generic code itself. It is not needed for the first version.
- Primitive impls are written in `std` (`impl Hash for i64;` next to the
  existing `fn i64.hash` in `std/hash.lm`).

### Monomorphization

No change to the model. Traits add a check (bounds against impls) before
`instantiate_type`/`instantiate_fn`, and default method bodies are
instantiated like generic methods with `Self` bound to the instance type.
Calls stay direct, so they are inlinable and cost nothing at run time.
Method-level type parameters (`fn Cursor.op<K>`), which now exist, are
needed for trait methods that are generic themselves.

### `vm` dispatch modes

No interaction: trait method calls are ordinary direct calls. Effects are
part of trait method signatures (`fn hash(k: *Self) -> u64 uses gc` would
be unusual but legal). An impl's method may have *fewer* effects than the
trait declares, but not more, so a handler calling `k.hash()` through a
bound is checked against the trait's declaration and stays sound for every
instance.

### GC shadow stack

None beyond effects: rooting is decided per call from the callee's effects,
and the callee is known after instantiation, as today.

### Cost

Parser: `trait`, `impl .. for ..;`, bounds in generic parameter lists.
Checker: a trait table, an impl table, bound checks in the instantiation
paths, defaults. No codegen changes.

## Step 1 scope (implemented)

Section 0 does not change this design: trait method signatures carry
effects like any function type, an impl's method may have fewer, and
calls stay direct, so rooting is decided per call as before. The first
version is deliberately smaller than section 2:

- `trait Name { fn m(a: Self, ..) -> R uses E; .. }`: signatures only,
  over `Self`. No default bodies, no generic traits (`trait Hooks<K>`), no
  generic trait methods, no associated types or constants. `pub trait`
  exports it like other items.
- `impl Trait for Type;` for a concrete, non-generic type (a struct,
  enum, value type or primitive such as `i64` or `str`). It is checked
  after all signatures are known: for every trait method, `fn Type.m`
  must exist with the same parameter and result types (with `Self` =
  `Type`), and effects that are a subset of the trait's. Duplicate impls
  are an error. There is no orphan rule yet.
- Bounds `T: A + B` (trait names may be module-qualified, `hash.Hash`)
  on the type parameters of generic structs, enums, functions and methods
  (receiver and method parameters). They are checked when a concrete
  instance is created, with the error at the instantiating type or call:
  "`Sym` does not implement `Hash` (needed by `HashMap<Sym, i64>`)".
  Instances that still contain type parameters (inside generic
  signatures) are not checked; their concrete instances are.
- Generic bodies are still checked per instance; method calls on a
  bounded `T` resolve as today. No `dyn`, no codegen changes.
- `std.hash` gets `trait Hash { hash, eq }` with impls for the integer
  types and `str`, and `std.map.HashMap<K: hash.Hash, V>`.

Left for later: defaults, generic impls (`impl Hash for Vec<T>`),
checking generic bodies against their bounds, `Pratt` as a trait.

## Recommendation

1. Effects on function types (section 0). This is a soundness fix, and both
   other features build on it.
2. Static traits with checked bounds and no `dyn`. They are cheap (checker
   only), fix the worst error messages (generic containers), and give
   `Pratt` an inlinable, pointer-free hook interface, which matters for
   single-pass compilers that parse megabytes of source.
3. Non-escaping `block fn` closures for comparators and visitors, with
   captures by reference. Revisit escaping closures only together with a
   story for environments in arenas or the GC heap.

## Step 2 (implemented)

Done 2026-11-02: the rest of section 2, and section 1 with the vm part
cut down. Language-level description in `language.md` §1.2 ("Traits",
"Closures"); tests in `tests/run/traits_full.lm`, `closures.lm`,
`gc_closures.lm`, `sort_by.lm`, `std_parse_hooks.lm` and
`tests/fail/{trait,closure,block}_*.lm`.

### Traits

- **Default methods** are trait methods with a body. Every trait method
  is registered as a generic function over `[trait params.., Self,
  method params..]` with `Self` bounded by the trait. Its signature is
  what a call through a bound checks against; its body, if any, is the
  default, instantiated like any generic function with `Self` = the
  implementing type. Method lookup on a concrete type tries the type's
  own methods first and falls back to a default of a trait the type
  implements (two traits providing the same default is an ambiguity
  error).
- **Generic traits** (`trait Hooks<K, E, X>`) and **trait arguments in
  bounds and impls** (`P: Hooks<K, E, X>`, `impl Conv<i64> for Sym;`).
  The impl table is keyed by (trait, arguments, type).
- **Generic trait methods** (`fn fold<U>(..)`): the impl check
  instantiates both the trait's signature and the type's method with
  fresh opaque types for `U` (carrying the trait method's bounds), so the
  method must be generic with the same number of parameters and match.
- **Generic impls** `impl<T: Hash> Hash for Vec<T>;`: the type and trait
  arguments are patterns over the impl's parameters; `implements(ty,
  trait, args)` unifies them with the instance and checks the impl's
  bounds on the bindings, recursively. The impl's methods are checked
  once, with its parameters as opaque types that have the impl's bounds.
- **Implied bounds**: a method of a generic type has the type's bounds
  on its receiver parameters (`fn HashMap<K, V>.probe` can call
  `key.hash()`), as `std.map` already assumed.
- **Generic bodies are checked against their bounds** ("later step" in
  section 2). The body of a generic function is still checked per
  instance (that is where its code comes from), but before an instance
  is checked, the body is checked once more with each *bounded*
  parameter replaced by `Ty::Opaque(i)`, a type that only has the
  methods of its bounds: method calls on it resolve to the trait
  method's signature (a pattern instance of the generic function above,
  which is never emitted), and fields, operators or coercions to a
  concrete type fail. Unbounded parameters keep their concrete argument
  in that pass, so they keep duck typing. The pass runs once per
  combination of the unbounded parameters' arguments, and its output is
  discarded (closures created in it get no body). Errors point into the
  generic code, with an "instantiated here" note.

  The alternative, checking a generic body once with every parameter
  opaque, would have broken generic code that relies on duck typing for
  unbounded parameters; with per-instance code generation still in
  place, the extra pass is the smallest change that gives bounded
  parameters real checking.
- **Static trait functions**: `T.zero()` on a type parameter resolves
  through the bound (type paths now see the type parameters in scope).
- Not done: `dyn`, associated types and constants, supertraits, an
  orphan rule or an overlap check between impls (the first matching
  impl wins), and checking default bodies at the trait itself (they are
  checked when an instance uses them).

### Closures

As proposed in section 1, with these choices and differences:

- **Escape rule.** `block fn` types are rejected anywhere but at the top
  of a parameter type (of a function, a function type or another
  `block fn`), and as type arguments. A `block fn` local can only be a
  callee or an argument for a `block fn` parameter (a flag on the name
  lookup, set only in those two positions), and a closure literal only
  such an argument. Everything else (storing, returning, `&f`, casts,
  passing it as a `T`) is an error.
- **Checking.** The body is checked with the enclosing `FnCx`, with the
  return type, loops and effects of the closure: `return` and `?` leave
  the closure, `break` cannot reach an outer loop, and calls need effects
  the `block fn` type allows. The closure becomes its own `FnDef`, which
  shares the enclosing function's local table (so no renumbering); the
  captured locals are the ones it uses that were declared before it
  (`tirwalk::locals`, which also sees nested closures' captures). In a
  generic call, closure arguments are checked last against the partly
  known parameter type; their result type may come from the body.
- **Lowering.** The proposal's `{ code, env }` pair, with two changes
  found by measuring (`bench/sort`, `bench/README.md`):
  - It is passed as **two scalar arguments**, not one aggregate. A
    closure literal's code pointer is a constant but its environment is
    a stack address; LLVM's function specialization clones a callee only
    for constant scalar arguments, so with the aggregate a sort called
    from several sites kept an indirect comparator call (1.5x the
    hand-written sort). Split, `mem.sort_by` with a closure runs as fast
    as the hand-written sort without `#[inline(always)]`.
  - **Small immutable captures are copied** into their environment entry
    (a `let` of at most 8 bytes that holds no GC reference): one load per
    use instead of two. A `let` cannot change while the closure lives,
    so the copy is never stale. With both, a closure call that stays
    indirect costs the same as a function pointer call.

  Full specialization per closure (each literal its own type, callee
  instantiated per closure) was not needed: LLVM already inlines or
  specializes the common case, and the indirect case is as cheap as a
  function pointer.
- **Plain functions** convert to a `block fn` through one trampoline per
  signature, `(env, args..) -> env(args..)`, with the function pointer as
  the environment; when the call stays indirect, that is one extra jump.
- **GC** as proposed: captured traced locals stay in the enclosing
  frame's slots and are captured by reference (never copied), so the
  shadow-stack frame of the call that received the closure roots them
  and the closure sees their current values. A call through a `block fn`
  roots the caller's traced locals when the type `uses gc`, like any
  call. `tests/run/gc_closures.lm` runs under GC stress, and fails if the
  calls through `block fn` are not rooted.
- **vm code: closures are not allowed** in op handlers, vm functions,
  entries or `on raise`, and vm functions cannot take `block fn`
  parameters. The proposal's capture rules for state variables (by
  value, read-only) and the cold-outlining interplay were not built:
  no interpreter in the repository needed them, and handlers can call a
  plain function that creates the closure, or pass a named function
  (which converts to a `block fn`). So `musttail`, cold outlining and
  state variables are unaffected. Lifting this needs the by-value
  capture of state variables described above.
- **C functions** cannot take `block fn` parameters.

### Standard library

- `std.mem`: `Vec<T>.sort_by(cmp: block fn(*T, *T) -> i32)` and
  `mem.sort_by(p, n, cmp)`, a stable merge sort.
- `std.parse`: `Parser<P: Hooks<K, E, X>, K, E, X>`, `Pratt` with its
  hooks as a trait (`before_rhs` and `led` have defaults; `led` is one
  method dispatching on the operator kind instead of a table of function
  pointers). `Pratt` stays for its existing users.
- Not converted: `std.obj.index_keys_first` and `Model`'s `new_slots`
  still take function pointers (the latter is stored, so it is a trait
  candidate rather than a closure), and `HashMap` has no `each`.
