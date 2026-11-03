#!/bin/sh
# Build the Python-subset interpreter in every dispatch mode and run the
# tests: each tests/NAME.py must print exactly tests/NAME.expected (CPython's
# output, made by `check.sh -u` with bench/python/install/bin/python3 or
# python3). Each test also runs under GC stress and, in the tail-call build,
# under the generational verify mode and in a --gc-check-roots build.
#   check.sh            all modes
#   check.sh -q         tail-call build only, normal runs only
#   check.sh -u         regenerate the .expected files with CPython
set -u
here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../.." && pwd)
loimi="$root/target/release/loimi"
out="$here/out"
mkdir -p "$out"
ulimit -v 2500000 2>/dev/null || true

if [ "${1:-}" = "-u" ]; then
    py="$root/bench/python/install/bin/python3"
    [ -x "$py" ] || py=python3
    for t in "$here"/tests/*.py; do
        "$py" "$t" > "${t%.py}.expected" 2>/dev/null
        echo "updated $(basename "${t%.py}.expected")"
    done
    exit 0
fi

quick=0
[ "${1:-}" = "-q" ] && quick=1
fail=0
pass=0

build() {   # build NAME ARGS...
    name=$1; shift
    if ! "$loimi" build "$here/main.lm" --release "$@" -o "$out/$name" >"$out/$name.log" 2>&1; then
        echo "BUILD FAILED: $name"; cat "$out/$name.log"; exit 1
    fi
}

run() {     # run BINARY LABEL FLAGS...
    bin=$1; label=$2; shift 2
    for t in "$here"/tests/*.py; do
        exp="${t%.py}.expected"
        [ -f "$exp" ] || continue
        got=$("$bin" "$t" "$@" 2>/dev/null)
        if [ "$got" = "$(cat "$exp")" ]; then
            pass=$((pass + 1))
        else
            fail=$((fail + 1))
            echo "FAIL [$label] $(basename "$t")"
            printf '%s\n' "$got" | diff - "$exp" | head -5
        fi
    done
}

build python-tailcall --dispatch=tailcall
run "$out/python-tailcall" tailcall
if [ $quick -eq 0 ]; then
    run "$out/python-tailcall" tailcall-stress --gc-stress
    run "$out/python-tailcall" tailcall-verify --gc-verify
    build python-goto --dispatch=goto
    run "$out/python-goto" goto
    run "$out/python-goto" goto-stress --gc-stress
    build python-switch --dispatch=switch
    run "$out/python-switch" switch
    run "$out/python-switch" switch-stress --gc-stress
    build python-checkroots --gc-check-roots
    run "$out/python-checkroots" checkroots-stress --gc-stress
fi
echo "$pass passed, $fail failed"
[ $fail -eq 0 ]
