#!/bin/sh
# Build the SIMD variants of three Benchmarks Game programs:
#
#   bench/wasm/programs-out/<name>_simd.wasm      the module
#   bench/wasm/programs-out/<name>_simd.expected  a copy of <name>.expected
#
# The same sources and problem sizes as programs.sh, compiled with
# `-mcpu=mvp -msimd128` (clang 19 auto-vectorizes: nbody gets ~50 SIMD
# instructions, fannkuchredux 12-13, spectralnorm 2; the other CLBG programs
# get none, so they are not built). The sysroot stays the MVP one. wasm3
# 0.5 has no SIMD, so their expected output is the MVP build's (the SIMD
# code does the same IEEE operations). Needs programs.sh first (the
# unpacked sources and the MVP .expected files).
set -eu
here=$(cd "$(dirname "$0")" && pwd)
src="$here/src/benchmarksgame-programs"
out="$here/programs-out"
CC=${CC:-clang-19}
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

CFLAGS="--target=wasm32-wasi --sysroot=$here/install/wasi-sysroot-mvp \
  -resource-dir $here/install/clang-resource -O2 -mcpu=mvp -msimd128 -w"
LDFLAGS="-Wl,--strip-debug -lm"

while read -r name file arg; do
  # shellcheck disable=SC2086
  "$CC" $CFLAGS -Dmain=bench_main -c -x c "$src/$file" -o "$tmp/$name.o"
  # shellcheck disable=SC2086
  "$CC" $CFLAGS -DBENCH_ARG="$arg" "$here/ports/argv_main.c" \
    "$tmp/$name.o" $LDFLAGS -o "$out/${name}_simd.wasm"
  cp "$out/$name.expected" "$out/${name}_simd.expected"
  n=$("$here/install/bin/wasm2wat" --enable-all "$out/${name}_simd.wasm" |
    grep -cE '(v128|[if](8x16|16x8|32x4|64x2))\.' || true)
  echo "${name}_simd: $n SIMD instructions"
done <<EOF
nbody          nbody/nbody.gcc                            2500000
fannkuchredux  fannkuchredux/fannkuchredux.gcc            10
spectralnorm   spectralnorm/spectralnorm.gcc-8.gcc        2000
EOF
