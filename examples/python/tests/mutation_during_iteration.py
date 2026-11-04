# Dicts, sets and lists changed while iterated or sorted.

def run(name, f):
    try:
        print(name, f())
    except Exception as e:
        print(name, "EXC", type(e).__name__, e)

def m1():
    d = {1: 1, 2: 2, 3: 3}
    for k in d:
        d[k + 10] = 0
def m2():
    d = {1: 1}
    for k in d:
        d.pop(k)
        d[k] = 2
def m3():
    d = {1: 1, 2: 2, 3: 3}
    for k in d.keys():
        d.pop(k)
def m4():
    s = {1}
    for x in s:
        s.add(2)
def m5():
    d = {1: 1, 2: 2}
    for k in d:
        d[k] = k * 5
    return d
def m6():
    a = [3, 1, 2]
    a.sort(key=lambda x: a.append(x) or x)
    return a
def m7():
    a = [3, 1, 2]
    seen = []
    a.sort(key=lambda x: seen.append(len(a)) or x)
    return a, seen
def m8():
    d = {1: 1, 2: 2}
    it = iter(d.items())
    next(it)
    d[5] = 5
    try:
        next(it)
    except RuntimeError as e:
        print("first", e)
    return list(it)
for f in [m1, m2, m3, m4, m5, m6, m7, m8]:
    run(f.__name__, f)
