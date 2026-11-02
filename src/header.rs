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
//! | enum without payloads    | its tag integer type                     |
//!
//! Everything else (structs, tuples, arrays and `Option`/`Result` by
//! value, other slices, `Gc<T>`, `value` types, enums with payloads) has no
//! C equivalent and is rejected.

use std::collections::BTreeMap;

use crate::ast::Linkage;
use crate::diag::Diag;
use crate::tir::{FnDef, Program, Ty};

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
    /// C name -> Loimi struct name, for the opaque declarations.
    opaque: BTreeMap<String, String>,
}

fn short(s: &str) -> &str {
    s.rsplit('.').next().unwrap_or(s)
}

fn ty_name(p: &Program, t: &Ty) -> String {
    match t {
        Ty::Int(it) => format!("{}{}", if it.signed { "i" } else { "u" }, it.bits),
        Ty::F32 => "f32".into(),
        Ty::F64 => "f64".into(),
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
        Ty::Param(i) => format!("?T{}", i),
        Ty::Gc(t) => format!("Gc<{}>", ty_name(p, t)),
    }
}

impl<'p> Gen<'p> {
    /// The C declaration of `inner` (a name, possibly empty, possibly with
    /// leading `*`s) with type `t`. `Err` says why `t` has no C equivalent.
    fn decl(&mut self, t: &Ty, inner: &str, is_result: bool) -> Result<String, String> {
        let p = self.p;
        let base = |b: &str| if inner.is_empty() { b.to_string() } else { format!("{} {}", b, inner) };
        Ok(match t {
            Ty::Int(it) => base(&format!("{}int{}_t", if it.signed { "" } else { "u" }, it.bits)),
            Ty::F32 => base("float"),
            Ty::F64 => base("double"),
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
                    let c = short(name).to_string();
                    if c.contains('<') {
                        return Err(format!("generic struct `{}` has no C name", c));
                    }
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
                base(&format!("{}int{}_t", if e.tag.signed { "" } else { "u" }, e.tag.bits))
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
            Ty::Param(_) => return Err("generic functions cannot be exported".into()),
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
    for (i, t) in f.param_tys.iter().enumerate() {
        let name = param_name(f, i);
        let d = g.decl(t, &name, false).map_err(|e| err(format!("parameter `{}: {}`", name, ty_name(g.p, t)), e))?;
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

/// Check that every `export "C"` function has a C signature.
pub fn check_exports(p: &Program) -> Result<(), Diag> {
    let mut g = Gen { p, opaque: BTreeMap::new() };
    for f in exports(p) {
        prototype(&mut g, f)?;
    }
    Ok(())
}

/// The header for `p`'s exports. `guard` names the include guard.
pub fn generate(p: &Program, source: &str, guard: &str) -> Result<String, Diag> {
    let mut g = Gen { p, opaque: BTreeMap::new() };
    let mut protos = Vec::new();
    for f in exports(p) {
        protos.push(prototype(&mut g, f)?);
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
        for c in g.opaque.keys() {
            out.push_str(&format!("typedef struct {0} {0};\n", c));
        }
        out.push('\n');
    }
    for d in protos {
        out.push_str(&d);
        out.push_str(";\n");
    }
    out.push_str("\n#ifdef __cplusplus\n}\n#endif\n\n");
    out.push_str(&format!("#endif /* {} */\n", guard));
    Ok(out)
}
