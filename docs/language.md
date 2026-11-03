# The Loimi language

Design draft. Syntax is provisional; the semantics and the reasons behind them
are the point. Code samples are illustrative. For a gentle introduction
that uses only what is implemented, see the [tutorial](tutorial/README.md).

Features described here that do not exist are marked **Not built:**
where they appear; [`status.md`](status.md) lists every difference
between this design and the compiler.

## 0. Principles

1. **The hot loop is the product.** A feature earns its place by improving the
   machine code of interpreter handlers, or by removing a class of bugs
   specific to interpreters. General-purpose niceties come second.
2. **One source, many dispatch strategies.** Tail-call threading, computed
   goto, and `switch` are build options, not rewrites.
3. **Declare, don't encode.** Instruction formats, value representations,
   object layouts, and inline caches are declarations. The compiler generates
   the bit manipulation and also the tools that must agree with it:
   encoders, disassembler, verifier, peephole and deoptimization passes.
   (A tracer is an instrumentation `variant`, section 9.)
4. **Make the slow path explicit, cheap, and correct.** Slow paths are
   outlined so they don't hurt register allocation. Effects ensure the runtime
   always sees consistent VM state when it needs to.
5. **Safe by default, unchecked by proof.** Bounds checks are removed because
   a verifier proved them redundant, not because someone wrote `unsafe`.
6. **Stock LLVM only.** Every feature lowers to standard LLVM IR, attributes,
   and calling conventions, so the language benefits from LLVM's optimizer and
   targets for free.

## 1. Core language

The core is a small, conventional, statically typed systems language with
roughly C's level of abstraction. It has deliberately few novel features.

### 1.1 Types

```loimi
i8 i16 i32 i64 isize   u8 u16 u32 u64 usize   f32 f64   bool   never
iN uN           // any width 1..=64: i48, u24, i61, ... (see below)
[T; N]          // fixed array
[]T             // slice (ptr + len), bounds-checked; [] is the empty one
*T, *mut T      // raw pointers; ?*T (None = null) for one that may be absent
?T              // option; ?*T is pointer-sized
Result<T, E>
Gc<T>           // traced reference into the GC heap (section 8)
fn(A, B) -> R   // function pointer; `fn(A) -> R uses gc` has effects (section 5)
block fn(A) -> R  // non-escaping closure; only as a parameter type (section 1.2)
closure fn(A) -> R      // escaping closure { code, env }, env from an allocator (section 1.2)
Gc<closure fn(A) -> R>  // escaping closure in the GC heap
```

Plus `struct`, `enum` (tagged unions), `bits` (packed bitfield integers),
`object` (GC-managed structs, section 8), and `value` (section 2).
**Not built:** `bits` (use shifts and masks, or a vm's generated field
readers, section 3.3).

### 1.2 Basics

```loimi
fn gcd(a: u64, b: u64) -> u64 {
    var (x, y) = (a, b);
    while y != 0 { (x, y) = (y, x % y); }
    x
}

struct Span { start: u32, end: u32 }

enum Token { Num(f64), Ident(Symbol), Punct(u8), Eof }

bits Insn: u32 { op: 8, a: 8, b: 8, c: 8 }   // insn.a reads bits 8..16
```

- `let` declares an immutable binding and `var` a mutable one. Blocks are expressions.
- Module-level `var NAME: T = init;` declares a global. `init` must be a
  constant expression (literals, consts, `[x; N]`, aggregate and enum
  literals of constants, functions). `#[thread_local] var` gives each thread
  its own copy, which is what an interpreter wants for "current VM" or
  per-thread caches. `pub var` is visible to other modules. Globals are not
  GC roots.
- **Integer-to-enum casts.** `x as Kind` converts an integer to an enum
  without payloads (decoding an opcode byte, a token kind from a table). The
  value must equal one of the discriminants. Like overflow, this is checked
  in debug builds (a panic naming the cast) and unchecked in release builds,
  where an invalid value is undefined behavior for any `match` on the
  result. A constant operand is checked at compile time. `Kind.COUNT` is the
  number of variants, typed like an integer literal (`[u32; Kind.COUNT]`,
  `for i in 0..Kind.COUNT { let k = i as Kind; }`); a variant named `COUNT`
  takes precedence. With explicit discriminants the count is not the
  largest discriminant plus one.
- **Local inference across branches.** Without an annotation, the branches
  of `if` and `match` are typed against each other: `let r = if c { Ok(e) }
  else { Err(m) };` gets `Result<typeof e, typeof m>`, and an arm that cannot
  be typed alone (`None`, `Ok(x)`) takes its type from the others. Inference
  is local: it does not look at later uses of `r`, so `var r = Ok(false);`
  needs an annotation (`var r: Result<bool, str> = Ok(false);`), and the
  error says so.
- **Reserved words**: `fn let var if else while loop for in return break
  continue struct enum const import extern export match as true false and
  or pub defer unsafe never`. They cannot be names (`in` included: write
  `in_` or `input`). The control words (`next`, `jump`, ..., section 3.4)
  and words like `vm`, `op`, `state` are not reserved.
- **Range loops.** `for i in lo..hi` counts from `lo` up to `hi - 1`. The
  range takes the type of a bound that is not a literal: `0..n` with
  `n: u64` is a `u64` range, `m..255` with `m: u8` a `u8` range. When both
  bounds are literals (or `Kind.COUNT`), the type comes from an annotation
  on the variable, `for i: u8 in 0..10`, and is `i64` without one; the
  bounds must fit it. (The loop variable's later uses are not looked at.)
- **Element loops.** `for x in e`, with `e` an array or a slice, runs
  the body with `x` bound to a copy of each element in order (`break`
  and `continue` work): `for row in table() { let (op, k) = row; .. }`.
  An array that is a place (a variable, field, `p.xs`) is read in
  place; an array-valued call is stored in a temporary first. The loop
  variable is a name, not a pattern.
- **`<` after `as`.** `x as u64 < y` is a comparison: a primitive type never
  takes type arguments. After any other type name, `<` begins type
  arguments only when a balanced `<...>` made of type tokens follows
  (`p as *Vec<T>`); otherwise it is a comparison. When in doubt, write
  `(x as T) < y`.
- **Type aliases.** `type Name = T;` and `pub type Name = T;` give a type a
  second name; `type Map<V> = HashMap<str, V>;` takes type parameters. An
  alias is transparent: `Name` *is* `T`, so values convert freely, methods
  of `T` apply, and `fn Name.m(..)` defines a method of `T`. The aliased
  type is resolved in the alias's module and sees only the alias's own
  parameters; aliases may refer to other aliases, in any order, but not
  to themselves. A non-generic alias can also stand for its type in
  expressions: `Name { .. }` (struct literals), `Name.f(..)` (static
  calls), `Name.Variant` (also in patterns). Generic aliases are for type
  positions only (`Map.new()` is not available; write `HashMap.new()`),
  and their parameters take no bounds: the bounds of the aliased type
  are checked where it is instantiated. Aliases are imported like other
  items (`import m.{Name}`, `m.Name`).
- `match` is exhaustive. `if let`, `let ... else`, and `while let` are supported.
- **Integer patterns** are literals (`0x45`, `-1`, `'a'`) and inclusive
  ranges (`0x28..=0x3E`, `'a'..='z'`); a pattern outside the
  scrutinee's type is an error. Integers count as covered only by `_`
  or a binding.
- **Constants in patterns.** A name in a pattern is, in this order: a
  variant of the type matched (`Nil`, `Red`), a constant (a `const` of
  this module or imported with `import m.{LIMIT}`, also written
  `m.LIMIT`, and a vm's generated constants such as `Vm.OP_ADD`), else
  a new binding. A constant matches its value, as if its literal stood
  there: `match loc { LOCAL => .., CONST => .., _ => .. }`. Integer,
  `bool`, `str`, enum and tuple constants qualify; float constants are
  an error (there are no float patterns: NaN and `-0.0` make equality
  the wrong test), and so is a constant whose type is not the
  scrutinee's. A local variable shadows a constant of the same name
  (the pattern binds again), and `var X` always binds. Arms after one
  that matches every value (a binding or `_`, no guard) can never run
  and draw the warning `unreachable_arm`; when that arm is a binding,
  the note says so, which catches a misspelled or unimported constant
  (`LOCAL =>` without `const LOCAL` in scope binds `LOCAL` to anything).
- **String patterns.** On a `str`, `match` arms may be string literals:
  `match name { "dup" => .., "swap" | "exch" => .., _ => .. }`. As with
  integers, only `_` or a binding makes such a match exhaustive. A run
  of string arms at the start of a `match` compiles to a `switch` on the
  length and then a compare of the bytes in 8-, 4-, 2- and 1-byte loads
  against constants; elsewhere (inside tuples, after a guarded arm, in
  `if let`) each pattern tests the length and the bytes in turn.
- **`..zero` in struct literals.** `Machine { heap: gc.Heap.new(), depth: 3, ..zero }`
  sets every field not given to zero. It is allowed only where zero is a
  value that needs no constructor: integers, floats, `bool`, `?T` (None,
  so `?*T`, `?Gc<T>` and `?fn(..)` too), slices (empty), raw pointers
  `*T` (null) and arrays and tuples of those. A field of type `Gc<T>` or
  `fn(..)`, a struct (its constructor may set invariants), an enum or a
  `value` type must be given, and so must private (`__`) fields; the
  error names the field (and suggests `?Gc<T>`/`?fn` for a reference).
  It also works in `const` and global initializers.

  The rule for pointers follows what the language already lets you
  write without `unsafe`. A null `*T` is what `0 as *T` makes: a
  placeholder for a field set before use (a vm's `stack_end`, `fp` or
  `words`, filled in once the stack is allocated or the program
  compiled), and dereferencing it crashes as in C. `?*T` remains the
  type of a pointer that is *meant* to be absent, with the check at each
  use. A null `Gc<T>` needs `unsafe`, and the collector and every reader
  assume it refers to an object; a null function pointer would be a call
  to address 0. Those stay required, and the field type that starts
  empty is the option.
- **Empty slices.** `[]` where a slice type is expected is the empty
  slice: `let s: []u32 = [];`, `f([])`, `x.words = [];`.
- **Match guards.** `pat if cond => body` matches only when `cond` (which
  sees the pattern's bindings) is true; otherwise the later arms are
  tried. Guarded arms do not count toward exhaustiveness. Note the
  difference from an arm whose *body* is an `if`:

  ```loimi
  match (a, b) {
      (Int(x), Int(y)) if y != 0 => { dst = Int(x / y); next }   // y == 0: tries the next arm
      (Int(x), Int(y)) => if y != 0 { dst = Int(x / y); next },  // y == 0: continues after the match
      (Int(x), _) => { .. }
      _ => {}
  }
  ```

  Once a pattern matches, its arm is chosen; a false `if` inside it
  falls out of the whole `match`, as in Rust. In handlers, where the code
  after a `match` is usually the slow path, that silently skips a later
  arm meant for the case. The compiler warns when, in vm code (op
  handlers, `-> never` vm functions, entries, `on raise`), an arm's body
  is an `if` without `else` that ends in a control word and a later arm
  with a non-empty body could match some of the same values. Use a guard
  to try the later arms, or write `else {}` to say that falling out of the
  `match` is intended. Arms followed only by `_ => {}` do not warn, since
  falling out of the `match` is then the same as trying the rest.
- **Warnings** are named (`warning[match_fallout]: ..`). `#[allow(name)]`
  silences one on a match arm, an item (function, `vm`, ...) or a vm item
  (`op`, `family`, vm function, entry), covering everything inside it;
  `#![allow(name)]` at the top of a file covers the file. An unknown
  name is an error. `loimi build -W error` (or `--deny-warnings`) turns
  the warnings that remain into errors. The lints are `match_fallout`,
  `unreachable_arm` (above) and `super_overlap` (section 7.3), and the
  opt-in `hot_gc`, turned on with `-W hot_gc`: a call that may collect
  (`uses gc`) on an op handler's fast path, outside `cold`, which syncs
  the vm state and roots the live traced locals on every execution
  (often right, as for an allocation the op exists to make: `#[allow(hot_gc)]`).
  The retired `state_root` (section 8: traced state is now rooted
  automatically) is still accepted by `#[allow(..)]`, with no effect.
- **Attributes** are checked where they appear: an unknown or misplaced
  one is an error (with a "did you mean" for near misses), never
  ignored, since a misspelled `#[require_verified]` would silently drop
  a check. `#[allow(..)]` goes anywhere (items, vm items, fields, enum
  variants, match arms, `#![..]`); functions (also extern, trait and vm
  functions) take `#[inline]`, `#[inline(always|never)]`, `#[cold]` and
  `#[unsafe]`; a `vm` takes `#[require_verified]` and
  `#[dispatch(mode)]`; a module-level `var` takes `#[thread_local]`; a
  `super` takes `#[forward]`.
- **Tests.** `#[test]` marks a function `fn name()` (no parameters or
  result; it may declare effects) as a unit test. In a normal build it
  is an ordinary function. `loimi test FILE.lm [build options]
  [FILTER..] [--list] [--update]` builds FILE.lm with its `main`
  replaced by one that runs FILE.lm's `#[test]` functions (not those of
  the modules it imports) through `std.test`'s `Runner`, and runs it in
  FILE.lm's directory: each test under a panic catch, so a failure stops
  only that test; the exit status is 1 if any failed. Filters select the
  tests whose name contains one of them; `--update` makes
  `test.golden` rewrite its files (stdlib.md, `std.test`).
- **`#[track_caller]`** on a function makes each call store the call's
  location (`"file.lm:12:5"`) in a thread-local before calling;
  `test.caller()` reads it. In a `#[track_caller]` function, calls to
  others pass on its own caller's location instead, so an assertion
  helper reports the line of the test that called it. Calls of other
  functions pay nothing.
- Generics are monomorphized. Interfaces (`trait`-like) use static dispatch only.
- **Traits** state what a generic parameter needs:

  ```loimi
  pub trait Hash {
      fn hash(k: Self) -> u64;
      fn eq(a: Self, b: Self) -> bool;
  }
  impl Hash for Sym;          // checks that fn Sym.hash and fn Sym.eq exist
  pub struct HashMap<K: hash.Hash, V> { .. }
  fn show_all<T: Show + Hash>(xs: []T) { .. }
  ```

  A trait lists method signatures over `Self`. `impl Trait for Type;`
  contains no code: it asserts that the ordinary methods `fn Type.name`
  exist with the trait's signatures (with `Self` replaced by `Type`) and
  no more effects than the trait declares. Bounds `T: A + B` on the type
  parameters of structs, enums, functions and methods are checked when
  an instance with concrete types is created, and a missing impl is
  reported there: "`Key` does not implement `Hash` (needed by
  `HashMap<Key, i64>`)". A method of a generic type has the type's
  bounds on the receiver's parameters without restating them
  (`fn HashMap<K, V>.get` may call `key.hash()`).

  Traits can also have:

  ```loimi
  trait Show {
      fn show(x: Self);
      fn show_line(x: Self) { x.show(); io.println(""); }  // a default
  }
  trait Conv<T> { fn conv(x: Self) -> T; }                // a type parameter
  trait Folder {
      fn fold<U>(f: *Self, init: U, step: fn(U, i64) -> U) -> U;  // a generic method
      fn zero() -> Self;                                  // called as `F.zero()`
  }
  impl Conv<i64> for Sym;
  impl<T: Show> Show for Vec<T>;      // every Vec<T> whose T is Show
  fn total<T: Conv<i64>>(xs: []T) -> i64 { .. xs[i].conv() .. }
  ```

  - **Default methods** have a body. An impl takes the type's own
    method if there is one (checked against the signature) and the
    default otherwise; the default is instantiated for the type, so
    `sym.show_line()` and `x.show_line()` on a bounded `x` both work.
  - **Generic traits** take type arguments in bounds and impls
    (`T: Conv<i64>`, `P: parse.Hooks<K, E, X>`, where the arguments may
    name other parameters). A type may implement one trait with
    different arguments.
  - **Generic trait methods** have their own type parameters; the type's
    method must have as many (`fn Range.fold<U>`), with the same
    signature.
  - **Generic impls** `impl<T: B> Trait for Type<T>;` cover every
    instance whose arguments satisfy the impl's bounds, checked when an
    instance is needed (`Vec<f64>` is not `Show` if `f64` is not). The
    methods are the generic type's (`fn Vec<T>.show`). Each parameter of
    the impl must appear in the type.
  - **Static functions** of a trait (no `Self` parameter needed) are
    called on the type: `T.zero()` for a bounded `T`.

  **Generic bodies are checked against their bounds.** Inside
  `fn f<T: Show>(x: T)`, `T` has only what its bounds give it: `x.show()`
  and the defaults of `Show`, and calls to functions whose bounds `T`
  meets. `x.extra()` is an error ("no method `extra` on type parameter
  `T`: its bounds (`Show`) do not provide it") even if every type `f` is
  used with has an `extra` method; so are its fields, operators and
  passing it where a concrete type is expected. A call through a bound
  has the effects the trait declares. The check runs once per
  combination of the arguments of the *unbounded* parameters: those keep
  the old duck typing (method calls on them resolve per instance), so
  generic code written before traits keeps compiling. Code is still
  generated per instance, and calls are direct (static dispatch, no
  `dyn`).
  Type arguments are inferred from arguments and the expected type, or given
  explicitly: `mem.alloc<u8>(n)`. Methods can have their own type parameters:
  `fn Cursor.op<K>(c: *Cursor, ops: *OpTable<K>) -> ?K`,
  `fn Vec<T>.map<U>(v: *Vec<T>, f: fn(T) -> U) -> Vec<U>`.
- **Variants of generic enums** can be qualified with the enum's name
  alone: with `enum Lookup<O> { Own(u32), Proto(O, u32), Missing }`,
  `Lookup.Missing` and `Lookup.Own(i)` take their instance from the
  expected type (`return Lookup.Missing;` in a function returning
  `Lookup<O>`), or else from the payload (`Lookup.Proto(2.5, 7)` is a
  `Lookup<f64>`); `let l = Lookup.Missing;` is an error. In patterns,
  `Lookup.Own(x)` matches any instance. `mod.Lookup.Missing` works too.
- **Generic functions as values.** A generic function used where a
  function type is expected gets its type arguments from that type
  (`Pratt.new(peek, ..)`). Explicit type arguments name an instance
  anywhere: `heap.add_scanner(&roots as *u8, scan<Gc<Box>>)`,
  `let f = size_of_t<u32>;` (all of the function's type arguments; not on
  a generic type's functions). `f<T>` without a call is read as type
  arguments when the `<` directly follows the name and the `>` is followed
  by `)`, `,`, `;`, `}` or `]`. When a generic function is passed to
  another generic function, the parameter's type may be only partly
  known (`fn apply<T, U>(f: fn(T) -> U, x: T) -> U`): the other arguments
  are checked first, and then the known parts (here `T`) are enough if
  they determine the function's type arguments, as in `apply(double, 21)`
  with `fn double<N>(x: N) -> N`.
- **Closures** (`block fn`, non-escaping):

  ```loimi
  fn count_if(xs: []i64, pred: block fn(i64) -> bool) -> i64 { .. pred(xs[i]) .. }
  fn Vec<T>.sort_by(v: *Vec<T>, cmp: block fn(*T, *T) -> i32) { .. }

  var hits = 0;
  let n = count_if(xs, |x| x > limit);       // captures `limit`
  each(v, |x| hits += x);                     // writes `hits`
  ids.sort_by(|a, b| names.at(*a).cmp(names.at(*b)));
  map(xs, |x: i64| -> str { if x > 5 { return "big"; } "small" });
  ```

  `block fn(A, B) -> R uses E` is the type of a closure parameter. A
  closure literal `|a, b| expr` (parameter types from the parameter,
  or written `|a: T|`; `|| expr` takes none; `|a| -> R { .. }` gives the
  result type) can only be an argument for such a parameter, and a
  `block fn` parameter can only be called or passed on as such an
  argument (also to a closure that calls it). It cannot be stored in a
  local, a field or a global, returned, cast, or be a type argument;
  `block fn` types are only allowed at the top of a parameter type. So a
  closure never outlives the call that receives it, which is what makes
  capturing by reference safe without lifetime analysis:

  - **Captures** are by reference: the closure sees the enclosing
    locals' current values and its writes are visible after the call.
    `let` locals stay read-only. (Small untraced `let`s are copied into
    the closure's environment, which nobody can observe.)
  - The body is a function of its own: `return` and `?` leave the
    closure, and `break`/`continue` cannot reach loops outside it.
  - **Effects**: the closure's body may only make calls whose effects
    the `block fn` type allows (`block fn(i64) uses gc` may collect), and
    calling a `block fn` has its type's effects, with the usual rules.
  - A plain function (or `fn` value) converts to a `block fn` with the
    same signature and no more effects: `v.sort_by(cmp_i64)` works.
  - In a generic call, closure arguments are checked last, so their
    parameter types can come from the other arguments, and their result
    type from the body (`map(xs, |x| x as f64 / 2.0)` infers `U = f64`).
  - Not in vm code: op handlers, vm functions, entries and `on raise`
    cannot contain closures (a closure is a separate function, so it
    could not use control words, and capturing state variables by
    reference would force them into memory), and vm functions cannot
    take `block fn` parameters. Call a plain function that makes the
    closure, or pass a named function. C functions cannot take them
    either.
  - **GC**: a captured traced local stays in the enclosing frame; the
    shadow-stack frame of the call that received the closure roots it.
    The callee roots its own traced locals at each call of the closure
    whose type `uses gc`, and the closure body roots its own.

  Lowering: a `block fn` is a code pointer and an environment pointer,
  passed as two arguments; the environment is an array, in the enclosing
  frame, of pointers to the captured locals (or their values). A call is
  indirect unless LLVM inlines or specializes the callee, which it does
  for a sort called with a closure literal: then the closure costs
  nothing (see `bench/README.md`).
- **Escaping closures** (`closure fn`, `Gc<closure fn>`; built): closures
  that can be stored, returned and kept in collections.

  ```loimi
  struct Builtin { name: str, call: closure fn(i64) -> i64 }

  fn adder(a: *mem.Arena, k: i64) -> closure fn(i64) -> i64 {
      new(a, |x| x + k)                       // environment in the arena
  }
  table.push(Builtin { name: "add7", call: adder(&arena, 7) });
  table.push(Builtin { name: "double", call: double });   // a plain fn
  v.sort_by(cfg.less);                        // closure fn -> block fn

  object Hooks { on_value: ?Gc<closure fn(Value) uses gc> }
  hooks.on_value = Some(new(heap, |v| log(out, v, tag)));  // in the GC heap
  ```

  - **Types.** `closure fn(A) -> R uses E` is a value of 16 bytes,
    `{ code, env }`, that can go anywhere a value can (fields, locals,
    globals, `Vec<..>`, `?..`, results, type arguments). Its environment
    is host memory and is **not traced**. `Gc<closure fn(A) -> R uses E>`
    is one traced word, a reference to a closure object `{ code,
    captures.. }` in the GC heap, rooted, traced, write-barriered and
    freed like any `Gc`. Plain `fn` types stay thin code pointers and
    `block fn` stays as it was: neither changes representation.
  - **Creation is an explicit allocation**: `new(a, |..| body)`. With
    `a: *gc.Heap` the result is a `Gc<closure fn>`; with any other `a`,
    the environment is one call of `a.alloc_bytes(size: u64, align: u64)
    -> *u8` (`*mem.Arena` has it; so can your own allocator), and the
    result is a `closure fn`. A closure that captures nothing allocates
    nothing. Parameter and result types come from the expected type
    (a `let` annotation, a field, a parameter, `Vec<closure fn(..)>.push`,
    including generic ones: `map(xs, new(a, |x: i64| x * 2))` infers
    `U`), or from annotations on the literal.
  - **Captures are copies**, made when the closure is created, so the
    closure may outlive the frame. Assigning a captured variable inside
    the body (also from a nested `block fn` closure) is an error: the
    closure has its own copy, and the write would not be seen outside.
    State the closure updates lives behind a pointer it captures (a
    generator's position, a counter). Capturing a `block fn` parameter
    is an error (it only lives during its call).
  - **GC.** Capturing a traced value (`Gc<T>`, a `value` type with
    references, a struct holding one) is only allowed in
    `new(heap, ..)`: the arena environment is invisible to the collector,
    and the error says so. A GC closure's environment is an object type
    generated per closure literal, so its captures are marked through it;
    while its body runs, the closure roots its own environment. Its
    traced captures cannot have their address taken (a store through the
    pointer would miss the write barrier).
  - **Effects** work as for `fn` and `block fn`: the body may only make
    calls the type allows, calling has the type's effects (rooting and
    handler `sync` included), and a closure converts to a type with more
    effects.
  - **Conversions**: a plain function converts to `closure fn` without
    allocating (a per-signature trampoline, the function as environment),
    so tables can mix both; `closure fn` converts to `block fn` for free.
    `Gc<closure fn>` converts to neither (an untraced copy would not keep
    it alive).
  - **Freeing** belongs to the allocator: `arena.free()` frees every
    environment in it; the GC frees GC closures. Calling a closure whose
    arena was freed is a use-after-free that the compiler does not
    detect (`--sanitize=address` does).
  - **vm code** cannot create closures (as for `block fn`), but handlers
    and vm functions can call stored ones, which are synced and rooted
    per their effects like `fn` fields; vm functions may take
    `closure fn` parameters. **C**: neither type crosses into C; pass a
    plain `fn` and a data pointer, and wrap them on the Loimi side
    (`new(&arena, |st| f(st, ud))`).

  Lowering: the closure body is a function `R (ptr env, A..)`, like a
  `block fn` literal's, reading its captures from the environment
  struct; a call loads the code (from the pair, or the object's first
  word) and passes the environment first. See
  `design/closures-traits.md`, "Escaping closures".
- `defer stmt` runs `stmt` when the enclosing scope exits.
- Error handling uses `Result` and `?`. There are no host-level exceptions or unwinding.
- **Operators**, from the tightest binding to the loosest:

  | Operators | Meaning |
  |---|---|
  | `f(x)` `a[i]` `a[i..j]` `x.f` `x?` | call, index, slice, field or method, try |
  | `-x` `!x` `~x` `*p` `&x` | negate; logical not (`bool`) or bitwise not (integers); bitwise not (integers only); dereference; address |
  | `x as T` | cast (`-x as u64` is `(-x) as u64`) |
  | `*` `/` `%` `*%` | multiply, divide, remainder, wrapping multiply |
  | `+` `-` `+%` `-%` | add, subtract, wrapping forms |
  | `<<` `>>` | shifts |
  | `&` | bitwise and |
  | `^` | bitwise xor |
  | `\|` | bitwise or |
  | `==` `!=` `<` `<=` `>` `>=` | comparisons |
  | `and` (or `&&`) | logical and, short-circuit |
  | `or` (or `\|\|`) | logical or, short-circuit |
  | `a ?? b` | option default, right-associative: `a`'s value if `Some`, else `b` (evaluated only then); with `b: ?T` the result is `?T` |
  | `=` `+=` `-=` `*=` `/=` `%=` `&=` `\|=` `^=` `<<=` `>>=` `+%=` `-%=` `*%=` | assignment (a statement) |

  Binary operators are left-associative. Unlike C, `&`, `^` and `|` bind
  tighter than the comparisons: `x & 1 == 0` is `(x & 1) == 0`. Both
  operands of a binary operator have the same type (shifts excepted, see
  below); there are no implicit conversions. `!x` and `~x` are the same
  on an integer; `~` exists so that bitwise code reads as it does in C,
  and rejects a `bool`. There is no `++`/`--`, `**`, `?:`, `===` or
  unary `+`; the parser says what to write instead.
- **Integer arithmetic:** `+ - *` trap on overflow in debug builds and wrap in
  release builds. Explicit forms: `a +% b`, `a -% b`, `a *% b` (wrapping),
  `a.add_checked(b) -> ?T` (and `sub_checked`, `mul_checked`). These map
  directly to LLVM's `*.with.overflow` intrinsics, which guest integer
  semantics need constantly. **Not built:** `a.add_overflow(b) -> (T, bool)`
  as a method; `std.num.add_overflow(a, b)` is a function over `i64`.
  Other built-in methods: `clz`, `ctz`, `popcount` on integers; `sqrt`,
  `floor`, `to_bits` and `f64.from_bits(u)` on floats (the list is in
  `stdlib.md`, `std.core`).
  In constant expressions (`const`s, array lengths, global initializers),
  overflow of `+ - *`, unary `-` and `/` is a compile error in all builds.
- **Shifts.** `a << n` and `a >> n` have the type of `a`; `>>` is
  arithmetic for signed `a` and logical for unsigned. The amount `n` may
  be any integer type (`x << shift` with `x: u32`, `shift: u64`), and is
  taken modulo the width of `a`, read as an unsigned number of its own
  type, so shifting never has undefined results. A literal `a` takes its
  type from the context like other literals (`w & (255 << shift)` with
  `w: u32` shifts a `u32`) and is an `i64` without one.
- **Arbitrary-width integers.** `iN` and `uN` for any `N` from 1 to 64 are
  integer types of exactly that width (LLVM's `iN`): arithmetic wraps,
  overflows and is checked at `N` bits, `as` sign- or zero-extends from `N`
  bits, and shift amounts are taken modulo `N`. Their main use is `value`
  payloads (`Int(i48)` in a nanbox, `Int(i61)` in a lowtag), where they
  record exactly how many bits the encoding keeps. Overflow-checked
  `+ - *` on an odd width are done in 64 bits with the operands shifted to
  the top, so `x.add_checked(y)` on `i48` costs two shifts more than on
  `i64` and no compares. In memory they take the next power-of-two size
  (`i48`: 8 bytes, `u24`: 4).
- Hints: `likely(c)`, `unlikely(c)`, `assume(c)`, `unreachable`, `#[cold]`,
  `#[inline(always)]`, `#[inline(never)]`. `trap()` aborts (there is no
  `assert`); `size_of<T>()` is a type's size in bytes.
- `unsafe { }` marks code whose correctness the compiler cannot check.
  As implemented, exactly one operation requires it: making a GC
  reference from something else (`p as Gc<T>` from a pointer, an address
  or another `Gc`; section 8), because the collector trusts every `Gc` it
  finds. Raw pointers do not need it (section 1.3). An `unsafe` block is
  an ordinary block otherwise, and may be used to mark other unchecked
  code for readers.

### 1.3 Memory

There is no borrow checker. Host memory is managed through explicit allocators,
mostly arenas (`std.mem`), which suit compilers and interpreters well: ASTs,
IR, and per-compilation data die together. Guest memory lives in the GC heap
(section 8). This is "safety-leaning", not memory-safe:

- **Arrays and slices are bounds-checked** (in debug and release builds),
  and integer overflow traps in debug builds.
- **Raw pointers are unchecked, and need no `unsafe`.** `*p`, `p.field`,
  `p[i]` and `p + n` on a `*T` compile to plain loads, stores and address
  arithmetic, anywhere. The pointer type is the marker: a `*T` says
  "no bounds, no lifetime", the way a slice says "checked". Interpreter
  host code (allocators, hash tables, the collector, register windows)
  is built on pointers, and the standard library and the example
  interpreters use them throughout; requiring `unsafe` around each use
  would mark most of that code without making any of it safer. Use a
  slice (`p.slice(n)`) where a bounds check is wanted.
- **Alignment.** `*p` assumes `p` is aligned for its type (LLVM may
  rely on it: vectorizing, combining accesses), as in C. For an address
  that may not be, such as a guest's memory address or a packed format,
  `p.read_unaligned()` and `p.write_unaligned(v)` load and store with
  alignment 1 (`*T` for integer, float and pointer `T`). On x86-64 and
  AArch64 they compile to the same single `mov`/`ldr` as an aligned
  access.
- **GC references cannot be forged outside `unsafe`.** A `Gc<T>` comes
  from `new`, from a field or value that holds one, or from
  `unsafe { p as Gc<T> }`. The collector marks through every `Gc` it
  finds, so a bad one corrupts the heap far from the bug; that is the one
  conversion the compiler makes you mark.
- **Bytecode is checked once, by the verifier** (section 6), and operand
  accesses in handlers are unchecked because it ran, not because of an
  annotation.

### 1.4 Compile-time evaluation

`comptime` expressions and `const fn` run in the compiler, Zig-style. This is
how tables (lexer DFAs, operator precedence tables, op variants) are built.
**Not built:** `comptime` and `const fn`; `inline for` (below) is the only
compile-time evaluation, and tables are filled at run time.
`inline for` unrolls at compile time and can generate items, including ops:

```loimi
inline for (name, cc) in [("JLT", Cond.Lt), ("JLE", Cond.Le), ("JEQ", Cond.Eq)] {
    op #name(a: reg, b: reg, target: label) : ABsC {
        if compare(cc, a, b)? { jump target } else { next }
    }
}
```

`inline for` is token-level expansion. The body is copied once per
element, and every identifier token equal to a loop variable is replaced
by that element's tokens (in parentheses when there is more than one
token, so `v * 10` with `v = 2 + 3` is `(2 + 3) * 10`). That includes
field names (`r.f`) and binders (`let v`), so pick loop variable names
that do not collide with names in the body. `#v` turns a string or
identifier value into an identifier.

It can appear at module level, among the items of a `vm`, and as a
statement in any block: function bodies, op handlers, vm functions,
entries.

```loimi
fn sum_fields(r: *Regs) -> i64 {
    var total: i64 = 0;
    inline for f in [a, b, c] { total += r.f }
    total
}
```

**Scoping.** In statement position, the copies become statements of the
enclosing block, each ending with `;`; there is no new scope. So
`let #name = ..;` declares a local of the enclosing block that later
statements (and later copies) see, a plain `let tmp` in the body is
shadowed by the next copy's `let tmp`, and `next`, `break` or `return` in
a copy act on the enclosing handler or loop. Write `{ .. }` inside the body
for a scope per copy. A spliced `#name` is an ordinary identifier at the
place it lands: it resolves, and binds, exactly as if it had been written
there. The same holds at module level and in a `vm`, where `#name`
usually names an item (`fn #name`, `op #name`).

Errors point into the original source: a token copied from the body keeps
its position in the body, and a substituted token its position in the
element list. Nested `inline for`s expand outside-in; an inner loop may
use the outer variables in its element list, but may not reuse their
names as its own variables. Inside a `vm` (also in handler and vm function
bodies), a loop variable may not have the name of a state variable: it
would replace every use of that name in the body.

### 1.5 Modules and interop

`import std.gc.{Heap, Weak};` (`Gc<T>` is built in). `extern "C" fn` declares C functions, and
`export "C"` exposes Loimi functions, which matters for embedding (§10):
in a library build they are the public API, and `--header` declares them
for C. `bool` crosses both kinds of boundary zero-extended, as C expects.

**Namespaces.** A module's names live in two namespaces:

- **types and modules**: structs, enums, `value` and `object` types,
  type aliases, traits, vms, and imported modules (`import std.parse;`
  binds `parse`). These are what type positions and the head of a path
  see: `T`, `m.T`, `T.f(..)`, `T.Variant`, `T { .. }`, `m.f(..)`;
- **values**: functions, consts and globals.

A function may share its name with a type or module, so `fn parse(..)`
can sit next to `import std.parse;`, and `struct Point` next to
`fn Point(..)`. Two types, a type and a module, two values, or a const or
global and a type or module with one name are an error ("defined more
than once"): a const or global can be the head of `x.y` (`ORIGIN.x`),
which would be ambiguous. `import m.{x}` imports whatever `m` binds as
`x` in either namespace.

How a name is resolved depends on where it is:

- a local variable always wins;
- `x(..)` and `x` in an expression name a value. If there is none, `x`
  may be a variant of the expected type (`Arr(a)` where a `Value` is
  expected, even with `object Arr` imported: a type is never a value),
  and otherwise it is an error;
- in `x.y`, `x` is looked up among types and modules first: with both
  `fn parse` and `import std.parse`, `parse.Prec` is the module's struct
  (a function has no fields, so nothing is lost);
- patterns resolve variant names against the type being matched first;
  a name that is not a variant is a constant in scope if there is one
  (section 1.2, "Constants in patterns"), else a binding.

Enum and `value` variants are not module items: bare `Arr(..)` or
`Nil` is found through the expected type (or written `Value.Arr(..)`).

---

## 2. `value` types: declared representations

Every dynamic-language interpreter has a `Value` type, and its encoding is the
most performance-critical decision in the VM. Loimi makes the encoding a
declaration:

```loimi
value Value repr nanbox {
    Num(f64) unboxed,          // any bit pattern that isn't a boxed tag
    Int(i48),                  // 48-bit integers: overflow at 2^47
    Nil,
    Bool(bool),
    Str(Gc<Str>),
    Obj(Gc<Object>),
    Fn(Gc<Closure>),
    Native(Gc<NativeFn>),
}
```

Available representations:

| `repr` | Layout | Typical user |
|---|---|---|
| `nanbox` | 64-bit word. Doubles stored directly; other variants in the NaN space with payloads of up to 48 bits | JS engines, LuaJIT |
| `lowtag` | 64-bit word. Aligned pointers with tag bits in the low bits; integers of up to 61 bits shifted | Ruby, OCaml, many Schemes |
| `boxed` | Two words `{tag, payload}`, passed and returned in two registers | PUC Lua (`TValue`) |
| `pointer` | **Not built:** everything is a `Gc<_>` with a header type tag | CPython |

The compiler chooses tag assignments, or you can pin them
(`Int(i32) = tag 0b001`; **Not built:** pinning). In return you get:

- **Constructors and patterns.** `Int(3)`, `match v { Int(x) => ... }`, and
  `if v is Nil` (**Not built:** `is`; write `if let Nil = v`). No macros.
- **Optimal match compilation**, including over tuples of values, which is the
  case interpreters care most about (binary operators). The compiler knows the
  bit-level encoding, so it can merge tests. For example, with NaN-boxing and
  an `Int` tag `T` in the high 16 bits, `(Int(x), Int(y))` compiles to
  `((a ^ T) | (b ^ T)) >> 48 == 0`, one test instead of two. `(Num, Num)`
  compiles to `umax(a, b) <= LIMIT`. See `lowering.md`.
- **GC precision for free.** The compiler knows which variants hold `Gc<_>`,
  so tracing a `Value` is generated code.
- **Canonicalization.** For `nanbox`, constructing `Num(x)` canonicalizes
  NaNs, so a guest NaN can never forge a pointer. This is a safety property
  hand-written NaN-boxing regularly gets wrong.
- **Portability.** `nanbox` requires 48-bit pointers. On targets where that
  doesn't hold, or in wasm32, the compiler can fall back to `boxed` with a
  warning, and the source does not change. **Not built:** the fallback
  (only x86-64 and AArch64 are targets today).

**Payload widths and the number of variants.** A variant's payload is an
integer (`iN`/`uN`, section 1.1), `bool`, a pointer (`*T`, `Gc<T>`), or
nothing; in a nanbox, one `unboxed` variant holds the `f64`. Integer
payloads are as wide as their type says, so `Int(i48)` keeps 48 bits and
`x.add_checked(y)` on its payload detects overflow of 48 bits.

| `repr` | Payloads | Tagged variants |
|---|---|---|
| `nanbox` | integers up to 48 bits (`i48`, `u32`, ...), `bool`, pointers (48 bits: x86-64 and AArch64 user-space addresses) | Up to 7 with any payloads. Beyond 7, the NaN space is divided by payload width: it holds seven 48-bit blocks and one 47-bit block, and each variant takes a block of 2^(payload bits) (a pointer or `i48` a whole 48-bit block, `i47` the 47-bit one, `u32` 2^32 words, a variant without payload one word). |
| `lowtag` | integers up to 61 bits (`i61`), `bool`, pointers (8-byte aligned) | Up to 8 with any payloads. Beyond 8, pointers and 61-bit integers take one 3-bit tag each, and a variant with an N-bit payload can have a tag of up to 64 - N bits, so narrow variants share what is left. |
| `boxed` | any number, `bool`, pointers | any number |

For example, a Scheme value with `Num(f64) unboxed`, `Int(i48)`, five
pointer types, `Char(u32)`, a `Prim(u16)` builtin index and nullary `Nil`,
`False`, `True`, `Eof` fits a nanbox: the integer and pointers take six
48-bit blocks and the rest share the remaining space. A declaration that
does not fit is a compile error naming the variant with no room.

The fast paths do not depend on the layout: a variant test is one shift and
compare (`(v >> w) == tag` in a nanbox, `(v & mask) == tag` in a lowtag,
`v == word` for a variant without payload), the unboxed `f64` test is one
unsigned compare, and tuple patterns whose elements have tags in the same
position merge into one test. Extracting a pointer is one `and`. See
[lowering.md](lowering.md) section 2 for the encodings.

Switching `repr` is a one-word change, which makes the representation
something you can benchmark rather than something you commit to on day one.

---

## 3. The `vm` declaration

A `vm` block declares one interpreter: its register state, operand kinds,
instruction encoding, inline caches, instructions, and entry and error
handlers. It is the center of the language.

```loimi
vm Mini {
    context ctx: *Interp;                 // cold state, reachable from all handlers
    state { pc: Cursor<u32>, regs: Window<Value>, k: *const Value }
    operand ...;                           // how operand fields are interpreted
    encoding ... { format ... }            // how instructions are laid out
    cache ...;                             // inline cache layouts
    op ... / family ... / super ...        // instructions
    fn ...                                 // vm functions (helpers with state access)
    entry ... / on raise ...               // ways in and the error path
    variant ...                            // instrumentation
}
```

### 3.1 `state` and `context`

`state` lists the variables that are **live across every dispatch**: program
counter, stack or register-window pointer, constants pointer, and so on.
Inside handlers they behave like mutable locals. The compiler guarantees they
live in machine registers for the entire life of the interpreter:

- in tail-call mode they are the arguments of every handler, in a calling
  convention (`preserve_none`) that assigns them fixed registers;
- in goto and switch modes they are loop-carried SSA values in one function.

`context` is one more register holding a pointer to everything else (heap,
frame stack, globals, interned strings). Fields accessed through `ctx` cost a
load, which is the right price for cold-ish data.

The compiler warns when `state` + `context` exceeds the target's argument
registers under `preserve_none` (about 12 general-purpose registers on x86-64
and more on AArch64, but fewer is better: 3 to 6 is typical). **Not built:**
the warning.

State variables have **state kinds**, which are thin typed wrappers over a
single pointer that the verifier understands:

| Kind | Meaning |
|---|---|
| `Cursor<W>` | instruction pointer over code words of type `W` |
| `Window<T>` | base of the current register window (register VMs) |
| `StackTop<T>` | operand stack top (stack VMs); `push`, `pop`, `peek(n)` |
| plain types | anything else (`*const Value`, `u32`, ...) |

**Not built:** state kinds. State variables are plain types (`pc: *u32`,
`regs: *Value`); `Cursor`, `Window` and `StackTop` are not types. A stack
VM keeps a plain pointer, and its ops declare stack effects for the
verifier (§3.4a).

### 3.2 Operand kinds

An operand kind says what an encoded field *means*. Its body is an expression
over the raw field `it`, evaluated in the handler. Its `verify` clause is
checked once by the generated verifier instead of on every execution.

```loimi
operand reg   = regs[it]   verify it < proto.nregs;
operand kst   = k[it]      verify it < proto.consts.len;
operand imm   = it;
operand label = pc + it    verify is_insn_boundary(pc + it);
operand upv   = ctx.closure.upvals[it] verify it < proto.nupvals;
```

`verify` clauses may refer to the `verify_context` declared in the vm
(typically the function prototype being checked, `verify_context proto: Gc<Proto>;`).
A verifier can take several context parameters, in one declaration
(`verify_context proto: *Proto, consts: *Consts;`) or in several; they
follow the code in `Vm.verify(code, proto, consts)` in declaration order.

If the expression is a place (like `regs[it]`), the operand is assignable:
`dst = Int(1)` writes the register. `x.raw` gives the undecoded field.

The raw field `it` (and `x.raw`) is a `u64`, or an `i64` for a signed field
(sign-extended), whatever the field's width: most fields are indices, and
`regs[a + 1]` should be one 64-bit add, not a wrapping 8-bit add and a
zero-extension. The width and signedness come from the format and matter
to the encoders (`Vm.OP(a: u8, ..)`), so one `label` kind serves 8-bit and
24-bit offsets alike.

**Constant operands.** A `const` clause says that what an operand reads
cannot change once the code is verified:

```loimi
operand kst = k[it] const proto.consts.data.ptr verify (it as u64) < proto.consts.data.len;
```

The body must be `S[it]` for a pointer state register `S` (the constant
pool). The clause is an expression of the verify context with `S`'s type:
the value `S` has while code verified against that context runs. It is a
promise, like a `verify` clause: the vm sets `S` to it whenever it starts
running that code (on calls and returns). The checker enforces the rest
within the vm: no handler, vm function, cold block or `on raise` may
write through `S` (`c = x` on the operand, `k[i] = x`, `*(k + i) = x`) or
take an address in it (`&k[i]`). Plain functions and the host must not
change the pool of verified code either. A vm has at most one pool.

`const` changes nothing in the interpreter: the operand reads `S[it]` as
before (the default build's handlers are the same). With
`--jit-stencils`, `Vm.build_verified` passes the pool to the JIT, which
patches each constant into compiled code as a 64-bit immediate, and uses
stencil variants per tag of a constant of a `value` type, with the
checks on that tag folded (`docs/design/copy-and-patch.md`, section 12).
Reads of the pool at a const operand's index through pointer arithmetic
(`*((k + c.raw) as *f64)`) are patched too.

### 3.3 Encodings and formats

```loimi
encoding fixed(u32) {                 // every instruction is one u32 word
    format ABC  { op: 8, a: 8, b: 8, c: 8 }
    format ABx  { op: 8, a: 8, bx: 16 }
    format ABsC { op: 8, a: 8, b: 8, sc: s8 }
    format sJ   { op: 8, sj: s24 }
}

encoding bytes {                       // variable length
    format Z    { op: u8 }
    format U8   { op: u8, x: u8 }
    format U16  { op: u8, x: u16le }
}
```

**Not built:** `encoding bytes` (variable-length instructions). The code
word of `encoding fixed(W)` is `u8`, `u16`, `u32` or `u64`. A format's
fields are listed from the low bits up; `N` is an unsigned field of N bits
and `sN` a signed one, and the fields need not fill the word. One field
must be `op`.

An op lists its operands and names its format. Operands map to the
format's non-`op` fields by position, whatever their names; an op may
have fewer operands than the format has fields, and the rest encode as 0
and are ignored (`op HALT() : ABC`, `op DUP() : N` with `N` holding a
24-bit `x`). The compiler type-checks the mapping.

**Generated API.** Each item is `Vm.NAME` (`Vm__NAME` in the vm's module;
private to the module unless the vm is `pub vm`). `W` is the code word
type.

| Item | What it is |
|---|---|
| `Vm.OP(a, b, ..) -> W` | encoder: the instruction word of `OP` (not for supers). A field of up to 8 bits takes a `u8` (`i8` if signed), up to 16 a `u16`/`i16`, up to 32 a `u32`/`i32` (so a 24-bit field takes `u32`, an `s24` one `i32`), wider a `u64`/`i64`. The value is masked to the field's width without a range check: use `Vm.set_label` or `bc.Field.fits` where it may not fit |
| `Vm.emit.OP(code, at, a, b, ..) -> u64` | emitter: stores the word at `code[at]`, zeroes the inline cache words, returns `at + op_size` |
| `Vm.OP_X: W` | opcode constant of each op, family member and super |
| `Vm.NUM_OPS: u64` | number of opcodes; opcodes are `0..NUM_OPS` |
| `Vm.CW_Cache: u64` | code words taken by inline cache layout `Cache` |
| `Vm.opcode(w) -> W` | the opcode field of a word |
| `Vm.with_opcode(w, op) -> W` | `w` with its opcode replaced |
| `Vm.base_op(w) -> W` | the opcode `w` stands for alone: a super's head gives its first part's op; other words, including quickened family members, give their own |
| `Vm.op_size(w) -> u64` | words the instruction at `w` takes, caches included (a super: all its parts) |
| `Vm.base_size(w) -> u64` | size of the op `w` stands for alone |
| `Vm.op_name(w) -> str` | name of the op (`"<invalid>"` for an unused opcode) |
| `Vm.Op`, `Vm.op_of(w) -> Vm.Op` | an enum with one variant per op as host code sees it (`Vm.Op.ADD`; no supers, a family as its generic op), whose discriminant is the opcode (`Vm.Op.ADD as W == Vm.OP_ADD`), and the op word `w` stands for alone: a super's head gives its first part, a quickened member its family. An invalid opcode panics (verified code has none). For host tools (a second interpreter, a compiler pass, a disassembler) that `match` over the ops: the match is exhaustive, so adding an op is a compile error there |
| `Vm.FMT_field(w) -> u64` / `i64` | field `field` of format `FMT` (`Vm.AsBx_sbx(w)`): `i64` for a signed field |
| `Vm.set_FMT_field(w, v) -> ?W` | `w` with field `field` of format `FMT` set to `v` (`u64`, or `i64` for a signed field), the other bits kept; None if `v` does not fit the field. For peepholes that retarget an operand (`Vm.set_ABC_a(w, r)`) |
| `Vm.encode.FMT(op, a, b, ..) -> W` | the word of opcode `op` in format `FMT` with these fields (typed and masked like an op's encoder): one encoder for every op of a format, for code that picks the opcode at run time (`Vm.encode.ABC(Vm.OP_ADD, d, a, b) == Vm.ADD(d, a, b)`) |
| `Vm.set_label(w, off) -> ?W` / `Vm.label(w) -> ?i64` | re-encode / decode the jump offset of an op whose operand kind is `pc + it` (below) |
| `Vm.operand_count(w)`, `operand_kind(w, i)`, `operand_bits(w, i)`, `operand_signed(w, i)`, `operand_name(w, i)` | per-op operand metadata (below) |
| `Vm.NUM_KINDS`, `kind_name(k)`, `kind_is_label(k)`, `verify_clause(k)`, `check_name(i)` | operand kinds and `verify` blocks, by index |
| `Vm.disasm(code, &out)`, `Vm.disasm_insn(code, i, &out) -> u64` | disassembler (below) |
| `Vm.verify(code, ctx..)`, `Verified<Vm>`, `Vm.VerifyError`, `Vm.VerifyReason`, `Vm.assume_verified`, `Vm.placeholder()`, `Vm.build_verified`, `Vm.build_assume_verified` | the verifier and its proof (section 6) |
| `Vm.stack_effect(w) -> ?(u32, u32)`, `Vm.stack_effect_text(w) -> str`, `v.max_stack() -> u32` | stack VMs: an op's declared (pops, pushes) when they depend only on its operands, its effect as text, and the verified maximum depth (section 3.4a) |
| `Vm.optimize(v)`, `Vm.optimize_words(code)` | superinstruction peephole (section 7.3) |
| `Vm.deopt(v)`, `Vm.deopt_words(code)` | back to generic ops, supers split, caches zeroed (section 7.2) |
| `Vm.real_word(w) -> W` | the real word behind a JIT enter word (below) |
| `Vm.depth: u32` | thread-local count of active nested runs, if the vm has a nested entry (section 10) |
| `Vm.ENTRY(..)` | each `entry` (section 3.6) |

In more detail:

- opcode constants (`Vm.OP_ADD`), `Vm.opcode(w)`, and the per-op
  decoders, inlined into the handlers;
- encoders (`Vm.ADD(dst, a, b)` returns the word) and emitters
  (`Vm.emit.ADD(code, at, dst, a, b)` stores it, zeroes the inline cache
  and returns the next offset). Labels with backpatching and branch
  range checks are in `std.bc`, a library over the generated `op_size`
  and `set_label` (see stdlib); there is no generated `Builder` type;
- a disassembler (below);
- `Mini.verify` (section 6);
- metadata for tools: `Vm.op_size(w)`, `Vm.op_name(w)`, `Vm.NUM_OPS`,
  branch targets through `Vm.label(w)`, and per-op operand metadata, so
  that fuzzers and assemblers can build plausible instructions:
  `Vm.operand_count(w)` operand fields, in encoder order, and for field
  `i` its kind `Vm.operand_kind(w, i)` (an index into the vm's `operand`
  declarations, in order; `Vm.NUM_KINDS` for none), width
  `Vm.operand_bits(w, i)` and `Vm.operand_signed(w, i)`;
  `Vm.kind_name(k)` and `Vm.kind_is_label(k)` describe a kind. A super's
  head answers for its first part;
- `Vm.optimize(v)`, the superinstruction peephole (section 7.3), and
  `Vm.deopt(v)` (section 7.2), over verified code (`optimize_words`,
  `deopt_words` over a raw slice);
- label and field helpers: `Vm.set_label(w, off) -> ?W` re-encodes the
  jump label of word `w` (its first operand whose kind is `pc + it`) and
  returns None if `off` does not fit the field or the op has no label;
  `Vm.label(w) -> ?i64` decodes it; `Vm.FMT_field(w)` reads field
  `field` of format `FMT` (`Vm.AsBx_sbx(w)`), as a `u64` or, for a
  signed field, an `i64`, and `Vm.set_FMT_field(w, v)` writes it
  (None if `v` does not fit). `Vm.encode.FMT(op, ..)` encodes any op of
  a format. The setters and format encoders are compiled only into
  programs that call them. A superinstruction's head word answers for its
  first part. `set_label` and `label` are the patch and jump-threading
  functions `std.bc` takes: `bc.Code.new(Vm.op_size, Vm.set_label)`;
- `Vm.real_word(w)`: the instruction word that code word `w` stands
  for. With the copy-and-patch JIT (`--jit-stencils`), compiled
  instructions' words are *enter words* while the JIT holds the code;
  every generated function above (and the disassembler and verifier)
  maps a word through `real_word` first, so host code sees the real
  words. Host code that inspects `code[i]` itself (hashing, comparing,
  serializing) calls `Vm.real_word(code[i])` on its instruction words
  (inline cache words are never enter words, but may look like one: read
  them as they are). Without the JIT it is the identity and costs
  nothing.

The format declaration is the single source of truth for all of these, so they
cannot drift apart.

**Disassembler.** `Vm.disasm(code, &out)` prints every instruction of
`code`, and `Vm.disasm_insn(code, i, &out) -> u64` prints the one at word
`i` and returns its size in words (0 if `i` is past the end). `out` points
to any type with `push_str(str)` and `push_int(i64)` methods (and
`push_f64(f64)` if a cache has a float field), such as `text.StrBuf`: the
two functions are generic over it, and cost nothing unless called. Each
line is the word offset, the op name, and the operands by name with their
decoded values, signed fields as signed:

```
    0  LOADK dst=r0 c=k3
    1  LOADI_ADD
    1    LOADI dst=r1 v=-7
    2    ADD dst=r2 a=r0 b=r1
    3  GETPROP dst=r3 obj=r2 [cache shape=null slot=0]
    7  JMP target -> 1
```

- A label operand (kind `pc + it`) prints its absolute target: `target -> 1`.
- An operand kind can say how its values print with `display`, a string
  with one `{}` for the value (for a label, the target):
  `operand reg = regs[it] verify it < proto.nregs display "r{}";`. Without
  one, the number prints alone.
- Inline cache words print as `[cache field=value ...]`: integers in
  decimal, `bool`s, `f64`s, pointers in hex (`null` for 0).
- A superinstruction prints its name, then each part on its own line at
  its own offset (the head word with the first part's operands). The parts'
  words are read as they are, so quickened parts show their current op.
  `then` followers are ordinary instructions and print as such.
- An invalid opcode prints as `<invalid 0x..>` (size 1); an instruction
  that runs past the end of `code` prints its name and `<truncated>`.

### 3.4 Ops and control words

```loimi
op MOVE(dst: reg, src: reg) : ABC { dst = src; next }
```

A handler body has type `never`: **every path must end in a control word**.

| Control word | Meaning |
|---|---|
| `next` | advance `pc` past this instruction and its inline caches, then dispatch |
| `jump t` | set `pc = t` and dispatch |
| `redo` | dispatch the current instruction again (after `become`) |
| `exit v` | leave the run loop; the entry returns `v` (its declared result type: `exit Ok(x)` for a `Result`) |
| `raise e` | transfer to the `on raise` handler with the current `pc` |
| `fail e` | leave the run loop, returning `Err(e)` |
| `suspend v` | **Not built:** leave the run loop with state saved for resumption (generators, coroutines, async) |

An operand may be in parentheses: `jump (t as *u32)`, `exit (x)`. In vm
code the control words are not function names, so `fail (..)` never
calls a function `fail` (call such a function through its module,
`m.fail(..)`); `next (` and `redo (` are errors, since they take no
operand. Outside vm code, a control word is an unknown name, and the
error says so.

Handlers and vm functions do not assign `pc` (or take its address):
`next`, `redo` and a superinstruction's next part are relative to the
instruction, so `jump` is the way to go elsewhere. Entries and `on raise`
may set it.

Inside a handler, `expr?` on a `Result<T, E>` means "on `Err(e)`, `raise e`".
`E` must be the vm's raise type.

Because the size of each instruction is static, `next` compiles to
`pc += size; dispatch`. In fixed-width encodings the compiler also
**decodes ahead**: it loads the next instruction word and handler address at
the top of the handler when the handler can't modify code, so the dispatch
load overlaps with the handler's work.

### 3.4a Stack effects (stack VMs)

A stack VM declares what each op pops and pushes, after the format (and
cache and `then`), in Forth notation:

```loimi
verify_context word: *Word, words: *mem.Vec<*Word>;
stack_entry word.nin;                       // depth the code starts with

op PUSHI(v: imm) : S stack(-- n) { .. }
op ADD() : N stack(a b -- c) { .. }
op MKARRAY(n: cnt) : N stack(xs[n] -- arr) { .. }
op CALL(w: wref) : N stack(args[words.at(w).nin] -- rs[words.at(w).nout]) { .. }
op RET() : N stack(rs[word.nout] --) { .. }
op HALT() : N stack(.. --) { exit Ok(()) }
```

- Each item is one value, or `name[count]` for `count` values: an
  expression over the op's operands (raw fields, as `it` in a verify
  clause) and the verify context. Commas between items are optional; the
  names are documentation (the disassembler prints them).
- `..` first among the inputs takes every value on the stack.
- If any op declares an effect, every op must (`stack(--)` for none);
  family members share their family's.
- `stack_entry EXPR;` (default 0) is the depth code starts with, from the
  verify context (a function's arguments).

The verifier (§6) walks the control flow: an op continues to the next
instruction if it may continue (as for "run off the end") and goes to
its label (`pc + it` operand) if it has one. It rejects an op that pops
more than the stack holds, two paths that reach an instruction with
different depths, and an op that ends the code (it neither continues nor
has a label: RET, HALT, a tail call) with values left beyond the ones it
takes, unless it takes `..`. It records the most the stack grows above
`stack_entry` in the proof: `v.max_stack() -> u32` on `Verified<Vm>`, so
the interpreter checks room for a call once (`sp + callee.code.max_stack()
>= stack_end`) and its ops push and pop without checks. Such a vm keeps
its proof in a `Verified<Vm>` field (a `verified [W] by Vm` array has no
room for the bound), and `assume_verified(code, max_stack)` takes the
bound too.

What the handlers do with the stack is up to them (they move a pointer
state register, perhaps with the top value cached in another), and the
compiler does not check them against the declared effects: the effect is
the contract, as a verify clause is for an operand. Ops that leave
verified code with values on the stack (exception handlers jumping to a
table target) are not followed; the target's depth is the vm's to
ensure.

For tools: `Vm.stack_effect(w) -> ?(u32, u32)` gives (pops, pushes) of
the op word `w` stands for alone, when they depend only on its operands
(None for `..` and counts that read the verify context), so a compiler
can track depths from the declarations (`examples/stack` does), and
`Vm.stack_effect_text(w)` the declared effect (`a b -- c`).

### 3.5 VM functions

A `fn` inside a `vm` block is a **vm function**. It can read and write state
and is always inlined into its callers. Like a handler, it may call
effectful functions without declaring their effects; those calls are
synced where they are made (section 5). If its return type is `never`, it
can use control words: `jump`, `next`, `redo`, `raise`, `exit`, `fail`, and
`?` on the raise type. That makes shared tails easy to write:

```loimi
fn call_closure(c: Gc<Closure>, base: *Value, nargs: u8) -> never { ...; jump c.proto.code }

// The word after a conditional op is a JMP holding the branch offset.
fn branch() -> never { jump pc + 1 + ((pc[1] as i32) >> 8) as i64 }
fn cond_branch(take: bool) -> never { if take { branch() } jump pc + 2 }
op JLT(a: reg, b: reg) : ABC { cond_branch(a < b) }
```

Rules for `-> never` vm functions:

- Every path must end in a control word.
- `next` and `redo` mean what they mean in the calling op: `next` skips the
  caller's instruction (in a superinstruction part, it continues with the
  next part). A function that uses `next` can only be reached from op
  handlers, not from entries or `on raise`.
- `next_pc` is the address `next` continues at: the calling op's `pc`
  plus its size (cache words included; in a superinstruction part, the
  part's own size, so the address of the next part's original
  instruction). It is the return address a shared call helper saves,
  correct for every calling op by construction:

  ```loimi
  fn call(f: *Func, base: u64) -> never { fp = fp + 1; fp.ret = next_pc; ..; jump f.code }
  op CALL(b: reg, fi: fidx) : ABC { call(ctx.funcs[fi], b.raw) }
  op CALL_IC(b: reg, fi: fidx) : ABC + CallIC { ..; call(f, b.raw) }   // returns past the cache word
  ```

  Writing `pc + 1` instead is wrong for an op with cache words, and
  nothing would catch it. `next_pc` is available where `next` is (op
  handlers, and `-> never` vm functions reached from them; a shared copy
  gets the size from its caller, like `next`). It is an ordinary name:
  a local or state variable called `next_pc` hides it.
- The body sees the vm state and its own parameters, but not the caller's
  operands: pass `dst.raw` or values as arguments. `become` is not
  allowed, since it belongs to the calling op, and neither is `cache`;
  pass the cache instead (`c: cache GlobalIC`, see below).
- They can only be called where control words are allowed: op handlers,
  entries, `on raise`, and other `-> never` vm functions.

How a call is compiled depends on the dispatch mode and on where it is:

| | Call site on the hot path | Call site in a `cold` block, or recursive |
|---|---|---|
| `tailcall` | body inlined | `musttail` to one shared copy with the handler signature |
| `goto`, `switch` | body inlined | body inlined (recursion is an error) |

The shared copy is reached like an outlined cold block: the arguments, and
the size `next` skips, go through thread-local slots, and the state stays
in registers. Called from a superinstruction's part before the last, it
also gets where its `next` continues (the super's next part, section
7.3), in one more slot; shared copies never called that way have none. `#[inline]` on the function inlines every call;
`#[inline(never)]` makes every call in tail-call mode use the shared copy.

**Passing the inline cache.** A vm function (either kind) can take the
calling op's inline cache as a parameter of type `cache Name`, and the
op passes `cache`:

```loimi
cache GlobalIC { version: u64, entry: ?*Entry }

fn get_global(c: cache GlobalIC, dst: u64, key: Sym) -> never {
    if let Some(e) = c.entry and c.version == ctx.globals.version {
        regs[dst] = e.value; next
    }
    cold { regs[dst] = lookup_and_fill(c, key)?; next }
}
fn lookup_and_fill(c: cache GlobalIC, key: Sym) -> Result<Value, Value> { .. c.entry = Some(e); .. }

op GETGLOBAL(dst: reg, key: kst) : ABx + GlobalIC { get_global(cache, dst.raw, key.as_sym()) }
```

`c` is a handle to the op's cache words: `c.field` reads and assigns
them exactly as `cache.field` does in the op, with the same types and
unaligned access for wide fields. The argument must be `cache` in an op
whose cache has that layout, or another handle of the same layout (a vm
function passing its `c` on); the handle is not a value otherwise
(`c` alone, `let`, return types are errors). At run time it is the
address of the instruction's first cache word, `pc + 1`:

- in an inlined copy (hot call sites of `-> never` functions, and every
  call of a plain vm function, which is always inlined), `c.field` folds
  to the same `pc`-relative access as `cache.field`, so it costs nothing;
- in a shared copy of a `-> never` function (calls from `cold` blocks,
  recursive calls, `#[inline(never)]`), the address travels through a
  thread-local slot like the other arguments. The shared copy therefore
  reads and fills the cache of the instruction that called it, whichever
  that is; every caller passes its own.

A superinstruction part passes the part's own cache. `become` stays
with the op: a helper that decides to re-specialize can return a value
the op acts on.

### 3.6 Entries and the error path

```loimi
entry run(c: Gc<Closure>) {
    ctx.frames.push_entry_marker();
    regs = ctx.stack.window_at_top(c.proto.nregs)?;
    k = c.proto.consts.ptr();
    jump c.proto.code
}

on raise(err: Value) {
    loop {
        if let Some(h) = ctx.proto.handlers.find(pc) {
            regs[h.reg] = err; jump h.target
        }
        let Some(f) = ctx.frames.pop_guest() else { fail err };
        restore(f);
    }
}
```

The compiler generates `Mini.run(ctx, c) -> Result<Value, Value>` with the C
ABI. (**Not built:** the C ABI; an entry is an internal Loimi function, and
a library wraps it in an `export "C"` function, section 10.1.) `c.proto.code` is a `Verified<Mini>` (section 6): with
`#[require_verified]` on the vm, an entry can only start in verified
code. `on raise` is an ordinary handler: it receives the live state, so the
`pc` it sees is exact without any syncing.

An entry declared `entry call(..) -> R uses reenter` is a *nested
entry*: it starts a run above the frames of a run that is already
active, for host code (a native) that calls guest code. See section 10.
An entry whose vm code may collect declares `uses gc`
(`entry run(..) -> R uses gc`), which its callers then need too
(section 5, "Effects on function types").

### 3.7 Dispatch strategies

Selected per build (`--dispatch=` or `#[dispatch(...)]` on the vm):

| Strategy | Lowering | When to use |
|---|---|---|
| `tailcall` (default where supported) | one LLVM function per handler, `preserve_none` convention, `musttail` calls | x86-64 and AArch64 with LLVM ≥ 19; best register allocation, fast compiles |
| `goto` | one function, `indirectbr` replicated at every `next` | targets without `preserve_none` |
| `switch` | one function, loop plus `switch` | portability, wasm without tail calls, debugging |

All three are produced from the same source, so a benchmark can compare them
directly.

In `tailcall` mode, handlers (and their outlined cold blocks and the
rest functions of superinstructions) carry LLVM's `no-builtin-memset`,
`no-builtin-memcpy` and `no-builtin-memmove`: a loop that zeroes or
copies memory stays a loop instead of becoming a library call, which
would give the handler a stack frame and spills on every path even when
the loop is short (as in a call op zeroing the callee's locals). Shared
copies of `-> never` vm functions (cold call sites) and ordinary
functions keep the usual behavior. The dispatch table pointer can be a state register (needed for
instrumentation variants, section 9) or a global constant (saves a register);
by default the compiler uses a register only if the vm declares variants.

---

## 4. Slow paths: `cold` blocks

Handler code splits into a short fast path and a large, rare slow path
(type coercions, metamethods, errors, allocation slow paths). In C, the slow
path's calls force the compiler to treat the VM registers as clobbered, so the
*fast* path ends up spilling them. Loimi makes the split explicit:

```loimi
op ADD(dst: reg, a: reg, b: reg) : ABC {
    if let (Int(x), Int(y)) = (a, b) and let Some(r) = x.add_checked(y) {
        dst = Int(r); next
    }
    cold { dst = arith(ctx, .Add, a, b)?; next }
}
```

A `cold` block is **outlined** into a separate function that uses a
callee-saves-everything convention (`preserve_most`, or `preserve_all` on
request). The handler's state registers survive the call untouched, so the fast
path is compiled as if the slow path weren't there. The block can still use
state and control words: the outlined function returns a small code ("next",
"jump to X", "raise E") and the handler performs the transfer.
**Not built** as described: in tail-call mode a cold block that ends in
control words becomes its own handler-shaped function reached with
`musttail` (no call, no return code); other cold blocks, and all of them in
goto and switch modes, stay inline and are marked unlikely (`status.md`). Branches into
cold blocks are weighted as unlikely. A `cold` block is also an expression:
`let ok = match (a, b) { ..., _ => cold { compare_slow(ctx, a, b)? } };`

---

## 5. Effects and sync points

The most common bug in hand-written interpreters is **stale VM state**: some
slow path allocates (and triggers GC, which scans the stack up to a stale top),
or raises an error (with a stale `pc`, so the wrong line or exception handler),
and nobody saved `pc`/`top` first. Lua guards against this with `Protect()`
and `savepc`; CPython with explicit frame syncs. Both rely on discipline.

Loimi has three ordered **effects** on functions:

| Effect | Meaning | Requires at the call site |
|---|---|---|
| `sync` | callee reads VM state (`pc` for tracebacks, `top`) | state written to `ctx.sync` (as built: the vm's `sync { .. }` block runs) |
| `gc` | callee may run a garbage collection | `sync`, plus every live `Gc`/`Value` local made visible as a root |
| `reenter` | callee may run guest code (call into the VM) | `gc`, plus state reloaded afterwards (the vm's `reload` block) |

```loimi
fn new_string(h: *Heap, s: str) -> Gc<Str> uses gc;
fn traceback(ctx: *Interp) -> String uses sync;
fn call_meta(ctx: *Interp, f: Value, args: []Value) -> Result<Value, Value> uses reenter;
```

Rules:

- Effects are part of public signatures and inferred for private functions.
  **Not built:** inference; every function declares its effects.
- A function may only call effectful functions if it has at least that effect
  itself, or if it is a handler or `cold` block. Forgetting `Protect()` becomes
  a type error.
- **In handlers**, the compiler inserts the sync at each effectful call site:
  store `pc` and the stack top to `ctx.sync`, and spill live `Value`/`Gc`
  locals, and state variables of those types, into a per-vm temporary
  root area. With a moving collector it also
  reloads them afterwards. Calls without effects pay nothing. (As built:
  the sync is the vm's `sync` block, and traced locals go into a
  shadow-stack frame, with copies of the traced state variables taken at
  the call (section 8). **Not built:** a moving collector.)
- **In plain vm functions** (section 3.5), effectful calls are synced in
  place exactly as in a handler, without declaring the effect: a vm
  function sees the calling handler's state through pointers and is
  inlined into it, so `fn gc_point() { if ... { cold { gc.collect(..) } } }`
  pays for the sync only on the path that collects. A handler (or vm
  function) calling such a function roots its own traced locals at that
  call, as for any call that may collect; it does not sync there. Declaring
  `uses gc` on a vm function instead makes every call of it a synced call.
  Called from an entry, nothing is synced (the run loop has not started).
- **In ordinary functions**, `Gc`/`Value` locals that are live across a `gc`
  call are registered in a compiler-maintained shadow-stack frame. This only
  happens on paths that can collect, which are slow paths by nature.
- Syncs only happen where they are needed, so code with no effects never
  writes `pc` to memory. Saving `pc` on every instruction would add a store per
  dispatch.

### `reenter` and the cached state

A call with `reenter` may run guest code on the same VM stack, nested
above the calling frame (section 10). Rooting traced locals, as for `gc`,
is not enough: while the guest code ran, the stack may have been
reallocated and frames pushed and popped, so every state register that
caches something derived from them (`regs`, `k`, `fp`, a stack top)
may be stale. (The dispatch table register is not among them: a nested
run starts with the default table, and `use_variant` there switches
only that run's.) A vm
whose handlers or vm functions make such calls declares how to get its
state back, next to `sync`:

```loimi
sync   { ctx.fp = fp; }                            // before: publish
reload { fp = ctx.fp; regs = ctx.stack + fp.base; k = fp.k; }   // after: recompute
```

- At each `reenter` call in an op, `on raise`, `cold` block or vm
  function, the compiler runs `sync`, roots traced locals, makes the
  call, then runs `reload` (through the state pointers in a vm
  function). The call's result is used after the reload, so
  `dst = native(ctx, a)?` stores into the reloaded `regs`. A `reenter`
  call in vm code without a `reload` block is an error.
- `reload` should recompute state from what the nested run cannot have
  invalidated: offsets or the frame record the native's caller left, not
  pointers saved before the call. `pc` usually needs no reload (code does
  not move), and the current instruction word, decoded operand fields
  and `next`'s size are fixed.
- What `reload` cannot fix is the handler's own derived data. A raw
  pointer into the stack computed before the call (`let p = regs + a`)
  is stale after it: compute it again from the reloaded state. The same
  goes for anything read from the heap or the code before the call: the
  guest code may have changed tables, upvalues (and through them the
  caller's registers), inline cache words, or quickened and fused the
  very instruction that is running (its operands stay the same, so
  `next` is still right). The compiler does not check these; keep the
  code after a `reenter` call short, as in a `cold` block that stores
  the result and continues.
- Thread-local slots the compiler uses to move values between handlers
  (cold-block captures, `-> never` arguments, the raise and result
  slots) are read before any call can re-enter, so nested runs do not
  clobber them.

### Effects on function types

A function type carries effects with the same syntax as a declaration:

```loimi
struct Hooks {
    scan:  fn(*Heap, *u8),                      // no effects
    alloc: fn(*Heap, u64) -> *u8 uses gc,       // may collect
    call:  fn(*Interp, Value) -> Value uses reenter,
}
```

- A function type names one effect, the strongest (`uses gc` already
  includes `sync`). The effect of a declaration and of its value's type are
  the same: `fn f(h: *Heap) uses gc` has type `fn(*Heap) -> () uses gc`.
- A function value converts to a function type whose effects include its
  own: a plain `fn(*Heap)` can be stored in a `fn(*Heap) uses gc` field,
  but not the other way around. The conversion applies to function values
  and `?fn` options, not behind pointers: `*fn() uses gc` and `*fn()` are
  unrelated types, since a store through one would be read through the
  other. Function types are otherwise compared exactly: `if c { f } else
  { g }` with different effects may need an annotation.
- A call through a function pointer has the effects of the pointer's type,
  with the same rules and code as a direct call: the caller must declare
  them (or be a handler or `cold` block, where the `sync` block runs
  first), and a `gc` call gets a shadow-stack frame for the traced locals
  in scope.
- Closure types follow the same rules: `block fn(i64) uses gc`,
  `closure fn(i64) uses gc` and `Gc<closure fn(i64) uses gc>` are called
  like a `uses gc` function, a closure's body may only make the calls its
  type allows, and a closure converts to the same kind of closure type
  with more effects (section 1.2).
- **Entries** carry the effects of the vm code they run. If any op,
  `on raise`, vm function, cold block or variant may collect (it makes
  a `uses gc` or `uses reenter` call), every entry of the vm declares
  `uses gc` (`entry run(c: *Ctx, cl: Gc<Closure>) -> Result<V, V>
  uses gc`), or `uses reenter` if it is a nested entry (section 10).
  The effect is part of `Mini.run`'s function type, so its callers
  declare it too and root their traced locals across the run, and the
  entry converts only to function types that include it: a plain
  `fn(*Ctx, Gc<Closure>) -> Result<V, V>` holding it would make calls
  that root nothing. An entry's own body calling a `uses gc` function
  needs the declaration as well. `reenter` reached from vm code does
  not make a plain entry `reenter`: the entry starts a run with no
  other run of the vm active, so its caller has no vm state to reload.
- Calls through a trait bound have the effects of the trait's method
  declaration; an impl's method may have fewer.
- Generic code carries effects through: `T` bound to `fn() uses gc` keeps
  the effect, and `fn(T) -> U uses gc` in a generic signature stays
  effectful in every instance. Inference does not look at effects; the
  argument is checked against the instantiated type afterwards. There are
  no effect variables: a generic function takes either plain or effectful
  callbacks, not both.
- `uses` binds to the innermost function type: in
  `fn get() -> fn() uses gc { .. }` the effect is part of the result type,
  not of `get`. Parenthesize to make that explicit (`-> (fn() uses gc)`).
  `(T)` is the same type as `T`; a one-element tuple is `(T,)`.
- Callbacks that run while the runtime is in a fragile state have no
  effects in their type, which makes the restriction checked: the GC's
  root scanners run during a collection, so `Heap.add_scanner` takes a
  `fn(*Heap, *u8)` and rejects a `uses gc` function.

Because allocation fast paths are inlined (`std.gc` bump allocation) and only
their refill path carries `uses gc`, allocating a value in a handler costs
nothing extra unless the refill path actually runs.

---

## 6. Verification: unchecked by proof

`Mini.verify(code, ctx..) -> Result<Verified<Mini>, Mini.VerifyError>` is
generated from the declarations. It returns the code wrapped in a proof,
or the first bad word and why it is bad. It checks:

- every word decodes to a valid opcode and format;
- every operand satisfies its kind's `verify` clause (register and constant
  indices in range, upvalue indices valid, ...);
- every `label` lands on an instruction boundary (never inside an instruction
  or an inline cache);
- in a superinstruction, each later part's word holds an op of that part's
  family. Those words are instructions of their own (a jump may land on
  them, and when a part quickens the super can split up), and are checked
  as such;
- `then OP`: the next instruction is an `OP` (or a member of its family);
- for stack VMs, ops declare stack effects (`op ADD() : N stack(a b -- c)`,
  §3.4a) and the verifier checks depths at every instruction (no
  underflow, paths agree where they meet, code ends with the stack its
  last op expects), like the JVM verifier minus types, and computes the
  maximum depth (`v.max_stack()`);
- code can't fall off the end: it is not empty, and its last instruction
  cannot continue past itself. An op may continue if its handler uses
  `next` or reads `pc` (to save a return address like `pc + 1`), directly,
  in a vm function it calls, or in an op it can `become`. A vm function
  called with literal `true`/`false` arguments is looked at per call:
  `call_slow(base, n, true)` does not continue if `next` and `pc + 1` are
  only under `if !tail { .. }`. An op with `then`
  that may continue (`jump pc + 2` past its JMP) needs an instruction after
  that JMP.

`verify` clauses see `it`, `pc` (the address of the operand's word) and the
verify context, nothing else; a verify context cannot be named `it` or `pc`.

**Errors.** `Mini.VerifyError` has `at` (the word index), `reason` (an
enum `Mini.VerifyReason`: `Empty`, `TooLong`, `BadOpcode`, `Truncated`,
`SuperPart`, `Forward`, `Then`, `RunsOffEnd`, `Operand`, `Check` for a failing
`verify` block, below; stack VMs add `StackUnderflow`, `StackMismatch`,
`StackExit`, `StackTooDeep`, `StackCount` and `BadTarget`), for `Operand` the
operand's index and kind, and `msg`, a `verify` block's message. `e.message()` describes the reason, and
`e.write(&out)` writes the whole diagnosis to any buffer with
`push_str`/`push_int`:

```
word 4 (LOADK): operand `c` (kst) fails `(it as u64) < nconsts`
word 7 (ADD): execution can run off the end of the code after this instruction
word 9 (DROP): stack underflow: the instruction takes 1 value(s), the stack has 0
```

**`Verified<Mini>`** (also spelled `Mini.Verified`) is the proof: an
opaque handle over the code slice, the same pointer and length at run
time. Only the vm's generated functions can build one: its field is
private to the vm's expansion (fields whose names start with `__` are
private to the file declaring their struct), so a struct literal or a
field write outside it is an error. It offers `v.start() -> *W`,
`v.len()`, and `v.words() -> []W` for reading (disassembly, line
tables; writing through it voids the proof). In vm code, `jump v` starts
running the verified code. The escape hatches:

- `unsafe { Mini.assume_verified(code) }`, for code the host built
  itself and trusts (`assume_verified` is declared `#[unsafe]`: naming
  it outside `unsafe { }` is an error);
- `Mini.placeholder()`, a proof over one static word holding an opcode
  no op has, which traps if it runs. It fills a `Verified` field until
  the real code is verified (a prototype built while the rest of the
  program, say the global count its verify context needs, is not known
  yet). It is not generated if every opcode value is taken.

**Requiring the proof.** `#[require_verified]` on the vm makes every
entry start in verified code: an entry's `jump` must take a
`Verified<Mini>` (`entry run(code: Verified<Mini>, ..) { ..; jump code }`),
so `Mini.run` cannot be called with raw code. A raw `jump` in an entry
(or in a `-> never` vm function inlined into one) is an error outside
`unsafe { }`, and an entry cannot reach a shared (cold) copy of a vm
function, which is checked once for all callers. Calls inside the vm
are made structural by the data: a prototype whose code is verified
(a `Verified<Mini>` field, or a verified array, below) has no raw code
to jump to, so `jump p.code` in CALL reaches only verified functions.
Mini, Scheme and MiniJS do this.

**Verified code in an object.** An `object` can hold its code as its
trailing array, marked as verified by a vm:

```loimi
object Proto { nregs: u32, consts: Gc<Arr>, .., code: verified [u32] by Mini }

vm Mini {
    verify_context owner proto: Gc<Proto>;
    operand reg = regs[it] verify (it as u32) < proto.nregs;
    ..
}

let proto = match Mini.build_verified(heap, Proto { nregs: n, .. }, words) {
    Ok(p) => p,
    Err(e) => ..,
};
```

- The generated `Mini.build_verified(heap, init, words, ctx..) ->
  Result<Gc<Obj>, Mini.VerifyError>` allocates the object with room for
  the words, copies them in and verifies them there. It is the only way
  to create such an object: `new` on it is an error elsewhere.
  (`unsafe { Mini.build_assume_verified(heap, init, words) }` skips the
  verifier, like `assume_verified`.)
- `p.code` reads as a `Verified<Mini>` over the array (`start()`,
  `len()`, `words()`, `Mini.optimize(p.code)`); it is not a place, so
  it cannot be assigned. `jump p.code` is the array's address, `p` plus
  a constant: no load, unlike a `Verified` field, which costs one
  dependent load per call (`lowering.md` §11).
- A verify context declared `owner` is the object being built:
  `build_verified` passes the new object, and its other context
  parameters are the caller's. The verify clauses then read the counts
  (and tables) the object holds, so the proof is tied to the object
  and to the context fields stored in it. (`Mini.verify(code, proto)`
  still takes it explicitly, for checking code against an existing
  prototype.) The object's other fields remain ordinary fields: the
  interpreter must not change those the verifier read (`nregs`,
  `consts`) afterwards.

Without an owner context, the proof records nothing about the context
it was checked against, and the interpreter stores it next to that
context. The tutorial's `Pip` *builds, then verifies*: it creates every
prototype with `code: Pip.placeholder()` and verifies once the program
is complete (its context includes the global count), storing each proof
in its prototype (`p.code = v`).

**Side tables.** `verify NAME { .. }` in a vm is a `bool` block that
the verifier runs after checking every instruction, with the verify
context, the code as `code` (a `[]W`) and `is_insn_boundary(p)` in
scope. It checks tables the code refers to but the operand clauses do
not see. MiniJS checks its exception handler table, so that `on raise`
jumps only to instructions:

```loimi
verify handlers {
    let t = proto.handlers.data;   // [start, end, target, reg] per entry
    var ok = t.len % 4 == 0;
    var i: u64 = 0;
    while ok and i < t.len {
        ok = is_insn_boundary(code.ptr + t[i + 2]) and t[i + 3] < proto.nregs;
        i += 4;
    }
    ok
}
```

A failing check is `VerifyReason.Check`, written as ``check `handlers`
fails``. To say where and why, call `reject(at, msg)` in the block: it
fails the check at word `at` with message `msg`, written like the
generated checks (``word 4 (DUP): check `stack` fails: depth 3 > 2``).
With an owner context the table is the object's own, verified
with its code.

`code` in a check is the **base view** of the code: every word as the op
it stands for alone, its family's generic op (a superinstruction's head
as its first part's), with inline caches zeroed, as `Vm.deopt` would
leave it. So a check written against the plain ops still holds for code
that went through `optimize` or quickening (verified again after loading
from a cache, say). If the vm has supers or families, `code` is a copy;
`is_insn_boundary` takes pointers into it. (`Vm.base_op(w)` maps one
word the same way, for host code.)

**Mutation.** Quickening (`become`) and `Mini.optimize` rewrite verified
code in place, and both preserve verification: `become` only switches
between members of a family, which share format, operand kinds and
cache, and the verifier checks every word of a superinstruction as an
instruction of its part's family, so quickening a part or splitting a
super leaves valid code; `optimize` fuses only sequences that verify
as their parts. The bytecode fuzzer checks both after every run
(`tests/fuzz`). `Mini.optimize(v) -> Verified<Mini>` and
`Mini.deopt(v) -> Verified<Mini>` take and return the proof; the raw
passes are `Mini.optimize_words(code)` and `Mini.deopt_words(code)`,
for code that is verified afterwards.

What the verifier and the proof cannot see is up to the vm: that the
run-time state matches the verify context (the register window really
has `nregs` registers), that operands without a `verify` clause
(`imm`) are safe for every value, that the context is not changed
after verification, and that jumps to computed addresses inside the vm
go to boundaries of verified code: return addresses (`jump fp.resume`,
saved by a verified CALL) and table targets, unless a `verify` block
checks the table (MiniJS's handler targets `p.code.start() + target`).
Those raw jumps are allowed in ops and `on raise`, where pc is already
in verified code; only entries are held to `Verified`.

Operand accesses compile **without bounds checks**, and stack VMs check for
overflow once per call (`max_stack` against the limit) instead of on every
push (`v.max_stack()`, §3.4a). The checks don't disappear; they move from run time to load time. This
matters for interpreters that load cached or untrusted bytecode files.

---

## 7. Specialization: quickening, inline caches, superinstructions

### 7.1 Inline caches

```loimi
cache PropIC { shape: ?*Shape, slot: u32 }
```

A cache is stored **inline in the code stream**, after the instruction (as in
CPython 3.11+), and is attached with `+`: `op GETPROP(...) : ABC + PropIC`.
`next` skips it automatically, the verifier forbids jumps into it, the builder
reserves and zero-fills it, and the disassembler prints it. Inside the handler,
`cache.shape` reads and writes the words directly.

**Field types and sizes.** A field is an integer, a float, `bool`, a
pointer, a function pointer, or an option of a pointer (`?*Shape`, None
is null, which is what a zero-filled cache holds). Each field starts a
new code word and takes as many words as its type needs: in a `u32`
vm, `u32` and smaller take one word, and `u64`, `f64` and pointers take
two, so `PropIC` above is three words and `op_size` of `GETPROP` is 4.
In a `u64` vm every field is one word. The generated encoders
(`Vm.emit.X` reserves and zeroes all of them), `op_size`, the verifier's
walk (no jump may land in any of them) and `optimize` count the words.

The code array is an array of words, so a two-word field is only
word-aligned: handlers read and write it with unaligned loads and stores
(`align 4`), which cost nothing extra on x86-64 and AArch64. Taking its
address (`&cache.shape`) is an error, since the pointer would claim an
alignment the field does not have; read the field, or assign it. Fields
no wider than a word may still be passed by address. Wide fields are
written with ordinary stores, which are not atomic if the code is shared
between threads.

**No GC references.** A cache field cannot hold `Gc<T>`, a `value` type
or anything else the collector traces: the collector does not scan
bytecode, so the reference would neither keep its target alive nor be
updated if it moved. (**Not built:** `weak` fields, which the GC would clear when their
target dies: the collector does not scan bytecode, so it could not clear
them.) Cache something whose
lifetime the interpreter controls instead: a shape id that is never
reused, a pointer to a structure that is never freed or moved (shapes
kept alive by a table), or a version number, and validate it on use.
Weak caching of GC objects belongs in the library, where it can be
written as a side table that a sweeper clears, or with `std.gc.Weak`.

### 7.1a The following instruction: `then`

Lua-style conditional ops are followed by a JMP word that holds the branch
offset, so the compiler's jump patching and the verifier treat it as an
ordinary jump. `then OP` after the format (and cache) says that the next
instruction is an `OP`, or a member of its family, and `then.x` reads its
operand `x`: decoded from that word with `OP`'s format and evaluated by
`OP`'s operand kind, with `pc` at that word.

```loimi
op JMP(target: label) : sJ { jump target }
op LT(a: reg, b: reg) : ABC then JMP {
    if a < b { jump then.target }
    jump pc + 2                    // skip the JMP
}
```

The JMP stays a separate instruction (`next` skips only `LT`); the
generated verifier checks that it is there, and checks its operands at its
own word. `then` operands are read-only, and not available in vm
functions (pass them as arguments). This is the same idea as an inline
cache (`cache.slot` is the word after the instruction), with the word
typed as an instruction instead of a cache layout.

### 7.2 Families and `become`

A `family` is a group of ops that share operands, format, and cache, with
specialized implementations. `become X` rewrites the current instruction's
opcode in place.

```loimi
family GETPROP(dst: reg, obj: reg, name: kst) : ABC + PropIC {
    generic {
        let Obj(o) = obj else cold { raise type_error(ctx, "index", obj) };
        let Some(slot) = o.shape.lookup(name.as_symbol()) else { dst = Nil; next };
        cache.shape = o.shape.id; cache.slot = slot;
        dst = o.slot(slot);
        become GETPROP_MONO; next
    }
    GETPROP_MONO {
        if let Obj(o) = obj and o.shape.id == cache.shape {
            dst = unsafe { o.slot_unchecked(cache.slot) };   // shape match implies slot is in range
            next
        }
        cold { become GETPROP; redo }
    }
}
```

The `generic` member, named after the family (`become GETPROP`), is what the compiler emits. Specialized members are only
reached through `become`. `Vm.deopt(v)` maps every member back to
`generic`, for debuggers and for code serialization: it rewrites each
instruction word to its family's generic op, splits superinstructions
(the head becomes its first part's generic op) and zeroes inline caches,
which gives the code the emitters produced.

Rewrites are single-word stores. With multiple guest threads they need an
atomic store, which the compiler would emit when the vm is declared
`#[threads(shared_code)]` (**Not built**).

### 7.3 Superinstructions

```loimi
super LOADK_ADD = LOADK; ADD;
super MOVE_RETURN = MOVE; RETURN;
super LOADI_ADD_F_RET = LOADI; ADD_F; RET1;   // any number of parts; family members too
```

The compiler builds the handler by inlining the component bodies in order.
Paths of a component that end in `next` fall through into the next one.
Other control words (`jump`, `raise`, ...) keep their meaning; `redo` in a
later part runs that part's own word alone.

Every `next` of a part before the last falls through, whichever path
reaches it: from the hot path, from a `cold` block, or from a `-> never`
vm function the part calls. In tail-call mode the last two leave the
super's handler (the cold block is outlined, the function may be a
shared copy), and continue in the super's *resume function* for the next
part, a copy of the handler from that part on (`Vm.op.S.resumeN`, made
only where some path needs it). So the later parts run as parts of the
super in every dispatch mode and with the JIT: a `become` in them
rewrites the head as described below, no instrumentation wrapper runs
for them, and the same program ends up with the same code words. Only
`redo` in a later part, and a jump to a part's word, run the word alone,
as its own op.

A part can name any op, including a specialized family member
(`GETGLOBAL_C`, `ADD_F`). No two supers may have the same parts.

**`become` inside a super.** A part that executes `become X` rewrites its
own word to `X`, as it would alone (so a jump into the middle sees the
specialization). It also rewrites the super's first word: to the super
whose parts are the same with this part replaced by `X`, if one is
declared, and otherwise to the sequence's first op, which splits the
super up. For example, with

```loimi
super LOADI_ADD = LOADI; ADD;
super LOADI_ADD_F = LOADI; ADD_F;
```

the first time `LOADI_ADD`'s `ADD` part sees two floats and runs
`become ADD_F`, the code changes to `LOADI_ADD_F, ADD_F`; when `ADD_F`
deoptimizes with `become ADD`, it changes back. Without `LOADI_ADD_F`, it
would become `LOADI, ADD_F`. `Vm.optimize` can be run again on quickened
code to fuse supers of specialized members.

With `fixed` encodings, the fused op **reuses the components' words**: the
first word's opcode is rewritten to the super-op and the following words stay
as they are, so the super-op reads operands from both words. A jump into the
second word still runs the original second instruction, so fusion is always
safe, needs no control-flow analysis, and can happen at build time (the
`Vm.optimize` peephole) or at run time (quickening).

`Vm.optimize(v)` is the generated peephole pass. It compares words by
the op they stand for alone (a super's head counts as its first part), so
it can run again on code that has been optimized and quickened. The
generated verifier treats every part's word as an instruction: jumps may
land on it, and it must hold an op of the same family as the super's part.

**The matching rule.** `optimize` reads the code as a straight line and
picks, by dynamic programming, the fusions that save the most
dispatches (a super of n parts saves n - 1 each time it runs); every
instruction where a super starts gets the one that is best from there
on. On a tie it takes the longest match, and a fusion over none, which
is what a greedy left-to-right pass would do: with only pairs, and
wherever a long super does not block a better pair of fusions, the
result is the same. Two supers can still not both run where they
overlap: with `super GETGLOBAL_SUBK = GETGLOBAL; SUBK;` and `super
SUBK_CALL = SUBK; CALL;`, the sequence `GETGLOBAL; SUBK; CALL` becomes
`GETGLOBAL_SUBK` followed by the plain `CALL` (a tie, so the leftmost).
(The `SUBK` word does become a `SUBK_CALL` head, but it only runs if
something jumps to it.) With `A_B` and `B_C_D`, `A; B; C; D` becomes
`A; B_C_D`. Declared supers where a proper suffix of one is a proper
prefix of another get a warning, `super_overlap`, at the later declared
one; declare the longer super if the whole sequence is hot, drop one of
the two, or silence it with `#[allow(super_overlap)]` on that super or
on the vm.

**Supers of quickened members.** `optimize` usually runs on freshly
emitted code, where every family is at its `generic` member, so a super
whose parts are specialized members (`GETGLOBAL_C; SUBK`) only forms when
`optimize` runs again after quickening, or by quickening inside a super:
when the `GETGLOBAL` part of a `GETGLOBAL_SUBK` runs `become GETGLOBAL_C`,
the head becomes `GETGLOBAL_C_SUBK` only if that super is declared, and
is otherwise split back into its parts. To keep a super across
quickening, declare both: the pair with the generic member, which
`optimize` forms, and the pair with the quickened member, which the
`become` turns it into.

**Forwarding.** A super's parts exchange values through the register
file: `MUL; ADD` stores the product to `R[dst]` and loads `R[a]` back,
and since both indices are operands, LLVM cannot tell that they are the
same register. The load waits for the store (a store-to-load round
trip, about 5 cycles on x86). In `#[forward] super A_B = A; B;`, an
operand of a later part that may name the register the previous part
writes (same operand kind, not written by the part itself; at most two
per part and three per super) is a *link*. The vm gets one more opcode
per nonempty set of links, a *forwarding variant* `A_B__fN` (N a bit
mask over the links), whose handler uses the previous part's field for
the linked operands, so LLVM forwards the stored value in a machine
register. `optimize` picks the variant whose linked fields are equal
(the plain super otherwise), the verifier rejects a variant whose
fields differ (`Forward`), and a `become` in a part keeps the variant
when the new super has it. There is no run-time test; the cost is
opcodes (up to 7 per super) and code size.

`loimi build --vm-stats` records opcode pair frequencies and sequences
of 3 and 4 adjacent ops, and at exit prints suggested `super`
declarations: pairs that run at least 1% of the time, and longer
sequences that do and are *dominant* (at least half the runs of their
shorter prefix and suffix continue into them), so that a longer super
saves more than pairs over the same ops could. They are ranked by
dispatches saved, and one that overlaps a better one or an existing
super (as the `super_overlap` lint defines it) is moved to a second,
commented-out list that names what it overlaps. So the first list can
be pasted whole without warnings, and the second lists alternatives.
In a library (`--lib`, `--shared`) the report prints when the host
process exits; several such libraries can link into one host (the stats
runtime's symbols are weak, and named by a hash of its source). (**Not built:** a separate `loimi vm suggest-supers`
command.)

---

## 8. GC integration

The collector is in the standard library (`std.gc`), but the language knows
enough to make it precise and cheap:

- **`object` declarations** are GC-managed structs. The compiler generates the
  header (type id, mark bits) and the trace function from field types.
  A trailing variable-length array is allowed (`slots: [Value]`).
- **Generic objects.** `object Obj<V> { shape: Gc<Shape>, inline: [V] }` is
  a family of object types: each instance (`Obj<Value>`, `Obj<i64>`) is an
  object type of its own, with its own type id and generated trace
  function, so `Obj<i64>`'s slots are not traced and `Obj<Value>`'s are.
  The trailing array may have a type parameter as its element type.
  `Gc<T>` may name a type parameter (`fn keep<T>(x: Gc<T>) -> Gc<T>`,
  `struct Pair<T> { a: Gc<T>, b: Gc<T> }`). There is no `Object` bound:
  like method calls in generic bodies, this is checked per instance, when
  T is bound. Binding T to something that is not an `object` is an error
  at the `Gc<T>` in the generic declaration, with a note pointing to the
  use that created the instance: "Gc<T> needs an `object` type, found
  `i64`".
- **`Gc<T>`** is a traced reference. A `Value` traces whichever variants hold `Gc`.
  It converts to a raw pointer or a `u64` address with `as`. The way back
  (`p as Gc<T>` from a pointer, a `u64`, or another `Gc`) is only allowed
  inside `unsafe { }`, since nothing checks that `p` points to a live `T`.
- **Write barriers.** Stores of `Gc`/`Value` into object fields are lowered
  through the heap policy's barrier, which is a compile-time parameter
  (`#![heap = std.gc.MarkSweep]`). Non-incremental mark-sweep has no barrier,
  so stores cost nothing. A generational policy adds a card-mark at each such
  store, and the source does not change. (**Not built:** the `heap`
  attribute and card marking. As implemented: Every such store, including `o.items.ptr[i] = v`,
  calls `std.gc.barrier(o)`, an object-logging barrier that tests one
  header byte, and a heap is made generational at run time with
  `heap.set_generational(true)`; see
  [`stdlib/gc.md`](stdlib/gc.md#as-implemented). Stores through other
  raw pointers into objects need `gc.write_barrier(h, obj)`, and
  `heap.set_verify(true)` finds the missing ones.)
- **Weak references** are library types: `heap.new_weak(x)` gives a
  `Gc<gc.Weak<T>>` that reads None once `x` died, and
  `gc.Ephemerons<K, V>` maps objects to values that only the key keeps
  alive. There is no `weak` field qualifier.
- **Roots.** These come from three places: the VM stacks (described by the
  `state` kinds and the `ctx.sync` area), compiler-managed temporary roots at
  effectful calls (section 5), and explicit `Root<T>` handles for host data
  structures. (As implemented: traced locals in shadow-stack frames, and
  root scanners registered with `heap.add_scanner`. **Not built:**
  `Root<T>`, the `ctx.sync` area and roots generated from state kinds.)
  A **state variable of a traced type** (a stack VM's cached top of stack,
  `state { .., tos: Value }`, or a current closure, `cl: Gc<Closure>`)
  is rooted by the compiler: neither `sync` nor a root scanner needs to
  know it. As built:
  - *Where.* At every call that may collect (`uses gc` or `reenter`, a
    `gc` function pointer or closure type) made from vm code: op
    handlers and their `cold` blocks (inline or outlined), `on raise`,
    variant wrappers, `-> never` vm functions, entries (also before the
    first dispatch), and plain vm functions. These are the calls that
    already run `sync` and link a shadow-stack frame for traced locals;
    the traced state joins that frame. A call that cannot collect, and
    every dispatch, pays nothing.
  - *How.* Each such variable is copied, at the call, into a stack slot
    of its own that the frame lists; the variable itself never has its
    address taken, so it stays in a register everywhere else in all three
    dispatch modes (in goto and switch modes, where the state lives in
    the one loop function, rooting the variable's own slot would have
    forced it into memory in every handler). In a vm function, which sees
    the state through pointers, the copy is read through them. A call of
    a vm function of the same vm roots no state: the function roots it at
    its own calls that may collect, so `fn gc_point() { if .. { cold {
    gc.collect(..) } } }` costs its callers nothing. Entries start with
    their traced state zeroed, so a variable that no one has assigned yet
    is a valid (empty) root.
  - *Nested runs and unwinding.* A `reenter` call keeps the caller's
    frame, and with it the copies, linked while the nested run executes,
    so the outer state survives collections in it; the nested run roots
    its own state the same way. `rt.catch_panic`, `rt.protect` and
    `#[catch_panic]` restore the saved shadow-stack top, which drops the
    frames (and copies) of the calls they unwound.
  - *Moving collectors* (not built) would also copy each slot back into
    its variable after the call, next to the reload of spilled locals.

  The lint `state_root`, which warned when `sync` did not store such a
  variable, is retired: `#[allow(state_root)]` is still accepted and does
  nothing. Code that stored the variable in `sync` and marked it in a
  scanner keeps working; the store and the mark can go.
- **Moving is opt-in.** The default `MarkSweep` is non-moving and
  size-segregated, which keeps interior pointers into the stack and objects
  stable across handlers. A moving policy makes the compiler reload spilled
  references after `gc` calls. **Not built:** moving policies.

Loimi uses compiler-managed roots instead of LLVM's statepoint/stackmap
machinery. Statepoints constrain the optimizer everywhere, while the effect
system confines rooting to the slow paths that can actually collect.

---

## 9. Instrumentation variants

```loimi
variant count wraps all {
    before { ctx.stats.ops[opcode] += 1; }
}
variant trace wraps all {
    before { ctx.tracer.on_op(opcode, pc, regs) }     // uses sync
}
variant debug wraps all {
    before { if ctx.breakpoints.has(pc) { cold { ctx.debugger.stop()? } } }
}
variant line_hook wraps [JMP, JLT, JLE, JEQ, CALL, RETURN] { ... }
```

Each variant generates a separate dispatch table of wrapper handlers, each
wrapping the original handler. Switching is a single register update
(`use_variant debug` in vm code, `use_variant default` to switch back), and
the new table takes effect at the next dispatch. (**Not built:**
`set_dispatch(Mini.tables.debug)` from outside the vm.) **Normal execution pays nothing**: no flag check in any handler, and
no second copy of the interpreter to maintain. This is how debuggers, profilers,
coverage, and `sys.settrace`-style hooks are implemented.

`loimi build --vm-stats` does not need a variant: it compiles op and
op-pair counters into the handlers and prints them at exit. The
program can read them too (`std.prof.op_counts`).

---

## 10. Calls, reentrancy, coroutines, embedding

**Not built:** `std.rt` (`FrameStack`, `Fiber`) and
`suspend`. A vm's `entry` functions are ordinary internal Loimi
functions (not C-ABI exports), and the example interpreters keep their
own frame stacks in vm state. Embedding (below) wraps entries in
`export "C"` functions the interpreter writes itself. Host-to-guest
calls are implemented as nested entries (below).

- **Guest-to-guest calls** should stay in the loop: push a frame (`std.rt.FrameStack`),
  slide the register window, and `jump`. This uses no C stack, so deep guest
  recursion is bounded by the VM stack, not the C stack.
- **Host-to-guest calls** (metamethods, callbacks, `sort` comparators)
  run a nested loop on top of the same VM stack, through an entry
  declared `uses reenter` (a *nested entry*):

  ```loimi
  entry call(c: *Ctx, at: *Frame, cl: Gc<Closure>) -> Result<V, V> uses reenter {
      ctx = c;
      fp = at;                       // a frame above the caller's, args in place
      fp.cl = cl; fp.nres = 1; k = cl.proto.consts.data.ptr; fp.k = k;
      regs = fp.base;
      c.base = fp;                   // the sentinel: RET from here exits
      c.fp = fp;
      jump cl.proto.code.ptr
  }
  ```

  - **It is an entry**: in tail-call mode, a fresh call that makes the
    first `preserve_none` call into the handlers and returns when one of
    them says `exit` or `fail`; in goto and switch modes, a call of the
    entry's own loop function (every entry has one, with all the
    handlers; recursive when runs nest). The nested loop runs on the C
    stack of the native that called it, and leaves the outer loop's
    state where the outer handler's `sync` put it.
  - **Its callers need `uses reenter`**: an ordinary function calling it
    declares the effect, and a handler calling it (or calling a native
    that may) gets `sync` before and `reload` after (section 5). An
    entry may declare no other effect.
  - **`Vm.depth`**: the compiler keeps a thread-local count of the nested
    runs active on the thread, one more while a nested entry runs (also
    when it returns early). Code that calls nested entries checks it to
    bound the C stack: Mini fails with "C stack overflow" past 200, as
    Lua does. It is per thread, not per vm state, because natives of one
    state may call into another on the same C stack.
  - **What the vm does itself**: where the new frame goes (above
    `ctx.fp`, which the caller's `sync` published), which frame is the
    sentinel (Mini's `RET` exits when `fp == ctx.base`, the bottom frame
    of the innermost run, instead of `ctx.frames`), restoring the
    caller's frame registers after the call, and turning an error into a
    result. The compiler does not know the frame layout, so these are
    the vm's code, typically one host function around the entry
    (Mini's `vm.call_value`).
  - **Errors**: `raise` in the nested run reaches that run's `on raise`,
    whose `exit`/`fail` returns from the nested entry; the native gets
    the `Result` and decides. If it re-raises, the outer `on raise` sees
    the error again: Mini marks errors that already carry their position
    (`ctx.rethrow`) so it does not prefix a second line number.
- **Not built:** **coroutines and generators** fall out of the explicit state: a coroutine is
  a separate `ValueStack` plus `FrameStack`, `suspend v` leaves the loop with
  state saved, and resuming re-enters through an entry that restores it.
  Suspending across a host frame is an error (as with Lua's "yield across a
  C-call boundary") unless the coroutine runs on a `std.rt.Fiber`, which has
  its own stack.

### 10.1 Embedding in a C host (implemented)

```sh
loimi build embed.lm --lib    -o libmini.a  --header mini.h   # static archive
loimi build embed.lm --shared -o libmini.so --header mini.h   # shared object
```

- **The API is the `export "C"` functions** of the program. A library
  needs no `main` and gets no `main` wrapper; `os.arg_count()` is 0.
- **No initialization call.** Every Loimi global has a constant
  initializer (static data), and the runtime's thread-locals (the
  shadow-stack top, vm scratch slots, `std.rt` handlers) start zeroed in
  every thread. A host can call any export from any thread at any time;
  the interpreter's own state (a heap, its roots) is created by an export
  such as `mini_new` and must be used by one thread at a time.
- **Code model.** `--lib` and `--shared` compile with `-fPIC`, so the
  archive links into PIE executables (the default) and shared objects.
  Executables keep `-fno-pic`.
- **TLS.** Runtime thread-locals are initial-exec in executables and
  `--lib` builds. `--shared` builds default to the local-dynamic model,
  so the object can be `dlopen`'ed whatever static TLS space is left
  (`--tls=initial-exec|local-dynamic|global-dynamic` overrides). The
  costs are in `lowering.md` §4.
- **AddressSanitizer.** `--sanitize=address` works with `--lib` and
  `--shared`: the library is instrumented, and the host links the ASan
  runtime by building with `-fsanitize=address`. Loimi instruments with
  clang but links executables against GCC's `libasan` when clang's own
  runtime (`libclang_rt.asan`, package `libclang-rt-19-dev`) is missing,
  which is the usual case on Debian; both speak the same
  instrumentation ABI. So `gcc -fsanitize=address host.c librx.a` works
  everywhere, and `clang-19 -fsanitize=address` works where clang's
  runtime is installed. A host that cannot be rebuilt can load a
  `--shared` library under `LD_PRELOAD=$(gcc -print-file-name=libasan.so)`.
  A library leaves ASan's options to the host (executables turn leak
  checking off by default; libraries do not). `--emit-llvm` with `--lib`
  gives IR you can also instrument yourself.
- **The header** (`--header out.h`) declares every export, in source
  order, with these C types:

  | Loimi | C |
  |---|---|
  | `i8`..`u64`, `f32`, `f64`, `bool` | `int8_t`..`uint64_t`, `float`, `double`, `bool` |
  | `()` result | `void` |
  | `*u8` | `void *` (Loimi's untyped byte pointer), `const void *` with `#[c_const]` |
  | `*T` | `T *` for any mappable `T` |
  | `*S`, `S` a struct | `S *`, with `typedef struct S S;` (opaque) |
  | `?*T`, `?fn(..)` | the pointer; `NULL` is `None` |
  | `fn(A) -> R` | `R (*)(A)`: a C function pointer, callable from Loimi |
  | `str` | `loimi_str`, `struct { const char *ptr; size_t len; }`, by value |
  | `enum E` without payloads | `typedef <tag type> E;` and constants `E_Variant` |

  Everything else is rejected with the reason: structs, tuples, arrays,
  `?T` and `Result` by value, other slices, enums with payloads, `value`
  types, and `Gc<T>` (the host would hold a reference the collector
  cannot see). A `str` parameter must fit in the first six integer
  argument registers: C passes a 16-byte struct whole, in two registers
  or on the stack, and LLVM would split one across both. The same checks
  run on every `--lib`/`--shared` build. `--header` also works on an
  executable.

  More in the header:

  - **Documentation.** The `///` comment above an export, an exported
    constant, or a struct or enum the header declares becomes a C
    comment above its declaration.
  - **Constants.** `export "C" const RX_SEARCH: u32 = 1;` is an ordinary
    constant in Loimi and `#define RX_SEARCH 1u` in the header (integers
    with a `u`/`LL`/`ULL` suffix by type, `bool`, floats, `str` as a
    string literal, and payload-free enum variants as `((E)n)`).
  - **`const` pointers.** Loimi has no `const`, so the header cannot
    tell which pointers an export only reads. `#[c_const(pat, buf)]` on
    the export declares those parameters `const T *` in C. It is a
    promise the Loimi side keeps by hand: nothing checks that the
    function does not write through them.
  - **C names.** A struct or enum is named as in Loimi, unless it has
    `#[c_name("rx_regex")]`, or `--header-prefix rx_` puts a prefix on
    every name without one (`rx_Regex`). Enum constants follow:
    `rx_Mode_Add`.
- **Recovering instead of exiting.** A library must not exit the host on
  a guest error. `std.rt.protect(f, data)` / `std.rt.throw()` unwind out
  of code with no error path (Mini's parser); see `stdlib.md`, `std.rt`.
- **Panics.** A failed bounds check, `unwrap` of None, a debug overflow
  check, division by zero, `trap()`, `rt.panic(msg)` and `mem.alloc` out
  of memory are panics. By default a panic prints its message on stderr
  and aborts the process, host included. An export can catch them
  instead:

  ```loimi
  #[catch_panic(-1)]
  export "C" fn rx_exec(r: *Regex, s: str, flags: u32) -> i32 { ... }
  ```

  A panic anywhere below the export on the same thread (in Loimi code,
  or in a Loimi export the host calls from a callback) returns `-1` from
  it, and `rt.panic_message()` gives the message (`rt.panic(msg)`'s is
  copied, its first 512 bytes, so it may be built in a buffer the
  unwinding abandons). The value is an
  integer, `true`/`false`, or a variant of the result enum; without one
  (`#[catch_panic]`) the result is zero: 0, false, NULL, an empty `str`,
  or nothing. `rt.catch_panic(f, data)` does the same inside Loimi.
  - **Cost.** A catching export installs a handler on each call: on
    x86-64 LLVM's builtin setjmp (three stores) plus saving the
    callee-saved registers and a few thread-local loads and stores,
    1-2 ns. Elsewhere it is glibc's `_setjmp`, a few ns. The regex
    library's `rx_exec`, called once per match, got 5-8% slower on its
    benchmarks with the most matches (one per 25 ns). Nothing else
    changes: panic sites were always out-of-line calls, and code without
    a catch compiles as before. Put the attribute on calls that do real
    work (compile, match), not on accessors called millions of times.
  - **What is left after a caught panic.** The frames between the panic
    and the catch are abandoned, as with `rt.throw`: their `defer`s do
    not run, and memory they allocated and had not yet stored somewhere
    reachable leaks. The shadow-stack top and the vm nesting depths
    (`Vm.depth`) are restored, so GC roots and re-entry counts are
    correct, also when the panic unwinds through nested vm runs and host
    callbacks (`tests/embed/panic.lm`, vm `R`). Everything else stays as the panic left it: a data
    structure that was being modified (a vm state, a `Vec` mid-`push`, a
    hash map mid-rehash) may break its own invariants. The safe rule is
    to treat the objects the failed call was modifying as poisoned: free
    them, or mark them failed and refuse further calls on them, and
    carry on with the rest. Thread-locals, globals the call did not
    touch, other objects and other threads are unaffected.
  - **Not caught.** A panic inside the collector itself (it does not
    panic, but out of memory in a GC heap aborts), stack overflow, a
    segmentation fault from an unchecked pointer, `--sanitize=address`
    reports, and, in executables, `trap()` (an illegal instruction there;
    in `--lib`/`--shared` builds `trap()` is a panic).
  - **Host frames.** A panic in a nested export without its own catch
    unwinds through the host's frames to the catching export below, like
    `longjmp`: host code in between must not hold resources it needs to
    release (C++ destructors do not run).
  - **Threads.** Handlers are per thread. A panic on a thread with no
    catch aborts, even if another thread has one.

**Re-entry.** A C function registered as a native runs inside a
handler's `cold` block. Its type in Loimi says so: Mini stores natives
as `fn(*MiniState, *u8) -> i32 uses reenter`, so the call through the
pointer has the effect, and the handler syncs before it and reloads
after (section 5). The header's C type is the plain function pointer:
C cannot see effects, and needs none, because the Loimi side assumes
the worst. A native may then:

- call a Mini function: `mini_call(S, nargs)` calls the function below
  `nargs` arguments on the state's value stack (pushed with
  `mini_push_global`, `mini_push_arg`, `mini_push_number`, ...), as a
  nested run above the native's caller (`vm.call_value`, the nested
  entry `Mini.call`), and pushes the result, or the error value with
  `MiniStatus_Runtime`. `mini_raise_top` re-raises that error unchanged;
- run a chunk: `mini_dostring` from a native is a nested run too;
- call into another state: a run of that state, counted in the same
  thread-local `Mini.depth`.

C never holds a Mini value: values stay on the value stack, a GC root
that `mini_call` and natives use by position (-1 is the top), and what a
native pushes is popped when it returns. Strings from `mini_to_string`
stay valid while their value is on the stack. More than 200 nested runs
on a thread fail with "C stack overflow". The one thing a native must
not do is free its own state. The host can also call Mini functions
between runs, with the same `mini_call`.

The worked example is `examples/mini/embed.lm` (the Mini interpreter as
a library, reusing `vm.lm`, `compile.lm` and `value.lm` through
`state.lm`) with `examples/embed/host.c`: two states alive at once,
globals both ways, C natives with user data, syntax and runtime errors
with messages, natives calling back into Mini (nested 50 levels through
Mini and C, past the limit, with errors passing through, under GC
stress, and into another state), the host calling Mini functions, a
GC-stress run through the API, and an internal error in a nested run.
`mini_new`, `mini_dostring` and `mini_call` are `#[catch_panic]`: a
panic in the interpreter returns NULL or `MiniStatus_Internal`, and
poisons the state, following the rule above: the state counts its API
calls in progress and its natives running, which agree in C code
unless a panic abandoned a call. A poisoned state refuses to run code,
`mini_error` gives the panic's message, and a native returning to a
run of it re-panics so that the outer run stops too. Its handlers
compile to the same IR as without the catches.

---

## 11. What Loimi deliberately leaves out

- **No optimizing JIT.** Each handler is a separate LLVM function with a known
  register interface, which is exactly the input a copy-and-patch baseline JIT
  needs (CPython 3.13 builds its JIT from LLVM-compiled templates); a
  prototype of one exists (`--jit-stencils`, `status.md`). See `roadmap.md`.
- **No borrow checker or ownership types.** Arenas and the GC cover
  interpreter workloads. The complexity budget goes to the hot loop.
- **No exceptions or unwinding in host code.** Guest exceptions are a VM
  concept (`raise` / `on raise`). Host errors are `Result`.
- **No implicit runtime.** No hidden allocations, no implicit GC outside `std.gc`.
