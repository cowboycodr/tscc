use crate::diagnostics::CompileError;
use crate::lexer::token::{Span, SpannedToken, Token};
use crate::parser::ast::*;

pub struct Parser {
    tokens: Vec<SpannedToken>,
    current: usize,
}

impl Parser {
    pub fn new(tokens: Vec<SpannedToken>) -> Self {
        Self { tokens, current: 0 }
    }

    pub fn parse(&mut self) -> Result<Program, CompileError> {
        let mut statements = Vec::new();
        while !self.is_at_end() {
            statements.push(self.statement()?);
        }
        Ok(Program { statements })
    }

    // --- Statements ---

    fn statement(&mut self) -> Result<Statement, CompileError> {
        match self.peek_token() {
            Token::Let | Token::Const => self.variable_declaration(false),
            Token::Function => self.function_declaration(false),
            Token::If => self.if_statement(),
            Token::While => self.while_statement(),
            Token::For => self.for_statement(),
            Token::Return => self.return_statement(),
            Token::LeftBrace => self.block_statement(),
            Token::Import => self.import_declaration(),
            Token::Export => self.export_declaration(),
            _ => self.expression_statement(),
        }
    }

    fn variable_declaration(&mut self, is_exported: bool) -> Result<Statement, CompileError> {
        let start_span = self.current_span();
        let is_const = matches!(self.peek_token(), Token::Const);
        self.advance();

        let name = self.expect_identifier("Expected variable name")?;

        let type_ann = if self.match_token(&Token::Colon) {
            Some(self.type_annotation()?)
        } else {
            None
        };

        let initializer = if self.match_token(&Token::Assign) {
            Some(self.expression()?)
        } else {
            None
        };

        self.consume_semicolon()?;

        Ok(Statement {
            kind: StmtKind::VariableDecl {
                name,
                is_const,
                type_ann,
                initializer,
                is_exported,
            },
            span: self.span_from(&start_span),
        })
    }

    fn function_declaration(&mut self, is_exported: bool) -> Result<Statement, CompileError> {
        let start_span = self.current_span();
        self.advance();

        let name = self.expect_identifier("Expected function name")?;

        self.expect(&Token::LeftParen, "Expected '(' after function name")?;
        let params = self.parameter_list()?;
        self.expect(&Token::RightParen, "Expected ')' after parameters")?;

        let return_type = if self.match_token(&Token::Colon) {
            Some(self.type_annotation()?)
        } else {
            None
        };

        self.expect(&Token::LeftBrace, "Expected '{' before function body")?;
        let body = self.block_body()?;

        Ok(Statement {
            kind: StmtKind::FunctionDecl {
                name,
                params,
                return_type,
                body,
                is_exported,
            },
            span: self.span_from(&start_span),
        })
    }

    fn import_declaration(&mut self) -> Result<Statement, CompileError> {
        let start_span = self.current_span();
        self.advance(); // consume 'import'

        self.expect(&Token::LeftBrace, "Expected '{' in import declaration")?;

        let mut specifiers = Vec::new();
        if !self.check(&Token::RightBrace) {
            loop {
                let spec_span = self.current_span();
                let imported = self.expect_identifier("Expected imported name")?;

                let local = if self.match_token(&Token::As) {
                    self.expect_identifier("Expected local name after 'as'")?
                } else {
                    imported.clone()
                };

                specifiers.push(ImportSpecifier {
                    imported,
                    local,
                    span: self.span_from(&spec_span),
                });

                if !self.match_token(&Token::Comma) {
                    break;
                }
            }
        }
        self.expect(&Token::RightBrace, "Expected '}' in import declaration")?;

        self.expect(&Token::From, "Expected 'from' after import specifiers")?;
        let source = self.expect_string("Expected module path string")?;
        self.consume_semicolon()?;

        Ok(Statement {
            kind: StmtKind::Import { specifiers, source },
            span: self.span_from(&start_span),
        })
    }

    fn export_declaration(&mut self) -> Result<Statement, CompileError> {
        let _start_span = self.current_span();
        self.advance(); // consume 'export'

        match self.peek_token() {
            Token::Function => self.function_declaration(true),
            Token::Let | Token::Const => self.variable_declaration(true),
            _ => Err(self.error("Expected function, let, or const after 'export'")),
        }
    }

    fn parameter_list(&mut self) -> Result<Vec<Parameter>, CompileError> {
        let mut params = Vec::new();
        if !self.check(&Token::RightParen) {
            loop {
                let param_span = self.current_span();
                let name = self.expect_identifier("Expected parameter name")?;
                let type_ann = if self.match_token(&Token::Colon) {
                    Some(self.type_annotation()?)
                } else {
                    None
                };
                params.push(Parameter {
                    name,
                    type_ann,
                    span: self.span_from(&param_span),
                });
                if !self.match_token(&Token::Comma) {
                    break;
                }
            }
        }
        Ok(params)
    }

    fn if_statement(&mut self) -> Result<Statement, CompileError> {
        let start_span = self.current_span();
        self.advance();

        self.expect(&Token::LeftParen, "Expected '(' after 'if'")?;
        let condition = self.expression()?;
        self.expect(&Token::RightParen, "Expected ')' after if condition")?;

        self.expect(&Token::LeftBrace, "Expected '{' after if condition")?;
        let then_branch = self.block_body()?;

        let else_branch = if self.match_token(&Token::Else) {
            if self.check(&Token::If) {
                let else_if = self.if_statement()?;
                Some(vec![else_if])
            } else {
                self.expect(&Token::LeftBrace, "Expected '{' after 'else'")?;
                Some(self.block_body()?)
            }
        } else {
            None
        };

        Ok(Statement {
            kind: StmtKind::If {
                condition,
                then_branch,
                else_branch,
            },
            span: self.span_from(&start_span),
        })
    }

    fn while_statement(&mut self) -> Result<Statement, CompileError> {
        let start_span = self.current_span();
        self.advance();

        self.expect(&Token::LeftParen, "Expected '(' after 'while'")?;
        let condition = self.expression()?;
        self.expect(&Token::RightParen, "Expected ')' after while condition")?;

        self.expect(&Token::LeftBrace, "Expected '{' after while condition")?;
        let body = self.block_body()?;

        Ok(Statement {
            kind: StmtKind::While { condition, body },
            span: self.span_from(&start_span),
        })
    }

    fn for_statement(&mut self) -> Result<Statement, CompileError> {
        let start_span = self.current_span();
        self.advance();

        self.expect(&Token::LeftParen, "Expected '(' after 'for'")?;

        let init = if self.match_token(&Token::Semicolon) {
            None
        } else if matches!(self.peek_token(), Token::Let | Token::Const) {
            let decl = self.variable_declaration(false)?;
            Some(Box::new(decl))
        } else {
            let expr = self.expression()?;
            self.consume_semicolon()?;
            Some(Box::new(Statement {
                span: expr.span.clone(),
                kind: StmtKind::Expression { expr },
            }))
        };

        let condition = if self.check(&Token::Semicolon) {
            None
        } else {
            Some(self.expression()?)
        };
        self.consume_semicolon()?;

        let update = if self.check(&Token::RightParen) {
            None
        } else {
            Some(self.expression()?)
        };
        self.expect(&Token::RightParen, "Expected ')' after for clauses")?;

        self.expect(&Token::LeftBrace, "Expected '{' after for clauses")?;
        let body = self.block_body()?;

        Ok(Statement {
            kind: StmtKind::For {
                init,
                condition,
                update,
                body,
            },
            span: self.span_from(&start_span),
        })
    }

    fn return_statement(&mut self) -> Result<Statement, CompileError> {
        let start_span = self.current_span();
        self.advance();

        let value = if self.check(&Token::Semicolon) || self.check(&Token::RightBrace) {
            None
        } else {
            Some(self.expression()?)
        };

        self.consume_semicolon()?;

        Ok(Statement {
            kind: StmtKind::Return { value },
            span: self.span_from(&start_span),
        })
    }

    fn block_statement(&mut self) -> Result<Statement, CompileError> {
        let start_span = self.current_span();
        self.advance();
        let statements = self.block_body()?;
        Ok(Statement {
            kind: StmtKind::Block { statements },
            span: self.span_from(&start_span),
        })
    }

    fn block_body(&mut self) -> Result<Vec<Statement>, CompileError> {
        let mut statements = Vec::new();
        while !self.check(&Token::RightBrace) && !self.is_at_end() {
            statements.push(self.statement()?);
        }
        self.expect(&Token::RightBrace, "Expected '}'")?;
        Ok(statements)
    }

    fn expression_statement(&mut self) -> Result<Statement, CompileError> {
        let start_span = self.current_span();
        let expr = self.expression()?;
        self.consume_semicolon()?;
        Ok(Statement {
            kind: StmtKind::Expression { expr },
            span: self.span_from(&start_span),
        })
    }

    // --- Type Annotations ---

    fn type_annotation(&mut self) -> Result<TypeAnnotation, CompileError> {
        let span = self.current_span();
        let kind = match self.peek_token() {
            Token::NumberType => {
                self.advance();
                TypeAnnKind::Number
            }
            Token::StringType => {
                self.advance();
                TypeAnnKind::String
            }
            Token::BooleanType => {
                self.advance();
                TypeAnnKind::Boolean
            }
            Token::Void => {
                self.advance();
                TypeAnnKind::Void
            }
            Token::Null => {
                self.advance();
                TypeAnnKind::Null
            }
            Token::Undefined => {
                self.advance();
                TypeAnnKind::Undefined
            }
            _ => {
                return Err(self.error("Expected type annotation"));
            }
        };
        Ok(TypeAnnotation {
            kind,
            span: self.span_from(&span),
        })
    }

    // --- Expressions ---

    fn expression(&mut self) -> Result<Expr, CompileError> {
        self.assignment()
    }

    fn assignment(&mut self) -> Result<Expr, CompileError> {
        let expr = self.logical_or()?;

        if self.match_token(&Token::Assign) {
            let value = self.assignment()?;
            if let ExprKind::Identifier(name) = &expr.kind {
                return Ok(Expr {
                    span: Span::new(
                        expr.span.start,
                        value.span.end,
                        expr.span.line,
                        expr.span.column,
                    ),
                    kind: ExprKind::Assignment {
                        name: name.clone(),
                        value: Box::new(value),
                    },
                });
            }
            return Err(CompileError::error("Invalid assignment target", expr.span));
        }

        Ok(expr)
    }

    fn logical_or(&mut self) -> Result<Expr, CompileError> {
        let mut left = self.logical_and()?;
        while self.match_token(&Token::PipePipe) {
            let right = self.logical_and()?;
            left = Expr {
                span: Span::new(
                    left.span.start,
                    right.span.end,
                    left.span.line,
                    left.span.column,
                ),
                kind: ExprKind::Binary {
                    left: Box::new(left.clone()),
                    op: BinOp::Or,
                    right: Box::new(right),
                },
            };
        }
        Ok(left)
    }

    fn logical_and(&mut self) -> Result<Expr, CompileError> {
        let mut left = self.equality()?;
        while self.match_token(&Token::AmpersandAmpersand) {
            let right = self.equality()?;
            left = Expr {
                span: Span::new(
                    left.span.start,
                    right.span.end,
                    left.span.line,
                    left.span.column,
                ),
                kind: ExprKind::Binary {
                    left: Box::new(left.clone()),
                    op: BinOp::And,
                    right: Box::new(right),
                },
            };
        }
        Ok(left)
    }

    fn equality(&mut self) -> Result<Expr, CompileError> {
        let mut left = self.comparison()?;
        loop {
            let op = match self.peek_token() {
                Token::EqualEqual => BinOp::Equal,
                Token::EqualEqualEqual => BinOp::StrictEqual,
                Token::BangEqual => BinOp::NotEqual,
                Token::BangEqualEqual => BinOp::StrictNotEqual,
                _ => break,
            };
            self.advance();
            let right = self.comparison()?;
            left = Expr {
                span: Span::new(
                    left.span.start,
                    right.span.end,
                    left.span.line,
                    left.span.column,
                ),
                kind: ExprKind::Binary {
                    left: Box::new(left.clone()),
                    op,
                    right: Box::new(right),
                },
            };
        }
        Ok(left)
    }

    fn comparison(&mut self) -> Result<Expr, CompileError> {
        let mut left = self.additive()?;
        loop {
            let op = match self.peek_token() {
                Token::Less => BinOp::Less,
                Token::Greater => BinOp::Greater,
                Token::LessEqual => BinOp::LessEqual,
                Token::GreaterEqual => BinOp::GreaterEqual,
                _ => break,
            };
            self.advance();
            let right = self.additive()?;
            left = Expr {
                span: Span::new(
                    left.span.start,
                    right.span.end,
                    left.span.line,
                    left.span.column,
                ),
                kind: ExprKind::Binary {
                    left: Box::new(left.clone()),
                    op,
                    right: Box::new(right),
                },
            };
        }
        Ok(left)
    }

    fn additive(&mut self) -> Result<Expr, CompileError> {
        let mut left = self.multiplicative()?;
        loop {
            let op = match self.peek_token() {
                Token::Plus => BinOp::Add,
                Token::Minus => BinOp::Subtract,
                _ => break,
            };
            self.advance();
            let right = self.multiplicative()?;
            left = Expr {
                span: Span::new(
                    left.span.start,
                    right.span.end,
                    left.span.line,
                    left.span.column,
                ),
                kind: ExprKind::Binary {
                    left: Box::new(left.clone()),
                    op,
                    right: Box::new(right),
                },
            };
        }
        Ok(left)
    }

    fn multiplicative(&mut self) -> Result<Expr, CompileError> {
        let mut left = self.unary()?;
        loop {
            let op = match self.peek_token() {
                Token::Star => BinOp::Multiply,
                Token::Slash => BinOp::Divide,
                Token::Percent => BinOp::Modulo,
                _ => break,
            };
            self.advance();
            let right = self.unary()?;
            left = Expr {
                span: Span::new(
                    left.span.start,
                    right.span.end,
                    left.span.line,
                    left.span.column,
                ),
                kind: ExprKind::Binary {
                    left: Box::new(left.clone()),
                    op,
                    right: Box::new(right),
                },
            };
        }
        Ok(left)
    }

    fn unary(&mut self) -> Result<Expr, CompileError> {
        match self.peek_token() {
            Token::Minus => {
                let span = self.current_span();
                self.advance();
                let operand = self.unary()?;
                Ok(Expr {
                    span: self.span_from(&span),
                    kind: ExprKind::Unary {
                        op: UnaryOp::Negate,
                        operand: Box::new(operand),
                    },
                })
            }
            Token::Bang => {
                let span = self.current_span();
                self.advance();
                let operand = self.unary()?;
                Ok(Expr {
                    span: self.span_from(&span),
                    kind: ExprKind::Unary {
                        op: UnaryOp::Not,
                        operand: Box::new(operand),
                    },
                })
            }
            Token::Typeof => {
                let span = self.current_span();
                self.advance();
                let operand = self.unary()?;
                Ok(Expr {
                    span: self.span_from(&span),
                    kind: ExprKind::Typeof {
                        operand: Box::new(operand),
                    },
                })
            }
            Token::PlusPlus => {
                let span = self.current_span();
                self.advance();
                let name = self.expect_identifier("Expected variable name after '++'")?;
                Ok(Expr {
                    span: self.span_from(&span),
                    kind: ExprKind::PrefixUpdate {
                        name,
                        op: UpdateOp::Increment,
                    },
                })
            }
            Token::MinusMinus => {
                let span = self.current_span();
                self.advance();
                let name = self.expect_identifier("Expected variable name after '--'")?;
                Ok(Expr {
                    span: self.span_from(&span),
                    kind: ExprKind::PrefixUpdate {
                        name,
                        op: UpdateOp::Decrement,
                    },
                })
            }
            _ => self.postfix(),
        }
    }

    fn postfix(&mut self) -> Result<Expr, CompileError> {
        let mut expr = self.call()?;

        match self.peek_token() {
            Token::PlusPlus => {
                if let ExprKind::Identifier(name) = &expr.kind {
                    let name = name.clone();
                    self.advance();
                    expr = Expr {
                        span: self.span_from(&expr.span),
                        kind: ExprKind::PostfixUpdate {
                            name,
                            op: UpdateOp::Increment,
                        },
                    };
                }
            }
            Token::MinusMinus => {
                if let ExprKind::Identifier(name) = &expr.kind {
                    let name = name.clone();
                    self.advance();
                    expr = Expr {
                        span: self.span_from(&expr.span),
                        kind: ExprKind::PostfixUpdate {
                            name,
                            op: UpdateOp::Decrement,
                        },
                    };
                }
            }
            _ => {}
        }

        Ok(expr)
    }

    fn call(&mut self) -> Result<Expr, CompileError> {
        let mut expr = self.primary()?;

        loop {
            if self.match_token(&Token::LeftParen) {
                let mut args = Vec::new();
                if !self.check(&Token::RightParen) {
                    loop {
                        args.push(self.expression()?);
                        if !self.match_token(&Token::Comma) {
                            break;
                        }
                    }
                }
                self.expect(&Token::RightParen, "Expected ')' after arguments")?;
                expr = Expr {
                    span: Span::new(
                        expr.span.start,
                        self.previous_span().end,
                        expr.span.line,
                        expr.span.column,
                    ),
                    kind: ExprKind::Call {
                        callee: Box::new(expr.clone()),
                        args,
                    },
                };
            } else if self.match_token(&Token::Dot) {
                let property = self.expect_identifier("Expected property name after '.'")?;
                expr = Expr {
                    span: Span::new(
                        expr.span.start,
                        self.previous_span().end,
                        expr.span.line,
                        expr.span.column,
                    ),
                    kind: ExprKind::Member {
                        object: Box::new(expr.clone()),
                        property,
                    },
                };
            } else {
                break;
            }
        }

        Ok(expr)
    }

    fn primary(&mut self) -> Result<Expr, CompileError> {
        let span = self.current_span();
        match self.peek_token() {
            Token::Number(n) => {
                let val = n;
                self.advance();
                Ok(Expr {
                    kind: ExprKind::NumberLiteral(val),
                    span: self.span_from(&span),
                })
            }
            Token::String(s) => {
                let val = s;
                self.advance();
                Ok(Expr {
                    kind: ExprKind::StringLiteral(val),
                    span: self.span_from(&span),
                })
            }
            Token::True => {
                self.advance();
                Ok(Expr {
                    kind: ExprKind::BooleanLiteral(true),
                    span: self.span_from(&span),
                })
            }
            Token::False => {
                self.advance();
                Ok(Expr {
                    kind: ExprKind::BooleanLiteral(false),
                    span: self.span_from(&span),
                })
            }
            Token::Null => {
                self.advance();
                Ok(Expr {
                    kind: ExprKind::NullLiteral,
                    span: self.span_from(&span),
                })
            }
            Token::Undefined => {
                self.advance();
                Ok(Expr {
                    kind: ExprKind::UndefinedLiteral,
                    span: self.span_from(&span),
                })
            }
            Token::Identifier(_) => {
                let name = self.expect_identifier("")?;
                Ok(Expr {
                    kind: ExprKind::Identifier(name),
                    span: self.span_from(&span),
                })
            }
            Token::LeftParen => {
                self.advance();
                if let Some(arrow) = self.try_arrow_function(&span)? {
                    return Ok(arrow);
                }
                let expr = self.expression()?;
                self.expect(&Token::RightParen, "Expected ')'")?;
                Ok(Expr {
                    kind: ExprKind::Grouping {
                        expr: Box::new(expr),
                    },
                    span: self.span_from(&span),
                })
            }
            _ => Err(self.error(&format!("Unexpected token: {:?}", self.peek_token()))),
        }
    }

    fn try_arrow_function(&mut self, start_span: &Span) -> Result<Option<Expr>, CompileError> {
        let saved = self.current;

        let params_result = self.try_parse_arrow_params();
        match params_result {
            Some(params) => {
                if self.match_token(&Token::RightParen) {
                    let return_type = if self.match_token(&Token::Colon) {
                        Some(self.type_annotation()?)
                    } else {
                        None
                    };

                    if self.match_token(&Token::Arrow) {
                        let body = if self.check(&Token::LeftBrace) {
                            self.advance();
                            ArrowBody::Block(self.block_body()?)
                        } else {
                            ArrowBody::Expr(Box::new(self.expression()?))
                        };

                        return Ok(Some(Expr {
                            kind: ExprKind::ArrowFunction {
                                params,
                                return_type,
                                body,
                            },
                            span: self.span_from(start_span),
                        }));
                    }
                }
                self.current = saved;
                Ok(None)
            }
            None => {
                self.current = saved;
                Ok(None)
            }
        }
    }

    fn try_parse_arrow_params(&mut self) -> Option<Vec<Parameter>> {
        let mut params = Vec::new();
        if self.check(&Token::RightParen) {
            return Some(params);
        }

        loop {
            let param_span = self.current_span();
            if let Token::Identifier(name) = self.peek_token() {
                self.advance();
                let type_ann = if self.match_token(&Token::Colon) {
                    match self.type_annotation() {
                        Ok(t) => Some(t),
                        Err(_) => return None,
                    }
                } else {
                    None
                };
                params.push(Parameter {
                    name,
                    type_ann,
                    span: self.span_from(&param_span),
                });
                if !self.match_token(&Token::Comma) {
                    break;
                }
            } else {
                return None;
            }
        }

        Some(params)
    }

    // --- Helpers ---

    fn peek_token(&self) -> Token {
        self.tokens
            .get(self.current)
            .map(|t| t.token.clone())
            .unwrap_or(Token::Eof)
    }

    fn current_span(&self) -> Span {
        self.tokens
            .get(self.current)
            .map(|t| t.span.clone())
            .unwrap_or(Span::new(0, 0, 0, 0))
    }

    fn previous_span(&self) -> Span {
        self.tokens
            .get(self.current.saturating_sub(1))
            .map(|t| t.span.clone())
            .unwrap_or(Span::new(0, 0, 0, 0))
    }

    fn span_from(&self, start: &Span) -> Span {
        let end = self.previous_span();
        Span::new(start.start, end.end, start.line, start.column)
    }

    fn advance(&mut self) -> &SpannedToken {
        if !self.is_at_end() {
            self.current += 1;
        }
        &self.tokens[self.current - 1]
    }

    fn check(&self, token: &Token) -> bool {
        std::mem::discriminant(&self.peek_token()) == std::mem::discriminant(token)
    }

    fn match_token(&mut self, token: &Token) -> bool {
        if self.check(token) {
            self.advance();
            true
        } else {
            false
        }
    }

    fn expect(&mut self, token: &Token, message: &str) -> Result<(), CompileError> {
        if self.check(token) {
            self.advance();
            Ok(())
        } else {
            Err(self.error(message))
        }
    }

    fn expect_identifier(&mut self, message: &str) -> Result<String, CompileError> {
        if let Token::Identifier(name) = self.peek_token() {
            self.advance();
            Ok(name)
        } else {
            Err(self.error(message))
        }
    }

    fn expect_string(&mut self, message: &str) -> Result<String, CompileError> {
        if let Token::String(s) = self.peek_token() {
            self.advance();
            Ok(s)
        } else {
            Err(self.error(message))
        }
    }

    fn consume_semicolon(&mut self) -> Result<(), CompileError> {
        self.match_token(&Token::Semicolon);
        Ok(())
    }

    fn is_at_end(&self) -> bool {
        matches!(self.peek_token(), Token::Eof)
    }

    fn error(&self, message: &str) -> CompileError {
        CompileError::error(message, self.current_span())
    }
}
