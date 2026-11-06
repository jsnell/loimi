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
pub fn kind_type(v: &VmDecl, k: &OperandDecl) -> Option<TypeExpr> {
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
        ExprKind::Field(a, _)
        | ExprKind::Unary(_, a)
        | ExprKind::Cast(a, _)
        | ExprKind::Is(a, _)
        | ExprKind::AsVariant(a, _)
        | ExprKind::Try(a)
        | ExprKind::TypeArgs(a, _) => walk(a, f),
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
        ExprKind::Closure(_, _, b, st) => {
            st.iter_mut().filter_map(|(_, _, x)| x.as_mut()).for_each(|x| walk(x, f));
            walk(b, f)
        }
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


fn named_ty(name: &str, span: Span) -> TypeExpr {
    TypeExpr { kind: TypeKind::Named(vec![Ident { name: name.to_string(), span }], vec![]), span }
}

/// The name of a one-segment type without arguments (`f64`, `u64`).
fn ty_name(t: &TypeExpr) -> Option<&str> {
    match &t.kind {
        TypeKind::Named(p, a) if p.len() == 1 && a.is_empty() => Some(p[0].name.as_str()),
        _ => None,
    }
}

fn method_call(recv: Expr, name: &str, args: Vec<Expr>) -> Expr {
    let sp = recv.span;
    let callee = Expr { kind: ExprKind::Field(Box::new(recv), Ident { name: name.to_string(), span: sp }), span: sp };
    Expr { kind: ExprKind::Call(Box::new(callee), args), span: sp }
}

/// What a value accumulator holds for the word `e` (of a `value` type):
/// its bits, as `f64.from_bits(e as u64)` or `e as u64`.
fn acc_of(a: &AccDecl, e: Expr) -> Expr {
    let sp = e.span;
    let bits = Expr { kind: ExprKind::Cast(Box::new(e), named_ty("u64", sp)), span: sp };
    if ty_name(&a.ty) == Some("f64") {
        method_call(ident_expr("f64", sp), "from_bits", vec![bits])
    } else {
        bits
    }
}

/// The value a value accumulator holds, as the kind's type `t`.
fn value_of(a: &AccDecl, t: &TypeExpr, sp: Span) -> Expr {
    let mut src = ident_expr(&a.name.name, sp);
    if ty_name(&a.ty) == Some("f64") {
        src = method_call(src, "to_bits", vec![]);
    }
    Expr { kind: ExprKind::Cast(Box::new(src), t.clone()), span: sp }
}

fn assign_stmt(lhs: &str, rhs: Expr) -> Stmt {
    let sp = rhs.span;
    Stmt::Expr { expr: Expr { kind: ExprKind::Assign(Box::new(ident_expr(lhs, sp)), Box::new(rhs)), span: sp }, semi: true }
}

/// `acc = slot`: a value accumulator takes the value of the result slot.
fn store_stmt(a: &AccDecl, dst: &str, sp: Span) -> Stmt {
    assign_stmt(&a.name.name, acc_of(a, ident_expr(dst, sp)))
}

/// `acc = 0`: the word 0, which holds no reference (checked by `vmcheck`).
fn zero_stmt(a: &AccDecl, sp: Span) -> Stmt {
    assign_stmt(&a.name.name, Expr { kind: ExprKind::Cast(Box::new(Expr { kind: ExprKind::Int(0), span: sp }), a.ty.clone()), span: sp })
}

/// Whether `jump t` goes where `Vm.accumulate` never puts a reader: a
/// label operand's target (`target`, `then.target`), or the instruction
/// itself (`jump pc`, as `redo`).
fn known_target(t: &Expr, labels: &[String]) -> bool {
    match &t.kind {
        ExprKind::Ident(n) => n == "pc" || labels.iter().any(|l| l == n),
        ExprKind::Field(b, _) => matches!(&b.kind, ExprKind::Ident(n) if n == "then"),
        _ => false,
    }
}

/// `next` (if `nexts`) and `jump` to a target that is not known become
/// `{ pre; next }`. Returns false for a node it replaced.
fn guard_one(x: &mut Expr, nexts: bool, labels: &[String], pre: &dyn Fn(Span) -> Vec<Stmt>) -> bool {
    let hit = match &x.kind {
        ExprKind::Vm(VmExpr::Next) => nexts,
        ExprKind::Vm(VmExpr::Jump(t)) => !known_target(t, labels),
        _ => false,
    };
    if !hit {
        return true;
    }
    let sp = x.span;
    let inner = std::mem::replace(x, Expr { kind: ExprKind::Unit, span: sp });
    let mut stmts = pre(sp);
    stmts.push(Stmt::Expr { expr: inner, semi: false });
    *x = Expr { kind: ExprKind::Block(Block { stmts, span: sp }), span: sp };
    false
}

fn walk_stmt(s: &mut Stmt, f: &mut dyn FnMut(&mut Expr) -> bool) {
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

/// Whether `b` has a `become` of an op `is_w` accepts.
fn has_become(b: &Block, is_w: &dyn Fn(&str) -> bool) -> bool {
    let mut found = false;
    let mut b = b.clone();
    walk_block(&mut b, &mut |x: &mut Expr| {
        if let ExprKind::Vm(VmExpr::Become(n)) = &x.kind {
            found |= is_w(&n.name);
        }
        true
    });
    found
}

/// Applies `g` to every statement that may run after a `become` of an op
/// `is_w` accepts, in source order (a loop that has one: all of it).
/// Returns whether `b` has such a `become`.
fn after_become_block(b: &mut Block, is_w: &dyn Fn(&str) -> bool, g: &dyn Fn(&mut Stmt)) -> bool {
    let mut seen = false;
    for s in &mut b.stmts {
        if seen {
            g(s);
            continue;
        }
        let found = match s {
            Stmt::Let { init, els, .. } => {
                let mut f = false;
                if let Some(e) = init {
                    f |= after_become_expr(e, is_w, g);
                }
                if let Some(b) = els {
                    f |= after_become_block(b, is_w, g);
                }
                f
            }
            Stmt::Expr { expr, .. } => after_become_expr(expr, is_w, g),
            Stmt::Defer(e) => after_become_expr(e, is_w, g),
        };
        seen |= found;
    }
    seen
}

fn after_become_expr(e: &mut Expr, is_w: &dyn Fn(&str) -> bool, g: &dyn Fn(&mut Stmt)) -> bool {
    let mut found = false;
    walk(e, &mut |x: &mut Expr| match &mut x.kind {
        ExprKind::Vm(VmExpr::Become(n)) => {
            found |= is_w(&n.name);
            false
        }
        ExprKind::Block(b) | ExprKind::Unsafe(b) | ExprKind::Vm(VmExpr::Cold(b)) => {
            found |= after_become_block(b, is_w, g);
            false
        }
        ExprKind::If(_, b, els) => {
            found |= after_become_block(b, is_w, g);
            if let Some(x) = els {
                found |= after_become_expr(x, is_w, g);
            }
            false
        }
        ExprKind::Loop(b) | ExprKind::While(_, b) | ExprKind::For(_, _, _, _, b) => {
            if has_become(b, is_w) {
                found = true;
                b.stmts.iter_mut().for_each(g);
            }
            false
        }
        _ => true,
    });
    found
}

/// Whether a block has `next` or `jump` (a vm function's shared tail).
fn has_exit(b: &Block) -> bool {
    let mut found = false;
    let mut b = b.clone();
    walk_block(&mut b, &mut |x: &mut Expr| {
        found |= matches!(&x.kind, ExprKind::Vm(VmExpr::Next) | ExprKind::Vm(VmExpr::Jump(_)));
        true
    });
    found
}

/// The first of the vm functions `names` that a body calls.
fn called(b: &Block, names: &[String]) -> Option<(String, Span)> {
    let mut found = None;
    let mut b = b.clone();
    walk_block(&mut b, &mut |x: &mut Expr| {
        if let ExprKind::Call(c, _) = &x.kind {
            if let ExprKind::Ident(n) = &c.kind {
                if found.is_none() && names.iter().any(|m| m == n) {
                    found = Some((n.clone(), x.span));
                }
            }
        }
        true
    });
    found
}

/// Every `become X` in a variant's body becomes `become X__L`, the variant
/// of X with the same letters, so that quickening keeps a word what
/// `Vm.accumulate` made it (a reader stays a reader). A reader of a value
/// accumulator (`wt`) may also become X itself, if X has no such variant:
/// the slot it reads is current (`vmcheck` allows the `__acc` field).
fn rename_becomes(b: &mut Block, letters: &str, variants: &[String], vname: &str, wt: bool) -> Res<()> {
    let mut err = None;
    walk_block(b, &mut |x: &mut Expr| {
        if let ExprKind::Vm(VmExpr::Become(n)) = &mut x.kind {
            let nn = format!("{}__{}", n.name, letters);
            if variants.iter().any(|v| *v == nn) {
                n.name = nn;
            } else if wt {
            } else if err.is_none() {
                err = Some(Diag::new(
                    n.span,
                    format!(
                        "`become {}` in accumulator variant `{}`: `{}` has no variant `{}` (name it in `accumulate`)",
                        n.name, vname, n.name, nn
                    ),
                ));
            }
        }
        true
    });
    err.map_or(Ok(()), Err)
}

pub fn desugar(v: &mut VmDecl) -> Res<()> {
    for n in std::mem::take(&mut v.acc_list) {
        match v.ops.iter_mut().find(|o| o.name.name == n.name) {
            // A family member's variants are members of its family, so
            // that `become` and `Vm.deopt` treat them as members.
            Some(o) => o.accumulate = true,
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
    let mut wts = Vec::new();
    for (ai, a) in v.accs.iter().enumerate() {
        if v.state.iter().chain(v.context.iter()).any(|p| p.name.name == a.name.name) {
            return Err(Diag::new(a.name.span, format!("`{}` is already a state variable", a.name.name)));
        }
        let Some((afloat, _)) = prim(&a.ty) else {
            return Err(Diag::new(a.ty.span, "an accumulator holds a number: an integer type (u8 .. u64, i8 .. i64), f32 or f64"));
        };
        let mut wt: Option<bool> = None;
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
            let value = match prim(&t) {
                Some((f, _)) if f == afloat => false,
                // A `value` type: `vmcheck` checks that it is a 64-bit
                // value whose word 0 holds no reference.
                None if matches!(ty_name(&a.ty), Some("f64" | "u64")) => true,
                None => {
                    return Err(Diag::new(
                        a.ty.span,
                        format!("operand kind `{}` holds a `value`: its accumulator holds the value's word, as `f64` (or `u64`)", k.name),
                    ))
                }
                _ => {
                    return Err(Diag::new(
                        k.span,
                        format!(
                            "operand kind `{}` holds a value that accumulator `{}` cannot: both must be integers or both floats",
                            k.name, a.name.name
                        ),
                    ))
                }
            };
            if wt.is_some_and(|w| w != value) {
                return Err(Diag::new(k.span, format!("accumulator `{}` has kinds of numbers and of values: one accumulator holds either", a.name.name)));
            }
            wt = Some(value);
            if kind_acc.iter().any(|(n, _, _)| *n == k.name) {
                return Err(Diag::new(k.span, format!("operand kind `{}` belongs to two accumulators", k.name)));
            }
            kind_acc.push((k.name.clone(), ai, t));
        }
        wts.push(wt == Some(true));
    }
    for (a, wt) in v.accs.iter_mut().zip(wts) {
        a.wt = wt;
    }
    let any_wt = v.accs.iter().any(|a| a.wt);
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
    // Per op: its index, result and sources, the operands its variants'
    // letters stand for, and the variants as (result in the accumulator,
    // source + 1, name).
    struct Plan {
        oi: usize,
        dst: Option<(usize, usize)>,
        srcs: Vec<(usize, usize)>,
        accd: Vec<usize>,
        variants: Vec<(bool, usize, String)>,
    }
    let mut plans = Vec::new();
    for (oi, op) in v.ops.iter().enumerate() {
        if !op.accumulate {
            continue;
        }
        // Operands of accumulator kinds: the one the body only assigns is
        // the result (the first of several: an op that also writes a
        // second slot, like a load that writes its address back, keeps
        // that write); the ones it never assigns are sources.
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
        let dst = dsts.first().copied();
        if dst.is_none() && srcs.is_empty() {
            // (An `inline for` may mark ops of other kinds too.)
            continue;
        }
        let accd: Vec<usize> = op.params.iter().enumerate().filter(|(i, _)| dst.map(|d| d.0) == Some(*i) || srcs.iter().any(|s| s.0 == *i)).map(|(i, _)| i).collect();
        // A value accumulator is written through: every form of the op
        // writes it (and the slot), so no variant puts the result in it.
        let dst_wt = dst.is_some_and(|d| v.accs[d.1].wt);
        let mut variants = Vec::new();
        for dacc in [false, true] {
            if dacc && (dst.is_none() || dst_wt) {
                continue;
            }
            for s in 0..=srcs.len() {
                if !dacc && s == 0 {
                    continue;
                }
                let in_acc = |i: usize| (dacc && dst.map(|d| d.0) == Some(i)) || (s > 0 && srcs[s - 1].0 == i);
                let letters: String = accd.iter().map(|&i| if in_acc(i) { 'A' } else { 'R' }).collect();
                variants.push((dacc, s, format!("{}__{}", op.name.name, letters)));
            }
        }
        plans.push(Plan { oi, dst, srcs, accd, variants });
    }
    if plans.is_empty() {
        let sp = v.ops.iter().find(|o| o.accumulate).map_or(v.accs[0].name.span, |o| o.name.span);
        return Err(Diag::new(sp, "no `#[accumulate]` op has an operand of an accumulator's kind (a result it only assigns, or a source it never assigns)"));
    }
    let all_variants: Vec<String> = plans.iter().flat_map(|p| p.variants.iter().map(|x| x.2.clone())).collect();
    let mut new_ops = Vec::new();
    for p in &plans {
        let op = &v.ops[p.oi];
        let dst = p.dst;
        for (dacc, s, name) in &p.variants {
            let (dacc, s) = (*dacc, *s);
            let in_acc = |i: usize| (dacc && dst.map(|d| d.0) == Some(i)) || (s > 0 && p.srcs[s - 1].0 == i);
            let letters: String = p.accd.iter().map(|&i| if in_acc(i) { 'A' } else { 'R' }).collect();
            let mut nop = op.clone();
            nop.name = Ident { name: name.clone(), span: op.name.span };
            nop.accumulate = false;
            let mut pre = Vec::new();
            let mut wt = false;
            for (i, pa) in op.params.iter().enumerate() {
                if !in_acc(i) {
                    continue;
                }
                nop.params[i].kind = Ident { name: ACC_KIND.to_string(), span: pa.kind.span };
                nop.params[i].name = Ident { name: format!("{}__acc", pa.name.name), span: pa.name.span };
                let (_, ai, t) = kind_acc.iter().find(|(k, _, _)| *k == pa.kind.name).unwrap();
                let a = &v.accs[*ai];
                wt |= a.wt;
                if dst.map(|d| d.0) == Some(i) {
                    redirect(&mut nop.body, &pa.name.name, &a.name.name, &a.ty);
                } else {
                    let sp = pa.name.span;
                    let read = if a.wt {
                        value_of(a, t, sp)
                    } else {
                        Expr { kind: ExprKind::Cast(Box::new(ident_expr(&a.name.name, sp)), t.clone()), span: sp }
                    };
                    pre.push(Stmt::Let {
                        pat: Pat { kind: PatKind::Ident(pa.name.name.clone(), false), span: sp },
                        mutable: false,
                        ty: None,
                        init: Some(read),
                        els: None,
                        span: sp,
                    });
                }
            }
            if wt {
                // A reader of a value accumulator is a member of its op's
                // family (of the op itself, for a plain op), so that
                // `Vm.deopt` turns it back into the op with the rest.
                nop.family = Some(op.family.clone().unwrap_or_else(|| op.name.name.clone()));
            }
            rename_becomes(&mut nop.body, &letters, &all_variants, name, wt)?;
            pre.append(&mut nop.body.stmts);
            nop.body.stmts = pre;
            new_ops.push(nop);
        }
    }
    v.ops.append(&mut new_ops);
    if any_wt {
        let w: Vec<(usize, Option<(usize, usize)>, Vec<String>)> = plans.iter().map(|p| (p.oi, p.dst, p.variants.iter().map(|x| x.2.clone()).collect())).collect();
        write_through(v, &w)?;
    }
    v.acc_ops = plans.into_iter().map(|p| AccOp { op: v.ops[p.oi].name.name.clone(), dst: p.dst, srcs: p.srcs, variants: p.variants }).collect();
    Ok(())
}

/// Value accumulators (`language.md` §7.4, "Value accumulators"): the
/// rules that keep the accumulator equal, at every reader's dispatch, to
/// the slot the reader takes from it, whatever rewrites the code meets.
///
/// - A *writer* (an op whose result is of a value accumulator's kind, and
///   each of its variants) writes the accumulator from the result slot
///   before every `next` and every `jump` to a target that `Vm.accumulate`
///   does not know. LLVM forwards the value just stored, so this costs a
///   register move. A writer leaves the accumulator current on every
///   path, whichever of its forms runs. So `redo` re-runs a reader with
///   the accumulator its writer left (`redocheck` rejects a store to it
///   before a `redo`).
/// - A writer may only `become` a writer of the same accumulator, and a
///   variant's `become X` is to X's variant with the same letters, so
///   quickening keeps each word what selection made it.
/// - An op that is not a writer but `become`s one (a generic member that
///   quickens to a float member, then calls `Vm.accumulate` from the
///   handler) writes the accumulator before every `next` that may follow
///   the `become`: the next word may have just become a reader.
/// - Elsewhere a `jump` to a target that is not known (a return address,
///   an exception handler) zeroes the accumulator. A reader reached that
///   way, which selection rules out, reads the word 0, not a stale
///   reference.
fn write_through(v: &mut VmDecl, plans: &[(usize, Option<(usize, usize)>, Vec<String>)]) -> Res<()> {
    // The writers of value accumulators: (name, result operand, accumulator).
    let mut w: Vec<(String, usize, usize)> = Vec::new();
    for (oi, dst, variants) in plans {
        let Some((di, ai)) = *dst else { continue };
        if !v.accs[ai].wt {
            continue;
        }
        w.push((v.ops[*oi].name.name.clone(), di, ai));
        for n in variants {
            w.push((n.clone(), di, ai));
        }
    }
    // The vm functions that may leave the op (`next`, `jump`), directly or
    // through one they call.
    let mut exits: Vec<String> = v.fns.iter().filter(|f| f.body.as_ref().is_some_and(has_exit)).map(|f| f.name.name.clone()).collect();
    loop {
        let more: Vec<String> = v
            .fns
            .iter()
            .filter(|f| !exits.contains(&f.name.name) && f.body.as_ref().is_some_and(|b| called(b, &exits).is_some()))
            .map(|f| f.name.name.clone())
            .collect();
        if more.is_empty() {
            break;
        }
        exits.extend(more);
    }
    let wts: Vec<usize> = (0..v.accs.len()).filter(|&i| v.accs[i].wt).collect();
    let accs = v.accs.clone();
    let zero = |sp: Span| wts.iter().map(|&i| zero_stmt(&accs[i], sp)).collect::<Vec<_>>();
    let label_kinds: Vec<String> = v.operands.iter().filter(|o| is_label(o)).map(|o| o.name.name.clone()).collect();
    let is_w = |n: &str| w.iter().any(|y| y.0 == n);
    for op in &mut v.ops {
        let labels: Vec<String> = op.params.iter().filter(|p| label_kinds.contains(&p.kind.name)).map(|p| p.name.name.clone()).collect();
        if let Some((_, di, ai)) = w.iter().find(|x| x.0 == op.name.name).cloned() {
            let mut err = None;
            walk_block(&mut op.body.clone(), &mut |x: &mut Expr| {
                if let ExprKind::Vm(VmExpr::Become(n)) = &x.kind {
                    if err.is_none() && !w.iter().any(|y| y.0 == n.name && y.1 == di && y.2 == ai) {
                        err = Some(Diag::new(
                            n.span,
                            format!("`{}` writes accumulator `{}` and may only `become` an op that does too: name `{}` in `accumulate`", op.name.name, accs[ai].name.name, n.name),
                        ));
                    }
                }
                true
            });
            if let Some(e) = err {
                return Err(e);
            }
            if let Some((f, sp)) = called(&op.body, &exits) {
                return Err(Diag::new(
                    sp,
                    format!(
                        "`{}` writes accumulator `{}`: it cannot leave through vm function `{}` (which has `next` or `jump`); leave in the op",
                        op.name.name, accs[ai].name.name, f
                    ),
                ));
            }
            let (a, d) = (&accs[ai], op.params[di].name.name.clone());
            walk_block(&mut op.body, &mut |x: &mut Expr| guard_one(x, true, &labels, &|sp| vec![store_stmt(a, &d, sp)]));
            continue;
        }
        walk_block(&mut op.body, &mut |x: &mut Expr| guard_one(x, false, &labels, &zero));
        // A `become` of a writer (which has this op's operand kinds).
        let mut target = None;
        walk_block(&mut op.body.clone(), &mut |x: &mut Expr| {
            if let ExprKind::Vm(VmExpr::Become(n)) = &x.kind {
                if let Some(y) = w.iter().find(|y| y.0 == n.name) {
                    target.get_or_insert((y.1, y.2));
                }
            }
            true
        });
        if let Some((di, ai)) = target {
            let (a, d) = (&accs[ai], op.params[di].name.name.clone());
            let g = |s: &mut Stmt| walk_stmt(s, &mut |x: &mut Expr| guard_one(x, true, &labels, &|sp| vec![store_stmt(a, &d, sp)]));
            after_become_block(&mut op.body, &is_w, &g);
        }
    }
    for f in &mut v.fns {
        if let Some(b) = &mut f.body {
            walk_block(b, &mut |x: &mut Expr| guard_one(x, false, &[], &zero));
        }
    }
    if let Some(r) = &mut v.on_raise {
        walk_block(&mut r.body, &mut |x: &mut Expr| guard_one(x, false, &[], &zero));
    }
    Ok(())
}

/// `operand label = pc + it`.
fn is_label(k: &OperandDecl) -> bool {
    let is = |e: &Expr, n: &str| matches!(&e.kind, ExprKind::Ident(x) if x == n);
    match &k.body.kind {
        ExprKind::Binary(BinOp::Add, a, b) => (is(a, "pc") && is(b, "it")) || (is(a, "it") && is(b, "pc")),
        _ => false,
    }
}
