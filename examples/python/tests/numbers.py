print(2 ** 64, -(2 ** 64), 2 ** 64 // 3, 2 ** 64 % 1000, (2 ** 64) * (2 ** 64))
print(12345678901234567890 + 1, 10 ** 30 - 1, -10 ** 20 // 7, -10 ** 20 % 7)
x = 1
for i in range(100):
    x *= 3
print(x, x.bit_length(), x > 10 ** 40, x == x + 0)
print(2147483647 + 1, -2147483648 - 1, 65536 * 65536, 46341 * 46341)
print(7 // 2, -7 // 2, 7 % -3, -7 % 3, 7 / 2, 2 ** -1, 2 ** 0.5)
print(1 << 40, (1 << 70) >> 65, -1 >> 3, 255 & 15, 1 | 6, 5 ^ 3, ~5)
print(abs(-3), abs(-2.5), round(2.5), round(3.5), round(2.675, 2), int(3.9), int(-3.9), float(3))
print(divmod(17, 5), divmod(-17, 5), pow(3, 4), pow(3, 200, 1000))
print(0.1 + 0.2, 1e16, 1.5e-7, 123456789.0, 1/3, -0.0, 1e300 * 10, float("nan"))
print(int("123"), int(" 42 "), int("-7"), int("ff", 16), int("0x1f", 16), float("2.5"), int("9" * 30))
print(hex(255), oct(8), bin(5), hex(-1), hex(2 ** 70))
print(3 == 3.0, 1 < 2.5, True + True, 10 * True, hash(1) == hash(1.0))
print(max(1, 2.5, -1), min([3, 1, 2]), sum([0.5, 0.25]), sum(range(101)))
print("%d %5.2f %-4s| %x %e %g %s %r %%" % (42, 3.14159, "ab", 255, 12345.678, 0.0001, None, "q"))
print("%05d|%+d|%.3s|%10.4f" % (42, 5, "abcdef", 2.5))
print(f"{12:04d} {3.5:8.3f} {'x':^7}| {1234567:,} {0.25:.1%} {42:x} {'r'!r}")
print("{} and {}".format(1, "two"), "{0}{1}{0}".format("a", "b"), "{x:>4}".format(x=7))
print(1e22, 1e-5, 5e-324, 2.0 ** 60, 123e-10, float(10 ** 20))
print(int(1e20), 10 ** 20 == int(1e20), 3 * 10 ** 15)
import math
print(math.sqrt(16), math.floor(-2.5), math.ceil(2.1), math.pi, math.gcd(12, 18), math.factorial(25))
print(math.isinf(float("inf")), math.cos(0.0), math.log(math.e), math.hypot(3, 4))
big = 1
for k in range(1, 30):
    big = big * k + k
print(big, big % 97, big // 10 ** 20, -big // 3, str(big)[:10], len(str(big)))
a, b = 0, 1
for _ in range(150):
    a, b = b, a + b
print(a, a % 1000000007)
print((2 ** 100) & (2 ** 100 - 1), (2 ** 100) | 1, (2 ** 65) ^ (2 ** 64), -(2 ** 70) & 0xFF)
print(int(2 ** 53 + 1.0), 10 ** 18 * 10 > 2 ** 63, (10 ** 25) / (10 ** 5))
print(sorted([3, 10 ** 20, -5, 2 ** 40]), max(2 ** 64, 2 ** 63), 2 ** 64 - 2 ** 64)
print(f"{2 ** 70}", "%d" % (2 ** 70), "%x" % 255, f"{-(2 ** 40):,}")
import random
random.seed(1234)
print(random.random(), random.randrange(100), random.randrange(10, 20), random.randint(1, 6))
random.seed(1)
print(random.randrange(9223372036854775807), random.choice("abcdef"))
xs = list(range(10))
random.shuffle(xs)
print(xs, random.uniform(1.0, 2.0))
