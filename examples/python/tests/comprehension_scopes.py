# Closures over comprehension variables and class bodies inside functions.

def f():
    class A:
        def m(self):
            return isinstance(self, A)
    return A().m()
print(f())
def g():
    fs = [lambda: i for i in range(3)]
    return [h() for h in fs]
print(g())
print([lambda: j for j in range(2)][0]())
def h():
    x = 5
    class B:
        y = x + 1
        def m(self):
            return x, self.y
    return B().m()
print(h())
i = 10
fs = [lambda: i for i in range(3)]
print(i, [f() for f in fs])
print({k: (lambda: k) for k in 'ab'}['a'](), sorted({(lambda: v)() for v in range(3)}))
def outer():
    q = 1
    class C:
        q = 2
        def m(self):
            return q
        r = q
    return C().m(), C.r
print(outer())
