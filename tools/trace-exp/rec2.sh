. /tmp/claude-1000/-workspace-loimi/07e73a19-2567-4a1c-b683-fbb2b82f16eb/scratchpad/trace-exp-r7q3/env.sh
for p in "$@"; do LOIMI_JIT=1 LOIMI_TRACE_DUMP=$S/$p.dump $S/mini-jit bench/mini/programs/$p.lua > /dev/null; grep '^code' $S/$p.dump | awk '$6>=0' | tail -3; done
