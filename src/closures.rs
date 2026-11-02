//! Non-escaping closures (`block fn`).
//!
//! A `block fn(A) -> R uses E` value is a pair `{ code, env }`. It exists
//! only as a function parameter: the checker lets a `block fn` local be
//! called or passed on as an argument for another `block fn` parameter,
//! and nothing else, so it cannot be stored, returned or copied anywhere
//! that outlives the call that received it. That is what lets closures
//! capture by reference without any lifetime analysis.
//!
//! A closure literal `|a, b| body` may only appear as such an argument.
//! Its body is checked in the enclosing function's scope (same `FnCx`), so
//! it sees the enclosing locals and type parameters, and is then turned
//! into a function of its own: its `FnDef` shares the enclosing function's
//! local table, and the locals it uses that were declared outside it are
//! its captures. The environment is an array of pointers to the captured
//! locals' stack slots, built in the enclosing frame (`TK::Closure`); the
//! closure function takes it as a hidden first parameter and uses the
//! pointers as the slots of those locals. Writes are therefore visible to
//! the enclosing function, and traced captured locals stay rooted by the
//! enclosing function's shadow-stack frame.
//!
//! A plain function converts to a `block fn` through a trampoline per
//! signature, with the function pointer as the environment (codegen).

use crate::ast::*;
use crate::check::{has_hole, mk, Checker, FnCx, HOLE};
use crate::tir::LocalDef;
use crate::diag::{Diag, Res, Span};
use crate::tir::*;

impl<'a> Checker<'a> {
    /// An argument for a parameter of `block fn` type `want` (which may
    /// have holes when it comes from a generic call that is still being
    /// inferred): a closure literal, a `block fn` parameter passed on, or a
    /// plain function value.
    pub fn block_arg(&mut self, cx: &mut FnCx, a: &Expr, want: &Ty) -> Res<TExpr> {
        let v = match &a.kind {
            ExprKind::Closure(ps, ret, body) => return self.closure(cx, ps, ret.as_ref(), body, want, a.span, false),
            ExprKind::Ident(n) if cx.lookup(n).is_some() => {
                cx.block_use = true;
                let r = self.expr(cx, a, Some(want));
                cx.block_use = false;
                r?
            }
            _ => {
                // A function (also a generic one, whose type arguments come
                // from the parameter's type).
                let Ty::Block(ps, r, e) = want else { unreachable!() };
                let as_fn = Ty::Fn(ps.clone(), r.clone(), *e);
                self.expr(cx, a, Some(&as_fn))?
            }
        };
        if !has_hole(want) {
            return self.coerce(v, want);
        }
        // Inference continues from the value's type.
        match v.ty.clone() {
            Ty::Fn(ps, r, e) => {
                let span = v.span;
                Ok(mk(Ty::Block(ps, r, e), TK::Cast(Box::new(v)), span))
            }
            _ => Ok(v),
        }
    }

    fn closure(
        &mut self,
        cx: &mut FnCx,
        params: &[(Ident, Option<TypeExpr>)],
        ret: Option<&TypeExpr>,
        body: &Expr,
        want: &Ty,
        span: Span,
        gc: bool,
    ) -> Res<TExpr> {
        if cx.vm.is_some() {
            return Err(Diag::new(
                span,
                "closures are not allowed in vm code (op handlers, vm functions, entries, `on raise`): pass a named function, or call a plain function that creates the closure",
            ));
        }
        let (Ty::Block(wps, wret, eff) | Ty::Closure(wps, wret, eff)) = want else {
            return Err(Diag::new(span, "a closure can only be an argument for a `block fn` parameter"));
        };
        let escaping = matches!(want, Ty::Closure(..));
        if params.len() != wps.len() {
            return Err(Diag::new(
                span,
                format!("this closure takes {} parameter(s), but `{}` takes {}", params.len(), self.ty_name(want), wps.len()),
            ));
        }
        let mut ptys = Vec::new();
        for ((name, ann), wt) in params.iter().zip(wps) {
            let t = match ann {
                Some(te) => {
                    let t = self.resolve_type(cx.module, te)?;
                    if !has_hole(wt) && t != *wt {
                        return Err(Diag::new(
                            te.span,
                            format!("closure parameter `{}` has type `{}`, but `{}` passes `{}`", name.name, self.ty_name(&t), self.ty_name(want), self.ty_name(wt)),
                        ));
                    }
                    t
                }
                None if has_hole(wt) => {
                    return Err(Diag::new(
                        name.span,
                        format!("cannot infer the type of closure parameter `{}`; write `{}: T`", name.name, name.name),
                    ))
                }
                None => wt.clone(),
            };
            ptys.push(t);
        }
        let ret_t: Option<Ty> = match ret {
            Some(te) => {
                let t = self.resolve_type(cx.module, te)?;
                if !has_hole(wret) && t != **wret {
                    return Err(Diag::new(
                        te.span,
                        format!("the closure returns `{}`, but `{}` needs `{}`", self.ty_name(&t), self.ty_name(want), self.ty_name(wret)),
                    ));
                }
                Some(t)
            }
            None if has_hole(wret) => None,
            None => Some((**wret).clone()),
        };
        let shown_ret = Box::new(ret_t.clone().unwrap_or(Ty::Param(HOLE)));
        let shown = if escaping { Ty::Closure(ptys.clone(), shown_ret, *eff) } else { Ty::Block(ptys.clone(), shown_ret, *eff) };
        // The body is a function of its own: `return` and `?` leave the
        // closure, loops outside it are not reachable, and its calls may
        // have the effects its type allows.
        let saved_ret = std::mem::replace(&mut cx.ret, ret_t.clone().unwrap_or(Ty::Param(HOLE)));
        let saved_loops = std::mem::take(&mut cx.loops);
        let saved_eff = std::mem::replace(&mut cx.effects, *eff);
        let saved_closure = std::mem::replace(&mut cx.closure_ty, Some(shown));
        cx.scopes.push(vec![]);
        let start = cx.locals.len() as LocalId;
        let mut pids = Vec::new();
        for ((name, _), t) in params.iter().zip(&ptys) {
            pids.push(cx.declare(&name.name, t.clone(), true));
        }
        let r = (|| -> Res<TExpr> {
            let b = self.expr(cx, body, ret_t.as_ref())?;
            match &ret_t {
                Some(t) => self.coerce(b, t),
                None => Ok(b),
            }
        })();
        cx.scopes.pop();
        cx.ret = saved_ret;
        cx.loops = saved_loops;
        cx.effects = saved_eff;
        cx.closure_ty = saved_closure;
        let b = r?;
        let rty = ret_t.unwrap_or_else(|| b.ty.clone());
        let mut used = Vec::new();
        let mut declared = Vec::new();
        crate::tirwalk::locals(&b, &mut used, &mut declared);
        let caps: Vec<LocalId> = used.into_iter().filter(|l| *l < start).collect();
        if escaping {
            self.check_escaping_captures(cx, &b, &caps, gc)?;
        }
        let n = self.closure_count;
        self.closure_count += 1;
        let owner = if cx.fn_name.is_empty() { "closure".to_string() } else { cx.fn_name.clone() };
        let fid = self.prog.fns.len() as FnId;
        let mut def = crate::check::placeholder_fn(span);
        def.name = format!("{} (closure {})", owner, n);
        def.symbol = format!("{}.{}.closure.{}", self.module_prefix(cx.module), owner, n);
        def.params = pids;
        def.param_tys = ptys.clone();
        def.ret = rty.clone();
        def.locals = cx.locals.clone();
        // Inside a generic body checked against its bounds, nothing is kept.
        def.body = if self.traits.opaque_depth > 0 { None } else { Some(b) };
        def.effects = *eff;
        // A `let` cannot change while the closure lives, so a small one is
        // copied into the environment. Traced values stay by reference, in
        // the slot the enclosing frame roots.
        def.env_val = caps
            .iter()
            .map(|l| {
                let t = &cx.locals[*l as usize].ty;
                !cx.locals[*l as usize].mutable
                    && crate::layout::size_of(&self.prog, t) <= 8
                    && !crate::gcgen::is_traced(&self.prog, t)
                    && !matches!(t, Ty::Opaque(_) | Ty::Param(_))
            })
            .collect();
        if escaping {
            // Captured by value into an environment struct: a GC object
            // `{ code, caps.. }`, or plain host memory `{ caps.. }`.
            let cty = Ty::Closure(ptys.clone(), Box::new(rty.clone()), *eff);
            def.env_val = vec![true; caps.len()];
            if self.traits.opaque_depth == 0 && (gc || !caps.is_empty()) {
                let mut fields: Vec<(String, Ty)> = Vec::new();
                if gc {
                    fields.push(("code".to_string(), Ty::Ptr(Box::new(Ty::u8()))));
                }
                for l in &caps {
                    let ld = &cx.locals[*l as usize];
                    fields.push((ld.name.clone(), ld.ty.clone()));
                }
                let sid = self.prog.structs.len() as u32;
                let object = if gc {
                    self.object_count += 1;
                    Some(self.object_count - 1)
                } else {
                    None
                };
                self.prog.structs.push(crate::tir::StructDef {
                    name: format!("{} env", def.symbol),
                    fields,
                    pattern: false,
                    object,
                    trailing: None,
                    verified: None,
                });
                def.env_struct = Some((sid, gc as u32));
            }
            if gc {
                // The environment object, rooted while the body runs.
                let id = cx.locals.len() as LocalId;
                cx.locals.push(LocalDef { name: "closure env".into(), ty: Ty::Gc(Box::new(cty.clone())), mutable: false, field: None });
                def.locals = cx.locals.clone();
                def.env_root = Some(id);
            }
            def.env = Some(caps.clone());
            self.prog.fns.push(def);
            let ty = if gc { Ty::Gc(Box::new(cty)) } else { cty };
            return Ok(mk(ty, TK::NewClosure(fid, caps, Box::new(mk(Ty::Unit, TK::Unit, span))), span));
        }
        def.env = Some(caps.clone());
        self.prog.fns.push(def);
        Ok(mk(Ty::Block(ptys, Box::new(rty), *eff), TK::Closure(fid, caps), span))
    }
}

/// The place `e` is rooted at, if it is (part of) a local's own storage:
/// `x`, `x.f`, `x[i]` on an array, but not through a pointer or slice.
fn place_root(e: &TExpr) -> Option<(LocalId, Span)> {
    match &e.kind {
        TK::Local(l) => Some((*l, e.span)),
        TK::Field(b, _) => place_root(b).map(|(l, _)| (l, e.span)),
        TK::Index(b, _) if matches!(b.ty, Ty::Array(..)) => place_root(b).map(|(l, _)| (l, e.span)),
        _ => None,
    }
}

impl<'a> Checker<'a> {
    /// `new(a, |..| ..)`: an escaping closure, with its environment
    /// allocated by `a` (a `closure fn`), or in the GC heap when `a` is a
    /// `*gc.Heap` (a `Gc<closure fn>`).
    pub fn new_closure(&mut self, cx: &mut FnCx, alloc: &Expr, lit: &Expr, exp: Option<&Ty>, span: Span) -> Res<TExpr> {
        let ExprKind::Closure(ps, ret, body) = &lit.kind else { unreachable!() };
        let a = self.expr(cx, alloc, None)?;
        let gc = matches!(&a.ty, Ty::Ptr(t) if matches!(**t, Ty::Struct(s) if self.prog.structs[s as usize].name == "std.gc.Heap"));
        let mut sig = exp;
        if let Some(Ty::Option(inner)) = sig {
            sig = Some(inner);
        }
        if let Some(Ty::Gc(inner)) = sig {
            sig = Some(inner);
        }
        let want = match sig {
            Some(t @ Ty::Closure(..)) => t.clone(),
            _ => {
                // No expected closure type: the literal's annotations.
                let mut pts = Vec::new();
                for (name, ann) in ps {
                    match ann {
                        Some(te) => pts.push(self.resolve_type(cx.module, te)?),
                        None => {
                            return Err(Diag::new(
                                name.span,
                                format!(
                                    "cannot infer the type of closure parameter `{}`: write `{}: T`, or give the closure a type (`let f: closure fn(..) -> R = new(..)`)",
                                    name.name, name.name
                                ),
                            ))
                        }
                    }
                }
                Ty::Closure(pts, Box::new(Ty::Param(HOLE)), 0)
            }
        };
        let mut v = self.closure(cx, ps, ret.as_ref(), body, &want, lit.span, gc)?;
        let TK::NewClosure(fid, caps, _) = &v.kind else { unreachable!() };
        let (fid, caps) = (*fid, caps.clone());
        let mem = if gc || caps.is_empty() {
            a
        } else {
            match self.prog.fns[fid as usize].env_struct {
                Some((sid, _)) => {
                    let st = Ty::Struct(sid);
                    let size = crate::layout::size_of(&self.prog, &st) as i128;
                    let align = crate::layout::align_of(&self.prog, &st) as i128;
                    let lit = |v: i128| Expr { kind: ExprKind::Int(v as u128), span: alloc.span };
                    let name = Ident { name: "alloc_bytes".into(), span: alloc.span };
                    let at = self.ty_name(&a.ty);
                    self.method_call(cx, a, &name, &[lit(size), lit(align)], None, span).map_err(|d| {
                        d.note(alloc.span, format!(
                            "the environment of `new(a, |..| ..)` is allocated with `a.alloc_bytes(size: u64, align: u64) -> *u8` (`a` is `{}`; `*mem.Arena` has one), or in the GC heap if `a` is a `*gc.Heap`",
                            at
                        ))
                    })?
                }
                // Inside a generic body checked against its bounds: no code.
                None => a,
            }
        };
        if !gc && !caps.is_empty() && !matches!(mem.ty, Ty::Ptr(_)) && self.traits.opaque_depth == 0 {
            return Err(Diag::new(alloc.span, "`alloc_bytes` must return a pointer (`*u8`)"));
        }
        v.kind = TK::NewClosure(fid, caps, Box::new(mem));
        Ok(v)
    }

    /// What an escaping closure may capture: no `block fn` (it only lives
    /// during its call), nothing traced unless the environment is in the
    /// GC heap, and captured locals are copies: assigning one is an error.
    fn check_escaping_captures(&mut self, cx: &FnCx, body: &TExpr, caps: &[LocalId], gc: bool) -> Res<()> {
        for l in caps {
            let ld = &cx.locals[*l as usize];
            if matches!(ld.ty, Ty::Block(..)) {
                return Err(Diag::new(
                    body.span,
                    format!(
                        "this closure escapes, but it captures `{}`, a `block fn` that only lives during the call that received it",
                        ld.name
                    ),
                ));
            }
            if !gc && self.traits.opaque_depth == 0 && crate::gcgen::is_traced(&self.prog, &ld.ty) {
                return Err(Diag::new(
                    body.span,
                    format!(
                        "this closure captures `{}` of type `{}`, which holds GC references, but its environment is not traced: create it in the GC heap with `new(heap, |..| ..)` (a `Gc<closure fn(..)>`)",
                        ld.name,
                        self.ty_name(&ld.ty)
                    ),
                ));
            }
        }
        let mut err: Option<Diag> = None;
        let prog = &self.prog;
        let check = |e: &TExpr, err: &mut Option<Diag>| {
            if err.is_some() {
                return;
            }
            match &e.kind {
                TK::Assign(lhs, _) => {
                    if let Some((l, sp)) = place_root(lhs) {
                        if caps.contains(&l) {
                            let name = &cx.locals[l as usize].name;
                            *err = Some(
                                Diag::new(
                                    sp,
                                    format!(
                                        "`{}` is captured by value: this closure escapes, so it has its own copy, and assigning it would not change `{}` outside",
                                        name, name
                                    ),
                                )
                                .note(body.span, "keep shared or changing state behind a pointer that the closure captures"),
                            );
                        }
                    }
                }
                TK::AddrOf(inner) if gc => {
                    if let Some((l, sp)) = place_root(inner) {
                        if caps.contains(&l) && crate::gcgen::is_traced(prog, &cx.locals[l as usize].ty) {
                            let name = &cx.locals[l as usize].name;
                            *err = Some(Diag::new(
                                sp,
                                format!(
                                    "cannot take the address of `{}`: it is a traced capture in a GC closure's environment, and a store through the pointer would miss the write barrier",
                                    name
                                ),
                            ));
                        }
                    }
                }
                _ => {}
            }
        };
        // Nested closures write the same locals.
        let mut bodies = vec![body.clone()];
        while let Some(b) = bodies.pop() {
            crate::tirwalk::for_each_expr(&b, &mut |e| {
                check(e, &mut err);
                if let TK::Closure(fid, _) | TK::NewClosure(fid, _, _) = &e.kind {
                    if let Some(nb) = &prog.fns[*fid as usize].body {
                        bodies.push(nb.clone());
                    }
                }
            });
        }
        match err {
            Some(d) => Err(d),
            None => Ok(()),
        }
    }
}
