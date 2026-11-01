//! Checking and expansion of `vm` and `value` declarations.
//! (Stub; filled in by the VM milestone.)

use crate::ast::*;
use crate::check::{Checker, FnCx};
use crate::diag::{Diag, Res, Span};
use crate::tir::*;
use crate::vmast::*;

#[derive(Clone, Debug)]
pub struct HandlerCx {
    pub vm: u32,
    pub is_handler: bool,
}

pub fn check_value_decl(_c: &mut Checker, _m: usize, v: &ValueDecl) -> Res<ValueDef> {
    Err(Diag::new(v.name.span, "`value` declarations are not implemented yet"))
}

pub fn check_vm(c: &mut Checker, i: usize) -> Res<()> {
    Err(Diag::new(c.vm_decls[i].1.name.span, "`vm` declarations are not implemented yet"))
}

pub fn check_vm_expr(_c: &mut Checker, _cx: &mut FnCx, _v: &VmExpr, _exp: Option<&Ty>, span: Span) -> Res<TExpr> {
    Err(Diag::new(span, "vm control words are only allowed in vm handlers"))
}

pub fn vm_ident(_c: &mut Checker, _cx: &mut FnCx, _name: &str, _span: Span) -> Res<Option<TExpr>> {
    Ok(None)
}

pub fn vm_fn(_c: &Checker, _cx: &FnCx, _name: &str) -> Option<FnId> {
    None
}

pub fn vm_fn_call(_c: &mut Checker, _cx: &mut FnCx, _name: &str, _args: &[Expr], _span: Span) -> Res<Option<TExpr>> {
    Ok(None)
}

pub fn check_raise_ty(_c: &mut Checker, _cx: &mut FnCx, _e: &Ty, span: Span) -> Res<()> {
    Err(Diag::new(span, "`?` on a result outside of a function returning a result"))
}

pub fn check_call_effects(_c: &mut Checker, _cx: &mut FnCx, _f: FnId, _span: Span) -> Res<()> {
    Ok(())
}
