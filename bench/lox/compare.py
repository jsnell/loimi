#!/usr/bin/env python3
"""Run the official Lox benchmarks under clox and other Lox binaries.

Each benchmark runs under every binary in turn (interleaved), N rounds;
the table shows the best wall-clock time of each. Outputs are compared
against clox's, ignoring lines that are timings (non-integral numbers).

usage: compare.py [-n ROUNDS] NAME=BINARY... [-- BENCH...]
  e.g. compare.py -n 5 lox=./lox

zoo_batch runs for a fixed 10 s and counts batches; its column is
seconds per 1000 batches. string_equality does not compile under clox
(more than 256 constants in one chunk), so it is skipped.
"""
import math, os, re, subprocess, sys, time

HERE = os.path.dirname(os.path.abspath(__file__))
BENCH = os.path.join(HERE, "install", "test", "benchmark")
CLOX = os.path.join(HERE, "install", "bin", "clox")
TIMING = re.compile(r"^-?\d+\.\d+(e[+-]\d+)?$|^-?\d+e[+-]\d+$")


def run(binary, path):
    t = time.perf_counter()
    r = subprocess.run([binary, path], capture_output=True, timeout=600)
    dt = time.perf_counter() - t
    lines = r.stdout.decode().splitlines()
    if path.endswith("zoo_batch.lox") and r.returncode == 0:
        # Runs for 10 s of clock() and prints the number of batches done:
        # score it as seconds per 1000 batches, and do not compare outputs.
        dt = 1000 * dt / max(1, int(lines[1]))
        return dt, r.returncode, []
    out = [l for l in lines if not TIMING.match(l)]
    return dt, r.returncode, out


def main():
    args = sys.argv[1:]
    rounds = 5
    if args[:1] == ["-n"]:
        rounds = int(args[1])
        args = args[2:]
    benches = None
    if "--" in args:
        i = args.index("--")
        benches = args[i + 1:]
        args = args[:i]
    impls = [("clox", CLOX)] + [tuple(a.split("=", 1)) for a in args]
    names = sorted(f[:-4] for f in os.listdir(BENCH) if f.endswith(".lox") and f != "string_equality.lox")
    if benches:
        names = [n for n in names if n in benches]
    best = {(b, i): math.inf for b in names for i, _ in impls}
    for b in names:
        path = os.path.join(BENCH, b + ".lox")
        ref = None
        for r in range(rounds):
            # Rotate the starting binary each round.
            order = impls[r % len(impls):] + impls[:r % len(impls)]
            for name, binary in order:
                dt, code, out = run(binary, path)
                if code != 0:
                    print("%s: %s exited with %d" % (b, name, code), file=sys.stderr)
                if name == "clox":
                    ref = ref or out
                elif ref is not None and out != ref:
                    print("%s: %s output differs from clox: %r vs %r" % (b, name, out, ref), file=sys.stderr)
                best[(b, name)] = min(best[(b, name)], dt)
        sys.stderr.write(".")
        sys.stderr.flush()
    sys.stderr.write("\n")
    hdr = "| benchmark | " + " | ".join(n for n, _ in impls) + " | " + " | ".join(
        "%s/clox" % n for n, _ in impls[1:]) + " |"
    print(hdr)
    print("|---" * (2 * len(impls)) + "|")
    logs = {n: [] for n, _ in impls[1:]}
    for b in names:
        c = best[(b, "clox")]
        row = [b] + ["%.3f" % best[(b, n)] for n, _ in impls]
        for n, _ in impls[1:]:
            ratio = best[(b, n)] / c
            logs[n].append(math.log(ratio))
            row.append("%.2f" % ratio)
        print("| " + " | ".join(row) + " |")
    geo = ["**geomean**"] + [""] * len(impls) + [
        "**%.2f**" % math.exp(sum(logs[n]) / len(logs[n])) for n, _ in impls[1:]]
    print("| " + " | ".join(geo) + " |")


if __name__ == "__main__":
    main()
