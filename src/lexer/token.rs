#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    // Literals
    Number(f64),
    String(String),
    True,
    False,
    Null,
    Undefined,

    // Identifier
    Identifier(String),

    // Keywords
    Let,
    Const,
    Function,
    Return,
    If,
    Else,
    While,
    For,
    Of,
    Void,

    // Type keywords (used in annotations)
    NumberType,
    StringType,
    BooleanType,

    // Operators
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Assign,
    EqualEqual,
    EqualEqualEqual,
    BangEqual,
    BangEqualEqual,
    Less,
    Greater,
    LessEqual,
    GreaterEqual,
    AmpersandAmpersand,
    PipePipe,
    Bang,
    PlusPlus,
    MinusMinus,

    // Punctuation
    LeftParen,
    RightParen,
    LeftBrace,
    RightBrace,
    LeftBracket,
    RightBracket,
    Colon,
    Semicolon,
    Comma,
    Dot,
    Arrow,    // =>
    Ellipsis, // ...

    // Special
    Eof,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
    pub line: usize,
    pub column: usize,
}

impl Span {
    pub fn new(start: usize, end: usize, line: usize, column: usize) -> Self {
        Self {
            start,
            end,
            line,
            column,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SpannedToken {
    pub token: Token,
    pub span: Span,
}

impl SpannedToken {
    pub fn new(token: Token, span: Span) -> Self {
        Self { token, span }
    }
}
