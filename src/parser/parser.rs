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
        // Empty statement — bare semicolons are valid TypeScript
        if self.match_token(&Token::Semicolon) {
            return Ok(Statement {
                kind: StmtKind::Empty,
                span: self.current_span(),
            });
        }

        match self.peek_token() {
            Token::Let | Token::Const => self.variable_declaration(false),
            Token::Function => self.function_declaration(false),
            Token::Class => self.class_declaration(),
            Token::Interface => self.interface_declaration(),
            Token::If => self.if_statement(),
            Token::While => self.while_statement(),
            Token::For => self.for_statement(),
            Token::Return => self.return_statement(),
            Token::Break => self.break_statement(),
            Token::Continue => self.continue_statement(),
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

        self.consume_semicolon()?;

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

    fn class_declaration(&mut self) -> Result<Statement, CompileError> {
        let start_span = self.current_span();
        self.advance(); // consume 'class'

        let name = self.expect_identifier("Expected class name")?;

        let parent = if self.match_token(&Token::Extends) {
            Some(self.expect_identifier("Expected parent class name")?)
        } else {
            None
        };

        self.expect(&Token::LeftBrace, "Expected '{' before class body")?;

        let mut fields = Vec::new();
        let mut constructor = None;
        let mut methods = Vec::new();

        while !self.check(&Token::RightBrace) && !self.is_at_end() {
            let member_span = self.current_span();

            // Constructor
            if self.check(&Token::Constructor) {
                self.advance(); // consume 'constructor'
                self.expect(&Token::LeftParen, "Expected '(' after 'constructor'")?;
                let params = self.parameter_list()?;
                self.expect(
                    &Token::RightParen,
                    "Expected ')' after constructor parameters",
                )?;
                self.expect(&Token::LeftBrace, "Expected '{' before constructor body")?;
                let body = self.block_body()?;
                self.consume_semicolon()?;
                constructor = Some(ClassConstructor {
                    params,
                    body,
                    span: self.span_from(&member_span),
                });
                continue;
            }

            // Method or field: starts with an identifier
            let member_name = self.expect_identifier("Expected class member name")?;

            if self.check(&Token::LeftParen) {
                // Method: name(params): ReturnType { body }
                self.advance(); // consume '('
                let params = self.parameter_list()?;
                self.expect(&Token::RightParen, "Expected ')' after method parameters")?;
                let return_type = if self.match_token(&Token::Colon) {
                    Some(self.type_annotation()?)
                } else {
                    None
                };
                self.expect(&Token::LeftBrace, "Expected '{' before method body")?;
                let body = self.block_body()?;
                self.consume_semicolon()?;
                methods.push(ClassMethod {
                    name: member_name,
                    params,
                    return_type,
                    body,
                    span: self.span_from(&member_span),
                });
            } else {
                // Field: name: Type
                let type_ann = if self.match_token(&Token::Colon) {
                    Some(self.type_annotation()?)
                } else {
                    None
                };
                self.consume_semicolon()?;
                fields.push(ClassField {
                    name: member_name,
                    type_ann,
                    span: self.span_from(&member_span),
                });
            }
        }

        self.expect(&Token::RightBrace, "Expected '}' after class body")?;
        self.consume_semicolon()?;

        Ok(Statement {
            kind: StmtKind::ClassDecl {
                name,
                parent,
                fields,
                constructor,
                methods,
            },
            span: self.span_from(&start_span),
        })
    }

    fn interface_declaration(&mut self) -> Result<Statement, CompileError> {
        let start_span = self.current_span();
        self.advance(); // consume 'interface'

        let name = self.expect_identifier("Expected interface name")?;

        self.expect(&Token::LeftBrace, "Expected '{' before interface body")?;

        let mut fields = Vec::new();
        while !self.check(&Token::RightBrace) && !self.is_at_end() {
            let field_name = self.expect_identifier("Expected field name")?;
            self.expect(&Token::Colon, "Expected ':' after field name")?;
            let type_ann = self.type_annotation()?;
            // Allow semicolons or commas as separators, or just newlines
            self.match_token(&Token::Semicolon);
            self.match_token(&Token::Comma);
            fields.push((field_name, type_ann));
        }

        self.expect(&Token::RightBrace, "Expected '}' after interface body")?;
        self.consume_semicolon()?;

        Ok(Statement {
            kind: StmtKind::InterfaceDecl { name, fields },
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

    fn break_statement(&mut self) -> Result<Statement, CompileError> {
        let start_span = self.current_span();
        self.advance(); // consume 'break'
        self.consume_semicolon()?;
        Ok(Statement {
            kind: StmtKind::Break,
            span: self.span_from(&start_span),
        })
    }

    fn continue_statement(&mut self) -> Result<Statement, CompileError> {
        let start_span = self.current_span();
        self.advance(); // consume 'continue'
        self.consume_semicolon()?;
        Ok(Statement {
            kind: StmtKind::Continue,
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

        // Check for function type: (params) => ReturnType
        if self.check(&Token::LeftParen) {
            if let Some(func_type) = self.try_function_type(&span)? {
                return Ok(func_type);
            }
        }

        let mut base = match self.peek_token() {
            Token::NumberType => {
                self.advance();
                TypeAnnotation {
                    kind: TypeAnnKind::Number,
                    span: self.span_from(&span),
                }
            }
            Token::StringType => {
                self.advance();
                TypeAnnotation {
                    kind: TypeAnnKind::String,
                    span: self.span_from(&span),
                }
            }
            Token::BooleanType => {
                self.advance();
                TypeAnnotation {
                    kind: TypeAnnKind::Boolean,
                    span: self.span_from(&span),
                }
            }
            Token::Void => {
                self.advance();
                TypeAnnotation {
                    kind: TypeAnnKind::Void,
                    span: self.span_from(&span),
                }
            }
            Token::Null => {
                self.advance();
                TypeAnnotation {
                    kind: TypeAnnKind::Null,
                    span: self.span_from(&span),
                }
            }
            Token::Undefined => {
                self.advance();
                TypeAnnotation {
                    kind: TypeAnnKind::Undefined,
                    span: self.span_from(&span),
                }
            }
            Token::LeftBrace => {
                // Object type: { x: number, y: string }
                self.advance();
                let mut fields = Vec::new();
                while !self.check(&Token::RightBrace) && !self.is_at_end() {
                    let field_name = self.expect_identifier("Expected field name in type")?;
                    self.expect(&Token::Colon, "Expected ':' in object type")?;
                    let field_type = self.type_annotation()?;
                    fields.push((field_name, field_type));
                    // Allow comma or semicolon as separator
                    if !self.match_token(&Token::Comma) {
                        self.match_token(&Token::Semicolon);
                    }
                }
                self.expect(&Token::RightBrace, "Expected '}' in object type")?;
                TypeAnnotation {
                    kind: TypeAnnKind::Object { fields },
                    span: self.span_from(&span),
                }
            }
            Token::Identifier(name) => {
                let name = name.clone();
                self.advance();
                TypeAnnotation {
                    kind: TypeAnnKind::Named(name),
                    span: self.span_from(&span),
                }
            }
            _ => {
                return Err(self.error("Expected type annotation"));
            }
        };

        // Check for array type suffix: Type[]
        while self.check(&Token::LeftBracket) {
            // Peek ahead to see if it's []
            if self.tokens.get(self.current + 1).map(|t| &t.token) == Some(&Token::RightBracket) {
                self.advance(); // consume [
                self.advance(); // consume ]
                base = TypeAnnotation {
                    kind: TypeAnnKind::Array(Box::new(base)),
                    span: self.span_from(&span),
                };
            } else {
                break;
            }
        }

        Ok(base)
    }

    fn try_function_type(
        &mut self,
        start_span: &Span,
    ) -> Result<Option<TypeAnnotation>, CompileError> {
        let saved = self.current;
        self.advance(); // consume '('

        // Try to parse parameter types
        let mut param_types = Vec::new();
        if !self.check(&Token::RightParen) {
            loop {
                // Could be "name: Type" or just "Type"
                if let Token::Identifier(_) = self.peek_token() {
                    let saved_inner = self.current;
                    self.advance(); // consume identifier
                    if self.match_token(&Token::Colon) {
                        // name: Type format
                        match self.type_annotation() {
                            Ok(t) => param_types.push(t),
                            Err(_) => {
                                self.current = saved;
                                return Ok(None);
                            }
                        }
                    } else {
                        // Just an identifier used as a type name — rollback
                        self.current = saved_inner;
                        match self.type_annotation() {
                            Ok(t) => param_types.push(t),
                            Err(_) => {
                                self.current = saved;
                                return Ok(None);
                            }
                        }
                    }
                } else {
                    match self.type_annotation() {
                        Ok(t) => param_types.push(t),
                        Err(_) => {
                            self.current = saved;
                            return Ok(None);
                        }
                    }
                }
                if !self.match_token(&Token::Comma) {
                    break;
                }
            }
        }

        if !self.match_token(&Token::RightParen) {
            self.current = saved;
            return Ok(None);
        }

        if !self.match_token(&Token::Arrow) {
            self.current = saved;
            return Ok(None);
        }

        let return_type = self.type_annotation()?;

        Ok(Some(TypeAnnotation {
            kind: TypeAnnKind::FunctionType {
                params: param_types,
                return_type: Box::new(return_type),
            },
            span: self.span_from(start_span),
        }))
    }

    // --- Expressions ---

    fn expression(&mut self) -> Result<Expr, CompileError> {
        self.assignment()
    }

    fn assignment(&mut self) -> Result<Expr, CompileError> {
        // Single-parameter arrow function without parentheses: x => expr
        if let Token::Identifier(name) = self.peek_token() {
            if self.peek_next_token() == Token::Arrow {
                let span = self.current_span();
                let param_name = name.clone();
                self.advance(); // consume identifier
                self.advance(); // consume =>
                let body = if self.check(&Token::LeftBrace) {
                    self.advance();
                    ArrowBody::Block(self.block_body()?)
                } else {
                    ArrowBody::Expr(Box::new(self.assignment()?))
                };
                return Ok(Expr {
                    kind: ExprKind::ArrowFunction {
                        params: vec![Parameter {
                            name: param_name,
                            type_ann: None,
                            span: span.clone(),
                        }],
                        return_type: None,
                        body,
                    },
                    span: self.span_from(&span),
                });
            }
        }

        let expr = self.ternary()?;

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
            // Member assignment: obj.prop = value or this.prop = value
            if let ExprKind::Member { object, property } = &expr.kind {
                return Ok(Expr {
                    span: Span::new(
                        expr.span.start,
                        value.span.end,
                        expr.span.line,
                        expr.span.column,
                    ),
                    kind: ExprKind::MemberAssignment {
                        object: object.clone(),
                        property: property.clone(),
                        value: Box::new(value),
                    },
                });
            }
            return Err(CompileError::error("Invalid assignment target", expr.span));
        }

        // Compound assignment: desugar x += y → x = x + y
        let compound_op = match self.peek_token() {
            Token::PlusEqual => Some(BinOp::Add),
            Token::MinusEqual => Some(BinOp::Subtract),
            Token::StarEqual => Some(BinOp::Multiply),
            Token::SlashEqual => Some(BinOp::Divide),
            Token::PercentEqual => Some(BinOp::Modulo),
            _ => None,
        };

        if let Some(op) = compound_op {
            self.advance();
            let rhs = self.assignment()?;
            if let ExprKind::Identifier(name) = &expr.kind {
                let binary = Expr {
                    span: Span::new(
                        expr.span.start,
                        rhs.span.end,
                        expr.span.line,
                        expr.span.column,
                    ),
                    kind: ExprKind::Binary {
                        left: Box::new(expr.clone()),
                        op,
                        right: Box::new(rhs),
                    },
                };
                return Ok(Expr {
                    span: binary.span.clone(),
                    kind: ExprKind::Assignment {
                        name: name.clone(),
                        value: Box::new(binary),
                    },
                });
            }
            return Err(CompileError::error(
                "Invalid compound assignment target",
                expr.span,
            ));
        }

        Ok(expr)
    }

    fn ternary(&mut self) -> Result<Expr, CompileError> {
        let expr = self.logical_or()?;

        if self.match_token(&Token::Question) {
            let consequent = self.assignment()?;
            self.expect(&Token::Colon, "Expected ':' in ternary expression")?;
            let alternate = self.assignment()?;
            return Ok(Expr {
                span: Span::new(
                    expr.span.start,
                    alternate.span.end,
                    expr.span.line,
                    expr.span.column,
                ),
                kind: ExprKind::Conditional {
                    condition: Box::new(expr),
                    consequent: Box::new(consequent),
                    alternate: Box::new(alternate),
                },
            });
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
        let mut left = self.exponentiation()?;
        loop {
            let op = match self.peek_token() {
                Token::Star => BinOp::Multiply,
                Token::Slash => BinOp::Divide,
                Token::Percent => BinOp::Modulo,
                _ => break,
            };
            self.advance();
            let right = self.exponentiation()?;
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

    fn exponentiation(&mut self) -> Result<Expr, CompileError> {
        let base = self.unary()?;

        if self.match_token(&Token::StarStar) {
            // Right-associative: 2 ** 3 ** 2 = 2 ** (3 ** 2)
            let exp = self.exponentiation()?;
            return Ok(Expr {
                span: Span::new(
                    base.span.start,
                    exp.span.end,
                    base.span.line,
                    base.span.column,
                ),
                kind: ExprKind::Binary {
                    left: Box::new(base),
                    op: BinOp::Power,
                    right: Box::new(exp),
                },
            });
        }

        Ok(base)
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
            } else if self.match_token(&Token::LeftBracket) {
                let index = self.expression()?;
                self.expect(&Token::RightBracket, "Expected ']' after index")?;
                expr = Expr {
                    span: Span::new(
                        expr.span.start,
                        self.previous_span().end,
                        expr.span.line,
                        expr.span.column,
                    ),
                    kind: ExprKind::IndexAccess {
                        object: Box::new(expr.clone()),
                        index: Box::new(index),
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
            Token::This => {
                self.advance();
                Ok(Expr {
                    kind: ExprKind::This,
                    span: self.span_from(&span),
                })
            }
            Token::New => {
                self.advance();
                let class_name = self.expect_identifier("Expected class name after 'new'")?;
                self.expect(&Token::LeftParen, "Expected '(' after class name")?;
                let mut args = Vec::new();
                if !self.check(&Token::RightParen) {
                    loop {
                        args.push(self.expression()?);
                        if !self.match_token(&Token::Comma) {
                            break;
                        }
                    }
                }
                self.expect(
                    &Token::RightParen,
                    "Expected ')' after constructor arguments",
                )?;
                Ok(Expr {
                    kind: ExprKind::NewExpr { class_name, args },
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
            Token::LeftBracket => {
                self.advance();
                let mut elements = Vec::new();
                if !self.check(&Token::RightBracket) {
                    loop {
                        elements.push(self.expression()?);
                        if !self.match_token(&Token::Comma) {
                            break;
                        }
                    }
                }
                self.expect(&Token::RightBracket, "Expected ']' after array elements")?;
                Ok(Expr {
                    kind: ExprKind::ArrayLiteral { elements },
                    span: self.span_from(&span),
                })
            }
            Token::LeftBrace => {
                // Object literal: { key: value, ... }
                self.advance();
                let mut properties = Vec::new();
                while !self.check(&Token::RightBrace) && !self.is_at_end() {
                    let prop_span = self.current_span();
                    let key = self.expect_identifier("Expected property name")?;

                    if self.check(&Token::LeftParen) {
                        // Method shorthand: name(params) { body }
                        self.advance(); // consume '('
                        let params = self.parameter_list()?;
                        self.expect(&Token::RightParen, "Expected ')'")?;
                        let return_type = if self.match_token(&Token::Colon) {
                            Some(self.type_annotation()?)
                        } else {
                            None
                        };
                        self.expect(&Token::LeftBrace, "Expected '{' before method body")?;
                        let body_stmts = self.block_body()?;
                        // Build a block expression for the method body — but we actually
                        // store it as an arrow function for codegen simplicity
                        let value = Expr {
                            kind: ExprKind::ArrowFunction {
                                params: params.clone(),
                                return_type: return_type.clone(),
                                body: ArrowBody::Block(body_stmts),
                            },
                            span: self.span_from(&prop_span),
                        };
                        properties.push(ObjectProperty {
                            key,
                            value,
                            is_method: true,
                            params,
                            return_type,
                            span: self.span_from(&prop_span),
                        });
                    } else {
                        // Regular property: key: value
                        self.expect(&Token::Colon, "Expected ':' after property name")?;
                        let value = self.expression()?;
                        properties.push(ObjectProperty {
                            key,
                            value,
                            is_method: false,
                            params: Vec::new(),
                            return_type: None,
                            span: self.span_from(&prop_span),
                        });
                    }

                    if !self.match_token(&Token::Comma) {
                        break;
                    }
                }
                self.expect(&Token::RightBrace, "Expected '}' after object literal")?;
                Ok(Expr {
                    kind: ExprKind::ObjectLiteral { properties },
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

    fn peek_next_token(&self) -> Token {
        self.tokens
            .get(self.current + 1)
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
