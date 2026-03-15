use std::collections::{HashMap, HashSet};
use std::path::Path;

use inkwell::attributes::{Attribute, AttributeLoc};
use inkwell::builder::Builder;
use inkwell::context::Context;
use inkwell::module::Module;
use inkwell::passes::PassBuilderOptions;
use inkwell::targets::{
    CodeModel, FileType, InitializationConfig, RelocMode, Target, TargetMachine,
};
use inkwell::types::{BasicMetadataTypeEnum, BasicType, BasicTypeEnum, StructType};
use inkwell::values::{BasicMetadataValueEnum, BasicValueEnum, FunctionValue, PointerValue};
use inkwell::OptimizationLevel;
use inkwell::{AddressSpace, FloatPredicate, IntPredicate};

use crate::diagnostics::CompileError;
use crate::lexer::token::Span;
use crate::parser::ast::*;

pub struct Codegen<'ctx> {
    context: &'ctx Context,
    module: Module<'ctx>,
    builder: Builder<'ctx>,
    variables: Vec<HashMap<String, (PointerValue<'ctx>, VarType)>>,
    functions: HashMap<String, FunctionValue<'ctx>>,
    string_type: StructType<'ctx>,
    /// If true, don't generate a main() — this is a library module
    pub is_library: bool,
    /// Functions whose number params/returns are compiled as i64
    integer_functions: HashSet<String>,
    /// Current number compilation mode (Number=f64, Integer=i64)
    number_mode: VarType,
}

#[derive(Debug, Clone)]
enum VarType {
    Number,
    Integer,
    String,
    Boolean,
}

impl<'ctx> Codegen<'ctx> {
    pub fn new(context: &'ctx Context, module_name: &str) -> Self {
        let module = context.create_module(module_name);
        let builder = context.create_builder();

        let string_type = context.struct_type(
            &[
                context.ptr_type(AddressSpace::default()).into(),
                context.i64_type().into(),
            ],
            false,
        );

        let mut codegen = Self {
            context,
            module,
            builder,
            variables: Vec::new(),
            functions: HashMap::new(),
            string_type,
            is_library: false,
            integer_functions: HashSet::new(),
            number_mode: VarType::Number,
        };

        codegen.declare_runtime_functions();
        codegen
    }

    fn declare_runtime_functions(&mut self) {
        let f64_type = self.context.f64_type();
        let void_type = self.context.void_type();
        let ptr_type = self.context.ptr_type(AddressSpace::default());
        let i64_type = self.context.i64_type();
        let i1_type = self.context.bool_type();

        // --- Print functions ---
        self.module.add_function(
            "mango_print_number",
            void_type.fn_type(&[f64_type.into()], false),
            None,
        );
        self.module.add_function(
            "mango_print_string",
            void_type.fn_type(&[ptr_type.into(), i64_type.into()], false),
            None,
        );
        self.module.add_function(
            "mango_print_boolean",
            void_type.fn_type(&[i1_type.into()], false),
            None,
        );
        self.module
            .add_function("mango_print_null", void_type.fn_type(&[], false), None);
        self.module
            .add_function("mango_print_undefined", void_type.fn_type(&[], false), None);
        self.module
            .add_function("mango_print_newline", void_type.fn_type(&[], false), None);

        // --- Stderr print (console.error / console.warn) ---
        self.module.add_function(
            "mango_eprint_number",
            void_type.fn_type(&[f64_type.into()], false),
            None,
        );
        self.module.add_function(
            "mango_eprint_string",
            void_type.fn_type(&[ptr_type.into(), i64_type.into()], false),
            None,
        );
        self.module.add_function(
            "mango_eprint_boolean",
            void_type.fn_type(&[i1_type.into()], false),
            None,
        );
        self.module
            .add_function("mango_eprint_newline", void_type.fn_type(&[], false), None);

        // --- String operations ---
        self.module.add_function(
            "mango_string_concat",
            self.string_type.fn_type(
                &[
                    ptr_type.into(),
                    i64_type.into(),
                    ptr_type.into(),
                    i64_type.into(),
                ],
                false,
            ),
            None,
        );
        self.module.add_function(
            "mango_number_to_string",
            self.string_type.fn_type(&[f64_type.into()], false),
            None,
        );
        self.module.add_function(
            "mango_boolean_to_string",
            self.string_type.fn_type(&[i1_type.into()], false),
            None,
        );

        // --- String methods ---
        self.module.add_function(
            "mango_string_toUpperCase",
            self.string_type
                .fn_type(&[ptr_type.into(), i64_type.into()], false),
            None,
        );
        self.module.add_function(
            "mango_string_toLowerCase",
            self.string_type
                .fn_type(&[ptr_type.into(), i64_type.into()], false),
            None,
        );
        self.module.add_function(
            "mango_string_charAt",
            self.string_type
                .fn_type(&[ptr_type.into(), i64_type.into(), f64_type.into()], false),
            None,
        );
        self.module.add_function(
            "mango_string_indexOf",
            f64_type.fn_type(
                &[
                    ptr_type.into(),
                    i64_type.into(),
                    ptr_type.into(),
                    i64_type.into(),
                ],
                false,
            ),
            None,
        );
        self.module.add_function(
            "mango_string_includes",
            i1_type.fn_type(
                &[
                    ptr_type.into(),
                    i64_type.into(),
                    ptr_type.into(),
                    i64_type.into(),
                ],
                false,
            ),
            None,
        );
        self.module.add_function(
            "mango_string_substring",
            self.string_type.fn_type(
                &[
                    ptr_type.into(),
                    i64_type.into(),
                    f64_type.into(),
                    f64_type.into(),
                ],
                false,
            ),
            None,
        );
        self.module.add_function(
            "mango_string_slice",
            self.string_type.fn_type(
                &[
                    ptr_type.into(),
                    i64_type.into(),
                    f64_type.into(),
                    f64_type.into(),
                ],
                false,
            ),
            None,
        );
        self.module.add_function(
            "mango_string_trim",
            self.string_type
                .fn_type(&[ptr_type.into(), i64_type.into()], false),
            None,
        );

        // --- Math functions ---
        let math_1 = f64_type.fn_type(&[f64_type.into()], false);
        let math_2 = f64_type.fn_type(&[f64_type.into(), f64_type.into()], false);
        let math_0 = f64_type.fn_type(&[], false);
        for name in &[
            "floor", "ceil", "round", "abs", "sqrt", "sin", "cos", "tan", "log", "exp",
        ] {
            self.module
                .add_function(&format!("mango_math_{}", name), math_1, None);
        }
        for name in &["pow", "min", "max"] {
            self.module
                .add_function(&format!("mango_math_{}", name), math_2, None);
        }
        self.module.add_function("mango_math_random", math_0, None);

        // --- Global functions ---
        self.module.add_function(
            "mango_parseInt",
            f64_type.fn_type(&[ptr_type.into(), i64_type.into()], false),
            None,
        );
        self.module.add_function(
            "mango_parseFloat",
            f64_type.fn_type(&[ptr_type.into(), i64_type.into()], false),
            None,
        );
    }

    // --- Integer narrowing analysis ---

    fn analyze_integer_functions(program: &Program) -> HashSet<String> {
        let mut result = HashSet::new();
        for stmt in &program.statements {
            if let StmtKind::FunctionDecl { name, body, .. } = &stmt.kind {
                if Self::is_function_integer_safe(name, body, &result) {
                    result.insert(name.clone());
                }
            }
        }
        result
    }

    fn is_function_integer_safe(name: &str, body: &[Statement], known: &HashSet<String>) -> bool {
        body.iter()
            .all(|s| Self::is_stmt_integer_safe(s, name, known))
    }

    fn is_stmt_integer_safe(stmt: &Statement, fn_name: &str, known: &HashSet<String>) -> bool {
        match &stmt.kind {
            StmtKind::VariableDecl { initializer, .. } => initializer
                .as_ref()
                .map_or(true, |e| Self::is_expr_integer_safe(e, fn_name, known)),
            StmtKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                Self::is_expr_integer_safe(condition, fn_name, known)
                    && then_branch
                        .iter()
                        .all(|s| Self::is_stmt_integer_safe(s, fn_name, known))
                    && else_branch.as_ref().map_or(true, |b| {
                        b.iter()
                            .all(|s| Self::is_stmt_integer_safe(s, fn_name, known))
                    })
            }
            StmtKind::While { condition, body } => {
                Self::is_expr_integer_safe(condition, fn_name, known)
                    && body
                        .iter()
                        .all(|s| Self::is_stmt_integer_safe(s, fn_name, known))
            }
            StmtKind::For {
                init,
                condition,
                update,
                body,
            } => {
                init.as_ref()
                    .map_or(true, |s| Self::is_stmt_integer_safe(s, fn_name, known))
                    && condition
                        .as_ref()
                        .map_or(true, |e| Self::is_expr_integer_safe(e, fn_name, known))
                    && update
                        .as_ref()
                        .map_or(true, |e| Self::is_expr_integer_safe(e, fn_name, known))
                    && body
                        .iter()
                        .all(|s| Self::is_stmt_integer_safe(s, fn_name, known))
            }
            StmtKind::Return { value } => value
                .as_ref()
                .map_or(true, |e| Self::is_expr_integer_safe(e, fn_name, known)),
            StmtKind::Expression { expr } => Self::is_expr_integer_safe(expr, fn_name, known),
            StmtKind::Block { statements } => statements
                .iter()
                .all(|s| Self::is_stmt_integer_safe(s, fn_name, known)),
            StmtKind::FunctionDecl { .. } | StmtKind::Import { .. } => true,
        }
    }

    fn is_expr_integer_safe(expr: &Expr, fn_name: &str, known: &HashSet<String>) -> bool {
        match &expr.kind {
            ExprKind::NumberLiteral(n) => n.fract() == 0.0,
            ExprKind::BooleanLiteral(_)
            | ExprKind::NullLiteral
            | ExprKind::UndefinedLiteral
            | ExprKind::Identifier(_) => true,
            ExprKind::Binary { left, op, right } => {
                // Division can produce floats
                if matches!(op, BinOp::Divide) {
                    return false;
                }
                Self::is_expr_integer_safe(left, fn_name, known)
                    && Self::is_expr_integer_safe(right, fn_name, known)
            }
            ExprKind::Unary { operand, .. } => Self::is_expr_integer_safe(operand, fn_name, known),
            ExprKind::Call { callee, args } => match &callee.kind {
                // Self-recursive or known integer function
                ExprKind::Identifier(name) if name == fn_name || known.contains(name) => args
                    .iter()
                    .all(|a| Self::is_expr_integer_safe(a, fn_name, known)),
                _ => false,
            },
            ExprKind::Assignment { value, .. } => Self::is_expr_integer_safe(value, fn_name, known),
            ExprKind::Grouping { expr } => Self::is_expr_integer_safe(expr, fn_name, known),
            ExprKind::PostfixUpdate { .. } | ExprKind::PrefixUpdate { .. } => true,
            // String ops, member access, typeof, arrow fns → not integer-safe
            _ => false,
        }
    }

    pub fn compile(&mut self, program: &Program) -> Result<(), CompileError> {
        // Analysis pass: detect functions that can use i64 instead of f64
        self.integer_functions = Self::analyze_integer_functions(program);

        // First pass: compile all function declarations
        for stmt in &program.statements {
            if let StmtKind::FunctionDecl {
                name,
                params,
                return_type,
                body,
                ..
            } = &stmt.kind
            {
                self.compile_function_decl(name, params, return_type, body)?;
            }
        }

        if self.is_library {
            // Library modules: compile top-level variable declarations as globals
            // (skip for now — top-level code in libraries requires init functions)
            return Ok(());
        }

        // Create main function
        let i32_type = self.context.i32_type();
        let main_fn_type = i32_type.fn_type(&[], false);
        let main_fn = self.module.add_function("main", main_fn_type, None);
        let nounwind_id = Attribute::get_named_enum_kind_id("nounwind");
        main_fn.add_attribute(
            AttributeLoc::Function,
            self.context.create_enum_attribute(nounwind_id, 0),
        );
        let entry = self.context.append_basic_block(main_fn, "entry");
        self.builder.position_at_end(entry);

        self.push_scope();

        for stmt in &program.statements {
            // Skip function declarations (already compiled)
            if matches!(&stmt.kind, StmtKind::FunctionDecl { .. }) {
                continue;
            }
            self.compile_statement(stmt, main_fn)?;
        }

        if self
            .builder
            .get_insert_block()
            .unwrap()
            .get_terminator()
            .is_none()
        {
            self.builder
                .build_return(Some(&i32_type.const_int(0, false)))
                .unwrap();
        }

        self.pop_scope();
        Ok(())
    }

    fn compile_statement(
        &mut self,
        stmt: &Statement,
        function: FunctionValue<'ctx>,
    ) -> Result<(), CompileError> {
        match &stmt.kind {
            StmtKind::VariableDecl {
                name,
                initializer,
                type_ann,
                ..
            } => {
                let (alloca, var_type) = if let Some(init) = initializer {
                    let (val, vt) = self.compile_expr(init, function)?;
                    let alloca = self.create_alloca(function, &vt, name);
                    self.builder.build_store(alloca, val).unwrap();
                    (alloca, vt)
                } else {
                    let vt = type_ann
                        .as_ref()
                        .map(|ann| self.type_ann_to_var_type(ann))
                        .unwrap_or(VarType::Number);
                    let alloca = self.create_alloca(function, &vt, name);
                    let default_val = self.default_value(&vt);
                    self.builder.build_store(alloca, default_val).unwrap();
                    (alloca, vt)
                };
                self.set_variable(name.clone(), alloca, var_type);
                Ok(())
            }

            StmtKind::FunctionDecl { .. } => {
                // Already compiled in first pass
                Ok(())
            }

            StmtKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                let (cond_val, _) = self.compile_expr(condition, function)?;
                let cond_bool = self.to_bool(cond_val)?;

                let then_bb = self.context.append_basic_block(function, "then");
                let else_bb = self.context.append_basic_block(function, "else");
                let merge_bb = self.context.append_basic_block(function, "merge");

                self.builder
                    .build_conditional_branch(cond_bool.into_int_value(), then_bb, else_bb)
                    .unwrap();

                self.builder.position_at_end(then_bb);
                self.push_scope();
                for s in then_branch {
                    self.compile_statement(s, function)?;
                }
                self.pop_scope();
                if self
                    .builder
                    .get_insert_block()
                    .unwrap()
                    .get_terminator()
                    .is_none()
                {
                    self.builder.build_unconditional_branch(merge_bb).unwrap();
                }

                self.builder.position_at_end(else_bb);
                if let Some(else_stmts) = else_branch {
                    self.push_scope();
                    for s in else_stmts {
                        self.compile_statement(s, function)?;
                    }
                    self.pop_scope();
                }
                if self
                    .builder
                    .get_insert_block()
                    .unwrap()
                    .get_terminator()
                    .is_none()
                {
                    self.builder.build_unconditional_branch(merge_bb).unwrap();
                }

                self.builder.position_at_end(merge_bb);
                Ok(())
            }

            StmtKind::While { condition, body } => {
                let cond_bb = self.context.append_basic_block(function, "while.cond");
                let body_bb = self.context.append_basic_block(function, "while.body");
                let exit_bb = self.context.append_basic_block(function, "while.exit");

                self.builder.build_unconditional_branch(cond_bb).unwrap();
                self.builder.position_at_end(cond_bb);
                let (cond_val, _) = self.compile_expr(condition, function)?;
                let cond_bool = self.to_bool(cond_val)?;
                self.builder
                    .build_conditional_branch(cond_bool.into_int_value(), body_bb, exit_bb)
                    .unwrap();

                self.builder.position_at_end(body_bb);
                self.push_scope();
                for s in body {
                    self.compile_statement(s, function)?;
                }
                self.pop_scope();
                if self
                    .builder
                    .get_insert_block()
                    .unwrap()
                    .get_terminator()
                    .is_none()
                {
                    self.builder.build_unconditional_branch(cond_bb).unwrap();
                }

                self.builder.position_at_end(exit_bb);
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
                    self.compile_statement(init, function)?;
                }

                let cond_bb = self.context.append_basic_block(function, "for.cond");
                let body_bb = self.context.append_basic_block(function, "for.body");
                let update_bb = self.context.append_basic_block(function, "for.update");
                let exit_bb = self.context.append_basic_block(function, "for.exit");

                self.builder.build_unconditional_branch(cond_bb).unwrap();

                self.builder.position_at_end(cond_bb);
                if let Some(cond) = condition {
                    let (cond_val, _) = self.compile_expr(cond, function)?;
                    let cond_bool = self.to_bool(cond_val)?;
                    self.builder
                        .build_conditional_branch(cond_bool.into_int_value(), body_bb, exit_bb)
                        .unwrap();
                } else {
                    self.builder.build_unconditional_branch(body_bb).unwrap();
                }

                self.builder.position_at_end(body_bb);
                for s in body {
                    self.compile_statement(s, function)?;
                }
                if self
                    .builder
                    .get_insert_block()
                    .unwrap()
                    .get_terminator()
                    .is_none()
                {
                    self.builder.build_unconditional_branch(update_bb).unwrap();
                }

                self.builder.position_at_end(update_bb);
                if let Some(upd) = update {
                    self.compile_expr(upd, function)?;
                }
                self.builder.build_unconditional_branch(cond_bb).unwrap();

                self.builder.position_at_end(exit_bb);
                self.pop_scope();
                Ok(())
            }

            StmtKind::Return { value } => {
                if let Some(val) = value {
                    let (ret_val, _) = self.compile_expr(val, function)?;
                    self.builder.build_return(Some(&ret_val)).unwrap();
                } else {
                    self.builder.build_return(None).unwrap();
                }
                Ok(())
            }

            StmtKind::Expression { expr } => {
                self.compile_expr(expr, function)?;
                Ok(())
            }

            StmtKind::Block { statements } => {
                self.push_scope();
                for s in statements {
                    self.compile_statement(s, function)?;
                }
                self.pop_scope();
                Ok(())
            }

            StmtKind::Import { .. } => Ok(()),
        }
    }

    fn compile_function_decl(
        &mut self,
        name: &str,
        params: &[Parameter],
        return_type: &Option<TypeAnnotation>,
        body: &[Statement],
    ) -> Result<(), CompileError> {
        // Switch to integer mode if this function was analyzed as integer-safe
        let saved_mode = self.number_mode.clone();
        if self.integer_functions.contains(name) {
            self.number_mode = VarType::Integer;
        }

        let param_types: Vec<VarType> = params
            .iter()
            .map(|p| {
                p.type_ann
                    .as_ref()
                    .map(|ann| self.type_ann_to_var_type(ann))
                    .unwrap_or_else(|| self.number_mode.clone())
            })
            .collect();

        let llvm_param_types: Vec<BasicMetadataTypeEnum<'ctx>> = param_types
            .iter()
            .map(|vt| self.var_type_to_llvm(vt).into())
            .collect();

        let ret_vt = return_type
            .as_ref()
            .map(|ann| self.type_ann_to_var_type(ann));

        let fn_type = match &ret_vt {
            Some(vt) => self.var_type_to_llvm(vt).fn_type(&llvm_param_types, false),
            None => self.context.void_type().fn_type(&llvm_param_types, false),
        };

        let function = self.module.add_function(name, fn_type, None);
        self.functions.insert(name.to_string(), function);

        // Mark function as nounwind (no exceptions) to enable better optimization
        let nounwind_id = Attribute::get_named_enum_kind_id("nounwind");
        function.add_attribute(
            AttributeLoc::Function,
            self.context.create_enum_attribute(nounwind_id, 0),
        );

        let entry = self.context.append_basic_block(function, "entry");
        let current_bb = self.builder.get_insert_block();

        self.builder.position_at_end(entry);
        self.push_scope();

        for (i, (param, vt)) in params.iter().zip(param_types.iter()).enumerate() {
            let param_val = function.get_nth_param(i as u32).unwrap();
            let alloca = self.create_alloca(function, vt, &param.name);
            self.builder.build_store(alloca, param_val).unwrap();
            self.set_variable(param.name.clone(), alloca, vt.clone());
        }

        for stmt in body {
            self.compile_statement(stmt, function)?;
        }

        if self
            .builder
            .get_insert_block()
            .unwrap()
            .get_terminator()
            .is_none()
        {
            match &ret_vt {
                Some(vt) => {
                    let default_val = self.default_value(vt);
                    self.builder.build_return(Some(&default_val)).unwrap();
                }
                None => {
                    self.builder.build_return(None).unwrap();
                }
            }
        }

        self.pop_scope();
        self.number_mode = saved_mode;

        if let Some(bb) = current_bb {
            self.builder.position_at_end(bb);
        }

        Ok(())
    }

    fn compile_expr(
        &mut self,
        expr: &Expr,
        function: FunctionValue<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, VarType), CompileError> {
        match &expr.kind {
            ExprKind::NumberLiteral(n) => {
                if matches!(self.number_mode, VarType::Integer) && n.fract() == 0.0 {
                    let val = *n as i64;
                    Ok((
                        self.context
                            .i64_type()
                            .const_int(val as u64, val < 0)
                            .into(),
                        VarType::Integer,
                    ))
                } else {
                    Ok((
                        self.context.f64_type().const_float(*n).into(),
                        VarType::Number,
                    ))
                }
            }

            ExprKind::StringLiteral(s) => Ok((self.create_string_literal(s), VarType::String)),

            ExprKind::BooleanLiteral(b) => Ok((
                self.context.bool_type().const_int(*b as u64, false).into(),
                VarType::Boolean,
            )),

            ExprKind::NullLiteral | ExprKind::UndefinedLiteral => Ok((
                self.context.f64_type().const_float(0.0).into(),
                VarType::Number,
            )),

            ExprKind::Identifier(name) => {
                let (ptr, vt) = self.get_variable(name).ok_or_else(|| {
                    CompileError::error(format!("Undefined variable '{}'", name), expr.span.clone())
                })?;
                let llvm_type = self.var_type_to_llvm(&vt);
                let val = self.builder.build_load(llvm_type, ptr, name).unwrap();
                Ok((val, vt))
            }

            ExprKind::Binary { left, op, right } => self.compile_binary(left, *op, right, function),

            ExprKind::Unary { op, operand } => {
                let (val, vt) = self.compile_expr(operand, function)?;
                match op {
                    UnaryOp::Negate => {
                        if matches!(vt, VarType::Integer) {
                            let result = self
                                .builder
                                .build_int_neg(val.into_int_value(), "neg")
                                .unwrap();
                            Ok((result.into(), VarType::Integer))
                        } else {
                            let result = self
                                .builder
                                .build_float_neg(val.into_float_value(), "neg")
                                .unwrap();
                            Ok((result.into(), VarType::Number))
                        }
                    }
                    UnaryOp::Not => {
                        let bool_val = self.to_bool(val)?;
                        let result = self
                            .builder
                            .build_not(bool_val.into_int_value(), "not")
                            .unwrap();
                        Ok((result.into(), VarType::Boolean))
                    }
                }
            }

            ExprKind::Typeof { operand } => self.compile_typeof(operand, function),

            ExprKind::Call { callee, args } => {
                self.compile_call(callee, args, function, &expr.span)
            }

            ExprKind::Member { object, property } => {
                self.compile_member_access(object, property, function, &expr.span)
            }

            ExprKind::Assignment { name, value } => {
                let (val, val_type) = self.compile_expr(value, function)?;
                let (ptr, _) = self.get_variable(name).ok_or_else(|| {
                    CompileError::error(format!("Undefined variable '{}'", name), expr.span.clone())
                })?;
                self.builder.build_store(ptr, val).unwrap();
                Ok((val, val_type))
            }

            ExprKind::Grouping { expr } => self.compile_expr(expr, function),

            ExprKind::PostfixUpdate { name, op } | ExprKind::PrefixUpdate { name, op } => {
                let (ptr, vt) = self.get_variable(name).ok_or_else(|| {
                    CompileError::error(format!("Undefined variable '{}'", name), expr.span.clone())
                })?;
                let llvm_type = self.var_type_to_llvm(&vt);
                let old_val = self.builder.build_load(llvm_type, ptr, name).unwrap();

                if matches!(vt, VarType::Integer) {
                    let old_int = old_val.into_int_value();
                    let one = self.context.i64_type().const_int(1, false);
                    let new_val = match op {
                        UpdateOp::Increment => {
                            self.builder.build_int_add(old_int, one, "inc").unwrap()
                        }
                        UpdateOp::Decrement => {
                            self.builder.build_int_sub(old_int, one, "dec").unwrap()
                        }
                    };
                    self.builder.build_store(ptr, new_val).unwrap();
                    let result = match &expr.kind {
                        ExprKind::PostfixUpdate { .. } => old_int,
                        _ => new_val,
                    };
                    Ok((result.into(), VarType::Integer))
                } else {
                    let old_float = old_val.into_float_value();
                    let one = self.context.f64_type().const_float(1.0);
                    let new_val = match op {
                        UpdateOp::Increment => {
                            self.builder.build_float_add(old_float, one, "inc").unwrap()
                        }
                        UpdateOp::Decrement => {
                            self.builder.build_float_sub(old_float, one, "dec").unwrap()
                        }
                    };
                    self.builder.build_store(ptr, new_val).unwrap();
                    let result = match &expr.kind {
                        ExprKind::PostfixUpdate { .. } => old_float,
                        _ => new_val,
                    };
                    Ok((result.into(), VarType::Number))
                }
            }

            ExprKind::ArrowFunction { .. } => Err(CompileError::error(
                "Arrow functions as expressions not yet supported in codegen",
                expr.span.clone(),
            )),
        }
    }

    fn compile_typeof(
        &mut self,
        operand: &Expr,
        function: FunctionValue<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, VarType), CompileError> {
        let (_val, vt) = self.compile_expr(operand, function)?;
        let type_str = match vt {
            VarType::Number | VarType::Integer => "number",
            VarType::String => "string",
            VarType::Boolean => "boolean",
        };
        Ok((self.create_string_literal(type_str), VarType::String))
    }

    fn compile_member_access(
        &mut self,
        object: &Expr,
        property: &str,
        function: FunctionValue<'ctx>,
        span: &Span,
    ) -> Result<(BasicValueEnum<'ctx>, VarType), CompileError> {
        // Math constants
        if let ExprKind::Identifier(name) = &object.kind {
            if name == "Math" {
                let val = match property {
                    "PI" => std::f64::consts::PI,
                    "E" => std::f64::consts::E,
                    "LN2" => std::f64::consts::LN_2,
                    "LN10" => std::f64::consts::LN_10,
                    "SQRT2" => std::f64::consts::SQRT_2,
                    _ => {
                        return Err(CompileError::error(
                            format!("Cannot access Math.{} as a property", property),
                            span.clone(),
                        ));
                    }
                };
                return Ok((
                    self.context.f64_type().const_float(val).into(),
                    VarType::Number,
                ));
            }
        }

        // String .length
        let (obj_val, obj_vt) = self.compile_expr(object, function)?;
        if matches!(obj_vt, VarType::String) && property == "length" {
            let len = self
                .builder
                .build_extract_value(obj_val.into_struct_value(), 1, "strlen")
                .unwrap();
            let len_f64 = self
                .builder
                .build_signed_int_to_float(len.into_int_value(), self.context.f64_type(), "lenf")
                .unwrap();
            return Ok((len_f64.into(), VarType::Number));
        }

        Err(CompileError::error(
            format!(
                "Standalone member access '.{}' not supported in this context",
                property
            ),
            span.clone(),
        ))
    }

    fn compile_binary(
        &mut self,
        left: &Expr,
        op: BinOp,
        right: &Expr,
        function: FunctionValue<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, VarType), CompileError> {
        let (left_val, left_vt) = self.compile_expr(left, function)?;
        let (right_val, right_vt) = self.compile_expr(right, function)?;

        // String concatenation
        if op == BinOp::Add
            && (matches!(left_vt, VarType::String) || matches!(right_vt, VarType::String))
        {
            let left_str = self.to_string(left_val, &left_vt)?;
            let right_str = self.to_string(right_val, &right_vt)?;

            let concat_fn = self.module.get_function("mango_string_concat").unwrap();
            let lp = self
                .builder
                .build_extract_value(left_str.into_struct_value(), 0, "lptr")
                .unwrap();
            let ll = self
                .builder
                .build_extract_value(left_str.into_struct_value(), 1, "llen")
                .unwrap();
            let rp = self
                .builder
                .build_extract_value(right_str.into_struct_value(), 0, "rptr")
                .unwrap();
            let rl = self
                .builder
                .build_extract_value(right_str.into_struct_value(), 1, "rlen")
                .unwrap();

            let result = self
                .builder
                .build_call(
                    concat_fn,
                    &[lp.into(), ll.into(), rp.into(), rl.into()],
                    "concat",
                )
                .unwrap()
                .try_as_basic_value()
                .left()
                .unwrap();
            return Ok((result, VarType::String));
        }

        // Integer operations (narrowed from f64 → i64)
        if matches!(left_vt, VarType::Integer) && matches!(right_vt, VarType::Integer) {
            let li = left_val.into_int_value();
            let ri = right_val.into_int_value();

            let result: BasicValueEnum = match op {
                BinOp::Add => self.builder.build_int_add(li, ri, "add").unwrap().into(),
                BinOp::Subtract => self.builder.build_int_sub(li, ri, "sub").unwrap().into(),
                BinOp::Multiply => self.builder.build_int_mul(li, ri, "mul").unwrap().into(),
                BinOp::Modulo => self
                    .builder
                    .build_int_signed_rem(li, ri, "rem")
                    .unwrap()
                    .into(),
                BinOp::Less => self
                    .builder
                    .build_int_compare(IntPredicate::SLT, li, ri, "lt")
                    .unwrap()
                    .into(),
                BinOp::Greater => self
                    .builder
                    .build_int_compare(IntPredicate::SGT, li, ri, "gt")
                    .unwrap()
                    .into(),
                BinOp::LessEqual => self
                    .builder
                    .build_int_compare(IntPredicate::SLE, li, ri, "le")
                    .unwrap()
                    .into(),
                BinOp::GreaterEqual => self
                    .builder
                    .build_int_compare(IntPredicate::SGE, li, ri, "ge")
                    .unwrap()
                    .into(),
                BinOp::Equal | BinOp::StrictEqual => self
                    .builder
                    .build_int_compare(IntPredicate::EQ, li, ri, "eq")
                    .unwrap()
                    .into(),
                BinOp::NotEqual | BinOp::StrictNotEqual => self
                    .builder
                    .build_int_compare(IntPredicate::NE, li, ri, "ne")
                    .unwrap()
                    .into(),
                BinOp::And => {
                    let zero = self.context.i64_type().const_int(0, false);
                    let lb = self
                        .builder
                        .build_int_compare(IntPredicate::NE, li, zero, "lb")
                        .unwrap();
                    let rb = self
                        .builder
                        .build_int_compare(IntPredicate::NE, ri, zero, "rb")
                        .unwrap();
                    self.builder.build_and(lb, rb, "and").unwrap().into()
                }
                BinOp::Or => {
                    let zero = self.context.i64_type().const_int(0, false);
                    let lb = self
                        .builder
                        .build_int_compare(IntPredicate::NE, li, zero, "lb")
                        .unwrap();
                    let rb = self
                        .builder
                        .build_int_compare(IntPredicate::NE, ri, zero, "rb")
                        .unwrap();
                    self.builder.build_or(lb, rb, "or").unwrap().into()
                }
                BinOp::Divide => {
                    // Should not reach here (analysis excludes division)
                    self.builder
                        .build_int_signed_div(li, ri, "div")
                        .unwrap()
                        .into()
                }
            };

            let result_type = match op {
                BinOp::Add | BinOp::Subtract | BinOp::Multiply | BinOp::Divide | BinOp::Modulo => {
                    VarType::Integer
                }
                _ => VarType::Boolean,
            };
            return Ok((result, result_type));
        }

        // Numeric operations (f64)
        if matches!(left_vt, VarType::Number) && matches!(right_vt, VarType::Number) {
            let lf = left_val.into_float_value();
            let rf = right_val.into_float_value();

            let result: BasicValueEnum = match op {
                BinOp::Add => self.builder.build_float_add(lf, rf, "add").unwrap().into(),
                BinOp::Subtract => self.builder.build_float_sub(lf, rf, "sub").unwrap().into(),
                BinOp::Multiply => self.builder.build_float_mul(lf, rf, "mul").unwrap().into(),
                BinOp::Divide => self.builder.build_float_div(lf, rf, "div").unwrap().into(),
                BinOp::Modulo => self.builder.build_float_rem(lf, rf, "rem").unwrap().into(),
                BinOp::Less => self
                    .builder
                    .build_float_compare(FloatPredicate::OLT, lf, rf, "lt")
                    .unwrap()
                    .into(),
                BinOp::Greater => self
                    .builder
                    .build_float_compare(FloatPredicate::OGT, lf, rf, "gt")
                    .unwrap()
                    .into(),
                BinOp::LessEqual => self
                    .builder
                    .build_float_compare(FloatPredicate::OLE, lf, rf, "le")
                    .unwrap()
                    .into(),
                BinOp::GreaterEqual => self
                    .builder
                    .build_float_compare(FloatPredicate::OGE, lf, rf, "ge")
                    .unwrap()
                    .into(),
                BinOp::Equal | BinOp::StrictEqual => self
                    .builder
                    .build_float_compare(FloatPredicate::OEQ, lf, rf, "eq")
                    .unwrap()
                    .into(),
                BinOp::NotEqual | BinOp::StrictNotEqual => self
                    .builder
                    .build_float_compare(FloatPredicate::ONE, lf, rf, "ne")
                    .unwrap()
                    .into(),
                BinOp::And => {
                    let lb = self
                        .builder
                        .build_float_compare(
                            FloatPredicate::ONE,
                            lf,
                            self.context.f64_type().const_float(0.0),
                            "lb",
                        )
                        .unwrap();
                    let rb = self
                        .builder
                        .build_float_compare(
                            FloatPredicate::ONE,
                            rf,
                            self.context.f64_type().const_float(0.0),
                            "rb",
                        )
                        .unwrap();
                    self.builder.build_and(lb, rb, "and").unwrap().into()
                }
                BinOp::Or => {
                    let lb = self
                        .builder
                        .build_float_compare(
                            FloatPredicate::ONE,
                            lf,
                            self.context.f64_type().const_float(0.0),
                            "lb",
                        )
                        .unwrap();
                    let rb = self
                        .builder
                        .build_float_compare(
                            FloatPredicate::ONE,
                            rf,
                            self.context.f64_type().const_float(0.0),
                            "rb",
                        )
                        .unwrap();
                    self.builder.build_or(lb, rb, "or").unwrap().into()
                }
            };

            let result_type = match op {
                BinOp::Add | BinOp::Subtract | BinOp::Multiply | BinOp::Divide | BinOp::Modulo => {
                    VarType::Number
                }
                _ => VarType::Boolean,
            };
            return Ok((result, result_type));
        }

        // Boolean operations
        if matches!(left_vt, VarType::Boolean) && matches!(right_vt, VarType::Boolean) {
            let li = left_val.into_int_value();
            let ri = right_val.into_int_value();
            let result: BasicValueEnum = match op {
                BinOp::Equal | BinOp::StrictEqual => self
                    .builder
                    .build_int_compare(IntPredicate::EQ, li, ri, "eq")
                    .unwrap()
                    .into(),
                BinOp::NotEqual | BinOp::StrictNotEqual => self
                    .builder
                    .build_int_compare(IntPredicate::NE, li, ri, "ne")
                    .unwrap()
                    .into(),
                BinOp::And => self.builder.build_and(li, ri, "and").unwrap().into(),
                BinOp::Or => self.builder.build_or(li, ri, "or").unwrap().into(),
                _ => {
                    return Err(CompileError::error(
                        "Invalid operator for boolean operands",
                        Span::new(0, 0, 0, 0),
                    ))
                }
            };
            return Ok((result, VarType::Boolean));
        }

        Err(CompileError::error(
            "Unsupported binary operation",
            Span::new(0, 0, 0, 0),
        ))
    }

    fn compile_call(
        &mut self,
        callee: &Expr,
        args: &[Expr],
        function: FunctionValue<'ctx>,
        span: &Span,
    ) -> Result<(BasicValueEnum<'ctx>, VarType), CompileError> {
        if let ExprKind::Member { object, property } = &callee.kind {
            if let ExprKind::Identifier(name) = &object.kind {
                // console.log / console.error / console.warn
                if name == "console" {
                    let is_stderr = property == "error" || property == "warn";
                    if property == "log" || is_stderr {
                        return self.compile_console_print(args, function, is_stderr);
                    }
                }
                // Math methods
                if name == "Math" {
                    return self.compile_math_call(property, args, function, span);
                }
            }

            // String methods: object.method(args)
            let (obj_val, obj_vt) = self.compile_expr(object, function)?;
            if matches!(obj_vt, VarType::String) {
                return self.compile_string_method(obj_val, property, args, function, span);
            }
        }

        // Global functions: parseInt, parseFloat
        if let ExprKind::Identifier(name) = &callee.kind {
            if name == "parseInt" || name == "parseFloat" {
                return self.compile_global_func(name, args, function, span);
            }

            let func = self
                .functions
                .get(name)
                .copied()
                .or_else(|| self.module.get_function(name))
                .ok_or_else(|| {
                    CompileError::error(format!("Undefined function '{}'", name), span.clone())
                })?;

            let is_target_integer = self.integer_functions.contains(name.as_str());
            let caller_is_integer = matches!(self.number_mode, VarType::Integer);

            let mut compiled_args: Vec<BasicMetadataValueEnum> = Vec::new();
            for arg in args {
                let (val, vt) = self.compile_expr(arg, function)?;
                // Convert f64 → i64 when calling an integer function from float context
                let val =
                    if is_target_integer && !caller_is_integer && matches!(vt, VarType::Number) {
                        self.builder
                            .build_float_to_signed_int(
                                val.into_float_value(),
                                self.context.i64_type(),
                                "f2i",
                            )
                            .unwrap()
                            .into()
                    } else {
                        val
                    };
                compiled_args.push(val.into());
            }

            let result = self
                .builder
                .build_call(func, &compiled_args, "call")
                .unwrap();

            if let Some(val) = result.try_as_basic_value().left() {
                let ret_vt = if let Some(ret_type) = func.get_type().get_return_type() {
                    if ret_type.is_float_type() {
                        VarType::Number
                    } else if ret_type.is_int_type() {
                        let bit_width = ret_type.into_int_type().get_bit_width();
                        if bit_width == 1 {
                            VarType::Boolean
                        } else {
                            VarType::Integer
                        }
                    } else {
                        VarType::String
                    }
                } else {
                    VarType::Number
                };

                // Convert i64 → f64 when returning from integer function to float context
                if matches!(ret_vt, VarType::Integer) && !caller_is_integer {
                    let float_val = self
                        .builder
                        .build_signed_int_to_float(
                            val.into_int_value(),
                            self.context.f64_type(),
                            "i2f",
                        )
                        .unwrap();
                    return Ok((float_val.into(), VarType::Number));
                }

                Ok((val, ret_vt))
            } else {
                Ok((
                    self.context.f64_type().const_float(0.0).into(),
                    VarType::Number,
                ))
            }
        } else {
            Err(CompileError::error(
                "Only direct function calls are supported",
                span.clone(),
            ))
        }
    }

    fn compile_console_print(
        &mut self,
        args: &[Expr],
        function: FunctionValue<'ctx>,
        is_stderr: bool,
    ) -> Result<(BasicValueEnum<'ctx>, VarType), CompileError> {
        let (print_num, print_str, print_bool, print_nl) = if is_stderr {
            (
                "mango_eprint_number",
                "mango_eprint_string",
                "mango_eprint_boolean",
                "mango_eprint_newline",
            )
        } else {
            (
                "mango_print_number",
                "mango_print_string",
                "mango_print_boolean",
                "mango_print_newline",
            )
        };

        for (i, arg) in args.iter().enumerate() {
            if i > 0 {
                let space = self.create_string_literal(" ");
                let ptr = self
                    .builder
                    .build_extract_value(space.into_struct_value(), 0, "sp")
                    .unwrap();
                let len = self
                    .builder
                    .build_extract_value(space.into_struct_value(), 1, "sl")
                    .unwrap();
                let f = self.module.get_function(print_str).unwrap();
                self.builder
                    .build_call(f, &[ptr.into(), len.into()], "")
                    .unwrap();
            }

            let (val, vt) = self.compile_expr(arg, function)?;
            match vt {
                VarType::Number => {
                    let f = self.module.get_function(print_num).unwrap();
                    self.builder.build_call(f, &[val.into()], "").unwrap();
                }
                VarType::Integer => {
                    // Convert i64 → f64 for printing
                    let float_val = self
                        .builder
                        .build_signed_int_to_float(
                            val.into_int_value(),
                            self.context.f64_type(),
                            "i2f_print",
                        )
                        .unwrap();
                    let f = self.module.get_function(print_num).unwrap();
                    self.builder.build_call(f, &[float_val.into()], "").unwrap();
                }
                VarType::String => {
                    let ptr = self
                        .builder
                        .build_extract_value(val.into_struct_value(), 0, "ptr")
                        .unwrap();
                    let len = self
                        .builder
                        .build_extract_value(val.into_struct_value(), 1, "len")
                        .unwrap();
                    let f = self.module.get_function(print_str).unwrap();
                    self.builder
                        .build_call(f, &[ptr.into(), len.into()], "")
                        .unwrap();
                }
                VarType::Boolean => {
                    let f = self.module.get_function(print_bool).unwrap();
                    self.builder.build_call(f, &[val.into()], "").unwrap();
                }
            }
        }

        let nl = self.module.get_function(print_nl).unwrap();
        self.builder.build_call(nl, &[], "").unwrap();
        Ok((
            self.context.f64_type().const_float(0.0).into(),
            VarType::Number,
        ))
    }

    fn compile_math_call(
        &mut self,
        method: &str,
        args: &[Expr],
        function: FunctionValue<'ctx>,
        span: &Span,
    ) -> Result<(BasicValueEnum<'ctx>, VarType), CompileError> {
        let func_name = format!("mango_math_{}", method);
        let func = self.module.get_function(&func_name).ok_or_else(|| {
            CompileError::error(format!("Unknown Math method '{}'", method), span.clone())
        })?;

        let mut compiled_args: Vec<BasicMetadataValueEnum> = Vec::new();
        for arg in args {
            let (val, _) = self.compile_expr(arg, function)?;
            compiled_args.push(val.into());
        }

        let result = self
            .builder
            .build_call(func, &compiled_args, method)
            .unwrap()
            .try_as_basic_value()
            .left()
            .unwrap();
        Ok((result, VarType::Number))
    }

    fn compile_string_method(
        &mut self,
        obj_val: BasicValueEnum<'ctx>,
        method: &str,
        args: &[Expr],
        function: FunctionValue<'ctx>,
        span: &Span,
    ) -> Result<(BasicValueEnum<'ctx>, VarType), CompileError> {
        let ptr = self
            .builder
            .build_extract_value(obj_val.into_struct_value(), 0, "sptr")
            .unwrap();
        let len = self
            .builder
            .build_extract_value(obj_val.into_struct_value(), 1, "slen")
            .unwrap();

        match method {
            "toUpperCase" | "toLowerCase" | "trim" => {
                let func_name = format!("mango_string_{}", method);
                let func = self.module.get_function(&func_name).unwrap();
                let result = self
                    .builder
                    .build_call(func, &[ptr.into(), len.into()], method)
                    .unwrap()
                    .try_as_basic_value()
                    .left()
                    .unwrap();
                Ok((result, VarType::String))
            }
            "charAt" => {
                let (idx, _) = self.compile_expr(&args[0], function)?;
                let func = self.module.get_function("mango_string_charAt").unwrap();
                let result = self
                    .builder
                    .build_call(func, &[ptr.into(), len.into(), idx.into()], "charAt")
                    .unwrap()
                    .try_as_basic_value()
                    .left()
                    .unwrap();
                Ok((result, VarType::String))
            }
            "indexOf" => {
                let (needle, _) = self.compile_expr(&args[0], function)?;
                let np = self
                    .builder
                    .build_extract_value(needle.into_struct_value(), 0, "np")
                    .unwrap();
                let nl = self
                    .builder
                    .build_extract_value(needle.into_struct_value(), 1, "nl")
                    .unwrap();
                let func = self.module.get_function("mango_string_indexOf").unwrap();
                let result = self
                    .builder
                    .build_call(
                        func,
                        &[ptr.into(), len.into(), np.into(), nl.into()],
                        "indexOf",
                    )
                    .unwrap()
                    .try_as_basic_value()
                    .left()
                    .unwrap();
                Ok((result, VarType::Number))
            }
            "includes" => {
                let (needle, _) = self.compile_expr(&args[0], function)?;
                let np = self
                    .builder
                    .build_extract_value(needle.into_struct_value(), 0, "np")
                    .unwrap();
                let nl = self
                    .builder
                    .build_extract_value(needle.into_struct_value(), 1, "nl")
                    .unwrap();
                let func = self.module.get_function("mango_string_includes").unwrap();
                let result = self
                    .builder
                    .build_call(
                        func,
                        &[ptr.into(), len.into(), np.into(), nl.into()],
                        "includes",
                    )
                    .unwrap()
                    .try_as_basic_value()
                    .left()
                    .unwrap();
                Ok((result, VarType::Boolean))
            }
            "substring" | "slice" => {
                let (start, _) = self.compile_expr(&args[0], function)?;
                let end_val = if args.len() > 1 {
                    self.compile_expr(&args[1], function)?.0
                } else {
                    // Default end = length as f64
                    let len_f64 = self
                        .builder
                        .build_signed_int_to_float(
                            len.into_int_value(),
                            self.context.f64_type(),
                            "lenf",
                        )
                        .unwrap();
                    len_f64.into()
                };
                let func_name = format!("mango_string_{}", method);
                let func = self.module.get_function(&func_name).unwrap();
                let result = self
                    .builder
                    .build_call(
                        func,
                        &[ptr.into(), len.into(), start.into(), end_val.into()],
                        method,
                    )
                    .unwrap()
                    .try_as_basic_value()
                    .left()
                    .unwrap();
                Ok((result, VarType::String))
            }
            _ => Err(CompileError::error(
                format!("Unknown string method '{}'", method),
                span.clone(),
            )),
        }
    }

    fn compile_global_func(
        &mut self,
        name: &str,
        args: &[Expr],
        function: FunctionValue<'ctx>,
        span: &Span,
    ) -> Result<(BasicValueEnum<'ctx>, VarType), CompileError> {
        if args.len() != 1 {
            return Err(CompileError::error(
                format!("{} expects 1 argument, got {}", name, args.len()),
                span.clone(),
            ));
        }
        let (arg_val, _) = self.compile_expr(&args[0], function)?;
        let ptr = self
            .builder
            .build_extract_value(arg_val.into_struct_value(), 0, "p")
            .unwrap();
        let len = self
            .builder
            .build_extract_value(arg_val.into_struct_value(), 1, "l")
            .unwrap();

        let func_name = format!("mango_{}", name);
        let func = self.module.get_function(&func_name).unwrap();
        let result = self
            .builder
            .build_call(func, &[ptr.into(), len.into()], name)
            .unwrap()
            .try_as_basic_value()
            .left()
            .unwrap();
        Ok((result, VarType::Number))
    }

    // --- Helpers ---

    fn create_string_literal(&self, s: &str) -> BasicValueEnum<'ctx> {
        let bytes = s.as_bytes();
        let global = self.builder.build_global_string_ptr(s, "str").unwrap();
        let ptr = global.as_pointer_value();
        let len = self.context.i64_type().const_int(bytes.len() as u64, false);

        let struct_val = self.string_type.const_zero();
        let struct_val = self
            .builder
            .build_insert_value(struct_val, ptr, 0, "str.ptr")
            .unwrap();
        let struct_val = self
            .builder
            .build_insert_value(struct_val.into_struct_value(), len, 1, "str.len")
            .unwrap();
        struct_val.into_struct_value().into()
    }

    fn to_string(
        &self,
        val: BasicValueEnum<'ctx>,
        vt: &VarType,
    ) -> Result<BasicValueEnum<'ctx>, CompileError> {
        match vt {
            VarType::String => Ok(val),
            VarType::Number => {
                let f = self.module.get_function("mango_number_to_string").unwrap();
                Ok(self
                    .builder
                    .build_call(f, &[val.into()], "numstr")
                    .unwrap()
                    .try_as_basic_value()
                    .left()
                    .unwrap())
            }
            VarType::Integer => {
                // Convert i64 → f64, then use number_to_string
                let float_val = self
                    .builder
                    .build_signed_int_to_float(val.into_int_value(), self.context.f64_type(), "i2f")
                    .unwrap();
                let f = self.module.get_function("mango_number_to_string").unwrap();
                Ok(self
                    .builder
                    .build_call(f, &[float_val.into()], "numstr")
                    .unwrap()
                    .try_as_basic_value()
                    .left()
                    .unwrap())
            }
            VarType::Boolean => {
                let f = self.module.get_function("mango_boolean_to_string").unwrap();
                Ok(self
                    .builder
                    .build_call(f, &[val.into()], "boolstr")
                    .unwrap()
                    .try_as_basic_value()
                    .left()
                    .unwrap())
            }
        }
    }

    fn to_bool(&self, val: BasicValueEnum<'ctx>) -> Result<BasicValueEnum<'ctx>, CompileError> {
        if val.is_float_value() {
            let result = self
                .builder
                .build_float_compare(
                    FloatPredicate::ONE,
                    val.into_float_value(),
                    self.context.f64_type().const_float(0.0),
                    "tobool",
                )
                .unwrap();
            Ok(result.into())
        } else if val.is_int_value() {
            let int_val = val.into_int_value();
            if int_val.get_type().get_bit_width() == 1 {
                // Already a boolean (i1)
                Ok(val)
            } else {
                // i64 integer — compare to 0
                let result = self
                    .builder
                    .build_int_compare(
                        IntPredicate::NE,
                        int_val,
                        self.context.i64_type().const_int(0, false),
                        "tobool",
                    )
                    .unwrap();
                Ok(result.into())
            }
        } else {
            let len = self
                .builder
                .build_extract_value(val.into_struct_value(), 1, "slen")
                .unwrap();
            let result = self
                .builder
                .build_int_compare(
                    IntPredicate::SGT,
                    len.into_int_value(),
                    self.context.i64_type().const_int(0, false),
                    "tobool",
                )
                .unwrap();
            Ok(result.into())
        }
    }

    fn create_alloca(
        &self,
        function: FunctionValue<'ctx>,
        vt: &VarType,
        name: &str,
    ) -> PointerValue<'ctx> {
        let builder = self.context.create_builder();
        let entry = function.get_first_basic_block().unwrap();
        match entry.get_first_instruction() {
            Some(inst) => builder.position_before(&inst),
            None => builder.position_at_end(entry),
        }
        builder
            .build_alloca(self.var_type_to_llvm(vt), name)
            .unwrap()
    }

    fn var_type_to_llvm(&self, vt: &VarType) -> BasicTypeEnum<'ctx> {
        match vt {
            VarType::Number => self.context.f64_type().into(),
            VarType::Integer => self.context.i64_type().into(),
            VarType::String => self.string_type.into(),
            VarType::Boolean => self.context.bool_type().into(),
        }
    }

    fn type_ann_to_var_type(&self, ann: &TypeAnnotation) -> VarType {
        match &ann.kind {
            TypeAnnKind::Number => self.number_mode.clone(),
            TypeAnnKind::String => VarType::String,
            TypeAnnKind::Boolean => VarType::Boolean,
            _ => self.number_mode.clone(),
        }
    }

    fn default_value(&self, vt: &VarType) -> BasicValueEnum<'ctx> {
        match vt {
            VarType::Number => self.context.f64_type().const_float(0.0).into(),
            VarType::Integer => self.context.i64_type().const_int(0, false).into(),
            VarType::String => self.create_string_literal(""),
            VarType::Boolean => self.context.bool_type().const_int(0, false).into(),
        }
    }

    fn push_scope(&mut self) {
        self.variables.push(HashMap::new());
    }
    fn pop_scope(&mut self) {
        self.variables.pop();
    }
    fn set_variable(&mut self, name: String, ptr: PointerValue<'ctx>, vt: VarType) {
        if let Some(scope) = self.variables.last_mut() {
            scope.insert(name, (ptr, vt));
        }
    }
    fn get_variable(&self, name: &str) -> Option<(PointerValue<'ctx>, VarType)> {
        for scope in self.variables.iter().rev() {
            if let Some((ptr, vt)) = scope.get(name) {
                return Some((*ptr, vt.clone()));
            }
        }
        None
    }

    // --- Output ---

    /// Run LLVM optimization passes on the module.
    pub fn optimize(&self) -> Result<(), String> {
        let machine = self.create_target_machine(OptimizationLevel::Aggressive)?;

        let options = PassBuilderOptions::create();
        options.set_loop_vectorization(true);
        options.set_loop_slp_vectorization(true);
        options.set_loop_unrolling(true);
        options.set_merge_functions(true);

        self.module
            .run_passes("default<O3>", &machine, options)
            .map_err(|e| e.to_string())
    }

    pub fn write_object_file(&self, path: &Path) -> Result<(), String> {
        let machine = self.create_target_machine(OptimizationLevel::Aggressive)?;
        machine
            .write_to_file(&self.module, FileType::Object, path)
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    fn create_target_machine(&self, opt_level: OptimizationLevel) -> Result<TargetMachine, String> {
        Target::initialize_all(&InitializationConfig::default());
        let triple = TargetMachine::get_default_triple();
        let cpu = TargetMachine::get_host_cpu_name();
        let features = TargetMachine::get_host_cpu_features();
        let target = Target::from_triple(&triple).map_err(|e| e.to_string())?;
        target
            .create_target_machine(
                &triple,
                cpu.to_str().unwrap(),
                features.to_str().unwrap(),
                opt_level,
                RelocMode::Default,
                CodeModel::Default,
            )
            .ok_or_else(|| "Failed to create target machine".to_string())
    }

    pub fn print_ir(&self) -> String {
        self.module.print_to_string().to_string()
    }

    /// Compile a function from an imported module into this LLVM module
    pub fn compile_exported_function(
        &mut self,
        name: &str,
        params: &[Parameter],
        return_type: &Option<TypeAnnotation>,
        body: &[Statement],
    ) -> Result<(), CompileError> {
        self.compile_function_decl(name, params, return_type, body)
    }
}
