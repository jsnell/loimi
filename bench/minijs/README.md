# MiniJS benchmarks

Benchmarks for MiniJS (`examples/minijs`, a JavaScript subset), timed
against V8's bytecode interpreter and QuickJS. The latest results are in
[`docs/results.md`](../../docs/results.md); [`RESULTS.md`](RESULTS.md)
records how MiniJS's performance got there.

## Running

```sh
bench/js/install.sh                # optional, once: builds QuickJS
bench/minijs/compare.py            # all benchmarks
bench/minijs/compare.py -n 5 --dispatch tailcall goto richards nbody
```

`node` must be on `PATH`. `compare.py` runs each program under MiniJS,
`node --jitless` (V8's Ignition interpreter), `node` (V8 with its JIT)
and QuickJS if installed. It checks the output against `NAME.expected`
and prints the best wall-clock time of each.

## Where the programs come from

The 14 programs in `programs/`, and MiniJS's conformance tests in
`examples/minijs/tests/conformance/`, were written clean-room. A
separate agent was given only `examples/minijs/SPEC.md` and `node`, and
was told not to look at the Loimi repository or the MiniJS
implementation. Its exact instructions are in
[`CLEANROOM_PROMPT.md`](CLEANROOM_PROMPT.md), and
[`programs/README.md`](programs/README.md) is its own description of the
programs.

- Every program prints byte-identical output under `node` and
  `node --jitless`.
- The `.expected` files are node's output.
