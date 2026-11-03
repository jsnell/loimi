# popitem and insertion in turn (std.dict used to fill its index with
# deleted slots and hang), and instance attributes added and deleted in turn.
d = {}
for i in range(1000):
    d[i] = i * 2
    k, v = d.popitem()
    assert k == i and v == i * 2
print(len(d), d)
d = {"a": 1, "b": 2}
for i in range(500):
    d["k%d" % i] = i
    d.popitem()
print(d)


class C:
    pass


c = C()
for i in range(300):
    setattr(c, "x%d" % i, i)
    if i % 3:
        delattr(c, "x%d" % i)
print(sorted(vars(c).items())[:5], len(vars(c)))
print(c.x0, c.x297, hasattr(c, "x1"))
