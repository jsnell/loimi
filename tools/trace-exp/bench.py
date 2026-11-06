#!/usr/bin/env python3
"""bench.py -n N PROG LABEL=BIN[:ENV=V,ENV=V] ...: interleaved runs, min user+sys CPU.
Checks every run's output against bench/mini/programs/PROG.expected."""
import os, subprocess, sys, resource

WT = "/workspace-loimi/.claude/worktrees/agent-afbf2c667b9e597c2"
S = os.path.dirname(os.path.abspath(__file__))
args = sys.argv[1:]
n = 5
if args[0] == "-n":
    n = int(args[1]); args = args[2:]
prog = args[0]
src = args[0] if os.path.exists(args[0]) else f"{WT}/bench/mini/programs/{prog}.lua"
expf = src[:-4] + ".expected"
expected = open(expf, "rb").read() if os.path.exists(expf) else None
cfgs = []
for c in args[1:]:
    label, rest = c.split("=", 1)
    binp, _, envs = rest.partition(":")
    env = dict(os.environ)
    env["LOIMI_STD"] = WT
    for e in filter(None, envs.split(",")):
        k, v = e.split("=", 1)
        env[k] = v
    cfgs.append((label, binp if binp.startswith("/") else f"{S}/{binp}", env))


def run(binp, env):
    p = subprocess.Popen([binp, src], stdout=subprocess.PIPE, stderr=subprocess.PIPE, env=env, cwd=WT)
    out, err = p.communicate()
    ru = resource.getrusage(resource.RUSAGE_CHILDREN)
    return p.returncode, out, err, ru


best = {l: None for l, _, _ in cfgs}
allt = {l: [] for l, _, _ in cfgs}
prev = resource.getrusage(resource.RUSAGE_CHILDREN)
for i in range(n):
    for label, binp, env in cfgs:
        rc, out, err, ru = run(binp, env)
        t = (ru.ru_utime - prev.ru_utime) + (ru.ru_stime - prev.ru_stime)
        prev = ru
        if rc != 0 or (expected is not None and out != expected):
            print(f"{label}: FAIL rc={rc}\n{out[:300]!r}\n{err[:1500].decode(errors='replace')}")
            best[label] = "FAIL"
            continue
        if best[label] != "FAIL":
            best[label] = t if best[label] is None else min(best[label], t)
        allt[label].append(t)
load = open("/proc/loadavg").read().split()[:3]
base = cfgs[0][0]
print(f"{prog}  n={n}  load {' '.join(load)}")
for label, _, _ in cfgs:
    b = best[label]
    if b == "FAIL":
        print(f"  {label:<14} FAIL")
        continue
    r = b / best[base] if isinstance(best[base], float) else 0
    med = sorted(allt[label])[len(allt[label]) // 2]
    print(f"  {label:<14} {b:7.3f}s  {r:5.3f}  median {med:.3f}  [{' '.join('%.3f' % x for x in allt[label])}]")
