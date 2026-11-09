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

Pip has two kinds of variable:

- A `let` at the top level defines a **global**. Globals are kept in a
  hash table and looked up by name.
- A `let` inside a block defines a **local**, which lives in a
  register. (In chapter 6, function parameters become locals too.)

Looking up globals quickly is where inline caches come in.

The code is in [`examples/tutorial/step5/`](../../examples/tutorial/step5).

## Locals cost nothing

A local is a register that the compiler has given a name. Reading a
local needs no instruction at all: when an operand is a local, the
compiler uses that local's register.

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

`bc.Regs` keeps locals below temporaries:

- `activate(1)` turns the register just allocated into a local.
- `enter` and `leave`, called around a block, free the block's locals
  when it ends.

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

Globals are looked up by name at run time. That way a program can
define them in any order, and a function can use a global defined after
it.

The globals table is a hash table with open addressing. It maps a
name's symbol (the number of an interned string) to a value:

```loimi
/// One global variable. `key` is the name's symbol plus one; 0 marks an
/// empty slot.
pub struct Slot { key: u32, value: Value }

/// The global variables: an open-addressing hash table. Growing it moves
/// variables to other slots, so a cached slot index has to be checked.
pub struct Globals { slots: *Slot, cap: u64, count: u64 }
```

Hashing on every access is slow. But each GETGLOBAL instruction almost
always finds its name in the same slot, so it can remember the slot.
It keeps it in an **inline cache**: a few words stored in the code
right after the instruction, as in CPython 3.11.

In Loimi you declare the layout of a cache, and attach it to an op with
`+` (as in `ABx + GlobalIC`, below):

```loimi
    cache GlobalIC { slot: u32 }
```

The compiler takes care of the cache words everywhere:

- `op_size` counts them, and `next` skips them.
- The emitters set them to zero.
- The verifier makes sure no jump lands in them.
- The disassembler prints them.

Inside the op, `cache.slot` reads and writes the word directly.

## Quickening: `family` and `become`

A cache that is filled once and then checked suggests two versions of
the instruction:

- a generic one that does the lookup and fills the cache
- a specialized one that only checks the cache

Rewriting an instruction's opcode in place, when it learns something,
is called *quickening*. In Loimi, the versions form a **family**, and
`become` rewrites the current instruction to another member:

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

All members share the operands, format and cache. So rewriting the
opcode byte is safe: what the verifier checked holds for every member.

The compiler only ever emits `GETGLOBAL`, the `generic` member.
`GETGLOBAL_C` is reached only through `become`. If its check fails,
because the table grew and the variable moved, the instruction turns
back into GETGLOBAL, and `redo` runs it again from the start.

When the cache hits, the cost is a bounds check, a load and a compare.

You can watch the rewriting happen: `--disasm` prints the code before
running, and `--disasm-after` prints it after. Before:

```text
    4  GETGLOBAL dst=r0 n=g1 [cache slot=0]
    6  LOADI dst=r1 v=3
    7  JNLT a=r0 b=r1
    8  JMP target -> 22
    9  GETGLOBAL dst=r1 n=g0 [cache slot=0]
```

After:

```text
    4  GETGLOBAL_C dst=r0 n=g1 [cache slot=2]
    6  LOADI dst=r1 v=3
    7  JNLT a=r0 b=r1
    8  JMP target -> 22
    9  GETGLOBAL_C dst=r1 n=g0 [cache slot=1]
```

Note the offsets: each GETGLOBAL takes two words, its own and its
cache's.

Be careful what you put in a cache. Cache only things whose lifetime
you control, and check them when you use them. Here that is a slot
index, checked against the key stored in the slot. (Chapter 7 adds
garbage-collected objects. A cache can't hold a pointer to one, because
the collector does not scan bytecode.)

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

`is_insn_boundary` can be used in verify clauses. The verifier walks the
code one instruction at a time and knows where each one starts, so it
rejects a jump into the middle of an instruction or into a cache.

`Pip.set_label(w, offset)` changes the offset in a jump word, or returns
`None` if the offset doesn't fit. `bc.Code` uses it to fill in forward
jumps.

```loimi
    // Branches. JMP jumps; JF jumps if its operand is false or nil.
    op JMP(target: label) : sJ { jump target }
    op JF(a: reg, target: label) : AsBx {
        if !value.truthy(a) { jump target }
        next
    }
```

The compiler builds `while` from labels in `bc.Code`:

| Method | What it does |
|---|---|
| `here()` | Makes a label at the current position |
| `label()` | Makes a label to be placed later, with `bind` |
| `jump(w, l)` | Emits the jump word `w`, with its offset filled in once `l` is placed |

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

If an offset doesn't fit its field, `bc.Code` records an error.
`c.code.check()` reports it with the source line of the jump.

## Compare and branch

`if i <= n` could compile to two instructions: `LE t, i, n` stores a
boolean, and `JF t, else` loads it back and tests it. A single
compare-and-branch instruction is faster. In Mini it made branch-heavy
benchmarks 4–13% faster.

But a compare-and-branch needs two registers and a jump offset, which
don't fit in one 32-bit word with a useful range. Lua's answer is to
follow the compare with a JMP that holds the offset. Loimi supports
this directly:

- `then JMP` on an op says that the next instruction is always a JMP.
- `then.target` reads the JMP's operand.

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
state. Because it returns `never`, it can end in control words. It is
inlined into each op that calls it.

The ops jump when the comparison is *false* ("jump if not less than"),
because that is what `if` and `while` need: fall into the body, or jump
past it. `a > b` compiles to `JNLT b, a`. There is no shortcut that
turns "not less than" into "greater or equal", because `!(a < b)` and
`a >= b` differ when one of them is a NaN.

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

The verifier also rejects code that could run off its end. A `then JMP`
op may skip its JMP (`jump pc + 2`), so another instruction must follow
the JMP. The compiler always ends the code with HALT, so one does.

Next: [functions and calls](06-functions.md).
