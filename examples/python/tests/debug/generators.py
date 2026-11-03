def countdown(n):
    while n > 0:
        got = yield n
        if got:
            print("got", got)
        n -= 1
    return "done"

def evens(xs):
    for x in xs:
        if x % 2 == 0:
            yield x * 10

def consume():
    total = 0
    for v in countdown(3):
        total += v
    g = countdown(2)
    first = next(g)
    second = g.send("hi")
    print(first, second, total)
    print(list(evens([1, 2, 3, 4])))

consume()
