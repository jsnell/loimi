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
    Fn(Vec<TypeExpr>, P<TypeExpr>),
    Never,
    /// `[T]`: trailing variable-length array (last field of an `object`).
    Trailing(P<TypeExpr>),
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
    StructLit(Vec<Ident>, Vec<(Ident, Expr)>),
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
    For(Ident, P<Expr>, P<Expr>, Block),
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
    /// Type parameters: `fn f<T>(..)`, or of the receiver: `fn Vec<T>.push(..)`.
    pub generics: Vec<Ident>,
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

#[derive(Clone, Debug)]
pub struct StructDecl {
    pub name: Ident,
    pub generics: Vec<Ident>,
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
    Import(Import),
    Vm(Box<crate::vmast::VmDecl>),
    Value(crate::vmast::ValueDecl),
}

#[allow(dead_code)]
pub struct Module {
    pub path: Vec<String>,
    pub file: u32,
    pub items: Vec<Item>,
}
