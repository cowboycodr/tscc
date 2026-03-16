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
        is_exported: bool,
    },
    FunctionDecl {
        name: String,
        type_params: Vec<TypeParam>,
        params: Vec<Parameter>,
        return_type: Option<TypeAnnotation>,
        body: Vec<Statement>,
        is_exported: bool,
    },
    ClassDecl {
        name: String,
        parent: Option<String>,
        fields: Vec<ClassField>,
        constructor: Option<ClassConstructor>,
        methods: Vec<ClassMethod>,
    },
    InterfaceDecl {
        name: String,
        extends: Vec<String>,
        fields: Vec<(String, TypeAnnotation)>,
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
    DoWhile {
        body: Vec<Statement>,
        condition: Expr,
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
    Import {
        specifiers: Vec<ImportSpecifier>,
        source: String,
    },
    Switch {
        discriminant: Expr,
        cases: Vec<SwitchCase>,
    },
    ForOf {
        var_name: String,
        iterable: Expr,
        body: Vec<Statement>,
    },
    ForIn {
        var_name: String,
        object: Expr,
        body: Vec<Statement>,
    },
    ArrayDestructure {
        names: Vec<String>,
        initializer: Expr,
        is_const: bool,
    },
    ObjectDestructure {
        /// (binding_name, key_name) pairs
        names: Vec<(String, String)>,
        initializer: Expr,
        is_const: bool,
    },
    TypeAlias {
        name: String,
        type_params: Vec<TypeParam>,
        type_ann: TypeAnnotation,
    },
    EnumDecl {
        name: String,
        members: Vec<EnumMember>,
    },
    Break {
        label: Option<String>,
    },
    Continue {
        label: Option<String>,
    },
    Labeled {
        label: String,
        body: Box<Statement>,
    },
    Empty,
}

#[derive(Debug, Clone)]
pub struct SwitchCase {
    pub test: Option<Expr>,
    pub body: Vec<Statement>,
}

#[derive(Debug, Clone)]
pub struct ClassField {
    pub name: String,
    pub type_ann: Option<TypeAnnotation>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct ClassConstructor {
    pub params: Vec<Parameter>,
    pub body: Vec<Statement>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct ClassMethod {
    pub name: String,
    pub params: Vec<Parameter>,
    pub return_type: Option<TypeAnnotation>,
    pub body: Vec<Statement>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct EnumMember {
    pub name: String,
    pub value: Option<EnumValue>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub enum EnumValue {
    Number(f64),
    String(String),
}

#[derive(Debug, Clone)]
pub struct ImportSpecifier {
    pub imported: String,
    pub local: String,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct TypeParam {
    pub name: String,
    pub constraint: Option<TypeAnnotation>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct Parameter {
    pub name: String,
    pub type_ann: Option<TypeAnnotation>,
    pub default: Option<Expr>,
    pub is_rest: bool,
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
    Array(Box<TypeAnnotation>),
    Object {
        fields: Vec<(String, TypeAnnotation)>,
    },
    /// A named type reference (e.g., a class or interface name)
    Named(String),
    /// typeof x — resolved by looking up the variable's type
    Typeof(String),
    /// String literal type: "red", "blue", etc.
    StringLiteral(String),
    /// Number literal type: 0, 1, 42, etc.
    NumberLiteral(f64),
    /// Union type: string | number
    Union(Vec<TypeAnnotation>),
    /// Intersection type: Named & Aged
    Intersection(Vec<TypeAnnotation>),
    /// keyof Type — resolves to union of string literal keys
    Keyof(Box<TypeAnnotation>),
    /// Tuple type: [number, string]
    Tuple(Vec<TypeAnnotation>),
    /// Generic named type reference with type arguments: IsNumber<number>
    Generic {
        name: String,
        type_args: Vec<TypeAnnotation>,
    },
    /// Conditional type: T extends number ? "yes" : "no"
    Conditional {
        check_type: Box<TypeAnnotation>,
        extends_type: Box<TypeAnnotation>,
        true_type: Box<TypeAnnotation>,
        false_type: Box<TypeAnnotation>,
    },
    /// Mapped type: { [P in keyof T]: T[P] }
    Mapped {
        param: String,
        constraint: Box<TypeAnnotation>,
        value_type: Box<TypeAnnotation>,
    },
    /// Indexed access type: T[P]
    IndexedAccess {
        object_type: Box<TypeAnnotation>,
        index_type: Box<TypeAnnotation>,
    },
    /// Function type: (params) => return_type
    FunctionType {
        params: Vec<TypeAnnotation>,
        return_type: Box<TypeAnnotation>,
    },
}

#[derive(Debug, Clone)]
pub struct Expr {
    pub kind: ExprKind,
    pub span: Span,
}

/// A single property in an object literal.
#[derive(Debug, Clone)]
pub struct ObjectProperty {
    pub key: String,
    pub value: Expr,
    pub is_method: bool,
    /// For methods: the parameters
    pub params: Vec<Parameter>,
    /// For methods: the return type annotation
    pub return_type: Option<TypeAnnotation>,
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
    This,
    ArrayLiteral {
        elements: Vec<Expr>,
    },
    ObjectLiteral {
        properties: Vec<ObjectProperty>,
    },
    IndexAccess {
        object: Box<Expr>,
        index: Box<Expr>,
    },
    Binary {
        left: Box<Expr>,
        op: BinOp,
        right: Box<Expr>,
    },
    Unary {
        op: UnaryOp,
        operand: Box<Expr>,
    },
    Typeof {
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
    OptionalMember {
        object: Box<Expr>,
        property: String,
    },
    Spread {
        expr: Box<Expr>,
    },
    Assignment {
        name: String,
        value: Box<Expr>,
    },
    /// Assignment to an object property: obj.prop = value
    MemberAssignment {
        object: Box<Expr>,
        property: String,
        value: Box<Expr>,
    },
    ArrowFunction {
        params: Vec<Parameter>,
        return_type: Option<TypeAnnotation>,
        body: ArrowBody,
    },
    NewExpr {
        class_name: String,
        args: Vec<Expr>,
    },
    Conditional {
        condition: Box<Expr>,
        consequent: Box<Expr>,
        alternate: Box<Expr>,
    },
    Grouping {
        expr: Box<Expr>,
    },
    PostfixUpdate {
        name: String,
        op: UpdateOp,
    },
    PrefixUpdate {
        name: String,
        op: UpdateOp,
    },
    /// Type assertion: expr as Type (erased at codegen)
    TypeAssertion {
        expr: Box<Expr>,
        target_type: TypeAnnotation,
    },
    /// Satisfies operator: expr satisfies Type (erased at codegen)
    Satisfies {
        expr: Box<Expr>,
        target_type: TypeAnnotation,
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
    NullishCoalescing,
    Power,
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
