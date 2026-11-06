. /tmp/claude-1000/-workspace-loimi/07e73a19-2567-4a1c-b683-fbb2b82f16eb/scratchpad/trace-exp-r7q3/env.sh
mkdir -p $S/keep
LOIMI_KEEP_STENCILS=$S/keep LOIMI_JIT_REPORT=1 loimi build examples/mini/main.lm --release --align-functions=64 --jit-stencils -o $S/mini-jit
ls -la $S/keep
