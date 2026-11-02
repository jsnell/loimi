//! Checking of `vm` declarations: handlers, operands, control words, vm
//! functions, entries and the error path.

use std::collections::HashMap;

use crate::ast::*;
use crate::check::{effect_name, mk, Checker, Def, FnCx};
use crate::diag::{Diag, Res, Span};
use crate::tir::*;
use crate::vmast::*;

#[allow(dead_code)]
pub struct OperandKind {
    pub name: String,
    pub body: Expr,
    pub verify: Option<Expr>,
}

#[allow(dead_code)]
pub struct OpInfo {
    pub name: String,
    pub opcode: u32,
    pub format: String,
    pub cache: Option<usize>,
    pub params: Vec<String>,
    pub kinds: Vec<usize>,
    /// (bits, shift, signed) per param.
    pub fields: Vec<(u32, u32, bool)>,
    pub parts: Vec<usize>,
    pub size: u32,
    pub fid: FnId,
    pub body: Option<Block>,
    pub span: Span,
}

#[allow(dead_code)]
pub struct VmInfo {
    pub module: usize,
    pub name: String,
    pub word: IntTy,
    pub state: Vec<(String, Ty)>,
    pub pc_idx: usize,
    pub kinds: Vec<OperandKind>,
    pub caches: Vec<(String, Vec<String>)>,
    pub ops: Vec<OpInfo>,
    pub ret: Ty,
    pub raise_ty: Option<Ty>,
    pub fns: HashMap<String, FnId>,
    /// `-> never` vm functions, which may end in control words.
    pub never_fns: Vec<NeverFn>,
    pub sync: Option<Block>,
}

/// A vm function returning `never`. Its body is checked again at each
/// call site that inlines it; in tail-call mode, cold call sites instead
/// `musttail` to one shared copy (`shared`), a function with the handler
/// signature that receives its arguments through thread-local slots.
pub struct NeverFn {
    pub decl: FnDecl,
    pub params: Vec<Ty>,
    pub shared: Option<FnId>,
    /// `#[inline]`: always inlined; `#[inline(never)]`: always shared.
    pub inline: Option<bool>,
    /// The shared copy uses `next` (directly or through shared callees).
    pub uses_next: bool,
    /// Shared callees of the shared copy.
    pub calls: Vec<usize>,
    /// Shared calls from entries and `on raise`, which have no `next`.
    pub no_op_calls: Vec<Span>,
}

#[derive(Clone, Debug)]
pub struct HandlerCx {
    pub vm: u32,
    /// An op handler or `on raise` (no `return`).
    pub is_handler: bool,
    /// Control words (`jump`, `exit`, ...) allowed.
    pub allow_control: bool,
    pub op: Option<usize>,
    /// In a non-final superinstruction part: (next part, size of this part).
    pub part: Option<(u32, u32)>,
    pub operands: Vec<(String, usize, LocalId)>,
    pub state: Vec<LocalId>,
    /// vm functions see the state through pointers.
    pub by_ptr: bool,
    /// In the body of a `-> never` vm function: no operands, `become` or `cache`.
    pub in_fn: bool,
    /// Inside a `cold` block.
    pub cold: bool,
    /// The shared copy of `-> never` vm function `i`: `next` skips the
    /// number of words in local `size`.
    pub shared: Option<(usize, LocalId)>,
    /// `-> never` vm functions being inlined here (to catch recursion).
    pub inlining: Vec<usize>,
}

impl HandlerCx {
    fn new(vm: u32, state: Vec<LocalId>) -> HandlerCx {
        HandlerCx {
            vm,
            is_handler: true,
            allow_control: true,
            op: None,
            part: None,
            operands: vec![],
            state,
            by_ptr: false,
            in_fn: false,
            cold: false,
            shared: None,
            inlining: vec![],
        }
    }
}

pub fn check_value_decl(c: &mut Checker, m: usize, v: &ValueDecl) -> Res<ValueDef> {
    crate::values::check_value_decl(c, m, v)
}

fn raw_ty(bits: u32, signed: bool) -> Ty {
    let b = if bits <= 8 {
        8
    } else if bits <= 16 {
        16
    } else if bits <= 32 {
        32
    } else {
        64
    };
    Ty::Int(IntTy { signed, bits: b })
}

/// Decode a field of the current code word.
fn decode(word: IntTy, bits: u32, shift: u32, signed: bool, span: Span) -> TExpr {
    let wt = Ty::Int(word);
    let w = mk(wt.clone(), TK::Vm(TVm::Word(0)), span);
    let int = |v: u64| mk(wt.clone(), TK::Int(v as i128), span);
    let wb = word.bits as u32;
    let rt = raw_ty(bits, signed);
    if signed {
        let st = Ty::Int(IntTy { signed: true, bits: word.bits });
        let up = mk(wt.clone(), TK::Binary(BinOp::Shl, Box::new(w), Box::new(int((wb - shift - bits) as u64))), span);
        let as_s = mk(st.clone(), TK::Cast(Box::new(up)), span);
        let sint = mk(st.clone(), TK::Int((wb - bits) as i128), span);
        let down = mk(st.clone(), TK::Binary(BinOp::Shr, Box::new(as_s), Box::new(sint)), span);
        if rt == st { down } else { mk(rt, TK::Cast(Box::new(down)), span) }
    } else {
        let mask = if bits >= 64 { u64::MAX } else { (1u64 << bits) - 1 };
        let sh = if shift == 0 { w } else { mk(wt.clone(), TK::Binary(BinOp::Shr, Box::new(w), Box::new(int(shift as u64))), span) };
        let m = if shift + bits >= wb { sh } else { mk(wt.clone(), TK::Binary(BinOp::And, Box::new(sh), Box::new(int(mask))), span) };
        if rt == wt { m } else { mk(rt, TK::Cast(Box::new(m)), span) }
    }
}

fn new_fn(c: &mut Checker, symbol: String, name: String, param_tys: Vec<Ty>, ret: Ty, cc: CallConv, vm: u32, role: VmRole, span: Span) -> FnId {
    let id = c.prog.fns.len() as FnId;
    c.prog.fns.push(FnDef {
        name,
        symbol,
        params: vec![],
        param_tys,
        ret,
        locals: vec![],
        body: None,
        linkage: Linkage::Internal,
        variadic: false,
        inline: if role == VmRole::VmFn { Some(true) } else { None },
        cold: false,
        cc,
        span,
        vm: Some(vm),
        vm_role: role,
        vm_state: vec![],
        effects: 0,
    });
    id
}

pub fn check_vm(c: &mut Checker, i: usize) -> Res<()> {
    let (m, d) = c.vm_decls[i].clone();
    let vm = i as u32;
    let l = crate::vmexpand::layout(&d)?;
    let prefix = format!("{}.{}", c.module_prefix(m), d.name.name);
    let word = IntTy { signed: false, bits: l.word_bits as u8 };

    let mut state: Vec<(String, Ty)> = Vec::new();
    let add_state = |c: &mut Checker, p: &Param, state: &mut Vec<(String, Ty)>| -> Res<()> {
        if state.iter().any(|(n, _)| *n == p.name.name) {
            return Err(Diag::new(p.name.span, format!("state variable `{}` declared twice", p.name.name)));
        }
        let t = c.resolve_type(m, &p.ty)?;
        state.push((p.name.name.clone(), t));
        Ok(())
    };
    if let Some(p) = &d.context {
        add_state(c, p, &mut state)?;
    }
    for p in &d.state {
        add_state(c, p, &mut state)?;
    }
    let tbl_idx = if d.variants.is_empty() {
        None
    } else {
        state.push(("__tbl".to_string(), Ty::Ptr(Box::new(Ty::u8()))));
        Some(state.len() - 1)
    };
    let pc_idx = state
        .iter()
        .position(|(n, _)| n == "pc")
        .ok_or_else(|| Diag::new(d.name.span, "a vm needs a state variable named `pc`"))?;
    if state[pc_idx].1 != Ty::Ptr(Box::new(Ty::Int(word))) {
        return Err(Diag::new(d.name.span, format!("`pc` must have type `*u{}` (pointer to the code word)", word.bits)));
    }
    let mut caches = Vec::new();
    for cd in &d.caches {
        let mut fields = Vec::new();
        for (f, t, _weak) in &cd.fields {
            let ft = c.resolve_type(m, t)?;
            if ft != Ty::Int(word) {
                return Err(Diag::new(t.span, format!("cache fields must have the code word type `u{}`", word.bits)));
            }
            fields.push(f.name.clone());
        }
        caches.push((cd.name.name.clone(), fields));
    }
    let kinds: Vec<OperandKind> = d
        .operands
        .iter()
        .map(|o| OperandKind { name: o.name.name.clone(), body: o.body.clone(), verify: o.verify.clone() })
        .collect();
    let dispatch = match c.dispatch_override {
        Some(x) => x,
        None => match d.attrs.get("dispatch").and_then(|a| a.first()).map(|s| s.as_str()) {
            Some("tailcall") | None => Dispatch::TailCall,
            Some("goto") => Dispatch::Goto,
            Some("switch") => Dispatch::Switch,
            Some(x) => return Err(Diag::new(d.name.span, format!("unknown dispatch mode `{}`", x))),
        },
    };
    let cc = if dispatch == Dispatch::TailCall { CallConv::PreserveNone } else { CallConv::C };
    if d.entries.is_empty() {
        return Err(Diag::new(d.name.span, "a vm needs at least one `entry`"));
    }
    let ret = c.resolve_type(m, &d.entries[0].ret)?;
    for e in &d.entries[1..] {
        if c.resolve_type(m, &e.ret)? != ret {
            return Err(Diag::new(e.ret.span, "all entries of a vm must return the same type"));
        }
    }
    let raise_ty = match &d.on_raise {
        Some(r) => Some(c.resolve_type(m, &r.param.ty)?),
        None => None,
    };
    let state_tys: Vec<Ty> = state.iter().map(|(_, t)| t.clone()).collect();

    // Ops.
    let mut ops = Vec::new();
    for (k, lo) in l.ops.iter().enumerate() {
        let (format, cache, params, kinds_of, body, span) = if lo.parts.is_empty() {
            let od = &d.ops[k];
            let mut ks = Vec::new();
            for p in &od.params {
                let ki = kinds
                    .iter()
                    .position(|x| x.name == p.kind.name)
                    .ok_or_else(|| Diag::new(p.kind.span, format!("unknown operand kind `{}`", p.kind.name)))?;
                ks.push(ki);
            }
            let cache = match &od.cache {
                Some(cn) => Some(caches.iter().position(|(n, _)| *n == cn.name).unwrap()),
                None => None,
            };
            (
                od.format.name.clone(),
                cache,
                od.params.iter().map(|p| p.name.name.clone()).collect(),
                ks,
                Some(od.body.clone()),
                od.name.span,
            )
        } else {
            let sd = &d.supers[k - d.ops.len()];
            (String::new(), None, vec![], vec![], None, sd.name.span)
        };
        let fid = new_fn(
            c,
            format!("{}.op.{}", prefix, lo.name),
            lo.name.clone(),
            state_tys.clone(),
            ret.clone(),
            cc,
            vm,
            VmRole::Handler(k as u32),
            span,
        );
        ops.push(OpInfo {
            name: lo.name.clone(),
            opcode: lo.opcode,
            format,
            cache,
            params,
            kinds: kinds_of,
            fields: lo.fields.iter().map(|f| (f.1, f.2, f.3)).collect(),
            parts: lo.parts.clone(),
            size: l.size(k),
            fid,
            body,
            span,
        });
    }
    // vm functions.
    let mut fns = HashMap::new();
    let mut fn_decls = Vec::new();
    let mut never_fns: Vec<NeverFn> = Vec::new();
    for f in &d.fns {
        let mut pt: Vec<Ty> = state_tys.iter().map(|t| Ty::Ptr(Box::new(t.clone()))).collect();
        for p in &f.params {
            pt.push(c.resolve_type(m, &p.ty)?);
        }
        let r = match &f.ret {
            Some(t) => c.resolve_type(m, t)?,
            None => Ty::Unit,
        };
        if fns.contains_key(&f.name.name) || never_fns.iter().any(|n| n.decl.name.name == f.name.name) {
            return Err(Diag::new(f.name.span, format!("vm function `{}` defined twice", f.name.name)));
        }
        if r == Ty::Never {
            let inline = match f.attrs.get("inline") {
                Some(a) => Some(a.first().map(|s| s.as_str()) != Some("never")),
                None => None,
            };
            let shared = if dispatch == Dispatch::TailCall && inline != Some(true) {
                let idx = never_fns.len() as u32;
                let sym = format!("{}.fn.{}", prefix, f.name.name);
                Some(new_fn(c, sym, f.name.name.clone(), state_tys.clone(), ret.clone(), cc, vm, VmRole::Shared(idx), f.name.span))
            } else {
                None
            };
            never_fns.push(NeverFn {
                decl: f.clone(),
                params: pt[state_tys.len()..].to_vec(),
                shared,
                inline,
                uses_next: false,
                calls: vec![],
                no_op_calls: vec![],
            });
            continue;
        }
        let fid = new_fn(c, format!("{}.fn.{}", prefix, f.name.name), f.name.name.clone(), pt, r, CallConv::C, vm, VmRole::VmFn, f.name.span);
        c.prog.fns[fid as usize].effects = crate::check::effects_of(&f.attrs)?;
        fns.insert(f.name.name.clone(), fid);
        fn_decls.push((fid, f.clone()));
    }
    // Entries.
    let mut entries = Vec::new();
    for e in &d.entries {
        let mut pt = Vec::new();
        for p in &e.params {
            pt.push(c.resolve_type(m, &p.ty)?);
        }
        let fid = new_fn(c, format!("{}.{}", prefix, e.name.name), e.name.name.clone(), pt, ret.clone(), CallConv::C, vm, VmRole::Entry, e.name.span);
        let gname = Ident { name: format!("{}__{}", d.name.name, e.name.name), span: e.name.span };
        if c.scopes[m].contains_key(&gname.name) {
            return Err(Diag::new(e.name.span, format!("`{}` clashes with a generated vm item", e.name.name)));
        }
        c.scopes[m].insert(gname.name, (Def::Fn(fid), d.is_pub));
        entries.push((fid, e.clone()));
    }
    let raise_fid = match &d.on_raise {
        Some(r) => Some(new_fn(
            c,
            format!("{}.on_raise", prefix),
            "on raise".into(),
            state_tys.clone(),
            ret.clone(),
            cc,
            vm,
            VmRole::Raise,
            r.body.span,
        )),
        None => None,
    };

    c.prog.vms.push(VmDef {
        name: d.name.name.clone(),
        symbol: prefix.clone(),
        dispatch,
        word,
        op_shift: l.op_shift,
        op_bits: l.op_bits,
        state_tys: state_tys.clone(),
        state_names: state.iter().map(|(n, _)| n.clone()).collect(),
        pc_idx,
        ret: ret.clone(),
        ops: ops
            .iter()
            .map(|o| VmOp {
                name: o.name.clone(),
                opcode: o.opcode,
                fid: o.fid,
                size: o.size,
                parts: o.parts.iter().map(|&p| p as u32).collect(),
            })
            .collect(),
        entries: entries.iter().map(|(f, _)| *f).collect(),
        on_raise: raise_fid,
        raise_ty: raise_ty.clone(),
        variants: vec![],
        tbl_idx,
    });
    c.vms.push(VmInfo {
        module: m,
        name: d.name.name.clone(),
        word,
        state,
        pc_idx,
        kinds,
        caches,
        ops,
        ret: ret.clone(),
        raise_ty,
        fns,
        never_fns,
        sync: d.sync.clone(),
    });

    // Bodies.
    for (fid, f) in fn_decls {
        check_vm_fn(c, vm, fid, &f)?;
    }
    for j in 0..c.vms[i].never_fns.len() {
        check_never_fn(c, vm, j)?;
    }
    for k in 0..c.vms[i].ops.len() {
        check_handler(c, vm, k)?;
    }
    for (fid, e) in entries {
        check_entry(c, vm, fid, &e)?;
    }
    if let (Some(fid), Some(r)) = (raise_fid, &d.on_raise) {
        check_raise(c, vm, fid, r)?;
    }
    // Instrumentation variants: a wrapper per wrapped op.
    for (vi, var) in d.variants.iter().enumerate() {
        if var.name.name == "default" || d.variants[..vi].iter().any(|x| x.name.name == var.name.name) {
            return Err(Diag::new(var.name.span, format!("bad or duplicate variant name `{}`", var.name.name)));
        }
        let ops: Vec<usize> = match &var.wraps {
            None => (0..c.vms[i].ops.len()).collect(),
            Some(list) => {
                let mut v = Vec::new();
                for o in list {
                    let k = c.vms[i]
                        .ops
                        .iter()
                        .position(|x| x.name == o.name)
                        .ok_or_else(|| Diag::new(o.span, format!("unknown op `{}`", o.name)))?;
                    v.push(k);
                }
                v
            }
        };
        let mut wrappers = Vec::new();
        for k in ops {
            let opname = c.vms[i].ops[k].name.clone();
            let fid = new_fn(
                c,
                format!("{}.variant.{}.{}", prefix, var.name.name, opname),
                format!("{} ({})", opname, var.name.name),
                state_tys.clone(),
                ret.clone(),
                cc,
                vm,
                VmRole::Wrapper(vi as u32, k as u32),
                var.before.span,
            );
            check_wrapper(c, vm, fid, k, &var.before)?;
            wrappers.push((k as u32, fid));
        }
        c.prog.vms[i].variants.push((var.name.name.clone(), wrappers));
    }
    check_shared_next(c, i)
}

fn check_wrapper(c: &mut Checker, vm: u32, fid: FnId, k: usize, before: &Block) -> Res<()> {
    let info = &c.vms[vm as usize];
    let m = info.module;
    let opcode = info.ops[k].opcode;
    let mut cx = FnCx::new(m, info.ret.clone());
    let state = declare_state(c, vm, &mut cx);
    let opc = cx.declare("opcode", Ty::Int(IntTy::U32), false);
    cx.vm = Some(HandlerCx { op: Some(k), ..HandlerCx::new(vm, state.clone()) });
    let b = c.block(&mut cx, before, Some(&Ty::Unit))?;
    let span = before.span;
    let body = mk(
        Ty::Never,
        TK::Block(
            vec![TStmt::Let(opc, Some(mk(Ty::Int(IntTy::U32), TK::Int(opcode as i128), span))), TStmt::Expr(b)],
            Some(Box::new(mk(Ty::Never, TK::Vm(TVm::Chain(k as u32)), span))),
        ),
        span,
    );
    let fd = &mut c.prog.fns[fid as usize];
    fd.params = state.clone();
    fd.locals = cx.locals;
    fd.body = Some(body);
    fd.vm_state = state;
    Ok(())
}

fn declare_state(c: &Checker, vm: u32, cx: &mut FnCx) -> Vec<LocalId> {
    let st = c.vms[vm as usize].state.clone();
    st.iter().map(|(n, t)| cx.declare(n, t.clone(), true)).collect()
}

/// The standalone body of `-> never` vm function `j`: its shared copy in
/// tail-call mode, and in every mode the check that reports errors in
/// functions nobody calls. Arguments arrive through thread-local slots.
fn check_never_fn(c: &mut Checker, vm: u32, j: usize) -> Res<()> {
    let info = &c.vms[vm as usize];
    let m = info.module;
    let nf = &info.never_fns[j];
    let decl = nf.decl.clone();
    let params = nf.params.clone();
    let shared = nf.shared;
    let mut cx = FnCx::new(m, info.ret.clone());
    cx.fn_name = decl.name.name.clone();
    let state = declare_state(c, vm, &mut cx);
    let span = decl.name.span;
    let arg = |k: usize, t: &Ty| mk(t.clone(), TK::Vm(TVm::SharedArg(shared.unwrap_or(u32::MAX), k as u32)), span);
    let size = cx.temp(Ty::i64());
    cx.locals[size as usize].name = "next.size".into();
    let mut lets = vec![TStmt::Let(size, Some(arg(params.len(), &Ty::i64())))];
    cx.scopes.push(vec![]);
    for (k, (p, t)) in decl.params.iter().zip(params.iter()).enumerate() {
        let l = cx.declare(&p.name.name, t.clone(), true);
        lets.push(TStmt::Let(l, Some(arg(k, t))));
    }
    cx.vm = Some(HandlerCx { in_fn: true, cold: true, shared: Some((j, size)), inlining: vec![j], ..HandlerCx::new(vm, state.clone()) });
    let b = c.block(&mut cx, decl.body.as_ref().unwrap(), Some(&Ty::Never))?;
    if b.ty != Ty::Never {
        return Err(never_falloff(&decl));
    }
    if let Some(fid) = shared {
        let body = mk(Ty::Never, TK::Block(lets, Some(Box::new(b))), span);
        let fd = &mut c.prog.fns[fid as usize];
        fd.params = state.clone();
        fd.locals = cx.locals;
        fd.body = Some(body);
        fd.vm_state = state;
    }
    Ok(())
}

fn never_falloff(f: &FnDecl) -> Diag {
    let span = f.body.as_ref().map(|b| b.span).unwrap_or(f.name.span);
    Diag::new(
        span,
        format!("vm function `{}` returns `never` but can reach its end; every path must end in a control word", f.name.name),
    )
}

/// Shared copies run `next` with the size their caller passes. Entries
/// and `on raise` have no instruction to skip, so they may not reach a
/// shared copy that uses `next`.
fn check_shared_next(c: &Checker, vm: usize) -> Res<()> {
    let nfs = &c.vms[vm].never_fns;
    let mut uses: Vec<bool> = nfs.iter().map(|n| n.uses_next).collect();
    loop {
        let mut changed = false;
        for (j, n) in nfs.iter().enumerate() {
            if !uses[j] && n.calls.iter().any(|&q| uses[q]) {
                uses[j] = true;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    for (j, n) in nfs.iter().enumerate() {
        if let Some(&sp) = n.no_op_calls.first() {
            if uses[j] {
                return Err(Diag::new(
                    sp,
                    format!("`{}` uses `next`, which is only allowed in op handlers, but is called here outside one", n.decl.name.name),
                ));
            }
        }
    }
    Ok(())
}

fn never_fn_index(c: &Checker, vm: u32, name: &str) -> Option<usize> {
    c.vms[vm as usize].never_fns.iter().position(|n| n.decl.name.name == name)
}

/// A call of `-> never` vm function `j`: inline its body here, or (tail-call
/// mode, cold or recursive call sites, `#[inline(never)]`) pass the
/// arguments through thread-local slots and `musttail` to its shared copy.
fn never_call(c: &mut Checker, cx: &mut FnCx, j: usize, args: &[Expr], span: Span) -> Res<TExpr> {
    let h = cx.vm.clone().unwrap();
    let vmi = h.vm as usize;
    let nf = &c.vms[vmi].never_fns[j];
    let name = nf.decl.name.name.clone();
    if !h.allow_control {
        return Err(Diag::new(
            span,
            format!("`{}` ends in a control word; call it from op handlers, entries, `on raise` or other `-> never` vm functions", name),
        ));
    }
    let params = nf.params.clone();
    let decl = nf.decl.clone();
    if args.len() != params.len() {
        return Err(Diag::new(span, format!("`{}` takes {} argument(s), got {}", name, params.len(), args.len())));
    }
    let recursive = h.inlining.contains(&j);
    let share = nf.shared.is_some() && (nf.inline == Some(false) || h.cold || recursive);
    let mut vals = Vec::new();
    for (a, t) in args.iter().zip(params.iter()) {
        vals.push(c.expr_coerce(cx, a, t)?);
    }
    if share {
        let fid = c.vms[vmi].never_fns[j].shared.unwrap();
        let int = |v: u32| mk(Ty::i64(), TK::Int(v as i128), span);
        let size = match (h.part, h.op, h.shared) {
            (Some((_, size)), _, _) => int(size),
            (None, Some(op), _) => int(c.vms[vmi].ops[op].size),
            (None, None, Some((caller, l))) => {
                c.vms[vmi].never_fns[caller].calls.push(j);
                mk(Ty::i64(), TK::Local(l), span)
            }
            (None, None, None) => {
                c.vms[vmi].never_fns[j].no_op_calls.push(span);
                int(0)
            }
        };
        return Ok(mk(Ty::Never, TK::Vm(TVm::Shared(fid, vals, Box::new(size))), span));
    }
    if recursive {
        return Err(Diag::new(
            span,
            format!("recursive call of `{}` cannot be inlined (`#[inline]` functions, and all of them under goto and switch dispatch, are inlined)", name),
        ));
    }
    // The body sees the vm state and its parameters, nothing of the caller.
    let info = &c.vms[vmi];
    let state_scope: Vec<(String, LocalId)> = info.state.iter().map(|(n, _)| n.clone()).zip(h.state.iter().copied()).collect();
    let saved_scopes = std::mem::replace(&mut cx.scopes, vec![state_scope, vec![]]);
    let saved_loops = std::mem::take(&mut cx.loops);
    let mut lets = Vec::new();
    for ((p, t), v) in decl.params.iter().zip(params.iter()).zip(vals) {
        let l = cx.declare(&p.name.name, t.clone(), true);
        lets.push(TStmt::Let(l, Some(v)));
    }
    let mut h2 = h.clone();
    h2.in_fn = true;
    h2.operands.clear();
    h2.inlining.push(j);
    cx.vm = Some(h2);
    let r = c.block(cx, decl.body.as_ref().unwrap(), Some(&Ty::Never));
    cx.scopes = saved_scopes;
    cx.loops = saved_loops;
    cx.vm = Some(h);
    let b = r?;
    if b.ty != Ty::Never {
        return Err(never_falloff(&decl));
    }
    Ok(mk(Ty::Never, TK::Block(lets, Some(Box::new(b))), span))
}

fn check_vm_fn(c: &mut Checker, vm: u32, fid: FnId, f: &FnDecl) -> Res<()> {
    let info = &c.vms[vm as usize];
    let m = info.module;
    let st = info.state.clone();
    let ret = c.prog.fns[fid as usize].ret.clone();
    let mut cx = FnCx::new(m, ret.clone());
    cx.effects = c.prog.fns[fid as usize].effects;
    cx.fn_name = f.name.name.clone();
    let mut params = Vec::new();
    let mut state = Vec::new();
    for (n, t) in &st {
        let id = cx.temp(Ty::Ptr(Box::new(t.clone())));
        cx.locals[id as usize].name = format!("state.{}", n);
        params.push(id);
        state.push(id);
    }
    let pts = c.prog.fns[fid as usize].param_tys.clone();
    for (p, t) in f.params.iter().zip(pts[st.len()..].iter()) {
        params.push(cx.declare(&p.name.name, t.clone(), true));
    }
    cx.vm = Some(HandlerCx { is_handler: false, allow_control: false, by_ptr: true, ..HandlerCx::new(vm, state.clone()) });
    let b = c.block(&mut cx, f.body.as_ref().unwrap(), Some(&ret))?;
    let b = c.coerce(b, &ret)?;
    let fd = &mut c.prog.fns[fid as usize];
    fd.params = params;
    fd.locals = cx.locals;
    fd.body = Some(b);
    fd.vm_state = state;
    Ok(())
}

fn check_handler(c: &mut Checker, vm: u32, k: usize) -> Res<()> {
    let info = &c.vms[vm as usize];
    let m = info.module;
    let word = info.word;
    let fid = info.ops[k].fid;
    let parts: Vec<usize> = if info.ops[k].parts.is_empty() { vec![k] } else { info.ops[k].parts.clone() };
    let ret = info.ret.clone();
    let mut cx = FnCx::new(m, ret);
    let state = declare_state(c, vm, &mut cx);
    let mut exprs = Vec::new();
    let n = parts.len();
    for (j, &p) in parts.iter().enumerate() {
        let op = &c.vms[vm as usize].ops[p];
        let span = op.span;
        let body = op.body.clone().unwrap();
        let size = op.size;
        let name = op.name.clone();
        let fields = op.fields.clone();
        let params = op.params.clone();
        let kinds = op.kinds.clone();
        cx.scopes.push(vec![]);
        let mut lets = Vec::new();
        let mut operands = Vec::new();
        for (pi, pname) in params.iter().enumerate() {
            let (bits, shift, signed) = fields[pi];
            let raw = cx.temp(raw_ty(bits, signed));
            cx.locals[raw as usize].name = format!("{}.raw", pname);
            cx.locals[raw as usize].mutable = false;
            lets.push(TStmt::Let(raw, Some(decode(word, bits, shift, signed, span))));
            operands.push((pname.clone(), kinds[pi], raw));
        }
        cx.vm = Some(HandlerCx {
            op: Some(p),
            part: if j + 1 < n { Some((j as u32 + 1, size)) } else { None },
            operands,
            ..HandlerCx::new(vm, state.clone())
        });
        let r = c.block(&mut cx, &body, Some(&Ty::Never));
        cx.scopes.pop();
        let b = r?;
        if b.ty != Ty::Never {
            return Err(Diag::new(
                body.span,
                format!("handler `{}` can reach its end; every path must end in `next`, `jump`, `exit`, `raise` or `redo`", name),
            ));
        }
        let e = mk(Ty::Never, TK::Block(lets, Some(Box::new(b))), span);
        exprs.push(if n > 1 { mk(Ty::Never, TK::Vm(TVm::Part(j as u32, Box::new(e))), span) } else { e });
    }
    let span = c.vms[vm as usize].ops[k].span;
    let mut body = if exprs.len() == 1 {
        exprs.pop().unwrap()
    } else {
        mk(Ty::Never, TK::Block(exprs.into_iter().map(TStmt::Expr).collect(), None), span)
    };
    if n == 1 {
        crate::tirwalk::extend_cold(&mut body);
    }
    let fd = &mut c.prog.fns[fid as usize];
    fd.params = state.clone();
    fd.locals = cx.locals;
    fd.body = Some(body);
    fd.vm_state = state;
    Ok(())
}

fn check_entry(c: &mut Checker, vm: u32, fid: FnId, e: &EntryDecl) -> Res<()> {
    let info = &c.vms[vm as usize];
    let m = info.module;
    let ret = info.ret.clone();
    let mut cx = FnCx::new(m, ret.clone());
    let mut params = Vec::new();
    let pts = c.prog.fns[fid as usize].param_tys.clone();
    for (p, t) in e.params.iter().zip(pts.iter()) {
        params.push(cx.declare(&p.name.name, t.clone(), true));
    }
    let state = declare_state(c, vm, &mut cx);
    cx.vm = Some(HandlerCx { is_handler: false, ..HandlerCx::new(vm, state.clone()) });
    let b = c.block(&mut cx, &e.body, Some(&ret))?;
    let b = c.coerce(b, &ret)?;
    let mut inits: Vec<TStmt> = state.iter().map(|&s| TStmt::Let(s, None)).collect();
    if let Some(ti) = c.prog.vms[vm as usize].tbl_idx {
        let t = Ty::Ptr(Box::new(Ty::u8()));
        let sp = e.body.span;
        inits.push(TStmt::Expr(mk(
            Ty::Unit,
            TK::Assign(Box::new(mk(t.clone(), TK::Local(state[ti]), sp)), Box::new(mk(t, TK::Vm(TVm::TableAddr(0)), sp))),
            sp,
        )));
    }
    let span = b.span;
    let body = mk(ret, TK::Block(inits, Some(Box::new(b))), span);
    let fd = &mut c.prog.fns[fid as usize];
    fd.params = params;
    fd.locals = cx.locals;
    fd.body = Some(body);
    fd.vm_state = state;
    Ok(())
}

fn check_raise(c: &mut Checker, vm: u32, fid: FnId, r: &RaiseDecl) -> Res<()> {
    let info = &c.vms[vm as usize];
    let m = info.module;
    let rt = info.raise_ty.clone().unwrap();
    let mut cx = FnCx::new(m, info.ret.clone());
    let state = declare_state(c, vm, &mut cx);
    let err = cx.declare(&r.param.name.name, rt.clone(), true);
    cx.vm = Some(HandlerCx::new(vm, state.clone()));
    let b = c.block(&mut cx, &r.body, Some(&Ty::Never))?;
    if b.ty != Ty::Never {
        return Err(Diag::new(r.body.span, "`on raise` can reach its end; every path must end in a control word"));
    }
    let span = b.span;
    let body = mk(
        Ty::Never,
        TK::Block(vec![TStmt::Let(err, Some(mk(rt, TK::Vm(TVm::RaiseSlot), span)))], Some(Box::new(b))),
        span,
    );
    let fd = &mut c.prog.fns[fid as usize];
    fd.params = state.clone();
    fd.locals = cx.locals;
    fd.body = Some(body);
    fd.vm_state = state;
    Ok(())
}

pub fn check_vm_expr(c: &mut Checker, cx: &mut FnCx, v: &VmExpr, exp: Option<&Ty>, span: Span) -> Res<TExpr> {
    if let VmExpr::Cold(b) = v {
        let saved = cx.vm.clone();
        if let Some(h) = cx.vm.as_mut() {
            h.cold = true;
        }
        let e = c.block(cx, b, exp);
        cx.vm = saved;
        let e = e?;
        let t = e.ty.clone();
        return Ok(mk(t, TK::Vm(TVm::Cold(Box::new(e))), span));
    }
    let Some(h) = cx.vm.clone() else {
        return Err(Diag::new(span, "vm control words are only allowed in vm handlers"));
    };
    if !h.allow_control {
        return Err(Diag::new(span, "control words are not allowed in vm functions; return a value instead"));
    }
    let vmi = h.vm as usize;
    let pc_ty = c.vms[vmi].state[c.vms[vmi].pc_idx].1.clone();
    let pc_local = h.state[c.vms[vmi].pc_idx];
    let pc = || mk(pc_ty.clone(), TK::Local(pc_local), span);
    let need_op = |what: &str| -> Res<usize> {
        h.op.ok_or_else(|| Diag::new(span, format!("`{}` is only allowed in op handlers", what)))
    };
    let never = |k: TVm| Ok(mk(Ty::Never, TK::Vm(k), span));
    match v {
        VmExpr::Next if h.op.is_none() && h.shared.is_some() => {
            let (j, size) = h.shared.unwrap();
            c.vms[vmi].never_fns[j].uses_next = true;
            let n = mk(Ty::i64(), TK::Local(size), span);
            let target = mk(pc_ty.clone(), TK::Intrinsic(Intr::PtrOffset, vec![pc(), n]), span);
            never(TVm::Jump(Box::new(target)))
        }
        VmExpr::Next => {
            let op = need_op("next")?;
            if let Some((next_part, size)) = h.part {
                return never(TVm::Fallthrough(size, next_part));
            }
            let size = c.vms[vmi].ops[op].size;
            let n = mk(Ty::i64(), TK::Int(size as i128), span);
            let target = mk(pc_ty.clone(), TK::Intrinsic(Intr::PtrOffset, vec![pc(), n]), span);
            never(TVm::Jump(Box::new(target)))
        }
        VmExpr::Redo => {
            if h.shared.is_none() {
                need_op("redo")?;
            }
            never(TVm::Jump(Box::new(pc())))
        }
        VmExpr::Jump(t) => {
            let t = c.expr_coerce(cx, t, &pc_ty)?;
            never(TVm::Jump(Box::new(t)))
        }
        VmExpr::Exit(e) => {
            let ret = c.vms[vmi].ret.clone();
            let e = c.expr_coerce(cx, e, &ret)?;
            never(TVm::Exit(Box::new(e)))
        }
        VmExpr::Fail(e) => {
            let ret = c.vms[vmi].ret.clone();
            let Ty::Result(_, et) = &ret else {
                return Err(Diag::new(span, "`fail` needs a vm whose entries return a Result"));
            };
            let e = c.expr_coerce(cx, e, et)?;
            let r = mk(ret.clone(), TK::Variant(1, vec![e]), span);
            never(TVm::Exit(Box::new(r)))
        }
        VmExpr::Raise(e) => {
            let Some(rt) = c.vms[vmi].raise_ty.clone() else {
                return Err(Diag::new(span, "`raise` needs an `on raise` handler in the vm"));
            };
            let e = c.expr_coerce(cx, e, &rt)?;
            never(TVm::Raise(Box::new(e)))
        }
        VmExpr::Become(name) => {
            if h.in_fn {
                return Err(Diag::new(span, "`become` is not allowed in vm functions; it rewrites the calling instruction"));
            }
            let op = need_op("become")?;
            let info = &c.vms[vmi];
            let Some(t) = info.ops.iter().position(|o| o.name == name.name && o.parts.is_empty()) else {
                return Err(Diag::new(name.span, format!("unknown op `{}`", name.name)));
            };
            let (a, b) = (&info.ops[op], &info.ops[t]);
            if a.format != b.format || a.cache != b.cache || a.kinds != b.kinds {
                return Err(Diag::new(
                    name.span,
                    format!("`become {}`: `{}` and `{}` must have the same format, cache and operand kinds", b.name, a.name, b.name),
                ));
            }
            Ok(mk(Ty::Unit, TK::Vm(TVm::Become(b.opcode)), span))
        }
        VmExpr::UseVariant(name) => {
            let vd = &c.prog.vms[vmi];
            let idx = if name.name == "default" {
                0
            } else {
                let Some(i) = c.vm_decls[vmi].1.variants.iter().position(|v| v.name.name == name.name) else {
                    return Err(Diag::new(name.span, format!("unknown variant `{}`", name.name)));
                };
                i as u32 + 1
            };
            let Some(ti) = vd.tbl_idx else {
                return Err(Diag::new(name.span, "this vm declares no variants"));
            };
            let t = Ty::Ptr(Box::new(Ty::u8()));
            let target = mk(t.clone(), TK::Local(h.state[ti]), span);
            let v = mk(t, TK::Vm(TVm::TableAddr(idx)), span);
            Ok(mk(Ty::Unit, TK::Assign(Box::new(target), Box::new(v)), span))
        }
        VmExpr::Cold(_) => unreachable!(),
    }
}

pub fn vm_ident(c: &mut Checker, cx: &mut FnCx, name: &str, span: Span) -> Res<Option<TExpr>> {
    let Some(h) = cx.vm.clone() else { return Ok(None) };
    if let Some((_, kind, raw)) = h.operands.iter().find(|(n, _, _)| n == name) {
        let body = c.vms[h.vm as usize].kinds[*kind].body.clone();
        cx.scopes.push(vec![("it".to_string(), *raw)]);
        let r = c.expr(cx, &body, None);
        cx.scopes.pop();
        let mut r = r?;
        r.span = span;
        return Ok(Some(r));
    }
    if h.by_ptr {
        let info = &c.vms[h.vm as usize];
        if let Some(i) = info.state.iter().position(|(n, _)| n == name) {
            let t = info.state[i].1.clone();
            let p = mk(Ty::Ptr(Box::new(t.clone())), TK::Local(h.state[i]), span);
            return Ok(Some(mk(t, TK::Deref(Box::new(p)), span)));
        }
    }
    Ok(None)
}

pub fn vm_field(c: &mut Checker, cx: &mut FnCx, base: &Expr, name: &Ident, span: Span) -> Res<Option<TExpr>> {
    let Some(h) = cx.vm.clone() else { return Ok(None) };
    let ExprKind::Ident(bn) = &base.kind else { return Ok(None) };
    if cx.lookup(bn).is_some() {
        return Ok(None);
    }
    if name.name == "raw" {
        if let Some((_, _, raw)) = h.operands.iter().find(|(n, _, _)| n == bn) {
            let t = cx.locals[*raw as usize].ty.clone();
            return Ok(Some(mk(t, TK::Local(*raw), span)));
        }
    }
    if bn == "cache" {
        if h.in_fn {
            return Err(Diag::new(base.span, "`cache` is not available in vm functions; pass the values as arguments"));
        }
        let info = &c.vms[h.vm as usize];
        let Some(op) = h.op else {
            return Err(Diag::new(base.span, "`cache` is only available in op handlers"));
        };
        let Some(ci) = info.ops[op].cache else {
            return Err(Diag::new(base.span, format!("op `{}` has no inline cache", info.ops[op].name)));
        };
        let Some(fi) = info.caches[ci].1.iter().position(|f| *f == name.name) else {
            return Err(Diag::new(name.span, format!("no field `{}` in cache `{}`", name.name, info.caches[ci].0)));
        };
        return Ok(Some(mk(Ty::Int(info.word), TK::Vm(TVm::Word(1 + fi as u32)), span)));
    }
    Ok(None)
}

fn vm_fn(c: &Checker, cx: &FnCx, name: &str) -> Option<FnId> {
    let h = cx.vm.as_ref()?;
    c.vms[h.vm as usize].fns.get(name).copied()
}

/// Whether `name` is a vm function visible here.
pub fn is_vm_fn(c: &Checker, cx: &FnCx, name: &str) -> bool {
    vm_fn(c, cx, name).is_some() || cx.vm.as_ref().is_some_and(|h| never_fn_index(c, h.vm, name).is_some())
}

pub fn vm_fn_call(c: &mut Checker, cx: &mut FnCx, name: &str, args: &[Expr], span: Span) -> Res<Option<TExpr>> {
    if let Some(h) = &cx.vm {
        if let Some(j) = never_fn_index(c, h.vm, name) {
            return never_call(c, cx, j, args, span).map(Some);
        }
    }
    let Some(fid) = vm_fn(c, cx, name) else { return Ok(None) };
    let h = cx.vm.clone().unwrap();
    let info = &c.vms[h.vm as usize];
    let mut pre = Vec::new();
    for (i, (_, t)) in info.state.iter().enumerate() {
        let pt = Ty::Ptr(Box::new(t.clone()));
        if h.by_ptr {
            pre.push(mk(pt, TK::Local(h.state[i]), span));
        } else {
            let l = mk(t.clone(), TK::Local(h.state[i]), span);
            pre.push(mk(pt, TK::AddrOf(Box::new(l)), span));
        }
    }
    let f = c.def_value(Def::Fn(fid), name, span)?;
    c.finish_call(cx, f, pre, args, span).map(Some)
}

pub fn check_raise_ty(c: &mut Checker, cx: &mut FnCx, e: &Ty, span: Span) -> Res<()> {
    let h = cx.vm.clone().unwrap();
    if !h.allow_control {
        return Err(Diag::new(span, "`?` on a result in a vm function returning something else"));
    }
    match &c.vms[h.vm as usize].raise_ty {
        Some(t) if t == e => Ok(()),
        Some(t) => {
            let msg = format!("`?` raises `{}` but the vm's `on raise` takes `{}`", c.ty_name(e), c.ty_name(t));
            Err(Diag::new(span, msg))
        }
        None => Err(Diag::new(span, "`?` on a result in a handler needs an `on raise` handler in the vm")),
    }
}

/// Check a call against the caller's effects. In handlers, effectful calls
/// are wrapped so that codegen syncs vm state and roots traced locals.
/// The effects come from the callee's type, so direct calls and calls
/// through function pointers are treated alike.
pub fn wrap_effects(c: &mut Checker, cx: &mut FnCx, call: TExpr) -> Res<TExpr> {
    let TK::Call(f, _) = &call.kind else { unreachable!() };
    let Ty::Fn(_, _, e) = f.ty else { unreachable!() };
    let callee = match f.kind {
        TK::FnRef(id) => format!("`{}`", c.prog.fns[id as usize].name),
        _ => format!("this call through a `{}`", c.ty_name(&f.ty)),
    };
    let missing = e & !cx.effects;
    if missing == 0 {
        return Ok(call);
    }
    let span = call.span;
    if let Some(h) = cx.vm.clone() {
        if h.allow_control && h.is_handler {
            let sync = match c.vms[h.vm as usize].sync.clone() {
                Some(b) => c.block(cx, &b, Some(&Ty::Unit))?,
                None => mk(Ty::Unit, TK::Unit, span),
            };
            let t = call.ty.clone();
            return Ok(mk(t, TK::Vm(TVm::Synced(Box::new(sync), Box::new(call))), span));
        }
        if h.allow_control {
            // Entries run before dispatch starts: nothing to sync yet.
            return Ok(call);
        }
    }
    let caller = if cx.fn_name.is_empty() { "this function".to_string() } else { format!("`{}`", cx.fn_name) };
    let eff = effect_name(missing);
    Err(Diag::new(
        span,
        format!("{} uses {}, but {} does not declare it; add `uses {}` to its signature", callee, eff, caller, eff),
    ))
}
