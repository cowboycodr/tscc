use crate::lexer::token::Span;

#[derive(Debug, Clone)]
pub struct Program {
    pub statements: Vec<Statement>,
}

#[derive(Debug, Clone)]
pub struct Statement {
    pub kind: StmtKind,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub enum StmtKind {
    VariableDecl {
        name: String,
        is_const: bool,
        type_ann: Option<TypeAnnotation>,
        initializer: Option<Expr>,
    },
    FunctionDecl {
        name: String,
        params: Vec<Parameter>,
        return_type: Option<TypeAnnotation>,
        body: Vec<Statement>,
    },
    If {
        condition: Expr,
        then_branch: Vec<Statement>,
        else_branch: Option<Vec<Statement>>,
    },
    While {
        condition: Expr,
        body: Vec<Statement>,
    },
    For {
        init: Option<Box<Statement>>,
        condition: Option<Expr>,
        update: Option<Expr>,
        body: Vec<Statement>,
    },
    Return {
        value: Option<Expr>,
    },
    Expression {
        expr: Expr,
    },
    Block {
        statements: Vec<Statement>,
    },
}

#[derive(Debug, Clone)]
pub struct Parameter {
    pub name: String,
    pub type_ann: Option<TypeAnnotation>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct TypeAnnotation {
    pub kind: TypeAnnKind,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub enum TypeAnnKind {
    Number,
    String,
    Boolean,
    Void,
    Null,
    Undefined,
}

#[derive(Debug, Clone)]
pub struct Expr {
    pub kind: ExprKind,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub enum ExprKind {
    NumberLiteral(f64),
    StringLiteral(String),
    BooleanLiteral(bool),
    NullLiteral,
    UndefinedLiteral,
    Identifier(String),
    Binary {
        left: Box<Expr>,
        op: BinOp,
        right: Box<Expr>,
    },
    Unary {
        op: UnaryOp,
        operand: Box<Expr>,
    },
    Call {
        callee: Box<Expr>,
        args: Vec<Expr>,
    },
    Member {
        object: Box<Expr>,
        property: String,
    },
    Assignment {
        name: String,
        value: Box<Expr>,
    },
    ArrowFunction {
        params: Vec<Parameter>,
        return_type: Option<TypeAnnotation>,
        body: ArrowBody,
    },
    Grouping {
        expr: Box<Expr>,
    },
    // Post-increment/decrement as expressions
    PostfixUpdate {
        name: String,
        op: UpdateOp,
    },
    // Prefix increment/decrement
    PrefixUpdate {
        name: String,
        op: UpdateOp,
    },
}

#[derive(Debug, Clone)]
pub enum ArrowBody {
    Expr(Box<Expr>),
    Block(Vec<Statement>),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BinOp {
    Add,
    Subtract,
    Multiply,
    Divide,
    Modulo,
    Equal,
    StrictEqual,
    NotEqual,
    StrictNotEqual,
    Less,
    Greater,
    LessEqual,
    GreaterEqual,
    And,
    Or,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum UnaryOp {
    Negate,
    Not,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum UpdateOp {
    Increment,
    Decrement,
}
