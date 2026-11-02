# 2. A calculator

Before any bytecode, we need a front end. This chapter builds a
calculator that reads `print <expr>;` statements, parses them into a
tree and evaluates the tree directly. Step 3 will compile the same tree
to bytecode, so everything here is reused.

The code is in [`examples/tutorial/step2/`](../../examples/tutorial/step2):
`lexer.lm`, `parser.lm` and `main.lm`.

```sh
./target/release/loimi run examples/tutorial/step2/main.lm -- examples/tutorial/step2/tests/arith.pip
```

```text
print 1 + 2 * 3;      // 7
print -2 * -(3 + 4);  // 14
print 17 / 5;         // 3
```

## The lexer: `std.lex`

Most interpreter lexers are the same 300 lines: skip whitespace and
comments while counting lines, read identifiers and look them up in a
keyword table, read numerals exactly (including the overflow cases),
decode string escapes, match operators longest first. `std.lex` is
those pieces as a toolkit. It is not a generator (Loimi has no
compile-time evaluation yet), so a lexer is a short function that tries
the scanners in order.

The token kinds are a payload-free enum. The lexer already knows every
token of the final language, so later steps do not touch it:

```loimi
pub enum T {
    Eof, Int, Float, Str, Name,
    // keywords
    Let, Fn, If, Else, While, Return, Print, True, False, Nil, Not,
    // punctuation
    Plus, Minus, Star, Slash, Percent, EqEq, NotEq, Lt, Le, Gt, Ge, Assign,
    LParen, RParen, LBrace, RBrace, LBracket, RBracket, Comma, Semi,
}
```

A `lex.Cursor` is configured with a `Syntax`, which holds the
conventions that differ between languages: comment delimiters, escape
rules, what happens to a numeral that overflows. `lex.c_like()` gives
`//` and `/* */` comments. Operators go into an `OpTable`, keywords into
a `Keywords` table:

```loimi
pub fn Lexer.new(src: str, file: u32, arena: *mem.Arena) -> Lexer {
    var lx = Lexer {
        cur: lex.Cursor.new(src, file, lex.c_like()),
        ops: lex.OpTable.new(), keywords: lex.Keywords.new(), buf: text.StrBuf.new(), arena: arena,
    };
    let kws = ["let", "fn", "if", "else", "while", "return", "print", "true", "false", "nil", "not"];
    let kwk = [T.Let, T.Fn, T.If, T.Else, T.While, T.Return, T.Print, T.True, T.False, T.Nil, T.Not];
    for i in 0..kws.len { lx.keywords.add(kws[i], kwk[i]); }
```

The lexer itself:

```loimi
pub fn Lexer.next(lx: *Lexer) -> Result<Token, lex.Error> {
    let c = &lx.cur;
    c.skip_trivia()?;
    let start = c.pos;
    var t = Token { kind: T.Eof, span: c.span_from(start), line: c.line, int: 0, float: 0.0, text: "" };
    if c.at_end() {
        return Ok(t);
    } else if let Some(name) = c.ident() {
        t.kind = lx.keywords.get(name) ?? T.Name;
        t.text = name;
    } else if c.at_number() {
        match c.number()? {
            Int(i) => { t.kind = T.Int; t.int = i; }
            Float(f) => { t.kind = T.Float; t.float = f; }
        }
    } else if c.peek() == '"' {
        lx.buf.clear();
        c.quoted(&lx.buf)?;
        t.kind = T.Str;
        t.text = lx.arena.str(lx.buf.as_str());
    } else if let Some(k) = c.op(&lx.ops) {
        t.kind = k;
    } else {
        c.bump();
        return Err(c.error_at(start, "unexpected character"));
    }
    t.span = c.span_from(start);
    Ok(t)
}
```

`c.number()` parses with `std.num`, which is exact (no `atof`
surprises) and reports `3abc` as a malformed number. `c.quoted` decodes
escapes into a buffer; the result is copied into an arena, because the
buffer is reused for the next string. `x ?? y` is "the option's value,
or `y`".

Errors are `lex.Error`: a static message and a span. `e.diag()` turns
one into a `std.diag` diagnostic, which renders like the Loimi
compiler's own errors:

```text
error: expected an expression
  --> tests/err_syntax.pip:2:11
    2 | print 1 + * 2;
      |           ^
```

## Trees

The tree is plain enums with pointers. Every node carries the line it
starts on, for error messages later.

```loimi
pub enum BinOp { Add, Sub, Mul, Div, Mod }

pub enum Expr {
    Int(i64),
    Neg(*Node),
    Binary(BinOp, *Node, *Node),
}

/// An expression and the line it starts on (for error messages).
pub struct Node { expr: Expr, line: u32 }
```

Nodes are allocated in an arena and freed together when the program has
been compiled. `size_of<Node>()` and a cast are the whole allocator:

```loimi
fn Parser.node(p: *Parser, e: Expr, line: u32) -> *Node {
    let n = p.arena.alloc_bytes(size_of<Node>(), 8) as *Node;
    *n = Node { expr: e, line: line };
    n
}
```

## The parser: `std.parse`

Expression parsing is where hand-written parsers grow a function per
precedence level. `std.parse.Pratt` replaces those with a table of
binding powers and a driver. You supply five hooks as function pointers:
`peek` (the current token's kind), `advance`, `atom` (an operand that
does not start with a prefix operator), `unary` and `binary` (build a
node). The driver is generic over the parser state, the token kind, the
result type and the error type:

```loimi
type Pratt = parse.Pratt<Parser, T, *Node, lex.Error>;
```

`type` declares an alias. The hooks are ordinary functions:

```loimi
fn peek(p: *Parser) -> T { p.tok.kind }

fn advance(p: *Parser) -> Result<(), lex.Error> { p.advance() }
```

```loimi
fn binary(p: *Parser, op: T, l: *Node, r: *Node) -> Result<*Node, lex.Error> {
    let b = match op {
        Plus => BinOp.Add,
        Minus => BinOp.Sub,
        Star => BinOp.Mul,
        Slash => BinOp.Div,
        _ => BinOp.Mod,
    };
    Ok(p.node(Expr.Binary(b, l, r), l.line))
}
```

`atom` handles literals and parentheses. A parenthesized expression
recurses into the driver, which `atom` reaches through a pointer in the
parser state:

```loimi
        LParen => {
            p.advance()?;
            let e = p.expr()?;
            p.expect(T.RParen, "expected ')'")?;
            Ok(e)
        }
```

The operator table is filled at run time. `infix_left(k, bp)` makes `k`
left-associative with binding power `bp` (higher binds tighter);
`prefix(k, bp)` makes it a prefix operator whose operand is parsed at
`bp`:

```loimi
pub fn parse_program(src: str, file: u32, arena: *mem.Arena) -> Result<mem.Vec<Stmt>, lex.Error> {
    var pr: Pratt = parse.Pratt.new(peek, advance, atom, unary, binary);
    pr.infix_left(T.Plus, 6);
    pr.infix_left(T.Minus, 6);
    pr.infix_left(T.Star, 7);
    pr.infix_left(T.Slash, 7);
    pr.infix_left(T.Percent, 7);
    pr.prefix(T.Minus, 8);
    defer pr.free();
```

Later chapters add comparisons (power 3) and `not`. The driver also has
`infix_right` for right-associative operators, `led` hooks for operators
that are not `lhs op rhs` (`?:`, postfix operators, assignment), and a
`before_rhs` hook that single-pass compilers use to put the left operand
in a register before the right one is parsed. Pip does not need them;
`tests/run/std_parse.lm` shows them all.

Statements are parsed by hand, which is the easy part.

## Evaluating the tree

The evaluator is a recursive function over the tree. Errors are values:
a message and a line.

```loimi
fn eval(n: *Node) -> Result<i64, (str, u32)> {
    match n.expr {
        Int(v) => Ok(v),
        Neg(e) => Ok(0 - eval(e)?),
        Binary(op, l, r) => {
            let a = eval(l)?;
            let b = eval(r)?;
            match op {
                Add => Ok(a + b),
                Sub => Ok(a - b),
                Mul => Ok(a * b),
                Div => if b == 0 { Err(("division by zero", n.line)) } else { Ok(a / b) },
                Mod => if b == 0 { Err(("division by zero", n.line)) } else { Ok(a % b) },
            }
        }
    }
}
```

A tree walker like this is a fine interpreter for many purposes. It is
slow for the usual reasons: every node is a pointer chase, every
evaluation is a call with its prologue and epilogue, and the dispatch on
`n.expr` is one hard-to-predict indirect branch shared by every kind of
node. The rest of the tutorial replaces it with bytecode.

Next: [a first vm](03-first-vm.md).
