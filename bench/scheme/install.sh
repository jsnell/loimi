#!/bin/sh
# Build the reference interpreter listed in MANIFEST from source.
# Result: bench/scheme/install/bin/chibi-scheme (gitignored), built with
# its stock makefile (-O2 by default; computed-goto dispatch is on by default).
set -eu
here=$(cd "$(dirname "$0")" && pwd)
src="$here/src"
dst="$here/install"
mkdir -p "$src" "$dst"
field() { awk -v n="$1" -v f="$2" '$1 == n { print $f }' "$here/MANIFEST"; }

url=$(field chibi-scheme 2); commit=$(field chibi-scheme 3)
[ -d "$src/chibi-scheme/.git" ] || git clone -q "$url" "$src/chibi-scheme"
git -C "$src/chibi-scheme" fetch -q origin
git -C "$src/chibi-scheme" checkout -q "$commit"
git -C "$src/chibi-scheme" clean -qfdx
make -C "$src/chibi-scheme" -j"${JOBS:-2}" PREFIX="$dst" >/dev/null
make -C "$src/chibi-scheme" PREFIX="$dst" install >/dev/null
LD_LIBRARY_PATH="$dst/lib" "$dst/bin/chibi-scheme" -V
