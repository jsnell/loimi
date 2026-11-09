# The Python subset

`python-loimi` runs single-file Python 3 programs. This document lists
what it supports, for people who want to run Python programs on it.

Everything listed behaves as in CPython 3.13, except where marked as a
**difference**. The tests in `tests/` (with expected output produced by
CPython) and the eleven pyperformance programs in `bench/python/programs`
check this.

## Running it

```sh
./target/release/loimi build examples/python/main.lm --release -o python-loimi
./python-loimi prog.py
```

| Option | What it does |
|---|---|
| `--gc-stress` | Collects garbage at every safepoint (for testing). |
| `--gc-verify` | Uses a generational heap and checks the write barriers at each collection. |
| `--disasm` | Prints the compiled bytecode. |
| `--no-supers` | Skips the superinstruction pass. |
| `--debug` | Runs the program under the debugger. Only in a debug build (see [Debugger](#debugger---cfg-debug)). |

To run the tests:

```sh
examples/python/check.sh       # all dispatch modes, GC stress, verify, --gc-check-roots
examples/python/check.sh -q    # tail-call build only, normal and GC stress
```

## Lexical structure

- Indentation-sensitive blocks, with spaces or tabs (tabs go to
  multiples of 8).
- Implicit line joining inside brackets, `\` continuation, comments, and
  `;` between simple statements.
- Integer literals: decimal, `0x`, `0o` and `0b`, with `_`, of any size.
- Float literals, with exponents. No complex literals.
- Strings:
  - `'..'`, `".."`, triple-quoted, and raw (`r".."`);
  - the usual escapes, including `\xHH` and `\uXXXX`;
  - adjacent literals concatenate;
  - no bytes literals: `b".."` is an error.
- f-strings: `{expr}`, `{expr!r}`, `!s`, `!a`, `{expr:spec}` with nested
  `{..}` inside the spec, and `{{`, `}}`. An expression inside an
  f-string may not contain the f-string's own quote character.

## Types

| Type | Notes |
|---|---|
| `None`, `bool` | |
| `int` | Arbitrary precision. Values up to 32 bits are stored inline, 64-bit values in one small box, larger ones as limbs. |
| `float` | |
| `str` | **Difference:** byte strings. Indexing is by byte, so non-ASCII text is not indexed by code point. |
| `list`, `tuple` | |
| `dict` | Insertion-ordered. |
| `set`, `frozenset` | **Difference:** iteration order is insertion order (CPython's depends on hashes). Don't print sets with more than one element directly. |
| `range`, `slice` | |
| functions, bound methods, classes, generators, modules | |

### Numbers

- **int:** `+ - * // % ** << >> & | ^ ~`, true division `/` (exact for
  big operands that divide), `divmod`, `pow` with a modulus, comparisons
  with floats, `bit_length`, `int(str, base)`, `hex`, `oct`, `bin`.
- **float:** CPython's `repr` (shortest round-trip, `1e+16`, `inf`,
  `nan`), `round(x)` (half to even), `round(x, n)`, `is_integer`.

### Methods

| Type | Methods |
|---|---|
| `str` | `join split rsplit strip lstrip rstrip startswith endswith find rfind index rindex replace upper lower title capitalize swapcase count isdigit isalpha isspace isupper islower isalnum splitlines ljust rjust center zfill partition rpartition format encode` (`encode` returns the string unchanged) |
| `list` | `append extend pop insert remove index count sort reverse copy clear` |
| `tuple` | `index count` |
| `dict` | `get keys values items pop setdefault update copy clear popitem fromkeys` |
| `set`, `frozenset` | `add remove discard pop clear copy update union intersection difference symmetric_difference issubset issuperset isdisjoint` |

Operators on containers:

- **list:** slicing with steps, slice assignment (any step), `del` of
  items and slices, `+`, `*`, and `+=` (in place).
- **tuple:** slicing, `+`, `*`, comparisons.
- **dict:** `|`. `keys()`, `values()` and `items()` are live views that
  support `len`, iteration, and `in` for keys. Keys may be of any
  hashable type, including instances with `__hash__` and `__eq__`.
- **set:** `| & - ^` and `< <= > >=`.

### String formatting

`%` formatting, `str.format` and f-strings share one format-spec
implementation.

- **`%` formatting:** `%s %r %d %i %f %e %g %x %X %o %c %%`, with flags,
  width, precision, `*` and `%(name)s`.
- **`str.format`:** positional, numbered, keyword and `name.attr` fields,
  with specs.
- **Format specs:** fill, align, sign, `#`, `0`, width, `,` or `_`,
  precision, and the types `d n x X o b c e E f F g G % s`.

## Statements

- expression statements;
- assignment: chained, tuple and list unpacking, nested, starred
  (`a, *b = ..`);
- augmented assignment to names, attributes and subscripts;
- annotated assignment (the annotation is ignored);
- `if`/`elif`/`else`, `while`/`else`, `for`/`else`, `break`, `continue`,
  `pass`;
- `def` and `class`, both with decorators;
- `return`, `global`, `nonlocal`, `del`, `assert`;
- `raise X`, `raise X(..)`, bare `raise` in an except clause, and
  `raise X from Y` (the cause is ignored);
- `try` with `except T`, `except (T1, T2) as e`, bare `except`, `else`
  and `finally`. `finally` also runs around `return`, `break` and
  `continue`;
- `with`: one or several managers, `as` targets, exceptions passed to
  `__exit__`, and suppression;
- `import m`, `import m as n`, `from m import a, b as c`;
- the walrus operator `:=`.

## Expressions

- all operators, with Python's precedence;
- chained comparisons; `and` and `or` returning an operand; `not`,
  `in`, `not in`, `is`, `is not`;
- conditional expressions and lambdas;
- calls with positional, keyword, `*args` and `**kwargs` arguments;
- attribute access, subscripts and slices;
- list, tuple, dict and set displays, with `*x` in lists, tuples and
  sets and `**d` in dicts;
- list, set and dict comprehensions and generator expressions, with
  several `for` and `if` clauses;
- `yield` and `yield from`. The value of `yield from` is the
  subgenerator's return value. **Difference:** sent values and `throw`
  are not forwarded to the subgenerator.

## Functions

- positional parameters, defaults (evaluated once), `*args`, keyword-only
  parameters, `**kwargs`;
- closures over variables of enclosing functions, `nonlocal`, `global`;
- generators: `yield`, `send`, `throw`, `close` (which runs `finally`
  blocks), and `return` values in `StopIteration`. **Difference:** a
  suspended generator that is dropped is not closed, so its `finally`
  blocks do not run.

### Limits

Past each limit, Python code gets the error shown.

| Limit | Value | Error | CPython |
|---|---|---|---|
| Python call depth | 4096 frames | `RecursionError` | 1000 frames |
| Calls through the runtime (`__repr__`, `__eq__`, `__lt__`, key functions, generators resumed from builtins) | about 180 nested | `RecursionError` | |
| The runtime's own recursion over nested values (`==`, `<`, `repr`, `hash` of lists, tuples and dicts) | 10000 levels, and the C stack size (from `RLIMIT_STACK`) | `RecursionError` | 10000 C levels |
| Source nesting (parentheses, unary operators, conditional expressions) | 2000 levels | `SyntaxError` | |
| Expression tree depth (`1 + 1 + ...` or `a or b or ...`) | 20000 | `SyntaxError` | parses `or` chains flat |

The repr of a list or dict that contains itself prints `[...]` or
`{...}`.

## Classes

- single inheritance, `class C(B)`. Several bases raise `TypeError`;
- class attributes, including ones set after the class is created;
  methods; `__init__`;
- `super()` with no arguments in methods, and `super(C, self)`;
- `@staticmethod`, `@classmethod`, and `@property` with `.setter`;
- `__slots__` (accepted and ignored);
- dynamic instance attributes;
- `__dict__` and `vars(obj)`: the live attribute dict, with any keys.
  Assigning `obj.__dict__` replaces it;
- `__class__`, `__name__`.

Special methods:

```
__init__ __repr__ __str__ __eq__ __ne__ __lt__ __le__ __gt__ __ge__
__hash__ __bool__ __len__ __getitem__ __setitem__ __delitem__
__contains__ __iter__ __next__ __call__ __getattr__ __enter__ __exit__
__neg__ __pos__ __invert__ __abs__ __int__ __float__ __index__ __format__
```

Binary operators, and their reflected `__r*__` forms:

```
__add__ __sub__ __mul__ __truediv__ __floordiv__ __mod__ __pow__
__lshift__ __rshift__ __and__ __or__ __xor__ __matmul__
```

Operator and comparison methods that return `NotImplemented` fall back to
the reflected method, then to `TypeError` (or identity for `==` and
`!=`), as in CPython.

**Differences:**

- A subclass of a builtin type other than `object` and the exception
  classes (`class C(list): pass`) creates plain instances of that builtin
  type. Methods the subclass adds are not visible on them.
- A plain-function `__add__` or `__lt__` with exactly two parameters that
  does not mention `NotImplemented` takes a fast path. If it returns a
  `NotImplemented` it got from elsewhere (say from another method), that
  becomes the result instead of triggering the fallback.
- `__eq__` is called on instances used as dict keys, but not on keys
  looked up with `in` against a list of builtin values.

## Exceptions

Exceptions are instances of a class hierarchy rooted at `BaseException`.
User classes may derive from any of these:

```
BaseException Exception ArithmeticError ZeroDivisionError OverflowError
LookupError IndexError KeyError ValueError TypeError NameError
UnboundLocalError AttributeError StopIteration RuntimeError
NotImplementedError RecursionError AssertionError GeneratorExit
KeyboardInterrupt SystemExit ImportError ModuleNotFoundError MemoryError
OSError EOFError UnicodeError
```

- `e.args`, `str(e)` and `repr(e)` work as in CPython.
- The messages of the common runtime errors match CPython's.
- An uncaught exception prints `Traceback (most recent call last):`, the
  line, and `Type: message` to stderr, and exits with status 1. There is
  no full traceback.

**Differences:**

- Reading a local before assignment is not detected: it reads whatever
  the register holds. Free variables read before assignment do raise
  `UnboundLocalError`.
- Messages name functions and classes by their plain names, not by
  CPython's qualified names: `f()` rather than `outer.<locals>.f()`, and
  "property has no setter".
- `raise X from Y` sets no `__cause__`.
- Converting an int to a string has no digit limit. (CPython raises
  `ValueError` past 4300 digits.)
- Huge results raise `OverflowError` ("exponent too large") where
  CPython runs out of memory.

## Builtins

```
print len isinstance issubclass type repr str int float bool list tuple
dict set frozenset range object property staticmethod classmethod super
slice abs min max sum sorted reversed enumerate zip map filter iter next
hash id ord chr divmod pow round any all getattr setattr hasattr delattr
callable hex oct bin format globals vars input NotImplemented
```

- `print` takes `sep`, `end` and `file`.
- `type` takes one argument.
- `min` and `max` take `key` and `default`.
- `sorted` takes `key` and `reverse`, and is stable.
- `input` raises `EOFError`.

## Modules

| Module | Contents |
|---|---|
| `math` | `sqrt sin cos tan asin acos atan atan2 exp log log10 log2 pow floor ceil fabs hypot isnan isinf isfinite radians degrees gcd factorial trunc copysign sinh cosh tanh fsum modf fmod isqrt prod`, and `pi e tau inf nan` |
| `random` | CPython's Mersenne Twister, bit for bit, so seeded programs print what CPython prints: `seed` (with ints), `random`, `randrange`, `randint`, `choice`, `shuffle`, `uniform`, `getrandbits`, `sample` |
| `time` | `time`, `perf_counter` |
| `sys` | `setrecursionlimit` (ignored), `getrecursionlimit`, `exit`, `argv` (empty), `maxsize` |
| `io` | `StringIO` with `write` and `getvalue`, written in Python |

## Not supported

- other modules;
- files and `open`;
- `async`;
- `match` statements;
- metaclasses and `__new__`;
- descriptors other than `property`, `staticmethod` and `classmethod`;
- `exec` and `eval`.

## Differential testing

`fuzz/gen.py` generates random programs in this subset and `fuzz/run.py`
compares their output with CPython's, snippet by snippet. The programs
cover comparisons, int, float and bigint arithmetic, string methods and
formatting, slicing, mutation during iteration, exceptions, generators,
closures, default arguments and classes. `check.sh -q` runs a short
fixed-seed campaign when `bench/python/install` is built.

## Debugger (`--cfg debug`)

A pdb-like debugger (`examples/python/debugger.lm`), included in a debug
build:

```sh
./target/release/loimi build examples/python/main.lm --release --cfg debug -o python-dbg
./python-dbg --debug prog.py < commands      # or interactively
```

It stops before the first line and reads commands. Locations print as
pdb prints them (`> FILE(LINE)FUNC()` and `-> source`), with the prompt
`(Pdb)`.

| Command | Short | What it does |
|---|---|---|
| `break [FILE:]LINE` | `b` | Sets a breakpoint. |
| `tbreak [FILE:]LINE` | | A breakpoint that is deleted after its first stop. |
| `delete ID` | `d` | Deletes a breakpoint. |
| `info` | `i` | Lists the breakpoints. |
| `continue` | `c` | Runs to the next stop. |
| `step` | `s` | Runs to the next line, entering calls. |
| `next` | `n` | Runs to the next line in this function. |
| `finish` | `fin` | Runs until the current function returns (gdb's `finish`; pdb's `return` stops before the return instead). |
| `bt` | `where` | Prints the call stack. |
| `locals` | `l` | Prints the locals. |
| `print NAME` | `p` | Prints a local, or else a global. |
| `op` | | Disassembles the current instruction. |
| `quit` | `q` | Ends the program. |

Behaviour:

- A breakpoint on a `for` or `while` line stops on every iteration, at
  the loop head. For a `for`, that is after its iterable is evaluated.
- `step` goes into calls, generator resumptions (`for`, `next(g)`,
  `g.send`) and functions called back from builtins (a `sorted` key).
- `next` over a `yield` stops in the code that resumed the generator, as
  pdb does.
- `next` or `finish` in a function called back from a builtin stops in
  its caller once the builtin returns.
- An exception unwinding out of the function stops `next` and `finish`
  at the `except` or `finally` handler that catches it.
- The prelude and the builtin modules written in Python are not stepped
  into and not shown in `bt`.
- `locals` shows a function's locals from their first assignment in code
  order (it does not track which paths ran), and comprehension targets
  inside the comprehension. At module level it shows the globals,
  without `__` names.
- Values print as `repr` does, except that no Python code runs:
  instances show as `<C object>`, exceptions as `E('message')`, and
  functions and generators without addresses.

**Differences from pdb:**

- `p` takes a name, not an expression.
- No `return`, `until`, `jump`, `display`, conditional breakpoints or
  `commands`.
- A line that jumps back to itself (a one-line loop) is one step.
- After a generator resumes, the stop is at the first line that runs
  after the `yield`. In `x = yield v`, the `yield` itself stores `x`, so
  that is the next statement.
