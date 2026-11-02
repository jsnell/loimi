# MiniJS: a JavaScript subset in Loimi

The third proof interpreter. It runs [MiniJS](SPEC.md), a subset of
ECMAScript 2020 scripts: closures, objects with prototypes, classes with
`extends`/`super`, arrays, strings, exceptions with `finally`, and a
small standard library. Every MiniJS program prints what `node` prints.

```sh
cargo build --release
./target/release/loimi build examples/minijs/main.lm --release -o minijs
./minijs program.js            # a second argument turns on GC stress
cargo test --release --test golden minijs_programs
bench/minijs/compare.py -n 3   # vs node --jitless, node and QuickJS
```

It follows Mini and Scheme (NaN-boxed values, a register VM with `cold`
paths, a `sync` block, `on raise`, quickened families), and it is the
first interpreter built mostly on the standard library: `std.lex`,
`std.parse`, `std.bc`, `std.obj`, `std.num`, `std.gc`, `std.map`.

## Files

| File | Contents |
|---|---|
| `value.lm` | the `V` value type, heap objects (strings, ropes, objects, arrays, functions, boxes, prototypes), conversions, `===` |
| `rt.lm` | the context (stacks, globals, keys, builtin prototypes, cache side tables), property access for every kind of value, ToString, arrays, errors, GC roots |
| `ast.lm` | tokens and the lexer (on `std.lex`), syntax trees, variables |
| `parser.lm` | recursive-descent statements, binary operators through `std.parse`'s Pratt driver |
| `compile.lm` | name resolution and capture analysis, then code generation with `std.bc` |
| `vm.lm` | the `vm JS` declaration: ops, inline caches, calls, exceptions |
| `builtins.lm` | natives and the global environment |
| `prelude.lm` | array methods written in JavaScript (`map`, `filter`, `forEach`, `reduce`, `some`, `every`, `find`, `findIndex`, `sort`) |
| `main.lm` | driver |
| `tests/*.js` | feature tests, `.expected` from node |
| `tests/conformance/*.js` | the clean-room conformance suite (do not edit) |

## Architecture

### Values

```loimi
pub value V repr nanbox {
    Num(f64) unboxed,
    Int(i32),
    S(Gc<Str>), O(Gc<Obj>), A(Gc<Arr>), F(Gc<Fn>), B(Gc<Box>), R(Gc<Rope>),
    Undef, Null, Bool(bool),
}
```

Ten tagged variants use the packed nanbox layout: the six pointer
variants take 48-bit blocks, `Int(i32)` a 32-bit one, and the
nullary/boolean variants share what is left.

**`Int` is unobservable.** Every number is a double; `Int` is an
optimization for integral values that fit in an i32 and are not `-0`.
Int arithmetic checks for overflow and falls back to doubles; `*` of a
zero product with a negative operand gives `Num(-0)`; `/` produces an Int
only for exact quotients; `===` compares Int with Num numerically; and
`value.number(x)` normalizes natives' results. `-0`, `NaN` and `x - 0` (not
`x + -0`, so no `ADDI` for it) are handled where they matter.

**Strings** are byte strings (MiniJS programs are ASCII). They are not
interned: `===` compares bytes (after pointer equality). Property names
are *keys*: small integers from a `text.Interner` that never die, cached
in the string (`Str.key`), so a string that is collected and rebuilt maps
to the same key, which std.obj requires.

**Ropes.** `a + b` of at least 64 bytes makes an `R(Gc<Rope>)` instead of
copying, so building a string with `+=` is linear. A rope is flattened
(iteratively: `+=` ropes are as deep as they are long) the first time
anything other than `+`, `.length`, `typeof` or truthiness looks at it;
the flat string is cached in the rope, and the ops that read strings
replace a rope in a register with its flat string. Natives only ever see
flat strings. This took the `strings` benchmark from 4.5 s to 2.2 s.

**Arrays** (`A`) are a length and a pointer into a GC'd `Vals` store
with spare capacity; `a[i]` with an Int index in range is the
`GETINDEX` fast path, and writing at `length` appends. Arrays are not
`std.obj` objects: they have no own named properties (out of the subset
in practice); their methods come from `Array.prototype`.

**Functions** (`F`) carry their prototype (bytecode), free variables,
`home` (the object `super.x` searches) and `parent` (the constructor
`super(...)` calls); arrows copy both from their creator. A function's own
properties live in a `props` bag (an `Obj`) created on first use, whose
slot 0 is always `prototype`, so `new` finds the prototype without a
lookup. Natives are `Fn`s with a native id.

### Objects: std.obj

Objects are `Obj { shape, dict, overflow, proto, flags, inline: [V] }`,
the interpreter's own object type over std.obj's generic `Model`. Shapes
are hidden classes in a transition tree per (prototype, inline count), so
a shape check also checks the prototype. Dictionary mode, overflow slots,
validity cells and key order (`Object.keys`) come from std.obj; MiniJS
adds JS's key order (array indices first) and enumerability (flags for
"all properties non-enumerable", used by builtin and class prototypes,
and "`constructor` is not enumerable", used by `F.prototype`).

**Allocation size.** The compiler counts the `this.x = ...` assignments
in a constructor; `class D extends B` adds B's count when the class is
created. `new` allocates exactly that many inline slots, and the root
shape for (prototype, slots) is cached in the function, so `NEW` is a
fast path that allocates and enters the constructor without a lookup.
If objects end up bigger, the function learns the larger size.

### Inline caches

Property sites carry a four-word cache, `{ shape, slot, ent, miss }`, and
are families:

- `GETPROP` / `GETMETHOD` (`o.m(...)`: method and receiver in one op):
  - `_IN` / `_OUT`: own property, inline or overflow slot;
  - `_P`: found on a prototype, cached as std.obj's `ProtoEntry`
    (receiver shape, holder, slot, validity cell) in a side table; a hit
    is a shape compare, a cell check and a load, whatever the depth;
  - `_AP` / `_SP`: methods of arrays and strings (`a.push`,
    `s.charCodeAt`): the receiver's variant plus the shape of
    `Array.prototype` / `String.prototype`;
  - `_POLY`: up to eight (shape, own slot or prototype entry) pairs;
  - `_MEGA`: std.obj's `MegaCache` for own properties, plus a
    direct-mapped table of prototype entries.

  A monomorphic site that misses is refilled a few times (sites are
  often monomorphic per phase) before it goes polymorphic; a full
  polymorphic entry is restarted a few times (see "shape ids" below)
  before the site goes megamorphic.
- `SETPROP`: `_IN` / `_OUT` stores, and `_ADD`, a cached transition
  (old shape → new shape, inline slot) for the property adds that
  constructors do.

**Shape ids across collections.** std.obj's transitions are weak: when
the last object of a shape dies, the shape is collected, and the next
object built the same way gets a new shape with a new id, which a cache
sees as a new shape. deltablue rebuilds its constraint graph again and
again, and its call sites went megamorphic. MiniJS now *pins* every shape
a cache records (`Shapes.pin`, added to std.obj), so its id stays valid.

### Calls

R0 is `this`, R1.. the parameters. `CALL base n` takes the callee from
R[base], `this` from R[base+1] and the arguments above it, so the
callee's window starts at base+1 and nothing is copied. The fast path
covers every bytecode function except class constructors, whatever the
argument count: missing parameters and the rest of the window are
cleared, extra arguments are overwritten by locals. Natives are called
from CALL's hot path too. `new` frames are flagged so that `RET` returns
`this` unless the constructor returns an object.

Derived constructors keep the pre-allocated object in a hidden register
until `super(...)` returns (`SUPERDONE`); until then `this` reads are
checked (`CHECKTHIS`), and returning without calling `super` throws,
as in JavaScript.

### Closures and variables

The compiler resolves names in a separate pass, so it knows which
variables are captured and assigned. Closures are flat: they copy the
values of their free variables when created. A variable lives in a box
when it is captured and either assigned after declaration or captured by
a closure that may be created before the variable has its value (a
textual-position test; hoisted functions count as created at block
entry). Hoisted function declarations that capture each other are
patched (`SETFREE`) after all of them exist. `for (let ...)` bindings that
are boxed get a fresh box per iteration (`REBOX` before the update), so
each iteration's closures see their own copy. Top-level declarations are
global slots resolved at compile time. Reading an undeclared variable
compiles to `THROWREF` (a ReferenceError when executed).

### Exceptions

`try` blocks record (start, end, handler, register) ranges in a handler
table per prototype; `on raise` walks frames from the faulting pc and
jumps to the first covering handler, with the exception in its register.
A `break`, `continue` or `return` that leaves a `try ... finally` runs a
copy of the finally block first (as javac does), and the protected
ranges of the `try` statements it crosses are split around that copy, so
an exception in it is not caught by them. Uncaught exceptions flush
standard output, print `Uncaught ...` to stderr and exit with status 1.

### Builtins and the prelude

Natives receive `this` and the arguments in place. They allocate but
never collect, and never call back into the VM: the array methods that
take callbacks, including a stable merge sort, are JavaScript in the
prelude, compiled and run before the program. That keeps the VM free of
`reenter`.

`Number::toString` uses `std.num.write_js` (added to std.num): the
shortest round-trip digits from `std.num.shortest`, laid out by
ECMAScript's rules. `toFixed` rounds the exact decimal expansion half up,
as JavaScript does (C's `printf` rounds ties to even).

### GC

Mark-sweep from std.gc, collecting only at safepoints: the ops that
allocate check the threshold first (in a cold path) and collect with the
frame pointer published by the `sync` block. Roots are the globals,
keys, builtin prototypes, cache side tables and every frame's register
window. GC stress (a second argument) collects at every safepoint; the
test runner uses it for every test. The minimum heap is raised to 64 MB,
since std.gc's policy (collect again after allocating as much as is live)
spent a quarter of binary_trees in the collector.

## Spec coverage

All of SPEC.md: the 56 clean-room conformance tests and the 14
benchmarks pass, normally and under GC stress. Beyond it, `==`/`!=`
(loosely), `??=`/`&&=`/`||=`, `find`/`findIndex`/`lastIndexOf`,
`RangeError`/`ReferenceError`/`TypeError` constructors, Error objects
printing as `Name: message`.

Not supported: `instanceof` (a compile error), class expressions,
getters and setters, holes in arrays and own named properties on arrays,
`arguments`, spread/rest, destructuring, template literals, regular
expressions, labels on non-loop statements other than blocks, a
`reduce` without its initial value (the subset requires it), named
function expressions referring to themselves from arrow functions inside
them, and `this` in arrow functions created in a derived constructor
before `super()`.

## Performance

See the table at the end of this file (`bench/minijs/compare.py`).
