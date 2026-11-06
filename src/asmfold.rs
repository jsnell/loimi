//! Folding the dispatch table's load into the tail call's `jmp`, in the
//! assembly of an x86-64 executable (docs/lowering.md §3).
//!
//! LLVM's x86 instruction selection folds the load of a tail call's target
//! into the jump (`jmp *T(,%rax,8)`) only when the call passes at most six
//! values in registers: the pattern `X86tcret_6regs` in
//! `X86InstrCompiler.td` counts the call's register operands and refuses
//! past six, a fixed count made for the C convention, where more arguments
//! would leave no scratch register for the address. Under `preserve_none`
//! that is not so: the address's registers are free either way. A
//! handler that passes seven (a vm with an accumulator, the wasm
//! interpreter) gets `mov T(,%rax,8),%rcx; ...; jmp *%rcx`, one more
//! instruction per dispatch.
//!
//! So the build compiles such a program to assembly and rewrites each
//! `movq T(,%rI,8), %rD` that is followed, in the same block, by
//! `jmpq *%rD # TAILCALL`, into the folded jump, when `T` is a dispatch
//! table (a constant: the load may move past anything) and every
//! instruction in between is a plain one whose only register write is
//! its last operand, which is neither `%rI`, and which does not mention
//! `%rD`. Anything else is left as LLVM wrote it.

use std::collections::HashSet;

/// The 64-bit register a register name (any width) belongs to.
fn family(r: &str) -> Option<String> {
    let r = r.trim_start_matches('%');
    for b in ["ax", "bx", "cx", "dx"] {
        let c = &b[..1];
        if r == format!("r{}", b) || r == format!("e{}", b) || r == b || r == format!("{}l", c) || r == format!("{}h", c) {
            return Some(format!("r{}", b));
        }
    }
    for b in ["si", "di", "bp", "sp"] {
        if r == format!("r{}", b) || r == format!("e{}", b) || r == b || r == format!("{}l", b) {
            return Some(format!("r{}", b));
        }
    }
    let n = r.strip_prefix('r')?;
    let digits: String = n.chars().take_while(|c| c.is_ascii_digit()).collect();
    let rest = &n[digits.len()..];
    if !digits.is_empty() && ["", "d", "w", "b"].contains(&rest) {
        if let Ok(k) = digits.parse::<u32>() {
            if (8..16).contains(&k) {
                return Some(format!("r{}", k));
            }
        }
    }
    None
}

/// The general registers an operand text names.
fn regs_in(s: &str) -> HashSet<String> {
    let mut out = HashSet::new();
    for (i, _) in s.match_indices('%') {
        let name: String = s[i + 1..].chars().take_while(|c| c.is_ascii_alphanumeric()).collect();
        if let Some(f) = family(&name) {
            out.insert(f);
        }
    }
    out
}

/// Instructions whose only register write is their last operand (and,
/// for `pop`, `%rsp`). Flags do not matter: a jump follows.
fn plain(mnemonic: &str) -> bool {
    const P: &[&str] = &[
        "mov", "movb", "movw", "movl", "movq", "movabsq", "movzbl", "movzbw", "movzbq", "movzwl", "movzwq", "movsbl",
        "movsbw", "movsbq", "movswl", "movswq", "movslq", "leaq", "leal", "addq", "addl", "subq", "subl", "andq", "andl",
        "orq", "orl", "xorq", "xorl", "incq", "incl", "decq", "decl", "negq", "negl", "notq", "notl", "shlq", "shll",
        "shrq", "shrl", "sarq", "sarl", "cmpq", "cmpl", "testq", "testl", "popq", "movsd", "movss", "movaps", "movapd",
        "movups", "movupd", "movdqa", "movdqu", "movd", "xorps", "xorpd", "pxor",
    ];
    P.contains(&mnemonic)
}

/// A parsed `movq SYM(,%rI,8), %rD`: (symbol, index register, destination).
fn table_load<'a>(line: &'a str, tables: &dyn Fn(&str) -> bool) -> Option<(&'a str, &'a str, String)> {
    let rest = line.strip_prefix("\tmovq\t")?;
    let (mem, dst) = rest.split_once(", ")?;
    let (sym, idx) = mem.split_once("(,")?;
    let idx = idx.strip_suffix(",8)")?;
    let d = dst.trim();
    if !tables(sym) || family(idx).is_none() || !d.starts_with('%') || family(d)? != d.trim_start_matches('%') {
        return None;
    }
    Some((sym, idx, family(d)?))
}

/// Rewrite the unfolded dispatches of `asm`; `tables` says which symbols
/// are dispatch tables. Returns the text and the number folded.
pub fn fold(asm: &str, tables: &dyn Fn(&str) -> bool) -> (String, usize) {
    let mut lines: Vec<String> = asm.lines().map(|l| l.to_string()).collect();
    let mut n = 0;
    for i in 0..lines.len() {
        let Some(rest) = lines[i].strip_prefix("\tjmpq\t*%") else { continue };
        let Some((reg, comment)) = rest.split_once(|c: char| c.is_whitespace()) else { continue };
        if !comment.trim_start().starts_with("# TAILCALL") {
            continue;
        }
        let Some(tgt) = family(reg) else { continue };
        if tgt != reg {
            continue;
        }
        let mut j = i;
        let mut found = None;
        while j > 0 && i - j < 32 {
            j -= 1;
            let l = &lines[j];
            if l.is_empty() || l.starts_with("\t.cfi") || l.starts_with("\t.loc") || l.trim_start().starts_with('#') {
                continue;
            }
            if let Some((sym, idx, d)) = table_load(l, tables) {
                if d == tgt {
                    found = Some((j, sym.to_string(), idx.to_string()));
                }
                break;
            }
            // In between: a plain instruction of this block.
            if !l.starts_with('\t') || l.starts_with("\t.") {
                break;
            }
            let body = l.trim_start();
            let mnem = body.split_whitespace().next().unwrap_or("");
            if !plain(mnem) || regs_in(body).contains(&tgt) {
                break;
            }
        }
        let Some((j, sym, idx)) = found else { continue };
        let idx_f = family(&idx).unwrap();
        let ok = lines[j + 1..i].iter().all(|l| {
            let body = l.trim_start();
            if l.is_empty() || l.starts_with("\t.") || body.starts_with('#') {
                return true;
            }
            let ops = body.split_once(char::is_whitespace).map_or("", |x| x.1);
            let dst = ops.rsplit_once(", ").map_or(ops, |x| x.1);
            !regs_in(dst).contains(&idx_f) && !(body.starts_with("popq") && idx_f == "rsp")
        });
        if !ok {
            continue;
        }
        lines[i] = format!("\tjmpq\t*{}(,{},8)                 # TAILCALL", sym, idx);
        lines[j] = String::new();
        n += 1;
    }
    let mut out = lines.join("\n");
    out.push('\n');
    (out, n)
}

/// Whether the IR has a `preserve_none` function (a tail-call handler)
/// with more than six parameters, all of which LLVM passes in registers:
/// only then does the build go through assembly.
pub fn wanted(ir: &str) -> bool {
    ir.lines().any(|l| {
        let Some(rest) = l.strip_prefix("define internal preserve_nonecc ") else { return false };
        let Some(open) = rest.find('(') else { return false };
        let params = &rest[open + 1..];
        let mut depth = 0;
        let mut count = 1;
        for c in params.chars() {
            match c {
                '(' | '{' | '[' | '<' => depth += 1,
                ')' if depth == 0 => break,
                ')' | '}' | ']' | '>' => depth -= 1,
                ',' if depth == 0 => count += 1,
                _ => {}
            }
        }
        count > 6
    })
}

/// A dispatch table's symbol: `Vm.table` or `Vm.table.N` (tail-call
/// mode's tables are internal constants).
pub fn is_table(sym: &str) -> bool {
    let s = sym.trim_matches('"');
    s.ends_with(".table") || s.rsplit_once(".table.").is_some_and(|(_, n)| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
}

/// Compile `ll` to assembly with the codegen flags of `cmd` (the build's
/// clang command, before its inputs), fold, and return the `.s` to build
/// from: `None` (build from the IR) if nothing folded or clang failed.
pub fn assemble(clang: &str, cmd: &std::process::Command, ll: &std::path::Path) -> Option<std::path::PathBuf> {
    let s = ll.with_extension("s");
    let link_only = ["-no-pie", "-fno-sanitize-link-runtime"];
    let ok = std::process::Command::new(clang)
        .args(cmd.get_args().filter(|a| !link_only.iter().any(|l| a == l)))
        .args(["-S", "-x", "ir"])
        .arg(ll)
        .arg("-o")
        .arg(&s)
        .status()
        .is_ok_and(|st| st.success());
    if !ok {
        return None;
    }
    let text = std::fs::read_to_string(&s).ok()?;
    let (out, n) = fold(&text, &is_table);
    if n == 0 {
        return None;
    }
    std::fs::write(&s, out).ok()?;
    Some(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folds_across_plain_instructions() {
        let asm = "f:\n\tmovzbl\t%sil, %ecx\n\tmovq\tvm.Mini.table(,%rcx,8), %rcx\n\txorpd\t%xmm0, %xmm0\n\tmovq\t%rax, %r13\n\tjmpq\t*%rcx                           # TAILCALL\n";
        let (out, n) = fold(asm, &is_table);
        assert_eq!(n, 1);
        assert!(out.contains("\tjmpq\t*vm.Mini.table(,%rcx,8)"));
        assert!(!out.contains("movq\tvm.Mini.table"));
        assert!(out.contains("movq\t%rax, %r13"));
    }

    #[test]
    fn keeps_when_index_written_or_target_read() {
        let w = "\tmovq\tvm.Mini.table(,%rcx,8), %rax\n\tmovq\t%rdx, %rcx\n\tjmpq\t*%rax                           # TAILCALL\n";
        assert_eq!(fold(w, &is_table).1, 0);
        let w8 = "\tmovq\tvm.Mini.table(,%rcx,8), %rax\n\tmovb\t$1, %cl\n\tjmpq\t*%rax                           # TAILCALL\n";
        assert_eq!(fold(w8, &is_table).1, 0);
        let r = "\tmovq\tvm.Mini.table(,%rcx,8), %rax\n\tmovq\t%rax, %rdi\n\tjmpq\t*%rax                           # TAILCALL\n";
        assert_eq!(fold(r, &is_table).1, 0);
        let call = "\tmovq\tvm.Mini.table(,%rcx,8), %rax\n\tcallq\tfoo\n\tjmpq\t*%rax                           # TAILCALL\n";
        assert_eq!(fold(call, &is_table).1, 0);
        let label = "\tmovq\tvm.Mini.table(,%rcx,8), %rax\n.LBB0_1:\n\tjmpq\t*%rax                           # TAILCALL\n";
        assert_eq!(fold(label, &is_table).1, 0);
        let other = "\tmovq\tsome.array(,%rcx,8), %rax\n\tjmpq\t*%rax                           # TAILCALL\n";
        assert_eq!(fold(other, &is_table).1, 0);
    }

    #[test]
    fn registers_and_tables() {
        assert_eq!(family("%r11d").as_deref(), Some("r11"));
        assert_eq!(family("%cl").as_deref(), Some("rcx"));
        assert_eq!(family("%sil").as_deref(), Some("rsi"));
        assert_eq!(family("%xmm0"), None);
        assert!(is_table("vm.Mini.table") && is_table("vm.Mini.table.2") && is_table("\"a b.table\""));
        assert!(!is_table("vm.Mini.tables") && !is_table("x.table.y"));
        assert!(wanted("define internal preserve_nonecc void @h(ptr %a, ptr %b, ptr %c, ptr %d, ptr %e, double %f, i32 %insn) {"));
        assert!(!wanted("define internal preserve_nonecc void @h(ptr %a, ptr %b, ptr %c, ptr %d, ptr %e, i32 %insn) {"));
    }
}
