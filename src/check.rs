//! Name resolution, type checking, and elaboration into TIR.

use std::collections::HashMap;

use crate::ast::*;
use crate::diag::{Diag, Res, Span};
use crate::tir::*;

#[derive(Clone, Debug)]
pub enum Def {
    Struct(u32),
    Enum(u32),
    Value(u32),
    Fn(FnId),
    Const(u32),
    Global(u32),
    Module(usize),
    Vm(u32),
    GenericType(u32),
    GenericFn(u32),
    Trait(u32),
    /// `type Name = T;` (index into `Checker::aliases`).
    Alias(u32),
}

/// A type alias. Non-generic aliases are resolved once, before type
/// definitions (`ty`); generic ones at each use, with their arguments.
pub struct AliasInfo {
    pub module: usize,
    pub decl: TypeAliasDecl,
    pub ty: Option<Ty>,
    pub busy: bool,
}

pub struct ConstInfo {
    pub module: usize,
    pub decl: ConstDecl,
    pub value: Option<TExpr>,
    pub busy: bool,
}

#[derive(Clone)]
pub struct LoopCx {
    pub id: u32,
    pub is_loop: bool,
    pub break_ty: Option<Ty>,
}

/// Per-function checking state.
pub struct FnCx {
    pub module: usize,
    pub locals: Vec<LocalDef>,
    pub scopes: Vec<Vec<(String, LocalId)>>,
    pub ret: Ty,
    pub loops: Vec<LoopCx>,
    pub next_loop: u32,
    pub vm: Option<crate::vmcheck::HandlerCx>,
    pub effects: u8,
    pub fn_name: String,
    /// Nesting depth of `unsafe { }` blocks.
    pub unsafe_depth: u32,
    /// The next name lookup may yield a `block fn` local: it is a callee
    /// or an argument for a `block fn` parameter.
    pub block_use: bool,
    /// Inside a closure body: the closure's type (for messages).
    pub closure_ty: Option<Ty>,
}

impl FnCx {
    pub fn new(module: usize, ret: Ty) -> FnCx {
        FnCx {
            module,
            locals: vec![],
            scopes: vec![vec![]],
            ret,
            loops: vec![],
            next_loop: 0,
            vm: None,
            effects: 0,
            fn_name: String::new(),
            unsafe_depth: 0,
            block_use: false,
            closure_ty: None,
        }
    }
    pub fn declare(&mut self, name: &str, ty: Ty, mutable: bool) -> LocalId {
        let id = self.locals.len() as LocalId;
        self.locals.push(LocalDef { name: name.to_string(), ty, mutable, field: None });
        self.scopes.last_mut().unwrap().push((name.to_string(), id));
        id
    }
    /// A copy for speculative checking: the original is unaffected.
    pub fn probe(&self) -> FnCx {
        FnCx {
            module: self.module,
            locals: self.locals.clone(),
            scopes: self.scopes.clone(),
            ret: self.ret.clone(),
            loops: self.loops.clone(),
            next_loop: self.next_loop,
            vm: self.vm.clone(),
            effects: self.effects,
            fn_name: self.fn_name.clone(),
            unsafe_depth: self.unsafe_depth,
            block_use: false,
            closure_ty: self.closure_ty.clone(),
        }
    }
    /// A local not visible by name.
    pub fn temp(&mut self, ty: Ty) -> LocalId {
        let id = self.locals.len() as LocalId;
        self.locals.push(LocalDef { name: format!("tmp{}", id), ty, mutable: true, field: None });
        id
    }
    pub fn lookup(&self, name: &str) -> Option<LocalId> {
        for s in self.scopes.iter().rev() {
            for (n, id) in s.iter().rev() {
                if n == name {
                    return Some(*id);
                }
            }
        }
        None
    }
}

pub struct Checker<'a> {
    pub prog: Program,
    pub modules: &'a [Module],
    /// Per module, the items by name: types, modules, vms, traits, and
    /// values (functions, consts, globals) whose name is not also one of
    /// those. This is what paths (`m.T`, `T.f`) and type positions see.
    pub scopes: Vec<HashMap<String, (Def, bool)>>,
    /// Per module, the values by name: functions, consts and globals. A
    /// value may share its name with a type or module (`fn parse` next to
    /// `import std.parse`); expressions that name a value look here.
    pub values: Vec<HashMap<String, (Def, bool)>>,
    pub methods: HashMap<(Ty, String), FnId>,
    pub consts: Vec<ConstInfo>,
    pub global_decls: Vec<(usize, GlobalDecl)>,
    pub pending: Vec<(FnId, usize, FnDecl, Vec<(String, Ty)>)>,
    pub generics: crate::generics::Generics,
    /// Type parameters in scope (while resolving generic instances).
    pub tenv: Vec<(String, Ty)>,
    pub object_count: u32,
    pub vm_decls: Vec<(usize, crate::vmast::VmDecl)>,
    pub dispatch_override: Option<Dispatch>,
    pub vms: Vec<crate::vmcheck::VmInfo>,
    pub traits: crate::traits::Traits,
    pub aliases: Vec<AliasInfo>,
    /// Warnings, in the order found (deduplicated by span: handler
    /// bodies can be checked more than once).
    pub warnings: Vec<Diag>,
    /// Declaring a signature-only function (an instance over opaque type
    /// parameters, or a trait method without a default): no body needed.
    pub sig_only: bool,
    /// Closures created so far (for unique symbols).
    pub closure_count: u32,
    /// Functions declared `#[unsafe]` (`Vm.assume_verified`): naming one
    /// needs an `unsafe { }` block.
    pub unsafe_fns: std::collections::HashSet<FnId>,
    /// The file declaring each struct: fields whose names start with `__`
    /// are private to it (the generated `Vm__Verified` is built only by
    /// the vm's own generated functions).
    pub struct_files: HashMap<u32, u32>,
}

pub fn mk(ty: Ty, kind: TK, span: Span) -> TExpr {
    TExpr { ty, kind, span }
}

/// A value of function type `from` may be used as `to`: same signature,
/// and `from`'s effects are a subset of `to`'s. Only at the top level (and
/// under `?`), never behind a pointer, where it would let a store through
/// `*fn() uses gc` be read back as a plain `fn()`.
fn fn_widens(from: &Ty, to: &Ty) -> bool {
    match (from, to) {
        (Ty::Fn(p1, r1, e1), Ty::Fn(p2, r2, e2))
        | (Ty::Block(p1, r1, e1), Ty::Block(p2, r2, e2))
        | (Ty::Closure(p1, r1, e1), Ty::Closure(p2, r2, e2)) => p1 == p2 && r1 == r2 && e1 & !e2 == 0,
        (Ty::Option(a), Ty::Option(b)) => fn_widens(a, b),
        _ => false,
    }
}

/// The signature a value of type `t` is called with: `t` itself, or the
/// closure type of a `Gc<closure fn(..)>`.
pub fn callee_sig(t: &Ty) -> Ty {
    match t {
        Ty::Gc(inner) if matches!(**inner, Ty::Closure(..)) => (**inner).clone(),
        t => t.clone(),
    }
}

/// An unknown part of a guessed type (`Checker::guess_ty`).
pub const HOLE: u32 = u32::MAX;

pub fn has_hole(t: &Ty) -> bool {
    match t {
        Ty::Param(HOLE) => true,
        Ty::Ptr(x) | Ty::Slice(x) | Ty::Array(x, _) | Ty::Option(x) | Ty::Gc(x) => has_hole(x),
        Ty::Result(a, b) => has_hole(a) || has_hole(b),
        Ty::Tuple(ts) => ts.iter().any(has_hole),
        Ty::Fn(ps, r, _) | Ty::Closure(ps, r, _) => ps.iter().any(has_hole) || has_hole(r),
        _ => false,
    }
}

/// Combine two guesses of the same value's type, filling holes.
fn merge_guess(a: Ty, b: Ty) -> Ty {
    match (a, b) {
        (Ty::Param(HOLE), t) | (t, Ty::Param(HOLE)) => t,
        (Ty::Option(x), Ty::Option(y)) => Ty::Option(Box::new(merge_guess(*x, *y))),
        (Ty::Result(a1, b1), Ty::Result(a2, b2)) => Ty::Result(Box::new(merge_guess(*a1, *a2)), Box::new(merge_guess(*b1, *b2))),
        (Ty::Tuple(xs), Ty::Tuple(ys)) if xs.len() == ys.len() => {
            Ty::Tuple(xs.into_iter().zip(ys).map(|(x, y)| merge_guess(x, y)).collect())
        }
        (a, _) => a,
    }
}

fn is_literal_expr(e: &Expr) -> bool {
    match &e.kind {
        ExprKind::Int(_) | ExprKind::Float(_) | ExprKind::Char(_) => true,
        ExprKind::Unary(UnOp::Neg, inner) => is_literal_expr(inner),
        // `Kind.COUNT` types like a literal.
        ExprKind::Field(_, n) => n.name == "COUNT",
        _ => false,
    }
}

impl<'a> Checker<'a> {
    pub fn new(modules: &'a [Module]) -> Checker<'a> {
        Checker {
            prog: Program::default(),
            modules,
            scopes: vec![HashMap::new(); modules.len()],
            values: vec![HashMap::new(); modules.len()],
            methods: HashMap::new(),
            consts: vec![],
            global_decls: vec![],
            pending: vec![],
            vm_decls: vec![],
            dispatch_override: None,
            vms: vec![],
            generics: Default::default(),
            traits: Default::default(),
            tenv: vec![],
            object_count: 0,
            aliases: vec![],
            warnings: vec![],
            sig_only: false,
            closure_count: 0,
            unsafe_fns: Default::default(),
            struct_files: HashMap::new(),
        }
    }

    pub fn module_prefix(&self, m: usize) -> String {
        self.modules[m].path.join(".")
    }

    // ── declaration collection ──────────────────────────────────────

    /// Bind `name` in module `m`. Values (functions, consts, globals) and
    /// the rest (types, modules, vms, traits) are separate namespaces: a
    /// name may be bound once in each (language.md §1.5).
    pub fn define(&mut self, m: usize, name: &Ident, def: Def, is_pub: bool) -> Res<()> {
        let dup = || Err(Diag::new(name.span, format!("`{}` is defined more than once", name.name)));
        // A const or global may have fields (`ORIGIN.x`), which would be
        // ambiguous with a path through a type or module of its name.
        let has_fields = |d: &Def| matches!(d, Def::Const(_) | Def::Global(_));
        if is_value_def(&def) {
            if self.values[m].contains_key(&name.name) {
                return dup();
            }
            if has_fields(&def) && self.scopes[m].contains_key(&name.name) {
                return dup();
            }
            self.values[m].insert(name.name.clone(), (def.clone(), is_pub));
            if !self.scopes[m].contains_key(&name.name) {
                self.scopes[m].insert(name.name.clone(), (def, is_pub));
            }
        } else {
            if self.scopes[m].get(&name.name).is_some_and(|(d, _)| !is_value_def(d)) {
                return dup();
            }
            if self.values[m].get(&name.name).is_some_and(|(d, _)| has_fields(d)) {
                return dup();
            }
            self.scopes[m].insert(name.name.clone(), (def, is_pub));
        }
        Ok(())
    }

    /// The value (function, const, global) named `name` in module `m`.
    pub fn value_def(&self, m: usize, name: &str) -> Option<(Def, bool)> {
        self.values[m].get(name).cloned()
    }

    pub fn run(&mut self) -> Res<()> {
        // Pass 1: register names.
        let mut fn_decls: Vec<(FnId, usize, FnDecl)> = Vec::new();
        for (m, module) in self.modules.iter().enumerate() {
            let prefix = self.module_prefix(m);
            for item in &module.items {
                match item {
                    Item::Struct(s) if !s.generics.is_empty() => {
                        let id = self.generics.types.len() as u32;
                        self.generics.types.push(crate::generics::GenericType {
                            module: m,
                            name: s.name.name.clone(),
                            params: s.generics.iter().map(|g| g.name.clone()).collect(),
                            bounds: s.bounds.clone(),
                            decl: crate::generics::GenDecl::Struct(s.clone()),
                        });
                        self.define(m, &s.name, Def::GenericType(id), s.is_pub)?;
                    }
                    Item::Enum(e) if !e.generics.is_empty() => {
                        let id = self.generics.types.len() as u32;
                        self.generics.types.push(crate::generics::GenericType {
                            module: m,
                            name: e.name.name.clone(),
                            params: e.generics.iter().map(|g| g.name.clone()).collect(),
                            bounds: e.bounds.clone(),
                            decl: crate::generics::GenDecl::Enum(e.clone()),
                        });
                        self.define(m, &e.name, Def::GenericType(id), e.is_pub)?;
                    }
                    Item::Struct(s) => {
                        let id = self.prog.structs.len() as u32;
                        let object = if s.is_object {
                            self.object_count += 1;
                            Some(self.object_count - 1)
                        } else {
                            None
                        };
                        self.struct_files.insert(id, s.name.span.file);
                        self.prog.type_spans.insert(format!("{}.{}", prefix, s.name.name), s.name.span);
                        if let Some(c) = &s.c_name {
                            self.prog.c_names.insert(format!("{}.{}", prefix, s.name.name), c.clone());
                        }
                        self.prog.structs.push(StructDef {
                            name: format!("{}.{}", prefix, s.name.name),
                            fields: vec![],
                            pattern: false,
                            object,
                            trailing: None,
                            verified: None,
                        });
                        self.define(m, &s.name, Def::Struct(id), s.is_pub)?;
                    }
                    Item::Enum(e) => {
                        let id = self.prog.enums.len() as u32;
                        self.prog.type_spans.insert(format!("{}.{}", prefix, e.name.name), e.name.span);
                        if let Some(c) = &e.c_name {
                            self.prog.c_names.insert(format!("{}.{}", prefix, e.name.name), c.clone());
                        }
                        self.prog.enums.push(EnumDef {
                            name: format!("{}.{}", prefix, e.name.name),
                            tag: IntTy::I32,
                            variants: vec![],
                        });
                        self.define(m, &e.name, Def::Enum(id), e.is_pub)?;
                    }
                    Item::Value(v) => {
                        let id = self.prog.values.len() as u32;
                        self.prog.values.push(ValueDef {
                            name: format!("{}.{}", prefix, v.name.name),
                            repr: ValueRepr::NanBox,
                            variants: vec![],
                            num_limit: 0,
                        });
                        self.define(m, &v.name, Def::Value(id), v.is_pub)?;
                    }
                    Item::Fn(f) if !f.generics.is_empty() => {
                        let id = self.generics.fns.len() as u32;
                        self.generics.fns.push(crate::generics::GenericFn {
                            module: m,
                            decl: f.clone(),
                            params: f.generics.iter().map(|g| g.name.clone()).collect(),
                        });
                        if f.recv.is_none() {
                            self.define(m, &f.name, Def::GenericFn(id), f.is_pub)?;
                        }
                    }
                    Item::Fn(f) => {
                        let id = self.prog.fns.len() as FnId;
                        self.prog.fns.push(placeholder_fn(f.span()));
                        if f.recv.is_none() {
                            self.define(m, &f.name, Def::Fn(id), f.is_pub || f.linkage != Linkage::Internal)?;
                        }
                        fn_decls.push((id, m, f.clone()));
                    }
                    Item::Const(c) => {
                        let id = self.consts.len() as u32;
                        self.consts.push(ConstInfo { module: m, decl: c.clone(), value: None, busy: false });
                        self.define(m, &c.name, Def::Const(id), c.is_pub)?;
                    }
                    Item::Global(g) => {
                        let id = self.global_decls.len() as u32;
                        self.global_decls.push((m, g.clone()));
                        self.define(m, &g.name, Def::Global(id), g.is_pub)?;
                    }
                    Item::Vm(v) => {
                        let id = self.vm_decls.len() as u32;
                        self.vm_decls.push((m, (**v).clone()));
                        self.define(m, &v.name, Def::Vm(id), v.is_pub)?;
                    }
                    Item::Trait(t) => {
                        let id = self.register_trait(m, t)?;
                        self.define(m, &t.name, Def::Trait(id), t.is_pub)?;
                    }
                    Item::Impl(i) => self.traits.impl_decls.push((m, i.clone())),
                    Item::TypeAlias(a) => {
                        let id = self.aliases.len() as u32;
                        self.aliases.push(AliasInfo { module: m, decl: a.clone(), ty: None, busy: false });
                        self.define(m, &a.name, Def::Alias(id), a.is_pub)?;
                    }
                    Item::Import(_) => {}
                }
            }
        }
        // Pass 2: imports.
        for (m, module) in self.modules.iter().enumerate() {
            for item in &module.items {
                if let Item::Import(imp) = item {
                    let path: Vec<String> = imp.path.iter().map(|i| i.name.clone()).collect();
                    let target = self
                        .modules
                        .iter()
                        .position(|mm| mm.path == path)
                        .ok_or_else(|| Diag::new(imp.span, format!("module `{}` not found", path.join("."))))?;
                    match &imp.names {
                        None => {
                            let last = imp.path.last().unwrap();
                            self.define(m, last, Def::Module(target), false)?;
                        }
                        Some(names) => {
                            for n in names {
                                // Both the value and the type or module of
                                // that name, if the module has both.
                                let value = self.values[target].get(&n.name).cloned();
                                let other = self.scopes[target].get(&n.name).cloned().filter(|(d, _)| !is_value_def(d));
                                if value.is_none() && other.is_none() {
                                    return Err(Diag::new(n.span, format!("`{}` not found in module `{}`", n.name, path.join("."))));
                                }
                                let found: Vec<(Def, bool)> = value.into_iter().chain(other).collect();
                                if found.iter().all(|(_, p)| !p) {
                                    return Err(Diag::new(n.span, format!("`{}` is private", n.name)));
                                }
                                for (def, is_pub) in found {
                                    if is_pub {
                                        self.define(m, n, def, false)?;
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        // Impls are known before any type is resolved, so that generic
        // instances created from here on can have their bounds checked.
        self.register_impls()?;
        // Non-generic type aliases, so that code taking `&self` (type
        // paths in expressions) can read them.
        for a in 0..self.aliases.len() {
            if self.aliases[a].decl.generics.is_empty() {
                let span = self.aliases[a].decl.name.span;
                self.alias_type(a as u32, vec![], span)?;
            }
        }
        // Pass 3: type definitions.
        for (m, module) in self.modules.iter().enumerate() {
            for item in &module.items {
                match item {
                    Item::Struct(s) if s.generics.is_empty() => {
                        let Some((Def::Struct(id), _)) = self.scopes[m].get(&s.name.name).cloned() else { unreachable!() };
                        let (fields, trailing, verified) = self.struct_fields(m, s)?;
                        self.prog.structs[id as usize].fields = fields;
                        self.prog.structs[id as usize].trailing = trailing;
                        self.prog.structs[id as usize].verified = verified;
                    }
                    Item::Enum(e) if e.generics.is_empty() => {
                        let Some((Def::Enum(id), _)) = self.scopes[m].get(&e.name.name).cloned() else { unreachable!() };
                        let (tag, variants) = self.enum_variants(m, e)?;
                        let def = &mut self.prog.enums[id as usize];
                        def.tag = tag;
                        def.variants = variants;
                    }
                    Item::Value(v) => {
                        let Some((Def::Value(id), _)) = self.scopes[m].get(&v.name.name).cloned() else { unreachable!() };
                        let def = crate::vmcheck::check_value_decl(self, m, v)?;
                        self.prog.values[id as usize] = def;
                    }
                    _ => {}
                }
            }
        }
        // Pass 4: function signatures; methods of generic types.
        for (id, m, f) in &fn_decls {
            self.declare_fn(*id, *m, f, None)?;
        }
        for g in 0..self.generics.fns.len() {
            let gf = &self.generics.fns[g];
            let Some(recv) = gf.decl.recv.clone() else { continue };
            let (m, mname) = (gf.module, gf.decl.name.clone());
            let nrecv = gf.decl.recv_generics;
            let Some((Def::GenericType(t), _)) = self.scopes[m].get(&recv.name).cloned() else {
                // A plain receiver type with method type parameters: `fn Cursor.op<K>`.
                if nrecv != 0 {
                    return Err(Diag::new(recv.span, format!("`{}` is not a generic type", recv.name)));
                }
                let rty = self.method_recv_ty(m, &recv)?;
                let key = (rty, mname.name.clone());
                if self.methods.contains_key(&key) || self.generics.ty_methods.insert(key, g as u32).is_some() {
                    return Err(Diag::new(mname.span, format!("method `{}` is defined more than once", mname.name)));
                }
                continue;
            };
            let tparams = self.generics.types[t as usize].params.len();
            if tparams != nrecv {
                return Err(Diag::new(
                    recv.span,
                    format!("`{}` takes {} type parameter(s); write them after the type name (`fn {}<..>.{}`)", recv.name, tparams, recv.name, mname.name),
                ));
            }
            if self.generics.methods.insert((t, mname.name.clone()), g as u32).is_some() {
                return Err(Diag::new(mname.span, format!("method `{}` is defined more than once", mname.name)));
            }
        }
        self.check_impls()?;
        // Module-level `var`s: types and constant initializers.
        for i in 0..self.global_decls.len() {
            let (m, g) = self.global_decls[i].clone();
            let ty = self.resolve_type(m, &g.ty)?;
            let mut cx = FnCx::new(m, Ty::Unit);
            let v = self.expr(&mut cx, &g.value, Some(&ty))?;
            let init = fold_deep(&self.coerce(v, &ty)?);
            if let Some(d) = const_overflow(&init) {
                return Err(d);
            }
            if let Some(bad) = self.static_init_error(&init) {
                return Err(Diag::new(
                    bad,
                    "the initializer of a module-level `var` must be a constant expression (literals, `[x; N]`, struct, tuple, array and enum literals of constants, functions)",
                ));
            }
            let symbol = format!("{}.{}", self.module_prefix(m), g.name.name);
            self.prog.globals.push(GlobalDef { symbol, ty, init, thread_local: g.thread_local });
        }
        // Pass 5: vm declarations (generates handler functions).
        for i in 0..self.vm_decls.len() {
            crate::vmcheck::check_vm(self, i)?;
        }
        // Pass 6: bodies.
        for (id, m, f) in fn_decls {
            self.pending.push((id, m, f, vec![]));
        }
        while let Some((id, m, f, env)) = self.pending.pop() {
            self.check_against_bounds(id).map_err(|d| self.instance_note(d, id))?;
            self.with_tenv(env, |c| c.check_fn_body(id, m, &f, None)).map_err(|d| self.instance_note(d, id))?;
        }
        // `export "C" const`s, for the header.
        for id in 0..self.consts.len() {
            if !self.consts[id].decl.export {
                continue;
            }
            let span = self.consts[id].decl.name.span;
            let v = self.const_value(id as u32, span)?;
            self.prog.c_consts.push((self.consts[id].decl.name.name.clone(), v, span));
        }
        Ok(())
    }

    /// The fields of a struct or object declaration (with type parameters
    /// bound in `tenv`), its trailing array, and for a `verified` trailing
    /// array the vm's proof type.
    #[allow(clippy::type_complexity)]
    pub fn struct_fields(&mut self, m: usize, s: &StructDecl) -> Res<(Vec<(String, Ty)>, Option<(String, Ty)>, Option<u32>)> {
        let mut fields = Vec::new();
        let mut trailing = None;
        let mut verified = None;
        for (i, (f, t)) in s.fields.iter().enumerate() {
            if fields.iter().any(|(n, _): &(String, Ty)| *n == f.name) {
                return Err(Diag::new(f.span, format!("duplicate field `{}`", f.name)));
            }
            if let TypeKind::Trailing(elem) | TypeKind::VerifiedTrailing(elem, _) = &t.kind {
                if !s.is_object || i + 1 != s.fields.len() {
                    return Err(Diag::new(t.span, "a trailing array `[T]` must be the last field of an `object`"));
                }
                let et = self.resolve_type(m, elem)?;
                if let TypeKind::VerifiedTrailing(_, path) = &t.kind {
                    let Some(vm) = self.vm_named(m, path) else {
                        return Err(Diag::new(path[0].span, "`verified [W] by Vm` names a vm after `by`"));
                    };
                    let (vm_m, vname, word) = {
                        let (vm_m, d) = &self.vm_decls[vm as usize];
                        (*vm_m, d.name.name.clone(), d.word.clone())
                    };
                    let word = match &word {
                        Some(w) => self.resolve_type(vm_m, w)?,
                        None => return Err(Diag::new(path[0].span, format!("vm `{}` has no encoding", vname))),
                    };
                    if word != et {
                        return Err(Diag::new(
                            elem.span,
                            format!("vm `{}` has `{}` code words, not `{}`", vname, self.ty_name(&word), self.ty_name(&et)),
                        ));
                    }
                    if self.vm_decls[vm as usize].1.ops.iter().any(|o| o.stack.is_some()) {
                        return Err(Diag::new(
                            t.span,
                            format!("vm `{vname}` declares stack effects, so its proof records the maximum stack depth: keep it in a `Verified<{vname}>` field, not a `verified [W] by {vname}` array"),
                        ));
                    }
                    match self.scopes[vm_m].get(&format!("{}__Verified", vname)) {
                        Some((Def::Struct(v), _)) => verified = Some(*v),
                        _ => unreachable!("vmexpand generates Vm__Verified"),
                    }
                }
                trailing = Some((f.name.clone(), et));
                continue;
            }
            fields.push((f.name.clone(), self.resolve_type(m, t)?));
        }
        Ok((fields, trailing, verified))
    }

    /// An error inside an instance of a generic function: say which
    /// instance, and where it was first needed.
    pub fn instance_note(&self, d: Diag, id: FnId) -> Diag {
        match self.generics.fn_site.get(&id) {
            Some(site) if !d.notes.iter().any(|(s, _)| s == site) => {
                let name = self.prog.fns[id as usize].name.clone();
                d.note(*site, format!("in `{}`, instantiated here", name))
            }
            _ => d,
        }
    }

    /// The LLVM constant an `export "C"` function with
    /// `#[catch_panic(v)]` returns after a panic: `v` (an integer, `true`,
    /// `false`, or a variant of the result enum), or zero (`0`, `false`,
    /// NULL, an empty `str`) without one.
    fn catch_panic_value(&self, f: &FnDecl, ret: &Ty, v: Option<&str>) -> Res<String> {
        let sp = f.attrs.list.iter().find(|(n, _, _)| n == "catch_panic").map(|a| a.2).unwrap_or(f.name.span);
        if f.linkage != Linkage::Export {
            return Err(Diag::new(sp, "`#[catch_panic]` applies to `export \"C\"` functions: the boundary where a panic turns into a C return value"));
        }
        if let Ty::Ptr(_) | Ty::Fn(..) | Ty::Gc(_) = ret {
            return Err(Diag::new(sp, format!("`#[catch_panic]`: after a panic the result is NULL, which `{}` cannot be; return `?{}`", self.ty_name(ret), self.ty_name(ret))));
        }
        let Some(v) = v else { return Ok("zeroinitializer".into()) };
        let bad = |what: &str| Err(Diag::new(sp, format!("`#[catch_panic({})]`: {}", v, what)));
        match ret {
            Ty::Unit | Ty::Never => bad("the function returns nothing; write `#[catch_panic]`"),
            Ty::Int(it) => match v.parse::<i128>() {
                Ok(n) => {
                    let (lo, hi) = if it.signed {
                        (-(1i128 << (it.bits - 1)), (1i128 << (it.bits - 1)) - 1)
                    } else {
                        (0, (1i128 << it.bits) - 1)
                    };
                    if n < lo || n > hi {
                        return bad(&format!("out of range for `{}`", self.ty_name(ret)));
                    }
                    Ok(n.to_string())
                }
                Err(_) => bad(&format!("expected an integer for the `{}` result", self.ty_name(ret))),
            },
            Ty::Bool => match v {
                "true" => Ok("true".into()),
                "false" => Ok("false".into()),
                _ => bad("expected `true` or `false` for the `bool` result"),
            },
            Ty::Enum(e) => {
                let ed = &self.prog.enums[*e as usize];
                match ed.variants.iter().find(|x| x.name == v) {
                    Some(x) if !ed.has_payload() => Ok(x.discr.to_string()),
                    Some(_) => bad("the result enum has payloads"),
                    None => bad(&format!("`{}` has no variant `{}`", self.ty_name(ret), v)),
                }
            }
            _ => bad(&format!("a `{}` result can only be zero after a panic; write `#[catch_panic]`", self.ty_name(ret))),
        }
    }

    pub fn declare_fn(&mut self, id: FnId, m: usize, f: &FnDecl, symbol: Option<String>) -> Res<()> {
        let mut param_tys = Vec::new();
        for p in &f.params {
            param_tys.push(self.resolve_param_type(m, &p.ty)?);
        }
        let ret = match &f.ret {
            Some(t) => self.resolve_type(m, t)?,
            None => Ty::Unit,
        };
        let prefix = self.module_prefix(m);
        let mut name = f.name.name.clone();
        if let (Some(recv), true) = (&f.recv, f.generics.is_empty()) {
            let rty = self.method_recv_ty(m, recv)?;
            name = format!("{}.{}", recv.name, f.name.name);
            let key = (rty, f.name.name.clone());
            if self.methods.contains_key(&key) {
                return Err(Diag::new(f.name.span, format!("method `{}` is defined more than once", name)));
            }
            self.methods.insert(key, id);
        }
        let symbol = symbol.unwrap_or_else(|| match f.linkage {
            Linkage::Internal => format!("{}.{}", prefix, name),
            _ => f.link_name.clone().unwrap_or_else(|| f.name.name.clone()),
        });
        if f.linkage == Linkage::Extern && f.body.is_some() {
            return Err(Diag::new(f.name.span, "extern functions cannot have a body"));
        }
        if f.linkage != Linkage::Extern && f.body.is_none() && !self.sig_only {
            return Err(Diag::new(f.name.span, "function needs a body"));
        }
        if f.linkage != Linkage::Internal {
            if param_tys.iter().any(|t| matches!(t, Ty::Block(..))) {
                return Err(Diag::new(f.name.span, "C functions cannot take a `block fn`: pass a plain `fn` and a context pointer"));
            }
            if param_tys.iter().chain(std::iter::once(&ret)).any(|t| matches!(callee_sig(t), Ty::Closure(..))) {
                return Err(Diag::new(f.name.span, "closures cannot cross into C: pass a plain `fn` and a context pointer"));
            }
            let odd = |t: &Ty| matches!(t, Ty::Int(it) if !matches!(it.bits, 8 | 16 | 32 | 64));
            if let Some(t) = param_tys.iter().chain(std::iter::once(&ret)).find(|t| odd(t)) {
                return Err(Diag::new(
                    f.name.span,
                    format!("`{}` has no C ABI: C functions take and return 8-, 16-, 32- or 64-bit integers", self.ty_name(t)),
                ));
            }
        }
        let inline = if f.attrs.has("inline") {
            match f.attrs.get("inline").unwrap().first().map(|s| s.as_str()) {
                Some("never") => Some(false),
                _ => Some(true),
            }
        } else {
            None
        };
        if f.attrs.has("unsafe") {
            self.unsafe_fns.insert(id);
        }
        if let Some((_, args, sp)) = f.attrs.list.iter().find(|(n, _, _)| n == "c_const") {
            if f.linkage != Linkage::Export {
                return Err(Diag::new(*sp, "`#[c_const(..)]` applies to `export \"C\"` functions"));
            }
            for a in args {
                match f.params.iter().position(|p| p.name.name == *a) {
                    Some(i) if matches!(param_tys[i], Ty::Ptr(_)) => {}
                    Some(_) => return Err(Diag::new(*sp, format!("`#[c_const({})]`: `{}` is not a pointer parameter", a, a))),
                    None => return Err(Diag::new(*sp, format!("`#[c_const({})]`: no parameter `{}`", a, a))),
                }
            }
            self.prog.c_const_params.insert(symbol.clone(), args.iter().map(|a| (a.clone(), *sp)).collect());
        }
        if let Some((_, _, sp)) = f.attrs.list.iter().find(|(n, _, _)| n == "test") {
            if !f.params.is_empty() || !f.generics.is_empty() || f.recv.is_some() || ret != Ty::Unit || f.body.is_none() || f.linkage != Linkage::Internal {
                return Err(Diag::new(*sp, "`#[test]` applies to functions `fn name()` without parameters, type parameters or a result"));
            }
        }
        let catch_panic = match f.attrs.get("catch_panic") {
            Some(args) => Some(self.catch_panic_value(f, &ret, args.first().map(|s| s.as_str()))?),
            None => None,
        };
        self.prog.fns[id as usize] = FnDef {
            name,
            symbol,
            params: vec![],
            param_tys,
            ret,
            locals: vec![],
            body: None,
            linkage: f.linkage,
            variadic: f.variadic,
            inline,
            cold: f.attrs.has("cold"),
            cc: CallConv::C,
            span: f.name.span,
            vm: None,
            vm_role: VmRole::None,
            vm_state: vec![],
            effects: effects_of(&f.attrs)?,
            sync_effects: 0,
            env: None,
            env_val: vec![],
            env_struct: None,
            env_root: None,
            catch_panic,
            track_caller: f.attrs.has("track_caller"),
        };
        Ok(())
    }

    pub fn check_fn_body(&mut self, id: FnId, m: usize, f: &FnDecl, vm: Option<crate::vmcheck::HandlerCx>) -> Res<()> {
        let Some(body) = &f.body else { return Ok(()) };
        let def = self.prog.fns[id as usize].clone();
        let mut cx = FnCx::new(m, def.ret.clone());
        cx.vm = vm;
        cx.effects = def.effects;
        cx.fn_name = def.name.clone();
        let mut params = Vec::new();
        for (p, t) in f.params.iter().zip(def.param_tys.iter()) {
            params.push(cx.declare(&p.name.name, t.clone(), true));
        }
        let ret = def.ret.clone();
        let b = self.block(&mut cx, body, Some(&ret))?;
        let b = self.coerce(b, &ret)?;
        let fd = &mut self.prog.fns[id as usize];
        fd.params = params;
        fd.locals = cx.locals;
        fd.body = Some(b);
        Ok(())
    }

    // ── types ───────────────────────────────────────────────────────

    pub fn ty_name(&self, t: &Ty) -> String {
        match t {
            Ty::Int(it) => format!("{}{}", if it.signed { "i" } else { "u" }, it.bits),
            Ty::F32 => "f32".into(),
            Ty::F64 => "f64".into(),
            Ty::Bool => "bool".into(),
            Ty::Unit => "()".into(),
            Ty::Never => "never".into(),
            Ty::Ptr(t) => format!("*{}", self.ty_name(t)),
            Ty::Slice(t) => format!("[]{}", self.ty_name(t)),
            Ty::Array(t, n) => format!("[{}; {}]", self.ty_name(t), n),
            Ty::Option(t) => format!("?{}", self.ty_name(t)),
            Ty::Result(a, b) => format!("Result<{}, {}>", self.ty_name(a), self.ty_name(b)),
            Ty::Tuple(ts) => format!("({})", ts.iter().map(|t| self.ty_name(t)).collect::<Vec<_>>().join(", ")),
            Ty::Fn(ps, r, e) => format!(
                "fn({}) -> {}{}",
                ps.iter().map(|t| self.ty_name(t)).collect::<Vec<_>>().join(", "),
                self.ty_name(r),
                if *e == 0 { String::new() } else { format!(" uses {}", effect_name(*e)) }
            ),
            Ty::Struct(i) => {
                // Generated vm types: `Mini__Verified` is `Verified<Mini>`,
                // `Mini__VerifyError` is `Mini.VerifyError`.
                let n = short(&self.prog.structs[*i as usize].name);
                match n.split_once("__") {
                    Some((vm, "Verified")) => format!("Verified<{}>", vm),
                    Some((vm, "VerifyError")) => format!("{}.VerifyError", vm),
                    _ => n,
                }
            }
            Ty::Enum(i) => short(&self.prog.enums[*i as usize].name),
            Ty::Value(i) => short(&self.prog.values[*i as usize].name),
            Ty::Param(i) => format!("?T{}", i),
            Ty::Gc(t) => format!("Gc<{}>", self.ty_name(t)),
            Ty::Opaque(o) => self.traits.opaques[*o as usize].name.clone(),
            Ty::Closure(ps, r, e) => format!(
                "closure fn({}) -> {}{}",
                ps.iter().map(|t| self.ty_name(t)).collect::<Vec<_>>().join(", "),
                self.ty_name(r),
                if *e == 0 { String::new() } else { format!(" uses {}", effect_name(*e)) }
            ),
            Ty::Block(ps, r, e) => format!(
                "block fn({}) -> {}{}",
                ps.iter().map(|t| self.ty_name(t)).collect::<Vec<_>>().join(", "),
                self.ty_name(r),
                if *e == 0 { String::new() } else { format!(" uses {}", effect_name(*e)) }
            ),
        }
    }

    pub fn enum_variants(&mut self, m: usize, e: &EnumDecl) -> Res<(IntTy, Vec<VariantDef>)> {
        let tag = match &e.repr {
            Some(t) => match self.resolve_type(m, t)? {
                Ty::Int(it) => it,
                _ => return Err(Diag::new(t.span, "enum representation must be an integer type")),
            },
            None => IntTy::I32,
        };
        let mut variants = Vec::new();
        let mut next = 0i128;
        for v in &e.variants {
            let mut fields = Vec::new();
            for f in &v.fields {
                fields.push(self.resolve_type(m, f)?);
            }
            let discr = v.discr.unwrap_or(next);
            next = discr + 1;
            variants.push(VariantDef { name: v.name.name.clone(), fields, discr });
        }
        Ok((tag, variants))
    }

    /// The alias a type path names (`Name` or `module.Name`), if any.
    fn alias_path(&self, m: usize, path: &[Ident]) -> Res<Option<(u32, String)>> {
        let (tm, id) = match path {
            [id] => {
                if self.tenv.iter().any(|(n, _)| *n == id.name) {
                    return Ok(None);
                }
                (m, id)
            }
            [module, id] => match self.scopes[m].get(&module.name) {
                Some((Def::Module(target), _)) => (*target, id),
                _ => return Ok(None),
            },
            _ => return Ok(None),
        };
        match self.scopes[tm].get(&id.name) {
            Some((Def::Alias(a), is_pub)) => {
                if tm != m && !is_pub {
                    return Err(Diag::new(id.span, format!("`{}` is private", id.name)));
                }
                Ok(Some((*a, id.name.clone())))
            }
            _ => Ok(None),
        }
    }

    /// The type alias `a` stands for, with its parameters bound to `args`.
    /// The aliased type is resolved in the alias's module, seeing only
    /// the alias's own parameters.
    pub fn alias_type(&mut self, a: u32, args: Vec<Ty>, span: Span) -> Res<Ty> {
        let info = &self.aliases[a as usize];
        if let Some(t) = &info.ty {
            return Ok(t.clone());
        }
        if info.busy {
            return Err(Diag::new(span, format!("type alias `{}` refers to itself", info.decl.name.name))
                .note(info.decl.name.span, "the alias is declared here"));
        }
        let (m, decl) = (info.module, info.decl.clone());
        let env: Vec<(String, Ty)> = decl.generics.iter().map(|g| g.name.clone()).zip(args).collect();
        self.aliases[a as usize].busy = true;
        let saved = std::mem::replace(&mut self.tenv, env);
        let r = self.resolve_type(m, &decl.ty);
        self.tenv = saved;
        self.aliases[a as usize].busy = false;
        let t = r?;
        if decl.generics.is_empty() {
            self.aliases[a as usize].ty = Some(t.clone());
        }
        Ok(t)
    }

    /// The type a method `fn Recv.name` is defined on. Method calls look
    /// through pointers and `Gc` to the type behind them, so a method on
    /// an alias of a pointer type could never be called.
    fn method_recv_ty(&self, m: usize, recv: &Ident) -> Res<Ty> {
        let rty = self.resolve_type_name(m, recv)?;
        if let Ty::Ptr(inner) | Ty::Gc(inner) = &rty {
            return Err(Diag::new(
                recv.span,
                format!(
                    "`{}` is `{}`: methods are found on the type behind a pointer; define the method on `{}` and take `{}` as the receiver",
                    recv.name,
                    self.ty_name(&rty),
                    self.ty_name(inner),
                    self.ty_name(&rty)
                ),
            ));
        }
        Ok(rty)
    }

    pub fn resolve_type_name(&self, m: usize, id: &Ident) -> Res<Ty> {
        if let Some((_, t)) = self.tenv.iter().rev().find(|(n, _)| *n == id.name) {
            return Ok(t.clone());
        }
        if let Some(t) = prim_type(&id.name) {
            return Ok(t);
        }
        if let Some((Def::GenericType(_), _)) = self.scopes[m].get(&id.name) {
            return Err(Diag::new(id.span, format!("`{}` needs type arguments", id.name)));
        }
        match self.scopes[m].get(&id.name) {
            Some((Def::Alias(a), _)) => {
                let a = &self.aliases[*a as usize];
                match &a.ty {
                    Some(t) => Ok(t.clone()),
                    None if !a.decl.generics.is_empty() => {
                        Err(Diag::new(id.span, format!("`{}` needs type arguments", id.name)))
                    }
                    None => Err(Diag::new(id.span, format!("type alias `{}` is used before it is resolved", id.name))),
                }
            }
            Some((Def::Struct(i), _)) => Ok(Ty::Struct(*i)),
            Some((Def::Enum(i), _)) => Ok(Ty::Enum(*i)),
            Some((Def::Value(i), _)) => Ok(Ty::Value(*i)),
            _ => Err(Diag::new(id.span, format!("unknown type `{}`", id.name))),
        }
    }

    /// The vm named by `path` (`Mini`, `vm.Mini`) as seen from module `m`.
    fn vm_named(&self, m: usize, path: &[Ident]) -> Option<u32> {
        let target = match path {
            [_] => m,
            [md, _] => match self.scopes[m].get(&md.name) {
                Some((Def::Module(t), _)) => *t,
                _ => return None,
            },
            _ => return None,
        };
        match self.scopes[target].get(&path[path.len() - 1].name) {
            Some((Def::Vm(i), _)) => Some(*i),
            _ => None,
        }
    }

    /// Types generated for a vm: `Verified<Mini>` (also `Mini.Verified`),
    /// `Mini.VerifyError`, `Mini.VerifyReason`. They live in the vm's
    /// module as `Mini__X`.
    fn vm_type(&mut self, m: usize, path: &[Ident], args: &[TypeExpr], span: Span) -> Res<Option<Ty>> {
        let (vm, item) = if path.len() == 1 && path[0].name == "Verified" && !self.scopes[m].contains_key("Verified") {
            let [a] = args else {
                return Err(Diag::new(span, "Verified takes one argument, a vm: `Verified<Mini>`"));
            };
            let TypeKind::Named(vp, va) = &a.kind else {
                return Err(Diag::new(a.span, "Verified takes a vm: `Verified<Mini>`"));
            };
            match self.vm_named(m, vp) {
                Some(vm) if va.is_empty() => (vm, "Verified".to_string()),
                _ => return Err(Diag::new(a.span, "Verified takes a vm: `Verified<Mini>`")),
            }
        } else if path.len() == 2 && args.is_empty() {
            match self.vm_named(m, &path[..1]) {
                Some(vm) => (vm, path[1].name.clone()),
                None => return Ok(None),
            }
        } else {
            return Ok(None);
        };
        let (vm_m, decl) = &self.vm_decls[vm as usize];
        let full = format!("{}__{}", decl.name.name, item);
        match self.scopes[*vm_m].get(&full) {
            Some((Def::Struct(i), _)) => Ok(Some(Ty::Struct(*i))),
            Some((Def::Enum(i), _)) => Ok(Some(Ty::Enum(*i))),
            _ => Err(Diag::new(
                path[path.len() - 1].span,
                format!("vm `{}` has no generated type `{}`", decl.name.name, item),
            )),
        }
    }

    pub fn resolve_type(&mut self, m: usize, t: &TypeExpr) -> Res<Ty> {
        Ok(match &t.kind {
            TypeKind::Named(path, args) => {
                if path.len() == 1 && path[0].name == "Gc" && !self.scopes[m].contains_key("Gc") {
                    if args.len() != 1 {
                        return Err(Diag::new(t.span, "Gc takes one type argument"));
                    }
                    let inner = self.resolve_type(m, &args[0])?;
                    // A type parameter is checked when it is bound: in a
                    // generic signature it is an inference variable here,
                    // and in an instance it is the concrete type.
                    let ok = match &inner {
                        Ty::Struct(i) => self.prog.structs[*i as usize].object.is_some(),
                        Ty::Param(_) | Ty::Opaque(_) | Ty::Closure(..) => true,
                        _ => false,
                    };
                    if !ok {
                        let msg = if self.tenv.is_empty() {
                            "Gc<T> needs an `object` type (or a `closure fn` type)".to_string()
                        } else {
                            format!("Gc<T> needs an `object` type, found `{}`", self.ty_name(&inner))
                        };
                        return Err(Diag::new(args[0].span, msg));
                    }
                    return Ok(Ty::Gc(Box::new(inner)));
                }
                if let Some((a, name)) = self.alias_path(m, path)? {
                    let mut targs = Vec::new();
                    for x in args {
                        targs.push(self.resolve_type(m, x)?);
                    }
                    let want = self.aliases[a as usize].decl.generics.len();
                    if targs.len() != want {
                        return Err(Diag::new(
                            t.span,
                            format!("type alias `{}` takes {} type argument(s), got {}", name, want, targs.len()),
                        ));
                    }
                    return self.alias_type(a, targs, t.span);
                }
                if let Some(t) = self.vm_type(m, path, args, t.span)? {
                    return Ok(t);
                }
                if path.len() == 1 && path[0].name == "Result" {
                    if args.len() != 2 {
                        return Err(Diag::new(t.span, "Result takes two type arguments"));
                    }
                    let a = self.resolve_type(m, &args[0])?;
                    let b = self.resolve_type(m, &args[1])?;
                    return Ok(Ty::Result(Box::new(a), Box::new(b)));
                }
                if !args.is_empty() {
                    let (gm, gname) = if path.len() == 1 {
                        (m, &path[0])
                    } else if path.len() == 2 {
                        match self.scopes[m].get(&path[0].name) {
                            Some((Def::Module(target), _)) => (*target, &path[1]),
                            _ => return Err(Diag::new(path[0].span, format!("unknown module `{}`", path[0].name))),
                        }
                    } else {
                        return Err(Diag::new(t.span, "unsupported type path"));
                    };
                    let Some((Def::GenericType(g), _)) = self.scopes[gm].get(&gname.name).cloned() else {
                        return Err(Diag::new(gname.span, format!("`{}` is not a generic type", gname.name)));
                    };
                    let mut targs = Vec::new();
                    for a in args {
                        targs.push(self.resolve_type(m, a)?);
                    }
                    return self.instantiate_type(g, targs, t.span);
                }
                if path.len() == 1 {
                    self.resolve_type_name(m, &path[0])?
                } else if path.len() == 2 {
                    match self.scopes[m].get(&path[0].name) {
                        Some((Def::Module(target), _)) => {
                            let target = *target;
                            match self.scopes[target].get(&path[1].name) {
                                Some((_, false)) => {
                                    return Err(Diag::new(path[1].span, format!("`{}` is private", path[1].name)))
                                }
                                _ => self.resolve_type_name(target, &path[1])?,
                            }
                        }
                        _ => return Err(Diag::new(path[0].span, format!("unknown module `{}`", path[0].name))),
                    }
                } else {
                    return Err(Diag::new(t.span, "unsupported type path"));
                }
            }
            TypeKind::Ptr(inner) => Ty::Ptr(Box::new(self.resolve_type(m, inner)?)),
            TypeKind::Slice(inner) => Ty::Slice(Box::new(self.resolve_type(m, inner)?)),
            TypeKind::Option(inner) => Ty::Option(Box::new(self.resolve_type(m, inner)?)),
            TypeKind::Array(inner, n) => {
                let elem = self.resolve_type(m, inner)?;
                let n = self.const_int(m, n)?;
                if n < 0 {
                    return Err(Diag::new(t.span, "negative array length"));
                }
                Ty::Array(Box::new(elem), n as u64)
            }
            TypeKind::Tuple(ts) => {
                if ts.is_empty() {
                    Ty::Unit
                } else {
                    let mut out = Vec::new();
                    for t in ts {
                        out.push(self.resolve_type(m, t)?);
                    }
                    Ty::Tuple(out)
                }
            }
            TypeKind::Block(..) => {
                return Err(Diag::new(
                    t.span,
                    "a `block fn` type can only be the type of a function parameter: closures cannot be stored or returned",
                ))
            }
            TypeKind::Fn(ps, r, eff) | TypeKind::Closure(ps, r, eff) => {
                let mut out = Vec::new();
                for p in ps {
                    out.push(self.resolve_param_type(m, p)?);
                }
                let e = match eff {
                    Some(e) => effect_bits(&e.name, e.span)?,
                    None => 0,
                };
                let r = Box::new(self.resolve_type(m, r)?);
                if matches!(t.kind, TypeKind::Closure(..)) { Ty::Closure(out, r, e) } else { Ty::Fn(out, r, e) }
            }
            TypeKind::Never => Ty::Never,
            TypeKind::Cache(_) => {
                return Err(Diag::new(t.span, "a `cache Name` handle can only be the type of a vm function parameter"))
            }
            TypeKind::Trailing(_) | TypeKind::VerifiedTrailing(..) => {
                return Err(Diag::new(t.span, "a trailing array `[T]` is only allowed as the last field of an `object`"))
            }
        })
    }

    /// The type of a function parameter: like `resolve_type`, but a
    /// `block fn` type is allowed at the top.
    pub fn resolve_param_type(&mut self, m: usize, t: &TypeExpr) -> Res<Ty> {
        let TypeKind::Block(ps, r, eff) = &t.kind else { return self.resolve_type(m, t) };
        let mut out = Vec::new();
        for p in ps {
            out.push(self.resolve_param_type(m, p)?);
        }
        let e = match eff {
            Some(e) => effect_bits(&e.name, e.span)?,
            None => 0,
        };
        Ok(Ty::Block(out, Box::new(self.resolve_type(m, r)?), e))
    }

    pub fn const_int(&mut self, m: usize, e: &Expr) -> Res<i128> {
        let mut cx = FnCx::new(m, Ty::Unit);
        let te = self.expr(&mut cx, e, Some(&Ty::i64()))?;
        let te = fold(&te);
        if let Some(d) = const_overflow(&te) {
            return Err(d);
        }
        match te.kind {
            TK::Int(v) => Ok(v),
            _ => Err(Diag::new(e.span, "expected a constant integer expression")),
        }
    }

    fn const_value(&mut self, id: u32, span: Span) -> Res<TExpr> {
        if let Some(v) = &self.consts[id as usize].value {
            return Ok(v.clone());
        }
        if self.consts[id as usize].busy {
            return Err(Diag::new(span, "recursive constant"));
        }
        self.consts[id as usize].busy = true;
        let m = self.consts[id as usize].module;
        let decl = self.consts[id as usize].decl.clone();
        let ty = self.resolve_type(m, &decl.ty)?;
        let mut cx = FnCx::new(m, Ty::Unit);
        let v = self.expr(&mut cx, &decl.value, Some(&ty))?;
        let v = fold_deep(&self.coerce(v, &ty)?);
        if let Some(d) = const_overflow(&v) {
            return Err(d);
        }
        if !is_const(&v) {
            return Err(Diag::new(decl.value.span, "constant initializer must be a constant expression"));
        }
        self.consts[id as usize].value = Some(v.clone());
        self.consts[id as usize].busy = false;
        Ok(v)
    }

    pub fn coerce(&self, e: TExpr, want: &Ty) -> Res<TExpr> {
        if e.ty == *want {
            return Ok(e);
        }
        if e.ty == Ty::Never {
            let span = e.span;
            return Ok(mk(want.clone(), TK::Block(vec![TStmt::Expr(e)], None), span));
        }
        if fn_widens(&e.ty, want) {
            // A function value converts to a function type with more effects.
            return Ok(TExpr { ty: want.clone(), ..e });
        }
        if let (Ty::Fn(p1, r1, e1), Ty::Block(p2, r2, e2)) = (&e.ty, want) {
            // A plain function is a closure without an environment.
            if p1 == p2 && r1 == r2 && e1 & !e2 == 0 {
                let span = e.span;
                return Ok(mk(want.clone(), TK::Cast(Box::new(e)), span));
            }
        }
        if let (Ty::Fn(p1, r1, e1), Ty::Closure(p2, r2, e2)) | (Ty::Closure(p1, r1, e1), Ty::Block(p2, r2, e2)) = (&e.ty, want) {
            // A plain function is a closure without an environment (through
            // a trampoline); a `closure fn` passes as a `block fn` as is.
            if p1 == p2 && r1 == r2 && e1 & !e2 == 0 {
                let span = e.span;
                return Ok(mk(want.clone(), TK::Cast(Box::new(e)), span));
            }
        }
        if let Ty::Option(inner) = want {
            if **inner == e.ty || fn_widens(&e.ty, inner) {
                let span = e.span;
                let e = TExpr { ty: (**inner).clone(), ..e };
                return Ok(mk(want.clone(), TK::Variant(1, vec![e]), span));
            }
        }
        if let (
            Ty::Fn(p1, r1, e1) | Ty::Block(p1, r1, e1) | Ty::Closure(p1, r1, e1),
            Ty::Fn(p2, r2, e2) | Ty::Block(p2, r2, e2) | Ty::Closure(p2, r2, e2),
        ) = (&e.ty, want)
        {
            if p1 == p2 && r1 == r2 && e1 & !e2 != 0 {
                return Err(Diag::new(
                    e.span,
                    format!(
                        "this function uses {}, but the expected type `{}` does not allow it",
                        effect_name(e1 & !e2),
                        self.ty_name(want)
                    ),
                ));
            }
        }
        Err(Diag::new(
            value_span(&e),
            format!("mismatched types: expected `{}`, found `{}`", self.ty_name(want), self.ty_name(&e.ty)),
        ))
    }

    // ── statements and blocks ───────────────────────────────────────

    pub fn block(&mut self, cx: &mut FnCx, b: &Block, exp: Option<&Ty>) -> Res<TExpr> {
        cx.scopes.push(vec![]);
        let r = self.block_inner(cx, b, exp);
        cx.scopes.pop();
        r
    }

    fn block_inner(&mut self, cx: &mut FnCx, b: &Block, exp: Option<&Ty>) -> Res<TExpr> {
        let mut stmts = Vec::new();
        let mut tail = None;
        let mut diverges = false;
        let n = b.stmts.len();
        for (i, s) in b.stmts.iter().enumerate() {
            match s {
                Stmt::Expr { expr, semi: false } if i == n - 1 => {
                    let e = self.expr(cx, expr, exp)?;
                    tail = Some(Box::new(e));
                }
                Stmt::Expr { expr, .. } => {
                    let e = self.expr(cx, expr, None)?;
                    if e.ty == Ty::Never {
                        diverges = true;
                    }
                    stmts.push(TStmt::Expr(e));
                }
                Stmt::Defer(e) => {
                    let e = self.expr(cx, e, None)?;
                    stmts.push(TStmt::Defer(e));
                }
                Stmt::Let { pat, mutable, ty, init, els, span } => {
                    let decl_ty = match ty {
                        Some(t) => Some(self.resolve_type(cx.module, t)?),
                        None => None,
                    };
                    let init = match init {
                        Some(e) => {
                            let v = self.expr(cx, e, decl_ty.as_ref())?;
                            Some(match &decl_ty {
                                Some(t) => self.coerce(v, t)?,
                                None => v,
                            })
                        }
                        None => None,
                    };
                    if let Some(init) = &init {
                        if init.ty == Ty::Never {
                            diverges = true;
                        }
                    }
                    let simple = matches!(&pat.kind, PatKind::Ident(name, _) if !self.names_unit_variant(name, init.as_ref().map(|e| &e.ty).or(decl_ty.as_ref())));
                    if simple && els.is_none() {
                        let PatKind::Ident(name, pm) = &pat.kind else { unreachable!() };
                        let t = match (&decl_ty, &init) {
                            (Some(t), _) => t.clone(),
                            (None, Some(e)) => e.ty.clone(),
                            (None, None) => {
                                return Err(Diag::new(*span, "a `var` without initializer needs a type"))
                            }
                        };
                        if init.is_none() && !mutable {
                            return Err(Diag::new(*span, "a `let` binding needs an initializer"));
                        }
                        let id = cx.declare(name, t, *mutable || *pm);
                        stmts.push(TStmt::Let(id, init));
                    } else {
                        let Some(init) = init else {
                            return Err(Diag::new(*span, "pattern bindings need an initializer"));
                        };
                        let els_t = match els {
                            Some(eb) => {
                                let e = self.block(cx, eb, None)?;
                                if e.ty != Ty::Never {
                                    return Err(Diag::new(eb.span, "the `else` block of a `let ... else` must diverge"));
                                }
                                Some(e)
                            }
                            None => None,
                        };
                        let p = self.pat(cx, pat, &init.ty, *mutable)?;
                        if els_t.is_none() && !irrefutable(&p) {
                            return Err(Diag::new(pat.span, "refutable pattern in `let`; add an `else` block"));
                        }
                        stmts.push(TStmt::LetPat(p, init, els_t));
                    }
                }
            }
        }
        let ty = match &tail {
            Some(t) => t.ty.clone(),
            None if diverges => Ty::Never,
            None => Ty::Unit,
        };
        Ok(mk(ty, TK::Block(stmts, tail), b.span))
    }

    fn names_unit_variant(&self, name: &str, ty: Option<&Ty>) -> bool {
        match ty {
            Some(t) => self.variant_index(t, name).is_some_and(|i| self.variant_fields(t, i).is_empty()),
            None => false,
        }
    }

    // ── patterns ────────────────────────────────────────────────────

    pub fn variant_index(&self, ty: &Ty, name: &str) -> Option<u32> {
        match ty {
            Ty::Enum(i) => self.prog.enums[*i as usize].variants.iter().position(|v| v.name == name).map(|i| i as u32),
            Ty::Option(_) => match name {
                "None" => Some(0),
                "Some" => Some(1),
                _ => None,
            },
            Ty::Result(..) => match name {
                "Ok" => Some(0),
                "Err" => Some(1),
                _ => None,
            },
            Ty::Value(i) => {
                self.prog.values[*i as usize].variants.iter().position(|v| v.name == name).map(|i| i as u32)
            }
            _ => None,
        }
    }

    pub fn variant_fields(&self, ty: &Ty, idx: u32) -> Vec<Ty> {
        match ty {
            Ty::Enum(i) => self.prog.enums[*i as usize].variants[idx as usize].fields.clone(),
            Ty::Option(t) => {
                if idx == 0 {
                    vec![]
                } else {
                    vec![(**t).clone()]
                }
            }
            Ty::Result(a, b) => {
                if idx == 0 {
                    vec![(**a).clone()]
                } else {
                    vec![(**b).clone()]
                }
            }
            Ty::Value(i) => self.prog.values[*i as usize].variants[idx as usize].payload.iter().cloned().collect(),
            _ => vec![],
        }
    }

    fn variant_count(&self, ty: &Ty) -> usize {
        match ty {
            Ty::Enum(i) => self.prog.enums[*i as usize].variants.len(),
            Ty::Option(_) | Ty::Result(..) => 2,
            Ty::Value(i) => self.prog.values[*i as usize].variants.len(),
            Ty::Bool => 2,
            _ => 0,
        }
    }

    /// The constant a pattern path names, if any: `LIMIT` (a `const` of
    /// this module or imported, not shadowed by a local), `mod.LIMIT`,
    /// `Vm.OP_ADD`.
    fn pat_const(&self, cx: &FnCx, path: &[Ident]) -> Option<u32> {
        if cx.lookup(&path[0].name).is_some() {
            return None;
        }
        if path.len() == 1 {
            let n = &path[0].name;
            return match self.value_def(cx.module, n).or_else(|| self.scopes[cx.module].get(n).cloned()) {
                Some((Def::Const(id), _)) => Some(id),
                _ => None,
            };
        }
        let mut base = Expr { kind: ExprKind::Ident(path[0].name.clone()), span: path[0].span };
        for id in &path[1..path.len() - 1] {
            base = Expr { kind: ExprKind::Field(Box::new(base), id.clone()), span: id.span };
        }
        let last = &path[path.len() - 1].name;
        if let Some(Def::Const(id)) = self.vm_item(cx, &base, last) {
            return Some(id);
        }
        let m = self.module_path(cx, &base)?;
        match self.scopes[m].get(last) {
            Some((Def::Const(id), true)) => Some(*id),
            _ => None,
        }
    }

    /// A pattern that matches the value of constant `id`.
    fn const_pat(&mut self, id: u32, name: &str, ty: &Ty, span: Span) -> Res<TPat> {
        let v = self.const_value(id, span)?;
        if v.ty != *ty {
            return Err(Diag::new(
                span,
                format!(
                    "constant `{}` has type `{}`, but the value matched has type `{}`",
                    name,
                    self.ty_name(&v.ty),
                    self.ty_name(ty)
                ),
            ));
        }
        fn conv(c: &Checker, e: &TExpr, name: &str, span: Span) -> Res<TPat> {
            let kind = match &e.kind {
                TK::Int(v) => TPK::Int(*v),
                TK::Bool(b) => TPK::Bool(*b),
                TK::Str(s) => TPK::Str(s.clone()),
                TK::Unit => TPK::Wild,
                TK::Tuple(es) => TPK::Tuple(es.iter().map(|x| conv(c, x, name, span)).collect::<Res<_>>()?),
                TK::Variant(i, es) if !matches!(e.ty, Ty::Value(_)) => {
                    TPK::Variant(*i, es.iter().map(|x| conv(c, x, name, span)).collect::<Res<_>>()?)
                }
                _ => {
                    return Err(Diag::new(
                        span,
                        format!(
                            "constant `{}` cannot be a pattern: only integer, `bool`, `str`, enum and tuple constants can (this part has type `{}`)",
                            name,
                            c.ty_name(&e.ty)
                        ),
                    ))
                }
            };
            Ok(TPat { ty: e.ty.clone(), kind })
        }
        conv(self, &v, name, span)
    }

    pub fn pat(&mut self, cx: &mut FnCx, p: &Pat, ty: &Ty, mutable: bool) -> Res<TPat> {
        let kind = match &p.kind {
            PatKind::Wild => TPK::Wild,
            PatKind::Ident(name, pm) => {
                if let Some(idx) = self.variant_index(ty, name) {
                    if self.variant_fields(ty, idx).is_empty() {
                        return Ok(TPat { ty: ty.clone(), kind: TPK::Variant(idx, vec![]) });
                    }
                }
                if !*pm {
                    if let Some(id) = self.pat_const(cx, std::slice::from_ref(&Ident { name: name.clone(), span: p.span })) {
                        return self.const_pat(id, name, ty, p.span);
                    }
                }
                TPK::Bind(cx.declare(name, ty.clone(), mutable || *pm))
            }
            // `mod.LIMIT`, `Vm.OP_ADD`: a constant compares with its value.
            PatKind::Variant(path, None) if path.len() >= 2 && !path[0].name.is_empty() && self.pat_const(cx, path).is_some() => {
                let id = self.pat_const(cx, path).unwrap();
                let name = path.iter().map(|i| i.name.as_str()).collect::<Vec<_>>().join(".");
                return self.const_pat(id, &name, ty, p.span);
            }
            PatKind::Int(_) | PatKind::Range(..) | PatKind::Char(..) if !ty.is_int() => {
                return Err(Diag::new(p.span, format!("integer pattern for a value of type `{}`", self.ty_name(ty))));
            }
            PatKind::Int(_) | PatKind::Range(..) | PatKind::Char(..) => {
                // Out of the type's range, a pattern would be truncated
                // to it and match another value.
                let Ty::Int(it) = ty else { unreachable!() };
                let (lo, hi) = match &p.kind {
                    PatKind::Int(v) => (*v, *v),
                    PatKind::Char(c) => (*c as i128, *c as i128),
                    PatKind::Range(a, b) => (*a, *b),
                    _ => unreachable!(),
                };
                for v in [lo, hi] {
                    if v < it.min() || v > it.max() {
                        return Err(Diag::new(p.span, format!("pattern `{}` does not fit in `{}`", v, self.ty_name(ty))));
                    }
                }
                if lo == hi && !matches!(p.kind, PatKind::Range(..)) { TPK::Int(lo) } else { TPK::Range(lo, hi) }
            }
            PatKind::Str(bytes) => {
                if *ty != Ty::str() {
                    return Err(Diag::new(p.span, format!("string pattern for a value of type `{}`", self.ty_name(ty))));
                }
                TPK::Str(bytes.clone())
            }
            PatKind::Bool(b) => {
                if *ty != Ty::Bool {
                    return Err(Diag::new(p.span, "boolean pattern for a non-bool value"));
                }
                TPK::Bool(*b)
            }
            // `()` matches the unit value, as in `Ok(())`.
            PatKind::Tuple(ps) if ps.is_empty() && *ty == Ty::Unit => TPK::Wild,
            PatKind::Tuple(ps) => {
                let Ty::Tuple(ts) = ty else {
                    return Err(Diag::new(p.span, format!("tuple pattern for a value of type `{}`", self.ty_name(ty))));
                };
                if ts.len() != ps.len() {
                    return Err(Diag::new(p.span, format!("expected a tuple of {} elements", ts.len())));
                }
                let mut out = Vec::new();
                for (sp, st) in ps.iter().zip(ts.clone().iter()) {
                    out.push(self.pat(cx, sp, st, mutable)?);
                }
                TPK::Tuple(out)
            }
            PatKind::Variant(path, args) => {
                let vname = &path.last().unwrap().name;
                // Check the qualifier, if any, names the scrutinee type.
                if path.len() >= 2 && !path[0].name.is_empty() {
                    let qual = &path[path.len() - 2];
                    // `Vm.Op.ADD`: a generated enum of a vm.
                    let vm_ty = match self.scopes[cx.module].get(&path[0].name) {
                        Some((Def::Vm(v), _)) if path.len() == 3 => {
                            let (vm_m, decl) = &self.vm_decls[*v as usize];
                            match self.scopes[*vm_m].get(&format!("{}__{}", decl.name.name, qual.name)) {
                                Some((Def::Enum(i), _)) => Some(Ty::Enum(*i)),
                                _ => {
                                    return Err(Diag::new(
                                        qual.span,
                                        format!("vm `{}` has no generated enum `{}`", decl.name.name, qual.name),
                                    ))
                                }
                            }
                        }
                        _ => None,
                    };
                    // `mod.Enum.Variant`: the enum is looked up in `mod`.
                    let qm = match path.len() {
                        _ if vm_ty.is_some() => cx.module,
                        3 => match self.scopes[cx.module].get(&path[0].name) {
                            Some((Def::Module(t), _)) => *t,
                            _ => return Err(Diag::new(path[0].span, format!("unknown module `{}`", path[0].name))),
                        },
                        _ => cx.module,
                    };
                    // `Lookup.Own(i)` for a value of type `Lookup<O>`: a
                    // generic enum's name stands for any of its instances.
                    let generic = match self.scopes[qm].get(&qual.name) {
                        Some((Def::GenericType(g), _)) if self.tenv.iter().all(|(n, _)| *n != qual.name) => Some(*g),
                        _ => None,
                    };
                    let qty = match generic {
                        _ if vm_ty.is_some() => vm_ty.clone().unwrap(),
                        Some(g) if self.generics.inst_of.get(ty).is_some_and(|(g2, _)| *g2 == g) => ty.clone(),
                        Some(g) => {
                            let gname = self.generics.types[g as usize].name.clone();
                            return Err(Diag::new(
                                p.span,
                                format!("pattern of type `{}` for a value of type `{}`", gname, self.ty_name(ty)),
                            ));
                        }
                        None => self.resolve_type_name(qm, qual)?,
                    };
                    if qty != *ty {
                        return Err(Diag::new(
                            p.span,
                            format!("pattern of type `{}` for a value of type `{}`", self.ty_name(&qty), self.ty_name(ty)),
                        ));
                    }
                }
                let Some(idx) = self.variant_index(ty, vname) else {
                    return Err(Diag::new(p.span, format!("no variant `{}` in type `{}`", vname, self.ty_name(ty))));
                };
                let fields = self.variant_fields(ty, idx);
                let args = args.clone().unwrap_or_default();
                if args.len() != fields.len() {
                    return Err(Diag::new(p.span, format!("variant `{}` has {} field(s)", vname, fields.len())));
                }
                let mut out = Vec::new();
                for (ap, ft) in args.iter().zip(fields.iter()) {
                    out.push(self.pat(cx, ap, ft, mutable)?);
                }
                TPK::Variant(idx, out)
            }
            PatKind::Or(alts) => {
                let mut out = Vec::new();
                let before = cx.locals.len();
                for a in alts {
                    out.push(self.pat(cx, a, ty, mutable)?);
                }
                if cx.locals.len() != before {
                    return Err(Diag::new(p.span, "bindings in `|` patterns are not supported"));
                }
                TPK::Or(out)
            }
        };
        Ok(TPat { ty: ty.clone(), kind })
    }

    /// Is the pattern matrix `rows` (one row per arm, one column per type
    /// in `tys`; `None` is a wildcard) exhaustive? A simplified version of
    /// the usual usefulness algorithm: constructors are enum/option/result/
    /// value variants, booleans and tuples; integers only count as covered
    /// by wildcards.
    fn rows_exhaustive(&self, tys: &[Ty], rows: Vec<Vec<Option<&TPat>>>) -> bool {
        if tys.is_empty() {
            return !rows.is_empty();
        }
        // Expand or-patterns and bindings in the first column.
        let mut flat: Vec<Vec<Option<&TPat>>> = Vec::new();
        let mut stack: Vec<Vec<Option<&TPat>>> = rows.into_iter().rev().collect();
        while let Some(row) = stack.pop() {
            match row[0].map(|p| &p.kind) {
                Some(TPK::Or(alts)) => {
                    for a in alts.iter().rev() {
                        let mut r = row.clone();
                        r[0] = Some(a);
                        stack.push(r);
                    }
                }
                Some(TPK::Wild) | Some(TPK::Bind(_)) => {
                    let mut r = row;
                    r[0] = None;
                    flat.push(r);
                }
                _ => flat.push(row),
            }
        }
        let rest = &tys[1..];
        if let Ty::Tuple(elems) = &tys[0] {
            let mut sub_tys = elems.clone();
            sub_tys.extend_from_slice(rest);
            let rows = flat
                .iter()
                .map(|r| {
                    let mut out: Vec<Option<&TPat>> = match r[0].map(|p| &p.kind) {
                        Some(TPK::Tuple(ps)) => ps.iter().map(Some).collect(),
                        _ => vec![None; elems.len()],
                    };
                    out.extend_from_slice(&r[1..]);
                    out
                })
                .collect();
            return self.rows_exhaustive(&sub_tys, rows);
        }
        let n = self.variant_count(&tys[0]);
        if n == 0 {
            let rows = flat.iter().filter(|r| r[0].is_none()).map(|r| r[1..].to_vec()).collect();
            return self.rows_exhaustive(rest, rows);
        }
        (0..n as u32).all(|i| self.variant_exhaustive(&tys[0], i, rest, &flat))
    }

    /// Are the rows that match variant `i` of `ty` exhaustive for its
    /// payload and the remaining columns?
    fn variant_exhaustive(&self, ty: &Ty, i: u32, rest: &[Ty], rows: &[Vec<Option<&TPat>>]) -> bool {
        let fields = if matches!(ty, Ty::Bool) { vec![] } else { self.variant_fields(ty, i) };
        let mut sub_tys = fields.clone();
        sub_tys.extend_from_slice(rest);
        let mut sub = Vec::new();
        for r in rows {
            let mut out: Vec<Option<&TPat>> = match r[0].map(|p| &p.kind) {
                None => vec![None; fields.len()],
                Some(TPK::Variant(j, ps)) if *j == i => ps.iter().map(Some).collect(),
                Some(TPK::Bool(b)) if *b as u32 == i => vec![],
                _ => continue,
            };
            out.extend_from_slice(&r[1..]);
            sub.push(out);
        }
        self.rows_exhaustive(&sub_tys, sub)
    }

    fn check_exhaustive(&self, ty: &Ty, arms: &[TArm], span: Span) -> Res<()> {
        let rows: Vec<Vec<Option<&TPat>>> = arms.iter().filter(|a| a.guard.is_none()).map(|a| vec![Some(&a.pat)]).collect();
        if self.rows_exhaustive(std::slice::from_ref(ty), rows.clone()) {
            return Ok(());
        }
        let n = self.variant_count(ty);
        if n > 0 {
            let flat: Vec<Vec<Option<&TPat>>> = arms
                .iter()
                .filter(|a| a.guard.is_none())
                .flat_map(|a| match &a.pat.kind {
                    TPK::Or(alts) => alts.iter().map(|p| vec![Some(p)]).collect::<Vec<_>>(),
                    _ => vec![vec![Some(&a.pat)]],
                })
                .collect();
            let missing: Vec<u32> = (0..n as u32).filter(|i| !self.variant_exhaustive(ty, *i, &[], &flat)).collect();
            let names: Vec<String> = missing
                .iter()
                .map(|i| match ty {
                    Ty::Bool => if *i == 1 { "true".into() } else { "false".into() },
                    Ty::Option(_) => if *i == 0 { "None".into() } else { "Some(_)".into() },
                    Ty::Result(..) => if *i == 0 { "Ok(_)".into() } else { "Err(_)".into() },
                    Ty::Enum(e) => self.prog.enums[*e as usize].variants[*i as usize].name.clone(),
                    Ty::Value(v) => self.prog.values[*v as usize].variants[*i as usize].name.clone(),
                    _ => "?".into(),
                })
                .collect();
            if !names.is_empty() {
                return Err(Diag::new(span, format!("non-exhaustive match: missing {}", names.join(", "))));
            }
        }
        Err(Diag::new(span, "non-exhaustive match: add a `_` arm"))
    }

    // ── expressions ─────────────────────────────────────────────────

    pub fn expr(&mut self, cx: &mut FnCx, e: &Expr, exp: Option<&Ty>) -> Res<TExpr> {
        let span = e.span;
        match &e.kind {
            ExprKind::Int(v) => self.int_lit(*v as i128, exp, span),
            ExprKind::Char(c) => {
                let t = match exp {
                    Some(t @ Ty::Int(_)) => t.clone(),
                    _ => Ty::u8(),
                };
                Ok(mk(t, TK::Int(*c as i128), span))
            }
            ExprKind::Float(f) => {
                let t = match exp {
                    Some(Ty::F32) => Ty::F32,
                    _ => Ty::F64,
                };
                Ok(mk(t, TK::Float(*f), span))
            }
            ExprKind::Bool(b) => Ok(mk(Ty::Bool, TK::Bool(*b), span)),
            ExprKind::Str(s) => Ok(mk(Ty::str(), TK::Str(s.clone()), span)),
            ExprKind::Unit => Ok(mk(Ty::Unit, TK::Unit, span)),
            ExprKind::Ident(name) => self.ident(cx, name, exp, span),
            ExprKind::DotVariant(name) => self.variant_ctor_expected(cx, name, &[], exp, span),
            ExprKind::Field(base, name) => self.field(cx, base, name, exp, span),
            ExprKind::Call(callee, args) => self.call(cx, callee, args, exp, span),
            ExprKind::Index(base, idx) => {
                let b = self.expr(cx, base, None)?;
                let b = self.auto_deref_for_index(b);
                let i = self.expr(cx, idx, Some(&Ty::Int(IntTy::U64)))?;
                if !i.ty.is_int() {
                    return Err(Diag::new(idx.span, "index must be an integer"));
                }
                let elem = match &b.ty {
                    Ty::Array(t, _) | Ty::Slice(t) | Ty::Ptr(t) => (**t).clone(),
                    t => return Err(Diag::new(base.span, format!("cannot index a value of type `{}`", self.ty_name(t)))),
                };
                Ok(mk(elem, TK::Index(Box::new(b), Box::new(i)), span))
            }
            ExprKind::SliceRange(base, lo, hi) => {
                let b = self.expr(cx, base, None)?;
                let b = self.auto_deref_for_index(b);
                let elem = match &b.ty {
                    Ty::Array(t, _) | Ty::Slice(t) => (**t).clone(),
                    Ty::Ptr(t) if hi.is_some() => (**t).clone(),
                    t => return Err(Diag::new(base.span, format!("cannot slice a value of type `{}`", self.ty_name(t)))),
                };
                let u = Ty::Int(IntTy::U64);
                let lo = match lo {
                    Some(l) => Some(Box::new(self.expr_coerce(cx, l, &u)?)),
                    None => None,
                };
                let hi = match hi {
                    Some(h) => Some(Box::new(self.expr_coerce(cx, h, &u)?)),
                    None => None,
                };
                Ok(mk(Ty::Slice(Box::new(elem)), TK::SliceRange(Box::new(b), lo, hi), span))
            }
            ExprKind::Unary(op, inner) => self.unary(cx, *op, inner, exp, span),
            ExprKind::Binary(op, l, r) => self.binary(cx, *op, l, r, exp, span),
            ExprKind::Cast(inner, t) => {
                let to = self.resolve_type(cx.module, t)?;
                let v = self.expr(cx, inner, None)?;
                if v.ty == to {
                    return Ok(v);
                }
                // A pointer (or address, or another object type) becomes a
                // traced reference: only the programmer knows that it
                // points to a live heap object of that type.
                let to_gc = matches!(to, Ty::Gc(_))
                    && matches!(&v.ty, Ty::Ptr(_) | Ty::Gc(_) | Ty::Int(IntTy { bits: 64, .. }));
                if to_gc {
                    if cx.unsafe_depth == 0 {
                        return Err(Diag::new(
                            span,
                            format!(
                                "casting `{}` to `{}` is unsafe: it must point to a live `{}` in the GC heap; wrap the cast in `unsafe {{ }}`",
                                self.ty_name(&v.ty),
                                self.ty_name(&to),
                                self.ty_name(&to).trim_start_matches("Gc<").trim_end_matches('>')
                            ),
                        ));
                    }
                    return Ok(mk(to, TK::Cast(Box::new(v)), span));
                }
                if !self.castable(&v.ty, &to) {
                    let why = match &to {
                        Ty::Enum(_) if v.ty.is_int() => " (only enums without payloads can be cast from integers)",
                        _ => "",
                    };
                    return Err(Diag::new(
                        span,
                        format!("cannot cast `{}` to `{}`{}", self.ty_name(&v.ty), self.ty_name(&to), why),
                    ));
                }
                if let Ty::Enum(eid) = &to {
                    // A constant converts at compile time.
                    if let TK::Int(x) = fold(&v).kind {
                        let vs = &self.prog.enums[*eid as usize].variants;
                        let Some(idx) = vs.iter().position(|var| var.discr == x) else {
                            return Err(Diag::new(span, format!("`{}` is not a discriminant of `{}`", x, self.ty_name(&to))));
                        };
                        return Ok(mk(to, TK::Variant(idx as u32, vec![]), span));
                    }
                }
                Ok(mk(to, TK::Cast(Box::new(v)), span))
            }
            ExprKind::StructLit(path, fields, rest) => {
                // `Type { .. }` or `module.Type { .. }`.
                let (m, name) = match path.as_slice() {
                    [name] => (cx.module, name),
                    [module, name] => match self.scopes[cx.module].get(&module.name) {
                        Some((Def::Module(target), _)) => (*target, name),
                        _ => return Err(Diag::new(module.span, format!("unknown module `{}`", module.name))),
                    },
                    _ => return Err(Diag::new(span, "unsupported struct literal path")),
                };
                let ty = match self.scopes[m].get(&name.name).cloned() {
                    Some((Def::GenericType(g), _)) => self.generic_struct_lit(cx, g, fields, exp, span)?,
                    _ => self.resolve_type_name(m, name)?,
                };
                let Ty::Struct(sid) = ty else {
                    return Err(Diag::new(span, format!("`{}` is not a struct", name.name)));
                };
                let defs = self.prog.structs[sid as usize].fields.clone();
                let mut vals: Vec<Option<TExpr>> = vec![None; defs.len()];
                for (f, v) in fields {
                    self.check_private_field(sid, f)?;
                    let Some(idx) = defs.iter().position(|(n, _)| *n == f.name) else {
                        return Err(Diag::new(f.span, format!("no field `{}` in struct `{}`", f.name, name.name)));
                    };
                    if vals[idx].is_some() {
                        return Err(Diag::new(f.span, format!("field `{}` given twice", f.name)));
                    }
                    vals[idx] = Some(self.expr_coerce(cx, v, &defs[idx].1)?);
                }
                let mut out = Vec::new();
                for (i, v) in vals.into_iter().enumerate() {
                    match (v, rest) {
                        (Some(v), _) => out.push(v),
                        (None, Some(rs)) => {
                            let (fname, fty) = &defs[i];
                            if fname.starts_with("__") || !zero_ok(fty) {
                                // A reference that is never null: `?T` is
                                // the type that starts as None.
                                let hint = match fty {
                                    Ty::Gc(_) | Ty::Fn(..) | Ty::Closure(..) if !fname.starts_with("__") => format!(
                                        " (a `{}` is never null; a field declared `?{}` starts as None)",
                                        self.ty_name(fty),
                                        self.ty_name(fty)
                                    ),
                                    _ => String::new(),
                                };
                                return Err(Diag::new(
                                    *rs,
                                    format!(
                                        "`..zero` cannot set field `{}` of type `{}`: give it a value{}",
                                        fname,
                                        self.ty_name(fty),
                                        hint
                                    ),
                                ));
                            }
                            out.push(mk(fty.clone(), TK::Zeroed, *rs));
                        }
                        (None, None) => return Err(Diag::new(span, format!("missing field `{}`", defs[i].0))),
                    }
                }
                Ok(mk(ty, TK::Struct(out), span))
            }
            ExprKind::Tuple(es) => {
                let exp_ts = match exp {
                    Some(Ty::Tuple(ts)) if ts.len() == es.len() => Some(ts.clone()),
                    _ => None,
                };
                let mut out = Vec::new();
                for (i, e) in es.iter().enumerate() {
                    let et = exp_ts.as_ref().map(|ts| &ts[i]);
                    let v = self.expr(cx, e, et)?;
                    out.push(match et {
                        Some(t) => self.coerce(v, t)?,
                        None => v,
                    });
                }
                let ty = Ty::Tuple(out.iter().map(|e| e.ty.clone()).collect());
                Ok(mk(ty, TK::Tuple(out), span))
            }
            ExprKind::ArrayLit(es) => {
                let mut elem = match exp {
                    Some(Ty::Array(t, _)) => Some((**t).clone()),
                    _ => None,
                };
                let mut out = Vec::new();
                for e in es {
                    let v = self.expr(cx, e, elem.as_ref())?;
                    let v = match &elem {
                        Some(t) => self.coerce(v, t)?,
                        None => {
                            elem = Some(v.ty.clone());
                            v
                        }
                    };
                    out.push(v);
                }
                if es.is_empty() {
                    if let Some(t @ Ty::Slice(_)) = exp {
                        // `[]` where a slice is expected: the empty slice.
                        return Ok(mk(t.clone(), TK::Zeroed, span));
                    }
                }
                let Some(elem) = elem else {
                    return Err(Diag::new(span, "cannot infer the type of an empty array (an empty slice needs its type: `let s: []T = [];`)"));
                };
                Ok(mk(Ty::Array(Box::new(elem), out.len() as u64), TK::Array(out), span))
            }
            ExprKind::ArrayRepeat(v, n) => {
                let elem_exp = match exp {
                    Some(Ty::Array(t, _)) => Some((**t).clone()),
                    _ => None,
                };
                let v = self.expr(cx, v, elem_exp.as_ref())?;
                let v = match &elem_exp {
                    Some(t) => self.coerce(v, t)?,
                    None => v,
                };
                let n = self.const_int(cx.module, n)? as u64;
                Ok(mk(Ty::Array(Box::new(v.ty.clone()), n), TK::ArrayRepeat(Box::new(v), n), span))
            }
            ExprKind::Block(b) => self.block(cx, b, exp),
            ExprKind::Unsafe(b) => {
                cx.unsafe_depth += 1;
                let r = self.block(cx, b, exp);
                cx.unsafe_depth -= 1;
                r
            }
            ExprKind::If(..) | ExprKind::Match(..) if exp.is_none() => self.branches_inferred(cx, e),
            ExprKind::If(conds, then, els) => self.if_expr(cx, conds, then, els.as_deref(), exp, span),
            ExprKind::Match(scrut, arms) => self.match_expr(cx, scrut, arms, exp, span),
            ExprKind::While(conds, body) => {
                let id = self.push_loop(cx, false);
                cx.scopes.push(vec![]);
                let r = (|| -> Res<TExpr> {
                    let tconds = self.conds(cx, conds)?;
                    let b = self.block(cx, body, Some(&Ty::Unit))?;
                    let b = if b.ty == Ty::Never { b } else { self.coerce(b, &Ty::Unit)? };
                    let brk = mk(Ty::Never, TK::Break(id, None), span);
                    Ok(mk(Ty::Unit, TK::If(tconds, Box::new(b), Some(Box::new(brk))), span))
                })();
                cx.scopes.pop();
                cx.loops.pop();
                let body = r?;
                Ok(mk(Ty::Unit, TK::Loop { body: Box::new(body), step: None, id }, span))
            }
            ExprKind::Loop(body) => {
                let id = self.push_loop(cx, true);
                let r = self.block(cx, body, Some(&Ty::Unit));
                let lc = cx.loops.pop().unwrap();
                let b = r?;
                let ty = lc.break_ty.unwrap_or(Ty::Never);
                Ok(mk(ty, TK::Loop { body: Box::new(b), step: None, id }, span))
            }
            ExprKind::For(var, ann, lo, hi, body) => {
                // The range's type: the annotation, else the bound that is
                // not a literal (`0..n` with `n: u64` is a u64 range), else
                // i64.
                let ann = match ann {
                    Some(t) => Some(self.resolve_type(cx.module, t)?),
                    None => None,
                };
                let (lo_t, hi_t) = if let Some(t) = &ann {
                    if !t.is_int() {
                        return Err(Diag::new(lo.span, "range bounds must be integers"));
                    }
                    (self.expr_coerce(cx, lo, t)?, self.expr_coerce(cx, hi, t)?)
                } else if is_literal_expr(lo) && !is_literal_expr(hi) {
                    let h = self.expr(cx, hi, None)?;
                    let l = self.expr_coerce(cx, lo, &h.ty.clone())?;
                    (l, h)
                } else {
                    let l = self.expr(cx, lo, None)?;
                    let h = self.expr_coerce(cx, hi, &l.ty.clone())?;
                    (l, h)
                };
                if !lo_t.ty.is_int() {
                    return Err(Diag::new(lo.span, "range bounds must be integers"));
                }
                let ity = lo_t.ty.clone();
                cx.scopes.push(vec![]);
                let ctr = cx.temp(ity.clone());
                let end = cx.temp(ity.clone());
                let id = self.push_loop(cx, false);
                cx.scopes.push(vec![]);
                let user = cx.declare(&var.name, ity.clone(), false);
                let b = self.block(cx, body, Some(&Ty::Unit));
                cx.scopes.pop();
                cx.loops.pop();
                cx.scopes.pop();
                let b = b?;
                let local = |id| mk(ity.clone(), TK::Local(id), span);
                let cond = mk(Ty::Bool, TK::Binary(BinOp::Lt, Box::new(local(ctr)), Box::new(local(end))), span);
                let inner = mk(
                    Ty::Unit,
                    TK::Block(vec![TStmt::Let(user, Some(local(ctr))), TStmt::Expr(b)], None),
                    span,
                );
                let brk = mk(Ty::Never, TK::Break(id, None), span);
                let body = mk(Ty::Unit, TK::If(vec![TCond::Expr(cond)], Box::new(inner), Some(Box::new(brk))), span);
                let one = mk(ity.clone(), TK::Int(1), span);
                let inc = mk(ity.clone(), TK::Binary(BinOp::AddWrap, Box::new(local(ctr)), Box::new(one)), span);
                let step = mk(Ty::Unit, TK::Assign(Box::new(local(ctr)), Box::new(inc)), span);
                let lp = mk(Ty::Unit, TK::Loop { body: Box::new(body), step: Some(Box::new(step)), id }, span);
                Ok(mk(
                    Ty::Unit,
                    TK::Block(vec![TStmt::Let(ctr, Some(lo_t)), TStmt::Let(end, Some(hi_t))], Some(Box::new(lp))),
                    span,
                ))
            }
            ExprKind::Return(v) => {
                let ret = cx.ret.clone();
                if cx.vm.as_ref().is_some_and(|h| h.is_handler || h.in_fn) {
                    return Err(Diag::new(span, "`return` is not allowed in a handler or `-> never` vm function; use `next`, `jump` or `exit`"));
                }
                let v = match v {
                    Some(v) => Some(Box::new(self.expr_coerce(cx, v, &ret)?)),
                    None => {
                        if ret != Ty::Unit {
                            return Err(Diag::new(span, format!("missing return value of type `{}`", self.ty_name(&ret))));
                        }
                        None
                    }
                };
                Ok(mk(Ty::Never, TK::Return(v), span))
            }
            ExprKind::Break(v) => {
                let Some(lp) = cx.loops.last() else {
                    return Err(Diag::new(span, "`break` outside of a loop"));
                };
                let (id, is_loop, bt) = (lp.id, lp.is_loop, lp.break_ty.clone());
                let v = match v {
                    Some(v) => {
                        if !is_loop {
                            return Err(Diag::new(span, "`break` with a value is only allowed in `loop`"));
                        }
                        let tv = self.expr(cx, v, bt.as_ref())?;
                        let tv = match &bt {
                            Some(t) => self.coerce(tv, t)?,
                            None => tv,
                        };
                        cx.loops.last_mut().unwrap().break_ty = Some(tv.ty.clone());
                        Some(Box::new(tv))
                    }
                    None => {
                        if is_loop {
                            match &bt {
                                Some(t) if *t != Ty::Unit => {
                                    return Err(Diag::new(span, "`break` without a value in a loop that yields one"))
                                }
                                _ => cx.loops.last_mut().unwrap().break_ty = Some(Ty::Unit),
                            }
                        }
                        None
                    }
                };
                Ok(mk(Ty::Never, TK::Break(id, v), span))
            }
            ExprKind::Continue => {
                let Some(lp) = cx.loops.last() else {
                    return Err(Diag::new(span, "`continue` outside of a loop"));
                };
                Ok(mk(Ty::Never, TK::Continue(lp.id), span))
            }
            ExprKind::Assign(lhs, rhs) => {
                let l = self.expr(cx, lhs, None)?;
                self.check_assignable(cx, &l, lhs.span)?;
                let r = self.expr_coerce(cx, rhs, &l.ty.clone())?;
                Ok(mk(Ty::Unit, TK::Assign(Box::new(l), Box::new(r)), span))
            }
            ExprKind::CompoundAssign(op, lhs, rhs) => {
                let desugared = Expr {
                    kind: ExprKind::Assign(
                        lhs.clone(),
                        Box::new(Expr { kind: ExprKind::Binary(*op, lhs.clone(), rhs.clone()), span }),
                    ),
                    span,
                };
                self.expr(cx, &desugared, None)
            }
            ExprKind::Coalesce(a, b) => {
                let av = self.expr(cx, a, exp.map(|t| Ty::Option(Box::new(t.clone()))).as_ref())?;
                let Ty::Option(inner) = av.ty.clone() else {
                    return Err(Diag::new(a.span, "left side of `??` must be an option"));
                };
                // `a ?? b` with `b: ?T` is `?T`: `a` if it is `Some`, else
                // `b`. A plain `b` (the usual case) is preferred.
                let opt_rhs = matches!(&b.kind, ExprKind::Ident(n) if n == "None")
                    || matches!(&b.kind, ExprKind::Call(f, _) if matches!(&f.kind, ExprKind::Ident(n) if n == "Some"));
                let bv = if opt_rhs {
                    self.expr_coerce(cx, b, &av.ty)?
                } else {
                    let bv = self.expr(cx, b, Some(&inner))?;
                    if bv.ty == av.ty && bv.ty != *inner { bv } else { self.coerce(bv, &inner)? }
                };
                let ty = if bv.ty == av.ty && bv.ty != *inner { av.ty.clone() } else { *inner };
                Ok(mk(ty, TK::Coalesce(Box::new(av), Box::new(bv)), span))
            }
            ExprKind::Try(inner) => {
                let v = self.expr(cx, inner, None)?;
                match (&v.ty, &cx.ret) {
                    (Ty::Option(t), Ty::Option(_)) => {
                        let t = (**t).clone();
                        Ok(mk(t, TK::Try(Box::new(v)), span))
                    }
                    (Ty::Result(t, e1), Ty::Result(_, e2)) if e1 == e2 => {
                        let t = (**t).clone();
                        Ok(mk(t, TK::Try(Box::new(v)), span))
                    }
                    (Ty::Result(t, e), _) if cx.vm.is_some() => {
                        let (t, e) = ((**t).clone(), (**e).clone());
                        crate::vmcheck::check_raise_ty(self, cx, &e, span)?;
                        Ok(mk(t, TK::Try(Box::new(v)), span))
                    }
                    (Ty::Option(_), _) | (Ty::Result(..), _) => Err(Diag::new(
                        span,
                        format!(
                            "`?` on `{}` in a function returning `{}`",
                            self.ty_name(&v.ty),
                            self.ty_name(&cx.ret)
                        ),
                    )),
                    _ => Err(Diag::new(span, "`?` needs an option or result")),
                }
            }
            ExprKind::TypeArgs(inner, targs) => self.type_args_value(cx, inner, targs, span),
            ExprKind::Closure(..) => Err(Diag::new(
                span,
                "a closure can only be an argument for a `block fn` parameter (`fn f(cb: block fn(i64) -> bool)`), or escape when created with `new(arena, |..| ..)` or `new(heap, |..| ..)`",
            )),
            ExprKind::SizeOf(t) => {
                let _ = self.resolve_type(cx.module, t)?;
                let ty = self.resolve_type(cx.module, t)?;
                let size = crate::layout::size_of(&self.prog, &ty);
                Ok(mk(Ty::Int(IntTy::U64), TK::Int(size as i128), span))
            }
            ExprKind::Vm(v) => crate::vmcheck::check_vm_expr(self, cx, v, exp, span),
        }
    }

    /// `if`/`match` without an expected type. When a branch cannot be
    /// typed on its own (`Ok(e)`, `None`), guess the type from all branches
    /// together and check again with that as the expected type, so
    /// `if c { Ok(e) } else { Err(m) }` works when `e` and `m` are typed.
    fn branches_inferred(&mut self, cx: &mut FnCx, e: &Expr) -> Res<TExpr> {
        let (nscopes, nloops) = (cx.scopes.len(), cx.loops.len());
        let check = |c: &mut Self, cx: &mut FnCx, exp: Option<&Ty>| match &e.kind {
            ExprKind::If(conds, then, els) => c.if_expr(cx, conds, then, els.as_deref(), exp, e.span),
            ExprKind::Match(scrut, arms) => c.match_expr(cx, scrut, arms, exp, e.span),
            _ => unreachable!(),
        };
        let err = match check(self, cx, None) {
            Ok(v) => return Ok(v),
            Err(d) => d,
        };
        if !err.msg.starts_with("cannot infer") {
            return Err(err);
        }
        cx.scopes.truncate(nscopes);
        cx.loops.truncate(nloops);
        let guess = self.guess_ty(cx, e);
        if has_hole(&guess) {
            return Err(err);
        }
        match check(self, cx, Some(&guess)) {
            Ok(v) => Ok(v),
            Err(_) => {
                cx.scopes.truncate(nscopes);
                cx.loops.truncate(nloops);
                Err(err)
            }
        }
    }

    /// The type of `e` as far as it can be told without an expected type,
    /// with `HOLE` for unknown parts: `Ok(1)` is `Result<i64, HOLE>`,
    /// `None` is `?HOLE`, and the branches of `if`/`match`/blocks are merged.
    /// Checks only on probes, so `cx` is unaffected.
    fn guess_ty(&mut self, cx: &FnCx, e: &Expr) -> Ty {
        let hole = Ty::Param(HOLE);
        let unshadowed = |c: &Self, n: &str| cx.lookup(n).is_none() && !c.values[cx.module].contains_key(n);
        match &e.kind {
            ExprKind::Block(b) | ExprKind::Unsafe(b) => {
                let Some(Stmt::Expr { expr: tail, semi: false }) = b.stmts.last() else {
                    return self.probe_ty(cx, e);
                };
                let mut p = cx.probe();
                p.scopes.push(vec![]);
                let prefix = Block { stmts: b.stmts[..b.stmts.len() - 1].to_vec(), span: b.span };
                if self.block_inner(&mut p, &prefix, None).is_err() {
                    return hole;
                }
                self.guess_ty(&p, tail)
            }
            ExprKind::If(conds, then, els) => {
                let Some(els) = els else { return Ty::Unit };
                let mut p = cx.probe();
                p.scopes.push(vec![]);
                let t = if self.conds(&mut p, conds).is_ok() {
                    let tb = Expr { kind: ExprKind::Block(then.clone()), span: then.span };
                    self.guess_ty(&p, &tb)
                } else {
                    hole.clone()
                };
                let f = self.guess_ty(cx, els);
                merge_guess(t, f)
            }
            ExprKind::Match(scrut, arms) => {
                let mut p = cx.probe();
                let Ok(s) = self.expr(&mut p, scrut, None) else { return hole };
                let mut acc = hole;
                for a in arms {
                    let mut q = p.probe();
                    q.scopes.push(vec![]);
                    if self.pat(&mut q, &a.pat, &s.ty, false).is_ok() {
                        let t = self.guess_ty(&q, &a.body);
                        acc = merge_guess(acc, t);
                    }
                }
                acc
            }
            ExprKind::Call(f, args) if args.len() == 1 => match &f.kind {
                ExprKind::Ident(n) if matches!(n.as_str(), "Ok" | "Err" | "Some") && unshadowed(self, n) => {
                    let x = self.guess_ty(cx, &args[0]);
                    match n.as_str() {
                        "Ok" => Ty::Result(Box::new(x), Box::new(hole)),
                        "Err" => Ty::Result(Box::new(hole), Box::new(x)),
                        _ => Ty::Option(Box::new(x)),
                    }
                }
                _ => self.probe_ty(cx, e),
            },
            ExprKind::Ident(n) if n == "None" && unshadowed(self, n) => Ty::Option(Box::new(hole)),
            ExprKind::Tuple(es) => Ty::Tuple(es.iter().map(|x| self.guess_ty(cx, x)).collect()),
            _ => self.probe_ty(cx, e),
        }
    }

    fn probe_ty(&mut self, cx: &FnCx, e: &Expr) -> Ty {
        let mut p = cx.probe();
        match self.expr(&mut p, e, None) {
            Ok(v) if v.ty != Ty::Never => v.ty,
            _ => Ty::Param(HOLE),
        }
    }

    fn if_expr(&mut self, cx: &mut FnCx, conds: &[Cond], then: &Block, els: Option<&Expr>, exp: Option<&Ty>, span: Span) -> Res<TExpr> {
        // `if c { 1 } else { x }`: type the literal from the other branch.
        let mut hint: Option<Ty> = None;
        if exp.is_none() {
            if let (Some(Stmt::Expr { expr: t, semi: false }), Some(e)) = (then.stmts.last(), els) {
                let else_tail = match &e.kind {
                    ExprKind::Block(b) => match b.stmts.last() {
                        Some(Stmt::Expr { expr, semi: false }) => Some(expr),
                        _ => None,
                    },
                    _ => None,
                };
                if is_literal_expr(t) && then.stmts.len() == 1 {
                    if let Some(et) = else_tail {
                        if !is_literal_expr(et) {
                            let mut probe = FnCx::new(cx.module, cx.ret.clone());
                            probe.scopes = cx.scopes.clone();
                            probe.locals = cx.locals.clone();
                            probe.vm = cx.vm.clone();
                            if let Ok(v) = self.expr(&mut probe, e, None) {
                                if v.ty.is_numeric() {
                                    hint = Some(v.ty);
                                }
                            }
                        }
                    }
                }
            }
        }
        let exp = if hint.is_some() { hint.as_ref() } else { exp };
        cx.scopes.push(vec![]);
        let tconds = self.conds(cx, conds)?;
        let then_exp = if els.is_none() { Some(&Ty::Unit) } else { exp };
        let t = self.block(cx, then, then_exp);
        cx.scopes.pop();
        let t = t?;
        match els {
            None => {
                let t = if t.ty == Ty::Never { t } else { self.coerce(t, &Ty::Unit)? };
                Ok(mk(Ty::Unit, TK::If(tconds, Box::new(t), None), span))
            }
            Some(els) => {
                let e_exp = exp.cloned().or_else(|| if t.ty == Ty::Never { None } else { Some(t.ty.clone()) });
                let ev = self.expr(cx, els, e_exp.as_ref())?;
                let ty = match &e_exp {
                    Some(ty) => ty.clone(),
                    None => ev.ty.clone(),
                };
                let t = self.coerce(t, &ty)?;
                let ev = self.coerce(ev, &ty)?;
                Ok(mk(ty, TK::If(tconds, Box::new(t), Some(Box::new(ev))), span))
            }
        }
    }

    fn match_expr(&mut self, cx: &mut FnCx, scrut: &Expr, arms: &[Arm], exp: Option<&Ty>, span: Span) -> Res<TExpr> {
        let s = self.expr(cx, scrut, None)?;
        let mut ty: Option<Ty> = exp.cloned();
        let mut tarms = Vec::new();
        for arm in arms {
            cx.scopes.push(vec![]);
            let r = (|| -> Res<TArm> {
                let p = self.pat(cx, &arm.pat, &s.ty, false)?;
                let g = match &arm.guard {
                    Some(g) => Some(self.expr_coerce(cx, g, &Ty::Bool)?),
                    None => None,
                };
                let b = self.expr(cx, &arm.body, ty.as_ref())?;
                Ok(TArm { pat: p, guard: g, body: b })
            })();
            cx.scopes.pop();
            let a = r?;
            if ty.is_none() && a.body.ty != Ty::Never {
                ty = Some(a.body.ty.clone());
            }
            tarms.push(a);
        }
        self.check_exhaustive(&s.ty, &tarms, span)?;
        self.lint_unreachable_arms(&tarms, arms);
        if cx.vm.as_ref().is_some_and(|h| h.allow_control) {
            self.lint_lone_if_arms(&tarms, arms);
        }
        let ty = ty.unwrap_or(Ty::Never);
        for a in tarms.iter_mut() {
            let body = std::mem::replace(&mut a.body, mk(Ty::Unit, TK::Unit, span));
            a.body = self.coerce(body, &ty)?;
        }
        Ok(mk(ty, TK::Match(Box::new(s), tarms), span))
    }

    /// Warn about a `match` arm in vm code whose body is a lone `if` (no
    /// `else`) ending in a control word, when a later arm that does
    /// something could also have matched: if the condition is false,
    /// control continues after the `match`, not with the later arm.
    /// An explicit `else {}` (or a guard, `pat if cond => ..`) says
    /// which is meant.
    /// Warn about arms after an arm that matches everything (a binding or
    /// `_`, no guard): they can never run. A binding named like a
    /// constant (`LOCAL =>` with no `const LOCAL` in scope) is the usual
    /// cause, so the note says what it is.
    fn lint_unreachable_arms(&mut self, tarms: &[TArm], arms: &[Arm]) {
        let Some(i) = tarms.iter().position(|a| a.guard.is_none() && irrefutable(&a.pat)) else { return };
        if i + 1 >= tarms.len() {
            return;
        }
        let span = arms[i + 1].pat.span;
        if self.warnings.iter().any(|w| w.span == span) {
            return;
        }
        let mut w = Diag::warning(
            "unreachable_arm",
            span,
            if tarms.len() - i - 1 == 1 { "unreachable arm".to_string() } else { format!("{} unreachable arms", tarms.len() - i - 1) },
        )
        .note(arms[i].pat.span, "this earlier arm matches every value");
        if let PatKind::Ident(n, false) = &arms[i].pat.kind {
            w = w.note(
                arms[i].pat.span,
                format!("`{}` binds a new variable; it is not a constant in scope (a `const` of this module, an imported one, or `mod.{}`, would compare with its value)", n, n),
            );
        }
        self.warnings.push(w);
    }

    pub fn lint_lone_if_arms(&mut self, tarms: &[TArm], arms: &[Arm]) {
        fn lone_if(e: &TExpr) -> bool {
            match &e.kind {
                TK::If(_, then, None) => then.ty == Ty::Never,
                TK::Block(stmts, None) if stmts.len() == 1 => matches!(&stmts[0], TStmt::Expr(x) if lone_if(x)),
                TK::Block(stmts, Some(t)) if stmts.is_empty() => lone_if(t),
                _ => false,
            }
        }
        fn empty(e: &TExpr) -> bool {
            match &e.kind {
                TK::Unit => true,
                TK::Block(stmts, None) => stmts.is_empty(),
                TK::Block(stmts, Some(t)) => stmts.is_empty() && empty(t),
                _ => false,
            }
        }
        for (i, a) in tarms.iter().enumerate() {
            if !lone_if(&a.body) {
                continue;
            }
            let Some(j) = (i + 1..tarms.len()).find(|&j| !empty(&tarms[j].body) && pats_overlap(&a.pat, &tarms[j].pat)) else {
                continue;
            };
            let span = arms[i].body.span;
            if self.warnings.iter().any(|w| w.span == span) {
                continue;
            }
            let w = Diag::warning(
                "match_fallout",
                span,
                "this arm's body is an `if` without `else`: when the condition is false, control continues after the `match`, not with the later arms",
            )
            .note(arms[j].pat.span, "this later arm also matches some of the same values")
            .note(arms[i].pat.span, "to try later arms when the condition is false, use a guard: `pat if cond => ..`; to continue after the `match`, write `else {}` (or put `#[allow(match_fallout)]` on the arm)");
            self.warnings.push(w);
        }
    }

    pub fn expr_coerce(&mut self, cx: &mut FnCx, e: &Expr, want: &Ty) -> Res<TExpr> {
        let v = self.expr(cx, e, Some(want))?;
        self.coerce(v, want)
    }

    fn push_loop(&mut self, cx: &mut FnCx, is_loop: bool) -> u32 {
        let id = cx.next_loop;
        cx.next_loop += 1;
        cx.loops.push(LoopCx { id, is_loop, break_ty: None });
        id
    }

    fn conds(&mut self, cx: &mut FnCx, conds: &[Cond]) -> Res<Vec<TCond>> {
        let mut out = Vec::new();
        for c in conds {
            match c {
                Cond::Expr(e) => out.push(TCond::Expr(self.expr_coerce(cx, e, &Ty::Bool)?)),
                Cond::Let(p, e) => {
                    let v = self.expr(cx, e, None)?;
                    let tp = self.pat(cx, p, &v.ty, false)?;
                    out.push(TCond::Let(tp, v));
                }
            }
        }
        Ok(out)
    }

    fn int_lit(&self, v: i128, exp: Option<&Ty>, span: Span) -> Res<TExpr> {
        let ty = match exp {
            Some(Ty::Int(it)) => Ty::Int(*it),
            Some(Ty::F64) => return Ok(mk(Ty::F64, TK::Float(v as f64), span)),
            Some(Ty::F32) => return Ok(mk(Ty::F32, TK::Float(v as f64), span)),
            _ => Ty::i64(),
        };
        let Ty::Int(it) = ty else { unreachable!() };
        if v < it.min() || v > it.max() {
            return Err(Diag::new(span, format!("literal `{}` does not fit in `{}`", v, self.ty_name(&ty))));
        }
        Ok(mk(ty, TK::Int(v), span))
    }

    pub fn ident(&mut self, cx: &mut FnCx, name: &str, exp: Option<&Ty>, span: Span) -> Res<TExpr> {
        if let Some(id) = cx.lookup(name) {
            let ty = cx.locals[id as usize].ty.clone();
            if matches!(ty, Ty::Block(..)) && !std::mem::take(&mut cx.block_use) {
                return Err(Diag::new(
                    span,
                    format!(
                        "`{}` is a `block fn`: it can only be called, or passed on as an argument for a `block fn` parameter",
                        name
                    ),
                ));
            }
            cx.block_use = false;
            return Ok(mk(ty, TK::Local(id), span));
        }
        if let Some(r) = crate::vmcheck::vm_ident(self, cx, name, span)? {
            return Ok(r);
        }
        if name == "unreachable" {
            return Ok(mk(Ty::Never, TK::Unreachable, span));
        }
        if name == "None" {
            return match exp {
                Some(t @ Ty::Option(_)) => Ok(mk(t.clone(), TK::Variant(0, vec![]), span)),
                _ => Err(Diag::new(span, "cannot infer the type of `None` (annotate it: `var x: ?T = None;`)")),
            };
        }
        if let Some((def, _)) = self.value_def(cx.module, name) {
            if let Def::GenericFn(g) = def {
                return self.generic_fn_value(g, exp, span);
            }
            self.check_unsafe_def(cx, &def, span)?;
            return self.def_value(def, name, span);
        }
        // Types are not values: a variant of the expected type may have the
        // name of a type in scope (`Arr(..)` with `object Arr` imported).
        if let Some(t) = exp {
            if let Some(idx) = self.variant_index(t, name) {
                return self.make_variant(cx, t, idx, &[], span);
            }
        }
        if let Some((def, _)) = self.scopes[cx.module].get(name).cloned() {
            self.check_unsafe_def(cx, &def, span)?;
            return self.def_value(def, name, span);
        }
        let hint = match name {
            "next" | "jump" | "redo" | "exit" | "raise" | "fail" | "become" | "use_variant" => format!(
                " (`{}` is a control word: it is only available in op handlers, vm functions, entries and `on raise`)",
                name
            ),
            _ => String::new(),
        };
        Err(Diag::new(span, format!("unknown name `{}`{}", name, hint)))
    }

    /// Naming a `#[unsafe]` function (to call it or take it as a value)
    /// needs an `unsafe { }` block.
    pub fn check_unsafe_def(&self, cx: &FnCx, def: &Def, span: Span) -> Res<()> {
        let name = match def {
            Def::Fn(id) if self.unsafe_fns.contains(id) => self.prog.fns[*id as usize].name.clone(),
            Def::GenericFn(g) if self.generics.fns[*g as usize].decl.attrs.has("unsafe") => {
                self.generics.fns[*g as usize].decl.name.name.clone()
            }
            _ => return Ok(()),
        };
        if cx.unsafe_depth == 0 {
            let name = name.rsplit('.').next().unwrap_or(&name).replace("__", ".");
            return Err(Diag::new(
                span,
                format!("`{}` is unsafe: nothing checks what it promises; wrap the call in `unsafe {{ }}`", name),
            ));
        }
        Ok(())
    }

    pub fn def_value(&mut self, def: Def, name: &str, span: Span) -> Res<TExpr> {
        match def {
            Def::Fn(id) => {
                let f = &self.prog.fns[id as usize];
                Ok(mk(f.fn_ty(), TK::FnRef(id), span))
            }
            Def::Const(id) => {
                let mut v = self.const_value(id, span)?;
                v.span = span;
                Ok(v)
            }
            Def::Global(id) => Ok(mk(self.prog.globals[id as usize].ty.clone(), TK::Global(id), span)),
            _ => Err(Diag::new(span, format!("`{}` is not a value", name))),
        }
    }

    /// If `e` names a type (`Point`, `mod.Point`), return it.
    pub fn type_path(&self, cx: &FnCx, e: &Expr) -> Option<Ty> {
        match &e.kind {
            ExprKind::Ident(n) if cx.lookup(n).is_none() => {
                if let Some(t) = prim_type(n) {
                    return Some(t);
                }
                if let Some((_, t)) = self.tenv.iter().rev().find(|(x, _)| x == n) {
                    return Some(t.clone());
                }
                match self.scopes[cx.module].get(n) {
                    Some((Def::Struct(i), _)) => Some(Ty::Struct(*i)),
                    Some((Def::Enum(i), _)) => Some(Ty::Enum(*i)),
                    Some((Def::Value(i), _)) => Some(Ty::Value(*i)),
                    Some((Def::Alias(a), _)) => self.aliases[*a as usize].ty.clone(),
                    _ => None,
                }
            }
            ExprKind::Field(base, name) => {
                // `Vm.Op`: a generated type (`Vm.Op.ADD`, `Vm.Op.COUNT`).
                match self.vm_item(cx, base, &name.name) {
                    Some(Def::Struct(i)) => return Some(Ty::Struct(i)),
                    Some(Def::Enum(i)) => return Some(Ty::Enum(i)),
                    _ => {}
                }
                let m = self.module_path(cx, base)?;
                match self.scopes[m].get(&name.name) {
                    Some((Def::Struct(i), true)) => Some(Ty::Struct(*i)),
                    Some((Def::Enum(i), true)) => Some(Ty::Enum(*i)),
                    Some((Def::Value(i), true)) => Some(Ty::Value(*i)),
                    Some((Def::Alias(a), true)) => self.aliases[*a as usize].ty.clone(),
                    _ => None,
                }
            }
            _ => None,
        }
    }

    fn module_path(&self, cx: &FnCx, e: &Expr) -> Option<usize> {
        match &e.kind {
            ExprKind::Ident(n) if cx.lookup(n).is_none() => match self.scopes[cx.module].get(n) {
                Some((Def::Module(m), _)) => Some(*m),
                _ => None,
            },
            _ => None,
        }
    }

    /// `Vm.X` / `Vm.emit.X`: generated vm items live in the vm's module as `Vm__X`.
    pub fn vm_item(&self, cx: &FnCx, base: &Expr, name: &str) -> Option<Def> {
        let vm_of = |e: &Expr| -> Option<u32> {
            match &e.kind {
                ExprKind::Ident(n) if cx.lookup(n).is_none() => match self.scopes[cx.module].get(n) {
                    Some((Def::Vm(i), _)) => Some(*i),
                    _ => None,
                },
                _ => None,
            }
        };
        let (vm, prefix) = match &base.kind {
            ExprKind::Field(b2, mid) if mid.name == "emit" => (vm_of(b2)?, "emit_"),
            _ => (vm_of(base)?, ""),
        };
        let (m, decl) = &self.vm_decls[vm as usize];
        self.scopes[*m].get(&format!("{}__{}{}", decl.name.name, prefix, name)).map(|(d, _)| d.clone())
    }

    fn field(&mut self, cx: &mut FnCx, base: &Expr, name: &Ident, exp: Option<&Ty>, span: Span) -> Res<TExpr> {
        if let Some(r) = crate::vmcheck::vm_field(self, cx, base, name, span)? {
            return Ok(r);
        }
        if let Some(def) = self.vm_item(cx, base, &name.name) {
            self.check_unsafe_def(cx, &def, span)?;
            return self.def_value(def, &name.name, span);
        }
        if let Some(m) = self.module_path(cx, base) {
            let Some((def, is_pub)) = self.value_def(m, &name.name).or_else(|| self.scopes[m].get(&name.name).cloned()) else {
                return Err(Diag::new(name.span, format!("`{}` not found in module", name.name)));
            };
            if !is_pub {
                return Err(Diag::new(name.span, format!("`{}` is private", name.name)));
            }
            self.check_unsafe_def(cx, &def, span)?;
            return self.def_value(def, &name.name, span);
        }
        if let Some(g) = self.generic_type_path(cx, base) {
            if let Some(v) = self.generic_variant(cx, g, name, &[], exp, span)? {
                return Ok(v);
            }
            let gname = self.generics.types[g as usize].name.clone();
            return Err(Diag::new(
                name.span,
                format!("no variant `{}` in `{}` (a generic type's functions can only be called)", name.name, gname),
            ));
        }
        if let Some(t) = self.type_path(cx, base) {
            if let Some(idx) = self.variant_index(&t, &name.name) {
                return self.make_variant(cx, &t, idx, &[], span);
            }
            if let (Ty::Enum(eid), "COUNT") = (&t, name.name.as_str()) {
                // The number of variants, typed like an integer literal.
                let n = self.prog.enums[*eid as usize].variants.len() as i128;
                return self.int_lit(n, exp, span);
            }
            if let Some(f) = self.methods.get(&(t.clone(), name.name.clone())).copied() {
                return self.def_value(Def::Fn(f), &name.name, span);
            }
            if let Some(gf) = self.generics.ty_methods.get(&(t.clone(), name.name.clone())).copied() {
                return self.generic_fn_value(gf, exp, span);
            }
            return Err(Diag::new(name.span, format!("no variant or function `{}` in `{}`", name.name, self.ty_name(&t))));
        }
        let _ = exp;
        let b = self.expr(cx, base, None)?;
        self.field_of(b, name, span)
    }

    /// Fields whose names start with `__` are private to the file that
    /// declares their struct. That is how generated types stay opaque:
    /// `Vm.Verified`'s code field is reachable only from the vm's
    /// expansion, so only `Vm.verify` (and `Vm.assume_verified`) build one.
    fn check_private_field(&self, sid: u32, f: &Ident) -> Res<()> {
        if !f.name.starts_with("__") {
            return Ok(());
        }
        match self.struct_files.get(&sid) {
            Some(&file) if file != f.span.file => {
                let sname = self.prog.structs[sid as usize].name.clone();
                let sname = sname.rsplit('.').next().unwrap_or(&sname).replace("__", ".");
                Err(Diag::new(
                    f.span,
                    format!("field `{}` of `{}` is private to the file that declares it", f.name, sname),
                ))
            }
            _ => Ok(()),
        }
    }

    pub fn field_of(&mut self, b: TExpr, name: &Ident, span: Span) -> Res<TExpr> {
        let b = self.auto_deref(b);
        match b.ty.clone() {
            Ty::Struct(sid) => {
                if let Some((tn, et)) = &self.prog.structs[sid as usize].trailing {
                    if *tn == name.name {
                        let et = et.clone();
                        let TK::Deref(obj) = b.kind else {
                            return Err(Diag::new(name.span, "trailing arrays are only reachable through a Gc reference"));
                        };
                        let words = mk(Ty::Slice(Box::new(et)), TK::Trailing(obj), span);
                        // `code: verified [W] by Vm` reads as the proof:
                        // `Vm__Verified { __code: <the trailing array> }`,
                        // a value, not a place, so it cannot be assigned.
                        if let Some(v) = self.prog.structs[sid as usize].verified {
                            return Ok(mk(Ty::Struct(v), TK::Struct(vec![words]), span));
                        }
                        return Ok(words);
                    }
                }
                self.check_private_field(sid, name)?;
                let fields = &self.prog.structs[sid as usize].fields;
                let Some(idx) = fields.iter().position(|(n, _)| *n == name.name) else {
                    return Err(Diag::new(
                        name.span,
                        format!("no field `{}` on type `{}`", name.name, self.ty_name(&b.ty)),
                    ));
                };
                let ft = fields[idx].1.clone();
                Ok(mk(ft, TK::Field(Box::new(b), idx as u32), span))
            }
            Ty::Tuple(ts) => {
                let idx: usize = name
                    .name
                    .parse()
                    .ok()
                    .filter(|i| *i < ts.len())
                    .ok_or_else(|| Diag::new(name.span, format!("no field `{}` on tuple", name.name)))?;
                Ok(mk(ts[idx].clone(), TK::Field(Box::new(b), idx as u32), span))
            }
            Ty::Slice(t) => match name.name.as_str() {
                "len" => Ok(mk(Ty::Int(IntTy::U64), TK::Field(Box::new(b), 1), span)),
                "ptr" => Ok(mk(Ty::Ptr(t), TK::Field(Box::new(b), 0), span)),
                _ => Err(Diag::new(name.span, format!("no field `{}` on a slice", name.name))),
            },
            Ty::Array(_, n) if name.name == "len" => Ok(mk(Ty::Int(IntTy::U64), TK::Int(n as i128), span)),
            t => Err(Diag::new(name.span, format!("no field `{}` on type `{}`", name.name, self.ty_name(&t)))),
        }
    }

    /// Follow pointers: `p.x` where `p: *Point`.
    pub fn auto_deref(&self, mut b: TExpr) -> TExpr {
        loop {
            match &b.ty {
                Ty::Ptr(inner) if matches!(**inner, Ty::Struct(_) | Ty::Tuple(_) | Ty::Slice(_) | Ty::Ptr(_) | Ty::Array(..) | Ty::Gc(_)) => {
                    let t = (**inner).clone();
                    let span = b.span;
                    b = mk(t, TK::Deref(Box::new(b)), span);
                }
                Ty::Gc(inner) => {
                    let t = (**inner).clone();
                    let span = b.span;
                    b = mk(t, TK::Deref(Box::new(b)), span);
                }
                _ => break,
            }
        }
        b
    }

    fn auto_deref_for_index(&self, mut b: TExpr) -> TExpr {
        while let Ty::Ptr(inner) = &b.ty {
            if matches!(**inner, Ty::Array(..)) {
                let t = (**inner).clone();
                let span = b.span;
                b = mk(t, TK::Deref(Box::new(b)), span);
            } else {
                break;
            }
        }
        b
    }

    pub fn make_variant(&mut self, cx: &mut FnCx, ty: &Ty, idx: u32, args: &[Expr], span: Span) -> Res<TExpr> {
        let fields = self.variant_fields(ty, idx);
        if fields.len() != args.len() {
            return Err(Diag::new(span, format!("variant expects {} argument(s), got {}", fields.len(), args.len())));
        }
        let mut out = Vec::new();
        for (a, ft) in args.iter().zip(fields.iter()) {
            out.push(self.expr_coerce(cx, a, ft)?);
        }
        Ok(mk(ty.clone(), TK::Variant(idx, out), span))
    }

    fn variant_ctor_expected(&mut self, cx: &mut FnCx, name: &str, args: &[Expr], exp: Option<&Ty>, span: Span) -> Res<TExpr> {
        let Some(t) = exp else {
            return Err(Diag::new(span, format!("cannot infer the type of `.{}`", name)));
        };
        let t = match t {
            // `.Variant` against an option of an enum.
            Ty::Option(inner) if self.variant_index(t, name).is_none() => (**inner).clone(),
            t => t.clone(),
        };
        let Some(idx) = self.variant_index(&t, name) else {
            return Err(Diag::new(span, format!("no variant `{}` in type `{}`", name, self.ty_name(&t))));
        };
        self.make_variant(cx, &t, idx, args, span)
    }

    /// `f<T..>(args)`: a call of a generic function or method with explicit
    /// type arguments (for a method, its own parameters only).
    fn call_type_args(&mut self, cx: &mut FnCx, inner: &Expr, targs: &[TypeExpr], args: &[Expr], exp: Option<&Ty>, span: Span) -> Res<TExpr> {
        let mut explicit = Vec::new();
        for t in targs {
            explicit.push(self.resolve_type(cx.module, t)?);
        }
        let (name, g, recv): (&Ident, u32, Option<TExpr>) = match &inner.kind {
            ExprKind::Ident(n) if cx.lookup(n).is_none() => {
                let id = Ident { name: n.clone(), span: inner.span };
                match self.value_def(cx.module, n) {
                    Some((Def::GenericFn(g), _)) => return self.explicit_call(cx, &id, g, explicit, None, args, exp, span),
                    _ => return Err(Diag::new(inner.span, format!("`{}` is not a generic function", n))),
                }
            }
            ExprKind::Field(base, name) => {
                if let Some(m) = self.module_path(cx, base) {
                    return match self.value_def(m, &name.name) {
                        Some((Def::GenericFn(g), true)) => self.explicit_call(cx, name, g, explicit, None, args, exp, span),
                        Some((Def::GenericFn(_), false)) => Err(Diag::new(name.span, format!("`{}` is private", name.name))),
                        _ => Err(Diag::new(name.span, format!("`{}` is not a generic function", name.name))),
                    };
                }
                if self.generic_type_path(cx, base).is_some() {
                    return Err(Diag::new(
                        span,
                        "explicit type arguments are not supported on a generic type's static functions; annotate the result instead",
                    ));
                }
                if let Some(t) = self.type_path(cx, base) {
                    let Some(g) = self.generics.ty_methods.get(&(t.clone(), name.name.clone())).copied() else {
                        return Err(Diag::new(name.span, format!("`{}` is not a generic method of `{}`", name.name, self.ty_name(&t))));
                    };
                    return self.explicit_call(cx, name, g, explicit, None, args, exp, span);
                }
                let recv = self.expr(cx, base, None)?;
                let mut base_ty = recv.ty.clone();
                while let Ty::Ptr(inner) | Ty::Gc(inner) = base_ty {
                    base_ty = *inner;
                }
                let found = match base_ty {
                    Ty::Opaque(_) => self.trait_method_for(&base_ty, &name.name, false, name.span)?,
                    _ => None,
                };
                if let Some((gf, known)) = found {
                    let own = self.generics.fns[gf as usize].params.len() - known.len();
                    if explicit.len() != own {
                        return Err(Diag::new(name.span, format!("`{}` takes {} type argument(s), got {}", name.name, own, explicit.len())));
                    }
                    let mut all = known;
                    all.extend(explicit.into_iter().map(Some));
                    return self.generic_call(cx, gf, vec![recv], all, args, exp, span);
                }
                let g = match self.generics.inst_of.get(&base_ty).cloned() {
                    Some((gt, rargs)) => match self.generics.methods.get(&(gt, name.name.clone())).copied() {
                        Some(g) => {
                            let mut all = rargs;
                            all.extend(explicit.drain(..));
                            explicit = all;
                            Some(g)
                        }
                        None => None,
                    },
                    None => self.generics.ty_methods.get(&(base_ty.clone(), name.name.clone())).copied(),
                };
                let Some(g) = g else {
                    return Err(Diag::new(name.span, format!("no generic method `{}` on type `{}`", name.name, self.ty_name(&recv.ty))));
                };
                (name, g, Some(recv))
            }
            _ => return Err(Diag::new(inner.span, "explicit type arguments need a generic function or method name")),
        };
        self.explicit_call(cx, name, g, explicit, recv, args, exp, span)
    }

    /// The generic function (or generic method of a plain type) that `e`
    /// names, if any: `f`, `mod.f`, `Type.f`, `mod.Type.f`.
    pub fn generic_fn_named(&self, cx: &FnCx, e: &Expr) -> Res<Option<(u32, Ident)>> {
        match &e.kind {
            ExprKind::Ident(n) if cx.lookup(n).is_none() => match self.values[cx.module].get(n) {
                Some((Def::GenericFn(g), _)) => Ok(Some((*g, Ident { name: n.clone(), span: e.span }))),
                _ => Ok(None),
            },
            ExprKind::Field(base, name) => {
                if let Some(m) = self.module_path(cx, base) {
                    return match self.values[m].get(&name.name) {
                        Some((Def::GenericFn(g), true)) => Ok(Some((*g, name.clone()))),
                        Some((Def::GenericFn(_), false)) => Err(Diag::new(name.span, format!("`{}` is private", name.name))),
                        _ => Ok(None),
                    };
                }
                if let Some(t) = self.type_path(cx, base) {
                    return Ok(self.generics.ty_methods.get(&(t, name.name.clone())).map(|g| (*g, name.clone())));
                }
                Ok(None)
            }
            _ => Ok(None),
        }
    }

    /// `f<T..>` used as a value: the instance of a generic function.
    fn type_args_value(&mut self, cx: &mut FnCx, inner: &Expr, targs: &[TypeExpr], span: Span) -> Res<TExpr> {
        let Some((g, name)) = self.generic_fn_named(cx, inner)? else {
            if let ExprKind::Field(base, _) = &inner.kind {
                if self.generic_type_path(cx, base).is_some() {
                    return Err(Diag::new(span, "explicit type arguments are not supported on a generic type's functions"));
                }
            }
            return Err(Diag::new(inner.span, "explicit type arguments need a generic function name"));
        };
        let mut explicit = Vec::new();
        for t in targs {
            explicit.push(self.resolve_type(cx.module, t)?);
        }
        let n = self.generics.fns[g as usize].params.len();
        if explicit.len() != n {
            return Err(Diag::new(name.span, format!("`{}` takes {} type argument(s), got {}", name.name, n, explicit.len())));
        }
        let fid = self.instantiate_fn(g, explicit, span)?;
        Ok(mk(self.prog.fns[fid as usize].fn_ty(), TK::FnRef(fid), span))
    }

    #[allow(clippy::too_many_arguments)]
    fn explicit_call(
        &mut self,
        cx: &mut FnCx,
        name: &Ident,
        g: u32,
        targs: Vec<Ty>,
        recv: Option<TExpr>,
        args: &[Expr],
        exp: Option<&Ty>,
        span: Span,
    ) -> Res<TExpr> {
        let gf = &self.generics.fns[g as usize];
        let (nall, nrecv) = (gf.params.len(), gf.decl.recv_generics);
        // The receiver's arguments come from the receiver (methods) or are
        // not given (static calls); count only the function's own.
        let given_own = if recv.is_some() { targs.len() - nrecv } else { targs.len() };
        if given_own != nall - nrecv {
            return Err(Diag::new(name.span, format!("`{}` takes {} type argument(s), got {}", name.name, nall - nrecv, given_own)));
        }
        if recv.is_none() && nrecv > 0 {
            return Err(Diag::new(name.span, format!("`{}` is a method of a generic type: call it on a value", name.name)));
        }
        let known = targs.into_iter().map(Some).collect();
        self.generic_call(cx, g, recv.into_iter().collect(), known, args, exp, span)
    }

    fn call(&mut self, cx: &mut FnCx, callee: &Expr, args: &[Expr], exp: Option<&Ty>, span: Span) -> Res<TExpr> {
        match &callee.kind {
            ExprKind::TypeArgs(inner, targs) => return self.call_type_args(cx, inner, targs, args, exp, span),
            ExprKind::Ident(name) if cx.lookup(name).is_none() => {
                if let Some((Def::GenericFn(g), _)) = self.value_def(cx.module, name) {
                    return self.generic_call(cx, g, vec![], vec![], args, exp, span);
                }
                if let Some(r) = self.builtin_call(cx, name, args, exp, span)? {
                    return Ok(r);
                }
                if name == "Some" {
                    if args.len() != 1 {
                        return Err(Diag::new(span, "`Some` takes one argument"));
                    }
                    let inner_exp = match exp {
                        Some(Ty::Option(t)) => Some((**t).clone()),
                        _ => None,
                    };
                    let v = self.expr(cx, &args[0], inner_exp.as_ref())?;
                    let v = match &inner_exp {
                        Some(t) => self.coerce(v, t)?,
                        None => v,
                    };
                    return Ok(mk(Ty::Option(Box::new(v.ty.clone())), TK::Variant(1, vec![v]), span));
                }
                if name == "Ok" || name == "Err" {
                    let Some(t @ Ty::Result(..)) = exp else {
                        return Err(Diag::new(
                            span,
                            format!(
                                "cannot infer the type of `{}`: a `Result` needs both its types, and inference is local (annotate it: `var r: Result<T, E> = {}(..);`)",
                                name, name
                            ),
                        ));
                    };
                    let idx = if name == "Ok" { 0 } else { 1 };
                    return self.make_variant(cx, &t.clone(), idx, args, span);
                }
                if self.value_def(cx.module, name).is_none() && !crate::vmcheck::is_vm_fn(self, cx, name) {
                    if let Some(t) = exp {
                        if let Some(idx) = self.variant_index(t, name) {
                            return self.make_variant(cx, &t.clone(), idx, args, span);
                        }
                    }
                }
                if let Some(r) = crate::vmcheck::vm_fn_call(self, cx, name, args, span)? {
                    return Ok(r);
                }
            }
            ExprKind::DotVariant(name) => return self.variant_ctor_expected(cx, name, args, exp, span),
            ExprKind::Field(base, name) => {
                if let Some(m) = self.module_path(cx, base) {
                    if let Some((Def::GenericFn(g), is_pub)) = self.value_def(m, &name.name) {
                        if !is_pub {
                            return Err(Diag::new(name.span, format!("`{}` is private", name.name)));
                        }
                        return self.generic_call(cx, g, vec![], vec![], args, exp, span);
                    }
                }
                if let Some(g) = self.generic_type_path(cx, base) {
                    if let Some(v) = self.generic_variant(cx, g, name, args, exp, span)? {
                        return Ok(v);
                    }
                    let inst = self.instance_from_expected(g, exp);
                    let Some(gf) = self.generics.methods.get(&(g, name.name.clone())).copied() else {
                        let gname = self.generics.types[g as usize].name.clone();
                        return Err(Diag::new(
                            name.span,
                            format!("no function `{}` in `{}` (or its type arguments cannot be inferred)", name.name, gname),
                        ));
                    };
                    let known = match inst {
                        Some(t) => self.generics.inst_of[&t].1.iter().cloned().map(Some).collect(),
                        None => vec![],
                    };
                    return self.generic_call(cx, gf, vec![], known, args, exp, span);
                }
                if let Some(def) = self.vm_item(cx, base, &name.name) {
                    if let Def::GenericFn(g) = def {
                        // `Vm.disasm(code, &out)`: generic over the sink.
                        return self.generic_call(cx, g, vec![], vec![], args, exp, span);
                    }
                    self.check_unsafe_def(cx, &def, callee.span)?;
                    let f = self.def_value(def, &name.name, callee.span)?;
                    return self.finish_call(cx, f, vec![], args, span);
                }
                if self.module_path(cx, base).is_none() {
                    if let Some(t) = self.type_path(cx, base) {
                        if let Some(idx) = self.variant_index(&t, &name.name) {
                            return self.make_variant(cx, &t, idx, args, span);
                        }
                        if let Some(r) = self.static_intrinsic(cx, &t, &name.name, args, span)? {
                            return Ok(r);
                        }
                        // `IntBox.make(..)` with `type IntBox = Box<i64>`: the
                        // generic type's function, with the instance's arguments.
                        if let Some((g, targs)) = self.generics.inst_of.get(&t).cloned() {
                            if let Some(gf) = self.generics.methods.get(&(g, name.name.clone())).copied() {
                                let known = targs.into_iter().map(Some).collect();
                                return self.generic_call(cx, gf, vec![], known, args, exp, span);
                            }
                        }
                        if let Some(gf) = self.generics.ty_methods.get(&(t.clone(), name.name.clone())).copied() {
                            return self.generic_call(cx, gf, vec![], vec![], args, exp, span);
                        }
                        let Some(f) = self.methods.get(&(t.clone(), name.name.clone())).copied() else {
                            // A trait's function through a bound (`T.zero()`) or a default.
                            if let Some((gf, known)) = self.trait_method_for(&t, &name.name, !matches!(t, Ty::Opaque(_)), name.span)? {
                                return self.generic_call(cx, gf, vec![], known, args, exp, span);
                            }
                            if let Ty::Opaque(o) = t {
                                return Err(self.no_bound_method(o, &name.name, name.span));
                            }
                            return Err(Diag::new(
                                name.span,
                                format!("no variant or function `{}` in `{}`", name.name, self.ty_name(&t)),
                            ));
                        };
                        let fref = self.def_value(Def::Fn(f), &name.name, callee.span)?;
                        return self.finish_call(cx, fref, vec![], args, span);
                    }
                    // Method call.
                    let recv = self.expr(cx, base, None)?;
                    return self.method_call(cx, recv, name, args, exp, span);
                }
            }
            _ => {}
        }
        // A `block fn` local may be called.
        cx.block_use = matches!(callee.kind, ExprKind::Ident(_));
        let f = self.expr(cx, callee, None);
        cx.block_use = false;
        let f = f?;
        self.finish_call(cx, f, vec![], args, span)
    }

    pub fn finish_call(&mut self, cx: &mut FnCx, f: TExpr, mut pre: Vec<TExpr>, args: &[Expr], span: Span) -> Res<TExpr> {
        let (Ty::Fn(params, ret, _) | Ty::Block(params, ret, _) | Ty::Closure(params, ret, _)) = callee_sig(&f.ty) else {
            return Err(Diag::new(f.span, format!("cannot call a value of type `{}`", self.ty_name(&f.ty))));
        };
        let variadic = matches!(f.kind, TK::FnRef(id) if self.prog.fns[id as usize].variadic);
        let n = pre.len() + args.len();
        if n != params.len() && !(variadic && n >= params.len()) {
            return Err(Diag::new(span, format!("expected {} argument(s), got {}", params.len(), n)));
        }
        let npre = pre.len();
        for (i, a) in args.iter().enumerate() {
            let pi = i + npre;
            if pi < params.len() {
                let pt = params[pi].clone();
                if matches!(pt, Ty::Block(..)) {
                    pre.push(self.block_arg(cx, a, &pt)?);
                } else {
                    pre.push(self.expr_coerce(cx, a, &pt)?);
                }
            } else {
                let mut v = self.expr(cx, a, None)?;
                if v.ty == Ty::F32 {
                    v = mk(Ty::F64, TK::Cast(Box::new(v)), a.span);
                }
                // C has no odd-width integers: pass an i48 as a (sign- or
                // zero-extended) 64-bit integer, like C's own promotions.
                if let Ty::Int(it) = v.ty {
                    if !matches!(it.bits, 8 | 16 | 32 | 64) {
                        let wide = IntTy { signed: it.signed, bits: 64 };
                        v = mk(Ty::Int(wide), TK::Cast(Box::new(v)), a.span);
                    }
                }
                pre.push(v);
            }
        }
        let call = mk(*ret, TK::Call(Box::new(f), pre), span);
        crate::vmcheck::wrap_effects(self, cx, call)
    }

    /// `Lookup.Missing`, `Lookup.Own(i)`: a variant of generic enum `g`.
    /// The instance comes from the expected type, or else from the
    /// payload arguments. None if `g` has no variant `name`.
    fn generic_variant(&mut self, cx: &mut FnCx, g: u32, name: &Ident, args: &[Expr], exp: Option<&Ty>, span: Span) -> Res<Option<TExpr>> {
        let gt = &self.generics.types[g as usize];
        let crate::generics::GenDecl::Enum(ed) = gt.decl.clone() else { return Ok(None) };
        let Some(vdecl) = ed.variants.iter().find(|v| v.name.name == name.name) else { return Ok(None) };
        if let Some(t) = self.instance_from_expected(g, exp) {
            let idx = self.variant_index(&t, &name.name).unwrap();
            return self.make_variant(cx, &t, idx, args, span).map(Some);
        }
        let (module, params, gname) = (gt.module, gt.params.clone(), gt.name.clone());
        let mut b: Vec<Option<Ty>> = vec![None; params.len()];
        if vdecl.fields.len() == args.len() {
            for (ft, a) in vdecl.fields.iter().zip(args) {
                let env = params.iter().enumerate().map(|(i, p)| (p.clone(), Ty::Param(i as u32))).collect();
                let pt = self.with_tenv(env, |c| c.resolve_type(module, ft))?;
                if !crate::generics::contains_param(&pt, &self.generics) {
                    continue;
                }
                // Check a clone to learn the type; the argument is checked again.
                let mut probe = FnCx::new(cx.module, cx.ret.clone());
                probe.scopes = cx.scopes.clone();
                probe.locals = cx.locals.clone();
                probe.vm = cx.vm.clone();
                if let Ok(v) = self.expr(&mut probe, a, None) {
                    self.unify(&pt, &v.ty, &mut b);
                }
            }
        }
        let mut targs = Vec::new();
        for (i, t) in b.into_iter().enumerate() {
            match t {
                Some(t) => targs.push(t),
                None => {
                    return Err(Diag::new(
                        span,
                        format!(
                            "cannot infer type parameter `{}` of `{}.{}`: use it where a `{}<..>` is expected",
                            params[i], gname, name.name, gname
                        ),
                    ))
                }
            }
        }
        let t = self.instantiate_type(g, targs, span)?;
        let idx = self.variant_index(&t, &name.name).unwrap();
        self.make_variant(cx, &t, idx, args, span).map(Some)
    }

    /// `Vec` (a generic type named in expression position).
    fn generic_type_path(&self, cx: &FnCx, e: &Expr) -> Option<u32> {
        match &e.kind {
            ExprKind::Ident(n) if cx.lookup(n).is_none() => match self.scopes[cx.module].get(n) {
                Some((Def::GenericType(g), _)) => Some(*g),
                _ => None,
            },
            ExprKind::Field(base, n) => {
                let m = self.module_path(cx, base)?;
                match self.scopes[m].get(&n.name) {
                    Some((Def::GenericType(g), true)) => Some(*g),
                    _ => None,
                }
            }
            _ => None,
        }
    }

    pub(crate) fn method_call(&mut self, cx: &mut FnCx, recv: TExpr, name: &Ident, args: &[Expr], exp: Option<&Ty>, span: Span) -> Res<TExpr> {
        let mut base_ty = recv.ty.clone();
        while let Ty::Ptr(inner) | Ty::Gc(inner) = base_ty {
            base_ty = *inner;
        }
        if let Ty::Opaque(o) = base_ty {
            // A bounded type parameter: only its bounds' methods.
            return match self.trait_method_for(&base_ty, &name.name, false, name.span)? {
                Some((gf, known)) => self.generic_call(cx, gf, vec![recv], known, args, exp, span),
                None => Err(self.no_bound_method(o, &name.name, name.span)),
            };
        }
        if let Some((g, targs)) = self.generics.inst_of.get(&base_ty).cloned() {
            if let Some(gf) = self.generics.methods.get(&(g, name.name.clone())).copied() {
                let known = targs.into_iter().map(Some).collect();
                return self.generic_call(cx, gf, vec![recv], known, args, exp, span);
            }
        }
        if let Some(gf) = self.generics.ty_methods.get(&(base_ty.clone(), name.name.clone())).copied() {
            return self.generic_call(cx, gf, vec![recv], vec![], args, exp, span);
        }
        if let Some(fid) = self.methods.get(&(base_ty.clone(), name.name.clone())).copied() {
            let fdef = &self.prog.fns[fid as usize];
            let Some(p0) = fdef.param_tys.first().cloned() else {
                return Err(Diag::new(name.span, format!("`{}` takes no receiver", name.name)));
            };
            let fty = fdef.fn_ty();
            let mut r = recv;
            loop {
                if r.ty == p0 {
                    break;
                }
                if p0 == Ty::Ptr(Box::new(r.ty.clone())) {
                    let span = r.span;
                    r = mk(p0.clone(), TK::AddrOf(Box::new(r)), span);
                    break;
                }
                if let Ty::Ptr(inner) = r.ty.clone() {
                    let span = r.span;
                    r = mk(*inner, TK::Deref(Box::new(r)), span);
                    continue;
                }
                return Err(Diag::new(span, format!("receiver type mismatch for `{}`", name.name)));
            }
            let f = mk(fty, TK::FnRef(fid), name.span);
            return self.finish_call(cx, f, vec![r], args, span);
        }
        if let Some(r) = self.intrinsic_method(cx, recv.clone(), &name.name, args, span)? {
            return Ok(r);
        }
        // A trait's default method, for a type that implements the trait.
        if let Some((gf, known)) = self.trait_method_for(&base_ty, &name.name, true, name.span)? {
            return self.generic_call(cx, gf, vec![recv], known, args, exp, span);
        }
        // A field holding a function pointer: `(n.f)(x)` style written as `n.f(x)`.
        if let Ok(fv) = self.field_of(recv.clone(), name, name.span) {
            if matches!(callee_sig(&fv.ty), Ty::Fn(..) | Ty::Closure(..)) {
                return self.finish_call(cx, fv, vec![], args, span);
            }
        }
        Err(Diag::new(name.span, format!("no method `{}` on type `{}`", name.name, self.ty_name(&recv.ty))))
    }

    fn intrinsic_method(&mut self, cx: &mut FnCx, recv: TExpr, name: &str, args: &[Expr], span: Span) -> Res<Option<TExpr>> {
        let recv = {
            let mut r = recv;
            while let Ty::Ptr(inner) = &r.ty {
                if matches!(**inner, Ty::Option(_) | Ty::Int(_) | Ty::F64 | Ty::F32) && name != "offset" && name != "slice" {
                    let t = (**inner).clone();
                    let s = r.span;
                    r = mk(t, TK::Deref(Box::new(r)), s);
                } else {
                    break;
                }
            }
            r
        };
        let t = recv.ty.clone();
        let argc = |n: usize| -> Res<()> {
            if args.len() != n {
                return Err(Diag::new(span, format!("`{}` takes {} argument(s)", name, n)));
            }
            Ok(())
        };
        let r = match (&t, name) {
            (Ty::Int(_), "add_checked" | "sub_checked" | "mul_checked") => {
                argc(1)?;
                let y = self.expr_coerce(cx, &args[0], &t)?;
                let intr = match name {
                    "add_checked" => Intr::AddChecked,
                    "sub_checked" => Intr::SubChecked,
                    _ => Intr::MulChecked,
                };
                mk(Ty::Option(Box::new(t.clone())), TK::Intrinsic(intr, vec![recv, y]), span)
            }
            (Ty::Int(_), "clz" | "ctz" | "popcount") => {
                argc(0)?;
                let intr = match name {
                    "clz" => Intr::Clz,
                    "ctz" => Intr::Ctz,
                    _ => Intr::Popcount,
                };
                mk(t.clone(), TK::Intrinsic(intr, vec![recv]), span)
            }
            (Ty::F64 | Ty::F32, "to_bits") => {
                argc(0)?;
                let bt = if t == Ty::F64 { IntTy::U64 } else { IntTy::U32 };
                mk(Ty::Int(bt), TK::Intrinsic(Intr::FloatBits, vec![recv]), span)
            }
            (Ty::F64 | Ty::F32, "sqrt" | "floor" | "ceil" | "trunc" | "nearest" | "abs") => {
                argc(0)?;
                let intr = match name {
                    "sqrt" => Intr::Sqrt,
                    "floor" => Intr::Floor,
                    "ceil" => Intr::Ceil,
                    "trunc" => Intr::Trunc,
                    "nearest" => Intr::Nearest,
                    _ => Intr::Fabs,
                };
                mk(t.clone(), TK::Intrinsic(intr, vec![recv]), span)
            }
            (Ty::F64 | Ty::F32, "is_nan") => {
                argc(0)?;
                mk(Ty::Bool, TK::Intrinsic(Intr::IsNan, vec![recv]), span)
            }
            (Ty::F64 | Ty::F32, "copysign" | "min" | "max" | "minimum" | "maximum") => {
                argc(1)?;
                let y = self.expr_coerce(cx, &args[0], &t)?;
                let intr = match name {
                    "copysign" => Intr::Copysign,
                    "min" => Intr::MinNum,
                    "max" => Intr::MaxNum,
                    "minimum" => Intr::Minimum,
                    _ => Intr::Maximum,
                };
                mk(t.clone(), TK::Intrinsic(intr, vec![recv, y]), span)
            }
            (Ty::Option(_), "is_some" | "is_none") => {
                argc(0)?;
                let intr = if name == "is_some" { Intr::IsSome } else { Intr::IsNone };
                mk(Ty::Bool, TK::Intrinsic(intr, vec![recv]), span)
            }
            (Ty::Option(inner), "unwrap") => {
                argc(0)?;
                mk((**inner).clone(), TK::Intrinsic(Intr::Unwrap, vec![recv]), span)
            }
            (Ty::Option(inner), "unwrap_or") => {
                argc(1)?;
                let d = self.expr_coerce(cx, &args[0], inner)?;
                mk((**inner).clone(), TK::Intrinsic(Intr::UnwrapOr, vec![recv, d]), span)
            }
            (Ty::Ptr(_), "offset") => {
                argc(1)?;
                let n = self.expr_coerce(cx, &args[0], &Ty::i64())?;
                mk(t.clone(), TK::Intrinsic(Intr::PtrOffset, vec![recv, n]), span)
            }
            (Ty::Ptr(inner), "slice") => {
                argc(1)?;
                let n = self.expr_coerce(cx, &args[0], &Ty::Int(IntTy::U64))?;
                mk(Ty::Slice(inner.clone()), TK::Intrinsic(Intr::SliceFromRaw, vec![recv, n]), span)
            }
            _ => return Ok(None),
        };
        Ok(Some(r))
    }

    fn static_intrinsic(&mut self, cx: &mut FnCx, t: &Ty, name: &str, args: &[Expr], span: Span) -> Res<Option<TExpr>> {
        match (t, name) {
            (Ty::F64 | Ty::F32, "from_bits") => {
                if args.len() != 1 {
                    return Err(Diag::new(span, "`from_bits` takes one argument"));
                }
                let bt = if *t == Ty::F64 { IntTy::U64 } else { IntTy::U32 };
                let a = self.expr_coerce(cx, &args[0], &Ty::Int(bt))?;
                Ok(Some(mk(t.clone(), TK::Intrinsic(Intr::BitsToFloat, vec![a]), span)))
            }
            _ => Ok(None),
        }
    }

    fn builtin_call(&mut self, cx: &mut FnCx, name: &str, args: &[Expr], exp: Option<&Ty>, span: Span) -> Res<Option<TExpr>> {
        if self.value_def(cx.module, name).is_some() {
            return Ok(None);
        }
        if name == "new" && args.len() == 2 && matches!(args[1].kind, ExprKind::Closure(..)) {
            return self.new_closure(cx, &args[0], &args[1], exp, span).map(Some);
        }
        if (name == "new" || name == "gc_mark") && !self.modules.iter().any(|m| m.path == ["std", "gc"]) {
            return Err(Diag::new(span, format!("`{}` needs the GC runtime: add `import std.gc;`", name)));
        }
        if name == "new" {
            if args.len() != 2 && args.len() != 3 {
                return Err(Diag::new(span, "`new` takes a heap, an object literal, and (for trailing arrays) a length"));
            }
            let h = self.expr(cx, &args[0], None)?;
            if !matches!(h.ty, Ty::Ptr(_)) {
                return Err(Diag::new(args[0].span, "the first argument of `new` must be a pointer to a heap"));
            }
            let want = match exp {
                Some(Ty::Gc(t)) => Some((**t).clone()),
                _ => None,
            };
            let init = self.expr(cx, &args[1], want.as_ref())?;
            let Ty::Struct(sid) = init.ty else {
                return Err(Diag::new(args[1].span, "`new` needs an object literal"));
            };
            let sd = &self.prog.structs[sid as usize];
            if sd.object.is_none() {
                return Err(Diag::new(args[1].span, format!("`{}` is a struct, not an `object`", self.ty_name(&init.ty))));
            }
            // An object with verified code is created only by its vm's
            // `build_verified`, which fills and verifies the code.
            if let Some(v) = sd.verified {
                if self.struct_files.get(&v) != Some(&span.file) {
                    let vname = self.ty_name(&Ty::Struct(v));
                    let vm = vname.trim_start_matches("Verified<").trim_end_matches('>').to_string();
                    let (tn, _) = sd.trailing.clone().unwrap();
                    return Err(Diag::new(
                        span,
                        format!(
                            "`{}` holds verified code (`{}: verified [..] by {}`): create it with `{}.build_verified(heap, {} {{ .. }}, words, ..)`",
                            self.ty_name(&init.ty), tn, vm, vm, self.ty_name(&init.ty)
                        ),
                    ));
                }
            }
            let len = match (&sd.trailing, args.len()) {
                (Some(_), 3) => Some(Box::new(self.expr_coerce(cx, &args[2], &Ty::Int(IntTy::U64))?)),
                (None, 2) => None,
                (Some(_), _) => return Err(Diag::new(span, "this object has a trailing array: pass its length")),
                (None, _) => return Err(Diag::new(span, "this object has no trailing array")),
            };
            let t = Ty::Gc(Box::new(init.ty.clone()));
            return Ok(Some(mk(t, TK::New { heap: Box::new(h), init: Box::new(init), len }, span)));
        }
        if name == "__verified_words" {
            // In a vm's generated code (`Vm.build_verified`): the words of
            // an object's `verified [W] by Vm` array, as a writable slice.
            let [a] = args else {
                return Err(Diag::new(span, "`__verified_words` takes an object"));
            };
            let o = self.expr(cx, a, None)?;
            let sid = match &o.ty {
                Ty::Gc(t) => match **t {
                    Ty::Struct(s) => Some(s),
                    _ => None,
                },
                _ => None,
            };
            let found = sid.and_then(|s| Some((self.prog.structs[s as usize].verified?, self.prog.structs[s as usize].trailing.clone()?)));
            let Some((v, (_, et))) = found else {
                return Err(Diag::new(
                    a.span,
                    format!("`{}` has no `verified [..] by Vm` trailing array", self.ty_name(&o.ty)),
                ));
            };
            if self.struct_files.get(&v) != Some(&span.file) {
                return Err(Diag::new(span, "`__verified_words` is private to the vm's generated code"));
            }
            return Ok(Some(mk(Ty::Slice(Box::new(et)), TK::Trailing(Box::new(o)), span)));
        }
        if name == "__panic" {
            let [Expr { kind: ExprKind::Str(_), .. }] = args else {
                return Err(Diag::new(span, "`__panic` takes a string literal"));
            };
            let m = self.expr(cx, &args[0], None)?;
            return Ok(Some(mk(Ty::Never, TK::Intrinsic(Intr::Panic, vec![m]), span)));
        }
        if name == "gc_mark" {
            if args.len() != 2 {
                return Err(Diag::new(span, "`gc_mark` takes a heap and a value"));
            }
            let h = self.expr(cx, &args[0], None)?;
            let v = self.expr(cx, &args[1], None)?;
            return Ok(Some(mk(Ty::Unit, TK::Intrinsic(Intr::GcMark, vec![h, v]), span)));
        }
        let (intr, ret) = match name {
            "likely" => (Intr::Likely, Ty::Bool),
            "unlikely" => (Intr::Unlikely, Ty::Bool),
            "assume" => (Intr::Assume, Ty::Unit),
            "trap" => (Intr::Trap, Ty::Never),
            _ => return Ok(None),
        };
        let mut out = Vec::new();
        for a in args {
            out.push(self.expr_coerce(cx, a, &Ty::Bool)?);
        }
        let want = if intr == Intr::Trap { 0 } else { 1 };
        if out.len() != want {
            return Err(Diag::new(span, format!("`{}` takes {} argument(s)", name, want)));
        }
        Ok(Some(mk(ret, TK::Intrinsic(intr, out), span)))
    }

    fn unary(&mut self, cx: &mut FnCx, op: UnOp, inner: &Expr, exp: Option<&Ty>, span: Span) -> Res<TExpr> {
        match op {
            UnOp::Neg => {
                if let ExprKind::Int(v) = inner.kind {
                    return self.int_lit(-(v as i128), exp, span);
                }
                let v = self.expr(cx, inner, exp)?;
                if !v.ty.is_numeric() {
                    return Err(Diag::new(span, format!("cannot negate `{}`", self.ty_name(&v.ty))));
                }
                Ok(mk(v.ty.clone(), TK::Unary(op, Box::new(v)), span))
            }
            UnOp::Not => {
                let v = self.expr(cx, inner, exp)?;
                if v.ty != Ty::Bool && !v.ty.is_int() {
                    return Err(Diag::new(span, format!("cannot apply `!` to `{}`", self.ty_name(&v.ty))));
                }
                Ok(mk(v.ty.clone(), TK::Unary(op, Box::new(v)), span))
            }
            UnOp::BitNot => {
                let v = self.expr(cx, inner, exp)?;
                if !v.ty.is_int() {
                    let hint = if v.ty == Ty::Bool { " (use `!` for logical not)" } else { "" };
                    return Err(Diag::new(span, format!("cannot apply `~` to `{}`{}", self.ty_name(&v.ty), hint)));
                }
                Ok(mk(v.ty.clone(), TK::Unary(UnOp::Not, Box::new(v)), span))
            }
            UnOp::Deref => {
                let v = self.expr(cx, inner, exp.map(|t| Ty::Ptr(Box::new(t.clone()))).as_ref())?;
                let Ty::Ptr(t) = v.ty.clone() else {
                    return Err(Diag::new(span, format!("cannot dereference `{}`", self.ty_name(&v.ty))));
                };
                Ok(mk(*t, TK::Deref(Box::new(v)), span))
            }
            UnOp::AddrOf => {
                let inner_exp = match exp {
                    Some(Ty::Ptr(t)) => Some((**t).clone()),
                    _ => None,
                };
                let v = self.expr(cx, inner, inner_exp.as_ref())?;
                if let (TK::Vm(TVm::Word(_) | TVm::WordAt(..)), Some(h)) = (&v.kind, &cx.vm) {
                    let wb = self.vms[h.vm as usize].word.bits as u64 / 8;
                    if crate::layout::align_of(&self.prog, &v.ty) > wb {
                        return Err(Diag::new(
                            span,
                            format!(
                                "cannot take the address of a `{}` cache field: code words are only {}-byte aligned; read and assign the field instead",
                                self.ty_name(&v.ty),
                                wb
                            ),
                        ));
                    }
                }
                let v = if is_place(&v) {
                    v
                } else {
                    // Materialize a temporary.
                    let t = cx.temp(v.ty.clone());
                    let ty = v.ty.clone();
                    let s = v.span;
                    mk(ty.clone(), TK::Block(vec![TStmt::Let(t, Some(v))], Some(Box::new(mk(ty, TK::Local(t), s)))), s)
                };
                Ok(mk(Ty::Ptr(Box::new(v.ty.clone())), TK::AddrOf(Box::new(v)), span))
            }
        }
    }

    fn binary(&mut self, cx: &mut FnCx, op: BinOp, l: &Expr, r: &Expr, exp: Option<&Ty>, span: Span) -> Res<TExpr> {
        use BinOp::*;
        if matches!(op, LAnd | LOr) {
            let a = self.expr_coerce(cx, l, &Ty::Bool)?;
            let b = self.expr_coerce(cx, r, &Ty::Bool)?;
            return Ok(mk(Ty::Bool, TK::Binary(op, Box::new(a), Box::new(b)), span));
        }
        if matches!(op, Add | Sub) {
            // Pointer arithmetic: `p + n`, `p - n` (in elements).
            let a = self.expr(cx, l, exp)?;
            if let Ty::Ptr(_) = a.ty {
                let n = self.expr(cx, r, Some(&Ty::i64()))?;
                if !n.ty.is_int() {
                    return Err(Diag::new(r.span, "pointer offset must be an integer"));
                }
                let n = if n.ty == Ty::i64() { n } else { let s = n.span; mk(Ty::i64(), TK::Cast(Box::new(n)), s) };
                let n = if op == Sub { mk(Ty::i64(), TK::Unary(UnOp::Neg, Box::new(n)), span) } else { n };
                let t = a.ty.clone();
                return Ok(mk(t, TK::Intrinsic(Intr::PtrOffset, vec![a, n]), span));
            }
            let operand_exp = exp;
            let (a, b) = if is_literal_expr(l) && !is_literal_expr(r) {
                let b = self.expr(cx, r, operand_exp)?;
                let a = self.expr_coerce(cx, l, &b.ty.clone())?;
                (a, b)
            } else {
                let at = a.ty.clone();
                let b = self.expr(cx, r, Some(&at))?;
                let b = if a.ty == Ty::Never { b } else { self.coerce(b, &at)? };
                (a, b)
            };
            if !a.ty.is_numeric() {
                return Err(Diag::new(span, format!("operator `{:?}` is not defined for `{}`", op, self.ty_name(&a.ty))));
            }
            let t = a.ty.clone();
            return Ok(mk(t, TK::Binary(op, Box::new(a), Box::new(b)), span));
        }
        let operand_exp = if op.is_cmp() { None } else { exp };
        let (a, b) = if matches!(op, Shl | Shr) {
            // The result has the left operand's type (a literal takes it
            // from the context, or is i64); the amount may be any integer.
            let a = self.expr(cx, l, operand_exp)?;
            let b = if is_literal_expr(r) && a.ty.is_int() {
                self.expr_coerce(cx, r, &a.ty.clone())?
            } else {
                self.expr(cx, r, None)?
            };
            if a.ty.is_int() && !b.ty.is_int() && b.ty != Ty::Never {
                return Err(Diag::new(r.span, format!("shift amount must be an integer, found `{}`", self.ty_name(&b.ty))));
            }
            (a, b)
        } else if is_literal_expr(l) && !is_literal_expr(r) {
            let b = self.expr(cx, r, operand_exp)?;
            let a = self.expr_coerce(cx, l, &b.ty.clone())?;
            (a, b)
        } else {
            let a = self.expr(cx, l, operand_exp)?;
            let at = a.ty.clone();
            let b = self.expr(cx, r, Some(&at))?;
            let b = if a.ty == Ty::Never { b } else { self.coerce(b, &at)? };
            (a, b)
        };
        let t = a.ty.clone();
        let ok = match op {
            Add | Sub | Mul | Div | Rem => t.is_numeric(),
            AddWrap | SubWrap | MulWrap | Shl | Shr => t.is_int(),
            And | Or | Xor => t.is_int() || t == Ty::Bool,
            Eq | Ne => {
                t.is_numeric()
                    || matches!(t, Ty::Bool | Ty::Ptr(_) | Ty::Fn(..) | Ty::Value(_) | Ty::Gc(_))
                    || matches!(&t, Ty::Enum(e) if !self.prog.enums[*e as usize].has_payload())
                    || matches!(&t, Ty::Option(inner) if inner.is_nullable_ptr() || inner.is_numeric()
                        || matches!(**inner, Ty::Bool | Ty::Value(_))
                        || matches!(&**inner, Ty::Enum(e) if !self.prog.enums[*e as usize].has_payload()))
            }
            Lt | Le | Gt | Ge => t.is_numeric() || matches!(t, Ty::Ptr(_)),
            LAnd | LOr => unreachable!(),
        };
        if !ok {
            return Err(Diag::new(span, format!("operator `{:?}` is not defined for `{}`", op, self.ty_name(&t))));
        }
        let ty = if op.is_cmp() { Ty::Bool } else { t };
        Ok(mk(ty, TK::Binary(op, Box::new(a), Box::new(b)), span))
    }

    fn castable(&self, from: &Ty, to: &Ty) -> bool {
        let num = |t: &Ty| t.is_numeric();
        match (from, to) {
            (a, b) if num(a) && num(b) => true,
            (Ty::Bool, Ty::Int(_)) => true,
            (Ty::Ptr(_) | Ty::Fn(..) | Ty::Gc(_), Ty::Ptr(_)) => true,
            (Ty::Gc(_), Ty::Int(it)) => it.bits == 64,
            (Ty::Ptr(_), Ty::Int(it)) | (Ty::Int(it), Ty::Ptr(_)) => it.bits == 64,
            (Ty::Enum(e), Ty::Int(_)) | (Ty::Int(_), Ty::Enum(e)) => {
                let ed = &self.prog.enums[*e as usize];
                !ed.has_payload() && !ed.variants.is_empty()
            }
            (Ty::Value(v), Ty::Int(it)) | (Ty::Int(it), Ty::Value(v)) => {
                it.bits == 64 && self.prog.values[*v as usize].repr != ValueRepr::Boxed
            }
            _ => false,
        }
    }

    /// The span of the first part of a global's initializer that cannot be
    /// emitted as an LLVM constant, if any.
    fn static_init_error(&self, e: &TExpr) -> Option<Span> {
        let all = |es: &[TExpr]| es.iter().find_map(|x| self.static_init_error(x));
        match &e.kind {
            TK::Int(_) | TK::Float(_) | TK::Bool(_) | TK::Str(_) | TK::Unit | TK::FnRef(_) => {
                if matches!(e.ty, Ty::Value(_)) { Some(e.span) } else { None }
            }
            TK::Zeroed => None,
            TK::Struct(es) | TK::Tuple(es) | TK::Array(es) => all(es),
            TK::ArrayRepeat(v, _) => self.static_init_error(v),
            TK::Variant(_, es) => match &e.ty {
                Ty::Enum(_) | Ty::Option(_) | Ty::Result(..) if es.is_empty() => None,
                Ty::Enum(_) => Some(e.span),
                Ty::Option(_) | Ty::Result(..) => all(es),
                _ => Some(e.span),
            },
            _ => Some(e.span),
        }
    }

    fn check_assignable(&self, cx: &FnCx, e: &TExpr, span: Span) -> Res<()> {
        match &e.kind {
            TK::Local(id) => {
                let l = &cx.locals[*id as usize];
                if !l.mutable {
                    return Err(Diag::new(span, format!("cannot assign to `{}`: declared with `let`", l.name)));
                }
                Ok(())
            }
            TK::Deref(_) | TK::Global(_) => Ok(()),
            TK::Field(b, _) => {
                if matches!(b.ty, Ty::Slice(_)) {
                    return Err(Diag::new(span, "cannot assign to a slice's length or pointer"));
                }
                self.check_assignable(cx, b, span)
            }
            TK::Index(b, _) => match b.ty {
                Ty::Array(..) => self.check_assignable(cx, b, span),
                _ => Ok(()),
            },
            TK::Vm(TVm::Word(_) | TVm::WordAt(..)) => Ok(()),
            TK::Block(_, Some(t)) if matches!(t.kind, TK::Local(_)) => Ok(()),
            _ => Err(Diag::new(span, "left side of assignment is not a place")),
        }
    }
}

trait SpanOf {
    fn span(&self) -> Span;
}
impl SpanOf for FnDecl {
    fn span(&self) -> Span {
        self.name.span
    }
}

/// Where the value of `e` comes from, for errors about its type: a
/// block's tail expression rather than its `{`.
fn value_span(e: &TExpr) -> Span {
    match &e.kind {
        TK::Block(_, Some(t)) if t.ty == e.ty => value_span(t),
        _ => e.span,
    }
}

/// Functions, consts and globals live in the value namespace.
pub fn is_value_def(d: &Def) -> bool {
    matches!(d, Def::Fn(_) | Def::GenericFn(_) | Def::Const(_) | Def::Global(_))
}

pub fn placeholder_fn(span: Span) -> FnDef {
    FnDef {
        name: String::new(),
        symbol: String::new(),
        params: vec![],
        param_tys: vec![],
        ret: Ty::Unit,
        locals: vec![],
        body: None,
        linkage: Linkage::Internal,
        variadic: false,
        inline: None,
        cold: false,
        cc: CallConv::C,
        span,
        vm: None,
        vm_role: VmRole::None,
        vm_state: vec![],
        effects: 0,
        sync_effects: 0,
        env: None,
        env_val: vec![],
        env_struct: None,
        env_root: None,
        catch_panic: None,
        track_caller: false,
    }
}

fn short(s: &str) -> String {
    s.rsplit('.').next().unwrap_or(s).to_string()
}

pub fn effects_of(attrs: &Attrs) -> Res<u8> {
    let mut e = 0;
    for (n, args, span) in &attrs.list {
        if n != "uses" {
            continue;
        }
        e |= effect_bits(&args[0], *span)?;
    }
    Ok(e)
}

/// An effect name as a mask, including the effects it implies.
pub fn effect_bits(name: &str, span: Span) -> Res<u8> {
    Ok(match name {
        "sync" => EFF_SYNC,
        "gc" => EFF_SYNC | EFF_GC,
        "reenter" => EFF_SYNC | EFF_GC | EFF_REENTER,
        x => return Err(Diag::new(span, format!("unknown effect `{}` (sync, gc or reenter)", x))),
    })
}

/// The strongest effect in a mask.
pub fn effect_name(e: u8) -> &'static str {
    if e & EFF_REENTER != 0 {
        "reenter"
    } else if e & EFF_GC != 0 {
        "gc"
    } else {
        "sync"
    }
}

pub fn prim_type(name: &str) -> Option<Ty> {
    Some(match name {
        "i8" => Ty::Int(IntTy { signed: true, bits: 8 }),
        "i16" => Ty::Int(IntTy { signed: true, bits: 16 }),
        "i32" => Ty::Int(IntTy::I32),
        "i64" | "isize" => Ty::Int(IntTy::I64),
        "u8" => Ty::Int(IntTy::U8),
        "u16" => Ty::Int(IntTy { signed: false, bits: 16 }),
        "u32" => Ty::Int(IntTy::U32),
        "u64" | "usize" => Ty::Int(IntTy::U64),
        "f32" => Ty::F32,
        "f64" => Ty::F64,
        "bool" => Ty::Bool,
        "str" => Ty::str(),
        _ => return arbitrary_int(name).map(Ty::Int),
    })
}

/// `iN` / `uN` for any width 1..=64 (`i48`, `u61`, ...): LLVM integer
/// types of that width, used mainly as `value` payloads.
fn arbitrary_int(name: &str) -> Option<IntTy> {
    let signed = match name.as_bytes().first()? {
        b'i' => true,
        b'u' => false,
        _ => return None,
    };
    let digits = &name[1..];
    if digits.is_empty() || digits.starts_with('0') || !digits.bytes().all(|b| b.is_ascii_digit()) || digits.len() > 2 {
        return None;
    }
    let bits: u8 = digits.parse().ok()?;
    (1..=64).contains(&bits).then_some(IntTy { signed, bits })
}

pub fn is_place(e: &TExpr) -> bool {
    match &e.kind {
        TK::Local(_) | TK::Deref(_) | TK::Global(_) => true,
        TK::Field(b, _) => is_place(b) || matches!(b.ty, Ty::Slice(_)),
        TK::Index(b, _) => !matches!(b.ty, Ty::Array(..)) || is_place(b),
        TK::Vm(TVm::Word(_) | TVm::WordAt(..)) => true,
        _ => false,
    }
}

/// Whether some value can match both patterns (conservative: true when
/// unsure).
fn pats_overlap(a: &TPat, b: &TPat) -> bool {
    match (&a.kind, &b.kind) {
        (TPK::Wild | TPK::Bind(_), _) | (_, TPK::Wild | TPK::Bind(_)) => true,
        (TPK::Or(xs), _) => xs.iter().any(|x| pats_overlap(x, b)),
        (_, TPK::Or(ys)) => ys.iter().any(|y| pats_overlap(a, y)),
        (TPK::Tuple(xs), TPK::Tuple(ys)) => xs.len() != ys.len() || xs.iter().zip(ys).all(|(x, y)| pats_overlap(x, y)),
        (TPK::Variant(i, xs), TPK::Variant(j, ys)) => {
            i == j && (xs.len() != ys.len() || xs.iter().zip(ys).all(|(x, y)| pats_overlap(x, y)))
        }
        (TPK::Int(x), TPK::Int(y)) => x == y,
        (TPK::Int(x), TPK::Range(lo, hi)) | (TPK::Range(lo, hi), TPK::Int(x)) => lo <= x && x <= hi,
        (TPK::Range(a1, a2), TPK::Range(b1, b2)) => a1 <= b2 && b1 <= a2,
        (TPK::Bool(x), TPK::Bool(y)) => x == y,
        (TPK::Str(x), TPK::Str(y)) => x == y,
        _ => true,
    }
}

fn irrefutable(p: &TPat) -> bool {
    match &p.kind {
        TPK::Wild | TPK::Bind(_) => true,
        TPK::Tuple(ps) => ps.iter().all(irrefutable),
        _ => false,
    }
}

/// Whether all-zero bytes are a valid value of `t` that needs no
/// constructor, for `..zero`: numbers, `bool`, `?T` (None, so also
/// `?*T`, `?Gc<T>` and `?fn`), slices (empty), raw pointers `*T` (null,
/// as `0 as *T` makes without `unsafe`: a placeholder to assign before
/// use), and arrays and tuples of those. Not `Gc<T>` (null only through
/// `unsafe`, and the collector's and every reader's invariant is that it
/// refers to an object), functions (a call through null), structs
/// (their constructor may set invariants), enums or `value` types.
pub fn zero_ok(t: &Ty) -> bool {
    match t {
        Ty::Int(_) | Ty::F32 | Ty::F64 | Ty::Bool | Ty::Unit | Ty::Option(_) | Ty::Slice(_) | Ty::Ptr(_) => true,
        Ty::Array(e, _) => zero_ok(e),
        Ty::Tuple(ts) => ts.iter().all(zero_ok),
        _ => false,
    }
}

fn is_const(e: &TExpr) -> bool {
    match &e.kind {
        TK::Int(_) | TK::Float(_) | TK::Bool(_) | TK::Str(_) | TK::Unit | TK::Zeroed => true,
        TK::Struct(es) | TK::Tuple(es) | TK::Array(es) | TK::Variant(_, es) => es.iter().all(is_const),
        _ => false,
    }
}

/// Constant folding for integer constant expressions.
pub fn fold(e: &TExpr) -> TExpr {
    use BinOp::*;
    let k = match &e.kind {
        TK::Binary(op, a, b) => {
            let (a, b) = (fold(a), fold(b));
            match (&a.kind, &b.kind, &e.ty) {
                (TK::Int(x), TK::Int(y), Ty::Int(it)) => {
                    let (x, y) = (*x, *y);
                    let v = match op {
                        Add | AddWrap => Some(x + y),
                        Sub | SubWrap => Some(x - y),
                        Mul => x.checked_mul(y),
                        MulWrap => Some(x.wrapping_mul(y)),
                        Div if y != 0 => Some(x / y),
                        Rem if y != 0 => Some(x % y),
                        And => Some(x & y),
                        Or => Some(x | y),
                        Xor => Some(x ^ y),
                        // As at run time: the amount is read as an
                        // unsigned number of its own type and taken
                        // modulo the width of the shifted value.
                        Shl | Shr => match &b.ty {
                            Ty::Int(bt) => {
                                let n = wrap_int(y, IntTy { signed: false, bits: bt.bits }) % it.bits as i128;
                                let x = wrap_int(x, *it);
                                Some(if *op == Shl { x << n } else { x >> n })
                            }
                            _ => None,
                        },
                        _ => None,
                    };
                    // An operation that overflows (traps at run time) is
                    // left unfolded; `const_overflow` reports it.
                    let checked = matches!(op, Add | Sub | Mul | Div);
                    match v {
                        Some(v) if checked && (v < it.min() || v > it.max()) => {
                            TK::Binary(*op, Box::new(a), Box::new(b))
                        }
                        Some(v) => TK::Int(wrap_int(v, *it)),
                        None => TK::Binary(*op, Box::new(a), Box::new(b)),
                    }
                }
                _ => TK::Binary(*op, Box::new(a), Box::new(b)),
            }
        }
        TK::Unary(UnOp::Neg, a) => match (fold(a), &e.ty) {
            (TExpr { kind: TK::Int(x), .. }, Ty::Int(it)) if -x < it.min() || -x > it.max() => {
                TK::Unary(UnOp::Neg, Box::new(mk(Ty::Int(*it), TK::Int(x), a.span)))
            }
            (TExpr { kind: TK::Int(x), .. }, _) => TK::Int(-x),
            (TExpr { kind: TK::Float(f), .. }, _) => TK::Float(-f),
            _ => e.kind.clone(),
        },
        TK::Unary(UnOp::Not, a) => match (fold(a).kind, &e.ty) {
            (TK::Int(x), Ty::Int(it)) => TK::Int(wrap_int(!x, *it)),
            (TK::Bool(b), _) => TK::Bool(!b),
            _ => e.kind.clone(),
        },
        TK::Cast(a) => match (fold(a).kind, &e.ty) {
            (TK::Int(x), Ty::Int(it)) => TK::Int(wrap_int(x, *it)),
            (TK::Int(x), Ty::F64 | Ty::F32) => TK::Float(x as f64),
            _ => e.kind.clone(),
        },
        _ => e.kind.clone(),
    };
    TExpr { ty: e.ty.clone(), kind: k, span: e.span }
}

/// After `fold`: an integer operation on constants that was left unfolded
/// because it overflows its type or divides by zero, as an error.
pub fn const_overflow(e: &TExpr) -> Option<Diag> {
    let int = |x: &TExpr| matches!(x.kind, TK::Int(_));
    let over = |e: &TExpr| match &e.ty {
        Ty::Int(it) => format!("constant expression overflows `{}{}`", if it.signed { "i" } else { "u" }, it.bits),
        _ => unreachable!(),
    };
    match &e.kind {
        TK::Binary(op, a, b) if int(a) && int(b) && e.ty.is_int() => {
            let what = if matches!(op, BinOp::Div | BinOp::Rem) && matches!(b.kind, TK::Int(0)) {
                "division by zero in a constant expression".to_string()
            } else {
                over(e)
            };
            Some(Diag::new(e.span, what))
        }
        TK::Unary(UnOp::Neg, a) if int(a) && e.ty.is_int() => Some(Diag::new(e.span, over(e))),
        TK::Binary(_, a, b) => const_overflow(a).or_else(|| const_overflow(b)),
        TK::Unary(_, a) | TK::Cast(a) => const_overflow(a),
        TK::Struct(es) | TK::Tuple(es) | TK::Array(es) | TK::Variant(_, es) => es.iter().find_map(const_overflow),
        TK::ArrayRepeat(v, _) => const_overflow(v),
        _ => None,
    }
}

/// `fold` applied to every element of aggregate literals.
pub fn fold_deep(e: &TExpr) -> TExpr {
    let e = fold(e);
    let all = |es: &[TExpr]| es.iter().map(fold_deep).collect::<Vec<_>>();
    let kind = match &e.kind {
        TK::Struct(es) => TK::Struct(all(es)),
        TK::Tuple(es) => TK::Tuple(all(es)),
        TK::Array(es) => TK::Array(all(es)),
        TK::Variant(i, es) => TK::Variant(*i, all(es)),
        TK::ArrayRepeat(v, n) => TK::ArrayRepeat(Box::new(fold_deep(v)), *n),
        k => k.clone(),
    };
    TExpr { ty: e.ty, kind, span: e.span }
}

pub fn wrap_int(v: i128, it: IntTy) -> i128 {
    let bits = it.bits as u32;
    let mask: i128 = if bits >= 128 { -1 } else { (1i128 << bits) - 1 };
    let u = v & mask;
    if it.signed && u > it.max() { u - (1i128 << bits) } else { u }
}
