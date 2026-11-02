//! Traversals over the typed IR.

use crate::tir::*;

fn pat_binds(p: &TPat, declared: &mut Vec<LocalId>) {
    match &p.kind {
        TPK::Bind(l) => declared.push(*l),
        TPK::Variant(_, ps) | TPK::Tuple(ps) | TPK::Or(ps) => ps.iter().for_each(|p| pat_binds(p, declared)),
        _ => {}
    }
}

fn push(v: &mut Vec<LocalId>, l: LocalId) {
    if !v.contains(&l) {
        v.push(l);
    }
}

/// Locals read or written in `e` (`used`, without repeats, in order of
/// first use), and locals declared inside it.
pub fn locals(e: &TExpr, used: &mut Vec<LocalId>, declared: &mut Vec<LocalId>) {
    walk(e, &mut |ev| match ev {
        Event::Leave(x) => match &x.kind {
            TK::Local(l) => push(used, *l),
            TK::Closure(_, caps) | TK::NewClosure(_, caps, _) => caps.iter().for_each(|l| push(used, *l)),
            TK::Vm(TVm::SharedNext(_, _, cont)) => push(used, *cont),
            _ => {}
        },
        Event::Let(l, _) => declared.push(l),
        Event::Pat(p) => pat_binds(p, declared),
        Event::Enter(_) => {}
    });
}

fn diverges(stmts: &[TStmt], tail: &Option<B>) -> bool {
    match tail {
        Some(t) => t.ty == Ty::Never,
        None => stmts.iter().any(|s| matches!(s, TStmt::Expr(e) if e.ty == Ty::Never)),
    }
}

/// `if c { cold { .. } } rest`: when the cold block falls through but the
/// rest of its enclosing block ends in a control word, copy the rest into
/// the cold block. The cold block then never rejoins the hot path and can
/// be outlined. Not if the rest leaves a loop (`break`, `continue`): an
/// outlined block has no loop to leave.
pub fn extend_cold(e: &mut TExpr) {
    if let TK::Block(stmts, tail) = &mut e.kind {
        for i in 0..stmts.len() {
            let rest_never = diverges(&stmts[i + 1..], tail) && !leaves_loop(&stmts[i + 1..], tail);
            if rest_never {
                let rest_stmts: Vec<TStmt> = stmts[i + 1..].to_vec();
                let rest_tail = tail.clone();
                if let TStmt::Expr(x) = &mut stmts[i] {
                    if let TK::If(_, then, els) = &mut x.kind {
                        for branch in [Some(then), els.as_mut()].into_iter().flatten() {
                            append_rest(branch, &rest_stmts, &rest_tail);
                        }
                    }
                }
            }
        }
    }
    // Recurse.
    match &mut e.kind {
        TK::Block(stmts, tail) => {
            for s in stmts.iter_mut() {
                match s {
                    TStmt::Let(_, Some(x)) | TStmt::Expr(x) | TStmt::Defer(x) => extend_cold(x),
                    TStmt::LetPat(_, x, els) => {
                        extend_cold(x);
                        if let Some(e) = els {
                            extend_cold(e);
                        }
                    }
                    _ => {}
                }
            }
            if let Some(t) = tail {
                extend_cold(t);
            }
        }
        TK::If(_, then, els) => {
            extend_cold(then);
            if let Some(x) = els {
                extend_cold(x);
            }
        }
        TK::Match(_, arms) => arms.iter_mut().for_each(|a| extend_cold(&mut a.body)),
        TK::Loop { body, .. } => extend_cold(body),
        TK::Vm(TVm::Cold(b)) | TK::Vm(TVm::Part(_, b)) => extend_cold(b),
        _ => {}
    }
}

/// Whether statements `stmts` and `tail` contain a `break` or `continue`.
fn leaves_loop(stmts: &[TStmt], tail: &Option<B>) -> bool {
    let mut found = false;
    let mut look = |x: &TExpr| {
        for_each_expr(x, &mut |e| {
            if matches!(e.kind, TK::Break(..) | TK::Continue(_)) {
                found = true;
            }
        })
    };
    for s in stmts {
        match s {
            TStmt::Let(_, Some(x)) | TStmt::Expr(x) | TStmt::Defer(x) => look(x),
            TStmt::LetPat(_, x, els) => {
                look(x);
                if let Some(e) = els {
                    look(e);
                }
            }
            _ => {}
        }
    }
    if let Some(t) = tail {
        look(t);
    }
    found
}

/// `branch` is `{ cold { b } }` with `b` falling through: make it `{ cold { b; rest } }`.
fn append_rest(branch: &mut TExpr, rest: &[TStmt], tail: &Option<B>) {
    let TK::Block(stmts, Some(t)) = &mut branch.kind else { return };
    if !stmts.is_empty() {
        return;
    }
    let TK::Vm(TVm::Cold(b)) = &mut t.kind else { return };
    if b.ty == Ty::Never {
        return;
    }
    let span = b.span;
    let old = std::mem::replace(&mut **b, TExpr { ty: Ty::Unit, kind: TK::Unit, span });
    let mut new_stmts = vec![TStmt::Expr(old)];
    new_stmts.extend(rest.iter().cloned());
    **b = TExpr { ty: Ty::Never, kind: TK::Block(new_stmts, tail.clone()), span };
    t.ty = Ty::Never;
    branch.ty = Ty::Never;
}

/// A binding site found by `for_each_bind`.
pub enum BindSite<'a> {
    /// A top-level pattern (match arm, `if let`, destructuring `let`).
    Pat(&'a TPat),
    /// `let x = init;`
    Let(LocalId, &'a TExpr),
}

/// Call `f` on every binding site in `e`, at any depth of nesting.
pub fn for_each_bind<'a>(e: &'a TExpr, f: &mut dyn FnMut(BindSite<'a>)) {
    walk(e, &mut |ev| match ev {
        Event::Let(l, Some(x)) => f(BindSite::Let(l, x)),
        Event::Pat(p) => f(BindSite::Pat(p)),
        _ => {}
    });
}

/// Call `f` on every expression in `e`, outermost first.
pub fn for_each_expr<'a>(e: &'a TExpr, f: &mut dyn FnMut(&'a TExpr)) {
    walk(e, &mut |ev| {
        if let Event::Enter(x) = ev {
            f(x)
        }
    });
}

/// What `walk` reports, in evaluation order.
enum Event<'a> {
    /// An expression, before its subexpressions.
    Enter(&'a TExpr),
    /// An expression, after its subexpressions.
    Leave(&'a TExpr),
    /// `let l = init;` (or `let l;`), after `init`.
    Let(LocalId, Option<&'a TExpr>),
    /// A pattern's bindings: of a destructuring `let` after its value and
    /// `else`, of an `if let` after its value, of a match arm before its
    /// guard.
    Pat(&'a TPat),
}

/// The one traversal of the typed IR the others are built on.
fn walk<'a>(e: &'a TExpr, f: &mut dyn FnMut(Event<'a>)) {
    f(Event::Enter(e));
    let go = |x: &'a TExpr, f: &mut dyn FnMut(Event<'a>)| walk(x, f);
    match &e.kind {
        TK::Local(_) | TK::Int(_) | TK::Float(_) | TK::Bool(_) | TK::Str(_) | TK::Unit | TK::Zeroed | TK::FnRef(_) => {}
        TK::Global(_) | TK::Continue(_) | TK::Unreachable | TK::Closure(..) => {}
        TK::Deref(a) | TK::Field(a, _) | TK::AddrOf(a) | TK::Unary(_, a) | TK::Cast(a) | TK::ArrayRepeat(a, _) => go(a, f),
        TK::Try(a) | TK::Trailing(a) | TK::NewClosure(_, _, a) => go(a, f),
        TK::Index(a, b) | TK::Binary(_, a, b) | TK::Assign(a, b) | TK::Coalesce(a, b) => {
            go(a, f);
            go(b, f);
        }
        TK::SliceRange(a, lo, hi) => {
            go(a, f);
            for x in [lo, hi].into_iter().flatten() {
                go(x, f);
            }
        }
        TK::Call(c, args) => {
            go(c, f);
            args.iter().for_each(|a| go(a, f));
        }
        TK::Intrinsic(_, args) | TK::Struct(args) | TK::Tuple(args) | TK::Array(args) | TK::Variant(_, args) => {
            args.iter().for_each(|a| go(a, f))
        }
        TK::Block(stmts, tail) => {
            for s in stmts {
                match s {
                    TStmt::Let(l, init) => {
                        if let Some(x) = init {
                            go(x, f);
                        }
                        f(Event::Let(*l, init.as_ref()));
                    }
                    TStmt::LetPat(p, x, els) => {
                        go(x, f);
                        if let Some(e) = els {
                            go(e, f);
                        }
                        f(Event::Pat(p));
                    }
                    TStmt::Expr(x) | TStmt::Defer(x) => go(x, f),
                }
            }
            if let Some(t) = tail {
                go(t, f);
            }
        }
        TK::If(conds, then, els) => {
            for c in conds {
                match c {
                    TCond::Expr(x) => go(x, f),
                    TCond::Let(p, x) => {
                        go(x, f);
                        f(Event::Pat(p));
                    }
                }
            }
            go(then, f);
            if let Some(x) = els {
                go(x, f);
            }
        }
        TK::Match(s, arms) => {
            go(s, f);
            for a in arms {
                f(Event::Pat(&a.pat));
                if let Some(g) = &a.guard {
                    go(g, f);
                }
                go(&a.body, f);
            }
        }
        TK::Loop { body, step, .. } => {
            go(body, f);
            if let Some(s) = step {
                go(s, f);
            }
        }
        TK::Break(_, v) | TK::Return(v) => {
            if let Some(x) = v {
                go(x, f);
            }
        }
        TK::New { heap, init, len } => {
            go(heap, f);
            go(init, f);
            if let Some(l) = len {
                go(l, f);
            }
        }
        TK::Vm(v) => match v {
            TVm::Jump(a) | TVm::Exit(a) | TVm::Raise(a) | TVm::Cold(a) | TVm::Part(_, a) | TVm::WordAt(a, _, _) => go(a, f),
            TVm::SharedNext(t, _, _) => go(t, f),
            TVm::Synced(a, b, c) => {
                go(a, f);
                go(b, f);
                go(c, f);
            }
            TVm::Shared(_, args, size, cont) => {
                args.iter().for_each(|a| go(a, f));
                go(size, f);
                if let Some(c) = cont {
                    go(c, f);
                }
            }
            TVm::Become(..) | TVm::Word(_) | TVm::Fallthrough(..) | TVm::RaiseSlot | TVm::TableAddr(_) | TVm::Chain(_) => {}
            TVm::SharedArg(..) => {}
        },
    }
    f(Event::Leave(e));
}
