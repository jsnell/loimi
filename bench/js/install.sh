#!/bin/sh
# Build QuickJS (a bytecode interpreter) from the source listed in MANIFEST.
# Result: bench/js/install/bin/qjs
set -eu
here=$(cd "$(dirname "$0")" && pwd)
src="$here/src"; dst="$here/install"
mkdir -p "$src" "$dst/bin"
field() { awk -v n="$1" -v f="$2" '$1 == n { print $f }' "$here/MANIFEST"; }
v=$(field quickjs 2); url=$(field quickjs 3); sum=$(field quickjs 4)
tarball="$src/quickjs-$v.tar.xz"
[ -f "$tarball" ] || curl -fsSL -o "$tarball" "$url"
echo "$sum  $tarball" | sha256sum -c -
rm -rf "$src/quickjs-$v"
tar -C "$src" -xJf "$tarball"
make -C "$src/quickjs-$v" -j2 qjs >/dev/null
cp "$src/quickjs-$v/qjs" "$dst/bin/qjs"
"$dst/bin/qjs" -e 'console.log("qjs", 1 + 1)'
