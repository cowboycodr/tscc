use std::collections::HashMap;

use crate::diagnostics::CompileError;
use crate::lexer::token::Span;
use crate::parser::ast::*;
use crate::types::ty::Type;

#[derive(Debug, Clone)]
struct Symbol {
    ty: Type,
    is_const: bool,
}

struct Scope {
    symbols: HashMap<String, Symbol>,
}

pub struct TypeChecker {
    scopes: Vec<Scope>,
    /// Track what type the current function should return
    current_return_type: Option<Type>,
}

impl TypeChecker {
    pub fn new() -> Self {
        let mut checker = Self {
            scopes: Vec::new(),
            current_return_type: None,
        };
        checker.push_scope();

        // Register built-in: console
        checker.define(
            "console".to_string(),
            Type::Unknown, // console is a special object, handled in member access
            true,
        );

        checker
    }

    pub fn check(&mut self, program: &Program) -> Result<(), CompileError> {
        for stmt in &program.statements {
            self.check_statement(stmt)?;
        }
        Ok(())
    }

    fn check_statement(&mut self, stmt: &Statement) -> Result<(), CompileError> {
        match &stmt.kind {
            StmtKind::VariableDecl {
                name,
                is_const,
                type_ann,
                initializer,
            } => {
                let declared_type = type_ann
                    .as_ref()
                    .map(|ann| self.resolve_type_annotation(ann));

                let init_type = if let Some(init) = initializer {
                    Some(self.check_expr(init)?)
                } else {
                    None
                };

                let final_type = match (&declared_type, &init_type) {
                    (Some(decl), Some(init)) => {
                        if !self.is_assignable(init, decl) {
                            return Err(CompileError {
                                message: format!(
                                    "Type '{}' is not assignable to type '{}'",
                                    init, decl
                                ),
                                span: stmt.span.clone(),
                            });
                        }
                        decl.clone()
                    }
                    (Some(decl), None) => decl.clone(),
                    (None, Some(init)) => init.clone(),
                    (None, None) => {
                        return Err(CompileError {
                            message: "Variable must have either a type annotation or initializer"
                                .to_string(),
                            span: stmt.span.clone(),
                        });
                    }
                };

                self.define(name.clone(), final_type, *is_const);
                Ok(())
            }

            StmtKind::FunctionDecl {
                name,
                params,
                return_type,
                body,
            } => {
                let param_types: Vec<Type> = params
                    .iter()
                    .map(|p| {
                        p.type_ann
                            .as_ref()
                            .map(|ann| self.resolve_type_annotation(ann))
                            .unwrap_or(Type::Unknown)
                    })
                    .collect();

                let ret_type = return_type
                    .as_ref()
                    .map(|ann| self.resolve_type_annotation(ann))
                    .unwrap_or(Type::Void);

                let func_type = Type::Function {
                    params: param_types.clone(),
                    return_type: Box::new(ret_type.clone()),
                };

                self.define(name.clone(), func_type, true);

                // Check function body in new scope
                self.push_scope();
                let prev_return_type = self.current_return_type.clone();
                self.current_return_type = Some(ret_type.clone());

                for (param, param_type) in params.iter().zip(param_types.iter()) {
                    self.define(param.name.clone(), param_type.clone(), false);
                }

                for stmt in body {
                    self.check_statement(stmt)?;
                }

                self.current_return_type = prev_return_type;
                self.pop_scope();
                Ok(())
            }

            StmtKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                self.check_expr(condition)?;

                self.push_scope();
                for stmt in then_branch {
                    self.check_statement(stmt)?;
                }
                self.pop_scope();

                if let Some(else_stmts) = else_branch {
                    self.push_scope();
                    for stmt in else_stmts {
                        self.check_statement(stmt)?;
                    }
                    self.pop_scope();
                }

                Ok(())
            }

            StmtKind::While { condition, body } => {
                self.check_expr(condition)?;
                self.push_scope();
                for stmt in body {
                    self.check_statement(stmt)?;
                }
                self.pop_scope();
                Ok(())
            }

            StmtKind::For {
                init,
                condition,
                update,
                body,
            } => {
                self.push_scope();
                if let Some(init) = init {
                    self.check_statement(init)?;
                }
                if let Some(cond) = condition {
                    self.check_expr(cond)?;
                }
                if let Some(upd) = update {
                    self.check_expr(upd)?;
                }
                for stmt in body {
                    self.check_statement(stmt)?;
                }
                self.pop_scope();
                Ok(())
            }

            StmtKind::Return { value } => {
                let ret_type = if let Some(val) = value {
                    self.check_expr(val)?
                } else {
                    Type::Void
                };

                if let Some(expected) = &self.current_return_type {
                    if !self.is_assignable(&ret_type, expected) {
                        return Err(CompileError {
                            message: format!(
                                "Type '{}' is not assignable to return type '{}'",
                                ret_type, expected
                            ),
                            span: stmt.span.clone(),
                        });
                    }
                }

                Ok(())
            }

            StmtKind::Expression { expr } => {
                self.check_expr(expr)?;
                Ok(())
            }

            StmtKind::Block { statements } => {
                self.push_scope();
                for stmt in statements {
                    self.check_statement(stmt)?;
                }
                self.pop_scope();
                Ok(())
            }
        }
    }

    fn check_expr(&mut self, expr: &Expr) -> Result<Type, CompileError> {
        match &expr.kind {
            ExprKind::NumberLiteral(_) => Ok(Type::Number),
            ExprKind::StringLiteral(_) => Ok(Type::String),
            ExprKind::BooleanLiteral(_) => Ok(Type::Boolean),
            ExprKind::NullLiteral => Ok(Type::Null),
            ExprKind::UndefinedLiteral => Ok(Type::Undefined),

            ExprKind::Identifier(name) => self.lookup(name).ok_or_else(|| CompileError {
                message: format!("Cannot find name '{}'", name),
                span: expr.span.clone(),
            }),

            ExprKind::Binary { left, op, right } => {
                let left_type = self.check_expr(left)?;
                let right_type = self.check_expr(right)?;
                self.check_binary_op(&left_type, *op, &right_type, &expr.span)
            }

            ExprKind::Unary { op, operand } => {
                let operand_type = self.check_expr(operand)?;
                match op {
                    UnaryOp::Negate => {
                        if operand_type != Type::Number {
                            return Err(CompileError {
                                message: format!(
                                    "Unary '-' requires number, got '{}'",
                                    operand_type
                                ),
                                span: expr.span.clone(),
                            });
                        }
                        Ok(Type::Number)
                    }
                    UnaryOp::Not => Ok(Type::Boolean),
                }
            }

            ExprKind::Call { callee, args } => {
                // Special case: console.log
                if let ExprKind::Member { object, property } = &callee.kind {
                    if let ExprKind::Identifier(name) = &object.kind {
                        if name == "console" && property == "log" {
                            // console.log accepts any types
                            for arg in args {
                                self.check_expr(arg)?;
                            }
                            return Ok(Type::Void);
                        }
                    }
                }

                let callee_type = self.check_expr(callee)?;
                match &callee_type {
                    Type::Function {
                        params,
                        return_type,
                    } => {
                        if args.len() != params.len() {
                            return Err(CompileError {
                                message: format!(
                                    "Expected {} arguments, got {}",
                                    params.len(),
                                    args.len()
                                ),
                                span: expr.span.clone(),
                            });
                        }
                        for (arg, param_type) in args.iter().zip(params.iter()) {
                            let arg_type = self.check_expr(arg)?;
                            if !self.is_assignable(&arg_type, param_type) {
                                return Err(CompileError {
                                    message: format!(
                                        "Argument of type '{}' is not assignable to parameter of type '{}'",
                                        arg_type, param_type
                                    ),
                                    span: arg.span.clone(),
                                });
                            }
                        }
                        Ok(*return_type.clone())
                    }
                    _ => Err(CompileError {
                        message: format!("Type '{}' is not callable", callee_type),
                        span: callee.span.clone(),
                    }),
                }
            }

            ExprKind::Member { object, property } => {
                let obj_type = self.check_expr(object)?;
                // For now, only handle console.log
                if let ExprKind::Identifier(name) = &object.kind {
                    if name == "console" && property == "log" {
                        return Ok(Type::Function {
                            params: vec![], // variadic, handled in Call
                            return_type: Box::new(Type::Void),
                        });
                    }
                }
                Err(CompileError {
                    message: format!(
                        "Property '{}' does not exist on type '{}'",
                        property, obj_type
                    ),
                    span: expr.span.clone(),
                })
            }

            ExprKind::Assignment { name, value } => {
                // Check that the variable exists and is not const
                let sym = self.lookup_symbol(name).ok_or_else(|| CompileError {
                    message: format!("Cannot find name '{}'", name),
                    span: expr.span.clone(),
                })?;

                if sym.is_const {
                    return Err(CompileError {
                        message: format!("Cannot assign to '{}' because it is a constant", name),
                        span: expr.span.clone(),
                    });
                }

                let var_type = sym.ty.clone();
                let val_type = self.check_expr(value)?;

                if !self.is_assignable(&val_type, &var_type) {
                    return Err(CompileError {
                        message: format!(
                            "Type '{}' is not assignable to type '{}'",
                            val_type, var_type
                        ),
                        span: expr.span.clone(),
                    });
                }

                Ok(var_type)
            }

            ExprKind::ArrowFunction {
                params,
                return_type,
                body,
            } => {
                let param_types: Vec<Type> = params
                    .iter()
                    .map(|p| {
                        p.type_ann
                            .as_ref()
                            .map(|ann| self.resolve_type_annotation(ann))
                            .unwrap_or(Type::Unknown)
                    })
                    .collect();

                let ret_type = return_type
                    .as_ref()
                    .map(|ann| self.resolve_type_annotation(ann))
                    .unwrap_or(Type::Unknown);

                self.push_scope();
                let prev_return_type = self.current_return_type.clone();
                self.current_return_type = Some(ret_type.clone());

                for (param, ptype) in params.iter().zip(param_types.iter()) {
                    self.define(param.name.clone(), ptype.clone(), false);
                }

                let actual_ret_type = match body {
                    ArrowBody::Expr(e) => self.check_expr(e)?,
                    ArrowBody::Block(stmts) => {
                        for s in stmts {
                            self.check_statement(s)?;
                        }
                        ret_type.clone()
                    }
                };

                self.current_return_type = prev_return_type;
                self.pop_scope();

                let final_ret = if ret_type == Type::Unknown {
                    actual_ret_type
                } else {
                    ret_type
                };

                Ok(Type::Function {
                    params: param_types,
                    return_type: Box::new(final_ret),
                })
            }

            ExprKind::Grouping { expr } => self.check_expr(expr),

            ExprKind::PostfixUpdate { name, .. } | ExprKind::PrefixUpdate { name, .. } => {
                let ty = self.lookup(name).ok_or_else(|| CompileError {
                    message: format!("Cannot find name '{}'", name),
                    span: expr.span.clone(),
                })?;
                if ty != Type::Number {
                    return Err(CompileError {
                        message: format!("Operator '++/--' requires number, got '{}'", ty),
                        span: expr.span.clone(),
                    });
                }
                Ok(Type::Number)
            }
        }
    }

    fn check_binary_op(
        &self,
        left: &Type,
        op: BinOp,
        right: &Type,
        span: &Span,
    ) -> Result<Type, CompileError> {
        match op {
            BinOp::Add => {
                if left == &Type::Number && right == &Type::Number {
                    Ok(Type::Number)
                } else if left == &Type::String || right == &Type::String {
                    // String concatenation: at least one side must be string
                    Ok(Type::String)
                } else {
                    Err(CompileError {
                        message: format!(
                            "Operator '+' cannot be applied to types '{}' and '{}'",
                            left, right
                        ),
                        span: span.clone(),
                    })
                }
            }
            BinOp::Subtract | BinOp::Multiply | BinOp::Divide | BinOp::Modulo => {
                if left == &Type::Number && right == &Type::Number {
                    Ok(Type::Number)
                } else {
                    Err(CompileError {
                        message: format!(
                            "Operator cannot be applied to types '{}' and '{}'",
                            left, right
                        ),
                        span: span.clone(),
                    })
                }
            }
            BinOp::Equal
            | BinOp::StrictEqual
            | BinOp::NotEqual
            | BinOp::StrictNotEqual
            | BinOp::Less
            | BinOp::Greater
            | BinOp::LessEqual
            | BinOp::GreaterEqual => Ok(Type::Boolean),
            BinOp::And | BinOp::Or => Ok(Type::Boolean),
        }
    }

    fn is_assignable(&self, from: &Type, to: &Type) -> bool {
        if from == to {
            return true;
        }
        // Unknown is compatible with anything (for untyped params)
        if from == &Type::Unknown || to == &Type::Unknown {
            return true;
        }
        false
    }

    fn resolve_type_annotation(&self, ann: &TypeAnnotation) -> Type {
        match &ann.kind {
            TypeAnnKind::Number => Type::Number,
            TypeAnnKind::String => Type::String,
            TypeAnnKind::Boolean => Type::Boolean,
            TypeAnnKind::Void => Type::Void,
            TypeAnnKind::Null => Type::Null,
            TypeAnnKind::Undefined => Type::Undefined,
        }
    }

    fn push_scope(&mut self) {
        self.scopes.push(Scope {
            symbols: HashMap::new(),
        });
    }

    fn pop_scope(&mut self) {
        self.scopes.pop();
    }

    fn define(&mut self, name: String, ty: Type, is_const: bool) {
        if let Some(scope) = self.scopes.last_mut() {
            scope.symbols.insert(name, Symbol { ty, is_const });
        }
    }

    fn lookup(&self, name: &str) -> Option<Type> {
        self.lookup_symbol(name).map(|s| s.ty.clone())
    }

    fn lookup_symbol(&self, name: &str) -> Option<Symbol> {
        for scope in self.scopes.iter().rev() {
            if let Some(sym) = scope.symbols.get(name) {
                return Some(sym.clone());
            }
        }
        None
    }
}
