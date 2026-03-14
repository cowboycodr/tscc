use crate::diagnostics::CompileError;
use crate::lexer::token::{Span, SpannedToken, Token};

pub struct Scanner {
    source: Vec<char>,
    tokens: Vec<SpannedToken>,
    start: usize,
    current: usize,
    line: usize,
    column: usize,
    start_column: usize,
}

impl Scanner {
    pub fn new(source: &str) -> Self {
        Self {
            source: source.chars().collect(),
            tokens: Vec::new(),
            start: 0,
            current: 0,
            line: 1,
            column: 1,
            start_column: 1,
        }
    }

    pub fn scan_tokens(mut self) -> Result<Vec<SpannedToken>, CompileError> {
        while !self.is_at_end() {
            self.start = self.current;
            self.start_column = self.column;
            self.scan_token()?;
        }

        self.tokens.push(SpannedToken::new(
            Token::Eof,
            Span::new(self.current, self.current, self.line, self.column),
        ));

        Ok(self.tokens)
    }

    fn scan_token(&mut self) -> Result<(), CompileError> {
        let c = self.advance();
        match c {
            '(' => self.add_token(Token::LeftParen),
            ')' => self.add_token(Token::RightParen),
            '{' => self.add_token(Token::LeftBrace),
            '}' => self.add_token(Token::RightBrace),
            '[' => self.add_token(Token::LeftBracket),
            ']' => self.add_token(Token::RightBracket),
            ':' => self.add_token(Token::Colon),
            ';' => self.add_token(Token::Semicolon),
            ',' => self.add_token(Token::Comma),
            '%' => self.add_token(Token::Percent),
            '*' => self.add_token(Token::Star),

            '.' => {
                if self.check('.') && self.check_next('.') {
                    self.advance();
                    self.advance();
                    self.add_token(Token::Ellipsis);
                } else {
                    self.add_token(Token::Dot);
                }
            }

            '+' => {
                if self.match_char('+') {
                    self.add_token(Token::PlusPlus);
                } else {
                    self.add_token(Token::Plus);
                }
            }

            '-' => {
                if self.match_char('-') {
                    self.add_token(Token::MinusMinus);
                } else {
                    self.add_token(Token::Minus);
                }
            }

            '=' => {
                if self.match_char('>') {
                    self.add_token(Token::Arrow);
                } else if self.match_char('=') {
                    if self.match_char('=') {
                        self.add_token(Token::EqualEqualEqual);
                    } else {
                        self.add_token(Token::EqualEqual);
                    }
                } else {
                    self.add_token(Token::Assign);
                }
            }

            '!' => {
                if self.match_char('=') {
                    if self.match_char('=') {
                        self.add_token(Token::BangEqualEqual);
                    } else {
                        self.add_token(Token::BangEqual);
                    }
                } else {
                    self.add_token(Token::Bang);
                }
            }

            '<' => {
                if self.match_char('=') {
                    self.add_token(Token::LessEqual);
                } else {
                    self.add_token(Token::Less);
                }
            }

            '>' => {
                if self.match_char('=') {
                    self.add_token(Token::GreaterEqual);
                } else {
                    self.add_token(Token::Greater);
                }
            }

            '&' => {
                if self.match_char('&') {
                    self.add_token(Token::AmpersandAmpersand);
                } else {
                    return Err(self.error("Unexpected character '&'. Did you mean '&&'?"));
                }
            }

            '|' => {
                if self.match_char('|') {
                    self.add_token(Token::PipePipe);
                } else {
                    return Err(self.error("Unexpected character '|'. Did you mean '||'?"));
                }
            }

            '/' => {
                if self.match_char('/') {
                    // Line comment
                    while !self.is_at_end() && self.peek() != '\n' {
                        self.advance();
                    }
                } else if self.match_char('*') {
                    // Block comment
                    self.block_comment()?;
                } else {
                    self.add_token(Token::Slash);
                }
            }

            ' ' | '\r' | '\t' => {}

            '\n' => {
                self.line += 1;
                self.column = 1;
            }

            '"' => self.string('"')?,
            '\'' => self.string('\'')?,
            '`' => {
                return Err(self.error("Template literals are not yet supported"));
            }

            c if c.is_ascii_digit() => self.number()?,

            c if c.is_alphabetic() || c == '_' || c == '$' => self.identifier(),

            _ => {
                return Err(self.error(&format!("Unexpected character '{}'", c)));
            }
        }
        Ok(())
    }

    fn string(&mut self, quote: char) -> Result<(), CompileError> {
        let mut value = String::new();
        while !self.is_at_end() && self.peek() != quote {
            if self.peek() == '\n' {
                self.line += 1;
                self.column = 1;
            }
            if self.peek() == '\\' {
                self.advance();
                match self.peek() {
                    'n' => value.push('\n'),
                    't' => value.push('\t'),
                    'r' => value.push('\r'),
                    '\\' => value.push('\\'),
                    '\'' => value.push('\''),
                    '"' => value.push('"'),
                    '0' => value.push('\0'),
                    _ => {
                        value.push('\\');
                        value.push(self.peek());
                    }
                }
                self.advance();
            } else {
                value.push(self.peek());
                self.advance();
            }
        }

        if self.is_at_end() {
            return Err(self.error("Unterminated string literal"));
        }

        self.advance(); // Closing quote
        self.add_token(Token::String(value));
        Ok(())
    }

    fn number(&mut self) -> Result<(), CompileError> {
        while !self.is_at_end() && self.peek().is_ascii_digit() {
            self.advance();
        }

        if !self.is_at_end() && self.peek() == '.' && self.peek_next().is_ascii_digit() {
            self.advance(); // consume the '.'
            while !self.is_at_end() && self.peek().is_ascii_digit() {
                self.advance();
            }
        }

        let text: String = self.source[self.start..self.current].iter().collect();
        let value: f64 = text
            .parse()
            .map_err(|_| self.error("Invalid number literal"))?;
        self.add_token(Token::Number(value));
        Ok(())
    }

    fn identifier(&mut self) {
        while !self.is_at_end()
            && (self.peek().is_alphanumeric() || self.peek() == '_' || self.peek() == '$')
        {
            self.advance();
        }

        let text: String = self.source[self.start..self.current].iter().collect();
        let token = match text.as_str() {
            "let" => Token::Let,
            "const" => Token::Const,
            "function" => Token::Function,
            "return" => Token::Return,
            "if" => Token::If,
            "else" => Token::Else,
            "while" => Token::While,
            "for" => Token::For,
            "of" => Token::Of,
            "true" => Token::True,
            "false" => Token::False,
            "null" => Token::Null,
            "undefined" => Token::Undefined,
            "void" => Token::Void,
            "number" => Token::NumberType,
            "string" => Token::StringType,
            "boolean" => Token::BooleanType,
            _ => Token::Identifier(text),
        };
        self.add_token(token);
    }

    fn block_comment(&mut self) -> Result<(), CompileError> {
        let mut depth = 1;
        while !self.is_at_end() && depth > 0 {
            if self.peek() == '/' && self.peek_next() == '*' {
                self.advance();
                self.advance();
                depth += 1;
            } else if self.peek() == '*' && self.peek_next() == '/' {
                self.advance();
                self.advance();
                depth -= 1;
            } else {
                if self.peek() == '\n' {
                    self.line += 1;
                    self.column = 1;
                }
                self.advance();
            }
        }

        if depth > 0 {
            return Err(self.error("Unterminated block comment"));
        }

        Ok(())
    }

    fn advance(&mut self) -> char {
        let c = self.source[self.current];
        self.current += 1;
        self.column += 1;
        c
    }

    fn peek(&self) -> char {
        if self.is_at_end() {
            '\0'
        } else {
            self.source[self.current]
        }
    }

    fn peek_next(&self) -> char {
        if self.current + 1 >= self.source.len() {
            '\0'
        } else {
            self.source[self.current + 1]
        }
    }

    fn check(&self, expected: char) -> bool {
        !self.is_at_end() && self.source[self.current] == expected
    }

    fn check_next(&self, expected: char) -> bool {
        self.current + 1 < self.source.len() && self.source[self.current + 1] == expected
    }

    fn match_char(&mut self, expected: char) -> bool {
        if self.is_at_end() || self.source[self.current] != expected {
            return false;
        }
        self.current += 1;
        self.column += 1;
        true
    }

    fn is_at_end(&self) -> bool {
        self.current >= self.source.len()
    }

    fn add_token(&mut self, token: Token) {
        let span = Span::new(self.start, self.current, self.line, self.start_column);
        self.tokens.push(SpannedToken::new(token, span));
    }

    fn error(&self, message: &str) -> CompileError {
        CompileError {
            message: message.to_string(),
            span: Span::new(self.start, self.current, self.line, self.start_column),
        }
    }
}
