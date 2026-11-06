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
    /// Per part: its fields of `const` operand kinds (shift, bits).
    pub consts: Vec<Vec<(u32, u32)>>,
    /// The constant pool's state local (`k`), if the vm has const kinds.
    pub pool: Option<LocalId>,
    /// Raw locals of const operands decoded so far: their hole (word
    /// offset from the head, shift, bits).
    pub const_raws: Vec<(LocalId, (u32, u32, u32))>,
    /// A variant stencil for constants of one tag: the constant's bits
    /// satisfy `bits & mask == value` (or `bits < value` if `ult`).
    pub variant: Option<(u64, u64, bool)>,
    /// The register file's state local (`regs`), for `LOIMI_JIT_REGDELAY`.
    pub regs: Option<LocalId>,
}

/// A stencil variant of an op for constants of one tag of a value type
/// (`copy-and-patch.md` section 12): the op's handler with its const
/// operand's tag known, so that LLVM folds the checks on it. The runtime
/// picks it when the constant matches (`loimi_jit_variants`).
#[derive(Clone, Debug)]
pub struct ConstVariant {
    /// The op's index in the vm.
    pub op: usize,
    /// Stencil name suffix: `OP.v.Tag`.
    pub name: String,
    /// The const field (shift, bits).
    pub field: (u32, u32),
    pub mask: u64,
    pub value: u64,
    pub ult: bool,
}

/// Whether to emit stencil variants per constant tag (on unless
/// `LOIMI_JIT_VARIANTS=0` at build time).
pub fn variants_enabled() -> bool {
    std::env::var_os("LOIMI_JIT_VARIANTS").is_none_or(|v| v != "0")
}

/// The stencil variants of vm `vi`: for each op (not a super) with one
/// const operand of a one-word value type, one per variant of that type
/// its handler tests for (the variants named in its patterns).
pub fn const_variants(p: &Program, vi: usize) -> Vec<ConstVariant> {
    let vd = &p.vms[vi];
    let mut out = Vec::new();
    if !variants_enabled() {
        return out;
    }
    let Some(b) = vd.const_base else { return out };
    let Ty::Ptr(elem) = &vd.state_tys[b] else { return out };
    let Ty::Value(vix) = **elem else { return out };
    let d = &p.values[vix as usize];
    if d.repr == ValueRepr::Boxed {
        return out;
    }
    fn tags(pat: &TPat, vix: u32, out: &mut Vec<u32>) {
        match &pat.kind {
            TPK::Variant(i, subs) => {
                if pat.ty == Ty::Value(vix) && !out.contains(i) {
                    out.push(*i);
                }
                subs.iter().for_each(|s| tags(s, vix, out));
            }
            TPK::Tuple(xs) | TPK::Or(xs) => xs.iter().for_each(|s| tags(s, vix, out)),
            _ => {}
        }
    }
    let low = crate::values::low_mask;
    for (k, o) in vd.ops.iter().enumerate() {
        if !o.parts.is_empty() || o.consts.len() != 1 {
            continue;
        }
        let mut ts = Vec::new();
        crate::tirwalk::for_each_bind(p.fns[o.fid as usize].body.as_ref().unwrap(), &mut |s| {
            if let crate::tirwalk::BindSite::Pat(pat) = s {
                tags(pat, vix, &mut ts);
            }
        });
        ts.sort();
        for t in ts {
            let v = &d.variants[t as usize];
            let (mask, value, ult) = match d.repr {
                ValueRepr::NanBox if v.unboxed => (0, d.num_limit, true),
                // A shared block: the high tag and the pointer's low 3 bits.
                ValueRepr::NanBox if v.sub.is_some() => (!(low(v.shift) & !7), (v.tag << v.shift) | v.sub.unwrap(), false),
                ValueRepr::NanBox => {
                    // The payload's bits: narrower integers are stored
                    // zero-extended.
                    let pm = match &v.payload {
                        None => 0,
                        Some(Ty::Bool) => 1,
                        Some(Ty::Int(it)) if (it.bits as u32) < v.shift => low(it.bits as u32),
                        Some(_) => low(v.shift),
                    };
                    (!pm, if v.shift >= 64 { 0 } else { v.tag << v.shift }, false)
                }
                _ => (low(v.shift), v.tag, false),
            };
            out.push(ConstVariant { op: k, name: format!("{}.v.{}", o.name, v.name), field: o.consts[0], mask, value, ult });
        }
    }
    out
}

/// The encodings of vm `vi`'s constant-pool values that hold no heap
/// pointer, as (mask, value, ult) tests on the bits (as `ConstVariant`):
/// traces (tracegen.rs) bake such constants into their code, and load
/// the others (addresses differ between runs) from the pool.
pub fn const_immediates(p: &Program, vi: usize) -> Vec<(u64, u64, bool)> {
    let vd = &p.vms[vi];
    let Some(b) = vd.const_base else { return Vec::new() };
    let Ty::Ptr(elem) = &vd.state_tys[b] else { return Vec::new() };
    let Ty::Value(vix) = **elem else { return Vec::new() };
    let d = &p.values[vix as usize];
    let low = crate::values::low_mask;
    let mut out = Vec::new();
    for v in &d.variants {
        let scalar = match &v.payload {
            None | Some(Ty::Bool) | Some(Ty::Int(_)) | Some(Ty::F64) | Some(Ty::F32) => true,
            _ => false,
        };
        if !scalar {
            continue;
        }
        out.push(match d.repr {
            ValueRepr::NanBox if v.unboxed => (0, d.num_limit, true),
            ValueRepr::NanBox if v.sub.is_some() => continue,
            ValueRepr::NanBox => {
                let pm = match &v.payload {
                    None => 0,
                    Some(Ty::Bool) => 1,
                    Some(Ty::Int(it)) if (it.bits as u32) < v.shift => low(it.bits as u32),
                    Some(_) => low(v.shift),
                };
                (!pm, if v.shift >= 64 { 0 } else { v.tag << v.shift }, false)
            }
            ValueRepr::LowTag => (low(v.shift), v.tag, false),
            ValueRepr::Boxed => return Vec::new(),
        });
    }
    out
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
            if crate::vmexpand::enter_opcode(vd.op_bits, vd.ops.iter().map(|o| o.opcode)).is_none() {
                return Err(format!("--jit-stencils: vm {} uses every opcode; the JIT needs a free one for its enter words", vd.name));
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
        if f.vm != Some(vi as u32) || !matches!(f.vm_role, VmRole::Handler(_) | VmRole::Shared(_) | VmRole::Resume(..)) {
            continue;
        }
        let Some(body) = &f.body else { continue };
        let pc_local = f.vm_state[vd.pc_idx];
        let mut base = false;
        // A super's outlined cold blocks continue in its resume functions.
        let mut callees: Vec<FnId> = match f.vm_role {
            VmRole::Handler(k) => vd.ops[k as usize].resume.iter().flatten().copied().collect(),
            _ => vec![],
        };
        crate::tirwalk::for_each_expr(body, &mut |e| match &e.kind {
            TK::Vm(TVm::Jump(t) | TVm::SharedNext(t, ..)) => {
                if !matches!(jump_kind(p, t, pc_local), JumpKind::Const(_)) {
                    base = true;
                }
            }
            TK::Vm(TVm::Become(..)) => base = true,
            _ => {}
        });
        callees.extend(continues_in(vd, body));
        if base {
            out.insert(fid as FnId);
        }
        refs.push((fid as FnId, callees));
    }
    close_over(&mut out, &refs);
    out
}

/// The functions `body` (of a function of vm `vd`) continues in: the
/// shared copies it tail-calls and the ops it chains to.
fn continues_in(vd: &VmDef, body: &TExpr) -> Vec<FnId> {
    let mut out = Vec::new();
    crate::tirwalk::for_each_expr(body, &mut |e| match &e.kind {
        TK::Vm(TVm::Shared(c, ..)) => out.push(*c),
        TK::Vm(TVm::Chain(k)) => out.push(vd.ops[*k as usize].fid),
        _ => {}
    });
    out
}

/// Add to `set` every function that continues in one of its members
/// (`edges`: each function with those it continues in), transitively.
fn close_over(set: &mut std::collections::HashSet<FnId>, edges: &[(FnId, Vec<FnId>)]) {
    loop {
        let n = set.len();
        for (f, cs) in edges {
            if cs.iter().any(|c| set.contains(c)) {
                set.insert(*f);
            }
        }
        if set.len() == n {
            return;
        }
    }
}

/// Per op of vm `vd`: whether an instruction word holding it may be
/// rewritten while its code is compiled. That is the case if it can
/// `become` (its handler, or a shared function or op it continues in,
/// rewrites the word), if it is a superinstruction (`Vm.deopt`, or a
/// `become` in a later part), or if it is the first part of one
/// (`Vm.optimize` fuses it). The JIT gives such instructions' compiled
/// code a 5-byte prefix that it can turn into a jump to new code.
pub fn rewritable_ops(p: &Program, vd: &VmDef) -> Vec<bool> {
    // The handlers and every function they continue in.
    let mut becomes = std::collections::HashSet::new();
    let mut edges: Vec<(FnId, Vec<FnId>)> = Vec::new();
    let mut work: Vec<FnId> = vd.ops.iter().map(|o| o.fid).collect();
    while let Some(fid) = work.pop() {
        if edges.iter().any(|(f, _)| *f == fid) {
            continue;
        }
        let mut callees = Vec::new();
        if let Some(body) = &p.fns[fid as usize].body {
            let mut b = false;
            crate::tirwalk::for_each_expr(body, &mut |e| b |= matches!(e.kind, TK::Vm(TVm::Become(..))));
            if b {
                becomes.insert(fid);
            }
            callees = continues_in(vd, body);
        }
        work.extend(callees.iter().copied());
        edges.push((fid, callees));
    }
    close_over(&mut becomes, &edges);
    let heads: Vec<u32> = vd.ops.iter().filter_map(|o| o.parts.first().copied()).collect();
    vd.ops.iter().enumerate().map(|(k, o)| becomes.contains(&o.fid) || !o.parts.is_empty() || heads.contains(&(k as u32))).collect()
}

/// The opcode the JIT writes over instructions that have compiled code
/// (`vmexpand::enter_opcode`; `jit_vm` checks there is one).
pub fn enter_opcode(vd: &VmDef) -> u32 {
    crate::vmexpand::enter_opcode(vd.op_bits, vd.ops.iter().map(|o| o.opcode)).expect("jit_vm checks for a free opcode")
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

/// A stencil decoded field `raw` (at word offset `off` of the current
/// part): if it is of a `const` operand kind, remember its hole, so that
/// reads of the constant become the constant (`const_read`).
pub fn note_const_field(g: &mut FnGen, raw: LocalId, off: u32, shift: u32, bits: u32) {
    let Some(st) = g.stencil.as_mut() else { return };
    if off == 0 && st.pool.is_some() && st.consts[st.part].contains(&(shift, bits)) {
        let at = st.part_offs[st.part];
        st.const_raws.push((raw, (at, shift, bits)));
    }
}

/// In a stencil, a read of a constant through the pool (`k[c]`, or
/// `*((k + c.raw) as *T)` with `c` a const operand) of a 64-bit type:
/// the constant itself, a 64-bit hole `__jit.c.OFF.SHIFT.BITS.u` patched
/// with its bits. Returns the address of a temporary holding it.
pub fn const_read(g: &mut FnGen, e: &TExpr) -> Option<String> {
    let st = g.stencil.as_ref()?;
    let pool = st.pool?;
    let raw_of = |e: &TExpr| match strip(e).kind {
        TK::Local(l) => st.const_raws.iter().find(|(r, _)| *r == l).map(|x| x.1),
        _ => None,
    };
    let is_pool = |e: &TExpr| matches!(strip(e).kind, TK::Local(l) if l == pool);
    let hole = match &e.kind {
        TK::Index(b, i) if matches!(b.ty, Ty::Ptr(_)) && is_pool(b) => raw_of(i)?,
        TK::Deref(p) => match &strip(p).kind {
            TK::Intrinsic(Intr::PtrOffset, a) if is_pool(&a[0]) => raw_of(&a[1])?,
            _ => return None,
        },
        _ => return None,
    };
    if e.ty.is_unitlike() || crate::layout::size_of(g.p(), &e.ty) != 8 {
        return None;
    }
    let name = format!("__jit.c.{}.{}.{}.u", hole.0, hole.1, hole.2);
    g.cg.jit_holes.insert(name.clone(), u64::MAX);
    // `movabs` in inline asm: as a plain symbol address, LLVM (19)
    // sometimes stores it as a sign-extended 32-bit immediate, whatever
    // its `!absolute_symbol` range says.
    let v = g.inst(format!("call i64 asm \"movabsq $$${{1:c}}, $0\", \"=r,i\"(ptr @\"{}\")", name));
    let v = match g.stencil.as_ref().and_then(|st| st.variant) {
        // The tag is known: rebuild the word from its payload bits and the
        // tag, so that LLVM knows the tag bits (and folds tests of them).
        Some((mask, value, false)) => {
            let pl = g.inst(format!("and i64 {}, {}", v, !mask as i64));
            g.inst(format!("or i64 {}, {}", pl, value as i64))
        }
        Some((_, limit, true)) => {
            let c = g.inst(format!("icmp ult i64 {}, {}", v, limit as i64));
            g.emit(format!("call void @llvm.assume(i1 {})", c));
            g.cg.decls.insert("declare void @llvm.assume(i1)");
            v
        }
        None => v,
    };
    let t = g.alloca(&e.ty);
    g.emit(format!("store i64 {}, ptr {}", v, t));
    Some(t)
}

/// `LOIMI_JIT_REGDELAY=n` at build time (an experiment, copy-and-patch.md
/// section 13): every read of the register file in a stencil goes through
/// n dependent no-op instructions (`imul $1` on integers, `addsd -0.0` on
/// floats; 3–4 cycles each), to measure how much of the run time waits on
/// register-file loads. Off (0) by default.
pub fn reg_delay() -> u32 {
    std::env::var("LOIMI_JIT_REGDELAY").ok().and_then(|v| v.parse().ok()).unwrap_or(0)
}

/// `e` without the casts around it.
fn strip(e: &TExpr) -> &TExpr {
    match &e.kind {
        TK::Cast(a) => strip(a),
        _ => e,
    }
}

/// In a stencil with `LOIMI_JIT_REGDELAY`, a read of the register file
/// (`regs[i]`, or `*((regs + i) as *T)`): the value, delayed.
pub fn delayed_reg_read(g: &mut FnGen, e: &TExpr) -> Option<String> {
    let n = g.cg.reg_delay;
    let regs = g.stencil.as_ref()?.regs?;
    if n == 0 {
        return None;
    }
    let is_regs = |e: &TExpr| matches!(strip(e).kind, TK::Local(l) if l == regs);
    let hit = match &e.kind {
        TK::Index(b, _) => matches!(b.ty, Ty::Ptr(_)) && is_regs(b),
        TK::Deref(p) => matches!(&strip(p).kind, TK::Intrinsic(Intr::PtrOffset, a) if is_regs(&a[0])),
        _ => false,
    };
    if !hit || e.ty.is_unitlike() || crate::layout::size_of(g.p(), &e.ty) != 8 {
        return None;
    }
    let p = g.place(e);
    let lt = g.ty(&e.ty);
    let mut v = g.inst(format!("load {}, ptr {}", lt, p));
    for _ in 0..n {
        v = if lt == "double" {
            g.inst(format!("call double asm sideeffect \"addsd $2, $0\", \"=x,0,x\"(double {}, double -0.0)", v))
        } else if lt == "i64" {
            g.inst(format!("call i64 asm sideeffect \"imulq $$1, $1, $0\", \"=r,r\"(i64 {})", v))
        } else {
            return Some(v);
        };
    }
    Some(v)
}

/// The instruction word at pc + off in a stencil, as a constant.
pub fn word_hole(g: &mut FnGen, off: u32) -> String {
    let name = format!("__jit.w.{}", off);
    g.cg.jit_holes.insert(name.clone(), 1u64 << 32);
    format!("ptrtoint (ptr @\"{}\" to i32)", name)
}

/// The signed decoded fields anywhere in a handler body.
pub fn signed_fields(body: &TExpr, locals: &[crate::tir::LocalDef]) -> Vec<(u32, u32, u32)> {
    let mut out = Vec::new();
    crate::tirwalk::for_each_expr(body, &mut |e| {
        if let TK::Block(stmts, _) = &e.kind {
            for s in stmts {
                if let TStmt::Let(l, Some(_)) = s {
                    if let Some(FieldDecode { off, shift, bits, signed: true }) = locals[*l as usize].field {
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
    /// A resume point (`jump r` with `r: Vm.Resume`): an instruction
    /// boundary of verified code saved by vm code. Counted like a call:
    /// Prolog's TRY/RETRY/TRUST and guard chains enter clauses this way.
    Resume,
    /// Read from memory (returns).
    Dynamic,
}

pub fn jump_kind(p: &Program, t: &TExpr, pc_local: LocalId) -> JumpKind {
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
        TK::Block(_, Some(tail)) => match jump_kind(p, tail, pc_local) {
            JumpKind::Const(_) => JumpKind::PcRel,
            k => k,
        },
        TK::Field(code, 0) if matches!(code.ty, Ty::Slice(_)) => JumpKind::Entry,
        TK::Field(r, 0) if is_resume(p, &r.ty) => JumpKind::Resume,
        _ => JumpKind::Dynamic,
    }
}

/// Whether `t` is a vm's generated resume point type `Vm__Resume`.
fn is_resume(p: &Program, t: &Ty) -> bool {
    let Ty::Struct(sid) = t else { return false };
    let sd = &p.structs[*sid as usize];
    sd.name.ends_with("__Resume") && sd.fields.len() == 1 && sd.fields[0].0 == "__pc"
}

/// A dynamic jump (call, return) in a JIT build: if the target holds an
/// enter word, jump straight to its compiled code, without counting.
/// Returns with the insertion point on the other path.
pub fn enter_if_compiled(g: &mut FnGen, vd: &VmDef, target: &str) {
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
    crate::vmgen::store_pc(g, target);
    crate::vmgen::tail_to(g, vd, &e, &w);
    g.label(&no);
}

/// Hot counting before a dispatch to `target` (counting copies and
/// stencils): decrement the counter of the target's hash slot; at
/// zero, continue in `Vm.jit_hot`, which may compile the code there.
/// `backward_only`: count only if the target is at or before pc (loops).
/// Returns with the insertion point on the path that dispatches normally.
pub fn hot_count(g: &mut FnGen, vd: &VmDef, target: &str, backward_only: bool) {
    let go = g.fresh_label("hotgo");
    if backward_only {
        let pc = crate::vmgen::load_pc(g);
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
    crate::vmgen::store_pc(g, target);
    let sym = format!("@\"{}.jit_hot\"", vd.symbol);
    crate::vmgen::tail_to(g, vd, &sym, "0");
    g.label(&go);
}

/// Module-level pieces of a JIT build (in the interpreter's module): the
/// enter handler, the hot trigger, op metadata, and the runtime's
/// declarations. The enter opcode's table slot is filled by `emit_tables`.
pub fn emit_runtime(p: &Program, vi: usize, globals: &mut String, fns: &mut String) {
    let vd = &p.vms[vi];
    let rt = crate::vmgen::handler_ret_llty(p, vd);
    let args = format!("{}, i32 %insn", state_params(p, vd));
    let call = |callee: &str, insn: &str| handler_tail(p, vd, callee, insn);
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
    let _ = writeln!(fns, "declare void @loimi_jit_hot(ptr)\ndeclare void @loimi_jit_become(ptr, i32)");
    // Per opcode: { the size of its first word's instruction (a super's
    // first part: the later parts' words are instructions too), the
    // opcode whose stencil to use (0xffff: invalid), flags, the size of
    // the whole op, the stencil to use if that one is missing (a super's
    // first part) }. Flag 1: reads the next word as its `then` operand;
    // 2: a super (the words of its later parts must keep their opcodes);
    // 4: its word may be rewritten at run time (`rewritable_ops`).
    let rw = rewritable_ops(p, vd);
    let mut info = Vec::new();
    let ty = "{ i16, i16, i16, i16, i16 }";
    for i in 0..n {
        let e = match vd.op_index(i as usize) {
            Some(k) => {
                let o = &vd.ops[k];
                let first = match o.parts.first() {
                    Some(&b) => &vd.ops[b as usize],
                    None => o,
                };
                let flags = (first.then as u32) | ((!o.parts.is_empty() as u32) << 1) | ((rw[k] as u32) << 2);
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
        let nm = vd.op_index(i as usize).map(|k| &vd.ops[k]).map(|o| o.name.as_str()).unwrap_or("?");
        let _ = writeln!(globals, "@loimi_jit_opname.{} = private constant [{} x i8] c\"{}\\00\"", i, nm.len() + 1, nm);
        names.push(format!("ptr @loimi_jit_opname.{}", i));
    }
    let _ = writeln!(globals, "@loimi_jit_opnames = constant [{} x ptr] [{}]", n, names.join(", "));
    let _ = writeln!(globals, "@loimi_jit_enter_op = constant i32 {}", enter_opcode(vd));
    // Stencil variants per constant tag: { opcode, stencil index, field
    // (shift << 8 | bits << 16), mask, value, ult }.
    let vs = const_variants(p, vi);
    let vty = "{ i32, i32, i64, i64, i64, i32 }";
    let items: Vec<String> = vs
        .iter()
        .enumerate()
        .map(|(j, cv)| {
            format!(
                "{vty} {{ i32 {}, i32 {}, i64 {}, i64 {}, i64 {}, i32 {} }}",
                vd.ops[cv.op].opcode,
                n + 1 + j as u32,
                (cv.field.0 << 8 | cv.field.1 << 16) as i64,
                cv.mask as i64,
                cv.value as i64,
                cv.ult as i32
            )
        })
        .collect();
    let _ = writeln!(globals, "@loimi_jit_variants = constant [{} x {vty}] [{}]", items.len(), items.join(", "));
    let _ = writeln!(globals, "@loimi_jit_nvariants = constant i32 {}", items.len());
    let _ = writeln!(globals, "@loimi_jit_op_bits = constant i32 {}", vd.op_bits);
}

/// The generic exit stencil (for ops without one): dispatch the
/// instruction at pc through the interpreter's table.
pub fn exit_stencil(p: &Program, vi: usize) -> String {
    let vd = &p.vms[vi];
    let rt = crate::vmgen::handler_ret_llty(p, vd);
    let mask = (1u32 << vd.op_bits) - 1;
    let tail = handler_tail(p, vd, "%h", "%w");
    format!(
        "define preserve_nonecc {rt} @\"__jit.exit\"({}, i32 %insn) nounwind {{\n  %w = load i32, ptr %p{pc}\n  %o = and i32 %w, {mask}\n  %oz = zext i32 %o to i64\n  %hp = getelementptr inbounds ptr, ptr {table}, i64 %oz\n  %h = load ptr, ptr %hp\n{tail}}}\n",
        state_params(p, vd),
        pc = vd.pc_idx,
        table = crate::vmgen::table_name(vd),
    )
}

/// The parameters of a function with the handler signature of vm `vd`
/// for its state: `%p0, %p1, ..` (the instruction word not included).
fn state_params(p: &Program, vd: &VmDef) -> String {
    vd.state_tys.iter().enumerate().map(|(i, t)| format!("{} %p{}", llty(p, t), i)).collect::<Vec<_>>().join(", ")
}

/// IR lines ending such a function: tail-call handler `callee` with the
/// state parameters and instruction word `insn`, and return its result.
fn handler_tail(p: &Program, vd: &VmDef, callee: &str, insn: &str) -> String {
    let rt = crate::vmgen::handler_ret_llty(p, vd);
    let a = format!("{}, i32 {}", state_params(p, vd), insn);
    if rt == "void" {
        format!("  musttail call preserve_nonecc void {}({})\n  ret void\n", callee, a)
    } else {
        format!("  %r = musttail call preserve_nonecc {} {}({})\n  ret {} %r\n", rt, callee, a, rt)
    }
}

/// Emit every op's stencil (not supers: the JIT compiles their parts).
pub fn emit_stencils(cg: &mut crate::codegen::Codegen) {
    let p = cg.p;
    let Ok(vi) = jit_vm(p) else { return };
    let vd = &p.vms[vi];
    let mut out = String::new();
    let variants = const_variants(p, vi);
    for o in &vd.ops {
        let f = &p.fns[o.fid as usize];
        let body = f.body.as_ref().unwrap();
        // A super's body is a block of `Part`s; each has its own fields.
        let (labels, part_offs) = if o.parts.is_empty() {
            (vec![signed_fields(body, &f.locals)], vec![0])
        } else {
            let mut ls = vec![vec![]; o.parts.len()];
            crate::tirwalk::for_each_expr(body, &mut |e| {
                if let TK::Vm(TVm::Part(j, b)) = &e.kind {
                    ls[*j as usize] = signed_fields(b, &f.locals);
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
        let consts = if o.parts.is_empty() { vec![o.consts.clone()] } else { o.parts.iter().map(|&q| vd.ops[q as usize].consts.clone()).collect() };
        let pool = vd.const_base.map(|b| f.vm_state[b]);
        let mut g = FnGen::new(cg, o.fid);
        g.stencil = Some(StencilGen {
            symbol: format!("__jit.op.{}", o.name),
            cold_syms: g.cg.cold_syms.get(&o.fid).cloned().unwrap_or_default(),
            cold_next: 0,
            labels,
            part_offs,
            part: 0,
            consts,
            pool,
            const_raws: Vec::new(),
            variant: None,
            regs: vd.state_names.iter().position(|n| n == "regs").map(|i| f.vm_state[i]),
        });
        let st = g.stencil.clone();
        out.push_str(&g.run());
        for cv in variants.iter().filter(|cv| vd.ops[cv.op].fid == o.fid) {
            let mut g = FnGen::new(cg, o.fid);
            let mut s = st.clone().unwrap();
            s.symbol = format!("__jit.op.{}", cv.name);
            s.variant = Some((cv.mask, cv.value, cv.ult));
            g.stencil = Some(s);
            out.push_str(&g.run());
        }
    }
    // Hot and cold parts (an experiment, off unless LOIMI_JIT_SPLIT is
    // set at build time; no faster, see copy-and-patch.md section 10): an
    // entry count makes the blocks LLVM thinks are never run cold, and
    // `-fsplit-machine-functions` moves them to a section of their own
    // (`.text.split.__jit.op.NAME`). The fast path then ends in its jump to
    // the next instruction, which the JIT drops (falling through), and
    // the cold parts go after all fast paths (`elf.rs`, `jit_rt.c`).
    let prof = cg.jit_holes.len();
    let split = split_stencils();
    let mut out = if split {
        out.lines()
            .map(|l| match l.strip_suffix(" {") {
                Some(head) if l.starts_with("define ") && l.contains("@\"__jit.op.") => format!("{} !prof !{} {{\n", head, prof),
                _ => format!("{}\n", l),
            })
            .collect::<String>()
    } else {
        out
    };
    out.push_str(&exit_stencil(p, vi));
    let mut md = String::new();
    for (i, (name, hi)) in cg.jit_holes.iter().enumerate() {
        // extern_weak: LLVM must not assume the address (the value) is
        // not null, as it does for other globals.
        let _ = writeln!(out, "@\"{}\" = extern_weak dso_local global i8, !absolute_symbol !{}", name, i);
        if *hi == u64::MAX {
            // A 64-bit hole (a constant, read by inline asm).
            let _ = writeln!(md, "!{} = !{{i64 -1, i64 -1}}", i);
        } else {
            let _ = writeln!(md, "!{} = !{{i64 0, i64 {}}}", i, hi);
        }
    }
    for line in cg.jit_conts.values() {
        let _ = writeln!(out, "{}", line);
    }
    if split {
        let _ = writeln!(md, "!{} = !{{!\"function_entry_count\", i64 1000}}", prof);
    }
    out.push_str(&md);
    cg.stencils = out;
}

/// Whether to split stencils into hot and cold parts (`LOIMI_JIT_SPLIT`).
pub fn split_stencils() -> bool {
    std::env::var_os("LOIMI_JIT_SPLIT").is_some_and(|v| !v.is_empty() && v != "0")
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
/// - Other definitions (`main`, exports) become `available_externally`.
/// - Functions the stencils cannot reach (through function bodies) become
///   declarations, so that clang compiles only what the stencils use.
pub fn stencil_module(ir: &str, stencils: &str) -> String {
    // The functions the stencils reach.
    let mut bodies: std::collections::HashMap<&str, Vec<&str>> = std::collections::HashMap::new();
    let mut cur: Option<(&str, Vec<&str>)> = None;
    for line in ir.lines() {
        if let Some((name, refs)) = cur.as_mut() {
            if line == "}" {
                bodies.insert(name, std::mem::take(refs));
                cur = None;
            } else {
                refs.extend(symbol_refs(line));
            }
        } else if line.starts_with("define ") {
            if let Some(name) = symbol_refs(line).into_iter().next() {
                cur = Some((name, Vec::new()));
            }
        }
    }
    let mut reached: std::collections::HashSet<&str> = std::collections::HashSet::new();
    let mut work: Vec<&str> = stencils.lines().flat_map(symbol_refs).collect();
    while let Some(n) = work.pop() {
        if reached.insert(n) {
            if let Some(rs) = bodies.get(n) {
                work.extend(rs.iter().copied());
            }
        }
    }
    let mut out = String::with_capacity(ir.len() + stencils.len());
    let mut skipping = false;
    for line in ir.lines() {
        if skipping {
            skipping = line != "}";
            continue;
        }
        if line.starts_with("define ") {
            if let Some(name) = symbol_refs(line).into_iter().next() {
                if !reached.contains(name) {
                    out.push_str(&declaration(line));
                    out.push('\n');
                    skipping = true;
                    continue;
                }
            }
        }
        if let Some(rest) = line.strip_prefix("define internal ") {
            if rest.starts_with("preserve_nonecc ") {
                out.push_str("define available_externally dso_local ");
                out.push_str(rest);
            } else {
                out.push_str(line);
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

/// The global symbols a line of IR names (`@name`, `@"name"`), in order.
fn symbol_refs(line: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let b = line.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'"' {
            // A string constant (c"...") or a quoted local name: skip it.
            if i > 0 && b[i - 1] == b'@' {
                let end = line[i + 1..].find('"').map(|e| i + 1 + e).unwrap_or(b.len());
                out.push(&line[i + 1..end]);
                i = end + 1;
                continue;
            }
            let end = line[i + 1..].find('"').map(|e| i + 1 + e).unwrap_or(b.len());
            i = end + 1;
            continue;
        }
        if b[i] == b'@' && i + 1 < b.len() && b[i + 1] != b'"' {
            let s = i + 1;
            let mut e = s;
            while e < b.len() && (b[e].is_ascii_alphanumeric() || b[e] == b'_' || b[e] == b'.' || b[e] == b'$') {
                e += 1;
            }
            if e > s {
                out.push(&line[s..e]);
            }
            i = e;
            continue;
        }
        i += 1;
    }
    out
}

/// A `declare` for the function a `define` line begins: its calling
/// convention, result and parameter types.
fn declaration(define: &str) -> String {
    let at = define.find('@').unwrap();
    let head: Vec<&str> = define[7..at].split_whitespace().filter(|w| !matches!(*w, "internal" | "available_externally" | "dso_local")).collect();
    // The name, then the parameter list up to its closing parenthesis.
    let mut i = at + 1;
    if define[i..].starts_with('"') {
        i += 1 + define[i + 1..].find('"').unwrap() + 1;
    } else {
        while i < define.len() && define.as_bytes()[i] != b'(' {
            i += 1;
        }
    }
    let name = &define[at..i];
    let mut depth = 0;
    let mut params = Vec::new();
    let mut start = i + 1;
    let mut in_str = false;
    for (k, c) in define[i..].char_indices().map(|(k, c)| (k + i, c)) {
        match c {
            '"' => in_str = !in_str,
            _ if in_str => {}
            '(' | '{' | '[' | '<' => depth += 1,
            ')' | '}' | ']' | '>' => {
                depth -= 1;
                if depth == 0 {
                    params.push(&define[start..k]);
                    break;
                }
            }
            ',' if depth == 1 => {
                params.push(&define[start..k]);
                start = k + 1;
            }
            _ => {}
        }
    }
    let tys: Vec<String> = params
        .iter()
        .map(|p| p.trim())
        .filter(|p| !p.is_empty())
        .map(|p| match p.rfind(" %") {
            Some(s) => p[..s].to_string(),
            None => p.to_string(),
        })
        .collect();
    format!("declare {} {}({})", head.join(" "), name, tys.join(", "))
}
