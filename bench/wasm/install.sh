#!/bin/sh
# Fetch and build the external WebAssembly tooling listed in MANIFEST.
# Every download is checked against the sha256 pinned in MANIFEST.
#
# Result (all under bench/wasm/install/, gitignored):
#   bin/wast2json bin/wasm-interp bin/wasm-objdump bin/wasm2wat bin/wat2wasm
#   bin/spectest-interp            wabt 1.0.36 (wasm-interp built with --wasi)
#   bin/wasm3                      wasm3 0.5.0, -O3, built-in WASI
#   wasi-sysroot-mvp/              wasi-libc (wasi-sdk-25 tag) built -mcpu=mvp
#   clang-resource/                resource dir for clang-19 that adds the
#                                  wasm32 compiler-rt builtins (wasi-sdk 25)
#   cmake/                         prebuilt cmake (only used to build wabt)
#
# Then run spec.sh (testsuite -> json) and programs.sh (benchmarks -> wasm).
# Env: CC/CXX (default clang-19/clang++-19), JOBS (default 2).
set -eu
here=$(cd "$(dirname "$0")" && pwd)
src="$here/src"
dst="$here/install"
jobs=${JOBS:-2}
CC=${CC:-clang-19}
CXX=${CXX:-clang++-19}
mkdir -p "$src" "$dst/bin"

field() { awk -v n="$1" -v f="$2" '$1 == n { print $f }' "$here/MANIFEST"; }

# fetch NAME FILE: download NAME's url to src/FILE (once) and verify it.
fetch() {
  url=$(field "$1" 3); sum=$(field "$1" 4)
  [ -n "$sum" ] || { echo "no MANIFEST entry for $1" >&2; exit 1; }
  if [ ! -f "$src/$2" ]; then
    curl -fsSL -o "$src/$2.part" "$url"
    mv "$src/$2.part" "$src/$2"
  fi
  echo "$sum  $src/$2" | sha256sum -c -
}

# ── cmake (prebuilt; the machine has none) ───────────────────────────
v=$(field cmake 2)
fetch cmake "cmake-$v-linux-x86_64.tar.gz"
if [ ! -x "$dst/cmake/bin/cmake" ]; then
  rm -rf "$dst/cmake" "$src/cmake-$v-linux-x86_64"
  tar -C "$src" -xzf "$src/cmake-$v-linux-x86_64.tar.gz"
  mv "$src/cmake-$v-linux-x86_64" "$dst/cmake"
fi
cmake="$dst/cmake/bin/cmake"

# ── wabt (+ libuv for wasm-interp --wasi via uvwasi) ─────────────────
v=$(field wabt 2)
fetch wabt "wabt-$v.tar.xz"
uv=$(field libuv 2)
fetch libuv "libuv-$uv.tar.gz"
if [ ! -x "$dst/bin/wasm-interp" ]; then
  rm -rf "$src/wabt-$v" "$src/libuv-$uv" "$src/wabt-build"
  tar -C "$src" -xJf "$src/wabt-$v.tar.xz"
  tar -C "$src" -xzf "$src/libuv-$uv.tar.gz"
  # uvwasi would git-clone libuv at configure time; point it at the
  # verified tarball instead and forbid any other download.
  "$cmake" -S "$src/wabt-$v" -B "$src/wabt-build" \
    -DCMAKE_BUILD_TYPE=Release \
    -DCMAKE_C_COMPILER="$CC" -DCMAKE_CXX_COMPILER="$CXX" \
    -DBUILD_TESTS=OFF -DBUILD_LIBWASM=OFF -DWITH_WASI=ON \
    -DUSE_INTERNAL_SHA256=ON \
    -DFETCHCONTENT_FULLY_DISCONNECTED=ON \
    -DFETCHCONTENT_SOURCE_DIR_LIBUV="$src/libuv-$uv" >/dev/null
  "$cmake" --build "$src/wabt-build" -j"$jobs" --target \
    wast2json wasm-interp wasm-objdump wasm2wat wat2wasm spectest-interp
  for t in wast2json wasm-interp wasm-objdump wasm2wat wat2wasm spectest-interp; do
    cp "$src/wabt-build/$t" "$dst/bin/$t"
  done
fi

# ── wasm3 (plain compile, -O3, simple built-in WASI) ─────────────────
v=$(field wasm3 2)
fetch wasm3 "wasm3-$v.tar.gz"
if [ ! -x "$dst/bin/wasm3" ]; then
  rm -rf "$src/wasm3-$v"
  tar -C "$src" -xzf "$src/wasm3-$v.tar.gz"
  w="$src/wasm3-$v"
  "$CC" -O3 -fomit-frame-pointer -fno-stack-check -fno-stack-protector \
    -w -Dd_m3HasWASI -I"$w/source" "$w"/source/*.c "$w/platforms/app/main.c" \
    -lm -o "$dst/bin/wasm3"
fi

# ── wasi sysroot + compiler-rt builtins for clang-19 ─────────────────
# wasi-libc is built from source with -mcpu=mvp: the prebuilt wasi-sdk
# sysroot is compiled for clang's "generic" CPU (memory.copy/fill in memcpy/
# memset, and call_indirect with a padded 5-byte table index from
# reference-types, which MVP decoders such as wasm3 0.5.0 reject).
# BULK_MEMORY_SOURCES= drops the -mbulk-memory that the Makefile forces on
# memcpy/memmove/memset; check-symbols is skipped (-o) because its list of
# expected predefined macros assumes -mcpu=generic.
v=$(field wasi-libc 2)
fetch wasi-libc "wasi-libc-$v.tar.gz"
if [ ! -f "$dst/wasi-sysroot-mvp/lib/wasm32-wasi/libc.a" ]; then
  rm -rf "$dst/wasi-sysroot-mvp" "$src/wasi-libc-$v"
  tar -C "$src" -xzf "$src/wasi-libc-$v.tar.gz"
  make -C "$src/wasi-libc-$v" -j"$jobs" CC="$CC" \
    AR="${AR:-llvm-ar-19}" NM="${NM:-llvm-nm-19}" \
    EXTRA_CFLAGS="-O2 -DNDEBUG -mcpu=mvp" BULK_MEMORY_SOURCES= \
    SYSROOT="$dst/wasi-sysroot-mvp" -o check-symbols finish >/dev/null
fi
v=$(field wasi-builtins 2)
fetch wasi-builtins "libclang_rt.builtins-wasm32-wasi-$v.tar.gz"
rd="$dst/clang-resource"
if [ ! -f "$rd/lib/wasi/libclang_rt.builtins-wasm32.a" ]; then
  rm -rf "$rd" "$src/libclang_rt.builtins-wasm32-wasi-$v"
  tar -C "$src" -xzf "$src/libclang_rt.builtins-wasm32-wasi-$v.tar.gz"
  mkdir -p "$rd/lib/wasi"
  ln -s "$("$CC" -print-resource-dir)/include" "$rd/include"
  mv "$src/libclang_rt.builtins-wasm32-wasi-$v/libclang_rt.builtins-wasm32.a" "$rd/lib/wasi/"
fi

"$dst/bin/wast2json" --version
"$dst/bin/wasm3" --version | head -1
echo "ok: $dst"
