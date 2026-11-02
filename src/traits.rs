//! Static traits: `trait` declarations (method signatures over `Self`),
//! `impl Trait for Type;` assertions, and bounds on type parameters, checked
//! when a concrete instance is created. Calls are not affected: methods
//! still resolve per instance, and dispatch is static.

use std::collections::HashMap;

use crate::ast::*;
use crate::check::{effect_name, effects_of, Checker, Def};
use crate::diag::{Diag, Res, Span};
use crate::tir::*;

#[derive(Default)]
pub struct Traits {
    /// Declaring module and declaration, indexed by trait id.
    pub decls: Vec<(usize, TraitDecl)>,
    /// `impl` items with their module, in source order.
    pub impl_decls: Vec<(usize, ImplDecl)>,
    /// (trait, type) → index into `impl_decls`.
    pub impls: HashMap<(u32, Ty), usize>,
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

    pub fn trait_name(&self, t: u32) -> String {
        self.traits.decls[t as usize].1.name.name.clone()
    }

    /// Record every `impl` (before type definitions, so that instances
    /// created while resolving them can be checked), and check that bounds
    /// name traits.
    pub fn register_impls(&mut self) -> Res<()> {
        for i in 0..self.traits.impl_decls.len() {
            let (m, d) = self.traits.impl_decls[i].clone();
            let t = self.resolve_trait(m, &d.trait_path)?;
            let ty = self.resolve_type(m, &d.ty)?;
            if self.generics.inst_of.contains_key(&ty) {
                return Err(Diag::new(d.ty.span, "`impl` for an instance of a generic type is not supported yet"));
            }
            if self.traits.impls.insert((t, ty.clone()), i).is_some() {
                return Err(Diag::new(
                    d.span,
                    format!("`{}` already implements `{}`", self.ty_name(&ty), self.trait_name(t)),
                ));
            }
        }
        let mut all: Vec<(usize, Vec<Bounds>)> = Vec::new();
        for gt in &self.generics.types {
            all.push((gt.module, gt.bounds.clone()));
        }
        for gf in &self.generics.fns {
            all.push((gf.module, gf.decl.bounds.clone()));
        }
        for (m, bounds) in all {
            for b in bounds.iter().flatten() {
                self.resolve_trait(m, b)?;
            }
        }
        Ok(())
    }

    /// After all signatures are declared: every impl provides each trait
    /// method as `fn Type.name` with the trait's signature (`Self` = the
    /// type) and no more effects than the trait allows.
    pub fn check_impls(&mut self) -> Res<()> {
        let mut impls: Vec<((u32, Ty), usize)> = self.traits.impls.iter().map(|(k, v)| (k.clone(), *v)).collect();
        impls.sort_by_key(|(_, i)| *i);
        for ((t, ty), i) in impls {
            let span = self.traits.impl_decls[i].1.span;
            let (tm, decl) = self.traits.decls[t as usize].clone();
            for f in &decl.methods {
                let env = vec![("Self".to_string(), ty.clone())];
                let (params, ret) = self.with_tenv(env, |c| -> Res<(Vec<Ty>, Ty)> {
                    let mut ps = Vec::new();
                    for p in &f.params {
                        ps.push(c.resolve_type(tm, &p.ty)?);
                    }
                    let r = match &f.ret {
                        Some(r) => c.resolve_type(tm, r)?,
                        None => Ty::Unit,
                    };
                    Ok((ps, r))
                })?;
                let want = Ty::Fn(params, Box::new(ret), effects_of(&f.attrs)?);
                let tname = self.ty_name(&ty);
                let what = format!("`{}` does not implement `{}`", tname, decl.name.name);
                let Some(fid) = self.methods.get(&(ty.clone(), f.name.name.clone())).copied() else {
                    return Err(Diag::new(span, format!("{}: no method `{}.{}`", what, tname, f.name.name)));
                };
                let have = self.prog.fns[fid as usize].fn_ty();
                let (Ty::Fn(p1, r1, e1), Ty::Fn(p2, r2, e2)) = (&have, &want) else { unreachable!() };
                if p1 != p2 || r1 != r2 {
                    return Err(Diag::new(
                        span,
                        format!(
                            "{}: `{}.{}` has type `{}`, but the trait needs `{}`",
                            what,
                            tname,
                            f.name.name,
                            self.ty_name(&have),
                            self.ty_name(&want)
                        ),
                    ));
                }
                if e1 & !e2 != 0 {
                    return Err(Diag::new(
                        span,
                        format!("{}: `{}.{}` uses {}, which the trait does not allow", what, tname, f.name.name, effect_name(e1 & !e2)),
                    ));
                }
            }
        }
        Ok(())
    }

    /// Check the bounds of a generic item against concrete type arguments.
    /// Instances that still contain type parameters are checked later, when
    /// they are instantiated with concrete types.
    pub fn check_bounds(&self, module: usize, bounds: &[Bounds], args: &[Ty], needed_by: impl Fn() -> String, span: Span) -> Res<()> {
        for (b, a) in bounds.iter().zip(args) {
            if b.is_empty() || crate::generics::contains_param(a, &self.generics) {
                continue;
            }
            for path in b {
                let t = self.resolve_trait(module, path)?;
                if !self.traits.impls.contains_key(&(t, a.clone())) {
                    return Err(Diag::new(
                        span,
                        format!("`{}` does not implement `{}` (needed by `{}`)", self.ty_name(a), self.trait_name(t), needed_by()),
                    ));
                }
            }
        }
        Ok(())
    }
}
