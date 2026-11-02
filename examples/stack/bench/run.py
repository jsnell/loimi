#!/usr/bin/env python3
"""Time Stak's benchmarks: best of N wall-clock runs per binary, interleaved.

    examples/stack/bench/run.py [-n 5] BINARY [BINARY ...] [-- extra args]
"""
import argparse, glob, os, subprocess, sys, time

here = os.path.dirname(os.path.abspath(__file__))
ap = argparse.ArgumentParser()
ap.add_argument('-n', type=int, default=3)
ap.add_argument('bins', nargs='+')
args, extra = ap.parse_known_args()
progs = sorted(glob.glob(os.path.join(here, '*.stk')))
best = {}
for _ in range(args.n):
    for p in progs:
        for b in args.bins:
            t = time.perf_counter()
            r = subprocess.run([os.path.abspath(b), p] + extra, capture_output=True, text=True)
            dt = time.perf_counter() - t
            if r.returncode != 0:
                sys.exit(f'{b} {p}: exit {r.returncode}\n{r.stderr}')
            k = (os.path.basename(p), b)
            best[k] = min(best.get(k, 1e9), dt)
print('benchmark'.ljust(14) + ''.join(os.path.basename(b).rjust(14) for b in args.bins))
for p in progs:
    n = os.path.basename(p)
    print(n.ljust(14) + ''.join(f'{best[(n, b)]:14.3f}' for b in args.bins))
