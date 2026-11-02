//! `--jit-stencils`: read the stencils back from the object file clang
//! made of them (ELF64, x86-64), and emit them as data for the run-time
//! JIT (`jit_rt.c`).
//!
//! Each stencil is one function in its own section (`-ffunction-sections`).
//! Its relocations become patch records: holes (`__jit.*` symbols, see
//! `jitgen.rs`), symbols of the interpreter (resolved through a table of
//! their addresses that the program carries), thread-locals (resolved to
//! their offsets from the thread pointer at run time) and read-only data
//! of the object (float constants and the like: copied once, next to the
//! code).

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt::Write;

const SHT_SYMTAB: u32 = 2;
const SHT_RELA: u32 = 4;
const SHF_WRITE: u64 = 1;
const SHF_ALLOC: u64 = 2;
const SHF_EXECINSTR: u64 = 4;

const R_X86_64_64: u32 = 1;
const R_X86_64_PC32: u32 = 2;
const R_X86_64_PLT32: u32 = 4;
const R_X86_64_32: u32 = 10;
const R_X86_64_32S: u32 = 11;
const R_X86_64_TPOFF32: u32 = 23;

// Patch kinds (must match jit_rt.c).
const K_SYM: u16 = 0;
const K_WORD: u16 = 1;
const K_FIELD: u16 = 2;
const K_NEXT: u16 = 3;
const K_JUMP: u16 = 4;
const K_TLS: u16 = 5;
const K_RODATA: u16 = 6;
const K_HELPER: u16 = 7;
/// A stencil's own code: arg is an offset into its hot part followed by
/// its cold part (which the JIT places apart).
const K_SELF: u16 = 8;
/// A constant of a `const` operand (a 64-bit hole): arg as for K_FIELD,
/// patched with the pool entry the field indexes.
const K_CONST: u16 = 9;

struct Section {
    name: String,
    ty: u32,
    flags: u64,
    off: usize,
    size: usize,
    link: u32,
    info: u32,
    align: u64,
}

struct Sym {
    name: String,
    shndx: u16,
    value: u64,
}

fn u16_at(b: &[u8], o: usize) -> u16 {
    u16::from_le_bytes(b[o..o + 2].try_into().unwrap())
}
fn u32_at(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(b[o..o + 4].try_into().unwrap())
}
fn u64_at(b: &[u8], o: usize) -> u64 {
    u64::from_le_bytes(b[o..o + 8].try_into().unwrap())
}
fn cstr(b: &[u8], o: usize) -> String {
    let end = b[o..].iter().position(|&c| c == 0).map(|e| o + e).unwrap_or(b.len());
    String::from_utf8_lossy(&b[o..end]).into_owned()
}

struct Elf {
    data: Vec<u8>,
    sections: Vec<Section>,
    syms: Vec<Sym>,
}

fn parse(data: Vec<u8>) -> Result<Elf, String> {
    if data.len() < 64 || &data[0..4] != b"\x7fELF" || data[4] != 2 || data[5] != 1 || u16_at(&data, 18) != 62 {
        return Err("not an x86-64 ELF64 object".into());
    }
    let shoff = u64_at(&data, 0x28) as usize;
    let shentsize = u16_at(&data, 0x3a) as usize;
    let shnum = u16_at(&data, 0x3c) as usize;
    let shstrndx = u16_at(&data, 0x3e) as usize;
    let mut sections = Vec::new();
    for i in 0..shnum {
        let h = shoff + i * shentsize;
        sections.push(Section {
            name: String::new(),
            ty: u32_at(&data, h + 4),
            flags: u64_at(&data, h + 8),
            off: u64_at(&data, h + 24) as usize,
            size: u64_at(&data, h + 32) as usize,
            link: u32_at(&data, h + 40),
            info: u32_at(&data, h + 44),
            align: u64_at(&data, h + 48),
        });
    }
    let stro = sections[shstrndx].off;
    for i in 0..shnum {
        let h = shoff + i * shentsize;
        sections[i].name = cstr(&data, stro + u32_at(&data, h) as usize);
    }
    let mut syms = Vec::new();
    if let Some(st) = sections.iter().find(|s| s.ty == SHT_SYMTAB) {
        let strtab = sections[st.link as usize].off;
        for k in 0..st.size / 24 {
            let e = st.off + k * 24;
            syms.push(Sym {
                name: cstr(&data, strtab + u32_at(&data, e) as usize),
                shndx: u16_at(&data, e + 6),
                value: u64_at(&data, e + 8),
            });
        }
    }
    Ok(Elf { data, sections, syms })
}

#[derive(Clone, Copy)]
struct Reloc {
    off: u32,
    ty: u32,
    kind: u16,
    addend: i64,
    arg: u64,
}

/// The program-side tables, as LLVM IR, and a report of stencils that
/// could not be used (their ops exit to the interpreter).
pub struct Tables {
    pub ir: String,
    pub rejected: Vec<(String, String)>,
    pub count: usize,
    pub bytes: usize,
    pub helper_bytes: usize,
    pub helpers: usize,
}

fn parse_field(rest: &str) -> Option<u64> {
    let v: Vec<&str> = rest.split('.').collect();
    let (off, shift, bits): (u64, u64, u64) = (v.first()?.parse().ok()?, v.get(1)?.parse().ok()?, v.get(2)?.parse().ok()?);
    let signed = match v.get(3) {
        Some(&"s") | None => 1,
        Some(&"u") => 0,
        _ => return None,
    };
    Some(off | shift << 8 | bits << 16 | signed << 24)
}

/// What the relocations of stencils and helpers refer to, collected as
/// they are read.
struct Cx<'a> {
    known: &'a HashSet<String>,
    syms: Vec<String>,
    sym_ix: HashMap<String, usize>,
    tls: Vec<String>,
    tls_ix: HashMap<String, usize>,
    /// Read-only sections copied into the JIT's data: section → offset.
    rodata: Vec<u8>,
    ro_ix: HashMap<usize, usize>,
    /// Relocations of the copied read-only data (jump tables of helpers).
    drelocs: Vec<Reloc>,
}

impl Cx<'_> {
    /// The patch records of text section `si`. `holes`: a stencil (a
    /// helper must not use holes). A call to another function of the
    /// object (a helper) is a `K_HELPER` record with arg (section << 32 |
    /// offset), resolved once the helper code is laid out.
    /// `own`: a stencil's hot and cold sections (references to them are
    /// `K_SELF` records), and the hot section's size.
    fn relocs(&mut self, elf: &Elf, si: usize, holes: bool, own: Option<(usize, Option<usize>, u64)>) -> Result<Vec<Reloc>, String> {
        let mut rs = Vec::new();
        for rsec in elf.sections.iter().filter(|r| r.ty == SHT_RELA && r.info as usize == si) {
            for k in 0..rsec.size / 24 {
                let e = rsec.off + k * 24;
                let r_off = u64_at(&elf.data, e) as u32;
                let info = u64_at(&elf.data, e + 8);
                let addend = u64_at(&elf.data, e + 16) as i64;
                let (sidx, ty) = ((info >> 32) as usize, info as u32);
                let s = &elf.syms[sidx];
                if !matches!(ty, R_X86_64_64 | R_X86_64_PC32 | R_X86_64_PLT32 | R_X86_64_32 | R_X86_64_32S | R_X86_64_TPOFF32) {
                    return Err(format!("relocation type {} against {}", ty, s.name));
                }
                let hole = s.name.starts_with("__jit.");
                if hole && !holes {
                    return Err(format!("a helper uses hole {}", s.name));
                }
                let (kind, arg) = if s.shndx == 0 {
                    if let Some(r) = s.name.strip_prefix("__jit.w.") {
                        (K_WORD, r.parse::<u64>().map_err(|_| format!("bad hole {}", s.name))?)
                    } else if let Some(r) = s.name.strip_prefix("__jit.f.") {
                        (K_FIELD, parse_field(r).ok_or(format!("bad hole {}", s.name))?)
                    } else if let Some(r) = s.name.strip_prefix("__jit.c.") {
                        (K_CONST, parse_field(r).ok_or(format!("bad hole {}", s.name))?)
                    } else if let Some(r) = s.name.strip_prefix("__jit.n.") {
                        (K_NEXT, r.parse::<i64>().map_err(|_| format!("bad hole {}", s.name))? as u64)
                    } else if let Some(r) = s.name.strip_prefix("__jit.j.") {
                        (K_JUMP, parse_field(r).ok_or(format!("bad hole {}", s.name))?)
                    } else if ty == R_X86_64_TPOFF32 {
                        if !self.known.contains(&s.name) {
                            return Err(format!("unknown thread-local {}", s.name));
                        }
                        (K_TLS, ix(&mut self.tls, &mut self.tls_ix, &s.name) as u64)
                    } else {
                        (K_SYM, ix(&mut self.syms, &mut self.sym_ix, &s.name) as u64)
                    }
                } else {
                    let t = s.shndx as usize;
                    let ts = elf.sections.get(t).ok_or("bad section index")?;
                    if let Some((hot, _, hot_len)) = own.filter(|o| o.0 == t || o.1 == Some(t)) {
                        (K_SELF, s.value + if t == hot { 0 } else { hot_len })
                    } else if ts.flags & SHF_EXECINSTR != 0 {
                        if ts.name.starts_with(".text.__jit.") || ts.name.starts_with(".text.split.__jit.") {
                            return Err(format!("reference to stencil {}", ts.name));
                        }
                        if ty != R_X86_64_PLT32 && !s.name.is_empty() && s.name != ts.name && self.known.contains(&s.name) {
                            // The function's address, not a call: the
                            // interpreter's copy, so that it compares
                            // equal to the interpreter's.
                            (K_SYM, ix(&mut self.syms, &mut self.sym_ix, &s.name) as u64)
                        } else {
                            (K_HELPER, (t as u64) << 32 | s.value)
                        }
                    } else {
                        if ts.flags & SHF_ALLOC == 0 || ts.flags & SHF_WRITE != 0 {
                            return Err(format!("reference to section {}", ts.name));
                        }
                        let relocated = elf.sections.iter().any(|r| r.ty == SHT_RELA && r.info as usize == t);
                        if relocated && holes {
                            // Per-copy data of a stencil: not supported.
                            return Err(format!("data with relocations ({})", ts.name));
                        }
                        let base = match self.ro_ix.get(&t) {
                            Some(&b) => b,
                            None => {
                                let a = ts.align.max(1) as usize;
                                while self.rodata.len() % a != 0 {
                                    self.rodata.push(0);
                                }
                                let b = self.rodata.len();
                                self.rodata.extend_from_slice(&elf.data[ts.off..ts.off + ts.size]);
                                self.ro_ix.insert(t, b);
                                if relocated {
                                    // A helper's jump table: patched once.
                                    for rl in self.relocs(elf, t, false, None)? {
                                        self.drelocs.push(Reloc { off: rl.off + b as u32, ..rl });
                                    }
                                }
                                b
                            }
                        };
                        (K_RODATA, base as u64 + s.value)
                    }
                };
                rs.push(Reloc { off: r_off, ty, kind, addend, arg });
            }
        }
        Ok(rs)
    }
}

fn ix(v: &mut Vec<String>, m: &mut HashMap<String, usize>, name: &str) -> usize {
    if let Some(&i) = m.get(name) {
        return i;
    }
    v.push(name.to_string());
    m.insert(name.to_string(), v.len() - 1);
    v.len() - 1
}

/// `ops`: (opcode, name) of every op; `nops`: 2^op_bits; `ir`: the
/// program's IR (to tell which symbols it defines).
pub fn stencil_tables(obj: Vec<u8>, ops: &[(u32, String)], nops: u32, ir: &str) -> Result<Tables, String> {
    let elf = parse(obj)?;
    // Names the program's IR defines or declares.
    let mut known: HashSet<String> = HashSet::new();
    for line in ir.lines() {
        let at = match line.find('@') {
            Some(a) if line.starts_with('@') || line.starts_with("define ") || line.starts_with("declare ") => a,
            _ => continue,
        };
        let rest = &line[at + 1..];
        let name = if let Some(r) = rest.strip_prefix('"') { r.split('"').next().unwrap_or("") } else { rest.split(|c: char| !(c.is_alphanumeric() || c == '_' || c == '.')).next().unwrap_or("") };
        known.insert(name.to_string());
    }
    let mut cx = Cx { known: &known, syms: Vec::new(), sym_ix: HashMap::new(), tls: Vec::new(), tls_ix: HashMap::new(), rodata: Vec::new(), ro_ix: HashMap::new(), drelocs: Vec::new() };
    let mut rejected = Vec::new();
    let by_name: HashMap<&str, u32> = ops.iter().map(|(o, n)| (n.as_str(), *o)).collect();
    // The stencils: (index (opcode, or nops for the exit stencil), name,
    // section, cold section (`-fsplit-machine-functions`), relocations).
    let mut found: Vec<(u32, String, usize, Option<usize>, Vec<Reloc>)> = Vec::new();
    for (si, sec) in elf.sections.iter().enumerate() {
        let Some(fname) = sec.name.strip_prefix(".text.") else { continue };
        let idx = if fname == "__jit.exit" {
            nops
        } else if let Some(op) = fname.strip_prefix("__jit.op.") {
            match by_name.get(op) {
                Some(&o) => o,
                None => continue,
            }
        } else {
            continue;
        };
        let name = if idx == nops { "<exit>".to_string() } else { fname.trim_start_matches("__jit.op.").to_string() };
        let split = format!(".text.split.{}", fname);
        let cold = elf.sections.iter().position(|c| c.name == split);
        let own = Some((si, cold, sec.size as u64));
        let rs = cx.relocs(&elf, si, true, own).and_then(|mut rs| {
            if let Some(c) = cold {
                for rl in cx.relocs(&elf, c, true, own)? {
                    rs.push(Reloc { off: rl.off + sec.size as u32, ..rl });
                }
            }
            Ok(rs)
        });
        match rs {
            Ok(rs) => found.push((idx, name, si, cold, rs)),
            Err(w) => rejected.push((name, w)),
        }
    }
    // Helpers: the functions the stencils call, and those they call.
    let deps = |rs: &[Reloc]| -> Vec<usize> { rs.iter().filter(|r| r.kind == K_HELPER).map(|r| (r.arg >> 32) as usize).collect() };
    let mut helpers: BTreeMap<usize, Result<Vec<Reloc>, String>> = BTreeMap::new();
    let mut work: Vec<usize> = found.iter().flat_map(|f| deps(&f.4)).collect();
    loop {
        while let Some(h) = work.pop() {
            if helpers.contains_key(&h) {
                continue;
            }
            let r = cx.relocs(&elf, h, false, None);
            if let Ok(rs) = &r {
                work.extend(deps(rs));
            }
            helpers.insert(h, r);
        }
        // Jump tables read so far may name more helpers.
        work = deps(&cx.drelocs).into_iter().filter(|h| !helpers.contains_key(h)).collect();
        if work.is_empty() {
            break;
        }
    }
    if let Some(d) = deps(&cx.drelocs).into_iter().find(|d| helpers[d].is_err()) {
        return Err(format!("a jump table refers to {}, which cannot be used: {}", elf.sections[d].name, helpers[&d].as_ref().err().unwrap()));
    }
    // A helper that calls one that cannot be used cannot be used either.
    loop {
        let bad: Vec<(usize, String)> = helpers
            .iter()
            .filter_map(|(h, r)| {
                let rs = r.as_ref().ok()?;
                let d = deps(rs).into_iter().find(|d| helpers[d].is_err())?;
                Some((*h, format!("calls {}: {}", elf.sections[d].name, helpers[&d].as_ref().err().unwrap())))
            })
            .collect();
        if bad.is_empty() {
            break;
        }
        for (h, why) in bad {
            helpers.insert(h, Err(why));
        }
    }
    // The helper code: each helper once, 16-byte aligned.
    let mut hcode: Vec<u8> = Vec::new();
    let mut hbase: HashMap<usize, usize> = HashMap::new();
    let mut hrelocs: Vec<Reloc> = Vec::new();
    for (h, r) in &helpers {
        let Ok(rs) = r else { continue };
        let sec = &elf.sections[*h];
        while hcode.len() % 16 != 0 {
            hcode.push(0xcc);
        }
        hbase.insert(*h, hcode.len());
        for rl in rs {
            hrelocs.push(Reloc { off: rl.off + hcode.len() as u32, ..*rl });
        }
        hcode.extend_from_slice(&elf.data[sec.off..sec.off + sec.size]);
    }
    let resolve = |rl: &mut Reloc| {
        if rl.kind == K_HELPER {
            rl.arg = hbase[&((rl.arg >> 32) as usize)] as u64 + (rl.arg & 0xffff_ffff);
        }
    };
    hrelocs.iter_mut().for_each(resolve);
    cx.drelocs.iter_mut().for_each(resolve);
    let mut code: Vec<u8> = Vec::new();
    let mut relocs: Vec<Reloc> = Vec::new();
    // Per stencil index: (code offset, length, first reloc, reloc count,
    // trim distance or -1, length of the hot part).
    let mut stencils: BTreeMap<u32, (usize, usize, usize, usize, i64, usize)> = BTreeMap::new();
    for (idx, name, si, cold, mut rs) in found {
        if let Some(d) = deps(&rs).into_iter().find(|d| helpers[d].is_err()) {
            rejected.push((name, format!("calls {}: {}", elf.sections[d].name, helpers[&d].as_ref().err().unwrap())));
            continue;
        }
        rs.iter_mut().for_each(resolve);
        let sec = &elf.sections[si];
        let bytes = &elf.data[sec.off..sec.off + sec.size];
        // The hot part ends in `jmp __jit.n.D`: the copy can drop it when
        // the code for pc + D follows.
        let n = bytes.len();
        let trim = rs
            .iter()
            .find(|r| r.kind == K_NEXT && r.off as usize + 4 == n && n >= 5 && bytes[n - 5] == 0xe9)
            .map(|r| r.arg as i64)
            .unwrap_or(-1);
        rs.sort_by_key(|r| r.off);
        let mut bytes = bytes.to_vec();
        if let Some(c) = cold {
            let cs = &elf.sections[c];
            bytes.extend_from_slice(&elf.data[cs.off..cs.off + cs.size]);
        }
        stencils.insert(idx, (code.len(), bytes.len(), relocs.len(), rs.len(), trim, n));
        code.extend_from_slice(&bytes);
        relocs.extend(rs);
    }
    let Cx { syms, tls, rodata, drelocs, .. } = cx;
    if !stencils.contains_key(&nops) {
        let why = rejected.iter().find(|r| r.0 == "<exit>").map(|r| r.1.clone()).unwrap_or_default();
        return Err(format!("the exit stencil is missing ({})", why));
    }
    let mut ir = String::new();
    let bytes_lit = |b: &[u8]| -> String {
        let mut s = String::with_capacity(b.len() * 3);
        for &c in b {
            let _ = write!(s, "\\{:02X}", c);
        }
        s
    };
    let _ = writeln!(ir, "@loimi_jit_code = constant [{} x i8] c\"{}\", align 16", code.len().max(1), bytes_lit(if code.is_empty() { &[0] } else { &code }));
    let rl: Vec<String> = relocs
        .iter()
        .map(|r| format!("{{ i32, i16, i16, i64, i64 }} {{ i32 {}, i16 {}, i16 {}, i64 {}, i64 {} }}", r.off, r.ty, r.kind, r.addend, r.arg as i64))
        .collect();
    let _ = writeln!(ir, "@loimi_jit_relocs = constant [{} x {{ i32, i16, i16, i64, i64 }}] [{}]", rl.len(), rl.join(", "));
    let hl: Vec<String> = hrelocs
        .iter()
        .map(|r| format!("{{ i32, i16, i16, i64, i64 }} {{ i32 {}, i16 {}, i16 {}, i64 {}, i64 {} }}", r.off, r.ty, r.kind, r.addend, r.arg as i64))
        .collect();
    let _ = writeln!(ir, "@loimi_jit_hcode = constant [{} x i8] c\"{}\", align 16", hcode.len().max(1), bytes_lit(if hcode.is_empty() { &[0] } else { &hcode }));
    let _ = writeln!(ir, "@loimi_jit_hcode_len = constant i64 {}", hcode.len());
    let _ = writeln!(ir, "@loimi_jit_hrelocs = constant [{} x {{ i32, i16, i16, i64, i64 }}] [{}]", hl.len(), hl.join(", "));
    let _ = writeln!(ir, "@loimi_jit_nhrelocs = constant i32 {}", hl.len());
    let dl: Vec<String> = drelocs
        .iter()
        .map(|r| format!("{{ i32, i16, i16, i64, i64 }} {{ i32 {}, i16 {}, i16 {}, i64 {}, i64 {} }}", r.off, r.ty, r.kind, r.addend, r.arg as i64))
        .collect();
    let _ = writeln!(ir, "@loimi_jit_drelocs = constant [{} x {{ i32, i16, i16, i64, i64 }}] [{}]", dl.len(), dl.join(", "));
    let _ = writeln!(ir, "@loimi_jit_ndrelocs = constant i32 {}", dl.len());
    let mut st = Vec::new();
    let last = ops.iter().map(|o| o.0).max().unwrap_or(0).max(nops);
    for i in 0..=last {
        let (o, l, r, nr, t, h, present) = match stencils.get(&i) {
            Some(&(o, l, r, nr, t, h)) => (o, l, r, nr, t, h, 1),
            None => (0, 0, 0, 0, -1, 0, 0),
        };
        st.push(format!("{{ i32, i32, i32, i32, i32, i32, i32 }} {{ i32 {}, i32 {}, i32 {}, i32 {}, i32 {}, i32 {}, i32 {} }}", o, l, r, nr, t, present, h));
    }
    let _ = writeln!(ir, "@loimi_jit_stencils = constant [{} x {{ i32, i32, i32, i32, i32, i32, i32 }}] [{}]", st.len(), st.join(", "));
    let mut decls = String::new();
    let sl: Vec<String> = syms
        .iter()
        .map(|s| {
            if !known.contains(s) {
                // A C library function LLVM called (memcpy, fmod, ...).
                let _ = writeln!(decls, "declare void @\"{}\"()", s);
            }
            format!("ptr @\"{}\"", s)
        })
        .collect();
    let _ = writeln!(ir, "@loimi_jit_syms = constant [{} x ptr] [{}]", sl.len(), sl.join(", "));
    ir.push_str(&decls);
    let _ = writeln!(ir, "@loimi_jit_rodata = constant [{} x i8] c\"{}\", align 64", rodata.len().max(1), bytes_lit(if rodata.is_empty() { &[0] } else { &rodata }));
    let _ = writeln!(ir, "@loimi_jit_rodata_len = constant i64 {}", rodata.len());
    let _ = writeln!(ir, "@loimi_jit_ntls = constant i32 {}", tls.len());
    // Offsets of the thread-locals the stencils use from the thread pointer
    // (%fs:0 holds the thread pointer itself on x86-64).
    let _ = writeln!(ir, "define void @loimi_jit_tls(ptr %out) {{");
    let _ = writeln!(ir, "  %tp = load i64, ptr addrspace(257) null");
    for (i, t) in tls.iter().enumerate() {
        let _ = writeln!(ir, "  %a{i} = call ptr @llvm.threadlocal.address.p0(ptr @\"{t}\")\n  %ai{i} = ptrtoint ptr %a{i} to i64\n  %o{i} = sub i64 %ai{i}, %tp\n  %p{i} = getelementptr inbounds i64, ptr %out, i64 {i}\n  store i64 %o{i}, ptr %p{i}");
    }
    let _ = writeln!(ir, "  ret void\n}}");
    if !tls.is_empty() && !known.contains("llvm.threadlocal.address.p0") {
        let _ = writeln!(ir, "declare ptr @llvm.threadlocal.address.p0(ptr)");
    }
    Ok(Tables { ir, rejected, count: stencils.len(), bytes: code.len(), helper_bytes: hcode.len(), helpers: hbase.len() })
}
