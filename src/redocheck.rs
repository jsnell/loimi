//! `redo` safety, and `new` in handlers lowered to the refill form
//! (docs/lowering.md section 10, "Allocation in handlers, polls and
//! `redo`").
//!
//! `redo` dispatches the current instruction again, from the start of
//! its handler (or of its part, in a superinstruction). That is right
//! only if nothing the op did before the `redo` is seen by the second
//! run: no store to memory (an operand, an object, the context), to a
//! state register, or call of a function that may store. This pass runs
//! on the typed program after checking, and follows every path of each
//! op handler to each `redo` (and each `jump` to the op's own `pc`, which
//! is the same thing), with the summaries of the functions called.
//!
//! What a path may do before a `redo` without making it unsafe:
//! - write locals, read anything;
//! - allocate (`new`, `try_new`): an object nothing refers to yet is
//!   garbage after the `redo`;
//! - call into `std.gc` and `std.gil` (collect, `make_room`, hand over
//!   the lock): these change only the runtime's bookkeeping, which is
//!   what a safepoint is for;
//! - rewrite its own instruction (`become`, stores to its inline cache)
//!   and switch the dispatch table (`use_variant`): `redo` exists to run
//!   the rewritten instruction.
//!
//! The same analysis lowers `new` in an op handler: where the path to it
//! is free of such effects, not inside a loop of the op and not in a
//! `cold` block, `new(h, T { .. })` becomes `try_new` with a cold
//! `make_room` of the object's size and `redo`, the form handlers wrote
//! by hand to keep the allocator's refill call (and the stack frame it
//! brings) off the fast path.

use crate::check::mk;
use crate::ast::BinOp;
use crate::diag::{Diag, Span};
use crate::tir::*;

/// Why a function may store to memory the caller can see: the first
/// store found, or call of one that may.
type Why = Option<(Span, String)>;

/// The largest object (without its header) that `std.gc.try_alloc`
/// allocates inline (`HEADER` 16, `NCLASS` 33 size classes of 16 bytes).
const MAX_SMALL: u64 = 496;

/// Functions whose calls are not side effects for `redo`: the runtime's
/// collector and lock.
fn runtime_fn(f: &FnDef) -> bool {
    f.symbol.starts_with("std.gc.") || f.symbol.starts_with("std.gil.")
}

/// Per function: whether (and why) it may store to memory it did not
/// allocate, directly or through what it calls. A least fixpoint over
/// the call graph; a callee without a body (extern) or called through a
/// value may store.
fn summaries(p: &Program) -> Vec<Why> {
    let n = p.fns.len();
    let mut why: Vec<Why> = vec![None; n];
    let mut calls: Vec<Vec<(FnId, Span)>> = vec![Vec::new(); n];
    for (i, f) in p.fns.iter().enumerate() {
        if runtime_fn(f) {
            continue;
        }
        let Some(b) = &f.body else {
            // An extern: one that takes no pointer can reach no memory
            // of the program (libm's `sin`, ..).
            if f.param_tys.iter().any(|t| !matches!(t, Ty::Int(_) | Ty::F32 | Ty::F64 | Ty::Bool)) {
                why[i] = Some((f.span, format!("`{}` is external and takes a pointer", f.name)));
            }
            continue;
        };
        let mut first: Why = None;
        crate::tirwalk::for_each_expr(b, &mut |e| {
            if first.is_some() {
                return;
            }
            match &e.kind {
                TK::Assign(l, _) => {
                    if let Some(w) = place_store(f, l) {
                        first = Some((e.span, w));
                    }
                }
                TK::Intrinsic(k, _) if store_intrinsic(*k) => first = Some((e.span, "an atomic or unaligned store".into())),
                TK::Call(c, _) => match c.kind {
                    TK::FnRef(id) => calls[i].push((id, e.span)),
                    _ => first = Some((e.span, "a call through a function value".into())),
                },
                _ => {}
            }
        });
        why[i] = first;
    }
    loop {
        let mut changed = false;
        for i in 0..n {
            if why[i].is_some() {
                continue;
            }
            for &(c, sp) in &calls[i] {
                if why[c as usize].is_some() && !runtime_fn(&p.fns[c as usize]) {
                    why[i] = Some((sp, format!("a call of `{}`, which may store to memory", p.fns[c as usize].name)));
                    changed = true;
                    break;
                }
            }
        }
        if !changed {
            break;
        }
    }
    why
}

fn store_intrinsic(k: Intr) -> bool {
    matches!(k, Intr::StoreUnaligned | Intr::AtomicStore | Intr::AtomicSwap | Intr::AtomicAdd | Intr::AtomicCas)
}

/// What a store to place `l` writes, if it is more than a local of the
/// function: None for a local (or a field or element of one).
fn place_store(f: &FnDef, l: &TExpr) -> Option<String> {
    match &l.kind {
        TK::Local(id) => {
            let name = &f.locals[*id as usize].name;
            f.vm_state.contains(id).then(|| format!("a store to the state register `{}`", name.trim_start_matches("state.")))
        }
        TK::Field(b, _) if !matches!(b.ty, Ty::Ptr(_) | Ty::Gc(_)) => place_store(f, b),
        TK::Index(b, _) if matches!(b.ty, Ty::Array(..)) => place_store(f, b),
        TK::Vm(TVm::Word(_) | TVm::WordAt(..)) => None,
        _ => Some("a store to memory".into()),
    }
}

/// The analysis state at a point of a handler: unreachable, or reached
/// with no effect yet, or after the effect at `Span`.
#[derive(Clone, PartialEq, Debug)]
enum St {
    Dead,
    Clean,
    Dirty(Span, String),
}

fn join(a: St, b: St) -> St {
    match (a, b) {
        (St::Dead, x) | (x, St::Dead) => x,
        (d @ St::Dirty(..), _) | (_, d @ St::Dirty(..)) => d,
        _ => St::Clean,
    }
}

struct Cx<'a> {
    p: &'a Program,
    why: &'a [Why],
    /// The function being walked (the handler, or a shared copy it
    /// reaches), its `pc` local and the table register's local.
    f: &'a FnDef,
    pc: LocalId,
    tbl: Option<LocalId>,
    op: String,
    /// Inside a `cold` block; inside loops of the op; inside a loop
    /// pass whose results are not final (no errors, no rewrites).
    cold: bool,
    loops: Vec<(u32, St, St)>,
    probe: u32,
    /// Shared copies being walked (recursion guard).
    shared: Vec<FnId>,
    /// Rewrite `new` (only in the handler itself).
    lower: bool,
    make_room: Option<FnId>,
    /// Locals added by the lowering (the binding of a `match`).
    new_locals: Vec<LocalDef>,
    errs: Vec<Diag>,
    lowered: u32,
}

impl Cx<'_> {
    fn redo(&mut self, st: &St, span: Span) {
        if self.probe > 0 {
            return;
        }
        if let St::Dirty(at, what) = st {
            let d = Diag::warning(
                "redo_effect",
                span,
                format!("`redo` runs `{}` again from its start, but on the way here the op may already have made {}, and the second run would make it again", self.op, what),
            )
            .note(
                *at,
                "this runs before the `redo`: put the poll or the allocation before it, or end the path with `next` or `jump`; if running it twice is harmless, `#[allow(redo_effect)]` on the op says so",
            );
            if !self.errs.iter().any(|e| e.span == d.span && e.notes[0].0 == d.notes[0].0) {
                self.errs.push(d);
            }
        }
    }

    fn walk(&mut self, e: &mut TExpr, st: St) -> St {
        if st == St::Dead {
            return St::Dead;
        }
        let lower = st == St::Clean && self.can_lower(e);
        let out = self.walk_inner(e, st);
        if lower {
            self.lower_expr(e);
        }
        if e.ty == Ty::Never {
            St::Dead
        } else {
            out
        }
    }

    fn walk_all(&mut self, es: &mut [TExpr], mut st: St) -> St {
        for x in es {
            st = self.walk(x, st);
        }
        st
    }

    fn walk_inner(&mut self, e: &mut TExpr, st: St) -> St {
        let span = e.span;
        match &mut e.kind {
            TK::Local(_) | TK::Int(_) | TK::Float(_) | TK::Bool(_) | TK::Str(_) | TK::Unit | TK::Zeroed | TK::FnRef(_) => st,
            TK::Global(_) | TK::Closure(..) => st,
            TK::Unreachable => St::Dead,
            TK::Continue(id) => {
                let id = *id;
                if let Some(l) = self.loops.iter_mut().rev().find(|l| l.0 == id) {
                    l.2 = join(l.2.clone(), st);
                }
                St::Dead
            }
            TK::Break(id, v) => {
                let id = *id;
                let st = match v {
                    Some(x) => self.walk(x, st),
                    None => st,
                };
                if let Some(l) = self.loops.iter_mut().rev().find(|l| l.0 == id) {
                    l.1 = join(l.1.clone(), st);
                }
                St::Dead
            }
            TK::Return(v) => {
                if let Some(x) = v {
                    self.walk(x, st);
                }
                St::Dead
            }
            TK::Deref(a) | TK::Field(a, _) | TK::AddrOf(a) | TK::Unary(_, a) | TK::Cast(a) | TK::ArrayRepeat(a, _) => self.walk(a, st),
            TK::Try(a) | TK::Trailing(a) | TK::NewClosure(_, _, a) => self.walk(a, st),
            TK::Index(a, b) | TK::Binary(_, a, b) => {
                let st = self.walk(a, st);
                self.walk(b, st)
            }
            TK::Coalesce(a, b) => {
                let st = self.walk(a, st);
                let s2 = self.walk(b, st.clone());
                join(st, s2)
            }
            TK::Assign(l, r) => {
                let st = self.walk(r, st);
                let st = self.place(l, st);
                if st == St::Dead {
                    return st;
                }
                // `use_variant`: the table register, switched to run the
                // instruction again in another variant.
                if let (TK::Local(id), TK::Vm(TVm::TableAddr(_))) = (&l.kind, &r.kind) {
                    if Some(*id) == self.tbl {
                        return st;
                    }
                }
                match place_store(self.f, l) {
                    Some(w) if st == St::Clean => St::Dirty(span, w),
                    _ => st,
                }
            }
            TK::SliceRange(a, lo, hi) => {
                let mut st = self.walk(a, st);
                for x in [lo, hi].into_iter().flatten() {
                    st = self.walk(x, st);
                }
                st
            }
            TK::Call(c, args) => {
                let st = self.walk(c, st);
                let st = self.walk_all(args, st);
                if st != St::Clean {
                    return st;
                }
                match c.kind {
                    TK::FnRef(id) => match &self.why[id as usize] {
                        Some(_) => St::Dirty(span, format!("a call of `{}`, which may store to memory", self.p.fns[id as usize].name)),
                        None => st,
                    },
                    _ => St::Dirty(span, "a call through a function value, which may store to memory".into()),
                }
            }
            TK::Intrinsic(k, args) => {
                let k = *k;
                let st = self.walk_all(args, st);
                if store_intrinsic(k) && st == St::Clean {
                    St::Dirty(span, "an atomic or unaligned store".into())
                } else {
                    st
                }
            }
            TK::Struct(args) | TK::Tuple(args) | TK::Array(args) | TK::Variant(_, args) => self.walk_all(args, st),
            TK::Block(stmts, tail) => {
                // A block that ends in `redo` is the op's fix-up before
                // it runs again (`gc.collect(..)`, `become X`, growing a
                // stack): what it does is the reason for the `redo`. What
                // the op did before entering it is what must not repeat.
                let redo_at = match (tail.as_deref(), stmts.last()) {
                    (Some(t), _) => self.is_redo(t).then_some(t.span),
                    (None, Some(TStmt::Expr(t))) => self.is_redo(t).then_some(t.span),
                    _ => None,
                };
                let entry = st.clone();
                let mut st = st;
                let mut i = 0;
                let n = if redo_at.is_some() && tail.is_none() { stmts.len() - 1 } else { stmts.len() };
                while i < n {
                    st = match &mut stmts[i] {
                        TStmt::Let(_, None) => st,
                        TStmt::Let(_, Some(x)) if st == St::Clean && self.can_lower(x) => {
                            // As the hand-written form: `let Some(x) =
                            // try_new(..) else cold { .. }`.
                            let s2 = self.walk_inner(x, st);
                            self.lower_let(&mut stmts[i]);
                            s2
                        }
                        TStmt::Let(_, Some(x)) => self.walk(x, st),
                        TStmt::LetPat(_, x, els) => {
                            let s1 = self.walk(x, st);
                            if let Some(e) = els {
                                self.walk(e, s1.clone());
                            }
                            s1
                        }
                        TStmt::Expr(x) | TStmt::Defer(x) => self.walk(x, st),
                    };
                    i += 1;
                }
                if let Some(sp) = redo_at {
                    self.redo(&entry, sp);
                    return St::Dead;
                }
                match tail {
                    Some(t) => self.walk(t, st),
                    None => st,
                }
            }
            TK::If(conds, then, els) => {
                let mut st = st;
                for c in conds.iter_mut() {
                    st = match c {
                        TCond::Expr(x) | TCond::Let(_, x) => self.walk(x, st),
                    };
                }
                let a = self.walk(then, st.clone());
                let b = match els {
                    Some(x) => self.walk(x, st),
                    None => st,
                };
                join(a, b)
            }
            TK::Match(s, arms) => {
                let st = self.walk(s, st);
                let mut out = St::Dead;
                for a in arms.iter_mut() {
                    let mut s = st.clone();
                    if let Some(g) = &mut a.guard {
                        s = self.walk(g, s);
                    }
                    out = join(out, self.walk(&mut a.body, s));
                }
                out
            }
            TK::Loop { body, step, id } => {
                let id = *id;
                // Iterate to the fixpoint without reporting or rewriting,
                // then walk once more from the state at the loop head.
                let mut head = st;
                self.probe += 1;
                for _ in 0..4 {
                    let after = self.loop_pass(id, body, step, head.clone());
                    let next = join(head.clone(), after.1);
                    if next == head {
                        break;
                    }
                    head = next;
                }
                self.probe -= 1;
                self.loop_pass(id, body, step, head).0
            }
            TK::New { heap, init, len, fill, .. } => {
                let mut st = self.walk(heap, st);
                st = self.walk(init, st);
                for x in [len, fill].into_iter().flatten() {
                    st = self.walk(x, st);
                }
                st
            }
            TK::Vm(v) => match v {
                TVm::Jump(t) => {
                    // (A `redo` that ends a block is checked there.)
                    let st = self.walk(t, st);
                    if matches!(t.kind, TK::Local(l) if l == self.pc) {
                        self.redo(&st, span);
                    }
                    St::Dead
                }
                TVm::Exit(a) | TVm::Raise(a) => {
                    self.walk(a, st);
                    St::Dead
                }
                TVm::Cold(a) => {
                    let saved = std::mem::replace(&mut self.cold, true);
                    let st = self.walk(a, st);
                    self.cold = saved;
                    st
                }
                TVm::Part(_, a) => {
                    // A part runs from its own word: `redo` there runs
                    // the word alone.
                    self.walk(a, St::Clean);
                    St::Dead
                }
                TVm::WordAt(a, _, _) => self.walk(a, st),
                TVm::SharedNext(t, _, _) => {
                    self.walk(t, st);
                    St::Dead
                }
                TVm::Synced(_, call, _) => self.walk(call, st),
                TVm::Shared(fid, args, size, _) => {
                    let fid = *fid;
                    let mut st = self.walk_all(args, st);
                    st = self.walk(size, st);
                    self.shared_copy(fid, st);
                    St::Dead
                }
                TVm::Become(..) | TVm::Word(_) | TVm::RaiseSlot | TVm::TableAddr(_) | TVm::SharedArg(..) => st,
                TVm::Fallthrough(..) | TVm::Chain(_) => St::Dead,
            },
        }
    }

    fn is_redo(&self, e: &TExpr) -> bool {
        matches!(&e.kind, TK::Vm(TVm::Jump(t)) if matches!(t.kind, TK::Local(l) if l == self.pc))
    }

    /// One pass over a loop's body (and step) from state `head`: the
    /// state after the loop (its breaks) and at the next iteration.
    fn loop_pass(&mut self, id: u32, body: &mut TExpr, step: &mut Option<B>, head: St) -> (St, St) {
        self.loops.push((id, St::Dead, St::Dead));
        let end = self.walk(body, head);
        let (_, brk, cont) = self.loops.pop().unwrap();
        let mut again = join(end, cont);
        if let Some(s) = step {
            again = self.walk(s, again);
        }
        (brk, again)
    }

    /// The `-> never` vm function's shared copy, reached in the state `st`
    /// (its `redo`s are checked as part of this op).
    fn shared_copy(&mut self, fid: FnId, st: St) {
        if self.shared.contains(&fid) {
            return;
        }
        let p = self.p;
        let g = &p.fns[fid as usize];
        let Some(body) = &g.body else { return };
        let Some(vm) = g.vm else { return };
        let mut body = body.clone();
        let saved = (self.f, self.pc, self.tbl, self.lower);
        let vd = &p.vms[vm as usize];
        self.f = g;
        self.pc = g.vm_state[vd.pc_idx];
        self.tbl = vd.tbl_idx.map(|i| g.vm_state[i]);
        self.lower = false;
        self.shared.push(fid);
        self.walk(&mut body, st);
        self.shared.pop();
        (self.f, self.pc, self.tbl, self.lower) = saved;
    }

    /// Whether `x` is a `new` to lower: in the handler itself, on the
    /// hot path, outside loops, of a heap the refill can evaluate again,
    /// and of a constant size the allocator's inline path covers (a
    /// large object, or a trailing array of a length not constant, is a
    /// call in `try_new` too: the handler keeps its frame, and the plain
    /// `new` does not run the op twice on a refill).
    fn can_lower(&self, x: &TExpr) -> bool {
        let TK::New { heap, len, fast: false, uninit: false, .. } = &x.kind else { return false };
        if !(self.lower && !self.cold && self.loops.is_empty() && self.probe == 0 && self.make_room.is_some() && pure(heap)) {
            return false;
        }
        let Ty::Gc(inner) = &x.ty else { return false };
        let Ty::Struct(sid) = **inner else { return false };
        let mut size = crate::layout::size_of(self.p, inner);
        if let (Some(l), Some((_, et))) = (len, &self.p.structs[sid as usize].trailing) {
            let n = match &l.kind {
                TK::Int(n) => *n,
                TK::Cast(a) => match a.kind {
                    TK::Int(n) => n,
                    _ => return false,
                },
                _ => return false,
            };
            if !(0..=MAX_SMALL as i128).contains(&n) {
                return false;
            }
            size += n as u64 * crate::layout::size_of(self.p, et);
        }
        size <= MAX_SMALL
    }

    /// The `make_room` call and `redo` that refill for the `new` `x`,
    /// as the `else` of its `try_new`.
    fn refill(&self, x: &TExpr) -> TExpr {
        let TK::New { heap, len, .. } = &x.kind else { unreachable!() };
        let span = x.span;
        let Ty::Gc(inner) = &x.ty else { unreachable!() };
        let p = self.p;
        let Ty::Struct(sid) = **inner else { unreachable!() };
        let size = crate::layout::size_of(p, inner) as i128;
        let u64t = Ty::Int(IntTy::U64);
        let bytes = match (len, &p.structs[sid as usize].trailing) {
            (Some(l), Some((_, et))) => {
                let es = crate::layout::size_of(p, et) as i128;
                let n = mk(u64t.clone(), TK::Binary(BinOp::Mul, l.clone(), Box::new(mk(u64t.clone(), TK::Int(es), span))), span);
                mk(u64t.clone(), TK::Binary(BinOp::Add, Box::new(mk(u64t.clone(), TK::Int(size), span)), Box::new(n)), span)
            }
            _ => mk(u64t.clone(), TK::Int(size), span),
        };
        let mr = self.make_room.unwrap();
        let callee = mk(p.fns[mr as usize].fn_ty(), TK::FnRef(mr), span);
        let call = mk(Ty::Unit, TK::Call(Box::new(callee), vec![(**heap).clone(), bytes]), span);
        let pc_ty = self.f.locals[self.pc as usize].ty.clone();
        let redo = mk(Ty::Never, TK::Vm(TVm::Jump(Box::new(mk(pc_ty, TK::Local(self.pc), span)))), span);
        let cold_body = mk(Ty::Never, TK::Block(vec![TStmt::Expr(call)], Some(Box::new(redo))), span);
        mk(Ty::Never, TK::Vm(TVm::Cold(Box::new(cold_body))), span)
    }

    /// `x`, a `new`, as a `try_new` (of type `?Gc<T>`).
    fn fast(x: &mut TExpr) {
        if let TK::New { fast, .. } = &mut x.kind {
            *fast = true;
        }
        x.ty = Ty::Option(Box::new(x.ty.clone()));
    }

    /// A `new` in an expression becomes
    /// `match try_new(..) { Some(t) => t, _ => cold { gc.make_room(h, size); redo } }`.
    fn lower_expr(&mut self, e: &mut TExpr) {
        let cold = self.refill(e);
        let (span, gt) = (e.span, e.ty.clone());
        let t = (self.f.locals.len() + self.new_locals.len()) as LocalId;
        self.new_locals.push(LocalDef { name: "new".into(), ty: gt.clone(), mutable: false, field: None });
        let mut x = std::mem::replace(e, mk(Ty::Unit, TK::Unit, span));
        Self::fast(&mut x);
        let opt = x.ty.clone();
        let arms = vec![
            TArm { pat: TPat { ty: opt.clone(), kind: TPK::Variant(1, vec![TPat { ty: gt.clone(), kind: TPK::Bind(t) }]) }, guard: None, body: mk(gt.clone(), TK::Local(t), span) },
            TArm { pat: TPat { ty: opt, kind: TPK::Wild }, guard: None, body: cold },
        ];
        *e = mk(gt, TK::Match(Box::new(x), arms), span);
        self.lowered += 1;
    }

    /// `let x = new(h, T { .. }, n);` becomes
    /// `let Some(x) = try_new(h, T { .. }, n) else cold { gc.make_room(h, size); redo };`.
    fn lower_let(&mut self, s: &mut TStmt) {
        let TStmt::Let(id, Some(x)) = s else { return };
        let id = *id;
        let els = mk(Ty::Never, TK::Block(vec![], Some(Box::new(self.refill(x)))), x.span);
        let gt = x.ty.clone();
        let TStmt::Let(_, Some(mut x)) = std::mem::replace(s, TStmt::Let(id, None)) else { unreachable!() };
        Self::fast(&mut x);
        let pat = TPat { ty: x.ty.clone(), kind: TPK::Variant(1, vec![TPat { ty: gt, kind: TPK::Bind(id) }]) };
        *s = TStmt::LetPat(pat, x, Some(els));
        self.lowered += 1;
    }

    /// The place `l` of an assignment: its subexpressions (an index, the
    /// pointer stored through) are evaluated, not the place itself.
    fn place(&mut self, l: &mut TExpr, st: St) -> St {
        match &mut l.kind {
            TK::Local(_) | TK::Global(_) => st,
            TK::Field(b, _) => self.place(b, st),
            TK::Index(b, i) => {
                let st = self.place(b, st);
                self.walk(i, st)
            }
            _ => self.walk(l, st),
        }
    }
}

/// An expression that can be evaluated again with the same result and
/// no effect: locals, constants, fields and addresses of them.
fn pure(e: &TExpr) -> bool {
    match &e.kind {
        TK::Local(_) | TK::Int(_) | TK::Global(_) => true,
        TK::Field(a, _) | TK::AddrOf(a) | TK::Deref(a) | TK::Cast(a) => pure(a),
        _ => false,
    }
}

/// Check every `redo` of every op handler, and lower the `new`s that
/// can be. `LOIMI_REDO_AUDIT=1` lists the lowered `new`s per handler.
pub fn run(p: &mut Program) -> Result<(), Vec<Diag>> {
    if p.vms.is_empty() {
        return Ok(());
    }
    let why = summaries(p);
    let make_room = p.fns.iter().position(|f| f.symbol == "std.gc.make_room" && f.body.is_some()).map(|i| i as FnId);
    let audit = std::env::var_os("LOIMI_REDO_AUDIT").is_some();
    let mut errs = Vec::new();
    for i in 0..p.fns.len() {
        let f = &p.fns[i];
        let (Some(vm), VmRole::Handler(_) | VmRole::Resume(..)) = (f.vm, f.vm_role) else { continue };
        let Some(body) = &f.body else { continue };
        let vd = &p.vms[vm as usize];
        let op = match f.vm_role {
            VmRole::Handler(k) | VmRole::Resume(k, _) => vd.ops[k as usize].name.clone(),
            _ => unreachable!(),
        };
        let mut body = body.clone();
        let mut cx = Cx {
            p,
            why: &why,
            f,
            pc: f.vm_state[vd.pc_idx],
            tbl: vd.tbl_idx.map(|t| f.vm_state[t]),
            op,
            cold: false,
            loops: Vec::new(),
            probe: 0,
            shared: Vec::new(),
            lower: true,
            make_room,
            new_locals: Vec::new(),
            errs: Vec::new(),
            lowered: 0,
        };
        cx.walk(&mut body, St::Clean);
        let (lowered, op, new_locals) = (cx.lowered, cx.op.clone(), std::mem::take(&mut cx.new_locals));
        for d in cx.errs {
            if !errs.iter().any(|e: &Diag| e.span == d.span && e.notes[0].0 == d.notes[0].0) {
                errs.push(d);
            }
        }
        if lowered > 0 {
            if audit {
                eprintln!("redo-audit: {}: {} `new` lowered to `try_new` + `make_room`", op, lowered);
            }
            p.fns[i].body = Some(body);
            p.fns[i].locals.extend(new_locals);
        }
    }
    if errs.is_empty() {
        Ok(())
    } else {
        Err(errs)
    }
}
