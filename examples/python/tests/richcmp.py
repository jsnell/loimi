# Rich comparisons: reflected methods, NotImplemented, sequences of
# instances, and the TypeErrors of unsupported comparisons.

class G:
    def __lt__(self, o):
        return True

class N:
    def __lt__(self, o):
        return NotImplemented

def show(f):
    try:
        print(f())
    except TypeError as e:
        print("TypeError:", e)

show(lambda: [G(), 1] < [0, 1])
show(lambda: [G(), 1] > [0, 1])
show(lambda: [0, 1] > [G(), 1])
show(lambda: [N(), 1] < [0, 1])
show(lambda: N() < 1)
show(lambda: 1 > N())
show(lambda: N() < N())
show(lambda: (N(),) <= (N(),))
show(lambda: G() > G())
show(lambda: G() <= 1)
show(lambda: None < 1)
show(lambda: [1] < (1,))
show(lambda: "a" < 1)
show(lambda: {1} < [1])
show(lambda: 1 == "1")
show(lambda: [1, 2] < [1, 2, 3])
show(lambda: (1, 2) >= (1, 2))
show(lambda: [1, "a"] < [1, 2])

class V:
    def __init__(self, v):
        self.v = v
    def __lt__(self, o):
        print("lt", self.v, o.v if isinstance(o, V) else o)
        if not isinstance(o, V):
            return NotImplemented
        return self.v < o.v
    def __gt__(self, o):
        print("gt", self.v, o.v if isinstance(o, V) else o)
        if not isinstance(o, V):
            return NotImplemented
        return self.v > o.v
    def __eq__(self, o):
        print("eq", self.v, o.v if isinstance(o, V) else o)
        if not isinstance(o, V):
            return NotImplemented
        return self.v == o.v
    __hash__ = None

class W(V):
    def __gt__(self, o):
        print("W.gt", self.v)
        return "from W"

show(lambda: V(1) < V(2))
show(lambda: V(1) < 3)
show(lambda: 3 > V(1))
show(lambda: V(1) < W(2))
show(lambda: W(1) < V(2))
show(lambda: [V(1), V(2)] < [V(1), V(3)])
show(lambda: [V(1)] == [V(1)])
show(lambda: V(1) != V(1))
show(lambda: V(1) != 5)
show(lambda: V(1) == 5)
show(lambda: {1: V(1)} == {1: V(1)})
show(lambda: {1: V(1)} != {1: V(2)})
quiet = [V(3), V(1), V(2)]
show(lambda: max([V(3), V(1), V(2)]).v)

# A __lt__ result that is not a bool is the result of the list comparison.
class S:
    def __lt__(self, o):
        return "yes"
    def __eq__(self, o):
        return False
print([S()] < [S()])
print(S() < S())
print(not (S() < 1))

class E:
    def __eq__(self, o):
        return 1
print(E() == E(), E() != E(), E() in [E()], [E()].count(E()), [5, E()].index(E()))

# Arithmetic fast path with NotImplemented
class A:
    def __add__(self, o):
        if not isinstance(o, A):
            return NotImplemented
        return "A+A"
    def __radd__(self, o):
        return "radd " + str(o)
print(A() + A(), 1 + A())
show(lambda: A() + 1)

class B:
    def __add__(self, o):
        return NotImplemented
class C:
    def __radd__(self, o):
        return "C.radd"
print(B() + C())
show(lambda: B() + B())
total = 0
for i in range(3):
    try:
        total = B() + i
    except TypeError as e:
        print("loop TypeError:", e)
