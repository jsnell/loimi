#!/usr/bin/env python3
"""Time the coroutine benchmarks under a Mini binary, PUC-Lua and LuaJIT.

Usage: bench/mini/coroutines/run.py [--mini PATH] [-n RUNS] [--lua-dir DIR] [NAME ...]

Each bench/mini/coroutines/NAME.lua must print exactly NAME.expected; a run
with wrong output is reported as FAIL, not timed. Times are the minimum
wall-clock time over RUNS runs (default 3), in seconds. Reference
interpreters come from bench/lua/install.sh (lua, and luajit -joff), or
from --lua-dir (e.g. the main checkout's bench/lua/install/bin when
running from a git worktree).
"""
import argparse, os, subprocess, sys, time

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.abspath(os.path.join(HERE, "..", "..", ".."))
LUA = os.path.join(ROOT, "bench", "lua", "install", "bin")


def run(cmds, expected, runs):
    """Best time of each command over `runs` interleaved runs, or "FAIL"."""
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
    ap.add_argument("--mini", help="Mini interpreter binary to time")
    ap.add_argument("-n", type=int, default=3)
    ap.add_argument("--lua-dir", default=LUA, help="directory holding lua and luajit")
    ap.add_argument("names", nargs="*")
    a = ap.parse_args()
    impls = []
    if a.mini:
        impls.append(("mini", [os.path.abspath(a.mini)]))
    for name, args in [("lua", ["lua"]), ("luajit-joff", ["luajit", "-joff"])]:
        exe = os.path.join(a.lua_dir, args[0])
        if os.path.exists(exe):
            impls.append((name, [exe] + args[1:]))
        else:
            print(f"note: {exe} missing; run bench/lua/install.sh", file=sys.stderr)
    names = a.names or sorted(f[:-4] for f in os.listdir(HERE) if f.endswith(".lua"))
    ratio = a.mini is not None and len(impls) > 1 and impls[1][0] == "lua"
    print(f"{'benchmark':<16}" + "".join(f"{n:>14}" for n, _ in impls)
          + (f"{'mini/lua':>10}" if ratio else ""))
    for b in names:
        src = os.path.join(HERE, b + ".lua")
        with open(os.path.join(HERE, b + ".expected"), "rb") as f:
            expected = f.read()
        times = run([cmd + [src] for _, cmd in impls], expected, a.n)
        cells = "".join(f"{t:>14}" if isinstance(t, str) else f"{t:>13.3f}s" for t in times)
        r = ""
        if ratio and isinstance(times[0], float) and isinstance(times[1], float):
            r = f"{times[0] / times[1]:>10.2f}"
        print(f"{b:<16}{cells}{r}", flush=True)


if __name__ == "__main__":
    main()
