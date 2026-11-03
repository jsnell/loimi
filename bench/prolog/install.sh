#!/bin/sh
# Build the reference Prolog systems listed in MANIFEST from source and
# generate the benchmark programs.
# Usage: install.sh [trealla] [swipl] [programs]      (default: all three)
#
# Result, all gitignored: tarballs in bench/prolog/src/dl/, sources and logs
# in bench/prolog/src/, and
#   install/trealla/bin/tpl   Trealla Prolog, stock GNUmakefile (-O3), with
#                             ISOCLINE=1 (bundled line editor instead of
#                             libedit, whose headers are missing here),
#                             NOFFI=1 and NOSSL=1 (optional, unused by the
#                             benchmarks).
#   install/swipl/bin/swipl   SWI-Prolog, CMake Release build of the core
#                             system only: -DSWIPL_PACKAGES=OFF (no
#                             packages), no docs, no tests; GMP from the
#                             system (unbounded integers, needed by perfect).
#                             CMake is Kitware's pinned binary release,
#                             unpacked into src/ (the host has no cmake and
#                             no python3-venv for pip's cmake). Its
#                             RUNPATHs are made $ORIGIN-relative by
#                             set_runpath.py, so the install can be moved.
#   programs/*.pl             the SWI-Prolog bench programs, made standalone
#                             by prepare.py (edits listed in README.md).
# Tarballs are verified against the sha256 in MANIFEST before extraction.
# Builds use make -j${JOBS:-2}. The swipl build takes ~5 min with -j2.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
src="$here/src"
dst="$here/install"
jobs="${JOBS:-2}"
mkdir -p "$src/dl" "$dst"
field() { awk -v n="$1" -v f="$2" '$1 == n { print $f }' "$here/MANIFEST"; }

# Download NAME's tarball to src/dl/NAME-VERSION.tar.gz, check its sha256,
# extract it afresh into $src; prints the extracted directory.
tarball() {
    v=$(field "$1" 2); url=$(field "$1" 3); sum=$(field "$1" 4)
    file="$src/dl/$1-$v.tar.gz"
    [ -f "$file" ] || curl -fsSL -o "$file" "$url"
    echo "$sum  $file" | sha256sum -c --quiet - >&2
    dir=$(tar tf "$file" | head -1); dir="$src/${dir%%/*}"
    rm -rf "$dir"
    tar xf "$file" -C "$src"
    echo "$dir"
}

trealla() {
    dir=$(tarball trealla)
    echo "building trealla (log: $src/trealla-make.log)"
    # GIT_DIR=/nonexistent: the GNUmakefile takes its version from
    # `git describe`, which would otherwise see this repository; without
    # git it falls back to the tarball's .tarball-version.
    flags="ISOCLINE=1 NOFFI=1 NOSSL=1 PREFIX=$dst/trealla"
    GIT_DIR=/nonexistent make -C "$dir" -j"$jobs" $flags >"$src/trealla-make.log" 2>&1
    GIT_DIR=/nonexistent make -C "$dir" $flags install >>"$src/trealla-make.log" 2>&1
    "$dst/trealla/bin/tpl" --version
}

swipl() {
    cmakedir=$(tarball cmake)
    dir=$(tarball swipl)
    echo "building swipl (logs: $src/swipl-*.log)"
    mkdir -p "$dir/build"
    (cd "$dir/build" && "$cmakedir/bin/cmake" .. \
        -DCMAKE_INSTALL_PREFIX="$dst/swipl" \
        -DCMAKE_BUILD_TYPE=Release \
        -DSWIPL_PACKAGES=OFF \
        -DINSTALL_DOCUMENTATION=OFF \
        -DBUILD_TESTING=OFF \
        -G "Unix Makefiles") >"$src/swipl-configure.log" 2>&1
    make -C "$dir/build" -j"$jobs" >"$src/swipl-make.log" 2>&1
    make -C "$dir/build" install >"$src/swipl-install.log" 2>&1
    # CMake installs swipl and libswipl.so with RUNPATH set to the absolute
    # library directory (cmake/Install.cmake sets CMAKE_INSTALL_RPATH, so a
    # -D cannot override it): make it $ORIGIN-relative, so that the install
    # still starts when moved (e.g. when built in a git worktree).
    arch="$dst/swipl/lib/swipl"
    python3 -I "$here/set_runpath.py" '$ORIGIN/../../lib/x86_64-linux' \
        "$arch/bin/x86_64-linux/swipl" "$arch/bin/x86_64-linux/swipl-win"
    python3 -I "$here/set_runpath.py" '$ORIGIN' "$arch"/lib/x86_64-linux/libswipl.so.*.*.*
    "$dst/swipl/bin/swipl" --version
}

programs() {
    dir=$(tarball swi-bench)
    python3 -I "$here/prepare.py" "$dir/programs" "$here/programs"
}

[ $# -gt 0 ] || set -- trealla swipl programs
for x in "$@"; do "$x"; done
