//! References into GC objects that the write barrier cannot follow
//! (docs/design/safety.md section 8).
//!
//! The compiler emits the write barrier for a store of a traced value
//! whose place it can see is inside an object: `o.f = v`,
//! `o.items[i] = v`, `o.items[a..b][i] = v` (`gcgen::in_object`). A slice
//! of an object's traced trailing array kept in a local (`let s =
//! o.items; s[i] = v`) would let a store skip it, and a generational heap
//! would then free a young object that an old one still references. So
//! would the address of a traced place in an object (`let p = &o.f; *p =
//! v`), which in a `#![safe]` module is not a raw-pointer operation.
//!
//! An *object reference* is such a slice (`o.items`, `o.items[a..b]`,
//! `o.arr[..]`, elements of a traced type) or such an address (`&o.f`,
//! `&o.items[i]`). This pass runs on the typed program after checking
//! (so every function's effects are known) and allows one only where the
//! barrier still covers the stores through it:
//!
//! - as the base of an index, a field, a sub-slice, `.len`, or of a `for`
//!   loop; `o.items.ptr` is a raw pointer (stores through it need
//!   `gc.write_barrier`, as for any raw pointer);
//! - bound by `let` to a local that is then only read through (indexed,
//!   `.len`, fields read, passed where nothing stores through it);
//! - as a direct argument of a call. Each function's slice and pointer
//!   parameters of traced element types are summarized: does the function
//!   store through it (or let it escape, or pass it on to one that does)?
//!   A call that may store through an object slice gets the barrier on
//!   the object after it (`&o.f` arguments always did), and must not
//!   collect: a collection between the store and the barrier would miss
//!   the young object. A function that may keep the parameter (return
//!   it, store it, capture it) must not get one at all.
//!
//! The slice rule holds in every module (a slice is the checked type).
//! The address rule holds in `#![safe]` modules; elsewhere `&o.f` is a
//! raw pointer, and stores through it need `gc.write_barrier`, except
//! that an `&o.f` argument (implicit for a method's receiver) still must
//! not go to a call that may collect and store through it. Code in
//! `unsafe { }` is exempt.

use std::collections::{HashMap, HashSet};

use crate::diag::{Diag, Span};
use crate::gcgen::is_traced;
use crate::tir::*;

/// Whether `e` is a slice into an object's traced memory: the trailing
/// array (`o.items`) or a sub-slice of it or of an array field.
pub fn traced_obj_slice(p: &Program, e: &TExpr) -> bool {
    let Ty::Slice(et) = &e.ty else { return false };
    is_traced(p, et) && crate::gcgen::slice_in_object(e)
}

/// Whether `e` is the address of a traced place inside an object
/// (`&o.f`, `&o.items[i]`, `&o.pair` holding a `Value`).
pub fn traced_obj_addr(p: &Program, e: &TExpr) -> bool {
    match &e.kind {
        TK::AddrOf(inner) => is_traced(p, &inner.ty) && crate::gcgen::in_object(inner),
        _ => false,
    }
}

/// Whether a parameter of type `t` is summarized: a slice of or pointer
/// to a traced type.
fn tracked_ty(p: &Program, t: &Ty) -> bool {
    match t {
        Ty::Slice(et) | Ty::Ptr(et) => is_traced(p, et),
        _ => false,
    }
}

pub fn may_collect(p: &Program, f: &TExpr) -> bool {
    let (Ty::Fn(_, _, mut effects) | Ty::Block(_, _, mut effects) | Ty::Closure(_, _, mut effects)) = crate::check::callee_sig(&f.ty) else {
        return false;
    };
    if let TK::FnRef(id) = f.kind {
        effects |= p.fns[id as usize].sync_effects;
    }
    effects & EFF_GC != 0
}

/// Whether the call of `f` may store through argument `i`: always for a
/// callee the summary does not know (a function value, an extern).
pub fn call_writes(p: &Program, f: &TExpr, i: usize) -> bool {
    summary(p, &p.ref_writes, f, i)
}

/// Whether the call of `f` may keep argument `i` beyond the call (return
/// it, store it, capture it): always for an unknown callee.
pub fn call_keeps(p: &Program, f: &TExpr, i: usize) -> bool {
    summary(p, &p.ref_keeps, f, i)
}

fn summary(p: &Program, m: &HashMap<FnId, u64>, f: &TExpr, i: usize) -> bool {
    match f.kind {
        TK::FnRef(id) if p.fns[id as usize].body.is_some() => m.get(&id).is_some_and(|w| i < 64 && w & (1 << i) != 0),
        _ => true,
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Use {
    /// The value escapes (bound to a `var`, stored, returned, put in an
    /// aggregate, captured, compared).
    Value,
    /// Read through: an index, a field read, `.len`.
    Read,
    /// Stored through: the base of an assignment's place.
    Write,
    /// `.ptr`: a raw pointer.
    Raw,
    /// Argument `i` of the innermost call being walked.
    Arg(usize),
    /// The initializer of a `let` that tracks it.
    Alias,
}

#[derive(Clone, Copy)]
enum Origin {
    /// An object reference made in this function; `slice`: a slice (else
    /// an address).
    Direct(bool),
    /// Parameter `i`.
    Param(usize),
}

struct Fa<'a> {
    p: &'a Program,
    f: &'a FnDef,
    /// The callees of the calls whose arguments are being walked.
    callee: Vec<&'a TExpr>,
    tracked: HashMap<LocalId, Origin>,
    written: u64,
    kept: u64,
    /// Report errors (the last pass), else only summarize.
    report: bool,
    safe: bool,
    unsafe_spans: &'a [Span],
    audit: Option<&'a crate::diag::SourceMap>,
    errs: Vec<Diag>,
}

impl<'a> Fa<'a> {
    fn in_unsafe(&self, s: Span) -> bool {
        self.unsafe_spans.iter().any(|u| u.file == s.file && u.lo <= s.lo && s.hi <= u.hi)
    }

    fn error(&mut self, span: Span, slice: bool, collects: bool, what: &str) {
        if !self.report || self.in_unsafe(span) || (!slice && !self.safe && !collects) {
            return;
        }
        let thing = if slice { "a slice of an object's traced trailing array" } else { "the address of a traced place inside an object" };
        if let Some(sm) = self.audit {
            let (l, c) = sm.line_col(span.file, span.lo);
            eprintln!("objref-audit {}:{}:{} {}: {} ({})", sm.files[span.file as usize].path, l, c, thing, what, self.f.symbol);
            return;
        }
        let hint = if collects {
            "the compiler's write barrier after the call cannot cover stores made before a collection in it: pass the object itself, or make the callee only read (or not collect)"
        } else if slice {
            "store through the object (`o.items[i] = v` gets the write barrier); a slice of it may be bound by `let` and read, or passed to a function that cannot collect (the barrier follows the call)"
        } else {
            "store through the object (`o.f = v` gets the write barrier); `&o.f` may be bound by `let` and read, or passed to a function that cannot collect (the barrier follows the call)"
        };
        self.errs.push(Diag::new(span, format!("{} {}: a store through it would skip the write barrier", thing, what)).note(span, hint.to_string()));
    }

    /// (may store through, may keep, may collect) for argument `idx` of
    /// the innermost call.
    /// `LOIMI_OBJREF_AUDIT`: an object slice that may be read across a
    /// collection (it dangles if nothing else keeps its object alive: not
    /// an error, the lifetime hole of safety.md section 2).
    fn audit_lifetime(&self, span: Span, what: &str) {
        if let (Some(sm), true) = (self.audit, self.report) {
            let (l, c) = sm.line_col(span.file, span.lo);
            eprintln!("objref-lifetime {}:{}:{} {} ({})", sm.files[span.file as usize].path, l, c, what, self.f.symbol);
        }
    }

    fn arg_info(&self, idx: usize) -> (bool, bool, bool) {
        let f = *self.callee.last().unwrap();
        (call_writes(self.p, f, idx), call_keeps(self.p, f, idx), may_collect(self.p, f))
    }

    /// An object reference made at `e` and used as `u`.
    fn direct(&mut self, e: &TExpr, slice: bool, u: Use) {
        match u {
            Use::Read | Use::Alias | Use::Raw => {}
            // `o.items[i] = v`: the compiler's barrier.
            Use::Write if slice => {}
            Use::Write => self.error(e.span, slice, false, "is stored through"),
            Use::Value => self.error(e.span, slice, false, "is kept"),
            Use::Arg(i) => {
                let (writes, keeps, collects) = self.arg_info(i);
                if keeps {
                    self.error(e.span, slice, false, "is passed to a function that may keep it (return or store it)");
                } else if writes && collects {
                    self.error(e.span, slice, true, "is passed to a call that may collect and may store through it");
                } else if collects && slice {
                    self.audit_lifetime(e.span, "object slice passed to a call that may collect");
                }
            }
        }
    }

    /// A use of a tracked local.
    fn local(&mut self, o: Origin, u: Use, span: Span) {
        // Inside `unsafe { }` a parameter's other uses are raw (a cast to
        // read its bytes): trusted not to keep it, counted as storing.
        let u = if u == Use::Value && matches!(o, Origin::Param(_)) && self.in_unsafe(span) { Use::Raw } else { u };
        // (stores through it, keeps it, what to say)
        let (w, k, what) = match u {
            Use::Read | Use::Alias => return,
            Use::Raw => match o {
                Origin::Direct(..) => return,
                Origin::Param(_) => (true, false, ""),
            },
            Use::Write => (true, false, "is stored through (held in a local)"),
            Use::Value => (false, true, "is kept (held in a local)"),
            Use::Arg(i) => {
                let (writes, keeps, _) = self.arg_info(i);
                if !writes && !keeps {
                    return;
                }
                (writes, keeps, "is passed (held in a local) to a function that may store through it or keep it")
            }
        };
        match o {
            Origin::Param(i) => {
                if i < 64 {
                    if w {
                        self.written |= 1 << i;
                    }
                    if k {
                        self.kept |= 1 << i;
                    }
                }
            }
            Origin::Direct(slice) => self.error(span, slice, false, what),
        }
    }

    /// The origin of the memory that place `x` lies in, if it is reached
    /// through a tracked local (`s[i]`, `p.f`, `(*p).a[2]`).
    fn place_origin(&self, x: &TExpr) -> Option<Origin> {
        match &x.kind {
            TK::Index(b, _) => match &b.ty {
                Ty::Array(..) => self.place_origin(b),
                _ => self.origin(b),
            },
            TK::Field(b, _) if !matches!(b.ty, Ty::Slice(_)) => self.place_origin(b),
            TK::Deref(p) => self.origin(p),
            _ => None,
        }
    }

    /// The one origin of all the branches that do not diverge.
    fn join(&self, bs: &[&TExpr]) -> Option<Origin> {
        let mut r: Option<Origin> = None;
        for b in bs {
            if diverges(b) {
                continue;
            }
            let o = self.origin(b)?;
            r = match (r, o) {
                (None, o) => Some(o),
                (Some(Origin::Direct(x)), Origin::Direct(y)) if x == y => r,
                (Some(Origin::Param(x)), Origin::Param(y)) if x == y => r,
                _ => return None,
            };
        }
        r
    }

    /// Where the object reference (or tracked local) `x` comes from, if
    /// it is one.
    fn origin(&self, x: &TExpr) -> Option<Origin> {
        if traced_obj_slice(self.p, x) {
            return Some(Origin::Direct(true));
        }
        if traced_obj_addr(self.p, x) {
            return Some(Origin::Direct(false));
        }
        match &x.kind {
            TK::Local(l) => self.tracked.get(l).copied(),
            TK::SliceRange(b, _, _) if matches!(b.ty, Ty::Slice(_)) => self.origin(b),
            TK::Block(_, Some(t)) => self.origin(t),
            // `&s[i]`, `&p.f`: a pointer into what a tracked local refers to.
            TK::AddrOf(x) => self.place_origin(x),
            TK::If(_, t, Some(e)) => self.join(&[t, e]),
            TK::Match(_, arms) => self.join(&arms.iter().map(|a| &a.body).collect::<Vec<_>>()),
            _ => None,
        }
    }

    /// `&x` (not itself an object reference) used as `u`: a pointer into
    /// the memory of the slice or pointer at the base of place `x`, so
    /// that base is used as the pointer is.
    fn addr(&mut self, x: &'a TExpr, u: Use) {
        let u = match u {
            Use::Read | Use::Alias => Use::Read,
            Use::Raw => Use::Write,
            u => u,
        };
        match &x.kind {
            TK::Index(b, i) => {
                match &b.ty {
                    Ty::Array(..) => self.addr(b, u),
                    _ => self.walk(b, u),
                }
                self.walk(i, Use::Value);
            }
            TK::Field(b, _) if !matches!(b.ty, Ty::Slice(_)) => self.addr(b, u),
            TK::Deref(p) => self.walk(p, u),
            // `&s` of a tracked local: a pointer to the slice itself.
            TK::Local(_) => self.walk(x, Use::Value),
            _ => self.walk(x, Use::Read),
        }
    }

    fn walk(&mut self, e: &'a TExpr, u: Use) {
        match &e.kind {
            TK::Local(l) => {
                if let Some(o) = self.tracked.get(l).copied() {
                    self.local(o, u, e.span);
                }
            }
            TK::Trailing(o) => {
                if traced_obj_slice(self.p, e) {
                    self.direct(e, true, u);
                }
                self.walk(o, Use::Read);
            }
            TK::SliceRange(b, lo, hi) => {
                match &b.ty {
                    // A sub-slice is used as the slice itself is.
                    Ty::Slice(_) => self.walk(b, u),
                    _ => {
                        if traced_obj_slice(self.p, e) {
                            self.direct(e, true, u);
                        }
                        self.walk(b, if u == Use::Write { Use::Write } else { Use::Read });
                    }
                }
                for x in [lo, hi].into_iter().flatten() {
                    self.walk(x, Use::Value);
                }
            }
            TK::AddrOf(x) => {
                if traced_obj_addr(self.p, e) {
                    self.direct(e, false, u);
                    self.walk(x, Use::Read);
                } else {
                    self.addr(x, u);
                }
            }
            TK::Index(b, i) => {
                self.walk(b, if u == Use::Write { Use::Write } else { Use::Read });
                self.walk(i, Use::Value);
            }
            TK::Field(b, k) => {
                let bu = match &b.ty {
                    Ty::Slice(_) if *k == 0 => Use::Raw,
                    Ty::Slice(_) => Use::Read,
                    _ if u == Use::Write => Use::Write,
                    _ => Use::Read,
                };
                self.walk(b, bu);
            }
            TK::Deref(x) => self.walk(x, if u == Use::Write { Use::Write } else { Use::Read }),
            TK::Assign(l, r) => {
                if !matches!(l.kind, TK::Local(_) | TK::Global(_)) {
                    // A store of a value without references needs no
                    // barrier (`ents[e].hash = DEAD`, `s[i] = Nil`).
                    let refs = !crate::gcgen::holds_no_refs(self.p, r);
                    self.walk(l, if refs { Use::Write } else { Use::Read });
                }
                self.walk(r, Use::Value);
            }
            TK::Call(f, args) => {
                if may_collect(self.p, f) && self.tracked.values().any(|o| matches!(o, Origin::Direct(true))) {
                    self.audit_lifetime(e.span, "call that may collect while a local holds an object slice");
                }
                self.walk(f, Use::Value);
                self.callee.push(f);
                for (i, a) in args.iter().enumerate() {
                    self.walk(a, Use::Arg(i));
                }
                self.callee.pop();
            }
            TK::Block(stmts, tail) => {
                for s in stmts {
                    match s {
                        TStmt::Let(l, Some(x)) => {
                            let ld = &self.f.locals[*l as usize];
                            match self.origin(x) {
                                // `for x in o.items` binds the slice to a
                                // compiler temporary that it only indexes.
                                Some(o) if !ld.mutable || ld.name.starts_with("tmp") => {
                                    self.walk(x, Use::Alias);
                                    self.tracked.insert(*l, o);
                                }
                                _ => self.walk(x, Use::Value),
                            }
                        }
                        TStmt::Let(_, None) => {}
                        TStmt::LetPat(_, x, els) => {
                            self.walk(x, Use::Value);
                            if let Some(e) = els {
                                self.walk(e, Use::Value);
                            }
                        }
                        TStmt::Expr(x) | TStmt::Defer(x) => self.walk(x, Use::Read),
                    }
                }
                if let Some(t) = tail {
                    self.walk(t, u);
                }
            }
            TK::If(conds, then, els) => {
                for c in conds {
                    match c {
                        TCond::Expr(x) | TCond::Let(_, x) => self.walk(x, Use::Value),
                    }
                }
                self.walk(then, u);
                if let Some(x) = els {
                    self.walk(x, u);
                }
            }
            TK::Match(s, arms) => {
                self.walk(s, Use::Value);
                for a in arms {
                    if let Some(g) = &a.guard {
                        self.walk(g, Use::Value);
                    }
                    self.walk(&a.body, u);
                }
            }
            TK::Loop { body, step, .. } => {
                self.walk(body, Use::Read);
                if let Some(s) = step {
                    self.walk(s, Use::Read);
                }
            }
            TK::Closure(_, caps) | TK::NewClosure(_, caps, _) => {
                for l in caps {
                    if let Some(o) = self.tracked.get(l).copied() {
                        self.local(o, Use::Value, e.span);
                    }
                }
                if let TK::NewClosure(_, _, a) = &e.kind {
                    self.walk(a, Use::Value);
                }
            }
            _ => {
                let mut kids: Vec<&'a TExpr> = Vec::new();
                children(e, &mut kids);
                for k in kids {
                    self.walk(k, Use::Value);
                }
            }
        }
    }
}

/// Whether `e` never produces a value (`trap()`, also as coerced to a
/// branch's type: a block without a tail).
fn diverges(e: &TExpr) -> bool {
    e.ty == Ty::Never || matches!(&e.kind, TK::Block(ss, None) if ss.iter().any(|s| matches!(s, TStmt::Expr(x) if diverges(x))))
}

/// The direct subexpressions of `e`, for the kinds `Fa::walk` does not
/// handle itself.
fn children<'a>(e: &'a TExpr, out: &mut Vec<&'a TExpr>) {
    match &e.kind {
        TK::Unary(_, a) | TK::Cast(a) | TK::ArrayRepeat(a, _) | TK::Try(a) => out.push(a),
        TK::Binary(_, a, b) | TK::Coalesce(a, b) => {
            out.push(a);
            out.push(b);
        }
        TK::Intrinsic(_, args) | TK::Struct(args) | TK::Tuple(args) | TK::Array(args) | TK::Variant(_, args) => out.extend(args.iter()),
        TK::Break(_, Some(v)) | TK::Return(Some(v)) => out.push(v),
        TK::New { heap, init, len, fill, .. } => {
            out.push(heap);
            out.push(init);
            for x in [len, fill].into_iter().flatten() {
                out.push(x);
            }
        }
        TK::Vm(v) => match v {
            TVm::Jump(a) | TVm::Exit(a) | TVm::Raise(a) | TVm::Cold(a) | TVm::Part(_, a) | TVm::WordAt(a, _, _) => out.push(a),
            TVm::SharedNext(t, _, _) => out.push(t),
            TVm::Synced(a, b, c) => {
                out.push(a);
                out.push(b);
                out.push(c);
            }
            TVm::Shared(_, args, size, cont) => {
                out.extend(args.iter());
                out.push(size);
                if let Some(c) = cont {
                    out.push(c);
                }
            }
            _ => {}
        },
        _ => {}
    }
}

/// Where the checker found `#![safe]` modules and `unsafe { }` blocks.
pub struct Ctx<'a> {
    pub safe_files: &'a HashSet<u32>,
    pub unsafe_spans: &'a [Span],
}

fn analyze<'a>(p: &'a Program, f: &'a FnDef, cx: &Ctx<'a>, report: bool, audit: Option<&'a crate::diag::SourceMap>) -> (u64, u64, Vec<Diag>) {
    let Some(b) = &f.body else { return (0, 0, Vec::new()) };
    let mut fa = Fa {
        p,
        f,
        callee: Vec::new(),
        tracked: HashMap::new(),
        written: 0,
        kept: 0,
        report,
        safe: cx.safe_files.contains(&f.span.file),
        unsafe_spans: cx.unsafe_spans,
        audit,
        errs: Vec::new(),
    };
    for (i, &l) in f.params.iter().enumerate() {
        if tracked_ty(p, &f.locals[l as usize].ty) {
            fa.tracked.insert(l, Origin::Param(i));
        }
    }
    fa.walk(b, Use::Value);
    (fa.written, fa.kept, fa.errs)
}

/// Summarize which parameters each function may store through
/// (`Program::ref_writes`), then check every function.
/// All errors found, each function checked.
pub fn check(p: &mut Program, cx: &Ctx, sm: &crate::diag::SourceMap) -> Result<(), Vec<Diag>> {
    let audit = std::env::var("LOIMI_OBJREF_AUDIT").is_ok().then_some(sm);
    let with_params: Vec<usize> = (0..p.fns.len())
        .filter(|&i| {
            let f = &p.fns[i];
            f.body.is_some() && f.params.iter().any(|&l| tracked_ty(p, &f.locals[l as usize].ty))
        })
        .collect();
    // The least fixpoint from "stores through nothing": a parameter is
    // marked once some use of it stores, escapes, or is passed on to a
    // marked parameter (marks only grow).
    loop {
        let mut changed = Vec::new();
        for &i in &with_params {
            let (w, k, _) = analyze(p, &p.fns[i], cx, false, None);
            let ow = p.ref_writes.get(&(i as FnId)).copied().unwrap_or(0);
            let ok = p.ref_keeps.get(&(i as FnId)).copied().unwrap_or(0);
            if w | ow != ow || k | ok != ok {
                changed.push((i as FnId, w | ow, k | ok));
            }
        }
        if changed.is_empty() {
            break;
        }
        for (i, w, k) in changed {
            p.ref_writes.insert(i, w);
            p.ref_keeps.insert(i, k);
        }
    }
    // A function's first error: the others may follow from it.
    let mut all = Vec::new();
    for f in &p.fns {
        let (_, _, errs) = analyze(p, f, cx, true, audit);
        all.extend(errs.into_iter().next());
    }
    if all.is_empty() {
        Ok(())
    } else {
        Err(all)
    }
}
