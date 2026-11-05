//! `loimi ir-diff A B`: compare the optimized code of two builds,
//! function by function, ignoring what does not change the code.
//!
//! A and B are each a `.lm` program (compiled here with the build options
//! given) or a `.ll` file written by `loimi build --emit-llvm` (possibly by
//! another compiler). Both go through the same clang pipeline as `loimi
//! build` (`-O3` with `--release`), to optimized IR and, with `--asm`, to
//! assembly. Before comparing, each function is normalized:
//!
//! - Source locations in string constants (`dir/vm.lm:120:7: ..`) become
//!   `vm.lm:L:C`, and the message length passed with a location-bearing
//!   string (`@loimi.panic(ptr @s, i64 141)`) becomes `<len>`.
//! - Private and internal constants (strings, tables, LLVM's switch
//!   tables) are renamed after their normalized contents, so `@.str.12`
//!   and `@.str.15` holding the same text compare equal, and so do two
//!   tables with the same rows under different names.
//! - Local value and label names are numbered in order of appearance;
//!   comments, `!dbg` attachments and the numbering of attribute groups
//!   and metadata (resolved to their contents) are dropped.
//!
//! Functions present on one side only are matched up when their
//! normalized bodies are equal (a renamed function), and calls to them
//! are compared under one name. Exit status: 0 when every function has
//! the same code, 1 when some differ, 2 on errors.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use crate::{codegen, diag::SourceMap, OutKind, Opts};

/// `ir-diff` options (the rest are build options).
pub struct DiffOpts {
    /// The second input.
    pub other: Option<PathBuf>,
    /// `--asm`: also compare the assembly.
    pub asm: bool,
    /// `--context=N`: lines of context around each difference.
    pub context: usize,
    /// `--max-lines=N`: lines of diff shown per function (0: all).
    pub max_lines: usize,
    /// `--keep=DIR`: keep the optimized and normalized files there.
    pub keep: Option<PathBuf>,
}

impl Default for DiffOpts {
    fn default() -> Self {
        DiffOpts { other: None, asm: false, context: 3, max_lines: 60, keep: None }
    }
}

pub fn main(mut opts: Opts) -> ExitCode {
    match run(&mut opts) {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::from(1),
        Err(e) => {
            eprintln!("{}", e);
            ExitCode::from(2)
        }
    }
}

/// One side: its front-end IR and the extra LLVM options its vms need.
struct Input {
    label: String,
    ir: String,
    llvm_args: Vec<&'static str>,
}

/// The first line of `loimi build --emit-llvm` output, when the build
/// passes LLVM options: `; loimi-llvm-args: -opt=..`.
pub const LLVM_ARGS_TAG: &str = "loimi-llvm-args:";

fn load_input(opts: &mut Opts, path: &Path) -> Result<Input, String> {
    let label = path.display().to_string();
    if path.extension().is_some_and(|e| e == "ll") {
        let ir = std::fs::read_to_string(path).map_err(|e| format!("error: cannot read {}: {}", label, e))?;
        // The vm options `compile` would pass: the ones the file names
        // (`LLVM_ARGS_TAG`), else as far as the IR shows them (goto
        // dispatch; a `.ll` from an older compiler). Give `.lm` inputs,
        // or a `.ll` with the tag, for exact `--asm`.
        let mut llvm_args: Vec<&'static str> = Vec::new();
        let known = || crate::GOTO_LLVM_ARGS.iter().chain(crate::SWITCH_LLVM_ARGS).chain(crate::BRANCH_ALIGN_LLVM_ARGS);
        if let Some(tagged) = ir.lines().next().and_then(|l| l.strip_prefix("; ")).and_then(|l| l.strip_prefix(LLVM_ARGS_TAG)) {
            for a in tagged.split_whitespace() {
                match known().find(|k| **k == a) {
                    Some(k) => llvm_args.push(k),
                    None => return Err(format!("error: {}: unknown LLVM option `{}` in its `{}` line", label, a, LLVM_ARGS_TAG)),
                }
            }
        } else if ir.contains("indirectbr ") {
            llvm_args.extend(crate::GOTO_LLVM_ARGS);
        }
        return Ok(Input { label, ir, llvm_args });
    }
    opts.input = Some(path.to_path_buf());
    let mut sm = SourceMap::default();
    match crate::compile(&mut sm, opts) {
        Ok((ir, _, _, llvm_args, _)) => Ok(Input { label, ir, llvm_args }),
        Err(d) => Err(format!("{}error: ir-diff: {} does not build", sm.render_all(&d.0, opts.max_errors), label)),
    }
}

fn run(opts: &mut Opts) -> Result<bool, String> {
    let d = std::mem::take(&mut opts.ir_diff).unwrap();
    let pa = opts.input.clone().unwrap();
    let pb = d.other.clone().ok_or("ir-diff needs two inputs: loimi ir-diff A B [build options] [--asm]")?;
    if opts.jit {
        return Err("ir-diff: --jit-stencils is not supported (compare the default builds)".into());
    }
    opts.emit_llvm = true;
    let a = load_input(opts, &pa)?;
    let b = load_input(opts, &pb)?;
    let tmp = std::env::temp_dir().join(format!("loimi-irdiff-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).map_err(|e| format!("error: {}: {}", tmp.display(), e))?;
    let r = compare(opts, &d, &a, &b, &tmp);
    let _ = std::fs::remove_dir_all(&tmp);
    r
}

/// The clang options `loimi build` would use (minus output and linking).
fn clang_args(opts: &Opts, llvm_args: &[&'static str]) -> Vec<String> {
    let mut v: Vec<String> = vec![if opts.release { "-O3" } else { "-O1" }.into(), "-Wno-override-module".into()];
    if let Some(t) = &opts.target {
        v.push(format!("--target={}", t.triple));
    }
    if let Some(s) = &opts.sysroot {
        v.push(format!("--sysroot={}", s.display()));
    }
    let arch = opts.target.as_ref().map_or(codegen::Arch::host(), |t| t.arch);
    if let Some(c) = &opts.cpu {
        v.push(c.flag(arch));
    }
    v.push(if opts.kind == OutKind::Exe { "-fno-pic" } else { "-fPIC" }.into());
    if let Some(n) = opts.align_functions {
        v.push("-mllvm".into());
        v.push(format!("-align-all-functions={}", n.trailing_zeros()));
    }
    for a in llvm_args {
        v.push("-mllvm".into());
        v.push(a.to_string());
    }
    v
}

/// Run clang on both sides at once: `-emit-llvm` for IR, else assembly.
fn optimize(opts: &Opts, inputs: [&Input; 2], tmp: &Path, emit_ir: bool) -> Result<[String; 2], String> {
    let clang = crate::clang_cmd();
    let mut kids = Vec::new();
    for (i, inp) in inputs.iter().enumerate() {
        let src = tmp.join(format!("{}.ll", "ab".as_bytes()[i] as char));
        if !src.exists() {
            std::fs::write(&src, &inp.ir).map_err(|e| format!("error: {}: {}", src.display(), e))?;
        }
        let out = tmp.join(format!("{}.{}", "ab".as_bytes()[i] as char, if emit_ir { "opt.ll" } else { "s" }));
        let mut c = Command::new(&clang);
        c.args(clang_args(opts, &inp.llvm_args)).arg("-S");
        if emit_ir {
            c.arg("-emit-llvm");
        }
        c.arg("-x").arg("ir").arg(&src).arg("-o").arg(&out);
        let child = c.spawn().map_err(|e| format!("error: cannot run {}: {}", clang, e))?;
        kids.push((child, out, &inp.label));
    }
    let mut res = Vec::new();
    for (mut k, out, label) in kids {
        let ok = k.wait().is_ok_and(|s| s.success());
        if !ok {
            return Err(format!("error: {} failed on the IR of {}", clang, label));
        }
        res.push(std::fs::read_to_string(&out).map_err(|e| format!("error: {}: {}", out.display(), e))?);
    }
    let b = res.pop().unwrap();
    let a = res.pop().unwrap();
    Ok([a, b])
}

fn compare(opts: &Opts, d: &DiffOpts, a: &Input, b: &Input, tmp: &Path) -> Result<bool, String> {
    let [oa, ob] = optimize(opts, [a, b], tmp, true)?;
    let ma = Module::parse(&oa);
    let mb = Module::parse(&ob);
    let fa = cold_ordinals(ma.functions(), false);
    let fb = cold_ordinals(mb.functions(), false);
    let mut out = String::new();
    let _ = writeln!(out, "ir-diff: A = {}\n         B = {}", a.label, b.label);
    let same_ir = report(&mut out, "IR", &fa, &fb, d);
    let mut same = same_ir;
    let mut asm_texts = None;
    if d.asm {
        let [sa, sb] = optimize(opts, [a, b], tmp, false)?;
        let ga = cold_ordinals(asm_functions(&sa, &ma), true);
        let gb = cold_ordinals(asm_functions(&sb, &mb), true);
        same &= report(&mut out, "asm", &ga, &gb, d);
        asm_texts = Some((sa, sb, ga, gb));
    }
    let _ = writeln!(out, "{}", if same { "result: same code" } else { "result: code differs" });
    print!("{}", out);
    if let Some(dir) = &d.keep {
        std::fs::create_dir_all(dir).map_err(|e| format!("error: {}: {}", dir.display(), e))?;
        let dump = |fs: &BTreeMap<String, Vec<String>>| {
            let mut s = String::new();
            for (n, body) in fs {
                let _ = writeln!(s, "== {}", n);
                for l in body {
                    let _ = writeln!(s, "{}", l);
                }
            }
            s
        };
        let w = |n: &str, t: &str| std::fs::write(dir.join(n), t).map_err(|e| format!("error: {}: {}", dir.display(), e));
        w("a.opt.ll", &oa)?;
        w("b.opt.ll", &ob)?;
        w("a.norm.ll", &dump(&fa))?;
        w("b.norm.ll", &dump(&fb))?;
        if let Some((sa, sb, ga, gb)) = &asm_texts {
            w("a.s", sa)?;
            w("b.s", sb)?;
            w("a.norm.s", &dump(ga))?;
            w("b.norm.s", &dump(gb))?;
        }
    }
    Ok(same)
}

/// Outlined cold blocks are numbered across the program
/// (`vm.Py.op.BOR.cold132`), so a cold block added to one handler would
/// rename those of every handler after it: number them per handler
/// instead, in order (`vm.Py.op.BOR.cold#0`, ..).
fn cold_ordinals(f: BTreeMap<String, Vec<String>>, asm: bool) -> BTreeMap<String, Vec<String>> {
    let mut groups: BTreeMap<String, Vec<(u64, String)>> = BTreeMap::new();
    for k in f.keys() {
        if let Some(i) = k.rfind(".cold") {
            if let Ok(n) = k[i + 5..].parse::<u64>() {
                groups.entry(k[..i].to_string()).or_default().push((n, k.clone()));
            }
        }
    }
    let mut map = HashMap::new();
    for (p, mut v) in groups {
        v.sort();
        for (j, (_, k)) in v.into_iter().enumerate() {
            map.insert(k, format!("{}.cold#{}", p, j));
        }
    }
    if map.is_empty() {
        return f;
    }
    // Also the globals named after them (`X.cold260.c10`, the slots a
    // cold block's captures go through).
    let dotted: Vec<(String, String)> = map.iter().map(|(k, v)| (format!("{}.", k), format!("{}.", v))).collect();
    let line = |l: &String| {
        let mut s = rename_symbols(l, &map, asm);
        if s.contains(".cold") {
            for (k, v) in &dotted {
                if s.contains(k.as_str()) {
                    s = s.replace(k.as_str(), v);
                }
            }
        }
        s
    };
    f.into_iter().map(|(k, v)| (map.get(&k).cloned().unwrap_or(k), v.iter().map(line).collect())).collect()
}

/// Compare two sets of normalized functions; print the summary and the
/// differences. True when the code is the same.
fn report(out: &mut String, what: &str, fa: &BTreeMap<String, Vec<String>>, fb: &BTreeMap<String, Vec<String>>, d: &DiffOpts) -> bool {
    let only_a: Vec<&String> = fa.keys().filter(|k| !fb.contains_key(*k)).collect();
    let only_b: Vec<&String> = fb.keys().filter(|k| !fa.contains_key(*k)).collect();
    // Renamed functions: one-sided functions with equal bodies. Then
    // compare every body with B's names mapped to A's.
    let mut by_body: HashMap<&Vec<String>, Vec<&String>> = HashMap::new();
    for n in &only_a {
        by_body.entry(&fa[*n]).or_default().push(n);
    }
    let mut renamed: Vec<(String, String)> = Vec::new();
    for n in &only_b {
        if let Some(v) = by_body.get_mut(&fb[*n]) {
            if let Some(x) = v.pop() {
                renamed.push((x.clone(), (*n).clone()));
            }
        }
    }
    // A renamed callee changes its callers' text: retry with B's names
    // replaced by A's, until no more pairs appear.
    let mut fb2 = fb.clone();
    loop {
        let map: HashMap<String, String> = renamed.iter().map(|(x, y)| (y.clone(), x.clone())).collect();
        if !map.is_empty() {
            fb2 = fb.iter().map(|(k, v)| (k.clone(), v.iter().map(|l| rename_symbols(l, &map, what == "asm")).collect())).collect();
        }
        let ra: HashSet<&String> = renamed.iter().map(|p| &p.0).collect();
        let rb: HashSet<&String> = renamed.iter().map(|p| &p.1).collect();
        let mut by_body: HashMap<&Vec<String>, Vec<&String>> = HashMap::new();
        for n in only_a.iter().filter(|n| !ra.contains(*n)) {
            by_body.entry(&fa[*n]).or_default().push(n);
        }
        let mut more = Vec::new();
        for n in only_b.iter().filter(|n| !rb.contains(*n)) {
            if let Some(x) = by_body.get_mut(&fb2[*n]).and_then(|v| v.pop()) {
                more.push((x.clone(), (*n).clone()));
            }
        }
        if more.is_empty() {
            break;
        }
        renamed.extend(more);
    }
    let ra: HashSet<&String> = renamed.iter().map(|p| &p.0).collect();
    let rb: HashSet<&String> = renamed.iter().map(|p| &p.1).collect();
    let mut differ: Vec<&String> = Vec::new();
    let mut same_n = 0;
    for (n, body) in fa {
        if let Some(bb) = fb2.get(n) {
            if body == bb {
                same_n += 1;
            } else {
                differ.push(n);
            }
        }
    }
    let gone: Vec<&&String> = only_a.iter().filter(|n| !ra.contains(**n)).collect();
    let new: Vec<&&String> = only_b.iter().filter(|n| !rb.contains(**n)).collect();
    let _ = writeln!(
        out,
        "{}: {} functions in A, {} in B: {} same, {} renamed, {} differ, {} only in A, {} only in B",
        what,
        fa.len(),
        fb.len(),
        same_n,
        renamed.len(),
        differ.len(),
        gone.len(),
        new.len()
    );
    for (x, y) in &renamed {
        let _ = writeln!(out, "  renamed: {} -> {}", x, y);
    }
    for n in &gone {
        let _ = writeln!(out, "  only in A: {} ({} lines)", n, fa[**n].len());
    }
    for n in &new {
        let _ = writeln!(out, "  only in B: {} ({} lines)", n, fb[**n].len());
    }
    for n in &differ {
        let _ = writeln!(out, "  differs: {} ({} lines in A, {} in B)", n, fa[*n].len(), fb2[*n].len());
    }
    for n in &differ {
        let _ = writeln!(out, "--- {} {}", what, n);
        unified(out, &fa[*n], &fb2[*n], d.context, d.max_lines);
    }
    differ.is_empty() && gone.is_empty() && new.is_empty()
}

/// Replace symbol names (`@x` in IR, bare in assembly) by `map`.
fn rename_symbols(line: &str, map: &HashMap<String, String>, asm: bool) -> String {
    if !asm {
        return map_tokens(line, &mut |sig, name| if sig == '@' { map.get(name).map(|n| format!("@{}", quote_name(n))) } else { None });
    }
    let mut s = line.to_string();
    {
        let mut out = String::new();
        let mut last = 0;
        for (lo, hi) in asm_symbols(&s) {
            let tok = &s[lo..hi];
            let key = tok.trim_matches('"');
            if let Some(n) = map.get(key) {
                out.push_str(&s[last..lo]);
                out.push_str(n);
                last = hi;
            }
        }
        if last > 0 {
            out.push_str(&s[last..]);
            s = out;
        }
    }
    s
}

// ── LLVM IR ─────────────────────────────────────────────────────────

fn is_ident(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '$' | '-')
}

fn quote_name(n: &str) -> String {
    if n.chars().all(is_ident) && !n.starts_with(|c: char| c.is_ascii_digit()) || n.chars().all(|c| c.is_ascii_digit()) {
        n.to_string()
    } else {
        format!("\"{}\"", n)
    }
}

/// Walk the `@name`, `%name`, `#N` and `!N` references of an IR line
/// (outside string literals); `f(sigil, name)` may return a replacement
/// for the whole token.
fn map_tokens(line: &str, f: &mut dyn FnMut(char, &str) -> Option<String>) -> String {
    let b = line.as_bytes();
    let mut out = String::with_capacity(line.len());
    let mut i = 0;
    while i < b.len() {
        let c = b[i] as char;
        // String literals: `c"..."` constants and inline asm text.
        if c == '"' && !(i > 0 && matches!(b[i - 1], b'@' | b'%' | b'!')) {
            let j = line[i + 1..].find('"').map_or(b.len(), |k| i + 1 + k + 1);
            out.push_str(&line[i..j]);
            i = j;
            continue;
        }
        let prev_ident = i > 0 && is_ident(b[i - 1] as char);
        if matches!(c, '@' | '%' | '#' | '!') && !prev_ident && i + 1 < b.len() {
            let (name, end) = if b[i + 1] == b'"' {
                let j = line[i + 2..].find('"').map_or(b.len(), |k| i + 2 + k);
                (&line[i + 2..j], (j + 1).min(b.len()))
            } else {
                let mut j = i + 1;
                while j < b.len() && is_ident(b[j] as char) {
                    j += 1;
                }
                (&line[i + 1..j], j)
            };
            let valid = !name.is_empty() && (c != '#' || name.chars().all(|x| x.is_ascii_digit()));
            if valid {
                if let Some(r) = f(c, name) {
                    out.push_str(&r);
                } else {
                    out.push_str(&line[i..end]);
                }
                i = end;
                continue;
            }
        }
        out.push(c);
        i += 1;
    }
    out
}

/// The name a line defines at its start (`@x = ..`, `!12 = ..`).
fn defined_name(line: &str, sigil: char) -> Option<(&str, &str)> {
    let rest = line.strip_prefix(sigil)?;
    let (name, after) = if let Some(r) = rest.strip_prefix('"') {
        let j = r.find('"')?;
        (&r[..j], &r[j + 1..])
    } else {
        let j = rest.find(|c: char| !is_ident(c)).unwrap_or(rest.len());
        (&rest[..j], &rest[j..])
    };
    let init = after.strip_prefix(" = ")?;
    Some((name, init))
}

/// Replace `path/file.lm:12:5` by `file.lm:L:C`. Returns None if the
/// text has no location.
pub fn normalize_locations(s: &str) -> Option<String> {
    let mut out = String::new();
    let mut last = 0;
    let mut found = false;
    let mut from = 0;
    while let Some(k) = s[from..].find(".lm:") {
        let at = from + k;
        from = at + 4;
        let tail = &s[at + 4..];
        let l = tail.find(|c: char| !c.is_ascii_digit()).unwrap_or(tail.len());
        if l == 0 || !tail[l..].starts_with(':') {
            continue;
        }
        let tail2 = &tail[l + 1..];
        let c = tail2.find(|c: char| !c.is_ascii_digit()).unwrap_or(tail2.len());
        if c == 0 {
            continue;
        }
        // The path: back to a quote, a space, or the end of an escape.
        let head = &s[last..at];
        let mut start = head.rfind(|ch: char| matches!(ch, '"' | ' ' | '\t' | '(' | '\'')).map_or(0, |p| p + 1);
        if let Some(p) = head[start..].rfind('\\') {
            start += (p + 3).min(head.len() - start);
        }
        let path = &head[start..];
        let base = path.rsplit('/').next().unwrap_or(path);
        out.push_str(&head[..start]);
        out.push_str(base);
        out.push_str(".lm:L:C");
        last = at + 4 + l + 1 + c;
        from = last;
        found = true;
    }
    if !found {
        return None;
    }
    out.push_str(&s[last..]);
    Some(out)
}

struct Global {
    init: String,
    /// Private or internal constant: renamed after its contents.
    constant: bool,
    /// The length of the string before normalization, when it held a
    /// source location.
    loc_len: Option<u64>,
}

struct Module {
    globals: HashMap<String, Global>,
    attrs: HashMap<String, String>,
    meta: HashMap<String, String>,
    fns: Vec<(String, Vec<String>)>,
    /// Canonical names of the renamed constants.
    canon: HashMap<String, String>,
}

impl Module {
    fn parse(text: &str) -> Module {
        let mut m = Module { globals: HashMap::new(), attrs: HashMap::new(), meta: HashMap::new(), fns: Vec::new(), canon: HashMap::new() };
        let mut cur: Option<(String, Vec<String>)> = None;
        for line in text.lines() {
            if let Some((_, body)) = &mut cur {
                if line == "}" {
                    let f = cur.take().unwrap();
                    m.fns.push(f);
                } else {
                    body.push(line.to_string());
                }
                continue;
            }
            if line.starts_with("define ") {
                let name = line.find(" @").map(|p| {
                    let r = &line[p + 2..];
                    if let Some(q) = r.strip_prefix('"') {
                        q[..q.find('"').unwrap_or(q.len())].to_string()
                    } else {
                        r[..r.find(|c: char| !is_ident(c)).unwrap_or(r.len())].to_string()
                    }
                });
                cur = Some((name.unwrap_or_default(), vec![line.to_string()]));
            } else if let Some((name, init)) = defined_name(line, '@') {
                let words: Vec<&str> = init.split_whitespace().take(6).collect();
                let local = words.first().is_some_and(|w| *w == "private" || *w == "internal");
                let constant = local && words.contains(&"constant");
                let loc_len = if constant && init.contains("c\"") {
                    normalize_locations(init).and_then(|_| {
                        let p = init.find('[')?;
                        init[p + 1..].split(' ').next()?.parse().ok()
                    })
                } else {
                    None
                };
                m.globals.insert(name.to_string(), Global { init: init.to_string(), constant, loc_len });
            } else if let Some(r) = line.strip_prefix("attributes #") {
                if let Some((n, v)) = r.split_once(" = ") {
                    m.attrs.insert(n.to_string(), v.to_string());
                }
            } else if let Some((name, init)) = defined_name(line, '!') {
                m.meta.insert(name.to_string(), init.to_string());
            }
        }
        let names: Vec<String> = m.globals.keys().cloned().collect();
        for n in names {
            let mut visiting = HashSet::new();
            m.canonical(&n, &mut visiting);
        }
        m
    }

    /// The canonical name of global `n`: its own for symbols that are not
    /// local constants, else `const.<hash of normalized contents>`.
    fn canonical(&mut self, n: &str, visiting: &mut HashSet<String>) -> String {
        if let Some(c) = self.canon.get(n) {
            return c.clone();
        }
        let Some(g) = self.globals.get(n) else { return n.to_string() };
        if !g.constant || !visiting.insert(n.to_string()) {
            return n.to_string();
        }
        let mut init = g.init.clone();
        // Drop linkage and alignment: only the contents count.
        if let Some(p) = init.find(" constant ") {
            init = init[p + 10..].to_string();
        }
        if let Some(p) = init.rfind(", align ") {
            init.truncate(p);
        }
        if g.loc_len.is_some() {
            init = normalize_locations(&init).unwrap_or(init);
            if let Some(p) = init.find(" x i8]") {
                init = format!("[?{}", &init[p..]);
            }
        }
        // References to other constants by their canonical names.
        let refs: Vec<String> = {
            let mut v = Vec::new();
            map_tokens(&init, &mut |s, name| {
                if s == '@' {
                    v.push(name.to_string());
                }
                None
            });
            v
        };
        let mut rmap = HashMap::new();
        for r in refs {
            let c = self.canonical(&r, visiting);
            rmap.insert(r, c);
        }
        let init = map_tokens(&init, &mut |s, name| if s == '@' { rmap.get(name).map(|c| format!("@{}", quote_name(c))) } else { None });
        let c = format!("const.{:016x}", fnv(init.as_bytes()));
        visiting.remove(n);
        self.canon.insert(n.to_string(), c.clone());
        c
    }

    /// `!N` and `#N` resolved to their contents (nested `!N` too).
    fn meta_text(&self, n: &str, depth: u32, seen: &mut Vec<String>) -> String {
        if depth > 6 || seen.iter().any(|s| s == n) {
            return "!self".into();
        }
        let Some(t) = self.meta.get(n) else { return format!("!{}", n) };
        seen.push(n.to_string());
        let t = t.trim_start_matches("distinct ").to_string();
        let r = map_tokens(&t, &mut |s, name| {
            if s == '!' && name.chars().all(|c| c.is_ascii_digit()) {
                Some(self.meta_text(name, depth + 1, seen))
            } else {
                None
            }
        });
        seen.pop();
        r
    }

    /// The normalized functions, by name.
    fn functions(&self) -> BTreeMap<String, Vec<String>> {
        let mut out = BTreeMap::new();
        for (name, body) in &self.fns {
            out.insert(name.clone(), self.normalize_fn(body));
        }
        out
    }

    fn normalize_fn(&self, body: &[String]) -> Vec<String> {
        let mut locals: HashMap<String, usize> = HashMap::new();
        let mut local = |n: &str| {
            let k = locals.len();
            format!("%v{}", *locals.entry(n.to_string()).or_insert(k))
        };
        // Lengths of the location strings this function passes.
        let mut fn_lens: HashSet<u64> = HashSet::new();
        let mut out = Vec::new();
        for line in body {
            let line = strip_comment(line);
            let mut line = line.trim_end().to_string();
            if line.trim().is_empty() {
                continue;
            }
            // `!dbg !N` and other debug attachments carry no code.
            for att in [", !dbg ", " !dbg "] {
                while let Some(p) = line.find(att) {
                    let e = line[p + att.len()..].find(|c: char| c == ',' || c == ' ').map_or(line.len(), |k| p + att.len() + k);
                    line.replace_range(p..e, "");
                }
            }
            // A label definition (`bb12:`).
            let trimmed_label = !line.starts_with(' ') && line.ends_with(':') && !line.starts_with("define");
            if trimmed_label {
                let n = &line[..line.len() - 1];
                let n = n.trim_matches('"');
                out.push(format!("{}:", local(n)));
                continue;
            }
            let mut lens: Vec<u64> = Vec::new();
            let mapped = map_tokens(&line, &mut |s, name| match s {
                '@' => {
                    if let Some(l) = self.globals.get(name).and_then(|g| g.loc_len) {
                        lens.push(l);
                    }
                    self.canon.get(name).map(|c| format!("@{}", c))
                }
                '%' => Some(local(name)),
                '#' => self.attrs.get(name).cloned(),
                '!' if name.chars().all(|c| c.is_ascii_digit()) => Some(self.meta_text(name, 0, &mut Vec::new())),
                _ => None,
            });
            let mut mapped = mapped;
            fn_lens.extend(lens.iter().copied());
            for l in &lens {
                mapped = replace_int(&mapped, &format!("i64 {}", l), "i64 <len>");
            }
            if mapped.contains("@loimi.caller_len") {
                for l in &fn_lens {
                    mapped = replace_int(&mapped, &format!("i64 {}", l), "i64 <len>");
                }
            }
            out.push(mapped);
        }
        out
    }
}

/// Replace `pat` where it is a whole token (not followed by a digit).
fn replace_int(s: &str, pat: &str, with: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(p) = rest.find(pat) {
        let after = &rest[p + pat.len()..];
        out.push_str(&rest[..p]);
        if after.starts_with(|c: char| c.is_ascii_digit()) {
            out.push_str(pat);
        } else {
            out.push_str(with);
        }
        rest = after;
    }
    out.push_str(rest);
    out
}

/// The line without a `;` comment (outside string literals).
fn strip_comment(line: &str) -> &str {
    let mut q = false;
    for (i, c) in line.char_indices() {
        match c {
            '"' => q = !q,
            ';' if !q => return &line[..i],
            _ => {}
        }
    }
    line
}

fn fnv(b: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for &x in b {
        h ^= x as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

// ── assembly ────────────────────────────────────────────────────────

/// The `(start, end)` of symbol-like tokens in an assembly line.
fn asm_symbols(s: &str) -> Vec<(usize, usize)> {
    let b = s.as_bytes();
    let mut v = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let c = b[i] as char;
        if c == '"' {
            let j = s[i + 1..].find('"').map_or(b.len(), |k| i + 1 + k + 1);
            v.push((i, j));
            i = j;
        } else if c.is_ascii_alphabetic() || c == '_' || c == '.' {
            let mut j = i;
            while j < b.len() && (is_ident(b[j] as char) && b[j] != b'-') {
                j += 1;
            }
            v.push((i, j));
            i = j;
        } else if c.is_ascii_digit() {
            // Skip numbers (and `0x..`).
            while i < b.len() && (b[i] as char).is_ascii_alphanumeric() {
                i += 1;
            }
        } else {
            i += 1;
        }
    }
    v
}

/// The assembly's functions, normalized: local labels numbered in order
/// of appearance, constants named as in the IR, constant pools inlined.
fn asm_functions(text: &str, m: &Module) -> BTreeMap<String, Vec<String>> {
    // Private symbols are `.L<name>` in ELF assembly.
    let mut names: HashMap<String, String> = HashMap::new();
    let mut loc_lens: HashMap<String, u64> = HashMap::new();
    for (n, c) in &m.canon {
        if c.starts_with("const.") {
            let g = &m.globals[n];
            let asm = if g.init.starts_with("private") { format!(".L{}", n) } else { n.clone() };
            if let Some(l) = g.loc_len {
                loc_lens.insert(asm.clone(), l);
            }
            names.insert(asm, c.clone());
        }
    }
    let lines: Vec<&str> = text.lines().collect();
    // Constant pools (`.LCPI0_1: .quad ..`) by label, and where they are.
    let mut pools: HashMap<&str, String> = HashMap::new();
    let mut in_pool = vec![false; lines.len()];
    let mut i = 0;
    while i < lines.len() {
        if let Some(l) = lines[i].strip_suffix(':').filter(|l| l.starts_with(".LCPI")) {
            let mut j = i + 1;
            let mut data = String::new();
            while j < lines.len() && is_data(lines[j]) {
                data.push_str(strip_asm_comment(lines[j]).trim());
                data.push(';');
                j += 1;
            }
            pools.insert(l, data);
            for k in i..j {
                in_pool[k] = true;
            }
            i = j;
        } else {
            i += 1;
        }
    }
    let mut out = BTreeMap::new();
    // A function is the lines from its `# -- Begin function` marker (its
    // `.globl` and alignment come before `.type`) to the next one, or to
    // the first data object.
    let mut cur: Option<(String, Vec<String>)> = None;
    let mut pending: Option<Vec<String>> = None;
    let flush = |cur: &mut Option<(String, Vec<String>)>, out: &mut BTreeMap<String, Vec<String>>| {
        if let Some((n, body)) = cur.take() {
            out.insert(n, normalize_asm(&body, &names, &loc_lens, &pools));
        }
    };
    for (k, line) in lines.iter().enumerate() {
        if line.contains("# -- Begin function") {
            flush(&mut cur, &mut out);
            pending = Some(vec![strip_asm_comment(line).to_string()]);
            continue;
        }
        if let Some(r) = line.strip_prefix("\t.type\t") {
            if let Some((n, kind)) = r.rsplit_once(',') {
                flush(&mut cur, &mut out);
                if kind == "@function" {
                    cur = Some((n.trim_matches('"').to_string(), pending.take().unwrap_or_default()));
                }
                pending = None;
                continue;
            }
        }
        if in_pool[k] {
            continue;
        }
        if let Some(p) = &mut pending {
            p.push(line.to_string());
        } else if let Some((_, body)) = &mut cur {
            body.push(line.to_string());
        }
    }
    flush(&mut cur, &mut out);
    out
}

fn is_data(l: &str) -> bool {
    let t = l.trim_start();
    [".quad", ".long", ".short", ".byte", ".zero", ".ascii", ".asciz", ".octa", ".p2align", ".word", ".xword"].iter().any(|d| t.starts_with(d))
}

fn strip_asm_comment(line: &str) -> &str {
    let mut q = false;
    for (i, c) in line.char_indices() {
        match c {
            '"' => q = !q,
            '#' if !q => return &line[..i],
            _ => {}
        }
    }
    line
}

fn normalize_asm(body: &[String], names: &HashMap<String, String>, loc_lens: &HashMap<String, u64>, pools: &HashMap<&str, String>) -> Vec<String> {
    let mut labels: HashMap<String, usize> = HashMap::new();
    let mut out: Vec<String> = Vec::new();
    // Indices of lines that load a location string, with its length.
    let mut loc_at: Vec<(usize, u64)> = Vec::new();
    for line in body {
        let t = strip_asm_comment(line).trim_end();
        let tt = t.trim_start();
        if tt.is_empty() || tt.starts_with(".file") || tt.starts_with(".loc") || tt.starts_with(".size") {
            continue;
        }
        let mut s = String::new();
        let mut last = 0;
        for (lo, hi) in asm_symbols(t) {
            let tok = &t[lo..hi];
            let rep = if let Some(c) = names.get(tok.trim_matches('"')) {
                if let Some(&l) = loc_lens.get(tok.trim_matches('"')) {
                    loc_at.push((out.len(), l));
                }
                Some(c.clone())
            } else if let Some(p) = pools.get(tok) {
                Some(format!("[{}]", p))
            } else if tok.starts_with(".L") && tok.len() > 2 {
                let k = labels.len();
                Some(format!(".L{}", *labels.entry(tok.to_string()).or_insert(k)))
            } else {
                None
            };
            if let Some(r) = rep {
                s.push_str(&t[last..lo]);
                s.push_str(&r);
                last = hi;
            }
        }
        s.push_str(&t[last..]);
        out.push(s);
    }
    // The message length is an immediate next to the string's address.
    for (i, l) in loc_at {
        let lo = i.saturating_sub(3);
        let hi = (i + 4).min(out.len());
        for line in &mut out[lo..hi] {
            *line = replace_int(line, &format!("${},", l), "$<len>,");
        }
    }
    out
}

// ── diff output ─────────────────────────────────────────────────────

/// Print a unified diff of `a` and `b`.
fn unified(out: &mut String, a: &[String], b: &[String], context: usize, max_lines: usize) {
    let ops = diff_ops(a, b);
    // Group into hunks with context.
    let n = ops.len();
    let changed: Vec<usize> = (0..n).filter(|&i| ops[i].0 != ' ').collect();
    let mut shown = 0;
    let mut i = 0;
    while i < changed.len() {
        let start = changed[i].saturating_sub(context);
        let mut end = changed[i];
        while i < changed.len() && changed[i] <= end + 2 * context + 1 {
            end = changed[i];
            i += 1;
        }
        let end = (end + context + 1).min(n);
        let _ = writeln!(out, "@@");
        for op in &ops[start..end] {
            if max_lines > 0 && shown >= max_lines {
                let _ = writeln!(out, "  ... (more differences; --max-lines=0 shows all)");
                return;
            }
            // Long lines (a 150-way `phi`) are cut for display.
            match op.1.char_indices().nth(400) {
                Some((k, _)) => {
                    let _ = writeln!(out, "{}{} ...", op.0, &op.1[..k]);
                }
                None => {
                    let _ = writeln!(out, "{}{}", op.0, op.1);
                }
            }
            shown += 1;
        }
    }
}

/// Line edit script: (' ' | '-' | '+', line). Common prefix and suffix,
/// then an LCS of the middle (or a plain replacement when it is huge).
fn diff_ops<'a>(a: &'a [String], b: &'a [String]) -> Vec<(char, &'a str)> {
    let mut p = 0;
    while p < a.len() && p < b.len() && a[p] == b[p] {
        p += 1;
    }
    let mut s = 0;
    while s < a.len() - p && s < b.len() - p && a[a.len() - 1 - s] == b[b.len() - 1 - s] {
        s += 1;
    }
    let (am, bm) = (&a[p..a.len() - s], &b[p..b.len() - s]);
    let mut ops: Vec<(char, &str)> = a[..p].iter().map(|l| (' ', l.as_str())).collect();
    if am.len() * bm.len() <= 4_000_000 {
        let (n, m) = (am.len(), bm.len());
        let mut t = vec![0u32; (n + 1) * (m + 1)];
        for i in (0..n).rev() {
            for j in (0..m).rev() {
                t[i * (m + 1) + j] = if am[i] == bm[j] { t[(i + 1) * (m + 1) + j + 1] + 1 } else { t[(i + 1) * (m + 1) + j].max(t[i * (m + 1) + j + 1]) };
            }
        }
        let (mut i, mut j) = (0, 0);
        while i < n || j < m {
            if i < n && j < m && am[i] == bm[j] {
                ops.push((' ', &am[i]));
                i += 1;
                j += 1;
            } else if i < n && (j == m || t[(i + 1) * (m + 1) + j] >= t[i * (m + 1) + j + 1]) {
                ops.push(('-', &am[i]));
                i += 1;
            } else {
                ops.push(('+', &bm[j]));
                j += 1;
            }
        }
    } else {
        ops.extend(am.iter().map(|l| ('-', l.as_str())));
        ops.extend(bm.iter().map(|l| ('+', l.as_str())));
    }
    ops.extend(a[a.len() - s..].iter().map(|l| (' ', l.as_str())));
    ops
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locations() {
        assert_eq!(normalize_locations("c\"/a/b/vm.lm:120:7: index out of bounds\\0A\"").unwrap(), "c\"vm.lm:L:C: index out of bounds\\0A\"");
        assert_eq!(normalize_locations("x\\0A/tmp/q.lm:1:2: y").unwrap(), "x\\0Aq.lm:L:C: y");
        assert!(normalize_locations("no location here.lm: 3").is_none());
    }

    #[test]
    fn same_modulo_names_and_locations() {
        let a = "@.str.1 = private unnamed_addr constant [20 x i8] c\"/x/a.lm:9:5: boom\\0A\", align 1\n\
                 define void @f(i64 %n) {\nentry:\n  %t3 = icmp eq i64 %n, 0\n  br i1 %t3, label %bad, label %ok\nbad:\n  call void @loimi.panic(ptr @.str.1, i64 20) ; boom\n  unreachable\nok:\n  ret void\n}\n";
        let b = "@.str.7 = private unnamed_addr constant [22 x i8] c\"/y/a.lm:100:5: boom\\0A\", align 1\n\
                 define void @f(i64 %x) {\nentry:\n  %c = icmp eq i64 %x, 0\n  br i1 %c, label %p, label %q\np:\n  call void @loimi.panic(ptr @.str.7, i64 22)\n  unreachable\nq:\n  ret void\n}\n";
        let (fa, fb) = (Module::parse(a).functions(), Module::parse(b).functions());
        assert_eq!(fa, fb);
        let c = b.replace("icmp eq", "icmp ne");
        assert_ne!(fa, Module::parse(&c).functions());
    }
}
