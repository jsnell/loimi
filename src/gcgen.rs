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

/// Emit code marking every GC reference in `v` (a value of type `t`).
pub fn mark_expr(g: &mut FnGen, t: &Ty, v: &str, h: &str) {
    let p = g.p();
    match t {
        Ty::Gc(_) => {
            g.emit(format!("call void @\"std.gc.mark_ptr\"(ptr {}, ptr {})", h, v));
        }
        Ty::Option(inner) if inner.is_nullable_ptr() => {
            if is_traced(p, inner) {
                g.emit(format!("call void @\"std.gc.mark_ptr\"(ptr {}, ptr {})", h, v));
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
                g.emit(format!("call void @\"std.gc.mark_ptr\"(ptr {}, ptr {})", h, ptr));
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
    let mut g = FnGen::new(cg, 0);
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
        if let Some((_, et)) = &s.trailing {
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
    let obj = g.inst(format!("call ptr @\"std.gc.alloc\"(ptr {}, i64 {}, i32 {}, i64 {})", h, total, tid, n));
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
    let (Ty::Fn(_, _, mut effects) | Ty::Block(_, _, mut effects)) = f.ty else { return None };
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
    if ids.is_empty() {
        return None;
    }
    let n = ids.len();
    let ft = format!("{{ ptr, i64, [{} x {{ ptr, ptr }}] }}", n);
    let fr = g.fresh();
    let _ = writeln!(g.allocas, "  {} = alloca {}, align 8", fr, ft);
    let prev = g.inst("load ptr, ptr @loimi.shadow_top");
    g.emit(format!("store ptr {}, ptr {}", prev, fr));
    let np = g.inst(format!("getelementptr inbounds {}, ptr {}, i32 0, i32 1", ft, fr));
    g.emit(format!("store i64 {}, ptr {}", n, np));
    for (i, l) in ids.iter().enumerate() {
        let t = fd.locals[*l as usize].ty.clone();
        let mf = mark_fn(g.cg, &t);
        let sp = g.inst(format!("getelementptr inbounds {}, ptr {}, i32 0, i32 2, i64 {}, i32 0", ft, fr, i));
        let slot = g.slots[*l as usize].clone();
        g.emit(format!("store ptr {}, ptr {}", slot, sp));
        let mp = g.inst(format!("getelementptr inbounds {}, ptr {}, i32 0, i32 2, i64 {}, i32 1", ft, fr, i));
        g.emit(format!("store ptr {}, ptr {}", mf, mp));
    }
    g.emit(format!("store ptr {}, ptr @loimi.shadow_top", fr));
    Some(prev)
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
            Ty::Slice(_) => matches!(b.kind, TK::Trailing(_)),
            // `o.items.ptr[i]`: unchecked indexing into a trailing array.
            Ty::Ptr(_) => trailing_ptr(b).is_some(),
            _ => false,
        },
        _ => false,
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
fn object_place(g: &mut FnGen, e: &TExpr) -> (String, String) {
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
            (_, TK::Trailing(o)) => {
                let p = g.p();
                let obj = g.val(o);
                let Ty::Gc(inner) = &o.ty else { unreachable!() };
                let size = layout::size_of(p, inner);
                let base = g.inst(format!("getelementptr inbounds i8, ptr {}, i64 {}", obj, size));
                let lp = g.inst(format!("getelementptr inbounds i8, ptr {}, i64 -8", obj));
                let len = g.inst(format!("load i64, ptr {}", lp));
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
