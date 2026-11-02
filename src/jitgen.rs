//! `--jit-stencils`: copy-and-patch stencils generated from a vm's own
//! handlers (docs/design/copy-and-patch.md).
//!
//! Each op handler is emitted a second time as a *stencil*: the same body,
//! with
//! - every operand field decoded from the instruction word replaced by a
//!   hole, the address of an absolute symbol `__jit.f.OFF.SHIFT.BITS.S`
//!   (patched with the field's value: unsigned fields times 8, signed
//!   ones biased by 2^(BITS-1), so the symbol's range is non-negative and
//!   fits a 32-bit immediate or displacement);
//! - the instruction word itself (passed to cold blocks and `on raise`)
//!   the hole `__jit.w.0`;
//! - `next` (and any `jump pc + d`) a tail call to `__jit.n.D`, patched
//!   with the address of the stencil copy for the instruction at pc + d;
//! - `jump` to a label operand a tail call to `__jit.j.OFF.SHIFT.BITS`,
//!   patched with the copy for pc + OFF + field;
//! - any other jump the interpreter's dispatch.
//!
//! The stencils are compiled to an object file (`elf.rs` reads it back),
//! and the run-time JIT (`jit_rt.c`) copies and patches them. Everything
//! else (cold blocks, helpers, tables) stays in the interpreter: stencils
//! reach it through relocations against its symbols.

use std::fmt::Write;

use crate::codegen::{llty, FnGen};
use crate::tir::*;

/// Size of the hot-counter table (`loimi_jit_hotcount`), a power of two.
pub const HOT_SLOTS: u32 = 4096;

#[derive(Clone, Debug)]
pub struct StencilGen {
    pub symbol: String,
    /// The interpreter handler's outlined cold blocks, in emission order:
    /// the stencil calls the same functions.
    pub cold_syms: Vec<String>,
    pub cold_next: usize,
    /// Per part (one for a plain op): its signed operand fields (word
    /// offset from the part, shift, bits), jump targets
    /// `pc + off + field` that become direct jumps.
    pub labels: Vec<Vec<(u32, u32, u32)>>,
    /// Word offset of each part from the instruction's first word.
    pub part_offs: Vec<u32>,
    /// The part being emitted.
    pub part: usize,
}

/// The vm that `--jit-stencils` compiles: the program's only tail-call vm.
pub fn jit_vm(p: &Program) -> Result<usize, String> {
    let vms: Vec<usize> = (0..p.vms.len()).filter(|&i| p.vms[i].dispatch == Dispatch::TailCall).collect();
    match vms.as_slice() {
        [v] => {
            let vd = &p.vms[*v];
            if vd.op_shift != 0 || vd.word.bits != 32 || vd.op_bits > 16 {
                return Err(format!("--jit-stencils: vm {} needs 32-bit words with the opcode in the low bits", vd.name));
            }
            if vd.tbl_idx.is_some() {
                return Err(format!("--jit-stencils: vm {} has variants (not supported by the prototype)", vd.name));
            }
            Ok(*v)
        }
        [] => Err("--jit-stencils: the program has no vm with tail-call dispatch".into()),
        _ => Err("--jit-stencils: the prototype supports one vm per program".into()),
    }
}

/// Whether `g` emits code that counts for the JIT (and enters compiled
/// code directly): a counting copy or a stencil.
pub fn counting(g: &FnGen) -> bool {
    g.jit_count || g.stencil.is_some()
}

/// The symbol of a function's counting copy.
pub fn counting_symbol(sym: &str) -> String {
    format!("{}.jc", sym)
}

/// The handler-convention function `fid` as a direct tail call from `g`
/// reaches it: its counting copy from counting code, if it has one.
pub fn callee_symbol(g: &FnGen, fid: FnId) -> String {
    let sym = &g.p().fns[fid as usize].symbol;
    if counting(g) && g.cg.jit_counted.contains(&fid) { counting_symbol(sym) } else { sym.clone() }
}

/// The functions of vm `vi` that get a counting copy in a JIT build: op
/// handlers and shared `-> never` functions that make a jump the JIT
/// counts or intercepts (a call, a return, a label jump) or a `become`
/// (cold blocks included), and those that tail-call one of them directly.
/// Everything else dispatches through the table, which the runtime points
/// at the counting copies when the JIT is on. With the JIT off the
/// interpreter is therefore the default build's: the copies are never
/// reached. `on raise` does not count: its jumps (to a handler) dispatch
/// normally, and reach compiled code through the enter handler.
pub fn counted_fns(p: &Program, vi: usize) -> std::collections::HashSet<FnId> {
    let vd = &p.vms[vi];
    let mut refs: Vec<(FnId, Vec<FnId>)> = Vec::new();
    let mut out = std::collections::HashSet::new();
    for (fid, f) in p.fns.iter().enumerate() {
        if f.vm != Some(vi as u32) || !matches!(f.vm_role, VmRole::Handler(_) | VmRole::Shared(_)) {
            continue;
        }
        let Some(body) = &f.body else { continue };
        let pc_local = f.vm_state[vd.pc_idx];
        let mut base = false;
        let mut callees = Vec::new();
        crate::tirwalk::for_each_expr(body, &mut |e| match &e.kind {
            TK::Vm(TVm::Jump(t)) => {
                if !matches!(jump_kind(t, pc_local), JumpKind::Const(_)) {
                    base = true;
                }
            }
            TK::Vm(TVm::Become(..)) => base = true,
            TK::Vm(TVm::Shared(c, ..)) => callees.push(*c),
            TK::Vm(TVm::Chain(k)) => callees.push(vd.ops[*k as usize].fid),
            _ => {}
        });
        if base {
            out.insert(fid as FnId);
        }
        refs.push((fid as FnId, callees));
    }
    loop {
        let n = out.len();
        for (f, cs) in &refs {
            if cs.iter().any(|c| out.contains(c)) {
                out.insert(*f);
            }
        }
        if out.len() == n {
            return out;
        }
    }
}

/// The opcode the JIT writes over instructions that have compiled code:
/// the highest one no op uses.
pub fn enter_opcode(vd: &VmDef) -> u32 {
    let n = 1u32 << vd.op_bits;
    (0..n).rev().find(|x| !vd.ops.iter().any(|o| o.opcode == *x)).expect("--jit-stencils needs a free opcode")
}

/// Matches the operand decode `vmcheck::decode` builds: (word offset,
/// shift, bits, signed).
pub fn match_decode(e: &TExpr, wb: u32) -> Option<(u32, u32, u32, bool)> {
    fn int(e: &TExpr) -> Option<u32> {
        match e.kind {
            TK::Int(v) => Some(v as u32),
            _ => None,
        }
    }
    fn word(e: &TExpr) -> Option<u32> {
        match e.kind {
            TK::Vm(TVm::Word(off)) => Some(off),
            _ => None,
        }
    }
    let inner = match &e.kind {
        TK::Cast(a) => a,
        _ => e,
    };
    // Signed: ((w << up) as sN) >> (wb - bits)
    if let TK::Binary(crate::ast::BinOp::Shr, a, s) = &inner.kind {
        if let TK::Cast(up) = &a.kind {
            if let TK::Binary(crate::ast::BinOp::Shl, w, u) = &up.kind {
                let (off, u, s) = (word(w)?, int(u)?, int(s)?);
                let bits = wb - s;
                let shift = wb - u - bits;
                return Some((off, shift, bits, true));
            }
        }
    }
    // Unsigned: ((w >> shift) & mask), either part optional.
    let (m, mask) = match &inner.kind {
        TK::Binary(crate::ast::BinOp::And, a, m) => (&**a, Some(int(m)?)),
        _ => (inner, None),
    };
    let (w, shift) = match &m.kind {
        TK::Binary(crate::ast::BinOp::Shr, w, s) => (&**w, int(s)?),
        _ => (m, 0),
    };
    let off = word(w)?;
    let bits = match mask {
        Some(m) => (m as u64 + 1).trailing_zeros(),
        None => wb - shift,
    };
    Some((off, shift, bits, false))
}

fn hole_name(off: u32, shift: u32, bits: u32, signed: bool) -> String {
    format!("__jit.f.{}.{}.{}.{}", off, shift, bits, if signed { "s" } else { "u" })
}

/// The value of an operand field in a stencil (an i64).
pub fn field_hole(g: &mut FnGen, off: u32, shift: u32, bits: u32, signed: bool) -> String {
    let name = hole_name(off, shift, bits, signed);
    let range = 1u64 << bits;
    let hi = if signed { range } else { range * 8 - 7 };
    g.cg.jit_holes.insert(name.clone(), hi);
    let h = g.inst(format!("ptrtoint ptr @\"{}\" to i64", name));
    if signed {
        g.inst(format!("sub i64 {}, {}", h, range / 2))
    } else {
        g.inst(format!("lshr exact i64 {}, 3", h))
    }
}

/// The instruction word at pc + off in a stencil, as a constant.
pub fn word_hole(g: &mut FnGen, off: u32) -> String {
    let name = format!("__jit.w.{}", off);
    g.cg.jit_holes.insert(name.clone(), 1u64 << 32);
    format!("ptrtoint (ptr @\"{}\" to i32)", name)
}

/// The signed decoded fields anywhere in a handler body.
pub fn signed_fields(body: &TExpr, wb: u32) -> Vec<(u32, u32, u32)> {
    let mut out = Vec::new();
    crate::tirwalk::for_each_expr(body, &mut |e| {
        if let TK::Block(stmts, _) = &e.kind {
            for s in stmts {
                if let TStmt::Let(_, Some(init)) = s {
                    if let Some((off, shift, bits, true)) = match_decode(init, wb) {
                        if !out.contains(&(off, shift, bits)) {
                            out.push((off, shift, bits));
                        }
                    }
                }
            }
        }
    });
    out
}

/// What a `jump` targets, for direct jumps and hot counting.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum JumpKind {
    /// `pc + d` for a constant d (`next`, `redo`, skipping a `then` word).
    Const(i64),
    /// Computed from pc and operands (a label), possibly through locals.
    PcRel,
    /// The start of a verified code object (`jump p.code`: a call).
    Entry,
    /// Read from memory (returns).
    Dynamic,
}

pub fn jump_kind(t: &TExpr, pc_local: LocalId) -> JumpKind {
    match &t.kind {
        TK::Intrinsic(Intr::PtrOffset, args) => {
            if let (TK::Local(l), TK::Int(d)) = (&args[0].kind, &args[1].kind) {
                if *l == pc_local {
                    return JumpKind::Const(*d as i64);
                }
            }
            JumpKind::PcRel
        }
        TK::Local(l) if *l == pc_local => JumpKind::Const(0),
        TK::Local(_) | TK::Binary(..) => JumpKind::PcRel,
        TK::Block(_, Some(tail)) => match jump_kind(tail, pc_local) {
            JumpKind::Const(_) => JumpKind::PcRel,
            k => k,
        },
        TK::Field(code, 0) if matches!(code.ty, Ty::Slice(_)) => JumpKind::Entry,
        _ => JumpKind::Dynamic,
    }
}

/// A dynamic jump (call, return) in a JIT build: if the target holds an
/// enter word, jump straight to its compiled code, without counting.
/// Returns with the insertion point on the other path.
pub fn enter_if_compiled(g: &mut FnGen, vd: &VmDef, target: &str) {
    let hg = g.vm.clone().unwrap();
    let w = g.inst(format!("load i32, ptr {}", target));
    let mask = (1u32 << vd.op_bits) - 1;
    let op = g.inst(format!("and i32 {}, {}", w, mask));
    let is = g.inst(format!("icmp eq i32 {}, {}", op, enter_opcode(vd)));
    let yes = g.fresh_label("enter");
    let no = g.fresh_label("notcompiled");
    g.cond_br(&is, &yes, &no);
    g.label(&yes);
    let i = g.inst(format!("lshr i32 {}, {}", w, vd.op_bits));
    let iz = g.inst(format!("zext i32 {} to i64", i));
    let b = g.inst("load ptr, ptr @loimi_jit_entries");
    let ep = g.inst(format!("getelementptr inbounds ptr, ptr {}, i64 {}", b, iz));
    let e = g.inst(format!("load ptr, ptr {}", ep));
    g.emit(format!("store ptr {}, ptr {}", target, hg.state_slots[vd.pc_idx]));
    crate::vmgen::tail_call(g, vd, &e, &w);
    g.label(&no);
}

/// Hot counting before a dispatch to `target` (counting copies and
/// stencils): decrement the counter of the target's hash slot; at
/// zero, continue in `Vm.jit_hot`, which may compile the code there.
/// `backward_only`: count only if the target is at or before pc (loops).
/// Returns with the insertion point on the path that dispatches normally.
pub fn hot_count(g: &mut FnGen, vd: &VmDef, target: &str, backward_only: bool) {
    let hg = g.vm.clone().unwrap();
    let go = g.fresh_label("hotgo");
    if backward_only {
        let pc = g.inst(format!("load ptr, ptr {}", hg.state_slots[vd.pc_idx]));
        let back = g.inst(format!("icmp ule ptr {}, {}", target, pc));
        let cnt = g.fresh_label("hotcnt");
        g.cond_br(&back, &cnt, &go);
        g.label(&cnt);
    }
    let ti = g.inst(format!("ptrtoint ptr {} to i64", target));
    let hi = g.inst(format!("lshr i64 {}, 2", ti));
    let hm = g.inst(format!("and i64 {}, {}", hi, HOT_SLOTS - 1));
    let hp = g.inst(format!("getelementptr inbounds i16, ptr @loimi_jit_hotcount, i64 {}", hm));
    let c = g.inst(format!("load i16, ptr {}", hp));
    let c1 = g.inst(format!("sub i16 {}, 1", c));
    g.emit(format!("store i16 {}, ptr {}", c1, hp));
    let z = g.inst(format!("icmp eq i16 {}, 0", c1));
    let z = g.inst(format!("call i1 @llvm.expect.i1(i1 {}, i1 false)", z));
    g.cg.decls.insert("declare i1 @llvm.expect.i1(i1, i1)");
    let trig = g.fresh_label("hottrig");
    g.cond_br(&z, &trig, &go);
    g.label(&trig);
    g.emit(format!("store ptr {}, ptr {}", target, hg.state_slots[vd.pc_idx]));
    let sym = format!("@\"{}.jit_hot\"", vd.symbol);
    crate::vmgen::tail_call(g, vd, &sym, "0");
    g.label(&go);
}

/// Module-level pieces of a JIT build (in the interpreter's module): the
/// enter handler, the hot trigger, op metadata, and the runtime's
/// declarations. The enter opcode's table slot is filled by `emit_tables`.
pub fn emit_runtime(p: &Program, vi: usize, globals: &mut String, fns: &mut String) {
    let vd = &p.vms[vi];
    let rt = crate::vmgen::handler_ret_llty(p, vd);
    let mut params: Vec<String> = vd.state_tys.iter().enumerate().map(|(i, t)| format!("{} %p{}", llty(p, t), i)).collect();
    params.push("i32 %insn".into());
    let args = params.join(", ");
    let call = |callee: &str, insn: &str| -> String {
        let mut a: Vec<String> = vd.state_tys.iter().enumerate().map(|(i, t)| format!("{} %p{}", llty(p, t), i)).collect();
        a.push(format!("i32 {}", insn));
        if rt == "void" {
            format!("  musttail call preserve_nonecc void {}({})\n  ret void\n", callee, a.join(", "))
        } else {
            format!("  %r = musttail call preserve_nonecc {} {}({})\n  ret {} %r\n", rt, callee, a.join(", "), rt)
        }
    };
    let table = crate::vmgen::table_name(vd);
    let n = 1u32 << vd.op_bits;
    let mask = n - 1;
    // An instruction the JIT replaced: its payload indexes the entry table.
    let _ = writeln!(
        fns,
        "define internal preserve_nonecc {rt} @\"{sym}.jit_enter\"({args}) nounwind {{\n  %i = lshr i32 %insn, {bits}\n  %iz = zext i32 %i to i64\n  %b = load ptr, ptr @loimi_jit_entries\n  %ep = getelementptr inbounds ptr, ptr %b, i64 %iz\n  %e = load ptr, ptr %ep\n{}}}\n",
        call("%e", "%insn"),
        sym = vd.symbol,
        bits = vd.op_bits,
    );
    // A hot counter reached zero at pc: maybe compile, then dispatch
    // (to the compiled code if the JIT replaced the word at pc).
    let _ = writeln!(
        fns,
        "define internal preserve_nonecc {rt} @\"{sym}.jit_hot\"({args}) nounwind noinline cold {{\n  call void @loimi_jit_hot(ptr %p{pc})\n  %w = load i32, ptr %p{pc}\n  %o = and i32 %w, {mask}\n  %oz = zext i32 %o to i64\n  %hp = getelementptr inbounds ptr, ptr {table}, i64 %oz\n  %h = load ptr, ptr %hp\n{}}}\n",
        call("%h", "%w"),
        sym = vd.symbol,
        pc = vd.pc_idx,
    );
    let _ = writeln!(globals, "@loimi_jit_entries = external global ptr");
    let _ = writeln!(globals, "@loimi_jit_hotcount = external global [{} x i16]", HOT_SLOTS);
    let _ = writeln!(fns, "declare void @loimi_jit_hot(ptr)\ndeclare void @loimi_jit_become(ptr)");
    // Per opcode: { the size of its first word's instruction (a super's
    // first part: the later parts' words are instructions too), the
    // opcode whose stencil to use (0xffff: invalid), flags, the size of
    // the whole op, the stencil to use if that one is missing (a super's
    // first part) }. Flag 1: reads the next word as its `then` operand;
    // 2: a super (the words of its later parts must keep their opcodes).
    let mut info = Vec::new();
    let ty = "{ i16, i16, i16, i16, i16 }";
    for i in 0..n {
        let e = match vd.ops.iter().find(|o| o.opcode == i) {
            Some(o) => {
                let first = match o.parts.first() {
                    Some(&b) => &vd.ops[b as usize],
                    None => o,
                };
                let flags = (first.then as u32) | ((!o.parts.is_empty() as u32) << 1);
                format!("{ty} {{ i16 {}, i16 {}, i16 {}, i16 {}, i16 {} }}", first.size, o.opcode, flags, o.size, first.opcode)
            }
            None => format!("{ty} {{ i16 0, i16 -1, i16 0, i16 0, i16 -1 }}"),
        };
        info.push(e);
    }
    let _ = writeln!(globals, "@loimi_jit_opinfo = constant [{} x {ty}] [{}]", n, info.join(", "));
    let _ = writeln!(globals, "@loimi_jit_nops = constant i32 {}", n);
    // Op names, for LOIMI_JIT_DEBUG.
    let mut names = Vec::new();
    for i in 0..n {
        let nm = vd.ops.iter().find(|o| o.opcode == i).map(|o| o.name.as_str()).unwrap_or("?");
        let _ = writeln!(globals, "@loimi_jit_opname.{} = private constant [{} x i8] c\"{}\\00\"", i, nm.len() + 1, nm);
        names.push(format!("ptr @loimi_jit_opname.{}", i));
    }
    let _ = writeln!(globals, "@loimi_jit_opnames = constant [{} x ptr] [{}]", n, names.join(", "));
    let _ = writeln!(globals, "@loimi_jit_enter_op = constant i32 {}", enter_opcode(vd));
    let _ = writeln!(globals, "@loimi_jit_op_bits = constant i32 {}", vd.op_bits);
}

/// The generic exit stencil (for ops without one): dispatch the
/// instruction at pc through the interpreter's table.
pub fn exit_stencil(p: &Program, vi: usize) -> String {
    let vd = &p.vms[vi];
    let rt = crate::vmgen::handler_ret_llty(p, vd);
    let params: Vec<String> = vd.state_tys.iter().enumerate().map(|(i, t)| format!("{} %p{}", llty(p, t), i)).collect();
    let mut a = params.clone();
    a.push("i32 %w".into());
    let mask = (1u32 << vd.op_bits) - 1;
    let tail = if rt == "void" {
        format!("  musttail call preserve_nonecc void %h({})\n  ret void\n", a.join(", "))
    } else {
        format!("  %r = musttail call preserve_nonecc {} %h({})\n  ret {} %r\n", rt, a.join(", "), rt)
    };
    format!(
        "define preserve_nonecc {rt} @\"__jit.exit\"({}, i32 %insn) nounwind {{\n  %w = load i32, ptr %p{pc}\n  %o = and i32 %w, {mask}\n  %oz = zext i32 %o to i64\n  %hp = getelementptr inbounds ptr, ptr {table}, i64 %oz\n  %h = load ptr, ptr %hp\n{tail}}}\n",
        params.join(", "),
        pc = vd.pc_idx,
        table = crate::vmgen::table_name(vd),
    )
}

/// Emit every op's stencil (not supers: the JIT compiles their parts).
pub fn emit_stencils(cg: &mut crate::codegen::Codegen) {
    let p = cg.p;
    let Ok(vi) = jit_vm(p) else { return };
    let vd = &p.vms[vi];
    let mut out = String::new();
    let wb = vd.word.bits as u32;
    for o in &vd.ops {
        let f = &p.fns[o.fid as usize];
        let body = f.body.as_ref().unwrap();
        // A super's body is a block of `Part`s; each has its own fields.
        let (labels, part_offs) = if o.parts.is_empty() {
            (vec![signed_fields(body, wb)], vec![0])
        } else {
            let mut ls = vec![vec![]; o.parts.len()];
            crate::tirwalk::for_each_expr(body, &mut |e| {
                if let TK::Vm(TVm::Part(j, b)) = &e.kind {
                    ls[*j as usize] = signed_fields(b, wb);
                }
            });
            let mut offs = Vec::new();
            let mut off = 0;
            for &q in &o.parts {
                offs.push(off);
                off += vd.ops[q as usize].size;
            }
            (ls, offs)
        };
        let mut g = FnGen::new(cg, o.fid);
        g.stencil = Some(StencilGen {
            symbol: format!("__jit.op.{}", o.name),
            cold_syms: g.cg.cold_syms.get(&o.fid).cloned().unwrap_or_default(),
            cold_next: 0,
            labels,
            part_offs,
            part: 0,
        });
        out.push_str(&g.run());
    }
    out.push_str(&exit_stencil(p, vi));
    let mut md = String::new();
    for (i, (name, hi)) in cg.jit_holes.iter().enumerate() {
        // extern_weak: LLVM must not assume the address (the value) is
        // not null, as it does for other globals.
        let _ = writeln!(out, "@\"{}\" = extern_weak dso_local global i8, !absolute_symbol !{}", name, i);
        let _ = writeln!(md, "!{} = !{{i64 0, i64 {}}}", i, hi);
    }
    for line in cg.jit_conts.values() {
        let _ = writeln!(out, "{}", line);
    }
    out.push_str(&md);
    cg.stencils = out;
}

/// The stencil module: the interpreter's IR, plus the stencils.
/// - Globals and handler-convention functions (cold blocks, `on raise`,
///   shared `-> never` functions) become `available_externally`: the
///   stencils use the interpreter's own, by name.
/// - Other functions (helpers) stay `internal`. The ones the stencils call
///   and do not inline are compiled into the object, and `elf.rs` turns
///   them into helper code that the JIT copies once. The interpreter
///   therefore never has to take a helper's address for the stencils,
///   which would change how LLVM inlines it there.
/// - Other definitions (`main`, exports) become `available_externally`,
///   so that only what the stencils reach is compiled.
pub fn stencil_module(ir: &str, stencils: &str) -> String {
    let mut out = String::with_capacity(ir.len() + stencils.len());
    for line in ir.lines() {
        if let Some(rest) = line.strip_prefix("define internal ") {
            if rest.starts_with("preserve_nonecc ") {
                out.push_str("define available_externally dso_local ");
                out.push_str(rest);
            } else {
                // No jump tables: they are data with relocations.
                out.push_str("define internal ");
                match rest.strip_suffix('{') {
                    Some(head) => {
                        out.push_str(head);
                        out.push_str("\"no-jump-tables\"=\"true\" {");
                    }
                    None => out.push_str(rest),
                }
            }
        } else if let Some(rest) = line.strip_prefix("define ") {
            out.push_str("define available_externally ");
            out.push_str(rest);
        } else if line.starts_with('@') && line.contains(" = internal ") {
            // dso_local: the stencils address the program's symbols
            // directly (absolute or pc-relative), not through a GOT.
            out.push_str(&line.replacen(" = internal ", " = available_externally dso_local ", 1).replace("thread_local(initialexec)", "thread_local(localexec)"));
        } else if line.starts_with('@') && line.contains(" = external global ") {
            out.push_str(&line.replacen(" = external global ", " = external dso_local global ", 1));
        } else {
            out.push_str(line);
        }
        out.push('\n');
    }
    out.push_str(stencils);
    out
}
