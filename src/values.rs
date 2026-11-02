//! `value` types: dynamic values with a declared representation.
//!
//! nanbox:  i64. The `unboxed` f64 variant is stored as its bits (NaNs
//!          canonicalized); other variants use tags 0xFFF9.. in the top
//!          16 bits with a 48-bit payload.
//! lowtag:  i64. Tag in the low 3 bits; integers shifted left by 3,
//!          pointers (8-byte aligned) or'ed with the tag.
//! boxed:   { i64 tag, i64 payload }, two registers.

use crate::check::Checker;
use crate::codegen::FnGen;
use crate::diag::{Diag, Res};
use crate::tir::*;
use crate::vmast::ValueDecl;

pub const NAN_TAG_BASE: u64 = 0xFFF9;
pub const CANONICAL_NAN: u64 = 0x7FF8_0000_0000_0000;
const LOW_BITS: u64 = 3;

pub fn check_value_decl(c: &mut Checker, m: usize, v: &ValueDecl) -> Res<ValueDef> {
    let repr = match v.repr.name.as_str() {
        "nanbox" => ValueRepr::NanBox,
        "lowtag" => ValueRepr::LowTag,
        "boxed" => ValueRepr::Boxed,
        r => return Err(Diag::new(v.repr.span, format!("unknown representation `{}` (nanbox, lowtag or boxed)", r))),
    };
    let mut variants = Vec::new();
    let mut boxed_count = 0u64;
    let mut have_unboxed = false;
    for (i, vv) in v.variants.iter().enumerate() {
        let payload = match &vv.payload {
            Some(t) => Some(c.resolve_type(m, t)?),
            None => None,
        };
        let small_ok = |t: &Ty| match t {
            Ty::Int(it) => it.bits <= 32,
            Ty::Bool | Ty::Ptr(_) | Ty::Gc(_) => true,
            _ => false,
        };
        let tag = match repr {
            ValueRepr::NanBox => {
                if vv.unboxed {
                    if payload != Some(Ty::F64) {
                        return Err(Diag::new(vv.name.span, "the `unboxed` variant of a nanbox must hold an f64"));
                    }
                    if have_unboxed {
                        return Err(Diag::new(vv.name.span, "a nanbox has at most one `unboxed` variant"));
                    }
                    have_unboxed = true;
                    0
                } else {
                    if let Some(t) = &payload {
                        if !small_ok(t) {
                            return Err(Diag::new(
                                vv.name.span,
                                "nanbox payloads must be integers of at most 32 bits, bool or pointers (or the unboxed f64)",
                            ));
                        }
                    }
                    let t = NAN_TAG_BASE + boxed_count;
                    boxed_count += 1;
                    if t > 0xFFFF {
                        return Err(Diag::new(vv.name.span, "a nanbox has room for at most 7 tagged variants"));
                    }
                    t
                }
            }
            ValueRepr::LowTag => {
                if vv.unboxed {
                    return Err(Diag::new(vv.name.span, "`unboxed` is only meaningful for nanbox"));
                }
                if let Some(t) = &payload {
                    if !small_ok(t) {
                        return Err(Diag::new(vv.name.span, "lowtag payloads must be integers of at most 32 bits, bool or pointers"));
                    }
                }
                if i as u64 >= 1 << LOW_BITS {
                    return Err(Diag::new(vv.name.span, "lowtag has room for at most 8 variants"));
                }
                i as u64
            }
            ValueRepr::Boxed => {
                if let Some(t) = &payload {
                    if !(small_ok(t) || matches!(t, Ty::F64 | Ty::Int(_))) {
                        return Err(Diag::new(vv.name.span, "boxed payloads must be numbers, bool or pointers"));
                    }
                }
                i as u64
            }
        };
        if variants.iter().any(|x: &ValueVariantDef| x.name == vv.name.name) {
            return Err(Diag::new(vv.name.span, format!("duplicate variant `{}`", vv.name.name)));
        }
        variants.push(ValueVariantDef { name: vv.name.name.clone(), payload, unboxed: vv.unboxed, tag });
    }
    Ok(ValueDef { name: format!("{}.{}", c.module_prefix(m), v.name.name), repr, variants })
}

fn def<'a>(g: &FnGen<'_, 'a>, t: &Ty) -> &'a ValueDef {
    let Ty::Value(i) = t else { unreachable!() };
    &g.p().values[*i as usize]
}

/// Payload (of type `pt`) → i64 bits.
fn payload_bits(g: &mut FnGen, pt: &Ty, v: &str) -> String {
    match pt {
        Ty::F64 => g.inst(format!("bitcast double {} to i64", v)),
        Ty::Bool => g.inst(format!("zext i1 {} to i64", v)),
        Ty::Ptr(_) | Ty::Gc(_) => g.inst(format!("ptrtoint ptr {} to i64", v)),
        Ty::Int(it) if it.bits == 64 => v.to_string(),
        Ty::Int(it) => g.inst(format!("zext i{} {} to i64", it.bits, v)),
        _ => unreachable!(),
    }
}

/// Locals of the current function that hold an f64 unboxed from a nanbox
/// value's `unboxed` variant, and are bound nowhere else and immutable.
fn nan_safe_local(g: &mut FnGen, l: LocalId) -> bool {
    let fid = g.fid;
    if !g.cg.nan_safe.contains_key(&fid) {
        let p = g.p();
        let f = &p.fns[fid as usize];
        let mut good = std::collections::HashSet::new();
        let mut bad = std::collections::HashSet::new();
        fn rec(p: &Program, pat: &TPat, under: bool, good: &mut std::collections::HashSet<LocalId>, bad: &mut std::collections::HashSet<LocalId>) {
            match &pat.kind {
                TPK::Bind(l) => {
                    if under {
                        good.insert(*l);
                    } else {
                        bad.insert(*l);
                    }
                }
                TPK::Variant(idx, subs) => {
                    let unboxed = match &pat.ty {
                        Ty::Value(v) => {
                            let d = &p.values[*v as usize];
                            d.repr == ValueRepr::NanBox && d.variants[*idx as usize].unboxed
                        }
                        _ => false,
                    };
                    subs.iter().for_each(|s| rec(p, s, unboxed, good, bad));
                }
                TPK::Tuple(ps) | TPK::Or(ps) => ps.iter().for_each(|s| rec(p, s, false, good, bad)),
                _ => {}
            }
        }
        if let Some(b) = &f.body {
            crate::tirwalk::for_each_pat(b, &mut |pat| rec(p, pat, false, &mut good, &mut bad));
        }
        let set = good.into_iter().filter(|l| !bad.contains(l) && !f.locals[*l as usize].mutable).collect();
        g.cg.nan_safe.insert(fid, set);
    }
    g.cg.nan_safe[&fid].contains(&l)
}

/// Whether the f64 expression `e` can only be a NaN that hardware made
/// from canonical inputs: such NaNs have a zero payload (0x7FF8.. or
/// 0xFFF8..), which is already a valid `unboxed` encoding, so building
/// the value needs no canonicalization. Leaves are literals, int-to-float
/// casts, and locals unboxed from the value (always canonical).
fn nan_safe(g: &mut FnGen, e: &TExpr) -> bool {
    use crate::ast::{BinOp, UnOp};
    match &e.kind {
        TK::Float(x) => !x.is_nan(),
        TK::Cast(inner) => matches!(inner.ty, Ty::Int(_)),
        TK::Local(l) => nan_safe_local(g, *l),
        TK::Binary(BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div, a, b) => nan_safe(g, a) && nan_safe(g, b),
        TK::Unary(UnOp::Neg, a) => nan_safe(g, a),
        TK::Intrinsic(Intr::Sqrt | Intr::Floor, args) => args.iter().all(|a| nan_safe(g, a)),
        _ => false,
    }
}

pub fn make(g: &mut FnGen, t: &Ty, idx: u32, args: &[TExpr], vals: &[String]) -> String {
    let d = def(g, t);
    let var = &d.variants[idx as usize];
    match d.repr {
        ValueRepr::NanBox => {
            if var.unboxed {
                let bits = g.inst(format!("bitcast double {} to i64", vals[0]));
                if nan_safe(g, &args[0]) {
                    return bits;
                }
                let nan = g.inst(format!("fcmp uno double {}, {}", vals[0], vals[0]));
                return g.inst(format!("select i1 {}, i64 {}, i64 {}", nan, CANONICAL_NAN as i64, bits));
            }
            let tag = ((var.tag << 48) as i64).to_string();
            match &var.payload {
                None => tag,
                Some(pt) => {
                    let b = match pt {
                        // Signed payloads are stored as their low bits.
                        Ty::Int(it) if it.signed => g.inst(format!("zext i{} {} to i64", it.bits, vals[0])),
                        _ => payload_bits(g, pt, &vals[0]),
                    };
                    let b = if matches!(pt, Ty::Ptr(_) | Ty::Gc(_)) { g.inst(format!("and i64 {}, {}", b, 0xFFFF_FFFF_FFFFu64)) } else { b };
                    g.inst(format!("or i64 {}, {}", b, tag))
                }
            }
        }
        ValueRepr::LowTag => match &var.payload {
            None => var.tag.to_string(),
            Some(Ty::Ptr(_) | Ty::Gc(_)) => {
                let b = g.inst(format!("ptrtoint ptr {} to i64", vals[0]));
                g.inst(format!("or i64 {}, {}", b, var.tag))
            }
            Some(pt) => {
                let b = match pt {
                    Ty::Int(it) if it.signed => g.inst(format!("sext i{} {} to i64", it.bits, vals[0])),
                    _ => payload_bits(g, pt, &vals[0]),
                };
                let s = g.inst(format!("shl i64 {}, {}", b, LOW_BITS));
                g.inst(format!("or i64 {}, {}", s, var.tag))
            }
        },
        ValueRepr::Boxed => {
            let _ = args;
            let a = g.inst(format!("insertvalue {{ i64, i64 }} poison, i64 {}, 0", var.tag));
            let b = match &var.payload {
                None => "0".to_string(),
                Some(Ty::Int(it)) if it.signed && it.bits < 64 => g.inst(format!("sext i{} {} to i64", it.bits, vals[0])),
                Some(pt) => payload_bits(g, pt, &vals[0]),
            };
            g.inst(format!("insertvalue {{ i64, i64 }} {}, i64 {}, 1", a, b))
        }
    }
}

/// i1: does `v` (loaded value) hold variant `idx`?
pub fn is_variant(g: &mut FnGen, d: &ValueDef, idx: u32, v: &str) -> String {
    let var = &d.variants[idx as usize];
    match d.repr {
        ValueRepr::NanBox if var.unboxed => g.inst(format!("icmp ult i64 {}, {}", v, (NAN_TAG_BASE << 48) as i64)),
        ValueRepr::NanBox => {
            let h = g.inst(format!("lshr i64 {}, 48", v));
            g.inst(format!("icmp eq i64 {}, {}", h, var.tag))
        }
        ValueRepr::LowTag => {
            let l = g.inst(format!("and i64 {}, {}", v, (1u64 << LOW_BITS) - 1));
            g.inst(format!("icmp eq i64 {}, {}", l, var.tag))
        }
        ValueRepr::Boxed => {
            let t = g.inst(format!("extractvalue {{ i64, i64 }} {}, 0", v));
            g.inst(format!("icmp eq i64 {}, {}", t, var.tag))
        }
    }
}

/// Extract the payload of variant `idx` from `v` (known to hold it).
pub fn payload(g: &mut FnGen, d: &ValueDef, idx: u32, v: &str) -> String {
    let var = &d.variants[idx as usize];
    let pt = var.payload.as_ref().unwrap();
    let raw = match d.repr {
        ValueRepr::NanBox => {
            if var.unboxed {
                return g.inst(format!("bitcast i64 {} to double", v));
            }
            if let Ty::Ptr(_) | Ty::Gc(_) = pt {
                let m = g.inst(format!("and i64 {}, {}", v, 0xFFFF_FFFF_FFFFu64));
                return g.inst(format!("inttoptr i64 {} to ptr", m));
            }
            v.to_string()
        }
        ValueRepr::LowTag => {
            if let Ty::Ptr(_) | Ty::Gc(_) = pt {
                let m = g.inst(format!("and i64 {}, {}", v, -(1i64 << LOW_BITS)));
                return g.inst(format!("inttoptr i64 {} to ptr", m));
            }
            let signed = matches!(pt, Ty::Int(it) if it.signed);
            g.inst(format!("{} i64 {}, {}", if signed { "ashr" } else { "lshr" }, v, LOW_BITS))
        }
        ValueRepr::Boxed => {
            let p = g.inst(format!("extractvalue {{ i64, i64 }} {}, 1", v));
            match pt {
                Ty::F64 => return g.inst(format!("bitcast i64 {} to double", p)),
                Ty::Ptr(_) | Ty::Gc(_) => return g.inst(format!("inttoptr i64 {} to ptr", p)),
                _ => p,
            }
        }
    };
    match pt {
        Ty::Bool => g.inst(format!("trunc i64 {} to i1", raw)),
        Ty::Int(it) if it.bits < 64 => g.inst(format!("trunc i64 {} to i{}", raw, it.bits)),
        _ => raw,
    }
}

fn bind_payload(g: &mut FnGen, d: &ValueDef, idx: u32, subs: &[TPat], v: &str, fail: &str) {
    if subs.is_empty() || matches!(subs[0].kind, TPK::Wild) {
        return;
    }
    let pt = d.variants[idx as usize].payload.clone().unwrap();
    let p = payload(g, d, idx, v);
    let slot = g.alloca(&pt);
    g.store(&pt, &p, &slot);
    g.test_pat(&subs[0], &slot, fail);
}

pub fn test_variant(g: &mut FnGen, t: &Ty, idx: u32, subs: &[TPat], ptr: &str, fail: &str) {
    let d = def(g, t);
    let lt = g.ty(t);
    let v = g.inst(format!("load {}, ptr {}", lt, ptr));
    let c = is_variant(g, d, idx, &v);
    g.branch_ok(&c, fail);
    bind_payload(g, d, idx, subs, &v, fail);
}

/// Tuple patterns whose elements are all value-variant patterns: test all
/// tags with one merged comparison. Returns false if not applicable.
pub fn try_merged_tuple(g: &mut FnGen, pats: &[TPat], tuple_ty: &Ty, ptr: &str, fail: &str) -> bool {
    let mut elems = Vec::new();
    for (i, p) in pats.iter().enumerate() {
        match (&p.ty, &p.kind) {
            (Ty::Value(_), TPK::Variant(idx, subs)) => elems.push((i, p.ty.clone(), *idx, subs.clone())),
            (_, TPK::Wild) => {}
            _ => return false,
        }
    }
    if elems.len() < 2 {
        return false;
    }
    let tl = g.ty(tuple_ty);
    let mut loaded = Vec::new();
    for (i, t, idx, subs) in &elems {
        let d = def(g, t);
        if d.repr == ValueRepr::Boxed {
            return false;
        }
        let fp = g.inst(format!("getelementptr inbounds {}, ptr {}, i32 0, i32 {}", tl, ptr, i));
        let v = g.inst(format!("load i64, ptr {}", fp));
        loaded.push((d, *idx, subs.clone(), v));
    }
    // Group: tag-mask tests (xor/or/and) and unboxed-double tests (umax).
    let mut tag_terms = Vec::new();
    let mut mask: Option<u64> = None;
    let mut unboxed = Vec::new();
    let mut conds = Vec::new();
    for (d, idx, _, v) in &loaded {
        let var = &d.variants[*idx as usize];
        match d.repr {
            ValueRepr::NanBox if var.unboxed => unboxed.push(v.clone()),
            ValueRepr::NanBox | ValueRepr::LowTag => {
                let (m, tagbits) = if d.repr == ValueRepr::NanBox {
                    (0xFFFFu64 << 48, var.tag << 48)
                } else {
                    ((1u64 << LOW_BITS) - 1, var.tag)
                };
                if mask.is_some_and(|x| x != m) {
                    let c = is_variant(g, d, *idx, v);
                    conds.push(c);
                    continue;
                }
                mask = Some(m);
                let x = if tagbits == 0 { v.clone() } else { g.inst(format!("xor i64 {}, {}", v, tagbits as i64)) };
                tag_terms.push(x);
            }
            ValueRepr::Boxed => unreachable!(),
        }
    }
    if !tag_terms.is_empty() {
        let mut acc = tag_terms[0].clone();
        for t in &tag_terms[1..] {
            acc = g.inst(format!("or i64 {}, {}", acc, t));
        }
        let m = mask.unwrap();
        let a = g.inst(format!("and i64 {}, {}", acc, m as i64));
        conds.push(g.inst(format!("icmp eq i64 {}, 0", a)));
    }
    if !unboxed.is_empty() {
        let mut acc = unboxed[0].clone();
        if unboxed.len() > 1 {
            g.cg.decls.insert("declare i64 @llvm.umax.i64(i64, i64)");
        }
        for u in &unboxed[1..] {
            acc = g.inst(format!("call i64 @llvm.umax.i64(i64 {}, i64 {})", acc, u));
        }
        conds.push(g.inst(format!("icmp ult i64 {}, {}", acc, (NAN_TAG_BASE << 48) as i64)));
    }
    let mut c = conds[0].clone();
    for x in &conds[1..] {
        c = g.inst(format!("and i1 {}, {}", c, x));
    }
    g.branch_ok(&c, fail);
    for (d, idx, subs, v) in &loaded {
        bind_payload(g, d, *idx, subs, v, fail);
    }
    true
}

pub fn is_boxed(p: &Program, t: &Ty) -> bool {
    matches!(t, Ty::Value(i) if p.values[*i as usize].repr == ValueRepr::Boxed)
}
