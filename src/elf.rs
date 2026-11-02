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
    let mut code: Vec<u8> = Vec::new();
    let mut relocs: Vec<Reloc> = Vec::new();
    // Per stencil index (opcode, or nops for the exit stencil): (code
    // offset, length, first reloc, reloc count, trim distance or -1).
    let mut stencils: BTreeMap<u32, (usize, usize, usize, usize, i64)> = BTreeMap::new();
    let mut syms: Vec<String> = Vec::new();
    let mut sym_ix: HashMap<String, usize> = HashMap::new();
    let mut tls: Vec<String> = Vec::new();
    let mut tls_ix: HashMap<String, usize> = HashMap::new();
    // Read-only sections copied into the JIT's data: section → offset.
    let mut rodata: Vec<u8> = Vec::new();
    let mut ro_ix: HashMap<usize, usize> = HashMap::new();
    let mut rejected = Vec::new();
    let by_name: HashMap<&str, u32> = ops.iter().map(|(o, n)| (n.as_str(), *o)).collect();
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
        let bytes = &elf.data[sec.off..sec.off + sec.size];
        let mut rs = Vec::new();
        let mut why: Option<String> = None;
        for rsec in elf.sections.iter().filter(|r| r.ty == SHT_RELA && r.info as usize == si) {
            for k in 0..rsec.size / 24 {
                let e = rsec.off + k * 24;
                let r_off = u64_at(&elf.data, e) as u32;
                let info = u64_at(&elf.data, e + 8);
                let addend = u64_at(&elf.data, e + 16) as i64;
                let (sidx, ty) = ((info >> 32) as usize, info as u32);
                let s = &elf.syms[sidx];
                if !matches!(ty, R_X86_64_64 | R_X86_64_PC32 | R_X86_64_PLT32 | R_X86_64_32 | R_X86_64_32S | R_X86_64_TPOFF32) {
                    why = Some(format!("relocation type {} against {}", ty, s.name));
                    break;
                }
                let (kind, arg) = if s.shndx == 0 {
                    if let Some(r) = s.name.strip_prefix("__jit.w.") {
                        (K_WORD, r.parse::<u64>().map_err(|_| format!("bad hole {}", s.name))?)
                    } else if let Some(r) = s.name.strip_prefix("__jit.f.") {
                        (K_FIELD, parse_field(r).ok_or(format!("bad hole {}", s.name))?)
                    } else if let Some(r) = s.name.strip_prefix("__jit.n.") {
                        (K_NEXT, r.parse::<i64>().map_err(|_| format!("bad hole {}", s.name))? as u64)
                    } else if let Some(r) = s.name.strip_prefix("__jit.j.") {
                        (K_JUMP, parse_field(r).ok_or(format!("bad hole {}", s.name))?)
                    } else if ty == R_X86_64_TPOFF32 {
                        if !known.contains(&s.name) {
                            why = Some(format!("unknown thread-local {}", s.name));
                            break;
                        }
                        let n = tls.len();
                        let i = *tls_ix.entry(s.name.clone()).or_insert_with(|| {
                            tls.push(s.name.clone());
                            n
                        });
                        (K_TLS, i as u64)
                    } else {
                        let n = syms.len();
                        let i = *sym_ix.entry(s.name.clone()).or_insert_with(|| {
                            syms.push(s.name.clone());
                            n
                        });
                        (K_SYM, i as u64)
                    }
                } else {
                    let t = s.shndx as usize;
                    let ts = elf.sections.get(t).ok_or("bad section index")?;
                    if ts.flags & SHF_ALLOC == 0 || ts.flags & (SHF_WRITE | SHF_EXECINSTR) != 0 {
                        why = Some(format!("reference to section {}", ts.name));
                        break;
                    }
                    if elf.sections.iter().any(|r| r.ty == SHT_RELA && r.info as usize == t) {
                        why = Some(format!("data with relocations ({})", ts.name));
                        break;
                    }
                    let base = match ro_ix.get(&t) {
                        Some(&b) => b,
                        None => {
                            let a = ts.align.max(1) as usize;
                            while rodata.len() % a != 0 {
                                rodata.push(0);
                            }
                            let b = rodata.len();
                            rodata.extend_from_slice(&elf.data[ts.off..ts.off + ts.size]);
                            ro_ix.insert(t, b);
                            b
                        }
                    };
                    (K_RODATA, base as u64 + s.value)
                };
                rs.push(Reloc { off: r_off, ty, kind, addend, arg });
            }
            if why.is_some() {
                break;
            }
        }
        let name = if idx == nops { "<exit>".to_string() } else { fname.trim_start_matches("__jit.op.").to_string() };
        if let Some(w) = why {
            rejected.push((name, w));
            continue;
        }
        // The stencil ends in `jmp __jit.n.D`: the copy can drop it when
        // the code for pc + D follows.
        let n = bytes.len();
        let trim = rs
            .iter()
            .find(|r| r.kind == K_NEXT && r.off as usize + 4 == n && n >= 5 && bytes[n - 5] == 0xe9)
            .map(|r| r.arg as i64)
            .unwrap_or(-1);
        rs.sort_by_key(|r| r.off);
        stencils.insert(idx, (code.len(), n, relocs.len(), rs.len(), trim));
        code.extend_from_slice(bytes);
        relocs.extend(rs);
    }
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
    let mut st = Vec::new();
    for i in 0..=nops {
        let (o, l, r, nr, t, present) = match stencils.get(&i) {
            Some(&(o, l, r, nr, t)) => (o, l, r, nr, t, 1),
            None => (0, 0, 0, 0, -1, 0),
        };
        st.push(format!("{{ i32, i32, i32, i32, i32, i32 }} {{ i32 {}, i32 {}, i32 {}, i32 {}, i32 {}, i32 {} }}", o, l, r, nr, t, present));
    }
    let _ = writeln!(ir, "@loimi_jit_stencils = constant [{} x {{ i32, i32, i32, i32, i32, i32 }}] [{}]", st.len(), st.join(", "));
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
    Ok(Tables { ir, rejected, count: stencils.len(), bytes: code.len() })
}
