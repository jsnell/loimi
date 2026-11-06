#!/usr/bin/env python3
"""myprof.py BIN PROG [ENV=V ...]: SIGPROF samples by symbol, JIT code by LOIMI_JIT_MAP."""
import sys, os, subprocess, bisect, collections
S = os.path.dirname(os.path.abspath(__file__))
WT = "/workspace-loimi/.claude/worktrees/agent-afbf2c667b9e597c2"
binp = sys.argv[1] if sys.argv[1].startswith("/") else os.path.join(S, sys.argv[1])
prog = sys.argv[2]
env = dict(os.environ, LOIMI_STD=WT, LD_PRELOAD=os.path.join(S, "prof", "sampler.so"), PROF_OUT=os.path.join(S, "samples.txt"),
           LOIMI_JIT_MAP=os.path.join(S, "jitmap.txt"))
for e in sys.argv[3:]:
    k, v = e.split("=", 1); env[k] = v
subprocess.run([binp, prog if prog.endswith(".lua") else f"{WT}/bench/mini/programs/{prog}.lua"], env=env, stdout=subprocess.DEVNULL, cwd=WT)
syms = []
for line in subprocess.run(["nm", "-n", "--defined-only", binp], capture_output=True, text=True).stdout.splitlines():
    p = line.split()
    if len(p) == 3 and p[1] in "tTwW":
        syms.append((int(p[0], 16), p[2]))
addrs = [x for x, _ in syms]
jit = []
if os.path.exists(env["LOIMI_JIT_MAP"]):
    for l in open(env["LOIMI_JIT_MAP"]):
        f = l.split()
        if len(f) >= 3:
            jit.append((int(f[0], 16), int(f[1], 16), "jit:" + f[2]))
jit.sort()
jlo = [x[0] for x in jit]
c = collections.Counter()
n = 0
for l in open(env["PROF_OUT"]):
    a = int(l, 16)
    n += 1
    i = bisect.bisect_right(jlo, a) - 1
    if i >= 0 and jit[i][0] <= a < jit[i][1]:
        c[jit[i][2]] += 1
        continue
    i = bisect.bisect_right(addrs, a) - 1
    c[syms[i][1] if i >= 0 and a < 0x1000000 else "?"] += 1
for k, v in c.most_common(int(os.environ.get("TOP", "25"))):
    print(f"{100.0 * v / n:6.1f}%  {k}")
print(n, "samples")
