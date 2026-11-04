#!/bin/sh
# A second benchmark set, held out from the tuning of examples/wasm (whose
# changes were measured on programs.sh's seven programs): PolyBench/C
# 4.2.1, the 30 kernels, pinned in MANIFEST. Built like programs.sh
# (clang-19 -O2 -mcpu=mvp, the MVP wasi-libc from install.sh):
#
#   bench/wasm/polybench-out/<name>.wasm        the timed build (no output)
#   bench/wasm/polybench-out/check/<name>.wasm  SMALL_DATASET with
#                                               POLYBENCH_DUMP_ARRAYS (its
#                                               arrays on stderr)
#
#   bench/wasm/polybench.sh                     build (needs install.sh)
#   bench/wasm/polybench.sh --check WASM_CMD    run every check build under
#                                               wasm3 and WASM_CMD, compare stderr
#
# Time them with bench.py --dir bench/wasm/polybench-out. The dataset
# per kernel (below) is the largest that runs in at most about 2 s under
# wasm3 (0.05-2 s: a few kernels have no size in between).
set -eu
here=$(cd "$(dirname "$0")" && pwd)
src="$here/src"
out="$here/polybench-out"
bin="$here/install/bin"
CC=${CC:-clang-19}

# name            directory (in the tarball)        dataset
KERNELS="
correlation      datamining/correlation             MEDIUM
covariance       datamining/covariance              MEDIUM
2mm              linear-algebra/kernels/2mm         MEDIUM
3mm              linear-algebra/kernels/3mm         MEDIUM
atax             linear-algebra/kernels/atax        EXTRALARGE
bicg             linear-algebra/kernels/bicg        EXTRALARGE
doitgen          linear-algebra/kernels/doitgen     MEDIUM
mvt              linear-algebra/kernels/mvt         EXTRALARGE
gemm             linear-algebra/blas/gemm           MEDIUM
gemver           linear-algebra/blas/gemver         EXTRALARGE
gesummv          linear-algebra/blas/gesummv        EXTRALARGE
symm             linear-algebra/blas/symm           MEDIUM
syr2k            linear-algebra/blas/syr2k          MEDIUM
syrk             linear-algebra/blas/syrk           MEDIUM
trmm             linear-algebra/blas/trmm           MEDIUM
cholesky         linear-algebra/solvers/cholesky    MEDIUM
durbin           linear-algebra/solvers/durbin      EXTRALARGE
gramschmidt      linear-algebra/solvers/gramschmidt MEDIUM
lu               linear-algebra/solvers/lu          MEDIUM
ludcmp           linear-algebra/solvers/ludcmp      MEDIUM
trisolv          linear-algebra/solvers/trisolv     EXTRALARGE
deriche          medley/deriche                     LARGE
floyd-warshall   medley/floyd-warshall              MEDIUM
nussinov         medley/nussinov                    MEDIUM
adi              stencils/adi                       MEDIUM
fdtd-2d          stencils/fdtd-2d                   MEDIUM
heat-3d          stencils/heat-3d                   MEDIUM
jacobi-1d        stencils/jacobi-1d                 EXTRALARGE
jacobi-2d        stencils/jacobi-2d                 MEDIUM
seidel-2d        stencils/seidel-2d                 MEDIUM
"

if [ "${1:-}" = "--check" ]; then
  cmd=$2
  fail=0
  tmp=$(mktemp -d)
  trap 'rm -rf "$tmp"' EXIT
  echo "$KERNELS" | while read -r name dir ds; do
    [ -n "$name" ] || continue
    w="$out/check/$name.wasm"
    "$bin/wasm3" "$w" 2> "$tmp/ref" > /dev/null
    # shellcheck disable=SC2086
    $cmd "$w" 2> "$tmp/out" > /dev/null || true
    if cmp -s "$tmp/ref" "$tmp/out"; then echo "ok   $name"; else echo "FAIL $name"; fi
  done
  exit 0
fi

mkdir -p "$src" "$out/check"
field() { awk -v n="$1" -v f="$2" '$1 == n { print $f }' "$here/MANIFEST"; }
v=$(field polybench 2)
url=$(field polybench 3); sum=$(field polybench 4)
tarball="$src/polybench-c-$v.tar.gz"
if [ ! -f "$tarball" ]; then
  curl -fsSL -o "$tarball.part" "$url"
  mv "$tarball.part" "$tarball"
fi
echo "$sum  $tarball" | sha256sum -c -
pb="$src/polybench-c-$v"
rm -rf "$pb"
tar -C "$src" -xzf "$tarball"

CFLAGS="--target=wasm32-wasi --sysroot=$here/install/wasi-sysroot-mvp \
  -resource-dir $here/install/clang-resource -O2 -mcpu=mvp -w \
  -I$pb/utilities -D_WASI_EMULATED_PROCESS_CLOCKS"
LDFLAGS="-Wl,--strip-debug -lm -lwasi-emulated-process-clocks"

tmp="$src/polybench-obj"
mkdir -p "$tmp"
# build OUT DEFINES..: the kernel with main renamed, then the argv shim
build() {
  o=$1; shift
  # shellcheck disable=SC2086
  "$CC" $CFLAGS "$@" -Dmain=bench_main -I"$pb/$dir" -c "$pb/$dir/$name.c" -o "$tmp/k.o"
  # shellcheck disable=SC2086
  "$CC" $CFLAGS "$@" -c "$pb/utilities/polybench.c" -o "$tmp/p.o"
  # shellcheck disable=SC2086
  "$CC" $CFLAGS -DBENCH_ARG=0 "$here/ports/argv_main.c" "$tmp/k.o" "$tmp/p.o" \
    $LDFLAGS -o "$o"
}
echo "$KERNELS" | while read -r name dir ds; do
  [ -n "$name" ] || continue
  build "$out/$name.wasm" -D${ds}_DATASET
  : > "$out/$name.expected"
  build "$out/check/$name.wasm" -DSMALL_DATASET -DPOLYBENCH_DUMP_ARRAYS
done
ls "$out"/*.wasm | wc -l
