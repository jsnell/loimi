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
    pub raise_slot: String,
    pub labels_global: String,
    /// Tail-call handlers receive the current instruction word in a register.
    pub insn_slot: Option<String>,
}

fn takes_insn(p: &Program, f: &FnDef) -> bool {
    match (f.vm, f.vm_role) {
        (Some(vm), VmRole::Handler(_) | VmRole::Raise) => p.vms[vm as usize].dispatch == Dispatch::TailCall,
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
    if combined {
        hg.dispatch_label = g.fresh_label("dispatch");
        hg.raise_label = g.fresh_label("raise");
        hg.invalid_label = g.fresh_label("invalid");
        hg.op_labels = vd.ops.iter().map(|o| format!("op.{}.{}", o.name, o.opcode)).collect();
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
    let mut emit_body = |g: &mut FnGen, fid: FnId, label: &str| {
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
    if vd.dispatch == Dispatch::Switch {
        g.label(&hg.dispatch_label);
        let pc = g.inst(format!("load ptr, ptr {}", hg.state_slots[vd.pc_idx]));
        let opc = opcode_of(g, vd, &pc);
        let wt = format!("i{}", vd.word.bits);
        let mut cases = String::new();
        for (k, op) in vd.ops.iter().enumerate() {
            let _ = write!(cases, " {} {}, label %{}", wt, op.opcode, hg.op_labels[k]);
        }
        let opc_w = if vd.word.bits == 64 { opc } else { g.inst(format!("trunc i64 {} to {}", opc, wt)) };
        g.term(format!("switch {} {}, label %{} [{} ]", wt, opc_w, hg.invalid_label, cases));
    } else {
        let sym = &g.f().symbol;
        let mut entries = Vec::new();
        for i in 0..n {
            let l = match vd.ops.iter().position(|o| o.opcode as usize == i) {
                Some(k) => &hg.op_labels[k],
                None => &hg.invalid_label,
            };
            entries.push(format!("ptr blockaddress(@\"{}\", %{})", sym, l));
        }
        let _ = writeln!(g.cg.globals, "{} = internal constant [{} x ptr] [{}]", hg.labels_global, n, entries.join(", "));
    }
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
            let n = 1usize << vd.op_bits;
            let a = g.inst(format!("getelementptr inbounds [{} x ptr], ptr {}, i64 0, i64 {}", n, hg.labels_global, opc));
            let h = g.inst(format!("load ptr, ptr {}", a));
            let mut dests: Vec<String> = hg.op_labels.iter().map(|l| format!("label %{}", l)).collect();
            dests.push(format!("label %{}", hg.invalid_label));
            g.term(format!("indirectbr ptr {}, [{}]", h, dests.join(", ")));
        }
    } else {
        let (opc, word) = opcode_and_word(g, vd, target);
        let n = 1usize << vd.op_bits;
        let a = g.inst(format!("getelementptr inbounds [{} x ptr], ptr {}, i64 0, i64 {}", n, table_name(vd), opc));
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
            let l = part_label(g, *part);
            g.br(&l);
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
                let _ = writeln!(globals, "@\"{}.raise_slot\" = internal thread_local global {} zeroinitializer", vd.symbol, llty(p, rt));
            }
        }
    }
}

/// Whether a function is emitted on its own (handlers of combined vms are
/// emitted inside their entries).
pub fn standalone(p: &Program, f: &FnDef) -> bool {
    match (f.vm, f.vm_role) {
        (Some(vm), VmRole::Handler(_) | VmRole::Raise) => p.vms[vm as usize].dispatch == Dispatch::TailCall,
        _ => true,
    }
}

pub fn test_value_variant(g: &mut FnGen, t: &Ty, idx: u32, subs: &[TPat], ptr: &str, fail: &str) {
    crate::values::test_variant(g, t, idx, subs, ptr, fail)
}

pub fn make_value(g: &mut FnGen, t: &Ty, idx: u32, args: &[TExpr], vals: &[String]) -> String {
    crate::values::make(g, t, idx, args, vals)
}

pub fn value_tag(_g: &mut FnGen, _a: &TExpr, _v: &str) -> String {
    unreachable!()
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


