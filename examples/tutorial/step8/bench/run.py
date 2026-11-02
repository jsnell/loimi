#!/usr/bin/env python3
"""Time the tutorial's benchmarks on some of its steps.

    examples/tutorial/step8/bench/run.py [-n RUNS] [STEP ...] [-- LOIMI-FLAGS ...]

A STEP is a step's name (step7) or a directory holding a main.lm. Builds
each step's main.lm with --release (plus any flags after --, such as
--dispatch=goto) and prints the best of RUNS wall-clock times (default 3)
per benchmark, in seconds. Steps default to step7 and step8.
"""

import os
import resource
import subprocess
import sys
import tempfile
import time

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", "..", "..", ".."))
LOIMI = os.environ.get("LOIMI", os.path.join(ROOT, "target", "release", "loimi"))
BENCH = os.path.join(ROOT, "examples", "tutorial", "step8", "bench")


def limit_memory():
    resource.setrlimit(resource.RLIMIT_AS, (2500 << 20, 2500 << 20))


def main():
    args = sys.argv[1:]
    runs, steps, flags = 3, [], []
    while args:
        a = args.pop(0)
        if a == "-n":
            runs = int(args.pop(0))
        elif a == "--":
            flags, args = args, []
        else:
            steps.append(a)
    steps = steps or ["step7", "step8"]
    env = dict(os.environ, LOIMI_STD=os.environ.get("LOIMI_STD", ROOT))
    with tempfile.TemporaryDirectory() as tmp:
        exes = {}
        for s in steps:
            exe = os.path.join(tmp, str(len(exes)))
            d = s if "/" in s else os.path.join(ROOT, "examples", "tutorial", s)
            src = os.path.join(d, "main.lm")
            subprocess.run([LOIMI, "build", src, "--release", *flags, "-o", exe],
                           env=env, check=True, preexec_fn=limit_memory)
            exes[s] = exe
        print("%-10s" % "benchmark" + "".join("%10s" % os.path.basename(s.rstrip("/")) for s in steps))
        for name in sorted(f for f in os.listdir(BENCH) if f.endswith(".pip")):
            # Interleaved, so that a change in machine load hits every
            # step alike.
            best = {}
            for _ in range(runs):
                for s in steps:
                    t = time.perf_counter()
                    subprocess.run([exes[s], os.path.join(BENCH, name)], stdout=subprocess.DEVNULL,
                                   check=True, preexec_fn=limit_memory)
                    t = time.perf_counter() - t
                    best[s] = min(best.get(s, t), t)
            print("%-10s" % name[:-4] + "".join("%10.3f" % best[s] for s in steps), flush=True)


main()
