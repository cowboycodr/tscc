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
    current_return_type: Option<Type>,
    /// Symbols exported from this module
    pub exported_symbols: HashMap<String, Type>,
    /// Symbols imported from other modules (populated before check)
    pub imported_symbols: HashMap<String, Type>,
}

impl TypeChecker {
    pub fn new() -> Self {
        let mut checker = Self {
            scopes: Vec::new(),
            current_return_type: None,
            exported_symbols: HashMap::new(),
            imported_symbols: HashMap::new(),
        };
        checker.push_scope();
        checker.register_builtins();
        checker
    }

    fn register_builtins(&mut self) {
        // console (special object)
        self.define("console".to_string(), Type::Unknown, true);
        // Math (special object)
        self.define("Math".to_string(), Type::Unknown, true);

        // Global functions
        self.define(
            "parseInt".to_string(),
            Type::Function {
                params: vec![Type::String],
                return_type: Box::new(Type::Number),
            },
            true,
        );
        self.define(
            "parseFloat".to_string(),
            Type::Function {
                params: vec![Type::String],
                return_type: Box::new(Type::Number),
            },
            true,
        );
    }

    pub fn check(&mut self, program: &Program) -> Result<(), CompileError> {
        // Register imported symbols in the current scope
        let imports: Vec<_> = self
            .imported_symbols
            .iter()
            .map(|(n, t)| (n.clone(), t.clone()))
            .collect();
        for (name, ty) in imports {
            self.define(name, ty, true);
        }

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
                is_exported,
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
                            return Err(CompileError::error(
                                format!("Type '{}' is not assignable to type '{}'", init, decl),
                                stmt.span.clone(),
                            ));
                        }
                        decl.clone()
                    }
                    (Some(decl), None) => decl.clone(),
                    (None, Some(init)) => init.clone(),
                    (None, None) => {
                        return Err(CompileError::error(
                            "Variable must have either a type annotation or initializer",
                            stmt.span.clone(),
                        ));
                    }
                };

                if *is_exported {
                    self.exported_symbols
                        .insert(name.clone(), final_type.clone());
                }

                self.define(name.clone(), final_type, *is_const);
                Ok(())
            }

            StmtKind::FunctionDecl {
                name,
                params,
                return_type,
                body,
                is_exported,
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

                if *is_exported {
                    self.exported_symbols
                        .insert(name.clone(), func_type.clone());
                }

                self.define(name.clone(), func_type, true);

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
                        return Err(CompileError::error(
                            format!(
                                "Type '{}' is not assignable to return type '{}'",
                                ret_type, expected
                            ),
                            stmt.span.clone(),
                        ));
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

            StmtKind::Import { .. } => {
                // Import resolution is handled before type checking
                Ok(())
            }

            StmtKind::Break | StmtKind::Continue => {
                // Validation that we're inside a loop could be added here
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

            ExprKind::Identifier(name) => self.lookup(name).ok_or_else(|| {
                CompileError::error(format!("Cannot find name '{}'", name), expr.span.clone())
            }),

            ExprKind::ArrayLiteral { elements } => {
                let mut elem_type = Type::Unknown;
                for elem in elements {
                    let t = self.check_expr(elem)?;
                    if elem_type == Type::Unknown {
                        elem_type = t;
                    }
                }
                if elem_type == Type::Unknown {
                    elem_type = Type::Number; // default for empty arrays
                }
                Ok(Type::Array(Box::new(elem_type)))
            }

            ExprKind::IndexAccess { object, index } => {
                let obj_type = self.check_expr(object)?;
                self.check_expr(index)?;
                match &obj_type {
                    Type::Array(elem) => Ok(*elem.clone()),
                    _ => Ok(Type::Unknown),
                }
            }

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
                            return Err(CompileError::error(
                                format!("Unary '-' requires number, got '{}'", operand_type),
                                expr.span.clone(),
                            ));
                        }
                        Ok(Type::Number)
                    }
                    UnaryOp::Not => Ok(Type::Boolean),
                }
            }

            ExprKind::Typeof { operand } => {
                self.check_expr(operand)?;
                Ok(Type::String)
            }

            ExprKind::Call { callee, args } => {
                // Special case: console.log / console.error / console.warn
                if let ExprKind::Member { object, property } = &callee.kind {
                    if let ExprKind::Identifier(name) = &object.kind {
                        if name == "console"
                            && (property == "log" || property == "error" || property == "warn")
                        {
                            for arg in args {
                                self.check_expr(arg)?;
                            }
                            return Ok(Type::Void);
                        }
                        // Math methods
                        if name == "Math" {
                            return self.check_math_call(property, args, &expr.span);
                        }
                    }
                    // String methods called on expressions
                    let obj_type = self.check_expr(object)?;
                    if obj_type == Type::String {
                        return self.check_string_method_call(property, args, &expr.span);
                    }
                    // Array methods called on expressions
                    if let Type::Array(ref elem_type) = obj_type {
                        return self.check_array_method_call(property, args, elem_type, &expr.span);
                    }
                }

                let callee_type = self.check_expr(callee)?;
                match &callee_type {
                    Type::Function {
                        params,
                        return_type,
                    } => {
                        if args.len() != params.len() {
                            return Err(CompileError::error(
                                format!("Expected {} arguments, got {}", params.len(), args.len()),
                                expr.span.clone(),
                            ));
                        }
                        for (arg, param_type) in args.iter().zip(params.iter()) {
                            let arg_type = self.check_expr(arg)?;
                            if !self.is_assignable(&arg_type, param_type) {
                                return Err(CompileError::error(
                                    format!(
                                        "Argument of type '{}' is not assignable to parameter of type '{}'",
                                        arg_type, param_type
                                    ),
                                    arg.span.clone(),
                                ));
                            }
                        }
                        Ok(*return_type.clone())
                    }
                    _ => Err(CompileError::error(
                        format!("Type '{}' is not callable", callee_type),
                        callee.span.clone(),
                    )),
                }
            }

            ExprKind::Member { object, property } => {
                if let ExprKind::Identifier(name) = &object.kind {
                    // console methods
                    if name == "console"
                        && (property == "log" || property == "error" || property == "warn")
                    {
                        return Ok(Type::Function {
                            params: vec![],
                            return_type: Box::new(Type::Void),
                        });
                    }
                    // Math constants
                    if name == "Math" {
                        match property.as_str() {
                            "PI" | "E" | "LN2" | "LN10" | "SQRT2" => return Ok(Type::Number),
                            // Math methods accessed as properties
                            "floor" | "ceil" | "round" | "abs" | "sqrt" | "sin" | "cos" | "tan"
                            | "log" | "exp" | "random" => {
                                return Ok(Type::Function {
                                    params: vec![Type::Number],
                                    return_type: Box::new(Type::Number),
                                });
                            }
                            "pow" | "min" | "max" => {
                                return Ok(Type::Function {
                                    params: vec![Type::Number, Type::Number],
                                    return_type: Box::new(Type::Number),
                                });
                            }
                            _ => {}
                        }
                    }
                }

                let obj_type = self.check_expr(object)?;

                // Array properties and methods
                if let Type::Array(ref elem_type) = obj_type {
                    match property.as_str() {
                        "length" => return Ok(Type::Number),
                        "push" => {
                            return Ok(Type::Function {
                                params: vec![*elem_type.clone()],
                                return_type: Box::new(Type::Number),
                            });
                        }
                        "pop" => {
                            return Ok(Type::Function {
                                params: vec![],
                                return_type: Box::new(*elem_type.clone()),
                            });
                        }
                        _ => {}
                    }
                }

                // String properties and methods
                if obj_type == Type::String {
                    match property.as_str() {
                        "length" => return Ok(Type::Number),
                        "toUpperCase" | "toLowerCase" | "trim" => {
                            return Ok(Type::Function {
                                params: vec![],
                                return_type: Box::new(Type::String),
                            });
                        }
                        "charAt" | "substring" | "slice" => {
                            return Ok(Type::Function {
                                params: vec![Type::Number],
                                return_type: Box::new(Type::String),
                            });
                        }
                        "indexOf" => {
                            return Ok(Type::Function {
                                params: vec![Type::String],
                                return_type: Box::new(Type::Number),
                            });
                        }
                        "includes" => {
                            return Ok(Type::Function {
                                params: vec![Type::String],
                                return_type: Box::new(Type::Boolean),
                            });
                        }
                        _ => {}
                    }
                }

                Err(CompileError::error(
                    format!(
                        "Property '{}' does not exist on type '{}'",
                        property, obj_type
                    ),
                    expr.span.clone(),
                ))
            }

            ExprKind::Assignment { name, value } => {
                let sym = self.lookup_symbol(name).ok_or_else(|| {
                    CompileError::error(format!("Cannot find name '{}'", name), expr.span.clone())
                })?;

                if sym.is_const {
                    return Err(CompileError::error(
                        format!("Cannot assign to '{}' because it is a constant", name),
                        expr.span.clone(),
                    ));
                }

                let var_type = sym.ty.clone();
                let val_type = self.check_expr(value)?;

                if !self.is_assignable(&val_type, &var_type) {
                    return Err(CompileError::error(
                        format!(
                            "Type '{}' is not assignable to type '{}'",
                            val_type, var_type
                        ),
                        expr.span.clone(),
                    ));
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

            ExprKind::Conditional {
                condition,
                consequent,
                alternate,
            } => {
                self.check_expr(condition)?;
                let then_type = self.check_expr(consequent)?;
                let else_type = self.check_expr(alternate)?;
                if then_type == else_type {
                    Ok(then_type)
                } else if then_type == Type::Unknown {
                    Ok(else_type)
                } else if else_type == Type::Unknown {
                    Ok(then_type)
                } else {
                    // Different types — return Unknown for now (union types later)
                    Ok(Type::Unknown)
                }
            }

            ExprKind::Grouping { expr } => self.check_expr(expr),

            ExprKind::PostfixUpdate { name, .. } | ExprKind::PrefixUpdate { name, .. } => {
                let ty = self.lookup(name).ok_or_else(|| {
                    CompileError::error(format!("Cannot find name '{}'", name), expr.span.clone())
                })?;
                if ty != Type::Number {
                    return Err(CompileError::error(
                        format!("Operator '++/--' requires number, got '{}'", ty),
                        expr.span.clone(),
                    ));
                }
                Ok(Type::Number)
            }
        }
    }

    fn check_math_call(
        &mut self,
        method: &str,
        args: &[Expr],
        span: &Span,
    ) -> Result<Type, CompileError> {
        let (expected_args, ret) = match method {
            "floor" | "ceil" | "round" | "abs" | "sqrt" | "sin" | "cos" | "tan" | "log" | "exp" => {
                (1, Type::Number)
            }
            "pow" | "min" | "max" => (2, Type::Number),
            "random" => (0, Type::Number),
            _ => {
                return Err(CompileError::error(
                    format!("'Math.{}' is not a known Math method", method),
                    span.clone(),
                ));
            }
        };

        if args.len() != expected_args {
            return Err(CompileError::error(
                format!(
                    "Math.{} expects {} argument{}, got {}",
                    method,
                    expected_args,
                    if expected_args == 1 { "" } else { "s" },
                    args.len()
                ),
                span.clone(),
            ));
        }

        for arg in args {
            let ty = self.check_expr(arg)?;
            if ty != Type::Number && ty != Type::Unknown {
                return Err(CompileError::error(
                    format!("Math.{} expects number arguments, got '{}'", method, ty),
                    arg.span.clone(),
                ));
            }
        }

        Ok(ret)
    }

    fn check_string_method_call(
        &mut self,
        method: &str,
        args: &[Expr],
        span: &Span,
    ) -> Result<Type, CompileError> {
        match method {
            "toUpperCase" | "toLowerCase" | "trim" => {
                if !args.is_empty() {
                    return Err(CompileError::error(
                        format!("'{}' takes no arguments", method),
                        span.clone(),
                    ));
                }
                Ok(Type::String)
            }
            "charAt" => {
                if args.len() != 1 {
                    return Err(CompileError::error(
                        "charAt expects 1 argument",
                        span.clone(),
                    ));
                }
                self.check_expr(&args[0])?;
                Ok(Type::String)
            }
            "indexOf" => {
                if args.len() != 1 {
                    return Err(CompileError::error(
                        "indexOf expects 1 argument",
                        span.clone(),
                    ));
                }
                self.check_expr(&args[0])?;
                Ok(Type::Number)
            }
            "includes" => {
                if args.len() != 1 {
                    return Err(CompileError::error(
                        "includes expects 1 argument",
                        span.clone(),
                    ));
                }
                self.check_expr(&args[0])?;
                Ok(Type::Boolean)
            }
            "substring" | "slice" => {
                if args.len() < 1 || args.len() > 2 {
                    return Err(CompileError::error(
                        format!("{} expects 1 or 2 arguments", method),
                        span.clone(),
                    ));
                }
                for arg in args {
                    self.check_expr(arg)?;
                }
                Ok(Type::String)
            }
            _ => Err(CompileError::error(
                format!("Property '{}' does not exist on type 'string'", method),
                span.clone(),
            )),
        }
    }

    fn check_array_method_call(
        &mut self,
        method: &str,
        args: &[Expr],
        elem_type: &Type,
        span: &Span,
    ) -> Result<Type, CompileError> {
        match method {
            "push" => {
                if args.len() != 1 {
                    return Err(CompileError::error(
                        format!("push expects 1 argument, got {}", args.len()),
                        span.clone(),
                    ));
                }
                self.check_expr(&args[0])?;
                Ok(Type::Number) // push returns new length
            }
            "pop" => {
                if !args.is_empty() {
                    return Err(CompileError::error("pop takes no arguments", span.clone()));
                }
                Ok(elem_type.clone())
            }
            _ => Err(CompileError::error(
                format!("Property '{}' does not exist on type 'array'", method),
                span.clone(),
            )),
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
                    Ok(Type::String)
                } else {
                    Err(CompileError::error(
                        format!(
                            "Operator '+' cannot be applied to types '{}' and '{}'",
                            left, right
                        ),
                        span.clone(),
                    ))
                }
            }
            BinOp::Subtract | BinOp::Multiply | BinOp::Divide | BinOp::Modulo | BinOp::Power => {
                if left == &Type::Number && right == &Type::Number {
                    Ok(Type::Number)
                } else {
                    Err(CompileError::error(
                        format!(
                            "Operator cannot be applied to types '{}' and '{}'",
                            left, right
                        ),
                        span.clone(),
                    ))
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
