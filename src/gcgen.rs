//! Code generation for GC integration: object allocation, trailing arrays,
//! tracing (generated per type, including pointers inside values), and
//! shadow-stack frames that make traced locals visible at calls that may
//! collect.

use std::fmt::Write;

use crate::codegen::{llty, Codegen, FnGen};
use crate::layout;
use crate::tir::*;

/// Does a value of this type hold GC references?
pub fn is_traced(p: &Program, t: &Ty) -> bool {
    match t {
        Ty::Gc(_) => true,
        Ty::Option(inner) | Ty::Array(inner, _) => is_traced(p, inner),
        Ty::Result(a, b) => is_traced(p, a) || is_traced(p, b),
        Ty::Tuple(ts) => ts.iter().any(|t| is_traced(p, t)),
        Ty::Enum(i) => p.enums[*i as usize].variants.iter().any(|v| v.fields.iter().any(|t| is_traced(p, t))),
        Ty::Struct(i) => {
            let s = &p.structs[*i as usize];
            s.object.is_none() && s.fields.iter().any(|(_, t)| is_traced(p, t))
        }
        Ty::Value(i) => p.values[*i as usize].variants.iter().any(|v| matches!(v.payload, Some(Ty::Gc(_)))),
        _ => false,
    }
}

pub fn gc_enabled(p: &Program) -> bool {
    p.fns.iter().any(|f| f.symbol == "std.gc.mark_ptr" && f.body.is_some())
}

/// `--gc-check-roots`: every reference that generated code marks (roots
/// in shadow-stack frames, state copies, `gc_mark` in scanners, fields
/// traced by `loimi_trace_object`) goes through `std.gc.rootck_mark`,
/// which validates it before `mark_ptr` reads its header. What is being
/// marked is kept in two thread-locals for the report: the info record
/// (`rootck_info`) of the frame, object type or `gc_mark` call, and an
/// index (the root's slot in its frame, or the object's address).
pub fn check_roots(g: &FnGen) -> bool {
    g.cg.opts.gc_check_roots
}

fn mark_call(g: &mut FnGen, h: &str, v: &str) {
    let f = if check_roots(g) { "std.gc.rootck_mark" } else { "std.gc.mark_ptr" };
    g.emit(format!("call void @\"{}\"(ptr {}, ptr {})", f, h, v));
}

/// Kinds of `rootck_info` records (`std.gc.rootck_fail` reads them).
pub const RC_FRAME: u32 = 0;
pub const RC_OBJECT: u32 = 1;
pub const RC_GC_MARK: u32 = 2;

/// A constant `{ str text, i64 kind }` describing what is being marked.
pub fn rootck_info(cg: &mut Codegen, kind: u32, text: &str) -> String {
    let s = cg.string_const(text.as_bytes());
    let name = format!("@\"loimi.rootck.{}\"", cg.rootck_count);
    cg.rootck_count += 1;
    let _ = writeln!(cg.globals, "{} = private unnamed_addr constant {{ ptr, i64, i64 }} {{ ptr {}, i64 {}, i64 {} }}", name, s, text.len(), kind);
    name
}

/// Set the current `rootck_info` record and index.
pub fn rootck_set(g: &mut FnGen, info: &str, idx: &str) {
    g.emit(format!("store ptr {}, ptr @loimi.rootck.what", info));
    g.emit(format!("store i64 {}, ptr @loimi.rootck.idx", idx));
}

/// A readable type name for the reports of `--gc-check-roots`.
pub fn ty_str(p: &Program, t: &Ty) -> String {
    match t {
        Ty::Gc(i) => format!("Gc<{}>", ty_str(p, i)),
        Ty::Option(i) => format!("?{}", ty_str(p, i)),
        Ty::Struct(i) => p.structs[*i as usize].name.clone(),
        Ty::Enum(i) => p.enums[*i as usize].name.clone(),
        Ty::Value(i) => p.values[*i as usize].name.clone(),
        Ty::Tuple(ts) => format!("({})", ts.iter().map(|t| ty_str(p, t)).collect::<Vec<_>>().join(", ")),
        Ty::Array(e, n) => format!("[{}; {}]", ty_str(p, e), n),
        Ty::Result(a, b) => format!("Result<{}, {}>", ty_str(p, a), ty_str(p, b)),
        _ => format!("{:?}", t),
    }
}

/// Emit code marking every GC reference in `v` (a value of type `t`).
pub fn mark_expr(g: &mut FnGen, t: &Ty, v: &str, h: &str) {
    let p = g.p();
    match t {
        Ty::Gc(_) => {
            mark_call(g, h, v);
        }
        Ty::Option(inner) if inner.is_nullable_ptr() => {
            if is_traced(p, inner) {
                mark_call(g, h, v);
            }
        }
        Ty::Option(inner) => {
            if !is_traced(p, inner) {
                return;
            }
            let (some, payload) = g.option_parts(t, v);
            let l_some = g.fresh_label("mksome");
            let l_end = g.fresh_label("mkend");
            g.cond_br(&some, &l_some, &l_end);
            g.label(&l_some);
            mark_expr(g, inner, &payload, h);
            g.br(&l_end);
            g.label(&l_end);
        }
        Ty::Tuple(ts) => {
            let lt = g.ty(t);
            for (i, et) in ts.iter().enumerate() {
                if is_traced(p, et) {
                    let x = g.inst(format!("extractvalue {} {}, {}", lt, v, i));
                    mark_expr(g, et, &x, h);
                }
            }
        }
        Ty::Struct(sid) => {
            let lt = g.ty(t);
            let fields = p.structs[*sid as usize].fields.clone();
            for (i, (_, ft)) in fields.iter().enumerate() {
                if is_traced(p, ft) {
                    let x = g.inst(format!("extractvalue {} {}, {}", lt, v, i));
                    mark_expr(g, ft, &x, h);
                }
            }
        }
        Ty::Result(a, b) => {
            let lt = g.ty(t);
            let tag = g.inst(format!("extractvalue {} {}, 0", lt, v));
            for (pos, ft) in [(1, a), (2, b)] {
                if !is_traced(p, ft) {
                    continue;
                }
                let l_yes = g.fresh_label("mkres");
                let l_no = g.fresh_label("mkresn");
                // Ok (field 1) has tag false, Err (field 2) tag true.
                if pos == 1 {
                    g.cond_br(&tag, &l_no, &l_yes);
                } else {
                    g.cond_br(&tag, &l_yes, &l_no);
                }
                g.label(&l_yes);
                let x = g.inst(format!("extractvalue {} {}, {}", lt, v, pos));
                mark_expr(g, ft, &x, h);
                g.br(&l_no);
                g.label(&l_no);
            }
        }
        Ty::Enum(eid) => {
            let e = &p.enums[*eid as usize];
            if !is_traced(p, t) {
                return;
            }
            let slot = g.alloca(t);
            let lt = g.ty(t);
            g.emit(format!("store {} {}, ptr {}", lt, v, slot));
            let tt = format!("i{}", e.tag.bits);
            let tag = g.inst(format!("load {}, ptr {}", tt, slot));
            let pp = g.inst(format!("getelementptr inbounds {}, ptr {}, i32 0, i32 1", lt, slot));
            for var in &e.variants {
                if !var.fields.iter().any(|ft| is_traced(p, ft)) {
                    continue;
                }
                let c = g.inst(format!("icmp eq {} {}, {}", tt, tag, var.discr));
                let l_yes = g.fresh_label("mkenum");
                let l_no = g.fresh_label("mkenumn");
                g.cond_br(&c, &l_yes, &l_no);
                g.label(&l_yes);
                let st = Ty::Tuple(var.fields.clone());
                let stl = g.ty(&st);
                for (i, ft) in var.fields.iter().enumerate() {
                    if is_traced(p, ft) {
                        let fp = g.inst(format!("getelementptr inbounds {}, ptr {}, i32 0, i32 {}", stl, pp, i));
                        let x = g.load(ft, &fp);
                        mark_expr(g, ft, &x, h);
                    }
                }
                g.br(&l_no);
                g.label(&l_no);
            }
        }
        Ty::Array(et, n) => {
            if !is_traced(p, et) {
                return;
            }
            let slot = g.alloca(t);
            let lt = g.ty(t);
            g.emit(format!("store {} {}, ptr {}", lt, v, slot));
            mark_range(g, et, &slot, &n.to_string(), h);
        }
        Ty::Value(_) => {
            let d = &p.values[match t {
                Ty::Value(i) => *i as usize,
                _ => unreachable!(),
            }];
            for (i, var) in d.variants.iter().enumerate() {
                if !matches!(var.payload, Some(Ty::Gc(_))) {
                    continue;
                }
                let c = crate::values::is_variant(g, d, i as u32, v);
                let l_yes = g.fresh_label("mkv");
                let l_no = g.fresh_label("mkvn");
                g.cond_br(&c, &l_yes, &l_no);
                g.label(&l_yes);
                let ptr = crate::values::payload(g, d, i as u32, v);
                mark_call(g, h, &ptr);
                g.br(&l_no);
                g.label(&l_no);
            }
        }
        _ => {}
    }
}

/// Mark elements `0..n` of type `et` starting at `base`.
fn mark_range(g: &mut FnGen, et: &Ty, base: &str, n: &str, h: &str) {
    let elt = g.ty(et);
    let ctr = g.alloca(&Ty::i64());
    g.emit(format!("store i64 0, ptr {}", ctr));
    let head = g.fresh_label("mkloop");
    let body = g.fresh_label("mkbody");
    let done = g.fresh_label("mkdone");
    g.label(&head);
    let i = g.inst(format!("load i64, ptr {}", ctr));
    let c = g.inst(format!("icmp ult i64 {}, {}", i, n));
    g.cond_br(&c, &body, &done);
    g.label(&body);
    let ep = g.inst(format!("getelementptr inbounds {}, ptr {}, i64 {}", elt, base, i));
    let x = g.load(et, &ep);
    mark_expr(g, et, &x, h);
    let i2 = g.inst(format!("add i64 {}, 1", i));
    g.emit(format!("store i64 {}, ptr {}", i2, ctr));
    g.br(&head);
    g.label(&done);
}

/// A standalone function `void (ptr %h, ptr %slot)` marking a slot of type `t`.
pub fn mark_fn(cg: &mut Codegen, t: &Ty) -> String {
    let key = format!("{:?}", t);
    if let Some(n) = cg.mark_fns.get(&key) {
        return n.clone();
    }
    let name = format!("@\"loimi.mark.{}\"", cg.mark_fns.len());
    cg.mark_fns.insert(key, name.clone());
    let mut g = FnGen::new(cg, 0);
    if check_roots(&g) {
        // The root's index in its frame (`loimi_rootck_frame` resets it).
        let i = g.inst("load i64, ptr @loimi.rootck.idx");
        let i2 = g.inst(format!("add i64 {}, 1", i));
        g.emit(format!("store i64 {}, ptr @loimi.rootck.idx", i2));
    }
    let v = g.load(t, "%slot");
    mark_expr(&mut g, t, &v, "%h");
    if !g.terminated {
        g.term("ret void");
    }
    let text = format!(
        "define internal void {}(ptr %h, ptr %slot) nounwind {{\nentry:\n{}  br label %start\nstart:\n{}}}\n\n",
        name, g.allocas, g.body
    );
    cg.fns.push_str(&text);
    name
}

/// Runtime definitions: `loimi_trace_object`, the shadow-stack top.
pub fn emit_runtime(cg: &mut Codegen) {
    let p = cg.p;
    let _ = writeln!(cg.globals, "@loimi.shadow_top = internal thread_local(initialexec) global ptr null");
    let _ = writeln!(
        cg.fns,
        "define internal ptr @loimi_shadow_top() {{\n  %v = load ptr, ptr @loimi.shadow_top\n  ret ptr %v\n}}\n"
    );
    cg.decls.0.remove("loimi_shadow_top");
    if !gc_enabled(p) {
        return;
    }
    cg.decls.0.remove("loimi_trace_object");
    if cg.opts.gc_check_roots {
        emit_rootck_runtime(cg);
    }
    let mut g = FnGen::new(cg, 0);
    if check_roots(&g) {
        // What `rootck_mark` reports for a bad field: the object.
        let a = g.inst("ptrtoint ptr %obj to i64");
        g.emit(format!("store i64 {}, ptr @loimi.rootck.idx", a));
    }
    let objects: Vec<(usize, u32)> =
        p.structs.iter().enumerate().filter(|(_, s)| !s.pattern).filter_map(|(i, s)| s.object.map(|t| (i, t))).collect();
    let l_end = g.fresh_label("traced");
    let mut cases = String::new();
    let mut labels = Vec::new();
    for (_, tid) in &objects {
        let l = g.fresh_label("trace");
        let _ = write!(cases, " i32 {}, label %{}", tid, l);
        labels.push(l);
    }
    g.term(format!("switch i32 %tid, label %{} [{} ]", l_end, cases));
    for ((sid, _), l) in objects.iter().zip(labels.iter()) {
        g.label(l);
        if check_roots(&g) {
            let text = format!("a field of an object of type {}", p.structs[*sid].name);
            let info = rootck_info(g.cg, RC_OBJECT, &text);
            g.emit(format!("store ptr {}, ptr @loimi.rootck.what", info));
        }
        let st = Ty::Struct(*sid as u32);
        let lt = g.ty(&st);
        let s = &p.structs[*sid];
        for (i, (_, ft)) in s.fields.iter().enumerate() {
            if is_traced(p, ft) {
                let fp = g.inst(format!("getelementptr inbounds {}, ptr %obj, i32 0, i32 {}", lt, i));
                let x = g.load(ft, &fp);
                mark_expr(&mut g, ft, &x, "%h");
            }
        }
        if let Some(hook) = s.trace {
            // `#[trace(f)]`: the hook traces the trailing array (as much of
            // it as is live) and what the object owns outside the heap.
            let sym = p.fns[hook as usize].symbol.clone();
            g.emit(format!("call void @\"{}\"(ptr %h, ptr %obj)", sym));
        } else if let Some((_, et)) = &s.trailing {
            if is_traced(p, et) {
                let size = layout::size_of(p, &st);
                let base = g.inst(format!("getelementptr inbounds i8, ptr %obj, i64 {}", size));
                let lp = g.inst("getelementptr inbounds i8, ptr %obj, i64 -8");
                let n = g.inst(format!("load i64, ptr {}", lp));
                mark_range(&mut g, et, &base, &n, "%h");
            }
        }
        g.br(&l_end);
    }
    g.label(&l_end);
    g.term("ret void");
    let text = format!(
        "define internal void @loimi_trace_object(ptr %h, ptr %obj, i32 %tid) nounwind {{\nentry:\n{}  br label %start\nstart:\n{}}}\n\n",
        g.allocas, g.body
    );
    cg.fns.push_str(&text);
    // `loimi_type_name(tid) -> str`, for the collector's diagnostics.
    cg.decls.0.remove("loimi_type_name");
    let mut cases = String::new();
    let mut arms = String::new();
    for (sid, tid) in &objects {
        let name = cg.p.structs[*sid].name.clone();
        let s = cg.string_const(name.as_bytes());
        let _ = write!(cases, " i32 {}, label %n{}", tid, tid);
        let _ = write!(arms, "n{}:\n  ret {{ ptr, i64 }} {{ ptr {}, i64 {} }}\n", tid, s, name.len());
    }
    let _ = write!(
        cg.fns,
        "define internal {{ ptr, i64 }} @loimi_type_name(i32 %tid) cold nounwind {{\nentry:\n  switch i32 %tid, label %none [{} ]\n{}none:\n  ret {{ ptr, i64 }} zeroinitializer\n}}\n\n",
        cases, arms
    );
}

/// `--gc-check-roots`: the thread-locals naming what is being marked,
/// and the accessors `std.gc.rootck_*` declares.
fn emit_rootck_runtime(cg: &mut Codegen) {
    let _ = writeln!(cg.globals, "@loimi.rootck.what = internal thread_local(initialexec) global ptr null");
    let _ = writeln!(cg.globals, "@loimi.rootck.idx = internal thread_local(initialexec) global i64 0");
    for d in ["loimi_rootck_what", "loimi_rootck_kind", "loimi_rootck_idx", "loimi_rootck_info"] {
        cg.decls.0.remove(d);
    }
    let _ = write!(
        cg.fns,
        "define internal void @loimi_rootck_frame(ptr %h, ptr %info) nounwind {{
  store ptr %info, ptr @loimi.rootck.what
  store i64 -1, ptr @loimi.rootck.idx
  ret void
}}

define internal {{ ptr, i64 }} @loimi_rootck_what() nounwind {{
  %w = load ptr, ptr @loimi.rootck.what
  %z = icmp eq ptr %w, null
  br i1 %z, label %none, label %some
none:
  ret {{ ptr, i64 }} zeroinitializer
some:
  %s = load {{ ptr, i64 }}, ptr %w
  ret {{ ptr, i64 }} %s
}}

define internal i64 @loimi_rootck_kind() nounwind {{
  %w = load ptr, ptr @loimi.rootck.what
  %z = icmp eq ptr %w, null
  br i1 %z, label %none, label %some
none:
  ret i64 -1
some:
  %p = getelementptr inbounds {{ ptr, i64, i64 }}, ptr %w, i32 0, i32 2
  %k = load i64, ptr %p
  ret i64 %k
}}

define internal i64 @loimi_rootck_idx() nounwind {{
  %i = load i64, ptr @loimi.rootck.idx
  ret i64 %i
}}

define internal i64 @loimi_rootck_info() nounwind {{
  %w = load ptr, ptr @loimi.rootck.what
  %i = ptrtoint ptr %w to i64
  ret i64 %i
}}

"
    );
}

/// `new(heap, init, len?)`
pub fn new_object(g: &mut FnGen, e: &TExpr, heap: &TExpr, init: &TExpr, len: Option<&TExpr>) -> String {
    let p = g.p();
    let h = g.val(heap);
    let v = g.val(init);
    let Ty::Struct(sid) = init.ty else { unreachable!() };
    let s = &p.structs[sid as usize];
    let size = layout::size_of(p, &init.ty);
    let (n, total) = match (len, &s.trailing) {
        (Some(l), Some((_, et))) => {
            let n = g.val(l);
            let es = layout::size_of(p, et);
            let ext = g.inst(format!("mul i64 {}, {}", n, es));
            let t = g.inst(format!("add i64 {}, {}", ext, size));
            (n, t)
        }
        _ => ("0".to_string(), size.to_string()),
    };
    let tid = s.object.unwrap();
    // `--gc-check-roots`: `rootck_alloc` installs the sweeper that
    // poisons freed objects before the heap's first collection.
    let alloc = if check_roots(g) { "std.gc.rootck_alloc" } else { "std.gc.alloc" };
    let obj = g.inst(format!("call ptr @\"{}\"(ptr {}, i64 {}, i32 {}, i64 {})", alloc, h, total, tid, n));
    if !s.fields.is_empty() {
        let lt = llty(p, &init.ty);
        g.emit(format!("store {} {}, ptr {}", lt, v, obj));
    }
    let _ = e;
    obj
}

/// The trailing array of the object `obj` as a slice.
pub fn trailing(g: &mut FnGen, obj: &TExpr) -> String {
    let p = g.p();
    let o = g.val(obj);
    let Ty::Gc(inner) = &obj.ty else { unreachable!() };
    let size = layout::size_of(p, inner);
    let base = g.inst(format!("getelementptr inbounds i8, ptr {}, i64 {}", o, size));
    let lp = g.inst(format!("getelementptr inbounds i8, ptr {}, i64 -8", o));
    let n = g.inst(format!("load i64, ptr {}", lp));
    let a = g.inst(format!("insertvalue {{ ptr, i64 }} poison, ptr {}, 0", base));
    g.inst(format!("insertvalue {{ ptr, i64 }} {}, i64 {}, 1", a, n))
}

/// Before a call that may collect: link a shadow-stack frame listing the
/// traced locals in scope. Returns the saved previous top. Whether the call
/// may collect comes from the callee's type, which for a call through a
/// function pointer is the pointer type's effects.
pub fn push_frame(g: &mut FnGen, f: &TExpr) -> Option<String> {
    let p = g.p();
    let (Ty::Fn(_, _, mut effects) | Ty::Block(_, _, mut effects) | Ty::Closure(_, _, mut effects)) = crate::check::callee_sig(&f.ty) else {
        return None;
    };
    if let TK::FnRef(id) = f.kind {
        effects |= p.fns[id as usize].sync_effects;
    }
    if effects & EFF_GC == 0 {
        return None;
    }
    let fd = g.f();
    let mut ids: Vec<LocalId> = Vec::new();
    for scope in &g.live {
        for &l in scope {
            if !ids.contains(&l) && is_traced(p, &fd.locals[l as usize].ty) {
                ids.push(l);
            }
        }
    }
    // (slot, type) of each root: traced locals, then pending values, then
    // the traced vm state.
    let mut roots: Vec<(String, Ty)> =
        ids.iter().map(|l| (g.slots[*l as usize].clone(), fd.locals[*l as usize].ty.clone())).collect();
    roots.extend(g.pending.iter().cloned());
    let nlocal = roots.len();
    state_roots(g, f, &mut roots);
    if roots.is_empty() {
        return None;
    }
    if check_roots(g) {
        // A first entry that names the frame (`loimi_rootck_frame`).
        let callee = match f.kind {
            TK::FnRef(id) => p.fns[id as usize].symbol.clone(),
            _ => "a function value".into(),
        };
        let mut text = format!("{} at {} (call of {}); roots:", fd.symbol, g.cg.location(f.span), callee);
        for (i, (_, t)) in roots.iter().enumerate() {
            let what = if i < ids.len() {
                fd.locals[ids[i] as usize].name.clone()
            } else if i < nlocal {
                "(a value kept for defers, or a scrutinee during a guard)".into()
            } else {
                "(a copy of a state variable)".into()
            };
            text.push_str(&format!(" #{} {}: {};", i, what, ty_str(p, t)));
        }
        let info = rootck_info(g.cg, RC_FRAME, &text);
        roots.insert(0, (info, Ty::Unit));
    }
    let n = roots.len();
    let ft = format!("{{ ptr, i64, [{} x {{ ptr, ptr }}] }}", n);
    let fr = g.fresh();
    let _ = writeln!(g.allocas, "  {} = alloca {}, align 8", fr, ft);
    let prev = g.inst("load ptr, ptr @loimi.shadow_top");
    g.emit(format!("store ptr {}, ptr {}", prev, fr));
    let np = g.inst(format!("getelementptr inbounds {}, ptr {}, i32 0, i32 1", ft, fr));
    g.emit(format!("store i64 {}, ptr {}", n, np));
    for (i, (slot, t)) in roots.iter().enumerate() {
        let mf = if i == 0 && check_roots(g) { "@loimi_rootck_frame".to_string() } else { mark_fn(g.cg, t) };
        let sp = g.inst(format!("getelementptr inbounds {}, ptr {}, i32 0, i32 2, i64 {}, i32 0", ft, fr, i));
        g.emit(format!("store ptr {}, ptr {}", slot, sp));
        let mp = g.inst(format!("getelementptr inbounds {}, ptr {}, i32 0, i32 2, i64 {}, i32 1", ft, fr, i));
        g.emit(format!("store ptr {}, ptr {}", mf, mp));
    }
    g.emit(format!("store ptr {}, ptr @loimi.shadow_top", fr));
    Some(prev)
}

/// In vm code, the state variables of traced types (a cached top of
/// stack, `state { .., tos: Value }`; a current closure, `cl: Gc<Closure>`)
/// are roots at every call that may collect. Each is copied, at the call,
/// into a slot of its own that the frame lists: the state variable itself
/// never has its address taken, so it stays in a register everywhere
/// else, and only the path that makes the call pays (one store per
/// variable, next to the stores of the vm's `sync`). In a vm function the
/// state arrives as pointers to the caller's variables: the copy is read
/// through them. A call of a vm function of the same vm roots no state:
/// the function roots it itself at its own calls that may collect, so a
/// `gc_point()` that collects only on a cold path costs its callers
/// nothing. (Not built: a moving collector, which would also copy the
/// slots back after the call.)
fn state_roots(g: &mut FnGen, callee: &TExpr, roots: &mut Vec<(String, Ty)>) {
    let p = g.p();
    let fd = g.f();
    let Some(vm) = fd.vm else { return };
    if let TK::FnRef(id) = callee.kind {
        let cf = &p.fns[id as usize];
        if cf.vm_role == VmRole::VmFn && cf.vm == Some(vm) {
            return;
        }
    }
    let by_ptr = fd.vm_role == VmRole::VmFn;
    for &l in &fd.vm_state {
        let lt = &fd.locals[l as usize].ty;
        let t = match (by_ptr, lt) {
            (true, Ty::Ptr(inner)) => (**inner).clone(),
            (true, _) => continue,
            (false, t) => t.clone(),
        };
        if !is_traced(p, &t) {
            continue;
        }
        let slot = g.slots[l as usize].clone();
        let at = if by_ptr { g.inst(format!("load ptr, ptr {}", slot)) } else { slot };
        let v = g.load(&t, &at);
        let copy = g.alloca(&t);
        g.store(&t, &v, &copy);
        roots.push((copy, t));
    }
}

/// Entries start with their traced state variables zeroed: every later
/// call that may collect roots them (`state_roots`), also those the entry
/// never assigns, and zero is a valid value of every traced type.
pub fn zero_traced_state(g: &mut FnGen) {
    let p = g.p();
    let fd = g.f();
    if fd.vm.is_none() || fd.vm_role != VmRole::Entry {
        return;
    }
    for &l in &fd.vm_state {
        let t = fd.locals[l as usize].ty.clone();
        if is_traced(p, &t) {
            let slot = g.slots[l as usize].clone();
            g.store(&t, "zeroinitializer", &slot);
        }
    }
}

pub fn pop_frame(g: &mut FnGen, prev: Option<String>) {
    if let Some(prev) = prev {
        g.emit(format!("store ptr {}, ptr @loimi.shadow_top", prev));
    }
}

/// Whether the place `e` lies inside a GC object: a field (or element of
/// an array field, or field of a struct field) of an object reached
/// through a `Gc`, or an element of an object's trailing array. Stores of
/// traced values there go through the write barrier.
fn in_object(e: &TExpr) -> bool {
    match &e.kind {
        TK::Deref(x) => matches!(x.ty, Ty::Gc(_)),
        TK::Field(b, _) => matches!(b.ty, Ty::Struct(_) | Ty::Tuple(_)) && in_object(b),
        TK::Index(b, _) => match &b.ty {
            Ty::Array(..) => in_object(b),
            Ty::Slice(_) => slice_in_object(b),
            // `o.items.ptr[i]`: unchecked indexing into a trailing array.
            Ty::Ptr(_) => trailing_ptr(b).is_some(),
            _ => false,
        },
        _ => false,
    }
}

/// Whether the slice `e` points into an object: a trailing array, or a
/// sub-slice of one or of an array field (`o.items[1..][0] = v`).
fn slice_in_object(e: &TExpr) -> bool {
    match &e.kind {
        TK::Trailing(_) => true,
        TK::SliceRange(b, _, _) => match &b.ty {
            Ty::Slice(_) => slice_in_object(b),
            Ty::Array(..) => in_object(b),
            _ => false,
        },
        _ => false,
    }
}

/// The slice `e` (for which `slice_in_object` holds) as (pointer, length),
/// and the object it points into. Bounds-checked like `TK::SliceRange`.
fn slice_place(g: &mut FnGen, e: &TExpr) -> (String, String, String) {
    match &e.kind {
        TK::Trailing(o) => {
            let p = g.p();
            let obj = g.val(o);
            let Ty::Gc(inner) = &o.ty else { unreachable!() };
            let size = layout::size_of(p, inner);
            let base = g.inst(format!("getelementptr inbounds i8, ptr {}, i64 {}", obj, size));
            let lp = g.inst(format!("getelementptr inbounds i8, ptr {}, i64 -8", obj));
            let len = g.inst(format!("load i64, ptr {}", lp));
            (base, len, obj)
        }
        TK::SliceRange(b, lo, hi) => {
            let (ptr, len, obj) = match &b.ty {
                Ty::Array(_, n) => {
                    let (p, obj) = object_place(g, b);
                    (p, n.to_string(), obj)
                }
                _ => slice_place(g, b),
            };
            let lo_v = match lo {
                Some(l) => g.val(l),
                None => "0".into(),
            };
            let hi_v = match hi {
                Some(h) => g.val(h),
                None => len.clone(),
            };
            let c1 = g.inst(format!("icmp ugt i64 {}, {}", hi_v, len));
            let c2 = g.inst(format!("icmp ugt i64 {}, {}", lo_v, hi_v));
            let c = g.inst(format!("or i1 {}, {}", c1, c2));
            g.check(&c, "slice range out of bounds", e.span);
            let Ty::Slice(et) = &e.ty else { unreachable!() };
            let el = g.ty(et);
            let np = g.inst(format!("getelementptr inbounds {}, ptr {}, i64 {}", el, ptr, lo_v));
            let nl = g.inst(format!("sub i64 {}, {}", hi_v, lo_v));
            (np, nl, obj)
        }
        _ => unreachable!(),
    }
}

/// The object of `o.items.ptr` (the pointer of a trailing array).
fn trailing_ptr(e: &TExpr) -> Option<&TExpr> {
    match &e.kind {
        TK::Field(s, 0) => match &s.kind {
            TK::Trailing(o) => Some(o),
            _ => None,
        },
        _ => None,
    }
}

/// Does the assignment to `lhs` need a write barrier?
pub fn needs_barrier(g: &mut FnGen, lhs: &TExpr) -> bool {
    if !in_object(lhs) || !is_traced(g.p(), &lhs.ty) {
        return false;
    }
    if g.cg.gc_barrier.is_none() {
        let p = g.p();
        let has = p.fns.iter().any(|f| f.symbol == "std.gc.barrier" && f.body.is_some());
        g.cg.gc_barrier = Some(has);
    }
    g.cg.gc_barrier == Some(true)
}

/// The address of the place `e` (for which `in_object` holds) and of the
/// object that contains it.
pub fn object_place(g: &mut FnGen, e: &TExpr) -> (String, String) {
    match &e.kind {
        TK::Deref(x) => {
            let v = g.val(x);
            (v.clone(), v)
        }
        TK::Field(b, i) => {
            let (base, obj) = object_place(g, b);
            let bt = g.ty(&b.ty);
            let p = g.inst(format!("getelementptr inbounds {}, ptr {}, i32 0, i32 {}", bt, base, i));
            (p, obj)
        }
        TK::Index(b, idx) => match (&b.ty, &b.kind) {
            (Ty::Array(_, n), _) => {
                let n = *n;
                let (base, obj) = object_place(g, b);
                let i = g.index_val(idx);
                let bad = g.inst(format!("icmp uge i64 {}, {}", i, n));
                g.check(&bad, "index out of bounds", e.span);
                let bt = g.ty(&b.ty);
                let p = g.inst(format!("getelementptr inbounds {}, ptr {}, i64 0, i64 {}", bt, base, i));
                (p, obj)
            }
            (Ty::Slice(_), _) => {
                let (base, len, obj) = slice_place(g, b);
                let i = g.index_val(idx);
                let bad = g.inst(format!("icmp uge i64 {}, {}", i, len));
                g.check(&bad, "index out of bounds", e.span);
                let elem = g.ty(&e.ty);
                let p = g.inst(format!("getelementptr inbounds {}, ptr {}, i64 {}", elem, base, i));
                (p, obj)
            }
            (Ty::Ptr(_), _) => {
                let o = trailing_ptr(b).unwrap();
                let p = g.p();
                let obj = g.val(o);
                let Ty::Gc(inner) = &o.ty else { unreachable!() };
                let size = layout::size_of(p, inner);
                let base = g.inst(format!("getelementptr inbounds i8, ptr {}, i64 {}", obj, size));
                let i = g.index_val(idx);
                let elem = g.ty(&e.ty);
                let p = g.inst(format!("getelementptr inbounds {}, ptr {}, i64 {}", elem, base, i));
                (p, obj)
            }
            _ => unreachable!(),
        },
        _ => unreachable!(),
    }
}

/// `lhs = v` into an object, followed by the write barrier on the object.
pub fn store_with_barrier(g: &mut FnGen, lhs: &TExpr, v: &str) {
    let (p, obj) = object_place(g, lhs);
    g.store(&lhs.ty, v, &p);
    g.emit(format!("call void @\"std.gc.barrier\"(ptr {})", obj));
}
