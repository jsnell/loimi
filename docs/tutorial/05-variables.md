# 5. Variables and control flow

Pip gets variables, `if` and `while`:

```text
let n = 10;
let total = 0;
let i = 1;
while i <= n {
    total = total + i;
    i = i + 1;
}
print total;
```

A `let` at the top level defines a **global**, kept in a hash table and
reached by name. A `let` inside a block defines a **local**, which lives
in a register. (Chapter 6 adds functions, whose parameters and `let`s
are locals too.) Globals are where this chapter meets inline caches.

Code: [`examples/tutorial/step5/`](../../examples/tutorial/step5).

## Locals cost nothing

A local is a register that the compiler has given a name. Reading one
needs no instruction at all: an operand that names a local is just that
register.

```loimi
/// A register holding the value of `n`: a local's own register, or a new
/// temporary.
fn C.operand(c: *C, n: *Node) -> u32 {
    if let Name(name) = n.expr and let Some(r) = c.local(name) { return r; }
    let r = c.regs.alloc(1);
    c.expr_to(n, r);
    r
}
```

`bc.Regs` keeps locals below temporaries: `activate(1)` turns the
register just allocated into a local, and `enter`/`leave` around a block
free the block's locals when it ends.

```loimi
                // The value goes straight into the local's register, which
                // becomes part of the block's frame.
                let r = c.regs.alloc(1);
                c.expr_to(e, r);
                c.regs.end_stmt();
                c.regs.activate(1);
                c.locals.push(Local { name: name, reg: r });
```

## Globals and inline caches

Globals are looked up by name at run time: a program can define them in
any order, and a function (chapter 6) can use a global defined after it.
The table is an open-addressing hash table from the name's symbol (an
interned string's number) to a value:

```loimi
/// One global variable. `key` is the name's symbol plus one; 0 marks an
/// empty slot.
pub struct Slot { key: u32, value: Value }

/// The global variables: an open-addressing hash table. Growing it moves
/// variables to other slots, so a cached slot index has to be checked.
pub struct Globals { slots: *Slot, cap: u64, count: u64 }
```

Hashing on every access is slow. Any given GETGLOBAL instruction almost
always finds its name in the same slot, so it can remember the slot, in
an **inline cache**: a few words stored in the code stream right after
the instruction, as CPython 3.11 does. In Loimi a cache is a declared
layout attached to an op with `+`:

```loimi
    cache GlobalIC { slot: u32 }
```

`op_size` counts the cache words, `next` skips them, the emitters zero
them, the verifier makes sure no jump lands in them, and the
disassembler prints them. Inside the handler, `cache.slot` reads and
writes the word directly.

## Quickening: `family` and `become`

A cache that is filled once and then checked suggests two versions of
the instruction: a generic one that does the lookup and fills the
cache, and a specialized one that only checks it. Rewriting an
instruction's opcode in place when it learns something is called
*quickening*. In Loimi, the versions form a **family**, and `become`
rewrites the current instruction to another member:

```loimi
    family GETGLOBAL(dst: reg, n: gname) : ABx + GlobalIC {
        generic {
            let Some(i) = ctx.globals.find(n as u32 + 1) else { cold { raise undefined(ctx, n) } };
            cache.slot = i as u32;
            dst = ctx.globals.slots[i].value;
            become GETGLOBAL_C;
            next
        }
        GETGLOBAL_C {
            let i = cache.slot as u64;
            if likely(i < ctx.globals.cap) {
                let s = ctx.globals.slots + i;
                if s.key == n as u32 + 1 { dst = s.value; next }
            }
            cold { become GETGLOBAL; redo }
        }
    }
```

All members share the operands, format and cache, so rewriting the
opcode byte is safe: the verifier's guarantees hold for every member.
The compiler only ever emits `GETGLOBAL` (the `generic` member);
`GETGLOBAL_C` is reached through `become`. When the check fails
(because the table grew and the variable moved), the instruction turns
back into GETGLOBAL and `redo` runs it again from the start.

The hit path is a bounds check, a load and a compare. You can watch it
happen. `--disasm` prints the code before running, `--disasm-after`
after:

```text
    4  GETGLOBAL dst=r0 n=g1 [cache slot=0]
    6  LOADI dst=r1 v=3
    7  JNLT a=r0 b=r1
    8  JMP target -> 22
    9  GETGLOBAL dst=r1 n=g0 [cache slot=0]
```

```text
    4  GETGLOBAL_C dst=r0 n=g1 [cache slot=2]
    6  LOADI dst=r1 v=3
    7  JNLT a=r0 b=r1
    8  JMP target -> 22
    9  GETGLOBAL_C dst=r1 n=g0 [cache slot=1]
```

Note the offsets: each GETGLOBAL takes two words, its own and its
cache's.

What a cache can hold matters. A cache field cannot be a `Gc<T>` or a
`Value` holding one (chapter 7): the collector does not scan bytecode.
Cache things whose lifetime you control and check them on use: here a
slot index, validated by the key in the slot. `std.obj` caches shape
ids, which are never reused, for the same reason.

The error path for an undefined variable needs the name, so `undefined`
builds the message in a buffer in the context:

```loimi
/// "undefined variable 'x'"
fn undefined(m: *Machine, name: u64) -> str {
    m.err.clear();
    m.err.push_str("undefined variable '");
    m.err.push_str(m.names.name(name as u32));
    m.err.push_str("'");
    m.err.as_str()
}
```

## Jumps

A jump's target is an operand of kind `pc + it`:

```loimi
    operand label = pc + it verify is_insn_boundary(pc + it);
```

`is_insn_boundary` is available in verify clauses: the verifier walks
the code instruction by instruction and knows where each one starts, so
a jump into the middle of an instruction, or into a cache, is rejected.
`Pip.set_label(w, offset)` re-encodes a jump word's offset (or returns
`None` if it does not fit the field), which is what `bc.Code` uses to
patch forward jumps.

```loimi
    // Branches. JMP jumps; JF jumps if its operand is false or nil.
    op JMP(target: label) : sJ { jump target }
    op JF(a: reg, target: label) : AsBx {
        if !value.truthy(a) { jump target }
        next
    }
```

`while` in the compiler is labels from `bc.Code`: `here()` is a label
placed at the current position, `label()` one placed later with
`bind`, and `jump(w, l)` emits a jump word whose offset is filled in
when `l` is bound:

```loimi
        While(cond, body) => {
            let top = c.code.here();
            let exit = c.code.label();
            c.jump_if_false(cond, exit);
            c.block(&body);
            c.code.jump(Pip.JMP(0), top);
            c.code.bind(exit);
        }
```

An offset that does not fit its field is recorded as an error, and
`c.code.check()` reports it with the source line of the jump.

## Compare and branch

`if i <= n` could compile to `LE t, i, n` (store a boolean) and then
`JF t, else` (load it back and test it). Mini measured the difference
between that and a fused compare-and-branch: 4-13% faster on its
branch-heavy benchmarks
([`bench/mini/RESULTS.md`](../../bench/mini/RESULTS.md), item 16).

A compare op needs two register operands and a jump offset, which do
not fit in one 32-bit word with a useful range. Lua's answer is to
follow the compare with a JMP that holds the offset. Loimi supports this
directly: `then JMP` says the next instruction is a JMP, and `then.target`
reads its operand:

```loimi
    // Compare and branch: these are always followed by a JMP (`then JMP`),
    // which holds the offset. If the comparison holds, skip the JMP and
    // run the code after it; otherwise take it. The jump word stays an
    // ordinary instruction, so std.bc patches it and the verifier checks
    // its target like any other.
    fn branch(holds: bool, target: *u32) -> never {
        if holds { jump pc + 2 }
        jump target
    }
    inline for (name, test, slow) in [("JNLT", x < y, less(a, b, false)), ("JNLE", x <= y, less(a, b, true))] {
        op #name(a: reg, b: reg) : AB then JMP {
            match (a, b) {
                (Int(x), Int(y)) => branch(test, then.target),
                (Num(x), Num(y)) => branch(test, then.target),
                _ => {}
            }
            cold { branch(slow?, then.target) }
        }
    }
```

The generated verifier checks that a JMP really follows each of these.
`branch` is a **vm function**: a helper inside the vm that can read the
state and, because it returns `never`, end in control words. It is
inlined into each caller.

The ops jump when the comparison is *false* ("jump if not less than"),
because that is what `if` and `while` need: fall into the body, or jump
past it. (`a > b` is `JNLT b, a`; `!(a < b)` is not the same as
`a >= b` when a NaN is involved, which is why there is no shortcut.)

```loimi
fn C.jump_if_false(c: *C, cond: *Node, l: bc.Label) {
    let mark = c.regs.mark();
    if let Binary(op, x, y) = cond.expr {
        let w: ?u32 = match op {
            Lt => Some(Pip.JNLT(c.operand(x) as u8, c.operand(y) as u8)),
            Le => Some(Pip.JNLE(c.operand(x) as u8, c.operand(y) as u8)),
            Gt => { let a = c.operand(x); Some(Pip.JNLT(c.operand(y) as u8, a as u8)) }
            Ge => { let a = c.operand(x); Some(Pip.JNLE(c.operand(y) as u8, a as u8)) }
            Eq => Some(Pip.JNEQ(c.operand(x) as u8, c.operand(y) as u8)),
            Ne => Some(Pip.JEQ(c.operand(x) as u8, c.operand(y) as u8)),
            _ => None,
        };
        if let Some(w) = w {
            c.code.set_loc(cond.line);
            c.code.op(w);
            c.code.jump(Pip.JMP(0), l);
            c.regs.reset(mark);
            return;
        }
    }
    let r = c.operand(cond);
    c.code.set_loc(cond.line);
    c.code.jump(Pip.JF(r as u8, 0), l);
    c.regs.reset(mark);
}
```

The verifier also refuses code that could run off its end. After a
`then JMP` op that may skip its JMP (`jump pc + 2`), there must be
another instruction; the compiler always ends with HALT, so there is.

Next: [functions and calls](06-functions.md).
