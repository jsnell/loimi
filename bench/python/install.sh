#!/bin/sh
# Build the reference Python interpreters listed in MANIFEST from source and
# fetch the pyperformance sources that programs/*.py were derived from.
# Result: bench/python/install/bin/{python3,micropython}
#
# CPython: plain `./configure --prefix=...`, i.e. CPython's default release
# flags (OPT = -DNDEBUG -g -O3 -Wall; computed gotos on with GCC).
# Deliberately NO --enable-optimizations (PGO) and NO --with-lto: a plain,
# non-PGO build. JIT and free-threading are off (3.13 defaults). Installed
# with `make install` into install/cpython so it finds its stdlib;
# install/bin/python3 is a symlink.
# MicroPython: the stock unix port, default "standard" variant, built with
# the port's own makefile flags (-Os). GIT_CEILING_DIRECTORIES stops its
# version script from picking up the enclosing loimi git repo.
#
# The build machine is shared: parallelism defaults to -j2 (JOBS=n overrides).
# Build times on the dev machine (GCC 14.2, -j2): CPython configure ~20 s,
# make ~1 m 55 s, make install ~5 s; MicroPython (mpy-cross + unix) ~40 s;
# whole script ~3 min.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
src="$here/src"
dst="$here/install"
jobs=${JOBS:-2}
mkdir -p "$src" "$dst/bin"

field() { awk -v n="$1" -v f="$2" '$1 == n { print $f }' "$here/MANIFEST"; }

# fetch NAME FILE: download (if needed) and verify the archive for NAME.
fetch() {
  url=$(field "$1" 3); sum=$(field "$1" 4)
  [ -f "$src/$2" ] || curl -fsSL -o "$src/$2" "$url"
  echo "$sum  $src/$2" | sha256sum -c -
}

# ── CPython ──────────────────────────────────────────────────────────
v=$(field cpython 2)
fetch cpython "Python-$v.tar.xz"
rm -rf "$src/cpython"; mkdir "$src/cpython"
tar -C "$src/cpython" -xJf "$src/Python-$v.tar.xz"
(cd "$src/cpython/Python-$v" &&
  ./configure --prefix="$dst/cpython" >/dev/null &&
  make -j"$jobs" >/dev/null &&
  make -j"$jobs" install >/dev/null)
ln -sf "../cpython/bin/python3" "$dst/bin/python3"

# ── MicroPython ──────────────────────────────────────────────────────
v=$(field micropython 2)
fetch micropython "micropython-$v.tar.xz"
rm -rf "$src/micropython"; mkdir "$src/micropython"
tar -C "$src/micropython" -xJf "$src/micropython-$v.tar.xz"
m="$src/micropython/micropython-$v"
GIT_CEILING_DIRECTORIES="$src/micropython"; export GIT_CEILING_DIRECTORIES
make -C "$m/mpy-cross" -j"$jobs" >/dev/null
make -C "$m/ports/unix" -j"$jobs" >/dev/null
cp "$m/ports/unix/build-standard/micropython" "$dst/bin/micropython"

# ── pyperformance (benchmark sources only; not installed or run) ─────
v=$(field pyperformance 2)
fetch pyperformance "pyperformance-$v.tar.gz"
rm -rf "$src/pyperformance"; mkdir "$src/pyperformance"
tar -C "$src/pyperformance" -xzf "$src/pyperformance-$v.tar.gz"

"$dst/bin/python3" -V
"$dst/bin/micropython" -c 'import sys; print("MicroPython", sys.version)'
