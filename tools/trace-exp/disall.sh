S=/tmp/claude-1000/-workspace-loimi/07e73a19-2567-4a1c-b683-fbb2b82f16eb/scratchpad/trace-exp-r7q3
for b in "$@"; do bash $S/dis.sh mini-$b loimi.trace.$b; done
