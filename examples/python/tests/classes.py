class Animal:
    count = 0
    def __init__(self, name, sound="..."):
        self.name = name
        self.sound = sound
        Animal.count += 1
    def speak(self):
        return f"{self.name} says {self.sound}"
    def __repr__(self):
        return "Animal(%r)" % self.name

class Dog(Animal):
    def __init__(self, name):
        super().__init__(name, "woof")
        self.tricks = []
    def add_trick(self, t):
        self.tricks.append(t)
        return self
    def speak(self):
        return super().speak() + "!"

class Puppy(Dog):
    def speak(self):
        base = super().speak()
        return base.upper()

a = Animal("cat", "meow")
d = Dog("rex")
p = Puppy("bit")
d.add_trick("sit").add_trick("roll")
print(a.speak())
print(d.speak(), d.tricks)
print(p.speak())
print(Animal.count, a, [a, d])
print(isinstance(p, Animal), isinstance(a, Dog), issubclass(Puppy, Dog))
print(type(p).__name__, type(3).__name__, type("x").__name__)
d.age = 3
d.age += 1
print(d.age, hasattr(d, "age"), hasattr(d, "zzz"), getattr(d, "zzz", 42))

class Vec:
    def __init__(self, x, y):
        self.x = x
        self.y = y
    def __add__(self, o):
        return Vec(self.x + o.x, self.y + o.y)
    def __sub__(self, o):
        return Vec(self.x - o.x, self.y - o.y)
    def __mul__(self, k):
        return Vec(self.x * k, self.y * k)
    __rmul__ = __mul__
    def __eq__(self, o):
        return isinstance(o, Vec) and self.x == o.x and self.y == o.y
    def __lt__(self, o):
        return (self.x, self.y) < (o.x, o.y)
    def __len__(self):
        return 2
    def __getitem__(self, i):
        return (self.x, self.y)[i]
    def __str__(self):
        return f"<{self.x}, {self.y}>"
    def __repr__(self):
        return f"Vec({self.x}, {self.y})"
    def __call__(self, k):
        return self.x * k + self.y
    def __hash__(self):
        return hash((self.x, self.y))

v = Vec(1, 2) + Vec(3, 4)
print(v, v - Vec(1, 1), v * 2, 3 * v, v == Vec(4, 6), v != Vec(4, 6), len(v), v[1], v(10))
print(sorted([Vec(3, 1), Vec(1, 2), Vec(1, 1)]))
print(str(Vec(0, 0)), [str(x) for x in [Vec(1, 1)]])
s = {Vec(1, 1), Vec(1, 1), Vec(2, 2)}
print(len(s))

class Counter:
    def __init__(self):
        self._n = 0
    @property
    def n(self):
        return self._n
    @n.setter
    def n(self, v):
        self._n = v * 10
    @staticmethod
    def helper(x):
        return x + 1
    @classmethod
    def make(cls, k):
        c = cls()
        c.n = k
        return c

c = Counter.make(5)
print(c.n, Counter.helper(1), c.helper(2))

class Base(object):
    def __init__(self, a):
        self.a = a
class Derived(Base):
    def __init__(self, a, b):
        Base.__init__(self, a)
        self.b = b
    def total(self):
        return self.a + self.b
print(Derived(1, 2).total())

class Two(Base):
    def __init__(self):
        super(Two, self).__init__(7)
print(Two().a)

class Empty: pass
e = Empty()
e.x = 1
print(e.x, Empty.__name__)

class Shape:
    sides = 0
    def describe(self):
        return "%s with %d sides" % (self.__class__.__name__, self.sides)
class Tri(Shape):
    sides = 3
class Sq(Shape):
    sides = 4
    def describe(self):
        return "square: " + Shape.describe(self)
print([s.describe() for s in (Tri(), Sq(), Shape())])
Tri.sides = 33
print(Tri().describe())
