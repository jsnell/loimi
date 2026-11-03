#!/usr/bin/env python3
"""Run the Prolog benchmarks under the reference systems that
bench/prolog/install.sh built, and optionally under our Prolog.

Usage: bench/prolog/run.py [-n RUNS] [--loimi PATH] [--exe NAME=CMD ...]
                           [--refs REF,...] [--emit] [--calibrate SECONDS]
                           [NAME ...]

Each runnable program is out/NAME.pl, the concatenation of

    programs/NAME.pl      the suite's program (made by install.sh/prepare.py)
    iterations(K).        K from iterations.txt
    driver.pl             runs top/0 K times, prints `done`
    :- initialization(main).

so loading the file runs the benchmark; every system is invoked on the
same file:

    swipl    install/swipl/bin/swipl -O -q -g halt out/NAME.pl
    trealla  install/trealla/bin/tpl -q out/NAME.pl -g halt
    loimi    PATH out/NAME.pl                    (with --loimi PATH)
    NAME     CMD out/NAME.pl                     (with --exe NAME=CMD, repeatable;
                                                  CMD may be a wrapper script)

SWI-Prolog runs with -O (arithmetic compiled inline; the suite's own
calibration did not use it, but it is SWI's fast mode). Trealla's default
is its optimized mode. A run passes if its stdout is exactly "done\\n"
(stderr is ignored: SWI prints style warnings there). Times are the
minimum over RUNS (default 3) runs, interleaved round-robin over
benchmarks and systems, of the whole process's wall-clock time, startup
and loading included (about 0.02 s for both references). Ratio columns
are each of our builds' time over each reference's (or, without --loimi
or --exe, trealla over swipl), with their geometric mean over the
benchmarks all compared systems passed. With --slow S, a benchmark that
took more than S seconds under a system is not run again under it (its
time is that one run).

--emit only writes out/*.pl. --calibrate S rewrites iterations.txt so
that each benchmark takes about S seconds under swipl.
"""
import argparse, math, os, subprocess, sys, time

HERE = os.path.dirname(os.path.abspath(__file__))
INSTALL = os.path.join(HERE, "install")
PROGRAMS = os.path.join(HERE, "programs")
OUT = os.path.join(HERE, "out")
ITERATIONS = os.path.join(HERE, "iterations.txt")
TIMEOUT = 300

REFS = {
    "swipl": lambda f: [os.path.join(INSTALL, "swipl/bin/swipl"), "-O", "-q", "-g", "halt", f],
    "trealla": lambda f: [os.path.join(INSTALL, "trealla/bin/tpl"), "-q", f, "-g", "halt"],
}

# swipl's RUNPATH is the absolute directory it was installed in; an install
# that moved (or was built in another checkout) finds libswipl through this.
_swipl_lib = os.path.join(INSTALL, "swipl/lib/swipl/lib/x86_64-linux")
if os.path.isdir(_swipl_lib):
    os.environ["LD_LIBRARY_PATH"] = _swipl_lib + (
        ":" + os.environ["LD_LIBRARY_PATH"] if os.environ.get("LD_LIBRARY_PATH") else "")


def read_iterations():
    its = {}
    with open(ITERATIONS) as fh:
        for line in fh:
            line = line.split("#")[0].split()
            if line:
                its[line[0]] = int(line[1])
    return its


def emit(name, k):
    os.makedirs(OUT, exist_ok=True)
    with open(os.path.join(PROGRAMS, name + ".pl")) as fh:
        prog = fh.read()
    with open(os.path.join(HERE, "driver.pl")) as fh:
        driver = fh.read()
    path = os.path.join(OUT, name + ".pl")
    with open(path, "w") as fh:
        fh.write(prog + "\n\niterations(%d).\n\n" % k + driver + "\n:- initialization(main).\n")
    return path


def run(cmd):
    """(seconds, ok, detail) for one run of cmd."""
    t0 = time.perf_counter()
    try:
        p = subprocess.run(cmd, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                           stdin=subprocess.DEVNULL, timeout=TIMEOUT)
    except subprocess.TimeoutExpired:
        return TIMEOUT, False, "timeout"
    t = time.perf_counter() - t0
    out = p.stdout.decode(errors="replace")
    if p.returncode == 0 and out == "done\n":
        return t, True, ""
    err = p.stderr.decode(errors="replace").strip().splitlines()
    return t, False, "rc=%d out=%r %s" % (p.returncode, out[-80:], err[-1][:120] if err else "")


def geomean(xs):
    return math.exp(sum(math.log(x) for x in xs) / len(xs)) if xs else float("nan")


def calibrate(names, its, target):
    swipl = REFS["swipl"]
    for name in names:
        k = its[name]
        while True:
            t = min(run(swipl(emit(name, k)))[0] for _ in range(2))
            if t > 0.25 or k > 10**8:
                break
            k *= 4
        base = min(run(swipl(emit(name, 0)))[0] for _ in range(2))
        new = k * target / max(t - base, 1e-3)
        new = max(1, round(new, 2 - len(str(int(new)))))  # 2 significant digits
        print("%-16s K %8d: %.3fs (startup %.3fs) -> K %d" % (name, k, t, base, new), file=sys.stderr)
        its[name] = new
    with open(ITERATIONS) as fh:
        header = [l for l in fh if l.startswith("#")]
    with open(ITERATIONS, "w") as fh:
        fh.writelines(header)
        for name in sorted(its):
            fh.write("%-16s %d\n" % (name, its[name]))


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("-n", type=int, default=3, help="runs per benchmark and system (best of)")
    ap.add_argument("--loimi", metavar="PATH", help="our Prolog: run as PATH FILE.pl")
    ap.add_argument("--exe", action="append", default=[], metavar="NAME=CMD",
                    help="one of our builds, run as CMD FILE.pl (repeatable)")
    ap.add_argument("--slow", type=float, metavar="SECONDS",
                    help="run a (benchmark, system) once if its first run took longer")
    ap.add_argument("--refs", default="swipl,trealla", help="comma-separated references")
    ap.add_argument("--emit", action="store_true", help="only write out/*.pl")
    ap.add_argument("--calibrate", type=float, metavar="SECONDS")
    ap.add_argument("names", nargs="*")
    a = ap.parse_args()

    its = read_iterations()
    names = a.names or sorted(its)
    for n in names:
        if n not in its:
            sys.exit("unknown benchmark %s" % n)
    if a.calibrate:
        calibrate(names, its, a.calibrate)
        return
    files = {n: emit(n, its[n]) for n in names}
    if a.emit:
        print("\n".join(files[n] for n in names))
        return

    systems = {}
    for r in filter(None, a.refs.split(",")):
        if not os.path.exists(REFS[r]("x")[0]):
            print("note: %s not installed, skipped" % r, file=sys.stderr)
            continue
        systems[r] = REFS[r]
    if a.loimi:
        path = os.path.abspath(a.loimi)
        systems["loimi"] = lambda f: [path, f]
    ours = ["loimi"] if a.loimi else []
    for spec in a.exe:
        name, sep, cmd = spec.partition("=")
        if not sep:
            ap.error("--exe wants NAME=CMD")
        systems[name] = (lambda c: lambda f: [c, f])(os.path.abspath(cmd))
        ours.append(name)
    sysnames = list(systems)

    best = {(n, s): None for n in names for s in sysnames}
    fail = {}
    for i in range(a.n):
        for n in names:
            for s in sysnames:
                if (n, s) in fail or (a.slow and i > 0 and best[(n, s)] > a.slow):
                    continue
                t, ok, detail = run(systems[s](files[n]))
                if not ok:
                    fail[(n, s)] = detail
                    print("FAIL %s %s: %s" % (n, s, detail), file=sys.stderr)
                elif best[(n, s)] is None or t < best[(n, s)]:
                    best[(n, s)] = t
        print("round %d/%d done" % (i + 1, a.n), file=sys.stderr)

    if ours:
        pairs = [(o, r) for o in ours for r in sysnames if r not in ours]
    elif "swipl" in systems and "trealla" in systems:
        pairs = [("trealla", "swipl")]
    else:
        pairs = []
    head = ["benchmark", "K"] + ["%s s" % s for s in sysnames] + ["%s/%s" % p for p in pairs]
    print("| " + " | ".join(head) + " |")
    print("|" + "|".join(["---"] + ["--:"] * (len(head) - 1)) + "|")
    ratios = {p: [] for p in pairs}
    for n in names:
        row = [n, str(its[n])]
        for s in sysnames:
            row.append("FAIL" if (n, s) in fail else "%.3f" % best[(n, s)])
        for p in pairs:
            x, y = best[(n, p[0])], best[(n, p[1])]
            if (n, p[0]) in fail or (n, p[1]) in fail:
                row.append("")
            else:
                ratios[p].append(x / y)
                row.append("%.2f" % (x / y))
        print("| " + " | ".join(row) + " |")
    row = ["geomean", ""] + [""] * len(sysnames) + ["%.2f" % geomean(ratios[p]) for p in pairs]
    print("| " + " | ".join(row) + " |")
    if fail:
        sys.exit(1)


if __name__ == "__main__":
    main()
