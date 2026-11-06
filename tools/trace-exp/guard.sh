# guard.sh: record and trace mandg.lua (all-float rows), then run mandg650.lua
# (rows from 650 on have an integer ci, which the trace has never seen)
. /tmp/claude-1000/-workspace-loimi/07e73a19-2567-4a1c-b683-fbb2b82f16eb/scratchpad/trace-exp-r7q3/env.sh
G=$S/progs
LOIMI_JIT=1 LOIMI_TRACE_DUMP=$S/mandg.dump $S/mini-jit $G/mandg.lua || exit 1
grep '^code' $S/mandg.dump
bash $S/gen.sh mandg mgreg region || exit 1
LOIMI_JIT=1 LOIMI_TRACE_PROFILE=$S/mgreg.prof $S/mini-mgreg $G/mandg.lua > /dev/null || exit 1
bash $S/gen.sh mandg mgtr trace --counters --profile $S/mgreg.prof --map $S/mgreg.json || exit 1
for f in mandg mandg650; do
  $S/mini-interp $G/$f.lua > $S/$f.interp.out
  LOIMI_JIT=1 LOIMI_JIT_DEBUG=1 LOIMI_TRACE_PROFILE=$S/mgtr.$f.prof $S/mini-mgtr $G/$f.lua > $S/$f.trace.out 2> $S/$f.trace.err
  grep trace $S/$f.trace.err
  cmp $S/$f.interp.out $S/$f.trace.out && echo "$f: trace output matches the interpreter: $(cat $S/$f.trace.out)"
done
