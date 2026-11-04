# Generator protocol: return values in StopIteration and `yield from`,
# throw() and close() running the generator's handlers.

def g():
    x = yield 1
    y = yield x
    return y
it = g()
r = [next(it), it.send(5)]
try:
    it.send(7)
except StopIteration as e:
    r.append(e.args)
    r.append(e.value)
try:
    next(it)
except StopIteration as e:
    r.append(e.args)
print(r)

def inner():
    yield 1
    return 'ret'
def outer():
    v = yield from inner()
    yield v
    w = yield from [1, 2]
    yield w
print(list(outer()))

def h():
    try:
        yield 1
        yield 2
    finally:
        print("closed")
it = h()
print(next(it))
it.close()
print(list(it))
it.close()

def k():
    try:
        yield 1
    except ValueError as e:
        print("caught", e)
        yield 'after'
    yield 'end'
it = k()
print(next(it))
print(it.throw(ValueError("boom")))
print(next(it))
try:
    it.throw(KeyError('x'))
except KeyError as e:
    print("propagated", repr(e))
print(list(it))

def fresh():
    yield 1
it = fresh()
try:
    it.throw(TypeError('t'))
except TypeError as e:
    print("unstarted", e)
print(list(it))

def ret_in_finally():
    try:
        yield 1
    finally:
        return 5
it = ret_in_finally()
next(it)
it.close()
print("closed ok")

def nested():
    try:
        try:
            yield 1
        finally:
            print("inner finally")
    finally:
        print("outer finally")
it = nested()
next(it)
it.close()

def gen_with(n):
    for i in range(n):
        try:
            yield i
        except ValueError:
            yield -i
it = gen_with(3)
print(next(it), it.throw(ValueError), next(it), next(it))
