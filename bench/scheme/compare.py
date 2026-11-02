#!/usr/bin/env python3
"""Run r7rs-benchmarks programs under our Scheme (examples/scheme) and chibi.

Usage: bench/scheme/compare.py [-n RUNS] [--dispatch MODE ...] [--check] [NAME ...]

Each runnable program is built the way the suite's own `bench` script
builds it: the concatenation of

    <impl>-prelude.scm  src/NAME.scm  src/common.scm  <impl>-postlude.scm  src/common-postlude.scm

where src/ is the unmodified suite (bench/scheme/fetch.sh). Chibi uses the
suite's own Chibi-prelude/postlude; our Scheme uses Loimi-prelude.scm and
Loimi-postlude.scm from this directory. The program reads its parameters
(iteration count, inputs, expected result) from stdin, from the suite's
inputs/NAME.input with the leading values replaced as listed in INPUTS
below, so that each run takes about 1-3 s under chibi.

A run passes if the benchmark's own result check passed (no "ERROR:" line)
and its output, minus the timing lines, equals chibi's. Times are the
minimum over RUNS runs, in seconds, of the whole process's wall-clock time
(startup and compilation included; chibi's startup is about 0.3 s), or with
--inner of the "Elapsed time" the benchmark itself reports.
"""
import argparse, os, re, subprocess, sys, time

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.abspath(os.path.join(HERE, "..", ".."))
SUITE = os.path.join(HERE, "r7rs-benchmarks")
CHIBI = os.path.join(HERE, "install")
OUT = os.path.join(os.environ.get("TMPDIR", "/tmp"), "loimi-scheme-bench")

def desc(n):
    """The list (n n-1 ... 1), as takl's inputs spell it."""
    return "(" + " ".join(str(i) for i in range(n, 0, -1)) + ")"


# Benchmarks we run, in the suite's grouping order, each with the data that
# replace the first data of the suite's inputs/NAME.input: the iteration
# count, then (where the suite's sizes are made for native-code compilers)
# smaller parameters and their expected result, which chibi verifies. Most
# smaller parameters are the "old inputs" listed in the suite's own input
# files. An empty list keeps the suite's input as is.
INPUTS = {
    # Gabriel
    "browse":    ["3"],
    "deriv":     ["250000"],
    "destruc":   ["80"],
    "diviter":   ["30000"],
    "divrec":    ["30000"],
    "puzzle":    ["10"],
    "triangl":   ["1"],
    "tak":       ["1", "32", "16", "8", "9"],
    "takl":      ["20", desc(18), desc(12), desc(6), "7"],
    "ntakl":     ["40", desc(18), desc(12), desc(6), "7"],
    "cpstak":    ["150", "18", "12", "6", "7"],
    "ctak":      ["7", "18", "12", "6", "7"],
    # numeric
    "fib":       ["20", "30", "832040"],
    "fibc":      ["2", "25", "75025"],
    "fibfp":     ["5", "30.0", "832040.0"],
    "sum":       ["5000"],
    "sumfp":     ["10"],
    "fft":       ["3"],
    "mbrot":     ["10"],
    "nucleic":   ["1"],
    "pnpoly":    ["10000"],
    "simplex":   ["15000"],
    # Kernighan-Van Wyk
    "ack":       ["5", "3", "9", "4093"],
    "array1":    ["20"],
    "string":    ["7"],
    # other
    "conform":   ["5"],
    "earley":    ["3", "10", "4862"],
    "graphs":    ["1", "6", "10275"],
    "lattice":   ["1000", "33", "10"],
    "matrix":    ["20"],
    "maze":      ["80"],
    "mazefun":   ["150"],
    "nqueens":   ["25", "10", "724"],
    "paraffins": ["25", "17", "24894"],
    "peval":     ["20"],
    "primes":    ["400"],
    "quicksort": ["20"],
    # GC
    "nboyer":    ["1", "3", "5375678"],
    "sboyer":    ["1", "3", "5375678"],
    "gcbench":   ["1", "16"],
    "mperm":     ["1", "9", "2", "1"],
}

def skip_space(text, i):
    while i < len(text):
        if text[i].isspace():
            i += 1
        elif text[i] == ";":
            while i < len(text) and text[i] != "\n":
                i += 1
        else:
            break
    return i


def datum_end(text, i):
    """Offset just past the datum that starts at text[i] (atoms, strings,
    lists and vectors; enough for the suite's input files)."""
    if text[i] == '"':
        i += 1
        while text[i] != '"':
            i += 2 if text[i] == "\\" else 1
        return i + 1
    if text[i] in "'`":
        return datum_end(text, skip_space(text, i + 1))
    if text[i] == "(" or text.startswith("#(", i):
        i = text.index("(", i) + 1
        while True:
            i = skip_space(text, i)
            if text[i] == ")":
                return i + 1
            i = datum_end(text, i)
    if text.startswith("#\\", i):
        i += 3
    while i < len(text) and not text[i].isspace() and text[i] not in "()\";":
        i += 1
    return i


def replace_data(text, over):
    """`text` with its first len(over) data replaced by those of `over`
    (None keeps the suite's datum)."""
    i, out = 0, []
    for d in over:
        j = skip_space(text, i)
        i = datum_end(text, j)
        out.append(text[j:i] if d is None else d)
    return "".join(d + "\n" for d in out) + text[i:]


def input_for(name, count=None):
    with open(os.path.join(SUITE, "inputs", name + ".input")) as f:
        text = replace_data(f.read(), INPUTS.get(name, []))
    if count is not None:
        text = replace_data(text, [str(count)])
    return text


def program(name, impl):
    src = os.path.join(SUITE, "src")
    if impl == "chibi":
        pre = os.path.join(src, "Chibi-prelude.scm")
        post = os.path.join(src, "Chibi-postlude.scm")
    else:
        pre = os.path.join(HERE, "Loimi-prelude.scm")
        post = os.path.join(HERE, "Loimi-postlude.scm")
    parts = [pre, os.path.join(src, name + ".scm"), os.path.join(src, "common.scm"),
             post, os.path.join(src, "common-postlude.scm")]
    text = "".join(open(p).read() for p in parts)
    path = os.path.join(OUT, f"{name}-{impl}.scm")
    with open(path, "w") as f:
        f.write(text)
    return path


def build_ours(dispatch):
    exe = os.path.join(OUT, f"scheme-{dispatch}")
    subprocess.run([os.path.join(ROOT, "target", "release", "loimi"), "build",
                    os.path.join(ROOT, "examples", "scheme", "main.lm"), "--release",
                    f"--dispatch={dispatch}", "-o", exe], check=True)
    return exe


def normalize(out):
    """The output minus the lines that carry timings or implementation names."""
    keep = [l for l in out.decode(errors="replace").splitlines()
            if not l.startswith("Elapsed time:") and not l.startswith("+!CSVLINE!+")]
    return "\n".join(keep)


ELAPSED = re.compile(rb"^Elapsed time: ([0-9.eE+-]+) seconds", re.M)


def run(cmd, stdin, runs, env=None, timeout=None, inner=False):
    """(best time or None, normalized output, error text). The time is the
    process's wall-clock time, or with `inner` the benchmark's own
    "Elapsed time" (which excludes startup and compilation)."""
    best, out = None, None
    for _ in range(runs):
        t = time.perf_counter()
        try:
            p = subprocess.run(cmd, input=stdin.encode(), capture_output=True, cwd=SUITE,
                               env=env, timeout=timeout)
        except subprocess.TimeoutExpired:
            return None, "", "timeout"
        dt = time.perf_counter() - t
        if inner:
            m = ELAPSED.search(p.stdout)
            dt = float(m.group(1)) if m else dt
        o = normalize(p.stdout)
        if p.returncode != 0 or "ERROR:" in o:
            return None, o, (o + "\n" + p.stderr.decode(errors="replace")).strip()
        out = o
        best = dt if best is None else min(best, dt)
    return best, out, ""


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("-n", type=int, default=3)
    ap.add_argument("--dispatch", nargs="*", default=["tailcall"])
    ap.add_argument("--check", action="store_true", help="one run each, report failures in full")
    ap.add_argument("--count", type=int, help="override the iteration count (for quick checks)")
    ap.add_argument("--inner", action="store_true",
                    help="report the benchmarks' own elapsed time instead of process wall time")
    ap.add_argument("--timeout", type=float, default=300, help="seconds per run")
    ap.add_argument("--no-chibi", action="store_true", help="run only our Scheme")
    ap.add_argument("names", nargs="*")
    a = ap.parse_args()
    if a.check:
        a.n = 1
    if not os.path.isdir(os.path.join(SUITE, "src")):
        sys.exit("bench/scheme/r7rs-benchmarks missing; run bench/scheme/fetch.sh")
    os.makedirs(OUT, exist_ok=True)
    subprocess.run(["cargo", "build", "--release", "-q"], cwd=ROOT, check=True)
    impls = [(f"loimi-{d}", "loimi", [build_ours(d)], None) for d in a.dispatch]
    chibi = os.path.join(CHIBI, "bin", "chibi-scheme")
    if a.no_chibi:
        pass
    elif os.path.exists(chibi):
        env = dict(os.environ, LD_LIBRARY_PATH=os.path.join(CHIBI, "lib"))
        impls.append(("chibi", "chibi", [chibi, "-m", "scheme.load", "-e"], env))
    else:
        print(f"note: {chibi} missing; run bench/scheme/install.sh", file=sys.stderr)
    names = a.names or list(INPUTS)
    print(f"{'benchmark':<12}" + "".join(f"{n:>16}" for n, _, _, _ in impls) + f"{'ours/chibi':>12}")
    status = 0
    for b in names:
        stdin = input_for(b, a.count)
        times, outs, errs = [], [], []
        for _, kind, cmd, env in impls:
            prog = program(b, kind)
            if kind == "chibi":
                cmd = cmd + [f'(load "{prog}")']
            else:
                cmd = cmd + [prog]
            t, o, e = run(cmd, stdin, a.n, env, a.timeout, a.inner)
            times.append(t); outs.append(o); errs.append(e)
        cells = []
        for t, o in zip(times, outs):
            if t is None:
                cells.append(f"{'FAIL':>16}")
            elif o != outs[-1]:
                cells.append(f"{'DIFFERS':>16}")
            else:
                cells.append(f"{t:>15.3f}s")
        ratio = ""
        if times[0] is not None and times[-1] is not None and outs[0] == outs[-1] and len(impls) > 1:
            ratio = f"{times[0] / times[-1]:>12.2f}"
        print(f"{b:<12}" + "".join(cells) + ratio, flush=True)
        for (n, _, _, _), t, o, e in zip(impls, times, outs, errs):
            if t is None or o != outs[-1]:
                status = 1
                if a.check:
                    detail = e if t is None else o
                    print(f"  {n}: " + "\n  ".join(detail.splitlines()[-6:]))
    sys.exit(status)


if __name__ == "__main__":
    main()
