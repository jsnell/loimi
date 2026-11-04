"""Differential testing of python-loimi against CPython with programs
from gen.py.

    python3 run.py LOIMI_PY [--seeds A:B] [--snippets N] [--gc-stress-every K]
                            [--out DIR] [--only g_name,...] [--quiet]
                            [--fail-on crash,semantic,message]

Runs each seed's program on CPython (bench/python/install/bin/python3, or
$CPYTHON) and on LOIMI_PY, compares stdout line by line and reports, per
mismatching snippet, the snippet (a minimized program: the prelude and
that snippet) and both outputs. Exit status 1 if a mismatch of a kind
in --fail-on (default: all) was found, 0 otherwise; 77 if CPython is
not available. check.sh -q runs a short fixed-seed smoke campaign.

Mismatches are classified as `crash` (signal or internal error),
`semantic` (a different value or exception type) and `message` (same
exception type, different message).
"""
import os
import resource
import subprocess
import sys
import tempfile

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import gen

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", "..", ".."))
CPY = os.environ.get("CPYTHON", os.path.join(ROOT, "bench", "python", "install", "bin", "python3"))


def limits():
    resource.setrlimit(resource.RLIMIT_AS, (2500000 * 1024, 2500000 * 1024))
    resource.setrlimit(resource.RLIMIT_CORE, (0, 0))


def run(cmd, timeout=20):
    try:
        p = subprocess.run(cmd, capture_output=True, timeout=timeout, preexec_fn=limits)
        return p.returncode, p.stdout.decode(errors="replace"), p.stderr.decode(errors="replace")
    except subprocess.TimeoutExpired:
        return "timeout", "", ""


def snippet_of(src, name):
    """The prelude plus the chunk(s) defining snippet `name`."""
    chunks = src.split("\n\n")
    keep = [chunks[0]]
    for i, ch in enumerate(chunks):
        if ("_run(%r" % name) in ch:
            j = i
            while j > 1 and (chunks[j - 1].startswith("class ") or chunks[j - 1].startswith("def _%s(" % name)):
                j -= 1
            keep += chunks[j:i + 1]
            break
    return "\n\n".join(keep) + "\n"


def blocks(out):
    """Output by snippet: each snippet's result line (`sN ...`) with the
    lines its code printed before it."""
    res, pending = {}, []
    for line in out.splitlines():
        head = line.split(" ", 1)[0]
        if head[:1] == "s" and head[1:].isdigit() and head not in res:
            res[head] = "\n".join(pending + [line])
            pending = []
        else:
            pending.append(line)
    if pending:
        res["s999999"] = "\n".join(pending)
    return res


def find_culprit(loimi, src, names, tmp, flags):
    path = os.path.join(tmp, "culprit.py")
    for name in names:
        with open(path, "w") as f:
            f.write(snippet_of(src, name))
        rc, out, err = run([loimi, path] + flags, timeout=10)
        if rc != 0:
            os.remove(path)
            return name, "<alone: rc %s, stderr %s>" % (rc, err.strip()[-200:])
    os.remove(path)
    return None


def classify(a, b):
    a, b = a.split("\n")[-1], b.split("\n")[-1]
    if " EXC " in a and " EXC " in b:
        ta, tb = a.split(" EXC ", 1)[1].split(" |")[0], b.split(" EXC ", 1)[1].split(" |")[0]
        if ta == tb:
            return "message"
    return "semantic"


def main():
    args = sys.argv[1:]
    if not args:
        print(__doc__)
        return 2
    loimi = args.pop(0)
    seeds = range(0, 100)
    nsnip = 40
    stress_every = 0
    outdir = None
    only = None
    quiet = False
    fail_on = ["crash", "semantic", "message"]
    while args:
        a = args.pop(0)
        if a == "--seeds":
            lo, hi = args.pop(0).split(":")
            seeds = range(int(lo), int(hi))
        elif a == "--snippets":
            nsnip = int(args.pop(0))
        elif a == "--gc-stress-every":
            stress_every = int(args.pop(0))
        elif a == "--out":
            outdir = args.pop(0)
        elif a == "--only":
            only = args.pop(0).split(",")
        elif a == "--fail-on":
            fail_on = args.pop(0).split(",")
        elif a == "--quiet":
            quiet = True
        else:
            print("unknown argument", a)
            return 2
    if not os.access(CPY, os.X_OK):
        print("run.py: no CPython at %s, skipped" % CPY)
        return 77
    tmp = outdir or tempfile.mkdtemp(prefix="pyfuzz-")
    os.makedirs(tmp, exist_ok=True)
    counts = {"crash": 0, "semantic": 0, "message": 0}
    seen = set()
    programs = 0
    for seed in seeds:
        src = gen.program(seed, nsnip, only)
        path = os.path.join(tmp, "p%d.py" % seed)
        with open(path, "w") as f:
            f.write(src)
        programs += 1
        flags = ["--gc-stress"] if stress_every and seed % stress_every == 0 else []
        rc_c, out_c, err_c = run([CPY, path])
        rc_l, out_l, err_l = run([loimi, path] + flags)
        if (rc_l, out_l) == (rc_c, out_c):
            os.remove(path)
            continue
        bc, bl = blocks(out_c), blocks(out_l)
        reported = False
        for name in sorted(set(bc) | set(bl), key=lambda n: int(n[1:]) if n[1:].isdigit() else 0):
            a = bc.get(name, "<missing>")
            b = bl.get(name, "<missing>")
            if a == b:
                continue
            if b == "<missing>" and rc_l != 0:
                kind = "crash"
                b = "<missing: rc %s, stderr %s>" % (rc_l, err_l.strip()[-200:])
                # Whole-program failure (a timeout loses buffered output, a
                # SyntaxError stops everything): find the snippet alone.
                culprit = find_culprit(loimi, src, sorted(bc, key=lambda n: int(n[1:]) if n[1:].isdigit() else 0), tmp, flags)
                if culprit:
                    name, b = culprit
                    a = bc.get(name, "<missing>")
            else:
                kind = classify(a, b)
            counts[kind] += 1
            reported = True
            key = (kind, a, b)
            if key in seen:
                if kind == "crash":
                    break
                continue
            seen.add(key)
            if not quiet:
                print("=== seed %d %s %s%s" % (seed, kind, name, " (--gc-stress)" if flags else ""))
                print("  cpython: %s" % a[:400].replace("\n", "\n           "))
                print("  loimi:   %s" % b[:400].replace("\n", "\n           "))
                print("  " + snippet_of(src, name).split("\n\n", 1)[-1].strip().replace("\n", "\n  "))
            if kind == "crash":
                break
        if not reported:
            counts["crash"] += 1
            if not quiet:
                print("=== seed %d exit status %s vs %s: %s" % (seed, rc_l, rc_c, err_l.strip()[-300:]))
        if outdir is None:
            os.remove(path)
    if outdir is None:
        os.rmdir(tmp)
    print("programs %d, snippets %d, mismatches: %s" % (programs, programs * nsnip,
          ", ".join("%s %d" % kv for kv in counts.items())))
    return 1 if any(counts[k] for k in fail_on) else 0


if __name__ == "__main__":
    sys.exit(main())
