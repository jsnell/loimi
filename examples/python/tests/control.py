for i in range(3):
    if i == 1:
        continue
    print("i", i)
else:
    print("for-else ran")
for i in range(3):
    if i == 1:
        break
else:
    print("not printed")
n = 0
while n < 10:
    n += 3
else:
    print("while-else", n)
x = 5
print("big" if x > 3 else "small", x > 3 and "yes", x < 3 or "no", 0 or [] or "last", 1 and 2 and 3)
if 1 < x < 10 and x != 6:
    print("chained ok")
print(1 < 2 < 3, 3 > 2 > 2, 1 == 1.0 == True, None is None, [] is not None)
def classify(v):
    if v < 0:
        return "neg"
    elif v == 0:
        return "zero"
    elif v < 10:
        return "small"
    return "large"
print([classify(v) for v in (-1, 0, 5, 50)])
if (y := 10) > 5:
    print("walrus", y)
total = 0
for a, b in [(1, 2), (3, 4)]:
    total += a * b
print(total)
for i, (a, b) in enumerate([(1, 2), (3, 4)]):
    print(i, a, b)
k = 0
while True:
    k += 1
    if k > 5:
        break
print(k)
def early(xs):
    for x in xs:
        if x % 7 == 0:
            return x
    return None
print(early(range(1, 20)), early([1, 2]))
v = 0
for _ in range(1000):
    v = (v * 31 + 7) % 1000003
print(v)
values = [3, 1, 4, 1, 5, 9, 2, 6]
print(sorted(values), sorted(values, key=lambda t: -t), max(values), min(values), sum(values) / len(values))
def nested_loops():
    found = []
    for i in range(4):
        for j in range(4):
            if i + j == 3:
                found.append((i, j))
                break
        else:
            found.append("none")
    return found
print(nested_loops())
count = 0
while count < 3:
    count += 1
    if count == 2:
        continue
    print("count", count)
print(not 0, not 1, not "", not "a", 0.0 == 0, bool(None), bool(0.1))
z = 0
if z:
    print("no")
elif not z and (z == 0 or z / 0):
    print("short circuit")
print([1, 2] and [3], [] and [3], None or 0, 2 if 0 else 3 if 0 else 4)
def sign(n):
    return (n > 0) - (n < 0)
print(sign(-5), sign(0), sign(7))
pos = [i for i, c in enumerate("a.b.c") if c == "."]
print(pos)
