#!/usr/bin/env python3
"""Differential test of std.bigint against Python's int.

    diff.py LOIMI [--seed N] [--cases N] [--big] [--stress] [--keep DIR]

Writes random cases (fixed seed), builds and runs tests/bigint/calc.lm
with LOIMI, and compares every result line with Python's. `--cases`
sets the count per run (default 3000, a few seconds), `--big` adds
operands of thousands of limbs (Karatsuba recursion, divide-and-conquer
formatting), `--stress` runs with the GC collecting at every safepoint.
tests/golden.rs (bigint_vs_python) runs the default; longer runs are
manual, e.g. `diff.py target/release/loimi --seed 7 --cases 200000 --big`.
Exit status 0 when everything matches.
"""
import os
import random
import struct
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))


def hexs(x):
    return ('-' if x < 0 else '') + format(abs(x), 'x')


def rand_int(rng, big):
    """A random integer: mostly limb-aligned shapes that find carries,
    borrows and normalization bugs, sometimes plain random bits."""
    r = rng.random()
    if r < 0.15:
        x = rng.choice([0, 1, 2, 3, 2**63 - 1, 2**63, 2**64 - 1, 2**64, 2**64 + 1, 10**19, 10**19 - 1, 2**128 - 1])
    elif r < 0.45:
        x = rng.getrandbits(rng.randint(1, 200))
    elif r < 0.8:
        n = rng.randint(1, 80)
        x = 0
        for _ in range(n):
            x = (x << 64) | rng.choice([0, 1, 2**63, 2**64 - 1, 2**64 - 2, rng.getrandbits(64), rng.getrandbits(64)])
    else:
        hi = 6000 if big else 400
        x = rng.getrandbits(64 * rng.randint(1, hi))
        if rng.random() < 0.3:
            x |= (2**64 - 1) << (64 * rng.randint(0, 3))
    return -x if rng.random() < 0.5 else x


def divisor(rng, a, big):
    """Divisors close to the dividend's size, with patterns that make
    Knuth's qhat estimate too large (the add-back step)."""
    r = rng.random()
    if r < 0.3 and a != 0:
        n = max(1, abs(a).bit_length() // 64 - rng.randint(0, 3))
        top = rng.choice([2**63, 2**63 + 1, 2**64 - 1, 2**63 - 1, 1])
        x = top << (64 * (n - 1))
        x += rng.choice([0, 1, (1 << (64 * (n - 1))) - 1, rng.getrandbits(64 * (n - 1) or 1)])
        return -x if rng.random() < 0.5 else x
    return rand_int(rng, big)


def floor_divmod(a, b):
    return divmod(a, b)


def trunc_divmod(a, b):
    q = abs(a) // abs(b)
    if (a < 0) != (b < 0):
        q = -q
    return q, a - q * b


def f64_bits(f):
    return struct.unpack('<Q', struct.pack('<d', f))[0]


def tof(x):
    try:
        return format(f64_bits(float(x)), 'x')
    except OverflowError:
        return 'inf'


def gen(rng, n, big):
    cases = []
    ops = ['add', 'sub', 'mul', 'and', 'or', 'xor', 'fdiv', 'tdiv', 'cmp', 'not', 'pow', 'shl', 'shr',
           'hash', 'bitlen', 'i64', 'tof', 'fromf', 'radix',
           'gadd', 'gsub', 'gmul', 'gand', 'gor', 'gxor', 'gfdiv', 'gtdiv', 'gnot', 'gpow', 'gshl', 'gshr']
    for _ in range(n):
        op = rng.choice(ops)
        a = rand_int(rng, big)
        if op in ('add', 'sub', 'mul', 'and', 'or', 'xor', 'cmp', 'gadd', 'gsub', 'gmul', 'gand', 'gor', 'gxor'):
            b = rand_int(rng, big) if rng.random() < 0.8 else a + rng.choice([-1, 0, 1])
            if op in ('cmp',) and rng.random() < 0.3:
                b = a
            cases.append((op, a, b))
        elif op in ('fdiv', 'tdiv', 'gfdiv', 'gtdiv'):
            b = divisor(rng, a, big) if rng.random() < 0.97 else 0
            cases.append((op, a, b))
        elif op in ('pow', 'gpow'):
            a = rng.choice([a % (1 << 200), rng.randint(-20, 20), rng.getrandbits(64)])
            if rng.random() < 0.5:
                a = -a
            e = rng.randint(0, 40) if abs(a) > 2**64 else rng.randint(0, 300)
            cases.append((op, a, e))
        elif op in ('shl', 'shr', 'gshl', 'gshr'):
            cases.append((op, a, rng.choice([0, 1, 63, 64, 65, 127, 128, rng.randint(0, 3000)])))
        elif op == 'tof':
            r = rng.random()
            if r < 0.5:
                # Near ties: m * 2^k plus half an ulp, plus or minus a little.
                k = rng.randint(1, 1000)
                m = rng.getrandbits(53) | (1 << 52)
                x = (m << k) + (1 << (k - 1)) + rng.choice([-1, 0, 0, 1, rng.getrandbits(max(1, k - 2))])
                a = -x if rng.random() < 0.5 else x
            elif r < 0.6:
                a = rng.choice([1, -1]) * rng.choice([2**1024 - 1, 2**1024 - 2**970, 2**1024 - 2**971, 2**1023])
            cases.append((op, a, None))
        elif op == 'fromf':
            while True:
                bits = rng.getrandbits(64)
                if rng.random() < 0.7:
                    e = rng.randint(1000, 1100)
                    bits = (bits & ~(0x7ff << 52)) | (e << 52)
                if (bits >> 52) & 0x7ff != 0x7ff or rng.random() < 0.02:
                    break
            cases.append((op, bits, None))
        elif op == 'radix':
            cases.append((op, a, rng.choice([2, 3, 7, 8, 10, 10, 10, 16, 32, 36, rng.randint(2, 36)])))
        else:
            cases.append((op, a, None))
    return cases


def expect(op, a, b):
    if op in ('add', 'gadd'):
        return str(a + b)
    if op in ('sub', 'gsub'):
        return str(a - b)
    if op in ('mul', 'gmul'):
        return str(a * b)
    if op in ('and', 'gand'):
        return str(a & b)
    if op in ('or', 'gor'):
        return str(a | b)
    if op in ('xor', 'gxor'):
        return str(a ^ b)
    if op in ('fdiv', 'gfdiv', 'tdiv', 'gtdiv'):
        if b == 0:
            return 'zerodiv'
        q, r = floor_divmod(a, b) if 'f' in op else trunc_divmod(a, b)
        return '%d %d' % (q, r)
    if op == 'cmp':
        return str((a > b) - (a < b))
    if op in ('not', 'gnot'):
        return str(~a)
    if op in ('pow', 'gpow'):
        return str(a ** b)
    if op in ('shl', 'gshl'):
        return str(a << b)
    if op in ('shr', 'gshr'):
        return str(a >> b)
    if op == 'hash':
        return str(hash(a))
    if op == 'bitlen':
        return str(a.bit_length())
    if op == 'i64':
        return str(a) if -2**63 <= a < 2**63 else 'none'
    if op == 'tof':
        return tof(a)
    if op == 'fromf':
        f = struct.unpack('<d', struct.pack('<Q', a))[0]
        if f != f or f in (float('inf'), float('-inf')):
            return 'none'
        return str(int(f))
    if op == 'radix':
        return to_radix(a, b)
    raise ValueError(op)


def to_radix(x, r):
    if x == 0:
        return '0'
    digits = '0123456789abcdefghijklmnopqrstuvwxyz'
    if r == 10:
        return str(x)
    if r == 16:
        return hexs(x)
    if r == 8:
        return ('-' if x < 0 else '') + format(abs(x), 'o')
    if r == 2:
        return ('-' if x < 0 else '') + format(abs(x), 'b')
    n = abs(x)
    # Divide and conquer, so that huge numbers stay fast here too.
    out = []

    def rec(n, pad):
        if n.bit_length() < 2000:
            s = ''
            while n:
                n, d = divmod(n, r)
                s = digits[d] + s
            out.append(s.rjust(pad, '0'))
            return
        k = 1
        while r ** (2 * k) <= n:
            k *= 2
        q, m = divmod(n, r ** k)
        rec(q, max(0, pad - k))
        rec(m, k)

    rec(n, 0)
    return ('-' if x < 0 else '') + ''.join(out)


def main():
    args = sys.argv[1:]
    if not args:
        print(__doc__)
        return 2
    loimi = os.path.abspath(args[0])
    seed, n, big, stress, keep = 1, 3000, False, False, None
    i = 1
    while i < len(args):
        if args[i] == '--seed':
            seed = int(args[i + 1]); i += 1
        elif args[i] == '--cases':
            n = int(args[i + 1]); i += 1
        elif args[i] == '--big':
            big = True
        elif args[i] == '--stress':
            stress = True
        elif args[i] == '--keep':
            keep = args[i + 1]; i += 1
        i += 1
    sys.set_int_max_str_digits(0)
    rng = random.Random(seed)
    cases = gen(rng, n, big)
    tmp = keep or tempfile.mkdtemp(prefix='bigint-diff-')
    os.makedirs(tmp, exist_ok=True)
    path = os.path.join(tmp, 'cases.txt')
    with open(path, 'w') as f:
        for op, a, b in cases:
            if op in ('pow', 'gpow', 'shl', 'gshl', 'shr', 'gshr', 'radix'):
                f.write('%s %s %d\n' % (op, hexs(a), b))
            elif b is None:
                f.write('%s %s\n' % (op, hexs(a)))
            else:
                f.write('%s %s %s\n' % (op, hexs(a), hexs(b)))
    exe = os.path.join(tmp, 'calc')
    env = dict(os.environ)
    env.setdefault('LOIMI_STD', os.path.dirname(os.path.dirname(HERE)))
    b = subprocess.run([loimi, 'build', os.path.join(HERE, 'calc.lm'), '--release', '-o', exe], env=env)
    if b.returncode != 0:
        print('build failed')
        return 1
    r = subprocess.run([exe, path, '1' if stress else '0'], stdout=subprocess.PIPE, text=True)
    if r.returncode != 0:
        print('calc exited with %d' % r.returncode)
        return 1
    got = r.stdout.split('\n')
    bad = 0
    for k, (op, a, b) in enumerate(cases):
        want = expect(op, a, b)
        g = got[k] if k < len(got) else '<missing>'
        if g != want:
            bad += 1
            if bad <= 5:
                print('case %d: %s %s %s' % (k, op, hexs(a), '' if b is None else (hexs(b) if not isinstance(b, int) or op not in ('pow', 'gpow', 'shl', 'gshl', 'shr', 'gshr', 'radix') else b)))
                print('  want %s' % want[:300])
                print('  got  %s' % g[:300])
    print('%d cases, %d mismatches (seed %d%s%s)' % (len(cases), bad, seed, ', big' if big else '', ', gc stress' if stress else ''))
    return 1 if bad else 0


if __name__ == '__main__':
    sys.exit(main())
