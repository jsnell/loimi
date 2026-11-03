# Ints around the 48-bit Int payload (2**47) and the medium (i64) tier.
M = 2**47
vals = [0, 1, -1, 2**31 - 1, 2**31, -2**31, -2**31 - 1, M - 1, M, -M, -M - 1, 2**63 - 1, 2**63, -2**63, 3 * M + 5]
for a in vals:
    print(a, -a, ~a, abs(a), a + 1, a - 1, a * 3, a // 7, a % 7, a >> 3, a << 2, a & 0xFFFF, a | 1, a ^ 5, hash(a) == hash(a + 0), a == M, a < M, float(a))
for a in vals[:9]:
    for b in [1, -3, 7, M - 1, -M]:
        print(a + b, a - b, a * b, a // b, a % b, a < b, a == b, divmod(a, b))
s = 0
for i in range(M - 3, M + 3):
    s += i
    print(i, s)
for i in range(-M + 2, -M - 3, -1):
    print(i)
d = {M - 1: 'a', M: 'b', -M: 'c', -M - 1: 'd'}
print(d[M - 1], d[M], d[-M], d[-M - 1], d.get(2**47 - 1 + 1))
x = M - 1
x += 1
print(x, x == M, type(x), str(x), repr(-x), len([0] * 3))
y = -M
y = -y
print(y, -(-M + 1) + 1, 0 - M)
print(sum(range(10**6)), 10**6 * 10**6, (M - 1) * 2, -(M - 1) - 1, -(-M))
print([1, 2, 3][-1], (4, 5)[-2], int("140737488355328"), int(140737488355327.0), 2**46 * 2)
