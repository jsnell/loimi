def make_counter():
    n = 0
    def inc(k=1):
        nonlocal n
        n += k
        return n
    return inc

c1 = make_counter()
c2 = make_counter()
print(c1(), c1(), c1(10), c2())

def adders():
    return [lambda x, i=i: x + i for i in range(3)]
print([f(10) for f in adders()])

def outer():
    x = 1
    def mid():
        def inner():
            return x + 1
        return inner()
    x = 5
    return mid()
print(outer())

total = 0
def add(v):
    global total
    total += v
add(3); add(4)
print(total)

def kw(a, b=2, *args, c=3, **kwargs):
    return (a, b, args, c, sorted(kwargs.items()))
print(kw(1))
print(kw(1, 5, 6, 7, c=8, d=9, e=10))
print(kw(b=1, a=0))
def spread(*a, **k):
    return a, k
print(spread(*[1, 2], *(3,), **{"x": 1}))
args = (1, 2)
print(kw(*args, c=0))

def apply(f, *a):
    return f(*a)
print(apply(max, 3, 9, 2), apply(lambda: "none"))

def compose(*fs):
    def run(x):
        for f in reversed(fs):
            x = f(x)
        return x
    return run
print(compose(str, lambda x: x * 2, abs)(-21))

def memo(f):
    cache = {}
    def wrapper(n):
        if n not in cache:
            cache[n] = f(n)
        return cache[n]
    return wrapper

@memo
def fibm(n):
    return n if n < 2 else fibm(n - 1) + fibm(n - 2)
print(fibm(80))

def gen_closure():
    k = 10
    def g():
        for i in range(3):
            yield i + k
    return g
print(list(gen_closure()()))

def defaults(a, b=[]):
    b.append(a)
    return b
print(defaults(1), defaults(2), defaults(3, []))

def kwonly(*, x, y=2):
    return x * y
print(kwonly(x=5), kwonly(y=3, x=1))

try:
    kw()
except TypeError as e:
    print(e)
try:
    kwonly(1)
except TypeError as e:
    print(e)

class Acc:
    def __init__(self):
        self.items = []
    def add(self, *xs, scale=1):
        for x in xs:
            self.items.append(x * scale)
        return self
print(Acc().add(1, 2).add(3, scale=10).items)

def counter_factory():
    count = [0]
    def bump():
        count[0] += 1
        return count[0]
    return bump, lambda: count[0]
b, get = counter_factory()
b(); b()
print(get())

def shadow():
    x = "local"
    def f():
        x = "inner"
        return x
    return f() + " " + x
print(shadow())
sq = lambda n: n * n
print(list(map(sq, range(4))), sorted([(1, "b"), (0, "c")], key=lambda p: p[1]))
