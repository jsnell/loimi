#!/usr/bin/env python3
"""Run r7rs-benchmarks programs under our Scheme (examples/scheme) and the
reference interpreters that bench/scheme/install.sh built.

Usage: bench/scheme/compare.py [-n RUNS] [--dispatch MODE ...] [--refs REF,...]
                               [--check] [NAME ...]

The references, each used when installed: chibi, gauche, guile (Guile 3
with its JIT) and guile-nojit (the same with GUILE_JIT_THRESHOLD=-1, i.e.
its bytecode VM only).

Each runnable program is built the way the suite's own `bench` script
builds it: the concatenation of

    <impl>-prelude.scm  src/NAME.scm  src/common.scm  <impl>-postlude.scm  src/common-postlude.scm

where src/ is the unmodified suite (bench/scheme/fetch.sh) and a missing
prelude or postlude is skipped. The references use the suite's own files
(Chibi-prelude/postlude; Gauche-postlude only; Guile-prelude only) and
are run with the same commands as in the suite's `bench` script; Guile's
program is compiled first with `guild compile -O3` (not timed), as there.
Our Scheme uses Loimi-prelude.scm and Loimi-postlude.scm from this
directory. The program reads its parameters (iteration count, inputs,
expected result) from stdin, from the suite's inputs/NAME.input with the
leading values replaced as listed in INPUTS below, so that each run takes
about 1-3 s under chibi.

A run passes if the benchmark's own result check passed (no "ERROR:" line)
and its output, minus the timing lines, equals that of the first reference
(chibi when installed; our Scheme's when there is no reference). Times are
the minimum over RUNS runs, in seconds, of the whole process's wall-clock
time (startup and compilation included; chibi's startup is about 0.3 s),
or with --inner of the "Elapsed time" the benchmark itself reports. The
ratio columns are the first of our columns over each reference.
"""
import argparse, math, os, re, resource, subprocess, sys, time

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.abspath(os.path.join(HERE, "..", ".."))
SUITE = os.path.join(HERE, "r7rs-benchmarks")
INSTALL = os.path.join(HERE, "install")
OUT = os.path.join(os.environ.get("TMPDIR", "/tmp"), "loimi-scheme-bench")

# The suite's prelude and postlude for each reference (None: the suite has
# none, so its `bench` script uses /dev/null).
SUITE_FILES = {
    "chibi": ("Chibi-prelude.scm", "Chibi-postlude.scm"),
    "gauche": (None, "Gauche-postlude.scm"),
    "guile": ("Guile-prelude.scm", None),
}

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
    # bignums: the suite's inputs (ours takes 0.2-0.3 s)
    "pi":        [],
    "chudnovsky": [],
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
    "gcbench":   ["1", "13"],
    "mperm":     ["1", "9", "2", "1"],
}

# (reference, benchmark) pairs not run: chibi's bignums take 21 s for one
# of pi's 100 iterations (ours: 3.5 ms), so it would need over half an hour.
SKIP = {("chibi", "pi"), ("chibi", "chudnovsky")}

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


def program(name, kind):
    """Write the runnable program for `kind` ("loimi" or a key of
    SUITE_FILES) and return its path. The file is rewritten only when its
    text changes, so that Guile's compiled code for it stays fresh."""
    src = os.path.join(SUITE, "src")
    if kind in SUITE_FILES:
        pre, post = (os.path.join(src, f) if f else os.devnull for f in SUITE_FILES[kind])
    else:
        pre = os.path.join(HERE, "Loimi-prelude.scm")
        post = os.path.join(HERE, "Loimi-postlude.scm")
    parts = [pre, os.path.join(src, name + ".scm"), os.path.join(src, "common.scm"),
             post, os.path.join(src, "common-postlude.scm")]
    text = "".join(open(p).read() for p in parts)
    path = os.path.join(OUT, f"{name}-{kind}.scm")
    if not os.path.exists(path) or open(path).read() != text:
        with open(path, "w") as f:
            f.write(text)
    return path


def build_ours(dispatch, align=None):
    exe = os.path.join(OUT, f"scheme-{dispatch}")
    subprocess.run([os.path.join(ROOT, "target", "release", "loimi"), "build",
                    os.path.join(ROOT, "examples", "scheme", "main.lm"), "--release",
                    f"--dispatch={dispatch}", "-o", exe]
                   + ([f"--align-functions={align}"] if align else []), check=True)
    return exe


class Impl:
    """One column: `cmd(prog)` is the command line that runs a program;
    `prepare(prog)`, if any, compiles it first (untimed) and returns an
    error text or None."""
    def __init__(self, name, kind, cmd, env=None, prepare=None):
        self.name, self.kind, self.cmd, self.env, self.prepare = name, kind, cmd, env, prepare


def binary(name):
    return os.path.join(INSTALL, "bin", name)


def references(wanted):
    """The installed references among `wanted`, each run as the suite's
    `bench` script runs it."""
    env = dict(os.environ, LD_LIBRARY_PATH=os.path.join(INSTALL, "lib"))
    refs = []
    for r in wanted:
        if r == "chibi" and os.path.exists(binary("chibi-scheme")):
            refs.append(Impl(r, r, lambda p: [binary("chibi-scheme"), "-m", "scheme.load",
                                              "-e", f'(load "{p}")'], env))
        elif r == "gauche" and os.path.exists(binary("gosh")):
            refs.append(Impl(r, r, lambda p: [binary("gosh"), "-r7", "-b", p], env))
        elif r in ("guile", "guile-nojit") and os.path.exists(binary("guile")):
            # `guild compile` writes the .go into Guile's cache (under
            # XDG_CACHE_HOME), where `guile prog` then finds it, as in the
            # suite's script.
            genv = dict(env, XDG_CACHE_HOME=os.path.join(OUT, "guile-cache"),
                        GC_INITIAL_HEAP_SIZE="100000000")
            if r == "guile-nojit":
                genv["GUILE_JIT_THRESHOLD"] = "-1"
            def compile_(p, genv=genv):
                c = subprocess.run([binary("guild"), "compile", "-O3", p], env=genv,
                                   capture_output=True)
                return None if c.returncode == 0 else c.stderr.decode(errors="replace")
            refs.append(Impl(r, "guile", lambda p: [binary("guile"), p], genv, compile_))
        else:
            print(f"note: {r} not installed; run bench/scheme/install.sh", file=sys.stderr)
    return refs


def normalize(out):
    """The output minus the lines that carry timings or implementation names."""
    keep = [l for l in out.decode(errors="replace").splitlines()
            if not l.startswith("Elapsed time:") and not l.startswith("+!CSVLINE!+")]
    return "\n".join(keep)


ELAPSED = re.compile(rb"^Elapsed time: ([0-9.eE+-]+) seconds", re.M)


def run(cmd, stdin, runs, env=None, timeout=None, inner=False, cpu=False):
    """(best time or None, normalized output, error text). The time is the
    process's wall-clock time, with `cpu` its user+system CPU time, or
    with `inner` the benchmark's own "Elapsed time" (which excludes
    startup and compilation)."""
    best, out = None, None
    for _ in range(runs):
        t = time.perf_counter()
        r0 = resource.getrusage(resource.RUSAGE_CHILDREN)
        try:
            p = subprocess.run(cmd, input=stdin.encode(), capture_output=True, cwd=SUITE,
                               env=env, timeout=timeout)
        except subprocess.TimeoutExpired:
            return None, "", "timeout"
        dt = time.perf_counter() - t
        if cpu:
            r1 = resource.getrusage(resource.RUSAGE_CHILDREN)
            dt = (r1.ru_utime - r0.ru_utime) + (r1.ru_stime - r0.ru_stime)
        if inner:
            m = ELAPSED.search(p.stdout)
            dt = float(m.group(1)) if m else dt
        o = normalize(p.stdout)
        if p.returncode != 0 or "ERROR:" in o:
            return None, o, (o + "\n" + p.stderr.decode(errors="replace")).strip()
        out = o
        best = dt if best is None else min(best, dt)
    return best, out, ""


ALL_REFS = ["chibi", "gauche", "guile", "guile-nojit"]


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("-n", type=int, default=3)
    ap.add_argument("--dispatch", nargs="*", default=["tailcall"])
    ap.add_argument("--refs", default=",".join(ALL_REFS),
                    help="comma-separated reference interpreters to run, of "
                    f"{','.join(ALL_REFS)} (default: all installed; '': only ours)")
    ap.add_argument("--check", action="store_true", help="one run each, report failures in full")
    ap.add_argument("--count", type=int, help="override the iteration count (for quick checks)")
    ap.add_argument("--inner", action="store_true",
                    help="report the benchmarks' own elapsed time instead of process wall time")
    ap.add_argument("--cpu", action="store_true",
                    help="time the process's user+system CPU instead of wall-clock time")
    ap.add_argument("--align", type=int,
                    help="build ours with --align-functions=N (steadier A/B comparisons)")
    ap.add_argument("--exe", nargs="*", default=[],
                    help="extra prebuilt binaries of ours to run as columns (A/B runs)")
    ap.add_argument("--timeout", type=float, default=300, help="seconds per run")
    ap.add_argument("names", nargs="*")
    a = ap.parse_args()
    if a.check:
        a.n = 1
    a.refs = [r for r in a.refs.split(",") if r]
    if set(a.refs) - set(ALL_REFS):
        ap.error(f"--refs: unknown {', '.join(set(a.refs) - set(ALL_REFS))}")
    if not os.path.isdir(os.path.join(SUITE, "src")):
        sys.exit("bench/scheme/r7rs-benchmarks missing; run bench/scheme/fetch.sh")
    os.makedirs(OUT, exist_ok=True)
    subprocess.run(["cargo", "build", "--release", "-q"], cwd=ROOT, check=True)
    ours = [Impl(f"loimi-{d}", "loimi", lambda p, exe=build_ours(d, a.align): [exe, p])
            for d in a.dispatch]
    ours += [Impl(os.path.basename(e), "loimi", lambda p, exe=os.path.abspath(e): [exe, p])
             for e in a.exe]
    refs = references(a.refs)
    impls = ours + refs
    names = a.names or list(INPUTS)
    print(f"{'benchmark':<12}" + "".join(f"{i.name:>14}" for i in impls)
          + "".join(f"{'/' + r.name:>13}" for r in refs))
    status = 0
    ratios = {r.name: [] for r in refs}
    ab = {k: [] for k in range(1, len(ours))}
    for b in names:
        stdin = input_for(b, a.count)
        # Runs are interleaved (one run of each implementation in turn), so
        # slow drifts in machine load affect all columns alike.
        times, outs, errs = [None] * len(impls), [""] * len(impls), [""] * len(impls)
        progs, failed = [], [False] * len(impls)
        skipped = [(i.name, b) in SKIP for i in impls]
        for k, i in enumerate(impls):
            if skipped[k]:
                progs.append(None)
                failed[k] = True
                continue
            prog = program(b, i.kind)
            progs.append(prog)
            err = i.prepare(prog) if i.prepare else None
            if err:
                failed[k], errs[k] = True, err
        for _ in range(a.n):
            for k, i in enumerate(impls):
                if failed[k]:
                    continue
                t, o, e = run(i.cmd(progs[k]), stdin, 1, i.env, a.timeout, a.inner, a.cpu)
                outs[k], errs[k] = o, e
                if t is None:
                    failed[k], times[k] = True, None
                else:
                    times[k] = t if times[k] is None else min(times[k], t)
        # The expected output is that of the first reference that passed.
        passed = [o for k, (t, o) in enumerate(zip(times, outs))
                  if t is not None and k >= len(ours)]
        expect = passed[0] if passed else outs[0]
        ok = [t is not None and o == expect for t, o in zip(times, outs)]
        cells = [f"{t:>13.3f}s" if good else f"{'skipped' if skip else 'FAIL' if t is None else 'DIFFERS':>14}"
                 for t, good, skip in zip(times, ok, skipped)]
        for k in range(1, len(ours)):
            if ok[0] and ok[k]:
                ab[k].append(times[k] / times[0])
        for k, r in enumerate(refs, len(ours)):
            if ok[0] and ok[k]:
                ratios[r.name].append(times[0] / times[k])
                cells.append(f"{times[0] / times[k]:>13.2f}")
            else:
                cells.append(f"{'-':>13}")
        print(f"{b:<12}" + "".join(cells), flush=True)
        for i, t, good, o, e, skip in zip(impls, times, ok, outs, errs, skipped):
            if not good and not skip:
                status = 1
                if a.check:
                    detail = e if t is None else o
                    print(f"  {i.name}: " + "\n  ".join(detail.splitlines()[-6:]))
    geo = lambda xs: math.exp(sum(map(math.log, xs)) / len(xs)) if xs else float("nan")
    for k in range(1, len(ours)):
        print(f"geomean {ours[k].name} / {ours[0].name}: {geo(ab[k]):.3f}")
    if refs:
        print(f"{'geomean':<12}" + " " * 14 * len(impls)
              + "".join(f"{geo(ratios[r.name]):>13.2f}" for r in refs))
    sys.exit(status)


if __name__ == "__main__":
    main()

