# Regex: a regular-expression engine as a Loimi vm, shipped as a C library

A pattern compiles to bytecode for the `Rx` vm (`vm.lm`). Two engines
run it: a backtracking matcher (the vm's handlers) and a Pike VM
(`pike.lm`, linear time, host code over the vm's generated decoders).
The results, captures included, are those of Python's `re` on bytes.

## Syntax

| | |
|---|---|
| literals | any byte; `\` before punctuation; `\n \t \r \f \v \0` |
| `.` | any byte but `\n` |
| classes | `[abc]` `[a-z]` `[^...]` (`]` first is literal, `-` first or last is literal), with escapes and `\d \w \s` inside |
| shorthands | `\d \w \s \D \W \S` (ASCII, as for Python bytes patterns) |
| anchors | `^` (start of input), `$` (end, or before a final `\n`) |
| groups | `( )` capturing, `(?: )` non-capturing |
| alternation | `\|` |
| quantifiers | `* + ? {m} {m,} {,n} {m,n}` and lazy `*? +? ?? {m,n}?`; a `{` that does not start a valid quantifier is a literal, as in Python; counts up to 1000 |

Not supported (compile error): flags and `(?...)` groups other than
`(?:`, backreferences, lookaround, `\b \B \A \Z`, octal escapes beyond
`\0`, possessive quantifiers, Unicode. Limits: 256K code words, 40
nested loops whose body can match the empty string.

Matching follows Python's `sre` exactly, including its rule for loops
whose body can match empty: an optional iteration starts only if the
previous optional iteration consumed input (`MARK`/`LOOP`/`NOPROG`/
`UNMARK` in the bytecode), so `(a|)+` on `aa` gives group 1 = `(2, 2)`.

## Bytecode

`encoding fixed(u32)`; formats `N {op}`, `A {op, a: 24}`, `J {op, sj: s24}`,
`AJ {op, a: 8, sj: s16}`. Operand kinds and what the verifier checks:
`byte` (< 256), `cls` (class table index), `lit` (literal table index),
`slot` (capture slot), `mark` (loop mark), `label` (instruction boundary).

| op | |
|---|---|
| `CHAR c`, `ANY`, `CLASS s`, `LIT l` | consume a byte / a byte but `\n` / a byte of a class / a literal string |
| `CSTAR s`, `CPLUS s` | greedy `[class]*` / `+`: one pair of backtrack records for the whole run (backtracker only) |
| `BOL`, `EOL` | `^`, `$` |
| `SAVE s` | capture slot := position (undone on backtracking) |
| `SPLIT t`, `SPLITX t` | fork: next first / `t` first |
| `JMP t` | |
| `MARK m`, `UNMARK m`, `LOOP m t`, `NOPROG m t` | empty-iteration control (above) |
| `MATCH` | |

Superinstructions (from `--vm-stats` on the benchmarks, `stats.lm`):
`SAVE_SPLIT_CPLUS_LIT`, `SAVE_CPLUS`, `SAVE_MATCH`, `SPLIT_LIT`. A
`budget` instrumentation variant counts instructions for
`rx_set_step_limit`. The compiler emits two programs per pattern: one
with `LIT`/`CSTAR`/`CPLUS` and supers for the backtracker, one with
single-byte steps for the Pike VM. Both are verified before use.

## C API (`rx.h`, from `loimi build lib.lm --lib|--shared --header rx.h`)

```c
#define RX_SEARCH 1u
#define RX_PIKE 2u
rx_regex *rx_compile(const void *pat, uint64_t len);  /* NULL only on an internal error; check rx_ok */
bool rx_ok(rx_regex *r);
loimi_str rx_error(rx_regex *r);                 /* "" if none; valid until the next call on r */
int64_t rx_error_offset(rx_regex *r);            /* byte offset in the pattern, or -1 */
uint32_t rx_groups(rx_regex *r);
int32_t rx_exec(rx_regex *r, const void *buf, uint64_t len, uint64_t start, uint32_t flags,
                int64_t *spans, uint64_t nspans);
void rx_set_step_limit(rx_regex *r, int64_t n);  /* 0: none */
loimi_str rx_disasm(rx_regex *r, uint32_t flags);
loimi_str rx_internal_error(void);               /* the last caught panic on this thread */
void rx_free(rx_regex *r);
```

- `rx_exec` flags: `RX_SEARCH` = search (`pattern.search(s, start)`),
  else an anchored match (`pattern.match(s, start)`); `RX_PIKE` = use
  the Pike VM. Returns 1 (match), 0 (none), -1 (error: the pattern did
  not compile, the backtrack stack limit of 4M records, or the step
  limit; see `rx_error`), -2 (internal error: a panic inside the library
  was caught, see `rx_internal_error`; only `rx_free` is safe after it).
  On a match, writes `nspans` (start, end) pairs: group 0, then each
  group; -1 for a group that did not participate.
- Ownership: the pattern and the subject are borrowed for the call; the
  library keeps no pointer into them. `spans` is the caller's. Strings
  returned (`rx_error`, `rx_disasm`) belong to the regex.
- Threads: a `Regex` holds its match scratch space, so use each one from
  one thread at a time; different regexes can run in parallel.
- No GC: the library allocates with `std.mem` only.

## Files and checks

| file | |
|---|---|
| `syntax.lm` | parser: pattern to a tree |
| `compile.lm` | tree to bytecode (`std.bc` labels), first-byte set, required byte |
| `vm.lm` | the `Rx` vm: backtracking handlers, supers, step-limit variant |
| `pike.lm` | the Pike VM |
| `lib.lm` | the C API |
| `host.c` | C host: `demo`, `test CASES`, `fuzz N SEED`, `bench` |
| `check.sh` | build, demo, bytecode fuzzer, verifier test, byte-pattern fuzz, differential tests vs Python (`--random N --seed S`, `--asan`) |
| `dispatch.sh` | tests and benchmarks in all three dispatch modes |
| `fuzz.lm` | mutates compiled code, runs what the verifier accepts on both engines |
| `verify_test.lm` | the verifier on hand-made bad programs |
| `stats.lm` | benchmark driver for `--vm-stats` |
| `bench.py` | the benchmarks with Python's `re` |
| `tests/gen.py`, `tests/cases.txt`, `tests/compare.py` | differential tests |

```sh
examples/regex/check.sh                      # 2000 random cases, seed 1
examples/regex/check.sh --random 5000 --seed 3
examples/regex/check.sh --asan
clang-19 -O2 -DHAVE_PCRE2 -Iexamples/regex/build examples/regex/host.c \
    examples/regex/build/librx.a -lpcre2-8 -o rxbench && ./rxbench bench
python3 examples/regex/bench.py
```
