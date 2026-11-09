# Stak: a small, statically stack-checked Forth

Stak is a dynamically typed, Forth-flavoured stack language. This
document describes the language and how to run it.

Programs are sequences of whitespace-separated words. Unlike Forth, every
definition declares its stack effect, and the compiler checks stack
depths before the program runs. So the interpreter needs no underflow
checks at run time.

## Running

```sh
loimi build examples/stack/main.lm --release -o stak
./stak FILE.stk
examples/stack/check.sh        # every test in all dispatch modes, and with GC stress
```

| Option | What it does |
|---|---|
| `--disasm` | Prints the compiled bytecode before running. |
| `--disasm-after` | Prints the bytecode after running. |
| `--gc-stress` | Collects garbage at every safepoint (for testing). |

## Lexical structure

- Words are maximal runs of non-whitespace bytes.
- `\` starts a comment to the end of the line. It must be a word of its
  own.
- `( ... )` is a comment, except directly after `: name`, where it is the
  word's stack effect.
- `"..."` is a string literal with C escapes (`\n`, `\t`, `\"`, `\\`).
- A word that parses as an integer (`42`, `-7`, `0x1f`) or a float
  (`2.5`, `-1e3`) is a literal.

## Values

| Type | Notes |
|---|---|
| `int` | 32-bit. `+ - *` that overflow give a float. |
| `float` | 64-bit. |
| `string` | Immutable. |
| `array` | Fixed length, mutable. |
| `bool` | `true` and `false` push the two values. |
| `nil` | `nil` pushes it. |

Only `false` and `nil` are false in conditions.

## Definitions, locals and globals

```
: name ( a b -- c )  body ;
```

- The names in the stack effect are documentation, but the counts are
  checked. The body starts with `a b` on the stack and must leave exactly
  one value on every path, including `exit`.
- A word may call itself recursively by name.
- Words must be defined before they are used. There are no forward
  references.
- `{ x y }` inside a definition pops the top two values into the locals
  `x` and `y`; `y` gets the top value. After that, `x` pushes the local's
  value and `to x` pops a value into it.
- `var name` at the top level declares a global, set to nil. `name`
  pushes its value and `to name` stores into it.

## Control flow

These work inside definitions and at the top level.

| Form | Effect |
|---|---|
| `c if A then` | Runs A if c is true. A must leave the stack depth unchanged. |
| `c if A else B then` | A and B must leave the same depth. |
| `begin A c until` | Repeats A until c is true. A must push one value net. |
| `begin A c while B repeat` | The net effect of an iteration must be zero. |
| `limit start do A loop` | Runs A for i = start, start+1, ... while i < limit (not at all if start >= limit). `i` and `j` push the inner and outer loop index. A must leave the depth unchanged. |
| `exit` | Returns from the current word. |

## Built-in words

| Word | Effect | Notes |
|---|---|---|
| `dup drop swap over rot nip tuck` | As in Forth | |
| `2dup 2drop` | `( a b -- a b a b )`, `( a b -- )` | |
| `+ - * / mod` | `( a b -- c )` | `/` on two ints truncates; `mod` is floored. |
| `negate abs` | `( a -- b )` | |
| `= <> < > <= >=` | `( a b -- bool )` | `=` compares numbers by value, strings by content. |
| `not and or` | | Logical, on truthiness; give bools. |
| `print` | `( x -- )` | Prints x and a newline. |
| `.` | `( x -- )` | Prints x and a space. |
| `cr` | `( -- )` | Prints a newline. |
| `concat` | `( a b -- s )` | Converts both to text and joins them. |
| `tostr` | `( x -- s )` | |
| `len` | `( s\|arr -- n )` | |
| `array` | `( n -- arr )` | An array of n nils. |
| `[ a b c ]` | `( -- arr )` | An array of the values pushed between the brackets. |
| `@` | `( arr i -- v )` | |
| `!` | `( v arr i -- )` | |
| `sqrt` | `( x -- f )` | |
| `int` | `( x -- n )` | Truncates a float. |

## Errors

**Compile errors** are reported with the source location and a snippet.
They include:

- an unknown word;
- stack underflow;
- branches that leave different depths;
- a definition that leaves the wrong number of values.

**Run-time errors** print `error: line N in WORD: message` and exit with
status 1. They include type errors, an index out of range, division by
zero and stack overflow.
