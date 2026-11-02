#!/bin/sh
# Build the library in each dispatch mode, run the tests and the
# backtracker benchmarks in each.
#
#   examples/regex/dispatch.sh [--random N]
set -e
here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../.." && pwd)
out=${RX_OUT:-$here/build}
mkdir -p "$out"
nrand=${2:-500}
python3 "$here/tests/gen.py" "$out/cases" "$out/expected" --random "$nrand" --seed 5
for d in tailcall goto switch; do
    "$root/target/release/loimi" build "$here/lib.lm" --release --lib --dispatch=$d -o "$out/librx_$d.a" --header "$out/rx.h"
    clang-19 -O2 -I"$out" "$here/host.c" "$out/librx_$d.a" -lm -o "$out/rxhost_$d"
    echo "== --dispatch=$d"
    "$out/rxhost_$d" test "$out/cases" > "$out/actual_$d"
    python3 "$here/tests/compare.py" "$out/cases" "$out/expected" "$out/actual_$d"
    "$out/rxhost_$d" bench | cut -c1-48
done
