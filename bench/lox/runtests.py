#!/usr/bin/env python3
"""Run the official Crafting Interpreters test suite against a Lox binary.

A Python port of the checking logic in the book repo's tool/bin/test.dart,
with the "clox" suite's expectations: everything under test/ except the
early-chapter directories test/scanning and test/expressions (and
test/benchmark, which holds no expectations).

usage: runtests.py INTERPRETER [FILTER...] [-v]
"""
import os, re, subprocess, sys

HERE = os.path.dirname(os.path.abspath(__file__))
TESTS = os.path.join(HERE, "install", "test")

EXPECT_OUT = re.compile(r"// expect: ?(.*)")
EXPECT_ERR = re.compile(r"// (Error.*)")
ERR_LINE = re.compile(r"// \[((java|c) )?line (\d+)\] (Error.*)")
EXPECT_RT = re.compile(r"// expect runtime error: (.+)")
SYNTAX_ERR = re.compile(r"\[.*line (\d+)\] (Error.+)")
STACK = re.compile(r"\[line (\d+)\]")
NONTEST = re.compile(r"// nontest")

SKIP_DIRS = ("scanning", "expressions", "benchmark")


def parse(path):
    out, errs, exit_code, rt, rt_line = [], set(), 0, None, None
    with open(path, encoding="utf-8") as f:
        for n, line in enumerate(f.read().split("\n"), 1):
            if NONTEST.search(line):
                return None
            m = EXPECT_OUT.search(line)
            if m:
                out.append((n, m.group(1)))
                continue
            m = EXPECT_ERR.search(line)
            if m:
                errs.add("[%d] %s" % (n, m.group(1)))
                exit_code = 65
                continue
            m = ERR_LINE.search(line)
            if m:
                if m.group(2) in (None, "c"):
                    errs.add("[%s] %s" % (m.group(3), m.group(4)))
                    exit_code = 65
                continue
            m = EXPECT_RT.search(line)
            if m:
                rt_line, rt, exit_code = n, m.group(1), 70
    return out, errs, exit_code, rt, rt_line


def run_one(interp, path):
    spec = parse(path)
    if spec is None:
        return None
    out, errs, exit_code, rt, rt_line = spec
    try:
        r = subprocess.run([interp, path], capture_output=True, timeout=30)
    except subprocess.TimeoutExpired:
        return ["timeout"]
    so = r.stdout.decode("utf-8", "replace").splitlines()
    se = r.stderr.decode("utf-8", "replace").splitlines()
    fails = []
    if rt is not None:
        if len(se) < 2:
            fails.append("Expected runtime error '%s' and got none." % rt)
        else:
            if se[0] != rt:
                fails.append("Expected runtime error '%s' and got: %s" % (rt, se[0]))
            m = None
            for l in se[1:]:
                m = STACK.search(l)
                if m:
                    break
            if not m:
                fails.append("Expected stack trace and got: %r" % se[1:])
            elif int(m.group(1)) != rt_line:
                fails.append("Expected runtime error on line %d but was on line %s" % (rt_line, m.group(1)))
    else:
        found = set()
        for l in se:
            m = SYNTAX_ERR.search(l)
            if m:
                e = "[%s] %s" % (m.group(1), m.group(2))
                if e in errs:
                    found.add(e)
                else:
                    fails.append("Unexpected error: " + l)
            elif l != "":
                fails.append("Unexpected output on stderr: " + l)
        for e in sorted(errs - found):
            fails.append("Missing expected error: " + e)
    if r.returncode != exit_code:
        fails.append("Expected return code %d and got %d" % (exit_code, r.returncode))
    if so and so[-1] == "":
        so.pop()
    for i, l in enumerate(so):
        if i >= len(out):
            fails.append("Got output '%s' when none was expected." % l)
        elif out[i][1] != l:
            fails.append("Expected output '%s' on line %d and got '%s'." % (out[i][1], out[i][0], l))
    for n, e in out[len(so):]:
        fails.append("Missing expected output '%s' on line %d." % (e, n))
    return fails


def main():
    args = [a for a in sys.argv[1:] if a != "-v"]
    verbose = "-v" in sys.argv
    interp = os.path.abspath(args[0])
    filters = args[1:]
    paths = []
    for root, dirs, files in os.walk(TESTS):
        rel = os.path.relpath(root, TESTS)
        if rel.split(os.sep)[0] in SKIP_DIRS:
            continue
        for f in files:
            if f.endswith(".lox"):
                paths.append(os.path.join(root, f))
    paths.sort()
    if filters:
        paths = [p for p in paths if any(f in p for f in filters)]
    passed = failed = 0
    for p in paths:
        fails = run_one(interp, p)
        if fails is None:
            continue
        if fails:
            failed += 1
            print("FAIL", os.path.relpath(p, TESTS))
            if verbose:
                for f in fails[:10]:
                    print("     ", f)
        else:
            passed += 1
    print("passed %d failed %d (of %d)" % (passed, failed, passed + failed))
    sys.exit(1 if failed else 0)


if __name__ == "__main__":
    main()
