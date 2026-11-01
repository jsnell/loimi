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
    Module(usize),
    Vm(u32),
}

pub struct ConstInfo {
    pub module: usize,
    pub decl: ConstDecl,
    pub value: Option<TExpr>,
    pub busy: bool,
}

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
}

impl FnCx {
    pub fn new(module: usize, ret: Ty) -> FnCx {
        FnCx { module, locals: vec![], scopes: vec![vec![]], ret, loops: vec![], next_loop: 0, vm: None }
    }
    pub fn declare(&mut self, name: &str, ty: Ty, mutable: bool) -> LocalId {
        let id = self.locals.len() as LocalId;
        self.locals.push(LocalDef { name: name.to_string(), ty, mutable });
        self.scopes.last_mut().unwrap().push((name.to_string(), id));
        id
    }
    /// A local not visible by name.
    pub fn temp(&mut self, ty: Ty) -> LocalId {
        let id = self.locals.len() as LocalId;
        self.locals.push(LocalDef { name: format!("tmp{}", id), ty, mutable: true });
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
    pub scopes: Vec<HashMap<String, (Def, bool)>>,
    pub methods: HashMap<(Ty, String), FnId>,
    pub consts: Vec<ConstInfo>,
    pub pending: Vec<(FnId, usize, FnDecl)>,
    pub vm_decls: Vec<(usize, crate::vmast::VmDecl)>,
    pub dispatch_override: Option<Dispatch>,
}

pub fn mk(ty: Ty, kind: TK, span: Span) -> TExpr {
    TExpr { ty, kind, span }
}

fn is_literal_expr(e: &Expr) -> bool {
    match &e.kind {
        ExprKind::Int(_) | ExprKind::Float(_) | ExprKind::Char(_) => true,
        ExprKind::Unary(UnOp::Neg, inner) => is_literal_expr(inner),
        _ => false,
    }
}

impl<'a> Checker<'a> {
    pub fn new(modules: &'a [Module]) -> Checker<'a> {
        Checker {
            prog: Program::default(),
            modules,
            scopes: vec![HashMap::new(); modules.len()],
            methods: HashMap::new(),
            consts: vec![],
            pending: vec![],
            vm_decls: vec![],
            dispatch_override: None,
        }
    }

    pub fn module_prefix(&self, m: usize) -> String {
        self.modules[m].path.join(".")
    }

    // ── declaration collection ──────────────────────────────────────

    fn define(&mut self, m: usize, name: &Ident, def: Def, is_pub: bool) -> Res<()> {
        if self.scopes[m].contains_key(&name.name) {
            return Err(Diag::new(name.span, format!("`{}` is defined more than once", name.name)));
        }
        self.scopes[m].insert(name.name.clone(), (def, is_pub));
        Ok(())
    }

    pub fn run(&mut self) -> Res<()> {
        // Pass 1: register names.
        let mut fn_decls: Vec<(FnId, usize, FnDecl)> = Vec::new();
        for (m, module) in self.modules.iter().enumerate() {
            let prefix = self.module_prefix(m);
            for item in &module.items {
                match item {
                    Item::Struct(s) => {
                        let id = self.prog.structs.len() as u32;
                        self.prog.structs.push(StructDef { name: format!("{}.{}", prefix, s.name.name), fields: vec![] });
                        self.define(m, &s.name, Def::Struct(id), s.is_pub)?;
                    }
                    Item::Enum(e) => {
                        let id = self.prog.enums.len() as u32;
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
                        });
                        self.define(m, &v.name, Def::Value(id), v.is_pub)?;
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
                    Item::Vm(v) => {
                        let id = self.vm_decls.len() as u32;
                        self.vm_decls.push((m, (**v).clone()));
                        self.define(m, &v.name, Def::Vm(id), v.is_pub)?;
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
                                let (def, is_pub) = self.scopes[target].get(&n.name).cloned().ok_or_else(|| {
                                    Diag::new(n.span, format!("`{}` not found in module `{}`", n.name, path.join(".")))
                                })?;
                                if !is_pub {
                                    return Err(Diag::new(n.span, format!("`{}` is private", n.name)));
                                }
                                self.define(m, n, def, false)?;
                            }
                        }
                    }
                }
            }
        }
        // Pass 3: type definitions.
        for (m, module) in self.modules.iter().enumerate() {
            for item in &module.items {
                match item {
                    Item::Struct(s) => {
                        let Some((Def::Struct(id), _)) = self.scopes[m].get(&s.name.name).cloned() else { unreachable!() };
                        let mut fields = Vec::new();
                        for (f, t) in &s.fields {
                            if fields.iter().any(|(n, _): &(String, Ty)| *n == f.name) {
                                return Err(Diag::new(f.span, format!("duplicate field `{}`", f.name)));
                            }
                            fields.push((f.name.clone(), self.resolve_type(m, t)?));
                        }
                        self.prog.structs[id as usize].fields = fields;
                    }
                    Item::Enum(e) => {
                        let Some((Def::Enum(id), _)) = self.scopes[m].get(&e.name.name).cloned() else { unreachable!() };
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
        // Pass 4: function signatures.
        for (id, m, f) in &fn_decls {
            self.declare_fn(*id, *m, f, None)?;
        }
        // Pass 5: vm declarations (generates handler functions).
        for i in 0..self.vm_decls.len() {
            crate::vmcheck::check_vm(self, i)?;
        }
        // Pass 6: bodies.
        for (id, m, f) in fn_decls {
            self.pending.push((id, m, f));
        }
        while let Some((id, m, f)) = self.pending.pop() {
            self.check_fn_body(id, m, &f, None)?;
        }
        Ok(())
    }

    pub fn declare_fn(&mut self, id: FnId, m: usize, f: &FnDecl, symbol: Option<String>) -> Res<()> {
        let mut param_tys = Vec::new();
        for p in &f.params {
            param_tys.push(self.resolve_type(m, &p.ty)?);
        }
        let ret = match &f.ret {
            Some(t) => self.resolve_type(m, t)?,
            None => Ty::Unit,
        };
        let prefix = self.module_prefix(m);
        let mut name = f.name.name.clone();
        if let Some(recv) = &f.recv {
            let rty = self.resolve_type_name(m, recv)?;
            name = format!("{}.{}", recv.name, f.name.name);
            let key = (rty, f.name.name.clone());
            if self.methods.contains_key(&key) {
                return Err(Diag::new(f.name.span, format!("method `{}` is defined more than once", name)));
            }
            self.methods.insert(key, id);
        }
        let symbol = symbol.unwrap_or_else(|| match f.linkage {
            Linkage::Internal => format!("{}.{}", prefix, name),
            _ => f.name.name.clone(),
        });
        if f.linkage == Linkage::Extern && f.body.is_some() {
            return Err(Diag::new(f.name.span, "extern functions cannot have a body"));
        }
        if f.linkage != Linkage::Extern && f.body.is_none() {
            return Err(Diag::new(f.name.span, "function needs a body"));
        }
        let inline = if f.attrs.has("inline") {
            match f.attrs.get("inline").unwrap().first().map(|s| s.as_str()) {
                Some("never") => Some(false),
                _ => Some(true),
            }
        } else {
            None
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
        };
        Ok(())
    }

    pub fn check_fn_body(&mut self, id: FnId, m: usize, f: &FnDecl, vm: Option<crate::vmcheck::HandlerCx>) -> Res<()> {
        let Some(body) = &f.body else { return Ok(()) };
        let def = self.prog.fns[id as usize].clone();
        let mut cx = FnCx::new(m, def.ret.clone());
        cx.vm = vm;
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
            Ty::Int(it) => {
                if it.bits == 64 && false {
                    String::new()
                } else {
                    format!("{}{}", if it.signed { "i" } else { "u" }, it.bits)
                }
            }
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
            Ty::Fn(ps, r) => format!(
                "fn({}) -> {}",
                ps.iter().map(|t| self.ty_name(t)).collect::<Vec<_>>().join(", "),
                self.ty_name(r)
            ),
            Ty::Struct(i) => short(&self.prog.structs[*i as usize].name),
            Ty::Enum(i) => short(&self.prog.enums[*i as usize].name),
            Ty::Value(i) => short(&self.prog.values[*i as usize].name),
        }
    }

    pub fn resolve_type_name(&self, m: usize, id: &Ident) -> Res<Ty> {
        if let Some(t) = prim_type(&id.name) {
            return Ok(t);
        }
        match self.scopes[m].get(&id.name) {
            Some((Def::Struct(i), _)) => Ok(Ty::Struct(*i)),
            Some((Def::Enum(i), _)) => Ok(Ty::Enum(*i)),
            Some((Def::Value(i), _)) => Ok(Ty::Value(*i)),
            _ => Err(Diag::new(id.span, format!("unknown type `{}`", id.name))),
        }
    }

    pub fn resolve_type(&mut self, m: usize, t: &TypeExpr) -> Res<Ty> {
        Ok(match &t.kind {
            TypeKind::Named(path, args) => {
                if path.len() == 1 && path[0].name == "Result" {
                    if args.len() != 2 {
                        return Err(Diag::new(t.span, "Result takes two type arguments"));
                    }
                    let a = self.resolve_type(m, &args[0])?;
                    let b = self.resolve_type(m, &args[1])?;
                    return Ok(Ty::Result(Box::new(a), Box::new(b)));
                }
                if !args.is_empty() {
                    return Err(Diag::new(t.span, "generic types are not supported yet"));
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
            TypeKind::Fn(ps, r) => {
                let mut out = Vec::new();
                for p in ps {
                    out.push(self.resolve_type(m, p)?);
                }
                Ty::Fn(out, Box::new(self.resolve_type(m, r)?))
            }
            TypeKind::Never => Ty::Never,
        })
    }

    pub fn const_int(&mut self, m: usize, e: &Expr) -> Res<i128> {
        let mut cx = FnCx::new(m, Ty::Unit);
        let te = self.expr(&mut cx, e, Some(&Ty::i64()))?;
        match fold(&te).kind {
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
        let v = fold(&self.coerce(v, &ty)?);
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
        if let Ty::Option(inner) = want {
            if **inner == e.ty {
                let span = e.span;
                return Ok(mk(want.clone(), TK::Variant(1, vec![e]), span));
            }
        }
        Err(Diag::new(
            e.span,
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

    pub fn pat(&mut self, cx: &mut FnCx, p: &Pat, ty: &Ty, mutable: bool) -> Res<TPat> {
        let kind = match &p.kind {
            PatKind::Wild => TPK::Wild,
            PatKind::Ident(name, pm) => {
                if let Some(idx) = self.variant_index(ty, name) {
                    if self.variant_fields(ty, idx).is_empty() {
                        return Ok(TPat { ty: ty.clone(), kind: TPK::Variant(idx, vec![]) });
                    }
                }
                TPK::Bind(cx.declare(name, ty.clone(), mutable || *pm))
            }
            PatKind::Int(_) | PatKind::Range(..) | PatKind::Char(..) if !ty.is_int() => {
                return Err(Diag::new(p.span, format!("integer pattern for a value of type `{}`", self.ty_name(ty))));
            }
            PatKind::Int(v) => TPK::Int(*v),
            PatKind::Char(c) => TPK::Int(*c as i128),
            PatKind::Range(a, b) => TPK::Range(*a, *b),
            PatKind::Bool(b) => {
                if *ty != Ty::Bool {
                    return Err(Diag::new(p.span, "boolean pattern for a non-bool value"));
                }
                TPK::Bool(*b)
            }
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
                    let qty = self.resolve_type_name(cx.module, qual)?;
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

    fn check_exhaustive(&self, ty: &Ty, arms: &[TArm], span: Span) -> Res<()> {
        fn catch_all(p: &TPat) -> bool {
            match &p.kind {
                TPK::Wild | TPK::Bind(_) => true,
                TPK::Tuple(ps) => ps.iter().all(catch_all),
                TPK::Or(ps) => ps.iter().any(catch_all),
                _ => false,
            }
        }
        fn covered(p: &TPat, out: &mut Vec<u32>) {
            match &p.kind {
                TPK::Variant(i, ps) if ps.iter().all(catch_all) => out.push(*i),
                TPK::Bool(b) => out.push(*b as u32),
                TPK::Or(ps) => ps.iter().for_each(|p| covered(p, out)),
                _ => {}
            }
        }
        let unguarded: Vec<&TArm> = arms.iter().filter(|a| a.guard.is_none()).collect();
        if unguarded.iter().any(|a| catch_all(&a.pat)) {
            return Ok(());
        }
        let n = self.variant_count(ty);
        if n > 0 {
            let mut cov = Vec::new();
            for a in &unguarded {
                covered(&a.pat, &mut cov);
            }
            let missing: Vec<u32> = (0..n as u32).filter(|i| !cov.contains(i)).collect();
            if missing.is_empty() {
                return Ok(());
            }
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
            return Err(Diag::new(span, format!("non-exhaustive match: missing {}", names.join(", "))));
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
                if !self.castable(&v.ty, &to) {
                    return Err(Diag::new(
                        span,
                        format!("cannot cast `{}` to `{}`", self.ty_name(&v.ty), self.ty_name(&to)),
                    ));
                }
                Ok(mk(to, TK::Cast(Box::new(v)), span))
            }
            ExprKind::StructLit(path, fields) => {
                let ty = if path.len() == 1 {
                    self.resolve_type_name(cx.module, &path[0])?
                } else {
                    return Err(Diag::new(span, "qualified struct literals are not supported yet"));
                };
                let Ty::Struct(sid) = ty else {
                    return Err(Diag::new(span, format!("`{}` is not a struct", path[0].name)));
                };
                let defs = self.prog.structs[sid as usize].fields.clone();
                let mut vals: Vec<Option<TExpr>> = vec![None; defs.len()];
                for (f, v) in fields {
                    let Some(idx) = defs.iter().position(|(n, _)| *n == f.name) else {
                        return Err(Diag::new(f.span, format!("no field `{}` in struct `{}`", f.name, path[0].name)));
                    };
                    if vals[idx].is_some() {
                        return Err(Diag::new(f.span, format!("field `{}` given twice", f.name)));
                    }
                    vals[idx] = Some(self.expr_coerce(cx, v, &defs[idx].1)?);
                }
                let mut out = Vec::new();
                for (i, v) in vals.into_iter().enumerate() {
                    match v {
                        Some(v) => out.push(v),
                        None => return Err(Diag::new(span, format!("missing field `{}`", defs[i].0))),
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
                let Some(elem) = elem else {
                    return Err(Diag::new(span, "cannot infer the type of an empty array"));
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
            ExprKind::Block(b) | ExprKind::Unsafe(b) => self.block(cx, b, exp),
            ExprKind::If(conds, then, els) => {
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
            ExprKind::Match(scrut, arms) => {
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
                let ty = ty.unwrap_or(Ty::Never);
                for a in tarms.iter_mut() {
                    let body = std::mem::replace(&mut a.body, mk(Ty::Unit, TK::Unit, span));
                    a.body = self.coerce(body, &ty)?;
                }
                Ok(mk(ty, TK::Match(Box::new(s), tarms), span))
            }
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
            ExprKind::For(var, lo, hi, body) => {
                let (lo_t, hi_t) = if is_literal_expr(lo) && !is_literal_expr(hi) {
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
                if cx.vm.as_ref().is_some_and(|h| h.is_handler) {
                    return Err(Diag::new(span, "`return` is not allowed in a handler; use `next`, `jump` or `exit`"));
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
                let bv = self.expr_coerce(cx, b, &inner)?;
                Ok(mk(*inner, TK::Coalesce(Box::new(av), Box::new(bv)), span))
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
            ExprKind::SizeOf(t) => {
                let _ = self.resolve_type(cx.module, t)?;
                let ty = self.resolve_type(cx.module, t)?;
                let size = crate::layout::size_of(&self.prog, &ty);
                Ok(mk(Ty::Int(IntTy::U64), TK::Int(size as i128), span))
            }
            ExprKind::Vm(v) => crate::vmcheck::check_vm_expr(self, cx, v, exp, span),
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
            return Ok(mk(cx.locals[id as usize].ty.clone(), TK::Local(id), span));
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
                _ => Err(Diag::new(span, "cannot infer the type of `None`")),
            };
        }
        if let Some((def, _)) = self.scopes[cx.module].get(name).cloned() {
            return self.def_value(def, name, span);
        }
        if let Some(t) = exp {
            if let Some(idx) = self.variant_index(t, name) {
                return self.make_variant(cx, t, idx, &[], span);
            }
        }
        Err(Diag::new(span, format!("unknown name `{}`", name)))
    }

    fn def_value(&mut self, def: Def, name: &str, span: Span) -> Res<TExpr> {
        match def {
            Def::Fn(id) => {
                let f = &self.prog.fns[id as usize];
                let ty = Ty::Fn(f.param_tys.clone(), Box::new(f.ret.clone()));
                Ok(mk(ty, TK::FnRef(id), span))
            }
            Def::Const(id) => {
                let mut v = self.const_value(id, span)?;
                v.span = span;
                Ok(v)
            }
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
                match self.scopes[cx.module].get(n) {
                    Some((Def::Struct(i), _)) => Some(Ty::Struct(*i)),
                    Some((Def::Enum(i), _)) => Some(Ty::Enum(*i)),
                    Some((Def::Value(i), _)) => Some(Ty::Value(*i)),
                    _ => None,
                }
            }
            ExprKind::Field(base, name) => {
                let m = self.module_path(cx, base)?;
                match self.scopes[m].get(&name.name) {
                    Some((Def::Struct(i), true)) => Some(Ty::Struct(*i)),
                    Some((Def::Enum(i), true)) => Some(Ty::Enum(*i)),
                    Some((Def::Value(i), true)) => Some(Ty::Value(*i)),
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

    fn field(&mut self, cx: &mut FnCx, base: &Expr, name: &Ident, exp: Option<&Ty>, span: Span) -> Res<TExpr> {
        if let Some(m) = self.module_path(cx, base) {
            let Some((def, is_pub)) = self.scopes[m].get(&name.name).cloned() else {
                return Err(Diag::new(name.span, format!("`{}` not found in module", name.name)));
            };
            if !is_pub {
                return Err(Diag::new(name.span, format!("`{}` is private", name.name)));
            }
            return self.def_value(def, &name.name, span);
        }
        if let Some(t) = self.type_path(cx, base) {
            if let Some(idx) = self.variant_index(&t, &name.name) {
                return self.make_variant(cx, &t, idx, &[], span);
            }
            if let Some(f) = self.methods.get(&(t.clone(), name.name.clone())).copied() {
                return self.def_value(Def::Fn(f), &name.name, span);
            }
            return Err(Diag::new(name.span, format!("no variant or function `{}` in `{}`", name.name, self.ty_name(&t))));
        }
        let _ = exp;
        let b = self.expr(cx, base, None)?;
        self.field_of(b, name, span)
    }

    pub fn field_of(&mut self, b: TExpr, name: &Ident, span: Span) -> Res<TExpr> {
        let b = self.auto_deref(b);
        match b.ty.clone() {
            Ty::Struct(sid) => {
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
        while let Ty::Ptr(inner) = &b.ty {
            if matches!(**inner, Ty::Struct(_) | Ty::Tuple(_) | Ty::Slice(_) | Ty::Ptr(_) | Ty::Array(..)) {
                let t = (**inner).clone();
                let span = b.span;
                b = mk(t, TK::Deref(Box::new(b)), span);
            } else {
                break;
            }
        }
        b
    }

    fn auto_deref_for_index(&self, mut b: TExpr) -> TExpr {
        while let Ty::Ptr(inner) = &b.ty {
            if matches!(**inner, Ty::Slice(_) | Ty::Array(..)) {
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

    fn call(&mut self, cx: &mut FnCx, callee: &Expr, args: &[Expr], exp: Option<&Ty>, span: Span) -> Res<TExpr> {
        match &callee.kind {
            ExprKind::Ident(name) if cx.lookup(name).is_none() => {
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
                        return Err(Diag::new(span, format!("cannot infer the type of `{}`", name)));
                    };
                    let idx = if name == "Ok" { 0 } else { 1 };
                    return self.make_variant(cx, &t.clone(), idx, args, span);
                }
                if self.scopes[cx.module].get(name).is_none() && crate::vmcheck::vm_fn(self, cx, name).is_none() {
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
                if self.module_path(cx, base).is_none() {
                    if let Some(t) = self.type_path(cx, base) {
                        if let Some(idx) = self.variant_index(&t, &name.name) {
                            return self.make_variant(cx, &t, idx, args, span);
                        }
                        if let Some(r) = self.static_intrinsic(cx, &t, &name.name, args, span)? {
                            return Ok(r);
                        }
                        let Some(f) = self.methods.get(&(t.clone(), name.name.clone())).copied() else {
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
                    return self.method_call(cx, recv, name, args, span);
                }
            }
            _ => {}
        }
        let f = self.expr(cx, callee, None)?;
        self.finish_call(cx, f, vec![], args, span)
    }

    fn finish_call(&mut self, cx: &mut FnCx, f: TExpr, mut pre: Vec<TExpr>, args: &[Expr], span: Span) -> Res<TExpr> {
        let Ty::Fn(params, ret) = f.ty.clone() else {
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
                pre.push(self.expr_coerce(cx, a, &pt)?);
            } else {
                let mut v = self.expr(cx, a, None)?;
                if v.ty == Ty::F32 {
                    v = mk(Ty::F64, TK::Cast(Box::new(v)), a.span);
                }
                pre.push(v);
            }
        }
        if let TK::FnRef(id) = f.kind {
            crate::vmcheck::check_call_effects(self, cx, id, span)?;
        }
        Ok(mk(*ret, TK::Call(Box::new(f), pre), span))
    }

    fn method_call(&mut self, cx: &mut FnCx, recv: TExpr, name: &Ident, args: &[Expr], span: Span) -> Res<TExpr> {
        let mut base_ty = recv.ty.clone();
        while let Ty::Ptr(inner) = base_ty {
            base_ty = *inner;
        }
        if let Some(fid) = self.methods.get(&(base_ty.clone(), name.name.clone())).copied() {
            let fdef = &self.prog.fns[fid as usize];
            let Some(p0) = fdef.param_tys.first().cloned() else {
                return Err(Diag::new(name.span, format!("`{}` takes no receiver", name.name)));
            };
            let fty = Ty::Fn(fdef.param_tys.clone(), Box::new(fdef.ret.clone()));
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
        // A field holding a function pointer: `(n.f)(x)` style written as `n.f(x)`.
        if let Ok(fv) = self.field_of(recv.clone(), name, name.span) {
            if matches!(fv.ty, Ty::Fn(..)) {
                return self.finish_call(cx, fv, vec![], args, span);
            }
        }
        Err(Diag::new(name.span, format!("no method `{}` on type `{}`", name.name, self.ty_name(&recv.ty))))
    }

    fn intrinsic_method(&mut self, cx: &mut FnCx, recv: TExpr, name: &str, args: &[Expr], span: Span) -> Res<Option<TExpr>> {
        let recv = {
            let mut r = recv;
            while let Ty::Ptr(inner) = &r.ty {
                if matches!(**inner, Ty::Option(_) | Ty::Int(_) | Ty::F64) && name != "offset" {
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
            (Ty::F64, "to_bits") => {
                argc(0)?;
                mk(Ty::Int(IntTy::U64), TK::Intrinsic(Intr::FloatBits, vec![recv]), span)
            }
            (Ty::F64, "sqrt" | "floor") => {
                argc(0)?;
                let intr = if name == "sqrt" { Intr::Sqrt } else { Intr::Floor };
                mk(Ty::F64, TK::Intrinsic(intr, vec![recv]), span)
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
            (Ty::F64, "from_bits") => {
                if args.len() != 1 {
                    return Err(Diag::new(span, "`from_bits` takes one argument"));
                }
                let a = self.expr_coerce(cx, &args[0], &Ty::Int(IntTy::U64))?;
                Ok(Some(mk(Ty::F64, TK::Intrinsic(Intr::BitsToFloat, vec![a]), span)))
            }
            _ => Ok(None),
        }
    }

    fn builtin_call(&mut self, cx: &mut FnCx, name: &str, args: &[Expr], _exp: Option<&Ty>, span: Span) -> Res<Option<TExpr>> {
        if self.scopes[cx.module].contains_key(name) {
            return Ok(None);
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
        let operand_exp = if op.is_cmp() { None } else { exp };
        let (a, b) = if matches!(op, Shl | Shr) {
            let a = self.expr(cx, l, operand_exp)?;
            let b = self.expr_coerce(cx, r, &a.ty.clone())?;
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
                    || matches!(t, Ty::Bool | Ty::Ptr(_) | Ty::Fn(..) | Ty::Value(_))
                    || matches!(&t, Ty::Enum(e) if !self.prog.enums[*e as usize].has_payload())
                    || matches!(&t, Ty::Option(inner) if inner.is_nullable_ptr())
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
            (Ty::Ptr(_) | Ty::Fn(..), Ty::Ptr(_)) => true,
            (Ty::Ptr(_), Ty::Int(it)) | (Ty::Int(it), Ty::Ptr(_)) => it.bits == 64,
            (Ty::Enum(e), Ty::Int(_)) => !self.prog.enums[*e as usize].has_payload(),
            (Ty::Value(_), Ty::Int(it)) | (Ty::Int(it), Ty::Value(_)) => it.bits == 64,
            _ => false,
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
            TK::Deref(_) => Ok(()),
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
            TK::Vm(TVm::Word(_)) => Ok(()),
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

fn placeholder_fn(span: Span) -> FnDef {
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
    }
}

fn short(s: &str) -> String {
    s.rsplit('.').next().unwrap_or(s).to_string()
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
        _ => return None,
    })
}

pub fn is_place(e: &TExpr) -> bool {
    match &e.kind {
        TK::Local(_) | TK::Deref(_) => true,
        TK::Field(b, _) => is_place(b) || matches!(b.ty, Ty::Slice(_)),
        TK::Index(b, _) => !matches!(b.ty, Ty::Array(..)) || is_place(b),
        TK::Vm(TVm::Word(_)) => true,
        _ => false,
    }
}

fn irrefutable(p: &TPat) -> bool {
    match &p.kind {
        TPK::Wild | TPK::Bind(_) => true,
        TPK::Tuple(ps) => ps.iter().all(irrefutable),
        _ => false,
    }
}

fn is_const(e: &TExpr) -> bool {
    match &e.kind {
        TK::Int(_) | TK::Float(_) | TK::Bool(_) | TK::Str(_) | TK::Unit => true,
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
                        Mul | MulWrap => Some(x.wrapping_mul(y)),
                        Div if y != 0 => Some(x / y),
                        Rem if y != 0 => Some(x % y),
                        And => Some(x & y),
                        Or => Some(x | y),
                        Xor => Some(x ^ y),
                        Shl if (0..128).contains(&y) => Some(x << y),
                        Shr if (0..128).contains(&y) => Some(x >> y),
                        _ => None,
                    };
                    match v {
                        Some(v) => TK::Int(wrap_int(v, *it)),
                        None => TK::Binary(*op, Box::new(a), Box::new(b)),
                    }
                }
                _ => TK::Binary(*op, Box::new(a), Box::new(b)),
            }
        }
        TK::Unary(UnOp::Neg, a) => match fold(a).kind {
            TK::Int(x) => TK::Int(-x),
            TK::Float(f) => TK::Float(-f),
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

pub fn wrap_int(v: i128, it: IntTy) -> i128 {
    let bits = it.bits as u32;
    let mask: i128 = if bits >= 128 { -1 } else { (1i128 << bits) - 1 };
    let u = v & mask;
    if it.signed && u > it.max() { u - (1i128 << bits) } else { u }
}
