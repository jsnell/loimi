# 3. A first vm

This chapter compiles the calculator's tree to bytecode and runs it on a
virtual machine declared with Loimi's `vm` block. The values are still
plain `i64`s; chapter 4 makes them dynamically typed.

Code: [`examples/tutorial/step3/`](../../examples/tutorial/step3)
(`vm.lm`, `compile.lm`, `main.lm`, plus the lexer and parser from step 2).

```sh
./target/release/loimi run examples/tutorial/step3/main.lm -- examples/tutorial/step3/tests/arith.pip --disasm
```

## What a C interpreter looks like

A bytecode interpreter in C is a loop around a `switch`:

```c
for (;;) {
    uint32_t i = *pc++;
    switch (i & 0xff) {
    case OP_ADD: R(A(i)) = R(B(i)) + R(C(i)); break;
    ...
    }
}
```

and then the work starts. The `switch` compiles to one indirect branch
that every opcode shares, which predicts badly, so you switch to GCC's
computed goto (`goto *labels[op]`) behind a macro, with a `switch`
fallback for compilers that lack it. The register allocator sees one
enormous function and spills `pc` and the register base around any call
in any case, so you sprinkle `register` hints or rewrite the hot ops.
`A(i)`, `B(i)`, `C(i)` are macros that must agree with the compiler's
`CREATE_ABC` macros and with the disassembler, which someone writes
later and which slowly drifts. Loading bytecode from a file means either
trusting it or bounds-checking every operand on every execution.

CPython 3.14 and some newer interpreters instead put each opcode in its
own function and chain them with guaranteed tail calls (`musttail`) in
a calling convention that keeps the interpreter state in registers
(`preserve_none`). That is the fastest portable shape, and the most
tedious to write by hand.

A Loimi `vm` declares the parts once, and the compiler produces the
dispatch loop in whichever of these shapes you ask for, plus the
encoders, the disassembler and a verifier, all from the same
declarations.

## The `vm` block

```loimi
pub vm Calc {
    // Live across every instruction, and kept in machine registers.
    state { pc: *u32, regs: *i64, k: *i64 }
```

`state` lists the variables that live across every instruction: the
program counter, the base of the register file, the constant table.
Inside handlers they are ordinary mutable locals. In the default
tail-call mode they are the parameters of every handler function, in
the `preserve_none` convention, so they sit in the same machine
registers through the whole run. Keep the list short (three to six):
each variable is a register taken from every handler.

```loimi
    // The generated verifier `Calc.verify(code, lim)` takes this.
    verify_context lim: *Limits;

    // What an encoded field means in a handler, and what the verifier
    // must check about it.
    operand reg = regs[it] verify (it as u32) < lim.nregs display "r{}";
    operand kst = k[it] verify (it as u32) < lim.nconsts display "k{}";
    operand imm = it;
```

An **operand kind** says what a field of an instruction means. `it` is
the raw field (a `u64`, or an `i64` for a signed field). An operand of
kind `reg` *is* `regs[it]`: reading it loads the register, assigning to
it stores. Its `verify` clause is not run by the handler; it is pasted
into the generated verifier, which checks it once per instruction when
the code is loaded. `display` says how the disassembler prints it.

```loimi
    // Every instruction is one 32-bit word.
    encoding fixed(u32) {
        format ABC  { op: 8, a: 8, b: 8, c: 8 }
        format ABx  { op: 8, a: 8, bx: 16 }
        format AsBx { op: 8, a: 8, sbx: s16 }
    }
```

**Formats** lay out the fields of a word, Lua-style. `s16` is a signed
field. These declarations are the single source of truth for the
encoders, the decoders inlined into handlers, the disassembler and the
verifier, so they cannot disagree.

Then the **ops**. Each names its operands in field order, with their
kinds, and its format:

```loimi
    op LOADI(dst: reg, v: imm) : AsBx { dst = v; next }
    op LOADK(dst: reg, c: kst) : ABx { dst = c; next }
    op MOVE(dst: reg, src: reg) : ABC { dst = src; next }
    op NEG(dst: reg, a: reg) : ABC { dst = 0 - a; next }
    op ADD(dst: reg, a: reg, b: reg) : ABC { dst = a + b; next }
```

A handler's body has type `never`: every path must end in a **control
word**. `next` advances `pc` past the instruction and dispatches the
next one; `jump t` sets `pc` and dispatches; `exit v` leaves the run
loop and returns `v`; `fail e` leaves it returning `Err(e)` (the entry
must return a `Result`):

```loimi
    op DIV(dst: reg, a: reg, b: reg) : ABC {
        if b == 0 { fail "division by zero" }
        dst = a / b;
        next
    }
```

```loimi
    op PRINT(a: reg) : ABC { io.print_int(a); io.println(""); next }
    op HALT() : ABC { exit Ok(()) }
```

Finally the **entry**, the way in. It sets up the state and dispatches
the first instruction:

```loimi
    // The way in: set up the state, then dispatch the first instruction.
    entry run(code: *u32, r: *i64, consts: *i64) -> Result<(), str> {
        pc = code;
        regs = r;
        k = consts;
        jump pc
    }
```

The compiler turns this into a function `Calc.run(code, regs, consts)`
that the rest of the program calls like any other.

## What it compiles to

`loimi build --release` and `objdump -d` show the ADD handler (each op
is its own symbol, `vm.Calc.op.ADD`, which also makes profiles easy to
read):

```text
<vm.Calc.op.ADD>:
  mov    %r15d,%eax              ; r15: the instruction word
  mov    %r15d,%ecx
  movzbl %ah,%edx                ; dst = field a
  shr    $0xd,%eax
  and    $0x7f8,%eax             ; field b, times 8
  shr    $0x18,%ecx              ; field c
  mov    0x0(%r13,%rcx,8),%rcx   ; r13: regs
  add    0x0(%r13,%rax,1),%rcx
  mov    %rcx,0x0(%r13,%rdx,8)
  mov    0x4(%r12),%r15d         ; r12: pc; load the next word
  add    $0x4,%r12
  movzbl %r15b,%eax
  jmp    *0x40b1c0(,%rax,8)      ; dispatch through the table
```

Thirteen instructions, no stack frame, no spills, and the dispatch is a
separate indirect jump at the end of every handler, so the branch
predictor learns per-op successors. `pc`, `regs` and the instruction
word arrive in registers and leave in the same registers. This is
`--dispatch=tailcall`, the default on x86-64 and AArch64:
each op is an LLVM function in the `preserve_none` convention, and
`next` is a `musttail` call through the table.

`--dispatch=goto` builds the same source as one function with a
replicated `indirectbr` (computed goto), and `--dispatch=switch` as a
loop around a `switch`. `examples/tutorial/check.sh` runs every step's
tests in all three. Chapter 8 compares their speed.

## Generated tools

From the declarations, the compiler generates, for each op:

- an **encoder**: `Calc.ADD(dst, a, b)` returns the instruction word,
  with parameter types from the format (`u8` for an 8-bit field, `i16`
  for `s16`);
- an **emitter**: `Calc.emit.ADD(code, at, dst, a, b)` stores it into a
  slice and returns the next offset;
- an opcode constant `Calc.OP_ADD`, `Calc.op_name(w)`, `Calc.op_size(w)`
  and `Calc.NUM_OPS`;

and for the vm: `Calc.disasm(code, &out)`, `Calc.verify(code, lim)`,
field readers such as `Calc.ABC_b(w)`, `Calc.label(w)` and
`Calc.set_label(w, off)` for jumps, `Calc.optimize` (chapter 8) and
`Calc.deopt`. They are ordinary Loimi functions named `Calc.*`.

## Compiling to bytecode: `std.bc`

`std.bc` holds the bookkeeping every bytecode compiler needs: a code
buffer with labels and jump patching, a deduplicating constant pool, and
a register allocator. It is generic over the vm's word type, and takes
two generated functions so it knows how long instructions are and how
to patch a jump:

```loimi
pub fn compile(stmts: *mem.Vec<Stmt>) -> Result<Chunk, str> {
    var c = C {
        code: bc.Code.new(Calc.op_size, Calc.set_label),
        consts: bc.Pool.new(65536),
        regs: bc.Regs.new(250),
        error: None,
    };
```

`bc.Regs` allocates registers as a stack, the way Lua's compiler uses
its "free register": `alloc(n)` takes the next `n`, `mark()` and
`reset(m)` free everything allocated since a mark, `end_stmt()` frees
all temporaries, and `frame_size()` is the most ever used, which becomes
the function's register count. A past-the-limit `alloc` records a
failure instead of returning an error, so the compiler can check once at
the end.

The compiler is one recursive function that puts an expression's value
into a given register:

```loimi
/// Compile `n` so that its value ends up in register `dst`.
fn C.expr_to(c: *C, n: *Node, dst: u32) {
    let d = dst as u8;
    match n.expr {
        Int(v) => {
            if v >= -32768 and v <= 32767 {
                c.code.op(Calc.LOADI(d, v as i16));
            } else {
                let Some(k) = c.consts.add(v) else { c.error = Some("too many constants"); return; };
                c.code.op(Calc.LOADK(d, k as u16));
            }
        }
        Neg(e) => {
            c.expr_to(e, dst);
            c.code.op(Calc.NEG(d, d));
        }
        Binary(op, l, r) => {
            let mark = c.regs.mark();
            let a = c.operand(l);
            let b = c.operand(r);
            c.code.op(arith(op, d, a as u8, b as u8));
            c.regs.reset(mark);
        }
    }
}
```

Small integers fit in LOADI's 16-bit immediate; larger ones go to the
constant pool. `c.code.op(w)` appends an instruction (and zeroes any
inline-cache words after it, chapter 5).

## Disassembler and verifier

`--disasm` prints the code with the generated disassembler. Operands
appear by name, with the `display` format of their kind:

```text
    0  LOADI dst=r1 v=1
    1  LOADI dst=r3 v=2
    2  LOADI dst=r4 v=3
    3  MUL dst=r2 a=r3 b=r4
    4  ADD dst=r0 a=r1 b=r2
    5  PRINT a=r0
```

That is `print 1 + 2 * 3;`. (`dst = r0` and its operands in fresh
temporaries: there is room to be cleverer, and chapter 8 is about where
cleverness pays.)

Before running, `main` verifies the code:

```loimi
    // The verifier proves every operand in range, once, so the handlers
    // can index registers and constants without bounds checks.
    var lim = Limits { nregs: chunk.nregs, nconsts: chunk.consts.len as u32 };
    if Calc.verify(chunk.code, &lim).is_some() {
        io.eprintln("error: the bytecode failed verification");
        return 1;
    }
```

`Calc.verify` returns `None` for valid code, or the index of the first
bad word. It checks that every word has a valid opcode, that every
operand satisfies its kind's `verify` clause, that no instruction is cut
off by the end of the code, that jumps land on instruction boundaries
(chapter 5), and that execution cannot run off the end. Because of
that, `regs[it]` in a handler compiles to a plain load, as you saw in the
assembly: the check moved from run time to load time.

Two things are still up to you, and the compiler says so in
[`status.md`](../status.md): `run` takes a raw code pointer, so nothing
forces you to verify before running; and the verifier knows only what
you give it, so `lim` must describe the register file you actually
allocate. Here `main` allocates exactly `chunk.nregs` registers.

## Running it

```loimi
    let regs: *i64 = mem.alloc(chunk.nregs as u64);
    match Calc.run(chunk.code.ptr, regs, chunk.consts.ptr) {
        Ok(_) => 0,
        Err(msg) => { io.eprint("error: "); io.eprintln(msg); 1 }
    }
```

The output is the same as step 2's, and `tests/` checks that it stays
the same in all three dispatch modes.

Next: [values, slow paths and errors](04-values.md).
