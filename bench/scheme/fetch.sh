#!/bin/sh
# Download the r7rs-benchmarks suite at the commit pinned in MANIFEST.
# Result: bench/scheme/r7rs-benchmarks/ (gitignored).
set -eu
here=$(cd "$(dirname "$0")" && pwd)
field() { awk -v n="$1" -v f="$2" '$1 == n { print $f }' "$here/MANIFEST"; }
url=$(field r7rs-benchmarks 2); commit=$(field r7rs-benchmarks 3)
dir="$here/r7rs-benchmarks"
[ -d "$dir/.git" ] || git clone -q "$url" "$dir"
git -C "$dir" fetch -q origin
git -C "$dir" checkout -q "$commit"
echo "r7rs-benchmarks at $(git -C "$dir" rev-parse HEAD)"
