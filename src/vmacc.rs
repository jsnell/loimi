//! Accumulator variants (`language.md` §7.4): `accumulator acc: T for
//! k1, k2;` declares a state register that can hold, between two adjacent
//! ops, a value that would otherwise go through a slot of kind k1 or k2;
//! `#[accumulate] op` derives the op's variants that write their result to
//! an accumulator instead of the destination slot and/or read one source
//! from an accumulator. The variants are ordinary ops (appended after the
//! declared ones), so handlers, supers, the verifier and the disassembler
//! treat them like any other; `Vm.accumulate(code, first_temp)` (generated
//! in `vmexpand`) selects them.
//!
//! An operand taken from or put in an accumulator keeps its field in the
//! word (the variant has the base op's format and operand positions), as a
//! hidden kind `__acc` whose value the handler ignores, so selecting a
//! variant only changes the opcode.

use crate::ast::*;
use crate::diag::{Diag, Res, Span};
use crate::vmast::*;

/// The operand kind of an operand that lives in an accumulator.
pub const ACC_KIND: &str = "__acc";

fn ident_expr(name: &str, span: Span) -> Expr {
    Expr { kind: ExprKind::Ident(name.to_string()), span }
}

/// A primitive number type: (is float, bits).
fn prim(t: &TypeExpr) -> Option<(bool, u32)> {
    let TypeKind::Named(p, a) = &t.kind else { return None };
    if p.len() != 1 || !a.is_empty() {
        return None;
    }
    Some(match p[0].name.as_str() {
        "u8" | "i8" => (false, 8),
        "u16" | "i16" => (false, 16),
        "u32" | "i32" => (false, 32),
        "u64" | "i64" => (false, 64),
        "f32" => (true, 32),
        "f64" => (true, 64),
        _ => return None,
    })
}

/// The type of the place an operand kind names, when the body spells it:
/// `*(E as *T)` or `s[it]` for a state variable `s: *T`.
fn kind_type(v: &VmDecl, k: &OperandDecl) -> Option<TypeExpr> {
    match &k.body.kind {
        ExprKind::Unary(UnOp::Deref, e) => match &e.kind {
            ExprKind::Cast(_, t) => match &t.kind {
                TypeKind::Ptr(inner) => Some((**inner).clone()),
                _ => None,
            },
            _ => None,
        },
        ExprKind::Index(b, _) => {
            let ExprKind::Ident(s) = &b.kind else { return None };
            let p = v.state.iter().find(|p| p.name.name == *s)?;
            match &p.ty.kind {
                TypeKind::Ptr(inner) => Some((**inner).clone()),
                _ => None,
            }
        }
        _ => None,
    }
}

/// How a body uses a name: (assigned with `x = e`, read or otherwise used).
#[derive(Default, Clone, Copy)]
struct Use {
    assigned: bool,
    read: bool,
}

fn walk_block(b: &mut Block, f: &mut dyn FnMut(&mut Expr) -> bool) {
    for s in &mut b.stmts {
        match s {
            Stmt::Let { init, els, .. } => {
                if let Some(e) = init {
                    walk(e, f);
                }
                if let Some(b) = els {
                    walk_block(b, f);
                }
            }
            Stmt::Expr { expr, .. } => walk(expr, f),
            Stmt::Defer(e) => walk(e, f),
        }
    }
}

fn walk_conds(cs: &mut [Cond], f: &mut dyn FnMut(&mut Expr) -> bool) {
    for c in cs {
        match c {
            Cond::Expr(e) | Cond::Let(_, e) => walk(e, f),
        }
    }
}

/// Visits every expression, outermost first; `f` returns false to skip
/// the children of the one it was given.
fn walk(e: &mut Expr, f: &mut dyn FnMut(&mut Expr) -> bool) {
    if !f(e) {
        return;
    }
    match &mut e.kind {
        ExprKind::Int(_)
        | ExprKind::Float(_)
        | ExprKind::Str(_)
        | ExprKind::Char(_)
        | ExprKind::Bool(_)
        | ExprKind::Ident(_)
        | ExprKind::DotVariant(_)
        | ExprKind::Unit
        | ExprKind::Continue
        | ExprKind::SizeOf(_) => {}
        ExprKind::Field(a, _) | ExprKind::Unary(_, a) | ExprKind::Cast(a, _) | ExprKind::Try(a) | ExprKind::TypeArgs(a, _) => walk(a, f),
        ExprKind::Call(c, args) => {
            walk(c, f);
            for a in args {
                walk(a, f);
            }
        }
        ExprKind::Index(a, b)
        | ExprKind::Binary(_, a, b)
        | ExprKind::Assign(a, b)
        | ExprKind::CompoundAssign(_, a, b)
        | ExprKind::Coalesce(a, b)
        | ExprKind::ArrayRepeat(a, b) => {
            walk(a, f);
            walk(b, f);
        }
        ExprKind::SliceRange(a, b, c) => {
            walk(a, f);
            if let Some(b) = b {
                walk(b, f);
            }
            if let Some(c) = c {
                walk(c, f);
            }
        }
        ExprKind::StructLit(_, fs, _) => {
            for (_, x) in fs {
                walk(x, f);
            }
        }
        ExprKind::Tuple(xs) | ExprKind::ArrayLit(xs) => {
            for x in xs {
                walk(x, f);
            }
        }
        ExprKind::Block(b) | ExprKind::Unsafe(b) | ExprKind::Loop(b) => walk_block(b, f),
        ExprKind::If(cs, b, els) => {
            walk_conds(cs, f);
            walk_block(b, f);
            if let Some(x) = els {
                walk(x, f);
            }
        }
        ExprKind::Match(x, arms) => {
            walk(x, f);
            for a in arms {
                if let Some(g) = &mut a.guard {
                    walk(g, f);
                }
                walk(&mut a.body, f);
            }
        }
        ExprKind::While(cs, b) => {
            walk_conds(cs, f);
            walk_block(b, f);
        }
        ExprKind::For(_, _, a, b, body) => {
            walk(a, f);
            if let Some(b) = b {
                walk(b, f);
            }
            walk_block(body, f);
        }
        ExprKind::Return(x) | ExprKind::Break(x) => {
            if let Some(x) = x {
                walk(x, f);
            }
        }
        ExprKind::Closure(_, _, b) => walk(b, f),
        ExprKind::Vm(w) => match w {
            VmExpr::Jump(x) | VmExpr::Exit(x) | VmExpr::Raise(x) | VmExpr::Fail(x) => walk(x, f),
            VmExpr::Cold(b) => walk_block(b, f),
            VmExpr::Next | VmExpr::Redo | VmExpr::Become(_) | VmExpr::UseVariant(_) => {}
        },
    }
}

fn uses(body: &Block, name: &str) -> Use {
    let mut u = Use::default();
    let mut b = body.clone();
    walk_block(&mut b, &mut |e: &mut Expr| {
        match &mut e.kind {
            ExprKind::Assign(lhs, rhs) if matches!(&lhs.kind, ExprKind::Ident(n) if n == name) => {
                u.assigned = true;
                let mut r = (**rhs).clone();
                let ru = uses_expr(&mut r, name);
                u.read |= ru.read;
                u.assigned |= ru.assigned;
                false
            }
            ExprKind::Ident(n) if n == name => {
                u.read = true;
                true
            }
            _ => true,
        }
    });
    u
}

fn uses_expr(e: &mut Expr, name: &str) -> Use {
    let b = Block { stmts: vec![Stmt::Expr { expr: e.clone(), semi: false }], span: e.span };
    uses(&b, name)
}

/// `name = e` becomes `acc = e as T`.
fn redirect(body: &mut Block, name: &str, acc: &str, ty: &TypeExpr) {
    walk_block(body, &mut |e: &mut Expr| {
        if let ExprKind::Assign(lhs, rhs) = &mut e.kind {
            if matches!(&lhs.kind, ExprKind::Ident(n) if n == name) {
                let sp = lhs.span;
                **lhs = ident_expr(acc, sp);
                let r = std::mem::replace(&mut **rhs, Expr { kind: ExprKind::Unit, span: sp });
                let rs = r.span;
                **rhs = Expr { kind: ExprKind::Cast(Box::new(r), ty.clone()), span: rs };
            }
        }
        true
    });
}

pub fn desugar(v: &mut VmDecl) -> Res<()> {
    for n in std::mem::take(&mut v.acc_list) {
        match v.ops.iter_mut().find(|o| o.name.name == n.name) {
            Some(o) if o.family.is_none() => o.accumulate = true,
            Some(_) => return Err(Diag::new(n.span, format!("`{}` is a family member: `accumulate` takes plain ops", n.name))),
            None => return Err(Diag::new(n.span, format!("unknown op `{}`", n.name))),
        }
    }
    if v.accs.is_empty() {
        if let Some(o) = v.ops.iter().find(|o| o.accumulate) {
            return Err(Diag::new(o.name.span, "`#[accumulate]` needs an `accumulator` declaration in the vm"));
        }
        return Ok(());
    }
    // Accumulators: per kind, (accumulator, the kind's value type).
    let mut kind_acc: Vec<(String, usize, TypeExpr)> = Vec::new();
    for (ai, a) in v.accs.iter().enumerate() {
        if v.state.iter().chain(v.context.iter()).any(|p| p.name.name == a.name.name) {
            return Err(Diag::new(a.name.span, format!("`{}` is already a state variable", a.name.name)));
        }
        let Some((afloat, _)) = prim(&a.ty) else {
            return Err(Diag::new(a.ty.span, "an accumulator holds a number: an integer type (u8 .. u64, i8 .. i64), f32 or f64"));
        };
        for k in &a.kinds {
            let Some(decl) = v.operands.iter().find(|o| o.name.name == k.name) else {
                return Err(Diag::new(k.span, format!("unknown operand kind `{}`", k.name)));
            };
            let Some(t) = kind_type(v, decl) else {
                return Err(Diag::new(
                    k.span,
                    format!("operand kind `{}` is not a place of a known type: an accumulator's kinds read `*(.. as *T)` or `s[it]` (`s: *T` a state variable)", k.name),
                ));
            };
            match prim(&t) {
                Some((f, _)) if f == afloat => {}
                _ => {
                    return Err(Diag::new(
                        k.span,
                        format!(
                            "operand kind `{}` holds a value that accumulator `{}` cannot: both must be integers or both floats",
                            k.name, a.name.name
                        ),
                    ))
                }
            }
            if kind_acc.iter().any(|(n, _, _)| *n == k.name) {
                return Err(Diag::new(k.span, format!("operand kind `{}` belongs to two accumulators", k.name)));
            }
            kind_acc.push((k.name.clone(), ai, t));
        }
    }
    // The registers, after the declared state; every entry zeroes them.
    for a in &v.accs {
        v.state.push(Param { name: a.name.clone(), ty: a.ty.clone() });
    }
    for e in &mut v.entries {
        let sp = e.body.span;
        let mut pre = Vec::new();
        for a in &v.accs {
            let zero = Expr {
                kind: ExprKind::Cast(Box::new(Expr { kind: ExprKind::Int(0), span: sp }), a.ty.clone()),
                span: sp,
            };
            let asg = Expr { kind: ExprKind::Assign(Box::new(ident_expr(&a.name.name, sp)), Box::new(zero)), span: sp };
            pre.push(Stmt::Expr { expr: asg, semi: true });
        }
        pre.append(&mut e.body.stmts);
        e.body.stmts = pre;
    }
    let sp0 = v.name.span;
    v.operands.push(OperandDecl {
        name: Ident { name: ACC_KIND.to_string(), span: sp0 },
        body: ident_expr("it", sp0),
        verify: None,
        konst: None,
        display: None,
    });
    let mut new_ops = Vec::new();
    let mut infos = Vec::new();
    for op in &v.ops {
        if !op.accumulate {
            continue;
        }
        // Operands of accumulator kinds: the one the body only assigns is
        // the result; the ones it never assigns are sources.
        let mut dsts = Vec::new();
        let mut srcs = Vec::new();
        for (i, p) in op.params.iter().enumerate() {
            let Some((_, ai, _)) = kind_acc.iter().find(|(k, _, _)| *k == p.kind.name) else { continue };
            let u = uses(&op.body, &p.name.name);
            if u.assigned && !u.read {
                dsts.push((i, *ai));
            } else if !u.assigned {
                srcs.push((i, *ai));
            }
        }
        let dst = if dsts.len() == 1 { Some(dsts[0]) } else { None };
        if dst.is_none() && srcs.is_empty() {
            // (An `inline for` may mark ops of other kinds too.)
            continue;
        }
        let accd: Vec<usize> = op.params.iter().enumerate().filter(|(i, _)| dst.map(|d| d.0) == Some(*i) || srcs.iter().any(|s| s.0 == *i)).map(|(i, _)| i).collect();
        let mut variants = Vec::new();
        for dacc in [false, true] {
            if dacc && dst.is_none() {
                continue;
            }
            for s in 0..=srcs.len() {
                if !dacc && s == 0 {
                    continue;
                }
                let in_acc = |i: usize| (dacc && dst.map(|d| d.0) == Some(i)) || (s > 0 && srcs[s - 1].0 == i);
                let letters: String = accd.iter().map(|&i| if in_acc(i) { 'A' } else { 'R' }).collect();
                let name = format!("{}__{}", op.name.name, letters);
                let mut nop = op.clone();
                nop.name = Ident { name: name.clone(), span: op.name.span };
                nop.accumulate = false;
                let mut pre = Vec::new();
                for (i, p) in op.params.iter().enumerate() {
                    if !in_acc(i) {
                        continue;
                    }
                    nop.params[i].kind = Ident { name: ACC_KIND.to_string(), span: p.kind.span };
                    nop.params[i].name = Ident { name: format!("{}__acc", p.name.name), span: p.name.span };
                    let (_, ai, t) = kind_acc.iter().find(|(k, _, _)| *k == p.kind.name).unwrap();
                    let a = &v.accs[*ai];
                    if dst.map(|d| d.0) == Some(i) {
                        redirect(&mut nop.body, &p.name.name, &a.name.name, &a.ty);
                    } else {
                        let sp = p.name.span;
                        let read = Expr { kind: ExprKind::Cast(Box::new(ident_expr(&a.name.name, sp)), t.clone()), span: sp };
                        pre.push(Stmt::Let {
                            pat: Pat { kind: PatKind::Ident(p.name.name.clone(), false), span: sp },
                            mutable: false,
                            ty: None,
                            init: Some(read),
                            els: None,
                            span: sp,
                        });
                    }
                }
                pre.append(&mut nop.body.stmts);
                nop.body.stmts = pre;
                variants.push((dacc, s, name));
                new_ops.push(nop);
            }
        }
        infos.push(AccOp { op: op.name.name.clone(), dst, srcs, variants });
    }
    if infos.is_empty() {
        let sp = v.ops.iter().find(|o| o.accumulate).map_or(v.accs[0].name.span, |o| o.name.span);
        return Err(Diag::new(sp, "no `#[accumulate]` op has an operand of an accumulator's kind (a result it only assigns, or a source it never assigns)"));
    }
    v.ops.append(&mut new_ops);
    v.acc_ops = infos;
    Ok(())
}
