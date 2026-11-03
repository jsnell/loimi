#!/bin/sh
# Build the benchmark programs (existing public C programs, pinned in
# MANIFEST) to MVP-only wasm32-wasi modules:
#
#   bench/wasm/programs-out/<name>.wasm       the module (no argv needed)
#   bench/wasm/programs-out/<name>.expected   its stdout under wasm3
#   bench/wasm/programs-out/REPORT            size, imports, MVP check
#
# Sources: CoreMark (eembc, with ports/coremark-wasi.patch for the timer)
# and C programs from the Computer Language Benchmarks Game; problem sizes
# are fixed at compile time (ports/argv_main.c), see the table below.
# Compiled with clang-19 -O2 -mcpu=mvp against the MVP wasi-libc built by
# install.sh. Needs install.sh first.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
src="$here/src"
out="$here/programs-out"
bin="$here/install/bin"
CC=${CC:-clang-19}
mkdir -p "$src" "$out"

field() { awk -v n="$1" -v f="$2" '$1 == n { print $f }' "$here/MANIFEST"; }
fetch() {
  url=$(field "$1" 3); sum=$(field "$1" 4)
  if [ ! -f "$src/$2" ]; then
    curl -fsSL -o "$src/$2.part" "$url"
    mv "$src/$2.part" "$src/$2"
  fi
  echo "$sum  $src/$2" | sha256sum -c -
}

CFLAGS="--target=wasm32-wasi --sysroot=$here/install/wasi-sysroot-mvp \
  -resource-dir $here/install/clang-resource -O2 -mcpu=mvp -w"
LDFLAGS="-Wl,--strip-debug -lm"

# ── CoreMark ─────────────────────────────────────────────────────────
c=$(field coremark 2)
fetch coremark "coremark-$c.tar.gz"
cm="$src/coremark-$c"
rm -rf "$cm"
tar -C "$src" -xzf "$src/coremark-$c.tar.gz"
patch -s -d "$cm" -p1 < "$here/ports/coremark-wasi.patch"
COREMARK_ITERATIONS=${COREMARK_ITERATIONS:-5000}
# shellcheck disable=SC2086
"$CC" $CFLAGS -I"$cm" -I"$cm/simple" \
  -DFLAGS_STR='"-O2 -mcpu=mvp"' -DITERATIONS="$COREMARK_ITERATIONS" \
  -DMAIN_HAS_NOARGC=1 -DPERFORMANCE_RUN=1 \
  "$cm"/core_list_join.c "$cm"/core_main.c "$cm"/core_matrix.c \
  "$cm"/core_state.c "$cm"/core_util.c "$cm"/simple/core_portme.c \
  $LDFLAGS -o "$out/coremark.wasm"

# ── Benchmarks Game ──────────────────────────────────────────────────
c=$(field benchmarksgame 2)
fetch benchmarksgame "benchmarksgame-$c.tar.gz"
bg="$src/benchmarksgame-$c"
rm -rf "$bg" "$src/benchmarksgame-programs"
tar -C "$src" -xzf "$src/benchmarksgame-$c.tar.gz" \
  "benchmarksgame-$c/public/download/benchmarksgame-sourcecode.zip"
mkdir -p "$src/benchmarksgame-programs"
unzip -q -o "$bg/public/download/benchmarksgame-sourcecode.zip" -d "$src/benchmarksgame-programs"

# name           source file (in the zip)                 argv[1]
while read -r name file arg; do
  # shellcheck disable=SC2086
  "$CC" $CFLAGS -Dmain=bench_main -c -x c "$src/benchmarksgame-programs/$file" \
    -o "$src/benchmarksgame-programs/$name.o"
  # shellcheck disable=SC2086
  "$CC" $CFLAGS -DBENCH_ARG="$arg" "$here/ports/argv_main.c" \
    "$src/benchmarksgame-programs/$name.o" $LDFLAGS -o "$out/$name.wasm"
done <<EOF
nbody          nbody/nbody.gcc                            2500000
fannkuchredux  fannkuchredux/fannkuchredux.gcc            10
spectralnorm   spectralnorm/spectralnorm.gcc-8.gcc        2000
binarytrees    binarytrees/binarytrees.gcc                15
mandelbrot     mandelbrot/mandelbrot.gcc-8.gcc            2400
fasta          fasta/fasta.gcc                            2500000
EOF

# ── report: size, imports, MVP check, expected output ────────────────
mvp="--disable-mutable-globals --disable-saturating-float-to-int \
  --disable-sign-extension --disable-simd --disable-multi-value \
  --disable-bulk-memory --disable-reference-types"
{
  for w in "$out"/*.wasm; do
    name=$(basename "$w" .wasm)
    echo "== $name  $(wc -c < "$w") bytes"
    # imports as "module.name (params) -> results"
    "$bin/wasm-objdump" -x -j Type "$w" | grep -- ' - type\[' > "$out/.types"
    "$bin/wasm-objdump" -x -j Import "$w" | grep -- ' - func' |
      awk 'NR == FNR { t = $2; sub(/^type\[/, "", t); sub(/\]$/, "", t);
                       s = $0; sub(/^ *- type\[[0-9]+\] /, "", s); sig[t] = s; next }
           { n = $3; sub(/^sig=/, "", n); print "  " $NF " " sig[n] }' "$out/.types" -
    rm -f "$out/.types"
    # shellcheck disable=SC2086
    if "$bin/wasm2wat" $mvp "$w" -o /dev/null 2>/dev/null; then
      echo "  mvp: ok"
    else
      echo "  mvp: NOT MVP"
      "$bin/wasm2wat" --enable-all "$w" | grep -oE \
        '(memory\.(copy|fill|init)|data\.drop|i(32|64)\.extend(8|16|32)_s|i(32|64)\.trunc_sat_f(32|64)_[su]|ref\.[a-z_]+|table\.[a-z]+|v128\.[a-z_0-9]+)' \
        | sort | uniq -c | sed 's/^/   /'
    fi
    "$bin/wasm3" "$w" > "$out/$name.expected"
  done
} > "$out/REPORT"
cat "$out/REPORT"
