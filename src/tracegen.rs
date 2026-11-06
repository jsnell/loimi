//! Ahead-of-time traces from a profile: stage M1 of the trace tier
//! (docs/design/trace-tier.md, "M1 as built").
//!
//! A `--jit-stencils` build keeps every op handler as a *stencil*: its
//! unoptimized IR with operand fields, words and constants as holes and
//! its continuations as tail calls to `__jit.n.D` / `__jit.j.O.S.B`
//! (jitgen.rs). This module stitches the stencils of a loop's
//! instructions into one function, with the holes filled from a
//! recording of the running program, and LLVM optimizes it with the rest
//! of the program. The JIT runtime (jit_rt.c) enters the function at the
//! loop header once the code object it was made for is compiled.
//!
//! The workflow (three steps, docs/design/copy-and-patch.md section 15):
//! 1. a JIT build run with `LOIMI_TRACE_RECORD=file` writes a *recording*:
//!    each compiled code object's words, ops and patched constants;
//! 2. `--trace-regions=REC` builds a *profiling* binary: every loop of the
//!    recording as a *region* (all its instructions, every block counted).
//!    Run with `LOIMI_TRACE_PROFILE=file`, it writes a *profile*: per
//!    region its recording and block counts;
//! 3. `--traces=PROF` builds the *trace* binary: per profiled loop that
//!    ran often enough, the hottest path around it (a *trace*), with the
//!    branches the profile never took as guards.
//!
//! Each instruction of a region or trace is a *step*: a copy of its
//! stencil, with the operands as literals, `next` and label jumps to the
//! following step as branches, and every other continuation an *exit*
//! (a tail call through the dispatch table, as stencils make).
//!
//! The register window. `regs` stays the real register file; the slots
//! `regs[0..NR)` are cached in a local array that SROA turns into SSA
//! values. A per-step analysis (constants and register-file offsets,
//! flow-sensitive over the handler's locals) decides each access:
//! constant offsets inside the window use it, other constant offsets
//! use memory, and unknown offsets choose at run time. Each block is in
//! *window mode* (the window holds the truth) or *real mode* (after a
//! call that may write memory: the window was written back before the
//! call, and memory holds the truth until the step's end, where the
//! window is reloaded if `regs` is still the trace's file). The window is
//! written back before exits and calls that may read the register file
//! (anything but pure intrinsics and read-only functions given no
//! pointer that reaches the register file), so the interpreter, the
//! collector and natives see the same state as without traces.
//!
//! Guards. In a trace, a branch from an on-trace block (counted) to an
//! off-trace one (never counted) becomes a guard if nothing on the way
//! from the step's start has an effect the interpreter would see twice:
//! a store outside the step's locals, the window and its own instruction
//! words, or a call that may store (the `redo_effect` rules of
//! redocheck.rs: the collector, the lock and `become` are no effects).
//! A guard writes the window back as it was at the step's start and
//! dispatches at the step's pc: the interpreter runs the instruction
//! again. Off-trace code after an effect stays, with exits at all its
//! continuations.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fmt::Write as _;

pub const FNV0: u64 = 1469598103934665603;
pub fn fnv(h: u64, x: u64) -> u64 {
    (h ^ x).wrapping_mul(1099511628211)
}
fn fnv_str(mut h: u64, s: &str) -> u64 {
    for b in s.bytes() {
        h = fnv(h, b as u64);
    }
    h
}

/// The largest window (register slots cached in SSA values).
const MAX_WINDOW: usize = 256;

/// A set of window slots.
#[derive(Clone, Copy, Default, PartialEq, Debug)]
struct Bits([u64; MAX_WINDOW / 64]);

impl Bits {
    fn set(&mut self, j: usize) {
        self.0[j / 64] |= 1 << (j % 64);
    }
    fn or(&self, o: &Bits) -> Bits {
        let mut r = *self;
        for (a, b) in r.0.iter_mut().zip(&o.0) {
            *a |= b;
        }
        r
    }
    fn iter(&self) -> impl Iterator<Item = usize> + '_ {
        (0..MAX_WINDOW).filter(|&j| self.0[j / 64] >> (j % 64) & 1 == 1)
    }
}

// ---------------------------------------------------------------- recordings

/// A compiled code object as recorded (`LOIMI_TRACE_RECORD`).
#[derive(Clone, Debug)]
pub struct Code {
    addr: String,
    pub n: usize,
    pub hash: u64,
    w: Vec<u32>,
    op: Vec<Option<String>>,
    /// Patched constants: (pc, word offset, shift, bits) -> value, or None
    /// if two values were seen.
    k: BTreeMap<(usize, u32, u32, u32), Option<u64>>,
    /// The section's text (written into profiles).
    text: String,
}

impl Code {
    /// Identity of the recorded code: words, ops and constants.
    fn key(&self) -> u64 {
        let mut h = fnv(fnv(FNV0, self.n as u64), self.hash);
        for (k, v) in &self.k {
            h = fnv(fnv(fnv(h, k.0 as u64), ((k.1 as u64) << 16) | ((k.2 as u64) << 8) | k.3 as u64), v.unwrap_or(0));
        }
        h
    }
}

fn hex(s: &str) -> Result<u64, String> {
    u64::from_str_radix(s, 16).map_err(|_| format!("bad hex number `{}`", s))
}
fn num(s: &str) -> Result<usize, String> {
    s.parse().map_err(|_| format!("bad number `{}`", s))
}

/// Parses `code`/`w`/`k`/`end` sections; other lines end a section.
fn parse_codes<'a>(lines: &mut std::iter::Peekable<impl Iterator<Item = &'a str>>) -> Result<Option<Code>, String> {
    let Some(first) = lines.next_if(|l| l.starts_with("code ")) else { return Ok(None) };
    let f: Vec<&str> = first.split_whitespace().collect();
    if f.len() < 4 {
        return Err(format!("bad line `{}`", first));
    }
    let n = num(f[2])?;
    let mut c = Code { addr: f[1].to_string(), n, hash: hex(f[3])?, w: vec![0; n], op: vec![None; n], k: BTreeMap::new(), text: String::new() };
    c.text.push_str(first);
    c.text.push('\n');
    while let Some(l) = lines.next_if(|l| l.starts_with("w ") || l.starts_with("k ")) {
        c.text.push_str(l);
        c.text.push('\n');
        let f: Vec<&str> = l.split_whitespace().collect();
        if f[0] == "w" && f.len() == 4 {
            let pc = num(f[1])?;
            if pc >= n {
                return Err(format!("word {} outside the code ({} words)", pc, n));
            }
            c.w[pc] = hex(f[2])? as u32;
            if f[3] != "-" {
                c.op[pc] = Some(f[3].to_string());
            }
        } else if f[0] == "k" && f.len() == 4 {
            let pc = num(f[1])?;
            let o: Vec<u32> = f[2].split('.').map(|x| x.parse().unwrap_or(u32::MAX)).collect();
            if o.len() != 3 || o.contains(&u32::MAX) {
                return Err(format!("bad line `{}`", l));
            }
            let v = hex(f[3])?;
            let e = c.k.entry((pc, o[0], o[1], o[2])).or_insert(Some(v));
            if *e != Some(v) {
                *e = None;
            }
        } else {
            return Err(format!("bad line `{}`", l));
        }
    }
    let _ = lines.next_if(|l| *l == "end");
    Ok(Some(c))
}

/// The code objects of a recording: for each address, its last compile
/// (the steady state after rewrites); the same code (words and
/// constants) once.
pub fn parse_recording(text: &str) -> Result<Vec<Code>, String> {
    let mut lines = text.lines().filter(|l| !l.trim().is_empty() && !l.starts_with('#')).peekable();
    let mut all: Vec<Code> = Vec::new();
    while lines.peek().is_some() {
        match parse_codes(&mut lines)? {
            Some(c) => all.push(c),
            None => return Err(format!("unexpected line `{}` in a recording", lines.next().unwrap())),
        }
    }
    let mut last: HashMap<String, usize> = HashMap::new();
    for (i, c) in all.iter().enumerate() {
        last.insert(c.addr.clone(), i);
    }
    let mut keep: Vec<usize> = last.into_values().collect();
    keep.sort();
    let mut seen = HashSet::new();
    Ok(keep.into_iter().map(|i| all[i].clone()).filter(|c| seen.insert(c.key())).collect())
}

/// A region's counts from a profile run.
pub struct ProfRegion {
    fingerprint: u64,
    header: usize,
    code: Code,
    counts: Vec<u64>,
}

/// The first line of a profile: the format and the build (a hash of
/// the stencils) it came from.
pub const PROFILE_MAGIC: &str = "loimi-trace-profile 1";

/// Parses profiles (`LOIMI_TRACE_PROFILE` output of a `--trace-regions`
/// build); regions with the same fingerprint are summed.
pub fn parse_profile(text: &str, stencil_hash: u64) -> Result<Vec<ProfRegion>, String> {
    let mut lines = text.lines().filter(|l| !l.trim().is_empty()).peekable();
    let mut out: Vec<ProfRegion> = Vec::new();
    while let Some(l) = lines.next() {
        if let Some(rest) = l.strip_prefix(PROFILE_MAGIC) {
            let h = hex(rest.trim())?;
            if h != stencil_hash {
                return Err("the profile comes from a build of a different vm or compiler (its stencils differ): \
                     build the profiling binary (--trace-regions) again"
                    .into());
            }
            continue;
        }
        let f: Vec<&str> = l.split_whitespace().collect();
        if f.len() != 3 || f[0] != "region" {
            return Err(format!("unexpected line `{}` in a profile", l));
        }
        let fingerprint = hex(f[1])?;
        let header = num(f[2])?;
        let code = parse_codes(&mut lines)?.ok_or("a region without its code")?;
        let cl = lines.next().unwrap_or("");
        let nc = cl.strip_prefix("counts ").ok_or_else(|| format!("expected `counts N`, got `{}`", cl))?;
        let mut counts = vec![0u64; num(nc.trim())?];
        while let Some(l) = lines.next_if(|l| l.starts_with("c ")) {
            let f: Vec<&str> = l.split_whitespace().collect();
            let i = num(f.get(1).copied().unwrap_or(""))?;
            let n: u64 = f.get(2).and_then(|x| x.parse().ok()).ok_or_else(|| format!("bad line `{}`", l))?;
            if i < counts.len() {
                counts[i] = counts[i].saturating_add(n);
            }
        }
        if lines.next() != Some("end") {
            return Err("a region without `end`".into());
        }
        match out.iter_mut().find(|r| r.fingerprint == fingerprint && r.counts.len() == counts.len()) {
            Some(r) => r.counts.iter_mut().zip(&counts).for_each(|(a, b)| *a = a.saturating_add(*b)),
            None => out.push(ProfRegion { fingerprint, header, code, counts }),
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------- IR text

fn ident_char(c: u8) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, b'-' | b'$' | b'.' | b'_')
}

/// The name at s[i..] after a sigil (`name` or `"name"`), and the index
/// after it.
fn name_at(s: &str, i: usize) -> Option<(&str, usize, bool)> {
    let b = s.as_bytes();
    if i < b.len() && b[i] == b'"' {
        let e = s[i + 1..].find('"')? + i + 1;
        return Some((&s[i + 1..e], e + 1, true));
    }
    let mut j = i;
    while j < b.len() && ident_char(b[j]) {
        j += 1;
    }
    if j == i { None } else { Some((&s[i..j], j, false)) }
}

/// The first global (`@name`) of a line.
fn first_global(line: &str) -> Option<&str> {
    let i = line.find('@')?;
    name_at(line, i + 1).map(|x| x.0)
}

/// `line` with every local (`%name`) replaced by `f(name, quoted)`;
/// string constants and global names are left alone.
fn map_locals(line: &str, f: &mut dyn FnMut(&str, bool) -> String) -> String {
    let b = line.as_bytes();
    let mut out = String::with_capacity(line.len() + 16);
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'%' => {
                if let Some((nm, e, q)) = name_at(line, i + 1) {
                    out.push_str(&f(nm, q));
                    i = e;
                    continue;
                }
                out.push('%');
                i += 1;
            }
            b'@' | b'!' | b'$' if i + 1 < b.len() && b[i + 1] == b'"' => {
                let e = line[i + 2..].find('"').map(|e| i + 2 + e + 1).unwrap_or(b.len());
                out.push_str(&line[i..e]);
                i = e;
            }
            b'"' => {
                // asm strings, c"..": not names
                let e = line[i + 1..].find('"').map(|e| i + 1 + e + 1).unwrap_or(b.len());
                out.push_str(&line[i..e]);
                i = e;
            }
            _ => {
                let c = line[i..].chars().next().unwrap();
                out.push(c);
                i += c.len_utf8();
            }
        }
    }
    out
}

/// Splits at top-level `sep` (outside brackets and quotes).
fn split_top(s: &str, sep: u8) -> Vec<&str> {
    let mut out = Vec::new();
    let (mut depth, mut start, mut q) = (0i32, 0usize, false);
    for (i, &c) in s.as_bytes().iter().enumerate() {
        if q {
            q = c != b'"';
            continue;
        }
        match c {
            b'"' => q = true,
            b'(' | b'{' | b'[' | b'<' => depth += 1,
            b')' | b'}' | b']' | b'>' => depth -= 1,
            _ if c == sep && depth == 0 => {
                out.push(s[start..i].trim());
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(s[start..].trim());
    out
}

/// The last whitespace-separated token at top level (an operand after
/// its type).
fn last_tok(s: &str) -> &str {
    let s = s.trim();
    let (mut depth, mut q, mut start) = (0i32, false, 0);
    for (i, &c) in s.as_bytes().iter().enumerate() {
        if q {
            q = c != b'"';
            continue;
        }
        match c {
            b'"' => q = true,
            b'(' | b'{' | b'[' | b'<' => depth += 1,
            b')' | b'}' | b']' | b'>' => depth -= 1,
            b' ' if depth == 0 => start = i + 1,
            _ => {}
        }
    }
    &s[start..]
}

/// `%x = ...`: the defined name (with its `%`) and the right-hand side.
fn def_of(line: &str) -> (Option<&str>, &str) {
    let t = line.trim_start();
    if t.starts_with('%') {
        if let Some((_, e, _)) = name_at(t, 1) {
            if t[e..].starts_with(" = ") {
                return (Some(&t[..e]), &t[e + 3..]);
            }
        }
    }
    (None, t)
}

/// A label line (`name:`): the label.
fn label_of(line: &str) -> Option<&str> {
    if line.starts_with(' ') || !line.ends_with(':') {
        return None;
    }
    Some(&line[..line.len() - 1])
}

/// Size in bytes of a simple type.
fn ty_size(t: &str) -> Option<i64> {
    Some(match t.trim() {
        "i8" => 1,
        "i16" => 2,
        "i32" | "float" => 4,
        "i64" | "ptr" | "double" => 8,
        _ => return None,
    })
}

fn int_bits(t: &str) -> Option<u32> {
    t.trim().strip_prefix('i').and_then(|b| b.parse().ok()).filter(|&b| (1..=64).contains(&b))
}

fn mask(bits: u32) -> u64 {
    if bits >= 64 { u64::MAX } else { (1u64 << bits) - 1 }
}

/// A parsed call: callee (a global's name, or a local with `%`), its
/// arguments (type text, value).
struct Call<'a> {
    musttail: bool,
    callee: &'a str,
    global: bool,
    asm: bool,
    sideeffect: bool,
    args: Vec<(&'a str, &'a str)>,
}

fn parse_call(rhs: &str) -> Option<Call<'_>> {
    let t = rhs.trim_start();
    let musttail = t.starts_with("musttail call ");
    let c = t.strip_prefix("musttail ").or_else(|| t.strip_prefix("tail ")).or_else(|| t.strip_prefix("notail ")).unwrap_or(t);
    let c = c.strip_prefix("call ")?;
    if let Some(a) = c.find(" asm ") {
        let sideeffect = c[a..].starts_with(" asm sideeffect") || c.contains("~{memory}");
        return Some(Call { musttail, callee: "", global: false, asm: true, sideeffect, args: Vec::new() });
    }
    // the callee: the first @name or %name followed by `(` at top level
    let b = c.as_bytes();
    let (mut i, mut depth) = (0usize, 0i32);
    while i < b.len() {
        match b[i] {
            b'(' | b'{' | b'[' | b'<' => depth += 1,
            b')' | b'}' | b']' | b'>' => depth -= 1,
            b'@' | b'%' if depth == 0 => {
                if let Some((nm, e, _)) = name_at(c, i + 1) {
                    if e < b.len() && b[e] == b'(' {
                        let close = matching_paren(c, e)?;
                        let args = split_top(&c[e + 1..close], b',')
                            .into_iter()
                            .filter(|a| !a.is_empty())
                            .map(|a| {
                                let v = last_tok(a);
                                (a[..a.len() - v.len()].trim(), v)
                            })
                            .collect();
                        let global = b[i] == b'@';
                        return Some(Call { musttail, callee: if global { nm } else { &c[i..e] }, global, asm: false, sideeffect: false, args });
                    }
                    i = e;
                    continue;
                }
            }
            b'"' => {
                i += c[i + 1..].find('"').map(|e| e + 1).unwrap_or(0);
            }
            _ => {}
        }
        i += 1;
    }
    None
}

fn matching_paren(s: &str, open: usize) -> Option<usize> {
    let (mut depth, mut q) = (0i32, false);
    for (i, &c) in s.as_bytes().iter().enumerate().skip(open) {
        if q {
            q = c != b'"';
            continue;
        }
        match c {
            b'"' => q = true,
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

/// `load T, ptr P`: (T, P).
fn parse_load(rhs: &str) -> Option<(&str, &str)> {
    let r = rhs.strip_prefix("load ")?;
    let r = r.strip_prefix("volatile ").unwrap_or(r);
    let p = split_top(r, b',');
    Some((p[0], p.get(1)?.strip_prefix("ptr ")?.trim()))
}

/// `store T V, ptr P`: (T, V, P).
fn parse_store(t: &str) -> Option<(&str, &str, &str)> {
    let r = t.strip_prefix("store ")?;
    let r = r.strip_prefix("volatile ").unwrap_or(r);
    let p = split_top(r, b',');
    let v = last_tok(p[0]);
    Some((p[0][..p[0].len() - v.len()].trim(), v, p.get(1)?.strip_prefix("ptr ")?.trim()))
}

/// `getelementptr [inbounds] T, ptr B, IT I, ..`: (T, B, indices).
fn parse_gep(rhs: &str) -> Option<(&str, &str, Vec<&str>)> {
    let r = rhs.strip_prefix("getelementptr ")?;
    let r = r.strip_prefix("inbounds ").unwrap_or(r);
    let p = split_top(r, b',');
    let base = p.get(1)?.strip_prefix("ptr ")?.trim();
    Some((p[0], base, p[2..].iter().map(|x| last_tok(x)).collect()))
}

fn parse_int(v: &str) -> Option<u64> {
    match v {
        "true" => Some(1),
        "false" | "null" | "zeroinitializer" => Some(0),
        _ => v.parse::<i64>().ok().map(|x| x as u64).or_else(|| v.parse::<u64>().ok()),
    }
}

// ---------------------------------------------------------------- the module

/// The program's functions and the stencils, as text.
struct Module<'a> {
    bodies: HashMap<&'a str, Vec<&'a str>>,
    heads: HashMap<&'a str, &'a str>,
    stencil_decls: HashMap<&'a str, &'a str>,
    program_syms: HashSet<&'a str>,
    types: HashSet<&'a str>,
    ro: RefCell<HashMap<String, bool>>,
    /// The handlers' state register types (from a stencil's signature).
    sig: Vec<String>,
    ret: String,
}

impl<'a> Module<'a> {
    fn new(ir: &'a str, stencils: &'a str) -> Module<'a> {
        let mut m = Module {
            bodies: HashMap::new(),
            heads: HashMap::new(),
            stencil_decls: HashMap::new(),
            program_syms: HashSet::new(),
            types: HashSet::new(),
            ro: RefCell::new(HashMap::new()),
            sig: Vec::new(),
            ret: "void".into(),
        };
        for (text, program) in [(ir, true), (stencils, false)] {
            let mut cur: Option<(&str, Vec<&str>)> = None;
            for line in text.lines() {
                if let Some((_, body)) = cur.as_mut() {
                    if line == "}" {
                        let (n, b) = cur.take().unwrap();
                        m.bodies.insert(n, b);
                    } else {
                        body.push(line);
                    }
                    continue;
                }
                if line.starts_with("define ") {
                    if let Some(n) = first_global(line) {
                        if !program && m.sig.is_empty() && n.starts_with("__jit.op.") {
                            if let Some(r) = line.strip_prefix("define preserve_nonecc ").and_then(|r| r.split(" @").next()) {
                                m.ret = r.to_string();
                            }
                            if let (Some(a), Some(b)) = (line.find('('), line.find(')')) {
                                m.sig = split_top(&line[a + 1..b], b',').iter().filter(|x| !x.ends_with("%insn")).map(|x| x[..x.len() - last_tok(x).len()].trim().to_string()).collect();
                            }
                        }
                        m.heads.insert(n, line);
                        if program {
                            m.program_syms.insert(n);
                        }
                        cur = Some((n, Vec::new()));
                    }
                } else if line.starts_with("declare ") {
                    if let Some(n) = first_global(line) {
                        if program {
                            m.program_syms.insert(n);
                        } else {
                            m.stencil_decls.insert(n, line);
                        }
                    }
                } else if line.starts_with('@') {
                    if let (Some(n), true) = (first_global(line), program) {
                        m.program_syms.insert(n);
                    }
                } else if line.starts_with('%') && line.contains(" = type ") {
                    if let Some((n, _, _)) = name_at(line, 1) {
                        m.types.insert(n);
                    }
                }
            }
        }
        m
    }

    /// Whether `name` stores nothing outside its own locals and calls
    /// only such functions: it cannot change the register file.
    fn readonly(&self, name: &str) -> bool {
        if let Some(i) = name.strip_prefix("llvm.") {
            return !effectful_intrinsic(i);
        }
        if let Some(&r) = self.ro.borrow().get(name) {
            return r;
        }
        self.ro.borrow_mut().insert(name.to_string(), false);
        let Some(body) = self.bodies.get(name) else { return false };
        let mut locals: HashSet<&str> = HashSet::new();
        let mut ok = true;
        for line in body {
            let (d, rhs) = def_of(line);
            if let Some(d) = d {
                if rhs.starts_with("alloca ") {
                    locals.insert(d);
                    continue;
                }
                if let Some((_, b, _)) = parse_gep(rhs) {
                    if locals.contains(b) {
                        locals.insert(d);
                    }
                    continue;
                }
            }
            if let Some((_, _, p)) = parse_store(rhs) {
                if !locals.contains(p) {
                    ok = false;
                    break;
                }
                continue;
            }
            if rhs.starts_with("atomicrmw ") || rhs.starts_with("cmpxchg ") || rhs.starts_with("fence") || rhs.starts_with("va_arg") {
                ok = false;
                break;
            }
            if let Some(c) = parse_call(rhs) {
                let fine = if c.asm {
                    !c.sideeffect
                } else if c.musttail || !c.global {
                    false
                } else {
                    self.readonly(c.callee)
                };
                if !fine {
                    ok = false;
                    break;
                }
            }
        }
        self.ro.borrow_mut().insert(name.to_string(), ok);
        ok
    }
}

fn effectful_intrinsic(i: &str) -> bool {
    ["memcpy", "memmove", "memset", "eh.", "va_", "init.trampoline", "coro.", "experimental.gc"].iter().any(|p| i.starts_with(p))
}

/// Calls whose effects re-execution may repeat (redocheck.rs
/// `runtime_fn`, and `become`): they change only the collector's, the
/// lock's or the JIT's state, never the register file.
fn redo_exempt(name: &str) -> bool {
    name.starts_with("std.gc.") || name.starts_with("std.gil.") || name == "loimi_jit_become"
}

// ---------------------------------------------------------------- analysis

/// What a value is, for the window and the effect rules.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Val {
    /// Not computed yet.
    Bot,
    /// An integer constant (its bits, masked to the type).
    Int(u64),
    /// A pointer into the register file: its byte offset from the
    /// trace's `regs` if known; `true` if it may also be some other
    /// pointer.
    Regs(Option<i64>, bool),
    /// A pointer to the instruction's own words (`pc`).
    Pc,
    /// The state registers through which the frames are reachable (ctx,
    /// fp), and pointers into them.
    Frame,
    /// A pointer to a local (alloca) of the step.
    Local,
    /// Anything else.
    Top,
}

fn join(a: Val, b: Val) -> Val {
    use Val::*;
    match (a, b) {
        (Bot, x) | (x, Bot) => x,
        _ if a == b => a,
        (Regs(o1, m1), Regs(o2, m2)) => Regs(if o1 == o2 { o1 } else { None }, m1 || m2),
        (Regs(..), _) | (_, Regs(..)) => Regs(None, true),
        _ => Top,
    }
}

/// How a call in a step is treated.
#[derive(Clone, Copy, PartialEq, Debug)]
enum CallK {
    /// Touches no memory the trace cares about.
    Pure,
    /// Reads memory, writes none: the window is written back first if it
    /// is given a pointer that reaches the register file.
    ReadOnly,
    /// May write, but not the register file, and re-execution may repeat
    /// it (the collector, the lock, `become`).
    Exempt,
    /// May read and write anything (natives, helpers that store).
    Unsafe,
}

/// A step: one instruction's stencil, its cold blocks inlined, renamed
/// and with its holes filled.
struct Step {
    pc: usize,
    /// (original label, lines) per block.
    blocks: Vec<(String, Vec<String>)>,
    succs: Vec<Vec<usize>>,
    vals: HashMap<String, Val>,
    allocas: HashSet<String>,
    /// Locals that hold a state register through which the frames are
    /// reached (regs, ctx, fp): their address reaches the register file.
    statevars: HashSet<String>,
}

struct Trace<'a> {
    m: &'a Module<'a>,
    code: &'a Code,
    table: &'a str,
    op_mask: u64,
    imm: &'a [(u64, u64, bool)],
    h: usize,
    /// Every recorded instruction's continuation targets.
    steps: Vec<Step>,
    /// pc -> index in steps.
    at: BTreeMap<usize, usize>,
    /// Pool constants baked in: (index, value).
    kchk: BTreeSet<(u64, u64)>,
    /// The handlers' state registers (the context, the state, the
    /// accumulators): their types, and which are pc and the register file.
    sig: Vec<String>,
    /// The handlers' return type (`void`, or a vm's result).
    ret: String,
    pc_idx: usize,
    regs_idx: Option<usize>,
    const_idx: Option<usize>,
}

/// The name of state register i of step prefix p (`sN.`): pc and the
/// register file have their own.
fn state_name(p: &str, i: usize, pc_idx: usize, regs_idx: Option<usize>) -> String {
    if i == pc_idx {
        format!("%{}pc", p)
    } else if Some(i) == regs_idx {
        "%tr.regs".to_string()
    } else {
        format!("%{}st{}", p, i)
    }
}

fn field(w: u32, shift: u32, bits: u32, signed: bool) -> i64 {
    let u = ((w as u64) >> shift) & mask(bits);
    if signed && bits > 0 && (u >> (bits - 1)) & 1 == 1 { u as i64 - (1i64 << bits) } else { u as i64 }
}

fn i32s(v: u32) -> i32 {
    v as i32
}

/// Continuation targets of a stencil's lines (pc offsets resolved).
fn cont_target(code: &Code, pc: usize, callee: &str) -> Option<Option<usize>> {
    let spec = callee.strip_prefix("__jit.")?;
    if let Some(d) = spec.strip_prefix("n.") {
        return Some(d.parse::<usize>().ok().map(|d| pc + d));
    }
    let j = spec.strip_prefix("j.")?;
    let p: Vec<u32> = j.split('.').filter_map(|x| x.parse().ok()).collect();
    if p.len() != 3 {
        return Some(None);
    }
    let wpc = pc + p[0] as usize;
    if wpc >= code.n {
        return Some(None);
    }
    let t = wpc as i64 + field(code.w[wpc], p[1], p[2], true);
    Some(if t >= 0 && (t as usize) < code.n { Some(t as usize) } else { None })
}

/// The stencil of the instruction at pc.
fn stencil_name(code: &Code, pc: usize) -> Option<String> {
    code.op[pc].as_ref().map(|o| format!("__jit.op.{}", o))
}

/// Loop headers of a code object (targets of backward continuations)
/// and their last back edge.
fn loops(m: &Module, code: &Code) -> BTreeMap<usize, usize> {
    let mut out: BTreeMap<usize, usize> = BTreeMap::new();
    for pc in 0..code.n {
        let Some(name) = stencil_name(code, pc) else { continue };
        let Some(body) = m.bodies.get(name.as_str()) else { continue };
        for l in body {
            if !l.contains("musttail call") {
                continue;
            }
            if let Some(c) = parse_call(def_of(l).1) {
                if let Some(Some(t)) = cont_target(code, pc, c.callee) {
                    if t <= pc && code.op[t].is_some() {
                        let b = out.entry(t).or_insert(pc);
                        *b = (*b).max(pc);
                    }
                }
            }
        }
    }
    out
}

impl<'a> Trace<'a> {
    fn new(m: &'a Module<'a>, code: &'a Code, vm: &'a VmInfo, h: usize) -> Trace<'a> {
        Trace {
            m,
            code,
            table: vm.table,
            op_mask: mask(vm.op_bits),
            imm: vm.imm,
            h,
            steps: Vec::new(),
            at: BTreeMap::new(),
            kchk: BTreeSet::new(),
            sig: m.sig.clone(),
            ret: m.ret.clone(),
            pc_idx: vm.pc_idx,
            regs_idx: vm.regs_idx,
            const_idx: vm.const_idx,
        }
    }

    /// Whether a recorded constant may be baked in as a literal: an
    /// immediate (no heap pointer, whose address differs between runs).
    fn immediate(&self, v: u64) -> bool {
        self.imm.iter().any(|&(mk, val, ult)| if ult { v < val } else { v & mk == val })
    }

    /// The stencil body of pc with its outlined cold blocks inlined once
    /// (`X.coldN` with a handler's signature: on a trace they may be the
    /// hot path, as Mini's native calls are).
    fn expand_cold(&self, body: &[&str]) -> Result<Vec<String>, String> {
        let mut out = Vec::new();
        let mut tail = Vec::new();
        let mut n = 0;
        let mut skip_ret = false;
        for &l in body {
            if skip_ret {
                skip_ret = false;
                if l.trim().starts_with("ret ") {
                    continue;
                }
            }
            let (_, rhs) = def_of(l);
            if rhs.starts_with("musttail call") {
                if let Some(c) = parse_call(rhs) {
                    let cold = c.global && c.callee.rsplit_once(".cold").is_some_and(|(_, d)| !d.is_empty() && d.bytes().all(|b| b.is_ascii_digit()));
                    if cold {
                        if let Some(cb) = self.m.bodies.get(c.callee) {
                            n += 1;
                            let pre = format!("c{}.", n);
                            let args: Vec<&str> = c.args.iter().map(|a| a.1).collect();
                            let mut params: Vec<String> = (0..self.sig.len()).map(|i| format!("p{}", i)).collect();
                            params.push("insn".into());
                            if args.len() != params.len() {
                                return Err(format!("cold block {} has {} arguments", c.callee, args.len()));
                            }
                            out.push(format!("  br label %{}entry", pre));
                            skip_ret = true;
                            for &cl in cb {
                                if let Some(lab) = label_of(cl) {
                                    tail.push(format!("{}{}:", pre, lab));
                                    continue;
                                }
                                if def_of(cl).1.starts_with("phi ") {
                                    return Err(format!("cold block {} has a phi", c.callee));
                                }
                                tail.push(map_locals(cl, &mut |nm, q| {
                                    if self.m.types.contains(nm) {
                                        return format!("%{}", if q { format!("\"{}\"", nm) } else { nm.to_string() });
                                    }
                                    if let Some(i) = params.iter().position(|p| *p == nm) {
                                        return args[i].to_string();
                                    }
                                    if q { format!("%\"{}{}\"", pre, nm) } else { format!("%{}{}", pre, nm) }
                                }));
                            }
                            continue;
                        }
                    }
                }
            }
            out.push(l.to_string());
        }
        out.extend(tail);
        Ok(out)
    }

    /// Renames the step's locals, fills the holes, and splits blocks.
    fn build_step(&mut self, pc: usize) -> Result<Step, String> {
        let code = self.code;
        let name = stencil_name(code, pc).ok_or("not an instruction")?;
        let body = self.m.bodies.get(name.as_str()).ok_or_else(|| format!("no stencil {}", name))?;
        let body = self.expand_cold(body)?;
        let p = format!("s{}.", pc);
        let word = code.w[pc];
        let mut blocks: Vec<(String, Vec<String>)> = Vec::new();
        let mut kp = 0;
        for l in &body {
            if let Some(lab) = label_of(l) {
                blocks.push((lab.to_string(), Vec::new()));
                continue;
            }
            if blocks.is_empty() {
                return Err(format!("{}: code before the first label", name));
            }
            let r = map_locals(l, &mut |nm, q| {
                if self.m.types.contains(nm) {
                    return format!("%{}", if q { format!("\"{}\"", nm) } else { nm.to_string() });
                }
                if let Some(i) = nm.strip_prefix('p').and_then(|d| d.parse::<usize>().ok()).filter(|&i| i < self.sig.len()) {
                    return state_name(&p, i, self.pc_idx, self.regs_idx);
                }
                match nm {
                    "insn" => i32s(word).to_string(),
                    _ if q => format!("%\"{}{}\"", p, nm),
                    _ => format!("%{}{}", p, nm),
                }
            });
            let lines = self.holes(pc, &p, &r, &mut kp)?;
            blocks.last_mut().unwrap().1.extend(lines);
        }
        let index: HashMap<String, usize> = blocks.iter().enumerate().map(|(i, b)| (format!("%{}{}", p, b.0), i)).collect();
        let mut succs = Vec::new();
        for (lab, ls) in &blocks {
            let t = ls.last().map(|s| s.trim()).unwrap_or("");
            let mut ss = Vec::new();
            if t.starts_with("br ") {
                for part in t.split("label ").skip(1) {
                    let tok = part.trim_end_matches(',').split([',', ' ']).next().unwrap_or("");
                    ss.push(*index.get(tok).ok_or_else(|| format!("{}: branch to unknown block {} in {}", name, tok, lab))?);
                }
            } else if !(t.starts_with("ret ") || t == "unreachable") {
                return Err(format!("{}: block {} ends in `{}`", name, lab, t));
            }
            succs.push(ss);
        }
        let mut st = Step { pc, blocks, succs, vals: HashMap::new(), allocas: HashSet::new(), statevars: HashSet::new() };
        analyze(&mut st);
        Ok(st)
    }

    /// Fills the holes of a renamed line (fields, words, constants).
    fn holes(&mut self, pc: usize, p: &str, line: &str, kp: &mut usize) -> Result<Vec<String>, String> {
        if !line.contains("@\"__jit.") || line.contains("musttail call") {
            return Ok(vec![line.to_string()]);
        }
        let code = self.code;
        let word_at = |o: usize| -> Result<u32, String> {
            let i = pc + o;
            if i >= code.n || code.op[i].is_none() {
                return Err(format!("a hole at {} reads word {}, which is not an instruction", pc, i));
            }
            Ok(code.w[i])
        };
        let mut l = line.to_string();
        let mut pre = Vec::new();
        while let Some(i) = l.find("@\"__jit.") {
            let e = l[i + 2..].find('"').ok_or("unterminated hole")? + i + 2;
            let hole = l[i + 2..e].to_string();
            let parts: Vec<&str> = hole.split('.').collect();
            // the value of the hole
            let (kind, val): (&str, String) = match parts.as_slice() {
                ["__jit", "f", o, s, b, sg] => {
                    let (o, s, b): (usize, u32, u32) = (o.parse().map_err(|_| "bad hole")?, s.parse().map_err(|_| "bad hole")?, b.parse().map_err(|_| "bad hole")?);
                    let f = field(word_at(o)?, s, b, *sg == "s");
                    ("f", (if *sg == "s" { f + (1i64 << (b - 1)) } else { f * 8 }).to_string())
                }
                ["__jit", "w", o] => ("w", i32s(word_at(o.parse().map_err(|_| "bad hole")?)?).to_string()),
                ["__jit", "c", o, s, b, "u"] => {
                    let (o, s, b): (usize, u32, u32) = (o.parse().map_err(|_| "bad hole")?, s.parse().map_err(|_| "bad hole")?, b.parse().map_err(|_| "bad hole")?);
                    let idx = field(word_at(o)?, s, b, false) as u64;
                    match code.k.get(&(pc, o as u32, s, b)) {
                        Some(Some(v)) if self.immediate(*v) => {
                            self.kchk.insert((idx, *v));
                            ("c", (*v as i64).to_string())
                        }
                        Some(None) => return Err(format!("constant {}.{}.{} at {}: two values recorded", o, s, b, pc)),
                        _ => {
                            // a heap constant (its address differs between
                            // runs) or not recorded: load it from the pool
                            *kp += 1;
                            let g = format!("%{}kp{}", p, kp);
                            let Some(ki) = self.const_idx else { return Err("a constant hole without a constant pool".into()) };
                            pre.push(format!("  {} = getelementptr inbounds i64, ptr {}, i64 {}", g, state_name(p, ki, self.pc_idx, self.regs_idx), idx));
                            ("p", g)
                        }
                    }
                }
                _ => return Err(format!("unknown hole @\"{}\" at {}", hole, pc)),
            };
            // its use
            let before = &l[..i];
            let after = &l[e + 1..];
            if let (Some(b), Some(rest)) = (before.strip_suffix("ptrtoint (ptr "), after.strip_prefix(" to ")) {
                // constant expression `ptrtoint (ptr @hole to iN)`
                let close = rest.find(')').ok_or("bad hole use")?;
                if kind == "p" || kind == "c" {
                    return Err(format!("constant hole used as an expression at {}", pc));
                }
                l = format!("{}{}{}", b, val, &rest[close + 1..]);
            } else if let (Some(b), Some(rest)) = (before.strip_suffix("= ptrtoint ptr "), after.strip_prefix(" to ")) {
                let ty = rest.split([' ', ',']).next().unwrap_or("i64");
                l = format!("{}= add {} 0, {}{}", b, ty, val, &rest[ty.len()..]);
            } else if kind == "c" || kind == "p" {
                let eq = before.find("= call ").ok_or("bad constant hole use")?;
                let close = after.find(')').ok_or("bad constant hole use")?;
                let tail = &after[close + 1..];
                l = if kind == "c" { format!("{}= add i64 0, {}{}", &before[..eq], val, tail) } else { format!("{}= load i64, ptr {}{}", &before[..eq], val, tail) };
            } else {
                return Err(format!("unhandled hole use at {}: {}", pc, line.trim()));
            }
        }
        pre.push(l);
        Ok(pre)
    }
}

/// The step's values: constants, register-file and pc pointers, over its
/// locals (allocas only ever loaded and stored, tracked per block) to a
/// fixpoint.
fn analyze(st: &mut Step) {
    // allocas, and those whose address is only a load/store pointer
    let mut allocas: HashSet<String> = HashSet::new();
    for (_, ls) in &st.blocks {
        for l in ls {
            if let (Some(d), rhs) = def_of(l) {
                if rhs.starts_with("alloca ") {
                    allocas.insert(d.to_string());
                }
            }
        }
    }
    // Allocas used other than as a load/store pointer or a call argument
    // (vm functions take the state registers by reference) are not
    // tracked: aggregates, addresses stored.
    let mut escaped: HashSet<String> = HashSet::new();
    for (_, ls) in &st.blocks {
        for l in ls {
            let (_, rhs) = def_of(l);
            let ok_ptr: Option<&str> = parse_load(rhs).map(|x| x.1).or_else(|| parse_store(rhs).map(|x| x.2));
            let call_args: Vec<&str> = parse_call(rhs).map(|c| c.args.iter().map(|a| a.1).collect()).unwrap_or_default();
            map_locals(l, &mut |nm, q| {
                let t = if q { format!("%\"{}\"", nm) } else { format!("%{}", nm) };
                if allocas.contains(&t) && ok_ptr != Some(t.as_str()) && def_of(l).0 != Some(t.as_str()) && !call_args.contains(&t.as_str()) {
                    escaped.insert(t.clone());
                }
                t
            });
            // a store of the alloca's address as a value
            if let Some((_, v, _)) = parse_store(rhs) {
                if allocas.contains(v) {
                    escaped.insert(v.to_string());
                }
            }
        }
    }
    let tracked: HashSet<String> = allocas.difference(&escaped).cloned().collect();
    let nb = st.blocks.len();
    let mut preds: Vec<Vec<usize>> = vec![Vec::new(); nb];
    for (b, ss) in st.succs.iter().enumerate() {
        for &s in ss {
            preds[s].push(b);
        }
    }
    let mut vals: HashMap<String, Val> = HashMap::new();
    for a in &allocas {
        vals.insert(a.clone(), Val::Local);
    }
    let mut out_state: Vec<Option<HashMap<String, Val>>> = vec![None; nb];
    let mut regvars: HashSet<String> = HashSet::new();
    let mut changed = true;
    let mut rounds = 0;
    while changed && rounds < 64 {
        changed = false;
        rounds += 1;
        for b in 0..nb {
            let mut state: HashMap<String, Val> = HashMap::new();
            if b != 0 {
                let ins: Vec<&HashMap<String, Val>> = preds[b].iter().filter_map(|&p| out_state[p].as_ref()).collect();
                if ins.is_empty() {
                    continue;
                }
                let keys: BTreeSet<&String> = ins.iter().flat_map(|m| m.keys()).collect();
                for k in keys {
                    let v = ins.iter().fold(Val::Bot, |a, m| join(a, m.get(k).copied().unwrap_or(Val::Bot)));
                    state.insert(k.clone(), v);
                }
            }
            for l in &st.blocks[b].1 {
                transfer(l, &mut state, &mut vals, &tracked, &mut regvars, &mut changed);
            }
            if out_state[b].as_ref() != Some(&state) {
                out_state[b] = Some(state);
                changed = true;
            }
        }
    }
    if changed {
        // no fixpoint: every value unknown (accesses choose at run time)
        for v in vals.values_mut() {
            if *v != Val::Local {
                *v = Val::Regs(None, true);
            }
        }
    }
    // slots of the state registers through which the frames are reached
    let mut statevars: HashSet<String> = regvars;
    for s in out_state.iter().flatten() {
        for (k, v) in s {
            if matches!(v, Val::Regs(..) | Val::Frame) {
                statevars.insert(k.clone());
            }
        }
    }
    st.vals = vals;
    st.allocas = allocas;
    st.statevars = statevars;
}

fn val_of(vals: &HashMap<String, Val>, v: &str) -> Val {
    if v == "%tr.regs" {
        return Val::Regs(Some(0), false);
    }
    if let Some(x) = vals.get(v) {
        return *x;
    }
    if v.starts_with('%') {
        if v.ends_with(".pc") && v.matches('.').count() == 1 {
            return Val::Pc;
        }
        if v.rsplit_once('.').is_some_and(|(a, b)| b.starts_with("st") && b[2..].bytes().all(|c| c.is_ascii_digit()) && !a.contains('.')) {
            return Val::Frame;
        }
        return Val::Bot;
    }
    match parse_int(v) {
        Some(x) => Val::Int(x),
        None => Val::Top,
    }
}

fn transfer(l: &str, state: &mut HashMap<String, Val>, vals: &mut HashMap<String, Val>, tracked: &HashSet<String>, regvars: &mut HashSet<String>, changed: &mut bool) {
    let (d, rhs) = def_of(l);
    if let Some(c) = parse_call(rhs) {
        // locals passed by reference: the callee may set them
        for (_, a) in &c.args {
            if tracked.contains(*a) {
                let h = match state.get(*a).copied() {
                    _ if regvars.contains(*a) => Val::Regs(None, true),
                    Some(Val::Frame) => Val::Frame,
                    _ => Val::Top,
                };
                state.insert(a.to_string(), h);
            }
        }
    }
    if let Some((_, v, p)) = parse_store(rhs) {
        if tracked.contains(p) {
            let mut x = val_of(vals, v);
            if matches!(x, Val::Regs(..)) && regvars.insert(p.to_string()) {
                *changed = true;
            }
            if regvars.contains(p) && !matches!(x, Val::Regs(..)) {
                // a register-file variable (`regs`): whatever it is set to
                // may be the trace's file
                x = Val::Regs(None, true);
            }
            state.insert(p.to_string(), x);
        }
        return;
    }
    let Some(d) = d else { return };
    let v = eval(rhs, state, vals, tracked);
    let old = vals.get(d).copied().unwrap_or(Val::Bot);
    if old != v {
        vals.insert(d.to_string(), v);
        *changed = true;
    }
}

fn eval(rhs: &str, state: &HashMap<String, Val>, vals: &HashMap<String, Val>, tracked: &HashSet<String>) -> Val {
    let op = rhs.split(' ').next().unwrap_or("");
    match op {
        "alloca" => Val::Local,
        "load" => {
            let Some((_, p)) = parse_load(rhs) else { return Val::Top };
            if tracked.contains(p) {
                return state.get(p).copied().unwrap_or(Val::Top);
            }
            Val::Top
        }
        "getelementptr" => {
            let Some((ty, b, idx)) = parse_gep(rhs) else { return Val::Top };
            match val_of(vals, b) {
                Val::Regs(o, m) => {
                    let off = match (o, ty_size(ty), idx.as_slice()) {
                        (Some(o), Some(sz), [i]) => match val_of(vals, i) {
                            Val::Int(x) => Some(o + (x as i64).wrapping_mul(sz)),
                            _ => None,
                        },
                        _ => None,
                    };
                    Val::Regs(off, m)
                }
                Val::Pc => Val::Pc,
                Val::Local => Val::Local,
                Val::Frame => Val::Frame,
                Val::Bot => Val::Bot,
                _ => Val::Top,
            }
        }
        "add" | "sub" | "mul" | "shl" | "lshr" | "ashr" | "and" | "or" | "xor" => {
            let r = rhs[op.len()..].trim_start();
            let r = r.trim_start_matches("nuw ").trim_start_matches("nsw ").trim_start_matches("exact ").trim_start_matches("disjoint ");
            let r = r.trim_start_matches("nuw ").trim_start_matches("nsw ");
            let parts = split_top(r, b',');
            if parts.len() != 2 {
                return Val::Top;
            }
            let a = last_tok(parts[0]);
            let ty = parts[0][..parts[0].len() - a.len()].trim();
            let Some(bits) = int_bits(ty) else { return Val::Top };
            match (val_of(vals, a), val_of(vals, parts[1])) {
                (Val::Int(x), Val::Int(y)) => {
                    let m = mask(bits);
                    let sx = if bits < 64 && (x >> (bits - 1)) & 1 == 1 { (x | !m) as i64 } else { x as i64 };
                    let r = match op {
                        "add" => x.wrapping_add(y),
                        "sub" => x.wrapping_sub(y),
                        "mul" => x.wrapping_mul(y),
                        "shl" => if y < bits as u64 { x << y } else { return Val::Top },
                        "lshr" => if y < bits as u64 { (x & m) >> y } else { return Val::Top },
                        "ashr" => if y < bits as u64 { (sx >> y) as u64 } else { return Val::Top },
                        "and" => x & y,
                        "or" => x | y,
                        _ => x ^ y,
                    };
                    Val::Int(r & m)
                }
                (Val::Bot, _) | (_, Val::Bot) => Val::Bot,
                _ => Val::Top,
            }
        }
        "zext" | "sext" | "trunc" => {
            let r = rhs[op.len()..].trim_start().trim_start_matches("nuw ").trim_start_matches("nsw ");
            let Some((src, to)) = r.split_once(" to ") else { return Val::Top };
            let a = last_tok(src);
            let (Some(fb), Some(tb)) = (int_bits(&src[..src.len() - a.len()]), int_bits(to)) else { return Val::Top };
            match val_of(vals, a) {
                Val::Int(x) => {
                    let x = x & mask(fb);
                    Val::Int(match op {
                        "sext" if fb < 64 && (x >> (fb - 1)) & 1 == 1 => (x | !mask(fb)) & mask(tb),
                        _ => x & mask(tb),
                    })
                }
                Val::Bot => Val::Bot,
                _ => Val::Top,
            }
        }
        "bitcast" | "addrspacecast" => {
            let r = rhs[op.len()..].trim_start();
            let Some((src, _)) = r.split_once(" to ") else { return Val::Top };
            val_of(vals, last_tok(src))
        }
        "select" => {
            let parts = split_top(&rhs[op.len()..], b',');
            if parts.len() != 3 {
                return Val::Top;
            }
            join(val_of(vals, last_tok(parts[1])), val_of(vals, last_tok(parts[2])))
        }
        _ => Val::Top,
    }
}

// ---------------------------------------------------------------- emission

/// Profile counts of a region build, by step pc and block label.
struct Prof {
    blocks: HashMap<(usize, String), u64>,
    /// The counts before the minimum fraction (for the report).
    raw: HashMap<(usize, String), u64>,
    /// A block counts as on the trace if run at least this many times.
    min: u64,
}

/// Per-trace globals the runtime knows (`struct loimi_trace.state`):
/// [0] invalidations, [1] guard failures, [2] iterations, [3] failures
/// at which to ask the runtime again.
const ST_GEN: usize = 0;
const ST_FAILS: usize = 1;
const ST_ITERS: usize = 2;
const ST_LIMIT: usize = 3;

struct Emit {
    out: Vec<String>,
    cur: String,
    ctr: usize,
    /// step pc -> incoming (block, state registers, iteration count)
    heads_in: BTreeMap<usize, Vec<(String, Vec<String>, String)>>,
}

impl Emit {
    fn fresh(&mut self, p: &str) -> String {
        self.ctr += 1;
        format!("%x{}.{}", self.ctr, p)
    }
    fn line(&mut self, s: String) {
        self.out.push(s);
    }
    fn label(&mut self, l: &str) {
        self.out.push(format!("{}:", l));
        self.cur = l.to_string();
    }
}

/// What a generated function is.
#[derive(Clone, Copy, PartialEq)]
enum Kind {
    /// Every instruction of the loop, every block counted.
    Region,
    /// One path around the loop, with guards.
    Trace,
}

struct FnGen<'t, 'a> {
    t: &'t mut Trace<'a>,
    kind: Kind,
    name: String,
    index: usize,
    /// The path (trace) or the region's instructions, in order.
    order: Vec<usize>,
    /// Trace: the next step of each.
    succ: HashMap<usize, usize>,
    prof: Option<Prof>,
    nr: usize,
    e: Emit,
    allocas: Vec<String>,
    guards: Vec<(String, usize)>,
    blockmap: Vec<(usize, String)>,
    edges: Vec<(usize, usize, usize)>,
    check: bool,
    nguards: usize,
    neffect: usize,
    /// Blocks emitted out of line (writebacks on edges into real mode).
    side: Vec<String>,
    /// The next exit's iterations are counted already (guards).
    flushed: bool,
    /// LOIMI_TRACE_DEBUG: annotate accesses with what the analysis found.
    debug: bool,
    /// The guards, for the report.
    guard_info: Vec<String>,
    /// Window slots the steps access at constant offsets.
    used: BTreeSet<usize>,
    /// Some access to the register file has an unknown offset.
    dynamic: bool,
    /// Per step (by prefix): the window slots it may store to (its
    /// snapshot: what a guard restores).
    snapsets: HashMap<String, BTreeSet<usize>>,
    /// The window slots the step being emitted stores to.
    cur_stores: BTreeSet<usize>,
}

impl<'t, 'a> FnGen<'t, 'a> {
    fn st(&self, name: &str) -> String {
        format!("@\"loimi.trace.st.{}\"", name)
    }
    fn st_gep(&mut self, i: usize) -> String {
        let g = self.e.fresh("stp");
        let s = self.st(&self.name.clone());
        self.e.line(format!("  {} = getelementptr inbounds i64, ptr {}, i64 {}", g, s, i));
        g
    }

    /// The function's parameter for state register i.
    fn param(&self, i: usize) -> String {
        if i == self.t.pc_idx {
            "%tr.pc".into()
        } else if Some(i) == self.t.regs_idx {
            "%tr.regs".into()
        } else {
            format!("%tr.s{}", i)
        }
    }

    fn window_slot(&mut self, j: usize) -> String {
        let g = self.e.fresh("ws");
        self.e.line(format!("  {} = getelementptr inbounds i64, ptr %lregs, i64 {}", g, j));
        g
    }

    /// Copies the window between `from` and `to` (`%lregs`, `%snap`,
    /// `%tr.regs`).
    fn copy_window(&mut self, from: &str, to: &str) {
        if self.nr > 0 {
            // expanded by `expand_copy` once the whole function is known
            self.e.line(format!("@@COPY {} {}", from, to));
        }
    }

    /// A copy of window slots `slots` (`(slot, from)`) to `to`.
    fn copy_slots(&mut self, slots: &[(usize, &str)], to: &str, out: &mut Vec<String>) {
        for &(j, from) in slots {
            let (a, v, b) = (self.e.fresh("wa"), self.e.fresh("wv"), self.e.fresh("wb"));
            out.push(format!("  {} = getelementptr inbounds i64, ptr {}, i64 {}", a, from, j));
            out.push(format!("  {} = load i64, ptr {}", v, a));
            out.push(format!("  {} = getelementptr inbounds i64, ptr {}, i64 {}", b, to, j));
            out.push(format!("  store i64 {}, ptr {}", v, b));
        }
    }

    /// Expands the window copies (`@@COPY from to`, `@@SNAP`, `@@GUARD`)
    /// once the whole function is known.
    ///
    /// Regions (many exits) and functions with an access at an unknown
    /// offset (the window is memory there anyway) copy the whole window
    /// with one memcpy. Otherwise copies name slots one by one, which
    /// SROA promotes each (a memcpy would leave the slots no step names
    /// as memory it copies on every iteration), and only the slots that
    /// need it: a flow analysis over the function's blocks finds the
    /// slots that may differ from memory (*dirty*: stored to, `@@DIRTY`,
    /// since the last writeback or reload); a writeback copies those; a
    /// step's snapshot (`@@SNAP`) copies the slots the step stores to; a
    /// guard (`@@GUARD`) restores the slots dirty at its step's start
    /// and those the step stores to, as they were at its start.
    fn expand_markers(&mut self, raw: Vec<String>) -> Vec<String> {
        let full = self.kind == Kind::Region || self.dynamic;
        let memcpy = |from: &str, to: &str, nr: usize| format!("  call void @llvm.memcpy.p0.p0.i64(ptr align 8 {}, ptr align 8 {}, i64 {}, i1 false)", to, from, 8 * nr);
        // blocks: (label, first line, end)
        let mut blocks: Vec<(String, usize, usize)> = vec![("entry".into(), 0, raw.len())];
        for (i, l) in raw.iter().enumerate() {
            if let Some(lab) = label_of(l) {
                blocks.last_mut().unwrap().2 = i;
                blocks.push((lab.trim_matches('"').to_string(), i + 1, raw.len()));
            }
        }
        let index: HashMap<&str, usize> = blocks.iter().enumerate().map(|(i, b)| (b.0.as_str(), i)).collect();
        let nb = blocks.len();
        let mut succs: Vec<Vec<usize>> = vec![Vec::new(); nb];
        for (b, &(_, lo, hi)) in blocks.iter().enumerate() {
            let Some(t) = raw[lo..hi].iter().rev().find(|l| !l.starts_with("@@")) else { continue };
            for part in t.split("label %").skip(1) {
                if let Some((nm, _, _)) = name_at(part, 0) {
                    if let Some(&s) = index.get(nm) {
                        succs[b].push(s);
                    }
                }
            }
        }
        // the flow: dirty slots at each block's start
        let mut din: Vec<Option<Bits>> = vec![None; nb];
        if !full {
            din[0] = Some(Bits::default());
            let mut work: Vec<usize> = vec![0];
            while let Some(b) = work.pop() {
                let mut d = din[b].unwrap();
                for l in &raw[blocks[b].1..blocks[b].2] {
                    if let Some(j) = l.strip_prefix("@@DIRTY ") {
                        d.set(j.parse().unwrap());
                    } else if l.starts_with("@@COPY ") && (l.ends_with(" %tr.regs") || l.ends_with(" %lregs")) {
                        // a writeback or a reload: the window is memory
                        d = Bits::default();
                    }
                }
                for &s in &succs[b] {
                    let n = match din[s] {
                        None => d,
                        Some(o) => o.or(&d),
                    };
                    if din[s] != Some(n) {
                        din[s] = Some(n);
                        work.push(s);
                    }
                }
            }
        }
        let head_dirty: HashMap<String, Bits> = blocks.iter().zip(&din).filter_map(|(b, d)| b.0.strip_suffix("head").map(|p| (p.to_string(), d.unwrap_or_default()))).collect();
        let used: Vec<usize> = self.used.iter().copied().collect();
        let mut out = Vec::with_capacity(raw.len() + 1024);
        for (b, &(_, lo, hi)) in blocks.iter().enumerate() {
            if lo > 0 {
                out.push(raw[lo - 1].clone());
            }
            let mut d = din[b].unwrap_or_default();
            for l in &raw[lo..hi] {
                if let Some(j) = l.strip_prefix("@@DIRTY ") {
                    d.set(j.parse().unwrap());
                } else if let Some(r) = l.strip_prefix("@@COPY ") {
                    let (from, to) = r.split_once(' ').unwrap();
                    if full {
                        out.push(memcpy(from, to, self.nr));
                    } else if to == "%tr.regs" {
                        let s: Vec<(usize, &str)> = d.iter().map(|j| (j, from)).collect();
                        self.copy_slots(&s, to, &mut out);
                    } else {
                        let s: Vec<(usize, &str)> = used.iter().map(|&j| (j, from)).collect();
                        self.copy_slots(&s, to, &mut out);
                    }
                    if to == "%tr.regs" || to == "%lregs" {
                        d = Bits::default();
                    }
                } else if let Some(p) = l.strip_prefix("@@SNAP ") {
                    if full {
                        out.push(memcpy("%lregs", "%snap", self.nr));
                    } else {
                        let s: Vec<(usize, &str)> = self.snapsets.get(p).into_iter().flatten().map(|&j| (j, "%lregs")).collect();
                        self.copy_slots(&s, "%snap", &mut out);
                    }
                } else if let Some(p) = l.strip_prefix("@@GUARD ") {
                    if full {
                        out.push(memcpy("%snap", "%tr.regs", self.nr));
                    } else {
                        let ss = self.snapsets.get(p).cloned().unwrap_or_default();
                        let mut all = head_dirty.get(p).copied().unwrap_or_default();
                        ss.iter().for_each(|&j| all.set(j));
                        let s: Vec<(usize, &str)> = all.iter().map(|j| (j, if ss.contains(&j) { "%snap" } else { "%lregs" })).collect();
                        self.copy_slots(&s, "%tr.regs", &mut out);
                    }
                } else {
                    out.push(l.clone());
                }
            }
        }
        out
    }
    fn writeback(&mut self) {
        self.copy_window("%lregs", "%tr.regs");
    }
    fn reload(&mut self) {
        self.copy_window("%tr.regs", "%lregs");
    }

    /// Adds the iterations of this entry to the trace's count (at exits).
    fn flush_iters(&mut self) {
        if self.kind != Kind::Trace || std::mem::take(&mut self.flushed) {
            return;
        }
        let g = self.st_gep(ST_ITERS);
        let (a, b) = (self.e.fresh("it"), self.e.fresh("it"));
        self.e.line(format!("  {} = load i64, ptr {}", a, g));
        self.e.line(format!("  {} = add i64 {}, %tr.it", b, a));
        self.e.line(format!("  store i64 {}, ptr {}", b, g));
    }

    /// Leaves through `callee` (a handler or the table's entry) with
    /// `args` (the state registers, then the instruction word), the
    /// window written back if this block is in window mode.
    fn exit_call(&mut self, callee: &str, args: &[String], window: bool) {
        if window {
            self.writeback();
        }
        self.flush_iters();
        let n = self.t.sig.len();
        let mut a: Vec<String> = (0..n).map(|i| format!("{} {}", self.t.sig[i], args[i])).collect();
        a.push(format!("i32 {}", args[n]));
        if self.t.ret == "void" {
            self.e.line(format!("  musttail call preserve_nonecc void {}({})", callee, a.join(", ")));
            self.e.line("  ret void".into());
        } else {
            let r = self.e.fresh("ret");
            self.e.line(format!("  {} = musttail call preserve_nonecc {} {}({})", r, self.t.ret, callee, a.join(", ")));
            self.e.line(format!("  ret {} {}", self.t.ret, r));
        }
    }

    /// Dispatches through the table at the pc of `state`.
    fn exit_dispatch(&mut self, state: &[String], window: bool) {
        let pcv = state[self.t.pc_idx].clone();
        let (w, o, oz, g, h) = (self.e.fresh("w"), self.e.fresh("o"), self.e.fresh("oz"), self.e.fresh("g"), self.e.fresh("h"));
        self.e.line(format!("  {} = load i32, ptr {}", w, pcv));
        self.e.line(format!("  {} = and i32 {}, {}", o, w, self.t.op_mask));
        self.e.line(format!("  {} = zext i32 {} to i64", oz, o));
        self.e.line(format!("  {} = getelementptr inbounds ptr, ptr @\"{}\", i64 {}", g, self.t.table, oz));
        self.e.line(format!("  {} = load ptr, ptr {}", h, g));
        let mut args = state.to_vec();
        args.push(w);
        self.exit_call(&h, &args, window);
    }

    /// Runs the instruction at the pc of `state`, whose word is `word`,
    /// in the interpreter: through the table at the word's op, not the
    /// word in memory (an enter word, which at the loop header leads back
    /// into this trace).
    fn exit_interp(&mut self, state: &[String], word: u32) {
        let (g, h) = (self.e.fresh("g"), self.e.fresh("h"));
        self.e.line(format!("  {} = getelementptr inbounds ptr, ptr @\"{}\", i64 {}", g, self.t.table, word as u64 & self.t.op_mask));
        self.e.line(format!("  {} = load ptr, ptr {}", h, g));
        let mut args = state.to_vec();
        args.push(i32s(word).to_string());
        self.exit_call(&h, &args, false);
    }

    fn pc_of(&mut self, t: usize) -> String {
        let pcv = self.e.fresh("xpc");
        self.e.line(format!("  {} = getelementptr inbounds i32, ptr %tr.pc, i64 {}", pcv, t as i64 - self.t.h as i64));
        pcv
    }

    /// Continues at step t with `state` if `regs` is still the trace's
    /// file (and, after calls that may rewrite code, the trace still
    /// valid); otherwise exits there.
    fn goto_head(&mut self, t: usize, state: &[String], window: bool, check_gen: bool) {
        let mut cond = "true".to_string();
        if let Some(ri) = self.t.regs_idx {
            cond = self.e.fresh("same");
            self.e.line(format!("  {} = icmp eq ptr {}, %tr.regs", cond, state[ri]));
        }
        if check_gen {
            let g = self.st_gep(ST_GEN);
            let (gv, ok, both) = (self.e.fresh("gen"), self.e.fresh("ok"), self.e.fresh("ok"));
            self.e.line(format!("  {} = load i64, ptr {}", gv, g));
            self.e.line(format!("  {} = icmp eq i64 {}, %tr.gen0", ok, gv));
            self.e.line(format!("  {} = and i1 {}, {}", both, cond, ok));
            cond = both;
        }
        let go = self.e.fresh("go")[1..].to_string();
        let no = self.e.fresh("leave")[1..].to_string();
        self.e.line(format!("  br i1 {}, label %{}, label %{}", cond, go, no));
        self.e.label(&go);
        if !window {
            self.reload();
        }
        let it = if t == self.t.h && self.kind == Kind::Trace {
            let n = self.e.fresh("itn");
            self.e.line(format!("  {} = add i64 %tr.it, 1", n));
            n
        } else {
            "%tr.it".to_string()
        };
        self.e.line(format!("  br label %s{}.head", t));
        let cur = self.e.cur.clone();
        self.e.heads_in.entry(t).or_default().push((cur, state.to_vec(), it));
        self.e.label(&no);
        let mut st = state.to_vec();
        st[self.t.pc_idx] = self.pc_of(t);
        self.exit_dispatch(&st, window);
    }

    fn counter(&mut self, idx: usize) {
        let (x, y, z) = (self.e.fresh("cp"), self.e.fresh("cv"), self.e.fresh("cn"));
        self.e.line(format!("  {} = getelementptr inbounds i64, ptr @\"loimi.trace.counts.{}\", i64 {}", x, self.name, idx));
        self.e.line(format!("  {} = load i64, ptr {}", y, x));
        self.e.line(format!("  {} = add i64 {}, 1", z, y));
        self.e.line(format!("  store i64 {}, ptr {}", z, x));
    }

    fn internal(&self, from: usize, t: usize) -> bool {
        match self.kind {
            Kind::Trace => self.succ.get(&from) == Some(&t),
            Kind::Region => self.t.at.contains_key(&t) && self.order.contains(&t),
        }
    }

    fn classify_call(&self, st: &Step, c: &Call) -> CallK {
        let ptr_args = || c.args.iter().filter(|a| a.0.starts_with("ptr")).map(|a| val_of(&st.vals, a.1));
        if c.asm {
            return if c.sideeffect { CallK::Unsafe } else { CallK::Pure };
        }
        if !c.global {
            return CallK::Unsafe;
        }
        if let Some(i) = c.callee.strip_prefix("llvm.") {
            if !effectful_intrinsic(i) {
                return CallK::Pure;
            }
            return if ptr_args().all(|v| v == Val::Local) { CallK::Pure } else { CallK::Unsafe };
        }
        if c.callee == "loimi_trace_check" {
            return CallK::Pure;
        }
        // read-only first: a write barrier's test (`std.gc.needs_barrier`)
        // only reads, and needs no writeback unless given the registers
        if self.t.m.readonly(c.callee) {
            return CallK::ReadOnly;
        }
        if redo_exempt(c.callee) {
            return CallK::Exempt;
        }
        CallK::Unsafe
    }

    /// Whether a store (to a pointer of class v, of type ty) may be seen
    /// by a re-execution of the instruction.
    fn store_effect(&self, v: Val, ty: &str) -> bool {
        match v {
            Val::Local | Val::Pc => false,
            Val::Regs(Some(o), false) => !self.in_window(o, ty),
            _ => true,
        }
    }

    fn in_window(&self, o: i64, ty: &str) -> bool {
        o >= 0 && o % 8 == 0 && ty_size(ty) == Some(8) && ((o / 8) as usize) < self.nr
    }

    fn on(&self, pc: usize, lab: &str) -> bool {
        match &self.prof {
            None => true,
            Some(p) => p.blocks.get(&(pc, lab.to_string())).copied().unwrap_or(0) >= p.min.max(1),
        }
    }

    /// Emits step `si` (index into steps).
    fn emit_step(&mut self, si: usize) -> Result<(), String> {
        let st = std::mem::replace(&mut self.t.steps[si], Step { pc: 0, blocks: vec![], succs: vec![], vals: HashMap::new(), allocas: HashSet::new(), statevars: HashSet::new() });
        let r = self.emit_step_inner(&st);
        self.t.steps[si] = st;
        r
    }

    fn emit_step_inner(&mut self, st: &Step) -> Result<(), String> {
        let pc = st.pc;
        let p = format!("s{}.", pc);
        let nb = st.blocks.len();
        // per block: effects (from the step's start) and calls
        let mut has_effect = vec![false; nb];
        let mut writes = vec![false; nb];
        let mut rewrites = false;
        for (b, (_, ls)) in st.blocks.iter().enumerate() {
            for l in ls {
                let (_, rhs) = def_of(l);
                if let Some((ty, _, ptr)) = parse_store(rhs) {
                    if self.store_effect(val_of(&st.vals, ptr), ty) {
                        has_effect[b] = true;
                    }
                } else if rhs.starts_with("atomicrmw ") || rhs.starts_with("cmpxchg ") || rhs.starts_with("fence") {
                    has_effect[b] = true;
                    writes[b] = true;
                } else if !rhs.starts_with("musttail") {
                    if let Some(c) = parse_call(rhs) {
                        match self.classify_call(st, &c) {
                            CallK::Unsafe => {
                                has_effect[b] = true;
                                writes[b] = true;
                                rewrites = true;
                            }
                            CallK::Exempt => rewrites = true,
                            _ => {}
                        }
                    }
                }
            }
        }
        let on: Vec<bool> = st.blocks.iter().map(|(lab, _)| self.on(pc, lab)).collect();
        // effects before the end of each on-trace block, over on-trace paths
        let mut eff_out = vec![false; nb];
        let mut changed = true;
        while changed {
            changed = false;
            for b in 0..nb {
                if !on[b] {
                    continue;
                }
                let ein = (0..nb).any(|q| on[q] && st.succs[q].contains(&b) && eff_out[q]);
                let e = ein || has_effect[b];
                if e != eff_out[b] {
                    eff_out[b] = e;
                    changed = true;
                }
            }
        }
        let mut guard_edges: HashSet<(usize, usize)> = HashSet::new();
        if self.kind == Kind::Trace {
            for b in 0..nb {
                if on[b] && !eff_out[b] {
                    for &s in &st.succs[b] {
                        if !on[s] {
                            guard_edges.insert((b, s));
                            let cnt = |l: &str| self.prof.as_ref().and_then(|p| p.raw.get(&(pc, l.to_string())).copied()).unwrap_or(0);
                            self.guard_info.push(format!("{}: {} ({}) -> {} ({})", pc, st.blocks[b].0, cnt(&st.blocks[b].0), st.blocks[s].0, cnt(&st.blocks[s].0)));
                        }
                    }
                }
            }
        }
        let mut reach = vec![false; nb];
        let mut work = vec![0usize];
        while let Some(b) = work.pop() {
            if reach[b] {
                continue;
            }
            reach[b] = true;
            work.extend(st.succs[b].iter().copied().filter(|&s| !guard_edges.contains(&(b, s))));
        }
        // window or real mode at each block's start (real if any
        // reachable predecessor ends in real mode)
        let mut real_in = vec![false; nb];
        let mut changed = true;
        while changed {
            changed = false;
            for b in 0..nb {
                if !reach[b] {
                    continue;
                }
                let r = (0..nb).any(|q| reach[q] && st.succs[q].contains(&b) && !guard_edges.contains(&(q, b)) && (real_in[q] || writes[q]));
                if r && !real_in[b] {
                    real_in[b] = true;
                    changed = true;
                }
            }
        }
        if real_in[0] {
            return Err(format!("step {}: entry in real mode", pc));
        }
        let snap = !guard_edges.is_empty();
        if self.kind == Kind::Trace && snap {
            self.nguards += guard_edges.len();
        }
        if eff_out.iter().zip(&on).any(|(e, o)| *e && *o) {
            self.neffect += 1;
        }

        // the head: phis, the step's pc, the snapshot
        self.e.label(&format!("{}head", p));
        self.e.line(format!("@@PHIS {}", pc));
        self.e.line(format!("  %{}pc = getelementptr inbounds i32, ptr %tr.pc, i64 {}", p, pc as i64 - self.t.h as i64));
        if snap && self.nr > 0 {
            // the slots this step stores to (known once it is emitted)
            self.e.line(format!("@@SNAP {}", p));
        }
        self.cur_stores.clear();
        self.e.line(format!("  br label %{}{}", p, st.blocks[0].0));
        let mut table_geps: HashSet<String> = HashSet::new();
        let mut table_fns: HashSet<String> = HashSet::new();
        let mut guard_ix = 0;
        for (b, (lab, ls)) in st.blocks.iter().enumerate() {
            if !reach[b] {
                for l in ls {
                    if def_of(l).1.starts_with("alloca ") {
                        self.allocas.push(l.clone());
                    }
                }
                continue;
            }
            self.e.label(&format!("{}{}", p, lab));
            let idx = self.blockmap.len();
            self.blockmap.push((pc, lab.clone()));
            if self.kind == Kind::Region {
                self.counter(idx);
            }
            let mut window = !real_in[b];
            let mut skip_ret = false;
            for l in ls {
                if skip_ret {
                    skip_ret = false;
                    if l.trim().starts_with("ret ") {
                        continue;
                    }
                }
                let (d, rhs) = def_of(l);
                if rhs.starts_with("alloca ") {
                    self.allocas.push(l.clone());
                    continue;
                }
                if let Some(d) = d {
                    if let Some((_, base, _)) = parse_gep(rhs) {
                        if base == format!("@\"{}\"", self.t.table) {
                            table_geps.insert(d.to_string());
                        }
                    }
                    if let Some((_, ptr)) = parse_load(rhs) {
                        if table_geps.contains(ptr) {
                            table_fns.insert(d.to_string());
                        }
                    }
                }
                if rhs.starts_with("musttail call") {
                    let c = parse_call(rhs).ok_or_else(|| format!("step {}: bad musttail", pc))?;
                    let args: Vec<String> = c.args.iter().map(|x| x.1.to_string()).collect();
                    let ns = self.t.sig.len();
                    if args.len() != ns + 1 {
                        return Err(format!("step {}: a continuation with {} arguments", pc, args.len()));
                    }
                    let state: Vec<String> = args[..ns].to_vec();
                    skip_ret = true;
                    if c.global {
                        if let Some(t) = cont_target(self.t.code, pc, c.callee) {
                            // `next` or a label jump: to the next step, or an exit
                            match t {
                                Some(t) if on[b] && self.internal(pc, t) => {
                                    self.edges.push((pc, idx, t));
                                    self.goto_head(t, &state, window, rewrites);
                                }
                                Some(t) => {
                                    let mut st2 = state.clone();
                                    st2[self.t.pc_idx] = self.pc_of(t);
                                    self.exit_dispatch(&st2, window);
                                }
                                None => return Err(format!("step {}: a jump outside the code", pc)),
                            }
                            continue;
                        }
                        self.exit_call(&format!("@\"{}\"", c.callee), &args, window);
                        continue;
                    }
                    if table_fns.contains(c.callee) && on[b] {
                        // a dispatch through the table (an inlined cold
                        // block's `next`): to the next step(s) by pc
                        let (x1, x2, dd) = (self.e.fresh("dp"), self.e.fresh("db"), self.e.fresh("doff"));
                        self.e.line(format!("  {} = ptrtoint ptr {} to i64", x1, args[self.t.pc_idx]));
                        self.e.line(format!("  {} = ptrtoint ptr %tr.pc to i64", x2));
                        self.e.line(format!("  {} = sub i64 {}, {}", dd, x1, x2));
                        let targets: Vec<usize> = match self.kind {
                            Kind::Trace => self.succ.get(&pc).copied().into_iter().collect(),
                            Kind::Region => self.order.clone(),
                        };
                        let xl = self.e.fresh("dexit")[1..].to_string();
                        let cases: Vec<(i64, String, usize)> = targets.iter().map(|&t| (4 * (t as i64 - self.t.h as i64), self.e.fresh("dedge")[1..].to_string(), t)).collect();
                        let sw: Vec<String> = cases.iter().map(|(c, l, _)| format!("i64 {}, label %{}", c, l)).collect();
                        self.e.line(format!("  switch i64 {}, label %{} [ {} ]", dd, xl, sw.join(" ")));
                        for (_, el, t) in cases {
                            self.e.label(&el);
                            let ei = self.blockmap.len();
                            self.blockmap.push((pc, format!("{}@{}", lab, t)));
                            if self.kind == Kind::Region {
                                self.counter(ei);
                            }
                            self.edges.push((pc, ei, t));
                            self.goto_head(t, &state, window, rewrites);
                        }
                        self.e.label(&xl);
                    }
                    self.exit_call(c.callee, &args, window);
                    continue;
                }
                // memory accesses through register-file pointers
                let access = parse_load(rhs).map(|(ty, p)| (ty, p, false)).or_else(|| parse_store(rhs).map(|(ty, _, p)| (ty, p, true)));
                if let Some((ty, ptr, is_store)) = access {
                    let v = val_of(&st.vals, ptr);
                    if self.debug {
                        self.e.line(format!("  ; {:?} window={}", v, window));
                    }
                    match v {
                        Val::Regs(Some(o), false) if window && self.in_window(o, ty) => {
                            let j = (o / 8) as usize;
                            let s = self.window_slot(j);
                            self.e.line(replace_ptr(l, ptr, &s));
                            if is_store {
                                // the slot now differs from memory
                                self.e.line(format!("@@DIRTY {}", j));
                                self.cur_stores.insert(j);
                            }
                            continue;
                        }
                        Val::Regs(Some(_), false) => {}
                        Val::Regs(..) if window => {
                            self.dynamic = true;
                            // unknown offset: the window if it is in it
                            let (x, y, dd, q, g, sel) = (self.e.fresh("ri"), self.e.fresh("rl"), self.e.fresh("rd"), self.e.fresh("rin"), self.e.fresh("rg"), self.e.fresh("rp"));
                            self.e.line(format!("  {} = ptrtoint ptr {} to i64", x, ptr));
                            self.e.line(format!("  {} = ptrtoint ptr %tr.regs to i64", y));
                            self.e.line(format!("  {} = sub i64 {}, {}", dd, x, y));
                            self.e.line(format!("  {} = icmp ult i64 {}, {}", q, dd, 8 * self.nr));
                            self.e.line(format!("  {} = getelementptr i8, ptr %lregs, i64 {}", g, dd));
                            self.e.line(format!("  {} = select i1 {}, ptr {}, ptr {}", sel, q, g, ptr));
                            self.e.line(replace_ptr(l, ptr, &sel));
                            continue;
                        }
                        Val::Top | Val::Frame | Val::Bot if window && self.check => {
                            self.e.line(format!("  call void @loimi_trace_check(ptr {}, ptr %tr.regs, i64 {})", ptr, self.nr));
                        }
                        _ => {}
                    }
                    self.e.line(l.clone());
                    continue;
                }
                if let Some(c) = parse_call(rhs) {
                    let k = self.classify_call(st, &c);
                    if self.debug {
                        self.e.line(format!("  ; call {:?} window={} on={}", k, window, on[b]));
                    }
                    // a pointer into the register file, a state register
                    // through which the frames are reached, or a local
                    // holding one (heap objects do not reach the running
                    // frame's registers: an assumption, section 11.3)
                    let reaches = c.args.iter().any(|a| a.0.starts_with("ptr") && (st.statevars.contains(a.1) || matches!(val_of(&st.vals, a.1), Val::Regs(..) | Val::Frame)));
                    match k {
                        CallK::Pure => self.e.line(l.clone()),
                        CallK::ReadOnly | CallK::Exempt => {
                            if window && (k == CallK::Exempt || reaches) {
                                self.writeback();
                            }
                            self.e.line(l.clone());
                        }
                        CallK::Unsafe => {
                            if window {
                                self.writeback();
                            }
                            self.e.line(l.clone());
                            window = false;
                        }
                    }
                    continue;
                }
                if rhs.starts_with("br i1 ") {
                    let parts: Vec<&str> = rhs.split("label ").collect();
                    if parts.len() == 3 {
                        let mut labs = Vec::new();
                        // metadata after the labels (`, !prof !3`) stays
                        let last = parts[2].trim();
                        let suffix = &last[last.find([',', ' ']).unwrap_or(last.len())..];
                        for part in &parts[1..] {
                            let tok = part.trim().split([',', ' ']).next().unwrap_or("");
                            let tb = st.blocks.iter().position(|bb| format!("%{}{}", p, bb.0) == tok).ok_or_else(|| format!("step {}: bad branch `{}`", pc, rhs))?;
                            if guard_edges.contains(&(b, tb)) {
                                guard_ix += 1;
                                let g = format!("guard.{}.{}", pc, guard_ix);
                                self.guards.push((g.clone(), pc));
                                labs.push(format!("%{}", g));
                            } else if !window && !real_in[tb] {
                                return Err(format!("step {}: real-mode edge into a window-mode block", pc));
                            } else if window && real_in[tb] {
                                labs.push(self.wb_edge(tok));
                            } else {
                                labs.push(tok.to_string());
                            }
                        }
                        self.e.line(format!("  {}label {}, label {}{}", parts[0], labs[0], labs[1], suffix));
                        continue;
                    }
                }
                if rhs.starts_with("br label ") {
                    let tok = rhs["br label ".len()..].trim().split([',', ' ']).next().unwrap_or("");
                    let tb = st.blocks.iter().position(|bb| format!("%{}{}", p, bb.0) == tok).ok_or_else(|| format!("step {}: bad branch `{}`", pc, rhs))?;
                    if window && real_in[tb] {
                        let w = self.wb_edge(tok);
                        self.e.line(format!("  br label {}", w));
                        continue;
                    }
                    if !window && !real_in[tb] {
                        return Err(format!("step {}: real-mode edge into a window-mode block", pc));
                    }
                }
                if rhs.starts_with("atomicrmw ") || rhs.starts_with("cmpxchg ") || rhs.starts_with("fence") {
                    if window {
                        self.writeback();
                    }
                    self.e.line(l.clone());
                    window = false;
                    continue;
                }
                self.e.line(l.clone());
            }
        }
        self.snapsets.insert(p, std::mem::take(&mut self.cur_stores));
        Ok(())
    }

    /// An edge from a window-mode block into a real-mode one: a block
    /// that writes the window back. Returns its label (with `%`).
    fn wb_edge(&mut self, to: &str) -> String {
        let l = self.e.fresh("wbe")[1..].to_string();
        // emitted out of line, after the function's blocks
        let save = std::mem::take(&mut self.e.out);
        let cur = self.e.cur.clone();
        self.e.label(&l);
        self.writeback();
        self.e.line(format!("  br label {}", to));
        let blk = std::mem::replace(&mut self.e.out, save);
        self.e.cur = cur;
        self.side.extend(blk);
        format!("%{}", l)
    }
}

/// `line` with its memory access's pointer operand replaced.
fn replace_ptr(line: &str, ptr: &str, by: &str) -> String {
    let pat = format!("ptr {}", ptr);
    match line.rfind(&pat) {
        Some(i) if line[i + pat.len()..].starts_with(',') || line[i + pat.len()..].is_empty() => {
            format!("{}ptr {}{}", &line[..i], by, &line[i + pat.len()..])
        }
        _ => line.to_string(),
    }
}

// ---------------------------------------------------------------- functions

/// Escapes text for a `c"..."` constant.
fn c_string(s: &str) -> String {
    let mut o = String::with_capacity(s.len() + 8);
    for b in s.bytes() {
        if (b' '..=b'~').contains(&b) && b != b'"' && b != b'\\' {
            o.push(b as char);
        } else {
            let _ = write!(o, "\\{:02X}", b);
        }
    }
    o
}

/// Options of a trace or region build.
pub struct Options {
    /// Check every access through a pointer the analysis does not know
    /// (`loimi_trace_check`): it must not be in the window.
    pub check: bool,
    /// A loop gets a trace if its path closed this many times.
    pub min_iters: u64,
    /// ... and this many times per entry, on average (a trace's entry
    /// and exits cost a load and a store per register it caches, which
    /// short runs do not repay).
    pub min_run: f64,
    /// A block is on the trace if it ran at least this fraction of its
    /// step's runs (and at least once).
    pub min_frac: f64,
    /// Guard failures between the runtime's blacklist checks.
    pub fail_limit: u64,
    /// Only loops with these headers (debugging; empty: all).
    pub only: Vec<usize>,
    /// Instructions left out of regions, as exits (debugging).
    pub skip: Vec<usize>,
    /// LOIMI_TRACE_REPORT=2: the report lists every guard.
    pub verbose: bool,
    /// Loops whose region or trace would be larger (lines of IR) get
    /// none (LOIMI_TRACE_MAXLINES; LLVM's time and memory grow with it).
    pub max_lines: usize,
    /// All regions or traces of a build together (LOIMI_TRACE_MAXTOTAL):
    /// loops past it get none.
    pub max_total: usize,
}

impl Options {
    /// From the environment: LOIMI_TRACE_CHECK=1, LOIMI_TRACE_MIN=n,
    /// LOIMI_TRACE_MINFRAC=x.
    pub fn from_env() -> Options {
        let var = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
        Options {
            check: var("LOIMI_TRACE_CHECK").is_some_and(|v| v != "0"),
            min_iters: var("LOIMI_TRACE_MIN").and_then(|v| v.parse().ok()).unwrap_or(1000),
            min_run: var("LOIMI_TRACE_MINRUN").and_then(|v| v.parse().ok()).unwrap_or(0.0),
            min_frac: var("LOIMI_TRACE_MINFRAC").and_then(|v| v.parse().ok()).unwrap_or(0.0),
            fail_limit: 1024,
            only: var("LOIMI_TRACE_ONLY").map(|v| v.split(',').filter_map(|x| x.trim().parse().ok()).collect()).unwrap_or_default(),
            verbose: var("LOIMI_TRACE_REPORT").is_some_and(|v| v == "2"),
            max_lines: var("LOIMI_TRACE_MAXLINES").and_then(|v| v.parse().ok()).unwrap_or(150_000),
            max_total: var("LOIMI_TRACE_MAXTOTAL").and_then(|v| v.parse().ok()).unwrap_or(3_000_000),
            skip: var("LOIMI_TRACE_SKIP").map(|v| v.split(',').filter_map(|x| x.trim().parse().ok()).collect()).unwrap_or_default(),
        }
    }
}

/// The vm a build stitches for.
pub struct VmInfo<'a> {
    /// The dispatch table's symbol (`vm.NAME.table`).
    pub table: &'a str,
    pub op_bits: u32,
    /// The handler arguments (context, state, accumulators) that are pc
    /// and the register file (`regs`, if the vm has one).
    pub nstate: usize,
    pub pc_idx: usize,
    pub regs_idx: Option<usize>,
    /// The constant pool's state register (`const` operands).
    pub const_idx: Option<usize>,
    /// Constant-pool values that are immediates (no heap pointer):
    /// (mask, value, ult) as `jit_variant`.
    pub imm: &'a [(u64, u64, bool)],
}

/// The highest word an instruction's stencil reads, + 1.
fn span_hi(m: &Module, code: &Code, pc: usize) -> usize {
    let mut hi = pc + 1;
    let Some(name) = stencil_name(code, pc) else { return hi };
    for l in m.bodies.get(name.as_str()).into_iter().flatten() {
        let mut s = *l;
        while let Some(i) = s.find("@\"__jit.") {
            let rest = &s[i + 8..];
            let mut it = rest.splitn(3, '.');
            if let (Some(k), Some(o)) = (it.next(), it.next()) {
                if matches!(k, "f" | "w" | "c" | "j") {
                    if let Ok(o) = o.trim_end_matches('"').parse::<usize>() {
                        hi = hi.max(pc + o + 1);
                    }
                }
            }
            s = &rest[1..];
        }
    }
    hi.min(code.n)
}

impl<'t, 'a> FnGen<'t, 'a> {
    #[allow(clippy::too_many_arguments)]
    fn new(t: &'t mut Trace<'a>, kind: Kind, name: String, index: usize, order: Vec<usize>, succ: HashMap<usize, usize>, prof: Option<Prof>, check: bool) -> Self {
        FnGen {
            t,
            kind,
            name,
            index,
            order,
            succ,
            prof,
            nr: 0,
            e: Emit { out: Vec::new(), cur: String::new(), ctr: 0, heads_in: BTreeMap::new() },
            allocas: Vec::new(),
            guards: Vec::new(),
            blockmap: Vec::new(),
            edges: Vec::new(),
            check,
            nguards: 0,
            neffect: 0,
            side: Vec::new(),
            flushed: false,
            debug: std::env::var_os("LOIMI_TRACE_DEBUG").is_some(),
            guard_info: Vec::new(),
            used: BTreeSet::new(),
            dynamic: false,
            snapsets: HashMap::new(),
            cur_stores: BTreeSet::new(),
        }
    }

    /// The window: every register slot a step accesses at a constant
    /// offset (as an 8-byte value), up to the largest; which slots those
    /// are; and whether some access has an unknown offset (then the
    /// window lives in memory, and copies of it are whole).
    fn window_size(&mut self) -> usize {
        if self.t.regs_idx.is_none() {
            return 0;
        }
        let mut nr = 0;
        for &pc in &self.order {
            let st = &self.t.steps[self.t.at[&pc]];
            for (_, ls) in &st.blocks {
                for l in ls {
                    let (_, rhs) = def_of(l);
                    let a = parse_load(rhs).or_else(|| parse_store(rhs).map(|(t, _, p)| (t, p)));
                    if let Some((ty, p)) = a {
                        match val_of(&st.vals, p) {
                            Val::Regs(Some(o), false) => {
                                if o >= 0 && o % 8 == 0 && ty_size(ty) == Some(8) && ((o / 8) as usize) < MAX_WINDOW {
                                    nr = nr.max(o as usize / 8 + 1);
                                    self.used.insert(o as usize / 8);
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
        nr.max(1)
    }

    /// The function's IR (definition only).
    fn emit(&mut self) -> Result<String, String> {
        self.nr = self.window_size();
        for pc in self.order.clone() {
            let si = self.t.at[&pc];
            self.emit_step(si)?;
        }
        // guard exits: the window as at the step's start, then the
        // instruction again in the interpreter
        let guards = std::mem::take(&mut self.guards);
        for (g, pc) in &guards {
            self.e.label(g);
            // this entry's iterations first: the runtime compares them
            self.flush_iters();
            let f = self.st_gep(ST_FAILS);
            let l = self.st_gep(ST_LIMIT);
            let (f0, f1, lv, hit) = (self.e.fresh("gf"), self.e.fresh("gf"), self.e.fresh("gl"), self.e.fresh("gh"));
            let (lb, lc) = (self.e.fresh("gb")[1..].to_string(), self.e.fresh("gc")[1..].to_string());
            self.e.line(format!("  {} = load i64, ptr {}", f0, f));
            self.e.line(format!("  {} = add i64 {}, 1", f1, f0));
            self.e.line(format!("  store i64 {}, ptr {}", f1, f));
            self.e.line(format!("  {} = load i64, ptr {}", lv, l));
            self.e.line(format!("  {} = icmp uge i64 {}, {}", hit, f1, lv));
            self.e.line(format!("  br i1 {}, label %{}, label %{}", hit, lb, lc));
            self.e.label(&lb);
            self.e.line(format!("  call void @loimi_trace_failing(i64 {})", self.index));
            self.e.line(format!("  br label %{}", lc));
            self.e.label(&lc);
            let p = format!("s{}.", pc);
            if self.nr > 0 {
                // the slots dirty at the step's start, as they were then
                self.e.line(format!("@@GUARD {}", p));
            }
            self.flushed = true;
            let state: Vec<String> = (0..self.t.sig.len()).map(|i| state_name(&p, i, self.t.pc_idx, self.t.regs_idx)).collect();
            let word = self.t.code.w[*pc];
            self.exit_interp(&state, word);
        }
        // phis
        let first = self.t.h;
        let mut res: Vec<String> = Vec::new();
        let out = std::mem::take(&mut self.e.out);
        for l in out {
            if let Some(pc) = l.strip_prefix("@@PHIS ") {
                let pc: usize = pc.parse().unwrap();
                let p = format!("s{}.", pc);
                let mut ins = self.e.heads_in.get(&pc).cloned().unwrap_or_default();
                let n = self.t.sig.len();
                let params: Vec<String> = (0..n).map(|i| self.param(i)).collect();
                if pc == first {
                    ins.insert(0, ("entry".into(), params.clone(), "0".into()));
                }
                let carried: Vec<usize> = (0..n).filter(|&i| i != self.t.pc_idx && Some(i) != self.t.regs_idx).collect();
                if ins.is_empty() {
                    for &i in &carried {
                        res.push(format!("  {} = bitcast {} {} to {}", state_name(&p, i, self.t.pc_idx, self.t.regs_idx), self.t.sig[i], params[i], self.t.sig[i]));
                    }
                    continue;
                }
                for &i in &carried {
                    let v: Vec<String> = ins.iter().map(|x| format!("[ {}, %{} ]", x.1[i], x.0)).collect();
                    res.push(format!("  {} = phi {} {}", state_name(&p, i, self.t.pc_idx, self.t.regs_idx), self.t.sig[i], v.join(", ")));
                }
                if pc == first && self.kind == Kind::Trace {
                    let v: Vec<String> = ins.iter().map(|x| format!("[ {}, %{} ]", x.2, x.0)).collect();
                    res.push(format!("  %tr.it = phi i64 {}", v.join(", ")));
                }
            } else {
                res.push(l);
            }
        }
        let mut f = String::new();
        let ps: Vec<String> = (0..self.t.sig.len()).map(|i| format!("{} {}", self.t.sig[i], self.param(i))).collect();
        let _ = writeln!(
            f,
            "define internal preserve_nonecc {} @\"loimi.trace.{}\"({}, i32 %tr.insn) nounwind{} \"no-builtin-memset\" \"no-builtin-memcpy\" \"no-builtin-memmove\" {{",
            self.t.ret,
            self.name,
            ps.join(", "),
            // regions are optimized too: `optnone` keeps every local, and the
            // optimizing register allocator, which `optnone` functions still
            // get in an -O3 module, took 2.75 times as long on them
            if self.kind == Kind::Region { " noinline" } else { "" }
        );
        f.push_str("entry:\n");
        for a in &self.allocas {
            f.push_str(a);
            f.push('\n');
        }
        let _ = writeln!(f, "  %lregs = alloca [{} x i64], align 8", self.nr);
        let _ = writeln!(f, "  %snap = alloca [{} x i64], align 8", self.nr);
        self.e.out.clear();
        self.reload();
        if self.kind == Kind::Region {
            self.e.line("  %tr.it = add i64 0, 0".into());
        }
        let g = self.st_gep(ST_GEN);
        self.e.line(format!("  %tr.gen0 = load i64, ptr {}", g));
        let entry = std::mem::take(&mut self.e.out);
        let side = std::mem::take(&mut self.side);
        let raw: Vec<String> = [entry, vec![format!("  br label %s{}.head", first)], res, side].into_iter().flatten().collect();
        let all = self.expand_markers(raw);
        for l in &all {
            f.push_str(l);
            f.push('\n');
        }
        f.push_str("}\n");
        Ok(f)
    }
}

/// What `generate_fn` returns: the function, its block map and edges
/// (region numbering), the constants baked in, the window size, guards,
/// steps with effects.
struct GenFn {
    ir: String,
    blockmap: Vec<(usize, String)>,
    edges: Vec<(usize, usize, usize)>,
    kchk: BTreeSet<(u64, u64)>,
    nr: usize,
    nguards: usize,
    neffect: usize,
    guard_info: Vec<String>,
    lines: usize,
}

/// The function `kind` for loop h of `code`, over `order`.
#[allow(clippy::too_many_arguments)]
fn generate_fn(m: &Module, vm: &VmInfo, code: &Code, h: usize, order: &[usize], kind: Kind, name: &str, index: usize, succ: HashMap<usize, usize>, prof: Option<Prof>, opts: &Options) -> Result<GenFn, String> {
    let mut t = Trace::new(m, code, vm, h);
    for (i, &pc) in order.iter().enumerate() {
        let st = t.build_step(pc).map_err(|e| format!("instruction {} ({}): {}", pc, code.op[pc].as_deref().unwrap_or("?"), e))?;
        t.steps.push(st);
        t.at.insert(pc, i);
    }
    let mut g = FnGen::new(&mut t, kind, name.to_string(), index, order.to_vec(), succ, prof, opts.check);
    let ir = g.emit()?;
    let lines = ir.bytes().filter(|&b| b == b'\n').count();
    if lines > opts.max_lines {
        return Err(format!("{} lines of IR (more than LOIMI_TRACE_MAXLINES={})", lines, opts.max_lines));
    }
    let r = GenFn { ir, blockmap: std::mem::take(&mut g.blockmap), edges: std::mem::take(&mut g.edges), kchk: BTreeSet::new(), nr: g.nr, nguards: g.nguards, neffect: g.neffect, guard_info: std::mem::take(&mut g.guard_info), lines };
    drop(g);
    Ok(GenFn { kchk: std::mem::take(&mut t.kchk), ..r })
}

/// The instructions of loop `h` (to its last back edge), without inner
/// loops (their headers are exits: they have regions of their own).
fn region_order(code: &Code, lp: &BTreeMap<usize, usize>, h: usize, skip: &[usize]) -> Vec<usize> {
    let b = lp[&h];
    let inner: Vec<(usize, usize)> = lp.iter().filter(|(h2, _)| **h2 > h && **h2 <= b).map(|(h2, b2)| (*h2, *b2)).collect();
    (h..=b).filter(|&pc| code.op[pc].is_some() && !inner.iter().any(|&(h2, b2)| pc >= h2 && pc <= b2) && (pc == h || !skip.contains(&pc))).collect()
}

/// The generated functions, their globals and table entries.
struct Out {
    lines: usize,
    defs: String,
    entries: Vec<String>,
    globals: Vec<String>,
}

const ENTRY_TY: &str = "{ i64, i64, i64, i64, i64, ptr, ptr, i64, ptr, ptr, ptr, i64, ptr, i64 }";

#[allow(clippy::too_many_arguments)]
fn add_entry(out: &mut Out, m: &Module, code: &Code, h: usize, order: &[usize], name: &str, ncounts: usize, kchk: &BTreeSet<(u64, u64)>, rec: Option<&str>, fingerprint: u64, fail_limit: u64) {
    let lo = *order.iter().min().unwrap();
    let hi = order.iter().map(|&pc| span_hi(m, code, pc)).max().unwrap();
    let mut hash = FNV0;
    for pc in lo..hi {
        if code.op[pc].is_some() {
            hash = fnv(hash, code.w[pc] as u64);
        }
    }
    out.globals.push(format!("@\"loimi.trace.st.{}\" = internal global [4 x i64] [i64 0, i64 0, i64 0, i64 {}]", name, fail_limit));
    out.globals.push(format!("@\"loimi.trace.counts.{}\" = internal global [{} x i64] zeroinitializer", name, ncounts.max(1)));
    out.globals.push(format!("@\"loimi.trace.name.{}\" = private constant [{} x i8] c\"{}\\00\"", name, name.len() + 1, name));
    let kv: Vec<String> = kchk.iter().flat_map(|(i, v)| [format!("i64 {}", i), format!("i64 {}", *v as i64)]).collect();
    out.globals.push(format!(
        "@\"loimi.trace.k.{}\" = private constant [{} x i64] [{}]",
        name,
        kv.len().max(1),
        if kv.is_empty() { "i64 0".to_string() } else { kv.join(", ") }
    ));
    let recg = match rec {
        Some(r) => {
            out.globals.push(format!("@\"loimi.trace.rec.{}\" = private constant [{} x i8] c\"{}\\00\"", name, r.len() + 1, c_string(r)));
            format!("ptr @\"loimi.trace.rec.{}\"", name)
        }
        None => "ptr null".to_string(),
    };
    out.entries.push(format!(
        "{} {{ i64 {}, i64 {}, i64 {}, i64 {}, i64 {}, ptr @\"loimi.trace.{}\", ptr @\"loimi.trace.counts.{}\", i64 {}, ptr @\"loimi.trace.name.{}\", ptr @\"loimi.trace.st.{}\", ptr @\"loimi.trace.k.{}\", i64 {}, {}, i64 {} }}",
        ENTRY_TY,
        code.n,
        lo,
        hi,
        hash as i64,
        h,
        name,
        name,
        ncounts,
        name,
        name,
        name,
        kchk.len(),
        recg,
        fingerprint as i64
    ));
}

/// Declarations for the globals `text` names that the program lacks.
fn declarations(m: &Module, text: &str) -> Result<Vec<String>, String> {
    let mut names = BTreeSet::new();
    let b = text.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'"' && (i == 0 || b[i - 1] != b'@') {
            // c"..." strings
            i += text[i + 1..].find('"').map(|e| e + 2).unwrap_or(1);
            continue;
        }
        if b[i] == b'@' {
            if let Some((nm, e, _)) = name_at(text, i + 1) {
                names.insert(nm.to_string());
                i = e;
                continue;
            }
        }
        i += 1;
    }
    let mut out = Vec::new();
    for n in names {
        let n = n.as_str();
        if m.program_syms.contains(n) || n.starts_with("loimi.trace.") || n == "loimi_traces" || n == "loimi_ntraces" || n == "loimi_trace_build" {
            continue;
        }
        if n == "llvm.memcpy.p0.p0.i64" {
            out.push("declare void @llvm.memcpy.p0.p0.i64(ptr, ptr, i64, i1)".to_string());
        } else if n == "loimi_trace_failing" {
            out.push("declare void @loimi_trace_failing(i64)".to_string());
        } else if n == "loimi_trace_check" {
            out.push("declare void @loimi_trace_check(ptr, ptr, i64)".to_string());
        } else if n.starts_with("__jit.") {
            return Err(format!("a hole (@{}) is left in a trace", n));
        } else if let Some(d) = m.stencil_decls.get(n) {
            out.push(d.to_string());
        } else {
            return Err(format!("a trace names @{}, which the program does not define", n));
        }
    }
    Ok(out)
}

fn finish(m: &Module, out: Out, stencil_hash: u64) -> Result<String, String> {
    let mut s = String::new();
    for d in declarations(m, &out.defs)? {
        s.push_str(&d);
        s.push('\n');
    }
    for g in &out.globals {
        s.push_str(g);
        s.push('\n');
    }
    let _ = writeln!(s, "@loimi_traces = constant [{} x {}] [{}]", out.entries.len(), ENTRY_TY, out.entries.join(", "));
    let _ = writeln!(s, "@loimi_ntraces = constant i64 {}", out.entries.len());
    let _ = writeln!(s, "@loimi_trace_build = constant i64 {}", stencil_hash as i64);
    s.push_str(&out.defs);
    Ok(s)
}

/// The stencils' arguments must be the vm's state registers.
fn check_sig(m: &Module, vm: &VmInfo) -> Result<(), String> {
    if m.sig.len() != vm.nstate || m.sig.get(vm.pc_idx).map(|s| s.as_str()) != Some("ptr") || vm.regs_idx.is_some_and(|r| m.sig.get(r).map(|s| s.as_str()) != Some("ptr")) {
        return Err(format!("the stencils take {:?}, not the vm's {} state registers", m.sig, vm.nstate));
    }
    Ok(())
}

/// A hash of the stencils: profiles name the build they came from.
pub fn stencil_hash(stencils: &str) -> u64 {
    fnv_str(FNV0, stencils)
}

fn region_fingerprint(stencil_hash: u64, code: &Code, h: usize) -> u64 {
    fnv(fnv(fnv(FNV0, stencil_hash), code.key()), h as u64)
}

/// `--trace-regions`: every loop of the recorded code objects as a
/// counted region. Returns the IR to append and a report.
pub fn regions(ir: &str, stencils: &str, vm: &VmInfo, codes: &[Code], opts: &Options) -> Result<(String, String), String> {
    let m = Module::new(ir, stencils);
    check_sig(&m, vm)?;
    let sh = stencil_hash(stencils);
    let mut out = Out { lines: 0, defs: String::new(), entries: Vec::new(), globals: Vec::new() };
    let mut report = String::new();
    for code in codes {
        let lp = loops(&m, code);
        for &h in lp.keys() {
            if !opts.only.is_empty() && !opts.only.contains(&h) {
                continue;
            }
            let order = region_order(code, &lp, h, &opts.skip);
            let name = format!("r{}", out.entries.len());
            let index = out.entries.len();
            match generate_fn(&m, vm, code, h, &order, Kind::Region, &name, index, HashMap::new(), None, opts) {
                Ok(g) if out.lines + g.lines > opts.max_total => {
                    let _ = writeln!(report, "no region for code {:016x} loop {}: the regions so far have {} lines of IR (LOIMI_TRACE_MAXTOTAL={})", code.hash, h, out.lines, opts.max_total);
                }
                Ok(g) => {
                    let fp = region_fingerprint(sh, code, h);
                    out.lines += g.lines;
                    out.defs.push_str(&g.ir);
                    add_entry(&mut out, &m, code, h, &order, &name, g.blockmap.len(), &g.kchk, Some(&code.text), fp, opts.fail_limit);
                    let _ = writeln!(report, "region {}: code {:016x} ({} words), loop {}..{}, {} instructions, window {}", name, code.hash, code.n, h, lp[&h], order.len(), g.nr);
                }
                Err(e) => {
                    let _ = writeln!(report, "no region for code {:016x} loop {}: {}", code.hash, h, e);
                }
            }
        }
    }
    Ok((finish(&m, out, sh)?, report))
}

/// `--traces`: per profiled loop that ran often enough, the hottest path
/// as a trace. Returns the IR to append and a report.
pub fn traces(ir: &str, stencils: &str, vm: &VmInfo, profs: &[ProfRegion], opts: &Options) -> Result<(String, String), String> {
    let m = Module::new(ir, stencils);
    check_sig(&m, vm)?;
    let sh = stencil_hash(stencils);
    let mut out = Out { lines: 0, defs: String::new(), entries: Vec::new(), globals: Vec::new() };
    let mut report = String::new();
    for pr in profs {
        let code = &pr.code;
        let h = pr.header;
        if region_fingerprint(sh, code, h) != pr.fingerprint {
            return Err(format!("profile region at {} does not match this build: build the profiling binary again", h));
        }
        let lp = loops(&m, code);
        if !lp.contains_key(&h) {
            let _ = writeln!(report, "no trace for code {:016x} loop {}: not a loop in this build", code.hash, h);
            continue;
        }
        let order = region_order(code, &lp, h, &opts.skip);
        // the region again, for its block numbering
        let probe = match generate_fn(&m, vm, code, h, &order, Kind::Region, "probe", 0, HashMap::new(), None, opts) {
            Ok(g) => g,
            Err(e) => {
                let _ = writeln!(report, "no trace for code {:016x} loop {}: {}", code.hash, h, e);
                continue;
            }
        };
        let (bm, edges) = (probe.blockmap, probe.edges);
        if bm.len() != pr.counts.len() {
            return Err(format!("profile region at {}: {} counters, the build has {}: build the profiling binary again", h, pr.counts.len(), bm.len()));
        }
        let mut eds: BTreeMap<(usize, usize), u64> = BTreeMap::new();
        for &(f, i, t) in &edges {
            *eds.entry((f, t)).or_default() += pr.counts[i];
        }
        // the hottest path around the loop
        let mut path = vec![h];
        let mut why = None;
        let mut back = 0;
        loop {
            let last = *path.last().unwrap();
            let best = eds.iter().filter(|((f, _), n)| *f == last && **n > 0).max_by_key(|((_, t), n)| (**n, std::cmp::Reverse(*t)));
            let Some(((_, t), n)) = best else {
                why = Some(format!("the path leaves the loop after {}", last));
                break;
            };
            if *t == h {
                back = *n;
                break;
            }
            if path.contains(t) {
                why = Some(format!("an inner cycle at {}", t));
                break;
            }
            path.push(*t);
        }
        if why.is_none() && back < opts.min_iters {
            why = Some(format!("{} iterations (fewer than {})", back, opts.min_iters));
        }
        // entries: the header's runs that did not come around the loop
        let hruns = bm.iter().position(|(pc, _)| *pc == h).map(|i| pr.counts[i]).unwrap_or(0);
        let around: u64 = edges.iter().filter(|e| e.2 == h).map(|e| pr.counts[e.1]).sum();
        let entries = hruns.saturating_sub(around).max(1);
        let per_entry = back as f64 / entries as f64;
        if why.is_none() && per_entry < opts.min_run {
            why = Some(format!("{:.1} iterations per entry (fewer than LOIMI_TRACE_MINRUN={})", per_entry, opts.min_run));
        }
        if let Some(w) = why {
            let _ = writeln!(report, "no trace for code {:016x} loop {}: {}", code.hash, h, w);
            continue;
        }
        // a block is on the trace if it ran a fraction of its step's runs
        let mut first_count: HashMap<usize, u64> = HashMap::new();
        let mut blocks: HashMap<(usize, String), u64> = HashMap::new();
        for (i, (pc, lab)) in bm.iter().enumerate() {
            first_count.entry(*pc).or_insert(pr.counts[i]);
            *blocks.entry((*pc, lab.clone())).or_default() += pr.counts[i];
        }
        let mut on_blocks = HashMap::new();
        for ((pc, lab), n) in &blocks {
            if path.contains(pc) {
                let mn = ((first_count[pc] as f64) * opts.min_frac).ceil() as u64;
                on_blocks.insert((*pc, lab.clone()), if *n >= mn.max(1) { *n } else { 0 });
            }
        }
        let prof = Prof { blocks: on_blocks, raw: blocks, min: 1 };
        let succ: HashMap<usize, usize> = (0..path.len()).map(|i| (path[i], path[(i + 1) % path.len()])).collect();
        let name = format!("t{}", out.entries.len());
        let index = out.entries.len();
        match generate_fn(&m, vm, code, h, &path, Kind::Trace, &name, index, succ, Some(prof), opts) {
            Ok(g) if out.lines + g.lines > opts.max_total => {
                let _ = writeln!(report, "no trace for code {:016x} loop {}: the traces so far have {} lines of IR (LOIMI_TRACE_MAXTOTAL={})", code.hash, h, out.lines, opts.max_total);
            }
            Ok(g) => {
                out.lines += g.lines;
                out.defs.push_str(&g.ir);
                add_entry(&mut out, &m, code, h, &path, &name, 0, &g.kchk, None, pr.fingerprint, opts.fail_limit);
                let ops: Vec<String> = path.iter().map(|&pc| format!("{}:{}", pc, code.op[pc].as_deref().unwrap_or("?"))).collect();
                let _ = writeln!(
                    report,
                    "trace {}: code {:016x} ({} words), loop {}, {} iterations ({:.1} per entry), window {}, {} guards, {} steps with effects: {}",
                    name,
                    code.hash,
                    code.n,
                    h,
                    back,
                    per_entry,
                    g.nr,
                    g.nguards,
                    g.neffect,
                    ops.join(" ")
                );
                if opts.verbose {
                    for gi in &g.guard_info {
                        let _ = writeln!(report, "  guard {}", gi);
                    }
                }
            }
            Err(e) => {
                let _ = writeln!(report, "no trace for code {:016x} loop {}: {}", code.hash, h, e);
            }
        }
    }
    Ok((finish(&m, out, sh)?, report))
}
