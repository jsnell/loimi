# list.pop / list.insert / list.append through the method-call fast
# paths, as bound methods, with odd indexes and errors; math functions.
import math

xs = [1, 2, 3, 4, 5]
print(xs.pop(), xs.pop(0), xs.pop(-1), xs.pop(1), xs)
xs = list(range(10))
pop = xs.pop
ins = xs.insert
for i in range(5):
    ins(i * 2, pop(0))
print(xs)
ins(-100, "a"); ins(100, "z"); ins(-1, "y"); ins(0, "b")
print(xs)
for bad in [lambda: [].pop(), lambda: [1].pop(5), lambda: [1].pop(-2), lambda: [1].pop("x"),
            lambda: [1].insert(1.5, 2), lambda: [1].insert(1), lambda: [1].pop(1, 2)]:
    try:
        bad()
        print("no error")
    except Exception as e:
        print(type(e).__name__, e)
ys = []
app = ys.append
for i in range(20):
    app(i)
    ys.append(-i)
print(ys, len(ys))
zs = [[] for _ in range(3)]
for i in range(30):
    zs[i % 3].insert(0, i)
print(zs)
print([z.pop(True) for z in zs])

print(math.sqrt(16), math.sqrt(2), math.sin(0), math.cos(0), math.tan(0.5), math.atan(1), math.exp(1), math.fabs(-3))
print(math.sqrt(9), math.fabs(-7), math.exp(0))
for bad in [lambda: math.sqrt(-1), lambda: math.sqrt("x"), lambda: math.cos(None)]:
    try:
        bad()
    except Exception as e:
        print(type(e).__name__, e)
from math import sqrt, cos
print(sqrt(25.0), cos(0.0), sqrt(True))
