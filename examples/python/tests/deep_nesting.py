# Deeply nested containers: ==, <, repr, str, `in`, sorted and print
# raise RecursionError instead of overflowing the C stack.
# (check.sh: no --gc-stress run: building the values collects at every
# allocation and takes minutes.)

def nest(n, f):
    a = 0
    for i in range(n):
        a = f(a)
    return a

def check(name, f):
    try:
        f()
        print(name, "ok")
    except RecursionError:
        print(name, "RecursionError")

check("eq list", lambda: nest(12000, lambda a: [a]) == nest(12000, lambda a: [a]))
check("lt list", lambda: nest(12000, lambda a: [a]) < nest(12000, lambda a: [a, 1]))
check("eq tuple", lambda: nest(12000, lambda a: (a,)) == nest(12000, lambda a: (a,)))
check("eq dict", lambda: nest(12000, lambda a: {1: a}) == nest(12000, lambda a: {1: a}))
check("repr list", lambda: repr(nest(12000, lambda a: [a])))
check("str tuple", lambda: str(nest(12000, lambda a: (a,))))
check("repr dict", lambda: repr(nest(12000, lambda a: {1: a})))
check("in", lambda: nest(12000, lambda a: [a]) in [nest(12000, lambda a: [a])])
check("sorted", lambda: sorted([nest(12000, lambda a: [a]), nest(12000, lambda a: [a])]))
check("print", lambda: print(nest(12000, lambda a: [a])))
check("f-string", lambda: f"{nest(12000, lambda a: [a])!r}")
