# 4. Values, slow paths and errors

Pip is dynamically typed. A register can hold an integer, a float, a
boolean or `nil`, and `+` has to look at both operands to know what to
do. This chapter adds:

- a value type
- a fast path and a slow path in each op
- run-time errors that report the line they happened on

The code is in [`examples/tutorial/step4/`](../../examples/tutorial/step4).
The vm is now called `Pip`, and `value.lm` is new.

```text
print 7 / 2;           // 3
print 7.0 / 2;         // 3.5
print 2147483647 + 1;  // 2147483648.0
print 1 == 1.0;        // true
print 2 + 3 * true;    // error: line 1: arithmetic on a boolean value
```

## A NaN-boxed value

C interpreters represent values in one of a few ways:

- **A tagged struct**: a tag word and a payload word, like `TValue` in
  Lua.
- **NaN-boxing**: doubles are stored as themselves, and everything else
  is hidden in the bit patterns of NaNs. LuaJIT and JavaScriptCore do
  this.
- **Pointer tagging**: small integers are marked by a set low bit, as
  in OCaml and many Schemes.

Each comes with a page of macros that build, test and unpack values,
and each macro is a chance to get the bit layout wrong. Changing the
representation later means rewriting all of them.

In Loimi the representation is a declaration:

```loimi
pub value Value repr nanbox {
    Num(f64) unboxed,
    Int(i32),
    Nil,
    Bool(bool),
}
```

A `Value` is one 64-bit word. `Num` is `unboxed`: a double is stored as
itself. The other variants are stored as NaN bit patterns, each with
its own tag. The compiler picks the tags. You get:

- constructors: `Int(3)`, `Value.Nil`
- patterns for `match`
- `==`, which compares the bits
- a cast to `u64`, if you need the raw word

To see the tags the compiler picked, run `loimi build
--explain-values`. It prints each value type's layout and stops:

```text
value value.Value repr nanbox: classic layout, 3 tagged variants
  Num(f64)    unboxed: the words below 0xFFF9_0000_0000_0000 (`limit`)
  Int(i32)    block 48   mask 0xFFFF_0000_0000_0000  tag 0xFFF9_0000_0000_0000  ...
  Nil         block 48   mask 0xFFFF_0000_0000_0000  tag 0xFFFA_0000_0000_0000  ...
  Bool(bool)  block 48   mask 0xFFFF_0000_0000_0000  tag 0xFFFB_0000_0000_0000  ...
  tuple patterns: elements whose variants have the same block width merge into one test, ...
    block 48: Int, Nil, Bool
    unboxed:  Num  (umax(a, b, ..) < limit)
```

With more than seven tagged variants, the compiler packs some of them
into a narrower range of bits, which changes the layout. It warns
(`value_layout`) when adding a variant would do that. Pip never gets
that far: it ends with six.

You get two things for free that hand-written NaN-boxing often gets
wrong:

- Making a `Num` turns any NaN into the one standard NaN. So a guest
  program can never compute a float whose bits look like a tagged
  value (from chapter 7 on, a pointer).
- The compiler skips that step where it can prove it isn't needed. The
  sum of two unboxed doubles is already safe, as the assembly below
  shows.

There are two other representations:

- `repr lowtag`: pointer tagging, with 61-bit integers
- `repr boxed`: two words

Switching is a one-word change, so you can measure instead of guessing.

Why `Int(i32)` and not a 48-bit integer? Overflow-checked arithmetic on
a 48-bit payload costs two extra shifts per operation. In Mini, the
Lua-like example, that made integer benchmarks 3–9% slower. LuaJIT's
dual-number mode uses 32 bits for the same reason.

## Handlers with a fast path

`ADD` must handle two integers, two floats, mixed operands, overflow
and type errors. Only the first two cases matter for speed:

```loimi
        op #name(dst: reg, a: reg, b: reg) : ABC {
            match (a, b) {
                (Int(x), Int(y)) => if let Some(r) = x.#checked(y) { dst = Int(r); next },
                (Num(x), Num(y)) => { dst = Num(x sym y); next }
                _ => {}
            }
            cold { dst = arith(kind, a, b)?; next }
        }
```

(The `#name` and `sym` come from an `inline for`, below.)

The compiler knows the bit layout when it compiles `match (a, b)`, so
it can test both values at once:

- "Both are `Int`" is a single test: xor each word with the `Int` tag,
  or the results together, and check that the top 16 bits are zero.
- "Both are `Num`" is `umax(a, b) < first tag`.

Here is the release build of `ADD`:

```text
<vm.Pip.op.ADD>:
  mov    %edi,%eax                  ; edi: the instruction word
  movabs $0xfff9000000000000,%rdx   ; the Int tag
  movzbl %ah,%ecx
  mov    %edi,%esi
  shr    $0xd,%esi
  and    $0x7f8,%esi
  mov    (%r14,%rsi,1),%rdi         ; a (r14: regs)
  mov    %eax,%esi
  shr    $0x18,%esi
  mov    (%r14,%rsi,8),%rsi         ; b
  mov    %rdi,%r8
  xor    %rdx,%r8
  mov    %rsi,%r9
  xor    %rdx,%r9
  or     %r8,%r9
  shr    $0x30,%r9                  ; (Int, Int)?
  jne    float_path
  add    %edi,%esi                  ; 32-bit add
  jo     slow                       ; overflow
  mov    %esi,%eax
  or     %rdx,%rax                  ; re-tag
  mov    %rax,(%r14,%rcx,8)
  mov    0x4(%r13),%edi             ; next
  add    $0x4,%r13
  movzbl %dil,%eax
  jmp    *0x40c4d0(,%rax,8)
float_path:
  cmp    %rsi,%rdi
  mov    %rsi,%r8
  cmova  %rdi,%r8
  cmp    %rdx,%r8                   ; (Num, Num)?
  jae    slow
  movq   %rdi,%xmm0
  movq   %rsi,%xmm1
  addsd  %xmm0,%xmm1
  movsd  %xmm1,(%r14,%rcx,8)        ; no NaN check needed
  ...                               ; next
slow:
  mov    %eax,%edi
  jmp    vm.Pip.op.ADD.cold0
```

## `cold` blocks

The interesting part of that assembly is what is *not* there. The slow
path calls `arith`, a real function. In C, a call anywhere in a handler
makes the compiler assume the call may overwrite registers. So it saves
the interpreter's state to the stack on entry and reloads it on every
exit, on the fast path too. C programmers fight this with `UNLIKELY()`,
manual outlining and `noinline`, and check the assembly after every
change.

A `cold { }` block marks the slow path. In tail-call mode, when the
block ends in a control word, the compiler moves it into a separate
function, and the op reaches it with a tail call
(`jmp vm.Pip.op.ADD.cold0` above). As a result:

- The fast path makes no call, needs no stack frame and saves nothing
  to memory.
- The cold function decodes the operands it needs again, from the
  instruction word, which is passed to it in a register.

In goto and switch mode, cold blocks stay in place and are marked as
unlikely.

The slow path itself is ordinary code:

```loimi
/// Everything the handlers' fast paths leave out: integer overflow,
/// mixed integer and float operands, division, and type errors.
fn arith(op: Arith, a: Value, b: Value) -> Result<Value, str> {
    if let (Int(x), Int(y)) = (a, b) {
        let (x, y) = (x as i64, y as i64);
        let r = match op {
            Add => x + y,
            Sub => x - y,
            Mul => x * y,
            Div => { if y == 0 { return Err("division by zero"); } x / y }
            Mod => { if y == 0 { return Err("division by zero"); } x % y }
        };
        // A result that does not fit an Int becomes a float.
        if r >= -2147483648 and r <= 2147483647 { return Ok(Value.Int(r as i32)); }
        return Ok(Value.Num(r as f64));
    }
```

Keep the hot path to the one or two cases that matter, and put
everything else in the cold block. In Mini, adding the mixed int and
float cases to the hot part of the arithmetic ops made one benchmark
18% slower. Moving them into the cold block fixed it.

## Generating ops with `inline for`

ADD, SUB and MUL differ in only three tokens. `inline for` copies a
template once per element, at compile time, filling in the tokens:

```loimi
    // Three ops from one template: `#name` makes an identifier, and `sym`
    // is pasted as a token.
    inline for (name, checked, sym, kind) in [
        ("ADD", "add_checked", +, Arith.Add),
        ("SUB", "sub_checked", -, Arith.Sub),
        ("MUL", "mul_checked", *, Arith.Mul),
    ] {
```

`#name` turns the string `"ADD"` into the identifier `ADD`, so the
template declares `op ADD`, `op SUB` and `op MUL`. In the same way,
`x.#checked(y)` becomes `x.add_checked(y)`.

An element can be any sequence of tokens, such as an operator or a
whole expression:

```loimi
    inline for (name, test, or_equal) in [("LT", x < y, false), ("LE", x <= y, true)] {
        op #name(dst: reg, a: reg, b: reg) : ABC {
            match (a, b) {
                (Int(x), Int(y)) => { dst = Bool(test); next }
                (Num(x), Num(y)) => { dst = Bool(test); next }
                _ => {}
            }
            cold { dst = Bool(less(a, b, or_equal)?); next }
        }
    }
```

This is what X-macros do in C, but names work as usual, and errors
point into the template.

## Guards and falling out of a `match`

DIV can take the integer fast path only when the divisor is positive:
dividing by zero is an error, and `MIN / -1` overflows. Here is the
right way to write that:

```loimi
    op DIV(dst: reg, a: reg, b: reg) : ABC {
        match (a, b) {
            (Int(x), Int(y)) if y > 0 => { dst = Int(x / y); next }
            (Num(x), Num(y)) => { dst = Num(x / y); next }
            _ => {}
        }
        cold { dst = arith(Arith.Div, a, b)?; next }
    }
```

There are two ways to write this that look alike, but behave
differently:

- **A guard**, `pattern if cond =>`, as above. If `cond` is false, the
  `match` moves on to the next arm.
- **An `if` in the arm's body**, `pattern => if cond { ...; next }`. If
  `cond` is false, the program leaves the whole `match` and skips the
  later arms. In an op, that usually means skipping a fast path by
  accident.

The compiler warns (`match_fallout`) when an arm's body is an `if`
without `else` that ends in a control word, and a later arm could match
the same values. ADD's `(Int, Int)` arm above is fine, because no later
arm can match two integers.

## Errors: `raise` and `on raise`

A run-time error must say where it happened, so the error code needs
`pc`. In a C interpreter, `pc` lives in a local variable (a register)
for speed. So every place that might raise an error has to store `pc`
to memory first. Lua's `savepc` and `Protect()` macros exist for this,
and forgetting one reports errors at the wrong line.

In a Loimi op, `raise e` jumps to the vm's `on raise` handler, taking
the current state with it. So does `?` on an `Err` of the vm's error
type. `on raise` sees `pc` as it was in the op that failed, without
anything having been saved:

```loimi
    // `raise msg` (and `?` on an Err) in any handler lands here, with the
    // state as it was in that handler: `pc` still points at the
    // instruction that failed.
    on raise(msg: str) {
        let i = ((pc as u64) - (ctx.code as u64)) / 4;
        ctx.error_line = ctx.lines[i];
        fail msg
    }
```

The vm's error type is the type of `on raise`'s parameter, here `str`.
In ADD, `cold { dst = arith(kind, a, b)?; next }` raises whatever
`arith` returned in its `Err`.

A real interpreter would look for an exception handler here and `jump`
to it. Pip just stops.

The line number comes from a table. `bc.Code` records a source position
for each instruction: the compiler calls `c.code.set_loc(line)` as it
goes, and `locs_into` expands the positions to one per word when the
code is finished. For interpreters that keep many functions around,
`encode_locs` makes a compact table instead, about two bytes per line
change.

## The context

`ctx.code`, `ctx.lines` and `ctx.error_line` live in the vm's
**context**. The context is a pointer to everything the ops need that
isn't used often enough to deserve a register of its own:

```loimi
pub vm Pip {
    context ctx: *Machine;
    state { pc: *u32, regs: *Value, k: *Value }
    verify_context lim: *Limits;
```

The context takes one more register, and reading a field from it costs
a load. The entry sets it, like the state:

```loimi
    entry run(m: *Machine, code: Verified<Pip>, r: *Value, consts: *Value) -> Result<(), str> {
        ctx = m;
        regs = r;
        k = consts;
        jump code
    }
```

`ctx.code` points to the first word of the code (`main` sets it to
`code.start()`). `on raise` uses it to turn `pc` into an index into the
line table.

The error line is returned in `ctx` instead of in the `Err`, which keeps
`str` as the error type everywhere. The entry could instead return a
`Result` of a struct holding a message and a line: an entry may return
any type.

## Printing values

`value.write` formats a value into a `StrBuf`. Two functions from
`std.num` handle floats:

- `write_shortest` prints the shortest form that reads back as the
  same float.
- `mark_float` adds `.0` to floats that look like integers, as Lua and
  Python do.

```loimi
pub fn write(b: *text.StrBuf, v: Value) {
    match v {
        Int(i) => b.push_int(i as i64),
        Num(x) => {
            let start = b.len();
            num.write_shortest(b, x);
            num.mark_float(b, start);
        }
        Bool(t) => b.push_str(if t { "true" } else { "false" }),
        Nil => b.push_str("nil"),
    }
}
```

## Constants

Float constants and large integers go in the constant pool, which is
now a `bc.Pool<Value>`. The pool finds duplicates with a hash map, so
`Value` needs the `hash.Hash` trait. In Loimi, an `impl` contains no
code. It states that the trait's methods exist:

```loimi
pub fn Value.hash(v: Value) -> u64 { hash.mix(v as u64) }
pub fn Value.eq(a: Value, b: Value) -> bool { a == b }
impl hash.Hash for Value;
```

Hashing and comparing the bits keeps `0`, `0.0` and `-0.0` apart. A
pool that compared numbers by value would merge them.

Next: [variables and control flow](05-variables.md).
