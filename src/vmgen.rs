//! Code generation for vm handlers: dispatch in tail-call, goto and switch
//! modes, control words, inline caches, superinstructions.

use std::collections::HashMap;
use std::fmt::Write;

use crate::codegen::{llty, ret_llty, FnGen};
use crate::diag::Span;
use crate::tir::*;

#[derive(Clone, Debug)]
pub struct HandlerGen {
    pub vm: u32,
    pub combined: bool,
    pub role: VmRole,
    pub state_slots: Vec<String>,
    pub parts: HashMap<u32, String>,
    pub dispatch_label: String,
    pub raise_label: String,
    pub invalid_label: String,
    pub op_labels: Vec<String>,
    /// Combined mode: labels of instrumentation wrappers, per variant (op index → label).
    pub wrapper_labels: Vec<Vec<(u32, String)>>,
    pub raise_slot: String,
    pub labels_global: String,
    /// Tail-call handlers receive the current instruction word in a register.
    pub insn_slot: Option<String>,
}

fn takes_insn(p: &Program, f: &FnDef) -> bool {
    match (f.vm, f.vm_role) {
        (Some(vm), VmRole::Handler(_) | VmRole::Raise | VmRole::Wrapper(..)) => p.vms[vm as usize].dispatch == Dispatch::TailCall,
        _ => false,
    }
}

/// Extra codegen-only parameter of tail-call handlers.
pub fn extra_param(p: &Program, f: &FnDef) -> Option<String> {
    if takes_insn(p, f) {
        Some(format!("i{} %insn", p.vms[f.vm.unwrap() as usize].word.bits))
    } else {
        None
    }
}

pub fn table_name(vd: &VmDef) -> String {
    format!("@\"{}.table\"", vd.symbol)
}

/// Called after parameters are stored: set up vm state for handlers/entries.
pub fn handler_prologue(g: &mut FnGen) {
    let f = g.f();
    let Some(vm) = f.vm else { return };
    if f.vm_role == VmRole::VmFn {
        return;
    }
    let p = g.p();
    let vd = &p.vms[vm as usize];
    let combined = vd.dispatch != Dispatch::TailCall;
    let state_slots: Vec<String> = f.vm_state.iter().map(|l| g.slots[*l as usize].clone()).collect();
    let mut hg = HandlerGen {
        vm,
        combined,
        role: f.vm_role,
        state_slots,
        parts: HashMap::new(),
        dispatch_label: String::new(),
        raise_label: String::new(),
        invalid_label: String::new(),
        op_labels: vec![],
        wrapper_labels: vec![],
        raise_slot: format!("@\"{}.raise_slot\"", vd.symbol),
        labels_global: format!("@\"{}.labels\"", f.symbol),
        insn_slot: None,
    };
    if takes_insn(p, f) {
        let wt = Ty::Int(vd.word);
        let slot = g.alloca(&wt);
        g.store(&wt, "%insn", &slot);
        hg.insn_slot = Some(slot);
    }
    if let (VmRole::Handler(k), None) = (f.vm_role, &g.cold) {
        if !combined {
            count_op(g, k);
        }
    }
    if combined {
        hg.dispatch_label = g.fresh_label("dispatch");
        hg.raise_label = g.fresh_label("raise");
        hg.invalid_label = g.fresh_label("invalid");
        hg.op_labels = vd.ops.iter().map(|o| format!("op.{}.{}", o.name, o.opcode)).collect();
        hg.wrapper_labels = vd
            .variants
            .iter()
            .enumerate()
            .map(|(v, (_, ws))| ws.iter().map(|(k, _)| (*k, format!("var.{}.{}", v, k))).collect())
            .collect();
        if let Some(rt) = &vd.raise_ty {
            hg.raise_slot = g.alloca(rt);
        }
    }
    g.vm = Some(hg);
}

/// Combined (goto/switch) entries: emit every handler into this function.
pub fn epilogue(g: &mut FnGen) {
    let Some(hg) = g.vm.clone() else { return };
    if !hg.combined || hg.role != VmRole::Entry {
        return;
    }
    let p = g.p();
    let vd = &p.vms[hg.vm as usize];
    let entry_fid = g.fid;
    let entry_slots = g.slots.clone();
    let emit_body = |g: &mut FnGen, fid: FnId, label: &str| {
        let hf = &p.fns[fid as usize];
        g.fid = fid;
        let mut slots = Vec::new();
        for (l, ld) in hf.locals.iter().enumerate() {
            match hf.vm_state.iter().position(|s| *s as usize == l) {
                Some(i) => slots.push(hg.state_slots[i].clone()),
                None => slots.push(g.alloca(&ld.ty)),
            }
        }
        g.slots = slots;
        g.live = vec![vec![]];
        if let Some(h) = g.vm.as_mut() {
            h.parts.clear();
            h.role = hf.vm_role;
        }
        g.label(label);
        if let VmRole::Handler(k) = hf.vm_role {
            count_op(g, k);
        }
        g.val(hf.body.as_ref().unwrap());
        if !g.terminated {
            g.term("unreachable");
        }
    };
    for (k, op) in vd.ops.iter().enumerate() {
        emit_body(g, op.fid, &hg.op_labels[k]);
    }
    if let Some(rf) = vd.on_raise {
        emit_body(g, rf, &hg.raise_label);
    }
    for (v, (_, ws)) in vd.variants.iter().enumerate() {
        for (j, (_, fid)) in ws.iter().enumerate() {
            emit_body(g, *fid, &hg.wrapper_labels[v][j].1);
        }
    }
    g.fid = entry_fid;
    g.slots = entry_slots;
    if let Some(h) = g.vm.as_mut() {
        h.role = VmRole::Entry;
    }
    g.label(&hg.invalid_label);
    g.cg.decls.insert("declare void @llvm.trap() noreturn");
    g.emit("call void @llvm.trap()");
    g.term("unreachable");
    let n = 1usize << vd.op_bits;
    let sym = g.f().symbol.clone();
    if vd.dispatch == Dispatch::Switch {
        g.label(&hg.dispatch_label);
        let pc = g.inst(format!("load ptr, ptr {}", hg.state_slots[vd.pc_idx]));
        let opc = opcode_of(g, vd, &pc);
        let wt = format!("i{}", vd.word.bits);
        let mut cases = String::new();
        for (k, op) in vd.ops.iter().enumerate() {
            let _ = write!(cases, " i32 {}, label %{}", op.opcode, hg.op_labels[k]);
        }
        let case = if let Some(ti) = vd.tbl_idx {
            // With variants, the table maps opcodes to case ids.
            for (v, ws) in hg.wrapper_labels.iter().enumerate() {
                for (k, l) in ws {
                    let _ = write!(cases, " i32 {}, label %{}", n as u32 * (v as u32 + 1) + k, l);
                }
            }
            let t = g.inst(format!("load ptr, ptr {}", hg.state_slots[ti]));
            let a = g.inst(format!("getelementptr inbounds i32, ptr {}, i64 {}", t, opc));
            g.inst(format!("load i32, ptr {}", a))
        } else {
            let _ = wt;
            g.inst(format!("trunc i64 {} to i32", opc))
        };
        g.term(format!("switch i32 {}, label %{} [{} ]", case, hg.invalid_label, cases));
        for t in 0..=vd.variants.len() {
            if t > 0 || vd.tbl_idx.is_some() {
                let mut ids = Vec::new();
                for i in 0..n {
                    let id = match vd.ops.iter().position(|o| o.opcode as usize == i) {
                        Some(k) => match (t > 0).then(|| hg.wrapper_labels[t - 1].iter().any(|(w, _)| *w as usize == k)) {
                            Some(true) => n as u32 * t as u32 + k as u32,
                            _ => vd.ops[k].opcode,
                        },
                        None => u32::MAX,
                    };
                    ids.push(format!("i32 {}", id));
                }
                let _ = writeln!(g.cg.globals, "{} = internal constant [{} x i32] [{}]", combined_table(&sym, t), n, ids.join(", "));
            }
        }
    } else {
        for t in 0..=vd.variants.len() {
            let mut entries = Vec::new();
            for i in 0..n {
                let l = match vd.ops.iter().position(|o| o.opcode as usize == i) {
                    Some(k) => {
                        let w = if t > 0 { hg.wrapper_labels[t - 1].iter().find(|(w, _)| *w as usize == k) } else { None };
                        match w {
                            Some((_, l)) => l,
                            None => &hg.op_labels[k],
                        }
                    }
                    None => &hg.invalid_label,
                };
                entries.push(format!("ptr blockaddress(@\"{}\", %{})", sym, l));
            }
            let _ = writeln!(g.cg.globals, "{} = internal constant [{} x ptr] [{}]", combined_table(&sym, t), n, entries.join(", "));
        }
    }
}

/// Name of dispatch table `t` (0 = default) of a combined-mode entry.
fn combined_table(entry_sym: &str, t: usize) -> String {
    if t == 0 { format!("@\"{}.labels\"", entry_sym) } else { format!("@\"{}.labels.{}\"", entry_sym, t) }
}

/// Name of dispatch table `t` of a tail-call vm.
fn tail_table(vd: &VmDef, t: usize) -> String {
    if t == 0 { table_name(vd) } else { format!("@\"{}.table.{}\"", vd.symbol, t) }
}

/// Opcode of the word at `pc`, as i64.
fn opcode_of(g: &mut FnGen, vd: &VmDef, pc: &str) -> String {
    opcode_and_word(g, vd, pc).0
}

/// (opcode as i64, the word) at `pc`.
fn opcode_and_word(g: &mut FnGen, vd: &VmDef, pc: &str) -> (String, String) {
    let wt = format!("i{}", vd.word.bits);
    let w = g.inst(format!("load {}, ptr {}", wt, pc));
    let word = w.clone();
    let sh = if vd.op_shift == 0 { w } else { g.inst(format!("lshr {} {}, {}", wt, w, vd.op_shift)) };
    let mask = (1u64 << vd.op_bits) - 1;
    let m = if vd.op_shift + vd.op_bits >= vd.word.bits as u32 { sh } else { g.inst(format!("and {} {}, {}", wt, sh, mask)) };
    let opc = if vd.word.bits == 64 { m } else { g.inst(format!("zext {} {} to i64", wt, m)) };
    (opc, word)
}

/// The current instruction word (from the register in tail-call handlers).
fn current_word(g: &mut FnGen, vd: &VmDef) -> String {
    let hg = g.vm.clone().unwrap();
    match &hg.insn_slot {
        Some(s) => g.inst(format!("load i{}, ptr {}", vd.word.bits, s)),
        None => {
            let pc = g.inst(format!("load ptr, ptr {}", hg.state_slots[vd.pc_idx]));
            g.inst(format!("load i{}, ptr {}", vd.word.bits, pc))
        }
    }
}

fn state_args(g: &mut FnGen, vd: &VmDef) -> String {
    let hg = g.vm.clone().unwrap();
    let mut args = Vec::new();
    for (i, t) in vd.state_tys.iter().enumerate() {
        let v = g.load(t, &hg.state_slots[i]);
        args.push(format!("{} {}", llty(g.p(), t), v));
    }
    args.join(", ")
}

/// Transfer control to `target` (already stored in the pc slot).
fn tail_to(g: &mut FnGen, vd: &VmDef, callee: &str, insn: &str) {
    let hg = g.vm.clone().unwrap();
    let args = format!("{}, i{} {}", state_args(g, vd), vd.word.bits, insn);
    let rt = ret_llty(g.p(), &vd.ret);
    let kind = if hg.role == VmRole::Entry { "" } else { "musttail " };
    if rt == "void" {
        g.emit(format!("{}call preserve_nonecc void {}({})", kind, callee, args));
        g.term("ret void");
    } else {
        let r = g.inst(format!("{}call preserve_nonecc {} {}({})", kind, rt, callee, args));
        g.term(format!("ret {} {}", rt, r));
    }
}

fn dispatch(g: &mut FnGen, target: &str) {
    let hg = g.vm.clone().unwrap();
    let p = g.p();
    let vd = &p.vms[hg.vm as usize];
    g.emit(format!("store ptr {}, ptr {}", target, hg.state_slots[vd.pc_idx]));
    if hg.combined {
        if vd.dispatch == Dispatch::Switch {
            g.br(&hg.dispatch_label);
        } else {
            let opc = opcode_of(g, vd, target);
            let tbl = match vd.tbl_idx {
                Some(ti) => g.inst(format!("load ptr, ptr {}", hg.state_slots[ti])),
                None => hg.labels_global.clone(),
            };
            let a = g.inst(format!("getelementptr inbounds ptr, ptr {}, i64 {}", tbl, opc));
            let h = g.inst(format!("load ptr, ptr {}", a));
            let mut dests: Vec<String> = hg.op_labels.iter().map(|l| format!("label %{}", l)).collect();
            for ws in &hg.wrapper_labels {
                for (_, l) in ws {
                    dests.push(format!("label %{}", l));
                }
            }
            dests.push(format!("label %{}", hg.invalid_label));
            g.term(format!("indirectbr ptr {}, [{}]", h, dests.join(", ")));
        }
    } else {
        let (opc, word) = opcode_and_word(g, vd, target);
        let tbl = match vd.tbl_idx {
            Some(ti) => g.inst(format!("load ptr, ptr {}", hg.state_slots[ti])),
            None => table_name(vd),
        };
        let a = g.inst(format!("getelementptr inbounds ptr, ptr {}, i64 {}", tbl, opc));
        let h = g.inst(format!("load ptr, ptr {}", a));
        tail_to(g, vd, &h, &word);
    }
}

pub fn word_place(g: &mut FnGen, off: u32) -> String {
    let hg = g.vm.clone().expect("code word access outside a handler");
    let p = g.p();
    let vd = &p.vms[hg.vm as usize];
    let pc = g.inst(format!("load ptr, ptr {}", hg.state_slots[vd.pc_idx]));
    if off == 0 {
        return pc;
    }
    g.inst(format!("getelementptr inbounds i{}, ptr {}, i64 {}", vd.word.bits, pc, off))
}

pub fn vm_expr(g: &mut FnGen, e: &TExpr, v: &TVm) -> String {
    match v {
        TVm::Cold(b) => {
            if can_outline(g, b) {
                outline(g, b);
                return g.poison(&e.ty);
            }
            // A cold call-site attribute on a no-op intrinsic: the block is
            // treated as unlikely, and no machine code is emitted.
            g.cg.decls.insert("declare void @llvm.sideeffect()");
            g.emit("call void @llvm.sideeffect() cold");
            g.val(b)
        }
        TVm::Jump(t) => {
            let tv = g.val(t);
            dispatch(g, &tv);
            g.poison(&e.ty)
        }
        TVm::Exit(x) => {
            let v = g.val(x);
            let rt = ret_llty(g.p(), &x.ty);
            if rt == "void" {
                g.term("ret void");
            } else {
                g.term(format!("ret {} {}", rt, v));
            }
            g.poison(&e.ty)
        }
        TVm::Raise(x) => {
            let v = g.val(x);
            let hg = g.vm.clone().unwrap();
            let p = g.p();
            let vd = &p.vms[hg.vm as usize];
            g.store(&x.ty, &v, &hg.raise_slot);
            if hg.combined {
                g.br(&hg.raise_label);
            } else {
                let sym = format!("@\"{}\"", p.fns[vd.on_raise.unwrap() as usize].symbol);
                let w = current_word(g, vd);
                tail_to(g, vd, &sym, &w);
            }
            g.poison(&e.ty)
        }
        TVm::RaiseSlot => {
            let hg = g.vm.clone().unwrap();
            g.load(&e.ty, &hg.raise_slot)
        }
        TVm::Become(opc) => {
            let hg = g.vm.clone().unwrap();
            let p = g.p();
            let vd = &p.vms[hg.vm as usize];
            let wt = format!("i{}", vd.word.bits);
            let pc = g.inst(format!("load ptr, ptr {}", hg.state_slots[vd.pc_idx]));
            let w = g.inst(format!("load {}, ptr {}", wt, pc));
            let mask = ((1u64 << vd.op_bits) - 1) << vd.op_shift;
            let cleared = g.inst(format!("and {} {}, {}", wt, w, !mask as i64 & word_mask(vd.word.bits)));
            let nw = g.inst(format!("or {} {}, {}", wt, cleared, (*opc as u64) << vd.op_shift));
            g.emit(format!("store {} {}, ptr {}", wt, nw, pc));
            "zeroinitializer".into()
        }
        TVm::Word(0) if g.vm.as_ref().is_some_and(|h| h.insn_slot.is_some()) => {
            let p = g.p();
            let vd = &p.vms[g.vm.as_ref().unwrap().vm as usize];
            current_word(g, vd)
        }
        TVm::Word(_) => {
            let pl = g.place(e);
            g.load(&e.ty, &pl)
        }
        TVm::Fallthrough(size, part) => {
            let hg = g.vm.clone().unwrap();
            let p = g.p();
            let vd = &p.vms[hg.vm as usize];
            let pc = g.inst(format!("load ptr, ptr {}", hg.state_slots[vd.pc_idx]));
            let np = g.inst(format!("getelementptr inbounds i{}, ptr {}, i64 {}", vd.word.bits, pc, size));
            g.emit(format!("store ptr {}, ptr {}", np, hg.state_slots[vd.pc_idx]));
            // The next part decodes its own word.
            if let Some(slot) = &hg.insn_slot {
                let w = g.inst(format!("load i{}, ptr {}", vd.word.bits, np));
                g.emit(format!("store i{} {}, ptr {}", vd.word.bits, w, slot));
            }
            let l = part_label(g, *part);
            g.br(&l);
            g.poison(&e.ty)
        }
        TVm::TableAddr(t) => {
            let hg = g.vm.clone().unwrap();
            let p = g.p();
            let vd = &p.vms[hg.vm as usize];
            if hg.combined {
                // The labels table of the entry this code is inlined into.
                let sym = hg.labels_global.trim_start_matches("@\"").trim_end_matches(".labels\"").to_string();
                combined_table(&sym, *t as usize)
            } else {
                tail_table(vd, *t as usize)
            }
        }
        TVm::Chain(k) => {
            let hg = g.vm.clone().unwrap();
            let p = g.p();
            let vd = &p.vms[hg.vm as usize];
            if hg.combined {
                g.br(&hg.op_labels[*k as usize]);
            } else {
                let sym = format!("@\"{}\"", p.fns[vd.ops[*k as usize].fid as usize].symbol);
                let w = current_word(g, vd);
                tail_to(g, vd, &sym, &w);
            }
            g.poison(&e.ty)
        }
        TVm::Synced(sync, call) => {
            g.val(sync);
            g.val(call)
        }
        TVm::Part(j, body) => {
            let l = part_label(g, *j);
            g.label(&l);
            g.val(body)
        }
    }
}

/// A cold block in a tail-call op handler that ends in control words on
/// every path can move to its own function, reached by `musttail`: the hot
/// path then makes no calls and needs no stack frame.
fn can_outline(g: &FnGen, b: &TExpr) -> bool {
    let Some(hg) = &g.vm else { return false };
    if std::env::var_os("LOIMI_NO_OUTLINE").is_some() {
        return false;
    }
    if hg.combined || b.ty != Ty::Never {
        return false;
    }
    let p = g.p();
    match g.f().vm_role {
        VmRole::Handler(k) => p.vms[hg.vm as usize].ops[k as usize].parts.is_empty(),
        _ => false,
    }
}

fn outline(g: &mut FnGen, b: &TExpr) {
    let p = g.p();
    let f = g.f();
    let vd = &p.vms[g.vm.as_ref().unwrap().vm as usize];
    let n = g.cg.cold_count;
    g.cg.cold_count += 1;
    let sym = format!("{}.cold{}", f.symbol, n);
    let mut used = Vec::new();
    let mut declared = Vec::new();
    crate::tirwalk::locals(b, &mut used, &mut declared);
    let captured: Vec<LocalId> =
        used.into_iter().filter(|l| !declared.contains(l) && !f.vm_state.contains(l)).collect();
    for &l in &captured {
        let t = f.locals[l as usize].ty.clone();
        let gname = format!("@\"{}.c{}\"", sym, l);
        let _ = writeln!(g.cg.globals, "{} = internal thread_local(initialexec) global {} zeroinitializer", gname, llty(p, &t));
        let slot = g.slots[l as usize].clone();
        let v = g.load(&t, &slot);
        g.store(&t, &v, &gname);
    }
    let w = current_word(g, vd);
    tail_to(g, vd, &format!("@\"{}\"", sym), &w);
    g.cg.cold_queue.push((g.fid, b.clone(), captured, sym));
}

/// `--vm-stats`: count this op and the (previous op, this op) pair.
fn count_op(g: &mut FnGen, k: u32) {
    if !g.cg.opts.vm_stats {
        return;
    }
    let p = g.p();
    let vd = &p.vms[g.f().vm.unwrap() as usize];
    let n = vd.ops.len();
    let sym = &vd.symbol;
    let cp = g.inst(format!("getelementptr inbounds [{} x i64], ptr @\"{}.stats.counts\", i64 0, i64 {}", n, sym, k));
    let c = g.inst(format!("load i64, ptr {}", cp));
    let c1 = g.inst(format!("add i64 {}, 1", c));
    g.emit(format!("store i64 {}, ptr {}", c1, cp));
    // Only pairs where this op directly follows the previous one in the
    // code can become superinstructions.
    let f = g.f();
    let pc_slot = g.slots[f.vm_state[vd.pc_idx] as usize].clone();
    let pc = g.inst(format!("load ptr, ptr {}", pc_slot));
    let expected = g.inst(format!("load ptr, ptr @\"{}.stats.next\"", sym));
    let adjacent = g.inst(format!("icmp eq ptr {}, {}", pc, expected));
    let last0 = g.inst(format!("load i64, ptr @\"{}.stats.last\"", sym));
    let last = g.inst(format!("select i1 {}, i64 {}, i64 {}", adjacent, last0, n));
    let after = g.inst(format!("getelementptr inbounds i{}, ptr {}, i64 {}", vd.word.bits, pc, vd.ops[k as usize].size));
    g.emit(format!("store ptr {}, ptr @\"{}.stats.next\"", after, sym));
    let row = g.inst(format!("mul i64 {}, {}", last, n));
    let idx = g.inst(format!("add i64 {}, {}", row, k));
    let pp = g.inst(format!("getelementptr inbounds i64, ptr @\"{}.stats.pairs\", i64 {}", sym, idx));
    let pc = g.inst(format!("load i64, ptr {}", pp));
    let pc1 = g.inst(format!("add i64 {}, 1", pc));
    g.emit(format!("store i64 {}, ptr {}", pc1, pp));
    g.emit(format!("store i64 {}, ptr @\"{}.stats.last\"", k, sym));
}

/// `--vm-stats`: counters, and a report printed at exit (op counts, and
/// frequent pairs as `super` declarations).
pub fn emit_stats(cg: &mut crate::codegen::Codegen) {
    if !cg.opts.vm_stats {
        return;
    }
    let p = cg.p;
    let mut dtors = Vec::new();
    for vd in &p.vms {
        let n = vd.ops.len();
        let sym = vd.symbol.clone();
        let _ = writeln!(cg.globals, "@\"{}.stats.counts\" = internal global [{} x i64] zeroinitializer", sym, n);
        let _ = writeln!(cg.globals, "@\"{}.stats.pairs\" = internal global [{} x i64] zeroinitializer", sym, (n + 1) * n);
        // `last` starts at the sentinel row n (no previous op).
        let _ = writeln!(cg.globals, "@\"{}.stats.last\" = internal global i64 {}", sym, n);
        let _ = writeln!(cg.globals, "@\"{}.stats.next\" = internal global ptr null", sym);
        let mut names = Vec::new();
        for o in &vd.ops {
            let s = cg.string_const(format!("{}\0", o.name).as_bytes());
            names.push(format!("ptr {}", s));
        }
        let _ = writeln!(cg.globals, "@\"{}.stats.names\" = internal constant [{} x ptr] [{}]", sym, n, names.join(", "));
        // Whether each op may appear in a suggested pair (supers may not).
        let plain: Vec<String> = vd.ops.iter().map(|o| format!("i1 {}", o.parts.is_empty())).collect();
        let _ = writeln!(cg.globals, "@\"{}.stats.plain\" = internal constant [{} x i1] [{}]", sym, n, plain.join(", "));
        let head = cg.string_const(format!("\n== vm stats: {} ==\n%14s  %6s  op\n\0", vd.name).as_bytes());
        let row = cg.string_const(b"%14llu  %5.1f%%  %s\n\0");
        let pairs_head = cg.string_const(b"\nfrequent adjacent pairs (at least 1%% of executed ops), as superinstructions:\n\0");
        let pair_row = cg.string_const(b"    super %s_%s = %s; %s;    // %llu, %.1f%%\n\0");
        let count_hdr = cg.string_const(b"count\0");
        let pct_hdr = cg.string_const(b"%\0");
        let f = format!("@\"{}.stats.report\"", sym);
        let _ = writeln!(
            cg.fns,
            r#"define internal void {f}() {{
entry:
  %err = load ptr, ptr @stderr
  call i32 (ptr, ptr, ...) @fprintf(ptr %err, ptr {head}, ptr {count_hdr}, ptr {pct_hdr})
  br label %sum
sum:
  %si = phi i64 [ 0, %entry ], [ %si1, %sum ]
  %tot = phi i64 [ 0, %entry ], [ %tot1, %sum ]
  %scp = getelementptr inbounds [{n} x i64], ptr @"{sym}.stats.counts", i64 0, i64 %si
  %sc = load i64, ptr %scp
  %tot1 = add i64 %tot, %sc
  %si1 = add i64 %si, 1
  %sdone = icmp eq i64 %si1, {n}
  br i1 %sdone, label %ops, label %sum
ops:
  %totf = uitofp i64 %tot1 to double
  %totz = icmp eq i64 %tot1, 0
  br i1 %totz, label %done, label %oploop
oploop:
  %oi = phi i64 [ 0, %ops ], [ %oi1, %opnext ]
  %ocp = getelementptr inbounds [{n} x i64], ptr @"{sym}.stats.counts", i64 0, i64 %oi
  %oc = load i64, ptr %ocp
  %onz = icmp ne i64 %oc, 0
  br i1 %onz, label %opprint, label %opnext
opprint:
  %ocf = uitofp i64 %oc to double
  %opct0 = fmul double %ocf, 100.0
  %opct = fdiv double %opct0, %totf
  %onp = getelementptr inbounds [{n} x ptr], ptr @"{sym}.stats.names", i64 0, i64 %oi
  %on = load ptr, ptr %onp
  call i32 (ptr, ptr, ...) @fprintf(ptr %err, ptr {row}, i64 %oc, double %opct, ptr %on)
  br label %opnext
opnext:
  %oi1 = add i64 %oi, 1
  %odone = icmp eq i64 %oi1, {n}
  br i1 %odone, label %pairs, label %oploop
pairs:
  call i32 (ptr, ptr, ...) @fprintf(ptr %err, ptr {pairs_head})
  br label %ploop
ploop:
  %pi = phi i64 [ 0, %pairs ], [ %pi1, %pnext ]
  %pcp = getelementptr inbounds i64, ptr @"{sym}.stats.pairs", i64 %pi
  %pc = load i64, ptr %pcp
  %pc100 = mul i64 %pc, 100
  %pbig = icmp uge i64 %pc100, %tot1
  %pnz = icmp ne i64 %pc, 0
  %pok0 = and i1 %pbig, %pnz
  %pa = udiv i64 %pi, {n}
  %pb = urem i64 %pi, {n}
  %paplp = getelementptr inbounds [{n} x i1], ptr @"{sym}.stats.plain", i64 0, i64 %pa
  %papl = load i1, ptr %paplp
  %pbplp = getelementptr inbounds [{n} x i1], ptr @"{sym}.stats.plain", i64 0, i64 %pb
  %pbpl = load i1, ptr %pbplp
  %pok1 = and i1 %pok0, %papl
  %pok = and i1 %pok1, %pbpl
  br i1 %pok, label %pprint, label %pnext
pprint:
  %panp = getelementptr inbounds [{n} x ptr], ptr @"{sym}.stats.names", i64 0, i64 %pa
  %pa_name = load ptr, ptr %panp
  %pbnp = getelementptr inbounds [{n} x ptr], ptr @"{sym}.stats.names", i64 0, i64 %pb
  %pb_name = load ptr, ptr %pbnp
  %pcf = uitofp i64 %pc to double
  %ppct0 = fmul double %pcf, 100.0
  %ppct = fdiv double %ppct0, %totf
  call i32 (ptr, ptr, ...) @fprintf(ptr %err, ptr {pair_row}, ptr %pa_name, ptr %pb_name, ptr %pa_name, ptr %pb_name, i64 %pc, double %ppct)
  br label %pnext
pnext:
  %pi1 = add i64 %pi, 1
  %pdone = icmp eq i64 %pi1, {nn}
  br i1 %pdone, label %done, label %ploop
done:
  ret void
}}
"#,
            nn = n * n,
        );
        dtors.push(format!("{{ i32, ptr, ptr }} {{ i32 65535, ptr {}, ptr null }}", f));
    }
    if !dtors.is_empty() {
        let _ = writeln!(cg.globals, "@llvm.global_dtors = appending global [{} x {{ i32, ptr, ptr }}] [{}]", dtors.len(), dtors.join(", "));
        let _ = writeln!(cg.globals, "@stderr = external global ptr");
        cg.decls.insert("declare i32 @fprintf(ptr, ptr, ...)");
    }
}

fn word_mask(bits: u8) -> i64 {
    if bits >= 64 { -1 } else { ((1u64 << bits) - 1) as i64 }
}

fn part_label(g: &mut FnGen, j: u32) -> String {
    if let Some(l) = g.vm.as_ref().unwrap().parts.get(&j) {
        return l.clone();
    }
    let l = g.fresh_label(&format!("part{}_", j));
    g.vm.as_mut().unwrap().parts.insert(j, l.clone());
    l
}

/// Module-level definitions for tail-call vms: dispatch table, invalid-op
/// handler, raise slot.
pub fn emit_tables(p: &Program, globals: &mut String, fns: &mut String) {
    // Calls to this mark a block as cold. The empty asm keeps the call from
    // being deleted; preserve_all keeps it from clobbering any register.
    let _ = writeln!(
        fns,
        "define internal preserve_allcc void @loimi.cold() cold noinline nounwind {{\n  call void asm sideeffect \"\", \"\"()\n  ret void\n}}\n"
    );
    for vd in &p.vms {
        if vd.dispatch == Dispatch::TailCall {
            let n = 1usize << vd.op_bits;
            let inv = format!("@\"{}.invalid\"", vd.symbol);
            let mut entries = Vec::new();
            for i in 0..n {
                match vd.ops.iter().find(|o| o.opcode as usize == i) {
                    Some(o) => entries.push(format!("ptr @\"{}\"", p.fns[o.fid as usize].symbol)),
                    None => entries.push(format!("ptr {}", inv)),
                }
            }
            let _ = writeln!(globals, "{} = internal constant [{} x ptr] [{}]", table_name(vd), n, entries.join(", "));
            for (v, (_, ws)) in vd.variants.iter().enumerate() {
                let mut entries = Vec::new();
                for i in 0..n {
                    match vd.ops.iter().position(|o| o.opcode as usize == i) {
                        Some(k) => {
                            let f = ws.iter().find(|(w, _)| *w as usize == k).map(|(_, f)| *f).unwrap_or(vd.ops[k].fid);
                            entries.push(format!("ptr @\"{}\"", p.fns[f as usize].symbol));
                        }
                        None => entries.push(format!("ptr {}", inv)),
                    }
                }
                let _ = writeln!(globals, "{} = internal constant [{} x ptr] [{}]", tail_table(vd, v + 1), n, entries.join(", "));
            }
            let mut params: Vec<String> = vd.state_tys.iter().map(|t| llty(p, t)).collect();
            params.push(format!("i{}", vd.word.bits));
            let _ = writeln!(
                fns,
                "define internal preserve_nonecc {} {}({}) noreturn nounwind {{\n  call void @llvm.trap()\n  unreachable\n}}\n",
                ret_llty(p, &vd.ret),
                inv,
                params.join(", ")
            );
            if let Some(rt) = &vd.raise_ty {
                let _ = writeln!(globals, "@\"{}.raise_slot\" = internal thread_local(initialexec) global {} zeroinitializer", vd.symbol, llty(p, rt));
            }
        }
    }
}

/// Whether a function is emitted on its own (handlers of combined vms are
/// emitted inside their entries).
pub fn standalone(p: &Program, f: &FnDef) -> bool {
    match (f.vm, f.vm_role) {
        (Some(vm), VmRole::Handler(_) | VmRole::Raise | VmRole::Wrapper(..)) => p.vms[vm as usize].dispatch == Dispatch::TailCall,
        _ => true,
    }
}

pub fn test_value_variant(g: &mut FnGen, t: &Ty, idx: u32, subs: &[TPat], ptr: &str, fail: &str) {
    crate::values::test_variant(g, t, idx, subs, ptr, fail)
}

pub fn make_value(g: &mut FnGen, t: &Ty, idx: u32, args: &[TExpr], vals: &[String]) -> String {
    crate::values::make(g, t, idx, args, vals)
}

pub fn raise_value(g: &mut FnGen, err: &str, t: &Ty, _span: Span) {
    let hg = g.vm.clone().unwrap();
    let p = g.p();
    let vd = &p.vms[hg.vm as usize];
    g.store(t, err, &hg.raise_slot);
    if hg.combined {
        g.br(&hg.raise_label);
    } else {
        let sym = format!("@\"{}\"", p.fns[vd.on_raise.unwrap() as usize].symbol);
        let w = current_word(g, vd);
        tail_to(g, vd, &sym, &w);
    }
}


