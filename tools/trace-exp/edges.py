#!/usr/bin/env python3
"""edges.py TAG: the counted internal edges of a region build's profile."""
import json, sys, os
from collections import defaultdict
S = os.path.dirname(os.path.abspath(__file__)) + "/"
t = sys.argv[1]
m = json.load(open(S + t + ".json"))
c = {int(l.split()[0]): int(l.split()[1]) for l in open(S + t + ".prof") if not l.startswith("trace")}
e = defaultdict(int)
for f, i, to in m["edges"]:
    e[(f, to)] += c.get(i, 0)
for (f, to), n in sorted(e.items()):
    if n:
        print(f, "->", to, n)
