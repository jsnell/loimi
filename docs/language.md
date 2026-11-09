# The Loimi language

The reference for the language as the compiler implements it: syntax,
rules and options, with short examples. It is for interpreter authors
looking something up. To learn the language step by step, start with
the [tutorial](tutorial/README.md).

**Contents**

- [0. Principles](#0-principles)
- [1. Core language](#1-core-language)
  - [1.1 Types](#11-types)
  - [1.2 Basics](#12-basics):
    [lexical structure](#lexical-structure),
    [bindings, loops and types](#bindings-loops-and-types),
    [variant names and data](#variant-names-and-data),
    [patterns and `match`](#patterns-literals-and-match),
    [warnings, attributes and tests](#warnings-attributes-and-tests),
    [generics and traits](#generics-and-traits),
    [closures](#closures),
    [operators and arithmetic](#errors-operators-and-arithmetic),
    [vectors](#vectors)
  - [1.3 Memory](#13-memory)
  - [1.4 Compile-time evaluation](#14-compile-time-evaluation)
  - [1.5 Modules and interop](#15-modules-and-interop)
- [2. `value` types: declared representations](#2-value-types-declared-representations)
- [3. The `vm` declaration](#3-the-vm-declaration)
  - [3.1 `state` and `context`](#31-state-and-context)
  - [3.2 Operand kinds](#32-operand-kinds)
  - [3.3 Encodings and formats](#33-encodings-and-formats)
  - [3.4 Ops and control words](#34-ops-and-control-words)
  - [3.4a Stack effects (stack VMs)](#34a-stack-effects-stack-vms)
  - [3.5 VM functions](#35-vm-functions)
  - [3.6 Entries and the error path](#36-entries-and-the-error-path)
  - [3.7 Dispatch strategies](#37-dispatch-strategies)
- [4. Slow paths: `cold` blocks](#4-slow-paths-cold-blocks)
- [5. Effects and sync points](#5-effects-and-sync-points)
  - [5.1 `reenter` and the cached state](#51-reenter-and-the-cached-state)
  - [5.2 Effects on function types](#52-effects-on-function-types)
- [6. Verification: unchecked by proof](#6-verification-unchecked-by-proof)
- [7. Specialization: quickening, inline caches, superinstructions](#7-specialization-quickening-inline-caches-superinstructions)
  - [7.1 Inline caches](#71-inline-caches)
  - [7.1a The following instruction: `then`](#71a-the-following-instruction-then)
  - [7.2 Families and `become`](#72-families-and-become)
  - [7.3 Superinstructions](#73-superinstructions)
  - [7.4 Accumulators](#74-accumulators)
  - [7.5 Relations and data on ops: `rel` and `data`](#75-relations-and-data-on-ops-rel-and-data)
- [8. GC integration](#8-gc-integration)
- [9. Instrumentation variants](#9-instrumentation-variants)
  - [9.1 Breakpoints: `on break`](#91-breakpoints-on-break)
- [10. Calls, reentrancy, coroutines, embedding](#10-calls-reentrancy-coroutines-embedding)
  - [10.1 Embedding in a C host](#101-embedding-in-a-c-host-implemented)
- [11. What Loimi deliberately leaves out](#11-what-loimi-deliberately-leaves-out)

## 0. Principles

1. **The hot loop is the product.** A feature earns its place by improving the
   machine code of interpreter handlers, or by removing a class of bugs
   specific to interpreters. General-purpose niceties come second.
2. **One source, many dispatch strategies.** Tail-call threading, computed
   goto, and `switch` are build options, not rewrites.
3. **Declare, don't encode.** Instruction formats, value representations,
   object layouts and inline caches are declarations. The compiler
   generates the bit manipulation, and also the tools that must agree
   with it: encoders, disassembler, verifier, peephole and
   deoptimization passes.
4. **Make the slow path explicit, cheap, and correct.** Slow paths are
   outlined so they don't hurt register allocation. Effects ensure the runtime
   always sees consistent VM state when it needs to.
5. **Safe by default, unchecked by proof.** Bounds checks are removed because
   a verifier proved them redundant, not because someone wrote `unsafe`.
6. **Stock LLVM only.** Every feature lowers to standard LLVM IR, attributes,
   and calling conventions, so the language benefits from LLVM's optimizer and
   targets for free.

## 1. Core language

The core is a small, conventional, statically typed systems language at
roughly C's level of abstraction.

### 1.1 Types

<!-- snippet: skip summary of the type syntax, not a program -->
```loimi
i8 i16 i32 i64 isize   u8 u16 u32 u64 usize   f32 f64   bool   never
iN uN           // any width 1..=64: i48, u24, i61, ... (see below)
i8x16 u8x16 i16x8 u16x8 i32x4 u32x4 i64x2 u64x2 f32x4 f64x2   // 128-bit vectors (section 1.2, "Vectors")
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

Plus `struct`, `enum` (tagged unions), `object` (GC-managed structs,
[section 8](#8-gc-integration)) and `value` ([section 2](#2-value-types-declared-representations)).
Bitfields are read with shifts and masks, or with a vm's generated
field readers ([section 3.3](#33-encodings-and-formats)).

### 1.2 Basics

<!-- snippet: check
type Symbol = u32;
{snippet}
-->
```loimi
fn gcd(a: u64, b: u64) -> u64 {
    var (x, y) = (a, b);
    while y != 0 { let r = x % y; x = y; y = r; }
    x
}

struct Span { start: u32, end: u32 }

enum Token { Num(f64), Ident(Symbol), Punct(u8), Eof }
```

#### Lexical structure

- **Comments**: `// ..` to the end of the line, and `/* .. */`, which
  nest.
- **Integer literals**: decimal, `0x`, `0o` or `0b`. `_` may separate
  digits (`1_000_000`).
- **Float literals** have a fraction or an exponent: `1.5`, `2e-3`.
  (`1..2` is a range.)
- **Character literals** (`'a'`, `'\n'`) are byte values, that is,
  integers. They take the same escapes as strings.
- **String literals** (`"..."`) have type `str` and may span lines.
  Escapes: `\n`, `\t`, `\r`, `\0`, `\\`, `\'`, `\"` and `\xHH`.
- **Raw string literals** `r"..."` process no escapes, so text in
  another syntax (a regular expression, a grammar) is written as is.
  `r#"..."#` may contain `"`: it ends at `"` followed by as many `#`s as
  it began with (`r##"say "#hi"#"##`). Raw strings may span lines and
  have type `str`.
- **Reserved words** cannot be used as names:

  ```text
  fn let var if else while loop for in return break continue struct
  enum const import extern export match as true false and or pub defer
  unsafe never
  ```

  `in` is among them: write `in_` or `input` instead. The control words
  (`next`, `jump`, ..., [section 3.4](#34-ops-and-control-words)) and
  words like `vm`, `op`, `state` and `is` are not reserved.

#### Bindings, loops and types

**Bindings.** `let` declares an immutable binding, `var` a mutable one.
Blocks are expressions.

**Globals.** `var NAME: T = init;` at module level declares a global.

- `init` must be a constant expression: literals, consts, `[x; N]`,
  aggregate and enum literals of constants, functions, and `0 as *T`
  (a null pointer).
- `#[thread_local] var` gives each thread its own copy, for things like
  "the current VM" or per-thread caches.
- `pub var` is visible to other modules.
- Globals are not GC roots.

**Type aliases.** `type Name = T;` (or `pub type`) gives a type a second
name. `type Map<V> = HashMap<str, V>;` takes type parameters.

- An alias is transparent: `Name` *is* `T`. Values convert freely,
  methods of `T` apply, and `fn Name.m(..)` defines a method of `T`.
- The aliased type is resolved in the alias's module and sees only the
  alias's own parameters. Aliases may refer to other aliases, in any
  order, but not to themselves.
- A non-generic alias can also stand for its type in expressions:
  `Name { .. }`, `Name.f(..)`, `Name.Variant` (also in patterns).
- Generic aliases are for type positions only (write `HashMap.new()`,
  not `Map.new()`), and their parameters take no bounds: the aliased
  type's bounds are checked where it is instantiated.
- Aliases are imported like other items (`import m.{Name}`, `m.Name`).

**Range loops.** `for i in lo..hi` counts from `lo` up to `hi - 1`.

- The range takes the type of a bound that is not a literal: `0..n` with
  `n: u64` is a `u64` range, `m..255` with `m: u8` a `u8` range.
- When both bounds are literals (or `Kind.COUNT`), the type comes from
  an annotation on the variable, `for i: u8 in 0..10`, and is `i64`
  without one. The bounds must fit the type. Later uses of the variable
  are not looked at.

**Element loops.** `for x in e`, with `e` an array or a slice, runs the
body with `x` bound to a copy of each element in order: `for row in
table() { let (op, k) = row; .. }`. `break` and `continue` work.

- An array that is a place (a variable, a field, `p.xs`) is read in
  place; an array returned by a call is stored in a temporary first.
- The loop variable is a name, not a pattern.

**Integer-to-enum casts.** `x as Kind` converts an integer to an enum
without payloads (decoding an opcode byte, say).

- The value must equal one of the discriminants. Debug builds check
  this and panic, naming the cast. Release builds don't check, and an
  invalid value is undefined behavior for any `match` on the result.
  A constant operand is checked at compile time.
- `Kind.COUNT` is the number of variants, typed like an integer literal:
  `[u32; Kind.COUNT]`, `for i in 0..Kind.COUNT { let k = i as Kind; }`.
  A variant named `COUNT` takes precedence. With explicit discriminants,
  the count is not the largest discriminant plus one.

**Type inference** is local.

- *Across branches.* Without an annotation, the branches of `if` and
  `match` are typed against each other: `let r = if c { Ok(e) } else {
  Err(m) };` gets `Result<typeof e, typeof m>`. An arm that cannot be
  typed alone (`None`, `Ok(x)`) takes its type from the others.
- *Not from later uses.* `var r = Ok(false);` needs an annotation
  (`var r: Result<bool, str> = Ok(false);`), and the error says so.
- *Literals* take their type from the other operand: in
  `(16 + 8 + 8 * n) >> 4` with `n: u64`, `16 + 8` is `u64`. Without
  another operand or an annotation a literal is `i64`. The same goes
  for an `if`, `match` or block whose every result is a literal:
  `(if c { 1 } else { 0 }) == y` with `y: i32` compares `i32`s.
- *Array lengths.* `let t: [(str, u32); _] = [..];` takes the length
  from the array literal.

**`<` after `as`.** `x as u64 < y` is a comparison: a primitive type
never takes type arguments. After any other type name, `<` begins type
arguments only when a balanced `<...>` of type tokens follows
(`p as *Vec<T>`). When in doubt, write `(x as T) < y`.

#### Variant names and data

An interpreter's builtins, exception kinds and operators are often
enums with tables beside them: name to variant, variant to arity or
message. Attributes put the tables on the variants, and the compiler
generates the lookups:

<!-- snippet: run "car 1 1" "+ 0 -1" "Some" -->
```loimi
import std.io;

/// Builtins by the id a value holds: the Scheme name, the argument
/// counts (-1: any) and whether the compiler inlines it.
#[data(min: i32 = 1, max: i32 = 1, inline: bool = false)]
enum Prim: i32 {
    #[name("+"), data(0, -1, inline = true)] Add,
    #[name("car"), data(inline = true)] Car,
    #[name("vector-ref"), data(2, 2)] VectorRef,
    #[name("display"), data(max = 2)] Display,
}

fn main() {
    for id: i32 in 0..Prim.COUNT {
        let p = id as Prim;
        io.print(p.name()); io.print(" ");
        io.print_int(p.min() as i64); io.print(" ");
        io.print_int(p.max() as i64); io.println("");
    }
    io.println(match Prim.from_name("car") { Some(_) => "Some", None => "None" });
}
```

**Names.** `#[name("..")]` on a variant, or `#[names]` on the enum,
generates:

- `E.name(k) -> str`;
- `E.from_name(s: str) -> ?E`, for an enum without payloads.

A variant without `#[name]` is named as written (`#[names] enum Exc {
TypeError, ValueError }` names them `"TypeError"` and `"ValueError"`).
Two variants may not have the same name.

**Data.** `#[data(key: Type, key: Type = default, ..)]` on the enum
declares keys. `#[data(..)]` on a variant gives its values, first by
position (in the keys' order), then as `key = value`.

- A key without a default must be given by every variant.
- Each key generates an accessor `E.key(k) -> Type` (`p.min()`).
- Values are constants: literals, names of constants and variants, and
  constructors, tuples and arrays of them (`Some(Op.Ge)`, `(4, true)`).
  A value of the wrong type is reported at the value.
- `name`, `from_name` and `COUNT` cannot be keys, and a hand-written
  `fn E.name` (or `fn E.key`) is an error.
- Generic enums take neither names nor data.

**Cost.** The functions are compiled only if something calls them.
`name` and the accessors are inlined `match`es that return constants,
so LLVM turns them into a table load, or folds them for a known variant
(`Prim.Car.min()` is `1`). `from_name` is a string `match`.
`LOIMI_DUMP_VM_EXPANSION=1` prints the generated functions (as `<enum
E expansion>`).

#### Patterns, literals and `match`

`match` is exhaustive. `if let`, `let ... else` and `while let` are
supported.

**Pattern forms**

| Pattern | Matches | Notes |
|---|---|---|
| `0x45`, `-1`, `'a'` | an integer literal | A pattern outside the scrutinee's type is an error. |
| `0x28..=0x3E`, `'a'..='z'` | an inclusive integer range | |
| `"dup"` | a string, on a `str` | |
| `Nil`, `Pair(a, _)` | a variant | |
| `LIMIT`, `m.LIMIT`, `Vm.OP_ADD` | a constant's value | See [constants in patterns](#constants-in-patterns). |
| `A \| B` | either alternative | May bind only at the top of a `match` arm. |
| `x`, `_` | anything | `x` binds. |

Integers and strings count as covered only by `_` or a binding.

**Match guards.** `pat if cond => body` matches only when `cond` (which
sees the pattern's bindings) is true; otherwise the later arms are
tried. Guarded arms do not count toward exhaustiveness.

**Alternatives that bind.** When `A | B` is the whole pattern of an arm,
it may bind names: `And(a, b) | Or(a, b) if a > 0 => a + b` is one arm
per alternative, each with the guard and the body. So the body is
compiled once per alternative, and each alternative must bind what the
body uses. Nested alternatives (`Some(A(x) | B(x))`) may not bind.

##### Constants in patterns

A name in a pattern is, in this order:

1. a variant of the type being matched (`Nil`, `Red`);
2. a constant in scope: a `const` of this module, one imported with
   `import m.{LIMIT}` (or written `m.LIMIT`), or a vm's generated
   constant such as `Vm.OP_ADD`;
3. otherwise, a new binding.

A constant matches its value, as if its literal stood there: `match loc
{ LOCAL => .., CONST => .., _ => .. }`.

- Integer, `bool`, `str`, enum and tuple constants qualify. Float
  constants are an error (there are no float patterns: NaN and `-0.0`
  make equality the wrong test), and so is a constant of another type
  than the scrutinee.
- A local variable shadows a constant of the same name (the pattern
  binds again), and `var X` always binds.
- Arms after one that matches every value (a binding or `_`, without a
  guard) can never run and get the warning `unreachable_arm`. When that
  arm is a binding, the note says so. This catches a misspelled or
  unimported constant: `LOCAL =>` without `const LOCAL` in scope binds
  `LOCAL` to anything.

##### Testing and unwrapping: `is` and `as!`

**`e is P`** tests a pattern without binding: `t is Ref`, `v is Int |
Float`, `x is Pair(0, _)`, `k is 0..=9`.

- It is a `bool` at the precedence of `==`, so `a is A and b is B`
  needs no parentheses, and `!(v is Nil)` does.
- A variant with fields may be named alone at the top of the pattern or
  of an alternative: `t is Ref` means `t is Ref(_)`. Nested, write the
  fields: `o is Some(Int(_))`.
- The pattern may not bind. A name that is not a variant or a constant
  is an error (so a misspelled variant is not a binding that always
  matches), and so is a pattern that matches every value.
- `is` is not a reserved word: it has this meaning only after an
  operand.

**`e as! V`** unwraps variant `V` of `e`, or fails: `let s = name as!
Str;`.

- It yields the field, or a tuple of several fields: `let (a, b) = p
  as! Pair;`.
- `V` is named as in a pattern (`Str`, `Value.Str`, `.Str`). `Some` and
  `Ok` work on options and results.
- It binds like `as`, so `x.f as! Str` needs no parentheses.
- On another variant: in a `--release` build it is `trap()`, the same
  code as `let Str(s) = name else { trap() };`. In a debug build it is
  a panic that names the variant and the place (`main.lm:12:13: as! Str
  of another variant`).
- A variant without fields is an error (test it with `is`).

Use `as!` where the variant is an invariant (a verifier or an earlier
type check guarantees it). For a guest-language type error, use `let ..
else { raise .. }`.

<!-- snippet: run "true 12" -->
```loimi
import std.io;
enum Term { Ref(i64), Atom(u32), Pair(i64, i64), Nil }
fn is_var(t: Term) -> bool { t is Ref }
fn main() {
    let p = Term.Pair(5, 4);
    let (a, b) = p as! Pair;
    let r = Term.Ref(3) as! Ref;
    io.print(if is_var(Term.Ref(1)) and !(p is Atom | Nil) { "true " } else { "false " });
    io.print_int(a + b + r);
    io.println("");
}
```

##### Strings in `match`

On a `str`, arms may be string literals: `match name { "dup" => ..,
"swap" | "exch" => .., _ => .. }`. A run of string arms at the start of
a `match` compiles to a `switch` on the length, then compares of the
bytes in 8-, 4-, 2- and 1-byte loads. Elsewhere (inside tuples, after a
guarded arm, in `if let`) each pattern tests the length and the bytes
in turn.

##### A false `if` in an arm falls out of the `match`

Once a pattern matches, its arm is chosen. An `if` without `else` in
the arm's body that is false falls out of the whole `match`, as in
Rust; it does not try the later arms. A guard does:

<!-- snippet: check
value Value repr nanbox { Num(f64) unboxed, Int(i32), Nil }
vm V {
    state { pc: *u32, regs: *Value }
    operand reg = regs[it];
    encoding fixed(u32) { format ABC { op: 8, a: 8, b: 8, c: 8 } }
    op DIV(dst: reg, a: reg, b: reg) : ABC {
{snippet}
        raise "division by zero"
    }
    entry run(code: *u32, r: *Value) -> Result<Value, str> { regs = r; jump code }
    on raise(e: str) { exit Err(e) }
}
-->
```loimi
match (a, b) {
    (Int(x), Int(y)) if y != 0 => { dst = Int(x / y); next }   // y == 0: tries the next arm
    (Int(x), Int(y)) => if y != 0 { dst = Int(x / y); next },  // y == 0: continues after the match
    (Int(x), Num(y)) => { dst = Num(x as f64 / y); next }
    _ => {}
}
```

In vm code (op handlers, `-> never` vm functions, entries, `on raise`)
the code after a `match` is usually the slow path, so falling out by
mistake silently skips the arm meant for the case. The compiler warns
(`match_fallout`) when an arm's body is an `if` without `else` that
ends in a control word, and a later arm with a non-empty body could
match some of the same values.

- Use a guard to try the later arms.
- Write `else {}` to say that falling out is intended.
- Arms followed only by `_ => {}` do not warn.

##### Other literal forms

**`..zero` in struct literals.** `Machine { heap: gc.Heap.new(), depth:
3, ..zero }` sets every field not given to zero. It also works in
`const` and global initializers. Allowed only for fields where zero is
a value that needs no constructor:

| Field type | Zero is |
|---|---|
| integers, floats, `bool` | `0`, `0.0`, `false` |
| `?T` (including `?*T`, `?Gc<T>`, `?fn(..)`) | `None` |
| slices | empty |
| raw pointers `*T` | null (dereferencing it crashes, as in C) |
| arrays and tuples of the above | all zero |

Fields of type `Gc<T>` or `fn(..)`, structs (their constructor may set
invariants), enums, `value` types and private (`__`) fields must be
given; the error names the field, and suggests `?Gc<T>` or `?fn` for a
reference that starts empty. A null `*T` is allowed because `0 as *T`
already makes one without `unsafe`; use `?*T` for a pointer that is
*meant* to be absent.

**Empty slices.** `[]` where a slice type is expected is the empty
slice: `let s: []u32 = [];`, `f([])`, `x.words = [];`.

#### Warnings, attributes and tests

##### Errors

A build reports every independent error, in source order, up to
`--max-errors N` (default 20, `0` for all).

- Checking goes on past an error in a function body, a vm handler or a
  statement.
- A binding whose initializer failed is not reported again where it is
  used.
- Errors in imports, types and signatures stop the build, since what
  follows depends on them.

This program gets two errors, `p.z` and the `str` assigned to `n`, and
none for `a`:

<!-- snippet: fail "error: 2 errors" -->
```loimi
struct P { x: i64 }
fn f(p: P) -> i64 {
    let a = p.z;
    let n: i64 = "one";
    a + n
}
fn main() {}
```

##### Warnings

Warnings are named: `warning[match_fallout]: ..`.

- `#[allow(name)]` silences one on a match arm, an item (function,
  `vm`, ...) or a vm item (`op`, `family`, vm function, entry), covering
  everything inside it.
- `#![allow(name)]` at the top of a file covers the file.
- An unknown name is an error.
- `loimi build -W error` (or `-Werror`, `--deny-warnings`) turns the
  remaining warnings into errors.

| Lint | Reports | On by default |
|---|---|---|
| `match_fallout` | an arm whose `if` may fall out of a `match` ([above](#a-false-if-in-an-arm-falls-out-of-the-match)) | yes |
| `unreachable_arm` | an arm after one that matches everything ([above](#constants-in-patterns)) | yes |
| `super_overlap` | two superinstructions that overlap ([section 7.3](#73-superinstructions)) | yes |
| `value_layout` | variants narrowed without a pin ([section 2](#2-value-types-declared-representations)) | yes |
| `next_pc_then` | `next_pc` in an op with `then` ([section 3.5](#35-vm-functions)) | yes |
| `state_registers` | more state than fits in registers ([section 3.1](#31-state-and-context)) | yes |
| `redo_effect` | an effect before a `redo` ([section 4](#redo-and-what-may-come-before-it)) | an error, not a warning |
| `hot_gc` | a call that may collect on an op's fast path | no: `-W hot_gc` |
| `handler_frame` | a tail-call handler whose fast path sets up a stack frame | no: `-W handler_frame` |

- **`hot_gc`** reports a `uses gc` call outside `cold` on an op
  handler's fast path. Such a call syncs the vm state and roots the
  live traced locals on every execution. That is often right, as for
  an allocation the op exists to make: say so with
  `#[allow(hot_gc)]`.
- **`handler_frame`** checks the generated assembly and names the calls
  in the handler that can cause the frame ([section 4](#4-slow-paths-cold-blocks)).
  A panic is named by what it checks and where ("the bounds check of an
  index at value.lm:224"); when LLVM merged several panic calls, the
  warning lists the candidates. It compiles the IR to assembly a second
  time, which roughly doubles clang's share of the build time.
- **`redo_effect`** is an error unless an `#[allow(redo_effect)]`
  covers the `redo` or the effect.
- `#[allow(state_root)]` names a retired lint. It is still accepted and
  does nothing.

##### Attributes

Attributes are checked where they appear. An unknown or misplaced one
is an error (with a "did you mean" for near misses), never ignored, so
a misspelled `#[require_verified]` cannot silently drop a check.

| Attribute | Goes on | See |
|---|---|---|
| `#[allow(..)]` | anything: items, vm items, fields, enum variants, match arms, `#![..]` | [warnings](#warnings) |
| `#[inline]`, `#[inline(always)]`, `#[inline(never)]`, `#[cold]` | functions (also extern, trait and vm functions) | |
| `#[unsafe]`, `#[raw]` | functions | [1.3](#13-memory) |
| `#[test]`, `#[track_caller]` | functions | [tests](#tests) |
| `#[catch_panic]`, `#[c_const(..)]` | `export "C"` functions | [10.1](#101-embedding-in-a-c-host-implemented) |
| `#[c_name("..")]` | structs and enums named in a C header | [10.1](#101-embedding-in-a-c-host-implemented) |
| `#[name(..)]`, `#[names]`, `#[data(..)]` | enums and their variants | [variant names and data](#variant-names-and-data) |
| `#[trace(f)]` | an `object` | [8](#8-gc-integration) |
| `#[trace]` | a plain struct, with `#[untraced]` on its fields | [8](#8-gc-integration) |
| `#[thread_local]` | a module-level `var` | [bindings](#bindings-loops-and-types) |
| `#[cfg(..)]` | items, fields, statements, vm items | [1.4](#14-compile-time-evaluation) |
| `#[require_verified]` | a `vm` | [6](#6-verification-unchecked-by-proof) |
| `#[dispatch(mode)]`, `#[profile("file")]` | a `vm` | [3.7](#37-dispatch-strategies) |
| `#[forward]` | a `super`, `supers` group or `supers from` | [7.3](#73-superinstructions) |
| `#[accumulate]` | an op | [7.4](#74-accumulators) |
| `#[poll(..)]` | an op or `family` | [4](#4-slow-paths-cold-blocks) |
| `#[unfused]` | a `variant` | [9](#9-instrumentation-variants) |
| `#[hot]` | a `raise`, `fail` or `redo` | [4](#4-slow-paths-cold-blocks) |
| `#![safe]`, `#![lifetimes(off)]` | a file | [1.3](#13-memory) |

##### Tests

`#[test]` marks a function `fn name()` (no parameters or result; it may
declare effects) as a unit test. In a normal build it is an ordinary
function.

```sh
loimi test FILE.lm [build options] [FILTER..] [--list] [--update]
```

- Builds `FILE.lm` with its `main` replaced by one that runs the file's
  `#[test]` functions (not those of the modules it imports) through
  `std.test`'s `Runner`, and runs it in the file's directory.
- Each test runs under a panic catch, so a failure stops only that
  test. The exit status is 1 if any test failed.
- Filters select the tests whose name contains one of them.
- `--update` makes `test.golden` rewrite its files (see `std.test` in
  [stdlib.md](stdlib.md)).

**`#[track_caller]`** on a function makes each call store the call's
location (`"file.lm:12:5"`) in a thread-local before calling;
`test.caller()` reads it. Inside a `#[track_caller]` function, the
calls it makes pass on its own caller's location instead, so an assertion
helper reports the line of the test that called it. Calls of other
functions pay nothing.

#### Generics and traits

Generics are monomorphized: code is generated per instance, and calls
are direct (static dispatch, no `dyn`).

**Type arguments** are inferred from the arguments and the expected
type, or given explicitly: `mem.alloc<u8>(n)`. Methods can have their
own type parameters: `fn Vec<T>.map<U>(v: *Vec<T>, f: fn(T) -> U) ->
Vec<U>`.

##### Traits

A trait states what a generic parameter needs:

<!-- snippet: skip excerpt: std/hash.lm declares the trait; the rest shows bounds with elided bodies -->
```loimi
pub trait Hash {
    fn hash(k: Self) -> u64;
    fn eq(a: Self, b: Self) -> bool;
}
impl Hash for Sym;          // checks that fn Sym.hash and fn Sym.eq exist
pub struct HashMap<K: hash.Hash, V> { .. }
fn show_all<T: Show + Hash>(xs: []T) { .. }
```

- A trait lists method signatures over `Self`.
- `impl Trait for Type;` contains no code. It asserts that the ordinary
  methods `fn Type.name` exist with the trait's signatures (`Self`
  replaced by `Type`) and no more effects than the trait declares.
- Bounds `T: A + B` on the type parameters of structs, enums, functions
  and methods are checked when an instance with concrete types is
  created. A missing impl is reported there: "`Key` does not implement
  `Hash` (needed by `HashMap<Key, i64>`)".
- A method of a generic type has the type's bounds without restating
  them (`fn HashMap<K, V>.get` may call `key.hash()`).

Traits can also have default methods, type parameters, generic methods
and static functions:

<!-- snippet: check
import std.io;
import std.mem.{Vec};
struct Sym { id: u32 }
fn Sym.show(s: Sym) { io.print_int(s.id as i64); }
fn Sym.conv(s: Sym) -> i64 { s.id as i64 }
fn Vec<T>.show(v: Vec<T>) {}
{snippet}
impl Show for Sym;
fn main() { Sym { id: 5 }.show_line(); }
-->
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
fn total<T: Conv<i64>>(xs: []T) -> i64 { var t = 0; for x in xs { t += x.conv(); } t }
```

- **Default methods** have a body. An impl takes the type's own method
  if there is one (checked against the signature), and the default
  otherwise. The default is instantiated for the type, so
  `sym.show_line()` and `x.show_line()` on a bounded `x` both work.
- **Generic traits** take type arguments in bounds and impls
  (`T: Conv<i64>`, `P: parse.Hooks<K, E, X>`); the arguments may name
  other parameters. A type may implement one trait with different
  arguments.
- **Generic trait methods** have their own type parameters. The type's
  method must have as many (`fn Range.fold<U>`), with the same
  signature.
- **Generic impls** `impl<T: B> Trait for Type<T>;` cover every instance
  whose arguments satisfy the impl's bounds, checked when an instance is
  needed (`Vec<f64>` is not `Show` if `f64` is not). The methods are
  the generic type's (`fn Vec<T>.show`). Each parameter of the impl
  must appear in the type.
- **Static functions** (no `Self` parameter) are called on the type:
  `T.zero()` for a bounded `T`.

**Impls do not overlap.** Two impls of one trait (with the same trait
arguments) may not apply to the same type:

- `impl Show for Vec<i64>;` next to `impl<T: Show> Show for Vec<T>;` is
  an error ("overlapping impls of `Show`: both apply to `Vec<i64>`").
- So are two generic impls whose types unify (`Pair<T, i64>` and
  `Pair<str, T>`).
- Bounds are not consulted, so a new impl of a bound's trait cannot
  make two existing impls start to overlap.
- There is no orphan rule: a program is compiled as a whole, so an impl
  may sit in any module that can name the trait and the type.

**Generic bodies are checked against their bounds.** Inside
`fn f<T: Show>(x: T)`, `T` has only what its bounds give it: `x.show()`,
the defaults of `Show`, and calls to functions whose bounds `T` meets.

- `x.extra()` is an error ("no method `extra` on type parameter `T`: its
  bounds (`Show`) do not provide it"), even if every type `f` is used
  with has an `extra` method. So are field accesses, operators, and
  passing `x` where a concrete type is expected.
- A call through a bound has the effects the trait declares.
- Parameters *without* bounds keep duck typing: method calls on them
  resolve per instance. The check runs once per combination of the
  unbounded parameters' arguments.

##### Generic enums and generic functions as values

**Variants of generic enums** can be qualified with the enum's name
alone. With `enum Lookup<O> { Own(u32), Proto(O, u32), Missing }`:

- `Lookup.Missing` and `Lookup.Own(i)` take their instance from the
  expected type (`return Lookup.Missing;` in a function returning
  `Lookup<O>`), or else from the payload (`Lookup.Proto(2.5, 7)` is a
  `Lookup<f64>`). `let l = Lookup.Missing;` is an error.
- In patterns, `Lookup.Own(x)` matches any instance.
- `mod.Lookup.Missing` works too.

**Generic functions as values.** A generic function used where a
function type is expected gets its type arguments from that type
(`Pratt.new(peek, ..)`).

- Explicit type arguments name an instance anywhere:
  `heap.add_scanner(&roots as *u8, scan<Gc<Box>>)`,
  `let f = size_of_t<u32>;`. Give all of the function's type
  arguments. This does not work on a generic type's functions.
- `f<T>` without a call is read as type arguments when the `<` directly
  follows the name and the `>` is followed by `)`, `,`, `;`, `}` or `]`.
- Passed to another generic function, the parameter's type may be only
  partly known (`fn apply<T, U>(f: fn(T) -> U, x: T) -> U`). The other
  arguments are checked first; the known parts (here `T`) are enough if
  they determine the function's type arguments, as in `apply(double,
  21)` with `fn double<N>(x: N) -> N`.

#### Closures

There are two kinds of closures:

| Type | Can be stored or returned | Captures | Memory |
|---|---|---|---|
| `block fn(A) -> R` | no: parameters and locals only | by reference | none (in the caller's frame) |
| `closure fn(A) -> R` | yes | by copy | an allocator you pass |
| `Gc<closure fn(A) -> R>` | yes | by copy | the GC heap |

Plain `fn` types stay thin code pointers.

##### Non-escaping closures: `block fn`

<!-- snippet: skip signatures and calls out of context, bodies elided -->
```loimi
fn count_if(xs: []i64, pred: block fn(i64) -> bool) -> i64 { .. pred(xs[i]) .. }
fn Vec<T>.sort_by(v: *Vec<T>, cmp: block fn(*T, *T) -> i32) { .. }

var hits = 0;
let n = count_if(xs, |x| x > limit);       // captures `limit`
each(v, |x| hits += x);                     // writes `hits`
ids.sort_by(|a, b| names.at(*a).cmp(names.at(*b)));
map(xs, |x: i64| -> str { if x > 5 { return "big"; } "small" });
```

`block fn(A, B) -> R uses E` is the type of a closure parameter.

**Literals.** `|a, b| expr` takes its parameter types from the
parameter, or they are written: `|a: T| ..`. `|| expr` takes no
parameters, and `|a| -> R { .. }` gives the result type.

**Where they can go.** A closure literal can only be an argument for a
`block fn` parameter, or a [local closure](#local-closures). A
`block fn` value can only be called or passed on as such an argument.
It cannot be stored in a field or global, returned, cast, or be a type
argument, and `block fn` types are only allowed at the top of a
parameter type. So a closure never outlives the call that receives it,
which makes capturing by reference safe without lifetime analysis.

- **Captures** are by reference: the closure sees the enclosing locals'
  current values, and its writes are visible after the call. `let`
  locals stay read-only.
- The body is a function of its own: `return` and `?` leave the
  closure, and `break` and `continue` cannot reach loops outside it.
- **Effects.** The body may only make calls whose effects the type
  allows (`block fn(i64) uses gc` may collect), and calling a `block fn`
  has its type's effects.
- **Conversion.** A plain function (or `fn` value) converts to a
  `block fn` with the same signature and no more effects:
  `v.sort_by(cmp_i64)` works.
- **Inference.** In a generic call, closure arguments are checked last,
  so their parameter types can come from the other arguments and their
  result type from the body (`map(xs, |x| x as f64 / 2.0)` infers
  `U = f64`).
- **Not in vm code.** Op handlers, vm functions, entries and `on raise`
  cannot contain closures, and vm functions cannot take `block fn`
  parameters. (A closure is a separate function, so it could not use
  control words, and capturing state variables by reference would force
  them into memory.) Call a plain function that makes the closure, or
  pass a named function. C functions cannot take them either.
- **GC.** A captured traced local stays in the enclosing frame and is
  rooted there. The callee roots its own traced locals at each call of
  a closure whose type `uses gc`.
- **Cost.** A `block fn` is a code pointer and an environment pointer,
  passed as two arguments. A call is indirect unless LLVM inlines the
  callee, which it does for a sort called with a closure literal.

##### Local closures

`let f = |x: i64| x * scale;` (parameter types written, result type from
the body) is a local `block fn`, with the same rules as a `block fn`
parameter: it can be called (`f(2)`) or passed on (`v.sort_by(f)`),
nothing else. Its type has no effects if its body makes no effectful
calls, else the enclosing function's effects.

##### Escaping closures: `closure fn`

Closures that can be stored, returned and kept in collections:

<!-- snippet: skip items and statements out of context -->
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

**Types.**

- `closure fn(A) -> R uses E` is a 16-byte value, `{ code, env }`, that
  can go anywhere a value can (fields, locals, globals, `Vec<..>`,
  `?..`, results, type arguments). Its environment is host memory and
  is **not traced**.
- `Gc<closure fn(A) -> R uses E>` is one traced word, a reference to a
  closure object in the GC heap. It is rooted, traced, write-barriered
  and freed like any `Gc`.

**Creation is an explicit allocation:** `new(a, |..| body)`.

- With `a: *gc.Heap` the result is a `Gc<closure fn>`.
- With any other `a`, the environment is one call of
  `a.alloc_bytes(size: u64, align: u64) -> *u8` (`*mem.Arena` has it,
  and so can your own allocator), and the result is a `closure fn`.
- A closure that captures nothing allocates nothing.
- Parameter and result types come from the expected type (a `let`
  annotation, a field, a parameter, `Vec<closure fn(..)>.push`, also
  generic ones: `map(xs, new(a, |x: i64| x * 2))` infers `U`), or from
  annotations on the literal.

**Captures are copies**, made when the closure is created, so the
closure may outlive the frame.

- Assigning a captured variable inside the body (also from a nested
  `block fn`) is an error: the write would not be seen outside. The
  error's note shows the two alternatives below.
- Capturing a `block fn` parameter is an error.

**State of its own.** A *state list* before the literal, `new(a, [var i
= lo, var n] |..| body)`, declares variables that belong to the
closure.

- They are fields of its environment, set when the closure is created:
  `var n` alone copies the local `n`, and `var n: T = e` gives a type.
- The body may assign them, and the values stay between calls.
- Copies of a closure value share one environment, so they share the
  state.
- The enclosing function's variables never change: `[var n]` makes a new
  `n` that hides the outer one in the body.

<!-- snippet: run "18 13 10" -->
```loimi
import std.io;
import std.mem.{Arena};

fn range(a: *Arena, lo: i64, hi: i64) -> closure fn() -> ?i64 {
    new(a, [var i = lo] || {              // the position, in the environment
        if i >= hi { return None; }
        i += 1;
        Some(i - 1)
    })
}

fn main() {
    var arena = Arena.new();
    let r = range(&arena, 3, 7);
    var sum: i64 = 0;
    while let Some(x) = r() { sum += x; }
    var n: i64 = 10;
    let count: closure fn() -> i64 = new(&arena, [var n] || { n += 1; n });
    count(); count();
    io.print_int(sum); io.print(" "); io.print_int(count()); io.print(" ");
    io.print_int(n); io.println("");   // 18 13 10
    arena.free();
}
```

Loimi does not allocate implicitly, so a closure that wants state says
so where it is created; an assignable capture would need the variable
moved to the heap behind your back.

In a GC closure, an assignment to traced state gets the write barrier.
Assigning traced state from a `block fn` nested in the body is an error
(that store would go through a pointer, without the barrier).

**Shared state** between a closure and its creator (or between
closures) is a cell both capture as a pointer. `new(a, v)` with an
allocator makes one: it copies `v` into `a.alloc_bytes(size, align)`
and gives a `*T`. (With a `*gc.Heap`, `new` makes an object instead,
[section 8](#8-gc-integration); GC references cannot go into allocator
memory.)

<!-- snippet: run "3 3"
import std.io;
import std.mem.{Arena};
{snippet}
fn main() {
    var arena = Arena.new();
    let c = counter(&arena);
    c.inc(); c.inc(); c.inc();
    io.print_int(c.get()); io.print(" "); io.print_int(*c.n); io.println("");
    arena.free();
}
-->
```loimi
struct Counter { n: *i64, inc: closure fn(), get: closure fn() -> i64 }

fn counter(a: *Arena) -> Counter {
    let n = new(a, 0);                    // a *i64 in the arena
    Counter { n: n, inc: new(a, || *n += 1), get: new(a, || *n) }
}
```

**GC.**

- Capturing a traced value (`Gc<T>`, a `value` type with references, a
  struct holding one) is allowed only in `new(heap, ..)`: an allocator's
  environment is invisible to the collector, and the error says so.
- A GC closure's environment is an object type generated per closure
  literal, so its captures are marked through it. While its body runs,
  the closure roots its own environment.
- Its traced captures cannot have their address taken (a store through
  the pointer would miss the write barrier).

**Effects** work as for `fn` and `block fn`: the body may only make the
calls the type allows, calling has the type's effects, and a closure
converts to a type with more effects.

**Conversions.**

- A plain function converts to `closure fn` without allocating, so
  tables can mix both.
- `closure fn` converts to `block fn` for free.
- `Gc<closure fn>` converts to neither (an untraced copy would not keep
  it alive).

**Freeing** belongs to the allocator.

- `arena.free()` frees every environment in the arena; the GC frees GC
  closures.
- `c.free(a)` frees one `closure fn`, where `a` is the allocator that
  made it. It calls `a.free_bytes(env: *u8)` if `new(a, ..)` allocated
  an environment, and does nothing for a closure that owns no memory
  (no captures or state, or a converted plain function). So a table
  mixing both can free every entry.
- `std.mem.Malloc` is an allocator on the C heap whose allocations are
  freed one by one (its `live` counts them). `Arena.free_bytes` does
  nothing, so code generic over the allocator can free either way.
- There is no ownership tracking. Calling a freed closure (or a copy of
  it), or freeing it again, is a use-after-free that the compiler does
  not detect (`--sanitize=address` does).
- Freeing the environment frees nothing its captures or state point to
  (a captured `Vec`'s buffer). A cell from `new(a, v)` is freed with
  `a.free_bytes(p as *u8)`.

<!-- snippet: run "live 0" -->
```loimi
import std.io;
import std.mem.{Malloc};

fn double(x: i64) -> i64 { x * 2 }

fn main() {
    var m = Malloc.new();
    let k = 5;
    let fs: [closure fn(i64) -> i64; 3] = [new(&m, |x| x + k), new(&m, |x| -x), double];
    io.print_int(fs[0](1) + fs[1](1) + fs[2](1)); io.println("");   // 6 - 1 + 2
    for f in fs { f.free(&m); }           // frees the first one's environment
    io.print("live "); io.print_int(m.live as i64); io.println("");
}
```

**Limits.**

- vm code cannot create closures, but handlers and vm functions can call
  stored ones (synced and rooted per their effects, like `fn` fields),
  and vm functions may take `closure fn` parameters.
- Neither closure type crosses into C. Pass a plain `fn` and a data
  pointer, and wrap them on the Loimi side: `new(&arena, |st| f(st, ud))`.

#### Errors, operators and arithmetic

**Errors and cleanup.**

- Error handling uses `Result` and `?`. There are no host-level
  exceptions or unwinding.
- `defer stmt` runs `stmt` when the enclosing scope exits.

##### Operators

From the tightest binding to the loosest:

| Operators | Meaning |
|---|---|
| `f(x)` `a[i]` `a[i..j]` `x.f` `x?` | call, index, slice, field or method, try |
| `-x` `!x` `~x` `*p` `&x` | negate; logical not (`bool`) or bitwise not (integers); bitwise not (integers only); dereference; address |
| `x as T`, `x as! V` | cast (`-x as u64` is `(-x) as u64`); unwrap a variant |
| `*` `/` `%` `*%` | multiply, divide, remainder, wrapping multiply |
| `+` `-` `+%` `-%` | add, subtract, wrapping forms |
| `<<` `>>` | shifts |
| `&` | bitwise and |
| `^` | bitwise xor |
| `\|` | bitwise or |
| `==` `!=` `<` `<=` `>` `>=` `is` | comparisons, pattern test |
| `and` (or `&&`) | logical and, short-circuit |
| `or` (or `\|\|`) | logical or, short-circuit |
| `a ?? b` | option default, right-associative: `a`'s value if `Some`, else `b` (evaluated only then); with `b: ?T` the result is `?T` |
| `=` `+=` `-=` `*=` `/=` `%=` `&=` `\|=` `^=` `<<=` `>>=` `+%=` `-%=` `*%=` | assignment (a statement) |

- Binary operators are left-associative.
- Unlike C, `&`, `^` and `|` bind tighter than the comparisons:
  `x & 1 == 0` is `(x & 1) == 0`.
- Both operands of a binary operator have the same type (shifts
  excepted). There are no implicit conversions.
- `!x` and `~x` are the same on an integer; `~` rejects a `bool`.
- There is no `++`, `--`, `**`, `?:`, `===` or unary `+`; the parser
  says what to write instead.

##### Integer arithmetic

- `+ - *` trap on overflow in debug builds and wrap in release builds.
- `a +% b`, `a -% b`, `a *% b` always wrap.
- `a.add_checked(b) -> ?T`, `sub_checked`, `mul_checked` return `None`
  on overflow (LLVM's `*.with.overflow` intrinsics).
  `std.num.add_overflow(a, b)` is a function over `i64`.
- In constant expressions (`const`s, array lengths, global
  initializers), overflow of `+ - *`, unary `-` and `/` is a compile
  error in all builds.

**Shifts.** `a << n` and `a >> n` have the type of `a`.

- `>>` is arithmetic for signed `a` and logical for unsigned.
- `n` may be any integer type (`x << shift` with `x: u32`, `shift:
  u64`). It is taken modulo the width of `a`, read as an unsigned
  number of `a`'s type, so a shift never has an undefined result.
- A literal `a` takes its type from the context (`w & (255 << shift)`
  with `w: u32` shifts a `u32`), and is an `i64` without one.

**Arbitrary-width integers.** `iN` and `uN`, for any `N` from 1 to 64,
are integers of exactly that width (LLVM's `iN`).

- Arithmetic wraps, overflows and is checked at `N` bits. `as` sign- or
  zero-extends from `N` bits. Shift amounts are taken modulo `N`.
- Their main use is `value` payloads (`Int(i48)` in a nanbox, `Int(i61)`
  in a lowtag), where they record how many bits the encoding keeps.
- A checked `+ - *` on an odd width costs two shifts more than on
  `i64`, and no compares.
- In memory they take the next power-of-two size (`i48`: 8 bytes,
  `u24`: 4).

##### Built-in methods

| Types | Methods |
|---|---|
| integers | `clz`, `ctz`, `popcount`, `add_checked`, `sub_checked`, `mul_checked` |
| `u64` | `mul_wide` (128-bit product), `mul_add_wide` (product plus two limbs), `div_wide` (quotient) |
| `u8`..`u64` | `add_carry`, `sub_borrow` (with a `bool` carry in and out, for `adc`/`sbb` chains) |
| floats | `sqrt`, `abs`, `floor`, `ceil`, `trunc`, `nearest`, `copysign`, `is_nan`, `min`/`max` (C's `fmin`/`fmax`), `minimum`/`maximum` (IEEE 754-2019), `to_bits` |
| `f64`, `f32` | `f64.from_bits(u)`, `f32.from_bits(u)` |

The exact semantics are in [stdlib.md](stdlib.md), `std.core`.

When `u` reads a place through a pointer (`p[i]`, `*p`, a register
operand `regs[it]`), optionally cast to `u64` from a 64-bit integer or
an unboxed `value` type, `from_bits` is a load of that place as the
float. So `f64.from_bits(a as u64)` on a `reg` operand loads straight
into a floating-point register, like the unsafe
`*((regs + a.raw) as *f64)`, and needs no `unsafe`.

##### Hints and `unsafe`

- **Hints:** `likely(c)`, `unlikely(c)`, `assume(c)`, `unreachable`,
  `#[cold]`, `#[inline(always)]`, `#[inline(never)]`.
- `trap()` aborts. There is no `assert`.
- `size_of<T>()` is a type's size in bytes.

`unsafe { }` marks code whose correctness the compiler cannot check.
It is otherwise an ordinary block. It is required:

- everywhere, to make a GC reference from something else (`p as Gc<T>`
  from a pointer, an address or another `Gc`; [section 8](#8-gc-integration)),
  because the collector trusts every `Gc` it finds;
- everywhere, to name a function marked `#[unsafe]`;
- in a module marked `#![safe]`, for raw-pointer operations
  ([section 1.3](#13-memory)). Elsewhere raw pointers do not need it.

#### Vectors

`i8x16`, `u8x16`, `i16x8`, `u16x8`, `i32x4`, `u32x4`, `i64x2`, `u64x2`,
`f32x4` and `f64x2` are 128-bit SIMD vectors (LLVM's `<N x T>`).

- They are plain values: locals, parameters, results, struct and object
  fields, enum payloads, `?T`, array elements and `vm` state registers
  (one vector register each, `xmm`/`q`, across tail calls).
- Signedness is part of the type. It decides what `>>`, `<`, `min`,
  `max`, `add_sat`, widening and `as` do, so `u8x16` and `i8x16` are
  different types holding the same bits.
- Nothing converts implicitly. A scalar operand is an error that says to
  `splat` it.

<!-- snippet: run "6 3 8 -2147483644 | 7 | 0 -1 0 0 | 255 0 3" -->
```loimi
import std.io;

fn show(v: i32x4) {
    io.print_int(v.lane(0) as i64); io.print(" "); io.print_int(v.lane(1) as i64); io.print(" ");
    io.print_int(v.lane(2) as i64); io.print(" "); io.print_int(v.lane(3) as i64);
}

fn main() {
    let a = i32x4.new(1, -2, 3, 2147483647);
    let b = i32x4.splat(5);
    show(a + b); io.print(" | ");                 // lane-wise, integer lanes wrap
    io.print_int((a < b).bitmask() as i64); io.print(" | ");  // lanes 0..2 are below 5
    let m = f32x4.new(1.0, 0.0 / 0.0, 2.0, -1.0) != f32x4.new(1.0, 0.0, 2.0, -1.0);
    show(m); io.print(" | ");                     // a mask: NaN != anything
    let bytes = u8x16.splat(250).add_sat(u8x16.splat(10));     // saturates at 255
    let ix = u8x16.new(0, 16, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3);
    let s = u8x16.new(255, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15).swizzle(ix);
    io.print_int(bytes.lane(15) as i64); io.print(" ");
    io.print_int(s.lane(1) as i64); io.print(" "); io.print_int(s.lane(2) as i64); io.println("");
}
```

##### Making vectors and reading lanes

| Expression | Result |
|---|---|
| `T.splat(x)` | every lane `x` |
| `T.new(a, b, ..)` | one argument per lane, lane 0 first |
| `T.from_bits(v)` | the same 128 bits as another vector type (no instruction) |
| `v.lane(i)` | lane `i`; `i` must be a constant expression |
| `v.with_lane(i, x)` | a copy with lane `i` replaced; `i` constant |
| `v.lane_at(i)` | lane `i & (N - 1)`, `i` of any integer type |
| `v.with_lane_at(i, x)` | a copy with lane `i & (N - 1)` replaced |

- `v.lane(i)` of a `u8x16` is a `u8`, so `as i32` zero-extends it.
- `lane_at` and `with_lane_at` are for a lane chosen at run time (an
  index decoded from bytecode). The index is **masked**, not checked,
  so there is no panic path. `lane_at` of a vector just loaded from
  memory is one scalar load; otherwise the vector goes through a stack
  slot. `with_lane_at` stays in registers (about 7 SSE2 instructions).

##### Operators

Lane by lane, both operands of one vector type:

- `+ - *`: integer lanes always wrap, in debug builds too (`+% -% *%`
  are the same).
- `/`: float lanes only. Unary `-`.
- `& | ^` and `~`/`!`: integer lanes.
- `v << n`, `v >> n`: shift every lane by the scalar integer `n`, taken
  modulo the lane width. `>>` is arithmetic on signed lanes and logical
  on unsigned ones.
- `== != < <= > >=` give a **mask**: a vector of signed integers of the
  lane width (`i32x4` for `f32x4` and `u32x4` operands), all ones where
  the comparison holds and zero elsewhere. Float comparisons are
  ordered except `!=`, as for scalars: a NaN lane compares unequal and
  neither less nor greater. A mask is not a `bool`; `m.bitmask()` gives
  one bit per lane.

##### Methods

Lane-wise unless said otherwise.

| Lanes | Methods |
|---|---|
| all | `abs`, `min`, `max` (integers by signedness; floats like C's `fmin`/`fmax`) |
| integer | `add_sat`, `sub_sat` (saturating, by signedness), `clz`, `ctz`, `popcount` |
| unsigned integer | `avgr`: `(a + b + 1) >> 1` without overflow |
| float | `sqrt`, `floor`, `ceil`, `trunc`, `nearest` (ties to even), `copysign`, `minimum`/`maximum` (IEEE 754-2019: NaN propagates, `-0 < +0`) |

Methods that work across lanes:

- `m.select(a, b)` (integer mask `m`): lane `i` of `a` where lane `i` of
  `m` is non-zero, else of `b`. `a` and `b` have `m`'s lane count.
- `v.bitmask() -> u32` (integer lanes): bit `i` is the top bit of lane
  `i`. A mask `v` is true in every lane when
  `(v == T.splat(0)).bitmask() == 0`.
- `v.widen_low()`, `v.widen_high()`: the low or high half of the lanes
  at twice the width, sign- or zero-extended by signedness (`f32x4` to
  `f64x2`).
- `T.narrow_sat(a, b)`: `a`'s lanes then `b`'s, each saturated into
  `T`'s lane range. `T` has 8-, 16- or 32-bit integer lanes; `a` and
  `b` have lanes twice as wide, read by their own signedness.
  `u8x16.narrow_sat(x, y)` with `i16x8` operands is x86's `packuswb`.
- `a.shuffle(b, [i0, i1, ..])`: lane `k` is lane `ik` of the lanes of
  `a` followed by those of `b`. One constant index per lane, each below
  twice the lane count.
- `a.swizzle(idx)` (8-bit lanes): lane `k` is lane `idx[k]` of `a`, or
  0 if `idx[k]` (read unsigned) is 16 or more. This is wasm's
  `i8x16.swizzle` and AArch64's `tbl`.

##### Conversions and memory

- **`as`** converts lane by lane between vector types with the same
  number of lanes: `i32x4 as f32x4`, `f64x2 as u64x2` (float to integer
  saturates, and NaN gives 0), `i32x4 as u32x4` (the same bits).
  Between different lane counts it is an error that points to
  `from_bits`, `widen_*` and `narrow_sat`.
- **Layout.** A vector is 16 bytes and 16-byte aligned in layouts
  (struct fields, arrays, `size_of`), as in C.
- **Loads and stores assume no alignment**, unlike scalars ([section
  1.3](#13-memory)). An 8-byte aligned GC object, a register file of
  8-byte slots read as `*((regs + it) as *i32x4)`, or guest memory can
  all hold vectors. On current x86-64 and AArch64 cores an unaligned
  vector access costs the same as an aligned one.
  `p.read_unaligned()` and `p.write_unaligned(v)` accept vector
  pointers too.
- **C.** `--header` rejects vectors in `export "C"` signatures (C has
  no portable vector type): pass a pointer.

##### Target CPU

Code is generated for each target's baseline: SSE2 on x86-64, NEON on
AArch64. Most operations are one or a few instructions on both. Where
SSE2 lacks an instruction, the operation expands:

| Operation | On baseline x86-64 |
|---|---|
| 8-bit shifts, 64-bit compares, `i64x2` multiply, `minimum`/`maximum` | a handful to about 30 instructions |
| float rounding methods (`floor`, `ceil`, `trunc`, `nearest`) | about 25 instructions, no call |
| `u16x8.narrow_sat` of `i32x4` | about 25 instructions |
| `f32x4 as i32x4` / `as u32x4` | about 10 / 15 instructions |
| `swizzle` | lane by lane through memory, about 100 instructions (one `tbl` on AArch64) |

`loimi build --cpu=NAME` compiles for a newer CPU: `x86-64-v2`,
`x86-64-v3`, `x86-64-v4`, `native`, or any name clang's `-march` takes
(on AArch64, clang's `-mcpu`, such as `neoverse-n1`). LLVM then uses
what the CPU has; with SSSE3 `swizzle` is 3 instructions, and with
SSE4.1 the rounding methods are 1. The binary then needs that CPU (it
dies with SIGILL elsewhere). `LOIMI_CPU` sets a default for `--cpu`.

### 1.3 Memory

There is no borrow checker. Host memory is managed through explicit
allocators, mostly arenas (`std.mem`), which suit compilers and
interpreters: ASTs, IR and per-compilation data die together.
`new(a, v)` with such an allocator copies `v` into memory from
`a.alloc_bytes` and gives a `*T` (`new(&arena, Node { .. })`). Guest
memory lives in the GC heap ([section 8](#8-gc-integration)).

Loimi is "safety-leaning", not memory-safe:

- **Arrays and slices are bounds-checked**, in debug and release
  builds. Integer overflow traps in debug builds.
- **Raw pointers are unchecked.** `*p`, `p.field`, `p[i]` and `p + n` on
  a `*T` compile to plain loads, stores and address arithmetic. Outside
  `#![safe]` modules they need no `unsafe`: the pointer type itself is
  the marker. Use a slice (`p.slice(n)`) where you want a bounds check.
- **Pointer stability is checked** in every module (below).
- **GC references cannot be forged** outside `unsafe` (below).
- **Bytecode is checked once, by the verifier** ([section 6](#6-verification-unchecked-by-proof)).
  Operand accesses in handlers are unchecked because it ran.

#### `#![safe]` modules

`#![safe]` at the top of a file makes raw-pointer use checked: there a
`*T` is a reference to one live value.

Allowed:

- following a pointer: `*p`, `p.f`, `p.read_unaligned()`, atomics;
- `&place`, `new(a, v)`, slices, `Vec`, and other modules' methods.

Errors outside `unsafe { }` (everything that makes a pointer reach
anything else):

- indexing and arithmetic: `p[i]`, `p[a..b]`, `p + n`, `p.offset(n)`,
  `p.slice(n)`;
- casts to a pointer (from an integer, another pointer type, a function
  or a `Gc`), and casts from an integer to an `enum` or `value` type;
- reading a slice's `.ptr`, or a raw-pointer field of another module's
  struct (a field holding a `*T` directly or in a tuple, array, option
  or function type);
- writing any field of another module's struct that has raw-pointer
  fields, directly or through its struct fields (`v.len = 0`), and
  building one with a literal;
- binding a pointer payload of another module's enum or `value` type in
  a pattern, and building such a variant;
- making a null `*T`: `..zero`, or a `var` without an initializer;
- `new(&a, v)` with an allocator whose `alloc_bytes` is not `#[raw]`;
- `assume` and `unreachable`;
- calls of `extern` functions and of functions marked `#[raw]`.

**`#[raw]`** marks a function that is unsound for some arguments that
are pointers to one value each: it takes a pointer and a length
(`mem.copy(dst, src, n)`), frees (`mem.release(p)`), returns memory
that is not one valid value (`mem.alloc<T>(n)`), or trusts an index it
does not check. `#[raw]` changes nothing in modules without `#![safe]`.

**vm code is exempt** (its operand accesses rest on the verifier), as is
code generated from a `vm` declaration. So a `#![safe]` module cannot
declare a `vm`.

**Objects.** The objects of a `#![safe]` module are built and written
only by that module's code. In a `#![safe]` module, another module's
`object` is treated like a struct with raw pointers.

`std.lex`, `std.parse`, `std.diag`, `std.text`, `std.hash`, `std.io`,
`std.bc`, `std.map` and `std.dict` are `#![safe]`.

<!-- snippet: fail "indexing a raw pointer"
#![safe]
{snippet}
fn main() {}
-->
```loimi
fn second(p: *u8) -> u8 { p[1] }               // error: use a slice
fn second_ok(s: []u8) -> u8 { s[1] }           // bounds-checked
fn second_raw(p: *u8) -> u8 {
    // unsafe: callers pass a buffer of at least two bytes
    unsafe { p[1] }
}
```

#### Pointer stability

In every module, std's included, the compiler checks that:

- a pointer or slice into a `Vec`, map, arena or GC object is not used
  after a call that may move or free that memory (`push`, `insert`,
  `free`, a call that may collect, in vm code a `reenter` call; inferred
  over the call graph);
- a copy of a `Vec` does not outlive a growth of the other copy;
- the address of a local is not returned or stored where it outlives
  the function;
- a pointer into a `Vec`'s or a map's buffer is not stored where it
  outlives the function.

<!-- snippet: fail "`s` may dangle"
import std.mem;
{snippet}
fn main() {}
-->
```loimi
fn first_after_push(v: *mem.Vec<u64>) -> u64 {
    let s = v.items();
    v.push(0);    // may move the buffer `s` points into
    s[0]          // error: `s` may dangle
}
```

Other pointers stored in memory and loaded back elsewhere are not
followed, so they can still dangle (an arena node after `Arena.free`).

Ways to relax the check:

| Escape | Effect |
|---|---|
| `#[may_alias(p)]` on a function | argument `p` may point into what the function grows (`Vec.extend`) |
| `#[shrinks(v)]` on a function | the function removes elements of `v` (`Vec.pop`) |
| `unsafe { }` | no check inside the block |
| `#![lifetimes(off)]` on a module | no check in the module (not allowed in a `#![safe]` module) |
| `LOIMI_LIFETIMES=off` | no check for the whole build |

A bare `#![lifetimes]` is accepted and changes nothing.

<!-- snippet: check
{snippet}
fn main() {}
-->
```loimi
#![lifetimes(off)]
import std.mem;
fn first_after_push(v: *mem.Vec<u64>) -> u64 {
    let s = v.items();
    v.push(0);
    s[0]          // not checked: reads freed memory if `push` moved it
}
```

With `--sanitize=address`, std's containers poison memory they keep but
no longer use (a `Vec`'s spare capacity, an arena chunk's unused tail),
so ASan reports a stale pointer into it as well as into freed memory.

#### Alignment, prefetch and atomics

**Alignment.** `*p` assumes `p` is aligned for its type, as in C, and
LLVM may rely on it. For an address that may not be aligned (a guest
memory address, a packed format), `p.read_unaligned()` and
`p.write_unaligned(v)` load and store with alignment 1. They work on
`*T` for integer, float, vector and pointer `T`, and on x86-64 and
AArch64 compile to the same single `mov`/`ldr` as an aligned access.
Vectors are the exception: `*p` on a vector pointer assumes no
alignment ([vectors](#vectors)).

**Prefetch.** `p.prefetch()` on any raw pointer asks the CPU to bring
the cache line at `p` into the cache (`llvm.prefetch`, read, high
locality). It never faults, whatever `p` is.

**Atomics**, for memory that threads share outside a lock. A pointer to
an 8-, 16-, 32- or 64-bit integer (or to a pointer) has:

| Method | Ordering |
|---|---|
| `p.atomic_load()` | acquire |
| `p.atomic_load_relaxed()` | monotonic |
| `p.atomic_store(v)` | release |
| `p.atomic_swap(v)` | sequentially consistent |
| `p.atomic_cas(old, new) -> bool` | sequentially consistent |
| `p.atomic_add(v)` (integers; returns the old value) | sequentially consistent |

Each is one LLVM atomic instruction, which ThreadSanitizer understands.

<!-- snippet: run "8 7"
import std.io;
fn main() {
    var n: u32 = 7;
    let p = &n;
    {snippet}
    io.print_int(p.atomic_load() as i64);
    io.print(" ");
    io.print_int(old as i64);
}
-->
```loimi
let old = p.atomic_add(1);              // n = 8, old = 7
if !p.atomic_cas(8, 8) { trap(); }      // stores: n was 8
```

#### GC references

A `Gc<T>` comes from `new`, from a field or value that holds one, or
from `unsafe { p as Gc<T> }`. The collector marks through every `Gc` it
finds, so a bad one corrupts the heap far from the bug; that is the one
conversion the compiler makes you mark. For the same reason:

- a trailing array starts zero-filled only where zero is a valid
  element (not a null `Gc`, not a word that encodes no variant of a
  `value` type);
- the compiler keeps the write barrier on stores through slices of an
  object's trailing array.

Both are described in [section 8](#8-gc-integration).

### 1.4 Compile-time evaluation

`inline for` and `#[cfg(..)]` are the only compile-time evaluation.
There is no `comptime` or `const fn`; tables such as lexer classes are
filled at run time.

#### Conditional compilation: `#[cfg]`

`#[cfg(NAME)]` keeps what follows it only when the build was given
`--cfg NAME`; `#[cfg(not(NAME))]` keeps it only without. It applies to:

- items (an `import`, a function, a `var`, a struct, ...);
- struct fields, and field initializers in struct literals;
- statements;
- items of a `vm` block (an op, a variant, `on break`, ...).

<!-- snippet: run "plain"
import std.io;
{snippet}
fn main() { io.print(build()); }
-->
```loimi
#[cfg(gil)]
fn build() -> str { "with the lock" }
#[cfg(not(gil))]
fn build() -> str { "plain" }
```

- The parser drops what is not kept, before names are resolved. A build
  without the name compiles exactly as if the code were not there.
- Several `#[cfg(..)]` in a row must all hold.
- Dropped code is not checked, so build both ways.
- `--sanitize=address` and `--sanitize=thread` also set
  `sanitize_address` and `sanitize_thread`.
- Panic messages carry line numbers. Write the attribute on the line it
  guards (`#[cfg(gil)] if .. { .. } jump target`) to keep later lines
  where they were.

#### `inline for`

`inline for` unrolls at compile time. It can appear at module level,
among the items of a `vm`, and as a statement in any block (function
bodies, op handlers, vm functions, entries).

<!-- snippet: run "6"
import std.io;
struct Regs { a: i64, b: i64, c: i64 }
{snippet}
fn main() { var r = Regs { a: 1, b: 2, c: 3 }; io.print_int(sum_fields(&r)); }
-->
```loimi
fn sum_fields(r: *Regs) -> i64 {
    var total: i64 = 0;
    inline for f in [a, b, c] { total += r.f }
    total
}
```

It can generate items, including ops:

<!-- snippet: check
value Value repr nanbox { Num(f64) unboxed, Int(i32), Nil }
enum Cond { Lt, Le, Eq }
fn compare(c: Cond, a: Value, b: Value) -> Result<bool, str> { Ok(false) }
vm V {
    state { pc: *u32, regs: *Value }
    operand reg = regs[it];
    operand label = pc + it verify is_insn_boundary(pc + it);
    encoding fixed(u32) { format ABsC { op: 8, a: 8, b: 8, sc: s8 } }
{snippet}
    entry run(code: *u32, r: *Value) -> Result<Value, str> { regs = r; jump code }
    on raise(e: str) { exit Err(e) }
}
-->
```loimi
inline for (name, cc) in [("JLT", Cond.Lt), ("JLE", Cond.Le), ("JEQ", Cond.Eq)] {
    op #name(a: reg, b: reg, target: label) : ABsC {
        if compare(cc, a, b)? { jump target } else { next }
    }
}
```

**Expansion is token-level.** The body is copied once per element, and
every identifier token equal to a loop variable is replaced by that
element's tokens.

- An element of more than one token is put in parentheses: `v * 10`
  with `v = 2 + 3` is `(2 + 3) * 10`.
- Field names (`r.f`) and binders (`let v`) are replaced too, so pick
  loop variable names that do not collide with names in the body.
- `#v` turns a string or identifier value into an identifier.
- `#(a ++ "_X" ++ b)` makes one identifier from several parts: loop
  variables (string, identifier or integer values), string literals and
  integer literals. So one table row can name several items (`op
  #name(..)` and `op #(name ++ "_X")(..)`).

**Scoping.** In statement position, the copies become statements of the
enclosing block, each ending with `;`. There is no new scope:

- `let #name = ..;` declares a local of the enclosing block, which later
  statements (and later copies) see;
- a plain `let tmp` in the body is shadowed by the next copy's
  `let tmp`;
- `next`, `break` or `return` in a copy act on the enclosing handler or
  loop.

Write `{ .. }` inside the body for a scope per copy. A spliced `#name`
is an ordinary identifier where it lands: it resolves, and binds,
exactly as if it had been written there.

**Nesting.** Nested `inline for`s expand outside-in, and each fills in
its own variables. An inner loop may use the outer variables in its
element list, but may not reuse their names as its own variables.

**Errors** point into the original source: a token copied from the body
keeps its position in the body, and a substituted token its position in
the element list.

**In a `vm`** (also in handler and vm function bodies), a loop variable
may not have the name of a state variable: it would replace every use
of that name in the body.

### 1.5 Modules and interop

`import std.gc.{Heap, Weak};` imports names from a module. `Gc<T>` is
built in.

**C interop.**

- `extern "C" fn` declares a C function.
- `export "C"` exposes a Loimi function. In a library build these are
  the public API, and `--header` declares them for C
  ([section 10.1](#101-embedding-in-a-c-host-implemented)).
- `bool` crosses both kinds of boundary zero-extended, as C expects.

**Namespaces.** A module's names live in two namespaces:

- **types and modules**: structs, enums, `value` and `object` types,
  type aliases, traits, vms, and imported modules (`import std.parse;`
  binds `parse`). Type positions and the head of a path see these:
  `T`, `m.T`, `T.f(..)`, `T.Variant`, `T { .. }`, `m.f(..)`.
- **values**: functions, consts and globals.

A function may share its name with a type or module: `fn parse(..)` can
sit next to `import std.parse;`, and `struct Point` next to
`fn Point(..)`. Any other clash is an error ("defined more than
once"): two types, a type and a module, two values, or a const or
global and a type or module (a const or global can be the head of
`x.y`, as in `ORIGIN.x`). `import m.{x}` imports whatever `m` binds as
`x`, in either namespace.

**Name resolution** depends on where the name is:

- A local variable always wins.
- `x(..)` and `x` in an expression name a value. If there is none, `x`
  may be a variant of the expected type (`Arr(a)` where a `Value` is
  expected, even with `object Arr` imported: a type is never a value).
  Otherwise it is an error.
- In `x.y`, `x` is looked up among types and modules first. With both
  `fn parse` and `import std.parse`, `parse.Prec` is the module's
  struct.
- Patterns resolve names as described in
  [constants in patterns](#constants-in-patterns).

Enum and `value` variants are not module items: bare `Arr(..)` or `Nil`
is found through the expected type (or written `Value.Arr(..)`).

---

## 2. `value` types: declared representations

A dynamic-language interpreter has a `Value` type, and its encoding is
the most performance-critical decision in the VM. In Loimi the encoding
is a declaration:

<!-- snippet: check
object Str { len: u64 }
object Object { n: u64 }
object Closure { n: u64 }
object NativeFn { n: u64 }
{snippet}
-->
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

Switching `repr` is a one-word change, so you can benchmark
representations instead of committing to one early.

### Representations

| `repr` | Layout | Typical user |
|---|---|---|
| `nanbox` | 64-bit word. Doubles stored directly; other variants in the NaN space, with payloads of up to 48 bits. Needs 48-bit pointers (x86-64 and AArch64 have them). | JS engines, LuaJIT |
| `nanbox(int_high)` | The same, rotated so that an integer is `payload << 16` and doubles cost a decode ([below](#integers-in-the-high-bits-repr-nanboxint_high)) | |
| `lowtag` | 64-bit word. Aligned pointers with tag bits in the low bits; integers of up to 61 bits, shifted | Ruby, OCaml, many Schemes |
| `boxed` | Two words `{tag, payload}`, passed and returned in two registers | PUC Lua (`TValue`) |

The compiler chooses the tags. You can pin a variant's block width
(`block N`, [below](#seeing-and-pinning-the-layout)), not the tag itself.

What you get from the declaration:

- **Constructors and patterns**: `Int(3)`, `match v { Int(x) => ... }`,
  `v is Nil`, `v as! Int` ([section 1.2](#patterns-literals-and-match)).
- **Merged tests.** The compiler knows the bit-level encoding, so it can
  merge tests, including over tuples of values (binary operators). With
  NaN-boxing and an `Int` tag `T` in the high 16 bits, `(Int(x),
  Int(y))` compiles to `((a ^ T) | (b ^ T)) >> 48 == 0`, one test
  instead of two. `(Num, Num)` compiles to `umax(a, b) <= LIMIT`.
- **Precise GC.** The compiler knows which variants hold `Gc<_>`, so
  tracing a `Value` is generated code.
- **NaN canonicalization.** In a `nanbox`, constructing `Num(x)`
  canonicalizes NaNs, so a guest NaN can never forge a pointer.

### Payloads and how many variants fit

A variant's payload is an integer (`iN`/`uN`, [section 1.1](#11-types)),
`bool`, a pointer (`*T`, `Gc<T>`), or nothing. In a nanbox, one
`unboxed` variant holds the `f64`. Integer payloads are as wide as
their type says, so `Int(i48)` keeps 48 bits, and `x.add_checked(y)` on
its payload detects overflow at 48 bits.

| `repr` | Payloads | Tagged variants |
|---|---|---|
| `nanbox` | integers up to 48 bits (`i48`, `u32`, ...), `bool`, pointers (48 bits) | Up to 7 with any payloads, or 8 if one has a payload of at most 47 bits. Beyond that, see below. |
| `lowtag` | integers up to 61 bits (`i61`), `bool`, pointers (8-byte aligned) | Up to 8 with any payloads. Beyond 8, pointers and 61-bit integers take one 3-bit tag each, and a variant with an N-bit payload can have a tag of up to 64 - N bits, so narrow variants share what is left. |
| `boxed` | any number, `bool`, pointers | any number |

**A nanbox with more variants.** The NaN space holds seven 48-bit
blocks and one 47-bit block. Each variant takes a block of
2^(payload bits) words:

- a pointer or `i48` takes a whole 48-bit block, and `i47` the 47-bit
  one;
- `u32` takes 2^32 words, and a variant without payload one word;
- `block N` sets the size explicitly.

When the 48-bit blocks run out, `Gc` variants share them, up to 8 in a
block, told apart by the low 3 bits of the pointer (GC objects are
8-byte aligned). That allows up to 56 `Gc` variants in all, minus 8 per
48-bit integer or raw pointer. Without pins, the compiler shares as few
`Gc` variants as it must, the last-declared first, so declare the hot
ones first. A declaration that does not fit is a compile error naming
the variant with no room.

For example, a value with `Num(f64) unboxed`, `Int(i48)`, five pointer
types, `Char(u32)`, `Prim(u16)` and four variants without payload fits
a nanbox: the integer and pointers take six 48-bit blocks, and the rest
share the remaining space.

<!-- snippet: run "cell 7" "list 2" -->
```loimi
import std.io;
import std.gc;
object Str { len: u64 }
object List { len: u64 }
object Cell { v: i64 }
object Class { n: u64 }
object Gen { pc: u64 }

// Nine 48-bit blocks needed (`Int` and eight pointers), seven available:
// `Class` is pinned to share one (`block 45`), and as few unpinned `Gc`
// variants as fit, the last declared first, share it too (`Gen`, `C`).
// `--explain-values` shows the layout.
#[allow(value_layout)]
value Value repr nanbox {
    Float(f64) unboxed,
    Int(i48),
    Nil,
    Str(Gc<Str>), List(Gc<List>), Cell(Gc<Cell>),
    A(Gc<Str>), B(Gc<Str>), C(Gc<Str>),
    Class(Gc<Class>) block 45,
    Gen(Gc<Gen>),
}

fn describe(v: Value) {
    match v {
        Cell(c) => { io.print("cell "); io.print_int(c.v); }
        List(l) => { io.print("list "); io.print_int(l.len as i64); }
        Class(_) | Gen(_) => io.print("shared"),
        _ => io.print("other"),
    }
    io.println("");
}

fn main() {
    var heap = gc.Heap.new();
    describe(Value.Cell(new(&heap, Cell { v: 7 })));
    describe(Value.List(new(&heap, List { len: 2 })));
}
```

**Cost.** The fast paths do not depend on the layout:

- a variant test is one shift and compare (`(v >> w) == tag` in a
  nanbox; `(v & mask) == tag` in a lowtag and in a shared block;
  `v == word` for a variant without payload);
- the unboxed `f64` test is one unsigned compare;
- tuple patterns whose elements have tags in the same position merge
  into one test;
- extracting a pointer is one `and`. In a shared block, the low tag is
  folded into the addressing of loads through the pointer
  (`[p - 3 + 8]`), at no cost;
- a `match` over the variants of one shared block becomes a jump table
  on the low bits.

[lowering.md](lowering.md) section 2 has the encodings and their
measured costs.

### Seeing and pinning the layout

Whether two variants' tests merge, and what a raw word looks like,
depends on the layout, and the layout depends on how many variants
there are. So adding a variant can change the encoding of the others
(for example, move `Int(i32)` from a 48-bit block to a 32-bit one, which
splits its tuple tests in two). These tools keep that from happening
silently:

- **`loimi build --explain-values`** prints every value type's layout
  and stops: each variant's block width, mask, tag and words, and which
  variants' tests merge in a tuple pattern (those with the same block
  width) and which do not.
- **`block N`** after a variant pins its block width. In a nanbox it is
  the payload bits below the tag (from the payload's width to 48;
  pointers take 48); in a lowtag, the bits above the tag (up to 61;
  pointers take 61). The layout honors it, or the declaration is an
  error ("no room for variant").
  - `Int(i32) block 48` keeps `Int`'s tests merging with the pointers'
    however many variants follow.
  - Pinning a variant narrower than the classic width selects the
    packed layout.
  - A `Gc` variant of a nanbox can be pinned `block 45`: it shares a
    48-bit block with other `Gc` variants (45 bits of pointer between
    the 16-bit tag and the 3-bit low tag).
- **The `value_layout` warning.** When a declaration has too many
  variants for the classic layout, every variant whose block is
  narrower than the classic 48 bits (61 in a lowtag) must be pinned at
  its width, or the compiler warns, naming them. So a new variant that
  narrows others is a warning, and an error under `-W error`. Pin the
  widths to accept them (`Nil block 0`), or use
  `#[allow(value_layout)]`.
- **Encoding constants**, for code that tests raw words. They follow
  the layout, and tests built from them cost what hand-written ones do.

  | Constant | Value |
  |---|---|
  | `V.Int.mask` | the bits that identify the variant |
  | `V.Int.tag` | their value: a word `w` holds `Int` exactly when `w & V.Int.mask == V.Int.tag` |
  | `V.Int.block` | the block width |
  | `V.Num.limit` | for the `unboxed` variant: its words are those below it (not under `int_high`) |
  | `V.Num.xor` | for the `unboxed` variant under `int_high`: the rotation constant ([below](#integers-in-the-high-bits-repr-nanboxint_high)) |

<!-- snippet: run "both ints" -->
```loimi
import std.io;
struct Obj { id: i64 }

value V repr nanbox {
    Num(f64) unboxed,
    Int(i32) block 48,      // the classic block, whatever comes later
    Nil block 0,            // one word, in the 47-bit block
    Bool(bool),
    A(*Obj), B(*Obj), C(*Obj), D(*Obj), E(*Obj),
}

// Two Ints, tested and ordered without extracting the payloads: the
// biased words have no bits under the mask exactly for Ints.
fn both_ints(a: V, b: V) -> bool {
    let bias = V.Int(-2147483648) as u64;
    (((a as u64 ^ bias) | (b as u64 ^ bias)) & V.Int.mask) == 0
}

fn main() {
    if both_ints(V.Int(1), V.Int(-2)) and !both_ints(V.Int(1), V.Nil) { io.println("both ints"); }
}
```

### Integers in the high bits: `repr nanbox(int_high)`

In a plain nanbox, an `Int(i48)` sits under a 16-bit tag, so
`x.add_checked(y)` shifts both payloads to the top of the word to get an
overflow flag at 48 bits, then shifts the sum back to re-tag it.

`int_high` rotates every word of the layout left by 16 bits and xors
the integer variant's tag away. That variant's word is then
`payload << 16`, with zero low bits:

| Operation on two integers | Code |
|---|---|
| checked add or subtract | `add; jo` on the words (the result is already tagged) |
| checked multiply | `sar; imul; jo` |
| compare | a word compare |
| test that both are integers | `(a \| b) & 0xFFFF == 0` |

Every other variant has non-zero low 16 bits, and a pointer payload is
`word >> 16`. The price is the unboxed `f64`: decoding it is `xor; ror`,
encoding it `rol; xor`, and its test decodes first. So it suits
integer-heavy code; code whose values are mostly floats pays four
instructions per float operation. `--explain-values` prints the rotated
masks and tags and these costs.

The integer variant is the first one with an integer payload and a
whole 48-bit block. A declaration without one is an error.

<!-- snippet: run "int 140737488355327" "overflow" "num 0.75" -->
```loimi
import std.io;
value V repr nanbox(int_high) {
    Num(f64) unboxed,
    Int(i48),             // the word is x << 16
    Nil,
}

fn add(a: V, b: V) -> V {
    match (a, b) {
        // `add; jo` on the two words
        (Int(x), Int(y)) => if let Some(r) = x.add_checked(y) { V.Int(r) } else { V.Nil },
        (Num(x), Num(y)) => V.Num(x + y),   // xor; ror each, addsd, rol; xor
        _ => V.Nil,
    }
}

fn show(v: V) {
    match v {
        Int(x) => { io.print("int "); io.print_int(x as i64); }
        Num(x) => { io.print("num "); io.print_f64(x); }
        Nil => io.print("overflow"),
    }
    io.println("");
}

fn main() {
    show(add(V.Int(140737488355326), V.Int(1)));
    show(add(V.Int(140737488355327), V.Int(1)));
    show(add(V.Num(0.5), V.Num(0.25)));
}
```

Two things differ from the plain nanbox:

- `V.Num.limit` does not exist.
- Code that reads a slot as a double before testing it
  (`f64.from_bits(a as u64)`) reads rotated words. Use `V.Num.xor`: the
  word of an f64 with bits `b` is `rotl(b, 16) ^ V.Num.xor`, so a word's
  f64 is `f64.from_bits(rotr(w ^ V.Num.xor, 16))`. Spell the rotate as
  `(x >> 16) | (x << 48)`; LLVM makes it one `ror`. Every other
  variant's word reads as a NaN.

### Raw-word scanning: `#[scan_raw_words]`

`#[scan_raw_words]` on a value type asks the compiler to guarantee that
every word below 2^48 (a user-space pointer on x86-64 and AArch64, or a
small non-negative integer) reads as a variant without references. It
is an error if the layout does not: only `repr nanbox` with an unboxed
float variant qualifies (such words are floats), and not
`nanbox(int_high)`.

A root scanner may then `gc_mark` memory that mixes raw pointers and
counts with values of the type, word by word. If the type later changes
to a layout that breaks this, the program fails to compile instead of
breaking the scanner.

---

## 3. The `vm` declaration

A `vm` block declares one interpreter: its register state, operand
kinds, instruction encoding, inline caches, instructions, and entry and
error handlers.

<!-- snippet: skip outline of a vm's parts -->
```loimi
vm Mini {
    context ctx: *Interp;                 // cold state, reachable from all handlers
    state { pc: *u32, regs: *Value, k: *Value }
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

`state` lists the variables that are **live across every dispatch**:
program counter, stack or register-window pointer, constants pointer,
and so on. Inside handlers they behave like mutable locals. The
compiler keeps them in machine registers for the whole life of the
interpreter:

- in tail-call mode they are the arguments of every handler, in a
  calling convention (`preserve_none`) that gives them fixed registers;
- in goto and switch modes they are loop-carried SSA values in one
  function.

State variables have plain types (`pc: *u32`, `regs: *Value`). A stack
VM keeps a plain pointer to its stack top, and its ops declare stack
effects for the verifier ([section 3.4a](#34a-stack-effects-stack-vms)).

`context` is one more register, holding a pointer to everything else
(heap, frame stack, globals, interned strings). A field read through
`ctx` costs a load, the right price for cold-ish data.

**How many fit.** In tail-call mode every handler takes the context,
the state and the instruction word as `preserve_none` arguments. LLVM 19
has 12 general-purpose and 8 floating-point registers for them on
x86-64, and 24 and 8 on AArch64. A vm that needs more gets the warning
`state_registers`: the rest would be passed on the stack, a store and a
load in every handler. 3 to 6 state variables is typical.

### 3.2 Operand kinds

An operand kind says what an encoded field *means*. Its body is an
expression over the raw field `it`, evaluated in the handler. Its
`verify` clause is checked once, by the generated verifier, instead of
on every execution.

<!-- snippet: check
value Value repr nanbox { Num(f64) unboxed, Int(i32), Nil }
struct Closure { upvals: *Value }
struct Ctx { closure: *Closure }
struct Proto { nregs: u64, consts: []Value, nupvals: u64 }
vm V {
    context ctx: *Ctx;
    state { pc: *u32, regs: *Value, k: *Value }
    verify_context proto: *Proto;
{snippet}
    encoding fixed(u32) { format ABC { op: 8, a: 8, b: 8, c: 8 } }
    op MOVE(dst: reg, src: reg) : ABC { dst = src; next }
    op RET(a: reg) : ABC { exit a }
    entry run(code: *u32, r: *Value) -> Value { regs = r; jump code }
}
-->
```loimi
operand reg   = regs[it]   verify it < proto.nregs;
operand kst   = k[it]      verify it < proto.consts.len;
operand imm   = it;
operand label = pc + it    verify is_insn_boundary(pc + it);
operand upv   = ctx.closure.upvals[it] verify it < proto.nupvals;
```

- If the body is a place (like `regs[it]`), the operand is assignable:
  `dst = Int(1)` writes the register.
- `x.raw` gives the undecoded field.
- The raw field `it` (and `x.raw`) is a `u64`, or an `i64` for a signed
  field (sign-extended), whatever the field's width. So `regs[a + 1]` is
  one 64-bit add. The width and signedness come from the format and
  matter only to the encoders, so one `label` kind serves 8-bit and
  24-bit offsets alike.
- There are no implicit conversions, so a clause that compares with a
  count of another type casts: with `nregs: u32`, write
  `verify (it as u32) < proto.nregs`.
- `display "r{}"` after the clause says how the disassembler prints the
  operand ([below](#disassembler)).

**Verify context.** `verify` clauses may refer to the vm's
`verify_context`, typically the function prototype being checked:
`verify_context proto: Gc<Proto>;`. A verifier can take several context
parameters, in one declaration (`verify_context proto: *Proto, consts:
*Consts;`) or several. They follow the code in `Vm.verify(code, proto,
consts)`, in declaration order. A clause sees `it`, `pc` and the verify
context, nothing else ([section 6](#6-verification-unchecked-by-proof)).

**Constant operands.** A `const` clause says that what an operand reads
cannot change once the code is verified:

<!-- snippet: check
value Value repr nanbox { Num(f64) unboxed, Int(i32), Nil }
struct Consts { data: []Value }
struct Proto { consts: Consts }
vm V {
    state { pc: *u32, regs: *Value, k: *Value }
    verify_context proto: *Proto;
    operand reg = regs[it];
{snippet}
    encoding fixed(u32) { format ABC { op: 8, a: 8, b: 8, c: 8 } }
    op LOADK(dst: reg, c: kst) : ABC { dst = c; next }
    op RET(a: reg) : ABC { exit a }
    entry run(code: *u32, r: *Value) -> Value { regs = r; jump code }
}
-->
```loimi
operand kst = k[it] const proto.consts.data.ptr verify (it as u64) < proto.consts.data.len;
```

- The body must be `S[it]` for a pointer state register `S` (the
  constant pool). A vm has at most one pool.
- The clause is an expression of the verify context with `S`'s type:
  the value `S` has while code verified against that context runs.
- It is a promise, like a `verify` clause: the vm sets `S` to it
  whenever it starts running that code (on calls and returns).
- The checker enforces the rest within the vm: no handler, vm function,
  cold block or `on raise` may write through `S` (`c = x` on the
  operand, `k[i] = x`, `*(k + i) = x`) or take an address in it
  (`&k[i]`). Plain functions and the host must not change the pool of
  verified code either.

`const` changes nothing in the interpreter: the operand reads `S[it]` as
before. With `--jit-stencils`, `Vm.build_verified` passes the pool to
the JIT, which patches each constant into compiled code as an
immediate and specializes on the tag of a `value` constant.

### 3.3 Encodings and formats

<!-- snippet: check
vm V {
    state { pc: *u32, acc: i64 }
    operand imm = it;
{snippet}
    op HALT() : ABC { exit acc }
    entry run(code: *u32) -> i64 { jump code }
}
-->
```loimi
encoding fixed(u32) {                 // every instruction is one u32 word
    format ABC  { op: 8, a: 8, b: 8, c: 8 }
    format ABx  { op: 8, a: 8, bx: 16 }
    format ABsC { op: 8, a: 8, b: 8, sc: s8 }
    format sJ   { op: 8, sj: s24 }
}
```

- Instructions are fixed-width words. The code word of
  `encoding fixed(W)` is `u8`, `u16`, `u32` or `u64`.
- A format's fields are listed from the low bits up. `N` is an unsigned
  field of N bits and `sN` a signed one. The fields need not fill the
  word.
- One field must be `op`.

**Operands and fields.** An op lists its operands and names its format.
Operands map to the format's non-`op` fields by position, whatever
their names, and the compiler type-checks the mapping.

- An op may have fewer operands than the format has fields. The rest
  encode as 0 and are ignored (`op HALT() : ABC`).
- An operand can name its field instead, `name: kind @ field`. The
  other operands take the fields not named, in order.
- The encoder and emitter take the operands in the order written
  (`V.DELGLOBAL(k)`), and the disassembler shows only them.

<!-- snippet: check
vm V {
    state { pc: *u32, regs: *i64, k: *i64 }
    operand reg = regs[it];
    operand kst = k[it];
    encoding fixed(u32) { format ABx { op: 8, a: 8, bx: 16 } format ABC { op: 8, a: 8, b: 8, c: 8 } }
{snippet}
    op RET(a: reg) : ABC { exit a }
    entry run(code: *u32, r: *i64, ks: *i64) -> i64 { regs = r; k = ks; jump code }
}
-->
```loimi
op DELGLOBAL(name: kst @ bx) : ABx { next }   // `a` unused, encodes as 0
op JLT(a: reg @ b, b: reg @ c) : ABC { next }            // `a` left for the compare it was
```

#### Generated API

Each item is `Vm.NAME` (`Vm__NAME` in the vm's module). Items are
private to the module unless the vm is `pub vm`. `W` is the code word
type. A superinstruction's head word answers for its first part. The
field setters, format encoders and disassembler are compiled only into
programs that use them.

**Encoding and emitting**

| Item | What it is |
|---|---|
| `Vm.OP(a, b, ..) -> W` | encoder: the instruction word of `OP` (not for supers). A field of up to 8 bits takes a `u8` (`i8` if signed), up to 16 a `u16`/`i16`, up to 32 a `u32`/`i32` (so a 24-bit field takes `u32`, an `s24` one `i32`), wider a `u64`/`i64`. The value is masked to the field's width without a range check: use `Vm.set_label` or `bc.Field.fits` where it may not fit. |
| `Vm.emit.OP(code, at, a, b, ..) -> u64` | emitter: stores the word at `code[at]`, zeroes the inline cache words, returns `at + op_size` |
| `Vm.emit.OP_cached(code, at, operands.., fields..)` | emitter that also sets cache fields ([section 7.1](#71-inline-caches)) |
| `Vm.encode.FMT(op, a, b, ..) -> W` | the word of opcode `op` in format `FMT`, typed and masked like an encoder: one encoder for every op of a format, for code that picks the opcode at run time (`Vm.encode.ABC(Vm.OP_ADD, d, a, b) == Vm.ADD(d, a, b)`) |

Labels with backpatching and branch range checks are in `std.bc`, a
library over `op_size` and `set_label`:
`bc.Code.new(Vm.op_size, Vm.set_label)`. There is no generated builder
type.

**Opcodes and decoding**

| Item | What it is |
|---|---|
| `Vm.OP_X: W` | opcode constant of each op, family member and super |
| `Vm.NUM_OPS: u64` | number of opcodes; opcodes are `0..NUM_OPS` |
| `Vm.opcode(w) -> W` | the opcode field of a word |
| `Vm.with_opcode(w, op) -> W` | `w` with its opcode replaced |
| `Vm.base_op(w) -> W` | the opcode `w` stands for alone: a super's head gives its first part's op; other words, including quickened family members, give their own |
| `Vm.Op`, `Vm.op_of(w) -> Vm.Op` | an enum with one variant per op as host code sees it (`Vm.Op.ADD`; no supers, a family as its generic op). The discriminant is the opcode (`Vm.Op.ADD as W == Vm.OP_ADD`). `op_of` gives the op `w` stands for alone: a super's head gives its first part, a quickened member its family. An invalid opcode panics. Use it in host tools that `match` over the ops: the match is exhaustive, so adding an op is a compile error there. |
| `Vm.op_size(w) -> u64` | words the instruction at `w` takes, caches included (a super: all its parts) |
| `Vm.base_size(w) -> u64` | size of the op `w` stands for alone |
| `Vm.op_name(w) -> str` | name of the op (`"<invalid>"` for an unused opcode) |
| `Vm.CW_Cache: u64` | code words taken by inline cache layout `Cache` |
| `Vm.FMT_field(w) -> u64` / `i64` | field `field` of format `FMT` (`Vm.AsBx_sbx(w)`); `i64` for a signed field |
| `Vm.set_FMT_field(w, v) -> ?W` | `w` with field `field` of format `FMT` set to `v` (`u64`, or `i64` for a signed field), other bits kept; `None` if `v` does not fit. For peepholes that retarget an operand (`Vm.set_ABC_a(w, r)`). |
| `Vm.label(w) -> ?i64` | the jump offset of `w`: its first operand whose kind is `pc + it` |
| `Vm.set_label(w, off) -> ?W` | `w` with that offset re-encoded; `None` if `off` does not fit or the op has no label |
| `Vm.real_word(w) -> W` | the real word behind a JIT enter word ([below](#the-jit-and-real_word)) |

**Metadata for tools** (fuzzers, assemblers)

| Item | What it is |
|---|---|
| `Vm.operand_count(w)` | the number of operand fields, in encoder order |
| `Vm.operand_kind(w, i)` | field `i`'s kind: an index into the vm's `operand` declarations, in order; `Vm.NUM_KINDS` for none |
| `Vm.operand_bits(w, i)`, `Vm.operand_signed(w, i)`, `Vm.operand_name(w, i)` | field `i`'s width, signedness and name |
| `Vm.NUM_KINDS`, `Vm.kind_name(k)`, `Vm.kind_is_label(k)`, `Vm.verify_clause(k)` | operand kinds, by index |
| `Vm.check_name(i)` | `verify` blocks, by index |
| `Vm.disasm(code, &out)`, `Vm.disasm_insn(code, i, &out) -> u64` | the disassembler ([below](#disassembler)) |

**Other passes and items**

| Item | What it is | See |
|---|---|---|
| `Vm.verify(code, ctx..)`, `Verified<Vm>`, `Vm.VerifyError`, `Vm.VerifyReason`, `Vm.assume_verified`, `Vm.placeholder()`, `Vm.build_verified`, `Vm.build_assume_verified` | the verifier and its proof | [6](#6-verification-unchecked-by-proof) |
| `Vm.stack_effect(w) -> ?(u32, u32)`, `Vm.stack_effect_text(w) -> str`, `v.max_stack() -> u32` | stack VMs: declared pops and pushes, and the verified maximum depth | [3.4a](#34a-stack-effects-stack-vms) |
| `Vm.optimize(v)`, `Vm.optimize_words(code)` | superinstruction peephole, over verified code or a raw slice | [7.3](#73-superinstructions) |
| `Vm.deopt(v)`, `Vm.deopt_words(code)` | back to generic ops: supers split, caches zeroed except resume and `const` fields | [7.2](#72-families-and-become) |
| `Vm.accumulate(code, first_temp)`, `Vm.acc_variant(op, d, s) -> W` | accumulator variant selection | [7.4](#74-accumulators) |
| `Vm.depth: u32` | thread-local count of active nested runs, if the vm has a nested entry | [10](#10-calls-reentrancy-coroutines-embedding) |
| `Vm.ENTRY(..)` | each `entry` | [3.6](#36-entries-and-the-error-path) |

The format declaration is the single source of truth for all of these,
so they cannot drift apart.

#### Disassembler

`Vm.disasm(code, &out)` prints every instruction of `code`.
`Vm.disasm_insn(code, i, &out) -> u64` prints the one at word `i` and
returns its size in words (0 if `i` is past the end).

`out` points to any type with `push_str(str)` and `push_int(i64)`
methods (and `push_f64(f64)` if a cache has a float field), such as
`text.StrBuf`. Each line is the word offset, the op name, and the
operands by name with their decoded values:

```
    0  LOADK dst=r0 c=k3
    1  LOADI_ADD
    1    LOADI dst=r1 v=-7
    2    ADD dst=r2 a=r0 b=r1
    3  GETPROP dst=r3 obj=r2 [cache shape=null slot=0]
    7  JMP target -> 1
```

- Signed fields print as signed.
- A label operand (kind `pc + it`) prints its absolute target:
  `target -> 1`.
- An operand kind can say how its values print with `display`, a string
  with one `{}` for the value (for a label, the target):
  `operand reg = regs[it] verify it < proto.nregs display "r{}";`.
  Without one, the number prints alone.
- Inline cache words print as `[cache field=value ...]`: integers in
  decimal, `bool`s, `f64`s, pointers in hex (`null` for 0).
- A superinstruction prints its name, then each part on its own line at
  its own offset. The parts' words are read as they are, so quickened
  parts show their current op. `then` followers are ordinary
  instructions.
- An invalid opcode prints as `<invalid 0x..>` (size 1). An instruction
  that runs past the end of `code` prints its name and `<truncated>`.

#### Errors in generated code

Part of the API is generated as Loimi source, the vm's *expansion*,
which is type-checked like the rest of the program. Text written in the
vm declaration that lands there (`verify` and `const` clauses,
`verify_context` types, `verify NAME { .. }` blocks, stack effect
counts, `stack_entry`) keeps its location. An error in it is reported
where it was written, with a note naming the generated code it is part
of. A panic in it (a bounds check in a `verify` block) names that line
too:

```
error: mismatched types: expected `u64`, found `u32`
  --> mini.lm:9:40
    9 |     operand reg = regs[it] verify it < proto.nregs;
      |                                        ^^^^^^^^^^^
note: in the generated verifier, checking operand kind `reg`
```

An error elsewhere in the expansion is a compiler bug (`note: internal:
generated code`). `LOIMI_DUMP_VM_EXPANSION=1` prints each vm's
expansion to stderr; `<vm NAME expansion>:LINE` locations refer to its
lines.

#### The JIT and `real_word`

With the copy-and-patch JIT (`--jit-stencils`), the words of compiled
instructions are *enter words* while the JIT holds the code. Every
generated function (including the disassembler and verifier) maps a
word through `Vm.real_word` first, so host code sees the real words.

Host code that inspects `code[i]` itself (hashing, comparing,
serializing) should call `Vm.real_word(code[i])` on instruction words.
Inline cache words are never enter words, but may look like one: read
them as they are. Without the JIT, `real_word` is the identity and
costs nothing.

### 3.4 Ops and control words

<!-- snippet: check
vm V {
    state { pc: *u32, regs: *i64 }
    operand reg = regs[it];
    encoding fixed(u32) { format ABC { op: 8, a: 8, b: 8, c: 8 } }
{snippet}
    op RET(a: reg) : ABC { exit a }
    entry run(code: *u32, r: *i64) -> i64 { regs = r; jump code }
}
-->
```loimi
op MOVE(dst: reg, src: reg) : ABC { dst = src; next }
```

A handler body has type `never`: **every path must end in a control
word**.

| Control word | Meaning |
|---|---|
| `next` | advance `pc` past this instruction and its inline caches, then dispatch |
| `jump t` | set `pc = t` and dispatch |
| `redo` | dispatch the current instruction again (after `become`, a collection, a refill); nothing before it may have an effect ([section 4](#redo-and-what-may-come-before-it)) |
| `exit v` | leave the run loop; the entry returns `v`, of its declared result type (`exit Ok(x)` for a `Result`) |
| `raise e` | transfer to the `on raise` handler with the current `pc` |
| `fail e` | leave the run loop, returning `Err(e)` |

- An operand may be in parentheses: `jump (t as *u32)`, `exit (x)`.
- In vm code the control words are not function names, so `fail (..)`
  never calls a function `fail` (call one through its module,
  `m.fail(..)`). `next (` and `redo (` are errors, since they take no
  operand.
- Outside vm code, a control word is an unknown name, and the error
  says so.
- Handlers and vm functions do not assign `pc` or take its address:
  `next`, `redo` and a superinstruction's next part are relative to the
  instruction, so use `jump` to go elsewhere. Entries and `on raise`
  may set `pc`.
- Inside a handler, `expr?` on a `Result<T, E>` means "on `Err(e)`,
  `raise e`". `E` must be the vm's raise type.

Each instruction's size is static, so `next` compiles to
`pc += size; dispatch`. In fixed-width encodings the compiler also
**decodes ahead**: when the handler can't modify code, it loads the next
instruction word and handler address at the top of the handler, so the
dispatch load overlaps with the handler's work.

### 3.4a Stack effects (stack VMs)

A stack VM declares what each op pops and pushes, after the format (and
cache and `then`), in Forth notation:

<!-- snippet: skip syntax excerpt, handler bodies elided (examples/stack has a complete stack vm) -->
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

- Each item is one value, or `name[count]` for `count` values. `count`
  is an expression over the op's operands (raw fields, as `it` in a
  verify clause) and the verify context.
- Commas between items are optional. The names are documentation (the
  disassembler prints them).
- `..` first among the inputs takes every value on the stack.
- If any op declares an effect, every op must (`stack(--)` for none).
  Family members share their family's.
- `stack_entry EXPR;` (default 0) is the depth the code starts with,
  from the verify context (a function's arguments).

**What the verifier checks.** It walks the control flow: an op continues
to the next instruction if it may continue, and goes to its label
(`pc + it` operand) if it has one. It rejects:

- an op that pops more than the stack holds;
- two paths that reach an instruction with different depths;
- an op that ends the code (it neither continues nor has a label: RET,
  HALT, a tail call) with values left beyond the ones it takes, unless
  it takes `..`.

**Maximum depth.** The verifier records the most the stack grows above
`stack_entry` in the proof, as `v.max_stack() -> u32` on `Verified<Vm>`.
The interpreter checks room once per call (`sp + callee.code.max_stack()
>= stack_end`), and its ops push and pop without checks. Such a vm keeps
its proof in a `Verified<Vm>` field (a `verified [W] by Vm` array has no
room for the bound), and `assume_verified(code, max_stack)` takes the
bound too.

**What is not checked.** The compiler does not check the handlers
against the declared effects: the effect is the contract, as a verify
clause is for an operand. Ops that leave verified code with values on
the stack (an exception handler jumping to a table target) are not
followed; the target's depth is the vm's to ensure.

**For tools.** `Vm.stack_effect(w) -> ?(u32, u32)` gives (pops, pushes)
of the op `w` stands for alone, when they depend only on its operands
(`None` for `..` and for counts that read the verify context).
`Vm.stack_effect_text(w)` gives the declared effect (`a b -- c`).

### 3.5 VM functions

A `fn` inside a `vm` block is a **vm function**.

- It can read and write state.
- It is always inlined into its callers.
- Like a handler, it may call effectful functions without declaring
  their effects; those calls are synced where they are made
  ([section 5](#5-effects-and-sync-points)).

If its return type is `never`, it can also use control words (`jump`,
`next`, `redo`, `raise`, `exit`, `fail`) and `?` on the raise type. That
makes shared tails easy to write:

<!-- snippet: skip vm functions out of context; `...` elides code -->
```loimi
fn call_closure(c: Gc<Closure>, base: *Value, nargs: u8) -> never { ...; jump c.proto.code }

// The word after a conditional op is a JMP holding the branch offset.
fn branch() -> never { jump pc + 1 + ((pc[1] as i32) >> 8) as i64 }
fn cond_branch(take: bool) -> never { if take { branch() } jump pc + 2 }
op JLT(a: reg, b: reg) : ABC { cond_branch(a < b) }
```

#### Rules for `-> never` vm functions

- Every path must end in a control word.
- They can only be called where control words are allowed: op handlers,
  entries, `on raise`, and other `-> never` vm functions.
- `next` and `redo` mean what they mean in the calling op: `next` skips
  the caller's instruction (in a superinstruction part, it continues
  with the next part). A function that uses `next` can only be reached
  from op handlers, not from entries or `on raise`.
- The body sees the vm state and its own parameters, but not the
  caller's operands: pass `dst.raw` or values as arguments.
- `become` is not allowed (it belongs to the calling op), and neither
  is `cache`: pass the cache instead ([below](#passing-the-inline-cache)).

#### `next_pc` and `then_next_pc`

`next_pc` is the address `next` continues at: the calling op's `pc` plus
its size, cache words included. In a superinstruction part, it is the
part's own size, so the address of the next part's original
instruction. It is the right return address for a shared call helper,
for every calling op:

<!-- snippet: skip excerpt with elided code -->
```loimi
fn call(f: *Func, base: u64) -> never { fp = fp + 1; fp.ret = next_pc; ..; jump f.code }
op CALL(b: reg, fi: fidx) : ABC { call(ctx.funcs[fi], b.raw) }
op CALL_IC(b: reg, fi: fidx) : ABC + CallIC { ..; call(f, b.raw) }   // returns past the cache word
```

Writing `pc + 1` instead is wrong for an op with cache words, and
nothing would catch it.

- `next_pc` is available where `next` is: op handlers, and `-> never` vm
  functions reached from them.
- It is an ordinary name: a local or state variable called `next_pc`
  hides it.
- `next_resume` is the same address as a `Vm.Resume`, a resume point
  that an entry may jump to ([section 6](#resume-points)).

**In an op with `then`.** In an op declared `then OP`
([section 7.1a](#71a-the-following-instruction-then)), `next` continues
at the `then` instruction, so `next_pc` is the address *of* that
instruction, not past it. That is rarely the return address you want (a
generator resumed by `FORITER .. then JMP` must continue past the JMP),
so `next_pc` in such an op, directly or in a vm function inlined into
it, gets the warning `next_pc_then`.

- `then_next_pc` is the address past the `then` instruction: the op's
  `pc`, plus its size, plus the `then` op's size.
- Like `then.x`, it is available in the op's own body only. Pass it to
  vm functions as an argument: `resume(g, then_next_pc)`.
- `#[allow(next_pc_then)]` says the `then` word is meant.

#### How calls are compiled

| Dispatch mode | Call site on the hot path | Call site in a `cold` block, or recursive |
|---|---|---|
| `tailcall` | body inlined | `musttail` to one shared copy with the handler signature |
| `goto`, `switch` | body inlined | body inlined (recursion is an error) |

- The shared copy is reached like an outlined cold block. The arguments,
  and the size `next` skips, go through thread-local slots; the state
  stays in registers.
- `#[inline]` on the function inlines every call.
- `#[inline(never)]` makes every call in tail-call mode use the shared
  copy.

#### Passing the inline cache

A vm function (either kind) can take the calling op's inline cache as a
parameter of type `cache Name`, and the op passes `cache`:

<!-- snippet: skip excerpt with elided code -->
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

- `c.field` reads and assigns the op's cache words exactly as
  `cache.field` does in the op, with the same types and unaligned access
  for wide fields.
- The argument must be `cache` in an op whose cache has that layout, or
  another handle of the same layout (a vm function passing its `c` on).
- The handle is not a value otherwise: `c` alone, `let`, and return
  types are errors. The exception: for a cache declared from a struct,
  `cache X = S;`, `&c` is a `*S` that any function takes
  ([section 7.1](#71-inline-caches)).
- A superinstruction part passes the part's own cache.
- `become` stays with the op: a helper that decides to re-specialize can
  return a value the op acts on.

At run time the handle is the address of the instruction's first cache
word, `pc + 1`. In an inlined copy, `c.field` folds to the same access
as `cache.field`, so it costs nothing. A shared copy gets the address
through a thread-local slot, so it reads and fills the cache of
whichever instruction called it.

### 3.6 Entries and the error path

An `entry` is how host code starts a run; `on raise` is where `raise`
goes. MiniJS's entry and error path:

<!-- snippet: excerpt examples/minijs/vm.lm -->
```loimi
entry run(c: *Ctx, cl: Gc<Fn>) -> V uses gc {
    ctx = c;
    let p = cl.proto;
    c.vs.base[0] = Undef;
    regs = c.vs.base + 1;
    for i in 0..p.nregs as u64 { regs[i] = Undef; }
    if !c.vs.ensure(regs + p.nregs as u64 + 1) { exit Undef }
    fp = c.frames;
    fp.cl = cl;
    fp.base = regs;
    fp.flags = rt.FR_BOTTOM;
    k = p.consts.data.ptr;
    fp.k = k;
    c.fp = fp;
    jump p.code
}

on raise(e: V) {
    var f = fp;
    var at = pc;
    loop {
        let p = f.cl.proto;
        if let Some((target, r)) = find_handler(p, at) {
            // `verify handlers` checked that `target` starts an
            // instruction of `p.code` and that `r` is in the frame.
            fp = f;
            regs = f.base;
            k = f.k;
            regs[r] = e;
            jump p.code.start() + target
        }
        if f == ctx.frames { break; }
        f = f - 1;
        at = f.resume - 1;
    }
    ctx.error = e;
    ctx.failed = true;
    ctx.fp = fp;
    exit Undef
}
```

- An entry is an internal Loimi function (`JS.run(c, cl)`). A library
  exposes it to C by wrapping it in an `export "C"` function
  ([section 10.1](#101-embedding-in-a-c-host-implemented)).
- With `#[require_verified]` on the vm, an entry can only start in
  verified code (`p.code` above is `verified [u32] by JS`;
  [section 6](#6-verification-unchecked-by-proof)).
- `on raise` is an ordinary handler: it receives the live state, so the
  `pc` it sees is exact without any syncing.
- An entry whose vm code may collect declares `uses gc` (`entry run(..)
  -> R uses gc`), and its callers then need it too
  ([section 5.2](#52-effects-on-function-types)).
- An entry declared `entry call(..) -> R uses reenter` is a *nested
  entry*: it starts a run above the frames of a run that is already
  active, for host code (a native) that calls guest code
  ([section 10](#10-calls-reentrancy-coroutines-embedding)).
- An entry may continue in the middle of verified code, at a resume
  point (`Vm.Resume`, saved with `next_resume`;
  [section 6](#resume-points)).

### 3.7 Dispatch strategies

Selected per build, with `--dispatch=` or `#[dispatch(...)]` on the vm:

| Strategy | Lowering | When to use |
|---|---|---|
| `tailcall` (default where supported) | one LLVM function per handler, `preserve_none` convention, `musttail` calls | x86-64 and AArch64 with LLVM ≥ 19; best register allocation, fast compiles |
| `goto` | one function; one shared `indirectbr` that LLVM copies into every handler | targets without `preserve_none` |
| `switch` | one function, a loop plus `switch` | portability, wasm without tail calls, debugging |

All three are produced from the same source, so a benchmark can compare
them directly.

- **Loops stay loops.** In `tailcall` mode, handlers (and their outlined
  cold blocks and superinstruction parts) are compiled so that a loop
  that zeroes or copies memory is not turned into a `memset` or
  `memcpy` call, which would give the handler a stack frame.
- **The dispatch table pointer** is a state register if the vm declares
  instrumentation variants ([section 9](#9-instrumentation-variants)),
  and in tail-call mode on AArch64. Otherwise it is a global constant,
  which saves a register.

#### Handler order: `#[profile]`

`#[profile("file")]` on a vm lays out its handlers by a dispatch
profile, in `tailcall` and `goto` builds. The ops the profile counts
come first, the most dispatched first, together in a section that
starts on a page; the rest follow in declaration order. (`switch` and
`--jit-stencils` builds keep declaration order.)

The path is relative to the file declaring the vm. The file is either:

- the standard error of a `--vm-stats` build ([section 7.3](#dispatch-counts---vm-stats)),
  as many runs concatenated as you like (each run's counts weigh the
  same however long it ran); or
- a list of op names, hottest first, one per line, with `//` comments
  allowed.

<!-- snippet: excerpt tests/run/vm_profile.lm -->
```loimi
#[profile("vm_profile.vmprofile")]
vm V {
    state { pc: *u32, acc: i64 }
    encoding fixed(u32) { format A { op: 8, x: 24 } }
    op ADD(x: imm) : A { acc = acc + (x as i64); next }
    op SUB(x: imm) : A { acc = acc - (x as i64); next }
    op MUL(x: imm) : A { acc = acc * (x as i64); next }
    op HALT() : A { exit acc }
```

```sh
loimi build --vm-stats --release main.lm -o vm-stats
for b in bench/*.in; do ./vm-stats $b > /dev/null 2>> vm.vmprofile; done
```

The profile changes only where the machine code goes, never what it
does. An op the profile does not name, or a name that is no longer an op
(a warning: regenerate the profile), is laid out as without one.

Its main use is steadier timings. Without it, which handlers share a
cache line or branch-predictor entry depends on the size of every
function before them, so unrelated changes move single benchmarks by
several percent. With a profile, the hot handlers' layout depends only
on their own code.

`--vm-profile=FILE` uses FILE for every vm of the program instead, and
`--vm-profile=none` uses no profile, for A/B measurements.

---

## 4. Slow paths: `cold` blocks

Handler code splits into a short fast path and a large, rare slow path
(type coercions, metamethods, errors, allocation refills). In C, the
slow path's calls force the compiler to treat the VM registers as
clobbered, so the *fast* path ends up spilling them. Loimi makes the
split explicit:

<!-- snippet: check
value Value repr nanbox { Num(f64) unboxed, Int(i32), Nil }
enum Arith { Add }
struct Interp { x: i64 }
fn arith(c: *Interp, op: Arith, a: Value, b: Value) -> Result<Value, str> { Err("bad operands") }
vm V {
    context ctx: *Interp;
    state { pc: *u32, regs: *Value }
    operand reg = regs[it];
    encoding fixed(u32) { format ABC { op: 8, a: 8, b: 8, c: 8 } }
{snippet}
    entry run(c: *Interp, code: *u32, r: *Value) -> Result<Value, str> { ctx = c; regs = r; jump code }
    on raise(e: str) { exit Err(e) }
}
-->
```loimi
op ADD(dst: reg, a: reg, b: reg) : ABC {
    if let (Int(x), Int(y)) = (a, b) and let Some(r) = x.add_checked(y) {
        dst = Int(r); next
    }
    cold { dst = arith(ctx, .Add, a, b)?; next }
}
```

A `cold` block is compiled so that the fast path is compiled as if the
slow path weren't there. Branches into cold blocks are weighted as
unlikely.

**Which cold blocks are outlined.** In tail-call mode:

- A cold block that ends in control words becomes its own
  handler-shaped function, reached with `musttail` (no call, no return).
  It takes the state registers as arguments, and can use state and
  control words.
- A cold block that falls through, but sits in an `if` whose enclosing
  block goes on to end in a control word (`if unlikely(c) { cold {
  gc.collect(&ctx.heap); } } ..; next`, at any nesting depth in the
  op), gets a copy of that rest, so that it ends in a control word too
  and is outlined.
- Other cold blocks (in a vm function, in a loop, or where the rest of
  the block does not end in a control word) stay inline, marked
  unlikely.

In goto and switch modes, all cold blocks stay inline and are marked
unlikely.

A `cold` block is also an expression:
`let ok = match (a, b) { ..., _ => cold { compare_slow(ctx, a, b)? } };`

**Calls left on the fast path cost a stack frame.** A call left inline
(a slow path outside `cold`, an inline cold block, a panic for
`.unwrap()` or a failed bounds check) needs an aligned stack, and LLVM
often sets up the frame at the handler's entry rather than on the rare
path, so every execution pays a `push` or more. `-W handler_frame`
finds such handlers ([warnings](#warnings)).

### Allocating in a handler

The write barrier after `o.f = v` makes no call, so storing references
costs a handler no frame. Stores of values statically free of
references (`Term.Unb`, `Int(n)`, `None`, literals of such) have no
barrier at all.

The allocator's free-list refill *is* a call. So in an op handler, `new`
is compiled to take only the allocator's inline path, and to refill in a
cold block that runs the op again:

<!-- snippet: skip excerpt of a vm, declarations elided -->
```loimi
op CONS(dst: reg, a: reg, b: reg) : ABC {
    let c = new(&ctx.heap, Pair { head: a, tail: b });
    dst = Value.Pair(c);
    next
}
// is compiled as
op CONS(dst: reg, a: reg, b: reg) : ABC {
    let Some(c) = try_new(&ctx.heap, Pair { head: a, tail: b }) else cold {
        gc.make_room(&ctx.heap, size_of<Pair>());   // never collects
        redo
    };
    dst = Value.Pair(c);
    next
}
```

This happens when it is safe to run the op again:

- the `new` is directly in an op (not in a vm function or a `cold`
  block, and not in a loop of the op);
- its size is constant and at most 496 bytes (an object, or one with a
  trailing array of literal length);
- the heap expression can be evaluated again (`&ctx.heap`);
- nothing before it on any path has an effect that `redo` would repeat
  ([below](#redo-and-what-may-come-before-it)).

Elsewhere `new` is the plain call. `LOIMI_REDO_AUDIT=1` lists the
handlers whose `new` was compiled this way.

`try_new(h, T { .. })` (and `try_new(h, T { .. }, n)` for a trailing
array) is the form to write by hand. It has type `?Gc<T>`: `None` where
`new` would refill. `gc.make_room(h, size)` makes the next allocation
of that size succeed. Use it where the automatic form does not apply,
such as a helper that picks a constant length per case, or a slow path
other than `redo`.

### `redo` and what may come before it

`redo` dispatches the instruction again from the start of its handler,
so it is right only if nothing the op did before it is seen by the
second run. The compiler checks this. On every path to a `redo`, before
the block that ends in it, the op must not:

- store to memory (an operand, an object, the context);
- assign a state register;
- call a function that may store. Functions are summarized through what
  they call, and an extern that takes a pointer may store.

The block that ends in the `redo` is its fix-up and may do anything:
collect, refill the allocator, `become` another op, grow a stack.

These are not effects: allocating, reading, writing locals, `become`,
stores to the instruction's own inline cache, `use_variant`, and calls
into `std.gc` and `std.gil`.

A violation is the error `redo_effect`:

```
error[redo_effect]: `redo` runs `ADD` again from its start, but on the way here the op may already have made a store to memory, and the second run would make it again
  --> vm.lm:12:100
   12 |     poll gc { if unlikely(ctx.heap.allocated > ctx.heap.threshold) { cold { gc.collect(&ctx.heap); redo } } }
note: this runs before the `redo`: put the poll or the allocation before it, or end the path with `next` or `jump`; if running it twice is harmless, `#[allow(redo_effect)]` on the op says so
  --> vm.lm:14:9
   14 |         dst = a + b;
```

The summaries are conservative. If running the code twice is harmless,
`#[allow(redo_effect)]` on the op, the `redo` or the effect says so.

### Polls

A vm declares a check that ops start with once, as a **poll**, and ops
name it:

<!-- snippet: check
import std.gc;
struct Ctx { heap: gc.Heap }
vm V {
    context ctx: *Ctx;
    state { pc: *u32, regs: *i64 }
    operand reg = regs[it];
    operand imm = it;
    encoding fixed(u32) { format ABC { op: 8, a: 8, b: 8, c: 8 } }
{snippet}
    op HALT() : ABC { exit 0 }
    entry run(c: *Ctx, code: *u32, r: *i64) -> i64 uses gc { ctx = c; regs = r; jump code }
}
fn main() {}
-->
```loimi
poll gc { if unlikely(ctx.heap.allocated > ctx.heap.threshold) { cold { gc.collect(&ctx.heap); redo } } }
#[cfg(gil)] poll gil { if unlikely(ctx.gil.waiting()) { cold { ctx.gil.yield_now(); } } }

#[poll(gc)] op SUM(dst: reg, a: reg, n: imm) : ABC {
    var s = 0;
    for i in 0..n as u64 { s += regs[a.raw + i]; }
    dst = s;
    next
}
op JNZ(a: reg, d: imm) : ABC {
    if a != 0 { poll gil; jump pc - d as u64 }
    next
}
```

- `poll NAME { .. }` is a vm item, declared before its uses.
- `#[poll(a, b)]` on an op or a `family` puts the statements of polls
  `a` and `b`, in that order, before the op's own.
- `poll NAME;` is a statement for anywhere in the vm's code (a branch of
  a jump op, a vm function).

The statements are the poll's text, so the code is what writing them
out gives. A poll that ends in `redo` runs the op again after
collecting (nothing has happened yet at its start, which the check
confirms). One whose cold block falls through gets the rest of the op
copied into it. A poll under a `#[cfg]` that is off expands to nothing,
so its uses need no `#[cfg]` of their own. Errors in a poll's statements
point at the declaration.

### Errors and deoptimization are cold by default

In an op handler or a `-> never` vm function, a `raise e`, a `fail e`,
or a `redo` after a `become` (a family member giving up on its guard)
that the code reaches only on some paths is compiled as if it were in a
`cold` block:

- a branch that ends in one (the block of an `if` or an `else`, a
  `match` arm, or the `else` of a `let .. else`) is cold as a whole;
- a block that ends in one after a statement that can leave the block
  is cold from the statement after that one. Statements that can leave
  contain `next`, `jump`, `redo`, `exit`, `raise` or `fail` outside a
  `cold` block, a call of a `-> never` vm function, `return`, `break`
  or `continue`.

<!-- snippet: check
value Value repr nanbox { Num(f64) unboxed, Int(i32), Nil }
struct Interp { x: i64 }
vm V {
    context ctx: *Interp;
    state { pc: *u32, regs: *Value }
    operand reg = regs[it];
    encoding fixed(u32) { format ABC { op: 8, a: 8, b: 8, c: 8 } }
    family NEG(dst: reg, a: reg) : ABC {
        generic { if let Num(x) = a { become NEG_N; } dst = Nil; next }
{snippet}
    }
    entry run(c: *Interp, code: *u32, r: *Value) -> Result<Value, str> { ctx = c; regs = r; jump code }
    on raise(e: str) { exit Err(e) }
}
fn main() {}
-->
```loimi
NEG_N {
    if let Num(x) = a { dst = Num(-x); next }
    if a is Nil { raise "operand is nil" }      // if a is Nil { cold { raise "operand is nil" } }
    let Int(i) = a else { become NEG; redo };   // else cold { become NEG; redo }
    dst = Int(-i);
    next
}
```

This gives the same code as writing `cold` by hand. The rule leaves
these alone:

- a `raise` the code always reaches, such as all of `op THROW(e: reg) :
  ABC { raise e }`. There is no fast path to keep it off; write
  `cold { raise e }` to outline it anyway.
- `?`, whose error arm is part of every call of a function that returns
  a `Result`;
- `redo` without a `become` (a refill can be the op's own path), and
  control words in `cold` blocks, which do not make the rest of a block
  cold;
- variant `before` blocks, `on raise`, entries, plain vm functions, and
  code already in a `cold` block.

`#[hot]` on the `raise`, `fail` or `redo` keeps its branch inline, for
an interpreter that raises on a path it expects to be common (a guest
`throw` used for control flow): `Err(ball) => { #[hot] raise ball }`.

`LOIMI_COLD_AUDIT=1 loimi build ..` lists every branch the rule made
cold. To see which slow paths actually run, `loimi build --vm-stats`
counts the entries into each `cold` block, per op, including the blocks
made cold by default ([section 7.3](#dispatch-counts---vm-stats)).

---

## 5. Effects and sync points

The most common bug in hand-written interpreters is **stale VM state**:
a slow path allocates (and triggers GC, which scans the stack up to a
stale top), or raises an error (with a stale `pc`, so the wrong line or
exception handler), and nobody saved `pc` or `top` first. Lua guards
against this with `Protect()` and `savepc`, CPython with explicit frame
syncs; both rely on discipline. Loimi makes it a type error.

Functions have three ordered **effects**:

| Effect | Meaning | Required at the call site |
|---|---|---|
| `sync` | the callee reads VM state (`pc` for tracebacks, `top`) | the vm's `sync { .. }` block runs (it stores `pc`, `top`, ... where the callee reads them) |
| `gc` | the callee may run a garbage collection | `sync`, plus every live `Gc`/`Value` local made visible as a root |
| `reenter` | the callee may run guest code (call into the VM) | `gc`, plus the state reloaded afterwards (the vm's `reload` block) |

<!-- snippet: skip signatures only -->
```loimi
fn new_string(h: *Heap, s: str) -> Gc<Str> uses gc;
fn traceback(ctx: *Interp) -> String uses sync;
fn call_meta(ctx: *Interp, f: Value, args: []Value) -> Result<Value, Value> uses reenter;
```

**Rules.**

- Effects are part of signatures. Every function declares its effects:
  they are not inferred, and there is no effect polymorphism.
- A function may only call effectful functions if it has at least that
  effect itself, or if it is a handler or `cold` block.
- Code with no effects never writes `pc` to memory.

**What the compiler does at an effectful call:**

- **In handlers** it runs the vm's `sync` block, and puts live
  `Value`/`Gc` locals into a shadow-stack frame, with copies of the
  traced state variables ([section 8](#8-gc-integration)). The heap
  never moves objects, so nothing is reloaded afterwards. Calls without
  effects pay nothing.
- **In plain vm functions** ([section 3.5](#35-vm-functions)), the call is
  synced in place exactly as in a handler, without declaring the effect.
  So `fn gc_point() { if ... { cold { gc.collect(..) } } }` pays for the
  sync only on the path that collects. A handler calling such a function
  roots its own traced locals at that call, but does not sync there.
  Declaring `uses gc` on a vm function instead makes every call of it a
  synced call. Called from an entry, nothing is synced (the run loop has
  not started).
- **In ordinary functions**, `Gc`/`Value` locals that are live across a
  `gc` call are registered in a shadow-stack frame, only on paths that
  can collect. A local is rooted from its initialization on: a
  pattern's bindings once the pattern has matched, so the else block of
  `let PAT = e else { .. }` does not root them.

### 5.1 `reenter` and the cached state

A call with `reenter` may run guest code on the same VM stack, nested
above the calling frame ([section 10](#10-calls-reentrancy-coroutines-embedding)).
Rooting traced locals is not enough: while the guest code ran, the stack
may have been reallocated and frames pushed and popped, so every state
register that caches something derived from them (`regs`, `k`, `fp`, a
stack top) may be stale. (The dispatch table register is not among them:
a nested run starts with the default table, and `use_variant` there
switches only that run's.)

A vm whose handlers or vm functions make such calls declares how to get
its state back, next to `sync`:

<!-- snippet: check
value Value repr nanbox { Num(f64) unboxed, Int(i32), Nil }
struct Frame { base: u64, k: *Value }
struct Ctx { fp: *Frame, stack: *Value }
fn native(c: *Ctx, v: Value) -> Result<Value, str> uses reenter { Ok(v) }
vm V {
    context ctx: *Ctx;
    state { pc: *u32, regs: *Value, k: *Value, fp: *Frame }
    operand reg = regs[it];
    encoding fixed(u32) { format ABC { op: 8, a: 8, b: 8, c: 8 } }
{snippet}
    op CALLN(dst: reg, a: reg) : ABC { dst = native(ctx, a)?; next }
    entry run(c: *Ctx, code: *u32) -> Result<Value, str> uses gc { ctx = c; fp = c.fp; regs = c.stack; k = fp.k; jump code }
    on raise(e: str) { exit Err(e) }
}
-->
```loimi
sync   { ctx.fp = fp; }                            // before: publish
reload { fp = ctx.fp; regs = ctx.stack + fp.base; k = fp.k; }   // after: recompute
```

- At each `reenter` call in an op, `on raise`, `cold` block or vm
  function, the compiler runs `sync`, roots traced locals, makes the
  call, then runs `reload`. The call's result is used after the reload,
  so `dst = native(ctx, a)?` stores into the reloaded `regs`.
- A `reenter` call in vm code without a `reload` block is an error.
- `reload` should recompute state from what the nested run cannot have
  invalidated: offsets, or the frame record the native's caller left,
  not pointers saved before the call. `pc` usually needs no reload (code
  does not move). The current instruction word, decoded operand fields
  and `next`'s size are fixed.

**What `reload` cannot fix** is the handler's own derived data. The
compiler does not check these, so keep the code after a `reenter` call
short:

- a raw pointer into the stack computed before the call
  (`let p = regs + a`) is stale after it: compute it again;
- anything read from the heap or the code before the call: the guest
  code may have changed tables, upvalues (and through them the caller's
  registers), inline cache words, or quickened and fused the very
  instruction that is running (its operands stay the same, so `next` is
  still right).

### 5.2 Effects on function types

A function type carries effects with the same syntax as a declaration:

<!-- snippet: check
import std.gc.{Heap};
value Value repr nanbox { Num(f64) unboxed, Int(i32), Nil }
struct Interp { x: i64 }
{snippet}
-->
```loimi
struct Hooks {
    scan:  fn(*Heap, *u8),                      // no effects
    alloc: fn(*Heap, u64) -> *u8 uses gc,       // may collect
    call:  fn(*Interp, Value) -> Value uses reenter,
}
```

- **One effect.** A function type names the strongest effect (`uses gc`
  includes `sync`). A declaration and its value have the same effect:
  `fn f(h: *Heap) uses gc` has type `fn(*Heap) -> () uses gc`.
- **Conversion.** A function value converts to a function type whose
  effects include its own: a plain `fn(*Heap)` can be stored in a
  `fn(*Heap) uses gc` field, but not the other way around. This applies
  to function values and `?fn` options, not behind pointers: `*fn() uses
  gc` and `*fn()` are unrelated types. Function types are otherwise
  compared exactly, so `if c { f } else { g }` with different effects may
  need an annotation.
- **Calls through a pointer** have the effects of the pointer's type,
  with the same rules and code as a direct call.
- **Closures** follow the same rules: `block fn(i64) uses gc`,
  `closure fn(i64) uses gc` and `Gc<closure fn(i64) uses gc>` are called
  like a `uses gc` function, a closure's body may only make the calls
  its type allows, and a closure converts to the same kind of closure
  type with more effects.
- **Traits.** Calls through a trait bound have the effects of the
  trait's method declaration; an impl's method may have fewer.
- **Generics** carry effects through: `T` bound to `fn() uses gc` keeps
  the effect. Inference does not look at effects; the argument is
  checked against the instantiated type afterwards. There are no effect
  variables, so a generic function takes either plain or effectful
  callbacks, not both.
- **Precedence.** `uses` binds to the innermost function type: in
  `fn get() -> fn() uses gc { .. }` the effect is part of the result
  type, not of `get`. Parenthesize to make it explicit:
  `-> (fn() uses gc)`. `(T)` is the same type as `T`; a one-element tuple
  is `(T,)`.
- **Callbacks that must not have effects** say so in their type. The
  GC's root scanners run during a collection, so `Heap.add_scanner`
  takes a `fn(*Heap, *T)` and rejects a `uses gc` function.

**Entries** carry the effects of the vm code they run.

- If any op, `on raise`, vm function, cold block or variant may collect
  (it makes a `uses gc` or `uses reenter` call), every entry of the vm
  declares `uses gc` (`entry run(c: *Ctx, cl: Gc<Closure>) -> Result<V,
  V> uses gc`), or `uses reenter` if it is a nested entry.
- The effect is part of the entry's function type, so its callers
  declare it too, and the entry converts only to function types that
  include it.
- An entry's own body calling a `uses gc` function needs the declaration
  as well.
- `reenter` reached from vm code does not make a plain entry `reenter`:
  the entry starts a run with no other run of the vm active, so its
  caller has no vm state to reload.

Allocation fast paths are inlined (`std.gc` bump allocation), and only
their refill path carries `uses gc`, so allocating in a handler costs
nothing extra unless the refill runs.

---

## 6. Verification: unchecked by proof

`Vm.verify(code, ctx..) -> Result<Verified<Vm>, Vm.VerifyError>`
is generated from the declarations. It returns the code wrapped in a
proof, or the first bad word and why it is bad. Operand accesses in
handlers then compile **without bounds checks**, and stack VMs check
for overflow once per call instead of on every push. The checks move
from run time to load time, which matters for interpreters that load
cached or untrusted bytecode.

**What it checks:**

- every word decodes to a valid opcode and format;
- every operand satisfies its kind's `verify` clause (register and
  constant indices in range, upvalue indices valid, ...), and every
  `const` inline cache field its own ([section 7.1](#71-inline-caches));
- every `label` lands on an instruction boundary, never inside an
  instruction or an inline cache;
- in a superinstruction, each later part's word holds an op of that
  part's family. Those words are instructions of their own (a jump may
  land on them) and are checked as such;
- `then OP`: the next instruction is an `OP` or a member of its family;
- stack VMs: depths at every instruction ([section 3.4a](#34a-stack-effects-stack-vms));
- `verify NAME { .. }` blocks ([side tables](#side-tables));
- code can't fall off the end: it is not empty, and its last instruction
  cannot continue past itself.

**"May continue past itself."** An op may continue if its handler uses
`next` or reads `pc` (to save a return address like `pc + 1`), directly,
in a vm function it calls, or in an op it can `become`.

- A vm function called with literal `true`/`false` arguments is looked
  at per call: `call_slow(base, n, true)` does not continue if `next`
  and `pc + 1` are only under `if !tail { .. }`.
- An op with `then` that may continue (`jump pc + 2` past its JMP) needs
  an instruction after that JMP.
- Code whose last instruction mentions `next_pc` or `next_resume` is
  rejected, as if it used `next`.
- This scan reads an op's tokens, so a statement dropped by `#[cfg]`
  that reads `pc` or says `next` makes it more conservative, never
  wrong.

`verify` clauses see `it`, `pc` (the address of the operand's word; for
a cache field's clause, of the instruction's) and the verify context,
nothing else. A verify context cannot be named `it` or `pc`.

### Errors

`Vm.VerifyError` has:

- `at`: the word index;
- `reason`: a `Vm.VerifyReason` (below);
- for `Operand`, the operand's index and kind;
- `msg`: a `verify` block's message (for `Cache`, the field and the
  clause).

`e.message()` describes the reason, and `e.write(&out)` writes the whole
diagnosis to any buffer with `push_str`/`push_int`:

```
word 4 (LOADK): operand `c` (kst) fails `(it as u64) < nconsts`
word 7 (ADD): execution can run off the end of the code after this instruction
word 9 (DROP): stack underflow: the instruction takes 1 value(s), the stack has 0
```

| `VerifyReason` | Meaning (`e.message()`) |
|---|---|
| `Empty` | the code is empty |
| `TooLong` | the code is longer than 524288 words |
| `BadOpcode` | invalid opcode |
| `Truncated` | the instruction runs past the end of the code |
| `SuperPart` | a superinstruction's word holds an op outside its part's family |
| `Forward` | a forwarding superinstruction's linked operands name different registers ([7.3](#forwarding)) |
| `Then` | the instruction is not followed by the op its `then` requires |
| `RunsOffEnd` | execution can run off the end of the code after this instruction |
| `Operand` | an operand fails its kind's verify clause |
| `Check` | a `verify` check of the vm fails ([side tables](#side-tables)) |

Stack VMs add `StackUnderflow`, `StackMismatch` (paths reach an
instruction with different depths), `StackExit` (the instruction ends
the code with values left on the stack), `StackTooDeep` (the stack can
grow past 2^31 values), `StackCount` (a stack effect's count is out of
range) and `BadTarget` (a jump target lies outside the code). Vms whose
`const` cache fields have `verify` clauses add `Cache`.

### The proof: `Verified<Vm>`

`Verified<Vm>` (also spelled `Vm.Verified`) is an opaque handle
over the code slice: the same pointer and length at run time. Only the
vm's generated functions can build one. Its field is private to the
vm's expansion (fields whose names start with `__` are private to the
file declaring their struct), so a struct literal or a field write
outside it is an error.

| Method | Result |
|---|---|
| `v.start() -> *W` | the first word |
| `v.len()` | the number of words |
| `v.words() -> []W` | the words, for reading (disassembly, line tables); writing through it voids the proof |
| `v.index_of(at: *W) -> u64` | the index of the word `at` points at, for line tables and exception ranges |
| `v.resume_at(i) -> ?Vm.Resume` | a resume point at word `i` ([below](#resume-points)) |
| `v.max_stack() -> u32` | stack VMs: the verified maximum depth |
| `v.fill_resumes(vals) -> bool` | fills `Vm.Resume` cache fields ([section 7.1](#code-addresses-vmresume-fields)) |

In vm code, `jump v` starts running the verified code.

**Escape hatches:**

- `unsafe { Vm.assume_verified(code) }`, for code the host built
  itself and trusts. `assume_verified` is declared `#[unsafe]`, so
  naming it outside `unsafe { }` is an error.
- `Vm.placeholder()`, a proof over one static word holding an opcode
  no op has, which traps if it runs. It fills a `Verified` field until
  the real code is verified (a prototype built before the rest of the
  program is known). It is not generated if every opcode value is
  taken.

### Requiring the proof: `#[require_verified]`

`#[require_verified]` on the vm makes every entry start in verified
code. An entry's `jump` must take a `Verified<Vm>` (`entry run(code:
Verified<Vm>, ..) { ..; jump code }`), so `Vm.run` cannot be called
with raw code.

- A raw `jump` in an entry (or in a `-> never` vm function inlined into
  one) is an error outside `unsafe { }`.
- An entry cannot reach a shared (cold) copy of a vm function.
- Calls inside the vm are made safe by the data: a prototype whose code
  is verified (a `Verified<Vm>` field, or a [verified
  array](#verified-code-in-an-object)) has no raw code to jump to, so
  `jump p.code` in CALL reaches only verified functions.

### Resume points

An entry may also continue in the middle of verified code, where an
earlier run left it: after a coroutine's yield, say. `Vm.Resume` is
the proof for that: an instruction boundary of verified code. `jump r`
with `r: Vm.Resume` is allowed in an entry under `#[require_verified]`.

How to get one:

| Source | Where | Notes |
|---|---|---|
| `next_resume` | vm code, where `next` is available | `next_pc` ([section 3.5](#next_pc-and-then_next_pc)) as a `Vm.Resume`; the verifier guarantees the next instruction exists |
| `v.resume_at(i) -> ?Vm.Resume` | on a `Verified<Vm>` | the instruction starting at word `i`, or `None`. It walks the code from its start (for host code such as a debugger, not a hot path). |
| `Vm.Resume.placeholder()` | anywhere | a resume point at the trap word of `Vm.placeholder()`, for a slot that needs a value before there is anything to resume |
| `unsafe { Vm.assume_resume(p) }` | anywhere | from a raw pointer, like `assume_verified` |

`r.pc()` reads the address.

A coroutine yield saves its resume point:

<!-- snippet: excerpt examples/mini/vm.lm -->
```loimi
fn yield_exit(base: u64, nres: u64) -> never {
    ctx.nret = NRET_ONE;
    ctx.ypc = next_resume;
    ctx.ydst = base as u32;
    ctx.ynres = nres as u32;
    ctx.fp = fp;
    exit Ok(Nil)
}
```

and the entry that continues the coroutine jumps to it:

<!-- snippet: excerpt examples/mini/vm.lm -->
```loimi
entry resume(c: *Ctx, vals: *V, n: u64) -> Result<V, V> uses reenter {
    ctx = c;
    fp = c.fp;
    regs = fp.base;
    k = fp.k;
    let dst = regs + c.ydst as u64;
    for i in 0..c.ynres as u64 { dst[i] = if i < n { vals[i] } else { V.Nil }; }
    // The instruction after the yield's CALL: a resume point, which
    // only vm code makes (`next_resume`), so `#[require_verified]`
    // lets the entry jump to it.
    #[cfg(debug)] if debugger.stepping(c) { use_variant step; } jump c.ypc
}
```

**What is checked:**

- *At compile time:* only the four sources above build a `Vm.Resume`.
  Its field is private to the vm's generated code, so a struct literal
  cannot forge one, and `..zero` cannot make a null one: a slot that
  starts empty holds `Vm.Resume.placeholder()`, or is a
  `?Vm.Resume`. The type is per vm: another vm's entry cannot jump to
  it.
- *At verification time:* code cannot end with an instruction that saves
  its successor, so `next_resume` always names an instruction.
- *At run time:* `resume_at` checks the boundary, and the placeholder
  traps.

**What is not checked** is the same as for return addresses (`jump
fp.resume`) and for a `Verified<Vm>` kept outside its object: that the
code is still there when the point is used, and that the state the
entry installs (registers, constants, frame) belongs to that code. A
resume point is an address, not a reference: it does not keep the code
alive, so the vm must keep the code reachable while it holds one.
Rewriting verified code in place (`become`, `optimize`, `deopt`) keeps
every boundary a boundary, so a resume point survives quickening. As
with `Verified`, the proof is about the type: raw pointer casts, which
Loimi allows without `unsafe`, can still forge anything.

The types and functions are generated only for programs that mention
them (`next_resume`, `Resume`, `resume_at` or `assume_resume`).

### Verified code in an object

An `object` can hold its code as its trailing array, marked as verified
by a vm:

<!-- snippet: skip excerpt with elided fields and code (tests/run/vm_verified_object.lm is complete) -->
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

- `Vm.build_verified(heap, init, words, ctx..) -> Result<Gc<Obj>,
  Vm.VerifyError>` allocates the object with room for the words,
  copies them in, and verifies them there. It is the only way to create
  such an object: `new` on it is an error.
  `unsafe { Vm.build_assume_verified(heap, init, words) }` skips the
  verifier.
- `p.code` reads as a `Verified<Vm>` over the array (`start()`,
  `len()`, `words()`, `Vm.optimize(p.code)`). It is not a place, so it
  cannot be assigned.
- `jump p.code` is the array's address, `p` plus a constant: no load,
  unlike a `Verified` field, which costs one dependent load per call.
- A verify context declared **`owner`** is the object being built:
  `build_verified` passes the new object, and the other context
  parameters are the caller's. The verify clauses then read the counts
  (and tables) the object holds, so the proof is tied to the object.
  `Vm.verify(code, proto)` still takes it explicitly, for checking
  code against an existing prototype.
- The object's other fields remain ordinary fields: the interpreter must
  not change those the verifier read (`nregs`, `consts`) afterwards.

Without an owner context, the proof records nothing about the context
it was checked against, and the interpreter stores it next to that
context. A common pattern is *build, then verify*: create every
prototype with `code: Vm.placeholder()`, and verify once the program is
complete (when, say, the global count the context needs is known),
storing each proof in its prototype (`p.code = v`).

### Side tables

`verify NAME { .. }` in a vm is a `bool` block that the verifier runs
after checking every instruction. It sees the verify context, the code
as `code` (a `[]W`) and `is_insn_boundary(p)`. Use it to check tables
the code refers to but the operand clauses do not see, such as an
exception handler table, so that `on raise` jumps only to
instructions:

<!-- snippet: check
struct Table { data: []u32 }
struct Proto { nregs: u32, handlers: Table }
vm V {
    state { pc: *u32, regs: *i64 }
    verify_context proto: *Proto;
    operand reg = regs[it] verify (it as u32) < proto.nregs;
    encoding fixed(u32) { format ABC { op: 8, a: 8, b: 8, c: 8 } }
    op RET(a: reg) : ABC { exit a }
{snippet}
    entry run(code: *u32, r: *i64) -> i64 { regs = r; jump code }
}
-->
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

- A failing check is `VerifyReason.Check`, written as ``check `handlers`
  fails``.
- To say where and why, call `reject(at, msg)` in the block. It fails
  the check at word `at` with message `msg`, written like the generated
  checks: ``word 4 (DUP): check `stack` fails: depth 3 > 2``.
- With an owner context, the table is the object's own, verified with
  its code.

`code` in a check is the **base view** of the code: every word as the op
it stands for alone (its family's generic op; a superinstruction's head
as its first part's), with inline caches zeroed except their `const`
fields, as `Vm.deopt` would leave it. So a check written against the
plain ops still holds for code that went through `optimize` or
quickening. If the vm has supers or families, `code` is a copy, and
`is_insn_boundary` takes pointers into it. (`Vm.base_op(w)` maps one
word the same way, for host code.)

### Mutation

Quickening (`become`) and `Vm.optimize` rewrite verified code in
place, and both preserve verification:

- `become` only switches between members of a family, which share
  format, operand kinds and cache;
- every word of a superinstruction is verified as an instruction of its
  part's family, so quickening a part or splitting a super leaves valid
  code;
- `optimize` fuses only sequences that verify as their parts.

`Vm.optimize(v) -> Verified<Vm>` and `Vm.deopt(v) ->
Verified<Vm>` take and return the proof. The raw passes,
`Vm.optimize_words(code)` and `Vm.deopt_words(code)`, are for code
that is verified afterwards.

### What the vm must ensure

The verifier and the proof cannot see:

- that the run-time state matches the verify context (the register
  window really has `nregs` registers);
- that operands without a `verify` clause (`imm`) are safe for every
  value;
- that the context is not changed after verification;
- that jumps to computed addresses inside the vm go to boundaries of
  verified code: return addresses (`jump fp.resume`, saved by a verified
  CALL) and table targets, unless a `verify` block checks the table.

Those raw jumps are allowed in ops and `on raise`, where `pc` is already
in verified code; only entries are held to `Verified`.

---

## 7. Specialization: quickening, inline caches, superinstructions

### 7.1 Inline caches

<!-- snippet: run "4"
import std.io;
struct Shape { id: u32 }
vm V {
    state { pc: *u32, regs: *i64 }
    operand reg = regs[it];
    operand imm = it;
    encoding fixed(u32) { format ABC { op: 8, a: 8, b: 8, c: 8 } }
{snippet}
    op GETPROP(dst: reg, obj: reg, name: imm) : ABC + PropIC { next }
    op RET(a: reg) : ABC { exit a }
    entry run(code: *u32, r: *i64) -> i64 { regs = r; jump code }
}
fn main() { io.print_int(V.op_size(V.GETPROP(0, 1, 2) as u32) as i64); }
-->
```loimi
cache PropIC { shape: ?*Shape, slot: u32 }
```

A cache is stored **inline in the code stream**, after the instruction
(as in CPython 3.11+), and is attached with `+`: `op GETPROP(...) : ABC +
PropIC`. Inside the handler, `cache.shape` reads and writes the words
directly.

- `next` skips the cache.
- The verifier forbids jumps into it.
- The emitters reserve and zero-fill it.
- The disassembler prints it.

**Field types.** A field is one of:

- an integer, a float, or `bool`;
- a pointer or a function pointer;
- an option of a pointer (`?*Shape`; `None` is null, which is what a
  zero-filled cache holds);
- the vm's resume point `Vm.Resume` ([below](#code-addresses-vmresume-fields)).

**No GC references.** A cache field cannot hold `Gc<T>`, a `value` type
or anything else the collector traces: the collector does not scan
bytecode, so the reference would not keep its target alive. Cache
something whose lifetime the interpreter controls instead (a shape id
that is never reused, a pointer to a structure that is never freed, a
version number), and validate it on use. For weak caching of GC
objects, use a side table that a sweeper clears, or `std.gc.Weak`.

**Sizes.** Each field starts a new code word and takes as many words as
its type needs. In a `u32` vm, `u32` and smaller take one word, and
`u64`, `f64` and pointers take two, so `PropIC` above is three words and
`op_size` of `GETPROP` is 4. In a `u64` vm every field is one word.

**Alignment.** A two-word field is only word-aligned, so handlers read
and write it with unaligned loads and stores, which cost nothing extra
on x86-64 and AArch64.

- Taking a wide field's address (`&cache.shape`) is an error, since the
  pointer would claim an alignment the field does not have. Fields no
  wider than a word may be passed by address.
- Wide fields are written with ordinary stores, which are not atomic if
  the code is shared between threads.

**Initial contents.** `Vm.emit.OP_cached(code, at, operands..,
fields..)` emits an op with its cache's integer, `bool` and pointer
fields set (in declaration order; other fields zero). It is generated
for ops whose cache has such fields, in programs that name one.

#### `const` fields

A field declared `const` is part of the instruction, not something it
learns:

- the emitter sets it (`Vm.emit.OP_cached`, or by writing the words
  after `Vm.emit.OP`);
- vm code only reads it: assigning it (`cache.k = ..`, `c.k` through a
  handle) or taking its address is an error;
- `Vm.deopt` keeps it;
- the verifier can check it. A `verify` clause after the field's type,
  as for an operand kind, sees the field's value as `it`, `pc` (the
  instruction's address) and the verify context, and is checked once
  per instruction.

`cache X = const S;` makes every field of a struct layout `const`. Its
clause (`cache X = const S verify e;`) sees `&cache`, a `*S`, as `it`:

<!-- snippet: run "7 hits 1, deopt: k 7 hits 0, k 9 refused"
import std.io;
import std.math;
struct Proto { nk: u32 }
vm V {
    state { pc: *u32, regs: *i64 }
    verify_context proto: *Proto;
    operand reg = regs[it];
    encoding fixed(u32) { format AB { op: 8, a: 8, b: 8 } }
{snippet}
    op RET(a: reg) : AB { exit a }
    entry run(code: *u32, r: *i64) -> i64 { regs = r; jump code }
}
fn main() {
    var p = Proto { nk: 8 };
    var code: [u32; 16] = [0; 16];
    var at = V.emit.ADDK_cached(code[..], 0, 0, 7, 0);
    let m = at;
    at = V.emit.MODK(code[..], at, 0);
    let Some(dv) = math.Divisor.new(1000) else { trap() };
    dv.store(code[m + 1..m + 5]);
    at = V.emit.RET(code[..], at, 0);
    let Ok(_) = V.verify(code[0..at], &p) else { trap() };
    var r: [i64; 1] = [2000];
    io.print_int(V.run(&code[0], &r[0]));                // (2000 + 7) mod 1000
    io.print(" hits "); io.print_int(code[2] as i64);
    V.deopt_words(code[0..at]);
    io.print(", deopt: k "); io.print_int(code[1] as i64);
    io.print(" hits "); io.print_int(code[2] as i64);
    code[1] = 9;
    match V.verify(code[0..at], &p) { Ok(_) => io.println(", k 9 verified"), Err(_) => io.println(", k 9 refused") }
}
-->
```loimi
cache KIx { const k: u32 verify it < proto.nk, hits: u32 }
cache DivK = const math.Divisor verify it.is_valid();
op ADDK(a: reg) : AB + KIx { a = a + cache.k as i64; cache.hits += 1; next }
op MODK(a: reg) : AB + DivK { a = (&cache).rem_floor(a); next }
```

- `&cache` of a `const` struct layout is still a `*S`, for methods that
  read it. Writing through that pointer is not checked.
- Reading a `const` field costs what reading any other field does.
- A `const` field cannot be a `Vm.Resume`.
- `verify` clauses are only for `const` fields: what a handler writes is
  not known at verification time.

#### Layouts from structs: `cache X = S;`

A cache may take its fields from a struct: `cache PropIC = obj.Site;`.
The fields are `S`'s, in order, with the rules above, and `cache.f`
reads and writes them as before. The struct adds a type for the whole
cache: in an op, `&cache` is a `*S`, and in a vm function, so is `&c`
for a handle `c: cache X` ([section 3.5](#passing-the-inline-cache)). So
the slow path that fills a cache can be a plain function, written once
and called from any op, vm function or host code that has an `S`:

<!-- snippet: run "miss 110, hit 10"
import std.io;
struct Site { shape: u32, slot: u32 }
fn fill(s: *Site, shape: u32) { s.shape = shape; s.slot = shape * 2; }
vm V {
    state { pc: *u32, regs: *i64 }
    operand reg = regs[it];
    operand imm = it;
    encoding fixed(u32) { format ABC { op: 8, a: 8, b: 8, c: 8 } }
{snippet}
    op RET(a: reg) : ABC { exit a }
    entry run(code: *u32, r: *i64) -> i64 { regs = r; pc = code; jump pc }
}
fn main() {
    var code: [u32; 16] = [0; 16];
    var at = V.emit.GET(code[..], 0, 0, 5);
    at = V.emit.RET(code[..], at, 0);
    var r: [i64; 2] = [0, 0];
    io.print("miss "); io.print_int(V.run(&code[0], &r[0]));
    io.print(", hit "); io.print_int(V.run(&code[0], &r[0])); io.println("");
}
-->
```loimi
cache SiteIC = Site;       // struct Site { shape: u32, slot: u32 }
op GET(dst: reg, shape: imm) : ABC + SiteIC {
    if cache.shape == shape as u32 { dst = cache.slot as i64; next }
    cold { fill(&cache, shape as u32); dst = 100 + cache.slot as i64; next }
}
```

Here `fill(s: *Site, shape: u32)` is an ordinary function outside the
vm. `&cache` is the address of the first cache word, so it costs
nothing.

`&cache` is a `*S` only where the words hold an `S` as laid out in
memory: every field must be at the byte offset of its word, and `S` no
more aligned than a word.

- In a `u32` vm, a struct of `u32`, `i32` and `f32` fields works (and
  `u8` or `bool` fields followed by nothing narrower); one with a `u64`
  or a pointer does not.
- In a `u64` vm, a struct of 8-byte fields works.
- Elsewhere `&cache` is an error that says which field is out of place,
  and the fields stay usable one by one.
- A cache declared with a field list has no struct type, so `&cache`
  there is an error too (take a field's address).

`&cache` points into code, which does not move while it runs, but the
pointer is only good while that code is alive: pass it down to the
functions that fill the cache, and do not keep it.

#### Code addresses: `Vm.Resume` fields

A cache field of type `Vm.Resume` holds a [resume point](#resume-points),
an instruction boundary of verified code. So `jump cache.to` is a jump
to verified code that `#[require_verified]` accepts, with no `unsafe`.
Use it for a try chain (each entry continuing at a clause) or a call
site that caches its callee's code.

Such a field is set only by vm code (`cache.to = next_resume`, or any
other `Vm.Resume`) and by the host through the proof:

- the verifier sets every `Vm.Resume` field of the code it checks to
  `Vm.Resume.placeholder()` (which traps if jumped to), whatever the
  words held: words the host wrote are not trusted;
- `v.fill_resumes(vals) -> bool`, on a `Verified<Vm>` (or a verified
  array, `p.code`), writes `vals[0]`, `vals[1]`, .. into the resume
  fields of its instructions in code order. It returns false if their
  number differs from `vals.len` (it fills what it can);
- `deopt` keeps resume fields (and `const` fields; it zeroes the rest).

<!-- snippet: skip excerpt: Prolog's try chains (examples/prolog), declarations elided -->
```loimi
cache Target { to: Wam.Resume }
op TRY(n: imm) : A + Target { push_choice(n, next_pc); jump cache.to }

// chain = [TRY, RETRY, .., TRUST] with zeroed caches, then
let ch = Wam.build_verified(heap, Code {}, words, &vc)?;
ch.code.fill_resumes(targets);   // targets[i] = clause[i].code.resume_at(0).unwrap()
```

As for other resume points, the vm must keep the target code alive.

### 7.1a The following instruction: `then`

Lua-style conditional ops are followed by a JMP word that holds the
branch offset, so jump patching and the verifier treat it as an
ordinary jump. `then OP` after the format (and cache) says that the next
instruction is an `OP`, or a member of its family. `then.x` reads its
operand `x`: decoded from that word with `OP`'s format, and evaluated by
`OP`'s operand kind with `pc` at that word.

<!-- snippet: check
vm V {
    state { pc: *u32, regs: *i64 }
    operand reg = regs[it];
    operand label = pc + it verify is_insn_boundary(pc + it);
    encoding fixed(u32) { format ABC { op: 8, a: 8, b: 8, c: 8 } format sJ { op: 8, sj: s24 } }
{snippet}
    op RET(a: reg) : ABC { exit a }
    entry run(code: *u32, r: *i64) -> i64 { regs = r; jump code }
}
-->
```loimi
op JMP(target: label) : sJ { jump target }
op LT(a: reg, b: reg) : ABC then JMP {
    if a < b { jump then.target }
    jump then_next_pc              // skip the JMP
}
```

- The JMP stays a separate instruction: `next` skips only `LT`.
- `next_pc` is the JMP's address, and `then_next_pc` the address after
  it ([section 3.5](#next_pc-and-then_next_pc)).
- The verifier checks that the JMP is there, and checks its operands at
  its own word.
- `then` operands are read-only, and not available in vm functions
  (pass them as arguments).

### 7.2 Families and `become`

A `family` is a group of ops that share operands, format and cache, with
specialized implementations. `become X` rewrites the current
instruction's opcode in place.

<!-- snippet: skip sketch over an object model that is not shown (examples/minijs has working GETPROP families over std.obj) -->
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

- The `generic` member, named after the family (`become GETPROP`), is
  what the emitters produce. Specialized members are only reached
  through `become`.
- Rewrites are single-word, non-atomic stores: code is not shared
  between threads that run at the same time
  ([section 10.1](#threads)).

**`Vm.deopt(v)`** maps every member back to `generic`, for debuggers and
for code serialization. It rewrites each instruction word to its
family's generic op, splits superinstructions (the head becomes its
first part's generic op), and zeroes inline caches except their resume
and `const` fields. The result is the code the emitters produced.

**`become match`** picks the member from a value, usually the state an
inline-cache miss handler returns (`std.obj`'s `IcState`):

<!-- snippet: check
enum IcState { Generic, OwnIn, OwnOut, Proto, Poly, Mega }
fn miss(k: u64) -> IcState { if k == 0 { IcState.OwnIn } else { IcState.Mega } }
vm V {
    state { pc: *u32, regs: *i64 }
    operand reg = regs[it];
    operand imm = it;
    encoding fixed(u32) { format AB { op: 8, a: 8, b: 16 } }
    family GET(dst: reg, k: imm) : AB {
        generic {
            let st = miss(k);
{snippet}
            next
        }
        GET_IN { next }
        GET_OUT { next }
        GET_POLY { next }
    }
    entry run(code: *u32, r: *i64) -> i64 { regs = r; jump code }
}
-->
```loimi
become match st {
    OwnIn => GET_IN,
    OwnOut | Proto => GET_OUT,
    Poly => GET_POLY,
}
```

- Each arm is a pattern of variants of the scrutinee's enum, and an op.
- The scrutinee is evaluated once.
- It compiles to the same code as the `if st == .. { become ..; }`
  ladder.
- A value no arm names keeps the current op, so there is no `_` arm.

### 7.3 Superinstructions

A superinstruction (*super*) runs several ops as one dispatch:

<!-- snippet: check
vm V {
    state { pc: *u32, regs: *f64, k: *f64 }
    operand reg = regs[it];
    operand kst = k[it];
    operand imm = it;
    encoding fixed(u32) { format ABC { op: 8, a: 8, b: 8, c: 8 } }
    op LOADK(dst: reg, c: kst) : ABC { dst = c; next }
    op LOADI(dst: reg, v: imm) : ABC { dst = v as f64; next }
    op MOVE(dst: reg, src: reg) : ABC { dst = src; next }
    family ADD(dst: reg, a: reg, b: reg) : ABC {
        generic { dst = a + b; become ADD_F; next }
        ADD_F { dst = a + b; next }
    }
    op RETURN(a: reg) : ABC { exit a }
    op RET1(a: reg) : ABC { exit a }
{snippet}
    entry run(code: *u32, r: *f64) -> f64 { regs = r; k = r; jump code }
}
-->
```loimi
super LOADK_ADD = LOADK; ADD;
super MOVE_RETURN = MOVE; RETURN;
super LOADI_ADD_F_RET = LOADI; ADD_F; RET1;   // any number of parts; family members too
```

The compiler builds the handler by inlining the parts' bodies in order.

- Paths of a part that end in `next` fall through into the next part,
  whichever path reaches the `next` (the hot path, a `cold` block, or a
  `-> never` vm function the part calls).
- Other control words (`jump`, `raise`, ...) keep their meaning.
- `redo` in a later part runs that part's own word alone.
- A part can name any op, including a specialized family member
  (`GETGLOBAL_C`, `ADD_F`).
- No two supers may have the same parts.

**Groups.** `supers { .. }` declares one super per `A, B, ..;` line,
named by its parts joined with `_` unless a name is given (`NAME = A,
B;`). Attributes before `supers` apply to every super in the group, and
an entry may add its own (`#[forward]`, `#[allow(super_overlap)]`).
Comments may sit between entries. The names, opcodes and code are those
of the `super` declarations written out:

<!-- snippet: check
vm V {
    state { pc: *u32, regs: *f64, k: *f64 }
    operand reg = regs[it];
    operand kst = k[it];
    operand imm = it;
    encoding fixed(u32) { format ABC { op: 8, a: 8, b: 8, c: 8 } }
    op LOADK(dst: reg, c: kst) : ABC { dst = c; next }
    op LOADI(dst: reg, v: imm) : ABC { dst = v as f64; next }
    op MOVE(dst: reg, src: reg) : ABC { dst = src; next }
    op MUL(dst: reg, a: reg, b: reg) : ABC { dst = a * b; next }
    op ADD(dst: reg, a: reg, b: reg) : ABC { dst = a + b; next }
    op RET1(a: reg) : ABC { exit a }
{snippet}
    entry run(code: *u32, r: *f64) -> f64 { regs = r; k = r; jump code }
}
-->
```loimi
#[forward] #[allow(super_overlap)] supers {
    MUL, ADD;               // MUL_ADD
    MUL, MUL;
    LOADK, MUL, ADD;        // LOADK_MUL_ADD
    LMA = LOADI, MUL, ADD;  // named
}
```

**Encoding.** With `fixed` encodings, a super **reuses its parts'
words**: the first word's opcode is rewritten to the super's opcode, and
the following words stay as they are. A jump into the second word still
runs the original second instruction, so fusion is always safe, needs no
control-flow analysis, and can happen at build time (`Vm.optimize`) or
at run time (quickening).

#### `Vm.optimize`: the peephole

`Vm.optimize(v)` is the generated peephole pass. It compares words by
the op they stand for alone (a super's head counts as its first part),
so it can run again on code that has been optimized and quickened.

**The matching rule.** `optimize` reads the code as a straight line and
picks the fusions that save the most dispatches (a super of n parts
saves n - 1 each time it runs). On a tie it takes the longest match.
Overlapping supers cannot both run:

- With `super GETGLOBAL_SUBK = GETGLOBAL; SUBK;` and `super SUBK_CALL =
  SUBK; CALL;`, the sequence `GETGLOBAL; SUBK; CALL` becomes
  `GETGLOBAL_SUBK` followed by the plain `CALL` (a tie, so the
  leftmost). The `SUBK` word becomes a `SUBK_CALL` head, but runs only
  if something jumps to it.
- With `A_B` and `B_C_D`, `A; B; C; D` becomes `A; B_C_D`.

Declared supers where a proper suffix of one is a proper prefix of
another get the warning `super_overlap`, at the one declared later.
Declare the longer super if the whole sequence is hot, drop one of the
two, or silence it with `#[allow(super_overlap)]` on that super, its
`supers` group, or the vm.

#### Quickening inside a super

A part that executes `become X` rewrites its own word to `X`, as it
would alone (so a jump into the middle sees the specialization). It
also rewrites the super's first word:

- to the super whose parts are the same with this part replaced by `X`,
  if one is declared;
- otherwise to the sequence's first op, which splits the super up.

For example, with

<!-- snippet: check
vm V {
    state { pc: *u32, regs: *f64 }
    operand reg = regs[it];
    operand imm = it;
    encoding fixed(u32) { format ABC { op: 8, a: 8, b: 8, c: 8 } }
    op LOADI(dst: reg, v: imm) : ABC { dst = v as f64; next }
    family ADD(dst: reg, a: reg, b: reg) : ABC {
        generic { dst = a + b; become ADD_F; next }
        ADD_F { dst = a + b; if a > 1e9 { become ADD; } next }
    }
    op RET(a: reg) : ABC { exit a }
{snippet}
    entry run(code: *u32, r: *f64) -> f64 { regs = r; jump code }
}
-->
```loimi
super LOADI_ADD = LOADI; ADD;
super LOADI_ADD_F = LOADI; ADD_F;
```

the first time `LOADI_ADD`'s `ADD` part sees two floats and runs
`become ADD_F`, the code changes to `LOADI_ADD_F, ADD_F`. When `ADD_F`
deoptimizes with `become ADD`, it changes back. Without `LOADI_ADD_F`,
it would become `LOADI, ADD_F`.

In tail-call mode, a part's `next` reached from an outlined cold block
or a shared vm function copy continues in a copy of the super's handler
from the next part on, so later parts still run as parts of the super
in every dispatch mode and with the JIT. Only `redo` in a later part,
and a jump to a part's word, run the word alone.

**Supers of quickened members.** `optimize` usually runs on freshly
emitted code, where every family is at its `generic` member. So a super
whose parts are specialized members (`GETGLOBAL_C; SUBK`) forms only
when `optimize` runs again after quickening, or by quickening inside a
super. To keep a super across quickening, declare both: the pair with
the generic member, which `optimize` forms, and the pair with the
quickened member, which the `become` turns it into.

#### Fusing again after `become`: `refuse`

A super of two quickened members (`MUL_F; ADD_F`) cannot form either
way: the first site quickens next to a generic op, and the second
cannot see the word before it. `refuse CODE;` in the vm fixes that.
`CODE` is an expression of the handler's state, of type `Verified<Vm>`:
the code the running instruction is in.

<!-- snippet: check
object Proto { code: verified [u32] by V }
#[require_verified]
vm V {
    state { pc: *u32, regs: *f64, p: Gc<Proto> }
    verify_context owner proto: Gc<Proto>;
    operand reg = regs[it] verify it < 8;
    encoding fixed(u32) { format ABC { op: 8, a: 8, b: 8, c: 8 } }
    family MUL(dst: reg, a: reg, b: reg) : ABC {
        generic { dst = a * b; become MUL_F; next }
        MUL_F { dst = a * b; next }
    }
    family ADD(dst: reg, a: reg, b: reg) : ABC {
        generic { dst = a + b; become ADD_F; next }
        ADD_F { dst = a + b; next }
    }
    op RET(a: reg) : ABC { exit a }
{snippet}
    entry run(q: Gc<Proto>, r: *f64) -> f64 { p = q; regs = r; jump q.code }
}
-->
```loimi
super MUL_F_ADD_F = MUL_F; ADD_F;   // forms once both sites have quickened
refuse p.code;                      // the code a `become` rewrites
```

- After every `become X` where `X` is a part of some super, the handler
  calls the generated `Vm.refuse(CODE)`, which runs `Vm.optimize` over
  the whole code again. A `become` of an op that no super names stays a
  single store.
- In a vm whose accumulators are all value accumulators
  ([section 7.4](#value-accumulators)), `Vm.refuse` runs
  `Vm.accumulate(code, 0)` first, and a `become` of an op with
  accumulator variants calls it too.
- `become X; redo` runs the word as it is after the re-fusion: the
  super that `X` now heads, if one formed, else `X` alone.
- With the JIT, `Vm.optimize` invalidates compiled code over the words
  it rewrites.
- **Cost.** The pass is over the whole code, once per `become`. A site
  that flips between two members that supers name repeats the pass each
  time.

#### Forwarding

A super's parts exchange values through the register file: `MUL; ADD`
stores the product to `R[dst]` and loads `R[a]` back. Both indices are
operands, so LLVM cannot tell they are the same register, and the load
waits for the store (about 5 cycles on x86).

In `#[forward] super A_B = A; B;`, an operand of a later part that may
name the register the previous part writes is a *link*:

- the operand has the same kind as the one written, and is not written
  by the part itself;
- at most two links per part and three per super;
- two place kinds whose bodies and `verify` clauses differ only in the
  types they cast to (`operand ri = *((regs + it) as *i32)` and
  `operand rd = *((regs + it) as *f64)`) count as the same kind, if the
  two views have the same size.

The vm gets one more opcode per non-empty set of links, a *forwarding
variant* `A_B__fN` (N a bit mask over the links), whose handler uses the
previous part's value for the linked operands, so LLVM keeps it in a
machine register.

- `optimize` picks the variant whose linked fields are equal (the plain
  super otherwise).
- The verifier rejects a variant whose fields differ (`Forward`).
- A `become` in a part keeps the variant when the new super has it.
- There is no run-time test. The cost is opcodes (up to 7 per super)
  and code size.

#### Supers from a profile

`supers from "file";` adds the supers that a `--vm-stats` report
suggests for the vm:

<!-- snippet: skip names a profile file (tests/run/vm_supers_from.lm builds one) -->
```loimi
#[forward] supers from "vm.vmprofile";
```

- It reads the first list of each `== vm stats: NAME ==` section of the
  file, the list that can be added whole; the commented-out alternatives
  are not taken. A file with no such section is read as `super` lines.
- The path is relative to the file declaring the vm.
- A suggestion whose part is not an op of the vm (a stale report) is
  skipped with a warning.
- A suggestion with the same parts or name as a declared super is
  skipped silently, so reports of several runs can be concatenated, and
  hand-declared supers kept beside them.
- With `#[forward]` on the item, the supers are `#[forward]`. With
  `#[allow(super_overlap)]`, overlaps among them and with the declared
  ones are not reported.

#### Dispatch counts: `--vm-stats`

`loimi build --vm-stats` compiles counters into the handlers and prints
a report on stderr at exit. No source change is needed. The program can
read the counts too (`std.prof.op_counts`), and `prof.OpCounts.reset`
clears them. The counters cost nothing without `--vm-stats`.

The report has, in order:

1. **Op counts**, and opcode pair frequencies and sequences of 3 and 4
   adjacent ops.
2. **Suggested `super` declarations**: pairs that run at least 1% of the
   time, and longer sequences that do and are *dominant* (at least half
   the runs of their shorter prefix and suffix continue into them).
   They are ranked by dispatches saved. One that overlaps a better one
   or an existing super is moved to a second, commented-out list that
   names what it overlaps. So the first list can be pasted whole
   without warnings. Supers over [accumulator variants](#74-accumulators)
   are suggested too, naming the super they split.
3. **Cold blocks entered**: one line per block and op it is generated in
   (a block in a plain vm function is counted under `fn name`), with
   the block's source location:

   ```text
   cold blocks entered:
          1595161  LOADMETHOD_CALLMETHOD (cold block at examples/python/vm.lm:643:9)
           987261  CALL (cold block at examples/python/vm.lm:681:9)
   ```

4. **Quickenings**: every `become` that rewrites an instruction's own
   word, counted per site (the word) and per transition. A site's first
   `become` is its quickening; each later one is a re-quickening or a
   deopt (a site flipping between members). The transitions with the
   most later ones come first:

   ```text
   quickening: 153735 becomes at 291 sites; 153444 later ones (re-quickening or deopt) at 67 sites,
   0.0554% of 276771674 dispatches; the most at one site 142799
            first           later  transition
               23           76294  SETPROP -> SETPROP_ADD
                0           76294  SETPROP_ADD -> SETPROP
   ```

   A site is the word's address, so code that is freed and reallocated
   at the same address counts as the same site.

In a library (`--lib`, `--shared`) the report prints when the host
process exits. Several such libraries can link into one host.

#### Sampling: `--vm-prof`

`--vm-stats` counts dispatches, not time, and one op may cost ten times
another. `loimi build --vm-prof` links a sampling profiler into the
program instead. It needs no source change, and works in any dispatch
mode, for executables and libraries.

A timer sends `SIGPROF` 1000 times a second. Each sample records the
interrupted instruction, the vm's `pc` and the return addresses on the
stack. At exit the samples are named in the process (no external tools)
and printed to stderr as three tables sorted by count. Mini on
`binary_trees`:

```text
== vm prof: 961 samples, 1000 Hz of wall-clock time, 0.961 s ==

self (where the samples landed):
   samples       %    cum%  location
       163   17.0%   17.0%  CALL
       106   11.0%   28.0%  GETINDEXI
        98   10.2%   38.2%  value.new_table_sized
        84    8.7%   46.9%  SETLIST
       ...
        30    3.1%   82.2%  std.gc.mark_ptr
       132   13.7%          (21 more)

by vm location, with what it calls (the innermost op or cold block on the stack):
   samples       %    cum%  location
       202   21.0%   21.0%  NEWTABLE
       163   17.0%   38.0%  CALL
       ...
        31    3.2%   90.9%  GETINDEXI (cold block at examples/mini/vm.lm:748:13)

by guest pc (the vm's pc when the op started):
   samples       %    cum%  pc              location
       111   11.6%   11.6%  0x7fce3443144c  NEWTABLE
        79    8.2%   19.8%  0x7fce34431440  NEWTABLE
       ...
```

| Table | What it counts |
|---|---|
| **self** | where the samples landed. Op handlers are named by op, cold blocks as `--vm-stats` names them, other functions by symbol. Shared-library code is named with the program function that called it (`libc.so.6:memmove <- value.table_resize`), and JIT code by stencil (`ADD (jit)`, or `ADD* (jit)` for a variant). |
| **by vm location** | the innermost op or cold block on the stack, so the collection that `NEWTABLE` triggers counts for `NEWTABLE`. Samples with no vm frame (compiling, startup) are `(outside vm code)`. |
| **by guest pc** | the vm's `pc` when the op started (in JIT code, the compiled instruction's word), with its location: which instructions of the guest program are hot. Mapping pcs to guest source lines is up to the interpreter. |

In goto and switch modes, where all handlers are one function, each
handler stores its op's number on entry, and the op that runs owns the
sample.

| Environment variable | Effect |
|---|---|
| `LOIMI_PROF_OUT=file` | write the report to `file` instead of stderr |
| `LOIMI_PROF_HZ=n` | the sampling rate (default 1000). Faster costs more: 10 kHz cost Mini 3–9%. |
| `LOIMI_PROF_TOP=n` | rows per table (default 30; 0 prints all) |
| `LOIMI_PROF_CLOCK=cpu` | sample CPU time instead of wall-clock time. A CPU-time timer fires at most once per kernel tick, often 250 Hz. With wall-clock time, time blocked in a system call shows as the libc function that made it. |
| `LOIMI_PROF_DEPTH=n` | return addresses kept per sample (default 24; 0 skips the stack walk, and the second table then sees only the interrupted function) |
| `LOIMI_PROF=0` | no sampling, to measure what the instrumentation alone costs |

**Limits.**

- **Cost:** one store per op (two in goto and switch modes), unwind
  tables, and about 100 bytes per sample. At 1000 Hz, Mini and Python in
  tail-call mode run 3% slower; goto-mode Mini 8% slower.
- One process-wide timer: with several threads, a sample goes to the
  thread that is running.
- The report runs at exit (a C destructor): a program that ends in
  `_exit` or crashes prints none.
- x86-64 and AArch64 Linux only.
- A `--vm-prof` executable or shared library is linked with `-z now`
  (the stack walk cannot unwind through glibc's AArch64 lazy-binding
  trampoline). A host linking a `--vm-prof` static library on AArch64
  should link with `-z now` too.

### 7.4 Accumulators

A register vm passes every intermediate value through a slot: in
`x*y + z` the multiply stores its product and the add loads it back, a
store and a load of the same address on the critical path. An
accumulator keeps one value in a machine register across ops, and the
compiler derives variants of each op that read or write it (as wasm3
does by hand):

<!-- snippet: check
vm V {
    state { pc: *u64, regs: *u64 }
    operand ri = *((regs + it) as *i32);
    operand rl = *((regs + it) as *i64);
    operand rd = *((regs + it) as *f64);
    operand imm = it;
    encoding fixed(u64) { format RRR { op: 16, a: 16, b: 16, c: 16 } }
{snippet}
    op RET(a: rd) : RRR { exit a }
    entry run(code: *u64, r: *u64) -> f64 { regs = r; jump code }
}
-->
```loimi
accumulator iacc: u64 for ri, rl;     // integer views of a slot
accumulator facc: f64 for rd;

#[accumulate] op ADD(d: ri, a: ri, b: ri) : RRR { d = a +% b; next }
#[accumulate] op CVT(d: rd, a: ri) : RRR { d = a as f64; next }
#[accumulate] op FMUL(d: rd, a: rd, b: rd) : RRR { d = a * b; next }
```

An `accumulator` is a state register for values of the listed operand
kinds. Every entry zeroes it, and in tail-call mode it is a handler
argument, so it stays in a machine register (a float one in `xmm0`).
Each kind must be:

- a place whose type the declaration spells (`*(E as *T)`, or `s[it]`
  for a state variable `s: *T`); and
- a number of the accumulator's class: integers for an integer
  accumulator, floats for a float one (converted with `as`, so an `i32`
  view of a `u64` accumulator reads its low 32 bits), or a `value` type
  (a [value accumulator](#value-accumulators)).

#### Variants

`#[accumulate]` on an op derives its **variants**.

- The *result* is the operand of an accumulator's kind that the body
  only assigns (`d = ..`). Of several such operands it is the first;
  the others stay slot writes.
- The *sources* are the operands of an accumulator's kind it never
  assigns.
- There is one variant per choice of result (slot or accumulator) and
  of at most one source taken from an accumulator, except the op
  itself.

The variant names spell the result and the sources in operand order,
`A` for the accumulator and `R` for a slot. `ADD` above gets:

| Variant | Does |
|---|---|
| `ADD__ARR` | `iacc = a +% b` |
| `ADD__AAR` | `iacc = iacc +% b` |
| `ADD__ARA` | `iacc = a +% iacc` |
| `ADD__RAR` | `d = iacc +% b` |
| `ADD__RRA` | `d = a +% iacc` |

- A variant has the op's format and operand positions. An operand in an
  accumulator keeps its field, which the handler ignores and the
  verifier does not check (the disassembler shows it as `d__acc`).
- Variants are ordinary ops after the declared ones: they get handlers,
  opcodes (`Vm.OP_ADD__ARR`) and `--vm-stats` counts, and can be parts
  of supers.
- An op whose result is also read (`d = d + a`) gets source variants
  only. An op without operands of an accumulator's kind gets none, so an
  `inline for` may mark a whole table.
- The vm item `accumulate ADD, CVT, FMUL;` marks the ops it names, by
  [op selectors](#op-selectors). It may name family members (`ADD_F`),
  whose variants are members of the family.
- Every variant is an opcode and a handler, so marking many ops grows
  build time and memory, most in goto mode, where all handlers are one
  function.

#### Selecting variants: `Vm.accumulate`

`Vm.accumulate(code, first_temp)` selects variants, on unverified words
before `Vm.verify` (for value accumulators: on verified code). It first
turns every variant back into its op and splits supers, so it may run
again on code it already ran on.

Where an op writes a slot at or above `first_temp` that the next op
reads as a source of the same accumulator (and names in no other
field), and that next op is not a jump target, the first op gets a
variant that writes the accumulator and the second one that reads it.
Chains continue: `ADD`, `ADDK`, `CVT` can become `ADD__ARR`,
`ADDK__AA`, `CVT__RA`.

**The caller's promise:** a value written to a slot at or above
`first_temp` is read at most once, by the first op that reads it after
the write. A translator's operand stack temporaries, popped once, meet
it. A broken promise gives wrong values, not memory unsafety:
accumulators hold numbers, and the verifier still checks every operand
that names a slot.

`Vm.optimize` can run afterwards; supers whose parts are variants form
like any other.

#### Value accumulators

An accumulator whose kinds hold a `value` type holds the value's word:
as an `f64` (`f64.from_bits`), so that it takes an SSE register and no
general one, or as a `u64`. The type must be a one-word value (`nanbox`
or `lowtag`) whose word 0 holds no reference. In a nanbox, a float
result is already the word, so a float chain costs no conversion.

<!-- snippet: check
value V repr nanbox { Num(f64) unboxed, Int(i32), Nil }
vm M {
    state { pc: *u32, regs: *V }
    operand reg = regs[it];
    encoding fixed(u32) { format ABC { op: 8, a: 8, b: 8, c: 8 } }
{snippet}
    op HALT() : ABC { exit 0 }
    entry run(code: *u32, r: *V) -> i64 { regs = r; jump code }
}
-->
```loimi
accumulator vacc: f64 for reg;          // reg holds a V: its word, in xmm0
accumulate ADD_F, MUL;                  // a family member and a plain op

family ADD(d: reg, a: reg, b: reg) : ABC {
    generic {
        if let (Int(x), Int(y)) = (a, b) { d = Int(x + y); next }
        if let (Num(x), Num(y)) = (a, b) { d = Num(x + y); become ADD_F; next }
        d = Nil;
        next
    }
    ADD_F {
        if let (Num(x), Num(y)) = (a, b) { d = Num(x + y); next }
        d = Nil;
        next
    }
}
op MUL(d: reg, a: reg, b: reg) : ABC {
    if let (Num(x), Num(y)) = (a, b) { d = Num(x * y); next }
    d = Nil;
    next
}
```

A value accumulator is *written through*:

- Every form of an op whose result is of its kind (a *writer*: `ADD_F`,
  `MUL` and their variants) stores the result slot and also leaves the
  word in the accumulator, before each `next`. So the slot is always
  current, and the collector, `raise`, the debugger and the JIT need to
  know nothing.
- There are no variants that put the result only in the accumulator,
  just readers (`ADD_F__RAR`, `ADD_F__RRA`, `MUL__RAR`, `MUL__RRA`).
- Selection needs no promise: pass `first_temp` 0, and locals chain too.

Quickening needs care, and the compiler handles it:

- **A word quickened in a handler.** Run `Vm.accumulate` again from the
  handler that quickened (or use `refuse`, [section 7.3](#fusing-again-after-become-refuse)).
  The next word may now be a reader of a result the generic member did
  not put in the accumulator, so in an op that is not a writer, every
  `next` that may follow a `become` of a writer also leaves the result
  in the accumulator (a load, on that path only).
- **`redo` after `become`.** The word re-run may now be a reader. Its
  writer ran just before and left the accumulator current, and nothing
  may store to it before a `redo`.
- **`become` keeps what selection made.** A writer may only `become` a
  writer of the same accumulator (an error otherwise). In a variant,
  `become X` goes to X's variant with the same letters, or to X itself
  if it has none.
- **Entries and computed jumps.** A reader is never a jump target, and
  every entry zeroes the accumulator. A `jump` to a computed target (a
  return address, an exception handler; not a label operand, `then.x` or
  `pc`) zeroes it too, so a reader it lands on reads word 0, which is no
  reference.
- **Verification and `deopt`.** `Vm.verify` rejects readers (as an
  invalid opcode): they are selected on verified code, where selection
  sees every jump target. `Vm.deopt` turns them back into their ops.

A writer cannot leave through a vm function with `next` or `jump` (an
error), such as a guest method call whose return lands on the next word.

**Cost.** The register is an argument of every handler's tail call, is
saved around the C calls of handlers that make them, and an integer
writer moves its result into it. A variant also breaks up the supers
over its op; `--vm-stats` suggests supers over the variants, naming the
super each one splits.

#### Op selectors

The ops of an `accumulate` list, and of a variant's `wraps [..]`
([section 9](#9-instrumentation-variants)), are given by selectors. The
list names their union.

| Selector | Names |
|---|---|
| `ADD` | the op |
| `I32_ADD..I32_ROTL` | the ops declared from the first to the last, in declaration order (the order of opcodes, and of `inline for` rows) |
| `BR_I32_*`, `*_TK`, `I64_*K` | the ops whose names match; `*` stands for any run of characters (no spaces) |
| `with label` | the ops with an operand of that kind |

- Ranges, globs and `with` leave out family members; name those
  directly.
- A selector that names no op is an error.
- `LOIMI_DUMP_VM_EXPANSION=1` prints each resolved list.

### 7.5 Relations and data on ops: `rel` and `data`

A translator that quickens or fuses needs to know how ops relate: the
constant-operand form of an op, the branch an op fuses with, the
negation of a compare. `rel` and `data` declare such tables next to the
ops:

| Declaration | Generates |
|---|---|
| `rel NAME(A) = B;` | `Vm.NAME(op) -> ?W`: `Some(OP_B)` for `OP_A`, `None` for any other op |
| `data NAME: T = D;` | `Vm.NAME(op) -> T`, with default `D` |
| `data NAME(A) = V;` | op `A`'s value for `NAME` |

Both may be written in an `inline for`, next to the ops a row defines.
`_` for either op of a `rel` declares nothing (a row without the
relation).

<!-- snippet: run "ADDK 1 true - false" "LT_B _ LT_BN"
import std.io;
vm V {
    state { pc: *u32, regs: *i64 }
    operand reg = regs[it];
    operand imm = it;
    encoding fixed(u32) { format ABC { op: 8, a: 8, b: 8, c: 8 } }
{snippet}
    op RET(a: reg) : ABC { exit a }
    entry run(code: *u32, r: *i64) -> i64 { regs = r; jump code }
}
fn show(o: ?u32) { match o { Some(x) => io.print(V.op_name(x)), None => io.print("-") } io.print(" "); }
fn main() {
    show(V.kform(V.OP_ADD)); io.print_int(V.cost(V.OP_ADD) as i64); io.print(" "); io.print_bool(V.commutative(V.OP_ADD)); io.print(" ");
    show(V.kform(V.OP_MULK)); io.print_bool(V.commutative(V.OP_SUB)); io.println("");
    show(V.branch(V.OP_LT)); io.print("_ "); show(V.negate(V.OP_LT_B)); io.println("");
}
-->
```loimi
data commutative: bool = false;
data cost: u32 = 1;
inline for (name, comm, expr, kexpr) in [
    ("ADD", true, a + b, a + (k as i64)), ("SUB", false, a - b, a - (k as i64)),
    ("MUL", true, a * b, a * (k as i64)),
] {
    op #name(d: reg, a: reg, b: reg) : ABC { d = expr; next }
    op #(name ++ "K")(d: reg, a: reg, k: imm) : ABC { d = kexpr; next }
    rel kform(#name) = #(name ++ "K");     // V.kform(V.OP_ADD) == Some(V.OP_ADDK)
    data commutative(#name) = comm;
}
data cost(MUL) = 3;
inline for (cmp, test) in [("LT", a < b), ("LE", a <= b)] {
    op #cmp(d: reg, a: reg, b: reg) : ABC { d = if test { 1 } else { 0 }; next }
    op #(cmp ++ "_B")(a: reg, b: reg) : ABC { if test { jump pc + 2 } next }
    op #(cmp ++ "_BN")(a: reg, b: reg) : ABC { if !(test) { jump pc + 2 } next }
    rel branch(#cmp) = #(cmp ++ "_B");
    rel negate(#(cmp ++ "_B")) = #(cmp ++ "_BN");
    rel negate(#(cmp ++ "_BN")) = #(cmp ++ "_B");
}
```

- The ops are named after `inline for` expansion, so they may be
  declared anywhere in the vm.
- A name that is not an op, a second `NAME(A)`, and a `data` value of
  another type (reported at the value) are errors.
- A relation or data key may not have the name of a generated vm item
  (`op_name`, `encode`, ..).
- The functions are host code: no handler reads them, and declaring
  them changes no handler, opcode or word.

---

## 8. GC integration

The collector is in the standard library (`std.gc`, described in
[stdlib/gc.md](stdlib/gc.md)), but the language knows enough to make it
precise and cheap. The heap is non-moving, so pointers into objects and
stacks stay valid across collections, and nothing is reloaded after a
`gc` call.

### Objects

`object` declarations are GC-managed structs. The compiler generates the
header (type id, mark bits) and the trace function from the field types.

**Trailing arrays.** An object may end in a variable-length array
(`slots: [Value]`). `new(h, Obj { .. }, n)` allocates it with `n`
elements, zero-filled. That is allowed only where zero is a valid
element:

- numbers, `bool`, options, slices, and raw pointers (outside `#![safe]`
  modules), and aggregates of those;
- an enum whose discriminant-0 variant is valid;
- a `value` type whose word 0 encodes a variant without a reference (a
  nanbox's unboxed `0.0`, a lowtag integer with tag 0).

Generic objects are checked per instance. Otherwise:

- pass a fill value: `new(h, Obj { .. }, n, v)`;
- use `?Gc<T>` elements;
- or write `unsafe { new(h, Obj { .. }, n) }` where every element is set
  before anything reads it.

A literal length `0` needs none of these.

<!-- snippet: fail "zero is a null `Gc<Node>`"
import std.gc;
object Node { v: i64 }
object Arr { items: [Gc<Node>] }
fn f(h: *gc.Heap, one: Gc<Node>) {
{snippet}
}
fn main() {}
-->
```loimi
let a = new(h, Arr {}, 4);              // error: four null Gc<Node>s
let b = new(h, Arr {}, 4, one);         // four references to `one`
```

**Unzeroed trailing arrays.** `new_uninit(h, Obj { .. }, n)` is
`new(h, Obj { .. }, n)` without zeroing the trailing array (the fixed
fields are set from the literal as by `new`). For an array that is
written in full right away, such as a string's bytes, it saves the
`memset`.

- The element type must not hold references the collector traces (an
  error even in `unsafe`).
- Outside `unsafe { }`, every bit pattern must be a valid element:
  numbers, SIMD vectors, and arrays, tuples and structs of those (not
  `bool`, enums, options or raw pointers).
- Verify mode and `--gc-check-roots` builds fill the array with 0x01
  bytes.

<!-- snippet: run "105"
import std.io;
import std.gc;
import std.mem;
object Str { hash: u64, data: [u8] }
fn main() uses gc {
    var heap = gc.Heap.new();
    let h = &heap;
    let s = "hi";
{snippet}
    io.print_int(o.data[1] as i64); io.println("");
    heap.free();
}
-->
```loimi
let o = new_uninit(h, Str { hash: 0 }, s.len);  // bytes not zeroed
mem.memcpy(o.data.ptr, s.ptr, s.len);           // every byte set
```

**Generic objects.** `object Obj<V> { shape: Gc<Shape>, inline: [V] }`
is a family of object types. Each instance (`Obj<Value>`, `Obj<i64>`)
is an object type of its own, with its own type id and trace function,
so `Obj<i64>`'s slots are not traced and `Obj<Value>`'s are.

- The trailing array may have a type parameter as its element type.
- `Gc<T>` may name a type parameter (`fn keep<T>(x: Gc<T>) -> Gc<T>`,
  `struct Pair<T> { a: Gc<T>, b: Gc<T> }`).
- There is no `Object` bound: binding `T` to something that is not an
  `object` is an error at the `Gc<T>`, with a note pointing to the use
  that created the instance ("Gc<T> needs an `object` type, found
  `i64`").

**Custom tracing.** `#[trace(f)]` on an `object` names a function of
the same module, `fn f(h: *gc.Heap, obj: Gc<Obj>)` without effects. The
trace function calls it after tracing the declared fields, instead of
tracing the trailing array. It marks (`gc_mark`) the live part of the
array, or references in memory the object owns outside the heap. Not on
generic objects. For the related free hooks and dirty pinning, see
[stdlib/gc.md](stdlib/gc.md#trace-hooks-free-hooks-and-dirty-pinning).

### References and barriers

**`Gc<T>`** is a traced reference. A `Value` traces whichever variants
hold a `Gc`.

- It converts to a raw pointer or a `u64` address with `as`.
- The way back (`p as Gc<T>` from a pointer, a `u64` or another `Gc`)
  is allowed only inside `unsafe { }`, since nothing checks that `p`
  points to a live `T`.

**Write barriers.** Every store of a `Gc`/`Value` into an object field,
including `o.items.ptr[i] = v`, calls `std.gc.barrier(o)`, an
object-logging barrier that tests one header byte.

- A heap is made generational at run time with
  `heap.set_generational(true)`; the source does not change.
- Stores through other raw pointers into objects need
  `gc.write_barrier(h, obj)`. `heap.set_verify(true)` finds missing
  ones.

**Slices of trailing arrays.** `o.items` is a `[]T` slice, which does
not know its object. So the compiler restricts where one with traced
elements can go, in every module:

- it may be indexed, sub-sliced, iterated, bound by `let` and read, or
  passed straight to a function;
- a call that may store through it gets the barrier on `o` after the
  call, and must not collect (a collection between the store and the
  barrier would miss the young object);
- keeping such a slice in a `var`, a field or a return value, storing
  through a `let` that holds one, and passing one to a function that may
  keep it, or that may collect and store, are errors.

Each function's traced slice and pointer parameters are summarized for
this (does it store through it, keep it, or pass it on to one that
does?). In `#![safe]` modules the same rules hold for `&o.f` of a traced
field.

<!-- snippet: fail "is stored through (held in a local)"
import std.gc;
object Node { v: i64 }
object Arr { items: [?Gc<Node>] }
fn put(s: []?Gc<Node>, n: Gc<Node>) { s[0] = Some(n); }
fn f(a: Gc<Arr>, n: Gc<Node>) {
{snippet}
}
fn main() {}
-->
```loimi
put(a.items, n);          // the barrier on `a` follows the call
let s = a.items;
let k = s.len;            // reading through `s` is fine
s[0] = Some(n);           // error: no barrier would run
```

**Weak references** are library types: `heap.new_weak(x)` gives a
`Gc<gc.Weak<T>>` that reads `None` once `x` died, and
`gc.Ephemerons<K, V>` maps objects to values that only the key keeps
alive. There is no `weak` field qualifier.

### Roots

The collector finds live objects from three kinds of roots:

| Root | Who registers it |
|---|---|
| traced locals live across an effectful call | the compiler, in shadow-stack frames ([section 5](#5-effects-and-sync-points)) |
| traced state variables | the compiler ([below](#traced-state-variables)) |
| everything else: VM stacks, an interpreter's globals, interned names, tables | root scanners you register |

Loimi uses these compiler-managed roots instead of LLVM's
statepoint/stackmap machinery. Statepoints constrain the optimizer
everywhere, while the effect system confines rooting to the slow paths
that can actually collect.

#### Root scanners and `#[trace] struct`

`heap.add_scanner(ctx, scan)` registers `scan: fn(*gc.Heap, *Ctx)`,
called with `ctx` at every collection. The scanner has no effects (it
runs inside a collection). `add_scanner<T>` takes any `*T` and a
function of that type (`add_sweeper` likewise), so neither side casts
through `*u8`.

`#[trace]` on a plain struct generates `S.trace(s: *S, h: *gc.Heap)`,
which marks every GC reference in the fields, in declaration order:

- `gc_mark(h, s.f)` for a field that holds references inline (`Gc`,
  options, `value` types, structs, tuples, enums);
- a loop over a fixed array of those;
- a loop over the elements of a `mem.Vec`, and over the keys and values
  of a `map.HashMap`, that hold them;
- the fields of a nested struct, in turn.

Raw pointers are not followed (the scanner marks the stack ranges they
delimit), and `#[untraced]` on a field leaves it to the scanner. A field
that holds GC references `#[trace]` cannot reach (any other container,
an array of `Vec`s, a raw pointer inside a nested struct) is an error
naming the field, so a context field cannot be left unscanned silently.
The method is compiled only if called. `LOIMI_DUMP_VM_EXPANSION=1` lists
what it marks (`Ctx.trace marks globals, types[0..24], strs (HashMap
values), ..`).

<!-- snippet: run "3" -->
```loimi
import std.io;
import std.gc;
import std.mem;
object Box { v: i64 }
#[trace]
struct Ctx {
    heap: gc.Heap,
    top: ?Gc<Box>,
    consts: mem.Vec<Gc<Box>>,
    stack: *Gc<Box>,
    sp: u64,
}
fn scan(h: *gc.Heap, c: *Ctx) {
    c.trace(h);                                     // top, consts
    for i in 0..c.sp { gc_mark(h, c.stack[i]); }    // the live stack
}
fn main() uses gc {
    var c = Ctx { heap: gc.Heap.new(), consts: mem.Vec.new(), ..zero };
    c.heap.add_scanner(&c, scan);
    c.top = Some(new(&c.heap, Box { v: 1 }));
    c.consts.push(new(&c.heap, Box { v: 2 }));
    gc.collect(&c.heap);
    io.print_int(c.top.unwrap().v + c.consts.at(0).v);
    io.println("");
}
```

A function value converts, inside `unsafe { }`, to a function type that
differs only in the types of raw pointers (`scan as fn(*gc.Heap, *u8)`).

#### Traced state variables

A state variable of a traced type (a stack VM's cached top of stack,
`state { .., tos: Value }`, or a current closure, `cl: Gc<Closure>`) is
rooted by the compiler. Neither `sync` nor a root scanner needs to know
about it.

- **Where.** At every call that may collect (`uses gc` or `reenter`, or
  a `gc` function pointer or closure type) made from vm code: op
  handlers and their `cold` blocks, `on raise`, variant wrappers,
  `-> never` vm functions, entries (also before the first dispatch), and
  plain vm functions. These calls already run `sync` and link a
  shadow-stack frame; the traced state joins that frame. A call that
  cannot collect, and every dispatch, pays nothing.
- **How.** Each variable is copied, at the call, into a stack slot that
  the frame lists. The variable itself never has its address taken, so
  it stays in a register everywhere else, in all three dispatch modes.
  A call of a vm function of the same vm roots no state: the function
  roots it at its own calls that may collect.
- **Entries** start with their traced state zeroed, so a variable no one
  has assigned yet is a valid (empty) root.
- **Nested runs.** A `reenter` call keeps the caller's frame, and with
  it the copies, linked while the nested run executes, so the outer
  state survives collections in it.
- **Unwinding.** `rt.catch_panic`, `rt.protect` and `#[catch_panic]`
  restore the saved shadow-stack top, which drops the frames (and
  copies) of the calls they unwound.

#### Checking roots

To find rooting mistakes (a reference kept in a raw pointer across a
call that collects, a scanner that misses one), build with
`--gc-check-roots`. The collector then checks every reference it is
handed and fills freed objects with a pattern
([stdlib/gc.md](stdlib/gc.md#checking-roots---gc-check-roots)).

---

## 9. Instrumentation variants

A `variant` adds code that runs before ops, without slowing the normal
build:

<!-- snippet: check
struct Stats { ops: [u64; 16] }
struct Tracer { n: u64 }
fn Tracer.on_op(t: *Tracer, op: u32, pc: *u32, regs: *i64) uses sync { t.n += 1; }
fn Tracer.on_line(t: *Tracer, pc: *u32) uses sync { t.n += 1; }
struct Breakpoints { at: ?*u32 }
fn Breakpoints.has(b: *Breakpoints, pc: *u32) -> bool { false }
struct Debugger { n: u64 }
fn Debugger.stop(d: *Debugger) -> Result<(), str> uses sync { Ok(()) }
struct Ctx { stats: Stats, tracer: Tracer, breakpoints: Breakpoints, debugger: Debugger, pc: *u32 }
vm V {
    context ctx: *Ctx;
    state { pc: *u32, regs: *i64 }
    sync { ctx.pc = pc; }
    operand reg = regs[it];
    operand label = pc + it;
    encoding fixed(u32) { format ABC { op: 8, a: 8, b: 8, c: 8 } format AsBx { op: 8, a: 8, sbx: s16 } }
    op JMP(target: label) : AsBx { jump target }
    op JLT(a: reg, target: label) : AsBx { if a < 0 { jump target } next }
    op JLE(a: reg, target: label) : AsBx { if a <= 0 { jump target } next }
    op JEQ(a: reg, target: label) : AsBx { if a == 0 { jump target } next }
    op CALL(a: reg) : ABC { next }
    op RETURN(a: reg) : ABC { exit Ok(a) }
    op DEBUG_ON() : ABC { use_variant debug; next }
{snippet}
    entry run(c: *Ctx, code: *u32, r: *i64) -> Result<i64, str> { ctx = c; regs = r; jump code }
    on raise(e: str) { exit Err(e) }
}
-->
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
variant line_hook wraps [JMP, JLT, JLE, JEQ, CALL, RETURN] {
    before { ctx.tracer.on_line(pc) }
}
```

- Each variant generates a separate dispatch table of wrapper handlers,
  each wrapping the original handler.
- Switching is a single register update: `use_variant debug` in vm
  code, `use_variant default` to switch back. The new table takes effect
  at the next dispatch.
- Host code switches by entering through an entry that runs
  `use_variant`.
- **Normal execution pays nothing**: no flag check in any handler, and
  no second copy of the interpreter to maintain.

Debuggers, profilers, coverage and `sys.settrace`-style hooks are built
this way.

**Which ops a list wraps.** `wraps all` wraps every op. `wraps [..]`
takes [op selectors](#op-selectors) (`JMP`, `A..B`, `BR_*`, `with
label`).

- A variant that wraps an op also wraps every superinstruction that has
  that op as a part, and its `#[forward]` variants. A super runs in
  place of its parts, so a hook on `JMP` must see `LT_JMP` too.
- A super may also be named in the list.

**`#[unfused]`.** Under a variant marked `#[unfused]`, superinstructions
run as their parts, so the wrapper sees every instruction (which a line
hook needs: a line can start at a later part of a super).

- A super's entry in the variant's table is its first part's wrapper,
  which runs that part alone; its `next` dispatches the next part's word
  through the same table.
- The default table still runs supers fused.
- A `become` in the first part rewrites the head word as it would for
  that op alone, which splits the super (`Vm.optimize` fuses it again).

Without `#[unfused]`, a super is one instruction to a variant.

**Building a debugger in only on request.** `#[cfg(NAME)]` keeps or
drops any item of a `vm` block ([section 1.4](#14-compile-time-evaluation)),
so an interpreter can include its debugger only with `--cfg debug`.

**Counting without a variant.** `loimi build --vm-stats` compiles op,
op-pair and cold-block counters into the handlers
([section 7.3](#dispatch-counts---vm-stats)).

### 9.1 Breakpoints: `on break`

A breakpoint replaces an instruction's opcode with a generated
breakpoint op, as CPython 3.12 does: code without breakpoints runs at
full speed, and nothing checks for them. `on break { .. }` in a vm
declares what a breakpoint does. It is an op body without operands: it
sees the state and `pc`, and must leave with a control word.

<!-- snippet: excerpt tests/run/vm_break.lm -->
```loimi
on break {
    ctx.hits += 1;
    // Step over: the instruction goes back for one run under `step`.
    let Some(b) = ctx.live else { exit -1 };
    if !V.clear_break(b) { exit -2 }
    ctx.pending = Some(b);
    ctx.live = None;
    ctx.fresh = true;
    use_variant step;
    jump pc
}
variant step wraps all {
    before {
        ctx.steps += 1;
        // The first wrapper to run is the restored instruction's own.
        if ctx.fresh { ctx.fresh = false; } else {
            if let Some(b) = ctx.pending {
                ctx.live = b.rearm();
                ctx.pending = None;
            }
            use_variant default;
        }
    }
}
```

The vm gets one breakpoint op per inline cache layout: `BREAK` (no
cache), and `BREAK__C` for each cache `C`, with the size of the
instructions they replace. So walks over the code (`resume_at`, the
disassembler, `deopt`, the verifier) stay on instruction boundaries
with breakpoints set.

| Generated | What it does |
|---|---|
| `Vm.set_break(v, i) -> ?Vm.Break` | patch the instruction at word `i` of verified code `v` (`None` if no instruction starts there, or it is a breakpoint already). A superinstruction that spans word `i` is split first. |
| `Vm.clear_break(b) -> bool` | put the replaced word back (false if the word there is not that breakpoint) |
| `b.rearm() -> ?Vm.Break` | set it again at the same word, over the instruction that is there now (it may have been quickened in between) |
| `b.word()`, `b.pc()` | the replaced word and its address |
| `Vm.is_break(w)` | whether `w` is a breakpoint op |

`Vm.Break` is the proof of a set breakpoint: its fields are private to
the vm's expansion, like `Verified`'s.

**Stepping over.** To run the instruction under a breakpoint, the
handler puts it back and executes it again (`jump pc` dispatches the
word anew), under a variant whose wrapper sets the breakpoint again
before the next instruction. `std.debug` implements this protocol
(`Breakpoints.hit` and `tick`), with line stepping and a command reader.
Mini's `--cfg debug` build is a debugger on top of it
(`examples/mini/debugger.lm`), with a C API and a Debug Adapter Protocol
server for editors (`examples/mini/dap`).

A hook that evaluates guest code at a stop (a breakpoint condition) is
declared `uses reenter`, so the handler syncs before it and reloads
after.

**What keeps this sound:**

- *Verification.* Code with breakpoints set does not verify (a
  breakpoint op is `BadOpcode`); the instruction it stands for is in the
  `Vm.Break`. `set_break` takes the proof of the code, checks the
  boundary, and changes nothing else.
- *Quickening.* No `become` writes a word under a breakpoint, since it
  does not run. When it is put back and runs, `become` rewrites it as
  usual, and `rearm` patches the new op.
- *Superinstructions.* A breakpoint is not a part of any super, so
  `optimize` does not fuse over it.
- *Inline caches.* The breakpoint op keeps the instruction's cache
  words, and `Vm.deopt` leaves them alone.

**Limits.** `on break` is for interpreted code of register vms: it is an
error with `--jit-stencils`, and stack vms do not support it.

---

## 10. Calls, reentrancy, coroutines, embedding

A vm's `entry` functions are ordinary internal Loimi functions (not
C-ABI exports), and each interpreter keeps its own frame stack in vm
state.

| Call | How |
|---|---|
| guest to guest | stays in the loop: push a frame on the interpreter's own frame stack, slide the register window, and `jump`. No C stack is used, so deep guest recursion is bounded by the VM stack, not the C stack. |
| host to guest (metamethods, callbacks, `sort` comparators) | a nested run on the same VM stack, through a nested entry (below) |
| C host to guest | an `export "C"` function the interpreter writes around an entry ([section 10.1](#101-embedding-in-a-c-host-implemented)) |

### Nested entries

An entry declared `uses reenter` is a *nested entry*:

<!-- snippet: excerpt examples/mini/vm.lm -->
```loimi
entry call(c: *Ctx, at: *Frame, cl: Gc<Closure>) -> Result<V, V> uses reenter {
    ctx = c;
    let p = cl.proto;
    fp = at;
    fp.cl = cl;
    fp.nres = 1;
    k = p.consts.data.ptr;
    fp.k = k;
    regs = fp.base;
    c.base = fp;
    c.fp = fp;
    #[cfg(debug)] if debugger.stepping(c) { use_variant step; } jump p.code
}
```

Here `at` is a frame above the caller's with the arguments in place, and
`c.base = fp` makes it the sentinel: RET from it exits the run.

- **It is an entry.** In tail-call mode it makes the first
  `preserve_none` call into the handlers and returns when one of them
  says `exit` or `fail`. In goto and switch modes it calls the entry's
  own loop function (recursively when runs nest). The nested loop runs
  on the C stack of the native that called it, and leaves the outer
  loop's state where the outer handler's `sync` put it.
- **Its callers need `uses reenter`.** An ordinary function calling it
  declares the effect, and a handler calling it (or calling a native
  that may) gets `sync` before and `reload` after
  ([section 5.1](#51-reenter-and-the-cached-state)). An entry may
  declare no other effect.
- **`Vm.depth`** is a thread-local count of the nested runs active on
  the thread, one more while a nested entry runs (also when it returns
  early). Check it to bound the C stack: Mini fails with "C stack
  overflow" past 200, as Lua does. It is per thread, not per vm state,
  because natives of one state may call into another on the same C
  stack.
- **The vm does the rest itself**, since the compiler does not know the
  frame layout: where the new frame goes (above `ctx.fp`, which the
  caller's `sync` published), which frame is the sentinel, restoring the
  caller's frame registers after the call, and turning an error into a
  result. Typically this is one host function around the entry.
- **Errors.** `raise` in the nested run reaches that run's `on raise`,
  whose `exit`/`fail` returns from the nested entry. The native gets the
  `Result` and decides. If it re-raises, the outer `on raise` sees the
  error again.

### Coroutines and generators

Coroutines are written with what handlers and entries already have.

- **Switching inside the run.** A guest coroutine with its own value and
  frame stacks is switched to like a call. A handler (resume, yield, or
  RET from a coroutine's bottom frame) saves the running thread's state
  where the vm keeps it (with `next_resume` as where it continues),
  assigns the state variables from the other thread's saved frame, and
  `jump`s to that thread's saved resume point (`r.pc()`). The run goes
  on dispatching: no native call, no nested entry, no C stack per
  coroutine. Traced state variables need nothing extra
  ([section 8](#traced-state-variables)); the collector reaches the
  saved threads through the vm's own root scanner.
- **From host code**, where a native resumes a coroutine (a sort
  comparator that is a coroutine, say), the coroutine runs in a nested
  entry. The yield saves `next_resume` and leaves the run with `exit`,
  and the next resume is an entry that reinstalls the coroutine's frame
  and jumps to that resume point, which `#[require_verified]` accepts
  ([section 6](#resume-points)).

`examples/mini/COROUTINES.md` describes Mini's coroutines, built this
way.

### 10.1 Embedding in a C host (implemented)

```sh
loimi build embed.lm --lib    -o libmini.a  --header mini.h   # static archive
loimi build embed.lm --shared -o libmini.so --header mini.h   # shared object
```

| Topic | Rule |
|---|---|
| **API** | the program's `export "C"` functions. A library needs no `main` and gets no `main` wrapper; `os.arg_count()` is 0. |
| **Initialization** | none. Every Loimi global has a constant initializer, and the runtime's thread-locals start zeroed in every thread, so a host can call any export from any thread at any time. The interpreter's own state (a heap, its roots) is created by an export such as `mini_new`, and must be used by one thread at a time ([threads](#threads)). |
| **Code model** | `--lib` and `--shared` compile with `-fPIC`, so the archive links into PIE executables and shared objects. Executables keep `-fno-pic`. |
| **TLS** | runtime thread-locals are initial-exec in executables and `--lib` builds. `--shared` builds default to local-dynamic, so the object can be `dlopen`'ed whatever static TLS space is left. `--tls=initial-exec\|local-dynamic\|global-dynamic` overrides. |

#### Sanitizers

`--sanitize=address` and `--sanitize=thread` work with `--lib` and
`--shared`: the library carries the instrumentation, and the host links
the runtime by building with `-fsanitize=address` (or `=thread`).

- Loimi instruments with clang, but links executables against GCC's
  `libasan` when clang's own runtime (`libclang_rt.asan`, package
  `libclang-rt-19-dev`) is missing, which is usual on Debian. Both use
  the same instrumentation ABI. So `gcc -fsanitize=address host.c
  librx.a` works everywhere, and `clang-19 -fsanitize=address` works
  where clang's runtime is installed. `gcc -fsanitize=thread` links
  GCC's `libtsan`.
- A host that cannot be rebuilt can load a `--shared` library under
  `LD_PRELOAD=$(gcc -print-file-name=libasan.so)`.
- A library leaves ASan's options to the host. (Executables turn leak
  checking off by default; libraries do not.)
- `--emit-llvm` with `--lib` gives IR you can instrument yourself.

#### The header

`--header out.h` declares every export, in source order, with these C
types:

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
types, SIMD vectors, and `Gc<T>` (the host would hold a reference the
collector cannot see).

- A `str` parameter must fit in the first six integer argument
  registers: C passes a 16-byte struct whole, in two registers or on the
  stack.
- The same checks run on every `--lib`/`--shared` build. `--header`
  also works on an executable.

More in the header:

- **Documentation.** The `///` comment above an export, an exported
  constant, or a struct or enum the header declares becomes a C comment
  above its declaration.
- **Constants.** `export "C" const RX_SEARCH: u32 = 1;` is an ordinary
  constant in Loimi and `#define RX_SEARCH 1u` in the header. Integers
  get a `u`/`LL`/`ULL` suffix by type; `bool`s, floats, `str`s (as
  string literals) and payload-free enum variants (as `((E)n)`) work
  too.
- **`const` pointers.** `#[c_const(pat, buf)]` on an export declares
  those parameters `const T *` in C. It is a promise the Loimi side
  keeps by hand: nothing checks that the function does not write
  through them.
- **C names.** A struct or enum is named as in Loimi, unless it has
  `#[c_name("rx_regex")]`, or `--header-prefix rx_` puts a prefix on
  every name without one (`rx_Regex`). Enum constants follow:
  `rx_Mode_Add`.

#### Errors and panics

A library must not exit the host on a guest error.
`std.rt.protect(f, data)` and `std.rt.throw()` unwind out of code with
no error path (see `std.rt` in [stdlib.md](stdlib.md)).

**Panics** are: a failed bounds check, `unwrap` of `None`, a debug
overflow check, division by zero, `trap()`, `rt.panic(msg)`, and
`mem.alloc` out of memory. By default a panic prints its message on
stderr and aborts the process, host included. An export can catch them
instead:

<!-- snippet: check
struct Regex { n: u32 }
fn run(r: *Regex, s: str, flags: u32) -> i32 { 0 }
{snippet}
-->
```loimi
#[catch_panic(-1)]
export "C" fn rx_exec(r: *Regex, s: str, flags: u32) -> i32 { run(r, s, flags) }
```

- A panic anywhere below the export on the same thread (in Loimi code,
  or in a Loimi export the host calls from a callback) returns `-1` from
  it.
- `rt.panic_message()` gives the message. `rt.panic(msg)`'s message is
  copied (its first 512 bytes), so it may be built in a buffer the
  unwinding abandons.
- The value is an integer, `true`/`false`, or a variant of the result
  enum. Without one (`#[catch_panic]`) the result is zero: 0, false,
  NULL, an empty `str`, or nothing.
- `rt.catch_panic(f, data)` does the same inside Loimi.

**Cost.** A catching export installs a handler on each call: on x86-64
about 1–2 ns (LLVM's builtin setjmp plus a few thread-local accesses),
elsewhere glibc's `_setjmp`, a few ns. Code without a catch compiles as
before. Put the attribute on calls that do real work (compile, match),
not on accessors called millions of times.

**What is left after a caught panic.** The frames between the panic and
the catch are abandoned, as with `rt.throw`: their `defer`s do not run,
and memory they allocated and had not yet stored somewhere reachable
leaks.

- Restored: the shadow-stack top and the vm nesting depths
  (`Vm.depth`), so GC roots and re-entry counts are correct, also when
  the panic unwinds through nested vm runs and host callbacks.
- Not restored: a data structure that was being modified (a vm state, a
  `Vec` mid-`push`, a hash map mid-rehash) may break its own invariants.
  Treat the objects the failed call was modifying as poisoned: free
  them, or mark them failed and refuse further calls on them.
- Unaffected: thread-locals, globals the call did not touch, other
  objects and other threads.

**Not caught:** a failure inside the collector itself (out of memory in
a GC heap aborts), stack overflow, a segmentation fault from an
unchecked pointer, `--sanitize=address` reports, and, in executables,
`trap()` (an illegal instruction there; in `--lib`/`--shared` builds
`trap()` is a panic).

**Host frames.** A panic in a nested export without its own catch
unwinds through the host's frames to the catching export below, like
`longjmp`. Host code in between must not hold resources it needs to
release (C++ destructors do not run).

**Threads.** Panic handlers are per thread. A panic on a thread with no
catch aborts, even if another thread has one.

#### Threads

Independent interpreter states may run on different threads at the
same time, with no lock: a server can give each worker thread its own
state. Where the runtime's state lives:

| State | Where it lives |
|---|---|
| shadow-stack top (GC roots of compiled code) | thread-local |
| `Vm.depth`, vm scratch slots (`cold` captures, `-> never` arguments, aggregate results, `raise` values) | thread-local |
| `rt.protect`/`catch_panic` handler chain, caught panic message and its 512-byte copy, `#[track_caller]` location | thread-local |
| GC heaps: chunks, segments, mark bitmaps, remembered set, scanners, weak tables, interners | per heap (`std.gc` has no globals) |
| inline caches, rewritten opcodes, constant pools | per state (in its code) |
| dispatch tables, string constants, `TypeInfo` | read-only data |
| `loimi.argc`/`argv` | written once by `main` (executables only) |

**Rules.**

- One thread at a time per state. A state may move between threads if
  the hand-over synchronizes, as a mutex or `pthread_join` does.
- A value of one state is never stored in another: copy it.
- Each thread needs the C stack its nesting depth needs (a native
  calling back into the interpreter uses C stack per level).
- States on one thread may call into each other (a native of B runs A).
  The collector then skips the other heap's objects on the shared
  shadow stack.

**Not thread-safe:**

- module-level `var`s without `#[thread_local]` (plain globals, as in
  C). In `std` these are `mem`'s read-only empty-`Vec` sentinel and
  `std.test`'s, used by the single-threaded test runner;
- `--vm-stats` counters (process-wide, updated without atomics: profile
  one thread);
- the copy-and-patch JIT (`--jit-stencils`: executables only, one global
  code buffer).

`--sanitize=thread` builds with ThreadSanitizer, as above.
`examples/embed/threads.c` runs four Mini states on four threads at
once, and is clean under it. Runtime thread-locals cost nothing
measurable: initial-exec TLS is one `%fs`-relative access, the same
instruction count as an absolute address.

#### Threads sharing one state: `std.gil`

`std.gil` gives a heap a global interpreter lock that a thread must hold
to run vm code or touch the heap. Mini built with `--cfg gil` uses it,
so any number of threads run one state (`examples/embed/shared.c`):

```c
MiniState *S = mini_new();              /* this thread holds the lock */
...                                      /* other threads: */
mini_thread_attach(S);                  /* own Mini thread; waits for the lock */
mini_push_global(S, "work"); mini_call(S, 0);
mini_unlock(S); sleep(1); mini_lock(S); /* Py_BEGIN/END_ALLOW_THREADS */
mini_thread_detach(S);                  /* lets go of the lock */
```

- **Shared**: the heap, globals, tables, functions and their code
  (inline caches, quickened opcodes), the string table.
- **Per thread**: each thread runs on its own Mini thread (value and
  frame stacks, main coroutine, open upvalues, the innermost run), as
  Lua's `lua_newthread` gives, with its own API value stack, native
  arguments and error message. The context holds the running thread's
  part; when the lock changes hands, a `save` hook moves it out and a
  `load` hook installs the next holder's (`examples/mini/threads.lm`).
  For a handler, a hand-over is a call that may collect (`uses gc`).
- **Hand-over at safepoints.** A thread that wants the lock takes a
  ticket and waits. The holder polls `gil.waiting()`, a relaxed atomic
  load, at every taken jump (loop back-edges among them) and every call
  of a Lua function ([polls](#polls)), and calls `yield_now()`, which
  lets the waiters run first (first come, first served) and takes the
  lock back. So a Lua loop that never calls a native or allocates still
  hands over. Natives that block bracket the wait with
  `mini_unlock`/`mini_lock`.
- **Collections** run on the holder, which marks its own shadow stack
  and the scanners' roots. The lock's scanner adds every other attached
  thread's shadow-stack frames (saved when it let go) and its saved
  per-thread state, so a thread stopped in a native keeps its values
  alive ([stdlib/gc.md](stdlib/gc.md)).
- **No atomics anywhere else.** Only the holder runs vm code or touches
  the heap, and the hand-over is a mutex, which orders one holder's
  writes before the next holder's reads. So `become`, opcode rewrites,
  inline-cache updates, the write barrier and the collector need
  nothing. ThreadSanitizer agrees.
- **Rules.** Every API call needs the lock. The thread that created the
  state stays attached until `mini_free` (it may let go of the lock);
  other threads detach outside any run, and before `mini_free`. A
  thread that has let go of the lock must not call the API. A Lua
  coroutine runs on whichever thread resumes it; resuming one that runs
  on another thread fails, as resuming a running one does. A
  read-modify-write such as `total = total + 1` has no safepoint inside,
  so it is atomic; anything spanning a call or a back-edge is not, as in
  Python.
- **Cost.** Without `--cfg gil`, nothing changes. With it,
  single-threaded Mini runs 1.2–1.4% slower (worst case 4–5%, on calls
  and tight loops), from the poll's two loads and a branch.
- **No free threading.** Threads share a heap one at a time; running vm
  code in one heap on several threads at once is not supported.

#### Re-entry from C natives

A C function registered as a native runs inside a handler's `cold`
block. Its type in Loimi says so: Mini stores natives as
`fn(*MiniState, *u8) -> i32 uses reenter`, so the call through the
pointer has the effect, and the handler syncs before it and reloads
after ([section 5](#5-effects-and-sync-points)). The header's C type is
the plain function pointer: C cannot see effects, and needs none,
because the Loimi side assumes the worst.

In Mini's API, a native may then:

- call a Mini function: `mini_call(S, nargs)` calls the function below
  `nargs` arguments on the state's value stack (pushed with
  `mini_push_global`, `mini_push_arg`, `mini_push_number`, ...), as a
  nested run, and pushes the result, or the error value with
  `MiniStatus_Runtime`. `mini_raise_top` re-raises that error unchanged;
- run a chunk: `mini_dostring` from a native is a nested run too;
- call into another state: a run of that state, counted in the same
  thread-local `Mini.depth`.

C never holds a Mini value: values stay on the value stack, a GC root
that `mini_call` and natives use by position (-1 is the top), and what a
native pushes is popped when it returns. Strings from `mini_to_string`
stay valid while their value is on the stack. More than 200 nested runs
on a thread fail with "C stack overflow". A native must not free its own
state. The host can also call Mini functions between runs, with the same
`mini_call`.

`mini_new`, `mini_dostring` and `mini_call` are `#[catch_panic]`: a
panic in the interpreter returns NULL or `MiniStatus_Internal` and
poisons the state, which then refuses to run code; `mini_error` gives
the panic's message.

The worked example is `examples/mini/embed.lm` (the Mini interpreter as
a library) with `examples/embed/host.c`: two states alive at once,
globals both ways, C natives with user data, errors with messages,
natives calling back into Mini (nested past the limit, under GC stress,
and into another state), and an internal error in a nested run.

---

## 11. What Loimi deliberately leaves out

- **No optimizing JIT.** Each handler is a separate LLVM function with a
  known register interface, which is the input a copy-and-patch baseline
  JIT needs. Loimi has one (`--jit-stencils`); there is no trace
  compiler.
- **No borrow checker or ownership types.** Arenas and the GC cover
  interpreter workloads. The complexity budget goes to the hot loop.
- **No exceptions or unwinding in host code.** Guest exceptions are a VM
  concept (`raise` / `on raise`). Host errors are `Result`. The escapes
  that exist (`std.rt.protect`/`throw`, `#[catch_panic]`) are
  `longjmp`-style tools for embedding: no `defer`s run
  ([section 10.1](#errors-and-panics)).
- **No implicit runtime.** No hidden allocations, no implicit GC outside
  `std.gc`.
