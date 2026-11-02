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
use crate::diag::{Diag, Res, Span};
use crate::tir::*;

impl<'a> Checker<'a> {
    /// An argument for a parameter of `block fn` type `want` (which may
    /// have holes when it comes from a generic call that is still being
    /// inferred): a closure literal, a `block fn` parameter passed on, or a
    /// plain function value.
    pub fn block_arg(&mut self, cx: &mut FnCx, a: &Expr, want: &Ty) -> Res<TExpr> {
        let v = match &a.kind {
            ExprKind::Closure(ps, ret, body) => return self.closure(cx, ps, ret.as_ref(), body, want, a.span),
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
    ) -> Res<TExpr> {
        if cx.vm.is_some() {
            return Err(Diag::new(
                span,
                "closures are not allowed in vm code (op handlers, vm functions, entries, `on raise`): pass a named function, or call a plain function that creates the closure",
            ));
        }
        let Ty::Block(wps, wret, eff) = want else {
            return Err(Diag::new(span, "a closure can only be an argument for a `block fn` parameter"));
        };
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
        let shown = Ty::Block(ptys.clone(), Box::new(ret_t.clone().unwrap_or(Ty::Param(HOLE))), *eff);
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
        def.env = Some(caps.clone());
        self.prog.fns.push(def);
        Ok(mk(Ty::Block(ptys, Box::new(rty), *eff), TK::Closure(fid, caps), span))
    }
}
