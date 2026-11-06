# dis.sh BIN SYMBOL: disassemble one function to BIN.SYMBOL.asm, print its size
S=/tmp/claude-1000/-workspace-loimi/07e73a19-2567-4a1c-b683-fbb2b82f16eb/scratchpad/trace-exp-r7q3
read A SZ <<< $(nm -S $S/$1 | awk -v s="$2" '$4==s{print $1, $2}')
objdump -d --no-show-raw-insn $S/$1 --start-address=0x$A --stop-address=$(printf '0x%x' $((0x$A + 0x$SZ))) | tail -n +7 > $S/$1.asm
echo "$2 at 0x$A, $((0x$SZ)) bytes, $(grep -c '^ ' $S/$1.asm) instructions"
