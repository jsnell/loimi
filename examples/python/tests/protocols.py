class Grid:
    def __init__(self, w, h):
        self.w = w
        self.h = h
        self.cells = {}
    def __getitem__(self, pos):
        return self.cells.get(pos, ".")
    def __setitem__(self, pos, v):
        self.cells[pos] = v
    def __delitem__(self, pos):
        del self.cells[pos]
    def __contains__(self, pos):
        return pos in self.cells
    def __len__(self):
        return len(self.cells)
    def __iter__(self):
        for y in range(self.h):
            yield "".join(self[x, y] for x in range(self.w))
    def __repr__(self):
        return "Grid(%d, %d)" % (self.w, self.h)

g = Grid(4, 2)
g[1, 0] = "#"
g[3, 1] = "@"
g[0, 0] = "x"
del g[0, 0]
print(list(g), len(g), (1, 0) in g, (0, 0) in g, repr(g), [g])

class Proxy:
    def __init__(self, target):
        self._target = target
    def __getattr__(self, name):
        return "proxied:" + name
p = Proxy(1)
print(p.anything, p._target, getattr(p, "other"))

class Money:
    def __init__(self, cents):
        self.cents = cents
    def __format__(self, spec):
        return "$%d.%02d" % divmod(self.cents, 100)
    def __neg__(self):
        return Money(-self.cents)
    def __abs__(self):
        return Money(abs(self.cents))
    def __int__(self):
        return self.cents // 100
    def __float__(self):
        return self.cents / 100
    def __radd__(self, other):
        return Money(self.cents + other * 100)
    def __bool__(self):
        return self.cents != 0
print(f"{Money(1234)}", format(Money(5), ""), int(Money(999)), float(Money(250)), (3 + Money(50)).cents)
print(bool(Money(0)), bool(Money(1)), abs(Money(-7)).cents, (-Money(3)).cents)

d = {"a": 1, "b": 2}
ks = d.keys()
vs = d.values()
d["c"] = 3
print(list(ks), list(vs), len(ks), "a" in ks, list(d.items())[-1])
print(d, {**{}} if False else {}, dict(d, z=0))

s = "café \x41\x42"
print(s, len("abc\n"), "\\n", r"\d+", 'tab\there', "a" "b" 'c')

def gen_err():
    for i in range(3):
        try:
            yield i
            if i == 1:
                raise KeyError(i)
        except KeyError as e:
            yield "caught %r" % e
print(list(gen_err()))

def finally_continue():
    out = []
    for i in range(4):
        try:
            if i % 2:
                continue
            out.append(i)
        finally:
            out.append(-i)
    return out
print(finally_continue())

def kwonly(a, *, b, c=3, **rest):
    return a + b + c + len(rest)
print(kwonly(1, b=2), kwonly(1, b=2, c=10, x=1, y=2))
args = [1]
kwargs = {"b": 5}
print(kwonly(*args, **kwargs), max(*[3, 9, 4]), "{}-{}".format(*"ab"))

class Temp:
    def __init__(self):
        self._c = 0.0
    @property
    def f(self):
        return self._c * 9 / 5 + 32
    @f.setter
    def f(self, v):
        self._c = (v - 32) * 5 / 9
t = Temp()
t.f = 212
print(round(t._c, 6), t.f, vars(t))
del t._c
print(hasattr(t, "_c"))

class Node:
    count = 0
    def __init__(self, v, nxt=None):
        self.v = v
        self.nxt = nxt
        Node.count += 1
    def __iter__(self):
        n = self
        while n is not None:
            yield n.v
            n = n.nxt
lst = None
for v in [3, 2, 1]:
    lst = Node(v, lst)
print(list(lst), Node.count, sum(lst), max(lst), sorted(lst, reverse=True))

class Base:
    def hello(self):
        return "base:" + self.name()
    def name(self):
        return "B"
class Mid(Base):
    def name(self):
        return "M"
class Leaf(Mid):
    def hello(self):
        return "leaf+" + super().hello()
print(Leaf().hello(), Mid().hello(), Base().hello())
print(Leaf.__mro__ if False else [c.__name__ for c in Leaf.__mro__])

try:
    class Bad(int, str):
        pass
except TypeError:
    print("no multiple inheritance here either")

matrix = [[1, 2, 3], [4, 5, 6]]
print([[row[i] for row in matrix] for i in range(3)], {(i, j): i * j for i in range(2) for j in range(2)})
print(sorted({"b": 1, "a": 2}.items(), key=lambda kv: kv[1]), list(zip(*[(1, "a"), (2, "b")])))
