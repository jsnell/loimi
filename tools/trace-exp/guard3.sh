# guard3.sh: the mandg trace without counters, with and without blacklisting; time mandg and mandg650
S=/tmp/claude-1000/-workspace-loimi/07e73a19-2567-4a1c-b683-fbb2b82f16eb/scratchpad/trace-exp-r7q3
G=$S/progs
bash $S/gen.sh mandg mgtr trace --profile $S/mgreg.prof --map $S/mgreg.json || exit 1
bash $S/gen.sh mandg mgtrnb trace --blacklist 0 --profile $S/mgreg.prof --map $S/mgreg.json || exit 1
cd $S
for f in mandg mandg650; do
  python3 bench.py -n 7 $G/$f.lua interp=mini-interp jit=mini-mgtr:LOIMI_JIT=1,LOIMI_TRACE=0 trace=mini-mgtr:LOIMI_JIT=1 noblacklist=mini-mgtrnb:LOIMI_JIT=1
done
