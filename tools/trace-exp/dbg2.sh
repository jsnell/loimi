. /tmp/claude-1000/-workspace-loimi/07e73a19-2567-4a1c-b683-fbb2b82f16eb/scratchpad/trace-exp-r7q3/env.sh
bin=$1; prog=$2; shift 2
env LOIMI_JIT=1 LOIMI_JIT_DEBUG=1 LOIMI_TRACE_PROFILE=$S/$bin.prof "$@" $S/$bin bench/mini/programs/$prog.lua 2>&1 | grep -v '^    *[0-9]'
