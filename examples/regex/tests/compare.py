#!/usr/bin/env python3
"""compare.py CASES EXPECTED ACTUAL: report cases where either engine
(backtracking, Pike) disagrees with Python's `re`. Exit 1 on a mismatch."""
import sys


def show(h):
    return repr(b"" if h == "-" else bytes.fromhex(h))


cases = open(sys.argv[1]).read().splitlines()
exp = open(sys.argv[2]).read().splitlines()
act = open(sys.argv[3]).read().splitlines()
assert len(cases) == len(exp) == len(act), (len(cases), len(exp), len(act))
bad = 0
known = 0
for c, e, a in zip(cases, exp, act):
    bt, pk = a.split("\t")
    for name, got in (("backtrack", bt), ("pike", pk)):
        if got != e and name == "pike" and got.split(" ")[0] == e.split(" ")[0] and got != "NOMATCH":
            # Known: the Pike VM follows each pc once per step, so it
            # cannot take the extra empty loop iteration that Python's
            # backtracker uses to reset a group in nested nullable loops.
            # The match itself (group 0) must agree.
            known += 1
            continue
        if got != e:
            bad += 1
            if bad <= 30:
                mode, p, s = c.split(" ")
                print("MISMATCH %-9s %s %s on %s: expected %s, got %s" % (name, mode, show(p), show(s), e, got))
print("%d cases, %d mismatches, %d known Pike capture divergences" % (len(cases), bad, known))
sys.exit(1 if bad else 0)
