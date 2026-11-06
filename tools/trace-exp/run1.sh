# run1.sh BIN PROG [env...]: one run, check the output, print user+sys
. /tmp/claude-1000/-workspace-loimi/07e73a19-2567-4a1c-b683-fbb2b82f16eb/scratchpad/trace-exp-r7q3/env.sh
bin=$1; prog=$2; shift 2
/usr/bin/time -f "%U %S" -o $S/time.$$ env "$@" $S/$bin bench/mini/programs/$prog.lua > $S/out.$$ 2> $S/err.$$
rc=$?
if cmp -s $S/out.$$ bench/mini/programs/$prog.expected; then ok=OK; else ok=FAIL; fi
echo "$bin $prog rc=$rc $ok $(cat $S/time.$$)"
[ $ok = OK ] || { head -c 600 $S/out.$$; head -c 1500 $S/err.$$; }
rm -f $S/time.$$ $S/out.$$ $S/err.$$
