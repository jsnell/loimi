//! Doc snippets: every ```loimi block in README.md, docs/*.md and
//! docs/stdlib/*.md compiles, runs or fails as the annotation before it
//! says. (The tutorial's blocks are checked against examples/tutorial by
//! `tutorial_steps` in golden.rs.)
//!
//! The annotation is an HTML comment on the line(s) right before the
//! fence, invisible in rendered markdown:
//!
//!   <!-- snippet: check -->            compiles (front end and IR only)
//!   <!-- snippet: run -->              builds and runs, exit status 0
//!   <!-- snippet: run "text" "..." --> ... and stdout contains each text
//!   <!-- snippet: fail "text" -->      fails to compile, the error
//!                                      contains the text
//!   <!-- snippet: skip reason -->      not compiled (pseudocode, a design
//!                                      that is not built, ...): give the
//!                                      reason
//!
//! A `check`, `run` or `fail` annotation may continue on further lines
//! with a harness: a program in which a line `{snippet}` stands for the
//! block. Without one the block is compiled as it is.
//!
//!   <!-- snippet: check
//!   struct Regs { a: i64, b: i64 }
//!   {snippet}
//!   -->
//!
//! `LOIMI_DOC=language` checks only the files whose path contains
//! `language`; `LOIMI_DOC_UNANNOTATED=check` checks blocks without an
//! annotation as if they had `check` (to triage a new document).

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;

#[derive(Debug)]
enum Kind {
    Check,
    Run(Vec<String>),
    Fail(String),
    Skip,
}

struct Snippet {
    file: PathBuf,
    line: usize,
    kind: Kind,
    source: String,
}

/// The quoted strings of `s` (`"a" "b"`), with `\"`, `\\` and `\n`.
fn quoted(s: &str) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    let mut it = s.chars().peekable();
    loop {
        while it.peek().is_some_and(|c| c.is_whitespace()) {
            it.next();
        }
        match it.next() {
            None => return Ok(out),
            Some('"') => {}
            Some(c) => return Err(format!("expected a quoted string at `{}`", c)),
        }
        let mut cur = String::new();
        loop {
            match it.next() {
                None => return Err("unterminated string".into()),
                Some('"') => break,
                Some('\\') => match it.next() {
                    Some('n') => cur.push('\n'),
                    Some(c) => cur.push(c),
                    None => return Err("unterminated string".into()),
                },
                Some(c) => cur.push(c),
            }
        }
        out.push(cur);
    }
}

fn parse(file: &Path, text: &str, errors: &mut Vec<String>) -> Vec<Snippet> {
    let lines: Vec<&str> = text.lines().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let t = lines[i].trim();
        if !t.starts_with("```") {
            i += 1;
            continue;
        }
        let lang = &t[3..];
        let mut end = i + 1;
        while end < lines.len() && lines[end].trim() != "```" {
            end += 1;
        }
        if lang == "loimi" {
            let at = format!("{}:{}", file.display(), i + 1);
            // The annotation: the comment ending on the last non-blank line before the fence.
            let mut j = i;
            while j > 0 && lines[j - 1].trim().is_empty() {
                j -= 1;
            }
            let mut ann: Option<Vec<&str>> = None;
            if j > 0 && lines[j - 1].trim_end().ends_with("-->") {
                let mut k = j - 1;
                while k > 0 && !lines[k].trim_start().starts_with("<!--") {
                    k -= 1;
                }
                if lines[k].trim_start().starts_with("<!-- snippet:") {
                    ann = Some(lines[k..j].to_vec());
                }
            }
            // Strip the block's common indentation (blocks inside list items).
            let body = &lines[i + 1..end];
            let indent = body.iter().filter(|l| !l.trim().is_empty()).map(|l| l.len() - l.trim_start().len()).min().unwrap_or(0);
            let source: String = body.iter().map(|l| if l.len() >= indent { &l[indent..] } else { "" }).collect::<Vec<_>>().join("\n");
            match ann {
                None if std::env::var("LOIMI_DOC_UNANNOTATED").is_ok_and(|v| v == "check") => {
                    out.push(Snippet { file: file.to_path_buf(), line: i + 1, kind: Kind::Check, source: source + "\n" })
                }
                None => errors.push(format!("{}: ```loimi block without a <!-- snippet: .. --> annotation", at)),
                Some(ann) => {
                    let first = ann[0].trim_start().strip_prefix("<!-- snippet:").unwrap().trim();
                    let one_line = ann.len() == 1;
                    let first = if one_line { first.strip_suffix("-->").unwrap().trim() } else { first };
                    let (word, rest) = first.split_once(char::is_whitespace).unwrap_or((first, ""));
                    let rest = rest.trim();
                    let kind = match word {
                        "check" if rest.is_empty() => Ok(Kind::Check),
                        "run" => quoted(rest).map(Kind::Run),
                        "fail" => quoted(rest).and_then(|q| if q.len() == 1 { Ok(Kind::Fail(q[0].clone())) } else { Err("fail takes one quoted error text".into()) }),
                        "skip" if !rest.is_empty() && one_line => Ok(Kind::Skip),
                        "skip" => Err("skip needs a reason, on one line".into()),
                        _ => Err(format!("unknown annotation `{}`", first)),
                    };
                    let harness = if one_line {
                        None
                    } else {
                        let last = ann[ann.len() - 1].trim_end();
                        let mut h: Vec<&str> = ann[1..ann.len() - 1].to_vec();
                        let last = last.strip_suffix("-->").unwrap();
                        if !last.trim().is_empty() {
                            h.push(last);
                        }
                        Some(h)
                    };
                    match kind {
                        Err(e) => errors.push(format!("{}: {}", at, e)),
                        Ok(kind) => {
                            let source = match harness {
                                None => source,
                                Some(h) => {
                                    if !h.iter().any(|l| l.trim() == "{snippet}") {
                                        errors.push(format!("{}: harness without a `{{snippet}}` line", at));
                                    }
                                    h.iter().map(|l| if l.trim() == "{snippet}" { source.as_str() } else { l }).collect::<Vec<_>>().join("\n")
                                }
                            };
                            out.push(Snippet { file: file.to_path_buf(), line: i + 1, kind, source: source + "\n" });
                        }
                    }
                }
            }
        }
        i = end + 1;
    }
    out
}

fn doc_files(root: &Path) -> Vec<PathBuf> {
    let mut v = vec![root.join("README.md")];
    for dir in ["docs", "docs/stdlib"] {
        let mut d: Vec<PathBuf> = std::fs::read_dir(root.join(dir))
            .unwrap()
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|x| x == "md"))
            .collect();
        d.sort();
        v.extend(d);
    }
    v
}

/// `prog` under a 2 GiB virtual memory cap.
fn limited(prog: impl AsRef<std::ffi::OsStr>) -> Command {
    let mut c = Command::new("sh");
    c.arg("-c").arg("ulimit -v 2097152 && ulimit -c 0 && exec \"$0\" \"$@\"").arg(prog);
    c
}

/// Checks one snippet in `dir`; an error message on failure.
fn check(s: &Snippet, dir: &Path) -> Result<(), String> {
    let src = dir.join("snippet.lm");
    std::fs::write(&src, &s.source).unwrap();
    let loimi = || {
        let mut c = limited(env!("CARGO_BIN_EXE_loimi"));
        c.env("LOIMI_STD", env!("CARGO_MANIFEST_DIR"));
        c
    };
    let show = |what: &str, out: &std::process::Output| {
        format!(
            "{}:{}: {}\n--- source ---\n{}--- stdout ---\n{}--- stderr ---\n{}",
            s.file.display(),
            s.line,
            what,
            s.source,
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        )
    };
    match &s.kind {
        Kind::Skip => Ok(()),
        Kind::Check | Kind::Fail(_) => {
            let out = loimi().arg("build").arg(&src).arg("--emit-llvm").arg("-o").arg("-").output().unwrap();
            match &s.kind {
                Kind::Check if !out.status.success() => Err(show("does not compile", &out)),
                Kind::Fail(_) if out.status.success() => Err(show("compiles, but should fail", &out)),
                Kind::Fail(e) if !String::from_utf8_lossy(&out.stderr).contains(e.as_str()) => Err(show(&format!("the error does not contain {:?}", e), &out)),
                _ => Ok(()),
            }
        }
        Kind::Run(expect) => {
            let exe = dir.join("snippet");
            let out = loimi().arg("build").arg(&src).arg("-o").arg(&exe).output().unwrap();
            if !out.status.success() {
                return Err(show("does not build", &out));
            }
            let out = limited(&exe).output().unwrap();
            if !out.status.success() {
                return Err(show("exits with failure", &out));
            }
            let stdout = String::from_utf8_lossy(&out.stdout);
            match expect.iter().find(|e| !stdout.contains(e.as_str())) {
                Some(e) => Err(show(&format!("stdout does not contain {:?}", e), &out)),
                None => Ok(()),
            }
        }
    }
}

#[test]
fn doc_snippets() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let only = std::env::var("LOIMI_DOC").unwrap_or_default();
    let mut errors = Vec::new();
    let mut snippets = Vec::new();
    for f in doc_files(root) {
        if !f.to_string_lossy().contains(&only) {
            continue;
        }
        let text = std::fs::read_to_string(&f).unwrap();
        snippets.extend(parse(f.strip_prefix(root).unwrap(), &text, &mut errors));
    }
    let count = |p: fn(&Kind) -> bool| snippets.iter().filter(|s| p(&s.kind)).count();
    eprintln!(
        "doc snippets: {} ({} check, {} run, {} fail, {} skip)",
        snippets.len(),
        count(|k| matches!(k, Kind::Check)),
        count(|k| matches!(k, Kind::Run(_))),
        count(|k| matches!(k, Kind::Fail(_))),
        count(|k| matches!(k, Kind::Skip))
    );
    let next = Mutex::new(0usize);
    let failures = Mutex::new(Vec::new());
    std::thread::scope(|sc| {
        for t in 0..2 {
            let (next, failures, snippets) = (&next, &failures, &snippets);
            sc.spawn(move || {
                let dir = std::env::temp_dir().join(format!("loimi-doc-{}-{}", std::process::id(), t));
                std::fs::create_dir_all(&dir).unwrap();
                loop {
                    let i = {
                        let mut n = next.lock().unwrap();
                        *n += 1;
                        *n - 1
                    };
                    let Some(s) = snippets.get(i) else { break };
                    if let Err(e) = check(s, &dir) {
                        failures.lock().unwrap().push((i, e));
                    }
                }
                let _ = std::fs::remove_dir_all(&dir);
            });
        }
    });
    let mut failures = failures.into_inner().unwrap();
    failures.sort();
    errors.extend(failures.into_iter().map(|(_, e)| e));
    if !errors.is_empty() {
        panic!("{} failure(s):\n\n{}", errors.len(), errors.join("\n\n"));
    }
}
