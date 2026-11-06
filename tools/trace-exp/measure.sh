S=/tmp/claude-1000/-workspace-loimi/07e73a19-2567-4a1c-b683-fbb2b82f16eb/scratchpad/trace-exp-r7q3
N=${N:-11}
cd $S
J=LOIMI_JIT=1
uptime
python3 bench.py -n $N mandelbrot interp=mini-interp jit=mini-mandtr:$J,LOIMI_TRACE=0 jitAA=mini-mandpeel:$J,LOIMI_TRACE=0 region=mini-mandregnc:$J noguard=mini-mandnoguard:$J trace=mini-mandtr:$J peel=mini-mandpeel:$J
python3 bench.py -n $N nbody interp=mini-interp jit=mini-nbtr:$J,LOIMI_TRACE=0 jitAA=mini-nbtr:$J,LOIMI_TRACE=0 trace=mini-nbtr:$J
python3 bench.py -n $N sieve interp=mini-interp jit=mini-sivboth:$J,LOIMI_TRACE=0 jitAA=mini-sivboth:$J,LOIMI_TRACE=0 trace=mini-sivboth:$J
python3 bench.py -n $N fannkuch interp=mini-interp jit=mini-fk40tr:$J,LOIMI_TRACE=0 jitAA=mini-fk40tr:$J,LOIMI_TRACE=0 trace=mini-fk40tr:$J
python3 bench.py -n $N $S/progs/mandg650.lua interp=mini-interp jit=mini-mgtr:$J,LOIMI_TRACE=0 trace=mini-mgtr:$J
uptime
