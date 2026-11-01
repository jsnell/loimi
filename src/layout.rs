//! Type sizes and alignments (x86-64 / AArch64 LP64 rules, matching LLVM's
//! default layout of the generated types).

use crate::tir::*;

pub fn align_of(p: &Program, t: &Ty) -> u64 {
    match t {
        Ty::Int(it) => (it.bits as u64 / 8).max(1),
        Ty::F32 => 4,
        Ty::F64 => 8,
        Ty::Bool | Ty::Unit | Ty::Never => 1,
        Ty::Ptr(_) | Ty::Fn(..) | Ty::Slice(_) | Ty::Value(_) => 8,
        Ty::Array(e, _) => align_of(p, e),
        Ty::Option(inner) => {
            if inner.is_nullable_ptr() {
                8
            } else {
                align_of(p, inner).max(1)
            }
        }
        Ty::Result(a, b) => align_of(p, a).max(align_of(p, b)),
        Ty::Tuple(ts) => ts.iter().map(|t| align_of(p, t)).max().unwrap_or(1),
        Ty::Struct(i) => p.structs[*i as usize].fields.iter().map(|(_, t)| align_of(p, t)).max().unwrap_or(1),
        Ty::Enum(i) => {
            let e = &p.enums[*i as usize];
            let tag = (e.tag.bits as u64 / 8).max(1);
            if e.has_payload() { tag.max(8) } else { tag }
        }
    }
}

pub fn round_up(x: u64, a: u64) -> u64 {
    x.div_ceil(a) * a
}

pub fn struct_size(p: &Program, fields: &[Ty]) -> u64 {
    let mut off = 0;
    let mut align = 1;
    for f in fields {
        let a = align_of(p, f);
        align = align.max(a);
        off = round_up(off, a) + size_of(p, f);
    }
    round_up(off, align)
}

/// Number of i64 words in an enum's payload area.
pub fn enum_payload_words(p: &Program, e: &EnumDef) -> u64 {
    e.variants.iter().map(|v| struct_size(p, &v.fields).div_ceil(8)).max().unwrap_or(0)
}

pub fn size_of(p: &Program, t: &Ty) -> u64 {
    match t {
        Ty::Int(it) => (it.bits as u64 / 8).max(1),
        Ty::F32 => 4,
        Ty::F64 => 8,
        Ty::Bool => 1,
        Ty::Unit | Ty::Never => 0,
        Ty::Ptr(_) | Ty::Fn(..) => 8,
        Ty::Value(i) => if p.values[*i as usize].repr == ValueRepr::Boxed { 16 } else { 8 },
        Ty::Slice(_) => 16,
        Ty::Array(e, n) => size_of(p, e) * n,
        Ty::Option(inner) => {
            if inner.is_nullable_ptr() {
                8
            } else {
                struct_size(p, &[Ty::Bool, (**inner).clone()])
            }
        }
        Ty::Result(a, b) => struct_size(p, &[Ty::Bool, (**a).clone(), (**b).clone()]),
        Ty::Tuple(ts) => struct_size(p, ts),
        Ty::Struct(i) => {
            let fs: Vec<Ty> = p.structs[*i as usize].fields.iter().map(|(_, t)| t.clone()).collect();
            struct_size(p, &fs)
        }
        Ty::Enum(i) => {
            let e = &p.enums[*i as usize];
            let tag = (e.tag.bits as u64 / 8).max(1);
            if e.has_payload() {
                round_up(tag, 8) + enum_payload_words(p, e) * 8
            } else {
                tag
            }
        }
    }
}
