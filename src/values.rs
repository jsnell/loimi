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
//!          0xFFF9..0xFFFF (48 bits each), widest first. If the 48-bit
//!          blocks run out (or a `Gc` variant is pinned `block 45`), `Gc`
//!          variants share 48-bit blocks, up to 8 in one, with a low tag
//!          in the pointer's 3 alignment bits: `tag << 48 | ptr | sub`.
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
/// The low bits of a `Gc` pointer that a shared nanbox block uses to tell
/// its variants apart (objects are at least 8-byte aligned), and the
/// block width of such a variant (`block 45`).
const NAN_SUB_BITS: u32 = 3;
const NAN_SHARED_BITS: u32 = NAN_PTR_BITS - NAN_SUB_BITS;

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
    let mut want_int_high = None;
    for o in &v.repr_opts {
        match o.name.as_str() {
            "int_high" if repr == ValueRepr::NanBox => want_int_high = Some(o.span),
            n => {
                return Err(Diag::new(
                    o.span,
                    format!("unknown option `{}` for `repr {}` (`nanbox` takes `int_high`; `lowtag` and `boxed` take none)", n, v.repr.name),
                ))
            }
        }
    }
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
        // `block N`: the block width, pinned. nanbox: the payload bits
        // below the tag; lowtag: the bits above the tag (64 - its length).
        let mut pinned = false;
        if let Some((n, sp)) = vv.block {
            if repr == ValueRepr::Boxed || vv.unboxed {
                return Err(Diag::new(
                    sp,
                    "`block` pins a variant's block in a nanbox or lowtag word; boxed values and the `unboxed` variant have none",
                ));
            }
            let max = if repr == ValueRepr::NanBox { 48 } else { 64 - LOW_BITS };
            let is_ptr = matches!(payload, Some(Ty::Ptr(_) | Ty::Gc(_)));
            // A nanbox `Gc` variant may share a 48-bit block (`block 45`).
            let shares = repr == ValueRepr::NanBox && matches!(payload, Some(Ty::Gc(_))) && n == NAN_SHARED_BITS;
            let min = if is_ptr { max } else { tagged.last().unwrap().1 };
            if (n < min || n > max) && !shares {
                let what = if is_ptr && repr == ValueRepr::NanBox && matches!(payload, Some(Ty::Gc(_))) {
                    format!("a `Gc` pointer takes a whole {}-bit block, or shares one with `block {}`", max, NAN_SHARED_BITS)
                } else if is_ptr {
                    format!("a pointer takes a whole {}-bit block", max)
                } else {
                    format!("it can be from its payload's {} bits to {}", min, max)
                };
                return Err(Diag::new(sp, format!("`block {}` is out of range for `{}`: {}", n, vv.name.name, what)));
            }
            tagged.last_mut().unwrap().1 = n;
            pinned = true;
        }
        variants.push(ValueVariantDef { name: vv.name.name.clone(), payload, unboxed: vv.unboxed, tag: i as u64, shift: 0, pinned, sub: None });
    }
    let too_many = |i: usize, what: &str| Diag::new(v.variants[i].name.span, format!("no room for variant `{}` in the {}", v.variants[i].name.name, what));
    let mut num_limit = NAN_TAG_BASE << 48;
    let classic = if repr == ValueRepr::NanBox { 48 } else { 64 - LOW_BITS };
    // Pinned blocks narrower than the classic ones.
    let narrow_pins: Vec<usize> = tagged.iter().filter(|&&(i, w)| variants[i].pinned && w < classic).map(|t| t.0).collect();
    // Variants pinned `block 45`: `Gc` pointers sharing a 48-bit block.
    let shared_pins: Vec<bool> = (0..variants.len())
        .map(|i| repr == ValueRepr::NanBox && variants[i].pinned && matches!(variants[i].payload, Some(Ty::Gc(_))) && tagged.iter().any(|t| t.0 == i && t.1 == NAN_SHARED_BITS))
        .collect();
    let shared_pin = |i: usize| shared_pins[i];
    let layout;
    match repr {
        ValueRepr::NanBox if tagged.len() <= 7 && narrow_pins.is_empty() => {
            layout = "classic";
            for (k, &(i, _)) in tagged.iter().enumerate() {
                variants[i].tag = NAN_TAG_BASE + k as u64;
                variants[i].shift = 48;
            }
        }
        // Eight: the classic layout for seven of them, in declaration
        // order, and the start of the 47-bit block for the narrowest (the
        // last of that width), or for the one variant pinned narrower than
        // 48 bits. So adding an eighth variant keeps every other variant's
        // 48-bit block: tuple tests that mix pointers with an i32 still
        // merge into one test (the packed layout would give an i32 a
        // 32-bit block).
        ValueRepr::NanBox if tagged.len() == 8 && narrow_pins.len() <= 1 && !narrow_pins.iter().any(|&i| shared_pin(i)) && tagged.iter().any(|t| t.1 <= 47) => {
            layout = "eight";
            let half = match narrow_pins.first() {
                Some(&i) => i,
                None => tagged.iter().rev().filter(|t| !variants[t.0].pinned).min_by_key(|t| t.1).unwrap().0,
            };
            let mut k = 0;
            for &(i, w) in &tagged {
                if i == half {
                    variants[i].tag = NAN_HALF_BLOCK >> w;
                    variants[i].shift = w;
                } else {
                    variants[i].tag = NAN_TAG_BASE + k;
                    variants[i].shift = 48;
                    k += 1;
                }
            }
            num_limit = NAN_HALF_BLOCK;
        }
        ValueRepr::NanBox => {
            // Packed: blocks sized to the payloads. If the 48-bit blocks
            // run out (or a variant is pinned `block 45`), `Gc` variants
            // share 48-bit blocks, up to 8 in one, told apart by the low 3
            // bits of the pointer: those pinned `block 45`, then as few
            // unpinned ones as fit, the last declared first.
            let mut free = vec![(NAN_HALF_BLOCK, 47)];
            free.extend((NAN_TAG_BASE..=0xFFFF).map(|t| (t << 48, 48)));
            let pinned_shared: Vec<usize> = tagged.iter().map(|t| t.0).filter(|&i| shared_pin(i)).collect();
            let candidates: Vec<usize> = tagged
                .iter()
                .rev()
                .map(|t| t.0)
                .filter(|&i| !variants[i].pinned && matches!(variants[i].payload, Some(Ty::Gc(_))))
                .collect();
            let group_size = 1usize << NAN_SUB_BITS;
            let mut result = None;
            let mut first_err = None;
            for k in 0..=candidates.len() {
                let mut shared: Vec<usize> = pinned_shared.iter().chain(&candidates[..k]).copied().collect();
                shared.sort();
                let groups: Vec<&[usize]> = shared.chunks(group_size).collect();
                // Pseudo-variants for the shared blocks: indices past the variants.
                let mut need: Vec<(usize, u32)> = tagged.iter().filter(|t| !shared.contains(&t.0)).copied().collect();
                need.extend((0..groups.len()).map(|g| (variants.len() + g, NAN_PTR_BITS)));
                need.sort_by_key(|&(_, w)| std::cmp::Reverse(w));
                // A block is (start, payload bits); halves are its low and high half.
                match buddy(free.clone(), &need, |start, s| (start, start + (1u64 << (s - 1)))) {
                    Ok(blocks) => {
                        result = Some((blocks, groups.iter().map(|g| g.to_vec()).collect::<Vec<_>>()));
                        break;
                    }
                    Err(i) => {
                        let i = if i >= variants.len() { groups[i - variants.len()][0] } else { i };
                        first_err.get_or_insert(i);
                    }
                }
            }
            let Some((blocks, groups)) = result else {
                return Err(too_many(
                    first_err.unwrap(),
                    "nanbox's NaN space (seven 48-bit blocks for 48-bit integers and pointers, up to 8 `Gc` pointers sharing one; one 47-bit block)",
                ));
            };
            layout = if groups.is_empty() { "packed" } else { "shared" };
            for (i, start) in blocks {
                if i >= variants.len() {
                    for (l, &j) in groups[i - variants.len()].iter().enumerate() {
                        variants[j].tag = start >> NAN_PTR_BITS;
                        variants[j].shift = NAN_PTR_BITS;
                        variants[j].sub = Some(l as u64);
                    }
                } else {
                    let w = tagged.iter().find(|t| t.0 == i).unwrap().1;
                    variants[i].tag = start >> w;
                    variants[i].shift = w;
                }
                num_limit = num_limit.min(start);
            }
        }
        ValueRepr::LowTag if tagged.len() <= 1 << LOW_BITS && narrow_pins.is_empty() => {
            layout = "classic";
            for &(i, _) in &tagged {
                variants[i].shift = LOW_BITS;
            }
        }
        ValueRepr::LowTag => {
            layout = "packed";
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
        _ => layout = "boxed",
    }
    // A layout other than the classic one narrows blocks. Say so, unless
    // every narrowed block is pinned (`block N`): adding a variant must not
    // silently change how the others are encoded (examples/mini/
    // COROUTINES.md, friction 2).
    if layout == "eight" || layout == "packed" || layout == "shared" {
        let narrowed: Vec<usize> =
            tagged.iter().map(|t| t.0).filter(|&i| !variants[i].pinned && block_bits(repr, &variants[i]) < classic).collect();
        if let Some(&first) = narrowed.first() {
            let list: Vec<String> =
                narrowed.iter().map(|&i| { let b = block_bits(repr, &variants[i]); format!("`{}` {} bit{}", variants[i].name, b, if b == 1 { "" } else { "s" }) }).collect();
            let vn = &variants[first];
            let decl = match &vn.payload {
                Some(t) => format!("{}({})", vn.name, c.ty_name(t)),
                None => vn.name.clone(),
            };
            let w = Diag::warning(
                "value_layout",
                v.name.span,
                if tagged.len() > if repr == ValueRepr::NanBox { 7 } else { 8 } {
                    format!(
                        "`{}` has {} tagged variants, more than the classic {} layout holds ({}), so these get blocks narrower than {} bits: {}",
                        v.name.name,
                        tagged.len(),
                        v.repr.name,
                        if repr == ValueRepr::NanBox { "7, or 8 if one is at most 47 bits wide" } else { "8" },
                        classic,
                        list.join(", ")
                    )
                } else {
                    // Few enough for the classic layout, but a pin narrower
                    // than the classic width selected another one.
                    format!(
                        "`{}` pins a block narrower than {} bits, which selects the {} layout, so these get blocks narrower than {} bits too: {}",
                        v.name.name, classic, layout, classic, list.join(", ")
                    )
                },
            )
            .note(
                v.variants[first].name.span,
                format!(
                    "a tuple pattern mixing a narrow variant with wider ones tests each separately, and code that tests raw words must use the new widths (`{}.{}.mask`); pin each width to make it part of the declaration, `{} block {}` (or `block {}` to keep the classic block), or allow this with `#[allow(value_layout)]`; `loimi build --explain-values` prints the layout",
                    v.name.name,
                    vn.name,
                    decl,
                    block_bits(repr, vn),
                    classic
                ),
            );
            c.warnings.push(w);
        }
    }
    // `nanbox(int_high)`: the first integer variant with a whole 48-bit
    // block gets the zero low tag (`k` is its block's 16-bit tag).
    let mut int_high = None;
    if let Some(sp) = want_int_high {
        let Some(iv) = variants.iter().find(|x| matches!(x.payload, Some(Ty::Int(_))) && x.sub.is_none() && x.shift == NAN_PTR_BITS) else {
            return Err(Diag::new(
                sp,
                format!(
                    "`int_high` puts an integer variant's payload in the high 48 bits, but `{}` has no integer variant with a 48-bit block (declare one such as `Int(i48)`, or pin `block 48`)",
                    v.name.name
                ),
            ));
        };
        int_high = Some(iv.tag);
    }
    // `#[scan_raw_words]`: a root scanner may read memory that mixes
    // values with raw pointers and small integers as values. Only a
    // nanbox whose unboxed (float) variant takes every word below 2^48
    // guarantees that (not `int_high`: there the low 16 bits of a word
    // pick the variant, so a raw pointer can read as a `Gc` one).
    if let Some(sp) = v.scan_raw {
        let ok = repr == ValueRepr::NanBox
            && int_high.is_none()
            && variants.iter().any(|x| x.unboxed && x.payload.as_ref().is_none_or(|t| !matches!(t, Ty::Gc(_))))
            && num_limit >= 1u64 << 48;
        if !ok {
            return Err(Diag::new(
                sp,
                format!(
                    "`#[scan_raw_words]`: `{}` does not read every word below 2^48 as a variant without references (that needs `repr nanbox` with an unboxed float variant, and not `int_high`)",
                    v.name.name
                ),
            ));
        }
    }
    Ok(ValueDef { name: format!("{}.{}", c.module_prefix(m), v.name.name), repr, variants, num_limit, int_high, layout })
}

/// A tagged variant's block width in bits: nanbox, the payload bits below
/// the tag; lowtag, the bits above it.
pub fn block_bits(repr: ValueRepr, v: &ValueVariantDef) -> u32 {
    if v.sub.is_some() {
        return NAN_SHARED_BITS;
    }
    match repr {
        ValueRepr::NanBox => v.shift,
        ValueRepr::LowTag => 64 - v.shift,
        ValueRepr::Boxed => 64,
    }
}

/// `(1 << bits) - 1`, for bits up to 64.
pub fn low_mask(bits: u32) -> u64 {
    if bits >= 64 { u64::MAX } else { (1u64 << bits) - 1 }
}

/// `x << s`, for s up to 64.
fn shl(x: u64, s: u32) -> u64 {
    if s >= 64 { 0 } else { x << s }
}

/// `nanbox(int_high)`: the word of the layout's word `c` (`rotl(c, 16) ^ k`).
pub fn int_high_word(k: u64, c: u64) -> u64 {
    c.rotate_left(16) ^ k
}

/// `nanbox(int_high)`: IR for the layout's word of `w` (`rotr(w ^ k, 16)`).
fn int_high_decode(g: &mut FnGen, k: u64, w: &str) -> String {
    g.cg.decls.insert("declare i64 @llvm.fshl.i64(i64, i64, i64)");
    let x = g.inst(format!("xor i64 {}, {}", w, k));
    g.inst(format!("call i64 @llvm.fshl.i64(i64 {}, i64 {}, i64 48)", x, x))
}

/// `nanbox(int_high)`: IR for the word of the layout's word `c`.
fn int_high_encode(g: &mut FnGen, k: u64, c: &str) -> String {
    g.cg.decls.insert("declare i64 @llvm.fshl.i64(i64, i64, i64)");
    let r = g.inst(format!("call i64 @llvm.fshl.i64(i64 {}, i64 {}, i64 16)", c, c));
    g.inst(format!("xor i64 {}, {}", r, k))
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
        TK::Intrinsic(
            Intr::Sqrt | Intr::Floor | Intr::Ceil | Intr::Trunc | Intr::Nearest | Intr::MinNum | Intr::MaxNum | Intr::Minimum | Intr::Maximum,
            args,
        ) => args.iter().all(|a| nan_safe_in(safe, a)),
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
                let bits = if nan_safe(g, &args[0]) {
                    bits
                } else {
                    let nan = g.inst(format!("fcmp uno double {}, {}", vals[0], vals[0]));
                    g.inst(format!("select i1 {}, i64 {}, i64 {}", nan, CANONICAL_NAN as i64, bits))
                };
                return match d.int_high {
                    Some(k) => int_high_encode(g, k, &bits),
                    None => bits,
                };
            }
            if let Some(k) = d.int_high {
                // The payload sits at bit 16 (above the low tag); a
                // pointer's bits above 48 shift out.
                let tag = int_high_word(k, shl(var.tag, var.shift) | var.sub.unwrap_or(0));
                return match &var.payload {
                    None => (tag as i64).to_string(),
                    Some(pt) => {
                        let b = match pt {
                            Ty::Int(it) if it.signed && it.bits < 64 => g.inst(format!("zext i{} {} to i64", it.bits, vals[0])),
                            _ => payload_bits(g, pt, &vals[0]),
                        };
                        let s = g.inst(format!("shl i64 {}, 16", b));
                        if tag == 0 { s } else { g.inst(format!("or i64 {}, {}", s, tag as i64)) }
                    }
                };
            }
            let tag = ((shl(var.tag, var.shift) | var.sub.unwrap_or(0)) as i64).to_string();
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
    if let Some(k) = d.int_high {
        if var.unboxed {
            let c = int_high_decode(g, k, v);
            return g.inst(format!("icmp ult i64 {}, {}", c, d.num_limit as i64));
        }
        let (mask, tag) = mask_tag(d, var);
        if mask == u64::MAX {
            return g.inst(format!("icmp eq i64 {}, {}", v, tag as i64));
        }
        let m = g.inst(format!("and i64 {}, {}", v, mask as i64));
        return g.inst(format!("icmp eq i64 {}, {}", m, tag as i64));
    }
    match d.repr {
        ValueRepr::NanBox if var.unboxed => g.inst(format!("icmp ult i64 {}, {}", v, d.num_limit as i64)),
        ValueRepr::NanBox if var.sub.is_some() => {
            let (mask, tag) = mask_tag(d, var);
            let m = g.inst(format!("and i64 {}, {}", v, mask as i64));
            g.inst(format!("icmp eq i64 {}, {}", m, tag as i64))
        }
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
        ValueRepr::NanBox if d.int_high.is_some() => {
            let k = d.int_high.unwrap();
            if var.unboxed {
                let c = int_high_decode(g, k, v);
                return g.inst(format!("bitcast i64 {} to double", c));
            }
            // The payload is the bits above the low 16. For the integer
            // variant with the zero low tag the shift is `exact`, so LLVM
            // folds it into a later `<< 16` (checked arithmetic, re-tagging):
            // `Int(x + y)` is `add; jo` on the words.
            let zero_tag = mask_tag(d, var).1 == 0;
            let signed = matches!(pt, Ty::Int(it) if it.signed);
            let s = if zero_tag && signed {
                g.inst(format!("ashr exact i64 {}, 16", v))
            } else {
                g.inst(format!("lshr {}i64 {}, 16", if zero_tag { "exact " } else { "" }, v))
            };
            if let Ty::Ptr(_) | Ty::Gc(_) = pt {
                let p = g.inst(format!("inttoptr i64 {} to ptr", s));
                return match var.sub {
                    Some(l) if l != 0 => g.inst(format!("getelementptr i8, ptr {}, i64 {}", p, -(l as i64))),
                    _ => p,
                };
            }
            s
        }
        ValueRepr::NanBox => {
            if var.unboxed {
                return g.inst(format!("bitcast i64 {} to double", v));
            }
            if let Ty::Ptr(_) | Ty::Gc(_) = pt {
                let m = g.inst(format!("and i64 {}, {}", v, low_mask(var.shift)));
                let p = g.inst(format!("inttoptr i64 {} to ptr", m));
                // A shared block: the pointer plus its low tag. Subtracting
                // the tag as an offset lets LLVM fold it into the
                // displacement of the loads and stores through it.
                return match var.sub {
                    Some(l) if l != 0 => g.inst(format!("getelementptr i8, ptr {}, i64 {}", p, -(l as i64))),
                    _ => p,
                };
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
    // With a variant of a shared block among them, whole-block `Gc`
    // variants of a nanbox merge with it: their pointers' low bits are 0.
    let shared_mask = loaded.iter().find_map(|(d, idx, _, _)| {
        let var = &d.variants[*idx as usize];
        var.sub.map(|_| mask_tag(d, var).0)
    });
    for (d, idx, _, v) in &loaded {
        let var = &d.variants[*idx as usize];
        match d.repr {
            ValueRepr::NanBox if var.unboxed => match d.int_high {
                Some(k) => {
                    let c = int_high_decode(g, k, v);
                    unboxed.push(c)
                }
                None => unboxed.push(v.clone()),
            },
            ValueRepr::NanBox | ValueRepr::LowTag => {
                let (mut m, tagbits) = mask_tag(d, var);
                if let Some(sm) = shared_mask {
                    if d.repr == ValueRepr::NanBox && var.shift == NAN_PTR_BITS && matches!(var.payload, Some(Ty::Gc(_))) && sm & m == m {
                        m = sm;
                    }
                }
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

/// `V.X.name`: an encoding constant of variant `idx` (language.md section
/// 2), with whether it is a u64 (else typed like an integer literal).
/// `mask`: the bits that identify the variant; `tag`: their value, so a
/// word `w` holds the variant iff `w & mask == tag`; `block`: the block
/// width in bits; `limit`: words below it are the `unboxed` variant.
pub fn variant_const(d: &ValueDef, idx: u32, name: &str) -> Result<(i128, bool), String> {
    let var = &d.variants[idx as usize];
    let short = crate::check::short_name(&d.name);
    if d.repr == ValueRepr::Boxed {
        return Err(format!("`{}` is boxed (a tag word and a payload word): its variants have no encoding constants", short));
    }
    if var.unboxed {
        return match (name, d.int_high) {
            ("limit", Some(_)) => Err(format!(
                "`{}` is `nanbox(int_high)`: its unboxed variant `{}` is not the words below a limit (its words are those that no tagged variant's `mask` and `tag` match; it has `xor`)",
                short, var.name
            )),
            ("limit", None) => Ok((d.num_limit as i128, true)),
            // int_high: the word of an f64 with bits `b` is
            // `rotl(b, 16) ^ xor`, so a word's f64 bits are
            // `rotr(w ^ xor, 16)` (a tagged word decodes to a NaN).
            ("xor", Some(k)) => Ok((k as i128, true)),
            ("xor", None) => Err(format!(
                "no constant `xor` for `{}.{}`: `{}` is not `nanbox(int_high)` (its words are the f64's bits; it has `limit`)",
                short, var.name, short
            )),
            (_, Some(_)) => Err(format!(
                "no constant `{}` for the unboxed variant `{}.{}`: it has `xor` (its word is `rotl(bits, 16) ^ xor`)",
                name, short, var.name
            )),
            (_, None) => Err(format!(
                "no constant `{}` for the unboxed variant `{}.{}`: it has `limit` (its words are those below it)",
                name, short, var.name
            )),
        };
    }
    let (mask, tag) = mask_tag(d, var);
    match name {
        "mask" => Ok((mask as i128, true)),
        "tag" => Ok((tag as i128, true)),
        "block" => Ok((block_bits(d.repr, var) as i128, false)),
        _ => Err(format!("no constant `{}` for variant `{}.{}`: it has `mask`, `tag` and `block`", name, short, var.name)),
    }
}

/// The variant that the all-zero word (or, boxed, the zero tag) encodes,
/// if any: what a zero-filled trailing array of `d` holds.
pub fn zero_variant(d: &ValueDef) -> Option<usize> {
    if d.repr == ValueRepr::Boxed {
        return d.variants.iter().position(|v| v.tag == 0);
    }
    if d.int_high.is_some() {
        // The all-zero word is the integer 0.
        return d.variants.iter().position(|v| !v.unboxed && mask_tag(d, v).1 == 0);
    }
    if d.repr == ValueRepr::NanBox && d.variants.iter().any(|v| v.unboxed) {
        // Words below `num_limit` are the unboxed f64: 0 is 0.0.
        return d.variants.iter().position(|v| v.unboxed);
    }
    d.variants.iter().position(|v| {
        if v.unboxed {
            return false;
        }
        if d.repr == ValueRepr::LowTag && v.shift >= 64 {
            return v.tag == 0;
        }
        let (_, tag) = mask_tag(d, v);
        tag == 0
    })
}

/// A tagged variant's (mask, tag): a word `w` holds it iff `w & mask == tag`.
fn mask_tag(d: &ValueDef, v: &ValueVariantDef) -> (u64, u64) {
    let (m, t) = if let Some(l) = v.sub {
        (!low_mask(v.shift) | low_mask(NAN_SUB_BITS), shl(v.tag, v.shift) | l)
    } else {
        match d.repr {
            ValueRepr::NanBox => (!low_mask(v.shift), shl(v.tag, v.shift)),
            _ => (low_mask(v.shift), v.tag),
        }
    };
    // int_high: the mask always covers the high 16 bits of the layout's
    // word, which become the low 16, so `^ k` stays inside it.
    match d.int_high {
        Some(k) => (m.rotate_left(16), int_high_word(k, t)),
        None => (m, t),
    }
}

/// `0xFFF8_8000_0000_0000`.
fn hex(x: u64) -> String {
    let h = format!("{:016X}", x);
    format!("0x{}_{}_{}_{}", &h[0..4], &h[4..8], &h[8..12], &h[12..16])
}

/// `loimi build --explain-values`: each value type's encoding, and which
/// variants' tests merge in tuple patterns.
pub fn explain(c: &Checker) -> String {
    use std::fmt::Write;
    let mut s = String::new();
    for d in &c.prog.values {
        let repr = match d.repr {
            ValueRepr::NanBox => "nanbox",
            ValueRepr::LowTag => "lowtag",
            ValueRepr::Boxed => "boxed",
        };
        let ntag = d.variants.iter().filter(|v| !v.unboxed).count();
        let why = match (d.repr, d.layout) {
            (ValueRepr::NanBox, "classic") => "every tagged variant has a 48-bit block, 0xFFF9.. in declaration order",
            (ValueRepr::NanBox, "eight") => {
                "seven tagged variants have 48-bit blocks, 0xFFF9.. in declaration order, and one the start of the 47-bit block 0xFFF8_8000_0000_0000"
            }
            (ValueRepr::NanBox, "shared") => {
                "more 48-bit blocks needed than the NaN space has (or `block 45` pinned): blocks sized to the payloads, and `Gc` variants sharing 48-bit blocks, up to 8 in one, told apart by the low 3 bits of the pointer"
            }
            (ValueRepr::NanBox, _) => {
                "more than 8 tagged variants (or 8 with none narrower than 48 bits, or narrow blocks pinned): blocks sized to the payloads (or to `block N`), allocated widest first"
            }
            (ValueRepr::LowTag, "classic") => "every variant has a 3-bit tag (the variant's index) below a 61-bit block",
            (ValueRepr::LowTag, _) => {
                "more than 8 variants (or narrow blocks pinned): pointers and 61-bit integers have 3-bit tags, narrower variants longer ones"
            }
            _ => "a tag word and a payload word",
        };
        let opts = if d.int_high.is_some() { "(int_high)" } else { "" };
        let _ = writeln!(s, "value {} repr {}{}: {} layout, {} tagged variant{}", d.name, repr, opts, d.layout, ntag, if ntag == 1 { "" } else { "s" });
        let _ = writeln!(s, "  {}", why);
        if let Some(k) = d.int_high {
            let iv = d.variants.iter().find(|v| !v.unboxed && mask_tag(d, v).1 == 0).map_or("", |v| v.name.as_str());
            let _ = writeln!(
                s,
                "  int_high: each word is that layout's word rotated left by 16 bits and xored with {:#06X}: `{}` is its payload << 16 (low 16 bits zero; checked + - * are `add; jo` on the words), every other variant has nonzero low 16 bits, a pointer payload is `word >> 16`",
                k, iv
            );
            if let Some(u) = d.variants.iter().find(|v| v.unboxed) {
                let _ = writeln!(
                    s,
                    "  int_high costs: `{}` is decoded with `xor; ror` and encoded with `rol; xor`, and its test decodes before the compare; `{}.{}.limit` does not exist",
                    u.name,
                    crate::check::short_name(&d.name),
                    u.name
                );
            }
        }
        let decl = |v: &ValueVariantDef| match &v.payload {
            Some(t) => format!("{}({})", v.name, c.ty_name(t)),
            None => v.name.clone(),
        };
        let width = d.variants.iter().map(|v| decl(v).len()).max().unwrap_or(0);
        for (i, v) in d.variants.iter().enumerate() {
            let name = format!("{:w$}", decl(v), w = width);
            if d.repr == ValueRepr::Boxed {
                let _ = writeln!(s, "  {}  tag {}", name, i);
                continue;
            }
            if v.unboxed {
                match d.int_high {
                    Some(k) => {
                        let _ = writeln!(s, "  {}  unboxed: the words w with rotr(w ^ {:#06X}, 16) below {} (the f64's bits)", name, k, hex(d.num_limit));
                    }
                    None => {
                        let _ = writeln!(s, "  {}  unboxed: the words below {} (`limit`)", name, hex(d.num_limit));
                    }
                }
                continue;
            }
            let (mask, tag) = mask_tag(d, v);
            let words = if mask == u64::MAX {
                format!("the word {}", hex(tag))
            } else if let (Some(l), Some(_)) = (v.sub, d.int_high) {
                format!("words {} | ptr << 16, ptr's low 3 bits {}", hex(tag & !(low_mask(NAN_SUB_BITS) << 16)), l)
            } else if let Some(l) = v.sub {
                format!("words {} | ptr, low 3 bits {}", hex(tag & !low_mask(NAN_SUB_BITS)), l)
            } else if d.int_high.is_some() {
                format!("words {} | payload << 16", hex(tag))
            } else if d.repr == ValueRepr::NanBox {
                format!("words {} ..= {}", hex(tag), hex(tag | !mask))
            } else {
                format!("words whose low {} bits are {:#b}", v.shift, v.tag)
            };
            let _ = writeln!(
                s,
                "  {}  block {:2}{}  mask {}  tag {}  {}",
                name,
                block_bits(d.repr, v),
                if v.pinned { " (pinned)" } else { "         " },
                hex(mask),
                hex(tag),
                words
            );
        }
        if d.repr == ValueRepr::Boxed {
            let _ = writeln!(s, "  tuple patterns test each element's tag word separately");
            continue;
        }
        // Tuple patterns: tag tests merge when the masks are equal.
        let mut groups: Vec<(u32, Vec<&str>)> = Vec::new();
        for v in d.variants.iter().filter(|v| !v.unboxed) {
            let b = block_bits(d.repr, v);
            match groups.iter_mut().find(|g| g.0 == b) {
                Some(g) => g.1.push(&v.name),
                None => groups.push((b, vec![&v.name])),
            }
        }
        groups.sort_by_key(|g| std::cmp::Reverse(g.0));
        let _ = writeln!(s, "  tuple patterns: elements whose variants have the same block width merge into one test, ((a ^ tag) | (b ^ tag') | ..) & mask == 0:");
        for (b, names) in &groups {
            let _ = writeln!(s, "    block {:2}: {}", b, names.join(", "));
        }
        if let Some(u) = d.variants.iter().find(|v| v.unboxed) {
            if d.int_high.is_some() {
                let _ = writeln!(s, "    unboxed:  {}  (umax(decoded a, b, ..) < {})", u.name, hex(d.num_limit));
            } else {
                let _ = writeln!(s, "    unboxed:  {}  (umax(a, b, ..) < limit)", u.name);
            }
        }
        if d.variants.iter().any(|v| v.sub.is_some()) {
            let _ = writeln!(s, "    (with a block-{} variant, the 48-bit blocks of `Gc` variants merge into its test: their pointers' low 3 bits are 0)", NAN_SHARED_BITS);
        }
        if groups.len() + d.variants.iter().filter(|v| v.unboxed).count() > 1 {
            let _ = writeln!(s, "  elements from different lines do not merge (their tags sit at different bits): one test per line, and-ed");
        }
    }
    s
}
