# obj.__dict__ and vars(obj) are the instance's live attribute dict.

class P:
    def __init__(self, x, y):
        self.x = x
        self.y = y
    def norm(self):
        return self.x * self.x + self.y * self.y

p = P(3, 4)
d = p.__dict__
print(type(d).__name__, d, len(d))
print(d is p.__dict__, vars(p) is d)

# Writes through the dict are attributes, and attributes show in the dict.
d['x'] = 10
print(p.x, p.norm())
p.z = 5
print(d, 'z' in d, sorted(d))
del d['y']
print(hasattr(p, 'y'), list(d.keys()))
try:
    p.y
except AttributeError as e:
    print('AttributeError', e)
p.y = 2
print(list(d.items()), p.norm())
del p.z
print(d)

# Non-string keys are allowed (not attributes).
d[1] = 2
d[(1, 2)] = 'tuple'
print(d[1], d[(1, 2)], len(d))
print([k for k in d])

# update, setdefault, pop, get, iteration, clear.
d.update({'a': 1, 'b': 2})
d.update(c=3)
print(p.a, p.b, p.c)
print(d.setdefault('e', 7), p.e)
print(d.pop('a'), hasattr(p, 'a'), d.get('a', 'none'))
for k, v in list(d.items()):
    if isinstance(k, int):
        del d[k]
print(d)

# An instance attribute in the dict hides a method of the class.
def triple():
    return 'shadowed'
for i in range(3):
    print(p.norm(), end=' ')
    if i == 0:
        d['norm'] = triple
    if i == 1:
        del d['norm']
print()

# Other instances of the class are unaffected and keep their own attributes.
q = P(1, 1)
print(q.norm(), vars(q), q.__dict__ is not d)
for i in range(3):
    q.x = i
    print(q.x, q.__dict__['x'], end=' ')
print()

# Setting attributes in loops on a converted instance.
for i in range(5):
    p.x = i
    setattr(p, 'w', i * 2)
print(p.x, p.w, d['x'], d['w'])

# Assigning __dict__.
r = P(0, 0)
nd = {'x': 7, 'y': 8}
r.__dict__ = nd
print(r.x, r.y, r.norm(), r.__dict__ is nd)
r.x = 1
print(nd)
try:
    r.__dict__ = 5
except TypeError:
    print('TypeError')

d.clear()
print(vars(p), hasattr(p, 'x'))

# Exceptions are instances too.
class E(Exception):
    pass
e = E('msg')
e.__dict__['code'] = 3
print(e.code, e.args)

# A dict comprehension of vars and a repr from __dict__.
class Pt:
    def __init__(self, x, y):
        self.x = x
        self.y = y
    def __repr__(self):
        return 'Pt(' + ', '.join(k + '=' + repr(v) for k, v in self.__dict__.items()) + ')'
print(Pt(1, 'a'), {k: v * 2 for k, v in vars(Pt(2, 3)).items()})
