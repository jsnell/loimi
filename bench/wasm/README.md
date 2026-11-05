# WebAssembly reference tools, spec tests and benchmarks

External pieces for the WebAssembly MVP interpreter in `examples/wasm/`.
Everything is downloaded from the URLs pinned (with sha256) in `MANIFEST`
and built from source; only the scripts, `MANIFEST` and `ports/` are
committed. Generated directories (`src/`, `install/`, `spec-json/`,
`programs-out/`) are gitignored.

| piece | pinned | used for |
|---|---|---|
| cmake 3.30.5 (prebuilt linux-x86_64) | release tarball | building wabt only |
| wabt 1.0.36 (+ libuv 1.37.0 for uvwasi) | release tarball | `wast2json`, `wasm-interp --wasi`, `wasm-objdump`, `wasm2wat`, `wat2wasm`, `spectest-interp` |
| wasm3 0.5.0 | tag tarball | fast reference interpreter (`-O3`, built-in WASI) |
| wasi-libc, tag wasi-sdk-25 | tag tarball | libc sysroot, rebuilt with `-mcpu=mvp` |
| compiler-rt builtins, wasi-sdk 25 | release asset | `libclang_rt.builtins-wasm32.a` for clang-19 |
| WebAssembly/testsuite f3f04866 | commit tarball | spec tests (the commit wabt 1.0.36 itself tests against) |
| eembc/coremark 1f483d5b | commit tarball | CoreMark |
| benchmarksgame-team/benchmarksgame 40296663 | commit tarball | Benchmarks Game C programs |
| PolyBench/C 4.2.1-beta | release tarball (SourceForge) | held-out set: 30 kernels (`polybench.sh`) |

## Usage

```sh
bench/wasm/install.sh      # tools + MVP sysroot -> bench/wasm/install/ (~5 min, JOBS=2)
bench/wasm/spec.sh         # testsuite -> bench/wasm/spec-json/<name>/<name>.json
bench/wasm/spec.sh --check #   ... and run wabt's spectest-interp on each json
bench/wasm/programs.sh     # benchmarks -> bench/wasm/programs-out/<name>.wasm
bench/wasm/programs-simd.sh  # nbody, fannkuchredux, spectralnorm with -msimd128 -> <name>_simd.wasm
bench/wasm/bench.py        # time wasm3 and wasm-interp (best of 3, markdown table)
bench/wasm/bench.py -n 1 --add-default --interp loimi='path/to/interp {wasm}' nbody
bench/wasm/polybench.sh   # PolyBench/C kernels -> bench/wasm/polybench-out/ (+ check/ builds)
bench/wasm/polybench.sh --check path/to/interp   # arrays of SMALL runs vs wasm3
bench/wasm/bench.py --dir bench/wasm/polybench-out --interp loimi='path/to/interp {wasm}'
```

`spec-json/STATUS` lists every top-level `.wast` and how it converted
(`ok` = wabt default features). The proposals/ subdirectories are skipped.
The top-level files include the post-MVP features of Wasm 2.0 (bulk memory,
reference types, multi-value, sign-ext, sat-trunc, SIMD); picking the MVP
subset is up to the interpreter's test runner.

## Benchmarks

Every module is compiled with `clang-19 --target=wasm32-wasi -O2 -mcpu=mvp`
against `install/wasi-sysroot-mvp`, needs no command-line arguments (problem
size fixed at compile time via `ports/argv_main.c`), and is checked by
`programs.sh` to validate with every post-MVP feature disabled (see
`programs-out/REPORT`). The prebuilt wasi-sdk sysroot is not used because it
is compiled for clang's "generic" CPU: `memory.copy`/`memory.fill` in
memcpy/memset, and `call_indirect` with a 5-byte padded table index (from
reference-types) that MVP decoders, including wasm3 0.5.0, reject.

| name | source | size |
|---|---|---|
| coremark | CoreMark, `simple/` port + `ports/coremark-wasi.patch` (timer via `clock_gettime`) | `ITERATIONS=5000` |
| nbody | Benchmarks Game `nbody.gcc` (#1) | 2500000 |
| fannkuchredux | `fannkuchredux.gcc` (#1) | 10 |
| spectralnorm | `spectralnorm.gcc-8.gcc` | 2000 |
| binarytrees | `binarytrees.gcc` (#1) | 15 |
| mandelbrot | `mandelbrot.gcc-8.gcc` (PBM to stdout) | 2400 |
| fasta | `fasta.gcc` (#1, 25 MB to stdout) | 2500000 |

`programs-simd.sh` builds three of them again with `-mcpu=mvp -msimd128`
(clang's auto-vectorizer; the libc stays MVP) as `<name>_simd.wasm`:
nbody (about 50 SIMD instructions), fannkuchredux (12) and spectralnorm
(2); the other programs get none. wasm3 0.5 has no SIMD, so their
`.expected` is a copy of the MVP build's; `bench/run_all.py` times them
as the `wasm-simd` suite against the MVP builds.

The Benchmarks Game programs are unmodified single-threaded C versions (no
pthreads/OpenMP/SIMD intrinsics); their wasm outputs match native gcc builds.
CoreMark validates its CRCs itself (`crclist 0xe714`, `crcmatrix 0x1fd7`,
`crcstate 0x8e3a` are the reference values for the 2K performance run); it
still ends with "Errors detected" because a run shorter than 10 s counts as
an error under CoreMark's reporting rules. `bench.py` ignores CoreMark's
timing lines when comparing outputs.

Imports (all `wasi_snapshot_preview1`): `fd_write`, `fd_seek`, `fd_close`,
`fd_fdstat_get`, `proc_exit`, plus `clock_time_get` for CoreMark.

## Held-out set: PolyBench/C

The interpreter's optimizations were measured on the seven programs
above, so `polybench.sh` builds a second, independent set: the 30
PolyBench/C 4.2.1 kernels (linear algebra, solvers, stencils, data
mining, dynamic programming), unmodified, with the same toolchain and
flags, problem size fixed per kernel (`MEDIUM`, `LARGE` or `EXTRALARGE`:
the largest that runs in at most about 2 s under wasm3; 0.05 to 1.7 s).
The timed builds print nothing; `check/NAME.wasm` are `SMALL_DATASET`
builds with `POLYBENCH_DUMP_ARRAYS`, whose stderr `--check` compares
with wasm3's. Results are in `examples/wasm/FRICTION.md`.

## Measuring

Changes to the interpreter are measured on both sets, interleaved
(each benchmark runs once under every build in turn, best of 5 to 7),
with `--align-functions=64` so that unrelated edits do not move the
handlers, and on user+sys CPU time when the machine is shared. Even so,
any change to a handler moves the ones after it, and on the i7-6700
(Skylake) used for these numbers placement alone moved single programs
by 3-4% either way, and before round three by 40%: a dispatch jump
that ended on a 32-byte boundary (Intel's JCC erratum) made cholesky, lu
and ludcmp take 1.4 times as long. `loimi build` now pads x86-64 vm
branches off those boundaries (`--no-branch-align` to compare;
`docs/lowering.md` §12). A change that moves a whole set's geometric
mean by less than 1% is within this placement noise.

