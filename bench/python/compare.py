#!/usr/bin/env python3
"""Compare a Loimi-built Python interpreter against CPython and MicroPython.

Runs every bench/python/programs/<name>.py under each interpreter, best of N
wall-clock runs (interleaved: run 1 of every interpreter, then run 2, ...),
checks stdout against <name>.expected, and prints a markdown table.

  bench/python/compare.py [--loimi CMD] [-n N] [--only a,b]

--loimi/--cpython/--micropython are commands (shell-split) that take the .py
path as their last argument; loimi defaults to examples/python/python-loimi.
A missing interpreter is skipped (e.g. no loimi binary -> references only).
Children run with `ulimit -v 2500000` (RLIMIT_AS) and a 600 s timeout.

Known MicroPython mismatches (expected; outputs are CPython's): chaos and go
use `random` (different generator), nbody differs in the last float digits.
"""
import argparse
import math
import os
import resource
import shlex
import shutil
import subprocess
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))
PROGRAMS = os.path.join(HERE, "programs")
MEM_LIMIT = 2500000 * 1024  # same as `ulimit -v 2500000`
TIMEOUT = 600


def limit_memory():
    resource.setrlimit(resource.RLIMIT_AS, (MEM_LIMIT, MEM_LIMIT))


def available(cmd):
    exe = cmd[0]
    if os.sep in exe:
        return os.path.isfile(exe) and os.access(exe, os.X_OK)
    return shutil.which(exe) is not None


def run(cmd, prog):
    """Returns (seconds, stdout) or (None, error-description)."""
    t0 = time.perf_counter()
    try:
        p = subprocess.run(cmd + [prog], stdout=subprocess.PIPE,
                           stderr=subprocess.PIPE, preexec_fn=limit_memory,
                           timeout=TIMEOUT, cwd=PROGRAMS)
    except subprocess.TimeoutExpired:
        return None, "timeout"
    except OSError as e:
        return None, str(e)
    dt = time.perf_counter() - t0
    if p.returncode != 0:
        err = p.stderr.decode(errors="replace").strip().splitlines()
        return None, "exit %d: %s" % (p.returncode, err[-1] if err else "")
    return dt, p.stdout.decode(errors="replace")


def main():
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--loimi", default=os.path.join(ROOT, "examples/python/python-loimi"))
    ap.add_argument("--cpython", default=os.path.join(HERE, "install/bin/python3"))
    # MicroPython's unix port has a fixed GC heap (default too small for
    # float.py); give it a large one, like CPython's unbounded heap.
    ap.add_argument("--micropython", default=os.path.join(HERE, "install/bin/micropython")
                    + " -X heapsize=256M")
    ap.add_argument("-n", type=int, default=3, help="runs per program (best of N)")
    ap.add_argument("--only", help="comma-separated benchmark names")
    args = ap.parse_args()

    names = sorted(f[:-3] for f in os.listdir(PROGRAMS) if f.endswith(".py"))
    if args.only:
        want = args.only.split(",")
        unknown = [w for w in want if w not in names]
        if unknown:
            sys.exit("unknown benchmark(s): " + ", ".join(unknown))
        names = want

    interps = []
    for label, cmd in (("loimi", args.loimi), ("cpython", args.cpython),
                       ("micropython", args.micropython)):
        argv = shlex.split(cmd)
        if os.sep in argv[0]:
            argv[0] = os.path.abspath(argv[0])  # children run in programs/
        if available(argv):
            interps.append((label, argv))
        else:
            print("note: %s not found (%s); skipped" % (label, cmd), file=sys.stderr)
    labels = [l for l, _ in interps]

    rows = []
    for name in names:
        prog = os.path.join(PROGRAMS, name + ".py")
        with open(os.path.join(PROGRAMS, name + ".expected")) as f:
            expected = f.read()
        best = {l: None for l in labels}
        note = {l: "" for l in labels}
        for _ in range(args.n):
            for label, argv in interps:
                if note[label] and best[label] is None:
                    continue  # already failed; don't retry
                dt, out = run(argv, prog)
                if dt is None:
                    note[label] = out
                    continue
                if out != expected:
                    note[label] = "mismatch"
                if best[label] is None or dt < best[label]:
                    best[label] = dt
        rows.append((name, best, note))
        print("%-14s " % name + "  ".join(
            "%s=%s%s" % (l, "-" if best[l] is None else "%.3f" % best[l],
                         " (%s)" % note[l] if note[l] else "") for l in labels),
              file=sys.stderr, flush=True)

    ratios = {"cpython": [], "micropython": []}
    hdr = ["benchmark"] + ["%s (s)" % l for l in labels]
    have_loimi = "loimi" in labels
    if have_loimi:
        hdr += ["loimi/%s" % r for r in ratios if r in labels]
    print("| " + " | ".join(hdr) + " |")
    print("|" + "---|" * len(hdr))
    for name, best, note in rows:
        cells = [name]
        for l in labels:
            if best[l] is None:
                cells.append("FAIL (%s)" % note[l])
            else:
                cells.append("%.3f%s" % (best[l], " MISMATCH" if note[l] else ""))
        if have_loimi:
            for r in ratios:
                if r not in labels:
                    continue
                ok = all(best[x] is not None and not note[x] for x in ("loimi", r))
                if ok:
                    q = best["loimi"] / best[r]
                    ratios[r].append(q)
                    cells.append("%.2f" % q)
                else:
                    cells.append("-")
        print("| " + " | ".join(cells) + " |")
    if have_loimi:
        cells = ["geomean"] + [""] * len(labels)
        for r in ratios:
            if r not in labels:
                continue
            qs = ratios[r]
            cells.append("%.2f (%d)" % (math.exp(sum(map(math.log, qs)) / len(qs)), len(qs))
                         if qs else "-")
        print("| " + " | ".join(cells) + " |")


if __name__ == "__main__":
    main()
