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
        Ty::Tuple(ts) => ts.iter().any(|t| is_traced(p, t)),
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
        p.structs.iter().enumerate().filter_map(|(i, s)| s.object.map(|t| (i, t))).collect();
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
/// traced locals in scope. Returns the saved previous top.
pub fn push_frame(g: &mut FnGen, f: &TExpr) -> Option<String> {
    let p = g.p();
    let TK::FnRef(id) = f.kind else { return None };
    if p.fns[id as usize].effects & EFF_GC == 0 {
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
