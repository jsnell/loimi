# 2. A calculator

Before any bytecode, we need a front end. This chapter builds a
calculator that reads `print <expr>;` statements, parses them into a
tree, and evaluates the tree directly. Chapter 3 compiles the same tree
to bytecode, so all of this code is reused.

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

Most interpreter lexers are the same 300 lines of code. They:

- skip whitespace and comments, counting lines
- read identifiers and look them up in a keyword table
- read numbers exactly, including numbers that overflow
- decode escapes in strings
- match operators, longest first

`std.lex` provides each of these pieces. It is a toolkit, not a lexer
generator, so your lexer is a short function that tries each piece in
turn.

The token kinds are an enum without payloads. The lexer already knows
every token of the final language, so later steps don't change it:

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

A `lex.Cursor` reads the source. It takes a `Syntax`, which holds the
rules that differ between languages, such as the comment markers, the
string escapes and what to do with a number that overflows.
`lex.c_like()` gives `//` and `/* */` comments.

Operators go into an `OpTable`, and keywords into a `Keywords` table:

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

Some details:

- `c.number()` parses with `std.num`, which is exact (no `atof`
  surprises) and reports `3abc` as a malformed number.
- `c.quoted` decodes escapes into a buffer. The result is copied into
  an arena, because the buffer is reused for the next string.
- `x ?? y` means "the option's value, or `y` if it is `None`".

Errors are `lex.Error` values: a fixed message and a span. `e.diag()`
turns one into a `std.diag` diagnostic, which prints like the Loimi
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

Nodes are allocated in an arena, and all freed together once the
program has been compiled. `size_of<Node>()` and a cast are the whole
allocator:

```loimi
fn Parser.node(p: *Parser, e: Expr, line: u32) -> *Node {
    let n = p.arena.alloc_bytes(size_of<Node>(), 8) as *Node;
    *n = Node { expr: e, line: line };
    n
}
```

## The parser: `std.parse`

Hand-written parsers usually have one function per precedence level.
`std.parse.Pratt` replaces those with a table of operator precedences
and a driver that reads it. You give the driver five functions:

| Function | What it does |
|---|---|
| `peek` | Returns the current token's kind |
| `advance` | Moves to the next token |
| `atom` | Parses an operand that does not start with a prefix operator |
| `unary` | Builds a node for a prefix operator |
| `binary` | Builds a node for a binary operator |

The driver is generic over four types: the parser state, the token
kind, the node type and the error type:

```loimi
type Pratt = parse.Pratt<Parser, T, *Node, lex.Error>;
```

`type` declares an alias. The five functions are ordinary functions:

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
calls the driver again, which `atom` reaches through a pointer in the
parser state:

```loimi
        LParen => {
            p.advance()?;
            let e = p.expr()?;
            p.expect(T.RParen, "expected ')'")?;
            Ok(e)
        }
```

The operator table is filled at run time:

- `infix_left(k, bp)` makes `k` a left-associative binary operator
  with precedence `bp`. Higher numbers bind tighter.
- `prefix(k, bp)` makes `k` a prefix operator whose operand is parsed
  at precedence `bp`.

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

Later chapters add comparisons (precedence 3) and `not`.

The driver can do more than Pip needs. `tests/run/std_parse.lm` shows
all of it:

- `infix_right` for right-associative operators
- `led` functions for operators that don't have the form `lhs op rhs`,
  such as `?:`, postfix operators and assignment
- a `before_rhs` function, which a single-pass compiler can use to put
  the left operand in a register before the right one is parsed

Statements are parsed by hand. That is the easy part.

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

A tree walker like this is a fine interpreter for many purposes. But it
is slow:

- Every node is reached through a pointer, which may miss the cache.
- Evaluating each node is a function call, with its setup and
  teardown.
- The `match` on `n.expr` is a single indirect branch shared by every
  kind of node, so the CPU predicts it badly.

The rest of the tutorial replaces the tree walker with bytecode.

Next: [a first vm](03-first-vm.md).
