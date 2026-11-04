# `for a, b in ...` with two plain locals (FORITER2) and in comprehensions.

def f():
    out = []
    for i, x in enumerate([10, 20, 30]):
        out.append((i, x))
    for i, x in enumerate((7, 8), 5):
        out.append((i, x))
    for a, b in zip([1, 2, 3], ["a", "b"]):
        out.append((a, b))
    for a, b in zip([1, 2], [3, 4, 5]):
        out.append((a, b))
    for k, v in {"x": 1, "y": 2}.items():
        out.append((k, v))
    for a, b in [(1, 2), [3, 4], "ab", range(5, 7)]:
        out.append((a, b))
    print(out)
    # the iterators are left where CPython leaves them
    it1 = iter([1, 2, 3])
    it2 = iter([4])
    for a, b in zip(it1, it2):
        print("zip", a, b)
    print(list(it1), list(it2))
    e = enumerate([5, 6])
    for i, x in e:
        print("en", i, x)
        break
    print(list(e))
    # unpacking errors keep the previous values
    a = b = None
    try:
        for a, b in [(1, 2), (3,)]:
            pass
    except ValueError as ex:
        print("ValueError:", ex, a, b)
    try:
        for a, b in [(1, 2, 3)]:
            pass
    except ValueError as ex:
        print("ValueError:", ex, a, b)
    try:
        for a, b in [5]:
            pass
    except TypeError as ex:
        print("TypeError:", ex)
    # a list changed while iterated
    xs = [(1, 2), (3, 4)]
    for a, b in xs:
        if len(xs) < 4:
            xs.append((a * 10, b * 10))
    print(xs)
    # both names the same
    for a, a in [(1, 2)]:
        print("same", a)
    # generators of pairs
    def g():
        yield 1, 2
        yield 3, 4
    for a, b in g():
        print("gen", a, b)
    # else clause and break
    for a, b in enumerate("xy"):
        pass
    else:
        print("else", a, b)
    return out

f()

# comprehensions
print([i * x for i, x in enumerate([3, 4, 5])])
print({k: v for k, v in zip("abc", range(3))})
print({a + b for a, b in [(1, 2), (2, 1), (0, 4)]})
print([(a, b) for a, b in zip([1, 2, 3], [4, 5, 6]) if a != 2])
print(sum(a * b for a, b in zip([1, 2], [3, 4])))

# at module level the targets are globals
for p, q in enumerate("hi"):
    pass
print(p, q)
