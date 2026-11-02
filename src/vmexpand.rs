//! vm expansion, source level: generates ordinary Loimi functions for each
//! `vm` declaration (encoders, emitters, opcode metadata, the superinstruction
//! peephole pass). They are parsed and added to the vm's module, and are
//! reachable as `Vm.NAME(...)`, `Vm.emit.NAME(...)`, `Vm.op_size(w)`, ...

use std::fmt::Write;

use crate::ast::*;
use crate::diag::{Diag, Res, SourceMap};
use crate::vmast::*;

pub struct OpLayout {
    pub name: String,
    /// Operand kind name per field.
    pub kinds: Vec<String>,
    pub opcode: u32,
    /// (param name, field bits, field shift, signed)
    pub fields: Vec<(String, u32, u32, bool)>,
    pub cache_words: u32,
    /// For superinstructions: component op indices.
    pub parts: Vec<usize>,
    /// Index of the family's generic op (the op itself outside families;
    /// the first part's family for superinstructions).
    pub family: usize,
    /// `then OP`: the op that must follow this one.
    pub then: Option<usize>,
}

pub struct VmLayout {
    pub word_bits: u32,
    pub op_shift: u32,
    pub op_bits: u32,
    pub ops: Vec<OpLayout>,
}

fn field_ty(bits: u32, signed: bool) -> String {
    let w = if bits <= 8 {
        8
    } else if bits <= 16 {
        16
    } else if bits <= 32 {
        32
    } else {
        64
    };
    format!("{}{}", if signed { "i" } else { "u" }, w)
}

pub fn layout(v: &VmDecl) -> Res<VmLayout> {
    let word = v.word.as_ref().ok_or_else(|| Diag::new(v.name.span, "a vm needs an `encoding fixed(T)` declaration"))?;
    let word_bits = match &word.kind {
        TypeKind::Named(p, _) if p.len() == 1 => match p[0].name.as_str() {
            "u8" => 8,
            "u16" => 16,
            "u32" => 32,
            "u64" => 64,
            _ => return Err(Diag::new(word.span, "the code word type must be u8, u16, u32 or u64")),
        },
        _ => return Err(Diag::new(word.span, "the code word type must be u8, u16, u32 or u64")),
    };
    let mut op_field: Option<(u32, u32)> = None;
    let mut formats = Vec::new();
    for f in &v.formats {
        let mut shift = 0;
        let mut fields = Vec::new();
        for fld in &f.fields {
            if fld.bits == 0 || fld.bits > 64 {
                return Err(Diag::new(fld.name.span, "bad field width"));
            }
            fields.push((fld.name.name.clone(), fld.bits, shift, fld.signed));
            shift += fld.bits;
        }
        if shift > word_bits {
            return Err(Diag::new(f.name.span, format!("format `{}` needs {} bits but the word has {}", f.name.name, shift, word_bits)));
        }
        let Some(of) = fields.iter().find(|x| x.0 == "op") else {
            return Err(Diag::new(f.name.span, "every format needs an `op` field"));
        };
        let this = (of.2, of.1);
        if let Some(prev) = op_field {
            if prev != this {
                return Err(Diag::new(f.name.span, "the `op` field must have the same position in every format"));
            }
        }
        op_field = Some(this);
        formats.push((f.name.name.clone(), fields));
    }
    let Some((op_shift, op_bits)) = op_field else {
        return Err(Diag::new(v.name.span, "a vm needs at least one format"));
    };
    let mut ops = Vec::new();
    for (i, op) in v.ops.iter().enumerate() {
        let Some((_, ffields)) = formats.iter().find(|(n, _)| *n == op.format.name) else {
            return Err(Diag::new(op.format.span, format!("unknown format `{}`", op.format.name)));
        };
        let non_op: Vec<&(String, u32, u32, bool)> = ffields.iter().filter(|f| f.0 != "op").collect();
        if op.params.len() > non_op.len() {
            return Err(Diag::new(op.name.span, format!("format `{}` has only {} operand field(s)", op.format.name, non_op.len())));
        }
        let fields = op.params.iter().zip(non_op.iter()).map(|(p, f)| (p.name.name.clone(), f.1, f.2, f.3)).collect();
        let cache_words = match &op.cache {
            Some(c) => v
                .caches
                .iter()
                .find(|x| x.name.name == c.name)
                .map(|x| x.fields.len() as u32)
                .ok_or_else(|| Diag::new(c.span, format!("unknown cache `{}`", c.name)))?,
            None => 0,
        };
        let kinds = op.params.iter().map(|p| p.kind.name.clone()).collect();
        let family = match &op.family {
            Some(f) => v.ops.iter().position(|o| o.name.name == *f).unwrap_or(i),
            None => i,
        };
        ops.push(OpLayout { name: op.name.name.clone(), kinds, opcode: i as u32, fields, cache_words, parts: vec![], family, then: None });
    }
    for (i, op) in v.ops.iter().enumerate() {
        if let Some(t) = &op.then {
            let Some(j) = v.ops.iter().position(|o| o.name.name == t.name) else {
                return Err(Diag::new(t.span, format!("unknown op `{}`", t.name)));
            };
            ops[i].then = Some(j);
        }
    }
    for s in &v.supers {
        let mut parts = Vec::new();
        for p in &s.parts {
            let idx = ops
                .iter()
                .position(|o| o.name == p.name && o.parts.is_empty())
                .ok_or_else(|| Diag::new(p.span, format!("unknown op `{}`", p.name)))?;
            parts.push(idx);
        }
        if parts.len() < 2 {
            return Err(Diag::new(s.name.span, "a superinstruction needs at least two parts"));
        }
        if let Some(o) = ops.iter().find(|o| o.parts == parts) {
            return Err(Diag::new(s.name.span, format!("superinstruction `{}` has the same parts as `{}`", s.name.name, o.name)));
        }
        let opcode = ops.len() as u32;
        let family = ops[parts[0]].family;
        ops.push(OpLayout { name: s.name.name.clone(), kinds: vec![], opcode, fields: vec![], cache_words: 0, parts, family, then: None });
    }
    if ops.len() as u64 > (1u64 << op_bits) {
        return Err(Diag::new(v.name.span, format!("{} ops do not fit in a {}-bit opcode field", ops.len(), op_bits)));
    }
    Ok(VmLayout { word_bits, op_shift, op_bits, ops })
}

impl VmLayout {
    /// Words occupied by an op (instruction + inline cache; sum for supers).
    pub fn size(&self, i: usize) -> u32 {
        let o = &self.ops[i];
        if o.parts.is_empty() { 1 + o.cache_words } else { o.parts.iter().map(|&p| self.size(p)).sum() }
    }
    /// The op a word stands for on its own: a superinstruction's first part.
    pub fn base(&self, i: usize) -> usize {
        self.ops[i].parts.first().copied().unwrap_or(i)
    }
    /// (word offset, op) of each part of op `i` (the op itself if not a super).
    pub fn part_offsets(&self, i: usize) -> Vec<(u32, usize)> {
        let o = &self.ops[i];
        if o.parts.is_empty() {
            return vec![(0, i)];
        }
        let mut off = 0;
        o.parts
            .iter()
            .map(|&p| {
                let r = (off, p);
                off += self.size(p);
                r
            })
            .collect()
    }
}

/// Source expression decoding field (bits, shift, signed) of word `__w`.
fn decode_src(word_bits: u32, bits: u32, shift: u32, signed: bool) -> String {
    // As in handlers: operand fields are 64-bit.
    let rt = if signed { "i64" } else { "u64" };
    if signed {
        format!(
            "(((__w << {}) as i{wb}) >> {}) as {}",
            word_bits - shift - bits,
            word_bits - bits,
            rt,
            wb = word_bits
        )
    } else {
        let m = if bits >= 64 { u64::MAX } else { (1u64 << bits) - 1 };
        format!("((__w >> {}) & {}) as {}", shift, m, rt)
    }
}

/// Ops whose handler may continue past the instruction's last word: it
/// uses `next`, or reads `pc` (to save a return address such as `pc + 1`),
/// directly, in a vm function it calls, or in an op it can `become`.
/// Code whose last instruction is one of these could run off the end.
/// The scan is over tokens, so it errs on the side of "may continue".
fn may_continue(sm: &SourceMap, v: &VmDecl, l: &VmLayout) -> Vec<bool> {
    use crate::lexer::Tok;
    struct Facts {
        direct: bool,
        calls: Vec<String>,
        becomes: Vec<String>,
    }
    let scan = |b: &Block| -> Facts {
        let text = &sm.files[b.span.file as usize].text[b.span.lo as usize..b.span.hi as usize];
        let Ok(toks) = crate::lexer::lex(b.span.file, text) else {
            return Facts { direct: true, calls: vec![], becomes: vec![] };
        };
        let mut f = Facts { direct: false, calls: vec![], becomes: vec![] };
        for (i, t) in toks.iter().enumerate() {
            let Tok::Ident(name) = &t.tok else { continue };
            if i > 0 && toks[i - 1].tok == Tok::Punct(".") {
                continue;
            }
            match (name.as_str(), toks.get(i + 1).map(|t| &t.tok)) {
                ("next" | "pc", _) => f.direct = true,
                ("become", Some(Tok::Ident(target))) => f.becomes.push(target.clone()),
                (_, Some(Tok::Punct("("))) => f.calls.push(name.clone()),
                _ => {}
            }
        }
        f
    };
    let fns: Vec<(String, Facts)> =
        v.fns.iter().filter_map(|f| Some((f.name.name.clone(), scan(f.body.as_ref()?)))).collect();
    let ops: Vec<Facts> = v.ops.iter().map(|o| scan(&o.body)).collect();
    let calls_cont = |calls: &[String], fn_cont: &[bool]| {
        calls.iter().any(|c| fns.iter().position(|(n, _)| n == c).is_some_and(|k| fn_cont[k]))
    };
    let mut fn_cont: Vec<bool> = fns.iter().map(|(_, f)| f.direct).collect();
    let mut cont = vec![false; l.ops.len()];
    loop {
        let mut changed = false;
        for k in 0..fns.len() {
            if !fn_cont[k] && calls_cont(&fns[k].1.calls, &fn_cont) {
                fn_cont[k] = true;
                changed = true;
            }
        }
        for (i, f) in ops.iter().enumerate() {
            let becomes_cont = f
                .becomes
                .iter()
                .any(|b| l.ops.iter().position(|o| o.name == *b && o.parts.is_empty()).is_some_and(|t| cont[t]));
            if !cont[i] && (f.direct || calls_cont(&f.calls, &fn_cont) || becomes_cont) {
                cont[i] = true;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    for (i, o) in l.ops.iter().enumerate() {
        if let Some(&last) = o.parts.last() {
            cont[i] = cont[last];
        }
    }
    cont
}

/// The verifier. It checks that
/// - the code is not empty, and every instruction holds a valid opcode and
///   fits (with its inline cache);
/// - instructions are walked one component at a time: the words after a
///   superinstruction's head are instructions too (jumps may land on them,
///   and `redo`, `next` from a shared tail or a quickened part runs them
///   alone), so each must hold an op of the same family as the super's part;
/// - `then OP`: the next instruction is an OP;
/// - every operand satisfies its kind's `verify` clause (a super's head
///   word carries its first part's operands; the other parts are checked
///   at their own words);
/// - the code cannot run off its end: the last instruction is not one that
///   may continue (see `may_continue`), and an op with `then` that may
///   continue (say by `jump pc + 2` past its follower) is not followed by
///   the last instruction.
/// Returns the index of the first bad word.
///
/// `verify` clauses are pasted into the generated function, so they see
/// `it`, `pc` and the verify context; the verifier's own names start with
/// `__` so that a clause cannot pick them up by accident.
fn generate_verifier(sm: &SourceMap, v: &VmDecl, l: &VmLayout) -> Res<String> {
    let n = &v.name.name;
    let w = format!("u{}", l.word_bits);
    let vis = if v.is_pub { "pub " } else { "" };
    for p in &v.verify_ctx {
        if matches!(p.name.name.as_str(), "it" | "pc") || p.name.name.starts_with("__") {
            return Err(Diag::new(p.name.span, format!("`{}` is reserved in verify clauses; rename this verify_context", p.name.name)));
        }
    }
    let clause = |kind: &str| -> Option<String> {
        let k = v.operands.iter().find(|o| o.name.name == kind)?;
        let e = k.verify.as_ref()?;
        let f = &sm.files[e.span.file as usize];
        let text = &f.text[e.span.lo as usize..e.span.hi as usize];
        Some(text.replace("is_insn_boundary(", &format!("{n}__boundary(__code, &__bits, ")))
    };
    let mut s = String::new();
    let _ = writeln!(
        s,
        "fn {n}__boundary(code: []{w}, bits: *[u8; 65536], p: *{w}) -> bool {{\n\
         \x20 if p < code.ptr or p >= code.ptr + code.len {{ return false; }}\n\
         \x20 let i = ((p as u64) - (code.ptr as u64)) / {};\n\
         \x20 (bits[i >> 3] >> ((i & 7) as u8)) & 1 == 1\n}}",
        l.word_bits / 8
    );
    let cont = may_continue(sm, v, l);
    let continuing: Vec<String> =
        l.ops.iter().filter(|o| cont[o.opcode as usize]).map(|o| format!("op == {}", o.opcode)).collect();
    let _ = writeln!(
        s,
        "fn {n}__continues(op: u64) -> bool {{ {} }}",
        if continuing.is_empty() { "false".to_string() } else { continuing.join(" or ") }
    );
    let mut params = vec![format!("__code: []{w}")];
    for p in &v.verify_ctx {
        let f = &sm.files[p.ty.span.file as usize];
        params.push(format!("{}: {}", p.name.name, &f.text[p.ty.span.lo as usize..p.ty.span.hi as usize]));
    }
    let _ = writeln!(s, "{vis}fn {n}__verify({}) -> ?u64 {{", params.join(", "));
    let _ = writeln!(s, "  if __code.len == 0 or __code.len > 524288 {{ return Some(0); }}");
    let _ = writeln!(s, "  var __bits: [u8; 65536] = [0; 65536];");
    let _ = writeln!(s, "  var __i: u64 = 0;");
    let _ = writeln!(s, "  var __last: u64 = 0;");
    let _ = writeln!(s, "  while __i < __code.len {{");
    let _ = writeln!(s, "    if {n}__opcode(__code[__i]) as u64 >= {n}__NUM_OPS {{ return Some(__i); }}");
    let _ = writeln!(s, "    __bits[__i >> 3] = __bits[__i >> 3] | ((1 as u8) << ((__i & 7) as u8));");
    let _ = writeln!(s, "    if __i + {n}__op_size(__code[__i]) > __code.len {{ return Some(__i); }}");
    let _ = writeln!(s, "    match {n}__opcode(__code[__i]) {{");
    for (k, o) in l.ops.iter().enumerate() {
        let mut conds: Vec<String> = l.part_offsets(k)[1..]
            .iter()
            .map(|(off, p)| format!("{n}__family(__code[__i + {off}]) != {}", l.ops[*p].family))
            .collect();
        // `then OP`: the next instruction must be an OP; if this op may
        // continue, something must come after that OP too.
        let b = l.base(k);
        if let Some(t) = l.ops[b].then {
            let sz = l.size(b);
            conds.push(format!("__i + {sz} >= __code.len or {n}__family(__code[__i + {sz}]) != {}", l.ops[t].family));
            if cont[b] {
                conds.push(format!("__i + {sz} + {n}__base_size(__code[__i + {sz}]) >= __code.len"));
            }
        }
        if !conds.is_empty() {
            let _ = writeln!(s, "      {} => {{ if {} {{ return Some(__i); }} }}", o.opcode, conds.join(" or "));
        }
    }
    let _ = writeln!(s, "      _ => {{}}\n    }}");
    let _ = writeln!(s, "    __last = __i;\n    __i += {n}__base_size(__code[__i]);\n  }}");
    // The walk is per component, so the last word group is never a
    // super's head.
    let _ = writeln!(s, "  if {n}__continues({n}__opcode(__code[__last]) as u64) {{ return Some(__last); }}");
    let _ = writeln!(s, "  __i = 0;\n  while __i < __code.len {{");
    let _ = writeln!(s, "    match {n}__opcode(__code[__i]) {{");
    for (k, o) in l.ops.iter().enumerate() {
        let mut checks = String::new();
        for (off, p) in l.part_offsets(k).into_iter().take(1) {
            let po = &l.ops[p];
            for (fi, (_, bits, shift, signed)) in po.fields.iter().enumerate() {
                if let Some(c) = clause(&po.kinds[fi]) {
                    let _ = write!(
                        checks,
                        "{{ let pc: *{w} = __code.ptr + __i + {off}; let __w = __code[__i + {off}]; let it = {}; if !({}) {{ return Some(__i + {off}); }} }} ",
                        decode_src(l.word_bits, *bits, *shift, *signed),
                        c
                    );
                }
            }
        }
        if !checks.is_empty() {
            let _ = writeln!(s, "      {} => {{ {} }}", o.opcode, checks);
        }
    }
    let _ = writeln!(s, "      _ => {{}}\n    }}");
    let _ = writeln!(s, "    __i += {n}__base_size(__code[__i]);\n  }}\n  None\n}}");
    Ok(s)
}

pub fn generate(v: &VmDecl, l: &VmLayout) -> String {
    let n = &v.name.name;
    let w = format!("u{}", l.word_bits);
    let mask = (1u64 << l.op_bits) - 1;
    let mut s = String::new();
    let vis = if v.is_pub { "pub " } else { "" };
    let _ = writeln!(s, "{vis}fn {n}__opcode(w: {w}) -> {w} {{ (w >> {}) & {} }}", l.op_shift, mask);
    let _ = writeln!(
        s,
        "{vis}fn {n}__with_opcode(w: {w}, op: {w}) -> {w} {{ (w & !({mask} << {sh})) | ((op & {mask}) << {sh}) }}",
        sh = l.op_shift
    );
    let _ = writeln!(s, "{vis}const {n}__NUM_OPS: u64 = {};", l.ops.len());
    for (i, o) in l.ops.iter().enumerate() {
        let _ = writeln!(s, "{vis}const {n}__OP_{}: {w} = {};", o.name, o.opcode);
        if !o.parts.is_empty() {
            continue;
        }
        let params: Vec<String> = o.fields.iter().map(|(p, b, _, sg)| format!("{}: {}", p, field_ty(*b, *sg))).collect();
        let mut enc = format!("(({} as {w}) << {})", o.opcode, l.op_shift);
        for (p, b, sh, _) in &o.fields {
            let m = if *b >= 64 { u64::MAX } else { (1u64 << b) - 1 };
            let _ = write!(enc, " | ((({} as {w}) & {}) << {})", p, m, sh);
        }
        let _ = writeln!(s, "{vis}fn {n}__{}({}) -> {w} {{ {} }}", o.name, params.join(", "), enc);
        let args: Vec<String> = o.fields.iter().map(|f| f.0.clone()).collect();
        let size = l.size(i);
        let mut body = format!("code[at] = {n}__{}({});", o.name, args.join(", "));
        for k in 1..size {
            let _ = write!(body, " code[at + {}] = 0;", k);
        }
        let mut ps = vec![format!("code: []{w}"), "at: u64".to_string()];
        ps.extend(params);
        let _ = writeln!(s, "{vis}fn {n}__emit_{}({}) -> u64 {{ {} at + {} }}", o.name, ps.join(", "), body, size);
    }
    // Sizes and names.
    let _ = writeln!(s, "{vis}fn {n}__op_size(w: {w}) -> u64 {{ match {n}__opcode(w) {{");
    for (i, o) in l.ops.iter().enumerate() {
        let _ = writeln!(s, "  {} => {},", o.opcode, l.size(i));
    }
    let _ = writeln!(s, "  _ => 1, }} }}");
    // Per word on its own: a superinstruction's first part.
    let _ = writeln!(s, "{vis}fn {n}__base_size(w: {w}) -> u64 {{ match {n}__opcode(w) {{");
    for (i, o) in l.ops.iter().enumerate() {
        let _ = writeln!(s, "  {} => {},", o.opcode, l.size(l.base(i)));
    }
    let _ = writeln!(s, "  _ => 1, }} }}");
    let _ = writeln!(s, "{vis}fn {n}__base_op(w: {w}) -> {w} {{ match {n}__opcode(w) {{");
    for (i, o) in l.ops.iter().enumerate() {
        let _ = writeln!(s, "  {} => {},", o.opcode, l.ops[l.base(i)].opcode);
    }
    let _ = writeln!(s, "  x => x, }} }}");
    let _ = writeln!(s, "fn {n}__family(w: {w}) -> u64 {{ match {n}__opcode(w) {{");
    for o in &l.ops {
        let _ = writeln!(s, "  {} => {},", o.opcode, o.family);
    }
    let _ = writeln!(s, "  _ => {}, }} }}", u64::MAX);
    let _ = writeln!(s, "{vis}fn {n}__op_name(w: {w}) -> str {{ match {n}__opcode(w) {{");
    for o in &l.ops {
        let _ = writeln!(s, "  {} => \"{}\",", o.opcode, o.name);
    }
    let _ = writeln!(s, "  _ => \"<invalid>\", }} }}");
    // Superinstruction peephole: rewrite the first word of each matching
    // sequence. The other words keep their opcodes, so jumps into the
    // middle of a fused sequence still execute the original ops. Words
    // are compared by the op they stand for alone, so code can be
    // optimized again (after quickening, say); the longest match wins.
    let _ = writeln!(s, "{vis}fn {n}__optimize(code: []{w}) {{");
    let _ = writeln!(s, "  var i: u64 = 0;\n  while i < code.len {{");
    let mut supers: Vec<usize> = (0..l.ops.len()).filter(|&k| !l.ops[k].parts.is_empty()).collect();
    supers.sort_by_key(|&k| std::cmp::Reverse(l.ops[k].parts.len()));
    let mut first = true;
    for k in supers {
        let o = &l.ops[k];
        let mut conds = vec![format!("i + {} <= code.len", l.size(k))];
        for (off, p) in l.part_offsets(k) {
            conds.push(format!("{n}__base_op(code[i + {off}]) == {}", l.ops[p].opcode));
        }
        let _ = writeln!(
            s,
            "    {}if {} {{ code[i] = {n}__with_opcode(code[i], {}); }}",
            if first { "" } else { "else " },
            conds.join(" and "),
            o.opcode
        );
        first = false;
    }
    let _ = writeln!(s, "    i += {n}__base_size(code[i]);\n  }}\n}}");
    s
}

/// Expand every vm in every module: append generated items.
pub fn expand(sm: &mut SourceMap, modules: &mut [Module]) -> Res<()> {
    for m in modules.iter_mut() {
        let mut extra = Vec::new();
        for it in &m.items {
            if let Item::Vm(v) = it {
                let l = layout(v)?;
                let mut src = generate(v, &l);
                src.push_str(&generate_verifier(sm, v, &l)?);
                if std::env::var_os("LOIMI_DUMP_VM_EXPANSION").is_some() {
                    eprintln!("// <vm {} expansion>\n{}", v.name.name, src);
                }
                let fid = sm.add(format!("<vm {} expansion>", v.name.name), src.clone());
                let toks = crate::lexer::lex(fid, &src)?;
                let mut p = crate::parser::Parser::new(toks);
                let (items, _) = p.parse_module()?;
                extra.extend(items);
            }
        }
        m.items.extend(extra);
    }
    Ok(())
}
