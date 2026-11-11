use std::path::PathBuf;
use std::rc::Rc;

use crate::token::{CommentKind, Keyword, LitKind, Token};

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq)]
pub struct File {
    pub path: PathBuf,
    pub line_info: Vec<usize>,
    pub container_docs: Vec<Rc<Comment>>,
    pub members: Vec<ContainerMember>,
    pub comments: Vec<Rc<Comment>>,
}

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Comment {
    pub pos: usize,
    pub kind: CommentKind,
    pub text: String,
}

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq)]
pub enum ContainerMember {
    Field(Box<ContainerField>),
    Fn(Box<FnDecl>),
    Var(Box<VarDecl>),
    Test(Box<TestDecl>),
    Comptime(Box<ComptimeDecl>),
}

impl ContainerMember {
    pub fn pos(&self) -> usize {
        match self {
            Self::Field(node) => node.pos,
            Self::Fn(node) => node.pos,
            Self::Var(node) => node.pos,
            Self::Test(node) => node.pos,
            Self::Comptime(node) => node.pos,
        }
    }
}

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq)]
pub struct ContainerField {
    pub pos: usize,
    pub docs: Vec<Rc<Comment>>,
    pub comptime: bool,
    pub name: Option<Ident>,
    pub ty: Option<Expression>,
    pub align: Option<Expression>,
    pub default: Option<Expression>,
    pub tokens: Vec<Token>,
}

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq)]
pub struct FnDecl {
    pub pos: usize,
    pub docs: Vec<Rc<Comment>>,
    pub visibility: Option<Keyword>,
    pub modifier: Option<FnModifier>,
    pub name: Option<Ident>,
    pub params: Vec<ParamDecl>,
    pub align: Option<Expression>,
    pub addrspace: Option<Expression>,
    pub linksection: Option<Expression>,
    pub callconv: Option<Expression>,
    pub bang: bool,
    pub ret: Option<Box<Expression>>,
    pub body: Option<Block>,
    pub tokens: Vec<Token>,
}

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq)]
pub struct VarDecl {
    pub pos: usize,
    pub docs: Vec<Rc<Comment>>,
    pub visibility: Option<Keyword>,
    pub extern_export: Option<VarModifier>,
    pub threadlocal: bool,
    pub keyword: Keyword,
    pub lhs: Vec<VarProtoOrExpr>,
    pub align: Option<Expression>,
    pub addrspace: Option<Expression>,
    pub linksection: Option<Expression>,
    pub init: Option<Expression>,
    pub tokens: Vec<Token>,
}

impl VarDecl {
    /// Type annotation of the first declared name, if any.
    ///
    /// It lives in the first [`VarProtoOrExpr::VarProto`] of `lhs`; it is not
    /// stored separately, because a second copy would duplicate everything
    /// nested inside the type.
    pub fn ty(&self) -> Option<&Expression> {
        match self.lhs.first() {
            Some(VarProtoOrExpr::VarProto { ty, .. }) => ty.as_ref(),
            _ => None,
        }
    }
}

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq)]
pub struct TestDecl {
    pub pos: usize,
    pub docs: Vec<Rc<Comment>>,
    pub name: Option<Expression>,
    pub body: Block,
    pub tokens: Vec<Token>,
}

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq)]
pub struct ComptimeDecl {
    pub pos: usize,
    pub docs: Vec<Rc<Comment>>,
    pub block: Block,
    pub tokens: Vec<Token>,
}

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ident {
    pub pos: usize,
    pub name: String,
}

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq)]
pub enum VarProtoOrExpr {
    VarProto {
        pos: usize,
        keyword: Keyword,
        name: Ident,
        ty: Option<Expression>,
    },
    Expr(Expression),
}

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq)]
pub enum FnModifier {
    Export,
    Extern(Option<String>),
    Inline,
    NoInline,
}

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq)]
pub enum VarModifier {
    Export,
    Extern(Option<String>),
}

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq)]
pub enum Expression {
    Ident(Ident),
    BasicLit(BasicLit),
    MultilineStr(MultilineStr),
    BuiltinCall(BuiltinCall),
    EnumLiteral(EnumLiteral),
    AnonInit(AnonInit),
    ErrorValue(ErrorValue),
    ErrorSetDecl(ErrorSetDecl),
    Grouped(Box<Expression>),
    Unreachable(usize),
    AnyframeType(AnyframeType),
    Unary(UnaryOp),
    Binary(BinaryOp),
    Catch(CatchExpr),
    Assign(AssignExpr),
    If(IfExpr),
    While(WhileExpr),
    For(ForExpr),
    Switch(SwitchExpr),
    Block(Block),
    Break(BreakExpr),
    Continue(ContinueExpr),
    Return(ReturnExpr),
    Comptime(Box<Expression>),
    Nosuspend(Box<Expression>),
    Resume(Box<Expression>),
    Asm(AsmExpr),
    Call(Call),
    Index(Index),
    Slice(Slice),
    FieldAccess(FieldAccess),
    Deref(Deref),
    Unwrap(Unwrap),
    InitList(CurlyInit),
    Optional(OptionalType),
    ErrorUnion(ErrorUnionType),
    Pointer(PointerType),
    SliceType(SliceType),
    ArrayType(ArrayType),
    Container(ContainerDecl),
    FnProto(Box<FnDecl>),
}

impl Expression {
    pub fn pos(&self) -> usize {
        match self {
            Self::Ident(node) => node.pos,
            Self::BasicLit(node) => node.pos,
            Self::MultilineStr(node) => node.pos,
            Self::BuiltinCall(node) => node.pos,
            Self::EnumLiteral(node) => node.pos,
            Self::AnonInit(node) => node.pos,
            Self::ErrorValue(node) => node.pos,
            Self::ErrorSetDecl(node) => node.pos,
            Self::Grouped(expr) => expr.pos(),
            Self::Unreachable(pos) => *pos,
            Self::AnyframeType(node) => node.pos,
            Self::Unary(node) => node.pos,
            Self::Binary(node) => node.pos,
            Self::Catch(node) => node.pos,
            Self::Assign(node) => node.pos,
            Self::If(node) => node.pos,
            Self::While(node) => node.pos,
            Self::For(node) => node.pos,
            Self::Switch(node) => node.pos,
            Self::Block(node) => node.pos,
            Self::Break(node) => node.pos,
            Self::Continue(node) => node.pos,
            Self::Return(node) => node.pos,
            Self::Comptime(expr) => expr.pos(),
            Self::Nosuspend(expr) => expr.pos(),
            Self::Resume(expr) => expr.pos(),
            Self::Asm(node) => node.pos,
            Self::Call(node) => node.pos,
            Self::Index(node) => node.pos,
            Self::Slice(node) => node.pos,
            Self::FieldAccess(node) => node.pos,
            Self::Deref(node) => node.pos,
            Self::Unwrap(node) => node.pos,
            Self::InitList(node) => node.pos,
            Self::Optional(node) => node.pos,
            Self::ErrorUnion(node) => node.pos,
            Self::Pointer(node) => node.pos,
            Self::SliceType(node) => node.pos,
            Self::ArrayType(node) => node.pos,
            Self::Container(node) => node.pos,
            Self::FnProto(node) => node.pos,
        }
    }
}

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq)]
pub struct BasicLit {
    pub pos: usize,
    pub kind: LitKind,
    pub text: String,
}

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq)]
pub struct MultilineStr {
    pub pos: usize,
    pub text: String,
}

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq)]
pub struct BuiltinCall {
    pub pos: usize,
    pub name: String,
    pub args: Vec<Expression>,
}

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq)]
pub struct EnumLiteral {
    pub pos: usize,
    pub name: Ident,
}

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq)]
pub struct AnonInit {
    pub pos: usize,
    pub entries: Vec<InitEntry>,
}

impl AnonInit {
    /// The value of each entry, in source order.
    ///
    /// Derived from `entries` rather than stored, because a second copy would
    /// duplicate everything nested inside the initializer.
    pub fn fields(&self) -> impl Iterator<Item = &Expression> {
        self.entries.iter().map(InitEntry::value)
    }
}

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq)]
pub struct ErrorValue {
    pub pos: usize,
    pub name: Ident,
}

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq)]
pub struct ErrorSetDecl {
    pub pos: usize,
    pub names: Vec<Ident>,
}

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq)]
pub enum InitEntry {
    Field { name: Ident, value: Expression },
    Expr(Expression),
}

impl InitEntry {
    /// The entry's value: the expression after `.name =`, or the bare one.
    pub fn value(&self) -> &Expression {
        match self {
            Self::Field { value, .. } | Self::Expr(value) => value,
        }
    }
}

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq)]
pub struct AnyframeType {
    pub pos: usize,
    pub result: Option<Box<Expression>>,
}

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq)]
pub struct UnaryOp {
    pub pos: usize,
    pub op: Token,
    pub expr: Box<Expression>,
}

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq)]
pub struct BinaryOp {
    pub pos: usize,
    pub op: Token,
    pub lhs: Box<Expression>,
    pub rhs: Box<Expression>,
}

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq)]
pub struct CatchExpr {
    pub pos: usize,
    pub lhs: Box<Expression>,
    pub payload: Option<Payload>,
    pub rhs: Box<Expression>,
}

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq)]
pub struct AssignExpr {
    pub pos: usize,
    pub lhs: Vec<Expression>,
    pub op: Token,
    pub rhs: Box<Expression>,
    pub tokens: Vec<Token>,
}

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq)]
pub struct IfExpr {
    pub pos: usize,
    pub condition: Option<Box<Expression>>,
    pub payload: Option<Payload>,
    pub then_branch: Option<Box<Expression>>,
    pub else_payload: Option<Payload>,
    pub else_branch: Option<Box<Expression>>,
    pub tokens: Vec<Token>,
}

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq)]
pub struct WhileExpr {
    pub pos: usize,
    pub label: Option<Ident>,
    pub inline_: bool,
    pub condition: Option<Box<Expression>>,
    pub payload: Option<Payload>,
    pub continue_expr: Option<Box<Expression>>,
    pub body: Option<Box<Expression>>,
    pub else_payload: Option<Payload>,
    pub else_branch: Option<Box<Expression>>,
    pub tokens: Vec<Token>,
}

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq)]
pub struct ForExpr {
    pub pos: usize,
    pub label: Option<Ident>,
    pub inline_: bool,
    pub items: Vec<ForItem>,
    pub payload: Option<Payload>,
    pub body: Option<Box<Expression>>,
    pub else_branch: Option<Box<Expression>>,
    pub tokens: Vec<Token>,
}

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq)]
pub struct SwitchExpr {
    pub pos: usize,
    pub label: Option<Ident>,
    pub target: Option<Box<Expression>>,
    pub prongs: Vec<SwitchProng>,
    pub tokens: Vec<Token>,
}

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq)]
pub struct Block {
    pub pos: usize,
    pub label: Option<Ident>,
    pub statements: Vec<Statement>,
    pub tokens: Vec<Token>,
}

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq)]
pub struct BreakExpr {
    pub pos: usize,
    pub label: Option<Ident>,
    pub value: Option<Box<Expression>>,
}

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq)]
pub struct ContinueExpr {
    pub pos: usize,
    pub label: Option<Ident>,
    pub value: Option<Box<Expression>>,
}

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq)]
pub struct ReturnExpr {
    pub pos: usize,
    pub value: Option<Box<Expression>>,
}

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq)]
pub struct AsmExpr {
    pub pos: usize,
    pub volatile: bool,
    pub expr: Option<Box<Expression>>,
    pub outputs: Vec<AsmOutput>,
    pub inputs: Vec<AsmInput>,
    pub clobbers: Option<Box<Expression>>,
    pub tokens: Vec<Token>,
}

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq)]
pub struct Call {
    pub pos: usize,
    pub callee: Box<Expression>,
    pub args: Vec<Expression>,
}

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq)]
pub struct Index {
    pub pos: usize,
    pub expr: Box<Expression>,
    pub index: Box<Expression>,
}

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq)]
pub struct Slice {
    pub pos: usize,
    pub expr: Box<Expression>,
    pub start: Option<Box<Expression>>,
    pub end: Option<Box<Expression>>,
    pub sentinel: Option<Box<Expression>>,
    pub tokens: Vec<Token>,
}

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq)]
pub struct FieldAccess {
    pub pos: usize,
    pub expr: Box<Expression>,
    pub field: Ident,
}

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq)]
pub struct Deref {
    pub pos: usize,
    pub expr: Box<Expression>,
}

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq)]
pub struct Unwrap {
    pub pos: usize,
    pub expr: Box<Expression>,
}

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq)]
pub struct CurlyInit {
    pub pos: usize,
    pub ty: Box<Expression>,
    pub entries: Vec<InitEntry>,
    pub tokens: Vec<Token>,
}

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq)]
pub struct OptionalType {
    pub pos: usize,
    pub child: Box<Expression>,
}

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq)]
pub struct ErrorUnionType {
    pub pos: usize,
    pub error: Box<Expression>,
    pub payload: Box<Expression>,
}

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq)]
pub struct PointerType {
    pub pos: usize,
    pub kind: PointerKind,
    pub modifiers: Vec<PointerModifier>,
    pub child: Box<Expression>,
    pub tokens: Vec<Token>,
}

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq)]
pub struct SliceType {
    pub pos: usize,
    pub sentinel: Option<Box<Expression>>,
    pub modifiers: Vec<PointerModifier>,
    pub child: Box<Expression>,
    pub tokens: Vec<Token>,
}

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq)]
pub struct ArrayType {
    pub pos: usize,
    pub len: Box<Expression>,
    pub sentinel: Option<Box<Expression>>,
    pub child: Box<Expression>,
    pub tokens: Vec<Token>,
}

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq)]
pub struct ContainerDecl {
    pub pos: usize,
    pub layout: Option<Keyword>,
    pub kind: Keyword,
    pub arg: Option<ContainerArg>,
    pub members: Vec<ContainerMember>,
    pub tokens: Vec<Token>,
}

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq)]
pub enum ContainerArg {
    Expr(Box<Expression>),
    Enum {
        pos: usize,
        tag_type: Option<Box<Expression>>,
        tokens: Vec<Token>,
    },
}

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq)]
pub enum PointerKind {
    One,
    Many,
    C,
    Sentinel(Box<Expression>),
}

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq)]
pub enum PointerModifier {
    Align(Vec<Expression>),
    AddrSpace(Box<Expression>),
    Const,
    Volatile,
    AllowZero,
}

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq)]
pub struct AsmOutput {
    pub pos: usize,
    pub name: Ident,
    pub constraint: Expression,
    pub target: AsmOutputTarget,
}

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq)]
pub enum AsmOutputTarget {
    Type(Expression),
    Ident(Ident),
}

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq)]
pub struct AsmInput {
    pub pos: usize,
    pub name: Ident,
    pub constraint: Expression,
    pub expr: Expression,
}

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq)]
pub enum Statement {
    Var(Box<VarDecl>),
    Defer(Box<DeferStmt>),
    If(Box<IfStmt>),
    Loop(Box<LoopStmt>),
    Switch(Box<SwitchStmt>),
    Suspend(Box<SuspendStmt>),
    Nosuspend(Box<NosuspendStmt>),
    Comptime(Box<ComptimeStmt>),
    Assign(Box<AssignStmt>),
    Expr(Box<ExprStmt>),
    Block(Box<Block>),
}

impl Statement {
    pub fn pos(&self) -> usize {
        match self {
            Self::Var(node) => node.pos,
            Self::Defer(node) => node.pos,
            Self::If(node) => node.pos,
            Self::Loop(node) => node.pos,
            Self::Switch(node) => node.pos,
            Self::Suspend(node) => node.pos,
            Self::Nosuspend(node) => node.pos,
            Self::Comptime(node) => node.pos,
            Self::Assign(node) => node.pos,
            Self::Expr(node) => node.pos,
            Self::Block(node) => node.pos,
        }
    }
}

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq)]
pub struct DeferStmt {
    pub pos: usize,
    pub err: bool,
    pub payload: Option<Payload>,
    pub body: Box<Statement>,
    pub tokens: Vec<Token>,
}

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq)]
pub struct IfStmt {
    pub pos: usize,
    pub condition: Expression,
    pub payload: Option<Payload>,
    pub then_branch: Box<Statement>,
    pub else_payload: Option<Payload>,
    pub else_branch: Option<Box<Statement>>,
    pub tokens: Vec<Token>,
}

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq)]
pub struct LoopStmt {
    pub pos: usize,
    pub label: Option<Ident>,
    pub inline_: bool,
    pub kind: LoopKind,
    pub body: Box<Statement>,
    pub else_payload: Option<Payload>,
    pub else_branch: Option<Box<Statement>>,
    pub tokens: Vec<Token>,
}

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq)]
pub enum LoopKind {
    While {
        condition: Box<Expression>,
        payload: Option<Payload>,
        continue_expr: Option<Box<Expression>>,
    },
    For {
        items: Vec<ForItem>,
        payload: Payload,
    },
}

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq)]
pub struct SwitchStmt {
    pub pos: usize,
    pub label: Option<Ident>,
    pub expr: SwitchExpr,
    pub tokens: Vec<Token>,
}

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq)]
pub struct SuspendStmt {
    pub pos: usize,
    pub body: Box<Statement>,
    pub tokens: Vec<Token>,
}

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq)]
pub struct NosuspendStmt {
    pub pos: usize,
    pub body: Box<Statement>,
    pub tokens: Vec<Token>,
}

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq)]
pub struct ComptimeStmt {
    pub pos: usize,
    pub body: Box<Statement>,
    pub tokens: Vec<Token>,
}

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq)]
pub struct AssignStmt {
    pub pos: usize,
    pub lhs: Vec<Expression>,
    pub op: Token,
    pub rhs: Expression,
    pub tokens: Vec<Token>,
}

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq)]
pub struct ExprStmt {
    pub pos: usize,
    pub expr: Expression,
    pub semicolon: bool,
    pub tokens: Vec<Token>,
}

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq)]
pub struct Payload {
    pub pos: usize,
    pub star: bool,
    pub names: Vec<Ident>,
    pub stars: Vec<bool>,
}

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq)]
pub struct ParamDecl {
    pub pos: usize,
    pub docs: Vec<Rc<Comment>>,
    pub noalias: bool,
    pub comptime: bool,
    pub name: Option<Ident>,
    pub ty: ParamType,
}

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq)]
pub enum ParamType {
    AnyType(usize),
    Type(Box<Expression>),
    VarArgs(usize),
}

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq)]
pub struct SwitchProng {
    pub pos: usize,
    pub inline_: bool,
    pub cases: Vec<SwitchItem>,
    pub payload: Option<Payload>,
    pub body: Expression,
}

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq)]
pub enum SwitchItem {
    Expr(Box<Expression>),
    Range(Box<Expression>, Box<Expression>),
    Else(usize),
}

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq)]
pub struct ForItem {
    pub pos: usize,
    pub expr: Expression,
    pub range: Option<Option<Expression>>,
}

#[cfg(test)]
mod tests {
    use crate::token::Symbol;

    use super::*;

    #[test]
    fn expression_pos_uses_inner_node() {
        let expr = Expression::Binary(BinaryOp {
            pos: 12,
            op: Token::Symbol(Symbol::Plus),
            lhs: Box::new(Expression::Ident(Ident {
                pos: 10,
                name: "a".to_owned(),
            })),
            rhs: Box::new(Expression::BasicLit(BasicLit {
                pos: 14,
                kind: LitKind::Integer,
                text: "1".to_owned(),
            })),
        });

        assert_eq!(expr.pos(), 12);
    }
}
