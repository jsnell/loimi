# Binary operators on instances (reflected methods, subclass priority,
# same-class operands), __index__, and CPython's operator error messages.

def show(f):
    try:
        print(repr(f()))
    except Exception as e:
        print(type(e).__name__, e)

class A:
    def __add__(self, o):
        return NotImplemented
    def __radd__(self, o):
        return "A.radd"
class B(A):
    def __radd__(self, o):
        return "B.radd"
show(lambda: A() + A())
show(lambda: A() + B())
show(lambda: 1 + A())
show(lambda: A() + 1)

class I:
    def __index__(self):
        return 2
show(lambda: [1, 2, 3][I()])
show(lambda: (1, 2, 3)[I()])
show(lambda: "abc"[I()])
show(lambda: hex(I()))
show(lambda: bin(I()))
show(lambda: list(range(I())))
show(lambda: list(range(I(), 5)))
def setdel():
    a = [1, 2, 3]
    a[I()] = 9
    del a[I() if False else 0]
    return a
show(setdel)

show(lambda: (1, 2) + [3])
show(lambda: [1] + (2,))
show(lambda: "a" * "b")
show(lambda: [1] * 1.5)
show(lambda: 2.5 * (1,))
show(lambda: 0 ** -1)
show(lambda: 0.0 ** -2)
show(lambda: 2.0 ** 10000)
show(lambda: 10.0 ** 400)
show(lambda: -0.0 ** 3)
show(lambda: -(0.0))
show(lambda: [1, 2].index(4))
show(lambda: [1, 2].index("x"))
show(lambda: (1, 2).index(4))
