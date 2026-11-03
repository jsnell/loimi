#!/bin/sh
# Build the reference interpreters listed in MANIFEST from source.
# Result: bench/lua/install/bin/{lua,luajit}
# Lua is built with its stock makefile (-O2; computed goto is on by default
# in 5.4 with GCC/clang). LuaJIT with its stock makefile.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
src="$here/src"
dst="$here/install"
mkdir -p "$src" "$dst/bin"

field() { awk -v n="$1" -v f="$2" '$1 == n { print $f }' "$here/MANIFEST"; }

# ── PUC-Lua ──────────────────────────────────────────────────────────
v=$(field lua 2); url=$(field lua 3); sum=$(field lua 4)
tgz="$src/lua-$v.tar.gz"
[ -f "$tgz" ] || curl -fsSL -o "$tgz" "$url"
echo "$sum  $tgz" | sha256sum -c -
rm -rf "$src/lua-$v"
tar -C "$src" -xzf "$tgz"
make -C "$src/lua-$v" -j"$(nproc)" linux CC="${CC:-cc}" >/dev/null
cp "$src/lua-$v/src/lua" "$dst/bin/lua"

# ── PUC-Lua test suite (sources only; not built or run here) ─────────
v=$(field luatests 2); url=$(field luatests 3); sum=$(field luatests 4)
tgz="$src/lua-$v-tests.tar.gz"
[ -f "$tgz" ] || curl -fsSL -o "$tgz" "$url"
echo "$sum  $tgz" | sha256sum -c -
rm -rf "$src/lua-$v-tests"
tar -C "$src" -xzf "$tgz"

# ── LuaJIT ───────────────────────────────────────────────────────────
url=$(field luajit 3); commit=$(field luajit 4)
if [ ! -d "$src/luajit/.git" ]; then
  git clone -q "$url" "$src/luajit"
fi
git -C "$src/luajit" fetch -q origin
git -C "$src/luajit" checkout -q "$commit"
git -C "$src/luajit" clean -qfdx
make -C "$src/luajit" -j"$(nproc)" >/dev/null
cp "$src/luajit/src/luajit" "$dst/bin/luajit"

"$dst/bin/lua" -v
"$dst/bin/luajit" -v
