# ** on floats and small ints (POW's fast paths) and its errors.
vals = [2.0, -2.0, 0.5, 1.0, 0.0, -0.0, 1e300, -1e-300, float("inf"), float("-inf"), float("nan"), 3, -3, 0]
exps = [2, -1, 0, 3, 2.0, -2.0, 0.0, 1000, float("inf"), float("nan")]
for x in vals:
    row = []
    for y in exps:
        try:
            r = x ** y
            row.append(repr(r))
        except Exception as e:
            row.append(type(e).__name__)
    print(x, row)
for x in [2.0, 0.5, 1e300, 1e-300, float("inf"), 3, 0.0]:
    for y in [-1.5, 0.5, -0.5]:
        try:
            print(x, y, x ** y)
        except Exception as e:
            print(x, y, type(e).__name__)
print(7 ** 2, (-7) ** 2, (2 ** 40) ** 2, 3.0 ** 2 == 9.0, 2 ** -1)
