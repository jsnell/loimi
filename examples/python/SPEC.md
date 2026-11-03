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
tuple, dict and set displays (with `*x` in lists, tuples and sets), list,
set and dict comprehensions (several `for` and `if` clauses), generator
expressions, `yield`, `yield from` (iteration only: sent values are not
forwarded).

## Functions

Positional parameters, defaults (evaluated once), `*args`, keyword-only
parameters, `**kwargs`; closures over enclosing function variables
(cells), `nonlocal`, `global`; generators (`yield`, `send`, `close`,
`return` ending the iteration); recursion up to the frame limit
(4096 frames; deeper raises `RecursionError`).

## Classes

Single inheritance (`class C(B)`; several bases raise `TypeError`),
class attributes, methods, `__init__`, `super()` (zero-argument in
methods, two-argument `super(C, self)`), `@staticmethod`,
`@classmethod`, `@property` with `.setter`, `__slots__` (accepted and
ignored), class attributes set after the class is created, dynamic
instance attributes, `__dict__`, `__class__`, `__name__`.

Special methods: `__init__ __repr__ __str__ __eq__ __ne__ __lt__ __le__
__gt__ __ge__ __hash__ __bool__ __len__ __getitem__ __setitem__
__delitem__ __contains__ __iter__ __next__ __call__ __getattr__
__enter__ __exit__ __neg__ __pos__ __invert__ __abs__ __int__ __float__
__index__ __format__`, the binary operators `__add__ __sub__ __mul__
__truediv__ __floordiv__ __mod__ __pow__ __lshift__ __rshift__ __and__
__or__ __xor__ __matmul__` and their reflected `__r*__` forms.

*Differences:* a subclass of a builtin type other than `object` and the
exception classes (`class C(list): pass`) creates plain instances of that
builtin type: methods it adds are not visible on them. Operator methods
that return `NotImplemented` are honoured on the general path but not on
the in-loop fast path (a plain-function `__add__` taking exactly two
parameters), where `NotImplemented` is the result. `__eq__` on instances
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
stderr and exits with status 1 (no full traceback). *Difference:*
reading a local before assignment is not detected (it reads whatever the
register holds); free variables before assignment raise
`UnboundLocalError`.

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
