#!/usr/bin/env python3
"""merge.py OUT A.ll B.ll ...: several generated traces in one LOIMI_TRACE_IR file."""
import sys, re
out, ins = sys.argv[1], sys.argv[2:]
decls, body, entries = [], [], []
seen = set()
TY = "{ i64, i64, i64, i64, i64, ptr, ptr, i64, ptr }"
for f in ins:
    for l in open(f):
        if l.startswith("@loimi_ntraces"):
            continue
        if l.startswith("@loimi_traces"):
            entries.append(l[l.index("] [") + 3:].rstrip().rstrip("]")[len(TY) + 1:])
            continue
        if l.startswith("declare "):
            if l not in seen:
                seen.add(l)
                decls.append(l)
            continue
        body.append(l)
with open(out, "w") as o:
    o.writelines(decls)
    o.writelines(body)
    o.write("@loimi_traces = constant [%d x %s] [%s]\n" % (len(entries), TY, ", ".join("%s %s" % (TY, e) for e in entries)))
    o.write("@loimi_ntraces = constant i64 %d\n" % len(entries))
