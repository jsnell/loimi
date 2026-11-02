#!/usr/bin/env python3
"""Run the clean-room MiniJS benchmarks under MiniJS, node --jitless, node and QuickJS.

Usage: bench/minijs/compare.py [-n RUNS] [--dispatch MODE ...] [NAME ...]

Each benchmark bench/minijs/programs/NAME.js must print exactly
NAME.expected. A run with wrong output is reported as FAIL, not timed.
Times are the minimum wall-clock time over RUNS runs, in seconds.
node must be on PATH; QuickJS comes from bench/js/install.sh (optional).
"""
import argparse, os, subprocess, sys, time

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
PROGS = os.path.join(ROOT, "bench", "minijs", "programs")
QJS = os.path.join(ROOT, "bench", "js", "install", "bin", "qjs")
OUT = os.path.join(os.environ.get("TMPDIR", "/tmp"), "loimi-minijs-bench")


def build_mini(dispatch, align=None):
    exe = os.path.join(OUT, f"minijs-{dispatch}")
    subprocess.run([os.path.join(ROOT, "target", "release", "loimi"), "build",
                    os.path.join(ROOT, "examples", "minijs", "main.lm"), "--release",
                    f"--dispatch={dispatch}", "-o", exe] + ([f"--align-functions={align}"] if align else []), check=True)
    return exe


def run(cmd, expected, runs):
    best = None
    for _ in range(runs):
        t = time.perf_counter()
        p = subprocess.run(cmd, capture_output=True)
        dt = time.perf_counter() - t
        if p.returncode != 0 or p.stdout != expected:
            return "FAIL"
        best = dt if best is None else min(best, dt)
    return best


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("-n", type=int, default=3)
    ap.add_argument("--dispatch", nargs="*", default=["tailcall"])
    ap.add_argument("--align", type=int, help="build with --align-functions=N (steadier A/B comparisons)")
    ap.add_argument("names", nargs="*")
    a = ap.parse_args()
    os.makedirs(OUT, exist_ok=True)
    subprocess.run(["cargo", "build", "--release", "-q"], cwd=ROOT, check=True)
    impls = [(f"minijs-{d}", [build_mini(d, a.align)]) for d in a.dispatch]
    impls.append(("node-jitless", ["node", "--jitless"]))
    impls.append(("node", ["node"]))
    if os.path.exists(QJS):
        impls.append(("qjs", [QJS]))
    names = a.names or sorted(f[:-3] for f in os.listdir(PROGS) if f.endswith(".js"))
    print(f"{'benchmark':<16}" + "".join(f"{n:>14}" for n, _ in impls) + f"{'mjs/jitless':>12}")
    for b in names:
        src = os.path.join(PROGS, b + ".js")
        with open(os.path.join(PROGS, b + ".expected"), "rb") as f:
            expected = f.read()
        times = [run(cmd + [src], expected, a.n) for _, cmd in impls]
        cells = "".join(f"{t:>14}" if isinstance(t, str) else f"{t:>13.3f}s" for t in times)
        ratio = ""
        if isinstance(times[0], float) and len(times) > len(a.dispatch) and isinstance(times[len(a.dispatch)], float):
            ratio = f"{times[0] / times[len(a.dispatch)]:>12.2f}"
        print(f"{b:<16}{cells}{ratio}", flush=True)


if __name__ == "__main__":
    main()
