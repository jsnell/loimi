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
//! The workflow (three steps, docs/design/trace-tier.md section 11.1):
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
//!
//! Window copies are expanded at the end from markers (`@@DIRTY`,
//! `@@USE`, `@@SNAP`, `@@GUARD`, `@@COPY`) by two flow analyses: exits
//! write back only the slots that may differ from memory, reloads load
//! only the slots read before they are written, and a step's snapshot
//! holds only the slots it stores to.
//!
//! Calls (trace-tier.md section 12). Regions count where their dynamic
//! jumps go (`loimi_trace_site`) and have an entry at every instruction;
//! small code objects get an *entry region*. A trace's path may go into a
//! call whose site called one code object, which becomes an inlined
//! *frame*: its steps are (frame, pc) nodes, its registers the window's at
//! an offset, its code address bound at run time (`loimi_trace_bind`). The
//! stencils push and pop the vm's frames as they do in the JIT, so exits
//! inside an inlined frame need no reconstruction.

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
    fn clear(&mut self, j: usize) {
        self.0[j / 64] &= !(1 << (j % 64));
    }
    fn has(&self, j: usize) -> bool {
        self.0[j / 64] >> (j % 64) & 1 == 1
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
    /// Per dynamic exit (in the region's numbering): its runs, and where
    /// they went (`loimi_trace_site`).
    sites: BTreeMap<usize, (u64, Vec<Target>)>,
}

/// Where a dynamic exit went: a code object (its words and their hash)
/// and an offset in it, and how often.
#[derive(Clone, Copy, Debug)]
pub struct Target {
    n: usize,
    hash: u64,
    off: usize,
    count: u64,
}

fn add_target(ts: &mut Vec<Target>, t: Target) {
    match ts.iter_mut().find(|x| (x.n, x.hash, x.off) == (t.n, t.hash, t.off)) {
        Some(x) => x.count = x.count.saturating_add(t.count),
        None => ts.push(t),
    }
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
        let mut sites: BTreeMap<usize, (u64, Vec<Target>)> = BTreeMap::new();
        let mut cur = None;
        while let Some(l) = lines.next_if(|l| l.starts_with("site ") || l.starts_with("to ")) {
            let f: Vec<&str> = l.split_whitespace().collect();
            let n = |s: Option<&&str>| -> Result<u64, String> { s.and_then(|x| x.parse().ok()).ok_or_else(|| format!("bad line `{}`", l)) };
            if f[0] == "site" && f.len() == 3 {
                let i = num(f[1])?;
                let e = sites.entry(i).or_default();
                e.0 = e.0.saturating_add(n(f.get(2))?);
                cur = Some(i);
            } else if f[0] == "to" && f.len() == 5 {
                let i = cur.ok_or("`to` before `site`")?;
                let t = Target { n: num(f[1])?, hash: hex(f[2])?, off: num(f[3])?, count: n(f.get(4))? };
                add_target(&mut sites.get_mut(&i).unwrap().1, t);
            } else {
                return Err(format!("bad line `{}`", l));
            }
        }
        if lines.next() != Some("end") {
            return Err("a region without `end`".into());
        }
        match out.iter_mut().find(|r| r.fingerprint == fingerprint && r.counts.len() == counts.len()) {
            Some(r) => {
                r.counts.iter_mut().zip(&counts).for_each(|(a, b)| *a = a.saturating_add(*b));
                for (i, (n, ts)) in sites {
                    let e = r.sites.entry(i).or_default();
                    e.0 = e.0.saturating_add(n);
                    for t in ts {
                        add_target(&mut e.1, t);
                    }
                }
            }
            None => out.push(ProfRegion { fingerprint, header, code, counts, sites }),
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

/// An instruction of a trace: its frame (0: the loop's code object,
/// others: code objects inlined at calls) and pc.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
struct Node {
    f: usize,
    pc: usize,
}

impl Node {
    fn new(f: usize, pc: usize) -> Node {
        Node { f, pc }
    }
    /// The prefix of the step's names.
    fn prefix(&self) -> String {
        if self.f == 0 { format!("s{}.", self.pc) } else { format!("s{}f{}.", self.pc, self.f) }
    }
}

/// A frame of a trace: the loop's code object (0) or a code object
/// inlined at a call (traces through calls, docs/design/trace-tier.md
/// section 12).
struct Frame<'a> {
    code: &'a Code,
    /// The frame it was called from.
    parent: Option<usize>,
    /// Its register file's offset from the trace's, in slots (known once
    /// the call is built).
    roff: Option<i64>,
    /// Pool constants baked in: (index, value), checked when the trace
    /// is installed (frame 0) or the callee bound (others).
    kchk: BTreeSet<(u64, u64)>,
}

/// A step: one instruction's stencil, its cold blocks inlined, renamed
/// and with its holes filled.
struct Step {
    node: Node,
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
    frames: Vec<Frame<'a>>,
    table: &'a str,
    op_mask: u64,
    imm: &'a [(u64, u64, bool)],
    h: usize,
    /// Every recorded instruction's continuation targets.
    steps: Vec<Step>,
    /// node -> index in steps.
    at: HashMap<Node, usize>,
    /// The handlers' state registers (the context, the state, the
    /// accumulators): their types, and which are pc and the register file.
    sig: Vec<String>,
    /// The handlers' return type (`void`, or a vm's result).
    ret: String,
    pc_idx: usize,
    regs_idx: Option<usize>,
    const_idx: Option<usize>,
}

/// The name of state register i of step prefix p (`sN.`, `sNfF.`) in
/// frame f: pc and the register file have their own.
fn state_name(p: &str, f: usize, i: usize, pc_idx: usize, regs_idx: Option<usize>) -> String {
    if i == pc_idx {
        format!("%{}pc", p)
    } else if Some(i) == regs_idx {
        frame_regs(f)
    } else {
        format!("%{}st{}", p, i)
    }
}

/// Frame f's register file (`%tr.regs` + its offset).
fn frame_regs(f: usize) -> String {
    if f == 0 { "%tr.regs".to_string() } else { format!("%f{}.regs", f) }
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

/// A call of a superinstruction's resume function (`VM.op.S.resumeK`, the
/// super from part K on, with `pc` at that part's word): the part's word,
/// the K-th instruction after pc (the parts' words are instructions).
fn resume_target(code: &Code, pc: usize, callee: &str) -> Option<usize> {
    let (head, k) = callee.rsplit_once(".resume")?;
    let k: usize = k.parse().ok()?;
    if k == 0 || !head.contains(".op.") {
        return None;
    }
    (pc + 1..code.n).filter(|&i| code.op[i].is_some()).nth(k - 1)
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
            frames: vec![Frame { code, parent: None, roff: Some(0), kchk: BTreeSet::new() }],
            table: vm.table,
            op_mask: mask(vm.op_bits),
            imm: vm.imm,
            h,
            steps: Vec::new(),
            at: HashMap::new(),
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
    fn build_step(&mut self, node: Node) -> Result<Step, String> {
        let pc = node.pc;
        let code = self.frames[node.f].code;
        let name = stencil_name(code, pc).ok_or("not an instruction")?;
        let body = self.m.bodies.get(name.as_str()).ok_or_else(|| format!("no stencil {}", name))?;
        let body = self.expand_cold(body)?;
        let p = node.prefix();
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
                    return state_name(&p, node.f, i, self.pc_idx, self.regs_idx);
                }
                match nm {
                    "insn" => i32s(word).to_string(),
                    _ if q => format!("%\"{}{}\"", p, nm),
                    _ => format!("%{}{}", p, nm),
                }
            });
            let lines = self.holes(node, &p, &r, &mut kp)?;
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
        let mut st = Step { node, pc, blocks, succs, vals: HashMap::new(), allocas: HashSet::new(), statevars: HashSet::new() };
        // an inlined frame's register file is the trace's at its offset
        let seed: Vec<(String, Val)> = match self.frames[node.f].roff {
            Some(o) if node.f > 0 => vec![(frame_regs(node.f), Val::Regs(Some(8 * o), false))],
            _ => Vec::new(),
        };
        analyze(&mut st, &seed);
        Ok(st)
    }

    /// Fills the holes of a renamed line (fields, words, constants).
    fn holes(&mut self, node: Node, p: &str, line: &str, kp: &mut usize) -> Result<Vec<String>, String> {
        if !line.contains("@\"__jit.") || line.contains("musttail call") {
            return Ok(vec![line.to_string()]);
        }
        let pc = node.pc;
        let code = self.frames[node.f].code;
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
                            self.frames[node.f].kchk.insert((idx, *v));
                            ("c", (*v as i64).to_string())
                        }
                        Some(None) => return Err(format!("constant {}.{}.{} at {}: two values recorded", o, s, b, pc)),
                        _ => {
                            // a heap constant (its address differs between
                            // runs) or not recorded: load it from the pool
                            *kp += 1;
                            let g = format!("%{}kp{}", p, kp);
                            let Some(ki) = self.const_idx else { return Err("a constant hole without a constant pool".into()) };
                            pre.push(format!("  {} = getelementptr inbounds i64, ptr {}, i64 {}", g, state_name(p, node.f, ki, self.pc_idx, self.regs_idx), idx));
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
fn analyze(st: &mut Step, seed: &[(String, Val)]) {
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
    let mut vals: HashMap<String, Val> = seed.iter().cloned().collect();
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
    /// By (frame, pc, label).
    blocks: HashMap<(usize, usize, String), u64>,
    /// The counts before the minimum fraction (for the report).
    raw: HashMap<(usize, usize, String), u64>,
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
/// [4]: side traces: nonzero once the runtime dropped it (its parent then
/// leaves where it would enter it).
const ST_OFF: usize = 4;

/// The shape of a trace: the steps each step continues to inside the
/// function, which trace (0: the root, k: side trace k) each step is in,
/// the edges where side traces start, the steps where loops inside the
/// trace close (iterations are counted there too), and per trace its name
/// and index in the runtime's table (sides[0]: the root).
#[derive(Default)]
struct Shape {
    succ: HashMap<Node, Vec<Node>>,
    owner: HashMap<Node, usize>,
    side_entry: HashMap<(Node, Node), usize>,
    heads: HashSet<Node>,
    /// Edges into a loop inside the trace from outside it: the window is
    /// written back there, so the loop's guards and exits restore only
    /// what it changed (not everything the trace changed before it).
    loop_entries: HashSet<(Node, Node)>,
    sides: Vec<(String, usize)>,
}

struct Emit {
    out: Vec<String>,
    cur: String,
    ctr: usize,
    /// step -> incoming (block, state registers, iteration count, the
    /// code's base if the step starts an inlined frame)
    heads_in: BTreeMap<Node, Vec<(String, Vec<String>, String, Option<String>)>>,
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
    order: Vec<Node>,
    /// Trace: the steps each continues to in the function (its path's
    /// next, and the first steps of side traces and loop exits from it),
    /// which side trace each step is in, and where side traces start.
    shape: Shape,
    /// The step being emitted.
    cur_node: Node,
    prof: Option<Prof>,
    nr: usize,
    e: Emit,
    allocas: Vec<String>,
    guards: Vec<(String, Node)>,
    /// Regions: the pc of each dynamic exit (its targets are counted).
    sites: Vec<usize>,
    blockmap: Vec<(usize, String)>,
    edges: Vec<(usize, usize, usize)>,
    /// Regions: static exits (pc, counter, target pc).
    exits: Vec<(usize, usize, usize)>,
    check: bool,
    nguards: usize,
    neffect: usize,
    /// Steps whose usual path calls a function that may write.
    nwrites: usize,
    /// ... in inlined frames.
    inlined_writes: usize,
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
    /// The types of those accesses (the register file's values).
    wtypes: HashSet<String>,
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
        // live slots (read before written: `@@USE`, snapshots, guards,
        // writebacks of dirty slots) after each reload, which loads only
        // those (after a call that may write, the window is reloaded, and
        // most slots are written before they are read again, or not used)
        let mut reload_live: HashMap<usize, Bits> = HashMap::new();
        if !full {
            let snapset = |p: &str| -> Bits {
                let mut r = Bits::default();
                self.snapsets.get(p).into_iter().flatten().for_each(|&j| r.set(j));
                r
            };
            // the dirty slots at each writeback
            let mut wb_dirty: HashMap<usize, Bits> = HashMap::new();
            for (b, &(_, lo, hi)) in blocks.iter().enumerate() {
                let mut d = din[b].unwrap_or_default();
                for (i, l) in raw[lo..hi].iter().enumerate() {
                    if let Some(j) = l.strip_prefix("@@DIRTY ") {
                        d.set(j.parse().unwrap());
                    } else if l.starts_with("@@COPY ") && (l.ends_with(" %tr.regs") || l.ends_with(" %lregs")) {
                        if l.ends_with(" %tr.regs") {
                            wb_dirty.insert(lo + i, d);
                        }
                        d = Bits::default();
                    }
                }
            }
            let mut preds: Vec<Vec<usize>> = vec![Vec::new(); nb];
            for (b, ss) in succs.iter().enumerate() {
                for &s in ss {
                    preds[s].push(b);
                }
            }
            let mut lin: Vec<Bits> = vec![Bits::default(); nb];
            let mut work: Vec<usize> = (0..nb).collect();
            let mut queued = vec![true; nb];
            while let Some(b) = work.pop() {
                queued[b] = false;
                let mut live = Bits::default();
                for &s in &succs[b] {
                    live = live.or(&lin[s]);
                }
                let (lo, hi) = (blocks[b].1, blocks[b].2);
                for i in (lo..hi).rev() {
                    let l = &raw[i];
                    if let Some(j) = l.strip_prefix("@@USE ") {
                        live.set(j.parse().unwrap());
                    } else if let Some(j) = l.strip_prefix("@@DIRTY ") {
                        live.clear(j.parse().unwrap());
                    } else if l.starts_with("@@COPY ") && l.ends_with(" %tr.regs") {
                        live = live.or(&wb_dirty[&i]);
                    } else if l.starts_with("@@COPY ") && l.ends_with(" %lregs") {
                        reload_live.insert(i, live);
                        live = Bits::default();
                    } else if let Some(p) = l.strip_prefix("@@SNAP ") {
                        live = live.or(&snapset(p));
                    } else if let Some(p) = l.strip_prefix("@@GUARD ") {
                        let ss = snapset(p);
                        for j in head_dirty.get(p).copied().unwrap_or_default().iter() {
                            if !ss.has(j) {
                                live.set(j);
                            }
                        }
                    }
                }
                if live != lin[b] {
                    lin[b] = live;
                    for &q in &preds[b] {
                        if !queued[q] {
                            queued[q] = true;
                            work.push(q);
                        }
                    }
                }
            }
        }
        let mut out = Vec::with_capacity(raw.len() + 1024);
        for (b, &(_, lo, hi)) in blocks.iter().enumerate() {
            if lo > 0 {
                out.push(raw[lo - 1].clone());
            }
            let mut d = din[b].unwrap_or_default();
            for (i, l) in raw[lo..hi].iter().enumerate() {
                if let Some(j) = l.strip_prefix("@@DIRTY ") {
                    d.set(j.parse().unwrap());
                } else if l.starts_with("@@USE ") {
                } else if let Some(r) = l.strip_prefix("@@COPY ") {
                    let (from, to) = r.split_once(' ').unwrap();
                    if full {
                        out.push(memcpy(from, to, self.nr));
                    } else if to == "%tr.regs" {
                        let s: Vec<(usize, &str)> = d.iter().map(|j| (j, from)).collect();
                        self.copy_slots(&s, to, &mut out);
                    } else {
                        let live = reload_live.get(&(lo + i)).copied();
                        let s: Vec<(usize, &str)> = used.iter().filter(|&&j| live.is_none_or(|lv| lv.has(j))).map(|&j| (j, from)).collect();
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
        let (a, b, n) = (self.e.fresh("it"), self.e.fresh("it"), self.e.fresh("itv"));
        self.e.line(format!("  {} = load i64, ptr %tr.itp", n));
        self.e.line(format!("  {} = load i64, ptr {}", a, g));
        self.e.line(format!("  {} = add i64 {}, {}", b, a, n));
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

    /// The address of frame f's code (its word 0).
    fn frame_base(f: usize) -> String {
        if f == 0 { "%tr.base".to_string() } else { format!("%f{}.code", f) }
    }

    /// The address of node t's word (`base`: its frame's code, if the
    /// frame is being entered).
    fn pc_of(&mut self, t: Node, base: Option<&str>) -> String {
        let pcv = self.e.fresh("xpc");
        if t.f == 0 {
            self.e.line(format!("  {} = getelementptr inbounds i32, ptr %tr.base, i64 {}", pcv, t.pc));
        } else {
            let b = base.map(|b| b.to_string()).unwrap_or_else(|| Self::frame_base(t.f));
            self.e.line(format!("  {} = getelementptr inbounds i32, ptr {}, i64 {}", pcv, b, t.pc));
        }
        pcv
    }

    /// Continues at step t with `state` if `regs` is its frame's file
    /// (and, after calls that may rewrite code, the trace still valid);
    /// otherwise exits there. `base`: the address of t's code, if t
    /// starts an inlined frame.
    fn goto_head(&mut self, t: Node, state: &[String], window: bool, check_gen: bool, base: Option<String>) {
        let mut cond = "true".to_string();
        if let Some(ri) = self.t.regs_idx {
            cond = self.e.fresh("same");
            self.e.line(format!("  {} = icmp eq ptr {}, {}", cond, state[ri], frame_regs(t.f)));
        }
        if check_gen {
            let g = self.st_gep(ST_GEN);
            let (gv, ok, both) = (self.e.fresh("gen"), self.e.fresh("ok"), self.e.fresh("ok"));
            self.e.line(format!("  {} = load i64, ptr {}", gv, g));
            self.e.line(format!("  {} = icmp eq i64 {}, %tr.gen0", ok, gv));
            self.e.line(format!("  {} = and i1 {}, {}", both, cond, ok));
            cond = both;
        }
        if let Some(&k) = self.shape.side_entry.get(&(self.cur_node, t)) {
            // the first step of side trace k: unless the runtime dropped it
            // (its guards failed too often), when this is an exit again
            let sname = self.shape.sides[k].0.clone();
            let (g, v, ok, both) = (self.e.fresh("sp"), self.e.fresh("soff"), self.e.fresh("son"), self.e.fresh("ok"));
            self.e.line(format!("  {} = getelementptr inbounds i64, ptr {}, i64 {}", g, self.st(&sname), ST_OFF));
            self.e.line(format!("  {} = load i64, ptr {}", v, g));
            self.e.line(format!("  {} = icmp eq i64 {}, 0", ok, v));
            self.e.line(format!("  {} = and i1 {}, {}", both, cond, ok));
            cond = both;
        }
        let go = self.e.fresh("go")[1..].to_string();
        let no = self.e.fresh("leave")[1..].to_string();
        self.e.line(format!("  br i1 {}, label %{}, label %{}", cond, go, no));
        self.e.label(&go);
        if !window {
            self.reload();
        } else if self.shape.loop_entries.contains(&(self.cur_node, t)) {
            self.writeback();
        }
        if self.kind == Kind::Trace && (t == Node::new(0, self.t.h) || self.shape.heads.contains(&t)) {
            // an iteration (of the loop or of a loop inside it): what the
            // runtime compares guard failures with
            let (a, b) = (self.e.fresh("it"), self.e.fresh("itn"));
            self.e.line(format!("  {} = load i64, ptr %tr.itp", a));
            self.e.line(format!("  {} = add i64 {}, 1", b, a));
            self.e.line(format!("  store i64 {}, ptr %tr.itp", b));
        }
        self.e.line(format!("  br label %{}head", t.prefix()));
        let cur = self.e.cur.clone();
        self.e.heads_in.entry(t).or_default().push((cur, state.to_vec(), String::new(), base.clone()));
        self.e.label(&no);
        let mut st = state.to_vec();
        st[self.t.pc_idx] = self.pc_of(t, base.as_deref());
        self.exit_dispatch(&st, window);
    }

    fn counter(&mut self, idx: usize) {
        let (x, y, z) = (self.e.fresh("cp"), self.e.fresh("cv"), self.e.fresh("cn"));
        self.e.line(format!("  {} = getelementptr inbounds i64, ptr @\"loimi.trace.counts.{}\", i64 {}", x, self.name, idx));
        self.e.line(format!("  {} = load i64, ptr {}", y, x));
        self.e.line(format!("  {} = add i64 {}, 1", z, y));
        self.e.line(format!("  store i64 {}, ptr {}", z, x));
    }

    /// Whether a static continuation from `from` to pc t (in its frame)
    /// stays in the function.
    fn internal(&self, from: Node, t: usize) -> bool {
        let t = Node::new(from.f, t);
        match self.kind {
            Kind::Trace => self.shape.succ.get(&from).is_some_and(|v| v.contains(&t)),
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

    /// Whether line l of step st has an effect a re-execution would
    /// repeat (a store outside the step's locals and the window, an
    /// atomic, a call that may write).
    fn line_effect(&self, st: &Step, l: &str) -> bool {
        let (_, rhs) = def_of(l);
        if let Some((ty, _, ptr)) = parse_store(rhs) {
            return self.store_effect(val_of(&st.vals, ptr), ty);
        }
        if rhs.starts_with("atomicrmw ") || rhs.starts_with("cmpxchg ") || rhs.starts_with("fence") {
            return true;
        }
        if !rhs.starts_with("musttail") {
            if let Some(c) = parse_call(rhs) {
                return self.classify_call(st, &c) == CallK::Unsafe;
            }
        }
        false
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

    /// An access through a register-file pointer the window cannot place
    /// (its offset is not known, or it may be another pointer).
    fn unplaced(&self, v: Val, ty: &str) -> bool {
        match v {
            Val::Regs(Some(o), false) => !self.in_window(o, ty) && o >= 0 && o < 8 * self.nr as i64,
            Val::Regs(..) => true,
            _ => false,
        }
    }

    fn in_window(&self, o: i64, ty: &str) -> bool {
        o >= 0 && o % 8 == 0 && ty_size(ty) == Some(8) && ((o / 8) as usize) < self.nr
    }

    fn on(&self, n: Node, lab: &str) -> bool {
        match &self.prof {
            None => true,
            Some(p) => p.blocks.get(&(n.f, n.pc, lab.to_string())).copied().unwrap_or(0) >= p.min.max(1),
        }
    }

    /// Emits step `si` (index into steps).
    fn emit_step(&mut self, si: usize) -> Result<(), String> {
        let st = std::mem::replace(&mut self.t.steps[si], Step { node: Node::new(0, 0), pc: 0, blocks: vec![], succs: vec![], vals: HashMap::new(), allocas: HashSet::new(), statevars: HashSet::new() });
        let r = self.emit_step_inner(&st);
        self.t.steps[si] = st;
        r
    }

    fn emit_step_inner(&mut self, st: &Step) -> Result<(), String> {
        let node = st.node;
        self.cur_node = node;
        let pc = st.pc;
        let p = node.prefix();
        let nb = st.blocks.len();
        // per block: effects (from the step's start) and calls
        let mut has_effect = vec![false; nb];
        let mut writes = vec![false; nb];
        // blocks with a call that may rewrite code
        let mut has_rw = vec![false; nb];
        // blocks with a call that may write
        let mut calls_w = vec![false; nb];
        for (b, (_, ls)) in st.blocks.iter().enumerate() {
            for l in ls {
                let (_, rhs) = def_of(l);
                if let Some((ty, _, ptr)) = parse_store(rhs) {
                    let v = val_of(&st.vals, ptr);
                    if self.store_effect(v, ty) {
                        has_effect[b] = true;
                    }
                    if self.kind == Kind::Trace && self.unplaced(v, ty) {
                        // a store into the register file at an offset not
                        // known: memory holds the registers after it
                        writes[b] = true;
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
                                has_rw[b] = true;
                                calls_w[b] = true;
                            }
                            CallK::Exempt => has_rw[b] = true,
                            _ => {}
                        }
                    }
                }
            }
        }
        let on: Vec<bool> = st.blocks.iter().map(|(lab, _)| self.on(node, lab)).collect();
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
                            let cnt = |l: &str| self.prof.as_ref().and_then(|p| p.raw.get(&(node.f, pc, l.to_string())).copied()).unwrap_or(0);
                            self.guard_info.push(format!("{}{}: {} ({}) -> {} ({})", pc, if node.f > 0 { format!(" (frame {})", node.f) } else { String::new() }, st.blocks[b].0, cnt(&st.blocks[b].0), st.blocks[s].0, cnt(&st.blocks[s].0)));
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
        // a call that may rewrite code on some path from the step's start
        // to the end of each block: continuing from there checks `gen`
        let mut rw_out = has_rw.clone();
        let mut changed = true;
        while changed {
            changed = false;
            for b in 0..nb {
                if reach[b] && !rw_out[b] && (0..nb).any(|q| reach[q] && rw_out[q] && st.succs[q].contains(&b) && !guard_edges.contains(&(q, b))) {
                    rw_out[b] = true;
                    changed = true;
                }
            }
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
        // effects before each on-trace block (on on-trace paths)
        let eff_in: Vec<bool> = (0..nb).map(|b| (0..nb).any(|q| on[q] && st.succs[q].contains(&b) && eff_out[q])).collect();
        // accesses through heap pointers in inlined frames that may be
        // into the window (open upvalues): guards where nothing before
        // them in the step has an effect
        let ptr_check = |l: &str| -> bool {
            if node.f == 0 || self.kind != Kind::Trace || self.used.is_empty() {
                return false;
            }
            let (_, rhs) = def_of(l);
            let a = parse_load(rhs).or_else(|| parse_store(rhs).map(|(t, _, p)| (t, p)));
            a.is_some_and(|(ty, ptr)| matches!(val_of(&st.vals, ptr), Val::Top | Val::Bot) && self.wtypes.contains(ty.trim()) && !ptr.starts_with('@'))
        };
        let mut ptr_guards = false;
        for b in 0..nb {
            if !on[b] || !reach[b] {
                continue;
            }
            let mut e = eff_in[b];
            for l in &st.blocks[b].1 {
                if !e && ptr_check(l) {
                    ptr_guards = true;
                }
                e = e || self.line_effect(st, l);
            }
        }
        let snap = !guard_edges.is_empty() || ptr_guards;
        if self.kind == Kind::Trace && snap {
            self.nguards += guard_edges.len();
        }
        if eff_out.iter().zip(&on).any(|(e, o)| *e && *o) {
            self.neffect += 1;
        }
        // a call that may write on the step's usual path (in at least
        // half of its runs): it writes the window back, and the window is
        // loaded again after it
        if let Some(pr) = &self.prof {
            let cnt = |l: &str| pr.raw.get(&(node.f, pc, l.to_string())).copied().unwrap_or(0);
            let first = cnt(&st.blocks[0].0);
            if (0..nb).any(|b| calls_w[b] && on[b] && 2 * cnt(&st.blocks[b].0) >= first && first > 0) {
                self.nwrites += 1;
                if node.f > 0 {
                    self.inlined_writes += 1;
                }
            }
        }

        // the head: phis, the step's pc, the snapshot
        self.e.label(&format!("{}head", p));
        self.e.line(format!("@@PHIS {} {}", node.f, pc));
        if node.f == 0 {
            self.e.line(format!("  %{}pc = getelementptr inbounds i32, ptr %tr.base, i64 {}", p, pc));
        } else {
            self.e.line(format!("  %{}pc = getelementptr inbounds i32, ptr {}, i64 {}", p, Self::frame_base(node.f), pc));
        }
        if snap && self.nr > 0 {
            // the slots this step stores to (known once it is emitted)
            self.e.line(format!("@@SNAP {}", p));
        }
        self.cur_stores.clear();
        self.e.line(format!("  br label %{}{}", p, st.blocks[0].0));
        let mut table_geps: HashSet<String> = HashSet::new();
        let mut table_fns: HashSet<String> = HashSet::new();
        let mut entry_bases: HashSet<String> = HashSet::new();
        let mut entry_geps: HashSet<String> = HashSet::new();
        let mut entry_fns: HashSet<String> = HashSet::new();
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
            let mut deferred: Vec<String> = Vec::new();
            let mut eff_now = eff_in[b] || !on[b];
            for l in ls {
                let eff_before = eff_now;
                eff_now = eff_now || self.line_effect(st, l);
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
                    // dispatches: through the table, or to compiled code
                    // through the JIT's entries (an enter word)
                    if let Some((_, base, _)) = parse_gep(rhs) {
                        if base == format!("@\"{}\"", self.t.table) {
                            table_geps.insert(d.to_string());
                        }
                        if entry_bases.contains(base) {
                            entry_geps.insert(d.to_string());
                        }
                    }
                    if let Some((_, ptr)) = parse_load(rhs) {
                        if table_geps.contains(ptr) {
                            table_fns.insert(d.to_string());
                        }
                        if entry_geps.contains(ptr) {
                            entry_fns.insert(d.to_string());
                        }
                        if ptr == "@loimi_jit_entries" {
                            entry_bases.insert(d.to_string());
                        }
                    }
                    // in traces, what only computes a dispatch's handler is
                    // emitted where the trace leaves (after a transition)
                    if self.kind == Kind::Trace && on[b] && [&table_geps, &table_fns, &entry_bases, &entry_geps, &entry_fns].iter().any(|s| s.contains(d)) {
                        deferred.push(l.clone());
                        continue;
                    }
                }
                let term = ["br ", "switch ", "ret", "unreachable", "indirectbr "].iter().any(|t| rhs.starts_with(t));
                if !deferred.is_empty() && !rhs.starts_with("musttail call") && (term || deferred.iter().any(|x| uses_local(l, def_of(x).0.unwrap()))) {
                    for x in std::mem::take(&mut deferred) {
                        self.e.line(x);
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
                    let rewrites = rw_out[b];
                    skip_ret = true;
                    if c.global {
                        if let Some(t) = cont_target(self.t.frames[node.f].code, pc, c.callee) {
                            // `next` or a label jump: to the next step, or an exit
                            match t {
                                Some(t) if on[b] && (self.internal(node, t) || (t == pc && self.kind == Kind::Trace)) => {
                                    // (to its own pc: an instruction that runs
                                    // again, `redo`, goes around its step)
                                    self.edges.push((pc, idx, t));
                                    self.goto_head(Node::new(node.f, t), &state, window, rewrites, None);
                                }
                                Some(t) => {
                                    if self.kind == Kind::Region {
                                        // (a static exit: its count is the
                                        // block's, for side traces and links)
                                        self.exits.push((pc, idx, t));
                                    }
                                    let mut st2 = state.clone();
                                    st2[self.t.pc_idx] = self.pc_of(Node::new(node.f, t), None);
                                    self.exit_dispatch(&st2, window);
                                }
                                None => return Err(format!("step {}: a jump outside the code", pc)),
                            }
                            continue;
                        }
                        if let Some(t) = resume_target(self.t.frames[node.f].code, pc, c.callee) {
                            // a superinstruction's resume function (its
                            // parts from K on, as a cold block's `next`
                            // calls it): the instruction at part K's word
                            if self.kind == Kind::Region {
                                self.exits.push((pc, idx, t));
                            } else if on[b] && self.internal(node, t) {
                                self.edges.push((pc, idx, t));
                                self.goto_head(Node::new(node.f, t), &state, window, rewrites, None);
                                continue;
                            }
                        }
                        self.exit_call(&format!("@\"{}\"", c.callee), &args, window);
                        continue;
                    }
                    let dispatch = table_fns.contains(c.callee) || entry_fns.contains(c.callee);
                    if self.kind == Kind::Trace && dispatch && on[b] {
                        // a dispatch to a computed pc: the next steps if it
                        // is one of them (a jump, a call, a return), hottest
                        // first
                        for nx in self.shape.succ.get(&node).cloned().unwrap_or_default() {
                            self.transition(node, nx, &args, &state, window, rewrites)?;
                        }
                        // `redo`: to this step again
                        self.transition(node, node, &args, &state, window, rewrites)?;
                    }
                    for x in std::mem::take(&mut deferred) {
                        self.e.line(x);
                    }
                    if self.kind == Kind::Region && dispatch {
                        // the targets of dynamic exits, for the profile
                        let site = self.sites.len();
                        self.sites.push(pc);
                        self.e.line(format!("  call void @loimi_trace_site(ptr @\"loimi.trace.sites.{}\", i64 {}, ptr {})", self.name, site, args[self.t.pc_idx]));
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
                            if !is_store {
                                self.e.line(format!("@@USE {}", j));
                            }
                            self.e.line(replace_ptr(l, ptr, &s));
                            if is_store {
                                // the slot now differs from memory
                                self.e.line(format!("@@DIRTY {}", j));
                                self.cur_stores.insert(j);
                            }
                            continue;
                        }
                        Val::Regs(Some(_), false) if !(window && self.kind == Kind::Trace && self.unplaced(v, ty)) => {}
                        _ if window && self.kind == Kind::Trace && self.unplaced(v, ty) => {
                            // an offset not known (Mini's CALL clears the
                            // callee's registers in a loop): on memory,
                            // after a writeback; after a store, memory
                            // holds the registers to the step's end
                            self.writeback();
                            self.e.line(l.clone());
                            if is_store {
                                window = false;
                            }
                            continue;
                        }
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
                        Val::Top | Val::Bot if window && node.f > 0 && self.kind == Kind::Trace && self.wtypes.contains(ty.trim()) && !ptr.starts_with('@') && !self.used.is_empty() => {
                            // In an inlined frame, a pointer from the heap
                            // may be into the window: an upvalue of a
                            // closure called inline that is open on a
                            // register of a frame below (the window caches
                            // those)
                            if eff_before {
                                self.ptr_access(l, ptr)?;
                            } else {
                                // into the window: the interpreter runs
                                // the instruction (a guard)
                                let (x, y, dd, inw, exp) = (self.e.fresh("ri"), self.e.fresh("rl"), self.e.fresh("rd"), self.e.fresh("rin"), self.e.fresh("rex"));
                                let g = format!("guard.{}.{}.p{}", pc, node.f, self.e.ctr);
                                let go = self.e.fresh("rgo")[1..].to_string();
                                self.e.line(format!("  {} = ptrtoint ptr {} to i64", x, ptr));
                                self.e.line(format!("  {} = ptrtoint ptr %tr.regs to i64", y));
                                self.e.line(format!("  {} = sub i64 {}, {}", dd, x, y));
                                self.e.line(format!("  {} = icmp ult i64 {}, {}", inw, dd, 8 * self.nr));
                                self.e.line(format!("  {} = call i1 @llvm.expect.i1(i1 {}, i1 false)", exp, inw));
                                self.e.line(format!("  br i1 {}, label %{}, label %{}", exp, g, go));
                                self.guards.push((g, node));
                                self.nguards += 1;
                                self.e.label(&go);
                                self.e.line(l.clone());
                            }
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
                        let mut tbs = Vec::new();
                        for part in &parts[1..] {
                            let tok = part.trim().split([',', ' ']).next().unwrap_or("");
                            let tb = st.blocks.iter().position(|bb| format!("%{}{}", p, bb.0) == tok).ok_or_else(|| format!("step {}: bad branch `{}`", pc, rhs))?;
                            tbs.push(tb);
                            if guard_edges.contains(&(b, tb)) {
                                guard_ix += 1;
                                let g = format!("guard.{}.{}.{}", pc, node.f, guard_ix);
                                self.guards.push((g.clone(), node));
                                labs.push(format!("%{}", g));
                            } else if !window && !real_in[tb] {
                                return Err(format!("step {}: real-mode edge into a window-mode block", pc));
                            } else if window && real_in[tb] {
                                labs.push(self.wb_edge(tok));
                            } else {
                                labs.push(tok.to_string());
                            }
                        }
                        // In traces, the profile says which way the branch
                        // goes (guards: never the guard's way). Without it
                        // LLVM takes each guard for a coin toss, so a loop
                        // that passes a few seems to run once, and registers
                        // go to the code around it (spills in mandelbrot's
                        // inner loop when the outer loops were in the trace).
                        let mut head = parts[0].to_string();
                        if self.kind == Kind::Trace && !suffix.contains("!prof") && tbs.len() == 2 {
                            let cnt = |tb: usize| -> u64 {
                                if guard_edges.contains(&(b, tb)) {
                                    return 0;
                                }
                                self.prof.as_ref().and_then(|pr| pr.raw.get(&(node.f, pc, st.blocks[tb].0.clone())).copied()).unwrap_or(0)
                            };
                            let (c0, c1) = (cnt(tbs[0]), cnt(tbs[1]));
                            let dir = if c0 > 0 && c0 >= 32 * c1 {
                                Some(true)
                            } else if c1 > 0 && c1 >= 32 * c0 {
                                Some(false)
                            } else {
                                None
                            };
                            if let (Some(d), Some(cond)) = (dir, parts[0].strip_prefix("br i1 ").map(|c| c.trim().trim_end_matches(',').trim())) {
                                let x = self.e.fresh("exp");
                                self.e.line(format!("  {} = call i1 @llvm.expect.i1(i1 {}, i1 {})", x, cond, d));
                                head = format!("br i1 {}, ", x);
                            }
                        }
                        self.e.line(format!("  {}label {}, label {}{}", head, labs[0], labs[1], suffix));
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

    /// A dispatch from step `node` to the computed pc in `args`: to step
    /// `nx` if that is where it goes. In the same frame or a return to
    /// the calling frame, that is a comparison with nx's address; a call
    /// compares with the callee's address the trace bound last
    /// (`@loimi.trace.slot.NAME`), and else asks the runtime to bind it
    /// (`loimi_trace_bind`: the code there is the code inlined, its words
    /// and constants checked) and leaves: the next call goes on inline.
    /// Leaves the current block where it does not go there (the caller
    /// exits).
    fn transition(&mut self, node: Node, nx: Node, args: &[String], state: &[String], window: bool, rewrites: bool) -> Result<(), String> {
        let tgt = args[self.t.pc_idx].clone();
        let go = self.e.fresh("tgo")[1..].to_string();
        let miss = self.e.fresh("tmiss")[1..].to_string();
        let parent = self.t.frames[node.f].parent;
        if nx.f == node.f || Some(nx.f) == parent {
            let exp = self.pc_of(nx, None);
            let hit = self.e.fresh("thit");
            self.e.line(format!("  {} = icmp eq ptr {}, {}", hit, tgt, exp));
            self.e.line(format!("  br i1 {}, label %{}, label %{}", hit, go, miss));
            self.e.label(&go);
            self.goto_head(nx, state, window, rewrites, None);
        } else if self.t.frames[nx.f].parent == Some(node.f) && nx.f > 0 {
            let k = nx.f - 1;
            let (sp, cached, hit, ok, base) = (self.e.fresh("slot"), self.e.fresh("bound"), self.e.fresh("thit"), self.e.fresh("bok"), self.e.fresh("fbase"));
            let bind = self.e.fresh("tbind")[1..].to_string();
            self.e.line(format!("  {} = getelementptr inbounds ptr, ptr @\"loimi.trace.slot.{}\", i64 {}", sp, self.name, k));
            self.e.line(format!("  {} = load ptr, ptr {}", cached, sp));
            self.e.line(format!("  {} = icmp eq ptr {}, {}", hit, tgt, cached));
            self.e.line(format!("  br i1 {}, label %{}, label %{}", hit, go, bind));
            self.e.label(&bind);
            // (bound or not, this call leaves: no path into the inlined
            // frame passes the runtime call, so memory facts flow on)
            self.e.line(format!("  {} = call i32 @loimi_trace_bind(i64 {}, i64 {}, ptr {})", ok, self.index, k, tgt));
            self.e.line(format!("  br label %{}", miss));
            self.e.label(&go);
            self.e.line(format!("  {} = getelementptr i32, ptr {}, i64 {}", base, tgt, -(nx.pc as i64)));
            self.goto_head(nx, state, window, rewrites, Some(base));
        } else {
            return Err(format!("step {}: the path goes from frame {} to frame {}", node.pc, node.f, nx.f));
        }
        self.e.label(&miss);
        Ok(())
    }

    /// A load or store through a pointer from the heap, in an inlined
    /// frame, after an effect (so not a guard): if it is into the window
    /// (an open upvalue on a register of a frame below), the access is
    /// on memory, between a writeback and a reload.
    fn ptr_access(&mut self, l: &str, ptr: &str) -> Result<(), String> {
        let (x, y, dd) = (self.e.fresh("ri"), self.e.fresh("rl"), self.e.fresh("rd"));
        self.e.line(format!("  {} = ptrtoint ptr {} to i64", x, ptr));
        self.e.line(format!("  {} = ptrtoint ptr %tr.regs to i64", y));
        self.e.line(format!("  {} = sub i64 {}, {}", dd, x, y));
        let join = self.e.fresh("rj")[1..].to_string();
        let mem = self.e.fresh("rm")[1..].to_string();
        let (d, rhs) = def_of(l);
        let (inw, exp) = (self.e.fresh("rin"), self.e.fresh("rex"));
        let cold = self.e.fresh("rcold")[1..].to_string();
        self.e.line(format!("  {} = icmp ult i64 {}, {}", inw, dd, 8 * self.nr));
        self.e.line(format!("  {} = call i1 @llvm.expect.i1(i1 {}, i1 false)", exp, inw));
        self.e.line(format!("  br i1 {}, label %{}, label %{}", exp, cold, mem));
        self.e.label(&cold);
        self.writeback();
        let v = self.e.fresh("rv");
        match d {
            Some(_) => self.e.line(format!("  {} = {}", v, rhs)),
            None => self.e.line(format!("  {}", rhs)),
        }
        self.reload();
        self.e.line(format!("  br label %{}", join));
        self.e.label(&mem);
        let v2 = self.e.fresh("rv");
        match d {
            Some(_) => self.e.line(format!("  {} = {}", v2, rhs)),
            None => self.e.line(format!("  {}", rhs)),
        }
        self.e.line(format!("  br label %{}", join));
        self.e.label(&join);
        if let Some(d) = d {
            let ty = parse_load(rhs).map(|x| x.0).ok_or_else(|| format!("unexpected access `{}`", l.trim()))?;
            self.e.line(format!("  {} = phi {} [ {}, %{} ], [ {}, %{} ]", d, ty, v, cold, v2, mem));
        }
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

/// Whether `line` uses local `name` (`%x`).
fn uses_local(line: &str, name: &str) -> bool {
    let mut found = false;
    map_locals(line, &mut |nm, q| {
        let t = if q { format!("%\"{}\"", nm) } else { format!("%{}", nm) };
        found |= t == name;
        t
    });
    found
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
    /// Calls inlined in a trace, nested this deep at most
    /// (LOIMI_TRACE_INLINE; 0: calls are exits, and regions have no entry
    /// regions).
    pub inline_depth: usize,
    /// Code objects with more instructions outside loops get no entry
    /// region (LOIMI_TRACE_INLINE_SIZE): they are not inlined.
    pub inline_size: usize,
    /// ... or more lines of IR (LOIMI_TRACE_INLINE_LINES).
    pub inline_lines: usize,
    /// A call site is inlined if this fraction of its calls went to one
    /// code object (LOIMI_TRACE_MONO).
    pub mono: f64,
    /// Keep traces whose inlined calls call functions that may write on
    /// their usual path (LOIMI_TRACE_INLINE_WRITERS=1; by default they are
    /// declined: Mini's closures, whose calls create closures and close
    /// upvalues, measured 1.46 of the JIT with them).
    pub inline_writers: bool,
    /// Side traces per trace, at most (LOIMI_TRACE_SIDES; 0: none).
    pub max_sides: usize,
    /// An exit is the start of a side trace if it ran this many times
    /// (LOIMI_TRACE_SIDE_MIN, default LOIMI_TRACE_MIN) and this fraction of the root's iterations
    /// (LOIMI_TRACE_SIDE_FRAC).
    pub side_min: u64,
    pub side_frac: f64,
    /// The steps of a trace and its side traces, in lines of stencil IR,
    /// at most (LOIMI_TRACE_SIDE_LINES): side traces past it are left out.
    pub side_lines: usize,
    /// Paths go through the loops inside the loop (LOIMI_TRACE_NEST=0:
    /// they end there, as in M1).
    pub nest: bool,
    /// The longest walk (steps) when forming a path.
    pub max_walk: usize,
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
            inline_depth: var("LOIMI_TRACE_INLINE").and_then(|v| v.parse().ok()).unwrap_or(4),
            inline_size: var("LOIMI_TRACE_INLINE_SIZE").and_then(|v| v.parse().ok()).unwrap_or(64),
            inline_lines: var("LOIMI_TRACE_INLINE_LINES").and_then(|v| v.parse().ok()).unwrap_or(40_000),
            mono: var("LOIMI_TRACE_MONO").and_then(|v| v.parse().ok()).unwrap_or(0.9),
            inline_writers: var("LOIMI_TRACE_INLINE_WRITERS").is_some_and(|v| v != "0"),
            max_sides: var("LOIMI_TRACE_SIDES").and_then(|v| v.parse().ok()).unwrap_or(32),
            side_min: var("LOIMI_TRACE_SIDE_MIN").or_else(|| var("LOIMI_TRACE_MIN")).and_then(|v| v.parse().ok()).unwrap_or(1000),
            side_frac: var("LOIMI_TRACE_SIDE_FRAC").and_then(|v| v.parse().ok()).unwrap_or(0.01),
            side_lines: var("LOIMI_TRACE_SIDE_LINES").and_then(|v| v.parse().ok()).unwrap_or(60_000),
            nest: var("LOIMI_TRACE_NEST").is_none_or(|v| v != "0"),
            max_walk: 1000,
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
    fn new(t: &'t mut Trace<'a>, kind: Kind, name: String, index: usize, order: Vec<Node>, shape: Shape, prof: Option<Prof>, check: bool) -> Self {
        FnGen {
            t,
            kind,
            name,
            index,
            order,
            shape,
            cur_node: Node::new(0, 0),
            prof,
            nr: 0,
            e: Emit { out: Vec::new(), cur: String::new(), ctr: 0, heads_in: BTreeMap::new() },
            allocas: Vec::new(),
            guards: Vec::new(),
            blockmap: Vec::new(),
            edges: Vec::new(),
            exits: Vec::new(),
            check,
            nguards: 0,
            neffect: 0,
            nwrites: 0,
            inlined_writes: 0,
            side: Vec::new(),
            flushed: false,
            debug: std::env::var_os("LOIMI_TRACE_DEBUG").is_some(),
            guard_info: Vec::new(),
            used: BTreeSet::new(),
            wtypes: HashSet::new(),
            dynamic: false,
            sites: Vec::new(),
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
        for n in &self.order {
            let st = &self.t.steps[self.t.at[n]];
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
                                    self.wtypes.insert(ty.trim().to_string());
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
        for n in self.order.clone() {
            let si = self.t.at[&n];
            self.emit_step(si)?;
        }
        // guard exits: the window as at the step's start, then the
        // instruction again in the interpreter
        let guards = std::mem::take(&mut self.guards);
        for (g, node) in &guards {
            self.e.label(g);
            // this entry's iterations first: the runtime compares them
            self.flush_iters();
            // the failure counts of the (side) trace the step is in
            let (oname, oindex) = match self.shape.owner.get(node).copied().unwrap_or(0) {
                0 => (self.name.clone(), self.index),
                k => self.shape.sides[k].clone(),
            };
            let (f, l) = (self.e.fresh("stp"), self.e.fresh("stp"));
            self.e.line(format!("  {} = getelementptr inbounds i64, ptr {}, i64 {}", f, self.st(&oname), ST_FAILS));
            self.e.line(format!("  {} = getelementptr inbounds i64, ptr {}, i64 {}", l, self.st(&oname), ST_LIMIT));
            let (f0, f1, lv, hit) = (self.e.fresh("gf"), self.e.fresh("gf"), self.e.fresh("gl"), self.e.fresh("gh"));
            let (lb, lc) = (self.e.fresh("gb")[1..].to_string(), self.e.fresh("gc")[1..].to_string());
            self.e.line(format!("  {} = load i64, ptr {}", f0, f));
            self.e.line(format!("  {} = add i64 {}, 1", f1, f0));
            self.e.line(format!("  store i64 {}, ptr {}", f1, f));
            self.e.line(format!("  {} = load i64, ptr {}", lv, l));
            self.e.line(format!("  {} = icmp uge i64 {}, {}", hit, f1, lv));
            self.e.line(format!("  br i1 {}, label %{}, label %{}", hit, lb, lc));
            self.e.label(&lb);
            self.e.line(format!("  call void @loimi_trace_failing(i64 {})", oindex));
            self.e.line(format!("  br label %{}", lc));
            self.e.label(&lc);
            let p = node.prefix();
            if self.nr > 0 {
                // the slots dirty at the step's start, as they were then
                self.e.line(format!("@@GUARD {}", p));
            }
            self.flushed = true;
            let state: Vec<String> = (0..self.t.sig.len()).map(|i| state_name(&p, node.f, i, self.t.pc_idx, self.t.regs_idx)).collect();
            let word = self.t.frames[node.f].code.w[node.pc];
            self.exit_interp(&state, word);
        }
        // phis
        let first = Node::new(0, self.t.h);
        let mut res: Vec<String> = Vec::new();
        let out = std::mem::take(&mut self.e.out);
        for l in out {
            if let Some(fp) = l.strip_prefix("@@PHIS ") {
                let (f, pc) = fp.split_once(' ').unwrap();
                let node = Node::new(f.parse().unwrap(), pc.parse().unwrap());
                let p = node.prefix();
                let mut ins = self.e.heads_in.get(&node).cloned().unwrap_or_default();
                let n = self.t.sig.len();
                let params: Vec<String> = (0..n).map(|i| self.param(i)).collect();
                if node == first || self.kind == Kind::Region {
                    ins.insert(0, ("entry".into(), params.clone(), "0".into(), None));
                }
                let carried: Vec<usize> = (0..n).filter(|&i| i != self.t.pc_idx && Some(i) != self.t.regs_idx).collect();
                if ins.is_empty() {
                    for &i in &carried {
                        res.push(format!("  {} = bitcast {} {} to {}", state_name(&p, node.f, i, self.t.pc_idx, self.t.regs_idx), self.t.sig[i], params[i], self.t.sig[i]));
                    }
                    continue;
                }
                for &i in &carried {
                    let v: Vec<String> = ins.iter().map(|x| format!("[ {}, %{} ]", x.1[i], x.0)).collect();
                    res.push(format!("  {} = phi {} {}", state_name(&p, node.f, i, self.t.pc_idx, self.t.regs_idx), self.t.sig[i], v.join(", ")));
                }
                if node.f > 0 && ins.iter().any(|x| x.3.is_some()) {
                    // the first step of an inlined frame: its code's address
                    // (the frame is entered only by its call: other edges
                    // come from inside it)
                    let fb = Self::frame_base(node.f);
                    let v: Vec<String> = ins.iter().map(|x| format!("[ {}, %{} ]", x.3.as_deref().unwrap_or(&fb), x.0)).collect();
                    res.push(format!("  {} = phi ptr {}", fb, v.join(", ")));
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
        // this entry's iterations (SROA keeps the count in a register)
        f.push_str("  %tr.itp = alloca i64, align 8\n  store i64 0, ptr %tr.itp\n");
        self.e.out.clear();
        if self.kind == Kind::Region {
            // entered at any of its instructions (`loimi.trace.NAME.eX`
            // pass X): the code's address from the entry's
            self.e.line("  %tr.ex = zext i32 %tr.insn to i64".into());
            self.e.line("  %tr.exn = sub i64 0, %tr.ex".into());
            self.e.line("  %tr.base = getelementptr i32, ptr %tr.pc, i64 %tr.exn".into());
        } else {
            self.e.line(format!("  %tr.base = getelementptr i32, ptr %tr.pc, i64 {}", -(self.t.h as i64)));
        }
        for f in 1..self.t.frames.len() {
            let o = self.t.frames[f].roff.unwrap_or(0);
            self.e.line(format!("  {} = getelementptr i64, ptr %tr.regs, i64 {}", frame_regs(f), o));
        }
        self.reload();
        let g = self.st_gep(ST_GEN);
        self.e.line(format!("  %tr.gen0 = load i64, ptr {}", g));
        let entry = std::mem::take(&mut self.e.out);
        let side = std::mem::take(&mut self.side);
        let jump = if self.kind == Kind::Region {
            let cases: Vec<String> = self.order.iter().filter(|n| **n != first).map(|n| format!("i32 {}, label %{}head", n.pc, n.prefix())).collect();
            format!("  switch i32 %tr.insn, label %{}head [ {} ]", first.prefix(), cases.join(" "))
        } else {
            format!("  br label %{}head", first.prefix())
        };
        let raw: Vec<String> = [entry, vec![jump], res, side].into_iter().flatten().collect();
        let all = self.expand_markers(raw);
        for l in &all {
            f.push_str(l);
            f.push('\n');
        }
        f.push_str("}\n");
        Ok(f)
    }
}

/// What `generate_fn` returns: the function, its block map, edges and
/// dynamic exits (region numbering), the constants baked in per frame and
/// the frames' register offsets, the window size, guards, steps with
/// effects.
struct GenFn {
    ir: String,
    blockmap: Vec<(usize, String)>,
    edges: Vec<(usize, usize, usize)>,
    exits: Vec<(usize, usize, usize)>,
    sites: Vec<usize>,
    kchk: Vec<BTreeSet<(u64, u64)>>,
    nr: usize,
    nguards: usize,
    neffect: usize,
    nwrites: usize,
    inlined_writes: usize,
    guard_info: Vec<String>,
    lines: usize,
}

/// A frame of a path: its code, the frame that called it and the call
/// (a step of the parent).
#[derive(Clone, Copy)]
struct PathFrame<'a> {
    code: &'a Code,
    parent: Option<usize>,
    call: Option<Node>,
}

/// The register-file offset (in slots) that step `st`, a call, gives the
/// frame it enters: its dispatches on the trace pass `regs` there.
fn call_regs(st: &Step, prof: Option<&Prof>, regs_idx: Option<usize>, nsig: usize) -> Result<i64, String> {
    let Some(ri) = regs_idx else { return Ok(0) };
    let mut seen: Option<Val> = None;
    for (lab, ls) in &st.blocks {
        let on = prof.is_none_or(|p| p.blocks.get(&(st.node.f, st.pc, lab.clone())).copied().unwrap_or(0) >= p.min.max(1));
        if !on {
            continue;
        }
        for l in ls {
            let (_, rhs) = def_of(l);
            if !rhs.starts_with("musttail call") {
                continue;
            }
            let Some(c) = parse_call(rhs) else { continue };
            if c.global || c.args.len() != nsig + 1 {
                continue;
            }
            let v = val_of(&st.vals, c.args[ri].1);
            if seen.is_some_and(|s| s != v) {
                return Err(format!("step {}: calls with different register files", st.pc));
            }
            seen = Some(v);
        }
    }
    match seen {
        Some(Val::Regs(Some(o), false)) if o >= 0 && o % 8 == 0 => Ok(o / 8),
        v => Err(format!("step {}: a call whose register file is not known ({:?})", st.pc, v)),
    }
}

/// How the continuations of the instruction at pc treat its frame, in a
/// vm with a register file: the static targets it continues at with
/// another frame's registers (a call of its own code), and whether one of
/// its dispatches passes on its own register file (a jump table). A jump
/// into the same code object with another frame (a recursive call, a
/// return from one) is not a path inside the loop: the trace would leave
/// there at every iteration.
fn frame_moves(m: &Module, vm: &VmInfo, code: &Code, pc: usize) -> (BTreeSet<usize>, bool) {
    let mut other = BTreeSet::new();
    let Some(ri) = vm.regs_idx else { return (other, true) };
    let mut t = Trace::new(m, code, vm, 0);
    let Ok(st) = t.build_step(Node::new(0, pc)) else { return (other, false) };
    let mut keeps = false;
    for (_, ls) in &st.blocks {
        for l in ls {
            let rhs = def_of(l).1;
            if !rhs.starts_with("musttail call") {
                continue;
            }
            let Some(c) = parse_call(rhs) else { continue };
            if c.args.len() != t.sig.len() + 1 {
                continue;
            }
            let v = val_of(&st.vals, c.args[ri].1);
            if c.global {
                if let (Some(Some(tg)), Val::Regs(Some(o), _)) = (cont_target(code, pc, c.callee), v) {
                    if o != 0 {
                        other.insert(tg);
                    }
                }
            } else if v == Val::Regs(Some(0), false) {
                keeps = true;
            }
        }
    }
    (other, keeps)
}

/// The function `kind` for loop h of frame 0's code, over `order`.
#[allow(clippy::too_many_arguments)]
fn generate_fn(m: &Module, vm: &VmInfo, frames: &[PathFrame], h: usize, order: &[Node], kind: Kind, name: &str, index: usize, shape: Shape, prof: Option<Prof>, opts: &Options) -> Result<GenFn, String> {
    let mut t = Trace::new(m, frames[0].code, vm, h);
    for pf in &frames[1..] {
        t.frames.push(Frame { code: pf.code, parent: pf.parent, roff: None, kchk: BTreeSet::new() });
    }
    for &n in order {
        if t.frames[n.f].roff.is_none() {
            // an inlined frame: its register file is where the call passes
            // it (the call is a step before the frame's in the order)
            let call = frames[n.f].call.ok_or("an inlined frame without its call")?;
            let j = *t.at.get(&call).ok_or_else(|| format!("frame {}'s call ({}) is not before it", n.f, call.pc))?;
            let o = call_regs(&t.steps[j], prof.as_ref(), t.regs_idx, t.sig.len())?;
            if o as usize >= MAX_WINDOW {
                return Err(format!("frame {}'s registers start at {}, past the largest window", n.f, o));
            }
            t.frames[n.f].roff = Some(o);
        }
        let code = t.frames[n.f].code;
        let st = t.build_step(n).map_err(|e| format!("instruction {} ({}){}: {}", n.pc, code.op[n.pc].as_deref().unwrap_or("?"), if n.f > 0 { format!(" of frame {}", n.f) } else { String::new() }, e))?;
        t.at.insert(n, t.steps.len());
        t.steps.push(st);
    }
    let mut g = FnGen::new(&mut t, kind, name.to_string(), index, order.to_vec(), shape, prof, opts.check);
    let ir = g.emit()?;
    let lines = ir.bytes().filter(|&b| b == b'\n').count();
    if lines > opts.max_lines {
        return Err(format!("{} lines of IR (more than LOIMI_TRACE_MAXLINES={})", lines, opts.max_lines));
    }
    let r = GenFn {
        ir,
        blockmap: std::mem::take(&mut g.blockmap),
        edges: std::mem::take(&mut g.edges),
        exits: std::mem::take(&mut g.exits),
        sites: std::mem::take(&mut g.sites),
        kchk: Vec::new(),
        nr: g.nr,
        nguards: g.nguards,
        neffect: g.neffect,
        nwrites: g.nwrites,
        inlined_writes: g.inlined_writes,
        guard_info: std::mem::take(&mut g.guard_info),
        lines,
    };
    drop(g);
    Ok(GenFn { kchk: t.frames.iter_mut().map(|f| std::mem::take(&mut f.kchk)).collect(), ..r })
}

/// The instructions of loop `h` (to its last back edge), without inner
/// loops (their headers are exits: they have regions of their own).
fn region_order(code: &Code, lp: &BTreeMap<usize, usize>, h: usize, skip: &[usize]) -> Vec<usize> {
    let b = lp[&h];
    let inner: Vec<(usize, usize)> = lp.iter().filter(|(h2, _)| **h2 > h && **h2 <= b).map(|(h2, b2)| (*h2, *b2)).collect();
    (h..=b).filter(|&pc| code.op[pc].is_some() && !inner.iter().any(|&(h2, b2)| pc >= h2 && pc <= b2) && (pc == h || !skip.contains(&pc))).collect()
}

/// A region's entry at pc: a function that enters region `name` with pc
/// as its `insn` (the region dispatches on it).
fn trampoline(m: &Module, name: &str, en: &str, pc: usize) -> String {
    let ps: Vec<String> = m.sig.iter().enumerate().map(|(i, t)| format!("{} %a{}", t, i)).collect();
    let call = format!("musttail call preserve_nonecc {} @\"loimi.trace.{}\"({}, i32 {})", m.ret, name, ps.join(", "), pc);
    let body = if m.ret == "void" { format!("  {}\n  ret void\n", call) } else { format!("  %r = {}\n  ret {} %r\n", call, m.ret) };
    format!("define internal preserve_nonecc {} @\"loimi.trace.{}\"({}, i32 %insn) nounwind {{\nentry:\n{}}}\n", m.ret, en, ps.join(", "), body)
}

/// A code object's *entry region*: its instructions outside loops (their
/// headers are exits), from word 0, if word 0 is not a loop header. Its
/// profile is what traces inline at calls.
fn entry_order(code: &Code, lp: &BTreeMap<usize, usize>) -> Option<Vec<usize>> {
    if lp.contains_key(&0) || code.op.first().is_none_or(|o| o.is_none()) {
        return None;
    }
    Some((0..code.n).filter(|&pc| code.op[pc].is_some() && !lp.iter().any(|(&h, &b)| pc >= h && pc <= b)).collect())
}

/// The generated functions, their globals and table entries.
struct Out {
    lines: usize,
    defs: String,
    entries: Vec<String>,
    globals: Vec<String>,
}

const ENTRY_TY: &str = "{ i64, i64, i64, i64, i64, ptr, ptr, i64, ptr, ptr, ptr, i64, ptr, i64, ptr, i64, ptr, i64, ptr, i64 }";
const CALLEE_TY: &str = "{ i64, i64, i64, ptr, i64 }";
/// Per dynamic exit of a region: [0] runs, then 16 targets of (pc, runs,
/// words, hash of the words, offset) (`loimi_trace_site`; a jump table has
/// a target per case).
const SITE_WORDS: usize = 82;

/// What `add_entry` needs besides the function: frame 0's instructions,
/// the counters, the recording (regions), the dynamic exits (regions),
/// and per inlined frame its code, offset and constants (traces).
struct EntryInfo<'a> {
    order: Vec<usize>,
    ncounts: usize,
    kchk: &'a BTreeSet<(u64, u64)>,
    rec: Option<&'a str>,
    nsites: usize,
    callees: Vec<(&'a Code, usize, &'a BTreeSet<(u64, u64)>)>,
    /// The function the runtime enters (default `loimi.trace.NAME`).
    fnsym: Option<String>,
}

fn add_entry(out: &mut Out, m: &Module, code: &Code, h: usize, name: &str, info: &EntryInfo, fingerprint: u64, fail_limit: u64) {
    let order = &info.order;
    let lo = *order.iter().min().unwrap();
    let hi = order.iter().map(|&pc| span_hi(m, code, pc)).max().unwrap();
    let mut hash = FNV0;
    for pc in lo..hi {
        if code.op[pc].is_some() {
            hash = fnv(hash, code.w[pc] as u64);
        }
    }
    let kv = |k: &BTreeSet<(u64, u64)>| -> String {
        let v: Vec<String> = k.iter().flat_map(|(i, v)| [format!("i64 {}", i), format!("i64 {}", *v as i64)]).collect();
        format!("[{} x i64] [{}]", v.len().max(1), if v.is_empty() { "i64 0".to_string() } else { v.join(", ") })
    };
    out.globals.push(format!("@\"loimi.trace.st.{}\" = internal global [5 x i64] [i64 0, i64 0, i64 0, i64 {}, i64 0]", name, fail_limit));
    out.globals.push(format!("@\"loimi.trace.counts.{}\" = internal global [{} x i64] zeroinitializer", name, info.ncounts.max(1)));
    out.globals.push(format!("@\"loimi.trace.sites.{}\" = internal global [{} x i64] zeroinitializer", name, (info.nsites * SITE_WORDS).max(1)));
    out.globals.push(format!("@\"loimi.trace.name.{}\" = private constant [{} x i8] c\"{}\\00\"", name, name.len() + 1, name));
    out.globals.push(format!("@\"loimi.trace.k.{}\" = private constant {}", name, kv(info.kchk)));
    let mut callees = Vec::new();
    for (i, (c, off, k)) in info.callees.iter().enumerate() {
        out.globals.push(format!("@\"loimi.trace.ck.{}.{}\" = private constant {}", name, i, kv(k)));
        callees.push(format!("{} {{ i64 {}, i64 {}, i64 {}, ptr @\"loimi.trace.ck.{}.{}\", i64 {} }}", CALLEE_TY, c.n, c.hash as i64, off, name, i, k.len()));
    }
    out.globals.push(format!(
        "@\"loimi.trace.callees.{}\" = private constant [{} x {}] [{}]",
        name,
        callees.len().max(1),
        CALLEE_TY,
        if callees.is_empty() { format!("{} zeroinitializer", CALLEE_TY) } else { callees.join(", ") }
    ));
    out.globals.push(format!("@\"loimi.trace.slot.{}\" = internal global [{} x ptr] zeroinitializer", name, info.callees.len().max(1)));
    let recg = match info.rec {
        Some(r) => {
            out.globals.push(format!("@\"loimi.trace.rec.{}\" = private constant [{} x i8] c\"{}\\00\"", name, r.len() + 1, c_string(r)));
            format!("ptr @\"loimi.trace.rec.{}\"", name)
        }
        None => "ptr null".to_string(),
    };
    out.entries.push(format!(
        "{} {{ i64 {}, i64 {}, i64 {}, i64 {}, i64 {}, ptr @\"{}\", ptr @\"loimi.trace.counts.{n}\", i64 {}, ptr @\"loimi.trace.name.{n}\", ptr @\"loimi.trace.st.{n}\", ptr @\"loimi.trace.k.{n}\", i64 {}, {}, i64 {}, ptr @\"loimi.trace.sites.{n}\", i64 {}, ptr @\"loimi.trace.callees.{n}\", i64 {}, ptr @\"loimi.trace.slot.{n}\", i64 -1 }}",
        ENTRY_TY,
        code.n,
        lo,
        hi,
        hash as i64,
        h,
        info.fnsym.clone().unwrap_or_else(|| format!("loimi.trace.{}", name)),
        info.ncounts,
        info.kchk.len(),
        recg,
        fingerprint as i64,
        info.nsites,
        info.callees.len(),
        n = name,
    ));
}

/// A side trace's entry (`NAME` = `tN.sK`, `parent` the root's index): its
/// counts and the flag the root reads (`ST_OFF`); it is not installed, its
/// steps are in the root's function.
fn add_side_entry(out: &mut Out, name: &str, parent: usize, fail_limit: u64) {
    out.globals.push(format!("@\"loimi.trace.st.{}\" = internal global [5 x i64] [i64 0, i64 0, i64 0, i64 {}, i64 0]", name, fail_limit));
    out.globals.push(format!("@\"loimi.trace.name.{}\" = private constant [{} x i8] c\"{}\\00\"", name, name.len() + 1, name));
    out.entries.push(format!(
        "{} {{ i64 0, i64 0, i64 0, i64 0, i64 0, ptr null, ptr null, i64 0, ptr @\"loimi.trace.name.{n}\", ptr @\"loimi.trace.st.{n}\", ptr null, i64 0, ptr null, i64 0, ptr null, i64 0, ptr null, i64 0, ptr null, i64 {} }}",
        ENTRY_TY,
        parent,
        n = name,
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
        } else if n == "llvm.expect.i1" {
            out.push("declare i1 @llvm.expect.i1(i1, i1)".to_string());
        } else if n == "loimi_trace_site" {
            out.push("declare void @loimi_trace_site(ptr, i64, ptr)".to_string());
        } else if n == "loimi_trace_bind" {
            out.push("declare i32 @loimi_trace_bind(i64, i64, ptr)".to_string());
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
/// counted region, and each code object's entry region (what traces
/// inline at calls) if it is small. Returns the IR to append and a
/// report.
pub fn regions(ir: &str, stencils: &str, vm: &VmInfo, codes: &[Code], opts: &Options) -> Result<(String, String), String> {
    let m = Module::new(ir, stencils);
    check_sig(&m, vm)?;
    let sh = stencil_hash(stencils);
    let mut out = Out { lines: 0, defs: String::new(), entries: Vec::new(), globals: Vec::new() };
    let mut report = String::new();
    // the loops first (traces), then the entry regions (what traces
    // inline), within the budget
    let mut todo: Vec<(&Code, BTreeMap<usize, usize>, usize, Vec<usize>, &str)> = Vec::new();
    for code in codes {
        let lp = loops(&m, code);
        for &h in lp.keys() {
            if !opts.only.is_empty() && !opts.only.contains(&h) {
                continue;
            }
            todo.push((code, lp.clone(), h, region_order(code, &lp, h, &opts.skip), "loop"));
        }
    }
    for code in codes {
        let lp = loops(&m, code);
        if opts.inline_depth > 0 && opts.only.is_empty() {
            if let Some(o) = entry_order(code, &lp).filter(|o| o.len() <= opts.inline_size) {
                todo.push((code, lp, 0, o, "entry"));
            }
        }
    }
    {
        // (entry regions have a budget of their own, and each one a smaller
        // size: what is worth inlining is small)
        let mut lines = [0usize; 2];
        for (code, lp, h, order, what) in todo {
            let name = format!("r{}", out.entries.len());
            let index = out.entries.len();
            let nodes: Vec<Node> = order.iter().map(|&pc| Node::new(0, pc)).collect();
            let k = (what == "entry") as usize;
            match generate_fn(&m, vm, &[PathFrame { code, parent: None, call: None }], h, &nodes, Kind::Region, &name, index, Shape::default(), None, opts) {
                Ok(g) if lines[k] + g.lines > opts.max_total => {
                    let _ = writeln!(report, "no region for code {:016x} {} {}: the {} regions so far have {} lines of IR (LOIMI_TRACE_MAXTOTAL={})", code.hash, what, h, what, lines[k], opts.max_total);
                }
                Ok(g) if k == 1 && g.lines > opts.inline_lines => {
                    let _ = writeln!(report, "no region for code {:016x} entry: {} lines of IR (more than LOIMI_TRACE_INLINE_LINES={})", code.hash, g.lines, opts.inline_lines);
                }
                Ok(g) => {
                    lines[k] += g.lines;
                    let fp = region_fingerprint(sh, code, h);
                    out.lines += g.lines;
                    out.defs.push_str(&g.ir);
                    // an entry at each instruction (a trampoline that
                    // passes its pc): code that returns from a call, or
                    // that comes back from an exit, is counted too
                    for &pc in &order {
                        let en = format!("{}.e{}", name, pc);
                        out.defs.push_str(&trampoline(&m, &name, &en, pc));
                        let main = pc == h;
                        let info = EntryInfo {
                            order: order.clone(),
                            ncounts: if main { g.blockmap.len() } else { 0 },
                            kchk: &g.kchk[0],
                            rec: if main { Some(&code.text) } else { None },
                            nsites: if main { g.sites.len() } else { 0 },
                            callees: Vec::new(),
                            fnsym: Some(format!("loimi.trace.{}", en)),
                        };
                        add_entry(&mut out, &m, code, pc, if main { &name } else { &en }, &info, fp, opts.fail_limit);
                    }
                    let _ = writeln!(report, "region {}: code {:016x} ({} words), {} {}{}, {} instructions, {} lines, window {}", name, code.hash, code.n, what, h, if what == "loop" { format!("..{}", lp[&h]) } else { String::new() }, order.len(), g.lines, g.nr);
                }
                Err(e) => {
                    let _ = writeln!(report, "no region for code {:016x} {} {}: {}", code.hash, what, h, e);
                }
            }
        }
    }
    Ok((finish(&m, out, sh)?, report))
}

/// A profiled region rebuilt (for its numbering) with its counts: the
/// edges between its instructions, its static exits, the targets of its
/// dynamic exits per instruction, and the block counts.
struct RegionData {
    edges: BTreeMap<(usize, usize), u64>,
    /// Static continuations that leave the region (into an inner loop, out
    /// of the loop): (pc, target) -> runs.
    exits: BTreeMap<(usize, usize), u64>,
    /// pc -> (runs, targets)
    sites: HashMap<usize, (u64, Vec<Target>)>,
    blocks: HashMap<(usize, String), u64>,
    first_count: HashMap<usize, u64>,
    /// Runs of the header, and how many came around the loop.
    hruns: u64,
    around: u64,
}

/// The instructions of profiled region `pr` in this build (a loop's, or a
/// code object's entry region).
fn region_pcs(m: &Module, pr: &ProfRegion, skip: &[usize]) -> Result<Vec<usize>, String> {
    let lp = loops(m, &pr.code);
    if lp.contains_key(&pr.header) {
        Ok(region_order(&pr.code, &lp, pr.header, skip))
    } else if pr.header == 0 {
        entry_order(&pr.code, &lp).ok_or_else(|| "not an entry region in this build".to_string())
    } else {
        Err("not a loop in this build".into())
    }
}

fn region_data(m: &Module, vm: &VmInfo, pr: &ProfRegion, opts: &Options) -> Result<RegionData, String> {
    let code = &pr.code;
    let h = pr.header;
    let order = region_pcs(m, pr, &opts.skip)?;
    let nodes: Vec<Node> = order.iter().map(|&pc| Node::new(0, pc)).collect();
    let probe = generate_fn(m, vm, &[PathFrame { code, parent: None, call: None }], h, &nodes, Kind::Region, "probe", 0, Shape::default(), None, opts)?;
    if probe.blockmap.len() != pr.counts.len() {
        return Err(format!("{} counters, the build has {}: build the profiling binary again", pr.counts.len(), probe.blockmap.len()));
    }
    let mut edges: BTreeMap<(usize, usize), u64> = BTreeMap::new();
    for &(f, i, t) in &probe.edges {
        *edges.entry((f, t)).or_default() += pr.counts[i];
    }
    let mut exits: BTreeMap<(usize, usize), u64> = BTreeMap::new();
    for &(f, i, t) in &probe.exits {
        *exits.entry((f, t)).or_default() += pr.counts[i];
    }
    let mut sites: HashMap<usize, (u64, Vec<Target>)> = HashMap::new();
    for (i, &pc) in probe.sites.iter().enumerate() {
        if let Some((n, ts)) = pr.sites.get(&i) {
            let e = sites.entry(pc).or_default();
            e.0 += n;
            for t in ts {
                match e.1.iter_mut().find(|x| (x.n, x.hash, x.off) == (t.n, t.hash, t.off)) {
                    Some(x) => x.count += t.count,
                    None => e.1.push(*t),
                }
            }
        }
    }
    let mut first_count: HashMap<usize, u64> = HashMap::new();
    let mut blocks: HashMap<(usize, String), u64> = HashMap::new();
    for (i, (pc, lab)) in probe.blockmap.iter().enumerate() {
        first_count.entry(*pc).or_insert(pr.counts[i]);
        *blocks.entry((*pc, lab.clone())).or_default() += pr.counts[i];
    }
    let hruns = probe.blockmap.iter().position(|(pc, _)| *pc == h).map(|i| pr.counts[i]).unwrap_or(0);
    let around: u64 = probe.edges.iter().filter(|e| e.2 == h).map(|e| pr.counts[e.1]).sum();
    Ok(RegionData { edges, exits, sites, blocks, first_count, hruns, around })
}

/// Where a trace's path goes next.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Next {
    /// To pc in the same frame.
    Same(usize),
    /// A return to pc in the calling frame.
    Ret(usize),
    /// A call of the code object of entry region (profile index).
    Call(usize),
}

/// A frame of a trace being formed: the profile whose code it runs (frame
/// 0: the loop's region; others: the callee's entry region), its caller and
/// the call's pc there.
#[derive(Clone, Copy, PartialEq)]
struct FrameRec {
    pi: usize,
    parent: Option<usize>,
    call_pc: usize,
}

/// A trace being formed (docs/design/trace-tier.md section 13): its
/// frames, its steps in order, the edges between them with their counts,
/// which (side) trace each step is in, the loops closed inside it, and the
/// side traces (the edge each starts at, its count).
#[derive(Clone, Default)]
struct Graph {
    frames: Vec<FrameRec>,
    nodes: Vec<Node>,
    set: HashSet<Node>,
    succ: HashMap<Node, Vec<(Node, u64)>>,
    owner: HashMap<Node, usize>,
    heads: HashSet<Node>,
    side_entry: HashMap<(Node, Node), usize>,
    /// Per side trace (1..): (from, first step, count, steps).
    sides: Vec<(Node, Node, u64, usize)>,
}

impl Graph {
    fn add_node(&mut self, n: Node, owner: usize) {
        if self.set.insert(n) {
            self.nodes.push(n);
            self.owner.insert(n, owner);
        }
    }
    fn add_edge(&mut self, a: Node, b: Node, n: u64) {
        let v = self.succ.entry(a).or_default();
        if !v.iter().any(|x| x.0 == b) {
            v.push((b, n));
        }
    }
    fn has_edge(&self, a: Node, b: Node) -> bool {
        self.succ.get(&a).is_some_and(|v| v.iter().any(|x| x.0 == b))
    }
}

/// What forming traces needs: the profiles, their regions' data (made on
/// demand), the regions of each code object, and the entry regions by
/// code.
struct Former<'p> {
    m: &'p Module<'p>,
    vm: &'p VmInfo<'p>,
    profs: &'p [ProfRegion],
    opts: &'p Options,
    data: HashMap<usize, Result<RegionData, String>>,
    /// code key -> (profile, its instructions)
    by_code: HashMap<u64, Vec<(usize, BTreeSet<usize>)>>,
    entries: HashMap<(usize, u64), usize>,
    /// The trace's header (frame 0).
    h: usize,
    /// `frame_moves` per (code, pc).
    moves: HashMap<(u64, usize), (BTreeSet<usize>, bool)>,
}

impl<'p> Former<'p> {
    fn rd(&mut self, pi: usize) -> Result<&RegionData, String> {
        let (m, vm, profs, opts) = (self.m, self.vm, self.profs, self.opts);
        self.data.entry(pi).or_insert_with(|| region_data(m, vm, &profs[pi], opts)).as_ref().map_err(|e| e.clone())
    }

    /// The profiled region of `code` that has pc (a loop's, without its
    /// inner loops, or the code's entry region), if its data is good.
    fn region_of(&mut self, code: &Code, pc: usize) -> Option<usize> {
        let pi = self.by_code.get(&code.key())?.iter().find(|(_, pcs)| pcs.contains(&pc))?.0;
        self.rd(pi).ok()?;
        Some(pi)
    }

    fn code(&self, g: &Graph, f: usize) -> &'p Code {
        &self.profs[g.frames[f].pi].code
    }

    fn depth(g: &Graph, f: usize) -> usize {
        let (mut d, mut f) = (0, f);
        while let Some(p) = g.frames[f].parent {
            d += 1;
            f = p;
        }
        d
    }

    /// Where the path may go from `node`, with counts: edges in its
    /// region, static exits to other regions of its code (an inner loop,
    /// the code after the loop), dynamic jumps in its code, a return to
    /// the calling frame, calls of profiled code objects (monomorphic
    /// sites only if `mono`).
    fn cands(&mut self, g: &Graph, node: Node, mono: bool) -> Vec<(u64, Next)> {
        let code = self.code(g, node.f);
        let Some(ri) = self.region_of(code, node.pc) else { return Vec::new() };
        let mut out: Vec<(u64, Next)> = Vec::new();
        let mut add = |n: u64, nx: Next| match out.iter_mut().find(|x| x.1 == nx) {
            Some(x) => x.0 += n,
            None => out.push((n, nx)),
        };
        let (edges, exits, site) = {
            let rd = self.rd(ri).unwrap();
            let r = (node.pc, 0)..=(node.pc, usize::MAX);
            let e: Vec<(usize, u64)> = rd.edges.range(r.clone()).map(|(k, n)| (k.1, *n)).collect();
            let x: Vec<(usize, u64)> = rd.exits.range(r).map(|(k, n)| (k.1, *n)).collect();
            (e, x, rd.sites.get(&node.pc).cloned())
        };
        let (m, vm) = (self.m, self.vm);
        let (other, keeps) = self.moves.entry((code.key(), node.pc)).or_insert_with(|| frame_moves(m, vm, code, node.pc)).clone();
        // (an edge to the instruction itself, `redo`, is not a path: the
        // step goes around itself)
        for (t, n) in edges {
            if t != node.pc && n > 0 && !other.contains(&t) {
                add(n, Next::Same(t));
            }
        }
        for (t, n) in exits {
            if t != node.pc && n > 0 && !other.contains(&t) && self.region_of(code, t).is_some() {
                add(n, Next::Same(t));
            }
        }
        if let Some((total, targets)) = site {
            let depth = Self::depth(g, node.f);
            if let Some(pf) = g.frames[node.f].parent {
                // a return: to the calling frame, after the call
                let pcode = self.code(g, pf);
                let call_pc = g.frames[node.f].call_pc;
                let mut rets: Vec<&Target> = targets.iter().filter(|t| t.n == pcode.n && t.hash == pcode.hash).collect();
                rets.sort_by_key(|t| (t.off <= call_pc, t.off));
                if let Some(t) = rets.first() {
                    if t.count > 0 {
                        add(t.count, Next::Ret(t.off));
                    }
                }
            }
            for t in &targets {
                if t.count == 0 {
                    continue;
                }
                // a jump to a computed pc in the same code (a jump table)
                // (one that may change frames only to the next
                // instruction: the end of a call of a native, which
                // dispatches through the table as a cold block's `next`)
                let same_frame = keeps || (node.pc + 1..code.n).find(|&i| code.op[i].is_some()) == Some(t.off);
                if t.n == code.n && t.hash == code.hash && t.off != node.pc && same_frame && g.frames[node.f].parent.is_none_or(|pf| (self.code(g, pf).n, self.code(g, pf).hash) != (t.n, t.hash)) {
                    if self.region_of(code, t.off).is_some() {
                        add(t.count, Next::Same(t.off));
                    }
                    continue;
                }
                let Some(&ei) = self.entries.get(&(t.n, t.hash)) else { continue };
                let recursive = {
                    let mut f = Some(node.f);
                    let mut r = false;
                    while let Some(x) = f {
                        let c = self.code(g, x);
                        r |= (c.n, c.hash) == (t.n, t.hash);
                        f = g.frames[x].parent;
                    }
                    r
                };
                // monomorphic: the site called this code (nearly) always
                if t.off == 0 && !recursive && depth < self.opts.inline_depth && (!mono || (t.count as f64) >= self.opts.mono * (total as f64)) {
                    add(t.count, Next::Call(ei));
                }
            }
        }
        if std::env::var_os("LOIMI_TRACE_DEBUG_CANDS").is_some() {
            eprintln!("cands f{} pc {}: {:?}", node.f, node.pc, out);
        }
        out
    }

    /// The step `nx` leads to from `cur`, if its frame exists (a call
    /// whose frame is not made yet: None).
    fn peek(g: &Graph, cur: Node, nx: Next) -> Option<Node> {
        match nx {
            Next::Same(t) => Some(Node::new(cur.f, t)),
            Next::Ret(t) => Some(Node::new(g.frames[cur.f].parent.unwrap(), t)),
            Next::Call(ei) => g.frames.iter().position(|fr| *fr == FrameRec { pi: ei, parent: Some(cur.f), call_pc: cur.pc }).map(|f| Node::new(f, 0)),
        }
    }

    /// ... making the frame of a call.
    fn materialize(g: &mut Graph, cur: Node, nx: Next) -> Node {
        if let Some(n) = Self::peek(g, cur, nx) {
            return n;
        }
        let Next::Call(ei) = nx else { unreachable!() };
        g.frames.push(FrameRec { pi: ei, parent: Some(cur.f), call_pc: cur.pc });
        Node::new(g.frames.len() - 1, 0)
    }

    /// Follows the hottest path from `first` (the header for the root,
    /// owner 0; else the first step of side trace `owner`, entered from
    /// `from`) until it reaches the header (the root) or a step of the
    /// trace (a side trace rejoins it). A loop on the way (an inner loop)
    /// is closed and left by its hottest exit. Returns the count of the
    /// edge that closed the path.
    fn walk(&mut self, g: &mut Graph, from: Option<Node>, first: Node, owner: usize, n0: u64) -> Result<u64, String> {
        let root = owner == 0;
        let header = Node::new(0, self.h);
        let mut path: Vec<Node> = vec![first];
        if let Some(f) = from {
            g.add_edge(f, first, n0);
        }
        g.add_node(first, owner);
        let mut cur = first;
        // a step that is in the trace already, not this walk's: the end
        let joins = |g: &Graph, path: &[Node], n: Node| if root { n == header } else { g.set.contains(&n) && !path.contains(&n) };
        for _ in 0..self.opts.max_walk {
            let cs = self.cands(g, cur, true);
            let best = cs.into_iter().max_by_key(|(n, nx)| {
                (*n, std::cmp::Reverse(match nx {
                    Next::Same(t) | Next::Ret(t) => *t,
                    Next::Call(_) => 0,
                }))
            });
            let Some((n, nx)) = best else {
                return Err(format!("the path leaves the loop after {}{}", cur.pc, if cur.f > 0 { format!(" (in frame {})", cur.f) } else { String::new() }));
            };
            let node = Self::materialize(g, cur, nx);
            if joins(g, &path, node) {
                g.add_edge(cur, node, n);
                return Ok(n);
            }
            let Some(mut i) = path.iter().position(|&x| x == node) else {
                g.add_edge(cur, node, n);
                g.add_node(node, owner);
                path.push(node);
                cur = node;
                continue;
            };
            // a loop inside the path (an inner loop): closed at `node`, and
            // left by the hottest edge out of it
            if !self.opts.nest {
                return Err(format!("an inner cycle at {}{}", node.pc, if node.f > 0 { format!(" (in frame {})", node.f) } else { String::new() }));
            }
            g.add_edge(cur, node, n);
            g.heads.insert(node);
            loop {
                let mut out: Option<(u64, Node, Next)> = None;
                for &u in &path[i..] {
                    for (c, nx) in self.cands(g, u, true) {
                        let tgt = Self::peek(g, u, nx);
                        if tgt == Some(u) || tgt.is_some_and(|t| path[i..].contains(&t)) {
                            continue;
                        }
                        if out.as_ref().is_none_or(|o| c > o.0) {
                            out = Some((c, u, nx));
                        }
                    }
                }
                let Some((c, u, nx)) = out else {
                    return Err(format!("an inner loop at {}{} that the path does not leave", node.pc, if node.f > 0 { format!(" (in frame {})", node.f) } else { String::new() }));
                };
                let w = Self::materialize(g, u, nx);
                g.add_edge(u, w, c);
                if joins(g, &path, w) {
                    return Ok(c);
                }
                if let Some(j) = path.iter().position(|&x| x == w) {
                    // into an earlier step of this walk: an outer loop
                    g.heads.insert(w);
                    i = j;
                    continue;
                }
                g.add_node(w, owner);
                path.push(w);
                cur = w;
                break;
            }
        }
        Err(format!("the path is longer than {} steps", self.opts.max_walk))
    }

    /// Lines of stencil IR of the instruction at `n` (with its cold
    /// blocks): what a trace's size is estimated from.
    fn est_lines(&self, g: &Graph, n: Node) -> usize {
        let code = self.code(g, n.f);
        let Some(name) = stencil_name(code, n.pc) else { return 0 };
        let Some(body) = self.m.bodies.get(name.as_str()) else { return 0 };
        let mut lines = body.len();
        for l in body {
            if let Some(c) = parse_call(def_of(l).1) {
                if c.global && c.callee.contains(".cold") {
                    lines += self.m.bodies.get(c.callee).map_or(0, |b| b.len());
                }
            }
        }
        lines
    }

    /// Adds side traces to `g` (whose root closed `back` times): the
    /// hottest edges from its steps that leave it, each the start of a
    /// walk that rejoins it, while their counts are over the threshold and
    /// the trace's estimated size within the budget.
    fn add_sides(&mut self, g: &mut Graph, back: u64) -> Vec<String> {
        let mut notes = Vec::new();
        let min = self.opts.side_min.max((self.opts.side_frac * back as f64).ceil() as u64).max(1);
        let mut tried: HashSet<(Node, Next)> = HashSet::new();
        let mut lines: usize = g.nodes.iter().map(|&n| self.est_lines(g, n)).sum();
        while g.sides.len() < self.opts.max_sides {
            // the hottest edge out of the trace not tried yet
            let mut best: Option<(u64, Node, Next)> = None;
            for u in g.nodes.clone() {
                for (c, nx) in self.cands(g, u, false) {
                    if c < min || tried.contains(&(u, nx)) {
                        continue;
                    }
                    let tgt = Self::peek(g, u, nx);
                    if tgt == Some(u) || tgt.is_some_and(|t| g.has_edge(u, t)) {
                        continue;
                    }
                    if best.as_ref().is_none_or(|b| c > b.0) {
                        best = Some((c, u, nx));
                    }
                }
            }
            let Some((c, u, nx)) = best else { break };
            tried.insert((u, nx));
            let saved = g.clone();
            let first = Self::materialize(g, u, nx);
            if g.set.contains(&first) {
                // into a step of the trace: an edge, no new steps
                g.add_edge(u, first, c);
                continue;
            }
            let k = g.sides.len() + 1;
            match self.walk(g, Some(u), first, k, c) {
                Ok(_) => {
                    let new: usize = g.nodes.len() - saved.nodes.len();
                    let add: usize = g.nodes[saved.nodes.len()..].iter().map(|&n| self.est_lines(g, n)).sum();
                    if lines + add > self.opts.side_lines {
                        notes.push(format!("side from {} to {} ({} runs): over the size budget (LOIMI_TRACE_SIDE_LINES={})", u.pc, first.pc, c, self.opts.side_lines));
                        *g = saved;
                        continue;
                    }
                    lines += add;
                    g.side_entry.insert((u, first), k);
                    g.sides.push((u, first, c, new));
                }
                Err(e) => {
                    if self.opts.verbose {
                        notes.push(format!("no side from {} to {} ({} runs): {}", u.pc, first.pc, c, e));
                    }
                    *g = saved;
                }
            }
        }
        notes
    }
}

/// `--traces`: per profiled loop that ran often enough, the hottest path
/// as a trace, through the calls it makes to code that was profiled (an
/// entry region) where the call site called one code object, and through
/// the loops inside it; then side traces from its hot exits. Returns the
/// IR to append and a report.
pub fn traces(ir: &str, stencils: &str, vm: &VmInfo, profs: &[ProfRegion], opts: &Options) -> Result<(String, String), String> {
    let m = Module::new(ir, stencils);
    check_sig(&m, vm)?;
    let sh = stencil_hash(stencils);
    let mut out = Out { lines: 0, defs: String::new(), entries: Vec::new(), globals: Vec::new() };
    let mut report = String::new();
    for pr in profs {
        if region_fingerprint(sh, &pr.code, pr.header) != pr.fingerprint {
            return Err(format!("profile region at {} does not match this build: build the profiling binary again", pr.header));
        }
    }
    // entry regions by their code (words and hash), and every region by
    // its code
    let mut entries: HashMap<(usize, u64), usize> = HashMap::new();
    let mut by_code: HashMap<u64, Vec<(usize, BTreeSet<usize>)>> = HashMap::new();
    for (i, pr) in profs.iter().enumerate() {
        if pr.header == 0 && !loops(&m, &pr.code).contains_key(&0) {
            entries.insert((pr.code.n, pr.code.hash), i);
        }
        if let Ok(pcs) = region_pcs(&m, pr, &opts.skip) {
            by_code.entry(pr.code.key()).or_default().push((i, pcs.into_iter().collect()));
        }
    }
    let mut fm = Former { m: &m, vm, profs, opts, data: HashMap::new(), by_code, entries, h: 0, moves: HashMap::new() };
    for (pi, pr) in profs.iter().enumerate() {
        let code = &pr.code;
        let h = pr.header;
        let lp = loops(&m, code);
        if !lp.contains_key(&h) {
            if !fm.entries.values().any(|&e| e == pi) {
                let _ = writeln!(report, "no trace for code {:016x} loop {}: not a loop in this build", code.hash, h);
            }
            continue;
        }
        if !opts.only.is_empty() && !opts.only.contains(&h) {
            continue;
        }
        if let Err(e) = fm.rd(pi) {
            if e.contains("build the profiling binary again") {
                return Err(format!("profile region at {}: {}", h, e));
            }
            let _ = writeln!(report, "no trace for code {:016x} loop {}: {}", code.hash, h, e);
            continue;
        }
        fm.h = h;
        // the root: the hottest path around the loop
        let mut g = Graph { frames: vec![FrameRec { pi, parent: None, call_pc: 0 }], ..Default::default() };
        let header = Node::new(0, h);
        let mut why = None;
        let back = match fm.walk(&mut g, None, header, 0, 0) {
            Ok(n) => n,
            Err(e) => {
                why = Some(e);
                0
            }
        };
        let (hruns, around) = {
            let rd0 = fm.rd(pi).unwrap();
            (rd0.hruns, rd0.around)
        };
        if why.is_none() && back < opts.min_iters {
            why = Some(format!("{} iterations (fewer than {})", back, opts.min_iters));
        }
        // entries: the header's runs that did not come around the loop
        let per_entry = back as f64 / (hruns.saturating_sub(around).max(1)) as f64;
        if why.is_none() && per_entry < opts.min_run {
            why = Some(format!("{:.1} iterations per entry (fewer than LOIMI_TRACE_MINRUN={})", per_entry, opts.min_run));
        }
        if let Some(w) = why {
            let _ = writeln!(report, "no trace for code {:016x} loop {}: {}", code.hash, h, w);
            if opts.verbose {
                let ps: Vec<String> = g.nodes.iter().map(|n| if n.f == 0 { format!("{}", n.pc) } else { format!("f{}/{}", n.f, n.pc) }).collect();
                let _ = writeln!(report, "  path {}", ps.join(" "));
            }
            continue;
        }
        let root_steps = g.nodes.len();
        let notes = if opts.max_sides > 0 { fm.add_sides(&mut g, back) } else { Vec::new() };
        // generated with fewer side traces while it is too large
        let mut nsides = g.sides.len();
        let full = g.clone();
        let made = loop {
            let g = if nsides == full.sides.len() { full.clone() } else { trim_sides(&full, nsides) };
            // a block is on the trace if it ran a fraction of its step's runs
            let mut on_blocks = HashMap::new();
            let mut raw = HashMap::new();
            for &n in &g.nodes {
                let code = fm.code(&g, n.f);
                let Some(ri) = fm.region_of(code, n.pc) else { continue };
                let rd = fm.rd(ri).unwrap();
                for ((pc, lab), c) in &rd.blocks {
                    if *pc != n.pc {
                        continue;
                    }
                    raw.insert((n.f, *pc, lab.clone()), *c);
                    let mn = ((rd.first_count[pc] as f64) * opts.min_frac).ceil() as u64;
                    on_blocks.insert((n.f, *pc, lab.clone()), if *c >= mn.max(1) { *c } else { 0 });
                }
            }
            let prof = Prof { blocks: on_blocks, raw, min: 1 };
            let pframes: Vec<PathFrame> = g.frames.iter().map(|f| PathFrame { code: &profs[f.pi].code, parent: f.parent, call: f.parent.map(|p| Node::new(p, f.call_pc)) }).collect();
            let name = format!("t{}", out.entries.len());
            let index = out.entries.len();
            let mut shape = Shape { owner: g.owner.clone(), side_entry: g.side_entry.clone(), heads: g.heads.clone(), loop_entries: loop_entries(&g, header), ..Default::default() };
            for (a, v) in &g.succ {
                let mut v = v.clone();
                v.sort_by_key(|x| std::cmp::Reverse(x.1));
                shape.succ.insert(*a, v.into_iter().map(|x| x.0).collect());
            }
            shape.sides.push((name.clone(), index));
            for k in 1..=g.sides.len() {
                shape.sides.push((format!("{}.s{}", name, k), index + k));
            }
            match generate_fn(&m, vm, &pframes, h, &g.nodes, Kind::Trace, &name, index, shape, Some(prof), opts) {
                Err(e) if e.contains("LOIMI_TRACE_MAXLINES") && nsides > 0 => nsides /= 2,
                r => {
                    break (r, g, pframes.iter().map(|f| f.code).collect::<Vec<&Code>>(), name, index);
                }
            }
        };
        let (r, g, fcodes, name, index) = made;
        match r {
            Ok(gf) if out.lines + gf.lines > opts.max_total => {
                let _ = writeln!(report, "no trace for code {:016x} loop {}: the traces so far have {} lines of IR (LOIMI_TRACE_MAXTOTAL={})", code.hash, h, out.lines, opts.max_total);
            }
            Ok(gf) if gf.inlined_writes > 0 && !opts.inline_writers => {
                let _ = writeln!(report, "no trace for code {:016x} loop {}: {} steps of its inlined calls call functions that may write (LOIMI_TRACE_INLINE_WRITERS=1 keeps it)", code.hash, h, gf.inlined_writes);
            }
            Ok(gf) => {
                out.lines += gf.lines;
                out.defs.push_str(&gf.ir);
                let order: Vec<usize> = g.nodes.iter().filter(|n| n.f == 0).map(|n| n.pc).collect();
                let callees = (1..g.frames.len()).map(|f| (fcodes[f], 0usize, &gf.kchk[f])).collect();
                let info = EntryInfo { order, ncounts: 0, kchk: &gf.kchk[0], rec: None, nsites: 0, callees, fnsym: None };
                add_entry(&mut out, &m, code, h, &name, &info, pr.fingerprint, opts.fail_limit);
                for k in 1..=g.sides.len() {
                    add_side_entry(&mut out, &format!("{}.s{}", name, k), index, opts.fail_limit);
                }
                let label = |n: &Node| {
                    let c = fcodes[n.f];
                    let op = c.op[n.pc].as_deref().unwrap_or("?");
                    if n.f == 0 { format!("{}:{}", n.pc, op) } else { format!("f{}/{}:{}", n.f, n.pc, op) }
                };
                let ops: Vec<String> = g.nodes[..root_steps.min(g.nodes.len())].iter().map(label).collect();
                let _ = writeln!(
                    report,
                    "trace {}: code {:016x} ({} words), loop {}, {} iterations ({:.1} per entry), {} inlined calls, {} loops inside, {} side traces ({} steps), window {}, {} guards, {} steps with effects, {} of {} calling writers: {}",
                    name,
                    code.hash,
                    code.n,
                    h,
                    back,
                    per_entry,
                    g.frames.len() - 1,
                    g.heads.len(),
                    g.sides.len(),
                    g.nodes.len() - root_steps,
                    gf.nr,
                    gf.nguards,
                    gf.neffect,
                    gf.nwrites,
                    g.nodes.len(),
                    ops.join(" ")
                );
                for (k, (from, first, c, n)) in g.sides.iter().enumerate() {
                    let steps: Vec<String> = g.nodes.iter().filter(|x| g.owner.get(x) == Some(&(k + 1))).map(label).collect();
                    let _ = writeln!(report, "  side {}.s{}: from {} at {} runs, {} steps: {}", name, k + 1, label(from), c, n, steps.join(" "));
                    let _ = first;
                }
                if nsides < full.sides.len() {
                    let _ = writeln!(report, "  ({} side traces left out: LOIMI_TRACE_MAXLINES)", full.sides.len() - nsides);
                }
                for nt in &notes {
                    let _ = writeln!(report, "  {}", nt);
                }
                if opts.verbose {
                    for gi in &gf.guard_info {
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

/// The edges into the loops inside trace `g` (at its heads) from outside
/// them: from a step the head does not dominate.
fn loop_entries(g: &Graph, header: Node) -> HashSet<(Node, Node)> {
    let idx: HashMap<Node, usize> = g.nodes.iter().enumerate().map(|(i, n)| (*n, i)).collect();
    let n = g.nodes.len();
    let mut preds: Vec<Vec<usize>> = vec![Vec::new(); n];
    for (a, v) in &g.succ {
        for (b, _) in v {
            if let (Some(&i), Some(&j)) = (idx.get(a), idx.get(b)) {
                preds[j].push(i);
            }
        }
    }
    // dominators, iteratively (traces have hundreds of steps at most)
    let h = idx[&header];
    let full: Vec<bool> = vec![true; n];
    let mut dom: Vec<Vec<bool>> = vec![full; n];
    dom[h] = vec![false; n];
    dom[h][h] = true;
    let mut changed = true;
    while changed {
        changed = false;
        for b in 0..n {
            if b == h {
                continue;
            }
            let mut d: Vec<bool> = vec![true; n];
            let mut any = false;
            for &p in &preds[b] {
                any = true;
                for (x, y) in d.iter_mut().zip(&dom[p]) {
                    *x = *x && *y;
                }
            }
            if !any {
                d = vec![false; n];
            }
            d[b] = true;
            if d != dom[b] {
                dom[b] = d;
                changed = true;
            }
        }
    }
    let mut out = HashSet::new();
    for (a, v) in &g.succ {
        for (b, _) in v {
            if *b != header && g.heads.contains(b) {
                if let (Some(&i), Some(&j)) = (idx.get(a), idx.get(b)) {
                    if !dom[i][j] {
                        out.insert((*a, *b));
                    }
                }
            }
        }
    }
    out
}

/// `g` with only its first `n` side traces (their steps, the edges into
/// and inside them, the frames they made).
fn trim_sides(g: &Graph, n: usize) -> Graph {
    let keep = |x: &Node| g.owner.get(x).is_some_and(|&o| o <= n);
    let mut t = g.clone();
    t.nodes.retain(keep);
    t.set.retain(keep);
    t.owner.retain(|x, _| keep(x));
    t.succ.retain(|x, _| keep(x));
    for v in t.succ.values_mut() {
        v.retain(|x| keep(&x.0));
    }
    t.side_entry.retain(|_, k| *k <= n);
    t.heads.retain(keep);
    t.sides.truncate(n);
    t
}
