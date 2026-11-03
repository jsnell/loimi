# Arbitrary-precision ints (std.bigint underneath).

def fact(n):
    r = 1
    for i in range(2, n + 1):
        r *= i
    return r


f = fact(60)
print(f)
print(f // fact(58), f % 1000000007, -f // 7, -f % 7)
print(divmod(f, -(10 ** 30)), divmod(-f, 10 ** 30))
print(2 ** 200, (-3) ** 41, 7 ** 0, 0 ** 0)
print(2 ** 100 - 2 ** 99 == 2 ** 99, 2 ** 64 > 2 ** 63, -(2 ** 70) < -(2 ** 69))
print(1 << 100, (1 << 100) >> 37, -(1 << 100) >> 3, -1 >> 100, -5 >> 1)
x = 2 ** 130 + 12345
print(x & 0xffffffff, x | -1, -x & 0xffff, -x ^ 2 ** 64, ~x, ~-x)
print(hash(2 ** 70) == hash(2 ** 70), hash(2 ** 70) == hash(2 ** 71))
print(hex(2 ** 100 + 255), oct(-(8 ** 30)), bin(2 ** 70 + 1))
print(int("123456789012345678901234567890"), int("-" + "ff" * 10, 16), int("1_000_000_000_000_000_000_000"))
print(str(-(10 ** 50)), repr(3 ** 100))
print(float(2 ** 80), float(-(3 ** 60)), int(1e25), int(-2.5e30))
print((2 ** 53 + 1) * 1.0, float(2 ** 1023 * 3 // 2))
print(abs(-(10 ** 40)), -(-(10 ** 40)), (10 ** 40).bit_length())
s = 0
for i in range(100):
    s += 3 ** i
print(s, s % 997)
a, b = 0, 1
for i in range(300):
    a, b = b, a + b
print(a)
print(9223372036854775807 + 1, -9223372036854775808 - 1, 9223372036854775807 * 9223372036854775807)
print(2 ** 63 // 3, (2 ** 63) % 10, -(2 ** 63) // -1)
print(f'{2 ** 70:,}', f'{2 ** 70:x}', f'{-(2 ** 70):X}', '%d' % (5 ** 30))
d = {2 ** 70: 'a', 10 ** 20: 'b'}
print(d[2 ** 70], d[10 ** 20], 2 ** 70 in d, 2 ** 71 in d)
print(sorted([3 ** 50, -(2 ** 80), 7, 2 ** 64]))
print(max(2 ** 100, 3 ** 60), min(-(2 ** 100), -(3 ** 60)))
print(pow(3, 200, 1000000007) if False else 3 ** 200 % 1000000007)
