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
    /// Current `this` type (set inside class methods / object methods)
    current_this_type: Option<Type>,
    /// Registered class types by name
    class_types: HashMap<String, Type>,
    /// Registered interface types by name
    interface_types: HashMap<String, Type>,
    /// Registered type aliases by name
    type_aliases: HashMap<String, TypeAnnotation>,
    /// Active type parameter substitutions (for generic function body checking)
    type_param_types: HashMap<String, Type>,
    /// Generic function signatures: name -> (type_param_names, param_types, return_type)
    generic_functions: HashMap<String, (Vec<String>, Vec<Type>, Type)>,
    /// Generic type aliases: name -> (type_param_names, body)
    generic_type_aliases: HashMap<String, (Vec<String>, TypeAnnotation)>,
}

impl TypeChecker {
    pub fn new() -> Self {
        let mut checker = Self {
            scopes: Vec::new(),
            current_return_type: None,
            exported_symbols: HashMap::new(),
            imported_symbols: HashMap::new(),
            current_this_type: None,
            class_types: HashMap::new(),
            interface_types: HashMap::new(),
            type_aliases: HashMap::new(),
            type_param_types: HashMap::new(),
            generic_functions: HashMap::new(),
            generic_type_aliases: HashMap::new(),
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
        // Number (special object with static methods)
        self.define("Number".to_string(), Type::Unknown, true);
        // NaN global constant
        self.define("NaN".to_string(), Type::Number, true);
        // Infinity global constant
        self.define("Infinity".to_string(), Type::Number, true);

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

        // Hoist function declarations: pre-scan and register signatures
        // so functions can be called before their declaration
        for stmt in &program.statements {
            if let StmtKind::FunctionDecl {
                name,
                type_params,
                params,
                return_type,
                is_exported,
                ..
            } = &stmt.kind
            {
                let is_generic = !type_params.is_empty();

                let prev_type_params = self.type_param_types.clone();
                if is_generic {
                    for tp in type_params {
                        let tp_type = tp
                            .constraint
                            .as_ref()
                            .map(|c| self.resolve_type_annotation(c))
                            .unwrap_or(Type::Unknown);
                        self.type_param_types.insert(tp.name.clone(), tp_type);
                    }
                }

                let param_types: Vec<Type> = params
                    .iter()
                    .map(|p| {
                        let ty = p
                            .type_ann
                            .as_ref()
                            .map(|ann| self.resolve_type_annotation(ann))
                            .unwrap_or(Type::Unknown);
                        if p.is_rest {
                            match ty {
                                Type::Array(_) => ty,
                                _ => Type::Array(Box::new(Type::Number)),
                            }
                        } else {
                            ty
                        }
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

                if is_generic {
                    let tp_names: Vec<String> =
                        type_params.iter().map(|tp| tp.name.clone()).collect();
                    self.generic_functions
                        .insert(name.clone(), (tp_names, param_types, ret_type));
                }

                self.type_param_types = prev_type_params;
            }
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
                    // Special case: array literal assigned to a tuple type —
                    // check each element against the tuple element types
                    if let Some(Type::Tuple(ref tuple_types)) = declared_type {
                        if let ExprKind::ArrayLiteral { elements } = &init.kind {
                            if elements.len() != tuple_types.len() {
                                return Err(CompileError::error(
                                    format!(
                                        "Tuple of length {} is not assignable to tuple of length {}",
                                        elements.len(),
                                        tuple_types.len()
                                    ),
                                    stmt.span.clone(),
                                ));
                            }
                            for (elem, expected) in elements.iter().zip(tuple_types.iter()) {
                                let elem_type = self.check_expr(elem)?;
                                if !self.is_assignable(&elem_type, expected) {
                                    return Err(CompileError::error(
                                        format!(
                                            "Type '{}' is not assignable to type '{}'",
                                            elem_type, expected
                                        ),
                                        stmt.span.clone(),
                                    ));
                                }
                            }
                            // All checks passed — use the declared tuple type
                            self.define(name.clone(), declared_type.unwrap(), *is_const);
                            return Ok(());
                        }
                    }
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
                type_params,
                params,
                return_type,
                body,
                is_exported,
            } => {
                let is_generic = !type_params.is_empty();

                // Register type params BEFORE resolving param/return types
                // so that Named("T") resolves to the constraint type
                let prev_type_params = self.type_param_types.clone();
                if is_generic {
                    for tp in type_params {
                        let tp_type = tp
                            .constraint
                            .as_ref()
                            .map(|c| self.resolve_type_annotation(c))
                            .unwrap_or(Type::Unknown);
                        self.type_param_types.insert(tp.name.clone(), tp_type);
                    }
                }

                let param_types: Vec<Type> = params
                    .iter()
                    .map(|p| {
                        let ty = p
                            .type_ann
                            .as_ref()
                            .map(|ann| self.resolve_type_annotation(ann))
                            .unwrap_or(Type::Unknown);
                        // Rest params are always array type
                        if p.is_rest {
                            match ty {
                                Type::Array(_) => ty,
                                _ => Type::Array(Box::new(Type::Number)),
                            }
                        } else {
                            ty
                        }
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

                // Register generic function signature for call-site inference
                if is_generic {
                    let tp_names: Vec<String> =
                        type_params.iter().map(|tp| tp.name.clone()).collect();
                    self.generic_functions.insert(
                        name.clone(),
                        (tp_names, param_types.clone(), ret_type.clone()),
                    );
                }

                self.push_scope();
                let prev_return_type = self.current_return_type.clone();
                self.current_return_type = Some(ret_type.clone());

                for (param, param_type) in params.iter().zip(param_types.iter()) {
                    self.define(param.name.clone(), param_type.clone(), false);
                }

                for stmt in body {
                    self.check_statement(stmt)?;
                }

                self.type_param_types = prev_type_params;
                self.current_return_type = prev_return_type;
                self.pop_scope();
                Ok(())
            }

            StmtKind::ClassDecl {
                name,
                type_params: _,
                parent,
                fields,
                constructor,
                methods,
            } => {
                // Collect parent fields if inheriting
                let mut all_fields: Vec<(String, Type)> = Vec::new();
                if let Some(parent_name) = parent {
                    if let Some(parent_type) = self.class_types.get(parent_name) {
                        if let Type::Class { fields: pf, .. } = parent_type {
                            all_fields.extend(pf.clone());
                        }
                    } else {
                        return Err(CompileError::error(
                            format!("Cannot find base class '{}'", parent_name),
                            stmt.span.clone(),
                        ));
                    }
                }

                // Add own fields
                for field in fields {
                    let ty = field
                        .type_ann
                        .as_ref()
                        .map(|ann| self.resolve_type_annotation(ann))
                        .unwrap_or(Type::Unknown);
                    // Override parent field if same name
                    all_fields.retain(|(n, _)| n != &field.name);
                    all_fields.push((field.name.clone(), ty));
                }

                // Add methods
                for method in methods {
                    let param_types: Vec<Type> = method
                        .params
                        .iter()
                        .map(|p| {
                            p.type_ann
                                .as_ref()
                                .map(|ann| self.resolve_type_annotation(ann))
                                .unwrap_or(Type::Unknown)
                        })
                        .collect();
                    let ret_type = method
                        .return_type
                        .as_ref()
                        .map(|ann| self.resolve_type_annotation(ann))
                        .unwrap_or(Type::Void);
                    let method_type = Type::Function {
                        params: param_types,
                        return_type: Box::new(ret_type),
                    };
                    // Override parent method if same name
                    all_fields.retain(|(n, _)| n != &method.name);
                    all_fields.push((method.name.clone(), method_type));
                }

                let class_type = Type::Class {
                    name: name.clone(),
                    fields: all_fields.clone(),
                };

                self.class_types.insert(name.clone(), class_type.clone());
                self.define(name.clone(), class_type.clone(), true);

                // Now type-check constructor and methods with `this` context
                let this_type = Type::Object {
                    fields: all_fields.clone(),
                };
                let prev_this = self.current_this_type.clone();
                self.current_this_type = Some(this_type);

                if let Some(ctor) = constructor {
                    self.push_scope();
                    let prev_ret = self.current_return_type.clone();
                    self.current_return_type = Some(Type::Void);

                    for param in &ctor.params {
                        let ty = param
                            .type_ann
                            .as_ref()
                            .map(|ann| self.resolve_type_annotation(ann))
                            .unwrap_or(Type::Unknown);
                        self.define(param.name.clone(), ty, false);
                    }
                    for s in &ctor.body {
                        self.check_statement(s)?;
                    }

                    self.current_return_type = prev_ret;
                    self.pop_scope();
                }

                for method in methods {
                    let param_types: Vec<Type> = method
                        .params
                        .iter()
                        .map(|p| {
                            p.type_ann
                                .as_ref()
                                .map(|ann| self.resolve_type_annotation(ann))
                                .unwrap_or(Type::Unknown)
                        })
                        .collect();
                    let ret_type = method
                        .return_type
                        .as_ref()
                        .map(|ann| self.resolve_type_annotation(ann))
                        .unwrap_or(Type::Void);

                    self.push_scope();
                    let prev_ret = self.current_return_type.clone();
                    self.current_return_type = Some(ret_type);

                    for (param, param_type) in method.params.iter().zip(param_types.iter()) {
                        self.define(param.name.clone(), param_type.clone(), false);
                    }
                    for s in &method.body {
                        self.check_statement(s)?;
                    }

                    self.current_return_type = prev_ret;
                    self.pop_scope();
                }

                self.current_this_type = prev_this;
                Ok(())
            }

            StmtKind::InterfaceDecl {
                name,
                extends,
                fields,
            } => {
                // Collect inherited fields from parent interfaces (prefix layout)
                let mut field_types: Vec<(String, Type)> = Vec::new();
                for parent_name in extends {
                    if let Some(parent_type) = self.interface_types.get(parent_name).cloned() {
                        if let Type::Object {
                            fields: parent_fields,
                        } = parent_type
                        {
                            for pf in parent_fields {
                                if !field_types.iter().any(|(n, _)| n == &pf.0) {
                                    field_types.push(pf);
                                }
                            }
                        }
                    }
                }
                // Append own fields
                for (n, ann) in fields {
                    let t = self.resolve_type_annotation(ann);
                    if !field_types.iter().any(|(existing, _)| existing == n) {
                        field_types.push((n.clone(), t));
                    }
                }
                let iface_type = Type::Object {
                    fields: field_types,
                };
                self.interface_types
                    .insert(name.clone(), iface_type.clone());
                // Also define as a type in scope so it can be referenced
                self.define(name.clone(), iface_type, true);
                Ok(())
            }

            StmtKind::EnumDecl { name, members } => {
                // Register enum as an object type with member fields
                let mut field_types = Vec::new();
                let mut next_index: f64 = 0.0;
                for member in members {
                    let member_type = match &member.value {
                        Some(EnumValue::String(_)) => Type::String,
                        Some(EnumValue::Number(_)) => Type::Number,
                        None => {
                            next_index += 1.0;
                            Type::Number
                        }
                    };
                    if let Some(EnumValue::Number(n)) = &member.value {
                        next_index = n + 1.0;
                    }
                    field_types.push((member.name.clone(), member_type));
                }
                let enum_type = Type::Object {
                    fields: field_types,
                };
                self.define(name.clone(), enum_type, true);
                Ok(())
            }

            StmtKind::TypeAlias {
                name,
                type_params,
                type_ann,
            } => {
                // Register the alias for use in resolve_type_annotation
                self.type_aliases.insert(name.clone(), type_ann.clone());
                // For non-generic aliases, resolve and register immediately
                if type_params.is_empty() {
                    let resolved = self.resolve_type_annotation(type_ann);
                    self.interface_types.insert(name.clone(), resolved);
                } else {
                    // Generic type alias — store type param names for later resolution
                    let tp_names: Vec<String> =
                        type_params.iter().map(|tp| tp.name.clone()).collect();
                    self.generic_type_aliases
                        .insert(name.clone(), (tp_names, type_ann.clone()));
                }
                Ok(())
            }

            StmtKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                self.check_expr(condition)?;

                // Detect typeof narrowing: typeof x === "type" / typeof x !== "type"
                let narrowing = Self::detect_typeof_narrowing(condition).and_then(
                    |(var_name, type_str, is_eq)| {
                        if let Some(Type::Union(variants)) = self.lookup(&var_name) {
                            let target = Self::type_string_to_type(&type_str);
                            let remaining: Vec<Type> = variants
                                .iter()
                                .filter(|v| !Self::is_same_type_category(v, &target))
                                .cloned()
                                .collect();
                            Some((var_name, target, remaining, is_eq))
                        } else {
                            None
                        }
                    },
                );

                self.push_scope();
                if let Some((ref var_name, ref target, ref remaining, is_eq)) = narrowing {
                    // In then-branch: narrow to matched type (=== → target, !== → remaining)
                    if is_eq {
                        self.define(var_name.clone(), target.clone(), false);
                    } else if remaining.len() == 1 {
                        self.define(var_name.clone(), remaining[0].clone(), false);
                    } else if !remaining.is_empty() {
                        self.define(var_name.clone(), Type::Union(remaining.clone()), false);
                    }
                }
                for stmt in then_branch {
                    self.check_statement(stmt)?;
                }
                self.pop_scope();

                if let Some(else_stmts) = else_branch {
                    self.push_scope();
                    if let Some((ref var_name, ref target, ref remaining, is_eq)) = narrowing {
                        // In else-branch: narrow to the complement
                        if is_eq {
                            if remaining.len() == 1 {
                                self.define(var_name.clone(), remaining[0].clone(), false);
                            } else if !remaining.is_empty() {
                                self.define(
                                    var_name.clone(),
                                    Type::Union(remaining.clone()),
                                    false,
                                );
                            }
                        } else {
                            self.define(var_name.clone(), target.clone(), false);
                        }
                    }
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

            StmtKind::DoWhile { body, condition } => {
                self.push_scope();
                for stmt in body {
                    self.check_statement(stmt)?;
                }
                self.pop_scope();
                self.check_expr(condition)?;
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

            StmtKind::Switch {
                discriminant,
                cases,
            } => {
                self.check_expr(discriminant)?;
                for case in cases {
                    if let Some(test) = &case.test {
                        self.check_expr(test)?;
                    }
                    self.push_scope();
                    for stmt in &case.body {
                        self.check_statement(stmt)?;
                    }
                    self.pop_scope();
                }
                Ok(())
            }

            StmtKind::ForOf {
                var_name,
                iterable,
                body,
            } => {
                self.check_expr(iterable)?;
                self.push_scope();
                // Element type is Number (arrays hold f64)
                self.define(var_name.clone(), Type::Number, false);
                for stmt in body {
                    self.check_statement(stmt)?;
                }
                self.pop_scope();
                Ok(())
            }

            StmtKind::ArrayDestructure {
                names,
                initializer,
                is_const,
            } => {
                self.check_expr(initializer)?;
                // Bind each name as Number (array elements are f64)
                for name in names {
                    self.define(name.clone(), Type::Number, *is_const);
                }
                Ok(())
            }

            StmtKind::ObjectDestructure {
                names,
                initializer,
                is_const,
            } => {
                let init_type = self.check_expr(initializer)?;
                let fields = match &init_type {
                    Type::Object { fields } => Some(fields.clone()),
                    Type::Class { fields, .. } => Some(fields.clone()),
                    _ => None,
                };
                for (local, key) in names {
                    let ty = fields
                        .as_ref()
                        .and_then(|fs| fs.iter().find(|(n, _)| n == key).map(|(_, t)| t.clone()))
                        .unwrap_or(Type::Unknown);
                    self.define(local.clone(), ty, *is_const);
                }
                Ok(())
            }

            StmtKind::ForIn {
                var_name,
                object,
                body,
            } => {
                self.check_expr(object)?;
                self.push_scope();
                // for-in iterates over string keys
                self.define(var_name.clone(), Type::String, false);
                for stmt in body {
                    self.check_statement(stmt)?;
                }
                self.pop_scope();
                Ok(())
            }

            StmtKind::Break { .. } | StmtKind::Continue { .. } | StmtKind::Empty => {
                // Validation that we're inside a loop could be added here
                Ok(())
            }

            StmtKind::Labeled { body, .. } => self.check_statement(body),
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

            ExprKind::This => self.current_this_type.clone().ok_or_else(|| {
                CompileError::error(
                    "'this' is only valid inside a class method or object method",
                    expr.span.clone(),
                )
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

            ExprKind::ObjectLiteral { properties } => {
                let mut fields = Vec::new();
                for prop in properties {
                    if prop.is_spread {
                        // Spread: { ...expr } — merge source fields into this object's type
                        let spread_ty = self.check_expr(&prop.value)?;
                        if let Type::Object {
                            fields: spread_fields,
                        } = spread_ty
                        {
                            for (name, ty) in spread_fields {
                                fields.push((name, ty));
                            }
                        }
                    } else if prop.computed_key.is_some() {
                        // Computed key: { [expr]: value } — type-check both sides, skip field tracking
                        self.check_expr(prop.computed_key.as_ref().unwrap())?;
                        self.check_expr(&prop.value)?;
                    } else if prop.is_method {
                        // For methods, build a function type
                        let param_types: Vec<Type> = prop
                            .params
                            .iter()
                            .map(|p| {
                                p.type_ann
                                    .as_ref()
                                    .map(|ann| self.resolve_type_annotation(ann))
                                    .unwrap_or(Type::Unknown)
                            })
                            .collect();
                        let ret_type = prop
                            .return_type
                            .as_ref()
                            .map(|ann| self.resolve_type_annotation(ann))
                            .unwrap_or(Type::Void);
                        let method_type = Type::Function {
                            params: param_types,
                            return_type: Box::new(ret_type),
                        };
                        fields.push((prop.key.clone(), method_type));
                    } else {
                        let ty = self.check_expr(&prop.value)?;
                        fields.push((prop.key.clone(), ty));
                    }
                }

                // For methods that use `this`, we need to type-check their bodies
                // with the object type set as `this`
                let obj_type = Type::Object {
                    fields: fields.clone(),
                };
                let prev_this = self.current_this_type.clone();
                self.current_this_type = Some(obj_type.clone());

                for prop in properties {
                    if prop.is_method {
                        if let ExprKind::ArrowFunction { params, body, .. } = &prop.value.kind {
                            let param_types: Vec<Type> = params
                                .iter()
                                .map(|p| {
                                    p.type_ann
                                        .as_ref()
                                        .map(|ann| self.resolve_type_annotation(ann))
                                        .unwrap_or(Type::Unknown)
                                })
                                .collect();
                            let ret_type = prop
                                .return_type
                                .as_ref()
                                .map(|ann| self.resolve_type_annotation(ann))
                                .unwrap_or(Type::Void);

                            self.push_scope();
                            let prev_ret = self.current_return_type.clone();
                            self.current_return_type = Some(ret_type);

                            for (param, param_type) in params.iter().zip(param_types.iter()) {
                                self.define(param.name.clone(), param_type.clone(), false);
                            }

                            match body {
                                ArrowBody::Block(stmts) => {
                                    for s in stmts {
                                        self.check_statement(s)?;
                                    }
                                }
                                ArrowBody::Expr(e) => {
                                    self.check_expr(e)?;
                                }
                            }

                            self.current_return_type = prev_ret;
                            self.pop_scope();
                        }
                    }
                }

                self.current_this_type = prev_this;
                Ok(obj_type)
            }

            ExprKind::IndexAccess { object, index } => {
                let obj_type = self.check_expr(object)?;
                let idx_type = self.check_expr(index)?;
                match &obj_type {
                    Type::Array(elem) => Ok(*elem.clone()),
                    Type::Object { fields } => {
                        // Bracket access with string literal: obj["key"]
                        if let ExprKind::StringLiteral(key) = &index.kind {
                            for (name, ty) in fields {
                                if name == key {
                                    return Ok(ty.clone());
                                }
                            }
                            return Err(CompileError::error(
                                format!("Property '{}' does not exist on object", key),
                                expr.span.clone(),
                            ));
                        }
                        let _ = idx_type;
                        Ok(Type::Unknown)
                    }
                    Type::Class { fields, .. } => {
                        if let ExprKind::StringLiteral(key) = &index.kind {
                            for (name, ty) in fields {
                                if name == key {
                                    return Ok(ty.clone());
                                }
                            }
                        }
                        Ok(Type::Unknown)
                    }
                    Type::Tuple(elements) => {
                        // Tuple indexing with numeric literal: pair[0], pair[1]
                        if let ExprKind::NumberLiteral(n) = &index.kind {
                            let idx = *n as usize;
                            if idx < elements.len() {
                                return Ok(elements[idx].clone());
                            }
                            return Err(CompileError::error(
                                format!(
                                    "Tuple index {} out of bounds for tuple of length {}",
                                    idx,
                                    elements.len()
                                ),
                                expr.span.clone(),
                            ));
                        }
                        // Dynamic index on tuple — return unknown
                        Ok(Type::Unknown)
                    }
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
                        // Number static methods
                        if name == "Number" {
                            return self.check_number_static_call(property, args, &expr.span);
                        }
                    }
                    // String methods called on expressions
                    let obj_type = self.check_expr(object)?;
                    if obj_type == Type::String {
                        return self.check_string_method_call(property, args, &expr.span);
                    }
                    // Number methods called on expressions
                    if obj_type == Type::Number {
                        return self.check_number_method_call(property, args, &expr.span);
                    }
                    // Array methods called on expressions
                    if let Type::Array(ref elem_type) = obj_type {
                        return self.check_array_method_call(property, args, elem_type, &expr.span);
                    }
                    // Object/class method calls
                    let fields = match &obj_type {
                        Type::Object { fields } => Some(fields.clone()),
                        Type::Class { fields, .. } => Some(fields.clone()),
                        _ => None,
                    };
                    if let Some(fields) = fields {
                        for (name, ty) in &fields {
                            if name == property {
                                if let Type::Function {
                                    params,
                                    return_type,
                                } = ty
                                {
                                    if args.len() != params.len() {
                                        return Err(CompileError::error(
                                            format!(
                                                "Expected {} arguments, got {}",
                                                params.len(),
                                                args.len()
                                            ),
                                            expr.span.clone(),
                                        ));
                                    }
                                    for arg in args {
                                        self.check_expr(arg)?;
                                    }
                                    return Ok(*return_type.clone());
                                }
                            }
                        }
                    }
                }

                // Special case: calls to generic functions — skip strict param assignability
                if let ExprKind::Identifier(fn_name) = &callee.kind {
                    if self.generic_functions.contains_key(fn_name.as_str()) {
                        // Just check that args are valid expressions; constraint
                        // validation is done at codegen time via monomorphization
                        for arg in args {
                            self.check_expr(arg)?;
                        }
                        // Return the function's declared return type
                        let callee_type = self.check_expr(callee)?;
                        if let Type::Function { return_type, .. } = &callee_type {
                            return Ok(*return_type.clone());
                        }
                        return Ok(Type::Unknown);
                    }
                }

                let callee_type = self.check_expr(callee)?;
                match &callee_type {
                    Type::Function {
                        params,
                        return_type,
                    } => {
                        // Check if function is variadic (last param is Array = rest param)
                        let is_variadic =
                            params.last().map_or(false, |t| matches!(t, Type::Array(_)));
                        if !is_variadic && args.len() > params.len() {
                            return Err(CompileError::error(
                                format!("Expected {} arguments, got {}", params.len(), args.len()),
                                expr.span.clone(),
                            ));
                        }
                        // For variadic: check non-rest args; rest args are unchecked
                        let check_count = if is_variadic {
                            params.len().saturating_sub(1).min(args.len())
                        } else {
                            args.len().min(params.len())
                        };
                        for (arg, param_type) in args.iter().take(check_count).zip(params.iter()) {
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
                        // Check remaining rest args
                        if is_variadic {
                            for arg in args.iter().skip(check_count) {
                                self.check_expr(arg)?;
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

                // Object/class field access
                let fields = match &obj_type {
                    Type::Object { fields } => Some(fields),
                    Type::Class { fields, .. } => Some(fields),
                    _ => None,
                };
                if let Some(fields) = fields {
                    for (name, ty) in fields {
                        if name == property {
                            return Ok(ty.clone());
                        }
                    }
                }

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
                        "includes" | "startsWith" | "endsWith" => {
                            return Ok(Type::Function {
                                params: vec![Type::String],
                                return_type: Box::new(Type::Boolean),
                            });
                        }
                        "repeat" => {
                            return Ok(Type::Function {
                                params: vec![Type::Number],
                                return_type: Box::new(Type::String),
                            });
                        }
                        "replace" => {
                            return Ok(Type::Function {
                                params: vec![Type::String, Type::String],
                                return_type: Box::new(Type::String),
                            });
                        }
                        "padStart" => {
                            return Ok(Type::Function {
                                params: vec![Type::Number, Type::String],
                                return_type: Box::new(Type::String),
                            });
                        }
                        "split" => {
                            return Ok(Type::Function {
                                params: vec![Type::String],
                                return_type: Box::new(Type::Array(Box::new(Type::String))),
                            });
                        }
                        _ => {}
                    }
                }

                // Number methods
                if obj_type == Type::Number {
                    match property.as_str() {
                        "toFixed" => {
                            return Ok(Type::Function {
                                params: vec![Type::Number],
                                return_type: Box::new(Type::String),
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

            // Optional chaining: obj?.prop — same as obj.prop for now (no runtime null)
            ExprKind::OptionalMember { object, property } => {
                let obj_type = self.check_expr(object)?;
                let fields = match &obj_type {
                    Type::Object { fields } => Some(fields.clone()),
                    Type::Class { fields, .. } => Some(fields.clone()),
                    _ => None,
                };
                if let Some(fields) = fields {
                    for (name, ty) in &fields {
                        if name == property {
                            return Ok(ty.clone());
                        }
                    }
                }
                Ok(Type::Unknown)
            }

            // Spread element: only valid inside array literals (handled there)
            ExprKind::Spread { expr: inner } => self.check_expr(inner),

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

            ExprKind::MemberAssignment {
                object,
                property,
                value,
            } => {
                let obj_type = self.check_expr(object)?;
                let val_type = self.check_expr(value)?;

                // Look up property type
                let fields = match &obj_type {
                    Type::Object { fields } => Some(fields),
                    Type::Class { fields, .. } => Some(fields),
                    _ => None,
                };
                if let Some(fields) = fields {
                    for (name, ty) in fields {
                        if name == property {
                            if !self.is_assignable(&val_type, ty) {
                                return Err(CompileError::error(
                                    format!(
                                        "Type '{}' is not assignable to type '{}'",
                                        val_type, ty
                                    ),
                                    expr.span.clone(),
                                ));
                            }
                            return Ok(val_type);
                        }
                    }
                }

                // Allow assignment to `this.x` even if not found in fields (constructor sets fields)
                if matches!(object.kind, ExprKind::This) {
                    return Ok(val_type);
                }

                Err(CompileError::error(
                    format!(
                        "Property '{}' does not exist on type '{}'",
                        property, obj_type
                    ),
                    expr.span.clone(),
                ))
            }

            ExprKind::NewExpr { class_name, args } => {
                let class_type = self.class_types.get(class_name).cloned().ok_or_else(|| {
                    CompileError::error(
                        format!("Cannot find class '{}'", class_name),
                        expr.span.clone(),
                    )
                })?;

                // Type-check constructor arguments
                for arg in args {
                    self.check_expr(arg)?;
                }

                // Return the class type
                Ok(class_type)
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

            ExprKind::TypeAssertion {
                expr: inner,
                target_type,
            } => {
                // Type-check the inner expression (for side effects / error detection)
                self.check_expr(inner)?;
                // Return the asserted type — trust the programmer
                Ok(self.resolve_type_annotation(target_type))
            }

            ExprKind::Satisfies {
                expr: inner,
                target_type,
            } => {
                // Check inner expression and verify it's assignable to target
                let inner_type = self.check_expr(inner)?;
                let target = self.resolve_type_annotation(target_type);
                if !self.is_assignable(&inner_type, &target) {
                    return Err(CompileError::error(
                        format!("Type '{}' does not satisfy '{}'", inner_type, target),
                        expr.span.clone(),
                    ));
                }
                // satisfies returns the original (narrower) type, not the target
                Ok(inner_type)
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
            "includes" | "startsWith" | "endsWith" => {
                if args.len() != 1 {
                    return Err(CompileError::error(
                        format!("{} expects 1 argument", method),
                        span.clone(),
                    ));
                }
                self.check_expr(&args[0])?;
                Ok(Type::Boolean)
            }
            "repeat" => {
                if args.len() != 1 {
                    return Err(CompileError::error(
                        "repeat expects 1 argument",
                        span.clone(),
                    ));
                }
                self.check_expr(&args[0])?;
                Ok(Type::String)
            }
            "split" => {
                if args.len() != 1 {
                    return Err(CompileError::error(
                        "split expects 1 argument",
                        span.clone(),
                    ));
                }
                self.check_expr(&args[0])?;
                Ok(Type::Array(Box::new(Type::String)))
            }
            "replace" => {
                if args.len() != 2 {
                    return Err(CompileError::error(
                        "replace expects 2 arguments",
                        span.clone(),
                    ));
                }
                for arg in args {
                    self.check_expr(arg)?;
                }
                Ok(Type::String)
            }
            "padStart" => {
                if args.len() != 2 {
                    return Err(CompileError::error(
                        "padStart expects 2 arguments",
                        span.clone(),
                    ));
                }
                for arg in args {
                    self.check_expr(arg)?;
                }
                Ok(Type::String)
            }
            "substring" | "slice" => {
                if args.is_empty() || args.len() > 2 {
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

    fn check_number_static_call(
        &mut self,
        method: &str,
        args: &[Expr],
        span: &Span,
    ) -> Result<Type, CompileError> {
        match method {
            "isFinite" | "isInteger" | "isNaN" => {
                if args.len() != 1 {
                    return Err(CompileError::error(
                        format!("Number.{} expects 1 argument", method),
                        span.clone(),
                    ));
                }
                self.check_expr(&args[0])?;
                Ok(Type::Boolean)
            }
            _ => Err(CompileError::error(
                format!("'Number.{}' is not a known Number method", method),
                span.clone(),
            )),
        }
    }

    fn check_number_method_call(
        &mut self,
        method: &str,
        args: &[Expr],
        span: &Span,
    ) -> Result<Type, CompileError> {
        match method {
            "toFixed" => {
                if args.len() != 1 {
                    return Err(CompileError::error(
                        "toFixed expects 1 argument",
                        span.clone(),
                    ));
                }
                self.check_expr(&args[0])?;
                Ok(Type::String)
            }
            _ => Err(CompileError::error(
                format!("Property '{}' does not exist on type 'number'", method),
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
            "map" => {
                if args.len() != 1 {
                    return Err(CompileError::error(
                        format!("map expects 1 argument, got {}", args.len()),
                        span.clone(),
                    ));
                }
                let cb_type = self.check_expr(&args[0])?;
                if let Type::Function { return_type, .. } = &cb_type {
                    Ok(Type::Array(return_type.clone()))
                } else {
                    Ok(Type::Array(Box::new(elem_type.clone())))
                }
            }
            "filter" => {
                if args.len() != 1 {
                    return Err(CompileError::error(
                        format!("filter expects 1 argument, got {}", args.len()),
                        span.clone(),
                    ));
                }
                self.check_expr(&args[0])?;
                Ok(Type::Array(Box::new(elem_type.clone())))
            }
            "reduce" => {
                if args.len() != 2 {
                    return Err(CompileError::error(
                        format!("reduce expects 2 arguments, got {}", args.len()),
                        span.clone(),
                    ));
                }
                self.check_expr(&args[0])?;
                let init_type = self.check_expr(&args[1])?;
                Ok(init_type)
            }
            "forEach" => {
                if args.len() != 1 {
                    return Err(CompileError::error(
                        format!("forEach expects 1 argument, got {}", args.len()),
                        span.clone(),
                    ));
                }
                self.check_expr(&args[0])?;
                Ok(Type::Void)
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
        // Unknown (any) is permissive — allow operations with it
        if left == &Type::Unknown || right == &Type::Unknown {
            return match op {
                BinOp::Equal
                | BinOp::StrictEqual
                | BinOp::NotEqual
                | BinOp::StrictNotEqual
                | BinOp::Less
                | BinOp::Greater
                | BinOp::LessEqual
                | BinOp::GreaterEqual
                | BinOp::And
                | BinOp::Or => Ok(Type::Boolean),
                BinOp::NullishCoalescing => Ok(Type::Unknown),
                BinOp::Add if left == &Type::String || right == &Type::String => Ok(Type::String),
                _ => Ok(Type::Number),
            };
        }
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
            BinOp::NullishCoalescing => {
                // Result type is the non-null type (RHS if LHS is null/undefined)
                if left == &Type::Null || left == &Type::Undefined {
                    Ok(right.clone())
                } else {
                    Ok(left.clone())
                }
            }
        }
    }

    fn is_assignable(&self, from: &Type, to: &Type) -> bool {
        if from == to {
            return true;
        }
        if from == &Type::Unknown || to == &Type::Unknown {
            return true;
        }
        // Literal types are subtypes of their primitive types
        if let Type::StringLiteral(_) = from {
            if matches!(to, Type::String) {
                return true;
            }
        }
        if let Type::NumberLiteral(_) = from {
            if matches!(to, Type::Number) {
                return true;
            }
        }
        // Primitives accept their literal types as targets too (for widening)
        if matches!(from, Type::String) {
            if matches!(to, Type::StringLiteral(_)) {
                return true;
            }
        }
        if matches!(from, Type::Number) {
            if matches!(to, Type::NumberLiteral(_)) {
                return true;
            }
        }
        // A value is assignable TO a union if it's assignable to any variant
        if let Type::Union(variants) = to {
            return variants.iter().any(|v| self.is_assignable(from, v));
        }
        // A union is assignable FROM if all variants are assignable to the target
        if let Type::Union(variants) = from {
            return variants.iter().all(|v| self.is_assignable(v, to));
        }
        // Object structural compatibility: from has all fields of to
        if let (
            Type::Object {
                fields: from_fields,
            },
            Type::Object { fields: to_fields },
        ) = (from, to)
        {
            return to_fields.iter().all(|(name, ty)| {
                from_fields
                    .iter()
                    .any(|(n, t)| n == name && self.is_assignable(t, ty))
            });
        }
        // Class instance is assignable to Object with matching fields
        if let (
            Type::Class {
                fields: from_fields,
                ..
            },
            Type::Object { fields: to_fields },
        ) = (from, to)
        {
            return to_fields.iter().all(|(name, ty)| {
                from_fields
                    .iter()
                    .any(|(n, t)| n == name && self.is_assignable(t, ty))
            });
        }
        // Same-name class types
        if let (Type::Class { name: n1, .. }, Type::Class { name: n2, .. }) = (from, to) {
            return n1 == n2;
        }
        // Function type compatibility: compare params and return types recursively
        if let (
            Type::Function {
                params: from_params,
                return_type: from_ret,
            },
            Type::Function {
                params: to_params,
                return_type: to_ret,
            },
        ) = (from, to)
        {
            if from_params.len() != to_params.len() {
                return false;
            }
            let params_ok = from_params
                .iter()
                .zip(to_params.iter())
                .all(|(f, t)| self.is_assignable(f, t));
            let ret_ok = self.is_assignable(from_ret, to_ret);
            return params_ok && ret_ok;
        }
        // Tuple structural compatibility: same length, each element assignable
        if let (Type::Tuple(from_elems), Type::Tuple(to_elems)) = (from, to) {
            if from_elems.len() != to_elems.len() {
                return false;
            }
            return from_elems
                .iter()
                .zip(to_elems.iter())
                .all(|(f, t)| self.is_assignable(f, t));
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
            TypeAnnKind::Array(inner) => Type::Array(Box::new(self.resolve_type_annotation(inner))),
            TypeAnnKind::Object { fields } => Type::Object {
                fields: fields
                    .iter()
                    .map(|(n, ann)| (n.clone(), self.resolve_type_annotation(ann)))
                    .collect(),
            },
            TypeAnnKind::Named(name) => {
                // Check active type parameter substitutions first (generics)
                if let Some(ty) = self.type_param_types.get(name) {
                    return ty.clone();
                }
                // Look up type alias, interface, or class type
                if let Some(ty) = self.interface_types.get(name) {
                    return ty.clone();
                }
                if let Some(ty) = self.class_types.get(name) {
                    return ty.clone();
                }
                Type::Unknown
            }
            TypeAnnKind::Typeof(name) => {
                // Look up the variable's type in scope
                self.lookup(name).unwrap_or(Type::Unknown)
            }
            TypeAnnKind::StringLiteral(s) => Type::StringLiteral(s.clone()),
            TypeAnnKind::NumberLiteral(n) => Type::NumberLiteral(n.to_string()),
            TypeAnnKind::BooleanLiteral(_) => Type::Boolean,
            TypeAnnKind::Union(variants) => {
                let types: Vec<Type> = variants
                    .iter()
                    .map(|v| self.resolve_type_annotation(v))
                    .collect();
                Type::Union(types)
            }
            TypeAnnKind::Intersection(variants) => {
                // Merge object/class fields from all variants
                let types: Vec<Type> = variants
                    .iter()
                    .map(|v| self.resolve_type_annotation(v))
                    .collect();
                // Flatten into a single Object type by merging fields
                let mut merged_fields: Vec<(String, Type)> = Vec::new();
                for ty in &types {
                    match ty {
                        Type::Object { fields } | Type::Class { fields, .. } => {
                            for (name, field_ty) in fields {
                                if !merged_fields.iter().any(|(n, _)| n == name) {
                                    merged_fields.push((name.clone(), field_ty.clone()));
                                }
                            }
                        }
                        _ => {}
                    }
                }
                if merged_fields.is_empty() {
                    Type::Intersection(types)
                } else {
                    Type::Object {
                        fields: merged_fields,
                    }
                }
            }
            TypeAnnKind::Keyof(inner) => {
                // Resolve inner type, extract field names as string literal union
                let inner_type = self.resolve_type_annotation(inner);
                match &inner_type {
                    Type::Object { fields } | Type::Class { fields, .. } => {
                        let literals: Vec<Type> = fields
                            .iter()
                            .map(|(name, _)| Type::StringLiteral(name.clone()))
                            .collect();
                        if literals.len() == 1 {
                            literals.into_iter().next().unwrap()
                        } else {
                            Type::Union(literals)
                        }
                    }
                    _ => Type::String, // fallback
                }
            }
            TypeAnnKind::FunctionType {
                params,
                return_type,
            } => Type::Function {
                params: params
                    .iter()
                    .map(|p| self.resolve_type_annotation(p))
                    .collect(),
                return_type: Box::new(self.resolve_type_annotation(return_type)),
            },
            TypeAnnKind::Tuple(elements) => Type::Tuple(
                elements
                    .iter()
                    .map(|e| self.resolve_type_annotation(e))
                    .collect(),
            ),
            TypeAnnKind::Generic { name, type_args } => {
                // Resolve generic type alias: IsNumber<number>
                if let Some((tp_names, body)) = self.generic_type_aliases.get(name).cloned() {
                    // Resolve type args, then substitute into the body
                    let mut subs: HashMap<String, Type> = HashMap::new();
                    for (tp_name, arg) in tp_names.iter().zip(type_args.iter()) {
                        subs.insert(tp_name.clone(), self.resolve_type_annotation(arg));
                    }
                    // Temporarily set type_param_types for the body resolution
                    // Since resolve_type_annotation takes &self, we need a workaround:
                    // Build a simple inline resolver for the body using the substitution map
                    return self.resolve_generic_type_body(&body, &subs);
                }
                // Fall back to named type lookup
                if let Some(ty) = self.interface_types.get(name) {
                    return ty.clone();
                }
                Type::Unknown
            }
            TypeAnnKind::Conditional {
                check_type,
                extends_type,
                true_type,
                false_type,
            } => {
                let check = self.resolve_type_annotation(check_type);
                let extends = self.resolve_type_annotation(extends_type);
                if self.is_assignable(&check, &extends) {
                    self.resolve_type_annotation(true_type)
                } else {
                    self.resolve_type_annotation(false_type)
                }
            }
            TypeAnnKind::Mapped { .. } => {
                // Mapped types produce no runtime values — resolve to Unknown
                Type::Unknown
            }
            TypeAnnKind::IndexedAccess {
                object_type,
                index_type,
            } => {
                let obj = self.resolve_type_annotation(object_type);
                let idx = self.resolve_type_annotation(index_type);
                // T[P] where T is an object type and P is a string literal → field type
                if let Type::Object { ref fields } = obj {
                    if let Type::StringLiteral(ref key) = idx {
                        for (name, ty) in fields {
                            if name == key {
                                return ty.clone();
                            }
                        }
                    }
                }
                Type::Unknown
            }
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

    /// Resolve a type annotation body with type parameter substitutions.
    /// Used for generic type alias instantiation (e.g., IsNumber<number>).
    fn resolve_generic_type_body(
        &self,
        ann: &TypeAnnotation,
        subs: &HashMap<String, Type>,
    ) -> Type {
        match &ann.kind {
            TypeAnnKind::Named(name) => {
                if let Some(ty) = subs.get(name) {
                    return ty.clone();
                }
                self.resolve_type_annotation(ann)
            }
            TypeAnnKind::Conditional {
                check_type,
                extends_type,
                true_type,
                false_type,
            } => {
                let check = self.resolve_generic_type_body(check_type, subs);
                let extends = self.resolve_generic_type_body(extends_type, subs);
                if self.is_assignable(&check, &extends) {
                    self.resolve_generic_type_body(true_type, subs)
                } else {
                    self.resolve_generic_type_body(false_type, subs)
                }
            }
            // For other kinds, delegate to the normal resolver (subs won't apply)
            _ => self.resolve_type_annotation(ann),
        }
    }

    /// Detect `typeof x === "type"` or `typeof x !== "type"` pattern.
    /// Returns (variable_name, type_string, is_equality).
    fn detect_typeof_narrowing(condition: &Expr) -> Option<(String, String, bool)> {
        if let ExprKind::Binary { left, op, right } = &condition.kind {
            if matches!(op, BinOp::StrictEqual | BinOp::StrictNotEqual) {
                let is_eq = *op == BinOp::StrictEqual;
                // typeof x === "type"
                if let ExprKind::Typeof { operand } = &left.kind {
                    if let ExprKind::Identifier(name) = &operand.kind {
                        if let ExprKind::StringLiteral(type_str) = &right.kind {
                            return Some((name.clone(), type_str.clone(), is_eq));
                        }
                    }
                }
                // "type" === typeof x
                if let ExprKind::Typeof { operand } = &right.kind {
                    if let ExprKind::Identifier(name) = &operand.kind {
                        if let ExprKind::StringLiteral(type_str) = &left.kind {
                            return Some((name.clone(), type_str.clone(), is_eq));
                        }
                    }
                }
            }
        }
        None
    }

    /// Map a typeof string to a Type.
    fn type_string_to_type(s: &str) -> Type {
        match s {
            "number" => Type::Number,
            "string" => Type::String,
            "boolean" => Type::Boolean,
            "object" => Type::Object { fields: Vec::new() },
            "function" => Type::Function {
                params: Vec::new(),
                return_type: Box::new(Type::Void),
            },
            _ => Type::Unknown,
        }
    }

    /// Check if a type matches the same category as a target (for narrowing filtering).
    fn is_same_type_category(ty: &Type, target: &Type) -> bool {
        matches!(
            (ty, target),
            (
                Type::Number | Type::NumberLiteral(_),
                Type::Number | Type::NumberLiteral(_)
            ) | (
                Type::String | Type::StringLiteral(_),
                Type::String | Type::StringLiteral(_)
            ) | (Type::Boolean, Type::Boolean)
        )
    }
}
