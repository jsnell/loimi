#!/bin/sh
# Build the reference interpreters listed in MANIFEST from source.
# Usage: install.sh [chibi] [gauche] [guile]      (default: all three)
#
# Result, all gitignored: sources and logs in bench/scheme/src/, binaries
# and libraries in bench/scheme/install/:
#   bin/chibi-scheme  chibi-scheme, stock makefile (-O3 by default;
#                     computed-goto dispatch is on by default)
#   bin/gosh          Gauche and SLIB, release tarballs, stock configure (-O2) minus
#                     optional extensions the benchmarks don't use
#   bin/guile         Guile 3 with its JIT, release tarball (-O2 -flto), on bdwgc and
#                     libunistring built here (GMP and libffi from the system)
# Tarballs are verified against the sha256 in MANIFEST before extraction.
# Builds use make -j${JOBS:-2}.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
src="$here/src"
dst="$here/install"
jobs="${JOBS:-2}"
mkdir -p "$src/dl" "$dst"
field() { awk -v n="$1" -v f="$2" '$1 == n { print $f }' "$here/MANIFEST"; }

# Download NAME's tarball, check its sha256, extract it afresh into
# $src; prints the extracted directory.
tarball() {
    url=$(field "$1" 2); sum=$(field "$1" 3)
    file="$src/dl/${url##*/}"
    [ -f "$file" ] || curl -fsSL -o "$file" "$url"
    echo "$sum  $file" | sha256sum -c --quiet - >&2
    dir=$(tar tf "$file" | head -1); dir="$src/${dir%%/*}"
    rm -rf "$dir"
    tar xf "$file" -C "$src"
    echo "$dir"
}

# configure && make && make install in DIR, logging to $src/NAME-*.log.
build() {
    name=$1; dir=$2; shift 2
    echo "building $name (logs: $src/$name-*.log)"
    (cd "$dir" && ./configure --prefix="$dst" "$@") >"$src/$name-configure.log" 2>&1
    make -C "$dir" -j"$jobs" >"$src/$name-make.log" 2>&1
    make -C "$dir" install >"$src/$name-install.log" 2>&1
}

chibi() {
    url=$(field chibi-scheme 2); commit=$(field chibi-scheme 3)
    [ -d "$src/chibi-scheme/.git" ] || git clone -q "$url" "$src/chibi-scheme"
    git -C "$src/chibi-scheme" fetch -q origin
    git -C "$src/chibi-scheme" checkout -q "$commit"
    git -C "$src/chibi-scheme" clean -qfdx
    make -C "$src/chibi-scheme" -j"$jobs" PREFIX="$dst" >/dev/null
    make -C "$src/chibi-scheme" PREFIX="$dst" install >/dev/null
    LD_LIBRARY_PATH="$dst/lib" "$dst/bin/chibi-scheme" -V
}

gauche() {
    # The suite's Gauche-postlude.scm does (import (slib)), which needs
    # Aubrey Jaffer's SLIB (portable Scheme source, nothing to build).
    slib=$(tarball slib)
    rm -rf "$dst/share/slib"
    mkdir -p "$dst/share"
    cp -R "$slib" "$dst/share/slib"
    # Gauche bundles its own Boehm GC (using the compiler's atomics, so no
    # libatomic_ops). TLS, dbm, zlib and iconv are optional extension
    # modules the benchmarks don't load.
    build gauche "$(tarball gauche)" --with-tls=no --with-dbm=no \
        --without-zlib --without-iconv --with-slib="$dst/share/slib"
    "$dst/bin/gosh" -V | head -1
}

guile() {
    # bdwgc with GCC atomic intrinsics instead of libatomic_ops.
    build bdwgc "$(tarball bdwgc)" --disable-static --disable-docs \
        --with-libatomic-ops=none --enable-threads=posix
    build libunistring "$(tarball libunistring)" --disable-static
    build guile "$(tarball guile)" --disable-static --disable-nls \
        --with-libunistring-prefix="$dst" \
        PKG_CONFIG_PATH="$dst/lib/pkgconfig" \
        CPPFLAGS="-I$dst/include" LDFLAGS="-L$dst/lib -Wl,-rpath,$dst/lib"
    "$dst/bin/guile" --version | head -1
}

[ $# -gt 0 ] || set -- chibi gauche guile
for impl in "$@"; do "$impl"; done
