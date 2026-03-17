use crate::diagnostics::CompileError;
use crate::lexer::token::{Span, SpannedToken, Token};
use crate::parser::ast::*;

pub struct Parser {
    tokens: Vec<SpannedToken>,
    current: usize,
    /// Depth of async function nesting — await is only valid when > 0
    async_depth: usize,
}

impl Parser {
    pub fn new(tokens: Vec<SpannedToken>) -> Self {
        Self {
            tokens,
            current: 0,
            async_depth: 0,
        }
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
            Token::Let | Token::Const | Token::Var => self.variable_declaration(false),
            Token::Function => self.function_declaration(false, false),
            Token::Async => self.async_declaration(false),
            Token::Class => self.class_declaration(),
            Token::Interface => self.interface_declaration(),
            Token::Enum => self.enum_declaration(),
            Token::If => self.if_statement(),
            Token::While => self.while_statement(),
            Token::Do => self.do_while_statement(),
            Token::For => self.for_statement(),
            Token::Switch => self.switch_statement(),
            Token::Return => self.return_statement(),
            Token::Break => self.break_statement(),
            Token::Continue => self.continue_statement(),
            Token::LeftBrace => self.block_statement(),
            Token::Import => self.import_declaration(),
            Token::Export => self.export_declaration(),
            Token::Throw => self.throw_statement(),
            Token::Try => self.try_statement(),
            // type alias: type X = Y (contextual keyword)
            Token::Identifier(ref name) if name == "type" => {
                // Peek ahead: if followed by an identifier, it's a type alias
                if matches!(
                    self.tokens.get(self.current + 1).map(|t| &t.token),
                    Some(Token::Identifier(_))
                ) {
                    self.type_alias_declaration()
                } else {
                    self.expression_statement()
                }
            }
            // Check for labeled statement: identifier ':' (not a type alias since we checked that above)
            Token::Identifier(ref name)
                if !matches!(name.as_str(), "type")
                    && matches!(
                        self.tokens.get(self.current + 1).map(|t| &t.token),
                        Some(Token::Colon)
                    )
                    && matches!(
                        self.tokens.get(self.current + 2).map(|t| &t.token),
                        Some(Token::For) | Some(Token::While) | Some(Token::Do)
                    ) =>
            {
                let label = self.expect_identifier("Expected label")?;
                self.advance(); // consume ':'
                let mut inner = self.statement()?;
                // Attach label to the inner loop statement
                // We use a wrapper approach: store the label in the for/while/do-while
                // by wrapping in a Block with a special label convention
                // Simpler approach: wrap in a LabeledStatement
                inner = Statement {
                    span: inner.span.clone(),
                    kind: StmtKind::Labeled {
                        label,
                        body: Box::new(inner),
                    },
                };
                Ok(inner)
            }
            _ => self.expression_statement(),
        }
    }

    fn variable_declaration(&mut self, is_exported: bool) -> Result<Statement, CompileError> {
        let start_span = self.current_span();
        let is_const = matches!(self.peek_token(), Token::Const);
        self.advance(); // consume let/const/var

        // Array destructuring: let [a, b, ...] = expr
        if self.match_token(&Token::LeftBracket) {
            let mut names = Vec::new();
            while !self.check(&Token::RightBracket) && !self.is_at_end() {
                names.push(self.expect_identifier("Expected identifier in array destructuring")?);
                if !self.match_token(&Token::Comma) {
                    break;
                }
            }
            self.expect(
                &Token::RightBracket,
                "Expected ']' after destructuring pattern",
            )?;
            self.expect(&Token::Assign, "Expected '=' in destructuring declaration")?;
            let initializer = self.expression()?;
            self.consume_semicolon()?;
            return Ok(Statement {
                kind: StmtKind::ArrayDestructure {
                    names,
                    initializer,
                    is_const,
                },
                span: self.span_from(&start_span),
            });
        }

        // Object destructuring: let { x, y } = expr  or  let { x: localX } = expr
        if self.match_token(&Token::LeftBrace) {
            let mut names = Vec::new();
            while !self.check(&Token::RightBrace) && !self.is_at_end() {
                let key =
                    self.expect_identifier("Expected property name in object destructuring")?;
                // Optional renaming: { key: localName }
                let local = if self.match_token(&Token::Colon) {
                    self.expect_identifier("Expected local name after ':' in destructuring")?
                } else {
                    key.clone()
                };
                names.push((local, key));
                if !self.match_token(&Token::Comma) {
                    break;
                }
            }
            self.expect(
                &Token::RightBrace,
                "Expected '}' after destructuring pattern",
            )?;
            self.expect(&Token::Assign, "Expected '=' in destructuring declaration")?;
            let initializer = self.expression()?;
            self.consume_semicolon()?;
            return Ok(Statement {
                kind: StmtKind::ObjectDestructure {
                    names,
                    initializer,
                    is_const,
                },
                span: self.span_from(&start_span),
            });
        }

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

    fn function_declaration(
        &mut self,
        is_exported: bool,
        is_async: bool,
    ) -> Result<Statement, CompileError> {
        let start_span = self.current_span();
        self.advance();

        let name = self.expect_identifier("Expected function name")?;

        // Parse optional type parameters: <T>, <T, U>, <T extends Type>
        let type_params = if self.check(&Token::Less) {
            self.parse_type_params()?
        } else {
            Vec::new()
        };

        self.expect(&Token::LeftParen, "Expected '(' after function name")?;
        let params = self.parameter_list()?;
        self.expect(&Token::RightParen, "Expected ')' after parameters")?;

        let return_type = if self.match_token(&Token::Colon) {
            Some(self.type_annotation()?)
        } else {
            None
        };

        self.expect(&Token::LeftBrace, "Expected '{' before function body")?;
        if is_async {
            self.async_depth += 1;
        }
        let body = self.block_body()?;
        if is_async {
            self.async_depth -= 1;
        }

        self.consume_semicolon()?;

        Ok(Statement {
            kind: StmtKind::FunctionDecl {
                name,
                type_params,
                params,
                return_type,
                body,
                is_exported,
                is_async,
            },
            span: self.span_from(&start_span),
        })
    }

    /// Parse `async function f()` or `async () =>` as a statement.
    fn async_declaration(&mut self, is_exported: bool) -> Result<Statement, CompileError> {
        let start_span = self.current_span();
        self.advance(); // consume 'async'
        match self.peek_token() {
            Token::Function => self.function_declaration(is_exported, true),
            _ => {
                // async arrow function used as a statement (unusual but valid)
                // fall back to expression statement
                let expr = self.finish_async_arrow(start_span.clone())?;
                self.consume_semicolon()?;
                Ok(Statement {
                    kind: StmtKind::Expression { expr },
                    span: self.span_from(&start_span),
                })
            }
        }
    }

    /// Finish parsing `async (params) => body` after 'async' has been consumed.
    fn finish_async_arrow(&mut self, start_span: Span) -> Result<Expr, CompileError> {
        self.expect(&Token::LeftParen, "Expected '(' after 'async'")?;
        let params = self.parameter_list()?;
        self.expect(
            &Token::RightParen,
            "Expected ')' after async arrow parameters",
        )?;
        let return_type = if self.match_token(&Token::Colon) {
            Some(self.type_annotation()?)
        } else {
            None
        };
        self.expect(&Token::Arrow, "Expected '=>' in async arrow function")?;
        self.async_depth += 1;
        let body = if self.check(&Token::LeftBrace) {
            self.advance();
            ArrowBody::Block(self.block_body()?)
        } else {
            ArrowBody::Expr(Box::new(self.assignment()?))
        };
        self.async_depth -= 1;
        Ok(Expr {
            kind: ExprKind::ArrowFunction {
                params,
                return_type,
                body,
                is_async: true,
            },
            span: self.span_from(&start_span),
        })
    }

    fn throw_statement(&mut self) -> Result<Statement, CompileError> {
        let start_span = self.current_span();
        self.advance(); // consume 'throw'
        let value = self.expression()?;
        self.consume_semicolon()?;
        Ok(Statement {
            kind: StmtKind::Throw { value },
            span: self.span_from(&start_span),
        })
    }

    fn try_statement(&mut self) -> Result<Statement, CompileError> {
        let start_span = self.current_span();
        self.advance(); // consume 'try'
        self.expect(&Token::LeftBrace, "Expected '{' after 'try'")?;
        let body = self.block_body()?;

        let (catch_binding, catch_body) = if self.check(&Token::Catch) {
            self.advance(); // consume 'catch'
            let binding = if self.match_token(&Token::LeftParen) {
                let name = self.expect_identifier("Expected catch binding name")?;
                // optional type annotation on catch binding (e.g. catch (e: unknown))
                if self.match_token(&Token::Colon) {
                    self.type_annotation()?; // discard
                }
                self.expect(&Token::RightParen, "Expected ')' after catch binding")?;
                Some(name)
            } else {
                None
            };
            self.expect(&Token::LeftBrace, "Expected '{' after 'catch'")?;
            let cb = self.block_body()?;
            (binding, Some(cb))
        } else {
            (None, None)
        };

        let finally_body = if self.check(&Token::Finally) {
            self.advance(); // consume 'finally'
            self.expect(&Token::LeftBrace, "Expected '{' after 'finally'")?;
            Some(self.block_body()?)
        } else {
            None
        };

        if catch_body.is_none() && finally_body.is_none() {
            return Err(self.error("try statement must have a catch or finally clause"));
        }

        Ok(Statement {
            kind: StmtKind::TryCatch {
                body,
                catch_binding,
                catch_body,
                finally_body,
            },
            span: self.span_from(&start_span),
        })
    }

    /// Parse type parameters: `<T>`, `<T, U>`, `<T extends Type>`
    fn parse_type_params(&mut self) -> Result<Vec<TypeParam>, CompileError> {
        self.expect(&Token::Less, "Expected '<'")?;
        let mut type_params = Vec::new();

        loop {
            let span = self.current_span();
            let name = self.expect_identifier("Expected type parameter name")?;

            // Check for constraint: T extends Type
            let constraint = if self.check(&Token::Extends) {
                self.advance(); // consume "extends"
                Some(self.type_annotation()?)
            } else {
                None
            };

            type_params.push(TypeParam {
                name,
                constraint,
                span: self.span_from(&span),
            });

            if !self.match_token(&Token::Comma) {
                break;
            }
        }

        self.expect(&Token::Greater, "Expected '>' after type parameters")?;
        Ok(type_params)
    }

    /// Parse type arguments: `<number>`, `<string, number>`
    fn parse_type_args(&mut self) -> Result<Vec<TypeAnnotation>, CompileError> {
        self.expect(&Token::Less, "Expected '<'")?;
        let mut type_args = Vec::new();
        loop {
            type_args.push(self.type_annotation()?);
            if !self.match_token(&Token::Comma) {
                break;
            }
        }
        self.expect(&Token::Greater, "Expected '>' after type arguments")?;
        Ok(type_args)
    }

    fn class_declaration(&mut self) -> Result<Statement, CompileError> {
        let start_span = self.current_span();
        self.advance(); // consume 'class'

        let name = self.expect_identifier("Expected class name")?;

        // Optional generic type parameters: class Foo<T, U extends Bar>
        let type_params = if self.check(&Token::Less) {
            self.parse_type_params()?
        } else {
            vec![]
        };

        let (parent, parent_type_args) = if self.match_token(&Token::Extends) {
            let parent_name = self.expect_identifier("Expected parent class name")?;
            let type_args = if self.check(&Token::Less) {
                self.parse_type_args()?
            } else {
                vec![]
            };
            (Some(parent_name), type_args)
        } else {
            (None, vec![])
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

            // Skip access modifiers: public, private, protected, static, readonly, abstract, override
            while let Token::Identifier(ref kw) = self.peek_token() {
                if matches!(
                    kw.as_str(),
                    "public"
                        | "private"
                        | "protected"
                        | "static"
                        | "readonly"
                        | "abstract"
                        | "override"
                ) {
                    self.advance();
                } else {
                    break;
                }
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
                // Field: name: Type = initializer
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
                fields.push(ClassField {
                    name: member_name,
                    type_ann,
                    initializer,
                    span: self.span_from(&member_span),
                });
            }
        }

        self.expect(&Token::RightBrace, "Expected '}' after class body")?;
        self.consume_semicolon()?;

        Ok(Statement {
            kind: StmtKind::ClassDecl {
                name,
                type_params,
                parent,
                parent_type_args,
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

        // Optional: extends A, B, C
        let mut extends = Vec::new();
        if self.match_token(&Token::Extends) {
            extends.push(self.expect_identifier("Expected parent interface name")?);
            while self.match_token(&Token::Comma) {
                extends.push(self.expect_identifier("Expected parent interface name")?);
            }
        }

        self.expect(&Token::LeftBrace, "Expected '{' before interface body")?;

        let mut fields = Vec::new();
        while !self.check(&Token::RightBrace) && !self.is_at_end() {
            // Skip 'readonly' modifier (parsed and discarded)
            if let Token::Identifier(ref kw) = self.peek_token() {
                if kw == "readonly" {
                    self.advance();
                }
            }
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
            kind: StmtKind::InterfaceDecl {
                name,
                extends,
                fields,
            },
            span: self.span_from(&start_span),
        })
    }

    fn type_alias_declaration(&mut self) -> Result<Statement, CompileError> {
        let start_span = self.current_span();
        self.advance(); // consume 'type' identifier

        let name = self.expect_identifier("Expected type alias name")?;

        // Optional type parameters: type IsNumber<T> = ...
        let type_params = if self.check(&Token::Less) {
            self.parse_type_params()?
        } else {
            Vec::new()
        };

        self.expect(&Token::Assign, "Expected '=' in type alias")?;
        let type_ann = self.type_annotation()?;
        self.consume_semicolon()?;

        Ok(Statement {
            kind: StmtKind::TypeAlias {
                name,
                type_params,
                type_ann,
            },
            span: self.span_from(&start_span),
        })
    }

    fn enum_declaration(&mut self) -> Result<Statement, CompileError> {
        let start_span = self.current_span();
        self.advance(); // consume 'enum'

        let name = self.expect_identifier("Expected enum name")?;
        self.expect(&Token::LeftBrace, "Expected '{' before enum body")?;

        let mut members = Vec::new();
        while !self.check(&Token::RightBrace) && !self.is_at_end() {
            let member_span = self.current_span();
            let member_name = self.expect_identifier("Expected enum member name")?;

            let value = if self.match_token(&Token::Assign) {
                match self.peek_token() {
                    Token::String(s) => {
                        let s = s.clone();
                        self.advance();
                        Some(EnumValue::String(s))
                    }
                    Token::Number(n) => {
                        let n = n;
                        self.advance();
                        Some(EnumValue::Number(n))
                    }
                    _ => {
                        return Err(self.error("Expected string or number value for enum member"));
                    }
                }
            } else {
                None
            };

            members.push(EnumMember {
                name: member_name,
                value,
                span: self.span_from(&member_span),
            });

            // Allow trailing comma
            self.match_token(&Token::Comma);
        }

        self.expect(&Token::RightBrace, "Expected '}' after enum body")?;
        self.consume_semicolon()?;

        Ok(Statement {
            kind: StmtKind::EnumDecl { name, members },
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
            Token::Function => self.function_declaration(true, false),
            Token::Async => self.async_declaration(true),
            Token::Let | Token::Const | Token::Var => self.variable_declaration(true),
            _ => Err(self.error("Expected function, let, or const after 'export'")),
        }
    }

    fn parameter_list(&mut self) -> Result<Vec<Parameter>, CompileError> {
        let mut params = Vec::new();
        if !self.check(&Token::RightParen) {
            loop {
                let param_span = self.current_span();
                let is_rest = self.match_token(&Token::Ellipsis);
                let name = self.expect_identifier("Expected parameter name")?;
                let type_ann = if self.match_token(&Token::Colon) {
                    Some(self.type_annotation()?)
                } else {
                    None
                };
                let default = if !is_rest && self.match_token(&Token::Assign) {
                    Some(self.expression()?)
                } else {
                    None
                };
                params.push(Parameter {
                    name,
                    type_ann,
                    default,
                    is_rest,
                    span: self.span_from(&param_span),
                });
                // Rest param must be last; stop parsing params after it
                if is_rest || !self.match_token(&Token::Comma) {
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

        let then_branch = if self.check(&Token::LeftBrace) {
            self.advance();
            self.block_body()?
        } else {
            // Single-statement if body (no braces)
            vec![self.statement()?]
        };

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

    fn do_while_statement(&mut self) -> Result<Statement, CompileError> {
        let start_span = self.current_span();
        self.advance(); // consume 'do'

        self.expect(&Token::LeftBrace, "Expected '{' after 'do'")?;
        let body = self.block_body()?;

        self.expect(&Token::While, "Expected 'while' after do block")?;
        self.expect(&Token::LeftParen, "Expected '(' after 'while'")?;
        let condition = self.expression()?;
        self.expect(&Token::RightParen, "Expected ')' after condition")?;
        self.consume_semicolon()?;

        Ok(Statement {
            kind: StmtKind::DoWhile { body, condition },
            span: self.span_from(&start_span),
        })
    }

    fn for_statement(&mut self) -> Result<Statement, CompileError> {
        let start_span = self.current_span();
        self.advance();

        self.expect(&Token::LeftParen, "Expected '(' after 'for'")?;

        // Detect `for (let/const/var x of iterable)` or `for (let/const/var x in obj)`
        if matches!(self.peek_token(), Token::Let | Token::Const | Token::Var) {
            // Peek: tokens[current] = let/const, tokens[current+1] = ident, tokens[current+2] = of/in
            let is_for_of = matches!(
                self.tokens.get(self.current + 1).map(|t| &t.token),
                Some(Token::Identifier(_))
            ) && matches!(
                self.tokens.get(self.current + 2).map(|t| &t.token),
                Some(Token::Of)
            );
            let is_for_in = matches!(
                self.tokens.get(self.current + 1).map(|t| &t.token),
                Some(Token::Identifier(_))
            ) && matches!(
                self.tokens.get(self.current + 2).map(|t| &t.token),
                Some(Token::In)
            );
            if is_for_of {
                let is_const = matches!(self.peek_token(), Token::Const);
                self.advance(); // consume let/const/var
                let var_name = self.expect_identifier("Expected variable name in for-of")?;
                self.advance(); // consume 'of'
                let iterable = self.expression()?;
                self.expect(&Token::RightParen, "Expected ')' after for-of iterable")?;
                self.expect(&Token::LeftBrace, "Expected '{' after for-of header")?;
                let body = self.block_body()?;
                return Ok(Statement {
                    kind: StmtKind::ForOf {
                        var_name,
                        is_const,
                        iterable,
                        body,
                    },
                    span: self.span_from(&start_span),
                });
            }
            if is_for_in {
                self.advance(); // consume let/const/var
                let var_name = self.expect_identifier("Expected variable name in for-in")?;
                self.advance(); // consume 'in'
                let object = self.expression()?;
                self.expect(&Token::RightParen, "Expected ')' after for-in object")?;
                self.expect(&Token::LeftBrace, "Expected '{' after for-in header")?;
                let body = self.block_body()?;
                return Ok(Statement {
                    kind: StmtKind::ForIn {
                        var_name,
                        object,
                        body,
                    },
                    span: self.span_from(&start_span),
                });
            }
        }

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

    fn switch_statement(&mut self) -> Result<Statement, CompileError> {
        let start_span = self.current_span();
        self.advance(); // consume 'switch'

        self.expect(&Token::LeftParen, "Expected '(' after 'switch'")?;
        let discriminant = self.expression()?;
        self.expect(&Token::RightParen, "Expected ')' after switch expression")?;
        self.expect(&Token::LeftBrace, "Expected '{' after switch expression")?;

        let mut cases = Vec::new();
        while !self.check(&Token::RightBrace) && !self.is_at_end() {
            let test = if self.match_token(&Token::Case) {
                let expr = self.expression()?;
                self.expect(&Token::Colon, "Expected ':' after case value")?;
                Some(expr)
            } else if self.match_token(&Token::Default) {
                self.expect(&Token::Colon, "Expected ':' after 'default'")?;
                None
            } else {
                return Err(self.error("Expected 'case' or 'default' in switch body"));
            };

            let mut body = Vec::new();
            while !matches!(
                self.peek_token(),
                Token::Case | Token::Default | Token::RightBrace | Token::Eof
            ) {
                body.push(self.statement()?);
            }
            cases.push(SwitchCase { test, body });
        }

        self.expect(&Token::RightBrace, "Expected '}' after switch body")?;
        self.consume_semicolon()?;

        Ok(Statement {
            kind: StmtKind::Switch {
                discriminant,
                cases,
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
        let label = if let Token::Identifier(_) = self.peek_token() {
            Some(self.expect_identifier("Expected label")?)
        } else {
            None
        };
        self.consume_semicolon()?;
        Ok(Statement {
            kind: StmtKind::Break { label },
            span: self.span_from(&start_span),
        })
    }

    fn continue_statement(&mut self) -> Result<Statement, CompileError> {
        let start_span = self.current_span();
        self.advance(); // consume 'continue'
        let label = if let Token::Identifier(_) = self.peek_token() {
            Some(self.expect_identifier("Expected label")?)
        } else {
            None
        };
        self.consume_semicolon()?;
        Ok(Statement {
            kind: StmtKind::Continue { label },
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

        // Consume optional leading pipe: `type T = | A | B` is valid TypeScript
        self.match_token(&Token::Pipe);

        // Parse the first type (handles typeof, keyof, function types,
        // primitives, identifiers, array suffix, and intersection &)
        let base = self.type_annotation_base()?;

        // Check for conditional type: CheckType extends ExtendsType ? TrueType : FalseType
        if self.check(&Token::Extends) {
            self.advance(); // consume 'extends'
            let extends_type = self.type_annotation_base()?;
            self.expect(&Token::Question, "Expected '?' in conditional type")?;
            let true_type = self.type_annotation()?;
            self.expect(&Token::Colon, "Expected ':' in conditional type")?;
            let false_type = self.type_annotation()?;
            return Ok(TypeAnnotation {
                kind: TypeAnnKind::Conditional {
                    check_type: Box::new(base),
                    extends_type: Box::new(extends_type),
                    true_type: Box::new(true_type),
                    false_type: Box::new(false_type),
                },
                span: self.span_from(&span),
            });
        }

        // Type predicate: `param is Type`
        // The base is the parameter name as a Named type annotation.
        // We preserve the predicate type so the checker can use it for narrowing.
        if let TypeAnnKind::Named(param_name) = &base.kind {
            let param_name = param_name.clone();
            if let Token::Identifier(ref kw) = self.peek_token() {
                if kw == "is" {
                    self.advance(); // consume 'is'
                    let predicate_type = self.type_annotation()?;
                    return Ok(TypeAnnotation {
                        kind: TypeAnnKind::TypePredicate {
                            param: param_name,
                            ty: Box::new(predicate_type),
                        },
                        span: self.span_from(&span),
                    });
                }
            }
        }

        // Check for union type: Type | Type | ...
        if self.check(&Token::Pipe) {
            let mut variants = vec![base];
            while self.match_token(&Token::Pipe) {
                variants.push(self.type_annotation_base()?);
            }
            return Ok(TypeAnnotation {
                kind: TypeAnnKind::Union(variants),
                span: self.span_from(&span),
            });
        }

        Ok(base)
    }

    /// Parse a single type (without union) — handles typeof, keyof, function types,
    /// primitives, identifiers, array suffix, and intersection &.
    fn type_annotation_base(&mut self) -> Result<TypeAnnotation, CompileError> {
        let span = self.current_span();

        // keyof Type
        if let Token::Identifier(ref kw) = self.peek_token() {
            if kw == "keyof" {
                self.advance();
                let inner = self.type_annotation()?;
                return Ok(TypeAnnotation {
                    kind: TypeAnnKind::Keyof(Box::new(inner)),
                    span: self.span_from(&span),
                });
            }
        }

        // typeof x
        if self.match_token(&Token::Typeof) {
            let name = self.expect_identifier("Expected identifier after 'typeof'")?;
            let mut base = TypeAnnotation {
                kind: TypeAnnKind::Typeof(name),
                span: self.span_from(&span),
            };
            while self.check(&Token::LeftBracket) {
                if self.tokens.get(self.current + 1).map(|t| &t.token) == Some(&Token::RightBracket)
                {
                    self.advance();
                    self.advance();
                    base = TypeAnnotation {
                        kind: TypeAnnKind::Array(Box::new(base)),
                        span: self.span_from(&span),
                    };
                } else {
                    break;
                }
            }
            return Ok(base);
        }

        // Function type
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
            Token::String(s) => {
                let s = s.clone();
                self.advance();
                TypeAnnotation {
                    kind: TypeAnnKind::StringLiteral(s),
                    span: self.span_from(&span),
                }
            }
            Token::Number(n) => {
                let n = n;
                self.advance();
                TypeAnnotation {
                    kind: TypeAnnKind::NumberLiteral(n),
                    span: self.span_from(&span),
                }
            }
            Token::True => {
                self.advance();
                TypeAnnotation {
                    kind: TypeAnnKind::BooleanLiteral(true),
                    span: self.span_from(&span),
                }
            }
            Token::False => {
                self.advance();
                TypeAnnotation {
                    kind: TypeAnnKind::BooleanLiteral(false),
                    span: self.span_from(&span),
                }
            }
            Token::LeftBrace => {
                self.advance();

                // Skip optional 'readonly' modifier
                if let Token::Identifier(ref kw) = self.peek_token() {
                    if kw == "readonly" {
                        self.advance();
                    }
                }

                // Check for mapped type: { [P in keyof T]: T[P] }
                if self.check(&Token::LeftBracket) {
                    let saved = self.current;
                    self.advance(); // consume '['
                    if let Ok(param) = self.expect_identifier("mapped type param") {
                        if self.check(&Token::In) {
                            {
                                self.advance(); // consume 'in'
                                let constraint = self.type_annotation()?;
                                self.expect(&Token::RightBracket, "Expected ']'")?;
                                self.expect(&Token::Colon, "Expected ':'")?;
                                let value_type = self.type_annotation()?;
                                self.match_token(&Token::Semicolon);
                                self.expect(&Token::RightBrace, "Expected '}'")?;
                                return Ok(TypeAnnotation {
                                    kind: TypeAnnKind::Mapped {
                                        param,
                                        constraint: Box::new(constraint),
                                        value_type: Box::new(value_type),
                                    },
                                    span: self.span_from(&span),
                                });
                            }
                        }
                    }
                    // Not a mapped type — backtrack
                    self.current = saved;
                }

                let mut fields = Vec::new();
                while !self.check(&Token::RightBrace) && !self.is_at_end() {
                    if let Token::Identifier(ref kw) = self.peek_token() {
                        if kw == "readonly" {
                            self.advance();
                        }
                    }
                    let field_name = self.expect_identifier("Expected field name in type")?;
                    self.expect(&Token::Colon, "Expected ':' in object type")?;
                    let field_type = self.type_annotation()?;
                    fields.push((field_name, field_type));
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

                // Check for type arguments: IsNumber<number>
                if self.check(&Token::Less) {
                    // Speculatively parse type args — save position
                    let saved = self.current;
                    if let Ok(type_args) = self.parse_type_args() {
                        TypeAnnotation {
                            kind: TypeAnnKind::Generic { name, type_args },
                            span: self.span_from(&span),
                        }
                    } else {
                        self.current = saved;
                        TypeAnnotation {
                            kind: TypeAnnKind::Named(name),
                            span: self.span_from(&span),
                        }
                    }
                } else {
                    TypeAnnotation {
                        kind: TypeAnnKind::Named(name),
                        span: self.span_from(&span),
                    }
                }
            }
            Token::LeftBracket => {
                // Tuple type: [number, string, ...]
                self.advance(); // consume '['
                let mut element_types = Vec::new();
                while !self.check(&Token::RightBracket) && !self.is_at_end() {
                    element_types.push(self.type_annotation()?);
                    if !self.match_token(&Token::Comma) {
                        break;
                    }
                }
                self.expect(&Token::RightBracket, "Expected ']' in tuple type")?;
                TypeAnnotation {
                    kind: TypeAnnKind::Tuple(element_types),
                    span: self.span_from(&span),
                }
            }
            _ => {
                return Err(self.error("Expected type annotation"));
            }
        };

        // Array suffix or indexed access type
        while self.check(&Token::LeftBracket) {
            if self.tokens.get(self.current + 1).map(|t| &t.token) == Some(&Token::RightBracket) {
                // Empty brackets: Type[] → array
                self.advance();
                self.advance();
                base = TypeAnnotation {
                    kind: TypeAnnKind::Array(Box::new(base)),
                    span: self.span_from(&span),
                };
            } else {
                // Non-empty brackets: T[P] → indexed access type
                self.advance(); // consume '['
                let index_type = self.type_annotation()?;
                self.expect(&Token::RightBracket, "Expected ']'")?;
                base = TypeAnnotation {
                    kind: TypeAnnKind::IndexedAccess {
                        object_type: Box::new(base),
                        index_type: Box::new(index_type),
                    },
                    span: self.span_from(&span),
                };
            }
        }

        // Intersection: Type & Type & ... (binds tighter than union |)
        if self.check(&Token::Ampersand) {
            let mut variants = vec![base];
            while self.match_token(&Token::Ampersand) {
                variants.push(self.type_annotation_atom()?);
            }
            return Ok(TypeAnnotation {
                kind: TypeAnnKind::Intersection(variants),
                span: self.span_from(&span),
            });
        }

        Ok(base)
    }

    /// Parse a single atomic type (no union, no intersection) — used by intersection parsing
    fn type_annotation_atom(&mut self) -> Result<TypeAnnotation, CompileError> {
        let span = self.current_span();

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
            Token::String(s) => {
                let s = s.clone();
                self.advance();
                TypeAnnotation {
                    kind: TypeAnnKind::StringLiteral(s),
                    span: self.span_from(&span),
                }
            }
            Token::Number(n) => {
                let n = n;
                self.advance();
                TypeAnnotation {
                    kind: TypeAnnKind::NumberLiteral(n),
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

        // Array suffix
        while self.check(&Token::LeftBracket) {
            if self.tokens.get(self.current + 1).map(|t| &t.token) == Some(&Token::RightBracket) {
                self.advance();
                self.advance();
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
                            default: None,
                            is_rest: false,
                            span: span.clone(),
                        }],
                        return_type: None,
                        body,
                        is_async: false,
                    },
                    span: self.span_from(&span),
                });
            }
        }

        let mut expr = self.ternary()?;

        // Type assertion: expr as Type  /  expr as const
        while self.check(&Token::As)
            || matches!(self.peek_token(), Token::Identifier(ref kw) if kw == "satisfies")
        {
            if self.match_token(&Token::As) {
                // as const — parse and discard (no-op at runtime)
                if self.match_token(&Token::Const) {
                    continue;
                }
                let target_type = self.type_annotation()?;
                expr = Expr {
                    span: Span::new(
                        expr.span.start,
                        target_type.span.end,
                        expr.span.line,
                        expr.span.column,
                    ),
                    kind: ExprKind::TypeAssertion {
                        expr: Box::new(expr),
                        target_type,
                    },
                };
            } else {
                // satisfies Type
                self.advance(); // consume 'satisfies' identifier
                let target_type = self.type_annotation()?;
                expr = Expr {
                    span: Span::new(
                        expr.span.start,
                        target_type.span.end,
                        expr.span.line,
                        expr.span.column,
                    ),
                    kind: ExprKind::Satisfies {
                        expr: Box::new(expr),
                        target_type,
                    },
                };
            }
        }

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
        let mut left = self.nullish_coalescing()?;
        while self.match_token(&Token::PipePipe) {
            let right = self.nullish_coalescing()?;
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

    fn nullish_coalescing(&mut self) -> Result<Expr, CompileError> {
        let mut left = self.logical_and()?;
        while self.match_token(&Token::QuestionQuestion) {
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
                    op: BinOp::NullishCoalescing,
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
            Token::Await => {
                let span = self.current_span();
                if self.async_depth == 0 {
                    return Err(CompileError::error(
                        "'await' can only be used inside an async function",
                        span,
                    ));
                }
                self.advance();
                let operand = self.unary()?;
                Ok(Expr {
                    span: self.span_from(&span),
                    kind: ExprKind::Await {
                        expr: Box::new(operand),
                    },
                })
            }
            Token::PlusPlus => {
                let span = self.current_span();
                self.advance();
                let target = self.call()?;
                Ok(Expr {
                    span: self.span_from(&span),
                    kind: ExprKind::PrefixUpdate {
                        target: Box::new(target),
                        op: UpdateOp::Increment,
                    },
                })
            }
            Token::MinusMinus => {
                let span = self.current_span();
                self.advance();
                let target = self.call()?;
                Ok(Expr {
                    span: self.span_from(&span),
                    kind: ExprKind::PrefixUpdate {
                        target: Box::new(target),
                        op: UpdateOp::Decrement,
                    },
                })
            }
            _ => self.postfix(),
        }
    }

    fn postfix(&mut self) -> Result<Expr, CompileError> {
        let mut expr = self.call()?;

        // Only consume ++/-- when the LHS is a valid lvalue
        let is_lvalue = matches!(
            expr.kind,
            ExprKind::Identifier(_) | ExprKind::IndexAccess { .. } | ExprKind::Member { .. }
        );

        if is_lvalue {
            match self.peek_token() {
                Token::PlusPlus => {
                    let start_span = expr.span.clone();
                    self.advance();
                    expr = Expr {
                        span: self.span_from(&start_span),
                        kind: ExprKind::PostfixUpdate {
                            target: Box::new(expr),
                            op: UpdateOp::Increment,
                        },
                    };
                }
                Token::MinusMinus => {
                    let start_span = expr.span.clone();
                    self.advance();
                    expr = Expr {
                        span: self.span_from(&start_span),
                        kind: ExprKind::PostfixUpdate {
                            target: Box::new(expr),
                            op: UpdateOp::Decrement,
                        },
                    };
                }
                _ => {}
            }
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
            } else if self.match_token(&Token::QuestionDot) {
                let property = self.expect_identifier("Expected property name after '?.'")?;
                expr = Expr {
                    span: Span::new(
                        expr.span.start,
                        self.previous_span().end,
                        expr.span.line,
                        expr.span.column,
                    ),
                    kind: ExprKind::OptionalMember {
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
            Token::Function => {
                // Function expression: function(params): RetType { body }
                // Desugared to an arrow function
                self.advance(); // consume 'function'
                                // Optional function name (ignored for now — anonymous or named expression)
                if let Token::Identifier(_) = self.peek_token() {
                    if !self.check(&Token::LeftParen) {
                        self.advance(); // consume optional name
                    }
                }
                self.expect(&Token::LeftParen, "Expected '(' after 'function'")?;
                let params = self.parameter_list()?;
                self.expect(&Token::RightParen, "Expected ')' after parameters")?;
                let return_type = if self.match_token(&Token::Colon) {
                    Some(self.type_annotation()?)
                } else {
                    None
                };
                self.expect(&Token::LeftBrace, "Expected '{' before function body")?;
                let body = self.block_body()?;
                Ok(Expr {
                    kind: ExprKind::ArrowFunction {
                        params,
                        return_type,
                        body: ArrowBody::Block(body),
                        is_async: false,
                    },
                    span: self.span_from(&span),
                })
            }
            Token::Async => {
                // async () => body  or  async (params) => body
                self.advance(); // consume 'async'
                self.finish_async_arrow(span)
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
                        let elem_span = self.current_span();
                        if self.match_token(&Token::Ellipsis) {
                            let spread_expr = self.expression()?;
                            elements.push(Expr {
                                span: self.span_from(&elem_span),
                                kind: ExprKind::Spread {
                                    expr: Box::new(spread_expr),
                                },
                            });
                        } else {
                            elements.push(self.expression()?);
                        }
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

                    // Spread element: { ...expr }
                    if self.match_token(&Token::Ellipsis) {
                        let spread_expr = self.expression()?;
                        properties.push(ObjectProperty {
                            key: String::new(),
                            computed_key: None,
                            value: spread_expr,
                            is_spread: true,
                            is_method: false,
                            params: Vec::new(),
                            return_type: None,
                            span: self.span_from(&prop_span),
                        });
                        self.match_token(&Token::Comma);
                        continue;
                    }

                    // Computed property key: { [expr]: value }
                    if self.match_token(&Token::LeftBracket) {
                        let key_expr = self.expression()?;
                        self.expect(&Token::RightBracket, "Expected ']' after computed key")?;
                        self.expect(&Token::Colon, "Expected ':' after computed property key")?;
                        let value = self.expression()?;
                        properties.push(ObjectProperty {
                            key: String::new(),
                            computed_key: Some(key_expr),
                            value,
                            is_spread: false,
                            is_method: false,
                            params: Vec::new(),
                            return_type: None,
                            span: self.span_from(&prop_span),
                        });
                        self.match_token(&Token::Comma);
                        continue;
                    }

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
                                is_async: false,
                            },
                            span: self.span_from(&prop_span),
                        };
                        properties.push(ObjectProperty {
                            key,
                            computed_key: None,
                            value,
                            is_method: true,
                            is_spread: false,
                            params,
                            return_type,
                            span: self.span_from(&prop_span),
                        });
                    } else if self.check(&Token::Comma) || self.check(&Token::RightBrace) {
                        // Shorthand property: { name } → { name: name }
                        let value = Expr {
                            kind: ExprKind::Identifier(key.clone()),
                            span: prop_span.clone(),
                        };
                        properties.push(ObjectProperty {
                            key,
                            computed_key: None,
                            value,
                            is_method: false,
                            is_spread: false,
                            params: Vec::new(),
                            return_type: None,
                            span: self.span_from(&prop_span),
                        });
                    } else {
                        // Regular property: key: value
                        self.expect(&Token::Colon, "Expected ':' after property name")?;
                        let value = self.expression()?;
                        properties.push(ObjectProperty {
                            key,
                            computed_key: None,
                            value,
                            is_method: false,
                            is_spread: false,
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
                                is_async: false,
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
                    default: None,
                    is_rest: false,
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
