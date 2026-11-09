# Design decisions

Questions the design left open, and how they were settled, for
contributors who want to know why things are the way they are. The
numbers come from the original list of questions; other documents cite
them ("roadmap, question 2"), so they are kept, with gaps where a
question was dropped. What exists is in [`status.md`](status.md);
benchmark numbers are in [`results.md`](results.md).

## 2. Should the GC move objects?

**No.** The heap is non-moving. It has an opt-in generational mode with
sticky mark bits (`heap.set_generational(true)`;
[`stdlib/gc.md`](stdlib/gc.md#generational-mode-sticky-mark-bits-non-moving)).

- **Why not moving.** Every interpreter keeps raw pointers into objects
  across safepoints: Mini's table parts and upvalue pointers, MiniJS's
  `Arr.ptr`, every `k` pointer into a constant array. A copying nursery
  would need each of them re-derived after every `gc` call. A
  non-moving heap needs only a write barrier.
- **Cost of the barrier.** One header byte test after each reference
  store. With generations off it costs nothing measurable (Mini 1.004,
  MiniJS 1.006 by geometric mean; worst case MiniJS `richards` +3%).
- **Gain from generations.** Mini 0.982, MiniJS 0.987, Scheme 0.986 of
  the time without them; all three turn them on.
- **Rejected: aging small objects.** Keeping every first survivor young
  traced growing live data twice (`nboyer` 0.798 s against 0.762 s,
  `earley` +9%, `mperm` +7%). Small objects are promoted when they
  first survive.
- **No card marking.** A large array that is stored into is retraced
  whole by each minor collection.

## 4. Threads

**One thread in a heap at a time.**

- Independent states run on different threads with no locks. The
  runtime's per-thread state is initial-exec TLS, within noise of plain
  globals (0.99–1.01).
- Several threads can share one heap and interpreter state through
  `std.gil`: a lock per heap, handed over at safepoints the interpreter
  polls. Mini built with `--cfg gil` is 1.2–1.4% slower single-threaded
  (geometric mean).
- Free threading (several threads running vm code in one heap at once)
  is not supported.

Details: `language.md` §10.1.

## 6. A JIT

**A copy-and-patch JIT is built; work on a template JIT stopped there**
([`design/copy-and-patch.md`](design/copy-and-patch.md)).

- `--jit-stencils` generates a baseline JIT from the vm's handlers with
  no extra source. It runs in 0.76–0.83 of the interpreters' time by
  geometric mean (Mini, MiniJS, Scheme; Prolog 0.94).
- These gained nothing: re-patching on quickening, patching constant
  operands into the code, and caching one guest register in a machine
  register (1.5% slower).
- What remains needs register allocation across instructions.
  Ahead-of-time traces did that and were removed
  (`design/speed-audit.md`, item 14).

## 7. A checked register window

**No.** The verifier is how a handler knows an index is in range: an
operand kind's `verify` clause states the whole range it is used for.
A run-time check on Mini's FOR* and RET1 accesses cost 2–9% (fib +5%,
nbody +7%, even with one check per op).

## 9. The embedding API

**The interpreter's own `export "C"` functions.** `loimi build --lib` or
`--shared` builds the library, and `--header` declares the functions
(`language.md` §10.1).

- The host gets an opaque state pointer (`MiniState *`).
- Payload-less enums are error codes; `loimi_str` carries strings;
  `#[c_const(..)]` on an export marks its `const` pointers.
- `Gc<T>` and `value` types don't cross into C. Hosts refer to values
  by position on a rooted value stack, as with Lua's stack API.
- Natives call back into the interpreter through nested entries
  (`uses reenter`).
- The header is `extern "C"`-guarded, so C++ can use it.

## 10. Immediates wider than a format's word

**`const` inline cache fields** (`cache Imm64 { const v: u64 }`,
`language.md` §7.1). The emitter sets them, `deopt` keeps them, and
`verify` clauses check them once. Wasm's `call_indirect` table index is
one.

## 11. The cost of a three-operand op

`F64_MUL d, a, b` is 14 instructions, against wasm3's 5:

- 6 to decode three 16-bit fields;
- 3 to load, multiply and store;
- 5 to dispatch.

**What helps:** accumulators (`language.md` §7.4; wasm runs in 0.91 of
its time) and `#[forward]` supers, which remove the store-then-load
round trips between ops.

**What doesn't pay on x86-64** (`design/speed-audit.md`, item 2):

- pre-scaled byte-offset fields: 1.003 on PolyBench;
- offset-threaded dispatch: 1.008;
- direct threading would save only the table load, which the predicted
  indirect jump already hides.
