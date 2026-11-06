# genall.sh PROG TAG [tracegen args]: region (counted) build, profile run, then the variants
S=/tmp/claude-1000/-workspace-loimi/07e73a19-2567-4a1c-b683-fbb2b82f16eb/scratchpad/trace-exp-r7q3
prog=$1; t=$2; shift 2
bash $S/gen.sh $prog ${t}reg region "$@" || exit 1
LOIMI_JIT=1 LOIMI_TRACE_PROFILE=$S/${t}reg.prof LOIMI_STD=/workspace-loimi/.claude/worktrees/agent-afbf2c667b9e597c2 $S/mini-${t}reg /workspace-loimi/.claude/worktrees/agent-afbf2c667b9e597c2/bench/mini/programs/$prog.lua > /dev/null || exit 1
P="--profile $S/${t}reg.prof --map $S/${t}reg.json"
bash $S/gen.sh $prog ${t}tr trace $P "$@" || exit 1
[ -n "$ALL" ] || exit 0
bash $S/gen.sh $prog ${t}regnc region --nocount "$@" || exit 1
bash $S/gen.sh $prog ${t}peel trace --peel $P "$@" || exit 1
bash $S/gen.sh $prog ${t}noguard trace --guards none $P "$@" || exit 1
