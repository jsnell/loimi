# guard2.sh: the trace of guard.sh again, with guard counters; run mandg650
. /tmp/claude-1000/-workspace-loimi/07e73a19-2567-4a1c-b683-fbb2b82f16eb/scratchpad/trace-exp-r7q3/env.sh
G=$S/progs
bash $S/gen.sh mandg mgtrc trace --counters --profile $S/mgreg.prof --map $S/mgreg.json || exit 1
for f in mandg mandg650; do
  LOIMI_JIT=1 LOIMI_TRACE_PROFILE=$S/mgtrc.$f.prof $S/mini-mgtrc $G/$f.lua
  python3 - $S/mgtrc.json $S/mgtrc.$f.prof <<'EOF'
import json, sys
m = json.load(open(sys.argv[1]))
inv = {v: k for k, v in m["blocks"].items()}
c = {int(l.split()[0]): int(l.split()[1]) for l in open(sys.argv[2]) if not l.startswith("trace")}
ent = c[m["blocks"]["%d:start" % m["steps"][0]]]
print("  first step entered %d times" % ent)
for i, n in sorted(c.items()):
    if inv[i].startswith("guard") and n:
        print("  %s: %d" % (inv[i], n))
EOF
done
