# Instance layout: attributes past the slots, different orders, deletion,
# __dict__, setattr with odd and computed names, many attributes,
# properties, subclasses, exceptions (ops.new_inst, value.Inst).

class P:
    def __init__(self, x, y):
        self.x = x
        self.y = y

def show(o):
    print(list(vars(o).items()))

# overflow past the slots (attributes set from outside)
ps = [P(i, i + 1) for i in range(5)]
for i, p in enumerate(ps):
    p.a = i * 10
    p.b = i * 100
    p.c = "c%d" % i
    p.d = None
    p.e = 1.5
for p in ps:
    p.c = p.c + "!"
    p.y += 1
    print(p.x, p.y, p.a, p.b, p.c, p.d, p.e)
show(ps[0])
q = P(1, 2)
print(list(vars(q).items()))

# different orders
r1 = P(0, 0); r1.m = 1; r1.n = 2
r2 = P(0, 0); r2.n = 2; r2.m = 1
print(list(vars(r1).keys()), list(vars(r2).keys()))
def get_m(o): return o.m
for o in [r1, r2, r1, r2]: print(get_m(o))

# deletion keeps order, re-adding goes to the end
d = P(1, 2); d.z = 3
del d.x
print(list(vars(d).items()))
d.x = 9
print(list(vars(d).items()), d.x, d.y, d.z)
try:
    del d.nope
except AttributeError as e:
    print("AttributeError", e)
try:
    print(d.nope)
except AttributeError as e:
    print("AttributeError", e)

# __dict__ live view and replacement
e = P(1, 2)
dd = e.__dict__
dd["w"] = 5
print(e.w)
e.v = 6
print(dd["v"], list(dd.keys()))
e.__dict__ = {"k": 1}
print(e.k, hasattr(e, "x"))

# setattr / getattr with odd and computed names
f = P(1, 2)
setattr(f, "not an identifier", 3)
setattr(f, "x" + "x", 4)
print(getattr(f, "not an identifier"), f.xx, getattr(f, "x" * 2))
print(list(vars(f).items()))

# many attributes (past the slot limit)
g = P(0, 0)
for i in range(100):
    setattr(g, "a%d" % i, i)
print(g.a0, g.a50, g.a99, len(vars(g)))
h = P(0, 0)
for i in range(100):
    setattr(h, "a%d" % i, i)
print(h.a99, sum(getattr(h, "a%d" % i) for i in range(100)))

# properties and class attributes
class Q:
    k = 10
    def __init__(self):
        self._v = 1
    @property
    def v(self):
        return self._v * 2
    @v.setter
    def v(self, val):
        self._v = val
    def meth(self):
        return "meth"
q = Q()
q.v = 5
print(q.v, q._v, q.k, list(vars(q).items()))
q.k = 11
q.meth = lambda: "inst"
print(q.k, Q.k, q.meth())

# subclasses sharing attribute order
class Base:
    def __init__(self):
        self.s = 1
class A(Base):
    def __init__(self):
        Base.__init__(self)
        self.a = 2
class B(Base):
    def __init__(self):
        super().__init__()
        self.b = 3
def gs(o): return o.s
print([gs(o) for o in [A(), B(), A(), B(), Base()]])
objs = [A(), B()]
for o in objs: o.s += 1
print([vars(o) for o in objs])

# exceptions carry args
try:
    raise ValueError("bad", 2)
except ValueError as ex:
    ex.extra = 1
    print(ex.args, ex.extra, repr(ex))
class MyErr(Exception):
    def __init__(self, msg):
        super().__init__(msg)
        self.msg = msg
try:
    raise MyErr("m")
except MyErr as ex:
    print(ex.msg, ex.args, str(ex))

# instance with no attributes
class E: pass
x = E()
print(vars(x))
x.a = 1; x.b = 2; x.c = 3
print(vars(x))
del x.a
del x.b
print(vars(x))
y = E()
y.q = 1
print(y.q, vars(y))
