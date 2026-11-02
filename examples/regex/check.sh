#!/bin/sh
# Build the regex library and its C host, then check both engines against
# Python's `re` on the hand-written cases and on random cases.
#
#   examples/regex/check.sh [--random N] [--seed S] [--asan]
set -e
here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../.." && pwd)
out=${RX_OUT:-$here/build}
mkdir -p "$out"
loimi=$root/target/release/loimi
asan=
nrand=2000
seed=1
while [ $# -gt 0 ]; do
    case $1 in
        --random) nrand=$2; shift ;;
        --seed) seed=$2; shift ;;
        --asan) asan=1 ;;
    esac
    shift
done

if [ -n "$asan" ]; then
    # ASan breaks under `ulimit -v`; bound it by RSS instead.
    export ASAN_OPTIONS=hard_rss_limit_mb=2000
    "$loimi" build "$here/lib.lm" --lib --sanitize=address -o "$out/librx_asan.a" --header "$out/rx.h"
    clang-19 -g -fsanitize=address -I"$out" "$here/host.c" "$out/librx_asan.a" -lm -o "$out/rxhost_asan"
    host=$out/rxhost_asan
else
    "$loimi" build "$here/lib.lm" --release --lib -o "$out/librx.a" --header "$out/rx.h"
    clang-19 -O2 -g -I"$out" "$here/host.c" "$out/librx.a" -lm -o "$out/rxhost"
    host=$out/rxhost
fi

python3 "$here/tests/gen.py" "$out/cases" "$out/expected" --random "$nrand" --seed "$seed"
"$host" test "$out/cases" > "$out/actual"
python3 "$here/tests/compare.py" "$out/cases" "$out/expected" "$out/actual"
