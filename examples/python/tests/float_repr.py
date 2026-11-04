# repr of floats from 1e16 on uses an exponent; round() of bools.

for x in [2**31 + 1e16, 1e16, -1e16, 1.5e16, 123456789012345678.0, -98765432109876543210.0, 9999999999999998.0, 1e22, 2.0**60, round(False, 2), round(True), divmod(2**53 + 1, 1/3)]:
    print(repr(x), str(x), [x])
print(f"{1.0000002147483648e+16}", "%r" % 1e17, str(-1.2e16))
