//! Errors and deoptimization are cold by default (language.md §4,
//! "Errors and deoptimization are cold by default").
//!
//! In an op handler or a `-> never` vm function, a block that ends in
//! `raise e` or `fail e`, or in `redo` after a `become`:
//! - if it is a branch (the `then` or `else` block of an `if`, the body
//!   of a `match` arm, the `else` block of a `let .. else`) and nothing
//!   in it leaves it before, is made a `cold` block as a whole;
//! - else, if a statement before can leave the block (`leaves`), the
//!   statements after the last such one are.
//!
//! The tree is rewritten to the one `cold { .. }` written there would
//! give, so everything after (checking, cold outlining, the `redo`
//! check, lifetimes, `--vm-stats`) sees the hand-written form. A block
//! inside a `cold` block (already cold) or a closure is left alone, and
//! so is one whose control word is marked `#[hot]`.

use crate::ast::*;
use crate::diag::{Diag, Res, Span};
use crate::vmast::VmDecl;

/// One block made cold: where it ends (the control word) and why.
#[derive(Clone, Debug)]
pub struct Site {
    pub span: Span,
    pub item: String,
    pub what: &'static str,
}

thread_local! {
    /// The `-> never` vm functions of the vm being rewritten: a call to
    /// one leaves the block.
    static NEVER_FNS: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
}

struct Walk<'a> {
    hot: &'a [Span],
    used_hot: Vec<Span>,
    item: String,
    sites: Vec<Site>,
}

/// Rewrite the vm's op handlers and `-> never` vm functions. `hot` are
/// the control words marked `#[hot]`; each must be one that would have
/// been made cold.
pub fn run(vm: &mut VmDecl, hot: &[Span]) -> Res<Vec<Site>> {
    let never: Vec<String> = vm.fns.iter().filter(|f| f.ret.as_ref().is_some_and(|t| matches!(t.kind, TypeKind::Never))).map(|f| f.name.name.clone()).collect();
    NEVER_FNS.with(|n| *n.borrow_mut() = never);
    let mut w = Walk { hot, used_hot: Vec::new(), item: String::new(), sites: Vec::new() };
    for op in &mut vm.ops {
        w.item = format!("op {}", op.name.name);
        w.block(&mut op.body);
    }
    for f in &mut vm.fns {
        if !f.ret.as_ref().is_some_and(|t| matches!(t.kind, TypeKind::Never)) {
            continue;
        }
        w.item = format!("fn {}", f.name.name);
        if let Some(b) = &mut f.body {
            w.block(b);
        }
    }
    if let Some(s) = hot.iter().find(|s| !w.used_hot.contains(s)) {
        return Err(Diag::new(
            *s,
            "`#[hot]` does nothing here: it keeps a `raise`, `fail` or `become ..; redo` that an op or `-> never` vm function reaches on some paths only from being made cold (language.md §4); this one is not made cold",
        ));
    }
    Ok(w.sites)
}

impl Walk<'_> {
    /// The cold part of a block that ends in `raise`/`fail`, or in `redo`
    /// after a `become`: it starts after the last earlier statement that
    /// can leave the block (0 if none), and the statements from there on
    /// run only on the way to the control word. Also its span, what it
    /// is, and whether the control word is marked `#[hot]`.
    fn cold_tail(&self, b: &Block) -> Option<(usize, Span, &'static str, bool)> {
        let Some(Stmt::Expr { expr, .. }) = b.stmts.last() else { return None };
        let n = b.stmts.len();
        let start = b.stmts[..n - 1].iter().rposition(leaves).map_or(0, |k| k + 1);
        let what = match &expr.kind {
            ExprKind::Vm(VmExpr::Raise(_)) => "raise",
            ExprKind::Vm(VmExpr::Fail(_)) => "fail",
            ExprKind::Vm(VmExpr::Redo)
                if b.stmts[start..n - 1].iter().any(|s| matches!(s, Stmt::Expr { expr: Expr { kind: ExprKind::Vm(VmExpr::Become(_)), .. }, .. })) =>
            {
                "become; redo"
            }
            _ => return None,
        };
        Some((start, expr.span, what, self.hot.contains(&expr.span)))
    }

    /// Whether to make the block (from `start`) cold, noting the site; a
    /// `#[hot]` control word keeps it as it is.
    fn take(&mut self, span: Span, what: &'static str, hot: bool) -> bool {
        if hot {
            self.used_hot.push(span);
            return false;
        }
        self.sites.push(Site { span, item: self.item.clone(), what });
        true
    }

    /// The statements from `k` on become one `cold { .. }`, the block's
    /// last statement (as `..; cold { raise e }` is written).
    fn wrap_tail(&mut self, b: &mut Block, k: usize) {
        self.stmts(&mut b.stmts[..k]);
        let tail: Vec<Stmt> = b.stmts.drain(k..).collect();
        let span = stmt_span(&tail[0]).to(stmt_span(tail.last().unwrap()));
        b.stmts.push(Stmt::Expr { expr: Expr { kind: ExprKind::Vm(VmExpr::Cold(Block { stmts: tail, span })), span }, semi: false });
    }

    /// A branch block (an `if`'s, a `let .. else`'s): made cold whole (as
    /// `{ cold { .. } }`) if nothing in it leaves before the control word,
    /// else from the last statement that can.
    fn branch(&mut self, b: &mut Block) {
        match self.cold_tail(b) {
            Some((0, sp, what, hot)) if self.take(sp, what, hot) => {
                let span = b.span;
                let inner = std::mem::replace(b, Block { stmts: Vec::new(), span });
                b.stmts.push(Stmt::Expr { expr: Expr { kind: ExprKind::Vm(VmExpr::Cold(inner)), span }, semi: false });
            }
            Some((k, sp, what, hot)) if k > 0 && self.take(sp, what, hot) => self.wrap_tail(b, k),
            _ => self.block_all(b),
        }
    }

    /// A branch that is an expression (an `else`, a `match` arm): a
    /// block or a bare control word made cold whole becomes `cold { .. }`
    /// itself, as `else cold { .. }` and `=> cold { .. }` are written.
    fn branch_expr(&mut self, e: &mut Expr) {
        let span = e.span;
        let block = match &mut e.kind {
            ExprKind::Block(b) => match self.cold_tail(b) {
                Some((0, sp, what, hot)) if self.take(sp, what, hot) => std::mem::replace(b, Block { stmts: Vec::new(), span }),
                Some((k, sp, what, hot)) if k > 0 && self.take(sp, what, hot) => return self.wrap_tail(b, k),
                _ => return self.block_all(b),
            },
            ExprKind::Vm(VmExpr::Raise(_) | VmExpr::Fail(_)) => {
                let b = Block { stmts: vec![Stmt::Expr { expr: e.clone(), semi: false }], span };
                match self.cold_tail(&b) {
                    Some((_, sp, what, hot)) if self.take(sp, what, hot) => b,
                    _ => return self.expr(e),
                }
            }
            _ => return self.expr(e),
        };
        e.kind = ExprKind::Vm(VmExpr::Cold(block));
    }

    /// Any other block (an op's body, a loop's): its tail is made cold
    /// if a statement before it can leave the block.
    fn block(&mut self, b: &mut Block) {
        match self.cold_tail(b) {
            Some((k, sp, what, hot)) if k > 0 && self.take(sp, what, hot) => self.wrap_tail(b, k),
            _ => self.block_all(b),
        }
    }

    fn block_all(&mut self, b: &mut Block) {
        self.stmts(&mut b.stmts);
    }

    fn stmts(&mut self, ss: &mut [Stmt]) {
        for s in ss {
            match s {
                Stmt::Let { init, els, .. } => {
                    if let Some(e) = init {
                        self.expr(e);
                    }
                    if let Some(b) = els {
                        self.branch(b);
                    }
                }
                Stmt::Expr { expr, .. } | Stmt::Defer(expr) => self.expr(expr),
            }
        }
    }


    fn conds(&mut self, cs: &mut [Cond]) {
        for c in cs {
            match c {
                Cond::Expr(e) | Cond::Let(_, e) => self.expr(e),
            }
        }
    }

    fn expr(&mut self, e: &mut Expr) {
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
            | ExprKind::SizeOf(_)
            // Already cold, or another function.
            | ExprKind::Closure(..)
            | ExprKind::Vm(VmExpr::Cold(_)) => {}
            ExprKind::Assign(a, b)
            | ExprKind::CompoundAssign(_, a, b)
            | ExprKind::Index(a, b)
            | ExprKind::Binary(_, a, b)
            | ExprKind::Coalesce(a, b)
            | ExprKind::ArrayRepeat(a, b) => {
                self.expr(a);
                self.expr(b);
            }
            ExprKind::Field(a, _)
            | ExprKind::Unary(_, a)
            | ExprKind::Cast(a, _)
            | ExprKind::Try(a)
            | ExprKind::TypeArgs(a, _)
            | ExprKind::Is(a, _)
            | ExprKind::AsVariant(a, _) => self.expr(a),
            ExprKind::Call(f, args) => {
                self.expr(f);
                args.iter_mut().for_each(|a| self.expr(a));
            }
            ExprKind::SliceRange(a, lo, hi) => {
                self.expr(a);
                lo.iter_mut().chain(hi.iter_mut()).for_each(|x| self.expr(x));
            }
            ExprKind::StructLit(_, fs, _) => fs.iter_mut().for_each(|(_, x)| self.expr(x)),
            ExprKind::Tuple(xs) | ExprKind::ArrayLit(xs) => xs.iter_mut().for_each(|x| self.expr(x)),
            ExprKind::Block(b) | ExprKind::Unsafe(b) | ExprKind::Loop(b) => self.block(b),
            ExprKind::If(cs, t, f) => {
                self.conds(cs);
                self.branch(t);
                if let Some(f) = f {
                    if matches!(f.kind, ExprKind::If(..)) {
                        self.expr(f);
                    } else {
                        self.branch_expr(f);
                    }
                }
            }
            ExprKind::While(cs, b) => {
                self.conds(cs);
                self.block(b);
            }
            ExprKind::For(_, _, lo, hi, b) => {
                self.expr(lo);
                if let Some(h) = hi {
                    self.expr(h);
                }
                self.block(b);
            }
            ExprKind::Match(s, arms) => {
                self.expr(s);
                for a in arms {
                    if let Some(g) = &mut a.guard {
                        self.expr(g);
                    }
                    self.branch_expr(&mut a.body);
                }
            }
            ExprKind::Return(v) | ExprKind::Break(v) => {
                if let Some(v) = v {
                    self.expr(v);
                }
            }
            ExprKind::Vm(v) => match v {
                VmExpr::Jump(a) | VmExpr::Exit(a) | VmExpr::Raise(a) | VmExpr::Fail(a) => self.expr(a),
                VmExpr::Next | VmExpr::Redo | VmExpr::Become(_) | VmExpr::UseVariant(_) | VmExpr::Cold(_) => {}
            },
        }
    }
}

fn stmt_span(s: &Stmt) -> Span {
    match s {
        Stmt::Let { span, .. } => *span,
        Stmt::Expr { expr, .. } | Stmt::Defer(expr) => expr.span,
    }
}

/// Whether a statement can leave the block it is in other than on a
/// slow path: a control word, a call of a `-> never` vm function,
/// `return`, `break` or `continue` somewhere in it, outside `cold` blocks
/// and closures. A `?` does not count: its error path is not a branch
/// the author wrote (language.md §4).
fn leaves(s: &Stmt) -> bool {
    match s {
        Stmt::Let { init, els, .. } => init.as_ref().is_some_and(exits) || els.as_ref().is_some_and(|b| b.stmts.iter().any(leaves)),
        Stmt::Expr { expr, .. } | Stmt::Defer(expr) => exits(expr),
    }
}

fn exits(e: &Expr) -> bool {
    let blk = |b: &Block| b.stmts.iter().any(leaves);
    let conds = |cs: &Vec<Cond>| cs.iter().any(|c| match c {
        Cond::Expr(e) | Cond::Let(_, e) => exits(e),
    });
    match &e.kind {
        ExprKind::Int(_)
        | ExprKind::Float(_)
        | ExprKind::Str(_)
        | ExprKind::Char(_)
        | ExprKind::Bool(_)
        | ExprKind::Ident(_)
        | ExprKind::DotVariant(_)
        | ExprKind::Unit
        | ExprKind::SizeOf(_)
        | ExprKind::Closure(..) => false,
        ExprKind::Continue | ExprKind::Return(_) | ExprKind::Break(_) => true,
        ExprKind::Assign(a, b)
        | ExprKind::CompoundAssign(_, a, b)
        | ExprKind::Index(a, b)
        | ExprKind::Binary(_, a, b)
        | ExprKind::Coalesce(a, b)
        | ExprKind::ArrayRepeat(a, b) => exits(a) || exits(b),
        ExprKind::Field(a, _)
        | ExprKind::Unary(_, a)
        | ExprKind::Cast(a, _)
        | ExprKind::Try(a)
        | ExprKind::TypeArgs(a, _)
        | ExprKind::Is(a, _)
        | ExprKind::AsVariant(a, _) => exits(a),
        ExprKind::Call(f, args) => {
            matches!(&f.kind, ExprKind::Ident(n) if NEVER_FNS.with(|v| v.borrow().contains(n))) || exits(f) || args.iter().any(exits)
        }
        ExprKind::SliceRange(a, lo, hi) => exits(a) || lo.as_ref().is_some_and(|x| exits(x)) || hi.as_ref().is_some_and(|x| exits(x)),
        ExprKind::StructLit(_, fs, _) => fs.iter().any(|(_, x)| exits(x)),
        ExprKind::Tuple(xs) | ExprKind::ArrayLit(xs) => xs.iter().any(exits),
        ExprKind::Block(b) | ExprKind::Unsafe(b) | ExprKind::Loop(b) => blk(b),
        ExprKind::If(cs, t, f) => conds(cs) || blk(t) || f.as_ref().is_some_and(|x| exits(x)),
        ExprKind::While(cs, b) => conds(cs) || blk(b),
        ExprKind::For(_, _, lo, hi, b) => exits(lo) || hi.as_ref().is_some_and(|x| exits(x)) || blk(b),
        ExprKind::Match(s, arms) => exits(s) || arms.iter().any(|a| exits(&a.body) || a.guard.as_ref().is_some_and(exits)),
        ExprKind::Vm(v) => match v {
            // A control word in a `cold` block (a poll's `redo`, a slow
            // path's `next`) is not a way out of the fast path: what
            // follows the statement is still the op's own path.
            VmExpr::Cold(_) | VmExpr::Become(_) | VmExpr::UseVariant(_) => false,
            VmExpr::Next | VmExpr::Redo | VmExpr::Jump(_) | VmExpr::Exit(_) | VmExpr::Raise(_) | VmExpr::Fail(_) => true,
        },
    }
}
