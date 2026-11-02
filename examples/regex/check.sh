#!/bin/sh
# Build the regex library and its C host, then: the demo, the bytecode
# fuzzer, the verifier test, random byte patterns (both engines must
# agree), and both engines against Python's `re` on the hand-written
# cases and on random cases.
#
#   examples/regex/check.sh [--random N] [--seed S] [--asan] [--fast]
#
# --fast: 2000 fuzz runs instead of 20000 (tests/golden.rs runs
# `--fast --random 100 --seed 1`). LOIMI and RX_OUT override the
# compiler and the build directory.
set -e
here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../.." && pwd)
out=${RX_OUT:-$here/build}
mkdir -p "$out"
loimi=${LOIMI:-$root/target/release/loimi}
asan=
nrand=2000
seed=1
nfuzz=20000
while [ $# -gt 0 ]; do
    case $1 in
        --random) nrand=$2; shift ;;
        --seed) seed=$2; shift ;;
        --asan) asan=1 ;;
        --fast) nfuzz=2000 ;;
    esac
    shift
done

if [ -n "$asan" ]; then
    # ASan breaks under `ulimit -v`; bound it by RSS instead.
    export ASAN_OPTIONS=hard_rss_limit_mb=2000
    # The archive is instrumented; the host links the ASan runtime
    # (gcc's libasan here, as Loimi's own --sanitize=address does).
    "$loimi" build "$here/lib.lm" --lib --sanitize=address -o "$out/librx_asan.a" --header "$out/rx.h"
    gcc -g -fsanitize=address -I"$out" "$here/host.c" "$out/librx_asan.a" -lm -o "$out/rxhost_asan"
    host=$out/rxhost_asan
else
    "$loimi" build "$here/lib.lm" --release --lib -o "$out/librx.a" --header "$out/rx.h"
    clang-19 -O2 -g -I"$out" "$here/host.c" "$out/librx.a" -lm -o "$out/rxhost"
    host=$out/rxhost
fi
"$host" demo > "$out/demo.out"
diff -u "$here/tests/demo.expected" "$out/demo.out"

# The bytecode fuzzer (mutated programs through the verifier) is an
# executable, so it can use --sanitize=address directly.
if [ -n "$asan" ]; then
    "$loimi" build "$here/fuzz.lm" --sanitize=address -o "$out/rxfuzz_asan"
    timeout 300 "$out/rxfuzz_asan" "$nfuzz" "$seed"
else
    "$loimi" build "$here/fuzz.lm" --release -o "$out/rxfuzz"
    timeout 300 "$out/rxfuzz" "$nfuzz" "$seed"
fi
"$loimi" run "$here/verify_test.lm" > "$out/verify.out"
diff -u "$here/tests/verify.expected" "$out/verify.out"

python3 "$here/tests/gen.py" "$out/cases" "$out/expected" --random "$nrand" --seed "$seed"
timeout 300 "$host" fuzz "$nfuzz" "$seed"
timeout 300 "$host" test "$out/cases" > "$out/actual"
python3 "$here/tests/compare.py" "$out/cases" "$out/expected" "$out/actual"
