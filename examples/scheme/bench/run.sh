#!/bin/sh
# Build the Scheme interpreter and time every benchmark, checking each
# program's output against its `; expect:` lines.
#
#   examples/scheme/bench/run.sh [loimi build flags...] [-- bench names...]
set -e
root=$(cd "$(dirname "$0")/../../.." && pwd)
dir="$root/examples/scheme/bench"
exe="${TMPDIR:-/tmp}/scheme-bench-$$"
flags=""
while [ $# -gt 0 ] && [ "$1" != "--" ]; do flags="$flags $1"; shift; done
[ "$1" = "--" ] && shift
"$root/target/release/loimi" build "$root/examples/scheme/main.lm" --release $flags -o "$exe"
names="$*"
[ -z "$names" ] && names=$(cd "$dir" && ls *.scm | sed 's/\.scm$//')
status=0
for b in $names; do
    f="$dir/$b.scm"
    want=$(sed -n 's/^; expect: \{0,1\}//p' "$f")
    start=$(date +%s.%N)
    got=$("$exe" "$f" 2>&1) || true
    end=$(date +%s.%N)
    if [ "$got" = "$want" ]; then ok=ok; else ok="WRONG OUTPUT"; status=1; fi
    printf "%-10s %7.2fs  %s\n" "$b" "$(awk "BEGIN { print $end - $start }")" "$ok"
    if [ "$ok" != ok ]; then printf '  expected: %s\n  got:      %s\n' "$want" "$got"; fi
done
rm -f "$exe"
exit $status
