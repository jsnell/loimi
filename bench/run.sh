#!/bin/sh
# Build and time the fib benchmark: Loimi in each dispatch mode vs C.
set -e
cd "$(dirname "$0")"
LOIMI=${LOIMI:-../target/release/loimi}
OUT=${TMPDIR:-/tmp}/loimi-bench
mkdir -p "$OUT"
for d in tailcall goto switch; do
    LOIMI_STD=.. $LOIMI build fib.lm --release --dispatch=$d -o "$OUT/fib-$d"
done
clang-19 -O3 fib.c -o "$OUT/fib-c"
echo "result: $($OUT/fib-tailcall)"
python3 - "$OUT" <<'PY'
import subprocess, sys, time
out = sys.argv[1]
for v in ["tailcall", "goto", "switch", "c"]:
    best = min(
        (lambda s: (subprocess.run([f"{out}/fib-{v}"], stdout=subprocess.DEVNULL), time.perf_counter() - s)[1])(time.perf_counter())
        for _ in range(7))
    print(f"{v:<10} {best*1000:7.1f} ms")
PY
