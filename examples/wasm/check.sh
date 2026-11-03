#!/bin/bash
# Build the wasm interpreter and test it.
#
#   examples/wasm/check.sh [--fast] [-u]
#
# - builds with -W error (every dispatch mode; --fast: tail calls only)
# - compiles tests/*.c to wasm32 with clang-19 (skipped without wasm-ld)
#   and compares each program's output with tests/NAME.expected
#   (-u rewrites them)
# - if the official spec tests are installed (bench/wasm/install.sh,
#   bench/wasm/spec.sh), runs the MVP .wast files and requires every
#   command to pass (--fast: a subset)
set -u
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
DIR=$ROOT/examples/wasm
LOIMI=${LOIMI:-$ROOT/target/release/loimi}
FAST=0
UPDATE=0
for a in "$@"; do
    case $a in
        --fast) FAST=1 ;;
        -u) UPDATE=1 ;;
    esac
done
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT
FAIL=0
modes="tailcall goto switch"
if [ $FAST = 1 ]; then modes=tailcall; fi

# The MVP files of the (Wasm 2.0-era) testsuite, plus those of the
# extensions the interpreter implements (sign extension, saturating
# truncation, multi-value, memory.copy/fill).
SPEC_MVP="address align binary-leb128 block br br_if br_table call comments const conversions custom
data endianness exports f32 f32_bitwise f32_cmp f64 f64_bitwise f64_cmp fac float_exprs float_literals
float_memory float_misc forward func func_ptrs i32 i64 if imports inline-module int_exprs int_literals
labels left-to-right linking load local_get local_set local_tee loop memory memory_copy memory_fill
memory_grow memory_redundancy memory_size memory_trap names nop return skip-stack-guard-page stack start
store switch table-sub token traps type unreachable unreached-invalid unwind utf8-custom-section-id
utf8-import-field utf8-import-module utf8-invalid-encoding"
SPEC_FAST="address block br_table call conversions f64 i32 i64 if loop memory_trap traps unreached-invalid"

for mode in $modes; do
    if ! (ulimit -v 2500000; cd "$DIR" && "$LOIMI" build -W error main.lm --release --dispatch="$mode" -o "$TMP/wasm-$mode") > "$TMP/build.log" 2>&1; then
        echo "FAIL: building ($mode)"; cat "$TMP/build.log"; FAIL=1; continue
    fi
    if command -v clang-19 > /dev/null && command -v wasm-ld > /dev/null; then
        for src in "$DIR"/tests/*.c; do
            name=$(basename "$src" .c)
            clang-19 --target=wasm32 -nostdlib -O2 -Wl,--no-entry -Wl,--allow-undefined "$src" -o "$TMP/$name.wasm" || { echo "FAIL: compiling $name"; FAIL=1; continue; }
            (ulimit -v 2500000; "$TMP/wasm-$mode" "$TMP/$name.wasm") > "$TMP/out" 2>&1
            if [ $UPDATE = 1 ] && [ $mode = tailcall ]; then cp "$TMP/out" "$DIR/tests/$name.expected"; fi
            if ! cmp -s "$TMP/out" "$DIR/tests/$name.expected"; then
                echo "FAIL: $name ($mode)"; diff "$TMP/out" "$DIR/tests/$name.expected" | head -10; FAIL=1
            fi
        done
    else
        echo "skip: tests/*.c (no clang-19/wasm-ld)"
    fi
    if [ -d "$ROOT/bench/wasm/spec-json" ]; then
        files=$SPEC_MVP
        if [ $FAST = 1 ]; then files=$SPEC_FAST; fi
        python3 "$ROOT/bench/wasm/spec_run.py" --wasm "$TMP/wasm-$mode" $files > "$TMP/spec.txt"
        tail -1 "$TMP/spec.txt" | sed "s/^/spec ($mode): /"
        if ! tail -1 "$TMP/spec.txt" | grep -q " 0 failed"; then
            grep -v " 0 failed" "$TMP/spec.txt"; FAIL=1
        fi
    else
        echo "skip: spec tests (run bench/wasm/install.sh)"
    fi
done
if [ $FAIL = 0 ]; then echo "ok"; fi
exit $FAIL
