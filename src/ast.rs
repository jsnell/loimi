//! Surface syntax tree, as produced by the parser.

use crate::diag::Span;

pub type P<T> = Box<T>;

#[derive(Clone, Debug)]
pub struct Ident {
    pub name: String,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct TypeExpr {
    pub kind: TypeKind,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub enum TypeKind {
    /// `i32`, `Point`, `io.File`, `Result<T, E>`
    Named(Vec<Ident>, Vec<TypeExpr>),
    Ptr(P<TypeExpr>),
    Slice(P<TypeExpr>),
    Array(P<TypeExpr>, P<Expr>),
    Option(P<TypeExpr>),
    Tuple(Vec<TypeExpr>),
    /// `fn(A, B) -> R uses gc`: the effect, if any, is the strongest one.
    Fn(Vec<TypeExpr>, P<TypeExpr>, Option<Ident>),
    /// `block fn(A) -> R uses gc`: a non-escaping closure (parameters only).
    Block(Vec<TypeExpr>, P<TypeExpr>, Option<Ident>),
    Never,
    /// `[T]`: trailing variable-length array (last field of an `object`).
    Trailing(P<TypeExpr>),
    /// `verified [W] by Vm`: a trailing array holding code that `Vm`'s
    /// verifier accepted. Only `Vm.build_verified` creates the object, and
    /// the field reads as `Verified<Vm>`.
    VerifiedTrailing(P<TypeExpr>, Vec<Ident>),
    /// `cache Name`: a handle to an op's inline cache (vm function parameters).
    Cache(Ident),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    AddWrap,
    SubWrap,
    MulWrap,
    And,
    Or,
    Xor,
    Shl,
    Shr,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    LAnd,
    LOr,
}

impl BinOp {
    pub fn is_cmp(self) -> bool {
        matches!(self, BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnOp {
    Neg,
    Not,
    /// `~x`: bitwise not, integers only (lowered to `Not`).
    BitNot,
    Deref,
    AddrOf,
}

#[derive(Clone, Debug)]
pub struct Expr {
    pub kind: ExprKind,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub enum ExprKind {
    Int(u128),
    Float(f64),
    Str(Vec<u8>),
    Char(u8),
    Bool(bool),
    Ident(String),
    /// `.Variant`, resolved against the expected type.
    DotVariant(String),
    Field(P<Expr>, Ident),
    Call(P<Expr>, Vec<Expr>),
    Index(P<Expr>, P<Expr>),
    SliceRange(P<Expr>, Option<P<Expr>>, Option<P<Expr>>),
    Unary(UnOp, P<Expr>),
    Binary(BinOp, P<Expr>, P<Expr>),
    Cast(P<Expr>, TypeExpr),
    /// `Type { f: e, .. }`; the span of a trailing `..zero`, if any.
    StructLit(Vec<Ident>, Vec<(Ident, Expr)>, Option<Span>),
    Tuple(Vec<Expr>),
    Unit,
    ArrayLit(Vec<Expr>),
    ArrayRepeat(P<Expr>, P<Expr>),
    Block(Block),
    Unsafe(Block),
    If(Vec<Cond>, Block, Option<P<Expr>>),
    Match(P<Expr>, Vec<Arm>),
    While(Vec<Cond>, Block),
    Loop(Block),
    /// `for i in lo..hi { .. }`, `for i: T in lo..hi { .. }`.
    For(Ident, Option<TypeExpr>, P<Expr>, P<Expr>, Block),
    Return(Option<P<Expr>>),
    Break(Option<P<Expr>>),
    Continue,
    Assign(P<Expr>, P<Expr>),
    CompoundAssign(BinOp, P<Expr>, P<Expr>),
    /// `a ?? b`: option default.
    Coalesce(P<Expr>, P<Expr>),
    /// `expr?`
    Try(P<Expr>),
    SizeOf(TypeExpr),
    /// `|a, b: T| body` or `|a| -> R { .. }`: a closure literal (only as an
    /// argument for a `block fn` parameter).
    Closure(Vec<(Ident, Option<TypeExpr>)>, Option<TypeExpr>, P<Expr>),
    /// `f<T, U>`: explicit type arguments (only as a callee: `f<T>(..)`).
    TypeArgs(P<Expr>, Vec<TypeExpr>),
    /// VM control words and constructs (only valid in handlers).
    Vm(VmExpr),
}

#[derive(Clone, Debug)]
pub enum VmExpr {
    Next,
    Jump(P<Expr>),
    Redo,
    Exit(P<Expr>),
    Raise(P<Expr>),
    Fail(P<Expr>),
    Become(Ident),
    /// `use_variant NAME` / `use_variant default`: switch dispatch tables.
    UseVariant(Ident),
    Cold(Block),
}

#[derive(Clone, Debug)]
pub enum Cond {
    Expr(Expr),
    Let(Pat, Expr),
}

#[derive(Clone, Debug)]
pub struct Arm {
    pub pat: Pat,
    pub guard: Option<Expr>,
    pub body: Expr,
}

#[derive(Clone, Debug)]
pub struct Pat {
    pub kind: PatKind,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub enum PatKind {
    Wild,
    /// Binding or payload-less variant; decided by the checker.
    Ident(String, bool),
    Int(i128),
    Char(u8),
    Bool(bool),
    /// A string literal, for `str` scrutinees.
    Str(Vec<u8>),
    Range(i128, i128),
    /// `Variant(p, ...)`, `Enum.Variant(..)`, `.Variant(..)`. The path is
    /// empty for the `.Variant` form.
    Variant(Vec<Ident>, Option<Vec<Pat>>),
    Tuple(Vec<Pat>),
    Or(Vec<Pat>),
}

#[derive(Clone, Debug)]
pub struct Block {
    pub stmts: Vec<Stmt>,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub enum Stmt {
    Let { pat: Pat, mutable: bool, ty: Option<TypeExpr>, init: Option<Expr>, els: Option<Block>, span: Span },
    Expr { expr: Expr, semi: bool },
    Defer(Expr),
}

#[derive(Clone, Debug, Default)]
pub struct Attrs {
    pub list: Vec<(String, Vec<String>, Span)>,
}

impl Attrs {
    pub fn get(&self, name: &str) -> Option<&Vec<String>> {
        self.list.iter().find(|(n, _, _)| n == name).map(|(_, a, _)| a)
    }
    pub fn has(&self, name: &str) -> bool {
        self.get(name).is_some()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Linkage {
    Internal,
    Extern,
    Export,
}

#[derive(Clone, Debug)]
pub struct Param {
    pub name: Ident,
    pub ty: TypeExpr,
}

#[derive(Clone, Debug)]
pub struct FnDecl {
    pub name: Ident,
    /// Type parameters: `fn f<T>(..)`, of the receiver: `fn Vec<T>.push(..)`,
    /// and of a method: `fn Cursor.op<K>(..)`.
    pub generics: Vec<Ident>,
    /// Trait bounds per entry of `generics` (`T: hash.Hash + Show`).
    pub bounds: Vec<Bounds>,
    /// How many of `generics` belong to the receiver type (`fn Vec<T>.map<U>`
    /// has generics `[T, U]` and `recv_generics == 1`).
    pub recv_generics: usize,
    /// `fn Type.method(...)`
    pub recv: Option<Ident>,
    pub params: Vec<Param>,
    pub variadic: bool,
    pub ret: Option<TypeExpr>,
    pub body: Option<Block>,
    pub linkage: Linkage,
    pub attrs: Attrs,
    pub is_pub: bool,
}

/// A trait named in a bound or an impl: `Hash`, `hash.Hash`, `Hooks<K, E>`.
#[derive(Clone, Debug)]
pub struct TraitRef {
    pub path: Vec<Ident>,
    pub args: Vec<TypeExpr>,
    pub span: Span,
}

/// The traits a type parameter must implement.
pub type Bounds = Vec<TraitRef>;

/// `trait Name<A> { fn m(a: Self) -> R; fn d(a: Self) { .. } .. }`: method
/// signatures over `Self`, with optional default bodies and their own type
/// parameters.
#[derive(Clone, Debug)]
pub struct TraitDecl {
    pub name: Ident,
    pub generics: Vec<Ident>,
    pub methods: Vec<FnDecl>,
    pub is_pub: bool,
}

/// `impl Trait for Type;`, `impl<T: Hash> Hash for Vec<T>;`
#[derive(Clone, Debug)]
pub struct ImplDecl {
    pub generics: Vec<Ident>,
    pub bounds: Vec<Bounds>,
    pub trait_ref: TraitRef,
    pub ty: TypeExpr,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct StructDecl {
    pub name: Ident,
    pub generics: Vec<Ident>,
    pub bounds: Vec<Bounds>,
    pub is_object: bool,
    pub fields: Vec<(Ident, TypeExpr)>,
    pub is_pub: bool,
}

#[derive(Clone, Debug)]
pub struct Variant {
    pub name: Ident,
    pub fields: Vec<TypeExpr>,
    pub discr: Option<i128>,
}

#[derive(Clone, Debug)]
pub struct EnumDecl {
    pub name: Ident,
    pub generics: Vec<Ident>,
    pub bounds: Vec<Bounds>,
    pub repr: Option<TypeExpr>,
    pub variants: Vec<Variant>,
    pub is_pub: bool,
}

#[derive(Clone, Debug)]
pub struct ConstDecl {
    pub name: Ident,
    pub ty: TypeExpr,
    pub value: Expr,
    pub is_pub: bool,
}

/// Module-level mutable variable: `var NAME: T = const_init;`.
#[derive(Clone, Debug)]
pub struct GlobalDecl {
    pub name: Ident,
    pub ty: TypeExpr,
    pub value: Expr,
    pub is_pub: bool,
    /// `#[thread_local]`: one copy per thread (initial-exec TLS).
    pub thread_local: bool,
}

/// `type Name<A, B> = T;`: a transparent alias.
#[derive(Clone, Debug)]
pub struct TypeAliasDecl {
    pub name: Ident,
    pub generics: Vec<Ident>,
    pub ty: TypeExpr,
    pub is_pub: bool,
}

#[derive(Clone, Debug)]
pub struct Import {
    pub path: Vec<Ident>,
    /// `import a.b.{X, Y}`; None means bind the last segment as a module.
    pub names: Option<Vec<Ident>>,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub enum Item {
    Fn(FnDecl),
    Struct(StructDecl),
    Enum(EnumDecl),
    Const(ConstDecl),
    Global(GlobalDecl),
    Import(Import),
    Trait(TraitDecl),
    Impl(ImplDecl),
    TypeAlias(TypeAliasDecl),
    Vm(Box<crate::vmast::VmDecl>),
    Value(crate::vmast::ValueDecl),
}

#[allow(dead_code)]
pub struct Module {
    pub path: Vec<String>,
    pub file: u32,
    pub items: Vec<Item>,
}
