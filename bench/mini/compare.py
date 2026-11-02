#!/usr/bin/env python3
"""Run the clean-room Mini benchmarks under Mini, PUC-Lua and LuaJIT.

Usage: bench/mini/compare.py [-n RUNS] [--dispatch MODE ...] [--exe BIN ...]
       [--refs REF,...] [NAME ...]

Each benchmark bench/mini/programs/NAME.lua must print exactly
NAME.expected. A run with wrong output is reported as FAIL, not timed.
Times are the minimum wall-clock time over RUNS runs, in seconds.
Reference interpreters come from bench/lua/install.sh.
"""
import argparse, os, subprocess, sys, time

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
PROGS = os.path.join(ROOT, "bench", "mini", "programs")
LUA = os.path.join(ROOT, "bench", "lua", "install", "bin")
OUT = os.path.join(os.environ.get("TMPDIR", "/tmp"), "loimi-mini-bench")


def build_mini(dispatch, align=None):
    exe = os.path.join(OUT, f"mini-{dispatch}")
    subprocess.run([os.path.join(ROOT, "target", "release", "loimi"), "build",
                    os.path.join(ROOT, "examples", "mini", "main.lm"), "--release",
                    f"--dispatch={dispatch}", "-o", exe] + ([f"--align-functions={align}"] if align else []), check=True)
    return exe


def run(cmds, expected, runs):
    """The best time of each command over `runs` runs, or "FAIL". Runs are
    interleaved (one run of each command in turn), so slow drifts in
    machine load affect all columns alike."""
    best = [None] * len(cmds)
    for _ in range(runs):
        for k, cmd in enumerate(cmds):
            if best[k] == "FAIL":
                continue
            t = time.perf_counter()
            p = subprocess.run(cmd, capture_output=True)
            dt = time.perf_counter() - t
            if p.returncode != 0 or p.stdout != expected:
                best[k] = "FAIL"
            else:
                best[k] = dt if best[k] is None else min(best[k], dt)
    return best


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("-n", type=int, default=3)
    ap.add_argument("--dispatch", nargs="*", default=["tailcall"])
    ap.add_argument("--align", type=int, help="build with --align-functions=N (steadier A/B comparisons)")
    ap.add_argument("--exe", nargs="*", default=[],
                    help="extra prebuilt binaries of ours to run as columns (A/B runs)")
    ap.add_argument("--refs", default=None,
                    help="comma-separated reference implementations to run (default: all; '': none)")
    ap.add_argument("names", nargs="*")
    a = ap.parse_args()
    os.makedirs(OUT, exist_ok=True)
    subprocess.run(["cargo", "build", "--release", "-q"], cwd=ROOT, check=True)
    impls = [(f"mini-{d}", [build_mini(d, a.align)]) for d in a.dispatch]
    impls += [(os.path.basename(e), [os.path.abspath(e)]) for e in a.exe]
    ours = len(impls)
    refs = [("lua", ["lua"]), ("luajit-joff", ["luajit", "-joff"]), ("luajit", ["luajit"])]
    if a.refs is not None:
        refs = [r for r in refs if r[0] in a.refs.split(",")]
    for name, args in refs:
        exe = os.path.join(LUA, args[0])
        if os.path.exists(exe):
            impls.append((name, [exe] + args[1:]))
        else:
            print(f"note: {exe} missing; run bench/lua/install.sh", file=sys.stderr)
    names = a.names or sorted(f[:-4] for f in os.listdir(PROGS) if f.endswith(".lua"))
    print(f"{'benchmark':<16}" + "".join(f"{n:>14}" for n, _ in impls) + f"{'mini/lua':>10}")
    for b in names:
        src = os.path.join(PROGS, b + ".lua")
        with open(os.path.join(PROGS, b + ".expected"), "rb") as f:
            expected = f.read()
        times = run([cmd + [src] for _, cmd in impls], expected, a.n)
        cells = "".join(f"{t:>14}" if isinstance(t, str) else f"{t:>13.3f}s" for t in times)
        ratio = ""
        if isinstance(times[0], float) and len(times) > ours and isinstance(times[ours], float):
            ratio = f"{times[0] / times[ours]:>10.2f}"
        print(f"{b:<16}{cells}{ratio}", flush=True)


if __name__ == "__main__":
    main()
