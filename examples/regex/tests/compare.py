#!/usr/bin/env python3
"""compare.py CASES EXPECTED ACTUAL: report cases where either engine
(backtracking, Pike) disagrees with Python's `re`, captures included.
Exit 1 on a mismatch."""
import sys


def show(h):
    return repr(b"" if h == "-" else bytes.fromhex(h))


cases = open(sys.argv[1]).read().splitlines()
exp = open(sys.argv[2]).read().splitlines()
act = open(sys.argv[3]).read().splitlines()
assert len(cases) == len(exp) == len(act), (len(cases), len(exp), len(act))
bad = 0
for c, e, a in zip(cases, exp, act):
    bt, pk = a.split("\t")
    for name, got in (("backtrack", bt), ("pike", pk)):
        if got != e:
            bad += 1
            if bad <= 30:
                mode, p, s = c.split(" ")
                print("MISMATCH %-9s %s %s on %s: expected %s, got %s" % (name, mode, show(p), show(s), e, got))
print("%d cases, %d mismatches" % (len(cases), bad))
sys.exit(1 if bad else 0)
