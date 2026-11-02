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
    /// Index of the op's inline cache layout (`op X(..) : F + Cache`).
    pub cache: Option<usize>,
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
    /// Code words per cache layout. A field takes as many words as its
    /// type needs (a u64 or pointer takes two u32 words), which only the
    /// checker knows: until it fills these in, each field counts as one
    /// word, and the generated source uses `size_src` instead.
    pub cache_words: Vec<u32>,
    pub cache_names: Vec<String>,
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
        if let Some(p) = op.params.iter().find(|p| p.name.name.starts_with("__")) {
            return Err(Diag::new(p.name.span, "names starting with `__` are reserved for generated code"));
        }
        let non_op: Vec<&(String, u32, u32, bool)> = ffields.iter().filter(|f| f.0 != "op").collect();
        if op.params.len() > non_op.len() {
            return Err(Diag::new(op.name.span, format!("format `{}` has only {} operand field(s)", op.format.name, non_op.len())));
        }
        let fields = op.params.iter().zip(non_op.iter()).map(|(p, f)| (p.name.name.clone(), f.1, f.2, f.3)).collect();
        let cache = match &op.cache {
            Some(c) => Some(
                v.caches
                    .iter()
                    .position(|x| x.name.name == c.name)
                    .ok_or_else(|| Diag::new(c.span, format!("unknown cache `{}`", c.name)))?,
            ),
            None => None,
        };
        let kinds = op.params.iter().map(|p| p.kind.name.clone()).collect();
        let family = match &op.family {
            Some(f) => v.ops.iter().position(|o| o.name.name == *f).unwrap_or(i),
            None => i,
        };
        ops.push(OpLayout { name: op.name.name.clone(), kinds, opcode: i as u32, fields, cache, parts: vec![], family, then: None });
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
        ops.push(OpLayout { name: s.name.name.clone(), kinds: vec![], opcode, fields: vec![], cache: None, parts, family, then: None });
    }
    if ops.len() as u64 > (1u64 << op_bits) {
        return Err(Diag::new(v.name.span, format!("{} ops do not fit in a {}-bit opcode field", ops.len(), op_bits)));
    }
    let cache_words = v.caches.iter().map(|c| c.fields.len() as u32).collect();
    let cache_names = v.caches.iter().map(|c| c.name.name.clone()).collect();
    Ok(VmLayout { word_bits, op_shift, op_bits, ops, cache_words, cache_names })
}

impl VmLayout {
    /// Words occupied by an op (instruction + inline cache; sum for supers).
    pub fn size(&self, i: usize) -> u32 {
        let o = &self.ops[i];
        match (o.parts.is_empty(), o.cache) {
            (true, Some(c)) => 1 + self.cache_words[c],
            (true, None) => 1,
            (false, _) => o.parts.iter().map(|&p| self.size(p)).sum(),
        }
    }
    /// `size` as a constant expression of the generated source, where
    /// cache sizes are the constants `generate` defines.
    pub fn size_src(&self, vm: &str, i: usize) -> String {
        let o = &self.ops[i];
        match (o.parts.is_empty(), o.cache) {
            (true, Some(c)) => format!("(1 + {}__CW_{})", vm, self.cache_names[c]),
            (true, None) => "1".to_string(),
            (false, _) => format!("({})", o.parts.iter().map(|&p| self.size_src(vm, p)).collect::<Vec<_>>().join(" + ")),
        }
    }
    /// `part_offsets` with offsets as source expressions.
    pub fn part_offsets_src(&self, vm: &str, i: usize) -> Vec<(String, usize)> {
        let o = &self.ops[i];
        if o.parts.is_empty() {
            return vec![("0".to_string(), i)];
        }
        let mut off: Vec<String> = Vec::new();
        o.parts
            .iter()
            .map(|&p| {
                let r = (if off.is_empty() { "0".to_string() } else { format!("({})", off.join(" + ")) }, p);
                off.push(self.size_src(vm, p));
                r
            })
            .collect()
    }
    /// The op a word stands for on its own: a superinstruction's first part.
    pub fn base(&self, i: usize) -> usize {
        self.ops[i].parts.first().copied().unwrap_or(i)
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
///
/// The scan is over tokens, so it errs on the side of "may continue",
/// with one refinement for shared slow paths such as
/// `fn call_slow(.., tail: bool) -> never` called as `call_slow(.., true)`:
/// a vm function is scanned once per combination of literal `true`/`false`
/// arguments, and `if p { .. }` / `if !p { .. }` blocks that such a
/// parameter rules out are skipped (unless the function rebinds `p`).
fn may_continue(sm: &SourceMap, v: &VmDecl, l: &VmLayout) -> Vec<bool> {
    use crate::lexer::{Tok, Token};
    use std::collections::HashMap;

    type Bind = Vec<Option<bool>>;
    #[derive(Default)]
    struct Facts {
        direct: bool,
        calls: Vec<(String, Bind)>,
        becomes: Vec<String>,
    }
    fn tokens(sm: &SourceMap, b: &Block) -> Option<Vec<Token>> {
        let text = &sm.files[b.span.file as usize].text[b.span.lo as usize..b.span.hi as usize];
        crate::lexer::lex(b.span.file, text).ok()
    }
    /// Index after the block whose `{` is at `i`.
    fn skip_block(t: &[Token], i: usize) -> usize {
        let mut depth = 0;
        let mut j = i;
        while j < t.len() {
            match t[j].tok {
                Tok::Punct("{") => depth += 1,
                Tok::Punct("}") => {
                    depth -= 1;
                    if depth == 0 {
                        return j + 1;
                    }
                }
                _ => {}
            }
            j += 1;
        }
        t.len()
    }
    /// Literal bool arguments of the call whose `(` is at `i`.
    fn call_args(t: &[Token], i: usize) -> Bind {
        let mut args = Vec::new();
        let mut cur: Vec<&Tok> = Vec::new();
        let mut depth = 0;
        let mut j = i;
        while j < t.len() {
            let tok = &t[j].tok;
            match tok {
                Tok::Punct("(") | Tok::Punct("[") | Tok::Punct("{") => depth += 1,
                Tok::Punct(")") | Tok::Punct("]") | Tok::Punct("}") => depth -= 1,
                _ => {}
            }
            if depth == 0 || (depth == 1 && *tok == Tok::Punct(",")) {
                if !cur.is_empty() {
                    args.push(match cur.as_slice() {
                        [Tok::Ident(b)] if b == "true" => Some(true),
                        [Tok::Ident(b)] if b == "false" => Some(false),
                        _ => None,
                    });
                }
                cur.clear();
                if depth == 0 {
                    break;
                }
            } else if j > i {
                cur.push(tok);
            }
            j += 1;
        }
        args
    }
    // `params`: the names that `bind` gives values to.
    let scan = |t: &[Token], params: &[String], bind: &Bind| -> Facts {
        let mut known: HashMap<&str, bool> = HashMap::new();
        for (k, p) in params.iter().enumerate() {
            if let Some(Some(b)) = bind.get(k) {
                known.insert(p.as_str(), *b);
            }
        }
        // A parameter that is assigned or shadowed is not known.
        for (i, tk) in t.iter().enumerate() {
            if let Tok::Ident(name) = &tk.tok {
                let assigned = matches!(t.get(i + 1).map(|x| &x.tok), Some(Tok::Punct("=")));
                let shadowed = i > 0 && matches!(&t[i - 1].tok, Tok::Ident(k) if k == "let" || k == "var");
                if assigned || shadowed {
                    known.remove(name.as_str());
                }
            }
        }
        let mut f = Facts::default();
        let mut i = 0;
        while i < t.len() {
            let Tok::Ident(name) = &t[i].tok else {
                i += 1;
                continue;
            };
            if i > 0 && t[i - 1].tok == Tok::Punct(".") {
                i += 1;
                continue;
            }
            if name == "if" {
                // `if p {` / `if !p {` with p known: skip a block that
                // cannot run.
                let neg = t.get(i + 1).map(|x| &x.tok) == Some(&Tok::Punct("!"));
                let k = if neg { i + 2 } else { i + 1 };
                if let (Some(Tok::Ident(p)), Some(Tok::Punct("{"))) = (t.get(k).map(|x| &x.tok), t.get(k + 1).map(|x| &x.tok)) {
                    if let Some(&b) = known.get(p.as_str()) {
                        if b == neg {
                            i = skip_block(t, k + 1);
                            continue;
                        }
                    }
                }
            }
            match (name.as_str(), t.get(i + 1).map(|x| &x.tok)) {
                ("next" | "pc", _) => f.direct = true,
                ("become", Some(Tok::Ident(target))) => f.becomes.push(target.clone()),
                (_, Some(Tok::Punct("("))) => f.calls.push((name.clone(), call_args(t, i + 1))),
                _ => {}
            }
            i += 1;
        }
        f
    };
    let fn_toks: Vec<(String, Vec<String>, Option<Vec<Token>>)> = v
        .fns
        .iter()
        .map(|f| {
            let params = f.params.iter().map(|p| p.name.name.clone()).collect();
            (f.name.name.clone(), params, f.body.as_ref().and_then(|b| tokens(sm, b)))
        })
        .collect();
    // Nodes: ops (by index), then vm function instances (name, bindings).
    let mut facts: Vec<Facts> = v
        .ops
        .iter()
        .map(|o| match tokens(sm, &o.body) {
            Some(t) => scan(&t, &[], &vec![]),
            None => Facts { direct: true, ..Facts::default() },
        })
        .collect();
    let mut inst: HashMap<(String, Bind), usize> = HashMap::new();
    let mut k = 0;
    while k < facts.len() {
        let calls = facts[k].calls.clone();
        for (name, bind) in calls {
            let Some((_, params, toks)) = fn_toks.iter().find(|(n, _, _)| *n == name) else { continue };
            let key = (name.clone(), bind.clone());
            if inst.contains_key(&key) {
                continue;
            }
            inst.insert(key, facts.len());
            facts.push(match toks {
                Some(t) => scan(t, params, &bind),
                None => Facts { direct: true, ..Facts::default() },
            });
        }
        k += 1;
    }
    let mut cont: Vec<bool> = facts.iter().map(|f| f.direct).collect();
    loop {
        let mut changed = false;
        for i in 0..facts.len() {
            if cont[i] {
                continue;
            }
            let f = &facts[i];
            let c = f.calls.iter().any(|c| inst.get(c).is_some_and(|&j| cont[j]))
                || f.becomes.iter().any(|b| v.ops.iter().position(|o| o.name.name == *b).is_some_and(|t| cont[t]));
            if c {
                cont[i] = true;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    let mut out: Vec<bool> = cont[..v.ops.len()].to_vec();
    out.resize(l.ops.len(), false);
    for (i, o) in l.ops.iter().enumerate() {
        if let Some(&last) = o.parts.last() {
            out[i] = out[last];
        }
    }
    out
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
        let mut conds: Vec<String> = l.part_offsets_src(n, k)[1..]
            .iter()
            .map(|(off, p)| format!("{n}__family(__code[__i + {off}]) != {}", l.ops[*p].family))
            .collect();
        // `then OP`: the next instruction must be an OP; if this op may
        // continue, something must come after that OP too.
        let b = l.base(k);
        if let Some(t) = l.ops[b].then {
            let sz = l.size_src(n, b);
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
        for (off, p) in l.part_offsets_src(n, k).into_iter().take(1) {
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
    // Code words of each inline cache layout: every field starts a word
    // and takes as many as its type needs.
    let wb = l.word_bits / 8;
    for c in &v.caches {
        let words: Vec<String> = c.fields.iter().map(|(_, t, _)| format!("(size_of<{}>() + {})/{}", type_src(t), wb - 1, wb)).collect();
        let total = if words.is_empty() { "0".to_string() } else { words.join(" + ") };
        let _ = writeln!(s, "{vis}const {n}__CW_{}: u64 = {};", c.name.name, total);
    }
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
        let size = l.size_src(n, i);
        // The emitter's own parameters start with `__`, so that they
        // cannot clash with operand names (`op SET(at: reg, code: imm)`).
        let mut body = format!("__code[__at] = {n}__{}({});", o.name, args.join(", "));
        if o.cache.is_some() {
            let _ = write!(body, " var __k: u64 = 1; while __k < {size} {{ __code[__at + __k] = 0; __k += 1; }}");
        }
        let mut ps = vec![format!("__code: []{w}"), "__at: u64".to_string()];
        ps.extend(params);
        let _ = writeln!(s, "{vis}fn {n}__emit_{}({}) -> u64 {{ {} __at + {} }}", o.name, ps.join(", "), body, size);
    }
    // Sizes and names.
    let _ = writeln!(s, "{vis}fn {n}__op_size(w: {w}) -> u64 {{ match {n}__opcode(w) {{");
    for (i, o) in l.ops.iter().enumerate() {
        let _ = writeln!(s, "  {} => {},", o.opcode, l.size_src(n, i));
    }
    let _ = writeln!(s, "  _ => 1, }} }}");
    // Per word on its own: a superinstruction's first part.
    let _ = writeln!(s, "{vis}fn {n}__base_size(w: {w}) -> u64 {{ match {n}__opcode(w) {{");
    for (i, o) in l.ops.iter().enumerate() {
        let _ = writeln!(s, "  {} => {},", o.opcode, l.size_src(n, l.base(i)));
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
        let mut conds = vec![format!("i + {} <= code.len", l.size_src(n, k))];
        for (off, p) in l.part_offsets_src(n, k) {
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
    // Deoptimization: every instruction word back to its family's
    // generic op, splitting supers (the head becomes its first part's
    // generic; the parts' words are instructions of their own). Members of
    // a family share format and cache, so sizes do not change. Inline
    // caches are zeroed, as the emitters leave them.
    let _ = writeln!(s, "{vis}fn {n}__deopt(__code: []{w}) {{");
    let _ = writeln!(s, "  var __i: u64 = 0;\n  while __i < __code.len {{");
    let _ = writeln!(s, "    let __f = {n}__family(__code[__i]);");
    let _ = writeln!(s, "    if __f != {} {{ __code[__i] = {n}__with_opcode(__code[__i], __f as {w}); }}", u64::MAX);
    let _ = writeln!(s, "    let __size = {n}__base_size(__code[__i]);");
    let _ = writeln!(s, "    var __k: u64 = 1;");
    let _ = writeln!(s, "    while __k < __size and __i + __k < __code.len {{ __code[__i + __k] = 0; __k += 1; }}");
    let _ = writeln!(s, "    __i += __size;\n  }}\n}}");
    s
}

/// A type as source text, for the generated code (which is parsed in the
/// vm's module, so names resolve as in the declaration). Types the
/// checker rejects as cache fields print as `u64`: the checker reports
/// them first, with the declaration's span.
fn type_src(t: &TypeExpr) -> String {
    match &t.kind {
        TypeKind::Named(path, args) => {
            let p = path.iter().map(|i| i.name.as_str()).collect::<Vec<_>>().join(".");
            if args.is_empty() {
                p
            } else {
                format!("{}<{}>", p, args.iter().map(type_src).collect::<Vec<_>>().join(", "))
            }
        }
        TypeKind::Ptr(inner) => format!("*{}", type_src(inner)),
        TypeKind::Option(inner) => format!("?{}", type_src(inner)),
        TypeKind::Fn(..) => "*u8".to_string(),
        _ => "u64".to_string(),
    }
}

/// Is operand kind `kind` a jump label: `operand label = pc + it`?
fn is_label_kind(v: &VmDecl, kind: &str) -> bool {
    let Some(k) = v.operands.iter().find(|o| o.name.name == kind) else { return false };
    let is = |e: &Expr, n: &str| matches!(&e.kind, ExprKind::Ident(x) if x == n);
    match &k.body.kind {
        ExprKind::Binary(BinOp::Add, a, b) => (is(a, "pc") && is(b, "it")) || (is(a, "it") && is(b, "pc")),
        _ => false,
    }
}

/// Label and field helpers (separate from the other generated items):
///
/// - `Vm.set_label(w, off) -> ?W`: word `w` with the offset of its jump
///   label (the first operand whose kind is `pc + it`) set to `off`, or
///   None if `off` does not fit the field or the op has no label. A
///   superinstruction's head word uses its first part's operands.
/// - `Vm.label(w) -> ?i64`: the label offset of word `w`, if its op has one.
/// - `Vm.FMT_field(w)`: field `field` of format `FMT`, decoded like an
///   operand (u64, or i64 for a signed field).
///
/// `set_label` and `label` have the shapes `std.bc` takes as its patch
/// and jump-threading functions.
fn generate_labels(v: &VmDecl, l: &VmLayout) -> String {
    let n = &v.name.name;
    let w = format!("u{}", l.word_bits);
    let vis = if v.is_pub { "pub " } else { "" };
    let mut s = String::new();
    // (opcode, (bits, shift, signed)) of each op's label field.
    let mut labels: Vec<(u32, (u32, u32, bool))> = Vec::new();
    for o in &l.ops {
        let base = &l.ops[l.base(o.opcode as usize)];
        if let Some(fi) = base.kinds.iter().position(|k| is_label_kind(v, k)) {
            let (_, bits, shift, signed) = base.fields[fi];
            labels.push((o.opcode, (bits, shift, signed)));
        }
    }
    let _ = writeln!(s, "{vis}fn {n}__set_label(w: {w}, off: i64) -> ?{w} {{");
    let _ = writeln!(s, "  match {n}__opcode(w) {{");
    for (op, (bits, shift, signed)) in &labels {
        let mask = if *bits >= 64 { u64::MAX } else { (1u64 << bits) - 1 };
        let range = if *bits >= 64 && *signed {
            String::new()
        } else if *signed {
            let lim = 1i128 << (bits - 1);
            format!("if off < {} or off > {} {{ return None; }} ", -lim, lim - 1)
        } else if *bits >= 63 {
            "if off < 0 { return None; } ".to_string()
        } else {
            format!("if off < 0 or off > {} {{ return None; }} ", mask)
        };
        let _ = writeln!(
            s,
            "    {op} => {{ {range}Some((w & !(({mask} as {w}) << {shift})) | (((off as {w}) & {mask}) << {shift})) }}"
        );
    }
    let _ = writeln!(s, "    _ => None,");
    let _ = writeln!(s, "  }}");
    let _ = writeln!(s, "}}");
    let _ = writeln!(s, "{vis}fn {n}__label(__w: {w}) -> ?i64 {{");
    let _ = writeln!(s, "  match {n}__opcode(__w) {{");
    for (op, (bits, shift, signed)) in &labels {
        let _ = writeln!(s, "    {op} => Some({} as i64),", decode_src(l.word_bits, *bits, *shift, *signed));
    }
    let _ = writeln!(s, "    _ => None,");
    let _ = writeln!(s, "  }}");
    let _ = writeln!(s, "}}");
    for f in &v.formats {
        let mut shift = 0;
        for fld in &f.fields {
            if fld.name.name != "op" {
                let rt = if fld.signed { "i64" } else { "u64" };
                let _ = writeln!(
                    s,
                    "{vis}fn {n}__{}_{}(__w: {w}) -> {rt} {{ {} }}",
                    f.name.name,
                    fld.name.name,
                    decode_src(l.word_bits, fld.bits, shift, fld.signed)
                );
            }
            shift += fld.bits;
        }
    }
    s
}

/// A string as a Loimi string literal.
fn str_lit(s: &str) -> String {
    let mut bytes = Vec::new();
    for b in s.bytes() {
        match b {
            b'"' => bytes.extend_from_slice(b"\\\""),
            b'\\' => bytes.extend_from_slice(b"\\\\"),
            0x20..=0x7e | 0x80..=0xff => bytes.push(b),
            _ => bytes.extend_from_slice(format!("\\x{:02x}", b).as_bytes()),
        }
    }
    format!("\"{}\"", String::from_utf8_lossy(&bytes))
}

/// The disassembler:
///
/// - `Vm.disasm_insn(code, i, out) -> u64` prints the instruction at word
///   `i` as one line (a superinstruction as a line with its name and one
///   indented line per part) and returns its size in words, or 0 if `i`
///   is past the end;
/// - `Vm.disasm(code, out)` prints all instructions.
///
/// `out` is a pointer to any type with `push_str(str)` and `push_int(i64)`
/// methods (and `push_f64(f64)` if a cache has a float field), such as
/// `text.StrBuf`; the functions are generic over it, so a vm module need
/// not import std.text.
///
/// A line is the word offset, the op name and the operands as
/// `name=value`, decoded as handlers see them (signed fields signed).
/// Label operands (`pc + it`) print the absolute target, `name -> 42`. An
/// operand kind's `display "r{}"` formats the value (or target). Inline
/// cache words print as `[cache field=value ...]`: integers in decimal,
/// `bool`s, floats, and pointers in hex (`null` for 0). Invalid opcodes
/// print as `<invalid 0x..>` (size 1), and an instruction that does not
/// fit in the code as its name and `<truncated>`.
fn generate_disasm(v: &VmDecl, l: &VmLayout) -> String {
    let n = &v.name.name;
    let w = format!("u{}", l.word_bits);
    let wb = l.word_bits / 8;
    let vis = if v.is_pub { "pub " } else { "" };
    let mut s = String::new();
    // Helpers: unsigned decimal, hex, padding, multi-word reads.
    let _ = writeln!(
        s,
        "{vis}fn {n}__disasm_u64<B>(__out: *B, __x: u64) {{\n\
         \x20 if __x > 9223372036854775807 {{ __out.push_int((__x / 10) as i64); __out.push_int((__x % 10) as i64); }}\n\
         \x20 else {{ __out.push_int(__x as i64); }}\n}}"
    );
    let _ = writeln!(
        s,
        "{vis}fn {n}__disasm_hex<B>(__out: *B, __x: u64) {{\n\
         \x20 let __digits = \"0123456789abcdef\";\n\
         \x20 __out.push_str(\"0x\");\n\
         \x20 var __sh: u64 = 64;\n\
         \x20 var __started = false;\n\
         \x20 while __sh > 0 {{\n\
         \x20   __sh -= 4;\n\
         \x20   let __d = (__x >> __sh) & 15;\n\
         \x20   if __d != 0 or __started or __sh == 0 {{ __started = true; __out.push_str(__digits[__d..__d + 1]); }}\n\
         \x20 }}\n}}"
    );
    let _ = writeln!(
        s,
        "{vis}fn {n}__disasm_offset<B>(__out: *B, __x: u64) {{\n\
         \x20 var __width: u64 = 1;\n\
         \x20 var __y = __x;\n\
         \x20 while __y >= 10 {{ __y = __y / 10; __width += 1; }}\n\
         \x20 while __width < 5 {{ __out.push_str(\" \"); __width += 1; }}\n\
         \x20 {n}__disasm_u64(__out, __x);\n}}"
    );
    let _ = writeln!(
        s,
        "{vis}fn {n}__disasm_word(__code: []{w}, __j: u64, __n: u64) -> u64 {{\n\
         \x20 var __r: u64 = 0;\n\
         \x20 var __k: u64 = 0;\n\
         \x20 while __k < __n and __k * {} < 64 {{ __r = __r | ((__code[__j + __k] as u64) << (__k * {})); __k += 1; }}\n\
         \x20 __r\n}}",
        l.word_bits,
        l.word_bits
    );
    // One component op, at word `__at`: name, operands, cache.
    let _ = writeln!(s, "{vis}fn {n}__disasm_op<B>(__code: []{w}, __at: u64, __out: *B) {{");
    let _ = writeln!(s, "  let __w = __code[__at];");
    let _ = writeln!(
        s,
        "  if {n}__opcode(__w) as u64 >= {n}__NUM_OPS {{ __out.push_str(\"<invalid \"); {n}__disasm_hex(__out, __w as u64); __out.push_str(\">\"); return; }}"
    );
    let _ = writeln!(s, "  __out.push_str({n}__op_name({n}__base_op(__w)));");
    let _ = writeln!(
        s,
        "  if __at + {n}__base_size(__w) > __code.len {{ __out.push_str(\" <truncated>\"); return; }}"
    );
    let _ = writeln!(s, "  match {n}__base_op(__w) {{");
    for o in &l.ops {
        if !o.parts.is_empty() {
            continue;
        }
        let mut body = String::new();
        for (fi, (pname, bits, shift, signed)) in o.fields.iter().enumerate() {
            let kind = &o.kinds[fi];
            let decl = v.operands.iter().find(|k| k.name.name == *kind);
            let (pre, post) = match decl.and_then(|k| k.display.as_ref()) {
                Some(d) => {
                    let (a, b) = d.split_once("{}").unwrap();
                    (a.to_string(), b.to_string())
                }
                None => (String::new(), String::new()),
            };
            let label = is_label_kind(v, kind);
            let _ = write!(body, "__out.push_str({}); ", str_lit(&format!(" {}{}{}", pname, if label { " -> " } else { "=" }, pre)));
            let dec = decode_src(l.word_bits, *bits, *shift, *signed);
            if label {
                let _ = write!(body, "__out.push_int((__at as i64) + ({dec}) as i64); ");
            } else if *signed {
                let _ = write!(body, "__out.push_int({dec}); ");
            } else {
                let _ = write!(body, "{n}__disasm_u64(__out, {dec}); ");
            }
            if !post.is_empty() {
                let _ = write!(body, "__out.push_str({}); ", str_lit(&post));
            }
        }
        if let Some(c) = o.cache {
            let _ = write!(body, "__out.push_str(\" [cache\"); var __j = __at + 1; ");
            for (fname, t, _) in &v.caches[c].fields {
                let ts = type_src(t);
                let _ = write!(
                    body,
                    "{{ let __n = (size_of<{ts}>() + {})/{wb}; let __r = {n}__disasm_word(__code, __j, __n); __out.push_str({}); ",
                    wb - 1,
                    str_lit(&format!(" {}=", fname.name))
                );
                let _ = write!(body, "{} __j += __n; }} ", cache_value_src(n, t));
            }
            let _ = write!(body, "__out.push_str(\"]\"); ");
        }
        if !body.is_empty() {
            let _ = writeln!(s, "    {} => {{ let __w = __code[__at]; {}}}", o.opcode, body);
        }
    }
    let _ = writeln!(s, "    _ => {{}}\n  }}\n}}");
    // One instruction, with its offset; superinstructions with their parts.
    let _ = writeln!(s, "{vis}fn {n}__disasm_insn<B>(__code: []{w}, __i: u64, __out: *B) -> u64 {{");
    let _ = writeln!(s, "  if __i >= __code.len {{ return 0; }}");
    let _ = writeln!(s, "  let __w = __code[__i];");
    let _ = writeln!(s, "  {n}__disasm_offset(__out, __i); __out.push_str(\"  \");");
    let _ = writeln!(
        s,
        "  if {n}__opcode(__w) as u64 >= {n}__NUM_OPS {{ {n}__disasm_op(__code, __i, __out); __out.push_str(\"\\n\"); return 1; }}"
    );
    let _ = writeln!(s, "  let __size = {n}__op_size(__w);");
    let _ = writeln!(
        s,
        "  if __i + __size > __code.len {{ __out.push_str({n}__op_name(__w)); __out.push_str(\" <truncated>\\n\"); return __code.len - __i; }}"
    );
    let _ = writeln!(s, "  match {n}__opcode(__w) {{");
    for (k, o) in l.ops.iter().enumerate() {
        if o.parts.is_empty() {
            continue;
        }
        let mut body = format!("__out.push_str({}); ", str_lit(&o.name));
        for (off, _) in l.part_offsets_src(n, k) {
            let _ = write!(
                body,
                "__out.push_str(\"\\n\"); {n}__disasm_offset(__out, __i + {off}); __out.push_str(\"    \"); {n}__disasm_op(__code, __i + {off}, __out); "
            );
        }
        let _ = writeln!(s, "    {} => {{ {}}}", o.opcode, body);
    }
    let _ = writeln!(s, "    _ => {n}__disasm_op(__code, __i, __out),\n  }}");
    let _ = writeln!(s, "  __out.push_str(\"\\n\");\n  __size\n}}");
    let _ = writeln!(
        s,
        "{vis}fn {n}__disasm<B>(__code: []{w}, __out: *B) {{\n\
         \x20 var __i: u64 = 0;\n\
         \x20 while __i < __code.len {{ __i += {n}__disasm_insn(__code, __i, __out); }}\n}}"
    );
    s
}

/// Source printing cache field value `__r` (the field's words, low word
/// first) of type `t` to `__out`.
fn cache_value_src(n: &str, t: &TypeExpr) -> String {
    let int_bits = |name: &str| -> Option<(bool, u32)> {
        let (signed, rest) = match name.as_bytes().first()? {
            b'i' => (true, &name[1..]),
            b'u' => (false, &name[1..]),
            _ => return None,
        };
        let bits: u32 = rest.parse().ok()?;
        (1..=64).contains(&bits).then_some((signed, bits))
    };
    match &t.kind {
        TypeKind::Named(path, args) if path.len() == 1 && args.is_empty() => {
            let name = path[0].name.as_str();
            match (name, int_bits(name)) {
                ("bool", _) => "if __r != 0 { __out.push_str(\"true\"); } else { __out.push_str(\"false\"); }".to_string(),
                ("f64", _) => "__out.push_f64(f64.from_bits(__r));".to_string(),
                (_, Some((true, bits))) => format!("__out.push_int(((__r << {}) as i64) >> {});", 64 - bits, 64 - bits),
                (_, Some((false, 64))) => format!("{n}__disasm_u64(__out, __r);"),
                (_, Some((false, bits))) => format!("{n}__disasm_u64(__out, __r & {});", (1u64 << bits) - 1),
                ("f32", _) => format!("{n}__disasm_hex(__out, __r & 4294967295);"),
                // Aliases and other named types: the raw words.
                _ => format!("{n}__disasm_u64(__out, __r);"),
            }
        }
        TypeKind::Ptr(_) | TypeKind::Option(_) | TypeKind::Fn(..) => {
            format!("if __r == 0 {{ __out.push_str(\"null\"); }} else {{ {n}__disasm_hex(__out, __r); }}")
        }
        _ => format!("{n}__disasm_u64(__out, __r);"),
    }
}

/// Generated items are named `Vm__X`: an op's encoder is `Vm__OP`, so an
/// op named like another generated item (`label`, `verify`, `OP_ADD`,
/// `emit_X`, a field reader `ABC_a`) would define it twice. Report that at
/// the op rather than in the generated source.
fn check_generated_names(v: &VmDecl, items: &[Item]) -> Res<()> {
    let n = &v.name.name;
    let mut seen = std::collections::HashSet::new();
    for it in items {
        let name = match it {
            Item::Fn(f) => &f.name.name,
            Item::Const(c) => &c.name.name,
            _ => continue,
        };
        if !seen.insert(name.clone()) {
            let short = name.strip_prefix(&format!("{n}__")).unwrap_or(name);
            let same: Vec<&OpDecl> = v.ops.iter().filter(|o| o.name.name == short).collect();
            if same.len() > 1 {
                return Err(Diag::new(same[1].name.span, format!("op `{short}` is declared twice")));
            }
            let span = same.first().map(|o| o.name.span).unwrap_or(v.name.span);
            return Err(Diag::new(
                span,
                format!("`{n}.{short}` is generated twice: an op cannot be named like a generated vm item; rename the op"),
            ));
        }
    }
    Ok(())
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
                src.push_str(&generate_labels(v, &l));
                src.push_str(&generate_disasm(v, &l));
                if std::env::var_os("LOIMI_DUMP_VM_EXPANSION").is_some() {
                    eprintln!("// <vm {} expansion>\n{}", v.name.name, src);
                }
                let fid = sm.add(format!("<vm {} expansion>", v.name.name), src.clone());
                let toks = crate::lexer::lex(fid, &src)?;
                let mut p = crate::parser::Parser::new(toks);
                let (items, _) = p.parse_module()?;
                check_generated_names(v, &items)?;
                extra.extend(items);
            }
        }
        m.items.extend(extra);
    }
    Ok(())
}
