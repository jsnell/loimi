# Mini benchmarks

Each `NAME.lua` is self-contained and deterministic. Its problem size is a
constant at the top of the file, and `NAME.expected` holds its output.
Output is byte-identical under `bin/lua` (PUC-Lua 5.4.9) and `bin/luajit`
(LuaJIT 2.1). Only integers are printed with `print`. Floats always go
through `string.format`. All integer values, intermediates included, stay
inside the 32-bit range.

| Benchmark | Stresses | Origin | bin/lua | bin/luajit |
|---|---|---|---:|---:|
| fib | recursive calls and returns, integer `+ -`, compares | classic naive Fibonacci (fib(37)) | 1.56 s | 0.20 s |
| binary_trees | table allocation, GC, recursion | Benchmarks Game binary-trees (depth 16) | 2.47 s | 0.97 s |
| nbody | float arithmetic, field reads/writes, `math.sqrt` | Benchmarks Game n-body (500k steps) | 1.22 s | 0.09 s |
| spectral_norm | float arithmetic, array indexing, small calls in inner loops | Benchmarks Game spectral-norm (N=1000) | 1.69 s | 0.07 s |
| fannkuch | array permutations, integer loops, multiple-assignment swaps, `repeat` | Benchmarks Game fannkuch-redux (N=10) | 1.68 s | 0.31 s |
| string_build | `..`, `tostring` of numbers, `table.concat`, `string.sub/byte/char/rep/format` | original | 1.72 s | 0.38 s |
| closures | closure creation, upvalue reads and writes, shared upvalues, per-iteration loop variables, higher-order calls | original | 1.22 s | 0.78 s |
| hash_strings | string-keyed hash tables: insert, lookup, update, delete with nil during `pairs`, record field access | original (word-frequency style) | 1.62 s | 0.27 s |
| oo_methods | `obj:m()` calls on tables of functions (no metatables), method overriding, chained calls | original, in the spirit of Richards/DeltaBlue | 1.33 s | 0.19 s |
| sieve | boolean array stores, integer `for` with step | classic Sieve of Eratosthenes | 1.34 s | 0.22 s |
| sort | quicksort, heapsort and insertion sort in Mini on ints, floats and strings; comparisons and recursion | original (classic algorithms) | 1.08 s | 0.31 s |
| mandelbrot | float arithmetic in tight loops, `break` | Benchmarks Game mandelbrot (1300x1300, bits packed arithmetically, checksummed) | 1.18 s | 0.15 s |
| queens | backtracking recursion, boolean arrays, nested closures | classic N-queens (4..12, three times) | 1.20 s | 0.52 s |
| interp | scanner (`string.byte/sub`), recursive-descent parser, AST tables, compiler, `if/elseif` opcode dispatch loop | original | 1.96 s | 0.38 s |

Times are wall-clock for one run on the development machine (8 cores, one
used). They were stable to about ±0.05 s across repeated runs.

Running one benchmark: `bin/lua bench/fib.lua | cmp - bench/fib.expected`.
