#!/usr/bin/env python3
"""Time the wasm32-wasi benchmarks in programs-out/ under several interpreters.

    bench.py [-n 3] [--timeout 300] [--interp NAME=CMD ...] [BENCH ...]

CMD is a shell-style command line; "{wasm}" is replaced by the module path
(appended if absent). Default interpreters: wasm3 and wabt's wasm-interp
--wasi (both from install.sh). Each benchmark runs n times per interpreter,
the best wall-clock time is reported. stdout is checked against
programs-out/<name>.expected (made with wasm3 by programs.sh); timing lines
of CoreMark are ignored in the comparison. A run that exceeds the timeout is
killed and reported as "timeout" (its remaining repetitions are skipped).
Prints a markdown table with seconds and ratios to the first interpreter
(the baseline), and geometric means over the benchmarks every interpreter
finished.
"""
import argparse
import math
import os
import re
import shlex
import signal
import subprocess
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))
OUT = os.path.join(HERE, "programs-out")
BIN = os.path.join(HERE, "install", "bin")

DEFAULT_INTERPS = [
    ("wasm3", os.path.join(BIN, "wasm3") + " {wasm}"),
    ("wasm-interp", os.path.join(BIN, "wasm-interp") + " --wasi {wasm}"),
]

# Lines that legitimately differ between runs. CoreMark's verdict depends on
# whether the run took >= 10 s; its CRC lines (and any "[0]ERROR! ... crc"
# lines) are still compared.
VOLATILE = {
    "coremark": re.compile(
        rb"^(Total ticks|Total time \(secs\)|Iterations/Sec|CoreMark 1\.0) *:.*\n"
        rb"|^ERROR! Must execute for at least 10 secs.*\n"
        rb"|^(Errors detected|Correct operation validated\..*)\n",
        re.M),
}


def normalize(name, data):
    pat = VOLATILE.get(name)
    return pat.sub(b"", data) if pat else data


def run_once(cmd, timeout):
    t0 = time.perf_counter()
    p = subprocess.Popen(cmd, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
                         stderr=subprocess.PIPE, start_new_session=True)
    try:
        out, err = p.communicate(timeout=timeout)
    except subprocess.TimeoutExpired:
        os.killpg(p.pid, signal.SIGKILL)
        p.communicate()
        return None, None, None, None
    return time.perf_counter() - t0, p.returncode, out, err


def main():
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("-n", type=int, default=3, help="runs per benchmark (best of n)")
    ap.add_argument("--timeout", type=float, default=300, help="seconds per run")
    ap.add_argument("--interp", action="append", default=[], metavar="NAME=CMD",
                    help="interpreter to time (repeatable; replaces the defaults)")
    ap.add_argument("--add-default", action="store_true",
                    help="keep the default interpreters in front of --interp ones")
    ap.add_argument("benchmarks", nargs="*", help="subset of programs-out/*.wasm names")
    a = ap.parse_args()

    interps = []
    if not a.interp or a.add_default:
        interps += DEFAULT_INTERPS
    for spec in a.interp:
        name, sep, cmd = spec.partition("=")
        if not sep:
            ap.error("--interp wants NAME=CMD")
        interps.append((name, cmd))

    names = a.benchmarks or sorted(f[:-5] for f in os.listdir(OUT) if f.endswith(".wasm"))
    results = {}  # (bench, interp) -> seconds | "timeout" | "FAIL ..." | "WRONG"
    for b in names:
        wasm = os.path.join(OUT, b + ".wasm")
        exp_path = os.path.join(OUT, b + ".expected")
        expected = None
        if os.path.exists(exp_path):
            with open(exp_path, "rb") as f:
                expected = normalize(b, f.read())
        for iname, icmd in interps:
            argv = shlex.split(icmd.replace("{wasm}", wasm))
            if "{wasm}" not in icmd:
                argv.append(wasm)
            best = None
            for _ in range(a.n):
                dt, rc, out, err = run_once(argv, a.timeout)
                if dt is None:
                    best = "timeout"
                    break
                if rc != 0:
                    best = "FAIL rc=%d" % rc
                    sys.stderr.write("%s/%s: exit %d\n%s\n" % (
                        b, iname, rc, err.decode(errors="replace")[-2000:]))
                    break
                if expected is not None and normalize(b, out) != expected:
                    best = "WRONG"
                    sys.stderr.write("%s/%s: stdout differs from %s\n" % (b, iname, exp_path))
                    break
                best = dt if best is None else min(best, dt)
            results[(b, iname)] = best
            shown = "%.3fs" % best if isinstance(best, float) else best
            sys.stderr.write("%-14s %-14s %s\n" % (b, iname, shown))

    base = interps[0][0]
    hdr = ["benchmark"] + [n for n, _ in interps]
    print("| " + " | ".join(hdr) + " |")
    print("|" + "---|" * len(hdr))

    def cell(b, n):
        r = results[(b, n)]
        if not isinstance(r, float):
            return r
        rb = results[(b, base)]
        if n != base and isinstance(rb, float):
            return "%.3f (%.1fx)" % (r, r / rb)
        return "%.3f" % r

    for b in names:
        print("| " + " | ".join([b] + [cell(b, n) for n, _ in interps]) + " |")
    done = [b for b in names if all(isinstance(results[(b, n)], float) for n, _ in interps)]
    if done:
        geo = {n: math.exp(sum(math.log(results[(b, n)]) for b in done) / len(done))
               for n, _ in interps}
        row = ["geomean (%d)" % len(done)]
        for n, _ in interps:
            row.append("%.3f" % geo[n] if n == base else
                       "%.3f (%.1fx)" % (geo[n], geo[n] / geo[base]))
        print("| " + " | ".join(row) + " |")


if __name__ == "__main__":
    main()
