#!/bin/sh
# Build the AArch64 cross toolchain listed in MANIFEST.
# Result: bench/cross/install/bin/qemu-aarch64 and bench/cross/sysroot-aarch64/
#
#   loimi build prog.lm --target=aarch64-linux-gnu --sysroot=bench/cross/sysroot-aarch64
#   bench/cross/install/bin/qemu-aarch64 -L bench/cross/sysroot-aarch64 ./prog
#
# Needs: curl, a C/C++ compiler, python3, pkg-config and the glib-2.0
# headers (for qemu), dpkg-deb. No root. JOBS=n sets make parallelism.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
src="$here/src"
dst="$here/install"
sysroot="$here/sysroot-aarch64"
jobs=${JOBS:-2}
mkdir -p "$src" "$dst/bin"

field() { awk -v n="$1" -v f="$2" '$1 == n { print $f }' "$here/MANIFEST"; }

# fetch NAME FILE: download NAME's url to $src/FILE and check its sha256.
fetch() {
  if [ ! -f "$src/$2" ]; then
    curl -fsSL -o "$src/$2.part" "$(field "$1" 3)"
    mv "$src/$2.part" "$src/$2"
  fi
  echo "$(field "$1" 4)  $src/$2" | sha256sum -c -
}

# ── sysroot: Debian arm64 packages, extracted ────────────────────────
rm -rf "$sysroot"
mkdir -p "$sysroot"
for p in libc6 libc6-dev libgcc-s1 libgcc-14-dev linux-libc-dev; do
  fetch "$p" "$p.deb"
  dpkg-deb -x "$src/$p.deb" "$sysroot"
done
# Debian 13 is merged-/usr, but the packages ship /lib/ld-linux-aarch64.so.1
# and friends under /lib: make /lib point into /usr/lib, as on a real
# system, so the absolute paths in libc.so's linker script and in the
# executable's interpreter resolve inside the sysroot.
if [ -d "$sysroot/lib" ]; then
  mkdir -p "$sysroot/usr/lib"
  cp -a "$sysroot/lib/." "$sysroot/usr/lib/"
  rm -rf "${sysroot:?}/lib"
fi
ln -s usr/lib "$sysroot/lib"

# ── ninja (qemu's build runs it) ─────────────────────────────────────
v=$(field ninja 2)
fetch ninja "ninja-$v.tar.gz"
rm -rf "$src/ninja-$v"
tar -C "$src" -xzf "$src/ninja-$v.tar.gz"
(cd "$src/ninja-$v" && python3 configure.py --bootstrap >/dev/null)
cp "$src/ninja-$v/ninja" "$dst/bin/ninja"

# ── qemu user-mode, aarch64 only ─────────────────────────────────────
v=$(field qemu 2)
fetch qemu "qemu-$v.tar.xz"
rm -rf "$src/qemu-$v"
tar -C "$src" -xJf "$src/qemu-$v.tar.xz"
mkdir -p "$src/qemu-$v/build"
# qemu's configure makes a Python venv, which needs ensurepip (Debian's
# python3-venv) unless pip and setuptools are importable: unpack them.
pyp=""
if ! python3 -c 'import ensurepip' 2>/dev/null; then
  rm -rf "$src/pylib"
  mkdir -p "$src/pylib"
  for p in pip setuptools; do
    fetch "$p" "$p.whl"
    python3 -I -m zipfile -e "$src/$p.whl" "$src/pylib"
  done
  pyp="$src/pylib"
fi
(
  cd "$src/qemu-$v/build"
  export PATH="$dst/bin:$PATH"
  [ -z "$pyp" ] || export PYTHONPATH="$pyp"
  ../configure --prefix="$dst" --target-list=aarch64-linux-user \
    --without-default-features --disable-docs --disable-werror >/dev/null
  ninja -j"$jobs" qemu-aarch64 >/dev/null
)
cp "$src/qemu-$v/build/qemu-aarch64" "$dst/bin/qemu-aarch64"

"$dst/bin/qemu-aarch64" --version | head -1
echo "sysroot: $sysroot"
