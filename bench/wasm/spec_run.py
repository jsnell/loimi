#!/usr/bin/env python3
"""Run the official WebAssembly spec tests (wast2json output) against
examples/wasm.

    spec_run.py --wasm PATH [--json-dir DIR] [-v] [NAME ...]

For each <name>/<name>.json under DIR (default bench/wasm/spec-json), the
commands are turned into a line-based script that the interpreter runs in
one process (`wasm --script FILE`), one output line per command:

    module PATH [NAME]          -> ok | malformed MSG | invalid MSG | unlinkable MSG | trap MSG
    check PATH                  -> the same, but the module is not kept
    register AS [NAME]          -> ok
    invoke NAME|- FIELD ARGS..  -> ok v:HEX.. | trap MSG
    get NAME|- FIELD            -> ok TYPE:HEX

Names are hex-encoded UTF-8 (export names in the suite contain spaces and
arbitrary Unicode). Text-format modules (assert_malformed/assert_invalid
on .wat) are skipped: the interpreter only reads the binary format.
"""
import argparse
import json
import os
import shlex
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))


def hexname(s):
    return s.encode("utf-8").hex() or "00"[:0] or "-empty-"


def enc(s):
    # The empty name must still be a token.
    h = s.encode("utf-8").hex()
    return h if h else "EMPTY"


MASK = {"i32": 0xFFFFFFFF, "f32": 0xFFFFFFFF, "i64": (1 << 64) - 1, "f64": (1 << 64) - 1}


def ref_bits(val):
    # null is 0; host reference N is N + 1
    return 0 if val == "null" else int(val) + 1


def arg_token(v):
    t = v["type"]
    if t in ("externref", "funcref"):
        if t == "funcref" and v["value"] != "null":
            return None
        return "ref:%x" % ref_bits(v["value"])
    if t not in MASK:
        return None
    val = v["value"]
    if not isinstance(val, str) or val.startswith("nan"):
        return None
    return "%s:%x" % (t, int(val) & MASK[t])


def match_value(exp, got_bits):
    t = exp["type"]
    if t in ("externref", "funcref"):
        val = exp.get("value")
        if val is None or (t == "funcref" and val != "null"):
            return got_bits != 0   # some non-null function
        return got_bits == ref_bits(val)
    if t not in MASK:
        return False
    bits = got_bits & MASK[t]
    val = exp["value"]
    if val == "nan:canonical":
        if t == "f32":
            return bits & 0x7FFFFFFF == 0x7FC00000
        return bits & 0x7FFFFFFFFFFFFFFF == 0x7FF8000000000000
    if val == "nan:arithmetic":
        if t == "f32":
            return bits & 0x7FC00000 == 0x7FC00000
        return bits & 0x7FF8000000000000 == 0x7FF8000000000000
    return bits == int(val) & MASK[t]


def action_line(a):
    mod = enc(a["module"]) if a.get("module") else "-"
    if a["type"] == "get":
        return "get %s %s" % (mod, enc(a["field"]))
    toks = []
    for v in a.get("args", []):
        tok = arg_token(v)
        if tok is None:
            return None
        toks.append(tok)
    return "invoke %s %s %s" % (mod, enc(a["field"]), " ".join(toks))


def run_file(wasm, jpath, verbose, timeout):
    d = os.path.dirname(jpath)
    with open(jpath) as f:
        data = json.load(f)
    lines = []   # (script line, command, kind)
    skipped = 0
    for c in data["commands"]:
        t = c["type"]
        if t == "module":
            if c.get("module_type", "binary") != "binary":
                skipped += 1
                continue
            line = "module %s" % os.path.join(d, c["filename"])
            if c.get("name"):
                line += " " + enc(c["name"])
            lines.append((line, c, "module"))
        elif t in ("assert_invalid", "assert_malformed", "assert_unlinkable", "assert_uninstantiable"):
            if c.get("module_type", "binary") != "binary":
                skipped += 1
                continue
            lines.append(("check %s" % os.path.join(d, c["filename"]), c, t))
        elif t == "register":
            line = "register %s" % enc(c["as"])
            if c.get("name"):
                line += " " + enc(c["name"])
            lines.append((line, c, "register"))
        elif t in ("assert_return", "action", "assert_trap", "assert_exhaustion"):
            line = action_line(c["action"])
            if line is None:
                skipped += 1
                continue
            lines.append((line, c, t))
        else:
            skipped += 1
    script = os.path.join(d, "script.txt")
    with open(script, "w") as f:
        for line, _, _ in lines:
            f.write(line + "\n")
    out_lines = []
    crashed = None
    try:
        # LOIMI_RUNNER (e.g. "qemu-aarch64 -L SYSROOT") runs a cross-built interpreter.
        runner = shlex.split(os.environ.get("LOIMI_RUNNER", ""))
        p = subprocess.run(["sh", "-c", "ulimit -v 2500000; exec \"$@\"", "sh"] + runner + [wasm, "--script", script],
                           capture_output=True, timeout=timeout * (5 if runner else 1))
        out_lines = p.stdout.decode("utf-8", "replace").split("\n")
        if p.returncode != 0:
            crashed = "exit status %d: %s" % (p.returncode, p.stderr.decode("utf-8", "replace")[-300:])
    except subprocess.TimeoutExpired as e:
        out_lines = (e.stdout or b"").decode("utf-8", "replace").split("\n")
        crashed = "timeout"
    passed = failed = 0
    msg_mismatch = 0
    failures = []
    for i, (line, c, kind) in enumerate(lines):
        got = out_lines[i] if i < len(out_lines) and (i < len(out_lines) - 1 or crashed is None) else None
        ok = False
        if got is None:
            ok = False
            got = "<no output: %s>" % crashed
        elif kind == "module" or kind == "register":
            ok = got == "ok"
        elif kind == "assert_invalid":
            ok = got.startswith("invalid")
        elif kind == "assert_malformed":
            ok = got.startswith("malformed")
        elif kind == "assert_unlinkable":
            ok = got.startswith("unlinkable")
        elif kind == "assert_uninstantiable":
            ok = got.startswith("trap")
        elif kind in ("assert_trap", "assert_exhaustion"):
            ok = got.startswith("trap")
            if ok and not got[5:].startswith(c.get("text", "")[:len(got) - 5]):
                if c.get("text", "") not in got:
                    msg_mismatch += 1
        elif kind in ("assert_return", "action"):
            exp = c.get("expected", [])
            toks = got.split()
            if toks and toks[0] == "ok":
                vals = toks[1:]
                if c["action"]["type"] == "get":
                    vals = [v.split(":")[1] for v in vals]
                else:
                    vals = [v[2:] for v in vals]
                if kind == "action":
                    ok = True
                elif len(vals) == len(exp):
                    ok = all(match_value(e, int(v, 16)) for e, v in zip(exp, vals))
                    # either-of results (relaxed): accept any
            else:
                ok = False
        if ok:
            passed += 1
        else:
            failed += 1
            failures.append("  line %s: %s\n    %s\n    got: %s" % (c.get("line"), kind, line[:200], got[:200]))
    return passed, failed, skipped, msg_mismatch, failures, crashed


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--wasm", required=True)
    ap.add_argument("--json-dir", default=os.path.join(HERE, "spec-json"))
    ap.add_argument("-v", "--verbose", action="store_true")
    ap.add_argument("--timeout", type=int, default=120)
    ap.add_argument("--markdown", action="store_true")
    ap.add_argument("--skip", action="append", default=[], help="skip files whose name contains this")
    ap.add_argument("names", nargs="*")
    a = ap.parse_args()
    names = a.names or sorted(n for n in os.listdir(a.json_dir) if os.path.isdir(os.path.join(a.json_dir, n)))
    names = [n for n in names if not any(k in n for k in a.skip)]
    tp = tf = ts = 0
    rows = []
    for n in names:
        j = os.path.join(a.json_dir, n, n + ".json")
        if not os.path.exists(j):
            continue
        p, f, s, mm, fails, crashed = run_file(a.wasm, j, a.verbose, a.timeout)
        tp += p
        tf += f
        ts += s
        rows.append((n, p, f, s, crashed))
        if a.verbose and fails:
            print("%s: %d failures%s" % (n, len(fails), (" (" + crashed + ")") if crashed else ""))
            for x in fails[:30]:
                print(x)
    if a.markdown:
        print("| file | passed | failed | skipped |")
        print("|---|---:|---:|---:|")
        for n, p, f, s, cr in rows:
            print("| %s | %d | %d | %d |%s" % (n, p, f, s, " crashed" if cr else ""))
    else:
        for n, p, f, s, cr in rows:
            print("%-32s %6d passed %5d failed %5d skipped%s" % (n, p, f, s, ("  [" + cr[:60] + "]") if cr else ""))
    print("TOTAL: %d passed, %d failed, %d skipped; files fully passing: %d/%d" %
          (tp, tf, ts, sum(1 for r in rows if r[2] == 0), len(rows)))


if __name__ == "__main__":
    main()
