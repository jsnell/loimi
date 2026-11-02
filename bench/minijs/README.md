# MiniJS benchmarks

`programs/` holds 14 benchmarks for MiniJS (`examples/minijs/SPEC.md`, a
JavaScript subset). The conformance tests are in
`examples/minijs/tests/conformance/`.

**Provenance.** Both were written clean-room. A separate agent was given
only `SPEC.md` and `node`. It was told not to look at the Loimi
repository or the MiniJS implementation. Its exact instructions are in
[`CLEANROOM_PROMPT.md`](CLEANROOM_PROMPT.md); `programs/README.md` is its
own description of the programs.

- Every program prints byte-identical output under `node` and
  `node --jitless`.
- The `.expected` files come from node.

The reference interpreters are V8's bytecode interpreter
(`node --jitless`), V8 with its JIT (`node`), and optionally QuickJS
(`bench/js/install.sh`).
