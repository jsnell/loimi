#!/bin/bash
# Bytecode fuzzing under AddressSanitizer.
#
#   tests/fuzz/run.sh [options] [TARGET...]
#
# Targets:
#   vm    tests/fuzz/fuzzvm.lm: a test vm using every verifier feature,
#         run in all three dispatch modes (default)
#   mini  tests/fuzz/minifuzz.lm: random code against Mini's verify + run
#
# Options:
#   -t SECONDS   time budget per target (default 60)
#   -n SEEDS     stop after this many seeds per target instead
#   -s SEED      first seed (default: random)
#   -b BATCH     seeds per process (default 2000 for vm, 500 for mini)
#   --release    build with --release (-O3, no overflow checks)
#   --no-asan    plain build (faster; crashes only)
#
# Each batch runs in one process. When a process dies (ASan report,
# signal, timeout), its last "seed N" line names the culprit: the seed is
# saved and fuzzing resumes after it. Results go to $FUZZ_OUT (default
# target/fuzz): <target>.failures lists failing seeds with the reason,
# <target>-seed-N.log holds each crash report, and history.log one line
# per run (date, target, seeds, seconds, failures). Reproduce a seed with
#   <binary> N 1 -d     (prints the program, then runs it)
#
# Memory: at most one ASan process runs at a time; ASan's own RSS limit
# (hard_rss_limit_mb) replaces `ulimit -v`, which ASan cannot run under.

set -u
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
OUT=${FUZZ_OUT:-$ROOT/target/fuzz}
LOIMI=${LOIMI:-$ROOT/target/release/loimi}
TIME=60
SEEDS=0
FIRST=""
BATCH=""
SAN="--sanitize=address"
REL=""
TARGETS=()
while [ $# -gt 0 ]; do
    case "$1" in
        -t) TIME=$2; shift 2 ;;
        -n) SEEDS=$2; shift 2 ;;
        -s) FIRST=$2; shift 2 ;;
        -b) BATCH=$2; shift 2 ;;
        --release) REL="--release"; shift ;;
        --no-asan) SAN=""; shift ;;
        vm|mini) TARGETS+=("$1"); shift ;;
        *) echo "usage: $0 [-t SECONDS | -n SEEDS] [-s SEED] [-b BATCH] [--release] [--no-asan] [vm] [mini]" >&2; exit 2 ;;
    esac
done
[ ${#TARGETS[@]} -eq 0 ] && TARGETS=(vm)
[ -z "$FIRST" ] && FIRST=$(( (RANDOM << 30 | RANDOM << 15 | RANDOM) * 1000 ))
mkdir -p "$OUT"
if [ ! -x "$LOIMI" ]; then
    (cd "$ROOT" && ulimit -v 2500000 && CARGO_BUILD_JOBS=2 cargo build --release) || exit 1
fi
export LOIMI_STD=$ROOT
export ASAN_OPTIONS=${ASAN_OPTIONS:-hard_rss_limit_mb=1500:exitcode=86:detect_leaks=0}

build() {   # build TARGET -> sets BIN
    local src
    BIN=$OUT/$1${SAN:+-asan}${REL:+-release}
    if [ "$1" = vm ]; then
        src=$ROOT/tests/fuzz/fuzzvm.lm
    else
        # Mini's modules, unchanged, next to the driver.
        local dir=$OUT/mini-src
        rm -rf "$dir"
        mkdir -p "$dir"
        for m in value vm lib; do ln -s "$ROOT/examples/mini/$m.lm" "$dir/$m.lm"; done
        ln -s "$ROOT/tests/fuzz/minifuzz.lm" "$dir/minifuzz.lm"
        src=$dir/minifuzz.lm
    fi
    echo "building $1: loimi build $SAN $REL"
    (ulimit -v 2500000; "$LOIMI" build "$src" $SAN $REL -o "$BIN") || exit 1
}

fuzz() {   # fuzz TARGET
    local t=$1 batch=$BATCH seed=$FIRST start=$SECONDS done_seeds=0 fails=0 timeouts=0 ran=0
    [ -z "$batch" ] && { [ "$t" = vm ] && batch=2000 || batch=500; }
    build "$t"
    local log=$OUT/$t.batch
    echo "fuzzing $t from seed $seed ($([ "$SEEDS" -gt 0 ] && echo "$SEEDS seeds" || echo "${TIME}s"))"
    while :; do
        if [ "$SEEDS" -gt 0 ]; then
            [ $done_seeds -ge "$SEEDS" ] && break
            [ $((SEEDS - done_seeds)) -lt "$batch" ] && batch=$((SEEDS - done_seeds))
        else
            [ $((SECONDS - start)) -ge "$TIME" ] && break
        fi
        # (Quiet: bash would report a child killed by a signal.)
        if [ -n "$SAN" ]; then
            (timeout 600 "$BIN" "$seed" "$batch" -v > "$log.out" 2> "$log.err"; exit $?) 2> /dev/null
        else
            (ulimit -v 2500000; timeout 600 "$BIN" "$seed" "$batch" -v > "$log.out" 2> "$log.err"; exit $?) 2> /dev/null
        fi
        local st=$?
        ran=$((ran + $(grep -c '^ran$' "$log.err")))
        if [ $st -eq 0 ] || [ $st -eq 1 ]; then
            # Checks the driver reports itself.
            local f
            f=$(grep -c '^FAIL seed' "$log.out")
            if [ "$f" -gt 0 ]; then
                grep '^FAIL seed' "$log.out" | sed 's/^FAIL seed //' >> "$OUT/$t.failures"
                grep '^FAIL seed' "$log.out"
                fails=$((fails + f))
            fi
            seed=$((seed + batch)); done_seeds=$((done_seeds + batch))
            continue
        fi
        local bad
        bad=$(grep -o '^seed [0-9]*' "$log.err" | tail -1 | cut -d' ' -f2)
        if [ -z "$bad" ]; then
            echo "$t: process failed (status $st) before its first seed:"
            tail -20 "$log.err"
            exit 1
        fi
        local k=$((bad - seed + 1))
        done_seeds=$((done_seeds + k))
        if [ "$t" = mini ] && [ $st -eq 142 ]; then
            # SIGALRM: the program ran past its time limit (Mini has no step budget).
            timeouts=$((timeouts + 1))
        else
            local why="status $st"
            [ $st -eq 86 ] && why="ASan: $(grep -o 'AddressSanitizer: [a-z-]*' "$log.err" | head -1 | cut -d' ' -f2)"
            [ $st -eq 124 ] && why="hang"
            echo "$bad $why" >> "$OUT/$t.failures"
            grep -v -e '^seed ' -e '^ran$' "$log.err" > "$OUT/$t-seed-$bad.log"
            echo "FAIL $t seed $bad: $why (log: $OUT/$t-seed-$bad.log)"
            fails=$((fails + 1))
        fi
        seed=$((bad + 1))
    done
    local secs=$((SECONDS - start))
    echo "$t: $done_seeds seeds ($ran verified and ran) in ${secs}s, $fails failures${timeouts:+, $timeouts timeouts}"
    echo "$(date -u +%FT%TZ) $t ${SAN:-noasan} ${REL:-debug} first=$FIRST seeds=$done_seeds ran=$ran secs=$secs failures=$fails timeouts=$timeouts" >> "$OUT/history.log"
    TOTAL_FAILS=$((TOTAL_FAILS + fails))
}

TOTAL_FAILS=0
for t in "${TARGETS[@]}"; do fuzz "$t"; done
[ $TOTAL_FAILS -eq 0 ]
