You are writing a benchmark suite and a conformance test suite for MiniJS,
a strict subset of JavaScript. Someone else is building a MiniJS
implementation, and your programs will be used to measure and test it
against V8's bytecode interpreter (`node --jitless`) and V8 with its JIT
(`node`). For the comparison to be fair, the programs must be written
independently of that implementation, from the language specification
alone.

Ground rules:
- Your only inputs are the specification in CLEANROOM/SPEC.md and the
  reference engine `node` (on PATH; use `node --jitless` for timing).
- Do NOT read, list or search anything outside CLEANROOM. In particular, do
  not look at /workspace-loimi or anything else on this machine to learn
  about the implementation. Do not search the web for it.
- You may use your general knowledge of JavaScript and of well-known
  benchmark programs: the Computer Language Benchmarks Game, the classic
  V8/Octane-style programs such as richards, deltablue and raytrace, the
  "Are We Fast Yet" suite, and so on.
- Every program must stay strictly inside the MiniJS subset. Re-read the
  spec's restrictions carefully:
  - no `==`, no `instanceof`, no template literals, no destructuring,
    no spread or rest, no default parameters;
  - no Map, Set or JSON; no Math.random (write your own generator);
  - semicolons are required;
  - never print objects or arrays directly;
  - no holes in arrays;
  - `sort` always takes a consistent comparator;
  - no getters or setters, and no class fields.
- Run every program with `node` to check that it is valid JavaScript.
- Resource limit: prefix commands with `ulimit -v 4000000;`. Node needs
  a large virtual address space; if 4 GB fails, use
  `node --max-old-space-size=1024` and no ulimit.

Deliverable 1: benchmarks in CLEANROOM/bench/
- One self-contained NAME.js per benchmark, with the problem size as a
  constant near the top of the file.
- Each benchmark must:
  - be deterministic;
  - print a short checksum, so a wrong implementation is detected;
  - take roughly 1-3 seconds under `node --jitless`.
- Write NAME.expected with the output.
- Cover:
  - richards (OO and method calls), deltablue (constraint solver,
    classes and inheritance), raytrace (object-heavy float math),
    binary-trees, fannkuch, nbody, spectral-norm, a string-building
    benchmark, a property-heavy benchmark (objects with many different
    shapes, i.e. polymorphic call sites), a closure/higher-order
    benchmark (map, filter, reduce), havlak or a json-like parser/
    tokenizer written by hand, and a bitops/crypto-style benchmark using
    `| 0` and shifts.
  - Twelve or more in total.
- Write README.md listing each benchmark, what it stresses, its origin,
  and the measured times under `node --jitless` and `node`.

Deliverable 2: conformance tests in CLEANROOM/tests/
- 40-80 small programs, NAME.js with NAME.expected from node.
- Together they should systematically cover every feature in SPEC.md:
  - lexical forms;
  - number semantics and ToString of numbers (many edge cases);
  - bitwise ops, string ops, objects, prototypes, `new`, classes and
    `extends`/`super`;
  - closures, including per-iteration let bindings;
  - every statement form, including labels, switch fallthrough and
    try/catch/finally ordering;
  - every library function.
- Error tests: NAME err_*.js. Their .expected holds only the stdout
  printed before the uncaught exception; the harness checks for a
  nonzero exit.
- Do not test unspecified behaviour, such as built-in error messages.

When done, reply with:
- a summary;
- the timing table;
- a list of places where SPEC.md was ambiguous or where you were unsure
  whether something is in the subset. This is important feedback.
