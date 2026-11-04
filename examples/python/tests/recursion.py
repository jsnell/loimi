# Deep recursion raises RecursionError instead of crashing: Python-level
# calls, calls back from the runtime (__repr__, __eq__, __lt__, key
# functions), and the runtime's own recursion over nested containers.

def nest(n, f):
    a = 0
    for i in range(n):
        a = f(a)
    return a

def check(name, f):
    try:
        f()
        print(name, "ok")
    except RecursionError:
        print(name, "RecursionError")

def down(n):
    return down(n + 1)
check("calls", lambda: down(0))
check("moderate", lambda: len(repr(nest(500, lambda a: [a]))) + (nest(500, lambda a: (a,)) == nest(500, lambda a: (a,))))
print(len(repr(nest(500, lambda a: [a, {1: (0, "x")}]))))

l = [1]
l.append(l)
print(l, repr(l), str(l))
d = {"k": 1}
d["self"] = d
print(d)
m = [d, l, [l]]
print(m)

class R:
    def __repr__(self):
        return repr(self)
check("__repr__", lambda: repr(R()))

class Eq:
    def __eq__(self, o):
        return self == o
check("__eq__", lambda: Eq() == 1)

class Lt:
    def __lt__(self, o):
        return o > self
    def __gt__(self, o):
        return o < self
check("__lt__/__gt__", lambda: Lt() < Lt())

class H:
    def __hash__(self):
        return hash(self)
check("__hash__", lambda: hash(H()))

class It:
    def __iter__(self):
        return iter(It())
check("__iter__", lambda: list(It()))

class Ga:
    def __getattr__(self, n):
        return getattr(self, n)
check("__getattr__", lambda: Ga().x)

def key(x):
    return sorted([x, x], key=key)
check("key", lambda: sorted([1, 2], key=key))

class Node:
    def __init__(self, child):
        self.child = child
    def __repr__(self):
        return "Node(" + repr([self.child]) + ")"
check("nested __repr__", lambda: repr(nest(3000, Node)))
print(repr(nest(3, Node)))

def gen(n):
    if n == 0:
        yield 0
    else:
        yield from gen(n - 1)
check("yield from", lambda: list(gen(100000)))
print(list(gen(50)))

def after():
    return down(0)
for i in range(3):
    check("again", after)
print("done")
