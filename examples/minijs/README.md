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
| `parser.lm` | recursive-descent statements; expressions through `std.parse`'s Pratt driver, with `led` hooks for assignment, `?:` and `x => ...` |
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

**Arithmetic** is quickened as in Mini: `ADD`, `SUB` and `MUL` are
families whose generic member tests (Int, Int) first; a site that sees
two floats becomes `_F`, which adds the raw words as doubles and keeps a
result that is not a NaN (a tagged word is a NaN as a double), and a site
that first sees an Int and a Num becomes `_IF`/`_FI`. Quickening re-runs
`JS.optimize` on the function, so supers of quickened members
(`MUL_F; ADD_F`) form. `DIV` divides the raw words first. Bit operations
and `%` have forms with an Int constant (`SHLK`, `BORK`, `MODK`, ...),
and conditions compare with small constants directly (`JLTI`, `JGEI`).

**The compiler** folds top-level `const`s initialized with literals
(SPEC.md rules out reading them before their declaration), compiles
`switch` on Int cases in a small range to a jump table (`SWITCHI` and
one `JMP` per value), puts loop tests at the bottom, and drops the
`TONUM` of `x++` when the old value is unused. `MINIJS_DIS=1` prints
every function's code after fusion (`JS.disasm`).

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

Objects are `Obj { shape, dict, overflow, proto, inline: [V] }`, the
interpreter's own object type over std.obj's generic `Model`. Shapes are
hidden classes in a transition tree per (prototype, inline count), so a
shape check also checks the prototype. Dictionary mode, overflow slots,
validity cells, the inline-cache state machine, per-property attributes
and JS key order (`obj.index_keys_first`, given MiniJS's "is this key an
array index") come from std.obj. Enumerability is attribute bit
`value.NONENUM`, set on built-in methods, class methods, `constructor`
and `prototype` (`Ctx.define_hidden`), so properties a program adds to
a built-in or class prototype, or to a function, are enumerable, as in
JavaScript. Objects become std.obj prototypes (`make_proto`) when they
are first used as one.

**Allocation size.** The compiler counts the `this.x = ...` assignments
in a constructor; `class D extends B` adds B's count when the class is
created. `new` allocates exactly that many inline slots, and the root
shape for (prototype, slots) is cached in the function, so `NEW` is a
fast path that allocates and enters the constructor without a lookup.
If objects end up bigger, the function learns the larger size.

### Inline caches

Property sites carry a six-word cache, std.obj's `Site`: `{ sid, slot,
cell, depth, ent, miss }`. std.obj runs the state machine in the slow
paths (`m.read_miss`, `m.write_miss`); the families' members are its
states:

- `GETPROP` / `GETMETHOD` (`o.m(...)`: method and receiver in one op):
  - `_IN` / `_OUT`: own property, inline or overflow slot;
  - `_P`: found on a prototype: the receiver's shape, the validity cell
    of its prototype, the holder's depth and slot, all plain words; a
    hit is a shape compare, a byte load for the cell, `depth` loads of
    `proto` and the slot load;
  - `_MISS`: a monomorphic member missed; std.obj refills the site
    (sites are often monomorphic per phase) and after 8 misses makes it
    polymorphic;
  - `_POLY`: up to eight (shape, own slot or prototype hit) entries; a
    full entry starts over up to 8 times before the site goes
    megamorphic;
  - `_MEGA`: std.obj's shared `MegaCache`, own and prototype hits;
  - reads of absent properties (`o.z === undefined` across shapes) are
    cached too, in Poly and Mega entries (std.obj's `ic.absent`): the
    receiver's shape plus the validity cell of its prototype;
  - `_AP` / `_SP`: methods of arrays and strings (`a.push`,
    `s.charCodeAt`), MiniJS's own: the receiver's variant plus the shape
    of `Array.prototype` / `String.prototype`. A rope receiver is
    replaced by its flat string in its register, so a loop over
    `s.charCodeAt(i)` flattens once.
- `SETPROP`: `_IN` / `_OUT` stores, and `_ADD`, a cached transition
  (old shape → new shape, inline slot) for the property adds that
  constructors do. The shape check is enough: objects that are
  prototypes have shapes in std.obj's prototype tree. A store site that
  keeps missing becomes `_POLY` (up to eight own-slot entries), then
  `_MEGA` (stores through the shared `MegaCache`), from std.obj's
  `write_miss_in`. `key in o` goes through the `MegaCache` too.

**Shape ids across collections.** When the last object of a shape dies,
the shape is collected, but std.obj keeps its id for the path that built
it: the next object built the same way gets a shape with the same id, so
caches keep hitting. deltablue rebuilds its constraint graph again and
again; before ids were stable its call sites went megamorphic, and
MiniJS pinned every shape a cache recorded.

### Calls

R0 is `this`, R1.. the parameters. `CALL base n` takes the callee from
R[base], `this` from R[base+1] and the arguments above it, so the
callee's window starts at base+1 and nothing is copied. The fast path
covers every bytecode function except class constructors, whatever the
argument count: the registers above the fixed parameters are cleared
two at a time (the register after the window may be cleared too: it is
free), missing arguments in a cold block, and extra arguments are
overwritten by locals. `new` frames and the bottom frame are flagged, so
`RET` tests one word before returning. A call's result is used where it
lands (`return f(x)` returns from the call base).

`CALL` is a family: a site whose callee is `charCodeAt`, `charAt`,
`push`, `Math.floor` or `Math.sqrt` (with one argument) becomes a member
that compares the callee with that native and runs it inline
(`CALL_CCA`, `CALL_PUSH`, ...); other callees, or arguments outside the
inline case, take the native (or generic) path. Other natives are
called out of line.

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
`reenter`. It costs little: `data.map(f)` runs as fast as the same loop
written in the program (0.136 s against 0.149 s for 6M elements), and
as fast as V8's native `map` in `--jitless` mode (0.128 s); a VM-level
"call a closure per element" op could only save the loop's own
dispatches (about half of a trivial callback's cost).

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
test runner uses it for every test. The growth policy is 300% with an
8 MiB minimum (`Heap.set_growth`): std.gc's default (collect again after
allocating as much as is live, at least 1 MiB) spent a quarter of
binary_trees in the collector. A 64 MiB minimum at 100% is as fast, but
uses 70 MB where 300% / 8 MiB uses 12-17 MB on programs with little live
data (deltablue, json, strings); with 100% / 8 MiB, binary_trees is 25%
slower (best of 3, wall-clock seconds, loaded machine):

| | 100% / 64 MiB | 300% / 8 MiB | 200% / 16 MiB | 100% / 8 MiB |
|---|---:|---:|---:|---:|
| total of the 14 benchmarks | 13.92 | 13.82 | 13.95 | 14.23 |
| binary_trees | 1.36 | 1.33 | 1.44 | 1.72 |
| peak RSS, binary_trees | 103 MB | 111 MB | 110 MB | 69 MB |
| peak RSS, json | 70 MB | 15 MB | 23 MB | 15 MB |

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

**Current numbers and the changelog of the performance pass are in
[`bench/minijs/RESULTS.md`](../../bench/minijs/RESULTS.md)**: after it,
MiniJS takes 0.53 of node --jitless's time and 0.40 of QuickJS's
(geometric means over the 14 benchmarks), 0.23-0.82 per benchmark
against V8's interpreter. The tables below are from before that pass.

CPU seconds (user + system), best of 3, `--dispatch=tailcall`, on the
shared 8-core development machine while other agents were running
(load average about 8), so absolute numbers are inflated and noisy;
the ratios are more meaningful. QuickJS is a bytecode interpreter like
MiniJS; `node --jitless` is V8's Ignition interpreter.

| Benchmark | MiniJS | QuickJS | node --jitless | MiniJS/QuickJS | MiniJS/jitless |
|---|---:|---:|---:|---:|---:|
| awfy_micro | 1.20 | 2.89 | 2.50 | 0.42 | 0.48 |
| binary_trees | 1.70 | 7.18 | 2.82 | 0.24 | 0.60 |
| bitops | 1.26 | 2.17 | 2.03 | 0.58 | 0.62 |
| closures | 1.97 | 3.71 | 2.01 | 0.53 | 0.98 |
| deltablue | 1.22 | 3.67 | 2.19 | 0.33 | 0.56 |
| fannkuch | 0.71 | 1.94 | 1.90 | 0.37 | 0.37 |
| json | 1.72 | 3.03 | 2.13 | 0.57 | 0.81 |
| nbody | 0.88 | 1.92 | 2.55 | 0.46 | 0.34 |
| properties | 1.70 | 2.27 | 2.11 | 0.75 | 0.81 |
| raytrace | 0.95 | 3.21 | 2.16 | 0.30 | 0.44 |
| richards | 1.20 | 2.81 | 1.94 | 0.43 | 0.62 |
| sort | 1.37 | 2.73 | 1.94 | 0.50 | 0.71 |
| spectral_norm | 1.25 | 1.98 | 2.54 | 0.63 | 0.49 |
| strings | 2.17 | 2.97 | 2.02 | 0.73 | 1.07 |

MiniJS is faster than QuickJS on all 14 (0.24-0.75x) and faster than
V8's interpreter on 13 (closures by a hair); on strings it is 7% slower
(V8's strings are ropes with cheap slices and character access, MiniJS
flattens and copies). The largest remaining
costs, from a sampling profile (every handler is its own symbol):
`CALL`/`RET` (15-27% of call-heavy programs: frame push, clearing the
callee's registers, no inlining), the `MOVE`s that stage arguments, and
the cached property loads themselves (`GETPROP_IN` is five instructions
plus the dispatch, 25-30% of nbody and richards).

After the std.obj rework (stable shape ids instead of pinning, the
cache state machine and plain-word prototype hits from std.obj,
per-property attributes) and the GC growth change, best of 5 wall-clock
seconds, interleaved runs of both binaries on the same loaded machine:

| Benchmark | before | after |
|---|---:|---:|
| awfy_micro | 0.87 | 0.85 |
| binary_trees | 1.93 | 1.51 |
| bitops | 0.95 | 0.94 |
| closures | 1.32 | 1.32 |
| deltablue | 0.92 | 0.98 |
| fannkuch | 0.50 | 0.52 |
| json | 1.34 | 1.40 |
| nbody | 0.62 | 0.62 |
| properties | 1.10 | 1.01 |
| raytrace | 0.66 | 0.64 |
| richards | 0.89 | 0.87 |
| sort | 0.94 | 0.97 |
| spectral_norm | 1.01 | 1.02 |
| strings | 1.51 | 1.51 |
| total | 14.55 | 14.14 |

What mattered, in order: inline caches for own properties and for
prototype methods (validity cells), ropes for `+=`, shape ids that
survive collections so that sites stay monomorphic, polymorphic entries
with a refill budget instead of thrashing a monomorphic site, the `NEW`
fast path with exact allocation sizes, a larger GC budget, mixed
int/float arithmetic inline, and natives called from `CALL`'s hot path.
