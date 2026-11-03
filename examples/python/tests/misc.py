import io
import math
from math import sqrt, pi
import random as rnd

buf = io.StringIO()
print("a", 1, end="", file=buf)
print("b", 2, sep="-", file=buf)
print(repr(buf.getvalue()))
print(1, 2, 3, sep=", ", end="!\n")
print(sqrt(2.0), round(pi, 3), math.floor(2.7), math.trunc(-2.7))
rnd.seed(42)
print([rnd.randint(1, 100) for _ in range(5)], rnd.random())

class Stack:
    __slots__ = ("items",)
    def __init__(self):
        self.items = []
    def push(self, x):
        self.items.append(x)
    def pop(self):
        return self.items.pop()
    def __len__(self):
        return len(self.items)
    def __bool__(self):
        return bool(self.items)
    def __iter__(self):
        return iter(self.items)
    def __contains__(self, x):
        return x in self.items

s = Stack()
for i in range(4):
    s.push(i * i)
print(len(s), bool(s), 9 in s, 5 in s, list(s), s.pop(), sum(s))
while s:
    s.pop()
print(bool(s), not s)

def describe(*args, **kwargs):
    parts = [repr(a) for a in args]
    parts += ["%s=%r" % kv for kv in sorted(kwargs.items())]
    return "describe(" + ", ".join(parts) + ")"
print(describe(1, "x", flag=True, n=None))
opts = {"sep": "|", "end": ".\n"}
print("x", "y", **opts)

data = [("bob", 25, 3.5), ("alice", 30, 2.25), ("carol", 25, 4.0)]
for name, age, score in sorted(data, key=lambda r: (r[1], r[0])):
    print(f"{name:<6}|{age:>3}|{score:6.2f}|{score:e}")
print(sorted(data, key=lambda r: r[2], reverse=True)[0][0])
by_age = {}
for name, age, _ in data:
    by_age.setdefault(age, []).append(name)
print(by_age)

table = [[r * c for c in range(1, 4)] for r in range(1, 4)]
print("\n".join(" ".join("%3d" % v for v in row) for row in table))
print(type(None).__name__, type(1.5).__name__, type([]).__name__, type({}).__name__, type(()).__name__)
print(isinstance(True, int), isinstance(1, (str, float)), isinstance("s", (int, str)))
print(callable(len), callable(1), callable(Stack), hasattr(Stack, "push"))
print(int("101", 2), int(" -12 "), float("1e3"), str(-0.5), bool("False"), list(divmod(-7, 2)))
g = globals()
print("Stack" in g, "nope" in g)
counter = 0
def bump():
    global counter
    counter += 1
    return counter
results = [bump() for _ in range(3)]
print(results, counter)
x = y = z = 0
x += 1; y -= 1; z ^= 7
print(x, y, z)
a, b = 1, 2
a, b = b, a
print(a, b)
nested = [1, [2, [3, [4]]]]
def flatten(xs):
    for x in xs:
        if isinstance(x, list):
            yield from flatten(x)
        else:
            yield x
print(list(flatten(nested)))
print(min("hello"), max([3, 7, 2], key=lambda v: -v), sorted({3: "c", 1: "a"}), abs(-0.0))
