#!/usr/bin/env python3
"""listing.py DUMP [ADDR-or-index] [LO HI]: the instructions of one compiled code object."""
import sys
codes = []
for line in open(sys.argv[1]):
    f = line.split()
    if f[0] == "code":
        codes.append([line.strip(), []])
    elif codes:
        codes[-1][1].append(line.rstrip())
if len(sys.argv) < 3:
    for i, c in enumerate(codes):
        print(i, c[0])
    sys.exit()
sel = sys.argv[2]
c = codes[int(sel)] if sel.lstrip("-").isdigit() else [c for c in codes if sel in c[0]][-1]
lo, hi = (int(sys.argv[3]), int(sys.argv[4])) if len(sys.argv) > 4 else (0, 1 << 30)
print(c[0])
for l in c[1]:
    f = l.split()
    if lo <= int(f[1]) < hi:
        print(l)
