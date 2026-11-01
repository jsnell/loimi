//! Types and the typed, name-resolved IR produced by the checker.

use crate::ast::{BinOp, Linkage, UnOp};
use crate::diag::Span;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct IntTy {
    pub signed: bool,
    pub bits: u8,
}

impl IntTy {
    pub const I8: IntTy = IntTy { signed: true, bits: 8 };
    pub const I32: IntTy = IntTy { signed: true, bits: 32 };
    pub const I64: IntTy = IntTy { signed: true, bits: 64 };
    pub const U8: IntTy = IntTy { signed: false, bits: 8 };
    pub const U32: IntTy = IntTy { signed: false, bits: 32 };
    pub const U64: IntTy = IntTy { signed: false, bits: 64 };

    pub fn min(self) -> i128 {
        if self.signed { -(1i128 << (self.bits - 1)) } else { 0 }
    }
    pub fn max(self) -> i128 {
        if self.signed { (1i128 << (self.bits - 1)) - 1 } else { (1i128 << self.bits) - 1 }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Ty {
    Int(IntTy),
    F32,
    F64,
    Bool,
    Unit,
    Never,
    Ptr(Box<Ty>),
    Slice(Box<Ty>),
    Array(Box<Ty>, u64),
    Option(Box<Ty>),
    Result(Box<Ty>, Box<Ty>),
    Tuple(Vec<Ty>),
    Fn(Vec<Ty>, Box<Ty>),
    Struct(u32),
    Enum(u32),
    Value(u32),
}

impl Ty {
    pub fn i64() -> Ty {
        Ty::Int(IntTy::I64)
    }
    pub fn u8() -> Ty {
        Ty::Int(IntTy::U8)
    }
    pub fn str() -> Ty {
        Ty::Slice(Box::new(Ty::u8()))
    }
    pub fn is_int(&self) -> bool {
        matches!(self, Ty::Int(_))
    }
    pub fn is_float(&self) -> bool {
        matches!(self, Ty::F32 | Ty::F64)
    }
    pub fn is_numeric(&self) -> bool {
        self.is_int() || self.is_float()
    }
    pub fn is_unitlike(&self) -> bool {
        matches!(self, Ty::Unit | Ty::Never)
    }
    /// Option of a pointer-like type uses null as None.
    pub fn is_nullable_ptr(&self) -> bool {
        matches!(self, Ty::Ptr(_) | Ty::Fn(..))
    }
}

pub type LocalId = u32;
pub type FnId = u32;

#[derive(Clone, Debug)]
pub struct LocalDef {
    pub name: String,
    pub ty: Ty,
    pub mutable: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Intr {
    Likely,
    Unlikely,
    Assume,
    Trap,
    AddChecked,
    SubChecked,
    MulChecked,
    IsSome,
    IsNone,
    Unwrap,
    UnwrapOr,
    PtrOffset,
    SliceFromRaw,
    Clz,
    Ctz,
    Popcount,
    FloatBits,
    BitsToFloat,
    Sqrt,
    Floor,
    /// Value types: construct variant (variant index in arg 0 as Int).
    ValueTag,
}

#[derive(Clone, Debug)]
pub struct TExpr {
    pub ty: Ty,
    pub kind: TK,
    pub span: Span,
}

pub type B = Box<TExpr>;

#[derive(Clone, Debug)]
pub enum TK {
    Int(i128),
    Float(f64),
    Bool(bool),
    Str(Vec<u8>),
    Unit,
    Zeroed,
    Local(LocalId),
    FnRef(FnId),
    Deref(B),
    Field(B, u32),
    Index(B, B),
    SliceRange(B, Option<B>, Option<B>),
    AddrOf(B),
    Unary(UnOp, B),
    Binary(BinOp, B, B),
    Cast(B),
    Call(B, Vec<TExpr>),
    Intrinsic(Intr, Vec<TExpr>),
    Struct(Vec<TExpr>),
    Tuple(Vec<TExpr>),
    Array(Vec<TExpr>),
    ArrayRepeat(B, u64),
    /// Variant index; the type says which enum / option / result / value.
    Variant(u32, Vec<TExpr>),
    Block(Vec<TStmt>, Option<B>),
    If(Vec<TCond>, B, Option<B>),
    Match(B, Vec<TArm>),
    Loop { body: B, step: Option<B>, id: u32 },
    Break(u32, Option<B>),
    Continue(u32),
    Return(Option<B>),
    Assign(B, B),
    Coalesce(B, B),
    /// `e?` on Option or Result inside a function returning Option/Result.
    Try(B),
    Unreachable,
    Vm(TVm),
}

#[derive(Clone, Debug)]
pub enum TVm {
    /// Dispatch to the instruction at the given pc (`next` and `redo`
    /// are lowered to this by the checker).
    Jump(B),
    Exit(B),
    Raise(B),
    /// Rewrite the current instruction's opcode.
    Become(u32),
    Cold(B),
    /// The code word at `pc + offset` (a place).
    Word(u32),
    /// In a superinstruction part: advance pc by `size` and continue with part `next`.
    Fallthrough(u32, u32),
    /// Part `i` of a superinstruction handler.
    Part(u32, B),
    /// The pending error value (in `on raise`).
    RaiseSlot,
}

#[derive(Clone, Debug)]
pub enum TStmt {
    Let(LocalId, Option<TExpr>),
    /// Refutable or destructuring let; `els` must diverge.
    LetPat(TPat, TExpr, Option<TExpr>),
    Expr(TExpr),
    Defer(TExpr),
}

#[derive(Clone, Debug)]
pub enum TCond {
    Expr(TExpr),
    Let(TPat, TExpr),
}

#[derive(Clone, Debug)]
pub struct TArm {
    pub pat: TPat,
    pub guard: Option<TExpr>,
    pub body: TExpr,
}

#[derive(Clone, Debug)]
pub struct TPat {
    pub ty: Ty,
    pub kind: TPK,
}

#[derive(Clone, Debug)]
pub enum TPK {
    Wild,
    Bind(LocalId),
    Int(i128),
    Bool(bool),
    Range(i128, i128),
    Variant(u32, Vec<TPat>),
    Tuple(Vec<TPat>),
    Or(Vec<TPat>),
}

#[derive(Clone, Debug)]
pub struct StructDef {
    pub name: String,
    pub fields: Vec<(String, Ty)>,
}

#[derive(Clone, Debug)]
pub struct VariantDef {
    pub name: String,
    pub fields: Vec<Ty>,
    pub discr: i128,
}

#[derive(Clone, Debug)]
pub struct EnumDef {
    pub name: String,
    pub tag: IntTy,
    pub variants: Vec<VariantDef>,
}

impl EnumDef {
    pub fn has_payload(&self) -> bool {
        self.variants.iter().any(|v| !v.fields.is_empty())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CallConv {
    C,
    /// Interpreter handlers in tail-call mode.
    PreserveNone,
    /// Outlined cold blocks.
    PreserveMost,
}

#[derive(Clone, Debug)]
pub struct FnDef {
    pub name: String,
    pub symbol: String,
    pub params: Vec<LocalId>,
    pub param_tys: Vec<Ty>,
    pub ret: Ty,
    pub locals: Vec<LocalDef>,
    pub body: Option<TExpr>,
    pub linkage: Linkage,
    pub variadic: bool,
    pub inline: Option<bool>,
    pub cold: bool,
    pub cc: CallConv,
    pub span: Span,
    /// Set for vm handlers / entries: index into Program::vms.
    pub vm: Option<u32>,
    pub vm_role: VmRole,
    /// Locals holding the vm state (handlers, entries), or pointers to it (vm fns).
    pub vm_state: Vec<LocalId>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ValueRepr {
    NanBox,
    LowTag,
    Boxed,
}

#[derive(Clone, Debug)]
pub struct ValueVariantDef {
    pub name: String,
    pub payload: Option<Ty>,
    /// The `unboxed` f64 variant of a nanbox.
    pub unboxed: bool,
    /// Encoding: tag value (meaning depends on repr).
    pub tag: u64,
}

#[derive(Clone, Debug)]
pub struct ValueDef {
    pub name: String,
    pub repr: ValueRepr,
    pub variants: Vec<ValueVariantDef>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dispatch {
    TailCall,
    Goto,
    Switch,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VmRole {
    None,
    /// Handler for the op with this index.
    Handler(u32),
    Entry,
    Raise,
    VmFn,
}

#[derive(Clone, Debug)]
pub struct VmOp {
    pub name: String,
    pub opcode: u32,
    pub fid: FnId,
    /// Words occupied (instruction + inline cache; sum of parts for supers).
    pub size: u32,
    pub parts: Vec<u32>,
}

#[derive(Clone, Debug)]
pub struct VmDef {
    pub name: String,
    pub symbol: String,
    pub dispatch: Dispatch,
    pub word: IntTy,
    pub op_shift: u32,
    pub op_bits: u32,
    pub state_tys: Vec<Ty>,
    pub state_names: Vec<String>,
    pub pc_idx: usize,
    pub ret: Ty,
    pub ops: Vec<VmOp>,
    pub entries: Vec<FnId>,
    pub on_raise: Option<FnId>,
    pub raise_ty: Option<Ty>,
}

#[derive(Default)]
pub struct Program {
    pub structs: Vec<StructDef>,
    pub enums: Vec<EnumDef>,
    pub values: Vec<ValueDef>,
    pub fns: Vec<FnDef>,
    pub vms: Vec<VmDef>,
}
