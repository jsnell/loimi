//! Names and constant data on enum variants (docs/language.md section
//! 1.2, "Variant names and data").
//!
//! ```text
//! #[data(min: i32, max: i32 = -1)]
//! pub enum Prim: i32 {
//!     #[name("+"), data(0)] Add,
//!     #[name("car"), data(1, 1)] Car,
//! }
//! ```
//!
//! generates, as Loimi source in the enum's module:
//!
//! - `Prim.name(k) -> str`: the `#[name("..")]` of the variant, else the
//!   variant's own name (when some variant has `#[name]` or the enum has
//!   `#[names]`);
//! - `Prim.from_name(s: str) -> ?Prim`: the inverse, a string `match`
//!   (for enums without payloads);
//! - `Prim.min(k) -> i32`, ..: one function per key declared by the
//!   enum's `#[data(key: Type)]`, a `match` on the variant whose arms are
//!   the values the variants give (the key's default where a variant gives
//!   none).
//!
//! The functions are `#[__lazy]`: emitted only if something calls them,
//! so an enum with names and data compiles to the same IR as one
//! without until they are used. The accessors are `#[inline]` (they are
//! a `match` returning constants, which LLVM turns into a lookup table or
//! folds when the variant is known).

use std::fmt::Write as _;

use crate::ast::{EnumDecl, Expr, ExprKind, Item, Module, UnOp};
use crate::diag::{Diag, Generated, Res, SourceMap};
use crate::vmexpand::{splice, str_lit, strip_marks, LAZY_ATTR};

/// Names a key may not take: the generated functions and `COUNT`.
const RESERVED: &[&str] = &["name", "from_name", "COUNT"];

pub fn expand(sm: &mut SourceMap, modules: &mut [Module]) -> Res<()> {
    for m in modules.iter_mut() {
        let mut extra = Vec::new();
        for it in &m.items {
            let Item::Enum(e) = it else { continue };
            let Some(src) = generate(sm, e, &m.items)? else { continue };
            let (src, origins) = strip_marks(&src);
            if std::env::var_os("LOIMI_DUMP_VM_EXPANSION").is_some() {
                eprintln!("// <enum {} expansion>\n{}", e.name.name, src);
            }
            let generated = Generated { vm: format!("enum {}", e.name.name), origins };
            let fid = sm.add_generated(format!("<enum {} expansion>", e.name.name), src.clone(), generated);
            let toks = crate::lexer::lex(fid, &src)?;
            let mut p = crate::parser::Parser::new(toks);
            let (mut items, _) = p.parse_module()?;
            for it in &mut items {
                if let Item::Fn(f) = it {
                    f.attrs.list.push((LAZY_ATTR.into(), vec![], f.name.span));
                }
            }
            extra.extend(items);
        }
        m.items.extend(extra);
    }
    Ok(())
}

/// A declared key: its name, type text and default.
struct Key<'a> {
    name: &'a str,
    ty: String,
    default: Option<&'a Expr>,
}

/// The generated source for `e`, if it has names or data.
fn generate(sm: &SourceMap, e: &EnumDecl, items: &[Item]) -> Res<Option<String>> {
    let named = e.attrs.has("names") || e.variants.iter().any(|v| v.attrs.has("name"));
    let has_data = e.attrs.has("data") || e.variants.iter().any(|v| v.attrs.has("data"));
    if !named && !has_data {
        return Ok(None);
    }
    let en = &e.name.name;
    if !e.generics.is_empty() {
        let sp = e.attrs.list.iter().chain(e.variants.iter().flat_map(|v| v.attrs.list.iter())).find(|a| a.0 == "names" || a.0 == "name" || a.0 == "data").map_or(e.name.span, |a| a.2);
        return Err(Diag::new(sp, format!("`{}` is generic: names and data are for enums without type parameters", en)));
    }
    if let Some(a) = e.attrs.list.iter().find(|a| a.0 == "names") {
        if !a.1.is_empty() {
            return Err(Diag::new(a.2, "`#[names]` takes no arguments (name a variant with `#[name(\"..\")]`)"));
        }
    }
    // The keys, from the enum's `#[data(key: Type [= default], ..)]`.
    let mut keys: Vec<Key> = Vec::new();
    for d in &e.attrs.data {
        let (Some(k), Some(ty)) = (&d.key, &d.ty) else {
            return Err(Diag::new(d.span, format!("`#[data]` on an enum declares keys: `#[data(min: i32, max: i32 = -1)]`; the variants give the values")));
        };
        if RESERVED.contains(&k.name.as_str()) {
            return Err(Diag::new(k.span, format!("`{}` cannot be a data key (`{}.{}` is generated or built in)", k.name, en, k.name)));
        }
        if keys.iter().any(|x| x.name == k.name) {
            return Err(Diag::new(k.span, format!("data key `{}` is declared twice", k.name)));
        }
        if let Some(v) = &d.value {
            check_const(v)?;
        }
        keys.push(Key { name: &k.name, ty: sm.text(ty.span).to_string(), default: d.value.as_ref() });
    }
    // Each variant's values, by key.
    let mut values: Vec<Vec<Option<&Expr>>> = Vec::new();
    let mut names: Vec<String> = Vec::new();
    for v in &e.variants {
        let mut row: Vec<Option<&Expr>> = vec![None; keys.len()];
        let mut pos = 0;
        for d in &v.attrs.data {
            if keys.is_empty() {
                return Err(Diag::new(d.span, format!("`{}` declares no data keys: declare them on the enum, `#[data(key: Type)] enum {} {{ .. }}`", en, en)));
            }
            if d.ty.is_some() {
                return Err(Diag::new(d.span, "a variant gives values (`key = value`); the key's type goes on the enum's `#[data]`"));
            }
            let i = match &d.key {
                Some(k) => match keys.iter().position(|x| x.name == k.name) {
                    Some(i) => i,
                    None => {
                        let known: Vec<&str> = keys.iter().map(|k| k.name).collect();
                        return Err(Diag::new(k.span, format!("`{}` has no data key `{}` (keys: {})", en, k.name, known.join(", "))));
                    }
                },
                None => {
                    if v.attrs.data.iter().take_while(|x| !std::ptr::eq(*x, d)).any(|x| x.key.is_some()) {
                        return Err(Diag::new(d.span, "positional values come before `key = value` ones"));
                    }
                    if pos >= keys.len() {
                        return Err(Diag::new(d.span, format!("too many values: `{}` declares {} data key{}", en, keys.len(), if keys.len() == 1 { "" } else { "s" })));
                    }
                    pos += 1;
                    pos - 1
                }
            };
            if row[i].is_some() {
                return Err(Diag::new(d.span, format!("`{}.{}` gives `{}` twice", en, v.name.name, keys[i].name)));
            }
            let val = d.value.as_ref().unwrap();
            check_const(val)?;
            row[i] = Some(val);
        }
        for (i, k) in keys.iter().enumerate() {
            if row[i].is_none() && k.default.is_none() {
                return Err(Diag::new(v.name.span, format!("`{}.{}` gives no `{}` and the key has no default (`#[data({}: {} = ..)]`)", en, v.name.name, k.name, k.name, k.ty)));
            }
        }
        values.push(row);
        let name = match v.attrs.list.iter().find(|a| a.0 == "name") {
            Some((_, args, sp)) => {
                if args.len() != 1 {
                    return Err(Diag::new(*sp, "`#[name]` takes one string: `#[name(\"car\")]`"));
                }
                args[0].clone()
            }
            None => v.name.name.clone(),
        };
        if let Some(j) = names.iter().position(|n| *n == name) {
            return Err(Diag::new(v.name.span, format!("`{}.{}` and `{}.{}` have the same name \"{}\"", en, e.variants[j].name.name, en, v.name.name, name)));
        }
        names.push(name);
    }
    // A method of the same name written by hand would clash.
    let mut generated: Vec<&str> = keys.iter().map(|k| k.name).collect();
    if named {
        generated.extend(["name", "from_name"]);
    }
    for it in items {
        if let Item::Fn(f) = it {
            if f.recv.as_ref().is_some_and(|r| r.name == *en) && generated.contains(&f.name.name.as_str()) {
                let why = if keys.iter().any(|k| k.name == f.name.name) { "by its `#[data]` key" } else { "by `#[name]`/`#[names]`" };
                return Err(Diag::new(f.name.span, format!("`{}.{}` is generated {}; rename one", en, f.name.name, why)));
            }
        }
    }
    // The patterns, one per variant: `E.V` or `E.V(_, _)`.
    let pats: Vec<String> = e
        .variants
        .iter()
        .map(|v| {
            if v.fields.is_empty() {
                format!("{}.{}", en, v.name.name)
            } else {
                format!("{}.{}({})", en, v.name.name, vec!["_"; v.fields.len()].join(", "))
            }
        })
        .collect();
    let vis = if e.is_pub { "pub " } else { "" };
    let mut src = String::new();
    if named {
        let _ = writeln!(src, "#[inline] {vis}fn {en}.name(k: {en}) -> str {{\n    match k {{");
        for (p, n) in pats.iter().zip(&names) {
            let _ = writeln!(src, "        {} => {},", p, str_lit(n));
        }
        let _ = writeln!(src, "    }}\n}}");
        if e.variants.iter().all(|v| v.fields.is_empty()) {
            let _ = writeln!(src, "{vis}fn {en}.from_name(s: str) -> ?{en} {{\n    match s {{");
            for (p, n) in pats.iter().zip(&names) {
                let _ = writeln!(src, "        {} => Some({}),", str_lit(n), p);
            }
            let _ = writeln!(src, "        _ => None,\n    }}\n}}");
        }
    }
    for (i, k) in keys.iter().enumerate() {
        let ctx = format!("the generated `{}.{}` (`#[data]`)", en, k.name);
        let _ = writeln!(src, "#[inline] {vis}fn {en}.{}(k: {en}) -> {} {{\n    match k {{", k.name, k.ty);
        for (p, row) in pats.iter().zip(&values) {
            let val = row[i].or(k.default).unwrap();
            let _ = writeln!(src, "        {} => {},", p, splice(sm, val.span, &ctx));
        }
        let _ = writeln!(src, "    }}\n}}");
    }
    Ok(Some(src))
}

/// Data values are constants: literals, names of constants and variants,
/// and constructors, tuples, arrays, casts and operators of them.
fn check_const(e: &Expr) -> Res<()> {
    let ok = match &e.kind {
        ExprKind::Int(_) | ExprKind::Float(_) | ExprKind::Str(_) | ExprKind::Char(_) | ExprKind::Bool(_) | ExprKind::Unit => true,
        ExprKind::Ident(_) | ExprKind::DotVariant(_) => true,
        ExprKind::Field(b, _) => return check_const(b),
        ExprKind::Unary(UnOp::Neg | UnOp::Not, b) | ExprKind::Cast(b, _) => return check_const(b),
        ExprKind::Binary(_, a, b) => {
            check_const(a)?;
            return check_const(b);
        }
        ExprKind::Tuple(xs) | ExprKind::ArrayLit(xs) => {
            for x in xs {
                check_const(x)?;
            }
            true
        }
        // `Some(c)`, `Op.Add(c)`: a constructor, named in upper case (a
        // function call is not a constant).
        ExprKind::Call(f, xs) => {
            let ctor = match &f.kind {
                ExprKind::Ident(n) => n.starts_with(|c: char| c.is_ascii_uppercase()),
                ExprKind::Field(_, n) => n.name.starts_with(|c: char| c.is_ascii_uppercase()),
                _ => false,
            };
            if !ctor {
                return Err(Diag::new(e.span, "a data value is a constant: a function call is not (constructors such as `Some(..)` are)"));
            }
            check_const(f)?;
            for x in xs {
                check_const(x)?;
            }
            true
        }
        _ => false,
    };
    if ok {
        Ok(())
    } else {
        Err(Diag::new(e.span, "a data value is a constant: a literal, a constant's or variant's name, or a tuple, array or constructor of them"))
    }
}
