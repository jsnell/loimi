# MiniJS: a JavaScript subset in Loimi

MiniJS is an interpreter for a subset of ECMAScript 2020 scripts, written
in Loimi. This README describes how it is built. The language it accepts
is defined in [SPEC.md](SPEC.md).

The subset has closures, objects with prototypes, classes with
`extends` and `super`, arrays, strings, exceptions with `finally`, and a
small standard library. Every MiniJS program prints what `node` prints.

MiniJS uses the same techniques as the Mini and Scheme examples:
NaN-boxed values, a register VM with `cold` slow paths, a `sync` block,
`on raise`, and quickened instruction families. Most of its machinery
comes from the standard library:

- `std.lex` and `std.parse` for the front end;
- `std.bc` for emitting bytecode;
- `std.obj` for objects, shapes and inline caches;
- `std.num` for number formatting;
- `std.gc` and `std.map`.

## Build, run and test

```sh
cargo build --release
./target/release/loimi build examples/minijs/main.lm --release -o minijs
./minijs program.js
cargo test --release --test golden minijs_programs
bench/minijs/compare.py -n 3   # against node --jitless, node and QuickJS
```

The command-line options and environment variables are listed in
[SPEC.md](SPEC.md#running-minijs).

## Files

| File | Contents |
|---|---|
| `value.lm` | The `V` value type; heap objects (strings, ropes, objects, arrays, functions, boxes, prototypes); conversions; `===`. |
| `rt.lm` | The context (stacks, globals, keys, builtin prototypes, cache side tables); property access for every kind of value; ToString; arrays; errors; GC roots. |
| `ast.lm` | Tokens and the lexer (on `std.lex`), syntax trees, variables. |
| `parser.lm` | Recursive-descent statements. Expressions use `std.parse`'s Pratt parser, with hooks for assignment, `?:` and `x => ...`. |
| `compile.lm` | Name resolution and capture analysis, then code generation with `std.bc`. |
| `vm.lm` | The `vm JS` declaration: ops, inline caches, calls, exceptions. |
| `builtins.lm` | Natives and the global environment. |
| `prelude.lm` | Array methods written in JavaScript (`map`, `filter`, `forEach`, `reduce`, `some`, `every`, `find`, `findIndex`, `sort`). |
| `main.lm` | The driver. |
| `tests/*.js` | Feature tests, with `.expected` output from node. |
| `tests/conformance/*.js` | The clean-room conformance suite. Do not edit. |

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

There are ten tagged variants, so the NaN box uses the packed layout:
the six pointer variants get 48-bit blocks, `Int(i32)` a 32-bit one, and
the variants without a payload, or with a boolean, share what is left.

**`Int` is invisible to programs.** Every JavaScript number is a double.
`Int` holds integral values that fit in an i32 and are not `-0`, as an
optimization:

- Int arithmetic checks for overflow and falls back to doubles.
- `*` of a zero product with a negative operand gives `Num(-0)`.
- `/` gives an Int only for exact quotients.
- `===` compares Int with Num by value.
- `value.number(x)` normalizes the results of natives.

### Arithmetic and comparisons

Arithmetic is quickened as in Mini:

- `ADD`, `SUB` and `MUL` are families. The generic member tests for two
  Ints first. A site that sees two floats becomes `_F`, which adds the
  raw words as doubles and keeps the result if it is not a NaN (a tagged
  word is a NaN as a double). A site that first sees an Int and a Num
  becomes `_IF` or `_FI`.
- `DIV` is a family too. The generic member divides two Ints first (an
  exact quotient stays an Int); `DIV_F` divides the raw words first.
- Quickening re-runs `JS.optimize` on the function, so superinstructions
  of quickened members (`MUL_F; ADD_F`) form.
- An Int overflow produces the double inline, without leaving the
  generic member.
- Bit operations and `%` have forms with an Int constant (`SHLK`, `BORK`,
  `MODK`, ...). Conditions compare with small constants directly
  (`JLTI`, `JGEI`) and with other number constants from the pool
  (`JLTK`, `JLEK`, ...). `===` and `!==` have constant forms (`EQK`,
  `NEK`).

The slow paths of all of these are `cold` blocks, so the handlers have
no stack frame (checked by `-W handler_frame`).

### Compiler

Besides plain code generation, the compiler:

- folds top-level `const`s initialized with literals (the spec rules out
  reading them before their declaration);
- compiles a `switch` on Int cases in a small range to a jump table
  (`SWITCHI` and one `JMP` per value);
- puts loop tests at the bottom;
- drops the `TONUM` of `x++` when the old value is unused.

### Strings and ropes

Strings are byte strings, since MiniJS programs are ASCII. They are not
interned: `===` compares pointers, then bytes.

Property names are *keys*: small integers from a `text.Interner` that are
never freed. A string caches its key (`Str.key`), so a string that is
collected and rebuilt maps to the same key, which std.obj requires.

`a + b` with a result of at least 64 bytes makes a rope
(`R(Gc<Rope>)`) instead of copying, so building a string with `+=` takes
linear time. A rope is flattened the first time anything other than `+`,
`.length`, `typeof` or a truth test looks at it. The flat string is
cached in the rope, and the ops that read strings replace a rope in a
register with its flat string. Natives only see flat strings. Ropes took
the `strings` benchmark from 4.5 s to 2.2 s.

### Arrays and functions

An array (`A`) is a length and a pointer into a GC'd `Vals` store with
spare capacity. `a[i]` with an Int index in range is the `GETINDEX` fast
path, and writing at `length` appends. Arrays are not `std.obj` objects:
they have no own named properties, and their methods come from
`Array.prototype`.

A function (`F`) carries:

- its prototype (the bytecode) and its free variables;
- `home`, the object that `super.x` searches;
- `parent`, the constructor that `super(...)` calls;
- `props`, its own properties, created on first use. Slot 0 is always
  `prototype`, so `new` finds the prototype without a lookup.

Arrow functions copy `home` and `parent` from their creator. Natives are
`Fn`s with a native id.

### Objects

Objects are `Obj { shape, dict, overflow, proto, inline: [V] }`, the
interpreter's own object type over std.obj's generic `Model`. Shapes
(hidden classes) form a transition tree per (prototype, inline slot
count), so a shape check also checks the prototype.

These come from std.obj: dictionary mode, overflow slots, validity cells,
the inline-cache state machine, per-property attributes, and JavaScript
key order (`obj.index_keys_first`).

Enumerability is the attribute bit `value.NONENUM`. It is set on built-in
methods, class methods, `constructor` and `prototype`
(`Ctx.define_hidden`). So properties a program adds to a built-in or
class prototype, or to a function, are enumerable, as in JavaScript.

**Allocation size.** The compiler counts the `this.x = ...` assignments
in a constructor, and `class D extends B` adds B's count when the class
is created. `new` allocates exactly that many inline slots, and the
function caches the root shape for (prototype, slots). So `NEW` is a fast
path that allocates and enters the constructor without a lookup. If
objects end up bigger, the function learns the larger size.

**Shape ids survive collections.** When the last object of a shape dies,
the shape is collected, but std.obj keeps its id for the path that built
it. The next object built the same way gets a shape with the same id, so
caches keep hitting. This matters for deltablue, which rebuilds its
constraint graph again and again.

### Inline caches

Each property site has a six-word cache, std.obj's `Site`. std.obj runs
the cache state machine in the slow paths (`m.read_miss`,
`m.write_miss`), and the members of each op family are its states.

`GETPROP`, and `GETMETHOD` for `o.m(...)` (method and receiver in one
op):

| Member | Handles |
|---|---|
| `_IN`, `_OUT` | An own property in an inline or overflow slot. |
| `_P` | A property found on a prototype. The site holds the receiver's shape, the prototype's validity cell, the holder's depth and slot. |
| `_MISS` | A monomorphic site that missed. std.obj refills it; after 8 misses it becomes polymorphic. |
| `_POLY` | Up to eight entries. After 8 refills it becomes megamorphic. |
| `_MEGA` | std.obj's shared `MegaCache`. |
| `_AP`, `_SP` | Methods of arrays and strings (`a.push`, `s.charCodeAt`): the receiver's variant plus the shape of `Array.prototype` or `String.prototype`. |

Reads of absent properties (`o.z === undefined`) are cached too, in
`_POLY` and `_MEGA` entries.

`SETPROP` has `_IN` and `_OUT` stores, and `_ADD`, a cached transition
(old shape to new shape, inline slot) for the property adds that
constructors do. A store site that keeps missing becomes `_POLY`, then
`_MEGA`. `key in o` goes through the `MegaCache` too.

### Calls

R0 is `this` and R1 onward are the parameters. `CALL base n` takes the
callee from R[base], `this` from R[base+1] and the arguments above it, so
the callee's window starts at base+1 and nothing is copied.

- **Fast path.** It covers every bytecode function except class
  constructors, with any argument count. Missing arguments are set to
  undefined in a cold block.
- **Registers are not cleared.** The compiler writes every register
  before reading it, so the callee's other registers may hold stale
  values. They are still safe for the collector: the stack is a
  `std.rt.ValueStack` with a high-water mark, and `scan_roots` clears the
  slots above the innermost window up to the mark. A stale register can
  at worst keep a dead object alive until the next collection.
- **Returns.** `new` frames and the bottom frame are flagged, so `RET`
  tests one word. A call's result is used where it lands (`return f(x)`
  returns from the call base).
- **Inline natives.** `CALL` is a family. A site whose callee is
  `charCodeAt`, `charAt`, `push`, `Math.floor` or `Math.sqrt` becomes a
  member that checks the callee and runs the native inline (`CALL_CCA`,
  `CALL_PUSH`, ...). Other natives are called out of line.
- **Derived constructors** keep the new object in a hidden register until
  `super(...)` returns (`SUPERDONE`). Until then `this` reads are checked
  (`CHECKTHIS`).

### Closures and variables

The compiler resolves names in a separate pass, so it knows which
variables are captured and which are assigned. Closures are flat: they
copy the values of their free variables when created.

A variable lives in a box when it is captured and either:

- assigned after its declaration, or
- captured by a closure that may be created before the variable has its
  value. Hoisted functions count as created at block entry.

Other details:

- Hoisted function declarations that capture each other are patched
  (`SETFREE`) after all of them exist.
- Boxed `for (let ...)` bindings get a fresh box per iteration (`REBOX`),
  so each iteration's closures see their own copy.
- Top-level declarations are global slots resolved at compile time.
- Reading an undeclared variable compiles to `THROWREF`, which throws a
  ReferenceError when executed.

### Exceptions

Each prototype has a handler table of (start, end, handler, register)
ranges for its `try` blocks. `on raise` walks the frames from the
faulting pc and jumps to the first handler that covers it, with the
exception in its register.

A `break`, `continue` or `return` that leaves a `try ... finally` runs a
copy of the finally block first, as javac does. The protected ranges of
the `try` statements it crosses are split around that copy, so an
exception in it is not caught by them.

An uncaught exception flushes standard output, prints `Uncaught ...` to
standard error and exits with status 1.

### Natives and the prelude

Natives receive `this` and the arguments in place. They allocate but
never collect and never call back into the VM. The array methods that
take callbacks, including a stable merge sort, are written in JavaScript
in the prelude, which is compiled and run before the program. That keeps
the VM free of `reenter`.

This costs little: `data.map(f)` runs as fast as the same loop written in
the program (0.136 s against 0.149 s for 6M elements), and as fast as
V8's native `map` in `--jitless` mode (0.128 s).

Number formatting uses `std.num.write_js`: the shortest round-trip digits
from `std.num.shortest`, laid out by ECMAScript's rules. `toFixed` rounds
the exact decimal expansion half up, as JavaScript does (C's `printf`
rounds ties to even).

### GC

MiniJS uses mark-sweep from std.gc and collects only at safepoints. The
ops that allocate check the threshold first, in a cold path, and collect
with the frame pointer published by the `sync` block. The roots are the
globals, keys, builtin prototypes, cache side tables and every frame's
register window. The test runner also runs every test with a collection
at every safepoint.

The heap grows by 300% with an 8 MiB minimum (`Heap.set_growth`).
std.gc's default (collect again after allocating as much as is live, at
least 1 MiB) spent a quarter of binary_trees in the collector. A 64 MiB
minimum at 100% is as fast, but uses 70 MB where 300% / 8 MiB uses
12–17 MB on programs with little live data.

## Spec coverage

All of SPEC.md is supported: the 56 clean-room conformance tests and the
14 benchmarks pass, normally and under GC stress.

Accepted beyond the spec:

- `==` and `!=` (loosely);
- `??=`, `&&=` and `||=`;
- `find`, `findIndex` and `lastIndexOf`;
- the `RangeError`, `ReferenceError` and `TypeError` constructors;
- Error objects printing as `Name: message`.

Not supported:

- `instanceof` (a compile error);
- class expressions, getters and setters;
- holes in arrays, and own named properties on arrays;
- `arguments`, spread and rest, destructuring;
- template literals and regular expressions;
- labels on statements other than loops and blocks;
- `reduce` without its initial value (the spec requires it);
- named function expressions referring to themselves from arrow
  functions inside them;
- `this` in arrow functions created in a derived constructor before
  `super()`.

## Performance

MiniJS takes about half of `node --jitless`'s time (V8's bytecode
interpreter) and about 0.4 of QuickJS's, by geometric mean over the 14
benchmarks. Current numbers are in
[`docs/results.md`](../../docs/results.md).

What mattered most, in order:

1. inline caches for own properties and for prototype methods (validity
   cells);
2. ropes for `+=`;
3. shape ids that survive collections, so that sites stay monomorphic;
4. polymorphic entries with a refill budget, instead of a monomorphic
   site that keeps missing;
5. the `NEW` fast path with exact allocation sizes;
6. a larger GC budget;
7. mixed Int/float arithmetic inline;
8. natives called from `CALL`'s hot path.
