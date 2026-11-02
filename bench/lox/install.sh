#!/bin/sh
# Fetch Crafting Interpreters at the commit pinned in MANIFEST and build clox
# as the book's final version has it: c/common.h defines NAN_BOXING and
# undefines every DEBUG_* flag; built with the repo's own `make clox`
# (MODE=release: -O3 -flto, -std=c99).
# Result: bench/lox/install/bin/clox, bench/lox/install/test/ (test suite,
#         benchmarks under test/benchmark/).
set -eu
here=$(cd "$(dirname "$0")" && pwd)
src="$here/src"
dst="$here/install"
mkdir -p "$src" "$dst/bin"

field() { awk -v n="$1" -v f="$2" '$1 == n { print $f }' "$here/MANIFEST"; }

url=$(field craftinginterpreters 2); sum=$(field craftinginterpreters 3)
tgz="$src/craftinginterpreters.tar.gz"
[ -f "$tgz" ] || curl -fsSL -o "$tgz" "$url"
echo "$sum  $tgz" | sha256sum -c -
rm -rf "$src/ci"
mkdir -p "$src/ci"
tar -C "$src/ci" --strip-components=1 -xzf "$tgz"
grep -q '^#define NAN_BOXING' "$src/ci/c/common.h"
make -C "$src/ci" clox CC="${CC:-cc}" >/dev/null
cp "$src/ci/build/clox" "$dst/bin/clox"
rm -rf "$dst/test"
cp -r "$src/ci/test" "$dst/test"
echo 'print "clox ok";' > "$src/smoke.lox"
"$dst/bin/clox" "$src/smoke.lox"
