//! Code generation for vm handlers and value types. (Stub.)

use crate::codegen::FnGen;
use crate::diag::Span;
use crate::tir::*;

#[derive(Clone, Debug)]
pub struct HandlerGen {}

pub fn handler_prologue(_g: &mut FnGen) {}

pub fn word_place(_g: &mut FnGen, _off: u32) -> String {
    unreachable!()
}

pub fn vm_expr(_g: &mut FnGen, _e: &TExpr, _v: &TVm) -> String {
    unreachable!()
}

pub fn test_value_variant(_g: &mut FnGen, _t: &Ty, _idx: u32, _subs: &[TPat], _ptr: &str, _fail: &str) {
    unreachable!()
}

pub fn make_value(_g: &mut FnGen, _t: &Ty, _idx: u32, _args: &[TExpr], _vals: &[String]) -> String {
    unreachable!()
}

pub fn value_tag(_g: &mut FnGen, _a: &TExpr, _v: &str) -> String {
    unreachable!()
}

pub fn raise_value(_g: &mut FnGen, _err: &str, _t: &Ty, _span: Span) {
    unreachable!()
}

pub fn before_call(_g: &mut FnGen, _f: &TExpr) -> Option<()> {
    None
}

pub fn after_call(_g: &mut FnGen, _f: &TExpr, _pre: Option<()>) {}
