# 6. Functions and calls

```text
fn fib(n) {
    if n < 2 { return n; }
    return fib(n - 1) + fib(n - 2);
}
print fib(20);
```

Functions are values: `fn fib` compiles to a **prototype** and stores
it in the global `fib`. Calls stay inside the interpreter loop: a call
pushes a frame and jumps to the callee's first instruction, and a return
pops it and jumps back. Nothing recurses on the C stack, so deep guest
recursion is limited by the vm's own stacks, which report overflow as an
ordinary error.

Code: [`examples/tutorial/step6/`](../../examples/tutorial/step6).

## Prototypes and frames

Each function compiles to its own code, constants and register count:

```loimi
/// A compiled function. Prototypes live in ordinary memory and are never
/// freed: a program's functions live as long as the program.
pub struct Proto {
    name: str,
    nparams: u32,
    /// The size of the function's register window.
    nregs: u32,
    code: []u32,
    /// The source line of each code word.
    lines: []u32,
    consts: []Value,
}
```

A function value is a pointer to its prototype. A nanbox payload can be
any pointer:

```loimi
pub value Value repr nanbox {
    Num(f64) unboxed,
    Int(i32),
    Nil,
    Bool(bool),
    Fn(*Proto),
}
```

The machine gets a value stack, which holds every active call's
registers, and a frame stack:

```loimi
/// One activation record. `ret` is where the caller continues when this
/// frame's callee returns.
pub struct Frame { proto: *Proto, base: *Value, ret: *u32 }
```

and the current frame becomes part of the vm state, next to `pc`,
`regs` and `k`:

```loimi
    state { pc: *u32, regs: *Value, k: *Value, fp: *Frame }
```

## Register windows

Like Lua, Pip passes arguments by placing them in consecutive registers
of the caller and making those the first registers of the callee. To
call `f(x, y)` the compiler puts `f` in some register `base`, `x` in
`base+1`, `y` in `base+2`, and emits `CALL base, 2`. The callee's
register window starts at `base+1`, so its parameters are already in
`r0` and `r1`: no copying. Its result goes to `regs[-1]` of its window,
which is the caller's `base`.

```loimi
    /// R[base] = R[base](R[base+1], ..., R[base+n]). The callee's
    /// registers start at R[base+1], so the arguments are already its
    /// first registers, and the result goes just below its window.
    op CALL(base: reg, n: nargs) : ABC {
        let f = base;
        if let Fn(p) = f {
            if n as u32 != p.nparams { cold { raise "wrong number of arguments" } }
            let callee = regs + base.raw + 1;
            if callee + p.nregs as u64 > ctx.stack_end or fp + 1 == ctx.frames_end {
                cold { raise "stack overflow" }
            }
            fp.ret = pc + 1;
            fp = fp + 1;
            fp.proto = p;
            fp.base = callee;
            regs = callee;
            k = p.consts.ptr;
            jump p.code.ptr
        }
        cold { raise "call of a non-function" }
    }
```

`base` is a `reg` operand, so reading it gives the register's value;
`base.raw` is the field itself, the register number. The handler
assigns all four state variables and jumps: the callee runs with its
own `regs` and `k` in the same machine registers.

```loimi
    /// Return R[src] to the caller.
    op RET(src: reg) : ABC {
        let v = src;
        if fp == ctx.frames { exit Ok(()) }
        *(regs - 1) = v;
        fp = fp - 1;
        regs = fp.base;
        k = fp.proto.consts.ptr;
        jump fp.ret
    }
```

The `fp == ctx.frames` test is never true for code from Pip's compiler,
which does not allow `return` at the top level. It is there because the
verifier cannot know that, and chapter 9's fuzzer will happily try.

`on raise` now finds the line through the current frame's prototype,
so an error inside a function reports the line inside the function:

```loimi
    on raise(msg: str) {
        let p = fp.proto;
        let i = ((pc as u64) - (p.code.ptr as u64)) / 4;
        ctx.error_line = p.lines[i];
        fail msg
    }
```

## Verifying a function

The verifier now checks one prototype at a time, against that
prototype. A vm can have several verify contexts; they become
parameters of `Pip.verify` in order:

```loimi
    // The verifier checks one function at a time, against its prototype.
    verify_context proto: *Proto, nnames: u32;

    operand reg = regs[it] verify (it as u32) < proto.nregs display "r{}";
    operand kst = k[it] verify it < proto.consts.len display "k{}";
```

CALL's argument count needs more than a range: `base + n` must be in
the frame too. A verify clause can read the instruction word through
`pc`, and the generated field readers decode it:

```loimi
    /// CALL's argument count: R[base+1 .. base+n] must be in the frame.
    operand nargs = it verify Pip.ABC_a(*pc) + it < proto.nregs as u64;
```

`nargs` decodes to the plain field in the handler, so the clause costs
nothing at run time. Mini found nine holes like this with its fuzzer
(counts whose range was never checked); each became an operand kind
with a clause of this shape.

What the verifier cannot check is up to the embedder: that the callee
of a CALL was verified, that `nparams <= nregs`, that the stack has room
for the main function's frame. `main` verifies every prototype before
running any of them:

```loimi
    // Verify every function before running anything.
    for i in 0..prog.protos.len {
        let p = prog.protos.at(i);
        if let Some(bad) = Pip.verify(p.code, p, names.names.len as u32) {
            error("bytecode failed verification", p.lines[bad]);
            return 1;
        }
    }
```

## Compiling calls

```loimi
        Call(f, args) => {
            // The function and its arguments go to consecutive registers
            // at the top of the frame; the result comes back in the first.
            let mark = c.regs.mark();
            let base = c.regs.alloc(1 + args.len as u32);
            c.expr_to(f, base);
            for i in 0..args.len { c.expr_to(args.at(i), base + 1 + i as u32); }
            c.code.set_loc(n.line);
            c.code.op(Pip.CALL(base as u8, args.len as u8));
            if base != dst { c.code.op(Pip.MOVE(d, base as u8)); }
            c.regs.reset(mark);
        }
```

The base is allocated at the top of the caller's registers, so
everything above it is a dead temporary that the callee may overwrite.

A function body compiles with its own `bc.Code`, pool and registers.
Parameters are the first locals; falling off the end returns `nil`:

```loimi
fn function(unit: *Unit, f: *FnDecl) -> *Proto {
    var c = new_c(unit, true);
    c.code.set_loc(f.line);
    for i in 0..f.params.len { c.add_local(f.params.at(i)); }
    for i in 0..f.body.len { c.stmt(f.body.at(i)); }
    // Falling off the end returns nil.
    let r = c.regs.alloc(1);
    c.code.op(Pip.LOADNIL(r as u8));
    c.code.op(Pip.RET(r as u8));
    c.finish(f.name, f.params.len as u32, f.line)
}
```

Here is `fn add(x, y) { return x + y; } print add(1, 2);`:

```text
fn add:
    0  ADD dst=r2 a=r0 b=r1
    1  RET src=r2
    2  LOADNIL dst=r2
    3  RET src=r2
fn main:
    0  LOADK dst=r0 c=k0
    1  DEFGLOBAL src=r0 n=g0
    2  GETGLOBAL dst=r1 n=g0 [cache slot=0]
    4  LOADI dst=r2 v=1
    5  LOADI dst=r3 v=2
    6  CALL base=r1 n=2
    7  MOVE dst=r0 src=r1
    8  PRINT a=r0
    9  HALT
```

The `MOVE` after the call is avoidable; chapter 8 removes it.

The inline caches from chapter 5 now earn their keep: `fib` reads the
global `fib` twice per call. `tests/recache.pip` defines globals after a
function has cached a slot, so that the table grows under the cache and
a cached slot may no longer hold the name; GETGLOBAL_C then falls back
to GETGLOBAL, which refills the cache.

Next: [heap objects and the GC](07-gc.md).
