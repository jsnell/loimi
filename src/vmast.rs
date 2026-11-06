//! Syntax for `vm` and `value` declarations.

use crate::ast::*;

#[derive(Clone, Debug)]
pub struct OperandDecl {
    pub name: Ident,
    pub body: Expr,
    pub verify: Option<Expr>,
    /// `const EXPR`: the operand reads `S[it]` for a state register `S`
    /// whose value, while code verified against a context runs, is EXPR
    /// of that context (a constant pool), and what it reads never changes
    /// after verification.
    pub konst: Option<Expr>,
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
    /// Each field starts a code word (and takes as many as its type
    /// needs). For `cache X = S;` these are `S`'s fields, filled in by
    /// `vmexpand::cache_structs` before the vm is expanded.
    pub fields: Vec<(Ident, TypeExpr, bool)>,
    /// `cache X = S;`: the layout is struct `S`'s fields, and `&cache`
    /// is a `*S` (language.md §7.1).
    pub from: Option<TypeExpr>,
}

#[derive(Clone, Debug)]
pub struct OpParam {
    pub name: Ident,
    pub kind: Ident,
}

/// One value of a stack effect: `a`, or `xs[n]` for a counted run of
/// values (`n` an expression over the op's raw operand fields and the
/// verify context).
#[derive(Clone, Debug)]
pub struct StackItem {
    pub name: Ident,
    pub count: Option<Expr>,
}

/// `stack(a b -- c)`: what an op pops and pushes on a stack VM's operand
/// stack. `rest` is `..` before the inputs: the op takes every value.
#[derive(Clone, Debug)]
pub struct StackEffect {
    pub rest: bool,
    pub ins: Vec<StackItem>,
    pub outs: Vec<StackItem>,
}

#[derive(Clone, Debug)]
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
    /// `stack(a b -- c)`: the op's stack effect (stack VMs).
    pub stack: Option<StackEffect>,
    /// `#[accumulate]`: derive accumulator variants (see `vmacc`).
    pub accumulate: bool,
}

/// `accumulator acc: T for k1, k2;`: a state register that holds one
/// value between adjacent ops, in place of a slot of kinds k1, k2.
#[derive(Clone, Debug)]
pub struct AccDecl {
    pub name: Ident,
    pub ty: TypeExpr,
    pub kinds: Vec<Ident>,
    /// A value accumulator (its kinds hold a `value` type, not numbers):
    /// written through, so the slot stays current (set by `vmacc`).
    pub wt: bool,
}

/// What `Vm.accumulate` needs to know of an `#[accumulate]` op: the
/// operand (index) it writes and those it reads, each with the
/// accumulator (index into `VmDecl.accs`) of its kind, and the variant
/// per (result in the accumulator, source from it: operand index + 1, or 0).
#[derive(Clone, Debug)]
pub struct AccOp {
    pub op: String,
    pub dst: Option<(usize, usize)>,
    pub srcs: Vec<(usize, usize)>,
    pub variants: Vec<(bool, usize, String)>,
}

#[derive(Clone, Debug)]
pub struct SuperDecl {
    pub name: Ident,
    pub parts: Vec<Ident>,
    /// `#[forward]`: forward each part's written register to the next
    /// part's reads (see vmcheck::check_handler).
    pub forward: bool,
}

/// An op selector, in `accumulate` and `variant .. wraps [..]` lists
/// (resolved by `vmparse::resolve_selectors` to the ops it names).
#[derive(Clone, Debug)]
pub enum Selector {
    /// `ADD`: the op.
    Name(Ident),
    /// `I32_ADD..I32_ROTL`: the ops declared from the first to the last.
    Range(Ident, Ident),
    /// `BR_I32_*`, `*_TK`: the ops whose names match (`*` is any run of
    /// characters).
    Glob(String, crate::diag::Span),
    /// `with label`: the ops with an operand of the kind.
    With(Ident),
}

/// `rel NAME(A) = B;`: `Vm.NAME(OP_A)` is `Some(OP_B)`.
#[derive(Clone, Debug)]
pub struct RelDecl {
    pub name: Ident,
    pub from: Ident,
    pub to: Ident,
}

/// `data NAME: T = DEFAULT;` declares per-op data; `data NAME(OP) =
/// VALUE;` gives an op its value. `Vm.NAME(op) -> T`.
#[derive(Clone, Debug)]
pub struct DataKeyDecl {
    pub name: Ident,
    pub ty: TypeExpr,
    pub default: Expr,
}

#[derive(Clone, Debug)]
pub struct DataValDecl {
    pub key: Ident,
    pub op: Ident,
    pub value: Expr,
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
    /// None = all ops. Resolved from `wraps_sel` by the parser (with the
    /// supers that contain a selected op).
    pub wraps: Option<Vec<Ident>>,
    pub wraps_sel: Option<Vec<Selector>>,
    pub before: Block,
    /// `#[unfused]`: superinstructions run as their parts under this
    /// variant (each part's word is dispatched, and wrapped, on its own).
    pub unfused: bool,
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
    /// `on break { .. }`: the body of the generated breakpoint ops
    /// (`vmparse::desugar_break`), which are added to `ops`.
    pub on_break: Option<Block>,
    pub variants: Vec<VariantDecl>,
    pub sync: Option<Block>,
    /// `reload { .. }`: run after a call that may re-enter the vm, to
    /// reload the state registers from wherever the vm keeps them.
    pub reload: Option<Block>,
    /// Parameters of the generated verifier (`verify_context proto: Gc<Proto>;`).
    pub verify_ctx: Vec<Param>,
    /// `verify_context owner proto: Gc<Proto>`: the parameter that
    /// `Vm.build_verified` fills with the object it builds.
    pub verify_owner: Option<usize>,
    /// `verify NAME { .. }`: checks the verifier runs after the code (side
    /// tables such as exception handler targets), in order.
    pub checks: Vec<(Ident, Block)>,
    /// `stack_entry EXPR;`: the operand stack depth code starts with (an
    /// expression of the verify context; 0 if absent).
    pub stack_entry: Option<Expr>,
    /// `refuse CODE;`: after every `become` of a family that is part of a
    /// super, fuse supers again in CODE, the `Verified` code the current
    /// instruction is in (`vmparse::desugar_refuse`, `Vm.refuse`).
    pub refuse: Option<Expr>,
    /// `supers from "file";`: supers suggested by a `--vm-stats` report
    /// (`vmexpand::supers_from`), with the item's attributes.
    pub supers_from: Vec<(String, crate::diag::Span, bool)>,
    /// `accumulator` declarations, and the `#[accumulate]` ops' variants.
    pub accs: Vec<AccDecl>,
    /// `accumulate OP, OP;`: more ops to derive variants of (selectors,
    /// resolved by the parser into `acc_list`).
    pub acc_list: Vec<Ident>,
    pub acc_sel: Vec<Selector>,
    /// `rel` and `data` declarations: generated host lookups.
    pub rels: Vec<RelDecl>,
    pub data_keys: Vec<DataKeyDecl>,
    pub data_vals: Vec<DataValDecl>,
    pub acc_ops: Vec<AccOp>,
    /// The branches made cold by default (src/autocold.rs), for
    /// `LOIMI_COLD_AUDIT=1`.
    pub auto_cold: Vec<crate::autocold::Site>,
    pub is_pub: bool,
}

#[derive(Clone, Debug)]
pub struct ValueVariant {
    pub name: Ident,
    pub payload: Option<TypeExpr>,
    pub unboxed: bool,
    /// `block N`: the variant's block width, pinned (language.md section 2).
    pub block: Option<(u32, crate::diag::Span)>,
}

#[derive(Clone, Debug)]
pub struct ValueDecl {
    pub name: Ident,
    pub repr: Ident,
    /// `repr nanbox(int_high)`: the options in parentheses.
    pub repr_opts: Vec<Ident>,
    pub variants: Vec<ValueVariant>,
    pub is_pub: bool,
    /// `#[scan_raw_words]`: the layout must read every word below 2^48 as a
    /// variant without references (checked).
    pub scan_raw: Option<crate::diag::Span>,
}
