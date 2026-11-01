//! `value` types: declared representations (nanbox, lowtag, boxed). (Stub.)

use crate::check::Checker;
use crate::codegen::FnGen;
use crate::diag::{Diag, Res};
use crate::tir::*;
use crate::vmast::ValueDecl;

pub fn check_value_decl(_c: &mut Checker, _m: usize, v: &ValueDecl) -> Res<ValueDef> {
    Err(Diag::new(v.name.span, "`value` declarations are not implemented yet"))
}

pub fn test_variant(_g: &mut FnGen, _t: &Ty, _idx: u32, _subs: &[TPat], _ptr: &str, _fail: &str) {
    unreachable!()
}

pub fn make(_g: &mut FnGen, _t: &Ty, _idx: u32, _args: &[TExpr], _vals: &[String]) -> String {
    unreachable!()
}
