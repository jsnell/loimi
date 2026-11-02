# Stak: a small, statically stack-checked Forth

Stak is a dynamically typed, Forth-flavoured stack language. Programs are
sequences of whitespace-separated words. Unlike Forth, every definition
declares its stack effect, and the compiler checks stack depths statically,
so the interpreter needs no underflow checks at run time.

## Lexical

- Words are maximal runs of non-whitespace bytes.
- `\` starts a comment to the end of the line (it must be a word of its own).
- `( ... )` is a comment, except directly after `: name`, where it is the
  word's stack effect.
- `"..."` is a string literal with C escapes (`\n`, `\t`, `\"`, `\\`).
- A word that parses as an integer (`42`, `-7`, `0x1f`) or a float (`2.5`,
  `-1e3`) is a literal.

## Values

`int` (32-bit; `+ - *` that overflow give a float), `float` (f64),
`string` (immutable), `array` (fixed length, mutable), `bool`, `nil`.
`true`, `false`, `nil` push those values. Only `false` and `nil` are falsy.

## Definitions, locals, globals

```
: name ( a b -- c )  body ;
```

The names in the effect are documentation; the counts are checked: the
body starts with `a b` on the stack and must leave exactly one value on
every path, including `exit`. A word may call itself recursively by name.
Words must be defined before use (no forward references).

`{ x y }` inside a definition pops the top two values into locals `x` and
`y` (`y` gets the top). Afterwards `x` pushes the local's value and
`to x` pops into it. `var name` at the top level declares a global (nil);
`name` pushes it and `to name` stores into it.

## Control flow (inside definitions or at the top level)

| Form | Effect |
|---|---|
| `c if A then` | runs A if c is truthy; A must be balanced |
| `c if A else B then` | A and B must leave the same depth |
| `begin A c until` | repeat A until c is truthy; A must push one value net |
| `begin A c while B repeat` | the net effect of an iteration must be zero |
| `limit start do A loop` | for i = start; i < limit; i++ (skipped if start >= limit); `i`, `j` give the inner and outer index; A must be balanced |
| `exit` | return from the current word |

## Built-in words

| Word | Effect | |
|---|---|---|
| `dup drop swap over rot nip tuck` | standard Forth | |
| `2dup 2drop` | `( a b -- a b a b )`, `( a b -- )` | |
| `+ - * / mod` | `( a b -- c )` | `/` on two ints truncates; `mod` is floored |
| `negate abs` | `( a -- b )` | |
| `= <> < > <= >=` | `( a b -- bool )` | `=` compares numbers by value, strings by content |
| `not and or` | logical, on truthiness, give bools | |
| `print` | `( x -- )` | prints x and a newline |
| `.` | `( x -- )` | prints x and a space |
| `cr` | `( -- )` | prints a newline |
| `concat` | `( a b -- s )` | both converted to text and joined |
| `tostr` | `( x -- s )` | |
| `len` | `( s|arr -- n )` | |
| `array` | `( n -- arr )` | n nils |
| `[ a b c ]` | `( -- arr )` | the values pushed between the brackets |
| `@` | `( arr i -- v )` | |
| `!` | `( v arr i -- )` | |
| `sqrt` | `( x -- f )` | |
| `int` | `( x -- n )` | truncates a float |

## Errors

Compile errors (unknown word, stack underflow, unbalanced branches, wrong
output count) are reported with the source location and a snippet.
Run-time errors (type errors, index out of range, division by zero, stack
overflow) print `error: line N in WORD: message` and exit with status 1.

## Running

```
loimi build examples/stack/main.lm --release -o stak
./stak FILE.stk [--disasm] [--disasm-after] [--gc-stress]
examples/stack/check.sh        # every test in all dispatch modes, and with GC stress
```
