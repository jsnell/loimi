//! Static traits: `trait` declarations (method signatures over `Self`, with
//! optional default bodies, type parameters on the trait and on its
//! methods), `impl Trait for Type;` assertions (also generic ones:
//! `impl<T: Hash> Hash for Vec<T>;`), and bounds on type parameters.
//!
//! Bounds are used twice:
//!
//! - **At instantiation**, against the impls: `HashMap<Sym, i64>` needs
//!   `Sym: Hash`, and the error is reported at the use.
//! - **On generic bodies.** Before an instance of a generic function with
//!   bounded type parameters is checked, its body is checked once more
//!   with each bounded parameter replaced by an opaque type
//!   (`Ty::Opaque`) that only has the methods of its bounds. A method call
//!   on it resolves through the bound, and anything else the bounds do not
//!   provide is an error in the generic code. Unbounded parameters keep
//!   their concrete type in that pass, so they stay duck-typed. The result
//!   of that pass is thrown away; code comes from the per-instance check,
//!   where calls resolve to the concrete type's methods (or to a trait's
//!   default body instantiated for it). Dispatch is static.

use std::collections::{HashMap, HashSet};

use crate::ast::*;
use crate::check::{effect_name, effects_of, Checker, Def};
use crate::diag::{Diag, Res, Span};
use crate::generics::GenericFn;
use crate::tir::*;

/// `impl<T: Hash> Hash for Vec<T>;`: the type and trait arguments are
/// patterns over the impl's parameters (`Ty::Param(i)`).
pub struct GenImpl {
    pub module: usize,
    pub trait_id: u32,
    pub params: Vec<String>,
    pub bounds: Vec<Bounds>,
    pub ty: Ty,
    pub args: Vec<Ty>,
}

/// A bounded type parameter seen from inside a generic body.
pub struct OpaqueInfo {
    pub name: String,
    pub bounds: Vec<(u32, Vec<Ty>)>,
}

/// Resolved bounds, per type parameter: (trait, trait arguments).
pub type ResolvedBounds = Vec<Vec<(u32, Vec<Ty>)>>;

#[derive(Default)]
pub struct Traits {
    /// Declaring module and declaration, indexed by trait id.
    pub decls: Vec<(usize, TraitDecl)>,
    /// Per trait: method name → the generic function holding its signature
    /// (and default body), with type parameters `[trait params.., Self,
    /// method params..]`.
    pub methods: Vec<HashMap<String, u32>>,
    /// `impl` items with their module, in source order.
    pub impl_decls: Vec<(usize, ImplDecl)>,
    /// (trait, trait arguments, type) → index into `impl_decls`.
    pub impls: HashMap<(u32, Vec<Ty>, Ty), usize>,
    pub gen_impls: Vec<GenImpl>,
    pub opaques: Vec<OpaqueInfo>,
    /// Generic functions already checked against their bounds, with the
    /// arguments of their unbounded parameters.
    pub checked: HashSet<(u32, Vec<Option<Ty>>)>,
    /// Inside a generic body checked against its bounds: closures and other
    /// generated functions are not kept.
    pub opaque_depth: u32,
    /// While impls are registered, bound checks wait until all are known.
    pub deferred: Option<Vec<(ResolvedBounds, Vec<Ty>, String, Span)>>,
}

impl<'a> Checker<'a> {
    /// `Hash` or `hash.Hash`, seen from module `m`.
    pub fn resolve_trait(&self, m: usize, path: &[Ident]) -> Res<u32> {
        let last = path.last().unwrap();
        let found = match path {
            [n] => self.scopes[m].get(&n.name).map(|(d, _)| d.clone()),
            [md, n] => match self.scopes[m].get(&md.name) {
                Some((Def::Module(t), _)) => match self.scopes[*t].get(&n.name) {
                    Some((d, true)) => Some(d.clone()),
                    Some((_, false)) => return Err(Diag::new(n.span, format!("`{}` is private", n.name))),
                    None => None,
                },
                _ => None,
            },
            _ => None,
        };
        match found {
            Some(Def::Trait(t)) => Ok(t),
            Some(_) => Err(Diag::new(last.span, format!("`{}` is not a trait", last.name))),
            None => Err(Diag::new(last.span, format!("unknown trait `{}`", last.name))),
        }
    }

    /// `Hooks<K, E>` (with type parameters bound in `tenv`).
    pub fn resolve_trait_ref(&mut self, m: usize, r: &TraitRef) -> Res<(u32, Vec<Ty>)> {
        let t = self.resolve_trait(m, &r.path)?;
        let n = self.traits.decls[t as usize].1.generics.len();
        if r.args.len() != n {
            return Err(Diag::new(r.span, format!("trait `{}` takes {} type argument(s), got {}", self.trait_name(t), n, r.args.len())));
        }
        let mut args = Vec::new();
        for a in &r.args {
            args.push(self.resolve_type(m, a)?);
        }
        Ok((t, args))
    }

    pub fn trait_name(&self, t: u32) -> String {
        self.traits.decls[t as usize].1.name.name.clone()
    }

    /// `Hash`, `Hooks<Tok, Expr>`.
    pub fn trait_ref_name(&self, t: u32, args: &[Ty]) -> String {
        if args.is_empty() {
            self.trait_name(t)
        } else {
            format!("{}<{}>", self.trait_name(t), args.iter().map(|a| self.ty_name(a)).collect::<Vec<_>>().join(", "))
        }
    }

    pub fn new_opaque(&mut self, name: &str) -> Ty {
        self.traits.opaques.push(OpaqueInfo { name: name.to_string(), bounds: vec![] });
        Ty::Opaque(self.traits.opaques.len() as u32 - 1)
    }

    /// Pass 1: a trait declaration. Each method becomes a generic function
    /// over `[trait params.., Self, method params..]` with `Self` bounded by
    /// the trait: its signature is what calls through a bound see, and its
    /// body (if any) is the default, instantiated per implementing type.
    pub fn register_trait(&mut self, m: usize, t: &TraitDecl) -> Res<u32> {
        let id = self.traits.decls.len() as u32;
        self.traits.decls.push((m, t.clone()));
        let mut map = HashMap::new();
        for f in &t.methods {
            if f.generics.iter().chain(t.generics.iter()).any(|g| g.name == "Self") {
                return Err(Diag::new(f.name.span, "`Self` cannot be a type parameter's name"));
            }
            let mut d = f.clone();
            d.name.name = format!("{}.{}", t.name.name, f.name.name);
            let self_id = Ident { name: "Self".into(), span: t.name.span };
            let mut generics = t.generics.clone();
            generics.push(self_id);
            generics.extend(f.generics.iter().cloned());
            let mut bounds: Vec<Bounds> = vec![vec![]; t.generics.len()];
            let targs = t.generics.iter().map(|g| TypeExpr { kind: TypeKind::Named(vec![g.clone()], vec![]), span: g.span }).collect();
            bounds.push(vec![TraitRef { path: vec![t.name.clone()], args: targs, span: t.name.span }]);
            bounds.extend(f.bounds.iter().cloned());
            d.generics = generics;
            d.bounds = bounds;
            d.recv = None;
            d.recv_generics = 0;
            let g = self.generics.fns.len() as u32;
            let params = d.generics.iter().map(|g| g.name.clone()).collect();
            self.generics.fns.push(GenericFn { module: m, decl: d, params });
            if map.insert(f.name.name.clone(), g).is_some() {
                return Err(Diag::new(f.name.span, format!("method `{}` is declared more than once", f.name.name)));
            }
        }
        self.traits.methods.push(map);
        Ok(id)
    }

    /// Record every `impl` (before type definitions, so that instances
    /// created while resolving them can be checked), and check that bounds
    /// name traits.
    pub fn register_impls(&mut self) -> Res<()> {
        self.traits.deferred = Some(vec![]);
        for i in 0..self.traits.impl_decls.len() {
            let (m, d) = self.traits.impl_decls[i].clone();
            if d.generics.is_empty() {
                let (t, targs) = self.resolve_trait_ref(m, &d.trait_ref)?;
                let ty = self.resolve_type(m, &d.ty)?;
                if self.traits.impls.insert((t, targs.clone(), ty.clone()), i).is_some() {
                    return Err(Diag::new(
                        d.span,
                        format!("`{}` already implements `{}`", self.ty_name(&ty), self.trait_ref_name(t, &targs)),
                    ));
                }
                continue;
            }
            let params: Vec<String> = d.generics.iter().map(|g| g.name.clone()).collect();
            let env: Vec<(String, Ty)> = params.iter().enumerate().map(|(i, p)| (p.clone(), Ty::Param(i as u32))).collect();
            let (t, targs, ty) = self.with_tenv(env, |c| -> Res<(u32, Vec<Ty>, Ty)> {
                let (t, targs) = c.resolve_trait_ref(m, &d.trait_ref)?;
                Ok((t, targs, c.resolve_type(m, &d.ty)?))
            })?;
            for (i, g) in d.generics.iter().enumerate() {
                if !mentions_param(&ty, i as u32, &self.generics) {
                    return Err(Diag::new(
                        g.span,
                        format!("the impl's type parameter `{}` must appear in the implementing type", g.name),
                    ));
                }
            }
            self.traits.gen_impls.push(GenImpl { module: m, trait_id: t, params, bounds: d.bounds.clone(), ty, args: targs });
        }
        let mut all: Vec<(usize, Vec<Bounds>)> = Vec::new();
        for gt in &self.generics.types {
            all.push((gt.module, gt.bounds.clone()));
        }
        for gf in &self.generics.fns {
            all.push((gf.module, gf.decl.bounds.clone()));
        }
        for (m, d) in &self.traits.impl_decls {
            all.push((*m, d.bounds.clone()));
        }
        for (m, bounds) in all {
            for b in bounds.iter().flatten() {
                self.resolve_trait(m, &b.path)?;
            }
        }
        for (bounds, args, needed_by, span) in self.traits.deferred.take().unwrap() {
            self.check_bounds(&bounds, &args, needed_by, span)?;
        }
        Ok(())
    }

    /// After all signatures are declared: every impl provides each trait
    /// method without a default as a method of the type, with the trait's
    /// signature (`Self` = the type, the trait's parameters = the impl's
    /// arguments) and no more effects than the trait allows. A generic impl
    /// is checked once, with its parameters as opaque types that have the
    /// impl's bounds.
    pub fn check_impls(&mut self) -> Res<()> {
        for i in 0..self.traits.impl_decls.len() {
            let (m, d) = self.traits.impl_decls[i].clone();
            let names: Vec<String> = d.generics.iter().map(|g| g.name.clone()).collect();
            let os: Vec<Ty> = names.iter().map(|n| self.new_opaque(n)).collect();
            let env: Vec<(String, Ty)> = names.iter().cloned().zip(os.iter().cloned()).collect();
            let rb = self.resolve_bounds(m, &names, &os, &d.bounds)?;
            for (o, b) in os.iter().zip(rb) {
                let Ty::Opaque(o) = o else { unreachable!() };
                self.traits.opaques[*o as usize].bounds = b;
            }
            let (t, targs, ty) = self.with_tenv(env, |c| -> Res<(u32, Vec<Ty>, Ty)> {
                let (t, targs) = c.resolve_trait_ref(m, &d.trait_ref)?;
                Ok((t, targs, c.resolve_type(m, &d.ty)?))
            })?;
            self.check_impl_methods(t, &targs, &ty, d.span)?;
        }
        Ok(())
    }

    fn check_impl_methods(&mut self, t: u32, targs: &[Ty], ty: &Ty, span: Span) -> Res<()> {
        let (tm, decl) = self.traits.decls[t as usize].clone();
        let tname = self.ty_name(ty);
        let what = format!("`{}` does not implement `{}`", tname, self.trait_ref_name(t, targs));
        for f in &decl.methods {
            let own: Vec<Ty> = f.generics.iter().map(|g| self.new_opaque(&g.name)).collect();
            let mut names: Vec<String> = decl.generics.iter().map(|g| g.name.clone()).collect();
            let mut tys: Vec<Ty> = targs.to_vec();
            names.push("Self".into());
            tys.push(ty.clone());
            names.extend(f.generics.iter().map(|g| g.name.clone()));
            tys.extend(own.iter().cloned());
            let k = decl.generics.len() + 1;
            let mut fb: Vec<Bounds> = vec![vec![]; k];
            fb.extend(f.bounds.iter().cloned());
            let rb = self.resolve_bounds(tm, &names, &tys, &fb)?;
            for (o, b) in own.iter().zip(rb.into_iter().skip(k)) {
                let Ty::Opaque(o) = o else { unreachable!() };
                self.traits.opaques[*o as usize].bounds = b;
            }
            let env: Vec<(String, Ty)> = names.into_iter().zip(tys).collect();
            let (params, ret) = self.with_tenv(env, |c| -> Res<(Vec<Ty>, Ty)> {
                let mut ps = Vec::new();
                for p in &f.params {
                    ps.push(c.resolve_param_type(tm, &p.ty)?);
                }
                let r = match &f.ret {
                    Some(r) => c.resolve_type(tm, r)?,
                    None => Ty::Unit,
                };
                Ok((ps, r))
            })?;
            let want = Ty::Fn(params, Box::new(ret), effects_of(&f.attrs)?);
            let mname = format!("{}.{}", tname, f.name.name);
            let have = match self.own_method(ty, &f.name.name, &own, span) {
                Ok(Some(h)) => h,
                Ok(None) if f.body.is_some() => continue,
                Ok(None) => return Err(Diag::new(span, format!("{}: no method `{}`", what, mname))),
                Err(d) => return Err(Diag::new(span, format!("{}: {}", what, d.msg))),
            };
            let (Ty::Fn(p1, r1, e1), Ty::Fn(p2, r2, e2)) = (&have, &want) else { unreachable!() };
            if p1 != p2 || r1 != r2 {
                return Err(Diag::new(
                    span,
                    format!("{}: `{}` has type `{}`, but the trait needs `{}`", what, mname, self.ty_name(&have), self.ty_name(&want)),
                ));
            }
            if e1 & !e2 != 0 {
                return Err(Diag::new(
                    span,
                    format!("{}: `{}` uses {}, which the trait does not allow", what, mname, effect_name(e1 & !e2)),
                ));
            }
        }
        Ok(())
    }

    /// The type of `ty`'s own method `name` (not a trait default), with
    /// `own` as the method's own type arguments. None if there is none.
    fn own_method(&mut self, ty: &Ty, name: &str, own: &[Ty], span: Span) -> Res<Option<Ty>> {
        // An instance of a generic type may also have methods of its own,
        // defined through an alias (`type IntBox = Box<i64>; fn IntBox.m`).
        let generic = match self.generics.inst_of.get(ty).cloned() {
            Some((g, rargs)) if self.generics.methods.contains_key(&(g, name.to_string())) => {
                self.generics.methods.get(&(g, name.to_string())).map(|gf| (*gf, rargs))
            }
            _ => {
                if let Some(fid) = self.methods.get(&(ty.clone(), name.to_string())).copied() {
                    if !own.is_empty() {
                        return Err(Diag::new(span, format!("the trait's `{}` takes {} type parameter(s), the method none", name, own.len())));
                    }
                    return Ok(Some(self.prog.fns[fid as usize].fn_ty()));
                }
                self.generics.ty_methods.get(&(ty.clone(), name.to_string())).map(|gf| (*gf, vec![]))
            }
        };
        let Some((gf, mut args)) = generic else { return Ok(None) };
        let gfd = &self.generics.fns[gf as usize];
        let n_own = gfd.params.len() - gfd.decl.recv_generics;
        if n_own != own.len() {
            return Err(Diag::new(
                span,
                format!("the trait's `{}` takes {} type parameter(s), the method {}", name, own.len(), n_own),
            ));
        }
        args.extend(own.iter().cloned());
        let fid = self.instantiate_fn(gf, args, span)?;
        Ok(Some(self.prog.fns[fid as usize].fn_ty()))
    }

    /// Resolve `bounds` (per parameter, in module `m`) with the parameters
    /// `names` bound to `args`.
    pub fn resolve_bounds(&mut self, m: usize, names: &[String], args: &[Ty], bounds: &[Bounds]) -> Res<ResolvedBounds> {
        let env: Vec<(String, Ty)> = names.iter().cloned().zip(args.iter().cloned()).collect();
        self.with_tenv(env, |c| -> Res<ResolvedBounds> {
            let mut out = Vec::new();
            for b in bounds {
                let mut v = Vec::new();
                for r in b {
                    v.push(c.resolve_trait_ref(m, r)?);
                }
                out.push(v);
            }
            Ok(out)
        })
    }

    /// The bounds of generic type `g`'s parameters, for arguments `args`.
    pub fn type_bounds(&mut self, g: u32, args: &[Ty]) -> Res<ResolvedBounds> {
        let gt = &self.generics.types[g as usize];
        let (m, params, bounds) = (gt.module, gt.params.clone(), gt.bounds.clone());
        self.resolve_bounds(m, &params, args, &bounds)
    }

    /// The generic type whose method `g` is, if any.
    fn recv_type(&self, g: u32) -> Option<u32> {
        let gf = &self.generics.fns[g as usize];
        let recv = gf.decl.recv.as_ref()?;
        if gf.decl.recv_generics == 0 {
            return None;
        }
        match self.scopes[gf.module].get(&recv.name) {
            Some((Def::GenericType(t), _)) => Some(*t),
            _ => None,
        }
    }

    /// The bounds of generic function `g`'s parameters, for arguments
    /// `args`. A method of a generic type also has the type's bounds on the
    /// receiver's parameters (`fn HashMap<K, V>.get` has `K: Hash`).
    pub fn fn_bounds(&mut self, g: u32, args: &[Ty]) -> Res<ResolvedBounds> {
        let gf = &self.generics.fns[g as usize];
        let (m, params, bounds) = (gf.module, gf.params.clone(), gf.decl.bounds.clone());
        let mut out = self.resolve_bounds(m, &params, args, &bounds)?;
        out.resize(params.len(), vec![]);
        if let Some(t) = self.recv_type(g) {
            let n = self.generics.types[t as usize].params.len();
            let implied = self.type_bounds(t, &args[..n])?;
            for (i, b) in implied.into_iter().enumerate() {
                for x in b {
                    if !out[i].contains(&x) {
                        out[i].push(x);
                    }
                }
            }
        }
        Ok(out)
    }

    /// Which of generic function `g`'s parameters have bounds (written or
    /// implied by the receiver type).
    fn fn_bounded(&self, g: u32) -> Vec<bool> {
        let gf = &self.generics.fns[g as usize];
        let mut out: Vec<bool> = (0..gf.params.len()).map(|i| gf.decl.bounds.get(i).is_some_and(|b| !b.is_empty())).collect();
        if let Some(t) = self.recv_type(g) {
            for (i, b) in self.generics.types[t as usize].bounds.iter().enumerate() {
                if !b.is_empty() {
                    out[i] = true;
                }
            }
        }
        out
    }

    /// Check resolved bounds against type arguments. Arguments that are
    /// still inference variables (inside generic signatures) are skipped;
    /// their concrete instances are checked.
    pub fn check_bounds(&mut self, bounds: &ResolvedBounds, args: &[Ty], needed_by: String, span: Span) -> Res<()> {
        if let Some(d) = &mut self.traits.deferred {
            d.push((bounds.clone(), args.to_vec(), needed_by, span));
            return Ok(());
        }
        for (b, a) in bounds.iter().zip(args) {
            if crate::generics::contains_infer(a, &self.generics) {
                continue;
            }
            for (t, targs) in b {
                if targs.iter().any(|t| crate::generics::contains_infer(t, &self.generics)) {
                    continue;
                }
                if !self.implements(a, *t, targs, 0)? {
                    return Err(Diag::new(
                        span,
                        format!("`{}` does not implement `{}` (needed by `{}`)", self.ty_name(a), self.trait_ref_name(*t, targs), needed_by),
                    ));
                }
            }
        }
        Ok(())
    }

    /// Does `ty` implement trait `t` with arguments `targs`: through an
    /// impl, a generic impl whose bounds hold, or (for an opaque type
    /// parameter) its bounds?
    pub fn implements(&mut self, ty: &Ty, t: u32, targs: &[Ty], depth: u32) -> Res<bool> {
        if depth > 32 {
            return Ok(false);
        }
        if let Ty::Opaque(o) = ty {
            if self.traits.opaques[*o as usize].bounds.iter().any(|(t2, a2)| *t2 == t && a2 == targs) {
                return Ok(true);
            }
        }
        if self.traits.impls.contains_key(&(t, targs.to_vec(), ty.clone())) {
            return Ok(true);
        }
        for i in 0..self.traits.gen_impls.len() {
            if self.traits.gen_impls[i].trait_id != t {
                continue;
            }
            let Some(bound) = self.match_gen_impl(i, ty, Some(targs)) else { continue };
            if self.gen_impl_holds(i, &bound, depth)? {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Bind generic impl `i`'s parameters by matching its type (and trait
    /// arguments, if given) against `ty`.
    fn match_gen_impl(&self, i: usize, ty: &Ty, targs: Option<&[Ty]>) -> Option<Vec<Ty>> {
        let gi = &self.traits.gen_impls[i];
        let mut b = vec![None; gi.params.len()];
        if !self.unify(&gi.ty, ty, &mut b) {
            return None;
        }
        if let Some(targs) = targs {
            for (p, a) in gi.args.iter().zip(targs) {
                if !self.unify(p, a, &mut b) {
                    return None;
                }
            }
        }
        b.into_iter().collect()
    }

    fn gen_impl_holds(&mut self, i: usize, bound: &[Ty], depth: u32) -> Res<bool> {
        let gi = &self.traits.gen_impls[i];
        let (m, params, bounds) = (gi.module, gi.params.clone(), gi.bounds.clone());
        let rb = self.resolve_bounds(m, &params, bound, &bounds)?;
        for (b, a) in rb.iter().zip(bound) {
            for (t2, a2) in b {
                if !self.implements(a, *t2, a2, depth + 1)? {
                    return Ok(false);
                }
            }
        }
        Ok(true)
    }

    /// The argument lists with which `ty` implements trait `t`.
    fn impl_trait_args(&mut self, ty: &Ty, t: u32) -> Res<Vec<Vec<Ty>>> {
        let mut out: Vec<Vec<Ty>> = Vec::new();
        if let Ty::Opaque(o) = ty {
            for (t2, a) in &self.traits.opaques[*o as usize].bounds {
                if *t2 == t && !out.contains(a) {
                    out.push(a.clone());
                }
            }
        }
        let mut found: Vec<(usize, Vec<Ty>)> =
            self.traits.impls.iter().filter(|((t2, _, ty2), _)| *t2 == t && ty2 == ty).map(|((_, a, _), i)| (*i, a.clone())).collect();
        found.sort_by_key(|(i, _)| *i);
        for (_, a) in found {
            if !out.contains(&a) {
                out.push(a);
            }
        }
        for i in 0..self.traits.gen_impls.len() {
            if self.traits.gen_impls[i].trait_id != t {
                continue;
            }
            let Some(bound) = self.match_gen_impl(i, ty, None) else { continue };
            if !self.gen_impl_holds(i, &bound, 0)? {
                continue;
            }
            let pats = self.traits.gen_impls[i].args.clone();
            let b: Vec<Option<Ty>> = bound.into_iter().map(Some).collect();
            let mut a = Vec::new();
            for p in &pats {
                a.push(self.subst(p, &b, Span::default())?.unwrap());
            }
            if !out.contains(&a) {
                out.push(a);
            }
        }
        Ok(out)
    }

    /// A trait method `name` that applies to `ty`: through a bound (for an
    /// opaque parameter) or an impl. For a concrete type only methods with
    /// default bodies are considered (`defaults_only`), since the others are
    /// the type's own methods. Returns the method's generic function and its
    /// known type arguments (the trait's arguments and `Self`).
    pub fn trait_method_for(&mut self, ty: &Ty, name: &str, defaults_only: bool, span: Span) -> Res<Option<(u32, Vec<Option<Ty>>)>> {
        let mut found: Vec<(u32, u32, Vec<Ty>)> = Vec::new();
        for t in 0..self.traits.decls.len() as u32 {
            let Some(gf) = self.traits.methods[t as usize].get(name).copied() else { continue };
            if defaults_only && self.generics.fns[gf as usize].decl.body.is_none() {
                continue;
            }
            for a in self.impl_trait_args(ty, t)? {
                found.push((t, gf, a));
            }
        }
        match found.len() {
            0 => Ok(None),
            1 => {
                let (_, gf, a) = found.pop().unwrap();
                let mut known: Vec<Option<Ty>> = a.into_iter().map(Some).collect();
                known.push(Some(ty.clone()));
                Ok(Some((gf, known)))
            }
            _ => Err(Diag::new(
                span,
                format!(
                    "method `{}` on `{}` is ambiguous: it comes from {}",
                    name,
                    self.ty_name(ty),
                    found.iter().map(|(t, _, a)| format!("`{}`", self.trait_ref_name(*t, a))).collect::<Vec<_>>().join(" and ")
                ),
            )),
        }
    }

    /// The error for a method that an opaque parameter's bounds lack.
    pub fn no_bound_method(&self, o: u32, name: &str, span: Span) -> Diag {
        let info = &self.traits.opaques[o as usize];
        let bounds: Vec<String> = info.bounds.iter().map(|(t, a)| format!("`{}`", self.trait_ref_name(*t, a))).collect();
        Diag::new(
            span,
            format!(
                "no method `{}` on type parameter `{}`: its bounds ({}) do not provide it",
                name,
                info.name,
                if bounds.is_empty() { "none".to_string() } else { bounds.join(", ") }
            ),
        )
    }

    /// Before instance `id` of a generic function is checked: check the
    /// generic body against the bounds of its type parameters, once per
    /// combination of the unbounded parameters' arguments.
    pub fn check_against_bounds(&mut self, id: FnId) -> Res<()> {
        let Some((g, args)) = self.generics.inst_generic.get(&id).cloned() else { return Ok(()) };
        let bounded = self.fn_bounded(g);
        if !bounded.iter().any(|b| *b) {
            return Ok(());
        }
        let key: Vec<Option<Ty>> = args.iter().zip(&bounded).map(|(a, b)| if *b { None } else { Some(a.clone()) }).collect();
        if !self.traits.checked.insert((g, key)) {
            return Ok(());
        }
        let gf = &self.generics.fns[g as usize];
        let (m, decl, params) = (gf.module, gf.decl.clone(), gf.params.clone());
        let mut env_args = args.clone();
        for (i, b) in bounded.iter().enumerate() {
            if *b {
                env_args[i] = self.new_opaque(&params[i]);
            }
        }
        let rb = self.fn_bounds(g, &env_args)?;
        for (a, b) in env_args.iter().zip(rb) {
            if let Ty::Opaque(o) = a {
                self.traits.opaques[*o as usize].bounds = b;
            }
        }
        let env: Vec<(String, Ty)> = params.iter().cloned().zip(env_args.iter().cloned()).collect();
        let name = self.inst_name(&decl, &env_args);
        let scratch = self.prog.fns.len() as FnId;
        self.prog.fns.push(crate::check::placeholder_fn(decl.name.span));
        self.traits.opaque_depth += 1;
        let r = self.with_tenv(env, |c| -> Res<()> {
            c.declare_fn(scratch, m, &decl, Some(format!("{}.{}.bounds", c.module_prefix(m), name)))?;
            c.prog.fns[scratch as usize].name = name.clone();
            c.check_fn_body(scratch, m, &decl, None)
        });
        self.traits.opaque_depth -= 1;
        let f = &mut self.prog.fns[scratch as usize];
        f.body = None;
        f.locals = vec![];
        r
    }
}

/// Does `t` mention `Ty::Param(i)`?
fn mentions_param(t: &Ty, i: u32, g: &crate::generics::Generics) -> bool {
    match t {
        Ty::Param(j) => *j == i,
        Ty::Ptr(x) | Ty::Slice(x) | Ty::Array(x, _) | Ty::Option(x) | Ty::Gc(x) => mentions_param(x, i, g),
        Ty::Result(a, b) => mentions_param(a, i, g) || mentions_param(b, i, g),
        Ty::Tuple(ts) => ts.iter().any(|t| mentions_param(t, i, g)),
        Ty::Fn(ps, r, _) | Ty::Block(ps, r, _) => ps.iter().any(|t| mentions_param(t, i, g)) || mentions_param(r, i, g),
        Ty::Struct(_) | Ty::Enum(_) => match g.inst_of.get(t) {
            Some((_, args)) => args.iter().any(|t| mentions_param(t, i, g)),
            None => false,
        },
        _ => false,
    }
}
