# The MiniJS language

MiniJS is a subset of JavaScript (ECMAScript 2020, non-module scripts).
This document defines the subset, for people who write MiniJS programs or
implement MiniJS.

A valid MiniJS program is a valid JavaScript program, and it produces the
same standard output under any conforming JavaScript engine, such as
`node`. For anything this document does not say, the ECMAScript
specification applies.

A program that uses a feature outside the subset is not a MiniJS
program. An implementation may reject it, or it may run it with
different behaviour.

## Running MiniJS

```sh
./target/release/loimi build examples/minijs/main.lm --release -o minijs
./minijs program.js
```

| Command line | What it does |
|---|---|
| `minijs PROGRAM` | Runs the program. |
| `minijs PROGRAM stress` | Runs it with a garbage collection at every safepoint (for testing). |
| `minijs PROGRAM verify` | As `stress`, with a generational heap whose write barriers are checked at each collection. |

| Environment variable | Effect |
|---|---|
| `MINIJS_DIS=1` | Prints every compiled function's bytecode. |
| `MINIJS_STATS=1` | Prints object-shape and cache statistics to standard error at exit. |

## 1. Lexical structure

- Source text is ASCII. Whitespace is spaces, tabs, CR and LF.
- **Comments** are `// ...` and `/* ... */`.
- **Identifiers** contain letters, digits, `_` and `$`, and do not start
  with a digit.
- **Reserved words** may not be used as variable names. They are the
  ECMAScript reserved words, including these:

  ```
  break case catch class const continue default delete do else extends
  false finally for function if in instanceof let new null return super
  switch this throw true try typeof undefined var void while
  ```

  Reserved words *may* be used as property names (`o.default`,
  `{class: 1}`).
- **Contextual words.** `constructor`, `static`, `of`, `get` and `set`
  are ordinary identifiers outside the places where they have meaning.
- Programs must not assign to `undefined`, `NaN` or `Infinity`.
- **Numerals:**
  - decimal: `42`, `3.25`, `.5`, `5.`, `1e9`, `1e+3`, `2.5E-3`;
  - hex: `0xff` and `0XFF`;
  - no octal, binary or BigInt literals, and no numeric separators.
- **Strings:**
  - delimited by `'` or `"`;
  - escapes `\n \t \r \\ \' \" \0`, and `\xHH` below `\x80`;
  - no line continuations, no template literals and no
    regular-expression literals.
- **Strict mode.** Programs behave the same in strict and sloppy mode:
  - no assignment to undeclared variables, and no reading of undeclared
    variables, including `typeof undeclared`;
  - no writes to string indices;
  - no use of `this` in plain function calls.

  A leading `"use strict";` directive is allowed and ignored.
- **Semicolons are required** after every statement that JavaScript
  terminates with one. Programs must not rely on automatic semicolon
  insertion.

## 2. Values and types

The types are undefined, null, boolean, number, string, object and
function. Arrays are objects.

### 2.1 Numbers

- **Doubles.** All numbers are IEEE-754 doubles, with JavaScript semantics
  (`5 / 2` is `2.5`, `1 / 0` is `Infinity`).
- **No arithmetic on strings.** Programs must not apply arithmetic,
  bitwise or relational operators to strings, except `+` for
  concatenation and string-to-string comparison. Write `Number(s)`.
- **Integer optimization.** An implementation may represent integral
  values specially, but that must not be observable.
- **`%`** is JavaScript's remainder: it takes the sign of the dividend.
- **Bitwise operators** `& | ^ ~ << >> >>>` convert their operands with
  ToInt32 (ToUint32 for `>>>`), as in JavaScript.
  `x | 0` is the idiomatic truncation to int32.

### 2.2 Strings

- A string is an immutable sequence of 16-bit code units. MiniJS
  programs use only ASCII characters, so in practice a string is a
  sequence of bytes.
- `s.length` is the length.
- `s[i]` is a one-character string, or undefined out of range.
- `+` concatenates.
- `< <= > >=` compare code units.

### 2.3 Objects

- **Properties.** Objects are collections of properties with string keys.
  Property order follows JavaScript: array-index keys (canonical
  decimal strings of integers 0 .. 2^32−2) ascending first, then the other
  string keys in insertion order. The order is only observable through
  `Object.keys` and `for ... in`.
- **Keys** are either small non-negative integers or not numeric-looking
  at all. Programs use no other keys.
- **Prototypes.** Every object has a prototype (an object, or null).
  Property reads walk the prototype chain. Property writes create or
  update an own property, since there are no setters.
- **Object literals:**
  - forms: `{a: 1, "b": 2, c}` (shorthand) and `{m() { ... }}`
    (method shorthand);
  - computed keys `[e]: v` are allowed;
  - the prototype of a literal is `Object.prototype`.
- **`delete o.p` and `delete o[k]`** remove an own property and evaluate
  to `true`.
- **Out of the subset:** getters and setters, property attributes
  (`Object.defineProperty`, freezing), symbols and proxies.

### 2.4 Arrays

- **Literals** are `[1, 2, 3]`, with no holes and no spread.
- **`new Array(n)`** creates holes in JavaScript. Programs must write
  every element (for example with `fill`) before reading it or
  iterating over it.
- **Elements** are read with `a[i]`. An index out of range gives
  undefined.
- **Writing** `a[i] = v` with `i == a.length` appends.
- **`a.length`** can be read. Assigning a smaller value truncates the
  array.
- **Writes past the end are out of the subset:** writing at
  `i > a.length` (creating holes), and assigning a larger `length`.
- **Methods:** see §5.

### 2.5 Functions

- **Closures.** Functions are first-class closures with lexical scoping.
  Each loop iteration gets fresh `let`/`const` bindings in `for`
  loops, as in JavaScript.
- **Declarations** are `function f(a, b) { ... }`, hoisted as in
  JavaScript.
- **Expressions** are function expressions and arrow functions:
  `x => x + 1`, `(a, b) => { ... }`, `() => expr`. Arrow functions
  capture `this` lexically.
- **Arguments.** Missing arguments are undefined, and extra arguments are
  ignored.
- **Out of the subset:** default parameters, rest parameters, spread,
  destructuring, `arguments`, generators and async functions.
- **`this`:**
  - In a method call `o.m(...)` or `o[k](...)`, `this` is `o`.
  - In a plain call `f()`, `this` is undefined, because scripts are
    treated as strict mode. Programs must not depend on non-strict
    `this`.
- **`new F(args)`** works with a function `F`: it creates an object whose
  prototype is `F.prototype`, and calls `F` with that object as `this`.
  If `F` returns an object, that object is the result.
- **`F.prototype`** is an ordinary object that programs may add methods
  to.

### 2.6 Classes

- **Form:** `class C { constructor(a) { this.x = a; } m() { ... } static s() { ... } }`.
- **Inheritance:** `class D extends C { constructor() { super(1); ... } m() { return super.m() + 1; } }`.
  `super(...)` must be called before `this` is used, as in JavaScript.
- **Out of the subset:** class fields, private names, getters and setters
  in classes, and class expressions.

### 2.7 Equality and truthiness

- **Equality** is `===` and `!==`. `==` and `!=` are out of the subset.
- **Falsy values** are `false`, `0`, `-0`, `NaN`, `""`, `null` and
  `undefined`. Everything else is truthy.
- **`typeof`** gives `"undefined" "boolean" "number" "string" "object"
  "function"`, and `typeof null` is `"object"`.

### 2.8 Conversions

- **`+` with a string operand.** If either operand is a string, `+`
  converts the other with ToString and concatenates. Otherwise both must
  be numbers. Programs must not apply arithmetic to undefined, null,
  booleans or objects; under JavaScript that silently gives NaN or
  similar, and it is out of the subset.
- **ToString of numbers** follows JavaScript's Number::toString:
  - shortest round-trip digits;
  - `1e21` and above, and below `1e-6`, use exponent form;
  - `-0` prints as `0`;
  - `NaN`, `Infinity` and `-Infinity` print as themselves.
- **ToString of other values:**
  - `true`, `false`, `null`, `undefined` print as their names;
  - arrays join their elements with `,`;
  - other objects give `"[object Object]"`.
- **Relational comparisons** `< <= > >=` require two numbers or two
  strings.

## 3. Expressions

- **Primary expressions:** literals, identifiers, `this`, object and
  array literals, function and arrow expressions, and `new F(...)`.
- **Member access and calls:** `e.name`, `e[e]`, calls, and method
  calls.
- **Operators,** with JavaScript precedence and associativity:
  - unary: `! - + ~ typeof void delete`;
  - prefix and postfix `++ --` on variables and properties;
  - `**` (right-associative);
  - `* / %`, then `+ -`, then `<< >> >>>`;
  - `< <= > >= in` (`instanceof` is out of the subset);
  - `=== !==`, then `& ^ |`, then `&& ||` and `??`;
  - `?:` (conditional);
  - assignment: `= += -= *= /= %= &= |= ^= <<= >>= >>>=`;
  - the comma operator only in `for` headers.
- **`in`:** `k in o` tests a property on `o` or its prototype chain.
- **Evaluation order** is left to right, as in JavaScript.

## 4. Statements

- **Declarations:** `var`, `let` and `const`, and function
  declarations. `let` and `const` are block scoped. Several declarators
  are allowed: `let a = 1, b = 2;`.
  - Programs must not read a `let`/`const` before its declaration (the
    temporal dead zone).
  - Programs must not use `var` in a way where hoisting changes
    behaviour.
- **Simple statements:** expression statements, `{ block }`, and `;`.
- **Conditionals:** `if`/`else`, and `switch` with `case`/`default` and
  fallthrough. Case values are compared with `===`.
- **Loops:**
  - `while`, `do ... while`, and `for (init; cond; step)`.
  - `for (const x of array)`, also with `let`. Only over arrays. The
    loop body may push to the array it iterates over, with JavaScript
    semantics.
  - `for (const k in object)`: own and inherited
    enumerable string keys, in JavaScript order. Programs should iterate
    only over objects without inherited enumerable properties.
  - `break` and `continue`, including labels: `outer: for (...)` with
    `break outer;`.
- **Control:** `return`, `throw`, and `try { } catch (e) { } finally { }`.
  - Programs throw `Error` objects (`new Error("msg")`, which has
    `.message`) or plain values.
  - Built-in errors (TypeError and similar) are thrown for calling a
    non-function, reading a property of undefined or null, and calling
    `super` wrongly. Programs may catch them, but must not print their
    messages, whose text is unspecified.
- **Uncaught exceptions.** An uncaught exception terminates the program
  with a nonzero exit status. Standard output written before it must be
  flushed. The text written to standard error is unspecified.

## 5. Standard library

Only the following exist.

### Global functions

- `console.log(...args)`: writes the arguments converted with ToString,
  separated by single spaces, followed by a newline.
  - Programs must not pass `-0`, which node prints as `-0`.
  - When more than one argument is passed, the first must not contain
    `%`, because node treats it as a format string.
  - Arguments must be numbers, strings, booleans, null or undefined.
  - Printing an object or array directly is out of the subset, because
    node prints them with its own inspection format. Use `String(a)` or
    `a.join(",")` instead.
- `String(v)` converts with ToString. `String` of functions and of
  Error objects is out of the subset.
- `Number(s)` converts with JavaScript's StringToNumber:
  - surrounding whitespace is ignored, and `""` gives 0;
  - an optional sign, decimal or hex numerals, and `Infinity` are
    accepted;
  - anything else gives `NaN`.
- `parseInt(s, radix?)` and `parseFloat(s)`, for ASCII inputs.
- `isNaN(x)` and `isFinite(x)`, for number arguments.
- `Error`: `new Error(msg)` gives an object with `.message`.

### `Math`

| Kind | Names |
|---|---|
| Functions with the same result in every engine | `abs floor ceil round trunc sqrt min max sign` |
| Functions whose result may differ in the last bit | `sin cos tan atan atan2 exp log pow` |
| Constants | `Math.PI`, `Math.E` |

Programs print the results of the second group through `toFixed`. There is no
`Math.random`; programs implement their own generator.

### `Object`

- `Object.keys(o)`: own enumerable keys, in order.
- `Object.create(proto)`, where `proto` is an object or null.
- `Object.getPrototypeOf(o)`.
- `o.hasOwnProperty(k)`.

### `Array`

- Creation: `new Array(n)` (n undefined elements; the only allowed form)
  and `Array.isArray(x)`.
- Methods:
  - `push(v...)` returns the new length;
  - `pop()`, `shift()` and `unshift(v...)`;
  - `slice(start?, end?)`, with negative indices from the end;
  - `concat(arr...)`, whose arguments must be arrays;
  - `indexOf(v)`, using `===`;
  - `join(sep?)`: the default separator is `,`, and undefined or null
    elements become `""`;
  - `reverse()`;
  - `sort(cmp)`. A comparator is required, and it must be consistent;
    the result is then the same in every engine, because the sort is
    stable.
  - `map(f)`, `filter(f)`, `forEach(f)`, `reduce(f, init)` (init is
    required), `some(f)`, `every(f)`, `fill(v)` and `includes(v)`.
  - Callbacks get `(element, index, array)`, except that `reduce`'s
    callback gets `(acc, element, index, array)`.
  - The optional arguments that JavaScript defines are allowed:
    `indexOf(v, from)`, `fill(v, start, end)`, `slice()`, and calls with
    no arguments.
  - `includes` uses SameValueZero, so `[NaN].includes(NaN)` is true.

### `String`

Methods on string values:

- `charCodeAt(i)`, `charAt(i)`, `indexOf(sub, from?)` and
  `lastIndexOf(sub)`;
- `slice(start?, end?)` and `substring(start, end?)`;
- `split(sep)`, with a string separator; `""` splits into characters;
- `toUpperCase()` and `toLowerCase()`, for ASCII;
- `trim()`, `repeat(n)`, `startsWith(s, pos?)`, `endsWith(s)` and
  `includes(s)`;
- `String.fromCharCode(c...)`, for code units below 128.

### `Number` methods

- `n.toFixed(d)`, for 0 ≤ d ≤ 20 and |n| < 1e21.
- `n.toString(radix?)`, for integer n and radix 2–36.

### Constants

`NaN`, `Infinity` and `undefined`.

### Out of the subset

- the global object and `globalThis`;
- `Map`, `Set`, `JSON`, `Date`, `RegExp` and `Symbol`;
- typed arrays;
- `eval` and `Function`.

## 6. Memory

Memory is managed automatically. Programs may allocate as much as they
like.
