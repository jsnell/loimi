# buildmerged.sh NAME A B ...: build mini with the traces A.ll B.ll .. together
. /tmp/claude-1000/-workspace-loimi/07e73a19-2567-4a1c-b683-fbb2b82f16eb/scratchpad/trace-exp-r7q3/env.sh
name=$1; shift
python3 $S/merge.py $S/$name.ll $(for t in "$@"; do echo $S/$t.ll; done) || exit 1
cat $S/mini-jit.ll $S/$name.ll > $S/$name.full.ll
opt -passes=verify -disable-output $S/$name.full.ll || exit 1
LOIMI_TRACE_IR=$S/$name.ll loimi build examples/mini/main.lm --release --align-functions=64 --jit-stencils -o $S/mini-$name || exit 1
echo built $S/mini-$name
