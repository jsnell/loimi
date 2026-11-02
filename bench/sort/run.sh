#!/bin/sh
# Build the sort comparator benchmark and report the best of 7 runs of
# each mode (CPU time of the sort alone, as the program measures it).
set -e
cd "$(dirname "$0")"
LOIMI=${LOIMI:-../../target/release/loimi}
OUT=${TMPDIR:-/tmp}/loimi-bench
mkdir -p "$OUT"
LOIMI_STD=../.. $LOIMI build sort.lm --release -o "$OUT/sort"
python3 - "$OUT/sort" <<'PY'
import subprocess, sys
exe = sys.argv[1]
for mode in ["direct", "closure", "plainfn", "fnptr", "closure-ni", "plainfn-ni", "fnptr-ni"]:
    times = []
    for _ in range(7):
        out = subprocess.run([exe, mode], capture_output=True, text=True).stdout.split()
        times.append(int(out[1]))
    print(f"{mode:<11} {min(times)/1000:7.1f} ms   (check {out[-1]})")
PY
