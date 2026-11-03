"""CPython side of bench/bigint/bench.lm: the same work, best of 5."""
import sys
import time

sys.set_int_max_str_digits(0)


def factorial(n):
    f = 1
    for i in range(2, n + 1):
        f *= i
    return f


def fib(n):
    if n == 0:
        return 0, 1
    a, b = fib(n // 2)
    c = a * (2 * b - a)
    d = a * a + b * b
    return (c, d) if n % 2 == 0 else (d, c + d)


best = [float('inf')] * 6
sums = [0] * 6
for _ in range(5):
    t0 = time.perf_counter(); f = factorial(5000); t1 = time.perf_counter()
    best[0] = min(best[0], t1 - t0); sums[0] = hash(f)
    t0 = time.perf_counter(); x, _y = fib(100000); t1 = time.perf_counter()
    best[1] = min(best[1], t1 - t0); sums[1] = hash(x)
    big = 7 ** 118000
    t0 = time.perf_counter(); s = str(big); t1 = time.perf_counter()
    best[2] = min(best[2], t1 - t0); sums[2] = len(s)
    t0 = time.perf_counter(); back = int(s); t1 = time.perf_counter()
    best[3] = min(best[3], t1 - t0); sums[3] = hash(back)
    t0 = time.perf_counter(); p = big * x; t1 = time.perf_counter()
    best[4] = min(best[4], t1 - t0); sums[4] = hash(p)
    t0 = time.perf_counter(); q, r = divmod(p, x); t1 = time.perf_counter()
    best[5] = min(best[5], t1 - t0); sums[5] = (hash(q) + hash(r) + 2**63) % 2**64 - 2**63
names = ["factorial(5000)", "fib(100000)", "str(7**118000)", "int(100k digits)", "mul 100k x 21k digits", "divmod 121k / 21k digits"]
for i in range(6):
    print("%s %d us  check %d" % (names[i], int(best[i] * 1e6), sums[i]))
