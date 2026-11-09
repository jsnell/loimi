# 3. A first vm

This chapter compiles the calculator's tree to bytecode and runs it on a
virtual machine declared with Loimi's `vm` block. The values are still
plain `i64`s; chapter 4 makes them dynamically typed.

The code is in [`examples/tutorial/step3/`](../../examples/tutorial/step3):
`vm.lm`, `compile.lm` and `main.lm`, plus the lexer and parser from
step 2. `--disasm` prints the bytecode before running it:

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

Then the problems start:

- **Dispatch predicts badly.** The `switch` compiles to one indirect
  branch that every opcode shares. So you move to GCC's computed goto
  (`goto *labels[op]`) behind a macro, with a `switch` fallback for
  compilers that lack it.
- **State leaves its registers.** The register allocator sees one huge
  function, and saves `pc` and the register base to memory around
  calls. So you add `register` hints or rewrite the hot ops.
- **Encodings drift.** `A(i)`, `B(i)` and `C(i)` are macros that must
  agree with the compiler's `CREATE_ABC` macros and with the
  disassembler, which someone writes later and which slowly falls out
  of date.
- **Untrusted bytecode is slow.** Loading bytecode from a file means
  either trusting it or bounds-checking every operand every time it
  runs.

CPython 3.14 and some newer interpreters use a different shape. Each
opcode is its own function, and the functions are chained with
guaranteed tail calls (`musttail`). A special calling convention
(`preserve_none`) keeps the interpreter state in registers. This is the
fastest portable shape, and the most tedious to write by hand.

In Loimi you declare the parts of the interpreter once, in a `vm`
block. From that, the compiler produces the dispatch loop in whichever
shape you ask for, plus the encoders, the disassembler and a verifier.

## The `vm` block

```loimi
#[require_verified]
pub vm Calc {
    // Live across every instruction, and kept in machine registers.
    state { pc: *u32, regs: *i64, k: *i64 }
```

(`#[require_verified]` is explained in the section on the verifier,
below.)

`state` lists the variables that live across every instruction: here
the program counter, the base of the register file and the constant
table. Inside an op they are ordinary mutable variables.

In the default tail-call mode, the state variables are passed as
parameters to every op's function, so they stay in the same machine
registers for the whole run. Keep the list short, three to six
variables: each one is a register that no op can use for anything
else.

```loimi
    // The generated verifier `Calc.verify(code, lim)` takes this.
    verify_context lim: *Limits;

    // What an encoded field means in a handler, and what the verifier
    // must check about it.
    operand reg = regs[it] verify (it as u32) < lim.nregs display "r{}";
    operand kst = k[it] verify (it as u32) < lim.nconsts display "k{}";
    operand imm = it;
```

An **operand kind** says what a field of an instruction means. In each
declaration:

- `it` is the raw field value: a `u64`, or an `i64` for a signed field.
- The expression after `=` is what the operand means. An operand of
  kind `reg` *is* `regs[it]`: reading it loads the register, and
  assigning to it stores to the register.
- The `verify` clause is a check that the op does not run. It goes into
  the generated verifier, which checks it once per instruction when the
  code is loaded.
- `display` says how the disassembler prints the operand.

```loimi
    // Every instruction is one 32-bit word.
    encoding fixed(u32) {
        format ABC  { op: 8, a: 8, b: 8, c: 8 }
        format ABx  { op: 8, a: 8, bx: 16 }
        format AsBx { op: 8, a: 8, sbx: s16 }
    }
```

**Formats** lay out the fields of a word, as in Lua. `s16` is a signed
field. The encoders, the decoding inside each op, the disassembler and
the verifier are all generated from these formats, so they cannot
disagree.

Then come the **ops**. Each op lists its operands in field order, with
their kinds, and then its format:

```loimi
    op LOADI(dst: reg, v: imm) : AsBx { dst = v; next }
    op LOADK(dst: reg, c: kst) : ABx { dst = c; next }
    op MOVE(dst: reg, src: reg) : ABC { dst = src; next }
    op NEG(dst: reg, a: reg) : ABC { dst = 0 - a; next }
    op ADD(dst: reg, a: reg, b: reg) : ABC { dst = a + b; next }
```

An op's body never finishes normally: every path must end in a
**control word**, which says what happens next.

| Control word | What it does |
|---|---|
| `next` | Moves `pc` past this instruction and runs the next one |
| `jump t` | Sets `pc` to `t` and runs the instruction there |
| `exit v` | Leaves the interpreter and returns `v` |
| `fail e` | Leaves the interpreter and returns `Err(e)` (the entry must return a `Result`) |

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

Finally comes the **entry**, the way into the interpreter. It sets up
the state and runs the first instruction:

```loimi
    // The way in: set up the state, then dispatch the first instruction.
    entry run(code: Verified<Calc>, r: *i64, consts: *i64) -> Result<(), str> {
        regs = r;
        k = consts;
        jump code
    }
```

The compiler turns this into a function `Calc.run(code, regs, consts)`
that the rest of the program calls like any other. `code` is not a
plain pointer but a `Verified<Calc>`: code that has passed the verifier
(see below). `jump code` sets `pc` to its first word.

## What it compiles to

Build with `loimi build --release` and disassemble with `objdump -d`.
Each op is its own symbol, such as `vm.Calc.op.ADD`, which also makes
profiles easy to read. Here is ADD:

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

What to notice:

- It is thirteen instructions, with no stack frame and nothing saved to
  memory.
- `pc`, `regs` and the instruction word arrive in registers and leave
  in the same registers.
- Each op ends with its own indirect jump to the next op. So the
  branch predictor learns, for each op, which op usually follows it.

This is `--dispatch=tailcall`, the default on x86-64 and AArch64. Each
op is an LLVM function using the `preserve_none` calling convention,
and `next` is a `musttail` call through the dispatch table.

There are two other dispatch modes, built from the same source:

- `--dispatch=goto`: one function, using computed goto.
- `--dispatch=switch`: a loop around a `switch`.

`examples/tutorial/check.sh` runs every step's tests in all three
modes. Chapter 8 compares their speed.

## Generated tools

From the declarations, the compiler generates ordinary Loimi functions
named `Calc.*`. For each op, using ADD as the example:

| Function | What it does |
|---|---|
| `Calc.ADD(dst, a, b)` | Encodes the instruction and returns the word. Parameter types come from the format: `u8` for an 8-bit field, `i16` for `s16` |
| `Calc.emit.ADD(code, at, dst, a, b)` | Stores the encoded word into a slice and returns the next offset |
| `Calc.OP_ADD` | The opcode, as a constant |

For the whole vm:

| Function | What it does |
|---|---|
| `Calc.disasm(code, &out)` | Disassembles code into a string buffer |
| `Calc.verify(code, lim)` | The verifier (see below) |
| `Calc.op_name(w)`, `Calc.op_size(w)`, `Calc.NUM_OPS` | An instruction's op name and size, and the number of ops |
| `Calc.ABC_b(w)` | Reads a field of a format |
| `Calc.set_ABC_a(w, r)` | Sets a field; returns `None` if `r` does not fit |
| `Calc.encode.ABC(Calc.OP_ADD, d, a, b)` | Encodes by format, for code that picks the opcode at run time |
| `Calc.label(w)`, `Calc.set_label(w, off)` | Read and set a jump's target |
| `Calc.optimize`, `Calc.deopt` | Rewrite code to use faster ops (chapter 8), and turn it back into generic ops |

## Compiling to bytecode: `std.bc`

`std.bc` has the bookkeeping every bytecode compiler needs:

- a code buffer with labels and jump patching (`bc.Code`)
- a constant pool that removes duplicates (`bc.Pool`)
- a register allocator (`bc.Regs`)

It is generic over the vm's word type. The code buffer takes two
generated functions, so that it knows how long instructions are and how
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

`bc.Regs` hands out registers like a stack, as Lua's compiler does:

| Method | What it does |
|---|---|
| `alloc(n)` | Takes the next `n` registers |
| `mark()`, `reset(m)` | Frees every register taken since the mark |
| `end_stmt()` | Frees all temporaries |
| `frame_size()` | The most registers ever in use: the function's register count |

If `alloc` goes past the limit, it records the failure instead of
returning an error, so the compiler can check once at the end.

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

Small integers fit in LOADI's 16-bit field. Larger ones go to the
constant pool. `c.code.op(w)` appends an instruction.

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

That is `print 1 + 2 * 3;`. The compiler is simple: it puts every
operand in a fresh temporary register. It could be cleverer, and
chapter 8 is about where that pays off.

Before running, `main` verifies the code:

```loimi
    // The verifier proves every operand in range, once, so the handlers
    // can index registers and constants without bounds checks.
    var lim = Limits { nregs: chunk.nregs, nconsts: chunk.consts.len as u32 };
    let code = match Calc.verify(chunk.code, &lim) {
        Ok(v) => v,
        Err(e) => {
            var b = text.StrBuf.new();
            b.push_str("error: the bytecode failed verification: ");
            e.write(&b);
            io.eprintln(b.as_str());
            return 1;
        }
    };
```

The verifier checks that:

- every word has a valid opcode
- every operand passes its kind's `verify` clause
- no instruction is cut off by the end of the code
- jumps land on the start of an instruction
- execution cannot run off the end of the code

Because of this, `regs[it]` in an op compiles to a plain load, as in
the assembly above. The check has moved from run time to load time.

`Calc.verify` returns a `Result<Verified<Calc>, Calc.VerifyError>`. The
error has the index of the first bad word (`e.at`) and the reason as an
enum (`e.reason`). `e.write` describes it, for example:

```text
word 0 (LOADK): operand `c` (kst) fails `(it as u32) < lim.nconsts`
```

On success you get a `Verified<Calc>`, which wraps the code slice.
`v.start()`, `v.len()` and `v.words()` read it. Only the verifier can
make one, and `run` accepts nothing else, so code that was not verified
cannot run.

`#[require_verified]` on the vm makes the compiler enforce this: every
entry must start with a `jump` to a `Verified` value, and a `jump` to a
raw pointer in an entry is an error.

Two things are still up to you:

- The verifier knows only what you tell it. `lim` must describe the
  register file you actually allocate. Here `main` allocates exactly
  `chunk.nregs` registers.
- A host that trusts code without checking it can say so with
  `unsafe { Calc.assume_verified(words) }`.

## Running it

```loimi
    let regs: *i64 = mem.alloc(chunk.nregs as u64);
    match Calc.run(code, regs, chunk.consts.ptr) {
        Ok(_) => 0,
        Err(msg) => { io.eprint("error: "); io.eprintln(msg); 1 }
    }
```

The output is the same as step 2's. The tests in `tests/` check that it
stays the same in all three dispatch modes.

Next: [values, slow paths and errors](04-values.md).
