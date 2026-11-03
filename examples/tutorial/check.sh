#!/bin/bash
# Build and test every step of the tutorial.
#
#   examples/tutorial/check.sh [-u] [stepN ...]
#
# For each step, main.lm is built with `-W error` (once per dispatch mode
# if the step declares a vm) and run on every tests/NAME.expected: with tests/NAME.pip
# as its argument if that file exists, plus the words in tests/NAME.args.
# If tests/NAME.main holds a name, NAME runs that program (NAME.lm in the
# step) instead of main.lm.
# stdout and stderr together must equal NAME.expected; tests named err_*
# must exit with a nonzero status, the others with 0. Steps whose main.lm
# understands `--gc-stress` run every test a second time with it.
# Programs run in their step's directory, so paths in messages are short.
#
# A step's NAME_test.lm files are unit tests, run with `loimi test` in
# each dispatch mode.
# -u rewrites the .expected files from the tailcall build's output.
# Run by `cargo test` (tests/golden.rs, tutorial_steps).

set -u
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
LOIMI=${LOIMI:-$ROOT/target/release/loimi}
export LOIMI_STD=${LOIMI_STD:-$ROOT}
UPDATE=0
if [ "${1:-}" = "-u" ]; then UPDATE=1; shift; fi
STEPS=("$@")
if [ ${#STEPS[@]} -eq 0 ]; then
    for d in "$ROOT"/examples/tutorial/step*/; do STEPS+=("$(basename "$d")"); done
fi
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT
FAIL=0
COUNT=0

for step in "${STEPS[@]}"; do
    dir=$ROOT/examples/tutorial/$step
    modes=(tailcall)
    if grep -qE '^(pub )?vm ' "$dir"/*.lm; then modes=(tailcall goto switch); fi
    stress=0
    if grep -q -- '--gc-stress' "$dir/main.lm"; then stress=1; fi
    progs=(main)
    for f in "$dir"/tests/*.main; do [ -f "$f" ] && progs+=("$(cat "$f")"); done
    for mode in "${modes[@]}"; do
        built=1
        for prog in "${progs[@]}"; do
            if ! (ulimit -v 2500000; "$LOIMI" build -W error "$dir/$prog.lm" --dispatch="$mode" -o "$TMP/$step-$mode-$prog") > "$TMP/build.log" 2>&1; then
                echo "FAIL $step ($mode): building $prog.lm failed"
                cat "$TMP/build.log"
                FAIL=1
                built=0
            fi
        done
        [ $built = 1 ] || continue
        # Unit tests (std.test): NAME_test.lm, run with `loimi test`.
        for t in "$dir"/*_test.lm; do
            [ -f "$t" ] || continue
            COUNT=$((COUNT + 1))
            if ! (cd "$dir" && ulimit -v 2500000 && "$LOIMI" test -W error --dispatch="$mode" "$(basename "$t")") > "$TMP/unit.log" 2>&1; then
                echo "FAIL $step/$(basename "$t") ($mode)"
                cat "$TMP/unit.log"
                FAIL=1
            fi
        done
        for exp in "$dir"/tests/*.expected; do
            name=$(basename "$exp" .expected)
            prog=main
            [ -f "$dir/tests/$name.main" ] && prog=$(cat "$dir/tests/$name.main")
            exe=$TMP/$step-$mode-$prog
            args=()
            [ -f "$dir/tests/$name.pip" ] && args+=("tests/$name.pip")
            [ -f "$dir/tests/$name.args" ] && read -r -a extra < "$dir/tests/$name.args" && args+=("${extra[@]}")
            runs=(normal)
            [ $stress = 1 ] && runs+=(stress)
            for run in "${runs[@]}"; do
                a=("${args[@]}")
                [ "$run" = stress ] && a+=(--gc-stress)
                out=$(cd "$dir" && ulimit -v 2500000 && ${LOIMI_RUNNER:-} "$exe" "${a[@]}" 2>&1)
                status=$?
                COUNT=$((COUNT + 1))
                label="$step/$name ($mode${run/normal/})"
                if [ $UPDATE = 1 ] && [ "$mode" = tailcall ] && [ "$run" = normal ]; then
                    printf '%s\n' "$out" > "$exp"
                fi
                if [ "$out" != "$(cat "$exp")" ]; then
                    echo "FAIL $label: output differs"
                    diff <(printf '%s\n' "$out") "$exp" | head -20
                    FAIL=1
                elif [[ $name == err_* ]] && [ $status = 0 ]; then
                    echo "FAIL $label: exit status 0, expected an error"
                    FAIL=1
                elif [[ $name != err_* ]] && [ $status != 0 ]; then
                    echo "FAIL $label: exit status $status"
                    FAIL=1
                fi
            done
        done
    done
done

if [ $FAIL = 0 ]; then echo "ok: $COUNT runs"; fi
exit $FAIL
