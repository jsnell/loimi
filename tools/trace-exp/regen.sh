# regen.sh: rebuild loimi, then every trace binary with the final generator
S=/tmp/claude-1000/-workspace-loimi/07e73a19-2567-4a1c-b683-fbb2b82f16eb/scratchpad/trace-exp-r7q3
bash $S/rebuild.sh || exit 1
. $S/env.sh
loimi build examples/mini/main.lm --release --align-functions=64 --jit-stencils --keep-ll -o $S/mini-jit || exit 1
ALL=1 bash $S/genall.sh mandelbrot mand || exit 1
bash $S/genall.sh nbody nb --code 0 --header 23 || exit 1
bash $S/genall.sh sieve siv32 --header 32 || exit 1
bash $S/genall.sh sieve siv23 --header 23 || exit 1
bash $S/buildmerged.sh sivboth siv23tr siv32tr || exit 1
bash $S/genall.sh fannkuch fk40 --header 40 || exit 1
bash $S/guard.sh || exit 1
bash $S/guard2.sh || exit 1
echo REGEN DONE
