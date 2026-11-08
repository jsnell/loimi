//! Lint `handler_frame` (opt-in, `-W handler_frame`): op handlers whose
//! fast path sets up a stack frame, found in the generated assembly.
//!
//! A tail-call handler with no frame is a straight line from its
//! instruction word to the next dispatch. A call anywhere in it (a slow
//! path left outside `cold`, a panic for `.unwrap()` or a bounds check, a
//! write barrier's `remember`) can make LLVM set up a frame at entry
//! instead of only on the path that needs it, and every execution pays.
//! This is invisible in the source: the lint compiles the program's IR to
//! assembly a second time (`clang -S`, same options) and follows each
//! handler's fall-through path, which LLVM lays out as the likely path,
//! from entry to its first jump, looking for frame setup (`push`, `sub
//! rsp`, spills; `stp`/`sub sp` on AArch64). It names the calls the
//! handler makes (with the message of each panic site) as the likely
//! causes. A panic is named by what it checks and where, from codegen's
//! panic-site table ("the overflow check of `/` at vm.lm:512"); when
//! LLVM has merged several panic calls into one, the candidates are the
//! panic messages the function refers to.

use crate::codegen::Arch;
use crate::diag::{Diag, Span};
use std::collections::HashMap;

/// A handler to check: its symbol, the op's name, and where it is declared.
pub struct Site {
    pub symbol: String,
    pub op: String,
    pub span: Span,
}

/// What the lint needs from a build: the handlers, and codegen's panic
/// sites (message text, location included → what checks it and where).
pub struct Lint {
    pub sites: Vec<Site>,
    pub panics: HashMap<String, String>,
}

/// The string constants of an assembly file: label → text.
fn strings(asm: &str) -> HashMap<&str, String> {
    let mut out = HashMap::new();
    let mut label: Option<&str> = None;
    for line in asm.lines() {
        let t = line.trim();
        if let Some(l) = t.strip_suffix(':') {
            label = Some(l);
            continue;
        }
        if let (Some(l), Some(rest)) = (label, t.strip_prefix(".ascii").or_else(|| t.strip_prefix(".asciz")).or_else(|| t.strip_prefix(".string"))) {
            let s = rest.trim().trim_matches('"').replace("\\n", "").replace("\\012", "").replace("\\n", "");
            out.insert(l, s);
        }
        label = None;
    }
    out
}

/// The instruction lines of each function: symbol → lines.
fn functions(asm: &str) -> HashMap<String, Vec<&str>> {
    let mut out: HashMap<String, Vec<&str>> = HashMap::new();
    let mut cur: Option<String> = None;
    for line in asm.lines() {
        if !line.starts_with(|c: char| c.is_whitespace()) {
            // A label at column 0: a function start (or a local label).
            let head = line.split_whitespace().next().unwrap_or("");
            if let Some(l) = head.strip_suffix(':') {
                if l.starts_with(".Lfunc_end") {
                    cur = None;
                } else if !l.starts_with(".L") {
                    let name = l.trim_matches('"').to_string();
                    out.entry(name.clone()).or_default();
                    cur = Some(name);
                }
            }
            if let Some(c) = &cur {
                out.get_mut(c).unwrap().push(line);
            }
            continue;
        }
        if let Some(c) = &cur {
            out.get_mut(c).unwrap().push(line);
        }
    }
    out
}

/// The instruction of a line without its comment, and the comment.
fn split<'a>(line: &'a str, arch: Arch) -> (&'a str, &'a str) {
    let sep = if arch == Arch::X86_64 { "#" } else { "//" };
    match line.find(sep) {
        Some(i) => (line[..i].trim(), &line[i..]),
        None => (line.trim(), ""),
    }
}

/// The first frame-setup instruction on the fall-through path from entry.
fn fast_frame(lines: &[&str], arch: Arch) -> Option<String> {
    for line in lines.iter().skip(1) {
        let (ins, comment) = split(line, arch);
        if ins.is_empty() || ins.starts_with('.') || ins.ends_with(':') {
            continue;
        }
        let mn = ins.split_whitespace().next().unwrap_or("");
        let ops = ins[mn.len()..].trim();
        let frame = match arch {
            Arch::X86_64 => {
                mn.starts_with("push")
                    || (mn.starts_with("sub") && ops.ends_with("%rsp"))
                    || comment.contains("Spill")
            }
            Arch::AArch64 => {
                (mn == "stp" && ops.contains("[sp"))
                    || (mn == "sub" && ops.starts_with("sp,"))
                    || (mn == "str" && ops.contains("[sp"))
                    || comment.contains("Spill")
            }
        };
        if frame {
            return Some(ins.split_whitespace().collect::<Vec<_>>().join(" "));
        }
        let stop = match arch {
            Arch::X86_64 => mn.starts_with("jmp") || mn.starts_with("ret") || mn.starts_with("call") || mn == "ud2",
            Arch::AArch64 => mn == "b" || mn == "br" || mn == "ret" || mn == "bl" || mn == "blr" || mn == "brk",
        };
        if stop {
            return None;
        }
    }
    None
}

/// The calls a function makes, in order, each once; panics with their
/// messages.
fn calls(lines: &[&str], arch: Arch, strs: &HashMap<&str, String>, panics: &HashMap<String, String>) -> Vec<String> {
    let name = |m: &str| match panics.get(m.trim()) {
        Some(d) => d.clone(),
        None => format!("a panic (`{}`)", m.trim()),
    };
    let mut out: Vec<String> = Vec::new();
    let mut recent: Vec<&str> = Vec::new();
    for line in lines {
        let (ins, _) = split(line, arch);
        if ins.is_empty() || ins.starts_with('.') {
            continue;
        }
        let mn = ins.split_whitespace().next().unwrap_or("");
        let is_call = match arch {
            Arch::X86_64 => mn.starts_with("call"),
            Arch::AArch64 => mn == "bl",
        };
        if is_call {
            let target = ins[mn.len()..].trim().trim_start_matches('*').trim_matches('"').to_string();
            let target = target.split('@').next().unwrap_or("").to_string();
            let what = if target.contains("panic") {
                // The message is the most recent string constant loaded.
                let msg = recent.iter().rev().find_map(|i| {
                    i.split(|c: char| c == ' ' || c == ',' || c == '$' || c == '(' || c == ':')
                        .find_map(|tok| strs.get(tok))
                });
                match msg {
                    Some(m) => name(m),
                    None => {
                        // Panic calls merged into one (the message is a
                        // phi): the panic messages the function loads.
                        let mut cands: Vec<String> = Vec::new();
                        for l in lines {
                            let (i, _) = split(l, arch);
                            for tok in i.split(|c: char| c == ' ' || c == ',' || c == '$' || c == '(' || c == ':') {
                                if let Some(m) = strs.get(tok).filter(|m| panics.contains_key(m.trim())) {
                                    let d = name(m);
                                    if !cands.contains(&d) {
                                        cands.push(d);
                                    }
                                }
                            }
                        }
                        match cands.len() {
                            0 => format!("`{}`", target),
                            1 => cands.pop().unwrap(),
                            _ => format!("a panic (one of: {})", cands.join("; ")),
                        }
                    }
                }
            } else if target.starts_with('%') || target.starts_with('x') && arch == Arch::AArch64 && target.len() <= 3 {
                "an indirect call".to_string()
            } else {
                format!("`{}`", target)
            };
            if !out.contains(&what) {
                out.push(what);
            }
        }
        recent.push(ins);
        if recent.len() > 4 {
            recent.remove(0);
        }
    }
    out
}

/// The warnings for `sites` in assembly `asm`.
pub fn check(asm: &str, arch: Arch, sites: &[Site], panics: &HashMap<String, String>) -> Vec<Diag> {
    let strs = strings(asm);
    let fns = functions(asm);
    let mut out = Vec::new();
    for s in sites {
        let Some(lines) = fns.get(&s.symbol) else { continue };
        let Some(ins) = fast_frame(lines, arch) else { continue };
        let cs = calls(lines, arch, &strs, panics);
        let cause = if cs.is_empty() {
            "it makes no calls: the frame holds spills (too many values live at once)".to_string()
        } else {
            const MAX: usize = 6;
            let mut l = cs.iter().take(MAX).cloned().collect::<Vec<_>>().join(", ");
            if cs.len() > MAX {
                l = format!("{}, and {} more", l, cs.len() - MAX);
            }
            format!("calls in it (any of which can make LLVM set up the frame at entry): {}; move rare calls (and `raise`, `.unwrap()`, checked indexing on rare paths) into `cold {{ }}`", l)
        };
        out.push(Diag::warning(
            "handler_frame",
            s.span,
            format!("op `{}` sets up a stack frame on its fast path (`{}`), on every execution; {}", s.op, ins, cause),
        ));
    }
    out
}
