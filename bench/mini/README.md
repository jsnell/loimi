# Mini vs PUC-Lua vs LuaJIT

`programs/` holds 14 benchmarks for the Mini language
(`examples/mini/SPEC.md`, a Lua 5.4 subset). The conformance tests are in
`examples/mini/tests/conformance/`.

**Provenance.** Both were written clean-room. A separate agent was given
only `SPEC.md` and the reference `lua` / `luajit` binaries. It was told
not to look at the Loimi repository or the Mini implementation. Its exact
instructions are in [`CLEANROOM_PROMPT.md`](CLEANROOM_PROMPT.md).
`programs/README.md` is that agent's own description of the programs.

Each program:
- is valid Lua;
- prints byte-identical output under PUC-Lua 5.4.9 and LuaJIT 2.1;
- takes 1–2.5 s under PUC-Lua.

The `.expected` files come from running PUC-Lua.

To run:

```sh
bench/lua/install.sh               # once: builds lua and luajit from source
bench/mini/compare.py              # all benchmarks
bench/mini/compare.py -n 5 --dispatch tailcall goto fib nbody
```
