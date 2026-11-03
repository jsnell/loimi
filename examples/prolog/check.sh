#!/bin/bash
# Build the Prolog interpreter and run its tests.
#
#   examples/prolog/check.sh [-q] [-u] [TEST ...]
#
# Every tests/NAME.pl must print tests/NAME.expected on stdout (stderr is
# ignored: warnings differ between systems). The expected files are
# SWI-Prolog's output (`swipl -q -g halt NAME.pl`); -u regenerates them with
# bench/prolog/install/swipl (bench/prolog/install.sh builds it).
#
# Default: release builds in all three dispatch modes, each test run
# normally, with --gc-stress (collect at every safepoint) and --gc-verify
# (generational heap, stress, write-barrier verification); then a
# --gc-check-roots build run with --gc-stress. -q: the tail-call build only,
# normally and with --gc-stress (what `cargo test` runs).
set -u
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
DIR=$ROOT/examples/prolog
LOIMI=${LOIMI:-$ROOT/target/release/loimi}
export LOIMI_STD=${LOIMI_STD:-$ROOT}
SWIPL=$ROOT/bench/prolog/install/swipl/bin/swipl
QUICK=0
UPDATE=0
while [ $# -gt 0 ]; do
    case "$1" in
        -q) QUICK=1; shift ;;
        -u) UPDATE=1; shift ;;
        *) break ;;
    esac
done
TESTS=("$@")
if [ ${#TESTS[@]} -eq 0 ]; then
    for f in "$DIR"/tests/*.pl; do TESTS+=("$(basename "$f" .pl)"); done
fi
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

if [ $UPDATE = 1 ]; then
    [ -x "$SWIPL" ] || { echo "no $SWIPL: run bench/prolog/install.sh"; exit 1; }
    for t in "${TESTS[@]}"; do
        (cd "$DIR/tests" && ulimit -v 2500000 && "$SWIPL" -q -g halt "$t.pl" > "$t.expected" 2>/dev/null)
        echo "updated $t.expected"
    done
    exit 0
fi

FAIL=0
COUNT=0
run_suite() {    # exe label runs...
    local exe=$1 label=$2
    shift 2
    for t in "${TESTS[@]}"; do
        for run in "$@"; do
            # Tests marked NAME.nostress allocate too much to collect at
            # every call: they run with a small heap instead.
            if [ -f "$DIR/tests/$t.nostress" ] && [ "$run" != normal ]; then run=tiny; fi
            local flag=""
            [ "$run" = stress ] && flag=--gc-stress
            [ "$run" = verify ] && flag=--gc-verify
            [ "$run" = tiny ] && flag=--gc-tiny
            COUNT=$((COUNT + 1))
            (cd "$DIR/tests" && ulimit -v 2500000 && timeout 300 "$exe" "$t.pl" $flag > "$TMP/out" 2> "$TMP/err")
            if ! cmp -s "$TMP/out" "$DIR/tests/$t.expected"; then
                echo "FAIL $t ($label, $run)"
                diff "$DIR/tests/$t.expected" "$TMP/out" | head -20
                head -5 "$TMP/err"
                FAIL=1
            fi
        done
    done
}

build() {    # out flags...
    local out=$1
    shift
    if ! (cd "$DIR" && ulimit -v 2500000 && "$LOIMI" build main.lm --release -W error "$@" -o "$out") > "$TMP/build.log" 2>&1; then
        echo "FAIL build $*"
        cat "$TMP/build.log"
        FAIL=1
        return 1
    fi
}

if [ $QUICK = 1 ]; then
    build "$TMP/pl" && run_suite "$TMP/pl" tailcall normal stress
else
    for mode in tailcall goto switch; do
        build "$TMP/pl-$mode" --dispatch=$mode && run_suite "$TMP/pl-$mode" $mode normal stress verify
    done
    build "$TMP/pl-roots" --gc-check-roots && run_suite "$TMP/pl-roots" check-roots stress
fi
if [ $FAIL = 0 ]; then echo "ok: $COUNT runs"; fi
exit $FAIL
