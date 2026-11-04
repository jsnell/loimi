# The Python subset

`python-loimi FILE.py` runs a single-file Python 3 program. Everything
below behaves as in CPython 3.13; the tests in `tests/` (expected output
produced by CPython) and the eleven pyperformance programs in
`bench/python/programs` check it. Where the subset differs from Python
it says so.

Build and run:

```sh
./target/release/loimi build examples/python/main.lm --release -o python-loimi
./python-loimi prog.py                 # --gc-stress, --gc-verify, --disasm, --no-supers
examples/python/check.sh               # all dispatch modes, GC stress, verify, --gc-check-roots
```

## Lexical structure

- Indentation-sensitive blocks (spaces or tabs, tabs to multiples of 8),
  implicit line joining inside brackets, `\` continuation, comments,
  `;` between simple statements.
- Integer literals: decimal, `0x`, `0o`, `0b`, with `_`; any size.
  Float literals with exponents. No complex literals.
- Strings: `'..'`, `".."`, triple-quoted, raw (`r".."`), the usual
  escapes including `\xHH` and `\uXXXX`; adjacent literals concatenate.
  No bytes literals (`b".."` is an error).
- f-strings: `{expr}`, `{expr!r}`/`!s`/`!a`, `{expr:spec}` with nested
  `{..}` inside the spec, `{{`/`}}`. (Expressions inside an f-string may
  not contain the quote character of the f-string itself.)

## Values and types

`None`, `bool`, `int` (arbitrary precision: 32-bit values inline,
64-bit values in one small box, larger ones as limbs), `float`, `str`
(byte strings; indexing is by byte, so non-ASCII text is not indexed by
code point), `list`, `tuple`, `dict` (insertion-ordered), `set`,
`frozenset`, `range`, functions, bound methods, classes, generators,
`slice`, modules.

- **int**: `+ - * // % ** << >> & | ^ ~`, true division `/` (exact for
  big operands that divide), `divmod`, `pow` with modulus, comparisons
  with floats, `bit_length`, `int(str, base)`, `hex/oct/bin`.
- **float**: CPython's `repr` (shortest round-trip, `1e+16`, `inf`,
  `nan`), `round(x)` (half to even) and `round(x, n)`, `is_integer`.
- **str** methods: `join split rsplit strip lstrip rstrip startswith
  endswith find rfind index rindex replace upper lower title capitalize
  swapcase count isdigit isalpha isspace isupper islower isalnum
  splitlines ljust rjust center zfill partition rpartition format encode`
  (the last returns the string). `%` formatting (`%s %r %d %i %f %e %g %x
  %X %o %c %%`, flags, width, precision, `*`, `%(name)s`), `str.format`
  (positional, numbered, keyword and `name.attr` fields, specs), and
  f-strings share one format-spec implementation (fill, align, sign, `#`,
  `0`, width, `,`/`_`, precision, types `d n x X o b c e E f F g G %
  s`).
- **list** methods: `append extend pop insert remove index count sort
  reverse copy clear`; slicing with steps, slice assignment (any step),
  `del` of items and slices, `+`, `*`, `+=` (in place).
- **tuple**: `index count`, slicing, `+`, `*`, comparisons.
- **dict**: `get keys values items pop setdefault update copy clear
  popitem fromkeys`, `|`, views (`keys()`, `values()`, `items()` are
  live views supporting `len`, iteration, `in` for keys), keys of any
  hashable type, including instances with `__hash__`/`__eq__`.
- **set/frozenset**: `add remove discard pop clear copy update union
  intersection difference symmetric_difference issubset issuperset
  isdisjoint`, `| & - ^`, `< <= > >=`. *Difference:* iteration order
  is insertion order (CPython's depends on hashes), so programs should not
  print sets with more than one element directly.

## Statements

Expression statements, assignment (chained, tuple/list unpacking,
nested, starred `a, *b = ..`), augmented assignment (to names,
attributes, subscripts), annotated assignment (annotation ignored),
`if/elif/else`, `while/else`, `for/else`, `break`, `continue`, `pass`,
`def` (with decorators), `class` (with decorators), `return`, `global`,
`nonlocal`, `del`, `raise` (`raise X`, `raise X(..)`, bare `raise` in an
except clause, `raise X from Y` with the cause ignored), `try` with
`except T`, `except (T1, T2) as e`, bare `except`, `else`, `finally`
(also around `return`, `break` and `continue`), `with` (one or several
managers, `as` targets, exceptions passed to `__exit__`, suppression),
`assert`, `import m`, `import m as n`, `from m import a, b as c`,
walrus `:=`.

## Expressions

All operators with Python's precedence, chained comparisons, `and`/`or`
returning operands, `not`, `in`/`not in`, `is`/`is not`, conditional
expressions, lambdas, calls with positional, keyword, `*args` and
`**kwargs` arguments, attribute access, subscripts and slices, list,
tuple, dict and set displays (with `*x` in lists, tuples and sets and `**d` in dicts), list,
set and dict comprehensions (several `for` and `if` clauses), generator
expressions, `yield`, `yield from` (its value is the subgenerator's
return value; sent values and `throw` are not forwarded to it).

## Functions

Positional parameters, defaults (evaluated once), `*args`, keyword-only
parameters, `**kwargs`; closures over enclosing function variables
(cells), `nonlocal`, `global`; generators (`yield`, `send`, `throw`,
`close` running `finally` blocks, `return` values in `StopIteration`;
*difference:* a suspended generator that is dropped is not closed, so
its `finally` blocks do not run); recursion up to the frame limit
(4096 frames; deeper raises `RecursionError`). Recursion through the
runtime (`__repr__`, `__eq__`, `__lt__`, key functions, generators
resumed from builtins) is limited to about 180 nested calls, and the
runtime's own recursion over nested values (`==`, `<`, `repr`, `hash`
of lists, tuples and dicts) to 10000 levels and to the C stack (sized
from `RLIMIT_STACK`); past them it raises `RecursionError` (CPython:
1000 frames and 10000 C levels). Source nested deeper than 2000 levels
(parentheses, unary operators, conditional expressions) or whose
expression tree is deeper than 20000 (a chain such as `1 + 1 + ...` or
`a or b or ...` with that many operators: CPython parses `or` chains
flat) is a `SyntaxError`. The repr of a list or dict that contains
itself prints `[...]` / `{...}`.

## Classes

Single inheritance (`class C(B)`; several bases raise `TypeError`),
class attributes, methods, `__init__`, `super()` (zero-argument in
methods, two-argument `super(C, self)`), `@staticmethod`,
`@classmethod`, `@property` with `.setter`, `__slots__` (accepted and
ignored), class attributes set after the class is created, dynamic
instance attributes, `__dict__` and `vars(obj)` (the live attribute
dict, any keys; assigning `obj.__dict__` replaces it),
`__class__`, `__name__`.

Special methods: `__init__ __repr__ __str__ __eq__ __ne__ __lt__ __le__
__gt__ __ge__ __hash__ __bool__ __len__ __getitem__ __setitem__
__delitem__ __contains__ __iter__ __next__ __call__ __getattr__
__enter__ __exit__ __neg__ __pos__ __invert__ __abs__ __int__ __float__
__index__ __format__`, the binary operators `__add__ __sub__ __mul__
__truediv__ __floordiv__ __mod__ __pow__ __lshift__ __rshift__ __and__
__or__ __xor__ __matmul__` and their reflected `__r*__` forms.

*Differences:* a subclass of a builtin type other than `object` and the
exception classes (`class C(list): pass`) creates plain instances of that
builtin type: methods it adds are not visible on them. Operator and
comparison methods returning `NotImplemented` fall back to the reflected
method and then to `TypeError` (or identity for `==`/`!=`) as in
CPython, with one exception: the in-loop fast path (a plain-function
`__add__` or `__lt__` taking exactly two parameters) is used for
functions that do not name `NotImplemented`, and a `NotImplemented`
they get elsewhere (say from another method) becomes the result.
`__eq__` on instances
used as dict keys is called; on keys looked up with `in` against a
list of builtin values it is not.

## Exceptions

Exceptions are instances of a class hierarchy rooted at `BaseException`:
`Exception`, `ArithmeticError`, `ZeroDivisionError`, `OverflowError`,
`LookupError`, `IndexError`, `KeyError`, `ValueError`, `TypeError`,
`NameError`, `UnboundLocalError`, `AttributeError`, `StopIteration`,
`RuntimeError`, `NotImplementedError`, `RecursionError`,
`AssertionError`, `GeneratorExit`, `KeyboardInterrupt`, `SystemExit`,
`ImportError`, `ModuleNotFoundError`, `MemoryError`, `OSError`,
`EOFError`, `UnicodeError`; user classes may derive from any of them.
`e.args`, `str(e)` and `repr(e)` as in CPython; the messages of the
common runtime errors match CPython's. An uncaught exception prints
`Traceback (most recent call last):`, the line, and `Type: message` to
stderr and exits with status 1 (no full traceback). *Differences:*
reading a local before assignment is not detected (it reads whatever the
register holds); free variables before assignment raise
`UnboundLocalError`; messages name functions and classes by their plain
names, not CPython's qualified names (`f()` rather than
`outer.<locals>.f()`, "property has no setter"); `raise X from Y` sets
no `__cause__`; int-to-str conversion has no digit limit (CPython
raises `ValueError` past 4300 digits) and huge results raise
`OverflowError` ("exponent too large") where CPython runs out of memory.

*Differential testing:* `fuzz/gen.py` generates random programs in this
subset (comparisons and rich comparisons, int/float/bigint arithmetic,
string methods and formatting, slicing, mutation during iteration,
exceptions, generators, closures, default arguments, classes) and
`fuzz/run.py` compares their output with CPython's, snippet by snippet;
`check.sh -q` runs a short fixed-seed campaign when
`bench/python/install` is built.

## Builtins

`print` (`sep`, `end`, `file`), `len`, `isinstance`, `issubclass`,
`type` (one argument), `repr`, `str`, `int`, `float`, `bool`, `list`,
`tuple`, `dict`, `set`, `frozenset`, `range`, `object`, `property`,
`staticmethod`, `classmethod`, `super`, `slice`, `abs`, `min`, `max`
(`key`, `default`), `sum`, `sorted` (`key`, `reverse`; stable),
`reversed`, `enumerate`, `zip`, `map`, `filter`, `iter`, `next`, `hash`,
`id`, `ord`, `chr`, `divmod`, `pow`, `round`, `any`, `all`, `getattr`,
`setattr`, `hasattr`, `delattr`, `callable`, `hex`, `oct`, `bin`,
`format`, `globals`, `vars`, `input` (raises EOFError),
`NotImplemented`.

## Modules

- `math`: `sqrt sin cos tan asin acos atan atan2 exp log log10 log2 pow
  floor ceil fabs hypot isnan isinf isfinite radians degrees gcd
  factorial trunc copysign sinh cosh tanh fsum modf fmod isqrt prod`,
  `pi e tau inf nan`.
- `random`: CPython's Mersenne Twister, bit for bit (`seed` with ints,
  `random`, `randrange`, `randint`, `choice`, `shuffle`, `uniform`,
  `getrandbits`, `sample`), so seeded programs print what CPython prints.
- `time`: `time`, `perf_counter`. `sys`: `setrecursionlimit` (ignored),
  `getrecursionlimit`, `exit`, `argv` (empty), `maxsize`.
- `io`: `StringIO` (`write`, `getvalue`), written in Python.

No other modules, no files, no `open`, no `async`, no `match`
statements, no metaclasses, no `__new__`, no descriptors other than
`property`, `staticmethod` and `classmethod`, no `exec`/`eval`.

## Debugger (`--cfg debug`)

```sh
./target/release/loimi build examples/python/main.lm --release --cfg debug -o python-dbg
./python-dbg --debug prog.py < commands      # or interactively
```

A pdb-like debugger (`examples/python/debugger.lm`, on the generic
pieces of `docs/design/debugging.md`). It stops before the first line
and reads commands: `break [FILE:]LINE` (`b`), `tbreak`, `delete ID`,
`info`, `continue` (`c`), `step` (`s`), `next` (`n`), `finish` (out of
the current function, gdb's; pdb's `return` stops before the return
instead), `bt` (`where`), `locals`, `print NAME` (`p`: a local, else a
global), `op` (the current instruction, disassembled), `quit`. Locations
print as pdb's (`> FILE(LINE)FUNC()` and `-> source`, prompt `(Pdb)`).

- A breakpoint on a `for` or `while` line stops on every iteration (at
  the loop head: for a `for`, after its iterable is evaluated).
- `step` goes into calls, generator resumptions (`for`, `next(g)`,
  `g.send`) and functions called back from builtins (a `sorted` key);
  `next` over a `yield` stops in the code that resumed the generator, as
  pdb does; `next` or `finish` in a function called back from a builtin
  stops in its caller once the builtin returns. An exception unwinding
  out of the function stops `next` and `finish` at the handler (`except`
  or `finally`) that catches it.
- The prelude and builtin modules written in Python are not stepped
  into and not shown in `bt`.
- `locals` shows a function's locals from their first assignment in
  code order (a local is not tracked through the paths that ran), and
  comprehension targets inside the comprehension; at module level, the
  globals (without `__` names). Values print as `repr` does, except that
  no Python code runs: instances show as `<C object>` (exceptions as
  `E('message')`), functions and generators without addresses.

*Differences from pdb:* no expressions (`p` takes a name), no `return`,
`until`, `jump`, `display`, conditions or `commands`; a line that jumps
back to itself (a one-line loop) is one step; a stop after a generator
resumes is at the first line that runs after the `yield` (in `x = yield
v`, the `yield` itself stores `x`, so that is the next statement).
