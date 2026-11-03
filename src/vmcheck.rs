//! Checking of `vm` declarations: handlers, operands, control words, vm
//! functions, entries and the error path.

use std::collections::HashMap;

use crate::ast::*;
use crate::check::{effect_name, mk, Checker, Def, FnCx};
use crate::diag::{Diag, Res, Span};
use crate::tir::*;
use crate::vmast::*;

pub struct OperandKind {
    pub name: String,
    pub body: Expr,
    /// A `const` kind: the state register its body indexes (`k` in
    /// `k[it]`), which vm code must not write through.
    pub konst: Option<usize>,
}

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
    /// `then OP`: the op that follows this one.
    pub then: Option<usize>,
    /// A forwarding variant of a `#[forward]` super: its links
    /// (`vmexpand::OpLayout::fwd`).
    pub fwd: Vec<(usize, usize, usize)>,
    pub size: u32,
    pub fid: FnId,
    pub body: Option<Block>,
    pub span: Span,
}

pub struct VmInfo {
    pub module: usize,
    pub name: String,
    pub word: IntTy,
    pub state: Vec<(String, Ty)>,
    pub pc_idx: usize,
    pub kinds: Vec<OperandKind>,
    pub caches: Vec<CacheInfo>,
    pub ops: Vec<OpInfo>,
    pub ret: Ty,
    pub raise_ty: Option<Ty>,
    pub fns: HashMap<String, FnId>,
    /// Per vm function (not `-> never`): parameter names, and the cache
    /// layout of each parameter declared `cache Name`.
    pub fn_caches: HashMap<String, (Vec<String>, Vec<Option<usize>>)>,
    /// `-> never` vm functions, which may end in control words.
    pub never_fns: Vec<NeverFn>,
    pub sync: Option<Block>,
    /// Run after a call that may re-enter the vm (`uses reenter`).
    pub reload: Option<Block>,
    /// Calls between plain vm functions (caller, callee), to propagate
    /// `sync_effects`.
    pub fn_calls: Vec<(FnId, FnId)>,
    /// The generated proof type `Vm__Verified` (`Verified<Vm>`).
    pub verified: u32,
    /// The generated resume point type `Vm__Resume` (`Vm.Resume`), if the
    /// program uses resume points (vmexpand's `uses_resume`).
    pub resume: Option<u32>,
    /// `#[require_verified]`: entries start only in verified code (`jump v`
    /// with `v: Verified<Vm>`), unless inside `unsafe { }`.
    pub require_verified: bool,
    /// The first call in vm code (ops, `on raise`, vm functions, cold
    /// blocks, variants) that may collect: (span, callee). Every entry
    /// then has to declare `uses gc` (or `reenter`), so that the effect
    /// is part of its function type.
    pub collects: Option<(Span, String)>,
}

/// An inline cache layout: per field its name, type and first word
/// (counted from the cache's start).
pub struct CacheInfo {
    pub name: String,
    pub fields: Vec<(String, Ty, u32)>,
}

/// A vm function returning `never`. Its body is checked again at each
/// call site that inlines it; in tail-call mode, cold call sites instead
/// `musttail` to one shared copy (`shared`), a function with the handler
/// signature that receives its arguments through thread-local slots.
pub struct NeverFn {
    pub decl: FnDecl,
    pub params: Vec<Ty>,
    /// The cache layout of each parameter declared `cache Name`.
    pub caches: Vec<Option<usize>>,
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
    /// In any superinstruction part: (super op, part index, word offset of the part).
    pub sup: Option<(usize, usize, u32)>,
    pub operands: Vec<(String, usize, LocalId)>,
    pub state: Vec<LocalId>,
    /// vm functions see the state through pointers.
    pub by_ptr: bool,
    /// In the body of a `-> never` vm function: no operands, `become` or `cache`.
    pub in_fn: bool,
    /// In the body of plain vm function `f`: its effectful calls are synced
    /// like a handler's, and their effects recorded in `sync_effects`.
    pub vm_fn: Option<FnId>,
    /// Inside a `cold` block.
    pub cold: bool,
    /// The shared copy of `-> never` vm function `i`: `next` skips the
    /// number of words in local `size`, and continues where local `cont`
    /// says (`TVm::SharedNext`).
    pub shared: Option<(usize, LocalId, LocalId)>,
    /// `-> never` vm functions being inlined here (to catch recursion).
    pub inlining: Vec<usize>,
    /// Parameters declared `cache Name` in scope: (name, handle local,
    /// cache layout). The handle points to the op's first cache word.
    pub cache_params: Vec<(String, LocalId, usize)>,
    /// In an entry (or a `-> never` vm function inlined into one).
    pub entry: bool,
}

impl HandlerCx {
    fn new(vm: u32, state: Vec<LocalId>) -> HandlerCx {
        HandlerCx {
            vm,
            is_handler: true,
            allow_control: true,
            op: None,
            part: None,
            sup: None,
            operands: vec![],
            state,
            by_ptr: false,
            in_fn: false,
            vm_fn: None,
            cold: false,
            shared: None,
            inlining: vec![],
            cache_params: vec![],
            entry: false,
        }
    }
}

pub fn check_value_decl(c: &mut Checker, m: usize, v: &ValueDecl) -> Res<ValueDef> {
    crate::values::check_value_decl(c, m, v)
}

/// Operand fields decode to 64-bit integers, whatever their width: they
/// are mostly indices, and `regs[a + 1]` should be a plain 64-bit add.
fn raw_ty(signed: bool) -> Ty {
    Ty::Int(IntTy { signed, bits: 64 })
}

/// Decode a field of the code word at `pc + off`.
fn decode(word: IntTy, d: FieldDecode, span: Span) -> TExpr {
    let FieldDecode { off, shift, bits, signed } = d;
    let wt = Ty::Int(word);
    let w = mk(wt.clone(), TK::Vm(TVm::Word(off)), span);
    let int = |v: u64| mk(wt.clone(), TK::Int(v as i128), span);
    let wb = word.bits as u32;
    let rt = raw_ty(signed);
    if signed {
        let st = Ty::Int(IntTy { signed: true, bits: word.bits });
        let up = mk(wt.clone(), TK::Binary(BinOp::Shl, Box::new(w), Box::new(int((wb - shift - bits) as u64))), span);
        let as_s = mk(st.clone(), TK::Cast(Box::new(up)), span);
        let sint = mk(st.clone(), TK::Int((wb - bits) as i128), span);
        let down = mk(st.clone(), TK::Binary(BinOp::Shr, Box::new(as_s), Box::new(sint)), span);
        if rt == st { down } else { mk(rt, TK::Cast(Box::new(down)), span) }
    } else {
        let mask = crate::values::low_mask(bits);
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
        sync_effects: 0,
        lazy: false,
        env: None,
        env_val: vec![],
        env_struct: None,
        env_root: None,
        catch_panic: None,
        track_caller: false,
    });
    id
}

/// Lint `super_overlap`: two supers where a proper suffix of one is a
/// proper prefix of the other (`GETGLOBAL; SUBK` and `SUBK; CALL`).
/// `Vm.optimize` can fuse only one of them there: the one that saves
/// more dispatches, the leftmost on a tie (so in `GETGLOBAL; SUBK; CALL`
/// the first super takes the shared `SUBK` and the second never runs
/// there).
/// One warning per super, at the later declared of each pair, naming the
/// earlier ones it overlaps.
fn warn_super_overlaps(c: &mut Checker, d: &VmDecl, l: &crate::vmexpand::VmLayout) {
    let supers: Vec<usize> = (0..l.ops.len()).filter(|&k| !l.ops[k].parts.is_empty() && l.ops[k].fwd.is_empty()).collect();
    let span_of = |k: usize| d.supers.iter().find(|s| s.name.name == l.ops[k].name).map(|s| s.name.span);
    let names = |ps: &[usize]| ps.iter().map(|&p| l.ops[p].name.as_str()).collect::<Vec<_>>().join("; ");
    // The sequence in which `x` then `y` both match, if they overlap that way.
    let overlap = |x: usize, y: usize| -> Option<Vec<usize>> {
        let (px, py) = (&l.ops[x].parts, &l.ops[y].parts);
        let k = (1..px.len().min(py.len())).find(|&k| px[px.len() - k..] == py[..k])?;
        let mut seq = px.clone();
        seq.extend_from_slice(&py[k..]);
        Some(seq)
    };
    for (bi, &b) in supers.iter().enumerate() {
        let Some(sb) = span_of(b) else { continue };
        let mut partners = Vec::new();
        let mut example = None;
        for &a in &supers[..bi] {
            let found = overlap(a, b).map(|s| (a, b, s)).or_else(|| overlap(b, a).map(|s| (b, a, s)));
            if let Some((x, y, seq)) = found {
                partners.push(format!("`{}`", l.ops[a].name));
                example.get_or_insert((x, y, seq, a));
            }
        }
        let Some((x, y, seq, a)) = example else { continue };
        // `optimize` keeps the one that saves more (the leftmost on a tie).
        let (x, y) = if l.ops[y].parts.len() > l.ops[x].parts.len() { (y, x) } else { (x, y) };
        let mut w = Diag::warning(
            "super_overlap",
            sb,
            format!(
                "superinstruction `{}` overlaps {}: in `{}`, `optimize` fuses `{}` and `{}` cannot run there (`optimize` keeps the super that saves more dispatches, the leftmost on a tie)",
                l.ops[b].name,
                partners.join(", "),
                names(&seq),
                l.ops[x].name,
                l.ops[y].name
            ),
        );
        if let Some(sa) = span_of(a) {
            w = w.note(sa, format!("`{}` is declared here", l.ops[a].name));
        }
        c.warnings.push(w);
    }
}

/// The state register a `const` operand kind's body `S[it]` indexes.
fn const_base(body: &Expr, state: &[(String, Ty)]) -> Option<usize> {
    let ExprKind::Index(b, ix) = &body.kind else { return None };
    let (ExprKind::Ident(s), ExprKind::Ident(it)) = (&b.kind, &ix.kind) else { return None };
    if it != "it" {
        return None;
    }
    let i = state.iter().position(|(n, _)| n == s)?;
    matches!(state[i].1, Ty::Ptr(_)).then_some(i)
}

/// What a `const` operand reads must not change once the code is
/// verified: no vm code (handlers, vm functions, cold blocks, `on raise`)
/// may write through the pool's state register, by assignment (`k[i] =`,
/// `*(k + i) =`, the operand itself) or by taking an address in it
/// (`&k[i]`). Plain functions the vm calls, and the host, are trusted not
/// to change the pool of verified code (as with `verify` clauses).
fn check_const_writes(c: &Checker, vi: usize) -> Res<()> {
    let info = &c.vms[vi];
    let Some((kname, si)) = info.kinds.iter().find_map(|k| k.konst.map(|s| (k.name.clone(), s))) else { return Ok(()) };
    let sname = &info.state[si].0;
    // Whether pointer expression `e` is computed from state local `l`.
    fn from(e: &TExpr, l: LocalId) -> bool {
        match &e.kind {
            TK::Local(x) => *x == l,
            TK::Deref(p) => matches!(p.kind, TK::Local(x) if x == l),
            TK::Cast(a) | TK::Unary(_, a) => from(a, l),
            TK::Binary(_, a, b) => from(a, l) || from(b, l),
            TK::Intrinsic(Intr::PtrOffset, a) => from(&a[0], l),
            TK::Block(_, Some(t)) => from(t, l),
            _ => false,
        }
    }
    // Whether place `e` is memory reached through `l`.
    fn through(e: &TExpr, l: LocalId) -> bool {
        match &e.kind {
            TK::Index(b, _) => matches!(b.ty, Ty::Ptr(_)) && from(b, l) || through(b, l),
            TK::Deref(p) => !matches!(p.kind, TK::Local(_)) && from(p, l),
            TK::Field(b, _) => through(b, l),
            _ => false,
        }
    }
    if let Some((span, addr)) = first_state_write(c, vi, si, |_| true, through) {
        let what = if addr { "takes an address through" } else { "assignment writes through" };
        return Err(Diag::new(
            span,
            format!("this {} `{}`, the constant pool of `const` operand kind `{}`: what a const operand reads cannot change after verification", what, sname, kname),
        ));
    }
    Ok(())
}

/// The first write in vm `vi`'s functions that `pick` selects to a place
/// `place(e, l)` holds for, with `l` the function's local of state
/// variable `si`: an assignment to it, or an address taken of it (`true`)
/// other than the state passed to a vm function (checked itself).
fn first_state_write(
    c: &Checker,
    vi: usize,
    si: usize,
    pick: impl Fn(&FnDef) -> bool,
    place: impl Fn(&TExpr, LocalId) -> bool,
) -> Option<(Span, bool)> {
    for f in c.prog.fns.iter() {
        if f.vm != Some(vi as u32) || !pick(f) {
            continue;
        }
        let (Some(body), Some(&l)) = (&f.body, f.vm_state.get(si)) else { continue };
        let mut bad = None;
        let mut passed: Vec<*const TExpr> = Vec::new();
        crate::tirwalk::for_each_expr(body, &mut |e| {
            if bad.is_some() {
                return;
            }
            match &e.kind {
                TK::Call(f, args) if matches!(f.kind, TK::FnRef(g) if c.prog.fns[g as usize].vm_role == VmRole::VmFn) => {
                    if let Some(a) = args.get(si) {
                        passed.push(a as *const TExpr);
                    }
                }
                TK::Assign(lhs, _) if place(lhs, l) => bad = Some((e.span, false)),
                TK::AddrOf(p) if place(p, l) && !passed.contains(&(e as *const TExpr)) => bad = Some((e.span, true)),
                _ => {}
            }
        });
        if bad.is_some() {
            return bad;
        }
    }
    None
}

/// Op handlers and vm functions leave `pc` to the control words: `next`,
/// `redo` and a super part's fallthrough are relative to the instruction
/// (a super's next part is compiled in, and the JIT patches `pc + d` as
/// an offset from the instruction's own word), so a handler that moved
/// `pc` and then said `next` would run the wrong code. `jump` takes any
/// target. Entries and `on raise` may set `pc`.
fn check_pc_writes(c: &Checker, vi: usize) -> Res<()> {
    let pc_idx = c.prog.vms[vi].pc_idx;
    // The state local itself, or (in a vm function, which reaches the
    // state through pointers) what it points to.
    let is_pc = |e: &TExpr, l: LocalId| match &e.kind {
        TK::Local(x) => *x == l,
        TK::Deref(p) => matches!(p.kind, TK::Local(x) if x == l),
        _ => false,
    };
    let pick = |f: &FnDef| matches!(f.vm_role, VmRole::Handler(_) | VmRole::VmFn);
    if let Some((span, addr)) = first_state_write(c, vi, pc_idx, pick, is_pc) {
        let what = if addr { "takes the address of" } else { "assigns" };
        return Err(Diag::new(
            span,
            format!("this {} `pc` in an op or vm function: `next`, `redo` and supers assume it stays at the instruction; use `jump` to go elsewhere", what),
        ));
    }
    Ok(())
}

/// The vm's inline cache layouts. Each cache field starts a code word
/// and takes as many as its type needs (recorded in `l.cache_words`). GC
/// references are not allowed: the collector does not scan bytecode (see
/// language.md §7.1).
fn cache_infos(c: &mut Checker, m: usize, d: &VmDecl, l: &mut crate::vmexpand::VmLayout) -> Res<Vec<CacheInfo>> {
    let mut caches = Vec::new();
    let wb = l.word_bits as u64 / 8;
    for (ci, cd) in d.caches.iter().enumerate() {
        let mut fields = Vec::new();
        let mut off = 0u32;
        for (f, t, weak) in &cd.fields {
            if *weak {
                return Err(Diag::new(
                    t.span,
                    "`weak` cache fields are not supported: inline caches cannot hold GC references (see language.md §7.1)",
                ));
            }
            if fields.iter().any(|(n, _, _): &(String, Ty, u32)| *n == f.name) {
                return Err(Diag::new(f.span, format!("duplicate field `{}` in cache `{}`", f.name, cd.name.name)));
            }
            let ft = c.resolve_type(m, t)?;
            if let Some(why) = bad_cache_type(&ft) {
                return Err(Diag::new(t.span, format!("cache field `{}` has type `{}`: {}", f.name, c.ty_name(&ft), why)));
            }
            let words = crate::layout::size_of(&c.prog, &ft).max(1).div_ceil(wb) as u32;
            fields.push((f.name.clone(), ft, off));
            off += words;
        }
        l.cache_words[ci] = off;
        caches.push(CacheInfo { name: cd.name.name.clone(), fields });
    }
    Ok(caches)
}

/// The vm's operand kinds; `const` kinds must all index the same state
/// register (one constant pool per vm: the JIT is given one per code
/// object).
fn operand_kinds(d: &VmDecl, state: &[(String, Ty)]) -> Res<Vec<OperandKind>> {
    let mut kinds: Vec<OperandKind> = Vec::new();
    for o in &d.operands {
        let konst = match &o.konst {
            None => None,
            Some(_) => Some(const_base(&o.body, state).ok_or_else(|| {
                Diag::new(
                    o.body.span,
                    format!("a `const` operand kind reads `S[it]` for a pointer state register `S` (operand `{}`)", o.name.name),
                )
            })?),
        };
        kinds.push(OperandKind { name: o.name.name.clone(), body: o.body.clone(), konst });
    }
    if let Some((a, b)) = kinds.iter().filter_map(|k| k.konst.map(|s| (k, s))).collect::<Vec<_>>().windows(2).find(|w| w[0].1 != w[1].1).map(|w| (w[0].0.name.clone(), w[1].0.name.clone())) {
        let o = d.operands.iter().find(|o| o.name.name == b).unwrap();
        return Err(Diag::new(o.body.span, format!("`const` operand kinds `{}` and `{}` read different state registers; a vm has at most one constant pool", a, b)));
    }
    Ok(kinds)
}

pub fn check_vm(c: &mut Checker, i: usize) -> Res<()> {
    let (m, d) = c.vm_decls[i].clone();
    let vm = i as u32;
    let mut l = crate::vmexpand::layout(&d)?;
    warn_super_overlaps(c, &d, &l);
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
    let dispatch = match c.dispatch_override {
        Some(x) => x,
        None => match d.attrs.get("dispatch").and_then(|a| a.first()).map(|s| s.as_str()) {
            Some("tailcall") | None => Dispatch::TailCall,
            Some("goto") => Dispatch::Goto,
            Some("switch") => Dispatch::Switch,
            Some(x) => return Err(Diag::new(d.name.span, format!("unknown dispatch mode `{}`", x))),
        },
    };
    // The dispatch table's address is a hidden state variable with
    // variants (`use_variant` switches it), and in tail-call mode on
    // AArch64, where a global's address costs `adrp`+`add` in every
    // handler (lowering.md §3).
    let tbl_idx = if d.variants.is_empty() && !(c.table_in_state && dispatch == Dispatch::TailCall) {
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
    let caches = cache_infos(c, m, &d, &mut l)?;
    let kinds = operand_kinds(&d, &state)?;
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
                Some(cn) => Some(caches.iter().position(|ci: &CacheInfo| ci.name == cn.name).unwrap()),
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
            // A forwarding variant has its super's parts.
            let base = l.ops.iter().position(|o| o.parts == lo.parts).unwrap();
            let sd = &d.supers[base - d.ops.len()];
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
            then: lo.then,
            fwd: lo.fwd.clone(),
            size: l.size(k),
            fid,
            body,
            span,
        });
    }
    // vm functions.
    let mut fns = HashMap::new();
    let mut fn_caches = HashMap::new();
    let mut fn_decls = Vec::new();
    let mut never_fns: Vec<NeverFn> = Vec::new();
    for f in &d.fns {
        let mut pt: Vec<Ty> = state_tys.iter().map(|t| Ty::Ptr(Box::new(t.clone()))).collect();
        // `c: cache Name` is a pointer to the calling op's first cache word.
        let mut pc_caches = Vec::new();
        for p in &f.params {
            if let TypeKind::Cache(cn) = &p.ty.kind {
                let Some(ci) = caches.iter().position(|x: &CacheInfo| x.name == cn.name) else {
                    return Err(Diag::new(cn.span, format!("unknown cache `{}`", cn.name)));
                };
                pt.push(Ty::Ptr(Box::new(Ty::Int(word))));
                pc_caches.push(Some(ci));
            } else {
                if let TypeKind::Block(..) = &p.ty.kind {
                    // Vm functions are inlined into handlers, which may not
                    // create closures; a callback would only be a plain fn.
                    return Err(Diag::new(p.ty.span, "a vm function cannot take a `block fn` parameter: use a plain `fn` type"));
                }
                pt.push(c.resolve_type(m, &p.ty)?);
                pc_caches.push(None);
            }
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
                caches: pc_caches,
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
        let pnames = f.params.iter().map(|p| p.name.name.clone()).collect();
        fn_caches.insert(f.name.name.clone(), (pnames, pc_caches));
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
        // `uses gc`: the vm code it runs may collect, so its callers root
        // their traced locals (checked below: required when vm code makes
        // such a call). `uses reenter`: a nested entry. Calling it runs
        // guest code, so its callers need the effect (in vm code: sync
        // before, reload after). Both are part of the entry's function
        // type, so a conversion to a function type without them is an error.
        let eff = crate::check::effects_of(&e.effects)?;
        if eff != 0 && eff != EFF_SYNC | EFF_GC && eff != EFF_SYNC | EFF_GC | EFF_REENTER {
            let span = e.effects.list[0].2;
            return Err(Diag::new(
                span,
                "an entry declares `uses gc` (its vm code may collect) or `uses reenter` (a nested entry), nothing else",
            ));
        }
        c.prog.fns[fid as usize].effects = eff;
        let gname = Ident { name: format!("{}__{}", d.name.name, e.name.name), span: e.name.span };
        if c.scopes[m].contains_key(&gname.name) || c.values[m].contains_key(&gname.name) {
            return Err(Diag::new(e.name.span, format!("`{}` clashes with a generated vm item", e.name.name)));
        }
        c.define(m, &gname, Def::Fn(fid), d.is_pub)?;
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
                then: o.then.is_some(),
                consts: o.kinds.iter().zip(&o.fields).filter(|(k, _)| kinds[**k].konst.is_some()).map(|(_, f)| (f.1, f.0)).collect(),
                resume: vec![None; o.parts.len()],
            })
            .collect(),
        const_base: kinds.iter().find_map(|k| k.konst),
        entries: entries.iter().map(|(f, _)| *f).collect(),
        on_raise: raise_fid,
        raise_ty: raise_ty.clone(),
        variants: vec![],
        tbl_idx,
        cont_fns: vec![],
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
        fn_caches,
        never_fns,
        sync: d.sync.clone(),
        reload: d.reload.clone(),
        fn_calls: vec![],
        verified: match c.scopes[m].get(&format!("{}__Verified", d.name.name)) {
            Some((Def::Struct(sid), _)) => *sid,
            _ => unreachable!("vmexpand generates Vm__Verified"),
        },
        resume: match c.scopes[m].get(&format!("{}__Resume", d.name.name)) {
            Some((Def::Struct(sid), _)) => Some(*sid),
            _ => None,
        },
        require_verified: d.attrs.has("require_verified"),
        collects: None,
    });

    // Bodies.
    for (fid, f) in fn_decls {
        check_vm_fn(c, vm, fid, &f)?;
    }
    // A vm function calling one whose calls are synced has those effects
    // too (for rooting at its own call sites).
    loop {
        let mut changed = false;
        for &(caller, callee) in &c.vms[i].fn_calls {
            let e = c.prog.fns[callee as usize].sync_effects & !c.prog.fns[caller as usize].effects;
            if e & !c.prog.fns[caller as usize].sync_effects != 0 {
                c.prog.fns[caller as usize].sync_effects |= e;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    for j in 0..c.vms[i].never_fns.len() {
        check_never_fn(c, vm, j)?;
    }
    for k in 0..c.vms[i].ops.len() {
        check_handler(c, vm, k)?;
        // The resume functions its parts asked for (which may ask for
        // more: a resume function's parts call shared copies too).
        while let Some(j) = (0..c.prog.vms[i].ops[k].resume.len())
            .find(|&j| c.prog.vms[i].ops[k].resume[j].is_some_and(|f| c.prog.fns[f as usize].body.is_none()))
        {
            check_resume(c, vm, k, j)?;
        }
    }
    shared_conts(c, i);
    for (fid, e) in entries {
        check_entry(c, vm, fid, &e)?;
    }
    if let (Some(fid), Some(r)) = (raise_fid, &d.on_raise) {
        check_raise(c, vm, fid, r)?;
    }
    check_const_writes(c, i)?;
    check_pc_writes(c, i)?;
    check_variants(c, &d, i, &prefix, cc)?;
    check_collects(c, &d, i)?;
    check_shared_next(c, i)
}

/// Instrumentation variants: a wrapper per wrapped op.
fn check_variants(c: &mut Checker, d: &VmDecl, i: usize, prefix: &str, cc: CallConv) -> Res<()> {
    let vm = i as u32;
    let (state_tys, ret) = (c.prog.vms[i].state_tys.clone(), c.prog.vms[i].ret.clone());
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
    Ok(())
}

/// Vm code that may collect runs inside every entry's call: the entry
/// must declare it, so its callers (also through function pointers) root
/// their traced locals.
fn check_collects(c: &mut Checker, d: &VmDecl, i: usize) -> Res<()> {
    if let Some((site, callee)) = c.vms[i].collects.clone() {
        for (e, fid) in d.entries.iter().zip(c.prog.vms[i].entries.clone()) {
            if c.prog.fns[fid as usize].effects & EFF_GC == 0 {
                let ret = crate::vmexpand::type_src(&e.ret);
                return Err(Diag::new(
                    e.name.span,
                    format!(
                        "entry `{}` runs vm code that may collect, so its callers must root their traced locals: declare it `entry {}(..) -> {} uses gc`",
                        e.name.name, e.name.name, ret
                    ),
                )
                .note(site, format!("{} may collect here", callee)));
            }
        }
    }
    Ok(())
}

/// Store the checked body of vm function `fid`, with its parameters, its
/// locals and the locals that hold the vm state.
fn set_body(c: &mut Checker, fid: FnId, params: Vec<LocalId>, locals: Vec<LocalDef>, body: TExpr, state: Vec<LocalId>) {
    let fd = &mut c.prog.fns[fid as usize];
    fd.params = params;
    fd.locals = locals;
    fd.body = Some(body);
    fd.vm_state = state;
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
    set_body(c, fid, state.clone(), cx.locals, body, state);
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
    let pcaches = nf.caches.clone();
    let shared = nf.shared;
    let mut cx = FnCx::new(m, info.ret.clone());
    cx.fn_name = decl.name.name.clone();
    let state = declare_state(c, vm, &mut cx);
    let span = decl.name.span;
    let arg = |k: usize, t: &Ty| mk(t.clone(), TK::Vm(TVm::SharedArg(shared.unwrap_or(u32::MAX), k as u32)), span);
    let size = cx.temp(Ty::i64());
    cx.locals[size as usize].name = "next.size".into();
    // Where `next` continues: a super's resume function, or null.
    let cont = cx.temp(Ty::Ptr(Box::new(Ty::u8())));
    cx.locals[cont as usize].name = "next.cont".into();
    let mut lets = vec![
        TStmt::Let(size, Some(arg(params.len(), &Ty::i64()))),
        TStmt::Let(cont, Some(mk(Ty::Ptr(Box::new(Ty::u8())), TK::Vm(TVm::SharedArg(shared.unwrap_or(u32::MAX), u32::MAX)), span))),
    ];
    cx.scopes.push(vec![]);
    let mut cache_params = Vec::new();
    for (k, (p, t)) in decl.params.iter().zip(params.iter()).enumerate() {
        let l = declare_param(&mut cx, &p.name.name, t, pcaches[k], &mut cache_params);
        lets.push(TStmt::Let(l, Some(arg(k, t))));
    }
    cx.vm = Some(HandlerCx {
        in_fn: true,
        cold: true,
        shared: Some((j, size, cont)),
        inlining: vec![j],
        cache_params,
        ..HandlerCx::new(vm, state.clone())
    });
    let b = c.block(&mut cx, decl.body.as_ref().unwrap(), Some(&Ty::Never))?;
    if b.ty != Ty::Never {
        return Err(never_falloff(&decl));
    }
    if let Some(fid) = shared {
        let body = mk(Ty::Never, TK::Block(lets, Some(Box::new(b))), span);
        set_body(c, fid, state.clone(), cx.locals, body, state);
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
    let pcaches = nf.caches.clone();
    let decl = nf.decl.clone();
    if args.len() != params.len() {
        return Err(Diag::new(span, format!("`{}` takes {} argument(s), got {}", name, params.len(), args.len())));
    }
    let recursive = h.inlining.contains(&j);
    let share = nf.shared.is_some() && (nf.inline == Some(false) || h.cold || recursive);
    let mut vals = Vec::new();
    for (k, (a, t)) in args.iter().zip(params.iter()).enumerate() {
        vals.push(match pcaches[k] {
            Some(ci) => cache_arg(c, cx, &h, a, ci, &decl.params[k].name.name)?,
            None => c.expr_coerce(cx, a, t)?,
        });
    }
    if share {
        let fid = c.vms[vmi].never_fns[j].shared.unwrap();
        let int = |v: u32| mk(Ty::i64(), TK::Int(v as i128), span);
        // `next` in the shared copy: from a super's part before the last,
        // it continues in the super's next part, as an inlined copy does.
        let cont = match (h.part, h.sup, h.shared) {
            (Some((next_part, _)), Some((sk, _, _)), _) => {
                let r = resume_fn(c, vmi, sk, next_part as usize);
                Some(Box::new(mk(Ty::Ptr(Box::new(Ty::u8())), TK::FnRef(r), span)))
            }
            (_, _, Some((_, _, cl))) => Some(Box::new(mk(Ty::Ptr(Box::new(Ty::u8())), TK::Local(cl), span))),
            _ => None,
        };
        let size = match (h.part, h.op, h.shared) {
            (Some((_, size)), _, _) => int(size),
            (None, Some(op), _) => int(c.vms[vmi].ops[op].size),
            (None, None, Some((caller, l, _))) => {
                c.vms[vmi].never_fns[caller].calls.push(j);
                mk(Ty::i64(), TK::Local(l), span)
            }
            (None, None, None) => {
                if h.entry && c.vms[vmi].require_verified && cx.unsafe_depth == 0 {
                    // The shared copy is checked once, not as part of the
                    // entry, so its jumps are not held to the entry's rule.
                    return Err(Diag::new(
                        span,
                        format!(
                            "`{}` is `#[require_verified]`: an entry cannot reach the shared copy of `{}` (a call from a `cold` block); call it on the hot path",
                            c.vms[vmi].name, name
                        ),
                    ));
                }
                c.vms[vmi].never_fns[j].no_op_calls.push(span);
                int(0)
            }
        };
        return Ok(mk(Ty::Never, TK::Vm(TVm::Shared(fid, vals, Box::new(size), cont)), span));
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
    let mut cache_params = Vec::new();
    for (k, ((p, t), v)) in decl.params.iter().zip(params.iter()).zip(vals).enumerate() {
        let l = declare_param(cx, &p.name.name, t, pcaches[k], &mut cache_params);
        lets.push(TStmt::Let(l, Some(v)));
    }
    let mut h2 = h.clone();
    h2.in_fn = true;
    h2.operands.clear();
    h2.cache_params = cache_params;
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
    let pcaches = info.fn_caches[&f.name.name].1.clone();
    let mut cache_params = Vec::new();
    for (k, (p, t)) in f.params.iter().zip(pts[st.len()..].iter()).enumerate() {
        params.push(declare_param(&mut cx, &p.name.name, t, pcaches[k], &mut cache_params));
    }
    cx.vm = Some(HandlerCx {
        is_handler: false,
        allow_control: false,
        by_ptr: true,
        vm_fn: Some(fid),
        cache_params,
        ..HandlerCx::new(vm, state.clone())
    });
    let b = c.block(&mut cx, f.body.as_ref().unwrap(), Some(&ret))?;
    let b = c.coerce(b, &ret)?;
    set_body(c, fid, params, cx.locals, b, state);
    Ok(())
}

fn check_handler(c: &mut Checker, vm: u32, k: usize) -> Res<()> {
    let fid = c.vms[vm as usize].ops[k].fid;
    check_parts(c, vm, k, 0, fid)
}

/// The resume function of super `k` from part `start` on (`VmRole::Resume`).
fn check_resume(c: &mut Checker, vm: u32, k: usize, start: usize) -> Res<()> {
    let fid = c.prog.vms[vm as usize].ops[k].resume[start].unwrap();
    check_parts(c, vm, k, start, fid)
}

/// Super `k`'s resume function for part `part` (tail-call mode), created
/// on first use; its body is checked after the super's handler.
fn resume_fn(c: &mut Checker, vmi: usize, k: usize, part: usize) -> FnId {
    if let Some(f) = c.prog.vms[vmi].ops[k].resume[part] {
        return f;
    }
    let vd = &c.prog.vms[vmi];
    let (sym, name) = (format!("{}.op.{}.resume{}", vd.symbol, vd.ops[k].name, part), format!("{} (from part {})", vd.ops[k].name, part));
    let (st, ret, span) = (vd.state_tys.clone(), vd.ret.clone(), c.vms[vmi].ops[k].span);
    let f = new_fn(c, sym, name, st, ret, CallConv::PreserveNone, vmi as u32, VmRole::Resume(k as u32, part as u32), span);
    c.prog.vms[vmi].ops[k].resume[part] = Some(f);
    f
}

/// The shared copies that take a continuation (`VmDef::cont_fns`): those
/// a super's part before the last calls, and those called from one that
/// takes it.
fn shared_conts(c: &mut Checker, vi: usize) {
    let nfs = &c.vms[vi].never_fns;
    let idx = |fid: FnId| nfs.iter().position(|n| n.shared == Some(fid));
    let mut cont = vec![false; nfs.len()];
    let mut edges: Vec<(Option<usize>, usize)> = Vec::new();
    for f in c.prog.fns.iter() {
        if f.vm != Some(vi as u32) {
            continue;
        }
        let caller = match f.vm_role {
            VmRole::Shared(i) => Some(i as usize),
            _ => None,
        };
        let Some(body) = &f.body else { continue };
        crate::tirwalk::for_each_expr(body, &mut |e| {
            if let TK::Vm(TVm::Shared(callee, _, _, Some(k))) = &e.kind {
                let Some(j) = idx(*callee) else { return };
                match k.kind {
                    TK::FnRef(_) => cont[j] = true,
                    _ => edges.push((caller, j)),
                }
            }
        });
    }
    loop {
        let mut changed = false;
        for &(a, b) in &edges {
            if a.is_some_and(|a| cont[a]) && !cont[b] {
                cont[b] = true;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    c.prog.vms[vi].cont_fns = cont;
}

/// The handler of op `k` (`start` 0), or the resume function of super
/// `k` from part `start` on: its parts from `start`, inlined in order.
fn check_parts(c: &mut Checker, vm: u32, k: usize, start: usize, fid: FnId) -> Res<()> {
    let info = &c.vms[vm as usize];
    let m = info.module;
    let word = info.word;
    let parts: Vec<usize> = if info.ops[k].parts.is_empty() { vec![k] } else { info.ops[k].parts.clone() };
    let fwd = info.ops[k].fwd.clone();
    let ret = info.ret.clone();
    let mut cx = FnCx::new(m, ret);
    let state = declare_state(c, vm, &mut cx);
    let mut exprs = Vec::new();
    let n = parts.len();
    let mut off = 0;
    // The previous part's raw field locals, for forwarding into this part.
    let mut prev_raws: Vec<LocalId> = Vec::new();
    for (j, &p) in parts.iter().enumerate() {
        let op = &c.vms[vm as usize].ops[p];
        let size = op.size;
        if j < start {
            off += size;
            continue;
        }
        let span = op.span;
        let body = op.body.clone().unwrap();
        let name = op.name.clone();
        let fields = op.fields.clone();
        let params = op.params.clone();
        let kinds = op.kinds.clone();
        cx.scopes.push(vec![]);
        let mut lets = Vec::new();
        let mut operands = Vec::new();
        for (pi, pname) in params.iter().enumerate() {
            let (bits, shift, signed) = fields[pi];
            let raw = cx.temp(raw_ty(signed));
            cx.locals[raw as usize].name = format!("{}.raw", pname);
            cx.locals[raw as usize].mutable = false;
            // A forwarded operand is a copy of the previous part's field.
            let fd = FieldDecode { off: 0, shift, bits, signed };
            cx.locals[raw as usize].field = (!fwd.iter().any(|l| l.0 == j && l.1 == pi && j > start)).then_some(fd);
            // A forwarding variant (`#[forward] super`): an operand linked
            // to the register the previous part wrote is a copy of that
            // part's field (`optimize` checked that they are equal), so
            // LLVM sees one address and forwards the stored value instead
            // of reloading it through memory, which takes a store-to-load
            // round trip off a dependency chain. A copy, not the previous
            // part's local: cold blocks decode fields again from the
            // current word, and a copy is passed to them instead.
            // A resume function decodes its first part's fields from the
            // part's own word (the variant's links say they are equal).
            let init = match fwd.iter().find(|l| l.0 == j && l.1 == pi && j > start) {
                Some(&(_, _, d)) => {
                    let (dl, rt) = (prev_raws[d], raw_ty(signed));
                    let dt = cx.locals[dl as usize].ty.clone();
                    let v = mk(dt.clone(), TK::Local(dl), span);
                    if dt == rt { v } else { mk(rt, TK::Cast(Box::new(v)), span) }
                }
                None => decode(word, fd, span),
            };
            lets.push(TStmt::Let(raw, Some(init)));
            operands.push((pname.clone(), kinds[pi], raw));
        }
        cx.vm = Some(HandlerCx {
            op: Some(p),
            part: if j + 1 < n { Some((j as u32 + 1, size)) } else { None },
            sup: if n > 1 { Some((k, j, off)) } else { None },
            operands,
            ..HandlerCx::new(vm, state.clone())
        });
        off += size;
        let raws: Vec<LocalId> = cx.vm.as_ref().unwrap().operands.iter().map(|o| o.2).collect();
        let r = c.block(&mut cx, &body, Some(&Ty::Never));
        let b = r?;
        if b.ty != Ty::Never {
            cx.scopes.pop();
            return Err(Diag::new(
                body.span,
                format!("handler `{}` can reach its end; every path must end in `next`, `jump`, `exit`, `raise` or `redo`", name),
            ));
        }
        cx.scopes.pop();
        prev_raws = raws;
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
    set_body(c, fid, state.clone(), cx.locals, body, state);
    Ok(())
}

/// Whether `b` assigns to the name `x` (`x = ..` or `x op= ..`), for
/// finding the operands an op writes.
pub(crate) fn block_assigns(b: &Block, x: &str) -> bool {
    let hit = |e: &Expr| match &e.kind {
        ExprKind::Assign(l, _) | ExprKind::CompoundAssign(_, l, _) => matches!(&l.kind, ExprKind::Ident(n) if n == x),
        _ => false,
    };
    AstAny { hit: &hit, vm: true, closures: false }.block(b)
}

/// A search over an AST block: is there an expression `hit` holds for?
/// `vm`: look into vm expressions (`jump`, `exit`, `raise`, `fail`,
/// `cold`); `closures`: look into closure bodies. Leaves (literals,
/// `continue`, ..) and what is not looked into answer `false`.
struct AstAny<'a> {
    hit: &'a dyn Fn(&Expr) -> bool,
    vm: bool,
    closures: bool,
}

impl AstAny<'_> {
    fn block(&self, b: &Block) -> bool {
        b.stmts.iter().any(|s| match s {
            Stmt::Let { init, els, .. } => init.as_ref().is_some_and(|e| self.expr(e)) || els.as_ref().is_some_and(|b| self.block(b)),
            Stmt::Expr { expr, .. } | Stmt::Defer(expr) => self.expr(expr),
        })
    }

    fn expr(&self, e: &Expr) -> bool {
        if (self.hit)(e) {
            return true;
        }
        let go = |e: &Expr| self.expr(e);
        let conds = |cs: &Vec<Cond>| cs.iter().any(|c| match c {
            Cond::Expr(e) | Cond::Let(_, e) => self.expr(e),
        });
        match &e.kind {
            ExprKind::Int(_)
            | ExprKind::Float(_)
            | ExprKind::Str(_)
            | ExprKind::Char(_)
            | ExprKind::Bool(_)
            | ExprKind::Ident(_)
            | ExprKind::DotVariant(_)
            | ExprKind::Unit
            | ExprKind::Continue
            | ExprKind::SizeOf(_) => false,
            ExprKind::Assign(l, r) | ExprKind::CompoundAssign(_, l, r) => go(l) || go(r),
            ExprKind::Field(a, _) | ExprKind::Unary(_, a) | ExprKind::Cast(a, _) | ExprKind::Try(a) | ExprKind::TypeArgs(a, _) => go(a),
            ExprKind::Call(f, args) => go(f) || args.iter().any(go),
            ExprKind::Index(a, b) | ExprKind::Binary(_, a, b) | ExprKind::Coalesce(a, b) | ExprKind::ArrayRepeat(a, b) => go(a) || go(b),
            ExprKind::SliceRange(a, lo, hi) => go(a) || lo.as_ref().is_some_and(|e| go(e)) || hi.as_ref().is_some_and(|e| go(e)),
            ExprKind::StructLit(_, fs, _) => fs.iter().any(|(_, e)| go(e)),
            ExprKind::Tuple(es) | ExprKind::ArrayLit(es) => es.iter().any(go),
            ExprKind::Block(b) | ExprKind::Unsafe(b) | ExprKind::Loop(b) => self.block(b),
            ExprKind::If(cs, t, f) => conds(cs) || self.block(t) || f.as_ref().is_some_and(|e| go(e)),
            ExprKind::While(cs, b) => conds(cs) || self.block(b),
            ExprKind::For(_, _, lo, hi, b) => go(lo) || hi.as_ref().is_some_and(|e| go(e)) || self.block(b),
            ExprKind::Match(s, arms) => go(s) || arms.iter().any(|a| go(&a.body) || a.guard.as_ref().is_some_and(|g| go(g))),
            ExprKind::Return(v) | ExprKind::Break(v) => v.as_ref().is_some_and(|e| go(e)),
            ExprKind::Closure(_, _, body) => self.closures && go(body),
            ExprKind::Vm(v) => {
                self.vm
                    && match v {
                        VmExpr::Jump(a) | VmExpr::Exit(a) | VmExpr::Raise(a) | VmExpr::Fail(a) => go(a),
                        VmExpr::Cold(b) => self.block(b),
                        VmExpr::Next | VmExpr::Redo | VmExpr::Become(_) | VmExpr::UseVariant(_) => false,
                    }
            }
        }
    }
}

fn check_entry(c: &mut Checker, vm: u32, fid: FnId, e: &EntryDecl) -> Res<()> {
    let info = &c.vms[vm as usize];
    let m = info.module;
    let ret = info.ret.clone();
    let mut cx = FnCx::new(m, ret.clone());
    cx.effects = c.prog.fns[fid as usize].effects;
    cx.fn_name = e.name.name.clone();
    let mut params = Vec::new();
    let pts = c.prog.fns[fid as usize].param_tys.clone();
    for (p, t) in e.params.iter().zip(pts.iter()) {
        params.push(cx.declare(&p.name.name, t.clone(), true));
    }
    let state = declare_state(c, vm, &mut cx);
    cx.vm = Some(HandlerCx { is_handler: false, entry: true, ..HandlerCx::new(vm, state.clone()) });
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
    set_body(c, fid, params, cx.locals, body, state);
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
    set_body(c, fid, state.clone(), cx.locals, body, state);
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
            let (j, size, cont) = h.shared.unwrap();
            c.vms[vmi].never_fns[j].uses_next = true;
            let n = mk(Ty::i64(), TK::Local(size), span);
            let target = mk(pc_ty.clone(), TK::Intrinsic(Intr::PtrOffset, vec![pc(), n]), span);
            never(TVm::SharedNext(Box::new(target), j as u32, cont))
        }
        VmExpr::Next => {
            let op = need_op("next")?;
            if let Some((next_part, size)) = h.part {
                // From a cold block, which tail-call mode outlines: the
                // outlined function continues in the super's resume
                // function for the next part.
                if h.cold && c.prog.vms[vmi].dispatch == Dispatch::TailCall {
                    resume_fn(c, vmi, h.sup.unwrap().0, next_part as usize);
                }
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
            // `jump v` with `v: Verified<Vm>` starts at the verified code.
            let te = c.expr(cx, t, Some(&pc_ty))?;
            let vsid = c.vms[vmi].verified;
            if te.ty == Ty::Struct(vsid) {
                let sp = te.span;
                let wt = Ty::Int(c.vms[vmi].word);
                // `jump p.code` with `code: verified [W] by Vm`: the
                // trailing array's address, computed from `p`.
                let code = match te.kind {
                    TK::Struct(mut fs) if fs.len() == 1 => fs.pop().unwrap(),
                    kind => mk(Ty::Slice(Box::new(wt.clone())), TK::Field(Box::new(TExpr { kind, ..te }), 0), sp),
                };
                let start = mk(Ty::Ptr(Box::new(wt)), TK::Field(Box::new(code), 0), sp);
                return never(TVm::Jump(Box::new(c.coerce(start, &pc_ty)?)));
            }
            // `jump r` with `r: Vm.Resume`: an instruction boundary of
            // verified code, saved by vm code (`next_resume`).
            if c.vms[vmi].resume.is_some_and(|r| te.ty == Ty::Struct(r)) {
                let sp = te.span;
                let wt = Ty::Int(c.vms[vmi].word);
                let at = mk(Ty::Ptr(Box::new(wt)), TK::Field(Box::new(te), 0), sp);
                return never(TVm::Jump(Box::new(c.coerce(at, &pc_ty)?)));
            }
            if h.entry && c.vms[vmi].require_verified && cx.unsafe_depth == 0 {
                let vname = c.vms[vmi].name.clone();
                return Err(Diag::new(
                    t.span,
                    format!(
                        "`{vname}` is `#[require_verified]`: an entry starts only in verified code, `jump v` with `v: Verified<{vname}>` (from `{vname}.verify`) or `v: {vname}.Resume` (from `next_resume`); found `{}`",
                        c.ty_name(&te.ty)
                    ),
                ));
            }
            let t = c.coerce(te, &pc_ty)?;
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
            let Some((sk, j, off)) = h.sup else {
                return Ok(mk(Ty::Unit, TK::Vm(TVm::Become(b.opcode, 0)), span));
            };
            // In a superinstruction: the part's own word becomes the new
            // op (for jumps into the middle), and the head word becomes the
            // super with this part replaced, if one is declared, or else
            // the sequence's first op: the super is split up. A forwarding
            // variant becomes the same variant of that super, if it has
            // one (`become` keeps the fields, so the links still hold).
            let mut parts = info.ops[sk].parts.clone();
            parts[j] = t;
            let fwd = &info.ops[sk].fwd;
            let head = match info.ops.iter().find(|o| o.parts == parts && o.fwd == *fwd).or_else(|| info.ops.iter().find(|o| o.parts == parts)) {
                Some(o) => o.opcode,
                None => info.ops[parts[0]].opcode,
            };
            let head_e = mk(Ty::Unit, TK::Vm(TVm::Become(head, off)), span);
            if j == 0 {
                return Ok(head_e);
            }
            let own = mk(Ty::Unit, TK::Vm(TVm::Become(b.opcode, 0)), span);
            Ok(mk(Ty::Unit, TK::Block(vec![TStmt::Expr(own), TStmt::Expr(head_e)], None), span))
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
            let (false, Some(ti)) = (c.vm_decls[vmi].1.variants.is_empty(), vd.tbl_idx) else {
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

/// Why `t` cannot be an inline cache field, if it cannot.
fn bad_cache_type(t: &Ty) -> Option<&'static str> {
    match t {
        Ty::Int(_) | Ty::F32 | Ty::F64 | Ty::Bool | Ty::Ptr(_) | Ty::Fn(..) => None,
        Ty::Option(inner) if matches!(**inner, Ty::Ptr(_) | Ty::Fn(..)) => None,
        Ty::Gc(_) | Ty::Value(_) | Ty::Option(_) => Some(
            "inline caches cannot hold GC references, since the collector does not scan bytecode; cache a pointer to a non-moving structure, an id or a version number, and validate it on use",
        ),
        _ => Some("cache fields must be integers, floats, `bool`, pointers or function pointers"),
    }
}

pub fn vm_ident(c: &mut Checker, cx: &mut FnCx, name: &str, span: Span) -> Res<Option<TExpr>> {
    let Some(h) = cx.vm.clone() else { return Ok(None) };
    if h.cache_params.iter().any(|(x, _, _)| x == name) {
        return Err(Diag::new(
            span,
            format!("`{}` is a cache handle: use its fields (`{}.field`) or pass it to a vm function's `cache` parameter", name, name),
        ));
    }
    if name == "cache" && h.op.is_some() && !h.in_fn {
        return Err(Diag::new(span, "`cache` on its own can only be passed to a vm function parameter declared `cache Name`"));
    }
    if name == "next_pc" {
        return next_pc(c, &h, span).map(Some);
    }
    if name == "next_resume" {
        // `next_pc` as a resume point: the address of the next instruction
        // of verified code (the verifier rejects code whose last
        // instruction reads it, as it rejects one that uses `next`).
        let at = next_pc(c, &h, span).map_err(|_| {
            Diag::new(span, "`next_resume` (where `next` continues, as a resume point) is only available where `next` is: op handlers and the `-> never` vm functions they call")
        })?;
        let rsid = c.vms[h.vm as usize].resume.expect("vmexpand generates Vm__Resume where `next_resume` appears");
        return Ok(Some(mk(Ty::Struct(rsid), TK::Struct(vec![at]), span)));
    }
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

/// `next_pc`: the address `next` would continue at, the calling op's
/// `pc` plus its size (a superinstruction part's own size), for saving a
/// return address. Where `next` is allowed: op handlers and the `-> never`
/// vm functions they call (a shared copy gets the size from its caller).
fn next_pc(c: &mut Checker, h: &HandlerCx, span: Span) -> Res<TExpr> {
    let vmi = h.vm as usize;
    let pc_ty = c.vms[vmi].state[c.vms[vmi].pc_idx].1.clone();
    let pc_local = h.state[c.vms[vmi].pc_idx];
    let mut pc = mk(pc_ty.clone(), TK::Local(pc_local), span);
    if h.by_ptr {
        pc = mk(pc_ty.clone(), TK::Deref(Box::new(mk(Ty::Ptr(Box::new(pc_ty.clone())), TK::Local(pc_local), span))), span);
    }
    let n = if let (None, Some((j, size, _))) = (h.op, h.shared) {
        c.vms[vmi].never_fns[j].uses_next = true;
        mk(Ty::i64(), TK::Local(size), span)
    } else if let (Some(op), true) = (h.op, h.allow_control) {
        let size = match h.part {
            Some((_, size)) => size,
            None => c.vms[vmi].ops[op].size,
        };
        mk(Ty::i64(), TK::Int(size as i128), span)
    } else {
        return Err(Diag::new(
            span,
            "`next_pc` (the address `next` continues at) is only available where `next` is: op handlers and the `-> never` vm functions they call",
        ));
    };
    Ok(mk(pc_ty, TK::Intrinsic(Intr::PtrOffset, vec![pc, n]), span))
}

pub fn vm_field(c: &mut Checker, cx: &mut FnCx, base: &Expr, name: &Ident, span: Span) -> Res<Option<TExpr>> {
    let Some(h) = cx.vm.clone() else { return Ok(None) };
    let ExprKind::Ident(bn) = &base.kind else { return Ok(None) };
    if cx.lookup(bn).is_some() {
        return Ok(None);
    }
    if let Some((_, l, ci)) = h.cache_params.iter().find(|(x, _, _)| x == bn) {
        let info = &c.vms[h.vm as usize];
        let Some((_, ft, off)) = info.caches[*ci].fields.iter().find(|f| f.0 == name.name) else {
            return Err(Diag::new(name.span, format!("no field `{}` in cache `{}`", name.name, info.caches[*ci].name)));
        };
        let wb = (info.word.bits as u32 / 8).max(1);
        let ht = cx.locals[*l as usize].ty.clone();
        let hv = mk(ht, TK::Local(*l), base.span);
        return Ok(Some(mk(ft.clone(), TK::Vm(TVm::WordAt(Box::new(hv), off * wb, wb)), span)));
    }
    if name.name == "raw" {
        if let Some((_, _, raw)) = h.operands.iter().find(|(n, _, _)| n == bn) {
            let t = cx.locals[*raw as usize].ty.clone();
            return Ok(Some(mk(t, TK::Local(*raw), span)));
        }
    }
    if bn == "then" {
        return then_operand(c, cx, &h, name, span).map(Some);
    }
    if bn == "cache" {
        if h.in_fn {
            return Err(Diag::new(base.span, "`cache` is not available in vm functions; declare a parameter `c: cache Name`, pass `cache` to it and use `c.field`"));
        }
        let info = &c.vms[h.vm as usize];
        let Some(op) = h.op else {
            return Err(Diag::new(base.span, "`cache` is only available in op handlers; a vm function can declare a parameter `c: cache Name`, take `cache` from the op and use `c.field`"));
        };
        let Some(ci) = info.ops[op].cache else {
            return Err(Diag::new(base.span, format!("op `{}` has no inline cache", info.ops[op].name)));
        };
        let Some((_, ft, off)) = info.caches[ci].fields.iter().find(|f| f.0 == name.name) else {
            return Err(Diag::new(name.span, format!("no field `{}` in cache `{}`", name.name, info.caches[ci].name)));
        };
        return Ok(Some(mk(ft.clone(), TK::Vm(TVm::Word(1 + off)), span)));
    }
    Ok(None)
}

/// `then.x`: operand `x` of the instruction after this one (declared with
/// `then OP`), decoded from its word with OP's format and evaluated by its
/// kind, with `pc` at that instruction.
fn then_operand(c: &mut Checker, cx: &mut FnCx, h: &HandlerCx, name: &Ident, span: Span) -> Res<TExpr> {
    let info = &c.vms[h.vm as usize];
    let op = match h.op {
        Some(op) if !h.in_fn => op,
        _ => return Err(Diag::new(span, "`then` is only available in op handlers")),
    };
    let Some(t) = info.ops[op].then else {
        return Err(Diag::new(span, format!("op `{}` does not declare `then OP`", info.ops[op].name)));
    };
    let to = &info.ops[t];
    let Some(pi) = to.params.iter().position(|p| *p == name.name) else {
        return Err(Diag::new(name.span, format!("op `{}` has no operand `{}`", to.name, name.name)));
    };
    let off = info.ops[op].size;
    let (bits, shift, signed) = to.fields[pi];
    let body = info.kinds[to.kinds[pi]].body.clone();
    let word = info.word;
    let pc_local = h.state[info.pc_idx];
    let pc_ty = info.state[info.pc_idx].1.clone();
    let raw = cx.temp(raw_ty(signed));
    cx.locals[raw as usize].name = format!("then.{}.raw", name.name);
    let fd = FieldDecode { off, shift, bits, signed };
    cx.locals[raw as usize].field = Some(fd);
    let pc2 = cx.temp(pc_ty.clone());
    cx.locals[pc2 as usize].name = "then.pc".into();
    let n = mk(Ty::i64(), TK::Int(off as i128), span);
    let pc_v = mk(pc_ty.clone(), TK::Local(pc_local), span);
    let pc_init = mk(pc_ty, TK::Intrinsic(Intr::PtrOffset, vec![pc_v, n]), span);
    cx.scopes.push(vec![("it".to_string(), raw), ("pc".to_string(), pc2)]);
    let r = c.expr(cx, &body, None);
    cx.scopes.pop();
    let v = r?;
    let lets = vec![TStmt::Let(raw, Some(decode(word, fd, span))), TStmt::Let(pc2, Some(pc_init))];
    Ok(mk(v.ty.clone(), TK::Block(lets, Some(Box::new(v))), span))
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
    if let Some(caller) = h.vm_fn {
        c.vms[h.vm as usize].fn_calls.push((caller, fid));
    }
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
    let (pnames, pcaches) = c.vms[h.vm as usize].fn_caches[name].clone();
    if pcaches.iter().all(|x| x.is_none()) {
        return c.finish_call(cx, f, pre, args, span).map(Some);
    }
    // Cache handles are not values: check those arguments here.
    let Ty::Fn(ptys, ret, _) = f.ty.clone() else { unreachable!() };
    if args.len() != pcaches.len() {
        return Err(Diag::new(span, format!("expected {} argument(s), got {}", pcaches.len(), args.len())));
    }
    for (k, a) in args.iter().enumerate() {
        let v = match pcaches[k] {
            Some(ci) => cache_arg(c, cx, &h, a, ci, &pnames[k])?,
            None => c.expr_coerce(cx, a, &ptys[pre.len()])?,
        };
        pre.push(v);
    }
    let call = mk(*ret, TK::Call(Box::new(f), pre), span);
    wrap_effects(c, cx, call).map(Some)
}

/// Declare parameter `name` of a vm function. A `cache Name` handle is a
/// hidden local, reachable only as `name.field` (see `vm_field`).
fn declare_param(cx: &mut FnCx, name: &str, t: &Ty, cache: Option<usize>, out: &mut Vec<(String, LocalId, usize)>) -> LocalId {
    match cache {
        Some(ci) => {
            let l = cx.temp(t.clone());
            cx.locals[l as usize].name = format!("cache.{}", name);
            cx.locals[l as usize].mutable = false;
            out.push((name.to_string(), l, ci));
            l
        }
        None => cx.declare(name, t.clone(), true),
    }
}

/// The argument for parameter `pname: cache Name` (layout `ci`): `cache`
/// in an op with that cache (a pointer to its first cache word), or a
/// cache handle of the caller with the same layout.
fn cache_arg(c: &mut Checker, cx: &mut FnCx, h: &HandlerCx, a: &Expr, ci: usize, pname: &str) -> Res<TExpr> {
    let info = &c.vms[h.vm as usize];
    let cname = info.caches[ci].name.clone();
    let bad = |what: String| {
        Diag::new(
            a.span,
            format!("`{}: cache {}` takes `cache` (in an op with cache `{}`) or a cache handle; {}", pname, cname, cname, what),
        )
    };
    let ExprKind::Ident(n) = &a.kind else { return Err(bad("found an expression".into())) };
    if cx.lookup(n).is_some() {
        return Err(bad(format!("`{}` is a local", n)));
    }
    if let Some((_, l, hci)) = h.cache_params.iter().find(|(x, _, _)| x == n) {
        if *hci != ci {
            return Err(bad(format!("`{}` is a handle to cache `{}`", n, info.caches[*hci].name)));
        }
        let t = cx.locals[*l as usize].ty.clone();
        return Ok(mk(t, TK::Local(*l), a.span));
    }
    if n != "cache" {
        return Err(bad(format!("found `{}`", n)));
    }
    let op = match h.op {
        Some(op) if !h.in_fn => op,
        _ => return Err(bad("`cache` is only available in op handlers".into())),
    };
    if info.ops[op].cache != Some(ci) {
        let has = match info.ops[op].cache {
            Some(k) => format!("op `{}` has cache `{}`", info.ops[op].name, info.caches[k].name),
            None => format!("op `{}` has no inline cache", info.ops[op].name),
        };
        return Err(bad(has));
    }
    let pt = info.state[info.pc_idx].1.clone();
    let pc = mk(pt.clone(), TK::Local(h.state[info.pc_idx]), a.span);
    let one = mk(Ty::i64(), TK::Int(1), a.span);
    Ok(mk(pt, TK::Intrinsic(Intr::PtrOffset, vec![pc, one]), a.span))
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
    let (Ty::Fn(_, _, e) | Ty::Block(_, _, e) | Ty::Closure(_, _, e)) = crate::check::callee_sig(&f.ty) else { unreachable!() };
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
        if h.entry {
            // Entries run before dispatch starts: nothing to sync yet. A
            // call that may collect is the entry's own effect, which it
            // declares (`uses gc`), as any function would.
            if missing & EFF_GC == 0 {
                return Ok(call);
            }
            let caller = format!("entry `{}`", cx.fn_name);
            return Err(Diag::new(
                span,
                format!("{} may collect, but {} does not declare it; declare `uses gc` after its result type", callee, caller),
            ));
        }
        // Vm code may collect while a run is active: callers of the vm's
        // entries root their traced locals across the call, as for a call
        // to a `uses gc` function. The entries must declare it (checked
        // once the whole vm is checked).
        if missing & EFF_GC != 0 && c.vms[h.vm as usize].collects.is_none() {
            c.vms[h.vm as usize].collects = Some((span, callee.clone()));
        }
        // Lint `hot_gc` (opt-in, `-W hot_gc`): a call that may collect on
        // an op's fast path (outside `cold`) syncs the state and roots the
        // live traced locals on every execution.
        if missing & EFF_GC != 0 && h.is_handler && h.op.is_some() && !h.cold {
            c.warnings.push(Diag::warning(
                "hot_gc",
                span,
                format!(
                    "{} may collect, on the fast path of this handler: every execution syncs the vm state and roots the live traced locals around it; if the call is rare, move it into a `cold {{ }}` block",
                    callee
                ),
            ));
        }
        // A plain vm function is inlined into vm code, and sees the
        // caller's state through pointers: its effectful calls are synced
        // in place, as in an op, so that only the path that makes the call
        // pays. Its callers root their traced locals (`sync_effects`).
        let in_vm_fn = h.vm_fn.is_some() && h.by_ptr;
        if (h.allow_control && h.is_handler) || in_vm_fn {
            if let Some(fid) = h.vm_fn {
                c.prog.fns[fid as usize].sync_effects |= missing;
            }
            let sync = match c.vms[h.vm as usize].sync.clone() {
                Some(b) => c.block(cx, &b, Some(&Ty::Unit))?,
                None => mk(Ty::Unit, TK::Unit, span),
            };
            // Guest code may run in the call: the stack may move and frames
            // change, so the state registers must be reloaded after it.
            let reload = if missing & EFF_REENTER != 0 {
                let Some(b) = c.vms[h.vm as usize].reload.clone() else {
                    return Err(Diag::new(
                        span,
                        format!(
                            "{} uses reenter: guest code may run during the call and move the stack or change frames, so vm `{}` needs a `reload {{ .. }}` block that reloads its state after the call",
                            callee, c.vms[h.vm as usize].name
                        ),
                    ));
                };
                c.block(cx, &b, Some(&Ty::Unit))?
            } else {
                mk(Ty::Unit, TK::Unit, span)
            };
            let t = call.ty.clone();
            return Ok(mk(t, TK::Vm(TVm::Synced(Box::new(sync), Box::new(call), Box::new(reload))), span));
        }
        if h.allow_control {
            // Entries run before dispatch starts: nothing to sync yet.
            return Ok(call);
        }
    }
    let eff = effect_name(missing);
    if let Some(t) = &cx.closure_ty {
        return Err(Diag::new(
            span,
            format!("{} uses {}, but this closure's type `{}` does not allow it", callee, eff, c.ty_name(t)),
        ));
    }
    let caller = if cx.fn_name.is_empty() { "this function".to_string() } else { format!("`{}`", cx.fn_name) };
    Err(Diag::new(
        span,
        format!("{} uses {}, but {} does not declare it; add `uses {}` to its signature", callee, eff, caller, eff),
    ))
}
