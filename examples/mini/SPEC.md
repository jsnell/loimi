# The Mini language

Mini is a subset of Lua 5.4. Every valid Mini program is a valid Lua 5.4
program, and running it under Lua 5.4 produces the same output, except
where this document says otherwise. This document defines the subset. For
anything it does not say, the Lua 5.4 reference manual applies.

A program that uses a feature outside the subset is not a Mini program.
An implementation may reject it, or it may run it with different
behaviour.

## 1. Lexical structure

- **Comments.** `--` starts a comment that runs to the end of the line.
  `--[[ ... ]]` is a long comment, and `--[==[ ... ]==]` with any number
  of `=` also works.
- **Names.** Letters, digits and `_`, not starting with a digit.
- **Keywords.** `and break do else elseif end false for function if in
  local nil not or repeat return then true until while`.
  `goto` is not part of Mini.
- **Numbers:**
  - Decimal integers: `42`.
  - Hexadecimal integers: `0xff`.
  - Decimal floats: `3.0`, `.5`, `1e10`, `2.5e-3`.
  - A numeral with a fraction or an exponent is a float. Otherwise it is
    an integer, and must fit the integer range (§2.1).
- **Strings:**
  - Delimited by `"` or `'`.
  - Escapes: `\n \t \r \\ \" \' \0`, `\ddd` (decimal, up to 3 digits),
    and `\` followed by a newline.
  - Long brackets (`[[ ... ]]`) are **not** supported.
- **Operators and punctuation.** `+ - * / % ^ # == ~= <= >= < > = ( ) { }
  [ ] ; : , . .. `.
  Not in Mini: `//`, the bitwise operators `& | ~ << >>`, and `...`.

## 2. Values and types

The types are nil, boolean, number, string, table, function and thread
(a coroutine, §5).

### 2.1 Numbers

A number is either an **integer** or a **float** (an IEEE-754 double),
as in Lua 5.4.

- **Integer range.** Mini integers are 32-bit: a program must keep every
  integer value, including intermediate results, within
  [-2^31, 2^31 - 1]. If an integer operation overflows, the result is
  unspecified. (Lua 5.4 wraps around at 64 bits.)
- **Floats** may hold any value. Integral floats such as `2^40` are fine.
- **Arithmetic.** For `+ - * %` and unary `-`, two integer operands give an
  integer and any float operand gives a float. `/` and `^` always give
  floats.
- **`%`** is floored modulo: `a - floor(a/b)*b`. An integer `% 0` is an
  error.
- **Comparison and equality.** Integers and floats compare by
  mathematical value, so `1 == 1.0` is true.
- **Strings are not numbers.** Arithmetic on a string is an error in
  Mini; Lua converts the string instead. Programs must not use strings in
  arithmetic.

### 2.2 Strings

- A string is an immutable sequence of bytes.
- `#s` is the length in bytes.
- `==` compares contents, and `<`, `<=`, `>`, `>=` compare bytewise.
- `..` concatenates. Each operand must be a string or a number; numbers
  are converted as by `tostring`.

### 2.3 Tables

- **Keys** may be any value except nil and NaN. A float key with an
  integral value is the same key as the matching integer.
- **Missing keys.** Reading a missing key gives nil. Assigning nil
  removes the key.
- **`#t`** returns a *border* of `t`, as defined by Lua 5.4. When the
  positive integer keys of `t` are exactly 1..n, `#t` is n.
- **Metatables do not exist.** `setmetatable`, `getmetatable` and all
  metamethods are not part of Mini.

### 2.4 Functions

- Functions are first-class closures with lexical scoping.
- A closure captures variables, not values. Closures created in the same
  scope share their captured variables. Each iteration of a loop creates
  fresh loop-body locals, as in Lua.

### 2.5 Truthiness and equality

- Only `nil` and `false` are false.
- Tables and functions are equal only if they are the same object.

## 3. Expressions

- **Primary expressions:**
  - `nil`, `true`, `false`, numerals and string literals;
  - names, `e.name` and `e[e]`;
  - function calls, function literals and table constructors;
  - parenthesized expressions.
- **Operators**, with Lua's precedence from lowest to highest:
  - `or`
  - `and`
  - `<  >  <=  >=  ~=  ==`
  - `..` (right associative)
  - `+  -`
  - `*  /  %`
  - unary `not  #  -`
  - `^` (right associative)
- **`and` and `or`** short-circuit and return one of their operands, as
  in Lua.
- **Ordering errors.** Comparing values of different types with
  `< <= > >=` is an error, unless both are numbers. So is ordering
  anything other than numbers and strings.
- **Table constructors.**
  - Field forms: `{}`, `{e1, e2, ...}`, `{name = e}` and `{[k] = e}`.
    The forms may be mixed, separated by `,` or `;`, with an optional
    trailing separator.
  - Positional fields are numbered from 1.
- **Calls.**
  - Calls take the forms `f(args)`, `f{...}`, `f"str"` and `obj:m(args)`.
  - `obj:m(args)` means `obj.m(obj, args)`, with `obj` evaluated once.
  - Missing arguments are nil, and extra arguments are discarded.

### 3.1 Multiple results

- **Keeping several results.** A function may return several values:
  `return a, b, c`. All of them are kept only when the call is the
  **last expression of the expression list in a `local` statement or an
  assignment**, as in `local q, r = divmod(a, b)` or `x, y = f()`.
- **Everywhere else** a call is adjusted to exactly one value: its first
  result, or nil if it returned none. That includes argument lists, table
  constructors, `return f()` and parentheses.
  - Lua would pass along all of the results in some of those positions.
    Programs must not depend on the difference. For example, write
    `local a, b = f() print(a, b)` instead of `print(f())`.
- **Uneven lists.** In multiple assignment, extra values are discarded and
  missing values are nil, as in Lua. All expressions are evaluated before
  any assignment happens.

### 3.2 Evaluation order

Function arguments, table constructor fields and operands of binary
operators are evaluated left to right. (Lua leaves this unspecified;
Mini programs should not depend on it.)

### 3.3 Numerals and negative literals

- Numerals may use `1e+2`, `1E2`, `0X1F` and uppercase hex digits.
- `-2147483648` is unary minus applied to a numeral that is out of
  range, so it is a float. Write `-2147483647 - 1` for the minimum
  integer.

## 4. Statements

```
local n1, n2, ... [= e1, e2, ...]
v1, v2, ... = e1, e2, ...          -- v is a name, e.name or e[e]
f(args)                            -- call statement
do block end
while e do block end
repeat block until e               -- the condition can see the block's locals
if e then block {elseif e then block} [else block] end
for v = e1, e2 [, e3] do block end
for k, v in pairs(e) do block end
for i, v in ipairs(e) do block end
function name(params) block end
function name.field.field(params) block end
function name:method(params) block end
local function name(params) block end
return [e1, e2, ...]               -- must be the last statement in its block
break                              -- must be the last statement in its block
```

- **Names and scope.**
  - Assigning to a name that is not a visible local sets a global, and
    reading an unset global gives nil.
  - Locals are visible from the statement after their declaration. For
    `local function`, the name is also visible inside the function's own
    body.
- **Numeric `for`.**
  - Same semantics as Lua 5.4. The loop variable is a fresh local in each
    iteration, and assigning to it does not affect the loop.
  - If both the start and the step are integers, the loop is an integer
    loop. Otherwise all three values are converted to floats.
  - A step of 0 is an error.
- **Generic `for`.** Only these two forms exist: a call to the global
  `pairs` or `ipairs`, written directly in the loop header.
  - `ipairs(t)` visits `t[1], t[2], ...` up to the first nil.
  - `pairs(t)` visits every key/value pair once, in an unspecified order.
    Keys set as integral floats come back as integers.
  - During traversal a program may assign to existing fields, including
    assigning nil. It must not add new keys.
  - With one variable, `for k in pairs(t)`, the value is dropped.
- **`;`** is an empty statement and may appear anywhere a statement can.
- **Method declarations.** `function a.b:m()` is allowed, as are the
  call forms `obj:m{...}` and `obj:m"str"`.

## 5. Standard library

Only the following globals exist. Every other Lua library function is not
part of Mini.

**Basic functions:**
- `print(...)`: writes its arguments converted with `tostring`, separated
  by tabs and followed by a newline.
- `tostring(v)`:
  - An integer becomes its decimal form.
  - A float is formatted with `%.14g`. If the result looks like an
    integer, `.0` is appended, so `3.0` gives `"3.0"`, `1e15` gives
    `"1e+15"` and `0.1` gives `"0.1"`. Infinities become `"inf"` and
    `"-inf"`.
  - Tables and functions become an unspecified string that starts with
    `"table: "` or `"function: "`.
- `tonumber(v)`:
  - A number is returned unchanged.
  - A string holding a decimal or hex integer numeral, or a decimal float
    numeral, is converted. An optional leading `-` or `+` and surrounding
    whitespace (space, tab, newline, CR) are allowed. An integer numeral
    outside the integer range gives a float.
  - Anything else gives nil.
- `type(v)`: the type name as a string.
- `pairs(t)` and `ipairs(t)`: only in generic `for` (§4).
- `assert(v [, msg])`: an error if `v` is false or nil. Otherwise it
  returns `v`.
- `error(msg)`: raises an error. `msg` should be a string.

**`math`:**
- Functions: `math.floor(x)`, `math.ceil(x)`, `math.abs(x)`,
  `math.sqrt(x)`, `math.sin(x)`, `math.cos(x)`, `math.exp(x)`,
  `math.log(x)` (natural log, one argument), `math.max(x, ...)` and
  `math.min(x, ...)`.
- Constants: `math.huge` and `math.pi`.
- Results:
  - `floor` and `ceil` return an integer when the result fits the
    integer range, as in Lua 5.4.
  - `abs`, `max` and `min` keep the integer or float subtype of their
    result. On ties, `max` and `min` return the first argument.
  - The others return floats.

**`string`:** functions are called as `string.f(s, ...)`. Method syntax on
strings, such as `s:sub(1, 2)`, is not part of Mini.
- `string.len(s)`.
- `string.sub(s, i [, j])`: negative indices count from the end, as in Lua.
- `string.byte(s [, i])`: one result only. The default for `i` is 1.
- `string.char(...)`.
- `string.rep(s, n)`.
- `string.format(fmt, ...)`. A negative argument to `%x` is out of the
  subset:
  - Directives: `%d`, `%s`, `%f`, `%g`, `%x`, `%c` and `%%`.
  - Optional flag `-`, optional width, optional `.precision`.
  - `%d`, `%x` and `%c` need integer arguments, or floats with an
    integral value. `%s` converts its argument with `tostring`.

**`table`:**
- `table.insert(t, v)`: appends at `#t + 1`. The 3-argument form is not
  in Mini.
- `table.remove(t)`: removes and returns `t[#t]`. The 2-argument form is
  not in Mini.
- `table.concat(t [, sep])`: elements 1..#t, each a string or number.
- `table.sort(t [, comp])`: sorts elements 1..#t in place, with
  `comp(a, b)` (true when `a` must come before `b`) or `<`. As in Lua
  5.4, the sort is not stable, `comp` must define a strict order (an
  inconsistent one may raise "invalid order function for sorting"), and
  errors raised by `comp` propagate unchanged. `comp` must be nil or a
  function (checked when `#t > 1`). Mini uses Lua 5.4's algorithm, so
  `comp` is called with the same arguments in the same order as in Lua
  for arrays of up to 100 elements (Lua randomizes some pivots in larger
  ones). Calling Mini functions from `comp` nests the interpreter: more
  than 200 nested calls through natives fail with "C stack overflow".

**`coroutine`:** as in Lua 5.4: `coroutine.create(f)`, `resume(co, ...)`,
`yield(...)`, `status(co)`, `isyieldable([co])`, `running()`, `wrap(f)`
and `close(co)`. Values pass both ways through resume and yield, an error
inside a coroutine makes `resume` return false and the message (and the
coroutine dead), and a `wrap` function raises it. Yield works from any
depth of Mini calls inside the coroutine.
- `resume` returns `true` followed by the values, so keep them with
  `local ok, a, b = coroutine.resume(co)` (§3.1).
- Yielding while a library function is between the yield and the
  coroutine, as from inside a `table.sort` comparator, is an error
  ("attempt to yield across a C-call boundary"), as in Lua for C
  functions without continuations. A coroutine may still resume others
  from a comparator.
- More than 200 coroutines resuming each other at once (nested resumes,
  together with nested comparator calls) fail with "C stack overflow".
- `tostring(co)` starts with `"thread: "`. Error messages are unspecified
  (Mini prefixes `line N: `; Lua's position looks different).

**`io`:**
- `io.write(...)`: writes strings and numbers, with nothing added
  between or after the arguments. Integers are written in decimal, and
  floats with `%.14g` and **no** `.0` suffix, as in Lua 5.4: `io.write(1.0)`
  writes `1`.

- `table.remove` on an empty table returns nil.

## 6. Errors

A runtime error stops the program. It writes a message to standard error
and exits with a nonzero status. Mini has no `pcall`, so errors cannot be
caught. The message text is unspecified.

Recursion at least 10000 calls deep must work. Deeper recursion may raise
a "stack overflow" error.

## 7. Memory

Memory is managed automatically. Programs may allocate as much as they
like, and unreachable objects are reclaimed.
