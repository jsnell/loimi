# True division of ints is correctly rounded, also for big operands.

def show(f):
    try:
        print(repr(f()))
    except Exception as e:
        print(type(e).__name__, e)

show(lambda: -7 / 2**63)
show(lambda: (2**63 << 1) / (-7 / 2**63))
show(lambda: -256 / (3**50 << 1))
show(lambda: (-256 << 47) / (3**50))
show(lambda: 10**30 / 3)
show(lambda: 10**400 / 10**399)
show(lambda: 1 / 10**400)
show(lambda: -1 / 10**400)
show(lambda: 10**400 / 3)
show(lambda: 2**1100 / 2**80)
show(lambda: (2**53 + 1) / 1)
show(lambda: (2**54 + 3) / 2)
show(lambda: 3**200 / 7**100)
show(lambda: 0 / 10**30)
show(lambda: 10**30 / 0)
show(lambda: (1, 2) * 2**62)
show(lambda: [0] * -5)
for i in range(1, 60, 7):
    print(i, (3**i) / (2**i + 1), -(5**i) / 7, (2**i * 3) / (3**(i // 2) + 1))
