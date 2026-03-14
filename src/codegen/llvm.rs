use std::collections::HashMap;
use std::path::Path;

use inkwell::builder::Builder;
use inkwell::context::Context;
use inkwell::module::Module;
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
}

#[derive(Debug, Clone)]
enum VarType {
    Number,
    String,
    Boolean,
}

impl<'ctx> Codegen<'ctx> {
    pub fn new(context: &'ctx Context, module_name: &str) -> Self {
        let module = context.create_module(module_name);
        let builder = context.create_builder();

        // String struct: { i8*, i64 } (pointer, length)
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
        };

        codegen.declare_runtime_functions();
        codegen
    }

    fn declare_runtime_functions(&mut self) {
        // void mango_print_number(double)
        let f64_type = self.context.f64_type();
        let void_type = self.context.void_type();

        let print_number_type = void_type.fn_type(&[f64_type.into()], false);
        self.module
            .add_function("mango_print_number", print_number_type, None);

        // void mango_print_string(i8*, i64)
        let ptr_type = self.context.ptr_type(AddressSpace::default());
        let i64_type = self.context.i64_type();
        let print_string_type = void_type.fn_type(&[ptr_type.into(), i64_type.into()], false);
        self.module
            .add_function("mango_print_string", print_string_type, None);

        // void mango_print_boolean(i1)
        let i1_type = self.context.bool_type();
        let print_bool_type = void_type.fn_type(&[i1_type.into()], false);
        self.module
            .add_function("mango_print_boolean", print_bool_type, None);

        // void mango_print_null()
        let print_null_type = void_type.fn_type(&[], false);
        self.module
            .add_function("mango_print_null", print_null_type, None);

        // void mango_print_undefined()
        let print_undefined_type = void_type.fn_type(&[], false);
        self.module
            .add_function("mango_print_undefined", print_undefined_type, None);

        // { i8*, i64 } mango_string_concat(i8*, i64, i8*, i64)
        let concat_type = self.string_type.fn_type(
            &[
                ptr_type.into(),
                i64_type.into(),
                ptr_type.into(),
                i64_type.into(),
            ],
            false,
        );
        self.module
            .add_function("mango_string_concat", concat_type, None);

        // void mango_print_newline()
        let print_newline_type = void_type.fn_type(&[], false);
        self.module
            .add_function("mango_print_newline", print_newline_type, None);

        // { i8*, i64 } mango_number_to_string(double)
        let num_to_str_type = self.string_type.fn_type(&[f64_type.into()], false);
        self.module
            .add_function("mango_number_to_string", num_to_str_type, None);

        // { i8*, i64 } mango_boolean_to_string(i1)
        let bool_to_str_type = self.string_type.fn_type(&[i1_type.into()], false);
        self.module
            .add_function("mango_boolean_to_string", bool_to_str_type, None);
    }

    pub fn compile(&mut self, program: &Program) -> Result<(), CompileError> {
        // Create main function
        let i32_type = self.context.i32_type();
        let main_fn_type = i32_type.fn_type(&[], false);
        let main_fn = self.module.add_function("main", main_fn_type, None);
        let entry = self.context.append_basic_block(main_fn, "entry");
        self.builder.position_at_end(entry);

        self.push_scope();

        for stmt in &program.statements {
            self.compile_statement(stmt, main_fn)?;
        }

        // Return 0 from main if no explicit return
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
                    // Default initialization based on type annotation
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

            StmtKind::FunctionDecl {
                name,
                params,
                return_type,
                body,
            } => {
                self.compile_function_decl(name, params, return_type, body)?;
                Ok(())
            }

            StmtKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                let (cond_val, _) = self.compile_expr(condition, function)?;
                let cond_bool = self.to_bool(cond_val, function)?;

                let then_bb = self.context.append_basic_block(function, "then");
                let else_bb = self.context.append_basic_block(function, "else");
                let merge_bb = self.context.append_basic_block(function, "merge");

                self.builder
                    .build_conditional_branch(cond_bool.into_int_value(), then_bb, else_bb)
                    .unwrap();

                // Then branch
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

                // Else branch
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

                // Condition
                self.builder.position_at_end(cond_bb);
                let (cond_val, _) = self.compile_expr(condition, function)?;
                let cond_bool = self.to_bool(cond_val, function)?;
                self.builder
                    .build_conditional_branch(cond_bool.into_int_value(), body_bb, exit_bb)
                    .unwrap();

                // Body
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

                // Condition
                self.builder.position_at_end(cond_bb);
                if let Some(cond) = condition {
                    let (cond_val, _) = self.compile_expr(cond, function)?;
                    let cond_bool = self.to_bool(cond_val, function)?;
                    self.builder
                        .build_conditional_branch(cond_bool.into_int_value(), body_bb, exit_bb)
                        .unwrap();
                } else {
                    self.builder.build_unconditional_branch(body_bb).unwrap();
                }

                // Body
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

                // Update
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
        }
    }

    fn compile_function_decl(
        &mut self,
        name: &str,
        params: &[Parameter],
        return_type: &Option<TypeAnnotation>,
        body: &[Statement],
    ) -> Result<(), CompileError> {
        let param_types: Vec<VarType> = params
            .iter()
            .map(|p| {
                p.type_ann
                    .as_ref()
                    .map(|ann| self.type_ann_to_var_type(ann))
                    .unwrap_or(VarType::Number)
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

        let entry = self.context.append_basic_block(function, "entry");

        // Save current position
        let current_bb = self.builder.get_insert_block();

        self.builder.position_at_end(entry);
        self.push_scope();

        // Bind parameters
        for (i, (param, vt)) in params.iter().zip(param_types.iter()).enumerate() {
            let param_val = function.get_nth_param(i as u32).unwrap();
            let alloca = self.create_alloca(function, vt, &param.name);
            self.builder.build_store(alloca, param_val).unwrap();
            self.set_variable(param.name.clone(), alloca, vt.clone());
        }

        for stmt in body {
            self.compile_statement(stmt, function)?;
        }

        // Add default return if needed
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

        // Restore builder position
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
                let val = self.context.f64_type().const_float(*n);
                Ok((val.into(), VarType::Number))
            }

            ExprKind::StringLiteral(s) => {
                let val = self.create_string_literal(s);
                Ok((val, VarType::String))
            }

            ExprKind::BooleanLiteral(b) => {
                let val = self.context.bool_type().const_int(*b as u64, false);
                Ok((val.into(), VarType::Boolean))
            }

            ExprKind::NullLiteral | ExprKind::UndefinedLiteral => {
                // Represent as 0.0 number for now
                let val = self.context.f64_type().const_float(0.0);
                Ok((val.into(), VarType::Number))
            }

            ExprKind::Identifier(name) => {
                let (ptr, vt) = self.get_variable(name).ok_or_else(|| CompileError {
                    message: format!("Undefined variable '{}'", name),
                    span: expr.span.clone(),
                })?;
                let llvm_type = self.var_type_to_llvm(&vt);
                let val = self.builder.build_load(llvm_type, ptr, name).unwrap();
                Ok((val, vt))
            }

            ExprKind::Binary { left, op, right } => self.compile_binary(left, *op, right, function),

            ExprKind::Unary { op, operand } => {
                let (val, _vt) = self.compile_expr(operand, function)?;
                match op {
                    UnaryOp::Negate => {
                        let result = self
                            .builder
                            .build_float_neg(val.into_float_value(), "neg")
                            .unwrap();
                        Ok((result.into(), VarType::Number))
                    }
                    UnaryOp::Not => {
                        let bool_val = self.to_bool(val, function)?;
                        let result = self
                            .builder
                            .build_not(bool_val.into_int_value(), "not")
                            .unwrap();
                        Ok((result.into(), VarType::Boolean))
                    }
                }
            }

            ExprKind::Call { callee, args } => {
                self.compile_call(callee, args, function, &expr.span)
            }

            ExprKind::Member { .. } => {
                // Member expressions are handled as part of Call for console.log
                Err(CompileError {
                    message: "Standalone member access not yet supported".to_string(),
                    span: expr.span.clone(),
                })
            }

            ExprKind::Assignment { name, value } => {
                let (val, val_type) = self.compile_expr(value, function)?;
                let (ptr, _) = self.get_variable(name).ok_or_else(|| CompileError {
                    message: format!("Undefined variable '{}'", name),
                    span: expr.span.clone(),
                })?;
                self.builder.build_store(ptr, val).unwrap();
                Ok((val, val_type))
            }

            ExprKind::Grouping { expr } => self.compile_expr(expr, function),

            ExprKind::PostfixUpdate { name, op } | ExprKind::PrefixUpdate { name, op } => {
                let (ptr, vt) = self.get_variable(name).ok_or_else(|| CompileError {
                    message: format!("Undefined variable '{}'", name),
                    span: expr.span.clone(),
                })?;
                let llvm_type = self.var_type_to_llvm(&vt);
                let old_val = self
                    .builder
                    .build_load(llvm_type, ptr, name)
                    .unwrap()
                    .into_float_value();

                let one = self.context.f64_type().const_float(1.0);
                let new_val = match op {
                    UpdateOp::Increment => {
                        self.builder.build_float_add(old_val, one, "inc").unwrap()
                    }
                    UpdateOp::Decrement => {
                        self.builder.build_float_sub(old_val, one, "dec").unwrap()
                    }
                };
                self.builder.build_store(ptr, new_val).unwrap();

                // Postfix returns old value, prefix returns new
                let result = match &expr.kind {
                    ExprKind::PostfixUpdate { .. } => old_val,
                    _ => new_val,
                };
                Ok((result.into(), VarType::Number))
            }

            ExprKind::ArrowFunction { .. } => Err(CompileError {
                message: "Arrow functions as expressions not yet supported in codegen".to_string(),
                span: expr.span.clone(),
            }),
        }
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
            let left_str = self.to_string(left_val, &left_vt, function)?;
            let right_str = self.to_string(right_val, &right_vt, function)?;

            let concat_fn = self.module.get_function("mango_string_concat").unwrap();

            let left_ptr = self
                .builder
                .build_extract_value(left_str.into_struct_value(), 0, "lptr")
                .unwrap();
            let left_len = self
                .builder
                .build_extract_value(left_str.into_struct_value(), 1, "llen")
                .unwrap();
            let right_ptr = self
                .builder
                .build_extract_value(right_str.into_struct_value(), 0, "rptr")
                .unwrap();
            let right_len = self
                .builder
                .build_extract_value(right_str.into_struct_value(), 1, "rlen")
                .unwrap();

            let result = self
                .builder
                .build_call(
                    concat_fn,
                    &[
                        left_ptr.into(),
                        left_len.into(),
                        right_ptr.into(),
                        right_len.into(),
                    ],
                    "concat",
                )
                .unwrap()
                .try_as_basic_value()
                .left()
                .unwrap();

            return Ok((result, VarType::String));
        }

        // Numeric operations
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
                            "lbool",
                        )
                        .unwrap();
                    let rb = self
                        .builder
                        .build_float_compare(
                            FloatPredicate::ONE,
                            rf,
                            self.context.f64_type().const_float(0.0),
                            "rbool",
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
                            "lbool",
                        )
                        .unwrap();
                    let rb = self
                        .builder
                        .build_float_compare(
                            FloatPredicate::ONE,
                            rf,
                            self.context.f64_type().const_float(0.0),
                            "rbool",
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
                    return Err(CompileError {
                        message: "Invalid operator for boolean operands".to_string(),
                        span: Span::new(0, 0, 0, 0),
                    })
                }
            };

            return Ok((result, VarType::Boolean));
        }

        Err(CompileError {
            message: "Unsupported binary operation".to_string(),
            span: Span::new(0, 0, 0, 0),
        })
    }

    fn compile_call(
        &mut self,
        callee: &Expr,
        args: &[Expr],
        function: FunctionValue<'ctx>,
        span: &Span,
    ) -> Result<(BasicValueEnum<'ctx>, VarType), CompileError> {
        // Handle console.log specially
        if let ExprKind::Member { object, property } = &callee.kind {
            if let ExprKind::Identifier(name) = &object.kind {
                if name == "console" && property == "log" {
                    return self.compile_console_log(args, function, span);
                }
            }
        }

        // Regular function call
        if let ExprKind::Identifier(name) = &callee.kind {
            let func = self
                .functions
                .get(name)
                .copied()
                .or_else(|| self.module.get_function(name))
                .ok_or_else(|| CompileError {
                    message: format!("Undefined function '{}'", name),
                    span: span.clone(),
                })?;

            let mut compiled_args: Vec<BasicMetadataValueEnum> = Vec::new();
            for arg in args {
                let (val, _) = self.compile_expr(arg, function)?;
                compiled_args.push(val.into());
            }

            let result = self
                .builder
                .build_call(func, &compiled_args, "call")
                .unwrap();

            if let Some(val) = result.try_as_basic_value().left() {
                // Determine return type from the function's return type
                let ret_vt = if func.get_type().get_return_type().is_some() {
                    let ret_type = func.get_type().get_return_type().unwrap();
                    if ret_type.is_float_type() {
                        VarType::Number
                    } else if ret_type.is_int_type() {
                        VarType::Boolean
                    } else {
                        VarType::String
                    }
                } else {
                    VarType::Number
                };
                Ok((val, ret_vt))
            } else {
                // Void function - return dummy value
                Ok((
                    self.context.f64_type().const_float(0.0).into(),
                    VarType::Number,
                ))
            }
        } else {
            Err(CompileError {
                message: "Only direct function calls are supported".to_string(),
                span: span.clone(),
            })
        }
    }

    fn compile_console_log(
        &mut self,
        args: &[Expr],
        function: FunctionValue<'ctx>,
        _span: &Span,
    ) -> Result<(BasicValueEnum<'ctx>, VarType), CompileError> {
        for (i, arg) in args.iter().enumerate() {
            if i > 0 {
                // Print space between arguments
                let space = self.create_string_literal(" ");
                let ptr = self
                    .builder
                    .build_extract_value(space.into_struct_value(), 0, "sp")
                    .unwrap();
                let len = self
                    .builder
                    .build_extract_value(space.into_struct_value(), 1, "sl")
                    .unwrap();
                let print_str_fn = self.module.get_function("mango_print_string").unwrap();
                self.builder
                    .build_call(print_str_fn, &[ptr.into(), len.into()], "")
                    .unwrap();
            }

            let (val, vt) = self.compile_expr(arg, function)?;
            match vt {
                VarType::Number => {
                    let print_fn = self.module.get_function("mango_print_number").unwrap();
                    self.builder
                        .build_call(print_fn, &[val.into()], "")
                        .unwrap();
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
                    let print_fn = self.module.get_function("mango_print_string").unwrap();
                    self.builder
                        .build_call(print_fn, &[ptr.into(), len.into()], "")
                        .unwrap();
                }
                VarType::Boolean => {
                    let print_fn = self.module.get_function("mango_print_boolean").unwrap();
                    self.builder
                        .build_call(print_fn, &[val.into()], "")
                        .unwrap();
                }
            }
        }

        // Print newline
        let newline_fn = self.module.get_function("mango_print_newline").unwrap();
        self.builder.build_call(newline_fn, &[], "").unwrap();

        Ok((
            self.context.f64_type().const_float(0.0).into(),
            VarType::Number,
        ))
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
        _function: FunctionValue<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, CompileError> {
        match vt {
            VarType::String => Ok(val),
            VarType::Number => {
                let conv_fn = self.module.get_function("mango_number_to_string").unwrap();
                let result = self
                    .builder
                    .build_call(conv_fn, &[val.into()], "numstr")
                    .unwrap()
                    .try_as_basic_value()
                    .left()
                    .unwrap();
                Ok(result)
            }
            VarType::Boolean => {
                let conv_fn = self.module.get_function("mango_boolean_to_string").unwrap();
                let result = self
                    .builder
                    .build_call(conv_fn, &[val.into()], "boolstr")
                    .unwrap()
                    .try_as_basic_value()
                    .left()
                    .unwrap();
                Ok(result)
            }
        }
    }

    fn to_bool(
        &self,
        val: BasicValueEnum<'ctx>,
        _function: FunctionValue<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, CompileError> {
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
            Ok(val) // Already a bool (i1)
        } else {
            // Struct (string) - truthy if length > 0
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

        let llvm_type = self.var_type_to_llvm(vt);
        builder.build_alloca(llvm_type, name).unwrap()
    }

    fn var_type_to_llvm(&self, vt: &VarType) -> BasicTypeEnum<'ctx> {
        match vt {
            VarType::Number => self.context.f64_type().into(),
            VarType::String => self.string_type.into(),
            VarType::Boolean => self.context.bool_type().into(),
        }
    }

    fn type_ann_to_var_type(&self, ann: &TypeAnnotation) -> VarType {
        match &ann.kind {
            TypeAnnKind::Number => VarType::Number,
            TypeAnnKind::String => VarType::String,
            TypeAnnKind::Boolean => VarType::Boolean,
            _ => VarType::Number, // default
        }
    }

    fn default_value(&self, vt: &VarType) -> BasicValueEnum<'ctx> {
        match vt {
            VarType::Number => self.context.f64_type().const_float(0.0).into(),
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

    pub fn write_object_file(&self, path: &Path) -> Result<(), String> {
        Target::initialize_all(&InitializationConfig::default());

        let triple = TargetMachine::get_default_triple();
        let target = Target::from_triple(&triple).map_err(|e| e.to_string())?;
        let machine = target
            .create_target_machine(
                &triple,
                "generic",
                "",
                OptimizationLevel::Default,
                RelocMode::Default,
                CodeModel::Default,
            )
            .ok_or("Failed to create target machine")?;

        machine
            .write_to_file(&self.module, FileType::Object, path)
            .map_err(|e| e.to_string())?;

        Ok(())
    }

    pub fn print_ir(&self) -> String {
        self.module.print_to_string().to_string()
    }
}
