#!/usr/bin/env python3
"""Run every example interpreter's benchmarks against its references, end to end.

    bench/run_all.py [--quick] [-n RUNS] [--out DIR] [--only SUITE,...] [--report JSON]

Builds each interpreter in all three dispatch modes (tailcall, goto,
switch) and, for Mini, MiniJS and Scheme, a `--jit-stencils` build run
with LOIMI_JIT=1, all with `--align-functions=64`, then runs the existing
per-suite scripts with those binaries as columns:

    mini      bench/mini/compare.py            vs PUC-Lua 5.4, LuaJIT -joff
    mini-coro bench/mini/coroutines/run.py     vs PUC-Lua 5.4, LuaJIT -joff
    minijs    bench/minijs/compare.py          vs node --jitless, QuickJS
    scheme    bench/scheme/compare.py          vs Gauche, Guile (VM only), Guile (JIT)
    lox       bench/lox/compare.py             vs clox
    regex     rxhost bench + examples/regex/bench.py   vs PCRE2 (no JIT), Python re
    wasm      bench/wasm/bench.py              vs wasm3
    stack     examples/stack/bench/run.py      (no reference)
    python    bench/python/compare.py          vs CPython 3.13, MicroPython

Every script interleaves its columns within a benchmark (bench.py does
not, so it runs -n 1 in RUNS rounds here and keeps the minimum). Times
are the minimum wall-clock time over RUNS runs (default 3). References
come from bench/*/install.sh; a missing one is skipped with a message.

Writes raw script output to OUT/raw/, all times to OUT/results.json, and
prints markdown tables (per suite: seconds, ratios, geometric means) that
docs/results.md is made from. `--report JSON` only re-renders the tables.
`--quick`: one benchmark per suite, one run (a smoke test, minutes).
"""
import argparse, json, math, os, re, shutil, subprocess, sys, time

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))
LOIMI = os.path.join(ROOT, "target", "release", "loimi")
MODES = ["tailcall", "goto", "switch"]
ALIGN = "--align-functions=64"

# Suite -> (entry file, JIT build?); our columns are named by mode.
INTERPS = {
    "mini": ("examples/mini/main.lm", True),
    "minijs": ("examples/minijs/main.lm", True),
    "scheme": ("examples/scheme/main.lm", True),
    "lox": ("examples/lox/main.lm", False),
    "wasm": ("examples/wasm/main.lm", False),
    "stack": ("examples/stack/main.lm", False),
    "python": ("examples/python/main.lm", False),
}
SUITES = ["mini", "mini-coro", "minijs", "scheme", "lox", "regex", "wasm", "stack", "python"]
QUICK = {"mini": "fib", "mini-coro": "coroutine_ring", "minijs": "richards", "scheme": "fib",
         "lox": "fib", "regex": None, "wasm": "nbody", "stack": None,
         "python": "richards"}
# Reference columns per suite, in table order (name as the script prints it).
REFS = {
    "mini": ["lua", "luajit-joff"],
    "mini-coro": ["lua", "luajit-joff"],
    "minijs": ["node-jitless", "qjs"],
    "scheme": ["gauche", "guile-nojit", "guile"],
    "lox": ["clox"],
    "regex": ["pcre2", "python-re"],
    "wasm": ["wasm3"],
    "stack": [],
    "python": ["cpython", "micropython"],
}


def log(*a):
    print(*a, file=sys.stderr, flush=True)


def sh(cmd, **kw):
    log("+ " + " ".join(cmd))
    return subprocess.run(cmd, check=True, **kw)


def build(suite, out):
    """Our binaries for `suite`: {column: path}."""
    src, jit = INTERPS[suite]
    exes = {}
    for m in MODES:
        exe = os.path.join(out, f"{suite}-{m}")
        sh([LOIMI, "build", os.path.join(ROOT, src), "--release", f"--dispatch={m}", ALIGN, "-o", exe])
        exes[m] = exe
    if jit:
        exe = os.path.join(out, f"{suite}-jitbin")
        sh([LOIMI, "build", os.path.join(ROOT, src), "--release", "--jit-stencils", ALIGN, "-o", exe])
        wrap = os.path.join(out, f"{suite}-jit")
        with open(wrap, "w") as f:
            f.write(f'#!/bin/sh\nLOIMI_JIT=1 exec "{exe}" "$@"\n')
        os.chmod(wrap, 0o755)
        exes["jit"] = wrap
    return exes


def capture(suite, out, cmd, env=None):
    log("+ " + " ".join(cmd))
    p = subprocess.run(cmd, capture_output=True, text=True, env=env)
    with open(os.path.join(out, "raw", suite + ".txt"), "a") as f:
        f.write("$ " + " ".join(cmd) + "\n" + p.stdout + "\n--- stderr\n" + p.stderr + "\n")
    if p.returncode != 0:
        log(f"{suite}: exit {p.returncode}\n{p.stderr[-2000:]}")
    return p.stdout


def num(cell):
    """Seconds from '1.234s', '1.234', '12.3ms', '1.234 (0.9x)'; None for FAIL etc."""
    m = re.match(r"^\*?\*?([0-9.]+)(ms|s)?", cell.strip())
    if not m:
        return None
    v = float(m.group(1))
    return v / 1000 if m.group(2) == "ms" else v


def parse_columns(text, rename):
    """Fixed-width tables of compare.py / run.py: a header line starting with
    'benchmark', then one row per benchmark."""
    res, hdr = {}, None
    for line in text.splitlines():
        f = line.split()
        if not f:
            continue
        if f[0] == "benchmark":
            hdr = [rename.get(h, h) for h in f[1:]]
            continue
        if hdr and len(f) >= 2 and not line.startswith(("geo", "note", "|")):
            row = {}
            for h, c in zip(hdr, f[1:]):
                if not h.startswith(("/", "mini/", "mjs/")):
                    row[h] = num(c)
            res[f[0]] = row
    return res


def parse_md(text, rename):
    """Markdown tables (lox compare.py, wasm bench.py)."""
    res, hdr = {}, None
    for line in text.splitlines():
        if not line.startswith("|") or line.startswith("|---"):
            continue
        cells = [c.strip() for c in line.strip().strip("|").split("|")]
        if cells[0] == "benchmark":
            hdr = [rename.get(h, h) for h in cells[1:]]
        elif hdr and not cells[0].startswith(("**", "geomean")):
            res[cells[0]] = {h: num(c) for h, c in zip(hdr, cells[1:]) if "/" not in h}
    return res


def merge_min(a, b):
    for k, row in b.items():
        r = a.setdefault(k, {})
        for c, v in row.items():
            if v is None:
                r.setdefault(c, None)
            elif r.get(c) is None or v < r[c]:
                r[c] = v
    return a


def have(path, what):
    if os.path.exists(path):
        return True
    log(f"note: {what} missing ({path}); skipped. Run its install.sh.")
    return False


def run_suite(suite, out, n, quick):
    q = [QUICK[suite]] if quick and QUICK[suite] else []
    if suite == "mini-coro":
        exes = build_cache(out, "mini")
        res = {}
        for m, exe in exes.items():
            t = capture(suite, out, [sys.executable, os.path.join(ROOT, "bench/mini/coroutines/run.py"),
                                     "--mini", exe, "-n", str(n)] + q)
            merge_min(res, parse_columns(t, {"mini": m}))
        return res
    if suite in ("mini", "minijs"):
        exes = build_cache(out, suite)
        refs = "lua,luajit-joff" if suite == "mini" else "node-jitless,qjs"
        rename = {os.path.basename(e): m for m, e in exes.items()}
        t = capture(suite, out, [sys.executable, os.path.join(ROOT, f"bench/{suite}/compare.py"), "-n", str(n),
                                 "--dispatch", "--exe"] + list(exes.values()) + ["--refs", refs] + q)
        return parse_columns(t, rename)
    if suite == "scheme":
        exes = build_cache(out, suite)
        if not have(os.path.join(ROOT, "bench/scheme/r7rs-benchmarks/src"), "r7rs-benchmarks (bench/scheme/fetch.sh)"):
            return {}
        rename = {os.path.basename(e): m for m, e in exes.items()}
        t = capture(suite, out, [sys.executable, os.path.join(ROOT, "bench/scheme/compare.py"), "-n", str(n),
                                 "--dispatch", "--exe"] + list(exes.values()) +
                    ["--refs", "gauche,guile-nojit,guile"] + q)
        return parse_columns(t, rename)
    if suite == "lox":
        exes = build_cache(out, suite)
        if not have(os.path.join(ROOT, "bench/lox/install/bin/clox"), "clox"):
            return {}
        t = capture(suite, out, [sys.executable, os.path.join(ROOT, "bench/lox/compare.py"), "-n", str(n)] +
                    [f"{m}={e}" for m, e in exes.items()] + (["--"] + q if q else []))
        return parse_md(t, {})
    if suite == "wasm":
        exes = build_cache(out, suite)
        if not have(os.path.join(ROOT, "bench/wasm/programs-out"), "wasm programs (bench/wasm/programs.sh)"):
            return {}
        args = [f"--interp=wasm3={os.path.join(ROOT, 'bench/wasm/install/bin/wasm3')} {{wasm}}"] \
            if have(os.path.join(ROOT, "bench/wasm/install/bin/wasm3"), "wasm3") else []
        args += [f"--interp={m}={e} {{wasm}}" for m, e in exes.items()]
        res = {}
        for _ in range(n):  # bench.py does not interleave: rounds of -n 1
            t = capture(suite, out, [sys.executable, os.path.join(ROOT, "bench/wasm/bench.py"), "-n", "1"] + args + q)
            merge_min(res, parse_md(t, {}))
        return res
    if suite == "stack":
        exes = build_cache(out, suite)
        log("note: stack: no CPython versions of the Stak benchmarks in the repository; no reference column.")
        t = capture(suite, out, [sys.executable, os.path.join(ROOT, "examples/stack/bench/run.py"), "-n", str(n)] +
                    list(exes.values()))
        res = parse_columns(t, {os.path.basename(e): m for m, e in exes.items()})
        return {k[:-4] if k.endswith(".stk") else k: v for k, v in res.items()
                if not quick or k == "fib.stk"}
    if suite == "python":
        exes = build_cache(out, suite)
        have(os.path.join(ROOT, "bench/python/install/bin/python3"), "CPython (bench/python/install.sh)")
        t = capture(suite, out, [sys.executable, os.path.join(ROOT, "bench/python/compare.py"), "-n", str(n)] +
                    [f"--exe={m}={e}" for m, e in exes.items()] + (["--only"] + q if q else []))
        # Cells of output mismatches ("1.234 MISMATCH": MicroPython's random
        # and last float digits) keep their time.
        return parse_md(t, {f"{c} (s)": c for c in MODES + REFS["python"]})
    if suite == "regex":
        return run_regex(out, n, quick)
    raise SystemExit(f"unknown suite {suite}")


def run_regex(out, n, quick):
    here = os.path.join(ROOT, "examples/regex")
    pcre = os.path.exists("/usr/include/pcre2.h")
    if not pcre:
        log("note: pcre2.h missing (libpcre2-dev); regex runs without the PCRE2 column.")
    hosts = {}
    for m in MODES:
        lib = os.path.join(out, f"librx_{m}.a")
        sh([LOIMI, "build", os.path.join(here, "lib.lm"), "--release", "--lib", f"--dispatch={m}", ALIGN,
            "-o", lib, "--header", os.path.join(out, "rx.h")])
        exe = os.path.join(out, f"rxhost_{m}")
        sh(["clang-19", "-O2", "-I" + out] + (["-DHAVE_PCRE2"] if pcre else []) +
           [os.path.join(here, "host.c"), lib, "-lm"] + (["-lpcre2-8"] if pcre else []) + ["-o", exe])
        hosts[m] = exe
    res = {}
    rounds = 1 if quick else n
    for _ in range(rounds):  # each run is itself best of 3 inside the process
        for m, exe in hosts.items():
            t = capture("regex", out, [exe, "bench"])
            for line in t.splitlines():
                f = line.split()
                if len(f) >= 6 and f[0] != "bench" and f[1].isdigit():
                    row = {m: num(f[2])}
                    if pcre:
                        row["pcre2"] = num(f[4])
                    merge_min(res, {f[0]: row})
        t = capture("regex", out, [sys.executable, "-I", os.path.join(here, "bench.py")])
        for line in t.splitlines():
            f = line.split()
            if len(f) == 3 and f[0] != "bench":
                merge_min(res, {f[0]: {"python-re": num(f[2])}})
    if quick:
        res = {k: v for k, v in res.items() if k == "capture"}
    return res


_built = {}


def build_cache(out, suite):
    if suite not in _built:
        _built[suite] = build(suite, out)
    return _built[suite]


def geo(xs):
    xs = [x for x in xs if x]
    return math.exp(sum(map(math.log, xs)) / len(xs)) if xs else None


def ours(suite):
    return MODES + (["jit"] if suite in ("mini", "mini-coro", "minijs", "scheme") else [])


def fmt(v, prec=3):
    return "—" if v is None else f"{v:.{prec}f}"


def render(data):
    lines, summary = [], []
    for suite in SUITES:
        res = data.get(suite)
        if not res:
            continue
        cols = [c for c in ours(suite) if any(r.get(c) is not None for r in res.values())]
        refs = [r for r in REFS[suite] if any(x.get(r) is not None for x in res.values())]
        ms = suite == "regex"
        unit = "ms" if ms else "s"
        lines.append(f"### {suite}\n")
        hdr = ["benchmark"] + cols + refs + [f"{c}/{r}" for r in refs for c in cols[:1]] + \
            [f"jit/{r}" for r in refs if "jit" in cols]
        lines.append("| " + " | ".join(hdr) + " |")
        lines.append("|---|" + "---:|" * (len(hdr) - 1))
        for b in sorted(res):
            row = res[b]
            k = 1000 if ms else 1
            cells = [b] + [fmt(row.get(c) and row[c] * k, 1 if ms else 3) for c in cols + refs]
            for r in refs:
                cells.append(fmt(ratio(row, cols[0], r), 2))
            for r in refs:
                if "jit" in cols:
                    cells.append(fmt(ratio(row, "jit", r), 2))
            lines.append("| " + " | ".join(cells) + " |")
        # Geometric means: each column of ours over each reference, and over tailcall.
        g = ["**geomean**"] + [""] * (len(cols) + len(refs))
        for r in refs:
            g.append("**" + fmt(geo([ratio(x, cols[0], r) for x in res.values()]), 2) + "**")
        for r in refs:
            if "jit" in cols:
                g.append("**" + fmt(geo([ratio(x, "jit", r) for x in res.values()]), 2) + "**")
        lines.append("| " + " | ".join(g) + " |")
        lines.append(f"\nTimes in {'milliseconds' if ms else 'seconds'}.\n")
        for c in cols:
            s = {"suite": suite, "ours": c, "n": len(res)}
            for r in refs:
                s[r] = geo([ratio(x, c, r) for x in res.values()])
            if c != "tailcall":
                s["tailcall"] = geo([ratio(x, c, "tailcall") for x in res.values()])
            summary.append(s)
    out = ["## Summary: geometric mean of ours / reference\n",
           "| interpreter | build | vs tailcall | references (ours / ref) |", "|---|---|---:|---|"]
    for s in summary:
        refs = ", ".join(f"{r} {fmt(s[r], 2)}" for r in REFS[s["suite"]] if s.get(r) is not None)
        out.append(f"| {s['suite']} ({s['n']}) | {s['ours']} | {fmt(s.get('tailcall'), 2) if s['ours'] != 'tailcall' else '1'} | {refs or '—'} |")
    return "\n".join(out) + "\n\n## Per suite\n\n" + "\n".join(lines)


def ratio(row, a, b):
    x, y = row.get(a), row.get(b)
    return x / y if x and y else None


def machine():
    def cmd(c):
        try:
            return subprocess.run(c, capture_output=True, text=True).stdout.strip()
        except OSError:
            return "?"
    cpu = "?"
    try:
        with open("/proc/cpuinfo") as f:
            cpu = next(l.split(":", 1)[1].strip() for l in f if l.startswith("model name"))
    except (OSError, StopIteration):
        pass
    return {"cpu": cpu, "cores": os.cpu_count(), "kernel": cmd(["uname", "-r"]),
            "clang": cmd(["clang-19", "--version"]).splitlines()[0] if shutil.which("clang-19") else "?",
            "commit": cmd(["git", "-C", ROOT, "rev-parse", "--short", "HEAD"]),
            "date": time.strftime("%Y-%m-%d"), "load_start": os.getloadavg()[0]}


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--quick", action="store_true", help="one benchmark per suite, one run")
    ap.add_argument("-n", type=int, default=3, help="runs per benchmark (best of n)")
    ap.add_argument("--out", default=os.path.join(os.environ.get("TMPDIR", "/tmp"), "loimi-run-all"))
    ap.add_argument("--only", help="comma-separated suites, of " + ",".join(SUITES))
    ap.add_argument("--report", metavar="JSON", help="only render the tables of an earlier results.json")
    a = ap.parse_args()
    if a.report:
        with open(a.report) as f:
            print(render(json.load(f)["results"]))
        return
    n = 1 if a.quick else a.n
    suites = a.only.split(",") if a.only else SUITES
    os.makedirs(os.path.join(a.out, "raw"), exist_ok=True)
    os.environ["TMPDIR"] = a.out  # the per-suite scripts build and cache under $TMPDIR
    sh(["cargo", "build", "--release", "-q"], cwd=ROOT)
    info = machine()
    path = os.path.join(a.out, "results.json")
    data = {"machine": info, "runs": n, "results": {}}
    if a.only and os.path.exists(path):
        with open(path) as f:
            data["results"] = json.load(f).get("results", {})
    for s in suites:
        t0 = time.time()
        data["results"][s] = run_suite(s, a.out, n, a.quick)
        log(f"== {s}: {time.time() - t0:.0f} s")
        with open(path, "w") as f:
            json.dump(data, f, indent=1)
    info["load_end"] = os.getloadavg()[0]
    with open(path, "w") as f:
        json.dump(data, f, indent=1)
    print(json.dumps(info))
    print(render(data["results"]))


if __name__ == "__main__":
    main()
