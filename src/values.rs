//! `value` types: dynamic values with a declared representation.
//!
//! nanbox:  i64. The `unboxed` f64 variant is stored as its bits (NaNs
//!          canonicalized to 0x7FF8_0000_0000_0000). Every other variant
//!          owns an aligned block of the NaN space above
//!          0xFFF8_0000_0000_0000: the word is `tag << shift | payload`,
//!          where the payload has `shift` bits. With at most 7 tagged
//!          variants each gets a 48-bit block 0xFFF9.. 0xFFFF in declaration
//!          order. With more, blocks are sized to the payload (48 bits for
//!          pointers, N for iN/uN, 1 for bool, 0 for none) and allocated
//!          buddy-style from 0xFFF8_8000_0000_0000 (47 bits) and
//!          0xFFF9..0xFFFF (48 bits each), widest first.
//! lowtag:  i64. The word is `payload << shift | tag`. With at most 8
//!          variants the tag is the variant index in the low 3 bits;
//!          pointers (8-byte aligned) are or'ed with it and integers (up to
//!          61 bits) shifted by 3. With more, pointers and 61-bit integers
//!          keep 3-bit tags and narrower variants share the rest with longer
//!          tags (`64 - payload bits` long at most), allocated buddy-style.
//! boxed:   { i64 tag, i64 payload }, two registers.

use crate::check::Checker;
use crate::codegen::FnGen;
use crate::diag::{Diag, Res};
use crate::tir::*;
use crate::vmast::ValueDecl;
use std::collections::HashSet;

pub const NAN_TAG_BASE: u64 = 0xFFF9;
pub const CANONICAL_NAN: u64 = 0x7FF8_0000_0000_0000;
/// The 47-bit block in the upper half of 0xFFF8: above the negative
/// default NaN 0xFFF8_0000_0000_0000 that x86 arithmetic produces.
const NAN_HALF_BLOCK: u64 = 0xFFF8_8000_0000_0000;
const LOW_BITS: u32 = 3;
/// Bits a nanbox keeps of a pointer payload.
const NAN_PTR_BITS: u32 = 48;

/// Bits a payload of type `t` needs (pointers: `ptr_bits`), or None if the
/// type cannot be a payload of a one-word value.
fn payload_width(t: Option<&Ty>, ptr_bits: u32) -> Option<u32> {
    Some(match t {
        None => 0,
        Some(Ty::Bool) => 1,
        Some(Ty::Int(it)) => it.bits as u32,
        Some(Ty::Ptr(_) | Ty::Gc(_)) => ptr_bits,
        Some(_) => return None,
    })
}

/// Buddy allocation. `free` holds blocks as (key, log2 size); `need` lists
/// (variant, log2 size) in allocation order. A request takes the smallest
/// free block that fits (lowest key on ties) and halves it down to the
/// requested size, keeping the half `split` returns first. Returns each
/// variant's block key, or the first variant that did not fit.
fn buddy(mut free: Vec<(u64, u32)>, need: &[(usize, u32)], split: impl Fn(u64, u32) -> (u64, u64)) -> Result<Vec<(usize, u64)>, usize> {
    let mut out = Vec::new();
    for &(v, size) in need {
        let best = free.iter().enumerate().filter(|(_, b)| b.1 >= size).min_by_key(|(_, b)| (b.1, b.0)).map(|(i, _)| i);
        let Some(i) = best else { return Err(v) };
        let (mut key, mut s) = free.remove(i);
        while s > size {
            let (lo, hi) = split(key, s);
            s -= 1;
            free.push((hi, s));
            key = lo;
        }
        out.push((v, key));
    }
    Ok(out)
}

pub fn check_value_decl(c: &mut Checker, m: usize, v: &ValueDecl) -> Res<ValueDef> {
    let repr = match v.repr.name.as_str() {
        "nanbox" => ValueRepr::NanBox,
        "lowtag" => ValueRepr::LowTag,
        "boxed" => ValueRepr::Boxed,
        r => return Err(Diag::new(v.repr.span, format!("unknown representation `{}` (nanbox, lowtag or boxed)", r))),
    };
    let mut variants: Vec<ValueVariantDef> = Vec::new();
    let mut have_unboxed = false;
    // (variant index, payload bits) of the variants that need a tag.
    let mut tagged: Vec<(usize, u32)> = Vec::new();
    for (i, vv) in v.variants.iter().enumerate() {
        let payload = match &vv.payload {
            Some(t) => Some(c.resolve_type(m, t)?),
            None => None,
        };
        if variants.iter().any(|x| x.name == vv.name.name) {
            return Err(Diag::new(vv.name.span, format!("duplicate variant `{}`", vv.name.name)));
        }
        if vv.unboxed && repr != ValueRepr::NanBox {
            return Err(Diag::new(vv.name.span, "`unboxed` is only meaningful for nanbox"));
        }
        match repr {
            ValueRepr::NanBox if vv.unboxed => {
                if payload != Some(Ty::F64) {
                    return Err(Diag::new(vv.name.span, "the `unboxed` variant of a nanbox must hold an f64"));
                }
                if have_unboxed {
                    return Err(Diag::new(vv.name.span, "a nanbox has at most one `unboxed` variant"));
                }
                have_unboxed = true;
            }
            ValueRepr::NanBox => match payload_width(payload.as_ref(), NAN_PTR_BITS) {
                Some(w) if w <= 48 => tagged.push((i, w)),
                _ => {
                    return Err(Diag::new(
                        vv.name.span,
                        "nanbox payloads must be integers of at most 48 bits (such as i48), bool or pointers (or the unboxed f64)",
                    ))
                }
            },
            ValueRepr::LowTag => match payload_width(payload.as_ref(), 64) {
                Some(w) if w <= 64 - LOW_BITS || matches!(payload, Some(Ty::Ptr(_) | Ty::Gc(_))) => tagged.push((i, w)),
                _ => {
                    return Err(Diag::new(
                        vv.name.span,
                        "lowtag payloads must be integers of at most 61 bits (such as i61), bool or pointers",
                    ))
                }
            },
            ValueRepr::Boxed => {
                if let Some(t) = &payload {
                    if !matches!(t, Ty::F64 | Ty::Int(_) | Ty::Bool | Ty::Ptr(_) | Ty::Gc(_)) {
                        return Err(Diag::new(vv.name.span, "boxed payloads must be numbers, bool or pointers"));
                    }
                }
            }
        }
        variants.push(ValueVariantDef { name: vv.name.name.clone(), payload, unboxed: vv.unboxed, tag: i as u64, shift: 0 });
    }
    let too_many = |i: usize, what: &str| Diag::new(v.variants[i].name.span, format!("no room for variant `{}` in the {}", v.variants[i].name.name, what));
    let mut num_limit = NAN_TAG_BASE << 48;
    match repr {
        ValueRepr::NanBox if tagged.len() <= 7 => {
            for (k, &(i, _)) in tagged.iter().enumerate() {
                variants[i].tag = NAN_TAG_BASE + k as u64;
                variants[i].shift = 48;
            }
        }
        ValueRepr::NanBox => {
            let mut need = tagged.clone();
            need.sort_by_key(|&(_, w)| std::cmp::Reverse(w));
            let mut free = vec![(NAN_HALF_BLOCK, 47)];
            free.extend((NAN_TAG_BASE..=0xFFFF).map(|t| (t << 48, 48)));
            // A block is (start, payload bits); halves are its low and high half.
            let blocks = buddy(free, &need, |start, s| (start, start + (1u64 << (s - 1)))).map_err(|i| {
                too_many(i, "nanbox's NaN space (seven 48-bit blocks for pointers and 48-bit integers, one 47-bit block)")
            })?;
            for (i, start) in blocks {
                let w = tagged.iter().find(|t| t.0 == i).unwrap().1;
                variants[i].tag = start >> w;
                variants[i].shift = w;
                num_limit = num_limit.min(start);
            }
        }
        ValueRepr::LowTag if tagged.len() <= 1 << LOW_BITS => {
            for &(i, _) in &tagged {
                variants[i].shift = LOW_BITS;
            }
        }
        ValueRepr::LowTag => {
            // A block is (code, 64 - code length): the words whose low bits
            // are `code`. Its halves extend the code by a 0 or a 1 bit.
            let mut need: Vec<(usize, u32)> = tagged
                .iter()
                .map(|&(i, w)| {
                    let is_ptr = matches!(variants[i].payload, Some(Ty::Ptr(_) | Ty::Gc(_)));
                    (i, if is_ptr { 64 - LOW_BITS } else { w })
                })
                .collect();
            need.sort_by_key(|&(_, w)| std::cmp::Reverse(w));
            let free = (0..1u64 << LOW_BITS).map(|c| (c, 64 - LOW_BITS)).collect();
            let blocks = buddy(free, &need, |code, s| (code, code | 1u64 << (64 - s)))
                .map_err(|i| too_many(i, "lowtag's tag space (8 three-bit tags; pointers and 61-bit integers take one each)"))?;
            for (i, code) in blocks {
                let w = need.iter().find(|t| t.0 == i).unwrap().1;
                variants[i].tag = code;
                variants[i].shift = 64 - w;
            }
        }
        _ => {}
    }
    Ok(ValueDef { name: format!("{}.{}", c.module_prefix(m), v.name.name), repr, variants, num_limit })
}

/// `(1 << bits) - 1`, for bits up to 64.
pub fn low_mask(bits: u32) -> u64 {
    if bits >= 64 { u64::MAX } else { (1u64 << bits) - 1 }
}

/// `x << s`, for s up to 64.
fn shl(x: u64, s: u32) -> u64 {
    if s >= 64 { 0 } else { x << s }
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

/// Per function: the locals whose f64 value is NaN-safe (see `nan_safe`):
/// immutable locals bound only from the `unboxed` variant of a nanbox
/// value, or by `let` from a NaN-safe expression.
fn nan_safe_local(g: &mut FnGen, l: LocalId) -> bool {
    let fid = g.fid;
    if !g.cg.nan_safe.contains_key(&fid) {
        let p = g.p();
        let f = &p.fns[fid as usize];
        let mut good = HashSet::new();
        let mut bad = HashSet::new();
        let mut lets: Vec<(LocalId, &TExpr)> = Vec::new();
        fn rec(p: &Program, pat: &TPat, under: bool, good: &mut HashSet<LocalId>, bad: &mut HashSet<LocalId>) {
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
            crate::tirwalk::for_each_bind(b, &mut |site| match site {
                crate::tirwalk::BindSite::Pat(pat) => rec(p, pat, false, &mut good, &mut bad),
                crate::tirwalk::BindSite::Let(l, init) => lets.push((l, init)),
            });
        }
        let immutable = |l: &LocalId| !f.locals[*l as usize].mutable;
        let mut set: HashSet<LocalId> = good.into_iter().filter(|l| !bad.contains(l) && immutable(l)).collect();
        // `let` locals, in order: an init can only use earlier locals.
        for (l, init) in lets {
            if immutable(&l) && !bad.contains(&l) && init.ty == Ty::F64 && nan_safe_in(&set, init) {
                set.insert(l);
            }
        }
        g.cg.nan_safe.insert(fid, set);
    }
    g.cg.nan_safe[&fid].contains(&l)
}

/// Whether the f64 expression `e` can only be a NaN that hardware made
/// from canonical inputs: such NaNs have a zero payload (0x7FF8.. or
/// 0xFFF8..), which is already a valid `unboxed` encoding, so building
/// the value needs no canonicalization. Leaves are literals, int-to-float
/// casts, and NaN-safe locals.
fn nan_safe(g: &mut FnGen, e: &TExpr) -> bool {
    if let TK::Local(l) = &e.kind {
        return nan_safe_local(g, *l);
    }
    nan_safe_local(g, u32::MAX);
    let set = &g.cg.nan_safe[&g.fid];
    nan_safe_in(set, e)
}

fn nan_safe_in(safe: &HashSet<LocalId>, e: &TExpr) -> bool {
    use crate::ast::{BinOp, UnOp};
    match &e.kind {
        TK::Float(x) => !x.is_nan(),
        TK::Cast(inner) => matches!(inner.ty, Ty::Int(_)),
        TK::Local(l) => safe.contains(l),
        TK::Binary(BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div, a, b) => nan_safe_in(safe, a) && nan_safe_in(safe, b),
        TK::Unary(UnOp::Neg, a) => nan_safe_in(safe, a),
        TK::Intrinsic(Intr::Sqrt | Intr::Floor, args) => args.iter().all(|a| nan_safe_in(safe, a)),
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
            let tag = (shl(var.tag, var.shift) as i64).to_string();
            match &var.payload {
                None => tag,
                Some(pt) => {
                    let b = match pt {
                        // Signed payloads are stored as their low bits.
                        Ty::Int(it) if it.signed && it.bits < 64 => g.inst(format!("zext i{} {} to i64", it.bits, vals[0])),
                        _ => payload_bits(g, pt, &vals[0]),
                    };
                    let b = if matches!(pt, Ty::Ptr(_) | Ty::Gc(_)) { g.inst(format!("and i64 {}, {}", b, low_mask(var.shift))) } else { b };
                    g.inst(format!("or i64 {}, {}", b, tag))
                }
            }
        }
        ValueRepr::LowTag => match &var.payload {
            None => (var.tag as i64).to_string(),
            Some(Ty::Ptr(_) | Ty::Gc(_)) => {
                let b = g.inst(format!("ptrtoint ptr {} to i64", vals[0]));
                g.inst(format!("or i64 {}, {}", b, var.tag))
            }
            Some(pt) => {
                let b = match pt {
                    // Sign-extended, unless the shift drops the extension
                    // bits anyway (i61): zext lets LLVM fold
                    // `zext(trunc(x >> 3)) << 3` into `x`.
                    Ty::Int(it) if it.signed && it.bits as u32 + var.shift < 64 => g.inst(format!("sext i{} {} to i64", it.bits, vals[0])),
                    _ => payload_bits(g, pt, &vals[0]),
                };
                let s = g.inst(format!("shl i64 {}, {}", b, var.shift));
                g.inst(format!("or i64 {}, {}", s, var.tag as i64))
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
        ValueRepr::NanBox if var.unboxed => g.inst(format!("icmp ult i64 {}, {}", v, d.num_limit as i64)),
        ValueRepr::NanBox if var.shift == 0 => g.inst(format!("icmp eq i64 {}, {}", v, var.tag as i64)),
        ValueRepr::NanBox => {
            let h = g.inst(format!("lshr i64 {}, {}", v, var.shift));
            g.inst(format!("icmp eq i64 {}, {}", h, var.tag as i64))
        }
        ValueRepr::LowTag if var.shift >= 64 => g.inst(format!("icmp eq i64 {}, {}", v, var.tag as i64)),
        ValueRepr::LowTag => {
            let l = g.inst(format!("and i64 {}, {}", v, low_mask(var.shift)));
            g.inst(format!("icmp eq i64 {}, {}", l, var.tag as i64))
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
                let m = g.inst(format!("and i64 {}, {}", v, low_mask(var.shift)));
                return g.inst(format!("inttoptr i64 {} to ptr", m));
            }
            v.to_string()
        }
        ValueRepr::LowTag => {
            if let Ty::Ptr(_) | Ty::Gc(_) = pt {
                let m = g.inst(format!("and i64 {}, {}", v, !low_mask(var.shift) as i64));
                return g.inst(format!("inttoptr i64 {} to ptr", m));
            }
            // `v` holds this variant, so `v ^ tag` has zero low bits and
            // the shift is `exact`: LLVM can then fold a later `<< shift`
            // (such as overflow-checked arithmetic) back into `v ^ tag`.
            let signed = matches!(pt, Ty::Int(it) if it.signed);
            let x = if var.tag == 0 { v.to_string() } else { g.inst(format!("xor i64 {}, {}", v, var.tag as i64)) };
            g.inst(format!("{} exact i64 {}, {}", if signed { "ashr" } else { "lshr" }, x, var.shift))
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
                    (!low_mask(var.shift), shl(var.tag, var.shift))
                } else {
                    (low_mask(var.shift), var.tag)
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
        // Unboxed variants of different value types: the strictest limit.
        let limit = loaded.iter().filter(|l| l.0.repr == ValueRepr::NanBox && l.0.variants[l.1 as usize].unboxed).map(|l| l.0.num_limit).min().unwrap();
        conds.push(g.inst(format!("icmp ult i64 {}, {}", acc, limit as i64)));
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
