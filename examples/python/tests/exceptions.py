def f(x):
    try:
        if x == 0:
            raise ValueError("zero")
        elif x == 1:
            return 1 / 0
        elif x == 2:
            return [][1]
        elif x == 3:
            return {}["k"]
        return "ok"
    except ValueError as e:
        return "value: " + str(e)
    except (ZeroDivisionError, IndexError) as e:
        return type(e).__name__ + ": " + str(e)
    finally:
        print("finally", x)

for i in range(5):
    try:
        print(f(i))
    except KeyError as e:
        print("key error", e, repr(e))

class MyError(Exception):
    def __init__(self, code):
        super().__init__("code %d" % code)
        self.code = code

try:
    raise MyError(42)
except Exception as e:
    print(type(e).__name__, e.code, e, e.args)

def g():
    try:
        try:
            raise TypeError("inner")
        finally:
            print("inner finally")
    except TypeError as e:
        print("caught", e)
        raise RuntimeError("outer") from e

try:
    g()
except RuntimeError as e:
    print("outer caught", e)

try:
    try:
        raise KeyError("a")
    except KeyError:
        print("re-raising")
        raise
except LookupError as e:
    print("lookup", repr(e))

def loop():
    out = []
    for i in range(5):
        try:
            if i == 1:
                continue
            if i == 3:
                break
            out.append(i)
        finally:
            out.append("f%d" % i)
    return out
print(loop())

try:
    x = undefined_name
except NameError as e:
    print("NameError:", e)

try:
    "a" + 1
except TypeError as e:
    print("TypeError:", e)

try:
    int("xyz")
except ValueError as e:
    print(e)

try:
    raise StopIteration
except StopIteration:
    print("stop")

r = None
try:
    r = 1
except Exception:
    r = 2
else:
    r = r + 10
print(r)

class Ctx:
    def __init__(self, name):
        self.name = name
    def __enter__(self):
        print("enter", self.name)
        return self
    def __exit__(self, t, v, tb):
        print("exit", self.name, t.__name__ if t else None)
        return self.name == "swallow"

with Ctx("a") as c:
    print("body", c.name)
with Ctx("swallow"):
    raise ValueError("hidden")
try:
    with Ctx("b"), Ctx("c"):
        raise IndexError("x")
except IndexError:
    print("propagated")

def wret():
    with Ctx("r"):
        return 5
print(wret())

try:
    assert 1 == 2, "math is broken"
except AssertionError as e:
    print("assert:", e)

def deep(n):
    if n == 0:
        raise ValueError("bottom")
    return deep(n - 1)
try:
    deep(50)
except ValueError as e:
    print("from deep:", e)

def nested_finally():
    try:
        try:
            return "inner"
        finally:
            print("f1")
    finally:
        print("f2")
print(nested_finally())

try:
    {}.pop("missing")
except KeyError as e:
    print("KeyError", e)
try:
    [].pop()
except IndexError as e:
    print(e)
try:
    None.foo
except AttributeError as e:
    print(e)
try:
    len(5)
except TypeError as e:
    print(e)
errs = []
for exc in (ValueError, TypeError, KeyError):
    try:
        raise exc("m")
    except (TypeError, KeyError) as e:
        errs.append(type(e).__name__)
    except Exception as e:
        errs.append("other " + type(e).__name__)
print(errs)
