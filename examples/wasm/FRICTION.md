# WebAssembly interpreter: friction report

A usability study: a WebAssembly interpreter written from the docs
(`README.md`, `docs/language.md`, `docs/stdlib.md`, `docs/tutorial/`,
`std/`, `loimi --help`, `tests/run/*.lm`), without reading the other
example interpreters. It stresses what the earlier studies (stack
language, regex, Lox) did not: untagged typed values, structured control
flow resolved to jumps at load time, a validator the bytecode verifier
leans on, linear memory with bounds checks, `call_indirect` with
signature checks, and traps.

@@SUMMARY@@

## Design

**Register vm over untyped 64-bit slots.** Wasm is a stack machine, but
after validation every operand stack height is static, so each stack
position gets a fixed *home slot* in the frame (`nlocals + position`)
and `i32.add` becomes `I32_ADD r5, r3, r4` (the approach of wasm3 and
Wizard's fast path). The translator goes further and does not
materialize `local.get` or constants: a stack entry can be *pending*
(a local's slot, or a constant), so `local.get a; local.get b; i32.add;
local.set c` is one instruction, `I32_ADD c, a, b` (the result of the
last instruction is retargeted to the local when no label intervenes,
using `bc.Code.last()`). Pending entries are flushed to their home
slots at block boundaries, before a local they read is assigned, and
for calls. A stack design would have needed `local.get`/`local.set`
dispatches (about 40% of wasm instructions) and stack-pointer updates;
the register design removes them and keeps operands typed per op.

**Typed views of one slot.** Values are untagged: the validator has
proved every type, so each op reads a slot as the type it needs.
Operand kinds do this directly:

```loimi
operand ri = *((regs + it) as *i32) verify it < (f.nslots as u64);
operand ru = *((regs + it) as *u32) verify it < (f.nslots as u64);
operand rd = *((regs + it) as *f64) verify it < (f.nslots as u64);
op I32_LT_U(d: ri, a: ru, b: ru) : RRR { d = b2i(a < b); next }
```

**What the vm verifier checks, and what the wasm validator checks.**
The two divide the work cleanly. The wasm validator (compile.lm) proves
types: no op ever reads a slot as a type that was not written there.
The generated `W.verify` proves memory safety of the translated code:
every slot index is below the frame size (`it < f.nslots`), every
function, global and type index is in range for the instance
(`verify_context f: *Func, inst: *Inst`), every jump lands on an
instruction, `then JMP` follows every compare-and-branch, and a
`verify brtable { .. }` block checks that `BR_TABLE n` is followed by
n + 1 JMPs. Type confusion between slots cannot break memory safety
(every view stays inside its 8-byte slot), so the vm verifier does not
need types. Bounds that only exist at run time (memory size, table
size, call depth, stack room for the callee's frame) are checked in the
handlers, on cold paths.

The verifier earned its keep: it rejected the translator's output
three times during development, each a real translator bug (see item 1).

Files: `decode.lm` (binary format, everything "malformed"),
`compile.lm` (validator and translator in one pass), `vm.lm` (the vm
and runtime structures), `rt.lm` (module validation, linking,
instantiation, invocation), `host.lm` (`spectest`, `env`, a WASI
subset), `main.lm` (CLI and the spec-test command protocol).

@@REST@@
