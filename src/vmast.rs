//! Syntax for `vm` and `value` declarations.

use crate::ast::*;

#[derive(Clone, Debug)]
pub struct OperandDecl {
    pub name: Ident,
    pub body: Expr,
    pub verify: Option<Expr>,
    /// `display "r{}"`: how the generated disassembler prints the operand
    /// (`{}` is the decoded field, or a label's absolute target).
    pub display: Option<String>,
}

#[derive(Clone, Debug)]
pub struct FormatField {
    pub name: Ident,
    pub bits: u32,
    pub signed: bool,
}

#[derive(Clone, Debug)]
pub struct FormatDecl {
    pub name: Ident,
    pub fields: Vec<FormatField>,
}

#[derive(Clone, Debug)]
pub struct CacheDecl {
    pub name: Ident,
    /// Each field occupies one code word.
    pub fields: Vec<(Ident, TypeExpr, bool)>,
}

#[derive(Clone, Debug)]
pub struct OpParam {
    pub name: Ident,
    pub kind: Ident,
}

#[derive(Clone, Debug)]
#[allow(dead_code)]
pub struct OpDecl {
    pub name: Ident,
    pub params: Vec<OpParam>,
    pub format: Ident,
    pub cache: Option<Ident>,
    pub body: Block,
    /// Set for members of a `family`: the family's generic op name.
    pub family: Option<String>,
    /// `then OP`: the instruction after this one is an OP (or a member of
    /// its family), and the body can read its operands as `then.x`.
    pub then: Option<Ident>,
}

#[derive(Clone, Debug)]
pub struct SuperDecl {
    pub name: Ident,
    pub parts: Vec<Ident>,
    /// `#[forward]`: forward each part's written register to the next
    /// part's reads (see vmcheck::check_handler).
    pub forward: bool,
}

#[derive(Clone, Debug)]
pub struct EntryDecl {
    pub name: Ident,
    pub params: Vec<Param>,
    pub ret: TypeExpr,
    pub body: Block,
    /// `uses reenter`: a nested entry, which runs guest code above the
    /// frames of a run that is already active (`language.md` §10).
    pub effects: Attrs,
}

#[derive(Clone, Debug)]
pub struct VariantDecl {
    pub name: Ident,
    /// None = all ops.
    pub wraps: Option<Vec<Ident>>,
    pub before: Block,
}

#[derive(Clone, Debug)]
pub struct RaiseDecl {
    pub param: Param,
    pub body: Block,
}

#[derive(Clone, Debug)]
pub struct VmDecl {
    pub name: Ident,
    pub attrs: Attrs,
    pub context: Option<Param>,
    pub state: Vec<Param>,
    pub operands: Vec<OperandDecl>,
    pub word: Option<TypeExpr>,
    pub formats: Vec<FormatDecl>,
    pub caches: Vec<CacheDecl>,
    pub ops: Vec<OpDecl>,
    pub supers: Vec<SuperDecl>,
    pub fns: Vec<FnDecl>,
    pub entries: Vec<EntryDecl>,
    pub on_raise: Option<RaiseDecl>,
    pub variants: Vec<VariantDecl>,
    pub sync: Option<Block>,
    /// `reload { .. }`: run after a call that may re-enter the vm, to
    /// reload the state registers from wherever the vm keeps them.
    pub reload: Option<Block>,
    /// Parameters of the generated verifier (`verify_context proto: Gc<Proto>;`).
    pub verify_ctx: Vec<Param>,
    pub is_pub: bool,
}

#[derive(Clone, Debug)]
pub struct ValueVariant {
    pub name: Ident,
    pub payload: Option<TypeExpr>,
    pub unboxed: bool,
}

#[derive(Clone, Debug)]
pub struct ValueDecl {
    pub name: Ident,
    pub repr: Ident,
    pub variants: Vec<ValueVariant>,
    pub is_pub: bool,
}
