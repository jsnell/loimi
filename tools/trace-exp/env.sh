ulimit -v 2500000; ulimit -c 0
export WT=/workspace-loimi/.claude/worktrees/agent-afbf2c667b9e597c2
export S=/tmp/claude-1000/-workspace-loimi/07e73a19-2567-4a1c-b683-fbb2b82f16eb/scratchpad/trace-exp-r7q3
export LOIMI_STD=$WT PATH=$WT/target/release:$PATH CARGO_BUILD_JOBS=2 RUST_TEST_THREADS=2
cd $WT
