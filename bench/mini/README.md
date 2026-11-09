# Mini benchmarks

Benchmarks for Mini (`examples/mini`, a Lua 5.4 subset), timed against
PUC-Lua and LuaJIT. The latest results are in
[`docs/results.md`](../../docs/results.md); [`RESULTS.md`](RESULTS.md)
records how Mini's performance got there.

| Directory | Contents |
|---|---|
| `programs/` | 14 clean-room benchmark programs |
| [`coroutines/`](coroutines/README.md) | 3 public Lua coroutine benchmarks, adapted to Mini |

## Running

```sh
bench/lua/install.sh               # once: builds lua and luajit from source
bench/mini/compare.py              # all benchmarks
bench/mini/compare.py -n 5 --dispatch tailcall goto fib nbody
```

`compare.py` builds Mini, runs each program under every implementation
in turn, checks the output against `NAME.expected`, and prints the best
time of each.

## Where the programs come from

The programs, and Mini's conformance tests in
`examples/mini/tests/conformance/`, were written clean-room. A separate
agent was given only `examples/mini/SPEC.md` and the reference `lua` and
`luajit` binaries, and was told not to look at the Loimi repository or
the Mini implementation. Its exact instructions are in
[`CLEANROOM_PROMPT.md`](CLEANROOM_PROMPT.md), and
[`programs/README.md`](programs/README.md) is that agent's own
description of the programs.

Each program:

- is valid Lua;
- prints byte-identical output under PUC-Lua 5.4.9 and LuaJIT 2.1;
- takes 1–2.5 s under PUC-Lua.

The `.expected` files are PUC-Lua's output.
