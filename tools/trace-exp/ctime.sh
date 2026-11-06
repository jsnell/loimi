# ctime.sh TRACE...: compile time of each trace function alone (llvm-extract, then clang -O2/-O3 -c)
S=/tmp/claude-1000/-workspace-loimi/07e73a19-2567-4a1c-b683-fbb2b82f16eb/scratchpad/trace-exp-r7q3
cd $S
for t in "$@"; do
  llvm-extract --func="loimi.trace.$t" -S $S/$t.full.ll -o $S/$t.alone.ll || continue
  n=$(grep -c '^  ' $S/$t.alone.ll)
  for o in -O1 -O2 -O3; do
    best=999
    for i in 1 2 3; do
      s=$(date +%s.%N); clang $o -fno-pic -c -x ir -Wno-override-module $S/$t.alone.ll -o $S/$t.alone.o; e=$(date +%s.%N)
      best=$(python3 -c "print(min($best, $e - $s))")
    done
    echo "$t: $n IR lines, clang $o: $best s, $(size $S/$t.alone.o | awk 'NR==2{print $1}') bytes text"
  done
done
