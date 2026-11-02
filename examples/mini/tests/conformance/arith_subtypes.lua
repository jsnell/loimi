-- int op int -> int ; any float -> float ; / and ^ always float
print(1 + 1, 1 + 1.0, 1.0 + 1)
print(3 - 1, 3 - 1.0, 2 * 3, 2 * 3.0)
print(7 % 3, 7 % 3.0, 7.0 % 3)
print(4 / 2, 4 ^ 1, 9 ^ 0.5)
print(-(2), -(2.0))
print(1e2 + 0, 100 + 0)
local a = 10
local b = a / 5
print(a, b, a == b)
-- floats with integral values compare equal to integers
print(2 == 2.0, 2 < 2.5, 3 > 2.5, 2 <= 2.0, -1 >= -1.0)
print(2147483647 + 0.0, (-2147483647 - 1) + 0.0)
