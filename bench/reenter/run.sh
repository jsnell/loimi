#!/bin/sh
# Re-entry cost: Mini's native table.sort calling a Lua comparator through
# a nested run per comparison (sort_native), against the same algorithm
# written in Lua, where each comparison is an ordinary call inside the one
# dispatch loop (sort_prelude), and the native sort comparing with `<`
# (sort_native_lt). sort_setup is the table filling and checking that all
# three include. Reports the best of 7 runs (user + system CPU time).
#
# 2026-11-02, release build, tail-call dispatch, loaded 8-core machine,
# 200,000 integers, 3,843,947 comparisons in both Lua-comparator runs:
#
#   sort_setup        11 ms
#   sort_native_lt    63 ms   ->  13 ns per comparison
#   sort_native      139 ms   ->  33 ns per comparison
#   sort_prelude     117 ms   ->  27 ns per comparison
#
# A nested run costs about 20 ns per comparison over the native `<`, of
# which the comparator's own instructions are a third or so; the in-loop
# version is 1.2x faster overall. See docs/lowering.md §4.
set -e
cd "$(dirname "$0")"
LOIMI=${LOIMI:-../../target/release/loimi}
OUT=${TMPDIR:-/tmp}/loimi-bench
mkdir -p "$OUT"
LOIMI_STD=../.. $LOIMI build ../../examples/mini/main.lm --release -o "$OUT/mini"
python3 - "$OUT/mini" <<'PY'
import resource, subprocess, sys
exe = sys.argv[1]
def cpu(prog):
    r0 = resource.getrusage(resource.RUSAGE_CHILDREN)
    subprocess.run([exe, prog + ".lua"], stdout=subprocess.DEVNULL, check=True)
    r1 = resource.getrusage(resource.RUSAGE_CHILDREN)
    return r1.ru_utime - r0.ru_utime + r1.ru_stime - r0.ru_stime
for prog in ["sort_setup", "sort_native_lt", "sort_native", "sort_prelude"]:
    print(f"{prog:<15} {min(cpu(prog) for _ in range(7)) * 1000:6.0f} ms")
PY
