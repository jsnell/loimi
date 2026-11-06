. /tmp/claude-1000/-workspace-loimi/07e73a19-2567-4a1c-b683-fbb2b82f16eb/scratchpad/trace-exp-r7q3/env.sh
prog=${1:-mandelbrot}
LOIMI_JIT=1 LOIMI_JIT_DEBUG=1 LOIMI_JIT_STATS=1 $S/mini-jit bench/mini/programs/$prog.lua 2>&1
