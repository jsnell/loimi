#!/usr/bin/env python3
"""annot.py BIN [MIN%]: annotate BIN.asm (dis.sh output) with samples.txt (last myprof run).
Prints instructions with samples and the count of instructions covering 99% of them."""
import sys, collections, os
S = os.path.dirname(os.path.abspath(__file__))
b = sys.argv[1]
c = collections.Counter(int(l, 16) for l in open(os.path.join(S, "samples.txt")))
lines = [l.rstrip() for l in open(os.path.join(S, b + ".asm")) if l.startswith("  ")]
tot = 0
rows = []
for l in lines:
    a = int(l.split(":")[0], 16)
    rows.append((a, c.get(a, 0), l))
    tot += c.get(a, 0)
allsamp = sum(c.values())
print(f"{tot} of {allsamp} samples in {b}")
hot = [r for r in rows if r[1] > 0]
# the instruction range that holds the hot loop: contiguous span of rows with samples
for a, n, l in rows:
    if n * 1000 >= tot * float(sys.argv[2] if len(sys.argv) > 2 else 2):
        print(f"{100.0 * n / tot:5.1f}%  {l}")
