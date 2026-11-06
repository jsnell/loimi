# Trace-tier experiment scripts (reference only)

The scripts behind `docs/design/trace-tier.md`, kept as they were run: an
offline trace generator (`tracegen.py`) that stitches a Mini loop's
`--jit-stencils` stencils into one LLVM function, plus the build, guard-test,
timing and profiling scripts around it. They are not maintained or tested.
Paths point at the experiment's scratch directory and worktree (`env.sh`);
change `WT` and `S` there to rerun. Stage 1 of the trace tier (doc §8)
replaces them with a compiler mode.
