//! Generics by monomorphization: instantiation of generic structs, enums,
//! functions and methods, and inference of type arguments.

use std::collections::HashMap;

use crate::ast::*;
use crate::check::{has_hole, mk, Checker, FnCx, HOLE};
use crate::diag::{Diag, Res, Span};
use crate::tir::*;

#[derive(Clone)]
pub enum GenDecl {
    Struct(StructDecl),
    Enum(EnumDecl),
}

pub struct GenericType {
    pub module: usize,
    pub name: String,
    pub params: Vec<String>,
    pub bounds: Vec<Bounds>,
    pub decl: GenDecl,
}

pub struct GenericFn {
    pub module: usize,
    pub decl: FnDecl,
    pub params: Vec<String>,
}

#[derive(Default)]
pub struct Generics {
    pub types: Vec<GenericType>,
    pub type_inst: HashMap<(u32, Vec<Ty>), Ty>,
    /// Instance type → (generic type, arguments).
    pub inst_of: HashMap<Ty, (u32, Vec<Ty>)>,
    pub fns: Vec<GenericFn>,
    pub fn_inst: HashMap<(u32, Vec<Ty>), FnId>,
    /// (generic type, method name) → generic fn.
    pub methods: HashMap<(u32, String), u32>,
    /// (non-generic type, method name) → generic fn: methods with their
    /// own type parameters on a plain type (`fn Cursor.op<K>`).
    pub ty_methods: HashMap<(Ty, String), u32>,
    /// Instance of a generic function → where it was first needed.
    pub fn_site: HashMap<FnId, Span>,
    /// Concrete instance of a generic function → (generic fn, arguments).
    pub inst_generic: HashMap<FnId, (u32, Vec<Ty>)>,
}

/// Does `t` contain an inference variable or an opaque type parameter
/// (making an instance of it a pattern that is never emitted)?
pub fn contains_param(t: &Ty, g: &Generics) -> bool {
    match t {
        Ty::Param(_) | Ty::Opaque(_) => true,
        Ty::Ptr(x) | Ty::Slice(x) | Ty::Array(x, _) | Ty::Option(x) | Ty::Gc(x) => contains_param(x, g),
        Ty::Result(a, b) => contains_param(a, g) || contains_param(b, g),
        Ty::Tuple(ts) => ts.iter().any(|t| contains_param(t, g)),
        Ty::Fn(ps, r, _) | Ty::Block(ps, r, _) | Ty::Closure(ps, r, _) => ps.iter().any(|t| contains_param(t, g)) || contains_param(r, g),
        Ty::Struct(_) | Ty::Enum(_) => match g.inst_of.get(t) {
            Some((_, args)) => args.iter().any(|t| contains_param(t, g)),
            None => false,
        },
        _ => false,
    }
}

/// Does `t` contain an inference variable (`Ty::Param`), as opposed to
/// only opaque type parameters?
pub fn contains_infer(t: &Ty, g: &Generics) -> bool {
    match t {
        Ty::Param(_) => true,
        Ty::Ptr(x) | Ty::Slice(x) | Ty::Array(x, _) | Ty::Option(x) | Ty::Gc(x) => contains_infer(x, g),
        Ty::Result(a, b) => contains_infer(a, g) || contains_infer(b, g),
        Ty::Tuple(ts) => ts.iter().any(|t| contains_infer(t, g)),
        Ty::Fn(ps, r, _) | Ty::Block(ps, r, _) | Ty::Closure(ps, r, _) => ps.iter().any(|t| contains_infer(t, g)) || contains_infer(r, g),
        Ty::Struct(_) | Ty::Enum(_) => match g.inst_of.get(t) {
            Some((_, args)) => args.iter().any(|t| contains_infer(t, g)),
            None => false,
        },
        _ => false,
    }
}

/// `new(a, |..| ..)`: an escaping closure literal.
fn is_new_closure(a: &Expr) -> bool {
    matches!(&a.kind, ExprKind::Call(f, args) if matches!(&f.kind, ExprKind::Ident(n) if n == "new")
        && args.len() == 2 && matches!(args[1].kind, ExprKind::Closure(..)))
}

/// Does `t` contain a `block fn` type (which may not be a type argument)?
pub fn contains_block(t: &Ty) -> bool {
    match t {
        Ty::Block(..) => true,
        Ty::Ptr(x) | Ty::Slice(x) | Ty::Array(x, _) | Ty::Option(x) | Ty::Gc(x) => contains_block(x),
        Ty::Result(a, b) => contains_block(a) || contains_block(b),
        Ty::Tuple(ts) => ts.iter().any(contains_block),
        Ty::Fn(ps, r, _) | Ty::Closure(ps, r, _) => ps.iter().any(contains_block) || contains_block(r),
        _ => false,
    }
}

impl<'a> Checker<'a> {
    /// Run `f` with type parameters bound.
    pub fn with_tenv<T>(&mut self, env: Vec<(String, Ty)>, f: impl FnOnce(&mut Self) -> T) -> T {
        let saved = std::mem::replace(&mut self.tenv, env);
        let r = f(self);
        self.tenv = saved;
        r
    }

    pub fn instantiate_type(&mut self, g: u32, args: Vec<Ty>, span: Span) -> Res<Ty> {
        let gt = &self.generics.types[g as usize];
        if args.len() != gt.params.len() {
            return Err(Diag::new(span, format!("`{}` takes {} type argument(s)", gt.name, gt.params.len())));
        }
        if let Some(t) = self.generics.type_inst.get(&(g, args.clone())) {
            return Ok(t.clone());
        }
        let module = gt.module;
        let gname = gt.name.clone();
        if let Some(a) = args.iter().find(|a| contains_block(a)) {
            return Err(Diag::new(span, format!("a `block fn` cannot be a type argument (`{}` in `{}`)", self.ty_name(a), gname)));
        }
        let bounds = self.type_bounds(g, &args)?;
        let needed_by = format!("{}<{}>", gname, args.iter().map(|a| self.ty_name(a)).collect::<Vec<_>>().join(", "));
        self.check_bounds(&bounds, &args, needed_by, span)?;
        if let Some(t) = self.generics.type_inst.get(&(g, args.clone())) {
            return Ok(t.clone());
        }
        let gt = &self.generics.types[g as usize];
        let pattern = args.iter().any(|a| contains_param(a, &self.generics));
        let name = format!(
            "{}.{}<{}>",
            self.module_prefix(module),
            gt.name,
            args.iter().map(|a| self.ty_name(a)).collect::<Vec<_>>().join(", ")
        );
        let env: Vec<(String, Ty)> = gt.params.iter().cloned().zip(args.iter().cloned()).collect();
        let decl = gt.decl.clone();
        let ty = match &decl {
            GenDecl::Struct(s) => {
                let id = self.prog.structs.len() as u32;
                // Every concrete instance of a generic `object` is an
                // object type of its own, with its own type id (and so its
                // own tracing code). Pattern instances are never allocated.
                let object = match (s.is_object, pattern) {
                    (false, _) => None,
                    (true, true) => Some(u32::MAX),
                    (true, false) => {
                        self.object_count += 1;
                        Some(self.object_count - 1)
                    }
                };
                self.prog.structs.push(StructDef { name, fields: vec![], pattern, object, trailing: None, verified: None });
                Ty::Struct(id)
            }
            GenDecl::Enum(_) => {
                let id = self.prog.enums.len() as u32;
                self.prog.enums.push(EnumDef { name, tag: IntTy::I32, variants: vec![] });
                Ty::Enum(id)
            }
        };
        self.generics.type_inst.insert((g, args.clone()), ty.clone());
        self.generics.inst_of.insert(ty.clone(), (g, args));
        let inst_name = self.ty_name(&ty);
        let note = |d: Diag| if d.span == span { d } else { d.note(span, format!("in `{}`, instantiated here", inst_name)) };
        match decl {
            GenDecl::Struct(s) => {
                let (fields, trailing, verified) = self.with_tenv(env, |c| c.struct_fields(module, &s)).map_err(note)?;
                let Ty::Struct(id) = ty else { unreachable!() };
                self.prog.structs[id as usize].fields = fields;
                self.prog.structs[id as usize].trailing = trailing;
                self.prog.structs[id as usize].verified = verified;
            }
            GenDecl::Enum(e) => {
                let (tag, variants) = self.with_tenv(env, |c| c.enum_variants(module, &e)).map_err(note)?;
                let Ty::Enum(id) = ty else { unreachable!() };
                self.prog.enums[id as usize].tag = tag;
                self.prog.enums[id as usize].variants = variants;
            }
        }
        Ok(ty)
    }

    /// The display name of an instance: `f<i64>`, `Vec<u8>.push`, `Box<T>.map<U>`.
    pub fn inst_name(&self, decl: &FnDecl, args: &[Ty]) -> String {
        let names: Vec<String> = args.iter().map(|a| self.ty_name(a)).collect();
        match &decl.recv {
            Some(r) => {
                let (ra, ma) = names.split_at(decl.recv_generics);
                let mut b = r.name.clone();
                if !ra.is_empty() {
                    b = format!("{}<{}>", b, ra.join(", "));
                }
                b = format!("{}.{}", b, decl.name.name);
                if !ma.is_empty() {
                    b = format!("{}<{}>", b, ma.join(", "));
                }
                b
            }
            None => format!("{}<{}>", decl.name.name, names.join(", ")),
        }
    }

    /// The instance of generic function `g` for `args`. An instance over
    /// opaque type parameters (while a generic body is checked against its
    /// bounds) only has a signature: it is never checked or emitted.
    pub fn instantiate_fn(&mut self, g: u32, args: Vec<Ty>, span: Span) -> Res<FnId> {
        if let Some(f) = self.generics.fn_inst.get(&(g, args.clone())) {
            return Ok(*f);
        }
        let gf = &self.generics.fns[g as usize];
        let module = gf.module;
        let decl = gf.decl.clone();
        let env: Vec<(String, Ty)> = gf.params.iter().cloned().zip(args.iter().cloned()).collect();
        let base = self.inst_name(&decl, &args);
        if let Some(a) = args.iter().find(|a| contains_block(a)) {
            return Err(Diag::new(span, format!("a `block fn` cannot be a type argument (`{}` in `{}`)", self.ty_name(a), base)));
        }
        let bounds = self.fn_bounds(g, &args)?;
        self.check_bounds(&bounds, &args, base.clone(), span)?;
        let pattern = args.iter().any(|a| contains_param(a, &self.generics));
        let id = self.prog.fns.len() as FnId;
        self.prog.fns.push(crate::check::placeholder_fn(span));
        self.generics.fn_inst.insert((g, args.clone()), id);
        let symbol = format!("{}.{}", self.module_prefix(module), base);
        self.prog.fns[id as usize].name = base.clone();
        let saved = std::mem::replace(&mut self.sig_only, pattern);
        let r = self.with_tenv(env.clone(), |c| c.declare_fn(id, module, &decl, Some(symbol)));
        self.sig_only = saved;
        r.map_err(|d| d.note(span, format!("in `{}`, instantiated here", base)))?;
        self.prog.fns[id as usize].name = base;
        if !pattern {
            self.generics.fn_site.insert(id, span);
            self.generics.inst_generic.insert(id, (g, args));
            self.pending.push((id, module, decl, env));
        }
        Ok(id)
    }

    /// Resolve a type with the given generic parameters as inference variables.
    fn pattern_type(&mut self, module: usize, params: &[String], t: &TypeExpr) -> Res<Ty> {
        let env = params.iter().enumerate().map(|(i, p)| (p.clone(), Ty::Param(i as u32))).collect();
        self.with_tenv(env, |c| c.resolve_param_type(module, t))
    }

    pub fn unify(&self, pat: &Ty, act: &Ty, b: &mut [Option<Ty>]) -> bool {
        match (pat, act) {
            // A hole in the actual type (an unknown part of a partly
            // inferred expected type) matches anything and binds nothing.
            (_, Ty::Param(HOLE)) => true,
            (Ty::Param(_), _) if has_hole(act) => true,
            (Ty::Param(i), _) => match &b[*i as usize] {
                Some(t) => t == act,
                None => {
                    b[*i as usize] = Some(act.clone());
                    true
                }
            },
            (Ty::Ptr(x), Ty::Ptr(y)) | (Ty::Slice(x), Ty::Slice(y)) | (Ty::Option(x), Ty::Option(y)) | (Ty::Gc(x), Ty::Gc(y)) => {
                self.unify(x, y, b)
            }
            (Ty::Array(x, n), Ty::Array(y, m)) => n == m && self.unify(x, y, b),
            (Ty::Result(a1, b1), Ty::Result(a2, b2)) => self.unify(a1, a2, b) && self.unify(b1, b2, b),
            (Ty::Tuple(xs), Ty::Tuple(ys)) => xs.len() == ys.len() && xs.iter().zip(ys).all(|(x, y)| self.unify(x, y, b)),
            // Effects are not unified: the argument is coerced to the
            // instantiated parameter type afterwards, which checks them.
            (Ty::Fn(p1, r1, _), Ty::Fn(p2, r2, _))
            | (Ty::Block(p1, r1, _), Ty::Block(p2, r2, _))
            | (Ty::Closure(p1, r1, _), Ty::Closure(p2, r2, _)) => {
                p1.len() == p2.len() && p1.iter().zip(p2).all(|(x, y)| self.unify(x, y, b)) && self.unify(r1, r2, b)
            }
            (Ty::Struct(_), Ty::Struct(_)) | (Ty::Enum(_), Ty::Enum(_)) if pat != act => {
                match (self.generics.inst_of.get(pat), self.generics.inst_of.get(act)) {
                    (Some((g1, a1)), Some((g2, a2))) if g1 == g2 => {
                        let (a1, a2) = (a1.clone(), a2.clone());
                        a1.iter().zip(a2.iter()).all(|(x, y)| self.unify(x, y, b))
                    }
                    _ => false,
                }
            }
            _ => pat == act,
        }
    }

    pub fn subst(&mut self, t: &Ty, b: &[Option<Ty>], span: Span) -> Res<Option<Ty>> {
        Ok(Some(match t {
            Ty::Param(i) => match &b[*i as usize] {
                Some(t) => t.clone(),
                None => return Ok(None),
            },
            Ty::Ptr(x) => match self.subst(x, b, span)? {
                Some(x) => Ty::Ptr(Box::new(x)),
                None => return Ok(None),
            },
            Ty::Slice(x) => match self.subst(x, b, span)? {
                Some(x) => Ty::Slice(Box::new(x)),
                None => return Ok(None),
            },
            Ty::Option(x) => match self.subst(x, b, span)? {
                Some(x) => Ty::Option(Box::new(x)),
                None => return Ok(None),
            },
            Ty::Gc(x) => match self.subst(x, b, span)? {
                Some(x) => {
                    if !matches!(&x, Ty::Closure(..)) && !matches!(&x, Ty::Struct(i) if self.prog.structs[*i as usize].object.is_some()) {
                        return Err(Diag::new(span, format!("Gc<T> needs an `object` type, found `{}`", self.ty_name(&x))));
                    }
                    Ty::Gc(Box::new(x))
                }
                None => return Ok(None),
            },
            Ty::Array(x, n) => match self.subst(x, b, span)? {
                Some(x) => Ty::Array(Box::new(x), *n),
                None => return Ok(None),
            },
            Ty::Result(x, y) => match (self.subst(x, b, span)?, self.subst(y, b, span)?) {
                (Some(x), Some(y)) => Ty::Result(Box::new(x), Box::new(y)),
                _ => return Ok(None),
            },
            Ty::Tuple(ts) => {
                let mut out = Vec::new();
                for t in ts {
                    match self.subst(t, b, span)? {
                        Some(t) => out.push(t),
                        None => return Ok(None),
                    }
                }
                Ty::Tuple(out)
            }
            Ty::Fn(ps, r, e) | Ty::Block(ps, r, e) | Ty::Closure(ps, r, e) => {
                let mut out = Vec::new();
                for t in ps {
                    match self.subst(t, b, span)? {
                        Some(t) => out.push(t),
                        None => return Ok(None),
                    }
                }
                match self.subst(r, b, span)? {
                    Some(r) if matches!(t, Ty::Block(..)) => Ty::Block(out, Box::new(r), *e),
                    Some(r) if matches!(t, Ty::Closure(..)) => Ty::Closure(out, Box::new(r), *e),
                    Some(r) => Ty::Fn(out, Box::new(r), *e),
                    None => return Ok(None),
                }
            }
            Ty::Struct(_) | Ty::Enum(_) if contains_param(t, &self.generics) => {
                let (g, args) = self.generics.inst_of.get(t).cloned().unwrap();
                let mut out = Vec::new();
                for a in &args {
                    match self.subst(a, b, span)? {
                        Some(a) => out.push(a),
                        None => return Ok(None),
                    }
                }
                self.instantiate_type(g, out, span)?
            }
            t => t.clone(),
        }))
    }

    /// `t` with the bound parameters substituted and the unbound ones
    /// left as holes: the partly known type of a generic call's argument.
    fn subst_partial(&mut self, t: &Ty, b: &[Option<Ty>], span: Span) -> Res<Ty> {
        Ok(self.subst_holes(t, b, span)?.unwrap_or(Ty::Param(HOLE)))
    }

    fn subst_holes(&mut self, t: &Ty, b: &[Option<Ty>], span: Span) -> Res<Option<Ty>> {
        Ok(Some(match t {
            Ty::Param(i) => b[*i as usize].clone().unwrap_or(Ty::Param(HOLE)),
            Ty::Ptr(x) => Ty::Ptr(Box::new(self.subst_holes(x, b, span)?.unwrap_or(Ty::Param(HOLE)))),
            Ty::Slice(x) => Ty::Slice(Box::new(self.subst_holes(x, b, span)?.unwrap_or(Ty::Param(HOLE)))),
            Ty::Option(x) => Ty::Option(Box::new(self.subst_holes(x, b, span)?.unwrap_or(Ty::Param(HOLE)))),
            Ty::Array(x, k) => Ty::Array(Box::new(self.subst_holes(x, b, span)?.unwrap_or(Ty::Param(HOLE))), *k),
            Ty::Tuple(ts) => {
                let mut out = Vec::new();
                for t in ts {
                    out.push(self.subst_holes(t, b, span)?.unwrap_or(Ty::Param(HOLE)));
                }
                Ty::Tuple(out)
            }
            Ty::Fn(ps, r, e) | Ty::Block(ps, r, e) | Ty::Closure(ps, r, e) => {
                let mut out = Vec::new();
                for t in ps {
                    out.push(self.subst_holes(t, b, span)?.unwrap_or(Ty::Param(HOLE)));
                }
                let r = Box::new(self.subst_holes(r, b, span)?.unwrap_or(Ty::Param(HOLE)));
                match t {
                    Ty::Block(..) => Ty::Block(out, r, *e),
                    Ty::Closure(..) => Ty::Closure(out, r, *e),
                    _ => Ty::Fn(out, r, *e),
                }
            }
            Ty::Gc(x) if matches!(**x, Ty::Closure(..)) => Ty::Gc(Box::new(self.subst_holes(x, b, span)?.unwrap_or(Ty::Param(HOLE)))),
            t if contains_param(t, &self.generics) => {
                // Gc<..>, Result<..> and generic instances: only when fully known.
                let full: Vec<Option<Ty>> = b.iter().map(|x| x.clone().filter(|t| !has_hole(t))).collect();
                return self.subst(t, &full, span);
            }
            t => t.clone(),
        }))
    }

    /// Generic fn `g` used as a value (not called): its type arguments are
    /// inferred from the expected function type, e.g. a `fn(*P) -> K`
    /// parameter or field.
    pub fn generic_fn_value(&mut self, g: u32, exp: Option<&Ty>, span: Span) -> Res<TExpr> {
        let gf = &self.generics.fns[g as usize];
        let (module, params, decl) = (gf.module, gf.params.clone(), gf.decl.clone());
        let Some(want @ Ty::Fn(..)) = exp else {
            return Err(Diag::new(
                span,
                format!("cannot infer the type arguments of generic function `{}`: use it where a function type is expected", decl.name.name),
            ));
        };
        let mut ptys = Vec::new();
        for p in &decl.params {
            ptys.push(self.pattern_type(module, &params, &p.ty)?);
        }
        let ret = match &decl.ret {
            Some(t) => self.pattern_type(module, &params, t)?,
            None => Ty::Unit,
        };
        let mut b: Vec<Option<Ty>> = vec![None; params.len()];
        if !self.unify(&Ty::Fn(ptys, Box::new(ret), 0), want, &mut b) {
            return Err(Diag::new(span, format!("`{}` does not match the expected type `{}`", decl.name.name, self.ty_name(want))));
        }
        let mut targs = Vec::new();
        for (i, t) in b.into_iter().enumerate() {
            match t {
                Some(t) => targs.push(t),
                None => return Err(Diag::new(span, format!("cannot infer type parameter `{}`", params[i]))),
            }
        }
        let fid = self.instantiate_fn(g, targs, span)?;
        let f = &self.prog.fns[fid as usize];
        Ok(mk(f.fn_ty(), TK::FnRef(fid), span))
    }

    /// Call of generic fn `g`. `pre` are already-checked leading arguments
    /// (a method receiver); `known` fixes some type parameters up front.
    pub fn generic_call(
        &mut self,
        cx: &mut FnCx,
        g: u32,
        pre: Vec<TExpr>,
        known: Vec<Option<Ty>>,
        args: &[Expr],
        exp: Option<&Ty>,
        span: Span,
    ) -> Res<TExpr> {
        self.check_unsafe_def(cx, &crate::check::Def::GenericFn(g), span)?;
        let gf = &self.generics.fns[g as usize];
        let module = gf.module;
        let params = gf.params.clone();
        let decl = gf.decl.clone();
        let mut b: Vec<Option<Ty>> = known;
        b.resize(params.len(), None);
        let mut ptys = Vec::new();
        for p in &decl.params {
            ptys.push(self.pattern_type(module, &params, &p.ty)?);
        }
        let ret = match &decl.ret {
            Some(t) => self.pattern_type(module, &params, t)?,
            None => Ty::Unit,
        };
        let n = pre.len() + args.len();
        if n != ptys.len() {
            return Err(Diag::new(span, format!("expected {} argument(s), got {}", ptys.len(), n)));
        }
        if let Some(e) = exp {
            let mut tmp = b.clone();
            if self.unify(&ret, e, &mut tmp) {
                b = tmp;
            }
        }
        let mut vals: Vec<TExpr> = Vec::new();
        for (i, r) in pre.into_iter().enumerate() {
            let mut r = r;
            // Auto-reference/dereference the receiver.
            let pt = &ptys[i];
            let mut tmp = b.clone();
            if !self.unify(pt, &r.ty, &mut tmp) {
                if let Ty::Ptr(_) = pt {
                    let s = r.span;
                    let t = Ty::Ptr(Box::new(r.ty.clone()));
                    r = mk(t, TK::AddrOf(Box::new(r)), s);
                } else if let Ty::Ptr(inner) = r.ty.clone() {
                    let s = r.span;
                    r = mk(*inner, TK::Deref(Box::new(r)), s);
                }
            }
            if !self.unify(pt, &r.ty, &mut b) {
                return Err(Diag::new(r.span, format!("receiver type mismatch for `{}`", decl.name.name)));
            }
            vals.push(r);
        }
        let off = vals.len();
        // A generic function passed as an argument gets its type arguments
        // from the parameter's type, which may depend on later arguments:
        // check those first (`apply(double, 21)` with `fn apply<T, U>(f:
        // fn(T) -> U, x: T)`).
        let mut order: Vec<usize> = Vec::new();
        let mut deferred: Vec<usize> = Vec::new();
        for (i, a) in args.iter().enumerate() {
            let pt = &ptys[off + i];
            let generic_fn = matches!(pt, Ty::Fn(..) | Ty::Block(..)) && self.generic_fn_named(cx, a)?.is_some();
            // Closures too: their parameter types may come from later arguments.
            let closure = matches!(pt, Ty::Block(..)) && matches!(a.kind, ExprKind::Closure(..))
                || matches!(crate::check::callee_sig(pt), Ty::Closure(..)) && is_new_closure(a);
            if contains_param(pt, &self.generics) && (generic_fn || closure) {
                deferred.push(i);
            } else {
                order.push(i);
            }
        }
        order.extend(deferred.iter().copied());
        let mut slots: Vec<Option<TExpr>> = vec![None; args.len()];
        for i in order {
            let a = &args[i];
            let pt = ptys[off + i].clone();
            let want = self.subst(&pt, &b, span)?;
            let v = if matches!(pt, Ty::Block(..)) {
                // A `block fn` parameter, maybe only partly known.
                let w = match &want {
                    Some(w) => w.clone(),
                    None => self.subst_partial(&pt, &b, span)?,
                };
                self.block_arg(cx, a, &w)?
            } else if want.is_none() && deferred.contains(&i) && is_new_closure(a) {
                // `new(a, |..| ..)` for a partly known `closure fn` parameter.
                let partial = self.subst_partial(&pt, &b, span)?;
                self.expr(cx, a, Some(&partial))?
            } else if want.is_none() && deferred.contains(&i) {
                // Only partly known: the parameters may still determine
                // the function's type arguments.
                let partial = self.subst_partial(&pt, &b, span)?;
                let (g, _) = self.generic_fn_named(cx, a)?.unwrap();
                self.generic_fn_value(g, Some(&partial), a.span)?
            } else {
                self.expr(cx, a, want.as_ref())?
            };
            let v = match &want {
                Some(t) => self.coerce(v, t)?,
                None => {
                    if !self.unify(&pt, &v.ty, &mut b) {
                        return Err(Diag::new(a.span, format!("argument of type `{}` does not match", self.ty_name(&v.ty))));
                    }
                    v
                }
            };
            slots[i] = Some(v);
        }
        vals.extend(slots.into_iter().map(|v| v.unwrap()));
        let mut targs = Vec::new();
        for (i, t) in b.iter().enumerate() {
            match t {
                Some(t) => targs.push(t.clone()),
                None => return Err(Diag::new(span, format!("cannot infer type parameter `{}`", params[i]))),
            }
        }
        let fid = self.instantiate_fn(g, targs, span)?;
        let f = &self.prog.fns[fid as usize];
        let fty = f.fn_ty();
        let ret = f.ret.clone();
        let pts = f.param_tys.clone();
        let mut out = Vec::new();
        for (v, t) in vals.into_iter().zip(pts.iter()) {
            out.push(self.coerce(v, t)?);
        }
        let call = mk(ret, TK::Call(Box::new(mk(fty, TK::FnRef(fid), span)), out), span);
        crate::vmcheck::wrap_effects(self, cx, call)
    }

    /// Infer the instance of generic type `g` from the expected type.
    pub fn instance_from_expected(&self, g: u32, exp: Option<&Ty>) -> Option<Ty> {
        let e = exp?;
        let e = match e {
            Ty::Option(inner) if !self.generics.inst_of.contains_key(e) => &**inner,
            e => e,
        };
        match self.generics.inst_of.get(e) {
            Some((g2, _)) if *g2 == g => Some(e.clone()),
            _ => None,
        }
    }

    /// Struct literal of a generic struct: infer arguments from the fields.
    pub fn generic_struct_lit(&mut self, cx: &mut FnCx, g: u32, fields: &[(Ident, Expr)], exp: Option<&Ty>, span: Span) -> Res<Ty> {
        if let Some(t) = self.instance_from_expected(g, exp) {
            return Ok(t);
        }
        let gt = &self.generics.types[g as usize];
        let module = gt.module;
        let params = gt.params.clone();
        let GenDecl::Struct(sd) = gt.decl.clone() else {
            return Err(Diag::new(span, format!("`{}` is not a struct", gt.name)));
        };
        let mut b: Vec<Option<Ty>> = vec![None; params.len()];
        for (f, e) in fields {
            let Some((_, ft)) = sd.fields.iter().find(|(n, _)| n.name == f.name) else { continue };
            let pt = self.pattern_type(module, &params, ft)?;
            if !contains_param(&pt, &self.generics) {
                continue;
            }
            // Check a clone to learn the type; the literal is checked again later.
            let mut probe = FnCx::new(cx.module, cx.ret.clone());
            probe.scopes = cx.scopes.clone();
            probe.locals = cx.locals.clone();
            probe.vm = cx.vm.clone();
            if let Ok(v) = self.expr(&mut probe, e, None) {
                self.unify(&pt, &v.ty, &mut b);
            }
        }
        let mut targs = Vec::new();
        for (i, t) in b.into_iter().enumerate() {
            match t {
                Some(t) => targs.push(t),
                None => return Err(Diag::new(span, format!("cannot infer type parameter `{}`", params[i]))),
            }
        }
        self.instantiate_type(g, targs, span)
    }
}
