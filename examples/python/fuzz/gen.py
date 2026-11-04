"""Random program generator for differential testing of python-loimi
against CPython (examples/python/fuzz/run.py drives it).

Each program is a sequence of independent snippets from the subset in
SPEC.md, chosen for semantics-heavy areas: comparisons and rich-comparison
methods, int/float/bigint arithmetic, string methods and formatting,
slicing, mutation during iteration, exceptions, generators, closures and
default arguments. Every snippet runs under `_run`, which prints its
result or the exception's type and message, so one mismatch does not hide
the others. Programs are deterministic: no sets with several elements
printed, no hash() of strings, no ids.

    python3 gen.py SEED            # print the program for SEED
"""
import random
import sys

PRELUDE = '''\
def _run(name, f):
    try:
        r = f()
        print(name, repr(r))
    except Exception as e:
        print(name, "EXC", type(e).__name__, "|", e)
'''

INTS = ["0", "1", "-1", "2", "-2", "3", "7", "-7", "10", "255", "-256", "2**31", "-2**31", "2**47",
        "2**48 - 1", "-2**47", "2**63", "-2**63", "2**64 + 3", "10**20", "-10**20", "3**50", "True", "False"]
FLOATS = ["0.0", "-0.0", "0.5", "-1.5", "2.0", "1e300", "-1e-300", "3.141592653589793", "1e16", "1.1",
          "float('inf')", "-float('inf')", "123456789.125", "2.5", "-2.5", "1/3"]
STRS = ["''", "'a'", "'abc'", "'Hello World'", "'  pad  '", "'a,b,,c'", "'x\\ty'", "'line1\\nline2'",
        "'ABCdef'", "'123'", "'-42'", "'aaa'", "'Ab Cd-eF'", "\"it's\"", "'%s'"]

ARITH = ["+", "-", "*", "/", "//", "%", "**", "<<", ">>", "&", "|", "^"]
CMPS = ["<", "<=", ">", ">=", "==", "!="]


class G:
    def __init__(self, seed):
        self.r = random.Random(seed)
        self.n = 0
        self.out = []

    def name(self):
        self.n += 1
        return "s%d" % self.n

    def pick(self, xs):
        return self.r.choice(xs)

    def emit(self, code):
        self.out.append(code)

    def run(self, expr, pre=""):
        """A snippet evaluating `expr` (with statements `pre` in a function)."""
        nm = self.name()
        if pre:
            body = "\n".join("    " + l for l in pre.split("\n"))
            self.emit("def _%s():\n%s\n    return %s\n_run(%r, _%s)" % (nm, body, expr, nm, nm))
        else:
            self.emit("_run(%r, lambda: %s)" % (nm, expr))

    # ── numbers ─────────────────────────────────────────────────────

    def num(self, depth=0):
        k = self.r.random()
        if depth > 2 or k < 0.45:
            return self.pick(INTS + FLOATS if self.r.random() < 0.35 else INTS)
        if k < 0.85:
            op = self.pick(ARITH)
            a, b = self.num(depth + 1), self.num(depth + 1)
            if op == "**":
                # (small exponents: CPython limits int->str to 4300 digits)
                b = self.pick(["0", "1", "2", "3", "-1", "-2", "0.5", "-3"])
                if not a.lstrip("-").isdigit():
                    a = self.pick(["2", "3", "-2", "10", "0", "1.5", "-0.5"])
            if op in ("<<", ">>"):
                b = self.pick(["0", "1", "3", "31", "47", "64", "100", "-1"])
            return "(%s %s %s)" % (a, op, b)
        u = self.pick(["-", "~", "+", "abs", "int", "float", "round", "divmod", "pow", "bool"])
        a = self.num(depth + 1)
        if u in ("-", "~", "+"):
            return "(%s%s)" % (u, a)
        if u == "divmod":
            return "divmod(%s, %s)" % (a, self.num(depth + 1))
        if u == "pow":
            return "pow(%s, %s, %s)" % (a, self.pick(["2", "3", "10", "-1", "0"]), self.pick(["7", "-7", "1", "97", "2**61-1"]))
        if u == "round":
            return "round(%s%s)" % (a, self.pick(["", ", 0", ", 1", ", 2", ", -1"]))
        return "%s(%s)" % (u, a)

    def g_arith(self):
        self.run(self.num())

    def g_intfmt(self):
        x = self.num(2)
        f = self.pick(["hex", "oct", "bin", "str", "repr"])
        self.run("%s(int(%s))" % (f, x))

    def g_intparse(self):
        s = self.pick(["'42'", "' -17 '", "'0x1f'", "'1_000'", "'ff'", "'z'", "'10'", "'-0'", "'12345678901234567890'", "'1.5'", "''"])
        base = self.pick(["", ", 16", ", 0", ", 2", ", 36", ", 8"])
        self.run("int(%s%s)" % (s, base))

    def g_floatops(self):
        x = self.pick(FLOATS + ["7", "-7", "2**53 + 1", "10**30"])
        y = self.pick(FLOATS + ["3", "-3", "0"])
        e = self.pick(["%s // %s", "%s %% %s", "divmod(%s, %s)", "%s / %s", "%s ** %s", "%s == %s", "%s < %s",
                       "round(%s / %s, 3)", "float(%s) .is_integer() and %s"])
        self.run(e % (x, y))

    # ── comparisons and rich comparison methods ─────────────────────

    def cmp_class(self, name, base="object"):
        lines = ["class %s(%s):" % (name, base), "    def __init__(self, v):", "        self.v = v",
                 "    def __repr__(self):", "        return '%s(%%r)' %% (self.v,)" % name]
        for m in self.r.sample(["__lt__", "__le__", "__gt__", "__ge__", "__eq__", "__ne__"], self.r.randint(0, 4)):
            kind = self.r.random()
            lines.append("    def %s(self, o):" % m)
            lines.append("        print('%s.%s', self, o)" % (name, m))
            if kind < 0.3:
                lines.append("        return NotImplemented")
            elif kind < 0.7:
                op = {"__lt__": "<", "__le__": "<=", "__gt__": ">", "__ge__": ">=", "__eq__": "==", "__ne__": "!="}[m]
                lines.append("        if not isinstance(o, %s):" % name)
                lines.append("            return NotImplemented")
                lines.append("        return self.v %s o.v" % op)
            elif kind < 0.85:
                lines.append("        return %s" % self.pick(["True", "False", "1", "0", "'s'", "[]", "None"]))
            else:
                lines.append("        return self.v %s o" % self.pick(CMPS))
        if self.r.random() < 0.3:
            lines.append("    __hash__ = None" if self.r.random() < 0.5 else "    def __hash__(self):\n        return hash(self.v)")
        return "\n".join(lines)

    def g_richcmp(self):
        a, b = "C%d" % self.n, "D%d" % self.n
        self.n += 1
        self.emit(self.cmp_class(a))
        self.emit(self.cmp_class(b, a if self.r.random() < 0.5 else "object"))
        vals = ["%s(1)" % a, "%s(2)" % a, "%s(1)" % b, "1", "'x'", "None", "[%s(1)]" % a, "(%s(2), 1)" % b]
        for _ in range(self.r.randint(2, 6)):
            x, y = self.pick(vals), self.pick(vals)
            e = self.pick(["%s %s %s" % (x, self.pick(CMPS), y), "[%s] %s [%s]" % (x, self.pick(CMPS), y),
                           "(%s, 0) %s (%s, 1)" % (x, self.pick(CMPS), y), "%s in [%s]" % (x, y),
                           "[%s, %s].count(%s)" % (x, y, x), "%s < %s < %s" % (x, y, x),
                           "not (%s %s %s)" % (x, self.pick(CMPS), y),
                           "max(%s, %s)" % (x, y), "min([%s, %s])" % (x, y), "{1: %s} == {1: %s}" % (x, y)])
            self.run(e)

    def builtin_val(self, depth=0):
        k = self.r.random()
        if depth > 1 or k < 0.5:
            return self.pick(INTS[:12] + FLOATS[:6] + STRS[:6] + ["None", "True"])
        if k < 0.7:
            return "[%s]" % ", ".join(self.builtin_val(depth + 1) for _ in range(self.r.randint(0, 3)))
        if k < 0.9:
            n = self.r.randint(0, 3)
            return "(%s)" % ", ".join([self.builtin_val(depth + 1) for _ in range(n)] + ([""] if n == 1 else []))
        return "{%s}" % ", ".join("%s: %s" % (self.pick(["1", "'a'", "2", "(1, 2)"]), self.builtin_val(depth + 1)) for _ in range(self.r.randint(0, 2)))

    def g_cmp_builtin(self):
        x, y = self.builtin_val(), self.builtin_val()
        self.run("%s %s %s" % (x, self.pick(CMPS), y))

    # ── strings ─────────────────────────────────────────────────────

    def g_strmeth(self):
        s = self.pick(STRS)
        m = self.pick([
            "split()", "split(',')", "split(',', 1)", "rsplit(',', 1)", "split('')", "strip()", "strip('a ')", "lstrip()",
            "rstrip('d')", "startswith('a')", "startswith(('x', 'H'))", "endswith('c', 0, 2)", "find('b')", "find('b', 2)",
            "rfind('a')", "index('z')", "rindex('a')", "replace('a', 'XY')", "replace('a', '', 1)", "replace('', '-')",
            "upper()", "lower()", "title()", "capitalize()", "swapcase()", "count('a')", "count('')", "isdigit()",
            "isalpha()", "isspace()", "isupper()", "islower()", "isalnum()", "splitlines()", "splitlines(True)",
            "ljust(8, '*')", "rjust(6)", "center(9, '-')", "center(4)", "zfill(6)", "partition(',')", "rpartition('a')",
            "join(['1', '2'])", "join('xyz')", "join([1])", "split(None, 1)", "rsplit()", "find('')", "index('a', 5)"])
        self.run("%s.%s" % (s, m))

    def g_strops(self):
        s = self.pick(STRS)
        e = self.pick(["%s * 3", "%s * -1", "%s + %s", "%s[::-1]", "len(%s)", "%s in %s", "%s < %s", "%s[1]",
                       "%s[-1]", "%s[5]", "%s + 1", "%s * 'a'", "list(%s)", "str(%s)", "repr(%s)", "ord(%s[0])",
                       "%s.split(%s)", "sorted(%s)", "max(%s)", "tuple(%s)[:2]"])
        n = e.count("%s")
        self.run(e % tuple(self.pick(STRS) if i else s for i in range(n)))

    def spec(self, kind):
        fill = self.pick(["", "", "*", "0", " "])
        align = self.pick(["", "", "<", ">", "^", "="])
        if fill and not align:
            fill = ""
        sign = self.pick(["", "", "+", "-", " "])
        alt = self.pick(["", "", "#"])
        zero = self.pick(["", "", "0"])
        width = self.pick(["", "", "1", "5", "10", "12"])
        group = self.pick(["", "", ",", "_"])
        prec = self.pick(["", "", ".0", ".1", ".3", ".10"])
        if kind == "s":
            t = self.pick(["", "s"])
            return fill + align + width + self.pick(["", ".2"]) + t
        if kind == "i":
            t = self.pick(["", "d", "x", "X", "o", "b", "e", "f", "g", "%", "n", "c", "E"])
            if t in "dxXobnc":
                prec = ""
            if t in ("n", "c"):
                group = ""
            return fill + align + sign + alt + zero + width + group + prec + t
        t = self.pick(["", "e", "E", "f", "F", "g", "G", "%"])
        return fill + align + sign + alt + zero + width + group + prec + t

    def g_format(self):
        kind = self.pick(["s", "i", "f"])
        v = self.pick(STRS[:6]) if kind == "s" else self.pick(INTS[:16]) if kind == "i" else self.pick(FLOATS)
        sp = self.spec(kind)
        if sp.endswith("c"):
            v = self.pick(["65", "10", "0", "97", "True"])  # strings are bytes: no non-ASCII chr (SPEC.md)
        q = "'" if '"' in v else '"'  # an f-string's expressions may not contain its quote (SPEC.md)
        e = self.pick(["format(%s, %r)" % (v, sp), "f%s{%s:%s}%s" % (q, v, sp, q), "'{:%s}'.format(%s)" % (sp, v),
                       "'{0!r:>8}|{0}'.format(%s)" % v, "f%s{%s!r}%s" % (q, v, q)])
        self.run(e)

    def g_percent(self):
        conv = self.pick(["s", "r", "d", "i", "f", "e", "g", "x", "X", "o", "c", "%", "E", "G"])
        flags = self.pick(["", "", "-", "+", " ", "0", "#", "-0"])
        width = self.pick(["", "", "5", "10", "*"])
        prec = self.pick(["", "", ".2", ".0", ".*"])
        v = self.pick(INTS[:12] + FLOATS[:8] + STRS[:4] + ["None", "[1, 2]", "(1,)"])
        if conv == "c" and v in ("255", "-256", "2**31", "-2**31", "2**47", "-1", "-2", "-7"):
            v = self.pick(["65", "97", "'q'"])  # strings are bytes: no non-ASCII chr (SPEC.md)
        args = []
        if width == "*":
            args.append(self.pick(["3", "-6"]))
        if prec == ".*":
            args.append(self.pick(["1", "4"]))
        if conv != "%":
            args.append(v)
        fmt = "%" + flags + width + prec + conv
        if self.r.random() < 0.2:
            self.run("'%%(k)%s|' %% {'k': %s}" % (conv if conv != "%" else "s", v))
        elif len(args) == 1 and self.r.random() < 0.5:
            self.run("'<%s>' %% %s" % (fmt, args[0]))
        else:
            self.run("'<%s>' %% (%s)" % (fmt, ", ".join(args) + ("," if len(args) == 1 else "")))

    # ── slicing ─────────────────────────────────────────────────────

    def idx(self):
        return self.pick(["", "None", "0", "1", "2", "-1", "-2", "-5", "3", "5", "100", "-100"])

    def g_slice(self):
        seq = self.pick(["[0, 1, 2, 3, 4, 5]", "(10, 20, 30)", "'abcdefg'", "range(10)", "range(2, 20, 3)", "[]", "range(5, 0, -1)"])
        st = self.pick(["", "", ":1", ":2", ":-1", ":-2", ":3", ":0", ":None", ":-3"])
        e = "%s[%s:%s%s]" % (seq, self.idx(), self.idx(), st)
        if seq.startswith("range"):
            e = "list(%s)" % e
        if self.r.random() < 0.15:
            e = "%s[%s]" % (seq, self.pick(["0", "-1", "6", "-7", "True"]))
        self.run(e)

    def g_slice_assign(self):
        st = self.pick(["", "", ":2", ":-1", ":-2", ":3"])
        rhs = self.pick(["[]", "[9]", "[7, 8]", "[1, 2, 3]", "'xy'", "range(3)", "(5,)", "[9, 9, 9, 9, 9, 9]"])
        k = self.r.random()
        if k < 0.45:
            stmt = "a[%s:%s%s] = %s" % (self.idx(), self.idx(), st, rhs)
        elif k < 0.7:
            stmt = "del a[%s:%s%s]" % (self.idx(), self.idx(), st)
        elif k < 0.85:
            stmt = "a[%s] = 'v'" % self.pick(["0", "-1", "5", "-7", "10"])
        else:
            stmt = "del a[%s]" % self.pick(["0", "-1", "5", "-7", "10"])
        self.run("a", "a = [0, 1, 2, 3, 4, 5]\n" + stmt)

    # ── mutation during iteration ───────────────────────────────────

    def g_mutate(self):
        body = self.pick([
            "d = {1: 1, 2: 2, 3: 3}\nfor k in d:\n    d[k + 10] = 0\n",
            "d = {1: 1, 2: 2}\nfor k in d:\n    del d[k]\n",
            "d = {1: 1, 2: 2}\nfor k in d:\n    d[k] = k * 5\n",
            "d = {1: 1, 2: 2, 3: 3}\nfor k in d.keys():\n    d.pop(k)\n",
            "d = {1: 1, 2: 2}\nfor k, v in d.items():\n    d[3] = 3\n",
            "d = {1: 1, 2: 2}\nfor v in d.values():\n    d.clear()\n",
            "d = {1: 1}\nfor k in d:\n    d.pop(k)\n    d[k] = 2\n",
            "a = [1, 2, 3, 4]\nfor x in a:\n    if x % 2 == 0:\n        a.remove(x)\n",
            "a = [1, 2, 3]\nfor x in a:\n    if len(a) < 6:\n        a.append(x * 10)\n",
            "a = [1, 2, 3]\nit = iter(a)\nnext(it)\na.clear()\nlist(it)\n",
            "a = [3, 1, 2]\na.sort(key=lambda x: a.append(x) or x)\n",
            "s = {1}\nfor x in s:\n    s.add(2)\n",
            "a = [1, 2, 3]\nfor i, x in enumerate(a):\n    a.insert(0, x) if i < 2 else None\n",
        ])
        ret = "d" if body.startswith("d") else "sorted(s)" if body.startswith("s") else "a"
        self.run(ret, body.rstrip("\n"))

    # ── exceptions ──────────────────────────────────────────────────

    def stmt_raise(self):
        return self.pick(["raise ValueError('v')", "raise KeyError('k')", "1 / 0", "[][1]", "{}['x']", "int('q')",
                          "raise TypeError", "pass", "pass", "log.append('ok')", "raise IndexError(1, 2)",
                          "raise StopIteration", "assert False, 'no'", "undefined_name", "None.x"])

    def try_block(self, depth, ind):
        p = " " * ind
        lines = [p + "try:", p + "    log.append('try%d')" % depth]
        if depth < 2 and self.r.random() < 0.4:
            lines += self.try_block(depth + 1, ind + 4)
        lines.append(p + "    " + self.stmt_raise())
        n = 0
        for exc in self.r.sample(["ValueError", "KeyError", "LookupError", "(TypeError, ZeroDivisionError)", "Exception", "ArithmeticError"], self.r.randint(0, 2)):
            n += 1
            lines.append(p + "except %s as e%d:" % (exc, depth))
            lines.append(p + "    log.append(('exc%d', type(e%d).__name__, str(e%d)))" % (depth, depth, depth))
            r = self.r.random()
            if r < 0.15:
                lines.append(p + "    raise")
            elif r < 0.25:
                lines.append(p + "    raise RuntimeError('in handler') from e%d" % depth)
            elif r < 0.35:
                lines.append(p + "    " + self.stmt_raise())
        if n and self.r.random() < 0.4:
            lines += [p + "else:", p + "    log.append('else%d')" % depth]
        if n == 0 or self.r.random() < 0.5:
            lines += [p + "finally:", p + "    log.append('fin%d')" % depth]
            r = self.r.random()
            if r < 0.15:
                lines.append(p + "    return log")
            elif r < 0.25:
                lines.append(p + "    " + self.stmt_raise())
        return lines

    def g_exceptions(self):
        nm = self.name()
        lines = ["def _%s():" % nm, "    log = []"] + self.try_block(0, 4) + ["    return log"]
        self.emit("\n".join(lines))
        self.emit("_run(%r, _%s)" % (nm, nm))

    def g_exc_misc(self):
        self.run(self.pick([
            "repr(ValueError('a', 1))", "str(KeyError('k'))", "repr(KeyError())", "ValueError('x').args",
            "str(Exception())", "isinstance(KeyError(), LookupError)", "issubclass(ZeroDivisionError, ArithmeticError)",
            "repr(StopIteration(5).args)", "str(IndexError('a', 'b'))", "type(OverflowError()).__name__",
        ]))
        self.run("r", self.pick([
            "r = []\nfor i in range(3):\n    try:\n        if i == 1:\n            continue\n        r.append(i)\n    finally:\n        r.append('f')",
            "r = []\nwhile True:\n    try:\n        break\n    finally:\n        r.append('f')",
            "def f():\n    try:\n        return 1\n    finally:\n        r.append('fin')\nr = []\nr.append(f())",
            "def f():\n    try:\n        raise ValueError\n    except ValueError:\n        return 'h'\n    finally:\n        r.append('fin')\nr = []\nr.append(f())",
            "r = []\ntry:\n    try:\n        raise KeyError('a')\n    finally:\n        r.append('inner')\nexcept LookupError as e:\n    r.append(repr(e))",
            "r = []\ntry:\n    raise ValueError('a')\nexcept ValueError as e:\n    try:\n        raise TypeError('b')\n    except TypeError as e2:\n        r.append(str(e2))\n    r.append(str(e))",
            "class MyErr(ValueError):\n    def __init__(self, x):\n        super().__init__('my', x)\n        self.x = x\nr = []\ntry:\n    raise MyErr(3)\nexcept ValueError as e:\n    r.append((e.x, e.args, str(e)))",
        ]))

    # ── generators ──────────────────────────────────────────────────

    def g_generators(self):
        self.run("r", self.pick([
            "def g():\n    x = yield 1\n    y = yield x\n    return y\nit = g()\nr = [next(it), it.send(5)]\ntry:\n    it.send(7)\nexcept StopIteration as e:\n    r.append(e.args)",
            "def g():\n    yield from range(3)\n    yield from 'ab'\nr = list(g())",
            "def g():\n    try:\n        yield 1\n        yield 2\n    finally:\n        r.append('closed')\nr = []\nit = g()\nr.append(next(it))\nit.close()\nr.append(list(it))",
            "def g(n):\n    for i in range(n):\n        if i == 2:\n            return 'done'\n        yield i\nr = list(g(5)) + [x * 2 for x in g(3)]",
            "def g():\n    yield 1\n    raise ValueError('in gen')\nr = []\ntry:\n    for x in g():\n        r.append(x)\nexcept ValueError as e:\n    r.append(str(e))",
            "r = sum(x * x for x in range(10) if x % 3), list(zip(range(3), (c for c in 'abc')))",
            "def g():\n    yield 1\nit = g()\nr = [next(it), next(it, 'dflt')]",
            "def g():\n    yield 1\nit = g()\nnext(it)\nr = next(it)",
            "def inner():\n    yield 1\n    return 'ret'\ndef outer():\n    v = yield from inner()\n    yield v\nr = list(outer())",
            "def g():\n    i = 0\n    while True:\n        i += 1\n        yield i\nit = g()\nr = [next(it) for _ in range(3)]",
            "r = list(map(lambda x: x + 1, filter(None, [0, 1, 2, None, 3])))",
            "r = list(enumerate(reversed([1, 2, 3]), 1))",
        ]))

    # ── closures, defaults ──────────────────────────────────────────

    def g_closures(self):
        self.run("r", self.pick([
            "def mk():\n    n = 0\n    def inc():\n        nonlocal n\n        n += 1\n        return n\n    return inc\nf = mk()\nr = [f(), f(), mk()()]",
            "fs = [lambda: i for i in range(3)]\nr = [f() for f in fs]",
            "fs = [lambda i=i: i for i in range(3)]\nr = [f() for f in fs]",
            "def f(a, l=[]):\n    l.append(a)\n    return l\nf(1)\nr = f(2)",
            "def f(a, b=2, *c, d, e=5, **k):\n    return (a, b, c, d, e, sorted(k.items()))\nr = f(1, d=4), f(1, 2, 3, d=0, z=9)",
            "def f(a, b):\n    return a - b\nr = f(b=1, a=5), f(*[3, 1]), f(**{'a': 1, 'b': 2})",
            "def f(a):\n    return a\nr = f(1, 2)",
            "def f(*, k):\n    return k\nr = f(1)",
            "def f(a, b=1):\n    return a\nr = f()",
            "def f(a):\n    return a\nr = f(a=1, b=2)",
            "x = 1\ndef f():\n    global x\n    x += 1\n    return x\nr = f(), x",
            "def outer():\n    x = 'o'\n    def mid():\n        def inner():\n            return x\n        return inner\n    x = 'changed'\n    return mid()()\nr = outer()",
            "def f():\n    v = 1\n    g = lambda: v * 10\n    v = 2\n    return g()\nr = f()",
            "def deco(fn):\n    def w(*a, **k):\n        return ('w', fn(*a, **k))\n    return w\n@deco\ndef h(x, y=3):\n    return x * y\nr = h(2), h(2, y=4)",
        ]))

    # ── containers ──────────────────────────────────────────────────

    def g_containers(self):
        self.run(self.pick([
            "[1, 2, 3].index(4)", "[1, 2] * -1", "[[0] * 2] * 2", "list(range(10))[::3]", "[3, 1, 2].sort()",
            "sorted([3, 1, 2], reverse=True)", "sorted(['b', 'A', 'c'], key=str.lower)", "{'a': 1}.get('b', 0)",
            "{'a': 1}.pop('b')", "{'a': 1}.setdefault('b', [])", "dict(a=1, b=2) | {'a': 3}", "list({1: 2, 3: 4}.items())",
            "[1, 2, 3].pop(5)", "[].pop()", "tuple([1, 2]) + (3,)", "(1, 2) + [3]", "[1] + (2,)", "dict([(1, 2), (3, 4)])",
            "list(zip([1, 2, 3], 'ab'))", "[x for x in range(5) if x % 2 for y in range(x)]", "{k: v for k, v in zip('ab', 'cd')}",
            "sum([[1], [2]], [])", "min([])", "max([], default=7)", "min(3, 1, 2, key=lambda x: -x)", "any([]), all([])",
            "list(reversed(range(3)))", "[1, 2, 3][True]", "len({1: 1, True: 2, 1.0: 3})", "{1: 'a', True: 'b'}",
            "dict.fromkeys('ab', 0)", "[1, 2, 3].count(2.0)", "list(range(0, 10, -1))", "range(10)[-3]", "range(3)[5]",
            "divmod(-7, 2), -7 // 2, -7 % 2, 7 % -2, -7.5 // 2, -7.5 % 2", "(1, 2) * 2", "[1, [2, 3]] == [1, [2, 3.0]]",
            "a, *b = [1, 2, 3]", "list(map(str, [1, 2]))", "' '.join(map(str, range(3)))", "[1, 2][::-1][0]",
            "{'b': 1, 'a': 2}.popitem()", "dict({1: 2}, x=3)", "list(dict.fromkeys([3, 1, 3, 2]))",
        ]).replace("a, *b = [1, 2, 3]", "(lambda a, *b: (a, b))(*[1, 2, 3])"))

    # ── classes and other protocols ─────────────────────────────────

    def g_classes(self):
        self.run("r", self.pick([
            "class A:\n    def __init__(self):\n        self.x = 1\n    def __getattr__(self, n):\n        return n * 2\nr = A().x, A().yy",
            "class A:\n    def __len__(self):\n        return 3\n    def __bool__(self):\n        return False\nr = len(A()), bool(A()), not A()",
            "class A:\n    def __len__(self):\n        return 0\nr = bool(A()), 'yes' if A() else 'no'",
            "class A:\n    def __getitem__(self, i):\n        if i > 3:\n            raise IndexError(i)\n        return i * i\nr = list(A()), 2 in A()",
            "class A:\n    def __iter__(self):\n        return iter([3, 4])\nr = list(A()), sorted(A()), max(A())",
            "class A:\n    def __call__(self, *a):\n        return a\nr = A()(1, 2)",
            "class A:\n    def __add__(self, o):\n        return 'add'\n    def __radd__(self, o):\n        return 'radd'\nr = A() + 1, 1 + A(), A() + A(), sum([A(), A()], 0)",
            "class A:\n    def __mul__(self, o):\n        return NotImplemented\nclass B:\n    def __rmul__(self, o):\n        return 'B.rmul'\nr = A() * B()",
            "class A:\n    def __sub__(self, o):\n        return NotImplemented\nr = A() - 1",
            "class A:\n    def __neg__(self):\n        return 'neg'\n    def __abs__(self):\n        return 'abs'\nr = -A(), abs(A())",
            "class A:\n    x = 1\nclass B(A):\n    pass\nb = B()\nb.x = 2\nr = A.x, B.x, b.x, isinstance(b, A), issubclass(B, A), type(b).__name__",
            "class A:\n    @property\n    def p(self):\n        return 5\nr = A().p\nA().p = 3",
            "class A:\n    def __eq__(self, o):\n        return True\nr = A() == 1, A() != 1, 1 == A(), A() in [1], [A()] == [2]",
            "class A:\n    def __contains__(self, x):\n        return x == 2\nr = 2 in A(), 3 not in A()",
            "class A:\n    def __index__(self):\n        return 2\nr = [1, 2, 3][A()], hex(A()), range(A())",
            "class A:\n    def __str__(self):\n        return 's'\n    def __repr__(self):\n        return 'r'\nr = str(A()), repr(A()), [A()], f'{A()}', f'{A()!r}', '%s %r' % (A(), A())",
            "class A:\n    def __format__(self, spec):\n        return 'F' + spec\nr = format(A(), 'xy'), f'{A():>5}', '{:q}'.format(A())",
            "class A:\n    def __hash__(self):\n        return 1\n    def __eq__(self, o):\n        return isinstance(o, A)\nd = {A(): 1}\nd[A()] = 2\nr = len(d), d[A()]",
            "class A:\n    def __lt__(self, o):\n        return False\nr = sorted([A(), A()]) and 'ok', A() > A()",
            "class Base:\n    def f(self):\n        return 'base'\nclass Sub(Base):\n    def f(self):\n        return 'sub+' + super().f()\nr = Sub().f()",
        ]))

    # ── programs ────────────────────────────────────────────────────

    GENS = [("g_arith", 6), ("g_intfmt", 1), ("g_intparse", 1), ("g_floatops", 2), ("g_richcmp", 3), ("g_cmp_builtin", 2),
            ("g_strmeth", 3), ("g_strops", 2), ("g_format", 4), ("g_percent", 3), ("g_slice", 3), ("g_slice_assign", 2),
            ("g_mutate", 1), ("g_exceptions", 3), ("g_exc_misc", 1), ("g_generators", 1), ("g_closures", 1),
            ("g_containers", 2), ("g_classes", 2)]

    def program(self, n=40, only=None):
        self.emit(PRELUDE)
        names = [g for g, w in self.GENS for _ in range(w) if only is None or g in only]
        for _ in range(n):
            getattr(self, self.r.choice(names))()
        return "\n\n".join(x.strip("\n") for x in self.out) + "\n"


def program(seed, n=40, only=None):
    return G(seed).program(n, only)


if __name__ == "__main__":
    sys.stdout.write(program(int(sys.argv[1]) if len(sys.argv) > 1 else 0))
