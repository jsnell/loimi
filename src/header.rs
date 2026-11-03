//! `loimi build --header out.h`: a C header declaring every `export "C"`
//! function, and the C-ABI checks that exports must pass in library builds.
//!
//! Type mapping (see docs/language.md §10):
//!
//! | Loimi                    | C                                        |
//! |--------------------------|------------------------------------------|
//! | `i8`..`i64`, `u8`..`u64` | `int8_t`..`uint64_t`                     |
//! | `f32`, `f64`, `bool`     | `float`, `double`, `bool`                |
//! | `()` (result only)       | `void`                                   |
//! | `*u8`                    | `void *` (Loimi's untyped byte pointer)  |
//! | `*T` (T mappable)        | `T *`                                    |
//! | `*S`, S a struct         | `S *`, S declared opaque                 |
//! | `?*T`, `?fn(..)`         | the pointer type; NULL is `None`         |
//! | `fn(A, B) -> R`          | `R (*)(A, B)`                            |
//! | `str`                    | `loimi_str`, `{ const char *ptr; size_t len; }` |
//! | enum E without payloads  | `E`, a typedef of its tag integer type, and constants `E_Variant` |
//!
//! Everything else (structs, tuples, arrays and `Option`/`Result` by
//! value, other slices, `Gc<T>`, `value` types, enums with payloads) has no
//! C equivalent and is rejected.

use std::collections::BTreeMap;

use crate::ast::Linkage;
use crate::diag::{Diag, SourceMap, Span};
use crate::tir::{FnDef, Program, TExpr, Ty, TK};

/// Integer argument registers on x86-64 (System V); AArch64 has eight.
const INT_ARG_REGS: usize = 6;

const C_KEYWORDS: &[&str] = &[
    "auto", "break", "case", "char", "const", "continue", "default", "do", "double", "else", "enum", "extern", "float",
    "for", "goto", "if", "inline", "int", "long", "register", "restrict", "return", "short", "signed", "sizeof",
    "static", "struct", "switch", "typedef", "union", "unsigned", "void", "volatile", "while", "bool", "true",
    "false", "class", "new", "delete", "this", "template", "namespace", "operator", "private", "public",
];

struct Gen<'p> {
    p: &'p Program,
    /// `--header-prefix`: prepended to the C names of structs and enums
    /// without a `#[c_name]`.
    prefix: &'p str,
    /// C name -> Loimi struct name, for the opaque declarations.
    opaque: BTreeMap<String, String>,
    /// C name -> enum id, for the typedefs and constants.
    enums: BTreeMap<String, u32>,
}

fn short(s: &str) -> &str {
    crate::check::short_name(s)
}

fn ty_name(p: &Program, t: &Ty) -> String {
    match t {
        Ty::Int(it) => format!("{}{}", if it.signed { "i" } else { "u" }, it.bits),
        Ty::F32 => "f32".into(),
        Ty::F64 => "f64".into(),
        Ty::Vector(l, n) => format!("{}x{}", ty_name(p, l), n),
        Ty::Bool => "bool".into(),
        Ty::Unit => "()".into(),
        Ty::Never => "never".into(),
        Ty::Ptr(t) => format!("*{}", ty_name(p, t)),
        Ty::Slice(t) if **t == Ty::u8() => "str".into(),
        Ty::Slice(t) => format!("[]{}", ty_name(p, t)),
        Ty::Array(t, n) => format!("[{}; {}]", ty_name(p, t), n),
        Ty::Option(t) => format!("?{}", ty_name(p, t)),
        Ty::Result(a, b) => format!("Result<{}, {}>", ty_name(p, a), ty_name(p, b)),
        Ty::Tuple(ts) => format!("({})", ts.iter().map(|t| ty_name(p, t)).collect::<Vec<_>>().join(", ")),
        Ty::Fn(ps, r, _) => {
            format!("fn({}) -> {}", ps.iter().map(|t| ty_name(p, t)).collect::<Vec<_>>().join(", "), ty_name(p, r))
        }
        Ty::Struct(i) => short(&p.structs[*i as usize].name).into(),
        Ty::Enum(i) => short(&p.enums[*i as usize].name).into(),
        Ty::Value(i) => short(&p.values[*i as usize].name).into(),
        Ty::Param(i) | Ty::Opaque(i) => format!("?T{}", i),
        Ty::Block(ps, r, _) => {
            format!("block fn({}) -> {}", ps.iter().map(|t| ty_name(p, t)).collect::<Vec<_>>().join(", "), ty_name(p, r))
        }
        Ty::Closure(ps, r, _) => {
            format!("closure fn({}) -> {}", ps.iter().map(|t| ty_name(p, t)).collect::<Vec<_>>().join(", "), ty_name(p, r))
        }
        Ty::Gc(t) => format!("Gc<{}>", ty_name(p, t)),
    }
}

impl<'p> Gen<'p> {
    /// The C name of struct or enum `name` (its Loimi name).
    fn c_name(&self, name: &str) -> String {
        match self.p.c_names.get(name) {
            Some(c) => c.clone(),
            None => format!("{}{}", self.prefix, short(name)),
        }
    }

    /// The C declaration of `inner` (a name, possibly empty, possibly with
    /// leading `*`s) with type `t`. `Err` says why `t` has no C equivalent.
    fn decl(&mut self, t: &Ty, inner: &str, is_result: bool) -> Result<String, String> {
        let p = self.p;
        let base = |b: &str| if inner.is_empty() { b.to_string() } else { format!("{} {}", b, inner) };
        Ok(match t {
            Ty::Int(it) if !matches!(it.bits, 8 | 16 | 32 | 64) => {
                return Err(format!("`{}` has no C equivalent: C has 8-, 16-, 32- and 64-bit integers", ty_name(p, t)))
            }
            Ty::Int(it) => base(&format!("{}int{}_t", if it.signed { "" } else { "u" }, it.bits)),
            Ty::F32 => base("float"),
            Ty::F64 => base("double"),
            Ty::Vector(..) => {
                return Err(format!("`{}` has no portable C equivalent; pass vectors through memory (`*{}`)", ty_name(p, t), ty_name(p, t)))
            }
            Ty::Bool => base("bool"),
            Ty::Unit | Ty::Never if is_result => base("void"),
            Ty::Unit | Ty::Never => return Err("`()` is only allowed as a result".into()),
            Ty::Slice(e) if **e == Ty::u8() => base("loimi_str"),
            Ty::Slice(_) => return Err("slices other than `str` have no C equivalent".into()),
            Ty::Ptr(x) => match &**x {
                Ty::Int(it) if it.bits == 8 && !it.signed => format!("void *{}", inner),
                Ty::Struct(i) => {
                    let name = &p.structs[*i as usize].name;
                    if p.structs[*i as usize].object.is_some() {
                        return Err(format!("`{}` is a GC object; pointers into the GC heap cannot cross into C", short(name)));
                    }
                    if short(name).contains('<') {
                        return Err(format!("generic struct `{}` has no C name", short(name)));
                    }
                    let c = self.c_name(name);
                    if let Some(prev) = self.opaque.get(&c) {
                        if prev != name {
                            return Err(format!("two exported structs are both named `{}` in C", c));
                        }
                    }
                    self.opaque.insert(c.clone(), name.clone());
                    format!("{} *{}", c, inner)
                }
                Ty::Unit => return Err("`*()` has no C equivalent; use `*u8`".into()),
                _ => self.decl(x, &format!("*{}", inner), false).map_err(|e| format!("in `{}`: {}", ty_name(p, t), e))?,
            },
            Ty::Fn(ps, r, _) => {
                let mut params = Vec::new();
                for t in ps {
                    params.push(self.decl(t, "", false)?);
                }
                if params.is_empty() {
                    params.push("void".into());
                }
                self.decl(r, &format!("(*{})({})", inner, params.join(", ")), true)?
            }
            Ty::Option(x) if x.is_nullable_ptr() => {
                if matches!(**x, Ty::Gc(_)) {
                    return Err("pointers into the GC heap cannot cross into C".into());
                }
                self.decl(x, inner, is_result)?
            }
            Ty::Option(_) => return Err("`?T` by value has no C equivalent (only `?*T` and `?fn`, as nullable pointers)".into()),
            Ty::Enum(i) => {
                let e = &p.enums[*i as usize];
                if e.has_payload() {
                    return Err(format!("enum `{}` has payloads; pass a pointer to a struct instead", short(&e.name)));
                }
                if short(&e.name).contains('<') {
                    return Err(format!("generic enum `{}` has no C name", short(&e.name)));
                }
                let c = self.c_name(&e.name);
                if let Some(&prev) = self.enums.get(&c) {
                    if prev != *i {
                        return Err(format!("two exported enums are both named `{}` in C", c));
                    }
                }
                self.enums.insert(c.clone(), *i);
                base(&c)
            }
            Ty::Struct(i) => {
                return Err(format!(
                    "struct `{}` by value has no C equivalent here; pass `*{}` (an opaque pointer in C)",
                    short(&p.structs[*i as usize].name),
                    short(&p.structs[*i as usize].name)
                ))
            }
            Ty::Gc(_) => return Err("`Gc<T>` cannot cross into C: the host would hold a reference the collector cannot see".into()),
            Ty::Value(_) => return Err("`value` types have no C equivalent; convert to a number, string or pointer".into()),
            Ty::Result(..) => return Err("`Result` has no C equivalent; return a status code".into()),
            Ty::Tuple(_) => return Err("tuples have no C equivalent".into()),
            Ty::Array(..) => return Err("arrays by value have no C equivalent; pass a pointer".into()),
            Ty::Param(_) | Ty::Opaque(_) => return Err("generic functions cannot be exported".into()),
            Ty::Block(..) => return Err("`block fn` closures cannot cross into C; pass a function pointer and a data pointer".into()),
            Ty::Closure(..) => return Err("closures cannot cross into C; pass a function pointer and a data pointer".into()),
        })
    }
}

/// Integer-class argument registers `t` takes when passed by value.
fn int_regs(t: &Ty) -> usize {
    match t {
        Ty::F32 | Ty::F64 => 0,
        Ty::Slice(_) => 2,
        _ => 1,
    }
}

fn exports(p: &Program) -> impl Iterator<Item = &FnDef> {
    p.fns.iter().filter(|f| f.linkage == Linkage::Export && f.body.is_some())
}

fn param_name(f: &FnDef, i: usize) -> String {
    let n = &f.locals[f.params[i] as usize].name;
    if C_KEYWORDS.contains(&n.as_str()) { format!("{}_", n) } else { n.clone() }
}

/// The C prototype of `f`, or why it has none.
fn prototype(g: &mut Gen, f: &FnDef) -> Result<String, Diag> {
    let err = |what: String, why: String| {
        Diag::new(f.span, format!("`export \"C\" fn {}`: {} has no C equivalent: {}", f.name, what, why))
    };
    if f.variadic {
        return Err(Diag::new(f.span, format!("`export \"C\" fn {}` cannot be variadic", f.name)));
    }
    let mut params = Vec::new();
    let mut regs = 0;
    let consts = g.p.c_const_params.get(&f.symbol);
    for (i, t) in f.param_tys.iter().enumerate() {
        let name = param_name(f, i);
        let loimi_name = &f.locals[f.params[i] as usize].name;
        let mut d = g.decl(t, &name, false).map_err(|e| err(format!("parameter `{}: {}`", name, ty_name(g.p, t)), e))?;
        if let Some((_, sp)) = consts.and_then(|c| c.iter().find(|(n, _)| n == loimi_name)) {
            // `#[c_const(p)]`: `const T *p`. The pointee must be a value,
            // not another pointer, so that the `const` reads unambiguously.
            match t {
                Ty::Ptr(x) if !matches!(**x, Ty::Ptr(_) | Ty::Fn(..) | Ty::Option(_)) => d = format!("const {}", d),
                _ => {
                    return Err(Diag::new(
                        *sp,
                        format!("`#[c_const({})]`: only a pointer to a value (`*u8`, `*T`) can be `const` in C", loimi_name),
                    ))
                }
            }
        }
        if matches!(t, Ty::Slice(_)) && regs + 2 > INT_ARG_REGS {
            // C passes a 16-byte struct whole, in two registers or on the
            // stack; LLVM splits `{ ptr, i64 }` and could put one half in
            // the last register. Keep `str` parameters where both agree.
            return Err(Diag::new(
                f.span,
                format!(
                    "`export \"C\" fn {}`: `str` parameter `{}` comes after {} integer or pointer arguments; \
                     it must fit in the first {} argument registers (move it earlier, or pass a pointer and a length)",
                    f.name, name, regs, INT_ARG_REGS
                ),
            ));
        }
        regs += int_regs(t);
        params.push(d);
    }
    if params.is_empty() {
        params.push("void".into());
    }
    let head = format!("{}({})", f.symbol, params.join(", "));
    g.decl(&f.ret, &head, true).map_err(|e| err(format!("result `{}`", ty_name(g.p, &f.ret)), e))
}

/// The `#define` of an `export "C" const`, or why it has none.
fn const_define(g: &mut Gen, name: &str, v: &TExpr, span: Span) -> Result<String, Diag> {
    let err = |why: &str| Diag::new(span, format!("`export \"C\" const {}`: {}", name, why));
    let value = match (&v.ty, &v.kind) {
        (Ty::Int(it), TK::Int(n)) => {
            let suffix = match (it.signed, it.bits) {
                (false, 64) => "ULL",
                (true, 64) => "LL",
                (false, _) => "u",
                (true, _) => "",
            };
            if *n < 0 {
                format!("({}{})", n, suffix)
            } else {
                format!("{}{}", n, suffix)
            }
        }
        (Ty::Bool, TK::Bool(b)) => b.to_string(),
        (Ty::F64, TK::Float(x)) => format!("{:?}", x),
        (Ty::F32, TK::Float(x)) => format!("{:?}f", x),
        (Ty::Slice(e), TK::Str(bytes)) if **e == Ty::u8() => {
            let mut s = String::from("\"");
            for &b in bytes {
                match b {
                    b'"' | b'\\' => {
                        s.push('\\');
                        s.push(b as char);
                    }
                    0x20..=0x7e => s.push(b as char),
                    _ => s.push_str(&format!("\\{:03o}", b)),
                }
            }
            s.push('"');
            s
        }
        (Ty::Enum(ei), TK::Variant(vi, fields)) if fields.is_empty() => {
            let c = g.decl(&v.ty, "", false).map_err(|e| err(&e))?;
            format!("(({}){})", c, g.p.enums[*ei as usize].variants[*vi as usize].discr)
        }
        _ => return Err(err("only integers, `bool`, floats, `str` and payload-free enum variants can be C constants")),
    };
    Ok(format!("#define {} {}", name, value))
}

/// Check that every `export "C"` function has a C signature, and every
/// `export "C" const` a C value.
pub fn check_exports(p: &Program) -> Result<(), Diag> {
    let mut g = Gen { p, prefix: "", opaque: BTreeMap::new(), enums: BTreeMap::new() };
    for f in exports(p) {
        prototype(&mut g, f)?;
    }
    for (name, v, span) in &p.c_consts {
        const_define(&mut g, name, v, *span)?;
    }
    Ok(())
}

/// The `///` comment lines above the item whose name is at `span`
/// (skipping attribute lines), without the slashes.
fn doc_lines(sm: &SourceMap, span: Span) -> Vec<String> {
    let Some(file) = sm.files.get(span.file as usize) else { return vec![] };
    let before = &file.text[..(span.lo as usize).min(file.text.len())];
    let start = before.rfind('\n').map(|i| i + 1).unwrap_or(0);
    let mut doc = Vec::new();
    for l in before[..start].lines().rev() {
        let t = l.trim_start();
        if let Some(d) = t.strip_prefix("///") {
            doc.push(d.strip_prefix(' ').unwrap_or(d).trim_end().replace("*/", "* /"));
        } else if !t.starts_with("#[") {
            break;
        }
    }
    doc.reverse();
    doc
}

/// `doc` as a C comment, or nothing.
fn c_comment(doc: &[String]) -> String {
    match doc {
        [] => String::new(),
        [one] => format!("/* {} */\n", one),
        _ => {
            let mut s = format!("/* {}\n", doc[0]);
            for (i, l) in doc[1..].iter().enumerate() {
                let end = if i + 2 == doc.len() { " */" } else { "" };
                if l.is_empty() {
                    s.push_str(&format!(" *{}\n", end));
                } else {
                    s.push_str(&format!(" * {}{}\n", l, end));
                }
            }
            s
        }
    }
}

/// The `///` comment of struct or enum `name` as a C comment, or nothing.
fn type_doc(p: &Program, sm: &SourceMap, name: &str) -> String {
    p.type_spans.get(name).map(|sp| c_comment(&doc_lines(sm, *sp))).unwrap_or_default()
}

/// The header for `p`'s exports. `guard` names the include guard;
/// `prefix` goes before the C names of structs and enums without a
/// `#[c_name]`.
pub fn generate(p: &Program, sm: &SourceMap, source: &str, guard: &str, prefix: &str) -> Result<String, Diag> {
    let mut g = Gen { p, prefix, opaque: BTreeMap::new(), enums: BTreeMap::new() };
    let mut protos = Vec::new();
    for f in exports(p) {
        protos.push((c_comment(&doc_lines(sm, f.span)), prototype(&mut g, f)?));
    }
    let mut defines = Vec::new();
    for (name, v, span) in &p.c_consts {
        defines.push((c_comment(&doc_lines(sm, *span)), const_define(&mut g, name, v, *span)?));
    }
    let mut out = String::new();
    out.push_str(&format!("/* Generated by `loimi build --header` from {}. Do not edit. */\n", source));
    out.push_str(&format!("#ifndef {0}\n#define {0}\n\n", guard));
    out.push_str("#include <stdbool.h>\n#include <stddef.h>\n#include <stdint.h>\n\n");
    out.push_str("#ifdef __cplusplus\nextern \"C\" {\n#endif\n\n");
    out.push_str(
        "#ifndef LOIMI_STR_DEFINED\n#define LOIMI_STR_DEFINED\n\
         /* A Loimi `str`: bytes and a length, not NUL-terminated. */\n\
         typedef struct loimi_str { const char *ptr; size_t len; } loimi_str;\n\
         #define LOIMI_STR(lit) ((loimi_str){ (lit), sizeof(lit) - 1 })\n#endif\n\n",
    );
    if !g.opaque.is_empty() {
        out.push_str("/* Opaque Loimi structs, used through pointers. */\n");
        for (c, name) in &g.opaque {
            out.push_str(&type_doc(p, sm, name));
            out.push_str(&format!("typedef struct {0} {0};\n", c));
        }
        out.push('\n');
    }
    for (c, &i) in &g.enums {
        let e = &p.enums[i as usize];
        let doc = type_doc(p, sm, &e.name);
        out.push_str(&if doc.is_empty() { format!("/* enum {} */\n", c) } else { doc });
        out.push_str(&format!(
            "typedef {}int{}_t {};\n",
            if e.tag.signed { "" } else { "u" },
            e.tag.bits,
            c
        ));
        if e.variants.iter().all(|v| i32::try_from(v.discr).is_ok()) {
            let vs: Vec<String> = e.variants.iter().map(|v| format!("{}_{} = {}", c, v.name, v.discr)).collect();
            out.push_str(&format!("enum {{ {} }};\n\n", vs.join(", ")));
        } else {
            // Out of range for C enum constants.
            for v in &e.variants {
                let suffix = if v.discr > i64::MAX as i128 { "ULL" } else { "LL" };
                out.push_str(&format!("#define {0}_{1} (({0}){2}{3})\n", c, v.name, v.discr, suffix));
            }
            out.push('\n');
        }
    }
    if !defines.is_empty() {
        for (doc, d) in &defines {
            out.push_str(doc);
            out.push_str(d);
            out.push('\n');
        }
        out.push('\n');
    }
    for (i, (doc, d)) in protos.iter().enumerate() {
        // A blank line before a documented declaration.
        if !doc.is_empty() && i > 0 {
            out.push('\n');
        }
        out.push_str(doc);
        out.push_str(d);
        out.push_str(";\n");
    }
    out.push_str("\n#ifdef __cplusplus\n}\n#endif\n\n");
    out.push_str(&format!("#endif /* {} */\n", guard));
    Ok(out)
}
