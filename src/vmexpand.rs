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
    pub opcode: u32,
    /// (param name, field bits, field shift, signed)
    pub fields: Vec<(String, u32, u32, bool)>,
    pub cache_words: u32,
    /// For superinstructions: component op indices.
    pub parts: Vec<usize>,
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
        ops.push(OpLayout { name: op.name.name.clone(), opcode: i as u32, fields, cache_words, parts: vec![] });
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
        let opcode = ops.len() as u32;
        ops.push(OpLayout { name: s.name.name.clone(), opcode, fields: vec![], cache_words: 0, parts });
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
    let _ = writeln!(s, "{vis}fn {n}__op_name(w: {w}) -> str {{ match {n}__opcode(w) {{");
    for o in &l.ops {
        let _ = writeln!(s, "  {} => \"{}\",", o.opcode, o.name);
    }
    let _ = writeln!(s, "  _ => \"<invalid>\", }} }}");
    // Superinstruction peephole: rewrite the first word of each matching
    // sequence. The other words keep their opcodes, so jumps into the
    // middle of a fused sequence still execute the original ops.
    let _ = writeln!(s, "{vis}fn {n}__optimize(code: []{w}) {{");
    let _ = writeln!(s, "  var i: u64 = 0;\n  while i < code.len {{\n    let n = {n}__op_size(code[i]);");
    for o in l.ops.iter().filter(|o| !o.parts.is_empty()) {
        let mut conds = Vec::new();
        let mut off = String::from("i");
        for (k, &p) in o.parts.iter().enumerate() {
            if k > 0 {
                conds.push(format!("{off} < code.len"));
            }
            conds.push(format!("{n}__opcode(code[{off}]) == {}", l.ops[p].opcode));
            off = format!("{off} + {}", l.size(p));
        }
        let _ = writeln!(
            s,
            "    if {} {{ code[i] = {n}__with_opcode(code[i], {}); }}",
            conds.join(" and "),
            o.opcode
        );
    }
    let _ = writeln!(s, "    i += n;\n  }}\n}}");
    s
}

/// Expand every vm in every module: append generated items.
pub fn expand(sm: &mut SourceMap, modules: &mut [Module]) -> Res<()> {
    for m in modules.iter_mut() {
        let mut extra = Vec::new();
        for it in &m.items {
            if let Item::Vm(v) = it {
                let l = layout(v)?;
                let src = generate(v, &l);
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
