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
            '%' => {
                if self.match_char('=') {
                    self.add_token(Token::PercentEqual);
                } else {
                    self.add_token(Token::Percent);
                }
            }
            '*' => {
                if self.match_char('*') {
                    self.add_token(Token::StarStar);
                } else if self.match_char('=') {
                    self.add_token(Token::StarEqual);
                } else {
                    self.add_token(Token::Star);
                }
            }
            '?' => {
                if self.match_char('?') {
                    self.add_token(Token::QuestionQuestion);
                } else if self.match_char('.') {
                    self.add_token(Token::QuestionDot);
                } else {
                    self.add_token(Token::Question);
                }
            }

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
                } else if self.match_char('=') {
                    self.add_token(Token::PlusEqual);
                } else {
                    self.add_token(Token::Plus);
                }
            }

            '-' => {
                if self.match_char('-') {
                    self.add_token(Token::MinusMinus);
                } else if self.match_char('=') {
                    self.add_token(Token::MinusEqual);
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
                    self.add_token(Token::Ampersand);
                }
            }

            '|' => {
                if self.match_char('|') {
                    self.add_token(Token::PipePipe);
                } else {
                    self.add_token(Token::Pipe);
                }
            }

            '/' => {
                if self.match_char('/') {
                    while !self.is_at_end() && self.peek() != '\n' {
                        self.advance();
                    }
                } else if self.match_char('*') {
                    self.block_comment()?;
                } else if self.match_char('=') {
                    self.add_token(Token::SlashEqual);
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
            '`' => self.template_literal()?,

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

        self.advance();
        self.add_token(Token::String(value));
        Ok(())
    }

    fn number(&mut self) -> Result<(), CompileError> {
        while !self.is_at_end() && self.peek().is_ascii_digit() {
            self.advance();
        }

        if !self.is_at_end() && self.peek() == '.' && self.peek_next().is_ascii_digit() {
            self.advance();
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
            "in" => Token::In,
            "break" => Token::Break,
            "continue" => Token::Continue,
            "do" => Token::Do,
            "switch" => Token::Switch,
            "case" => Token::Case,
            "var" => Token::Var,
            "true" => Token::True,
            "false" => Token::False,
            "null" => Token::Null,
            "undefined" => Token::Undefined,
            "void" => Token::Void,
            "number" => Token::NumberType,
            "string" => Token::StringType,
            "boolean" => Token::BooleanType,
            "import" => Token::Import,
            "export" => Token::Export,
            "from" => Token::From,
            "as" => Token::As,
            "default" => Token::Default,
            "typeof" => Token::Typeof,
            "class" => Token::Class,
            "new" => Token::New,
            "this" => Token::This,
            "extends" => Token::Extends,
            "super" => Token::Super,
            "interface" => Token::Interface,
            "enum" => Token::Enum,
            "constructor" => Token::Constructor,
            _ => Token::Identifier(text),
        };
        self.add_token(token);
    }

    fn template_literal(&mut self) -> Result<(), CompileError> {
        // Opening backtick already consumed by scan_token
        let template_span = Span::new(self.start, self.current, self.line, self.start_column);

        // Collect parts: (is_text, content)
        let mut parts: Vec<(bool, String)> = Vec::new();
        let mut current_text = String::new();

        while !self.is_at_end() && self.peek() != '`' {
            if self.peek() == '\\' {
                self.advance();
                match self.peek() {
                    'n' => current_text.push('\n'),
                    't' => current_text.push('\t'),
                    'r' => current_text.push('\r'),
                    '\\' => current_text.push('\\'),
                    '`' => current_text.push('`'),
                    '$' => current_text.push('$'),
                    '0' => current_text.push('\0'),
                    c => {
                        current_text.push('\\');
                        current_text.push(c);
                    }
                }
                self.advance();
            } else if self.peek() == '$' && self.peek_next() == '{' {
                // Save current text part
                parts.push((true, current_text.clone()));
                current_text.clear();

                self.advance(); // skip $
                self.advance(); // skip {

                // Collect expression source until matching }
                let expr_start = self.current;
                let mut depth = 1;
                while !self.is_at_end() && depth > 0 {
                    let c = self.peek();
                    match c {
                        '{' => {
                            depth += 1;
                            self.advance();
                        }
                        '}' => {
                            depth -= 1;
                            if depth > 0 {
                                self.advance();
                            }
                        }
                        '\'' | '"' => {
                            let quote = c;
                            self.advance();
                            while !self.is_at_end() && self.peek() != quote {
                                if self.peek() == '\\' {
                                    self.advance();
                                }
                                if !self.is_at_end() {
                                    self.advance();
                                }
                            }
                            if !self.is_at_end() {
                                self.advance(); // skip closing quote
                            }
                        }
                        '\n' => {
                            self.line += 1;
                            self.column = 1;
                            self.advance();
                        }
                        _ => {
                            self.advance();
                        }
                    }
                }

                if depth > 0 {
                    return Err(self.error("Unterminated template expression"));
                }

                let expr_source: String = self.source[expr_start..self.current].iter().collect();
                self.advance(); // skip closing }

                parts.push((false, expr_source));
            } else {
                if self.peek() == '\n' {
                    self.line += 1;
                    self.column = 1;
                }
                current_text.push(self.peek());
                self.advance();
            }
        }

        if self.is_at_end() {
            return Err(self.error("Unterminated template literal"));
        }
        self.advance(); // skip closing `

        // Don't forget remaining text
        if !current_text.is_empty() {
            parts.push((true, current_text));
        }

        // Check if there are any expressions
        let has_expr = parts.iter().any(|(is_text, _)| !is_text);

        if !has_expr {
            // No expressions — just concat all text parts and emit as string
            let combined: String = parts.iter().map(|(_, s)| s.as_str()).collect();
            self.tokens
                .push(SpannedToken::new(Token::String(combined), template_span));
            return Ok(());
        }

        // Has expressions — desugar to string concatenation: ("" + text + (expr) + ...)
        let span = template_span;

        // Check if we need a "" prefix to ensure string context
        let need_prefix = !parts
            .iter()
            .any(|(is_text, content)| *is_text && !content.is_empty());

        self.tokens
            .push(SpannedToken::new(Token::LeftParen, span.clone()));

        if need_prefix {
            self.tokens.push(SpannedToken::new(
                Token::String(String::new()),
                span.clone(),
            ));
        }

        let mut emitted = need_prefix;

        for (is_text, content) in &parts {
            if *is_text && content.is_empty() {
                continue; // skip empty text parts
            }

            if emitted {
                self.tokens
                    .push(SpannedToken::new(Token::Plus, span.clone()));
            }

            if *is_text {
                self.tokens.push(SpannedToken::new(
                    Token::String(content.clone()),
                    span.clone(),
                ));
            } else {
                // Wrap expression in parens to preserve evaluation order
                self.tokens
                    .push(SpannedToken::new(Token::LeftParen, span.clone()));
                let sub_scanner = Scanner::new(content);
                let sub_tokens = sub_scanner.scan_tokens()?;
                for t in sub_tokens {
                    if !matches!(t.token, Token::Eof) {
                        self.tokens.push(SpannedToken::new(t.token, span.clone()));
                    }
                }
                self.tokens
                    .push(SpannedToken::new(Token::RightParen, span.clone()));
            }

            emitted = true;
        }

        self.tokens.push(SpannedToken::new(Token::RightParen, span));

        Ok(())
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
        CompileError::error(
            message,
            Span::new(self.start, self.current, self.line, self.start_column),
        )
    }
}
