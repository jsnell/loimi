# gen.sh PROG NAME MODE [extra tracegen args]: generate a trace and build mini with it
. /tmp/claude-1000/-workspace-loimi/07e73a19-2567-4a1c-b683-fbb2b82f16eb/scratchpad/trace-exp-r7q3/env.sh
prog=$1; name=$2; mode=$3; shift 3
python3 $S/tracegen.py --stencils $S/keep/main.stencils.ll --main $S/mini-jit.ll --dump $S/$prog.dump \
  --mode $mode --name $name --out $S/$name.ll --mapout $S/$name.json "$@" || exit 1
# check the IR on its own: the trace appended to the program
cat $S/mini-jit.ll $S/$name.ll > $S/$name.full.ll
opt -passes=verify -disable-output $S/$name.full.ll || exit 1
LOIMI_TRACE_IR=$S/$name.ll loimi build examples/mini/main.lm --release --align-functions=64 --jit-stencils -o $S/mini-$name || exit 1
echo built $S/mini-$name
