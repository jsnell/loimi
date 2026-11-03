#!/bin/sh
# Convert the pinned WebAssembly spec testsuite (top-level *.wast files only,
# not proposals/) to JSON + module files with wabt's wast2json.
#
#   bench/wasm/spec-json/<name>/<name>.json   (+ <name>.N.wasm / .wat files)
#   bench/wasm/spec-json/STATUS               one line per file:
#                                             "<name> ok|enable-all|FAILED"
#
# wast2json runs with wabt's default features first; a file it rejects is
# retried with --enable-all (recorded as "enable-all"). Needs install.sh first.
# Optional: spec.sh --check also runs wabt's spectest-interp on every json
# as a sanity check of the reference results (adds "<name> spectest P/T").
set -eu
here=$(cd "$(dirname "$0")" && pwd)
src="$here/src"
out="$here/spec-json"
bin="$here/install/bin"
check=${1:-}

field() { awk -v n="$1" -v f="$2" '$1 == n { print $f }' "$here/MANIFEST"; }
commit=$(field testsuite 2); url=$(field testsuite 3); sum=$(field testsuite 4)
tgz="$src/testsuite-$commit.tar.gz"
mkdir -p "$src"
if [ ! -f "$tgz" ]; then
  curl -fsSL -o "$tgz.part" "$url"
  mv "$tgz.part" "$tgz"
fi
echo "$sum  $tgz" | sha256sum -c -
rm -rf "$src/testsuite-$commit"
tar -C "$src" -xzf "$tgz"

rm -rf "$out"
mkdir -p "$out"
status="$out/STATUS"
: > "$status"
n_ok=0; n_all=0; n_fail=0
for w in "$src/testsuite-$commit"/*.wast; do
  name=$(basename "$w" .wast)
  d="$out/$name"
  mkdir -p "$d"
  if "$bin/wast2json" "$w" -o "$d/$name.json" 2>"$d/wast2json.err"; then
    echo "$name ok" >> "$status"; n_ok=$((n_ok + 1))
  elif rm -f "$d"/* && "$bin/wast2json" --enable-all "$w" -o "$d/$name.json" 2>"$d/wast2json.err"; then
    echo "$name enable-all" >> "$status"; n_all=$((n_all + 1))
  else
    echo "$name FAILED" >> "$status"; n_fail=$((n_fail + 1))
  fi
  [ -s "$d/wast2json.err" ] || rm -f "$d/wast2json.err"
done
echo "testsuite $commit: $n_ok ok, $n_all needed --enable-all, $n_fail failed"
grep -v ' ok$' "$status" || true

if [ "$check" = "--check" ]; then
  while read -r name how; do
    [ "$how" = FAILED ] && continue
    flags=""; [ "$how" = enable-all ] && flags="--enable-all"
    res=$(cd "$out/$name" && "$bin/spectest-interp" $flags "$name.json" 2>&1 | tail -1)
    echo "$name spectest $res"
  done < "$status" | tee "$out/SPECTEST"
fi
