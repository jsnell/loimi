# Writing the docs

How the documentation in this repository is written. Read this before
adding or changing a document.

## Know the reader

Each document has one main reader. Write for that reader, and leave out
what they don't need.

| Reader | Wants | Documents |
|---|---|---|
| Someone deciding whether Loimi is interesting | What it is, what it's good at, a taste of the code, how to try it | `README.md` |
| An interpreter author learning Loimi | A guided path, one idea at a time, with working code | `docs/tutorial/` |
| An interpreter author looking something up | Precise rules, complete options, short examples | `docs/language.md`, `docs/stdlib.md`, `docs/stdlib/` |
| A contributor to the compiler | How things work inside, and why they were built that way | `docs/lowering.md`, `docs/design/`, `docs/status.md`, `docs/roadmap.md` |
| Someone checking performance claims | What was measured, how, and the numbers | `docs/results.md`, `bench/` |
| A user of an example language | What the language accepts and how to run it | `examples/*/SPEC.md`, `examples/*/README.md` |

## Say only what is true now

- Describe what is built. Don't describe plans or features that don't
  exist.
- A feature that was tried and dropped gets one to three lines, in the
  one place where such records are kept for that area. Not a section.
- History (who did what when, the order things were tried in) belongs
  in `git log`, not in reference or tutorial text. Keep a date only
  where the reader needs it, such as on a measurement.

## Use plain language

- Use short sentences and common words.
- Don't invent jargon. If a term isn't standard in the field, either
  use a plain description instead or define it once, where it is first
  used.
- Don't use internal labels (milestone names like "M1", item numbers
  from old plans, section numbers of other documents) as if the reader
  knows them.
- Prefer the active voice: "the compiler checks", not "it is checked".

## Structure the information

- Start each document with one or two sentences: what it covers and who
  it's for.
- Put the most important information first: what something is and how
  to use it, then the details, then the edge cases.
- Use headings so a reader can scan and jump.
- Keep paragraphs short. Break up anything longer than about six lines.
- Use a bulleted list for three or more parallel items. Don't string a
  long list through a sentence with commas and parentheses.
- Use a table when items share the same attributes (an option, its
  default, what it does).
- Use a code block for anything the reader might type or copy.

## Cross-references

Link to another document only when the reader is likely to need it at
that point. Don't add "see also" links for completeness, and don't cite
section numbers when a link with a plain name will do.

## Code snippets

`loimi` code blocks in `README.md` and `docs/` are compiled by
`tests/docs.rs`. Each block is preceded by a `<!-- snippet: ... -->`
marker (see the top of `tests/docs.rs`). Keep the markers with their
blocks, and run `cargo test --release --test docs` after editing.
