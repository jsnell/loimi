def count(n):
    i = 0
    while i < n:
        yield i
        i += 1

print(list(count(5)))
print(sum(count(10)))

def fib():
    a, b = 0, 1
    while True:
        yield a
        a, b = b, a + b

for i, x in enumerate(fib()):
    if i >= 10:
        break
    print(x, end=" ")
print()

g = count(3)
print(next(g), next(g), next(g), next(g, "done"))

def gen_with_return(n):
    for i in range(n):
        if i == 2:
            return
        yield i * i
print(list(gen_with_return(10)))

def nested():
    def inner(k):
        for j in range(k):
            yield j
    for k in range(4):
        yield from inner(k)
print(list(nested()))

print(sum(x * x for x in range(10)))
print(tuple(x for x in "abc"), set(x % 3 for x in range(10)) == {0, 1, 2})
print(max(len(w) for w in ["a", "abc", "ab"]))
print(",".join(str(i) for i in range(5)))

def echo():
    total = 0
    while True:
        x = yield total
        if x is None:
            break
        total += x
e = echo()
print(next(e), e.send(5), e.send(10))

def perms(items):
    if len(items) <= 1:
        yield items
    else:
        for i in range(len(items)):
            for p in perms(items[:i] + items[i + 1:]):
                yield [items[i]] + p
print(list(perms([1, 2, 3])))

def gen_try():
    try:
        yield 1
        yield 2
    finally:
        print("gen finally")
print(list(gen_try()))

it = iter([1, 2, 3])
print(next(it), list(it))
print(list(zip([1, 2, 3], "ab")), list(map(lambda x: x * 2, [1, 2])), list(filter(None, [0, 1, 2, 0])))
print(list(reversed([1, 2, 3])), list(reversed(range(4))))
print(all(x > 0 for x in [1, 2]), any(x > 5 for x in [1, 2]))

class Countdown:
    def __init__(self, n):
        self.n = n
    def __iter__(self):
        return self
    def __next__(self):
        if self.n <= 0:
            raise StopIteration
        self.n -= 1
        return self.n
print(list(Countdown(4)), [x * 2 for x in Countdown(3)])

class Tree:
    def __init__(self, v, kids=()):
        self.v = v
        self.kids = kids
    def __iter__(self):
        yield self.v
        for k in self.kids:
            yield from k
t = Tree(1, [Tree(2, [Tree(3)]), Tree(4)])
print(list(t))

def gen_exc():
    yield 1
    raise ValueError("in gen")
try:
    for v in gen_exc():
        print("got", v)
except ValueError as e:
    print("caught", e)

squares = (x * x for x in range(5))
print(next(squares), list(squares), list(squares))
def take(n, it):
    out = []
    for x in it:
        if len(out) >= n:
            break
        out.append(x)
    return out
print(take(3, fib()), take(2, (c for c in "xyz")))
print(sorted((x for x in [3, 1, 2]), reverse=True), dict((i, i * i) for i in range(3)))
