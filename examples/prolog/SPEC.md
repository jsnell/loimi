# Prolog in Loimi: the supported subset

`prolog FILE.pl [-g GOAL] [--gc-stress | --gc-verify | --gc-tiny] [--disasm]`
consults FILE (running its directives as it goes), then runs the goals
of its `:- initialization(G)` directives in order; with none, `main/0`
if FILE defines it; with `-g`, GOAL instead. Output goes to stdout
(buffered); warnings for failed directives and uncaught exceptions go
to stderr. The exit status is 0, 1 if an initialization goal failed or
raised, or the code given to `halt/1`.

The reference for every behaviour below is SWI-Prolog 10 (the tests'
expected output is its output).

## Syntax

- Standard Prolog syntax: atoms (letter-digit, symbol-char, solo,
  quoted with `\` escapes including `\xHH\` and octal `\NNN\`, `''`
  for a quote), variables, integers (decimal, `0x`, `0o`, `0b`, `0'c`,
  `_` digit groups), floats (`1.5`, `1.0e10`, `1e10`, `1.0Inf`),
  `"..."` (code list by default; `set_prolog_flag(double_quotes,
  codes|chars|atom)`), back-quoted strings (codes), `[...]`, `{...}`,
  `%` and `/* */` comments.
- Operators: SWI-Prolog's default table (including `|` at 1100 read
  as `;`, `dynamic`/`discontiguous`/`initialization`/... as prefix
  operators at 1150), changed with `op/3`, read by `current_op/3`.
  Like SWI-Prolog, an argument may contain an operator above 999 other
  than `,` and `|` (`f(a :- b)`).
- Differences: `[]` is the atom `'[]'` (SWI-Prolog 7 distinguishes
  them); no strings as a separate type (SWI's default `double_quotes`
  is `string`: programs that print strings should set the flag to
  `codes`); no `0'` for multi-byte characters other than UTF-8; no
  block-quoted `/* */` nesting.

## Terms and numbers

- Integers are unbounded: 48-bit integers are immediate, larger ones
  are `std.bigint` numbers in the GC heap, and results switch between
  the two as they grow and shrink.
- Floats are IEEE doubles, printed as SWI-Prolog prints them (shortest
  round-trip digits, positional unless the exponent is below −4 or the
  number would need more than three trailing zeros: `100.0`, `1.0e+04`,
  `1.0e-05`).
- Standard order of terms: Var < Number < Atom < Compound; numbers by
  value, a float before an equal integer; compound terms by arity,
  name, then arguments.

## Control

`,` `;` `->` `*->` `\+` `!` `call/1..8` `catch/3` `throw/1` `true`
`fail` `false` `once/1` `ignore/1` `forall/2` `not/1` `halt/0,1`.
Cut is transparent through `,`, `;`, `->` (then/else branches) and
opaque in `call/N`, `\+`, `findall/3` and if-then-else conditions, as
ISO specifies. `catch/3` catches only while its goal runs (a goal that
exited leaving choicepoints is re-entered by backtracking, and only then
is the catch active again).

Errors are thrown as `error(Formal, _)` with ISO formal terms:
`instantiation_error`, `type_error(T, Culprit)`,
`domain_error(D, Culprit)`, `existence_error(procedure, N/A)`,
`evaluation_error(zero_divisor|undefined|int_overflow)`,
`permission_error(modify, static_procedure, N/A)`,
`representation_error`, `resource_error(R)`. The context argument is
left unbound (SWI fills it; programs that print it differ).

## Builtins

- Unification and comparison: `=` `\=` `==` `\==` `@<` `@>` `@=<`
  `@>=` `compare/3`.
- Types: `var` `nonvar` `atom` `number` `integer` `float` `atomic`
  `compound` `callable` `is_list` `ground`.
- Arithmetic: `is` `=:=` `=\=` `<` `>` `=<` `>=` `succ/2` `plus/3`,
  with `+ - * / // mod rem div min max abs sign gcd msb sqrt sin cos tan
  asin acos atan atan/2 atan2 sinh cosh tanh exp log log2 ** ^ float
  integer float_integer_part float_fractional_part truncate round
  ceiling floor >> << /\ \/ xor \ copysign random random_float pi e inf
  nan epsilon max_tagged_integer cputime realtime`.
- Terms: `functor/3` `arg/3` (integer N) `=../2` `copy_term/2`
  `term_variables/2` `numbervars/3`.
- Atoms: `atom_codes` `atom_chars` `char_code` `atom_length`
  `atom_concat/3` (all modes) `sub_atom/5` `number_codes`
  `number_chars` `atom_number` `upcase_atom` `atomic_list_concat/2,3`
  (split mode too) `char_type/2` (common types) `atom_to_term/3`
  `term_to_atom/2`.
- Output: `write/1,2` `print/1` `writeq/1,2` `write_canonical/1`
  `write_term/2` (`quoted`, `ignore_ops`, `numbervars`) `nl/0,1`
  `tab/1,2` `put_char/1` `format/1,2,3` with `~w ~p ~q ~a ~d ~Nd ~D ~s
  ~e ~f ~g ~c ~r ~n ~~ ~t ~N| ~N+ ~i ~*c`. Streams are `user_output`
  and `user_error` only.
- All solutions: `findall/3,4` `bagof/3` `setof/3` (with `^`)
  `aggregate_all/3` (`count`, `sum`, `max`, `min`, `bag`, `set`).
- Database: `assert/1` `asserta/1` `assertz/1` `retract/1`
  `retractall/1` `clause/2` `abolish/1` (no-op) `dynamic/1`, with the
  logical update view; `nb_setval/2` `nb_getval/2` (`b_` too, without
  backtracking).
- Lists (library, in Prolog; a program's own definition replaces them):
  `append/2,3` `member` `memberchk` `length` `reverse` `nth0` `nth1`
  `last` `msort` `sort/2,4` `predsort` `keysort` `sum_list` `sumlist`
  `max_list` `min_list` `max_member` `min_member` `numlist/3`
  `select/3,4` `selectchk` `subtract` `intersection` `union` `delete`
  `exclude` `include` `partition/4` `maplist/2..5` `foldl/4,5`
  `list_to_set` `permutation` `flatten` `pairs_keys_values`
  `pairs_keys` `pairs_values` `between/3` (also `inf`).
- DCG: `-->` rules (no pushback), `phrase/2,3`, `call//N`, `{}`, `!`,
  `\+`, terminal lists and code strings.
- System: `op/3` `current_op/3` `set_prolog_flag/2` (`double_quotes`,
  `unknown`) `statistics/2` (`runtime`, `cputime`, `walltime`,
  `inferences` as 0) `garbage_collect/0`; `ensure_loaded`,
  `use_module`, `discontiguous`, `style_check` are accepted and ignored.

## Not supported

Modules, streams other than the two standard ones, `read/1`, strings,
rationals, attributed variables, `freeze`, tabling,
global variables with backtracking, `assert` of clauses with more than
255 distinct variables plus arguments, `consult/1` from a running
program, `succ_or_fail`-style SWI extensions not listed above.
