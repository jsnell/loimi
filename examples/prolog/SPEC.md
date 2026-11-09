# Prolog in Loimi: the supported subset

This document lists what the Prolog interpreter in `examples/prolog`
supports, for people who want to run Prolog programs on it. The
reference for every behaviour is SWI-Prolog 10: the tests' expected
output is its output.

## Running it

```sh
./target/release/loimi build examples/prolog/main.lm --release -o prolog
./prolog FILE.pl
```

The interpreter:

1. consults `FILE.pl`, running its directives as it goes;
2. runs the goals of the file's `:- initialization(G)` directives, in
   order;
3. runs the goal given with `-g`, if there is one. Without `-g` and
   without initialization directives, it runs `main/0` if the file
   defines it.

| Option | What it does |
|---|---|
| `-g GOAL` | Runs `GOAL` after the initialization goals. |
| `--gc-stress` | Collects garbage at every safepoint (for testing). |
| `--gc-verify` | As `--gc-stress`, with a generational heap whose write barriers are checked at each collection. |
| `--gc-tiny` | Collects often: after allocating half the live data, at least 64 KiB. |
| `--gc-gen` | Uses a generational heap. |
| `--gc-growth N` | Allocates N% of the live data, at least 2 MiB, before the next collection. The default is 400. |
| `--disasm` | Prints every compiled block. |
| `--no-supers` | Skips the superinstruction pass. |

Output goes to standard output, buffered. Warnings for failed directives
and uncaught exceptions go to standard error.

| Exit status | When |
|---|---|
| 0 | All goals succeeded. |
| 2 | The command line was wrong or the file could not be read. |
| 1 | A goal failed or raised an exception. |
| N | The program called `halt(N)`. |

## Syntax

Standard Prolog syntax:

- **Atoms:** letter-digit, symbol-char, solo, and quoted. Quoted atoms
  take `\` escapes, including `\xHH\` and octal `\NNN\`, and `''` for a
  quote.
- **Variables.**
- **Integers:** decimal, `0x`, `0o`, `0b`, `0'c`, with `_` digit groups.
- **Floats:** `1.5`, `1.0e10`, `1e10`, `1.0Inf`.
- **Double-quoted text** `"..."` is a code list by default. Change it with
  `set_prolog_flag(double_quotes, codes|chars|atom)`.
- **Back-quoted text** is a code list.
- `[...]`, `{...}`, and `%` and `/* */` comments.

**Operators** follow SWI-Prolog's default table. That includes `|` at
1100, read as `;`, and `dynamic`, `discontiguous`, `initialization` and
similar as prefix operators at 1150. Change the table with `op/3` and
read it with `current_op/3`. As in SWI-Prolog, an argument may contain an
operator above 999 other than `,` and `|`: `f(a :- b)`.

**Differences from SWI-Prolog:**

- `[]` is the atom `'[]'`. (SWI-Prolog 7 distinguishes them.)
- There is no separate string type. SWI-Prolog's default for
  `double_quotes` is `string`, so programs that print strings should set
  the flag to `codes`.
- `0'` takes no multi-byte characters other than UTF-8.
- `/* */` comments do not nest.

## Terms and numbers

- **Integers are unbounded.** 48-bit integers are stored directly in the
  term; larger ones are `std.bigint` numbers in the GC heap. Results
  switch between the two as they grow and shrink.
- **Floats** are IEEE doubles, printed as SWI-Prolog prints them:
  shortest round-trip digits, positional unless the exponent is below −4
  or the number would need more than three trailing zeros (`100.0`,
  `1.0e+04`, `1.0e-05`).
- **Standard order of terms:** Var < Number < Atom < Compound. Numbers
  compare by value, with a float before an equal integer. Compound terms
  compare by arity, then name, then arguments.

## Control

```
,  ;  ->  *->  \+  !  call/1..8  catch/3  throw/1  true  fail  false
once/1  ignore/1  forall/2  not/1  halt/0,1
```

- **Cut** is transparent through `,`, `;` and the then and else branches
  of `->`. It is local to `call/N`, `\+`, `findall/3` and if-then-else
  conditions, as ISO specifies.
- **`catch/3`** catches only while its goal runs. If the goal exited and
  left choicepoints, the catch is active again only when backtracking
  re-enters the goal.

### Errors

Errors are thrown as `error(Formal, _)` with ISO formal terms:

- `instantiation_error`
- `type_error(T, Culprit)`
- `domain_error(D, Culprit)`
- `existence_error(procedure, N/A)`
- `evaluation_error(zero_divisor|undefined|int_overflow)`
- `permission_error(modify, static_procedure, N/A)`
- `representation_error`
- `resource_error(R)`

The context argument is left unbound. SWI-Prolog fills it in, so
programs that print it give different output.

## Builtins

### Unification, comparison and types

```
=  \=  ==  \==  @<  @>  @=<  @>=  compare/3
var  nonvar  atom  number  integer  float  atomic  compound  callable
is_list  ground
```

### Arithmetic

Predicates:

```
is  =:=  =\=  <  >  =<  >=  succ/2  plus/3
```

Evaluable functions and constants:

```
+ - * / // mod rem div min max abs sign gcd msb sqrt sin cos tan asin
acos atan atan/2 atan2 sinh cosh tanh exp log log2 ** ^ float integer
float_integer_part float_fractional_part truncate round ceiling floor
>> << /\ \/ xor \ copysign random random_float pi e inf nan epsilon
max_tagged_integer cputime realtime
```

### Terms and atoms

| Predicate | Notes |
|---|---|
| `functor/3`, `=../2`, `copy_term/2`, `term_variables/2`, `numbervars/3` | |
| `arg/3` | N must be an integer. |
| `atom_codes`, `atom_chars`, `char_code`, `atom_length` | |
| `atom_concat/3` | All modes. |
| `sub_atom/5` | |
| `number_codes`, `number_chars`, `atom_number` | |
| `upcase_atom` | |
| `atomic_list_concat/2,3` | Including split mode. |
| `char_type/2` | Common types. |
| `atom_to_term/3`, `term_to_atom/2` | |

### Output

| Predicate | Notes |
|---|---|
| `write/1,2`, `print/1`, `writeq/1,2`, `write_canonical/1` | |
| `write_term/2` | Options `quoted`, `ignore_ops`, `numbervars`. |
| `nl/0,1`, `tab/1,2`, `put_char/1` | |
| `format/1,2,3` | Directives below. |

`format` directives:

```
~w ~p ~q ~a ~d ~Nd ~D ~s ~e ~f ~g ~c ~r ~n ~~ ~t ~N| ~N+ ~i ~*c
```

The only streams are `user_output` and `user_error`.

### All solutions

- `findall/3,4`
- `bagof/3` and `setof/3`, with `^`
- `aggregate_all/3`, with `count`, `sum`, `max`, `min`, `bag` and `set`

### Database and global variables

- `assert/1`, `asserta/1`, `assertz/1`, `retract/1`, `retractall/1`,
  `clause/2`, `dynamic/1`, with the logical update view.
- `abolish/1` is accepted and does nothing.
- `nb_setval/2` and `nb_getval/2`. `b_setval/2` and `b_getval/2` exist
  too, but do not undo on backtracking.

### Lists

These are a library written in Prolog. A program's own definition of one
of them replaces it.

```
append/2,3  member  memberchk  length  reverse  nth0  nth1  last  msort
sort/2,4  predsort  keysort  sum_list  sumlist  max_list  min_list
max_member  min_member  numlist/3  select/3,4  selectchk  subtract
intersection  union  delete  exclude  include  partition/4  maplist/2..5
foldl/4,5  list_to_set  permutation  flatten  pairs_keys_values
pairs_keys  pairs_values  between/3
```

`between/3` also accepts `inf` as the upper bound.

### DCG

- `-->` rules, without pushback;
- `phrase/2,3`;
- in rule bodies: `call//N`, `{}`, `!`, `\+`, terminal lists and code
  strings.

### System

| Predicate | Notes |
|---|---|
| `op/3`, `current_op/3` | |
| `set_prolog_flag/2` | Flags `double_quotes` and `unknown`. |
| `statistics/2` | Keys `runtime`, `cputime`, `walltime`, and `inferences` (always 0). |
| `garbage_collect/0` | |
| `ensure_loaded`, `use_module`, `discontiguous`, `style_check` | Accepted and ignored. |

## Not supported

- modules;
- streams other than the two standard ones;
- `read/1`;
- strings, rationals;
- attributed variables, `freeze`, tabling;
- global variables that are restored on backtracking;
- `assert` of clauses with more than 255 distinct variables plus
  arguments;
- `consult/1` from a running program;
- SWI-Prolog extensions not listed above, such as yall lambdas,
  `format(atom(A), ..)` and `string_*`.
