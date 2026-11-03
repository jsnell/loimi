a = [5, 3, 8, 1]
a.sort()
print(a, sorted(a, reverse=True), a.index(8), a.count(3), a.pop(), a.pop(0), a)
a.insert(1, 99); a.extend([7, 7]); a.remove(7)
print(a, a[::-1], a * 2, a + [0], 99 in a, len(a))
b = a[:]
b[0] = -1
print(a, b, a == b, a < b)
del b[0]
print(b)
m = [[0] * 3 for _ in range(3)]
m[1][2] = 5
print(m, [row[2] for row in m], [x for row in m for x in row if x])
l = list(range(10))
l[2:5] = [20, 30]
print(l, l[-3:], l[1:8:3])
l[:3] = l[2::-1]
print(l)
del l[::2]
print(l)
t = (1, 2, 3)
x, y, z = t
first, *rest = [1, 2, 3, 4]
*init, last = "abc"
print(x, y, z, first, rest, init, last, t + (4,), t[1:], (1,), ())
(p, q), r = (1, 2), 3
print(p, q, r, t.index(2), t.count(1), len(t), t * 2)
d = {"b": 2, "a": 1}
d["c"] = 3
del d["b"]
print(d, list(d), list(d.keys()), list(d.values()), "a" in d, d.pop("a"), d)
d.update({"x": 9}, y=8)
d.setdefault("z", 0)
print(d, len(d), sorted(d.items()), {k: v * 2 for k, v in d.items() if v})
e = dict(a=1, b=2)
f = dict([("k", 1), ("j", 2)])
print(e, f, e == {"b": 2, "a": 1}, dict.fromkeys("ab", 0))
nested = {(1, 2): "tuple key", 3: [1, 2], "s": {"inner": True}}
print(nested[(1, 2)], nested[3], nested["s"]["inner"])
s = {3, 1, 2}
s.add(4); s.discard(1)
print(sorted(s), 2 in s, len(s), sorted(s | {9}), sorted(s & {2, 3}), sorted(s - {2}), set() == set([]))
fs = frozenset([1, 2])
print(sorted(fs), {1, 2} == fs, {1} < {1, 2}, {1, 2}.issubset([1, 2, 3]))
print(list(enumerate("ab", 1)), dict(zip("abc", range(3))), list(range(10, 0, -3)), range(5)[2])
print(min([(2, "b"), (1, "z"), (1, "a")]), max({"a": 3, "b": 1}, key=lambda k: {"a": 3, "b": 1}[k]))
stack = []
for i in range(3):
    stack.append(i)
while stack:
    print(stack.pop(), end=",")
print()
grid = {}
for i in range(3):
    for j in range(3):
        grid[i, j] = i * j
print(grid[2, 2], len(grid), sum(grid.values()))
print([i * j for i in range(1, 4) for j in range(i)], {x % 3 for x in range(10)} == {0, 1, 2})
print(list(map(str, [1, 2])), [None] * 3, [1, [2, [3]]], bool([]), bool([0]), not {})
big = {}
for i in range(1000):
    big[str(i)] = i
for i in range(0, 1000, 2):
    del big[str(i)]
print(len(big), big["999"], sum(big.values()), list(big)[:5])
lst = [3, 1, 2]
lst += [4]
alias = lst
alias += [5]
print(lst, alias is lst)
pairs = [(2, "b"), (1, "a"), (2, "a")]
pairs.sort()
print(pairs, sorted(pairs, key=lambda p: p[1]), list(reversed(pairs)))
matrix = [[1, 2], [3, 4]]
print([list(r) for r in zip(*matrix)], [sum(r) for r in matrix])
dd = {}
for w in ["a", "b", "a"]:
    dd.setdefault(w, []).append(len(dd))
print(dd, list(dd.items()))
counts = {}
for ch in "hello world":
    if ch != " ":
        counts[ch] = counts.get(ch, 0) + 1
print(sorted(counts.items(), key=lambda kv: (-kv[1], kv[0]))[:3])
r = range(2, 20, 3)
print(list(r), len(r), 5 in r, 6 in r, r[-1], list(r[1:3]))
print(list(range(5))[::-1], [x for x in range(20) if x % 3 == 0 if x % 2 == 0])
