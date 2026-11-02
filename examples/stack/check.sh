#!/bin/bash
# Build Stak in every dispatch mode and run tests/*.stk against
# tests/*.expected (stdout and stderr together), normally and with
# --gc-stress. Tests named err_* must exit nonzero, the others with 0.
#
#   examples/stack/check.sh [-u]     (-u rewrites the .expected files)

set -u
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
DIR=$ROOT/examples/stack
LOIMI=${LOIMI:-$ROOT/target/release/loimi}
UPDATE=0
if [ "${1:-}" = "-u" ]; then UPDATE=1; fi
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT
FAIL=0
COUNT=0
cd "$DIR" || exit 1
for mode in tailcall goto switch; do
    if ! (ulimit -v 2500000; "$LOIMI" build -W error main.lm --dispatch="$mode" -o "$TMP/stak-$mode") > "$TMP/build.log" 2>&1; then
        echo "FAIL: building ($mode)"; cat "$TMP/build.log"; FAIL=1; continue
    fi
    for src in tests/*.stk; do
        name=$(basename "$src" .stk)
        exp=tests/$name.expected
        for stress in "" --gc-stress; do
            COUNT=$((COUNT + 1))
            "$TMP/stak-$mode" "$src" $stress > "$TMP/out" 2>&1
            status=$?
            if [ $UPDATE = 1 ] && [ $mode = tailcall ] && [ -z "$stress" ]; then cp "$TMP/out" "$exp"; fi
            case $name in
                err_*) [ $status -ne 0 ] || { echo "FAIL $name ($mode $stress): exit status 0"; FAIL=1; } ;;
                *) [ $status -eq 0 ] || { echo "FAIL $name ($mode $stress): exit status $status"; FAIL=1; } ;;
            esac
            if ! cmp -s "$TMP/out" "$exp"; then
                echo "FAIL $name ($mode $stress)"; diff "$exp" "$TMP/out" | head -10; FAIL=1
            fi
        done
    done
done
[ $FAIL = 0 ] && echo "ok: $COUNT runs"
exit $FAIL
