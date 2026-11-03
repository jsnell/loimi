//! Types and the typed, name-resolved IR produced by the checker.

use crate::ast::{BinOp, Linkage, UnOp};
use crate::diag::Span;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct IntTy {
    pub signed: bool,
    pub bits: u8,
}

impl IntTy {
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
    /// Parameters, result, effects (EFF_* mask, closed downward).
    Fn(Vec<Ty>, Box<Ty>, u8),
    Struct(u32),
    Enum(u32),
    Value(u32),
    /// Type parameter being inferred (only during generic-call checking).
    Param(u32),
    /// Traced reference into the GC heap.
    Gc(Box<Ty>),
    /// A bounded type parameter while a generic body is checked against
    /// its bounds (index into `Traits::opaques`). Never reaches codegen.
    Opaque(u32),
    /// `block fn(A) -> R uses E`: a non-escaping closure, `{ code, env }`.
    Block(Vec<Ty>, Box<Ty>, u8),
    /// `closure fn(A) -> R uses E`: an escaping closure, `{ code, env }`,
    /// with the environment in host memory (not traced). Under `Gc<..>`:
    /// a closure object `{ code, captures.. }` in the GC heap.
    Closure(Vec<Ty>, Box<Ty>, u8),
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
        matches!(self, Ty::Ptr(_) | Ty::Fn(..) | Ty::Gc(_))
    }
}

pub type LocalId = u32;
pub type FnId = u32;

#[derive(Clone, Debug)]
pub struct LocalDef {
    pub name: String,
    pub ty: Ty,
    pub mutable: bool,
    /// An operand field the vm checker decodes from a code word (a
    /// handler's `x.raw`, a `then` operand's), and how: a stencil makes
    /// it a hole, and cold code decodes it again.
    pub field: Option<FieldDecode>,
}

/// Where an operand field lies: in the code word at `pc + off` (`off`
/// within a superinstruction's part), `bits` wide at bit `shift`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FieldDecode {
    pub off: u32,
    pub shift: u32,
    pub bits: u32,
    pub signed: bool,
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
    Ceil,
    Trunc,
    /// Round to nearest, ties to even (`llvm.roundeven`).
    Nearest,
    Fabs,
    IsNan,
    Copysign,
    /// C `fmin`/`fmax` (IEEE 754-2008 minNum: a NaN operand is ignored).
    MinNum,
    MaxNum,
    /// IEEE 754-2019 `minimum`/`maximum` (wasm `min`/`max`): NaN
    /// propagates, -0 < +0.
    Minimum,
    Maximum,
    /// `p.read_unaligned()`, `p.write_unaligned(v)`: `load`/`store` with
    /// `align 1`.
    LoadUnaligned,
    StoreUnaligned,
    /// `gc_mark(heap, x)`: mark every GC reference held by `x`.
    GcMark,
    /// `__panic("msg")` (generated code only): panic with a message.
    Panic,
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
    /// A module-level `var` (a place).
    Global(u32),
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
    /// `new(heap, Obj { .. })` / `new(heap, Obj { .. }, n)`: allocate a GC object.
    New { heap: B, init: B, len: Option<B> },
    /// The trailing array of an object, as a slice.
    Trailing(B),
    /// A closure literal: the closure's function and the captured locals,
    /// whose addresses make up its environment.
    Closure(FnId, Vec<LocalId>),
    /// `new(a, |..| ..)`: an escaping closure. The environment holds
    /// copies of the captured locals. The third part is the allocation:
    /// for a `closure fn`, the `a.alloc_bytes(size, align)` call (or, with
    /// no captures, `a` itself, evaluated and ignored); for a
    /// `Gc<closure fn>`, the heap.
    NewClosure(FnId, Vec<LocalId>, B),
}

#[derive(Clone, Debug)]
pub enum TVm {
    /// Dispatch to the instruction at the given pc (`next` and `redo`
    /// are lowered to this by the checker).
    Jump(B),
    Exit(B),
    Raise(B),
    /// Rewrite the opcode of the word `back` words before pc (0: the
    /// current instruction; more for the head of a superinstruction).
    Become(u32, u32),
    Cold(B),
    /// The code word at `pc + offset` (a place).
    Word(u32),
    /// A cache field through a cache handle: the place `bytes` bytes past
    /// the pointer, accessed with alignment `align` (code words only
    /// have the word's alignment).
    WordAt(B, u32, u32),
    /// In a superinstruction part: advance pc by `size` and continue with part `next`.
    Fallthrough(u32, u32),
    /// Part `i` of a superinstruction handler.
    Part(u32, B),
    /// The pending error value (in `on raise`).
    RaiseSlot,
    /// Address of dispatch table `i` (0 = default, 1.. = variants).
    TableAddr(u32),
    /// Continue with the real handler of op `i` (from an instrumentation wrapper).
    Chain(u32),
    /// Tail-call mode: continue in the shared copy of a `-> never` vm
    /// function (its FnId), passing the arguments and the size `next`
    /// skips through thread-local slots. The last part is where its
    /// `next` continues: None to dispatch the following word (an op,
    /// or a super's last part), `FnRef` of a super's resume function
    /// (from a part before the last), or a shared copy's own
    /// continuation `Local`. Only shared copies in `VmDef::cont_fns`
    /// take it (`vmcheck::shared_conts`); the others ignore it. The slot
    /// is cleared when read, so None stores nothing.
    Shared(FnId, Vec<TExpr>, B, Option<B>),
    /// In a shared copy: argument `i` (the last one is the `next` size;
    /// `u32::MAX` is the continuation).
    SharedArg(FnId, u32),
    /// `next` in the shared copy of `-> never` vm function `i`
    /// (`VmRole::Shared(i)`): dispatch the target, or, if the copy takes
    /// a continuation and the one in local `cont` is not null, continue
    /// there (in the super it was called from) with pc at the target.
    SharedNext(B, u32, LocalId),
    /// A call that may collect or observe vm state: run the vm's `sync`
    /// block, keep traced locals visible to the GC, then call; for a call
    /// that may re-enter the vm, run its `reload` block after the call
    /// (the third part; `()` otherwise).
    Synced(B, B, B),
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
    /// Equal bytes (the scrutinee is a `str`).
    Str(Vec<u8>),
    Range(i128, i128),
    Variant(u32, Vec<TPat>),
    Tuple(Vec<TPat>),
    Or(Vec<TPat>),
}

#[derive(Clone, Debug)]
pub struct StructDef {
    pub name: String,
    pub fields: Vec<(String, Ty)>,
    /// An instance over inference variables; never emitted.
    pub pattern: bool,
    /// GC-managed `object`: its type id, and the element type of a trailing array.
    pub object: Option<u32>,
    pub trailing: Option<(String, Ty)>,
    /// `code: verified [W] by Vm`: the trailing array holds verified code;
    /// the struct id of `Vm`'s proof type `Vm__Verified`.
    pub verified: Option<u32>,
    /// `#[trace(f)]`: the function that traces the trailing array and the
    /// memory the object owns, after the generated tracing of its fields.
    pub trace: Option<FnId>,
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
    /// Effects: EFF_SYNC | EFF_GC | EFF_REENTER.
    pub effects: u8,
    /// Plain vm functions: effects of the calls they make, directly or
    /// through other vm functions, beyond `effects`. Those calls are
    /// synced in place, as in an op; callers root their traced locals at
    /// a call of this function as if it had these effects.
    pub sync_effects: u8,
    /// Emitted only if some emitted function refers to it (generated vm
    /// helpers few programs use: `crate::vmexpand::LAZY_ATTR`).
    pub lazy: bool,
    /// Closure bodies: the captured locals of the enclosing function, in
    /// environment order. The function takes the environment pointer as
    /// a hidden first parameter, and reaches these locals through it.
    pub env: Option<Vec<LocalId>>,
    /// Per captured local: captured by value. The environment entry holds
    /// the value itself instead of the local's address (immutable,
    /// untraced locals of at most 8 bytes), which saves a load per use.
    pub env_val: Vec<bool>,
    /// Escaping closures: the environment is a struct (this struct id)
    /// holding the captured values from field `.1` on (a GC closure's
    /// object has the code pointer in field 0).
    pub env_struct: Option<(u32, u32)>,
    /// GC closures: a hidden local holding the environment object, so
    /// that the body's `gc` calls root it.
    pub env_root: Option<LocalId>,
    /// `#[catch_panic(v)]` on an `export "C"` function: the LLVM constant
    /// it returns when a panic unwinds to it (`rtgen::emit_catch_wrappers`).
    pub catch_panic: Option<String>,
    /// `#[track_caller]`: callers store their location for
    /// `test.caller()` (`codegen::store_caller`).
    pub track_caller: bool,
}

impl FnDef {
    /// The type of this function as a value.
    pub fn fn_ty(&self) -> Ty {
        Ty::Fn(self.param_tys.clone(), Box::new(self.ret.clone()), self.effects)
    }
}

pub const EFF_SYNC: u8 = 1;
pub const EFF_GC: u8 = 2;
pub const EFF_REENTER: u8 = 4;

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
    /// Tag position. nanbox: the word is `tag << shift | payload` (the
    /// payload has `shift` bits); lowtag: `payload << shift | tag` (the tag
    /// has `shift` bits). Unused for boxed and the unboxed variant.
    pub shift: u32,
}

#[derive(Clone, Debug)]
pub struct ValueDef {
    pub name: String,
    pub repr: ValueRepr,
    pub variants: Vec<ValueVariantDef>,
    /// nanbox: the lowest tagged encoding; words below it are the unboxed
    /// f64 variant.
    pub num_limit: u64,
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
    /// Shared copy of the `-> never` vm function with this index (tail-call mode).
    Shared(u32),
    /// Tail-call mode: the rest of superinstruction `op` from part
    /// `part` on, with pc at that part's word. `next` in an earlier part
    /// continues here when it leaves the super's handler (from an
    /// outlined cold block or a shared copy), as it would inside it.
    Resume(u32, u32),
    /// Instrumentation wrapper: (variant index, op index).
    Wrapper(u32, u32),
}

#[derive(Clone, Debug)]
pub struct VmOp {
    pub name: String,
    pub opcode: u32,
    pub fid: FnId,
    /// Words occupied (instruction + inline cache; sum of parts for supers).
    pub size: u32,
    pub parts: Vec<u32>,
    /// Declared `then OP`: the word after it is read as part of it.
    pub then: bool,
    /// Its fields of `const` operand kinds: (shift, bits).
    pub consts: Vec<(u32, u32)>,
    /// A superinstruction's resume functions (`VmRole::Resume`) per part
    /// (tail-call mode, created where needed).
    pub resume: Vec<Option<FnId>>,
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
    /// Instrumentation variants: name and (op index, wrapper fn) pairs.
    pub variants: Vec<(String, Vec<(u32, FnId)>)>,
    /// State index of the hidden dispatch-table register (vms with variants).
    pub tbl_idx: Option<usize>,
    /// The state register `const` operand kinds index (the constant pool).
    pub const_base: Option<usize>,
    /// Shared copies of `-> never` vm functions (by index) that take a
    /// continuation for `next` (`TVm::Shared`): those reachable from a
    /// super's part before the last.
    pub cont_fns: Vec<bool>,
}

impl VmDef {
    /// The index of the op with opcode `opcode`, if one has it.
    pub fn op_index(&self, opcode: usize) -> Option<usize> {
        self.ops.iter().position(|o| o.opcode as usize == opcode)
    }
}

/// A module-level `var`. `init` is a constant expression.
#[derive(Clone, Debug)]
pub struct GlobalDef {
    pub symbol: String,
    pub ty: Ty,
    pub init: TExpr,
    pub thread_local: bool,
}

#[derive(Default)]
pub struct Program {
    pub globals: Vec<GlobalDef>,
    pub structs: Vec<StructDef>,
    pub enums: Vec<EnumDef>,
    pub values: Vec<ValueDef>,
    pub fns: Vec<FnDef>,
    pub vms: Vec<VmDef>,
    /// `#[c_name("..")]` of structs and enums, by Loimi name.
    pub c_names: std::collections::BTreeMap<String, String>,
    /// The name spans of structs and enums, by Loimi name (for the doc
    /// comments in `--header`).
    pub type_spans: std::collections::BTreeMap<String, crate::diag::Span>,
    /// `export "C" const`s, in source order: name, value, name span.
    pub c_consts: Vec<(String, TExpr, crate::diag::Span)>,
    /// `#[c_const(p, ..)]` on an export, by symbol: parameters that are
    /// `const` pointers in C.
    pub c_const_params: std::collections::BTreeMap<String, Vec<(String, crate::diag::Span)>>,
}
