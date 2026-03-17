use std::collections::{HashMap, HashSet};
use std::path::Path;

use inkwell::attributes::{Attribute, AttributeLoc};
use inkwell::basic_block::BasicBlock;
use inkwell::builder::Builder;
use inkwell::context::Context;
use inkwell::module::Module;
use inkwell::passes::PassBuilderOptions;
use inkwell::targets::{
    CodeModel, FileType, InitializationConfig, RelocMode, Target, TargetMachine,
};
use inkwell::types::{BasicMetadataTypeEnum, BasicType, BasicTypeEnum, StructType};
use inkwell::values::{
    BasicMetadataValueEnum, BasicValueEnum, FunctionValue, IntValue, PointerValue,
};
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
    array_type: StructType<'ctx>,
    /// Closure type: { fn_ptr: ptr, env_ptr: ptr }
    closure_type: StructType<'ctx>,
    /// If true, don't generate a main() — this is a library module
    pub is_library: bool,
    /// Functions whose number params/returns are compiled as i64
    integer_functions: HashSet<String>,
    /// Current number compilation mode (Number=f64, Integer=i64)
    number_mode: VarType,
    /// Stack of loop contexts for break/continue
    loop_stack: Vec<LoopContext<'ctx>>,
    /// Counter for generating unique arrow function names
    arrow_counter: usize,
    /// Registered class struct types by class name: (struct_type, fields, parent_name)
    class_struct_types: HashMap<String, (StructType<'ctx>, Vec<(String, VarType)>, Option<String>)>,
    /// Current `this` pointer (set during method compilation)
    current_this: Option<(PointerValue<'ctx>, VarType)>,
    /// Return VarTypes for compiled functions (for correct call return type inference)
    function_return_types: HashMap<String, VarType>,
    /// Default parameter expressions for functions (for call-site insertion)
    function_param_defaults: HashMap<String, Vec<Option<Expr>>>,
    /// Index of rest parameter for variadic functions (fn_name -> rest param index)
    function_rest_param_index: HashMap<String, usize>,
    /// Parameter VarTypes for functions (for union wrapping at call sites)
    function_param_var_types: HashMap<String, Vec<VarType>>,
    /// Generic function templates: name -> (type_param_names, params, return_type, body)
    generic_templates: HashMap<
        String,
        (
            Vec<String>,
            Vec<Parameter>,
            Option<TypeAnnotation>,
            Vec<Statement>,
        ),
    >,
    /// Active type parameter substitutions for monomorphization
    type_substitutions: HashMap<String, VarType>,
    /// Type alias bodies for codegen resolution
    type_aliases_for_codegen: HashMap<String, TypeAnnotation>,
    /// Generic type alias param names: name -> (param_names, body)
    generic_alias_params: HashMap<String, (Vec<String>, TypeAnnotation)>,
    /// Pending label for the next loop statement (set by Labeled, consumed by loop compilation)
    pending_loop_label: Option<String>,
    /// String enum member values: enum_name -> (member_name -> string_value)
    /// Used to resolve computed property keys like [TaskStatus.Todo] → "todo"
    enum_string_values: HashMap<String, HashMap<String, String>>,
    /// Class field initializers: class_name -> Vec<(field_name, initializer_expr)>
    class_field_initializers: HashMap<String, Vec<(String, Expr)>>,
    /// Inside an async function body: holds the alloca'd promise pointer variable.
    /// Used by `return` statements and `compile_await_expr` to resolve/reject the promise.
    current_async_promise: Option<PointerValue<'ctx>>,
    /// VarType of the value resolved by the current async function's promise.
    current_async_resolve_vt: Option<VarType>,
}

#[derive(Debug, Clone)]
enum VarType {
    Number,
    Integer,
    String,
    Boolean,
    Array,
    FunctionPtr {
        fn_name: String,
    },
    /// Closure: a function value with optional captured environment
    /// Represented as { fn_ptr, env_ptr } struct in LLVM
    Closure {
        fn_name: String,
        param_types: Vec<VarType>,
        return_type: Box<VarType>,
    },
    /// Object/class instance with ordered fields
    Object {
        struct_type_name: String,
        fields: Vec<(String, VarType)>,
    },
    /// Tagged union: runtime type tag + variant data
    /// LLVM layout: { i8 tag, double num_slot, ptr str_ptr_slot, i64 aux_slot }
    Union(Vec<VarType>),
    /// Tuple: heterogeneous fixed-length struct
    Tuple(Vec<VarType>),
    /// String array: array of {char*, i64} strings
    StringArray,
    /// JavaScript Map<K, V> — opaque heap pointer to MgMap
    Map {
        val_vt: Box<VarType>,
    },
    /// Array of typed objects — {void**, i64, i64}; elements accessed via pointer load
    ObjArray {
        elem_vt: Box<VarType>,
    },
    /// A Promise<T> — a pointer to MgPromise in the runtime heap.
    /// inner_vt is the type of the resolved value (used for unboxing after await).
    Promise {
        inner_vt: Box<VarType>,
    },
}

struct LoopContext<'ctx> {
    exit_bb: BasicBlock<'ctx>,
    continue_bb: BasicBlock<'ctx>,
    label: Option<String>,
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

        // Array type: { double*, i64 length, i64 capacity }
        let array_type = context.struct_type(
            &[
                context.ptr_type(AddressSpace::default()).into(),
                context.i64_type().into(),
                context.i64_type().into(),
            ],
            false,
        );

        // Closure type: { fn_ptr: ptr, env_ptr: ptr }
        let closure_type = context.struct_type(
            &[
                context.ptr_type(AddressSpace::default()).into(),
                context.ptr_type(AddressSpace::default()).into(),
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
            array_type,
            closure_type,
            is_library: false,
            integer_functions: HashSet::new(),
            number_mode: VarType::Number,
            loop_stack: Vec::new(),
            arrow_counter: 0,
            class_struct_types: HashMap::new(),
            current_this: None,
            function_return_types: HashMap::new(),
            function_param_defaults: HashMap::new(),
            function_rest_param_index: HashMap::new(),
            function_param_var_types: HashMap::new(),
            generic_templates: HashMap::new(),
            type_substitutions: HashMap::new(),
            type_aliases_for_codegen: HashMap::new(),
            generic_alias_params: HashMap::new(),
            pending_loop_label: None,
            enum_string_values: HashMap::new(),
            class_field_initializers: HashMap::new(),
            current_async_promise: None,
            current_async_resolve_vt: None,
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
            "tscc_print_number",
            void_type.fn_type(&[f64_type.into()], false),
            None,
        );
        self.module.add_function(
            "tscc_print_string",
            void_type.fn_type(&[ptr_type.into(), i64_type.into()], false),
            None,
        );
        self.module.add_function(
            "tscc_print_boolean",
            void_type.fn_type(&[i1_type.into()], false),
            None,
        );
        self.module
            .add_function("tscc_print_null", void_type.fn_type(&[], false), None);
        self.module
            .add_function("tscc_print_undefined", void_type.fn_type(&[], false), None);
        self.module
            .add_function("tscc_print_newline", void_type.fn_type(&[], false), None);

        // --- Stderr print (console.error / console.warn) ---
        self.module.add_function(
            "tscc_eprint_number",
            void_type.fn_type(&[f64_type.into()], false),
            None,
        );
        self.module.add_function(
            "tscc_eprint_string",
            void_type.fn_type(&[ptr_type.into(), i64_type.into()], false),
            None,
        );
        self.module.add_function(
            "tscc_eprint_boolean",
            void_type.fn_type(&[i1_type.into()], false),
            None,
        );
        self.module
            .add_function("tscc_eprint_newline", void_type.fn_type(&[], false), None);

        // --- String operations ---
        self.module.add_function(
            "tscc_string_concat",
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
            "tscc_number_to_string",
            self.string_type.fn_type(&[f64_type.into()], false),
            None,
        );
        self.module.add_function(
            "tscc_boolean_to_string",
            self.string_type.fn_type(&[i1_type.into()], false),
            None,
        );

        // --- String methods ---
        self.module.add_function(
            "tscc_string_toUpperCase",
            self.string_type
                .fn_type(&[ptr_type.into(), i64_type.into()], false),
            None,
        );
        self.module.add_function(
            "tscc_string_toLowerCase",
            self.string_type
                .fn_type(&[ptr_type.into(), i64_type.into()], false),
            None,
        );
        self.module.add_function(
            "tscc_string_charAt",
            self.string_type
                .fn_type(&[ptr_type.into(), i64_type.into(), f64_type.into()], false),
            None,
        );
        self.module.add_function(
            "tscc_string_indexOf",
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
            "tscc_string_includes",
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
            "tscc_string_substring",
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
            "tscc_string_slice",
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
            "tscc_string_trim",
            self.string_type
                .fn_type(&[ptr_type.into(), i64_type.into()], false),
            None,
        );
        self.module.add_function(
            "tscc_string_startsWith",
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
            "tscc_string_endsWith",
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
            "tscc_string_repeat",
            self.string_type
                .fn_type(&[ptr_type.into(), i64_type.into(), f64_type.into()], false),
            None,
        );
        self.module.add_function(
            "tscc_string_replace",
            self.string_type.fn_type(
                &[
                    ptr_type.into(),
                    i64_type.into(),
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
            "tscc_string_padStart",
            self.string_type.fn_type(
                &[
                    ptr_type.into(),
                    i64_type.into(),
                    f64_type.into(),
                    ptr_type.into(),
                    i64_type.into(),
                ],
                false,
            ),
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
                .add_function(&format!("tscc_math_{}", name), math_1, None);
        }
        for name in &["pow", "min", "max"] {
            self.module
                .add_function(&format!("tscc_math_{}", name), math_2, None);
        }
        self.module.add_function("tscc_math_random", math_0, None);

        // --- Array functions ---
        // tscc_array_push(MgArray* arr, double value) → modifies in place
        self.module.add_function(
            "tscc_array_push",
            void_type.fn_type(&[ptr_type.into(), f64_type.into()], false),
            None,
        );
        self.module.add_function(
            "tscc_print_array",
            void_type.fn_type(&[ptr_type.into(), i64_type.into()], false),
            None,
        );
        self.module.add_function(
            "tscc_eprint_array",
            void_type.fn_type(&[ptr_type.into(), i64_type.into()], false),
            None,
        );

        // --- String split ---
        // tscc_string_split(data, len, sep_data, sep_len, *out_data, *out_len)
        self.module.add_function(
            "tscc_string_split",
            void_type.fn_type(
                &[
                    ptr_type.into(),
                    i64_type.into(),
                    ptr_type.into(),
                    i64_type.into(),
                    ptr_type.into(), // out_data (MgString**)
                    ptr_type.into(), // out_len (long long*)
                ],
                false,
            ),
            None,
        );
        self.module.add_function(
            "tscc_print_string_array",
            void_type.fn_type(&[ptr_type.into(), i64_type.into()], false),
            None,
        );

        // --- Number methods ---
        // tscc_number_toFixed(value, digits, *out_data, *out_len)
        self.module.add_function(
            "tscc_number_toFixed",
            void_type.fn_type(
                &[
                    f64_type.into(),
                    f64_type.into(),
                    ptr_type.into(), // out_data (char**)
                    ptr_type.into(), // out_len (long long*)
                ],
                false,
            ),
            None,
        );
        self.module.add_function(
            "tscc_number_isFinite",
            f64_type.fn_type(&[f64_type.into()], false),
            None,
        );
        self.module.add_function(
            "tscc_number_isInteger",
            f64_type.fn_type(&[f64_type.into()], false),
            None,
        );
        self.module.add_function(
            "tscc_number_isNaN",
            f64_type.fn_type(&[f64_type.into()], false),
            None,
        );

        // --- Global functions ---
        self.module.add_function(
            "tscc_parseInt",
            f64_type.fn_type(&[ptr_type.into(), i64_type.into()], false),
            None,
        );
        self.module.add_function(
            "tscc_parseFloat",
            f64_type.fn_type(&[ptr_type.into(), i64_type.into()], false),
            None,
        );

        // --- Map functions ---
        // tscc_map_alloc() → MgMap*
        self.module
            .add_function("tscc_map_alloc", ptr_type.fn_type(&[], false), None);
        // tscc_map_set(map*, key_ptr, key_len, val_ptr, val_size) → void
        self.module.add_function(
            "tscc_map_set",
            void_type.fn_type(
                &[
                    ptr_type.into(),
                    ptr_type.into(),
                    i64_type.into(),
                    ptr_type.into(),
                    i64_type.into(),
                ],
                false,
            ),
            None,
        );
        // tscc_map_get(map*, key_ptr, key_len) → void*
        self.module.add_function(
            "tscc_map_get",
            ptr_type.fn_type(&[ptr_type.into(), ptr_type.into(), i64_type.into()], false),
            None,
        );
        // tscc_map_has(map*, key_ptr, key_len) → i1
        self.module.add_function(
            "tscc_map_has",
            i1_type.fn_type(&[ptr_type.into(), ptr_type.into(), i64_type.into()], false),
            None,
        );
        // tscc_map_delete(map*, key_ptr, key_len) → i1
        self.module.add_function(
            "tscc_map_delete",
            i1_type.fn_type(&[ptr_type.into(), ptr_type.into(), i64_type.into()], false),
            None,
        );
        // tscc_map_size(map*) → i64
        self.module.add_function(
            "tscc_map_size",
            i64_type.fn_type(&[ptr_type.into()], false),
            None,
        );
        // tscc_map_values_alloc(map*, *out_count) → void** (malloc'd array of value ptrs)
        self.module.add_function(
            "tscc_map_values_alloc",
            ptr_type.fn_type(&[ptr_type.into(), ptr_type.into()], false),
            None,
        );

        // libc memcmp(s1, s2, n) → i32  (used for runtime string key comparisons)
        let i32_type = self.context.i32_type();
        self.module.add_function(
            "memcmp",
            i32_type.fn_type(&[ptr_type.into(), ptr_type.into(), i64_type.into()], false),
            None,
        );

        // tscc_crypto_random_uuid() → {i8*, i64}  (RFC 4122 UUID v4 via CSPRNG)
        self.module.add_function(
            "tscc_crypto_random_uuid",
            self.string_type.fn_type(&[], false),
            None,
        );

        // --- Date functions ---
        // tscc_date_now() → i64  (ms since Unix epoch)
        self.module
            .add_function("tscc_date_now", i64_type.fn_type(&[], false), None);
        // tscc_date_get_time(ms) → i64
        self.module.add_function(
            "tscc_date_get_time",
            i64_type.fn_type(&[i64_type.into()], false),
            None,
        );
        // local-time getters: all (ms: i64) → i64
        for name in &[
            "tscc_date_get_full_year",
            "tscc_date_get_month",
            "tscc_date_get_date",
            "tscc_date_get_hours",
            "tscc_date_get_minutes",
            "tscc_date_get_seconds",
            "tscc_date_get_milliseconds",
            "tscc_date_get_utc_full_year",
            "tscc_date_get_utc_month",
            "tscc_date_get_utc_date",
            "tscc_date_get_utc_hours",
            "tscc_date_get_utc_minutes",
            "tscc_date_get_utc_seconds",
            "tscc_date_get_utc_milliseconds",
        ] {
            self.module
                .add_function(name, i64_type.fn_type(&[i64_type.into()], false), None);
        }
        // tscc_date_to_iso_string(ms) → {i8*, i64}
        self.module.add_function(
            "tscc_date_to_iso_string",
            self.string_type.fn_type(&[i64_type.into()], false),
            None,
        );

        // Pre-register Date struct: { i64 } — stores ms since epoch
        let date_struct_type = self.context.struct_type(&[i64_type.into()], false);
        self.class_struct_types.insert(
            "Date".to_string(),
            (
                date_struct_type,
                vec![("__ms".to_string(), VarType::Integer)],
                None,
            ),
        );

        // --- Exception handling (setjmp / longjmp based try/catch/throw) ---
        // jmp_buf* tscc_try_enter(void)
        self.module
            .add_function("tscc_try_enter", ptr_type.fn_type(&[], false), None);
        // void tscc_try_exit(void)
        self.module
            .add_function("tscc_try_exit", void_type.fn_type(&[], false), None);
        // void* tscc_catch_value(void)
        self.module
            .add_function("tscc_catch_value", ptr_type.fn_type(&[], false), None);
        // void tscc_throw(void* value)
        self.module.add_function(
            "tscc_throw",
            void_type.fn_type(&[ptr_type.into()], false),
            None,
        );
        // void* tscc_box_number(double)
        self.module.add_function(
            "tscc_box_number",
            ptr_type.fn_type(&[f64_type.into()], false),
            None,
        );
        // double tscc_unbox_number(void*)
        self.module.add_function(
            "tscc_unbox_number",
            f64_type.fn_type(&[ptr_type.into()], false),
            None,
        );
        // void* tscc_box_boolean(int)
        self.module.add_function(
            "tscc_box_boolean",
            ptr_type.fn_type(&[i1_type.into()], false),
            None,
        );
        // int tscc_unbox_boolean(void*)
        self.module.add_function(
            "tscc_unbox_boolean",
            i1_type.fn_type(&[ptr_type.into()], false),
            None,
        );
        // void* tscc_box_string(char* data, i64 len)
        self.module.add_function(
            "tscc_box_string",
            ptr_type.fn_type(&[ptr_type.into(), i64_type.into()], false),
            None,
        );
        // MgString tscc_unbox_string(void*)
        self.module.add_function(
            "tscc_unbox_string",
            self.string_type.fn_type(&[ptr_type.into()], false),
            None,
        );

        // --- setjmp declaration (marked returns_twice) ---
        let i32_type = self.context.i32_type();
        let setjmp_fn =
            self.module
                .add_function("setjmp", i32_type.fn_type(&[ptr_type.into()], false), None);
        // Mark as returns_twice so LLVM doesn't eliminate the catch path
        let returns_twice_id = Attribute::get_named_enum_kind_id("returns_twice");
        let attr = self.context.create_enum_attribute(returns_twice_id, 0);
        setjmp_fn.add_attribute(AttributeLoc::Function, attr);

        // --- Async runtime: fibers + promises + event loop ---
        // MgFiber* tscc_fiber_create(fn_ptr, arg)
        self.module.add_function(
            "tscc_fiber_create",
            ptr_type.fn_type(&[ptr_type.into(), ptr_type.into()], false),
            None,
        );
        // void tscc_fiber_resume(MgFiber*)
        self.module.add_function(
            "tscc_fiber_resume",
            void_type.fn_type(&[ptr_type.into()], false),
            None,
        );
        // void tscc_fiber_yield(void)
        self.module
            .add_function("tscc_fiber_yield", void_type.fn_type(&[], false), None);
        // MgFiber* tscc_fiber_current(void)
        self.module
            .add_function("tscc_fiber_current", ptr_type.fn_type(&[], false), None);
        // MgPromise* tscc_promise_new(void)
        self.module
            .add_function("tscc_promise_new", ptr_type.fn_type(&[], false), None);
        // void tscc_promise_resolve(MgPromise*, void* value)
        self.module.add_function(
            "tscc_promise_resolve",
            void_type.fn_type(&[ptr_type.into(), ptr_type.into()], false),
            None,
        );
        // void tscc_promise_reject(MgPromise*, void* reason)
        self.module.add_function(
            "tscc_promise_reject",
            void_type.fn_type(&[ptr_type.into(), ptr_type.into()], false),
            None,
        );
        // MgPromise* tscc_promise_then(MgPromise*, fn_ptr, ctx)
        self.module.add_function(
            "tscc_promise_then",
            ptr_type.fn_type(&[ptr_type.into(), ptr_type.into(), ptr_type.into()], false),
            None,
        );
        // MgPromise* tscc_promise_catch(MgPromise*, fn_ptr, ctx)
        self.module.add_function(
            "tscc_promise_catch",
            ptr_type.fn_type(&[ptr_type.into(), ptr_type.into(), ptr_type.into()], false),
            None,
        );
        // void* tscc_await(MgPromise*)
        self.module.add_function(
            "tscc_await",
            ptr_type.fn_type(&[ptr_type.into()], false),
            None,
        );
        // MgPromise* tscc_promise_resolve_val(void*)
        self.module.add_function(
            "tscc_promise_resolve_val",
            ptr_type.fn_type(&[ptr_type.into()], false),
            None,
        );
        // MgPromise* tscc_promise_reject_val(void*)
        self.module.add_function(
            "tscc_promise_reject_val",
            ptr_type.fn_type(&[ptr_type.into()], false),
            None,
        );
        // void tscc_event_loop_run(void)
        self.module
            .add_function("tscc_event_loop_run", void_type.fn_type(&[], false), None);
        // void tscc_set_timeout(fn_ptr, arg, delay_ms)
        self.module.add_function(
            "tscc_set_timeout",
            void_type.fn_type(&[ptr_type.into(), ptr_type.into(), i64_type.into()], false),
            None,
        );
    }

    // --- Integer narrowing analysis ---

    fn analyze_integer_functions(program: &Program) -> HashSet<String> {
        let mut result = HashSet::new();
        for stmt in &program.statements {
            if let StmtKind::FunctionDecl {
                name,
                type_params,
                body,
                ..
            } = &stmt.kind
            {
                // Generic functions are monomorphized — skip integer analysis
                if type_params.is_empty() && Self::is_function_integer_safe(name, body, &result) {
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
            StmtKind::DoWhile { body, condition } => {
                body.iter()
                    .all(|s| Self::is_stmt_integer_safe(s, fn_name, known))
                    && Self::is_expr_integer_safe(condition, fn_name, known)
            }
            StmtKind::Switch {
                discriminant,
                cases,
            } => {
                Self::is_expr_integer_safe(discriminant, fn_name, known)
                    && cases.iter().all(|c| {
                        c.test
                            .as_ref()
                            .map_or(true, |e| Self::is_expr_integer_safe(e, fn_name, known))
                            && c.body
                                .iter()
                                .all(|s| Self::is_stmt_integer_safe(s, fn_name, known))
                    })
            }
            // for-of iterates arrays (f64 elements) — not integer-safe
            StmtKind::ForOf { .. } => false,
            // for-in iterates string keys — not integer-safe
            StmtKind::ForIn { .. } => false,
            StmtKind::ArrayDestructure { initializer, .. } => {
                Self::is_expr_integer_safe(initializer, fn_name, known)
            }
            StmtKind::ObjectDestructure { initializer, .. } => {
                Self::is_expr_integer_safe(initializer, fn_name, known)
            }
            StmtKind::Throw { value } => Self::is_expr_integer_safe(value, fn_name, known),
            StmtKind::TryCatch {
                body,
                catch_body,
                finally_body,
                ..
            } => {
                body.iter()
                    .all(|s| Self::is_stmt_integer_safe(s, fn_name, known))
                    && catch_body.as_ref().map_or(true, |b| {
                        b.iter()
                            .all(|s| Self::is_stmt_integer_safe(s, fn_name, known))
                    })
                    && finally_body.as_ref().map_or(true, |b| {
                        b.iter()
                            .all(|s| Self::is_stmt_integer_safe(s, fn_name, known))
                    })
            }
            StmtKind::FunctionDecl { .. }
            | StmtKind::Import { .. }
            | StmtKind::Break { .. }
            | StmtKind::Continue { .. }
            | StmtKind::Labeled { .. }
            | StmtKind::Empty
            | StmtKind::ClassDecl { .. }
            | StmtKind::InterfaceDecl { .. }
            | StmtKind::TypeAlias { .. }
            | StmtKind::EnumDecl { .. } => true,
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
                // Division and exponentiation can produce floats
                if matches!(op, BinOp::Divide | BinOp::Power) {
                    return false;
                }
                Self::is_expr_integer_safe(left, fn_name, known)
                    && Self::is_expr_integer_safe(right, fn_name, known)
            }
            ExprKind::Conditional {
                condition,
                consequent,
                alternate,
            } => {
                Self::is_expr_integer_safe(condition, fn_name, known)
                    && Self::is_expr_integer_safe(consequent, fn_name, known)
                    && Self::is_expr_integer_safe(alternate, fn_name, known)
            }
            ExprKind::ArrayLiteral { .. } | ExprKind::IndexAccess { .. } => false,
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

        // First pass: register type aliases, interfaces, and classes
        // (so type_ann_to_var_type works when compiling functions in the second pass)
        for stmt in &program.statements {
            // Register type aliases first so they're available for interface/class resolution
            if let StmtKind::TypeAlias {
                name,
                type_params,
                type_ann,
            } = &stmt.kind
            {
                self.type_aliases_for_codegen
                    .insert(name.clone(), type_ann.clone());
                if !type_params.is_empty() {
                    let tp_names: Vec<String> =
                        type_params.iter().map(|tp| tp.name.clone()).collect();
                    self.generic_alias_params
                        .insert(name.clone(), (tp_names, type_ann.clone()));
                }
            }
        }
        for stmt in &program.statements {
            match &stmt.kind {
                StmtKind::InterfaceDecl {
                    name,
                    extends,
                    fields,
                } => {
                    // Prepend inherited fields from parent interfaces
                    let mut field_vts: Vec<(String, VarType)> = Vec::new();
                    for parent_name in extends {
                        if let Some((_, parent_fvts, _)) =
                            self.class_struct_types.get(parent_name).cloned()
                        {
                            for pf in parent_fvts {
                                if !field_vts.iter().any(|(n, _)| n == &pf.0) {
                                    field_vts.push(pf);
                                }
                            }
                        }
                    }
                    for (fname, ann) in fields {
                        let vt = self.type_ann_to_var_type(ann);
                        if !field_vts.iter().any(|(n, _)| n == fname) {
                            field_vts.push((fname.clone(), vt));
                        }
                    }
                    let field_llvm_types: Vec<BasicTypeEnum> = field_vts
                        .iter()
                        .map(|(_, vt)| self.var_type_to_llvm(vt))
                        .collect();
                    let struct_type = self.context.struct_type(&field_llvm_types, false);
                    self.class_struct_types
                        .insert(name.clone(), (struct_type, field_vts, None));
                }
                _ => {}
            }
        }

        // Reserve the LLVM entry-point 'main' name before user functions are compiled.
        // This ensures that if a user defines a TypeScript function also named 'main',
        // it gets the name 'main.1' and the C entry point stays as 'main'.
        if !self.is_library {
            let i32_type = self.context.i32_type();
            let main_placeholder_type = i32_type.fn_type(&[], false);
            self.module
                .add_function("main", main_placeholder_type, None);
        }

        // Second pass: compile all function declarations (skip generics — they're monomorphized on demand)
        for stmt in &program.statements {
            if let StmtKind::FunctionDecl {
                name,
                type_params,
                params,
                return_type,
                body,
                is_async,
                ..
            } = &stmt.kind
            {
                if !type_params.is_empty() {
                    // Store as template for monomorphization at call sites
                    let tp_names: Vec<String> =
                        type_params.iter().map(|tp| tp.name.clone()).collect();
                    self.generic_templates.insert(
                        name.clone(),
                        (tp_names, params.clone(), return_type.clone(), body.clone()),
                    );
                } else if *is_async {
                    self.compile_async_function_decl(name, params, return_type, body)?;
                } else {
                    self.compile_function_decl(name, params, return_type, body)?;
                }
            }
        }

        if self.is_library {
            // Library modules: compile top-level variable declarations as globals
            // (skip for now — top-level code in libraries requires init functions)
            return Ok(());
        }

        // Retrieve the LLVM entry-point 'main' that was pre-reserved before user functions
        // were compiled (to avoid name collision with a TypeScript function also named 'main').
        let i32_type = self.context.i32_type();
        let main_fn = self
            .module
            .get_function("main")
            .expect("LLVM entry 'main' should have been pre-reserved");
        let nounwind_id = Attribute::get_named_enum_kind_id("nounwind");
        main_fn.add_attribute(
            AttributeLoc::Function,
            self.context.create_enum_attribute(nounwind_id, 0),
        );
        let entry = self.context.append_basic_block(main_fn, "entry");
        self.builder.position_at_end(entry);

        self.push_scope();

        for stmt in &program.statements {
            // Skip declarations already handled in earlier passes or type-only
            if matches!(
                &stmt.kind,
                StmtKind::FunctionDecl { .. }
                    | StmtKind::InterfaceDecl { .. }
                    | StmtKind::TypeAlias { .. }
            ) {
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
            // Run the event loop to drain any pending async tasks before exiting
            let event_loop_fn = self.module.get_function("tscc_event_loop_run").unwrap();
            self.builder.build_call(event_loop_fn, &[], "").unwrap();
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
                // Check if the target type is a tuple
                let target_vt = type_ann.as_ref().map(|ann| self.type_ann_to_var_type(ann));

                let (alloca, var_type) = if let Some(init) = initializer {
                    // Special case: tuple-typed variable with array literal initializer
                    if let Some(VarType::Tuple(ref elem_types)) = target_vt {
                        if let ExprKind::ArrayLiteral { elements } = &init.kind {
                            let tuple_vt = VarType::Tuple(elem_types.clone());
                            let (val, _) =
                                self.compile_tuple_literal(elements, elem_types, function)?;
                            let alloca = self.create_alloca(function, &tuple_vt, name);
                            self.builder.build_store(alloca, val).unwrap();
                            self.set_variable(name.clone(), alloca, tuple_vt);
                            return Ok(());
                        }
                    }

                    let (val, vt) = self.compile_expr(init, function)?;
                    // Register non-closure arrow function under the variable name
                    if let VarType::FunctionPtr { ref fn_name } = vt {
                        if let Some(func) = self.functions.get(fn_name).copied() {
                            self.functions.insert(name.clone(), func);
                        }
                    }
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

            StmtKind::ClassDecl {
                name,
                type_params: _,
                parent,
                fields,
                constructor,
                methods,
            } => self.compile_class_decl(name, parent, fields, constructor, methods, function),

            StmtKind::InterfaceDecl {
                name,
                extends,
                fields,
            } => {
                // Interfaces produce no runtime code, but we register the struct layout
                // so type_ann_to_var_type can resolve Named(interface_name)
                // Prepend inherited fields from parent interfaces
                let mut field_vts: Vec<(String, VarType)> = Vec::new();
                for parent_name in extends {
                    if let Some((_, parent_fvts, _)) =
                        self.class_struct_types.get(parent_name).cloned()
                    {
                        for pf in parent_fvts {
                            if !field_vts.iter().any(|(n, _)| n == &pf.0) {
                                field_vts.push(pf);
                            }
                        }
                    }
                }
                for (fname, ann) in fields {
                    let vt = self.type_ann_to_var_type(ann);
                    if !field_vts.iter().any(|(n, _)| n == fname) {
                        field_vts.push((fname.clone(), vt));
                    }
                }
                let field_llvm_types: Vec<BasicTypeEnum> = field_vts
                    .iter()
                    .map(|(_, vt)| self.var_type_to_llvm(vt))
                    .collect();
                let struct_type = self.context.struct_type(&field_llvm_types, false);
                self.class_struct_types
                    .insert(name.clone(), (struct_type, field_vts, None));
                Ok(())
            }

            StmtKind::TypeAlias {
                name,
                type_params,
                type_ann,
            } => {
                // Register for codegen type resolution
                self.type_aliases_for_codegen
                    .insert(name.clone(), type_ann.clone());
                if !type_params.is_empty() {
                    let tp_names: Vec<String> =
                        type_params.iter().map(|tp| tp.name.clone()).collect();
                    self.generic_alias_params
                        .insert(name.clone(), (tp_names, type_ann.clone()));
                }
                // Type aliases are erased — no runtime code
                Ok(())
            }

            StmtKind::EnumDecl { name, members } => {
                // Compile enum as an object — each member is a field with a constant value.
                // Numeric enums: auto-increment from 0. String enums: use specified values.
                let mut field_names = Vec::new();
                let mut field_values: Vec<(BasicValueEnum<'ctx>, VarType)> = Vec::new();
                let mut next_index: i64 = 0;

                // Collect string enum values for computed property key resolution
                let mut string_members: HashMap<String, String> = HashMap::new();

                for member in members {
                    match &member.value {
                        Some(EnumValue::String(s)) => {
                            field_names.push(member.name.clone());
                            field_values.push((self.create_string_literal(s), VarType::String));
                            string_members.insert(member.name.clone(), s.clone());
                        }
                        Some(EnumValue::Number(n)) => {
                            next_index = *n as i64;
                            field_names.push(member.name.clone());
                            field_values.push((
                                self.context.f64_type().const_float(*n).into(),
                                VarType::Number,
                            ));
                            next_index += 1;
                        }
                        None => {
                            field_names.push(member.name.clone());
                            field_values.push((
                                self.context
                                    .f64_type()
                                    .const_float(next_index as f64)
                                    .into(),
                                VarType::Number,
                            ));
                            next_index += 1;
                        }
                    }
                }

                // Build the LLVM struct type from member types
                let field_vts: Vec<(String, VarType)> = field_names
                    .iter()
                    .zip(field_values.iter())
                    .map(|(n, (_, vt))| (n.clone(), vt.clone()))
                    .collect();
                let field_llvm_types: Vec<BasicTypeEnum> = field_values
                    .iter()
                    .map(|(_, vt)| self.var_type_to_llvm(vt))
                    .collect();
                let struct_type = self.context.struct_type(&field_llvm_types, false);

                // Allocate and initialize
                let alloca = self.builder.build_alloca(struct_type, name).unwrap();

                for (i, (val, _)) in field_values.iter().enumerate() {
                    let field_ptr = self
                        .builder
                        .build_struct_gep(struct_type, alloca, i as u32, &field_names[i])
                        .unwrap();
                    self.builder.build_store(field_ptr, *val).unwrap();
                }

                let var_type = VarType::Object {
                    struct_type_name: name.clone(),
                    fields: field_vts.clone(),
                };
                self.set_variable(name.clone(), alloca, var_type);

                // Register in class_struct_types so member access resolution works
                self.class_struct_types
                    .insert(name.clone(), (struct_type, field_vts, None));

                // Register string enum values for computed property key resolution
                if !string_members.is_empty() {
                    self.enum_string_values.insert(name.clone(), string_members);
                }

                Ok(())
            }

            StmtKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                // Check for typeof narrowing pattern on a union variable
                let narrowing = Self::detect_typeof_narrowing(condition).and_then(
                    |(var_name, type_str, is_eq)| {
                        if let Some((ptr, VarType::Union(ref variants))) =
                            self.get_variable(&var_name)
                        {
                            let target_vt = self.type_string_to_var_type(&type_str);
                            let target_tag = Self::union_tag_for_var_type(&target_vt);
                            // Compute remaining variants after narrowing
                            let remaining: Vec<VarType> = variants
                                .iter()
                                .filter(|v| Self::union_tag_for_var_type(v) != target_tag)
                                .cloned()
                                .collect();
                            Some((var_name, ptr, target_vt, target_tag, remaining, is_eq))
                        } else {
                            None
                        }
                    },
                );

                if let Some((var_name, union_ptr, target_vt, target_tag, remaining, is_eq)) =
                    narrowing
                {
                    // Compile as tag comparison instead of generic condition
                    let union_type = self.get_union_llvm_type();
                    let tag_ptr = self
                        .builder
                        .build_struct_gep(union_type, union_ptr, 0, "tag_ptr")
                        .unwrap();
                    let tag = self
                        .builder
                        .build_load(self.context.i8_type(), tag_ptr, "tag")
                        .unwrap()
                        .into_int_value();
                    let expected_tag = self.context.i8_type().const_int(target_tag as u64, false);
                    let cmp = self
                        .builder
                        .build_int_compare(IntPredicate::EQ, tag, expected_tag, "tag_cmp")
                        .unwrap();

                    let then_bb = self.context.append_basic_block(function, "then");
                    let else_bb = self.context.append_basic_block(function, "else");
                    let merge_bb = self.context.append_basic_block(function, "merge");

                    self.builder
                        .build_conditional_branch(cmp, then_bb, else_bb)
                        .unwrap();

                    // Determine which type goes in which branch based on === vs !==
                    let (then_vt, _else_vt_list) = if is_eq {
                        (target_vt.clone(), remaining.clone())
                    } else {
                        // !== : then-branch gets remaining, else-branch gets target
                        let else_list = vec![target_vt.clone()];
                        // For then-branch with !==, use first remaining or fallback
                        let then_single = remaining.first().cloned().unwrap_or(target_vt.clone());
                        (then_single, else_list)
                    };
                    let (then_extract_vt, else_extract_list) = if is_eq {
                        (target_vt.clone(), remaining.clone())
                    } else {
                        let else_list = vec![target_vt.clone()];
                        let then_single = remaining.first().cloned().unwrap_or(target_vt.clone());
                        (then_single, else_list)
                    };

                    // Then branch: narrow to matched type
                    self.builder.position_at_end(then_bb);
                    self.push_scope();
                    let narrowed_val = self.extract_from_union(union_ptr, &then_extract_vt);
                    let narrowed_alloca = self.create_alloca(function, &then_vt, &var_name);
                    self.builder
                        .build_store(narrowed_alloca, narrowed_val)
                        .unwrap();
                    self.set_variable(var_name.clone(), narrowed_alloca, then_vt);
                    for s in then_branch {
                        self.compile_statement(s, function)?;
                    }
                    self.pop_scope();
                    let then_terminated = self
                        .builder
                        .get_insert_block()
                        .unwrap()
                        .get_terminator()
                        .is_some();
                    if !then_terminated {
                        self.builder.build_unconditional_branch(merge_bb).unwrap();
                    }

                    // Else branch: narrow to remaining type(s)
                    self.builder.position_at_end(else_bb);
                    if let Some(else_stmts) = else_branch {
                        self.push_scope();
                        // If only one remaining type, extract it
                        if else_extract_list.len() == 1 {
                            let else_vt = &else_extract_list[0];
                            let else_val = self.extract_from_union(union_ptr, else_vt);
                            let else_alloca = self.create_alloca(function, else_vt, &var_name);
                            self.builder.build_store(else_alloca, else_val).unwrap();
                            self.set_variable(var_name.clone(), else_alloca, else_vt.clone());
                        }
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

                    // Post-if narrowing: if then-branch terminated (return/break),
                    // the merge block only runs from the else path. Narrow the variable
                    // to the else type in the current scope.
                    if then_terminated && else_branch.is_none() {
                        if else_extract_list.len() == 1 {
                            let post_vt = &else_extract_list[0];
                            let post_val = self.extract_from_union(union_ptr, post_vt);
                            let post_alloca = self.create_alloca(function, post_vt, &var_name);
                            self.builder.build_store(post_alloca, post_val).unwrap();
                            self.set_variable(var_name.clone(), post_alloca, post_vt.clone());
                        }
                    }

                    Ok(())
                } else {
                    // Normal (non-narrowing) if statement
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
                let loop_label = self.pending_loop_label.take();
                self.loop_stack.push(LoopContext {
                    exit_bb,
                    continue_bb: cond_bb,
                    label: loop_label,
                });
                for s in body {
                    self.compile_statement(s, function)?;
                }
                self.loop_stack.pop();
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

            StmtKind::DoWhile { body, condition } => {
                let body_bb = self.context.append_basic_block(function, "dowhile.body");
                let cond_bb = self.context.append_basic_block(function, "dowhile.cond");
                let exit_bb = self.context.append_basic_block(function, "dowhile.exit");

                self.builder.build_unconditional_branch(body_bb).unwrap();
                self.builder.position_at_end(body_bb);
                self.push_scope();
                let loop_label = self.pending_loop_label.take();
                self.loop_stack.push(LoopContext {
                    exit_bb,
                    continue_bb: cond_bb,
                    label: loop_label,
                });
                for s in body {
                    self.compile_statement(s, function)?;
                }
                self.loop_stack.pop();
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

                self.builder.position_at_end(cond_bb);
                let (cond_val, _) = self.compile_expr(condition, function)?;
                let cond_bool = self.to_bool(cond_val)?;
                self.builder
                    .build_conditional_branch(cond_bool.into_int_value(), body_bb, exit_bb)
                    .unwrap();

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
                let loop_label = self.pending_loop_label.take();
                self.loop_stack.push(LoopContext {
                    exit_bb,
                    continue_bb: update_bb,
                    label: loop_label,
                });
                for s in body {
                    self.compile_statement(s, function)?;
                }
                self.loop_stack.pop();
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
                // Inside an async function body, `return x` resolves the promise
                if let Some(promise_alloca) = self.current_async_promise {
                    let ptr_type = self.context.ptr_type(AddressSpace::default());
                    // Load the actual promise pointer from the alloca
                    let actual_promise = self
                        .builder
                        .build_load(ptr_type, promise_alloca, "actual_promise")
                        .unwrap();
                    let resolve_vt = self
                        .current_async_resolve_vt
                        .clone()
                        .unwrap_or(VarType::Number);
                    let boxed = if let Some(val) = value {
                        let (ret_val, vt) = self.compile_expr(val, function)?;
                        self.box_value(ret_val, &vt)
                    } else {
                        self.context
                            .ptr_type(AddressSpace::default())
                            .const_null()
                            .into()
                    };
                    let resolve_fn = self.module.get_function("tscc_promise_resolve").unwrap();
                    self.builder
                        .build_call(resolve_fn, &[actual_promise.into(), boxed.into()], "")
                        .unwrap();
                    let _ = resolve_vt;
                    self.builder.build_return(None).unwrap();
                } else if let Some(val) = value {
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

            StmtKind::Import { specifiers, .. } => {
                // Register aliases: if `import { add as sum }`, make `sum` resolve to `add`
                for spec in specifiers {
                    if spec.local != spec.imported {
                        if let Some(func) = self.functions.get(&spec.imported).cloned() {
                            self.functions.insert(spec.local.clone(), func);
                        }
                        // Also copy return type and param type metadata
                        if let Some(rt) = self.function_return_types.get(&spec.imported).cloned() {
                            self.function_return_types.insert(spec.local.clone(), rt);
                        }
                        if let Some(pt) = self.function_param_var_types.get(&spec.imported).cloned()
                        {
                            self.function_param_var_types.insert(spec.local.clone(), pt);
                        }
                        if let Some(defaults) =
                            self.function_param_defaults.get(&spec.imported).cloned()
                        {
                            self.function_param_defaults
                                .insert(spec.local.clone(), defaults);
                        }
                        if let Some(rest_idx) =
                            self.function_rest_param_index.get(&spec.imported).cloned()
                        {
                            self.function_rest_param_index
                                .insert(spec.local.clone(), rest_idx);
                        }
                    }
                }
                Ok(())
            }

            StmtKind::Switch {
                discriminant,
                cases,
            } => {
                let (disc_val, disc_vt) = self.compile_expr(discriminant, function)?;
                let exit_bb = self.context.append_basic_block(function, "switch.exit");

                // Create body blocks for each case
                let body_bbs: Vec<BasicBlock> = cases
                    .iter()
                    .enumerate()
                    .map(|(i, _)| {
                        self.context
                            .append_basic_block(function, &format!("case.{}", i))
                    })
                    .collect();

                // Build comparison chain
                let mut default_idx: Option<usize> = None;
                let mut test_bbs: Vec<BasicBlock> = Vec::new();
                for (i, case) in cases.iter().enumerate() {
                    if case.test.is_some() {
                        let test_bb = self
                            .context
                            .append_basic_block(function, &format!("case.test.{}", i));
                        test_bbs.push(test_bb);
                    } else {
                        default_idx = Some(i);
                    }
                }

                // Branch to first test (or default/exit if no cases)
                if let Some(&first_test) = test_bbs.first() {
                    self.builder.build_unconditional_branch(first_test).unwrap();
                } else if let Some(di) = default_idx {
                    self.builder
                        .build_unconditional_branch(body_bbs[di])
                        .unwrap();
                } else {
                    self.builder.build_unconditional_branch(exit_bb).unwrap();
                }

                // Emit test blocks
                let mut test_idx = 0;
                for (i, case) in cases.iter().enumerate() {
                    if let Some(ref test_expr) = case.test {
                        self.builder.position_at_end(test_bbs[test_idx]);
                        let (test_val, _) = self.compile_expr(test_expr, function)?;

                        // Compare discriminant with case value
                        let cmp = if matches!(disc_vt, VarType::Number) {
                            self.builder
                                .build_float_compare(
                                    FloatPredicate::OEQ,
                                    disc_val.into_float_value(),
                                    test_val.into_float_value(),
                                    "case.eq",
                                )
                                .unwrap()
                        } else if matches!(disc_vt, VarType::Integer) {
                            self.builder
                                .build_int_compare(
                                    IntPredicate::EQ,
                                    disc_val.into_int_value(),
                                    test_val.into_int_value(),
                                    "case.eq",
                                )
                                .unwrap()
                        } else if matches!(disc_vt, VarType::Boolean) {
                            self.builder
                                .build_int_compare(
                                    IntPredicate::EQ,
                                    disc_val.into_int_value(),
                                    test_val.into_int_value(),
                                    "case.eq",
                                )
                                .unwrap()
                        } else {
                            // String comparison: compare lengths then memcmp
                            // For simplicity, compare as f64 (fallback)
                            self.builder
                                .build_float_compare(
                                    FloatPredicate::OEQ,
                                    disc_val.into_float_value(),
                                    test_val.into_float_value(),
                                    "case.eq",
                                )
                                .unwrap()
                        };

                        // If match, go to body; else try next test or default/exit
                        let next = if test_idx + 1 < test_bbs.len() {
                            test_bbs[test_idx + 1]
                        } else if let Some(di) = default_idx {
                            body_bbs[di]
                        } else {
                            exit_bb
                        };
                        self.builder
                            .build_conditional_branch(cmp, body_bbs[i], next)
                            .unwrap();
                        test_idx += 1;
                    }
                }

                // Push a LoopContext so `break` works inside switch
                let loop_label = self.pending_loop_label.take();
                self.loop_stack.push(LoopContext {
                    exit_bb,
                    continue_bb: exit_bb, // continue in switch goes to exit (not ideal, but safe)
                    label: loop_label,
                });

                // Emit body blocks
                for (i, case) in cases.iter().enumerate() {
                    self.builder.position_at_end(body_bbs[i]);
                    for s in &case.body {
                        self.compile_statement(s, function)?;
                    }
                    // Fall-through: if no terminator, branch to next case body or exit
                    if self
                        .builder
                        .get_insert_block()
                        .unwrap()
                        .get_terminator()
                        .is_none()
                    {
                        let next = if i + 1 < body_bbs.len() {
                            body_bbs[i + 1]
                        } else {
                            exit_bb
                        };
                        self.builder.build_unconditional_branch(next).unwrap();
                    }
                }

                self.loop_stack.pop();
                self.builder.position_at_end(exit_bb);
                Ok(())
            }

            StmtKind::ForOf {
                var_name,
                iterable,
                body,
            } => {
                let i64_type = self.context.i64_type();
                let f64_type = self.context.f64_type();

                // Compile the iterable to get an array struct value
                let (arr_val, _arr_vt) = self.compile_expr(iterable, function)?;
                let arr_struct = arr_val.into_struct_value();

                // Extract data pointer and length (fixed before loop)
                let data_ptr = self
                    .builder
                    .build_extract_value(arr_struct, 0, "forof.data")
                    .unwrap()
                    .into_pointer_value();
                let len = self
                    .builder
                    .build_extract_value(arr_struct, 1, "forof.len")
                    .unwrap()
                    .into_int_value();

                // Loop counter alloca
                let i_alloca = self.create_alloca(function, &VarType::Integer, "forof.i");
                self.builder
                    .build_store(i_alloca, i64_type.const_int(0, false))
                    .unwrap();

                let cond_bb = self.context.append_basic_block(function, "forof.cond");
                let body_bb = self.context.append_basic_block(function, "forof.body");
                let update_bb = self.context.append_basic_block(function, "forof.update");
                let exit_bb = self.context.append_basic_block(function, "forof.exit");

                self.builder.build_unconditional_branch(cond_bb).unwrap();

                // Condition: i < len
                self.builder.position_at_end(cond_bb);
                let i_val = self
                    .builder
                    .build_load(i64_type, i_alloca, "i")
                    .unwrap()
                    .into_int_value();
                let cond = self
                    .builder
                    .build_int_compare(IntPredicate::SLT, i_val, len, "forof.cond")
                    .unwrap();
                self.builder
                    .build_conditional_branch(cond, body_bb, exit_bb)
                    .unwrap();

                // Body: load arr[i], bind to var_name
                self.builder.position_at_end(body_bb);
                self.push_scope();

                let i_val = self
                    .builder
                    .build_load(i64_type, i_alloca, "i")
                    .unwrap()
                    .into_int_value();
                let elem_ptr = unsafe {
                    self.builder
                        .build_gep(f64_type, data_ptr, &[i_val], "forof.elem_ptr")
                        .unwrap()
                };
                let elem_val = self
                    .builder
                    .build_load(f64_type, elem_ptr, "forof.elem")
                    .unwrap();

                let elem_alloca = self.create_alloca(function, &VarType::Number, var_name);
                self.builder.build_store(elem_alloca, elem_val).unwrap();
                self.set_variable(var_name.clone(), elem_alloca, VarType::Number);

                let loop_label = self.pending_loop_label.take();
                self.loop_stack.push(LoopContext {
                    exit_bb,
                    continue_bb: update_bb,
                    label: loop_label,
                });
                for s in body {
                    self.compile_statement(s, function)?;
                }
                self.loop_stack.pop();
                self.pop_scope();

                if self
                    .builder
                    .get_insert_block()
                    .unwrap()
                    .get_terminator()
                    .is_none()
                {
                    self.builder.build_unconditional_branch(update_bb).unwrap();
                }

                // Update: i++
                self.builder.position_at_end(update_bb);
                let i_val = self
                    .builder
                    .build_load(i64_type, i_alloca, "i")
                    .unwrap()
                    .into_int_value();
                let i_next = self
                    .builder
                    .build_int_add(i_val, i64_type.const_int(1, false), "i.next")
                    .unwrap();
                self.builder.build_store(i_alloca, i_next).unwrap();
                self.builder.build_unconditional_branch(cond_bb).unwrap();

                self.builder.position_at_end(exit_bb);
                Ok(())
            }

            StmtKind::ForIn {
                var_name,
                object,
                body,
            } => {
                // For-in iterates over object property keys (strings).
                // Since object shapes are known at compile time, we unroll:
                // for each field name, set var_name to that string and execute the body.
                let (_obj_val, obj_vt) = self.compile_expr(object, function)?;

                let field_names: Vec<String> = match &obj_vt {
                    VarType::Object { fields, .. } => {
                        fields.iter().map(|(name, _)| name.clone()).collect()
                    }
                    _ => Vec::new(),
                };

                let exit_bb = self.context.append_basic_block(function, "forin.exit");

                for (i, key) in field_names.iter().enumerate() {
                    let body_bb = self
                        .context
                        .append_basic_block(function, &format!("forin.body.{}", i));
                    self.builder.build_unconditional_branch(body_bb).unwrap();
                    self.builder.position_at_end(body_bb);

                    self.push_scope();

                    // Create the key string
                    let key_val = self.create_string_literal(key);
                    let key_alloca = self.create_alloca(function, &VarType::String, var_name);
                    self.builder.build_store(key_alloca, key_val).unwrap();
                    self.set_variable(var_name.clone(), key_alloca, VarType::String);

                    // continue_bb for break/continue support
                    let continue_bb = self
                        .context
                        .append_basic_block(function, &format!("forin.cont.{}", i));

                    let loop_label = self.pending_loop_label.take();
                    self.loop_stack.push(LoopContext {
                        exit_bb,
                        continue_bb,
                        label: loop_label,
                    });
                    for s in body {
                        self.compile_statement(s, function)?;
                    }
                    self.loop_stack.pop();
                    self.pop_scope();

                    // If body didn't terminate, branch to continue (next iteration)
                    if self
                        .builder
                        .get_insert_block()
                        .unwrap()
                        .get_terminator()
                        .is_none()
                    {
                        self.builder
                            .build_unconditional_branch(continue_bb)
                            .unwrap();
                    }

                    self.builder.position_at_end(continue_bb);
                }

                // After all iterations, branch to exit
                if self
                    .builder
                    .get_insert_block()
                    .unwrap()
                    .get_terminator()
                    .is_none()
                {
                    self.builder.build_unconditional_branch(exit_bb).unwrap();
                }

                self.builder.position_at_end(exit_bb);
                Ok(())
            }

            StmtKind::ArrayDestructure {
                names, initializer, ..
            } => {
                let f64_type = self.context.f64_type();
                let i64_type = self.context.i64_type();

                let (arr_val, _) = self.compile_expr(initializer, function)?;
                let data_ptr = self
                    .builder
                    .build_extract_value(arr_val.into_struct_value(), 0, "destr.data")
                    .unwrap()
                    .into_pointer_value();

                for (i, name) in names.iter().enumerate() {
                    let idx = i64_type.const_int(i as u64, false);
                    let elem_ptr = unsafe {
                        self.builder
                            .build_gep(f64_type, data_ptr, &[idx], "destr.ptr")
                            .unwrap()
                    };
                    let elem_val = self
                        .builder
                        .build_load(f64_type, elem_ptr, "destr.val")
                        .unwrap();
                    let alloca = self.create_alloca(function, &VarType::Number, name);
                    self.builder.build_store(alloca, elem_val).unwrap();
                    self.set_variable(name.clone(), alloca, VarType::Number);
                }
                Ok(())
            }

            StmtKind::ObjectDestructure {
                names, initializer, ..
            } => {
                let (obj_val, obj_vt) = self.compile_expr(initializer, function)?;
                if let VarType::Object { ref fields, .. } = obj_vt {
                    for (local, key) in names {
                        // Find field index
                        if let Some((idx, (_, field_vt))) =
                            fields.iter().enumerate().find(|(_, (n, _))| n == key)
                        {
                            let field_vt = field_vt.clone();
                            let val = self
                                .builder
                                .build_extract_value(
                                    obj_val.into_struct_value(),
                                    idx as u32,
                                    &format!("destr.{}", key),
                                )
                                .unwrap();
                            let alloca = self.create_alloca(function, &field_vt, local);
                            self.builder.build_store(alloca, val).unwrap();
                            self.set_variable(local.clone(), alloca, field_vt);
                        } else {
                            return Err(CompileError::error(
                                format!("Property '{}' does not exist on object", key),
                                stmt.span.clone(),
                            ));
                        }
                    }
                } else {
                    return Err(CompileError::error(
                        "Object destructuring requires an object type",
                        stmt.span.clone(),
                    ));
                }
                Ok(())
            }

            StmtKind::Break { ref label } => {
                let ctx = if let Some(lbl) = label {
                    self.loop_stack
                        .iter()
                        .rev()
                        .find(|c| c.label.as_deref() == Some(lbl))
                } else {
                    self.loop_stack.last()
                };
                if let Some(ctx) = ctx {
                    let exit_bb = ctx.exit_bb;
                    self.builder.build_unconditional_branch(exit_bb).unwrap();
                    // Create dead block for any unreachable code after break
                    let dead_bb = self.context.append_basic_block(function, "break.dead");
                    self.builder.position_at_end(dead_bb);
                } else {
                    return Err(CompileError::error(
                        "'break' can only be used inside a loop or switch",
                        stmt.span.clone(),
                    ));
                }
                Ok(())
            }

            StmtKind::Continue { ref label } => {
                let ctx = if let Some(lbl) = label {
                    self.loop_stack
                        .iter()
                        .rev()
                        .find(|c| c.label.as_deref() == Some(lbl))
                } else {
                    self.loop_stack.last()
                };
                if let Some(ctx) = ctx {
                    let continue_bb = ctx.continue_bb;
                    self.builder
                        .build_unconditional_branch(continue_bb)
                        .unwrap();
                    // Create dead block for any unreachable code after continue
                    let dead_bb = self.context.append_basic_block(function, "continue.dead");
                    self.builder.position_at_end(dead_bb);
                } else {
                    return Err(CompileError::error(
                        "'continue' can only be used inside a loop",
                        stmt.span.clone(),
                    ));
                }
                Ok(())
            }

            StmtKind::Labeled { label, body } => {
                // Set pending label; the next loop push will consume it
                self.pending_loop_label = Some(label.clone());
                self.compile_statement(body, function)?;
                // Clear in case the inner statement wasn't a loop
                self.pending_loop_label = None;
                Ok(())
            }

            StmtKind::Empty => Ok(()),

            StmtKind::Throw { value } => self.compile_throw(value, function),

            StmtKind::TryCatch {
                body,
                catch_binding,
                catch_body,
                finally_body,
            } => self.compile_try_catch(
                body,
                catch_binding.as_deref(),
                catch_body.as_deref(),
                finally_body.as_deref(),
                function,
            ),
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

        // Store default parameter expressions for call-site insertion
        let defaults: Vec<Option<Expr>> = params.iter().map(|p| p.default.clone()).collect();
        if defaults.iter().any(|d| d.is_some()) {
            self.function_param_defaults
                .insert(name.to_string(), defaults);
        }

        // Store rest parameter index if present
        if let Some((rest_idx, _)) = params.iter().enumerate().find(|(_, p)| p.is_rest) {
            self.function_rest_param_index
                .insert(name.to_string(), rest_idx);
        }

        // Store return type for call-site inference
        if let Some(ref vt) = ret_vt {
            self.function_return_types
                .insert(name.to_string(), vt.clone());
        }

        // Store parameter VarTypes for call-site union wrapping
        self.function_param_var_types
            .insert(name.to_string(), param_types.clone());

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

    // -----------------------------------------------------------------------
    // Box/unbox helpers: convert typed values to/from void* for async runtime
    // -----------------------------------------------------------------------

    /// Box a typed value into a `void*` (ptr) for passing to tscc_promise_resolve, etc.
    fn box_value(&mut self, val: BasicValueEnum<'ctx>, vt: &VarType) -> BasicValueEnum<'ctx> {
        let ptr_type = self.context.ptr_type(AddressSpace::default());
        match vt {
            VarType::Number => {
                let f = self.module.get_function("tscc_box_number").unwrap();
                self.builder
                    .build_call(f, &[val.into()], "boxed_num")
                    .unwrap()
                    .try_as_basic_value()
                    .left()
                    .unwrap()
            }
            VarType::Integer => {
                // Convert i64 → f64 then box
                let f64_val = self
                    .builder
                    .build_signed_int_to_float(val.into_int_value(), self.context.f64_type(), "i2f")
                    .unwrap();
                let f = self.module.get_function("tscc_box_number").unwrap();
                self.builder
                    .build_call(f, &[f64_val.into()], "boxed_int")
                    .unwrap()
                    .try_as_basic_value()
                    .left()
                    .unwrap()
            }
            VarType::Boolean => {
                let f = self.module.get_function("tscc_box_boolean").unwrap();
                self.builder
                    .build_call(f, &[val.into()], "boxed_bool")
                    .unwrap()
                    .try_as_basic_value()
                    .left()
                    .unwrap()
            }
            VarType::String => {
                let data = self
                    .builder
                    .build_extract_value(val.into_struct_value(), 0, "str_data")
                    .unwrap();
                let len = self
                    .builder
                    .build_extract_value(val.into_struct_value(), 1, "str_len")
                    .unwrap();
                let f = self.module.get_function("tscc_box_string").unwrap();
                self.builder
                    .build_call(f, &[data.into(), len.into()], "boxed_str")
                    .unwrap()
                    .try_as_basic_value()
                    .left()
                    .unwrap()
            }
            // For pointers/objects/promises, pass the value directly as a ptr
            VarType::Promise { .. } | VarType::Map { .. } | VarType::FunctionPtr { .. } => val,
            _ => {
                // For other types (objects, arrays): store on a heap alloca and return ptr
                // This is an approximation — objects would need malloc for proper lifetime
                let _ = ptr_type;
                val
            }
        }
    }

    /// Unbox a `void*` back to a typed value of VarType `vt`.
    fn unbox_value(&mut self, ptr: BasicValueEnum<'ctx>, vt: &VarType) -> BasicValueEnum<'ctx> {
        match vt {
            VarType::Number => {
                let f = self.module.get_function("tscc_unbox_number").unwrap();
                self.builder
                    .build_call(f, &[ptr.into()], "unboxed_num")
                    .unwrap()
                    .try_as_basic_value()
                    .left()
                    .unwrap()
            }
            VarType::Integer => {
                let f = self.module.get_function("tscc_unbox_number").unwrap();
                let f64_val = self
                    .builder
                    .build_call(f, &[ptr.into()], "unboxed_num_f")
                    .unwrap()
                    .try_as_basic_value()
                    .left()
                    .unwrap();
                self.builder
                    .build_float_to_signed_int(
                        f64_val.into_float_value(),
                        self.context.i64_type(),
                        "f2i",
                    )
                    .unwrap()
                    .into()
            }
            VarType::Boolean => {
                let f = self.module.get_function("tscc_unbox_boolean").unwrap();
                self.builder
                    .build_call(f, &[ptr.into()], "unboxed_bool")
                    .unwrap()
                    .try_as_basic_value()
                    .left()
                    .unwrap()
            }
            VarType::String => {
                let f = self.module.get_function("tscc_unbox_string").unwrap();
                self.builder
                    .build_call(f, &[ptr.into()], "unboxed_str")
                    .unwrap()
                    .try_as_basic_value()
                    .left()
                    .unwrap()
            }
            // Promises/pointers pass through as-is
            _ => ptr,
        }
    }

    // -----------------------------------------------------------------------
    // Async function compilation
    // -----------------------------------------------------------------------

    /// Compile an `async function f(params): Promise<T> { body }`.
    ///
    /// Strategy (stackful fibers):
    ///   1. Generate a fiber body function `__async_body_f(void* args_ptr)` that
    ///      unpacks args, runs the body, and resolves the promise on return.
    ///   2. Generate the outer public function `f(params) -> MgPromise*` that:
    ///      a. Creates a new promise via `tscc_promise_new()`
    ///      b. Allocates an args struct on the heap (promise + params)
    ///      c. Creates a fiber: `tscc_fiber_create(__async_body_f, args_ptr)`
    ///      d. Resumes the fiber immediately: `tscc_fiber_resume(fiber)`
    ///      e. Returns the promise pointer
    fn compile_async_function_decl(
        &mut self,
        name: &str,
        params: &[Parameter],
        return_type: &Option<TypeAnnotation>,
        body: &[Statement],
    ) -> Result<(), CompileError> {
        let saved_mode = self.number_mode.clone();
        if self.integer_functions.contains(name) {
            self.number_mode = VarType::Integer;
        }

        let ptr_type = self.context.ptr_type(AddressSpace::default());
        let void_type = self.context.void_type();

        // Determine the inner resolve VarType (unwrap Promise<T> -> T)
        let resolve_vt = if let Some(ann) = return_type {
            let vt = self.type_ann_to_var_type(ann);
            match vt {
                VarType::Promise { inner_vt } => *inner_vt,
                other => other,
            }
        } else {
            VarType::Number
        };

        // ---- Build the fiber body function ----
        let body_name = format!("__async_body_{}", name);
        let body_fn_type = void_type.fn_type(&[ptr_type.into()], false);
        let body_fn = self.module.add_function(&body_name, body_fn_type, None);

        let saved_bb = self.builder.get_insert_block();
        let entry_bb = self.context.append_basic_block(body_fn, "entry");
        self.builder.position_at_end(entry_bb);
        self.push_scope();

        // Unpack args struct: first element is MgPromise*, rest are the params
        let args_raw = body_fn.get_nth_param(0).unwrap();

        // Load promise pointer from args[0]
        let promise_alloca = self
            .builder
            .build_alloca(ptr_type, "__async_promise")
            .unwrap();

        // The first 8 bytes of args_raw is the promise pointer
        let promise_ptr = self
            .builder
            .build_load(ptr_type, args_raw.into_pointer_value(), "promise")
            .unwrap()
            .into_pointer_value();
        self.builder
            .build_store(promise_alloca, promise_ptr)
            .unwrap();

        // Unpack param values from args_raw + 8 offset per param
        let param_vts: Vec<VarType> = params
            .iter()
            .map(|p| {
                p.type_ann
                    .as_ref()
                    .map(|ann| self.type_ann_to_var_type(ann))
                    .unwrap_or_else(|| self.number_mode.clone())
            })
            .collect();

        for (i, (param, vt)) in params.iter().zip(param_vts.iter()).enumerate() {
            let byte_offset = (i + 1) * 8; // after the promise pointer
            let byte_offset_val = self.context.i64_type().const_int(byte_offset as u64, false);
            let elem_ptr = unsafe {
                self.builder
                    .build_gep(
                        self.context.i8_type(),
                        args_raw.into_pointer_value(),
                        &[byte_offset_val],
                        &format!("arg{}_ptr", i),
                    )
                    .unwrap()
            };
            let llvm_type = self.var_type_to_llvm(vt);
            let val = self
                .builder
                .build_load(llvm_type, elem_ptr, &param.name)
                .unwrap();
            let alloca = self.create_alloca(body_fn, vt, &param.name);
            self.builder.build_store(alloca, val).unwrap();
            self.set_variable(param.name.clone(), alloca, vt.clone());
        }

        // Set up async context
        let prev_async_promise = self.current_async_promise;
        let prev_async_resolve_vt = self.current_async_resolve_vt.clone();
        self.current_async_promise = Some(promise_alloca);
        self.current_async_resolve_vt = Some(resolve_vt.clone());

        // Compile body
        for stmt in body {
            self.compile_statement(stmt, body_fn)?;
        }

        // If body fell through without returning, resolve with undefined (null ptr)
        if self
            .builder
            .get_insert_block()
            .unwrap()
            .get_terminator()
            .is_none()
        {
            // Reload promise from alloca
            let actual_promise = self
                .builder
                .build_load(ptr_type, promise_alloca, "actual_promise_fallthrough")
                .unwrap();
            let resolve_fn = self.module.get_function("tscc_promise_resolve").unwrap();
            let null_val = ptr_type.const_null();
            self.builder
                .build_call(resolve_fn, &[actual_promise.into(), null_val.into()], "")
                .unwrap();
            self.builder.build_return(None).unwrap();
        }

        self.current_async_promise = prev_async_promise;
        self.current_async_resolve_vt = prev_async_resolve_vt;
        self.pop_scope();

        // ---- Build the outer wrapper function (returns MgPromise*) ----
        if let Some(bb) = saved_bb {
            self.builder.position_at_end(bb);
        }

        let llvm_param_types: Vec<BasicMetadataTypeEnum<'ctx>> = param_vts
            .iter()
            .map(|vt| self.var_type_to_llvm(vt).into())
            .collect();
        let outer_fn_type = ptr_type.fn_type(&llvm_param_types, false);
        let outer_fn = self.module.add_function(name, outer_fn_type, None);
        self.functions.insert(name.to_string(), outer_fn);

        // Store return type as Promise
        self.function_return_types.insert(
            name.to_string(),
            VarType::Promise {
                inner_vt: Box::new(resolve_vt.clone()),
            },
        );
        self.function_param_var_types
            .insert(name.to_string(), param_vts.clone());

        let outer_entry = self.context.append_basic_block(outer_fn, "entry");
        let caller_bb = self.builder.get_insert_block();
        self.builder.position_at_end(outer_entry);

        // 1. Create promise
        let promise_new_fn = self.module.get_function("tscc_promise_new").unwrap();
        let promise = self
            .builder
            .build_call(promise_new_fn, &[], "promise")
            .unwrap()
            .try_as_basic_value()
            .left()
            .unwrap();

        // 2. Allocate args struct on the stack:
        //    [ promise_ptr(8 bytes), param0(8 bytes), param1(8 bytes), ... ]
        let args_size = (params.len() + 1) as u64 * 8;
        let args_size_val = self.context.i64_type().const_int(args_size, false);
        let malloc_fn = self.module.get_function("malloc").unwrap_or_else(|| {
            self.module.add_function(
                "malloc",
                ptr_type.fn_type(&[self.context.i64_type().into()], false),
                None,
            )
        });
        let args_buf = self
            .builder
            .build_call(malloc_fn, &[args_size_val.into()], "args_buf")
            .unwrap()
            .try_as_basic_value()
            .left()
            .unwrap()
            .into_pointer_value();

        // Store promise pointer at offset 0
        self.builder.build_store(args_buf, promise).unwrap();

        // Store each param at offset (i+1)*8
        for (i, (param_val, vt)) in outer_fn
            .get_params()
            .iter()
            .zip(param_vts.iter())
            .enumerate()
        {
            let byte_offset = (i + 1) as u64 * 8;
            let byte_offset_val = self.context.i64_type().const_int(byte_offset, false);
            let elem_ptr = unsafe {
                self.builder
                    .build_gep(
                        self.context.i8_type(),
                        args_buf,
                        &[byte_offset_val],
                        &format!("arg{}_dst", i),
                    )
                    .unwrap()
            };
            let llvm_type = self.var_type_to_llvm(vt);
            let _ = llvm_type;
            self.builder.build_store(elem_ptr, *param_val).unwrap();
        }

        // 3. Create fiber
        let fiber_create_fn = self.module.get_function("tscc_fiber_create").unwrap();
        let body_fn_ptr: BasicValueEnum = body_fn.as_global_value().as_pointer_value().into();
        let fiber = self
            .builder
            .build_call(
                fiber_create_fn,
                &[body_fn_ptr.into(), args_buf.into()],
                "fiber",
            )
            .unwrap()
            .try_as_basic_value()
            .left()
            .unwrap();

        // 4. Resume fiber (starts execution up to first await or return)
        let fiber_resume_fn = self.module.get_function("tscc_fiber_resume").unwrap();
        self.builder
            .build_call(fiber_resume_fn, &[fiber.into()], "")
            .unwrap();

        // 5. Return promise
        self.builder.build_return(Some(&promise)).unwrap();

        if let Some(bb) = caller_bb {
            self.builder.position_at_end(bb);
        }

        self.number_mode = saved_mode;
        Ok(())
    }

    // -----------------------------------------------------------------------
    // throw statement
    // -----------------------------------------------------------------------

    fn compile_throw(
        &mut self,
        value: &Expr,
        function: FunctionValue<'ctx>,
    ) -> Result<(), CompileError> {
        let (val, vt) = self.compile_expr(value, function)?;
        let boxed = self.box_value(val, &vt);
        let throw_fn = self.module.get_function("tscc_throw").unwrap();
        self.builder
            .build_call(throw_fn, &[boxed.into()], "")
            .unwrap();
        // tscc_throw never returns (it longjmps), but LLVM doesn't know that.
        // Emit an unreachable to avoid "missing terminator" errors.
        self.builder.build_unreachable().unwrap();
        Ok(())
    }

    // -----------------------------------------------------------------------
    // try/catch/finally
    // -----------------------------------------------------------------------

    fn compile_try_catch(
        &mut self,
        body: &[Statement],
        catch_binding: Option<&str>,
        catch_body: Option<&[Statement]>,
        finally_body: Option<&[Statement]>,
        function: FunctionValue<'ctx>,
    ) -> Result<(), CompileError> {
        let ptr_type = self.context.ptr_type(AddressSpace::default());
        let i32_type = self.context.i32_type();

        // Basic blocks:
        //   try_bb   — the try body
        //   catch_bb — entered when setjmp returns non-zero (an exception was thrown)
        //   finally_bb — always-executed cleanup (if any)
        //   merge_bb — continuation after try/catch/finally
        let try_bb = self.context.append_basic_block(function, "try");
        let catch_bb = self.context.append_basic_block(function, "catch");
        let merge_bb = self.context.append_basic_block(function, "try_merge");

        // Alloca for setjmp return value (must be accessible in both branches)
        let sjlj_result_alloca = self.builder.build_alloca(i32_type, "sjlj_result").unwrap();

        // 1. Call tscc_try_enter() → jmp_buf*
        let try_enter_fn = self.module.get_function("tscc_try_enter").unwrap();
        let jb_ptr = self
            .builder
            .build_call(try_enter_fn, &[], "jb_ptr")
            .unwrap()
            .try_as_basic_value()
            .left()
            .unwrap();

        // 2. Call setjmp(jb_ptr) — marked returns_twice
        let setjmp_fn = self.module.get_function("setjmp").unwrap();
        let sjlj_ret = self
            .builder
            .build_call(setjmp_fn, &[jb_ptr.into()], "sjlj_ret")
            .unwrap()
            .try_as_basic_value()
            .left()
            .unwrap();
        self.builder
            .build_store(sjlj_result_alloca, sjlj_ret)
            .unwrap();

        // 3. Branch: if sjlj_ret == 0, go to try_bb, else go to catch_bb
        let is_catch = self
            .builder
            .build_int_compare(
                IntPredicate::NE,
                sjlj_ret.into_int_value(),
                i32_type.const_int(0, false),
                "is_catch",
            )
            .unwrap();
        self.builder
            .build_conditional_branch(is_catch, catch_bb, try_bb)
            .unwrap();

        // ---- try body ----
        self.builder.position_at_end(try_bb);
        self.push_scope();
        for stmt in body {
            self.compile_statement(stmt, function)?;
            // If a statement emitted a terminator (e.g., return/throw), stop
            if self
                .builder
                .get_insert_block()
                .unwrap()
                .get_terminator()
                .is_some()
            {
                break;
            }
        }
        self.pop_scope();
        if self
            .builder
            .get_insert_block()
            .unwrap()
            .get_terminator()
            .is_none()
        {
            // Normal exit: pop the handler, jump to finally/merge
            let try_exit_fn = self.module.get_function("tscc_try_exit").unwrap();
            self.builder.build_call(try_exit_fn, &[], "").unwrap();
            self.builder.build_unconditional_branch(merge_bb).unwrap();
        }

        // ---- catch body ----
        self.builder.position_at_end(catch_bb);
        if let Some(catch_stmts) = catch_body {
            self.push_scope();
            if let Some(binding) = catch_binding {
                // tscc_catch_value() returns the thrown void*
                let catch_val_fn = self.module.get_function("tscc_catch_value").unwrap();
                let caught_raw = self
                    .builder
                    .build_call(catch_val_fn, &[], "caught_raw")
                    .unwrap()
                    .try_as_basic_value()
                    .left()
                    .unwrap();
                // Bind the caught value as a ptr (type unknown at this point)
                let binding_alloca = self.builder.build_alloca(ptr_type, binding).unwrap();
                self.builder
                    .build_store(binding_alloca, caught_raw)
                    .unwrap();
                self.set_variable(
                    binding.to_string(),
                    binding_alloca,
                    VarType::FunctionPtr {
                        fn_name: String::new(),
                    },
                );
            }
            for stmt in catch_stmts {
                self.compile_statement(stmt, function)?;
                if self
                    .builder
                    .get_insert_block()
                    .unwrap()
                    .get_terminator()
                    .is_some()
                {
                    break;
                }
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

        // ---- merge / finally ----
        self.builder.position_at_end(merge_bb);
        if let Some(finally_stmts) = finally_body {
            self.push_scope();
            for stmt in finally_stmts {
                self.compile_statement(stmt, function)?;
                if self
                    .builder
                    .get_insert_block()
                    .unwrap()
                    .get_terminator()
                    .is_some()
                {
                    break;
                }
            }
            self.pop_scope();
        }
        // If merge_bb has no terminator yet, it just falls through (normal execution continues)

        Ok(())
    }

    // -----------------------------------------------------------------------
    // await expression
    // -----------------------------------------------------------------------

    fn compile_await_expr(
        &mut self,
        inner: &Expr,
        function: FunctionValue<'ctx>,
        span: &Span,
    ) -> Result<(BasicValueEnum<'ctx>, VarType), CompileError> {
        let (val, vt) = self.compile_expr(inner, function)?;

        // Determine the inner VarType we expect back
        let resolve_vt = match &vt {
            VarType::Promise { inner_vt } => *inner_vt.clone(),
            _ => {
                // await non-promise: wrap in a resolved promise and await it
                // (TypeScript spec: await x where x is not a Promise == await Promise.resolve(x))
                let boxed = self.box_value(val, &vt);
                let resolve_val_fn = self
                    .module
                    .get_function("tscc_promise_resolve_val")
                    .unwrap();
                let promise = self
                    .builder
                    .build_call(resolve_val_fn, &[boxed.into()], "wrapped_promise")
                    .unwrap()
                    .try_as_basic_value()
                    .left()
                    .unwrap();
                let await_fn = self.module.get_function("tscc_await").unwrap();
                let raw_result = self
                    .builder
                    .build_call(await_fn, &[promise.into()], "await_result")
                    .unwrap()
                    .try_as_basic_value()
                    .left()
                    .unwrap();
                let unboxed = self.unbox_value(raw_result, &vt);
                return Ok((unboxed, vt));
            }
        };

        // val is already a MgPromise* — call tscc_await
        let await_fn = self.module.get_function("tscc_await").unwrap();

        // The promise value might be a struct value if stored as such; need ptr
        // For Promise VarType, val should be a pointer (ptr_type)
        let promise_ptr = if val.is_pointer_value() {
            val
        } else {
            return Err(CompileError::error(
                "internal: await on non-pointer promise value",
                span.clone(),
            ));
        };

        let raw_result = self
            .builder
            .build_call(await_fn, &[promise_ptr.into()], "await_result")
            .unwrap()
            .try_as_basic_value()
            .left()
            .unwrap();

        // Unbox the result based on the resolve VarType
        let unboxed = self.unbox_value(raw_result, &resolve_vt);
        Ok((unboxed, resolve_vt))
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

            ExprKind::ArrayLiteral { elements } => {
                let has_spread = elements
                    .iter()
                    .any(|e| matches!(e.kind, ExprKind::Spread { .. }));
                let f64_type = self.context.f64_type();
                let i64_type = self.context.i64_type();
                let ptr_type = self.context.ptr_type(AddressSpace::default());

                let malloc_fn = self.module.get_function("malloc").unwrap_or_else(|| {
                    self.module.add_function(
                        "malloc",
                        ptr_type.fn_type(&[i64_type.into()], false),
                        None,
                    )
                });

                if !has_spread {
                    // Fast path: no spread — allocate exact size upfront
                    let count = elements.len() as u64;
                    let capacity = if count > 0 { count } else { 4 };
                    let alloc_size = i64_type.const_int(capacity * 8, false);
                    let data_ptr = self
                        .builder
                        .build_call(malloc_fn, &[alloc_size.into()], "arr_data")
                        .unwrap()
                        .try_as_basic_value()
                        .left()
                        .unwrap()
                        .into_pointer_value();

                    for (i, elem) in elements.iter().enumerate() {
                        let (val, vt) = self.compile_expr(elem, function)?;
                        let float_val = match vt {
                            VarType::Integer => self
                                .builder
                                .build_signed_int_to_float(val.into_int_value(), f64_type, "i2f")
                                .unwrap()
                                .into(),
                            _ => val,
                        };
                        let idx = i64_type.const_int(i as u64, false);
                        let elem_ptr = unsafe {
                            self.builder
                                .build_gep(f64_type, data_ptr, &[idx], "elem_ptr")
                                .unwrap()
                        };
                        self.builder.build_store(elem_ptr, float_val).unwrap();
                    }

                    let arr = self.array_type.const_zero();
                    let arr = self
                        .builder
                        .build_insert_value(arr, data_ptr, 0, "arr.data")
                        .unwrap();
                    let arr = self
                        .builder
                        .build_insert_value(
                            arr.into_struct_value(),
                            i64_type.const_int(count, false),
                            1,
                            "arr.len",
                        )
                        .unwrap();
                    let arr = self
                        .builder
                        .build_insert_value(
                            arr.into_struct_value(),
                            i64_type.const_int(capacity, false),
                            2,
                            "arr.cap",
                        )
                        .unwrap();

                    Ok((arr.into_struct_value().into(), VarType::Array))
                } else {
                    // Spread path: build incrementally using tscc_array_push
                    let push_fn = self.module.get_function("tscc_array_push").unwrap();

                    // Create an initial empty array with some capacity
                    let init_cap = 8u64;
                    let init_data = self
                        .builder
                        .build_call(
                            malloc_fn,
                            &[i64_type.const_int(init_cap * 8, false).into()],
                            "spread_data",
                        )
                        .unwrap()
                        .try_as_basic_value()
                        .left()
                        .unwrap()
                        .into_pointer_value();

                    let init_arr = self.array_type.const_zero();
                    let init_arr = self
                        .builder
                        .build_insert_value(init_arr, init_data, 0, "arr.data")
                        .unwrap();
                    let init_arr = self
                        .builder
                        .build_insert_value(
                            init_arr.into_struct_value(),
                            i64_type.const_int(0, false),
                            1,
                            "arr.len",
                        )
                        .unwrap();
                    let init_arr = self
                        .builder
                        .build_insert_value(
                            init_arr.into_struct_value(),
                            i64_type.const_int(init_cap, false),
                            2,
                            "arr.cap",
                        )
                        .unwrap();

                    // Store in alloca so push can modify it
                    let result_alloca =
                        self.create_alloca(function, &VarType::Array, "spread_result");
                    self.builder
                        .build_store(result_alloca, init_arr.into_struct_value())
                        .unwrap();

                    for elem in elements {
                        if let ExprKind::Spread { expr: spread_expr } = &elem.kind {
                            // Spread: iterate all elements of the spread array and push
                            let (spread_val, _) = self.compile_expr(spread_expr, function)?;
                            let spread_struct = spread_val.into_struct_value();
                            let sp_data = self
                                .builder
                                .build_extract_value(spread_struct, 0, "sp_data")
                                .unwrap()
                                .into_pointer_value();
                            let sp_len = self
                                .builder
                                .build_extract_value(spread_struct, 1, "sp_len")
                                .unwrap()
                                .into_int_value();

                            // Loop: for si in 0..sp_len, push sp_data[si]
                            let si_alloca = self.create_alloca(function, &VarType::Integer, "si");
                            self.builder
                                .build_store(si_alloca, i64_type.const_int(0, false))
                                .unwrap();

                            let sp_cond_bb =
                                self.context.append_basic_block(function, "spread.cond");
                            let sp_body_bb =
                                self.context.append_basic_block(function, "spread.body");
                            let sp_next_bb =
                                self.context.append_basic_block(function, "spread.next");

                            self.builder.build_unconditional_branch(sp_cond_bb).unwrap();

                            self.builder.position_at_end(sp_cond_bb);
                            let si_val = self
                                .builder
                                .build_load(i64_type, si_alloca, "si")
                                .unwrap()
                                .into_int_value();
                            let sp_cond = self
                                .builder
                                .build_int_compare(IntPredicate::SLT, si_val, sp_len, "sp_cond")
                                .unwrap();
                            self.builder
                                .build_conditional_branch(sp_cond, sp_body_bb, sp_next_bb)
                                .unwrap();

                            self.builder.position_at_end(sp_body_bb);
                            let si_val = self
                                .builder
                                .build_load(i64_type, si_alloca, "si")
                                .unwrap()
                                .into_int_value();
                            let sep = unsafe {
                                self.builder
                                    .build_gep(f64_type, sp_data, &[si_val], "sep")
                                    .unwrap()
                            };
                            let sev = self.builder.build_load(f64_type, sep, "sev").unwrap();
                            self.builder
                                .build_call(push_fn, &[result_alloca.into(), sev.into()], "")
                                .unwrap();
                            let si_next = self
                                .builder
                                .build_int_add(si_val, i64_type.const_int(1, false), "si.next")
                                .unwrap();
                            self.builder.build_store(si_alloca, si_next).unwrap();
                            self.builder.build_unconditional_branch(sp_cond_bb).unwrap();

                            self.builder.position_at_end(sp_next_bb);
                        } else {
                            // Regular element: push it
                            let (val, vt) = self.compile_expr(elem, function)?;
                            let float_val: BasicValueEnum = match vt {
                                VarType::Integer => self
                                    .builder
                                    .build_signed_int_to_float(
                                        val.into_int_value(),
                                        f64_type,
                                        "i2f",
                                    )
                                    .unwrap()
                                    .into(),
                                _ => val,
                            };
                            self.builder
                                .build_call(push_fn, &[result_alloca.into(), float_val.into()], "")
                                .unwrap();
                        }
                    }

                    // Load the completed array
                    let result = self
                        .builder
                        .build_load(self.array_type, result_alloca, "spread_arr")
                        .unwrap();
                    Ok((result, VarType::Array))
                }
            }

            ExprKind::IndexAccess { object, index } => {
                let (obj_val, obj_vt) = self.compile_expr(object, function)?;
                let (idx_val, idx_vt) = self.compile_expr(index, function)?;

                if matches!(obj_vt, VarType::Array) {
                    let data_ptr = self
                        .builder
                        .build_extract_value(obj_val.into_struct_value(), 0, "data")
                        .unwrap()
                        .into_pointer_value();

                    // Convert index to i64
                    let idx_i64 = match idx_vt {
                        VarType::Integer => idx_val.into_int_value(),
                        VarType::Number => self
                            .builder
                            .build_float_to_signed_int(
                                idx_val.into_float_value(),
                                self.context.i64_type(),
                                "idx",
                            )
                            .unwrap(),
                        _ => {
                            return Err(CompileError::error(
                                "Array index must be a number",
                                expr.span.clone(),
                            ));
                        }
                    };

                    let elem_ptr = unsafe {
                        self.builder
                            .build_gep(self.context.f64_type(), data_ptr, &[idx_i64], "elem_ptr")
                            .unwrap()
                    };
                    let val = self
                        .builder
                        .build_load(self.context.f64_type(), elem_ptr, "elem")
                        .unwrap();
                    Ok((val, VarType::Number))
                } else if let VarType::Object { ref fields, .. } = obj_vt {
                    // Bracket access with string literal: obj["key"]
                    if let ExprKind::StringLiteral(key) = &index.kind {
                        for (i, (name, field_vt)) in fields.iter().enumerate() {
                            if name == key {
                                let val = self
                                    .builder
                                    .build_extract_value(
                                        obj_val.into_struct_value(),
                                        i as u32,
                                        &format!("obj.{}", key),
                                    )
                                    .unwrap();
                                return Ok((val.into(), field_vt.clone()));
                            }
                        }
                        return Err(CompileError::error(
                            format!("Property '{}' does not exist on object", key),
                            expr.span.clone(),
                        ));
                    }
                    Err(CompileError::error(
                        "Dynamic object property access not supported",
                        expr.span.clone(),
                    ))
                } else if let VarType::Tuple(ref elem_types) = obj_vt {
                    // Tuple index access with compile-time constant index
                    if let ExprKind::NumberLiteral(n) = &index.kind {
                        let idx = *n as usize;
                        if idx < elem_types.len() {
                            let elem_vt = elem_types[idx].clone();
                            let val = self
                                .builder
                                .build_extract_value(
                                    obj_val.into_struct_value(),
                                    idx as u32,
                                    &format!("tup.{}", idx),
                                )
                                .unwrap();
                            return Ok((val.into(), elem_vt));
                        }
                        return Err(CompileError::error(
                            format!(
                                "Tuple index {} out of bounds for tuple of length {}",
                                idx,
                                elem_types.len()
                            ),
                            expr.span.clone(),
                        ));
                    }
                    Err(CompileError::error(
                        "Tuple index must be a numeric literal",
                        expr.span.clone(),
                    ))
                } else if let VarType::ObjArray { ref elem_vt } = obj_vt {
                    // ObjArray[i] — load void* at data[i], then load the element type
                    let (idx_val, idx_vt) = self.compile_expr(index, function)?;
                    let idx_i64 = match idx_vt {
                        VarType::Integer => idx_val.into_int_value(),
                        _ => self
                            .builder
                            .build_float_to_signed_int(
                                idx_val.into_float_value(),
                                self.context.i64_type(),
                                "idx",
                            )
                            .unwrap(),
                    };
                    let data_ptr = self
                        .builder
                        .build_extract_value(obj_val.into_struct_value(), 0, "oa_data")
                        .unwrap()
                        .into_pointer_value();
                    let ptr_type = self.context.ptr_type(AddressSpace::default());
                    let elem_llvm = self.var_type_to_llvm(elem_vt);
                    let elem_ptr_ptr = unsafe {
                        self.builder
                            .build_gep(ptr_type, data_ptr, &[idx_i64], "oa_epp")
                            .unwrap()
                    };
                    let elem_ptr = self
                        .builder
                        .build_load(ptr_type, elem_ptr_ptr, "oa_ep")
                        .unwrap()
                        .into_pointer_value();
                    let elem_val = self
                        .builder
                        .build_load(elem_llvm, elem_ptr, "oa_elem")
                        .unwrap();
                    Ok((elem_val, *elem_vt.clone()))
                } else {
                    Err(CompileError::error(
                        "Index access only supported on arrays, objects, and tuples",
                        expr.span.clone(),
                    ))
                }
            }

            ExprKind::Identifier(name) => {
                // Built-in global constants
                if name == "NaN" {
                    return Ok((
                        self.context.f64_type().const_float(f64::NAN).into(),
                        VarType::Number,
                    ));
                }
                if name == "Infinity" {
                    return Ok((
                        self.context.f64_type().const_float(f64::INFINITY).into(),
                        VarType::Number,
                    ));
                }
                let (ptr, vt) = self.get_variable(name).ok_or_else(|| {
                    CompileError::error(format!("Undefined variable '{}'", name), expr.span.clone())
                })?;
                let llvm_type = self.var_type_to_llvm(&vt);
                let val = self.builder.build_load(llvm_type, ptr, name).unwrap();
                Ok((val, vt))
            }

            ExprKind::Binary {
                left,
                op: BinOp::NullishCoalescing,
                right,
            } => {
                // Nullish coalescing: if LHS is null/undefined, use RHS
                // Without union types, handle at compile time based on AST
                if matches!(
                    left.kind,
                    ExprKind::NullLiteral | ExprKind::UndefinedLiteral
                ) {
                    return self.compile_expr(right, function);
                }
                // LHS is not nullable (no union types yet), so use LHS
                self.compile_expr(left, function)
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

            // Optional chaining: obj?.prop — treated as obj.prop (no runtime null support yet)
            ExprKind::OptionalMember { object, property } => {
                self.compile_member_access(object, property, function, &expr.span)
            }

            // Spread is only valid inside ArrayLiteral — if reached standalone, return inner expr
            ExprKind::Spread { expr: inner } => self.compile_expr(inner, function),

            ExprKind::Assignment { name, value } => {
                let (val, val_type) = self.compile_expr(value, function)?;
                let (ptr, _) = self.get_variable(name).ok_or_else(|| {
                    CompileError::error(format!("Undefined variable '{}'", name), expr.span.clone())
                })?;
                self.builder.build_store(ptr, val).unwrap();
                Ok((val, val_type))
            }

            ExprKind::Conditional {
                condition,
                consequent,
                alternate,
            } => {
                let (cond_val, _) = self.compile_expr(condition, function)?;
                let cond_bool = self.to_bool(cond_val)?;

                let then_bb = self.context.append_basic_block(function, "ternary.then");
                let else_bb = self.context.append_basic_block(function, "ternary.else");
                let merge_bb = self.context.append_basic_block(function, "ternary.merge");

                self.builder
                    .build_conditional_branch(cond_bool.into_int_value(), then_bb, else_bb)
                    .unwrap();

                // Then branch
                self.builder.position_at_end(then_bb);
                let (then_val, then_vt) = self.compile_expr(consequent, function)?;
                let then_bb_end = self.builder.get_insert_block().unwrap();
                self.builder.build_unconditional_branch(merge_bb).unwrap();

                // Else branch
                self.builder.position_at_end(else_bb);
                let (else_val, _else_vt) = self.compile_expr(alternate, function)?;
                let else_bb_end = self.builder.get_insert_block().unwrap();
                self.builder.build_unconditional_branch(merge_bb).unwrap();

                // Merge with phi
                self.builder.position_at_end(merge_bb);
                let phi_type = self.var_type_to_llvm(&then_vt);
                let phi = self.builder.build_phi(phi_type, "ternary").unwrap();
                phi.add_incoming(&[(&then_val, then_bb_end), (&else_val, else_bb_end)]);

                Ok((phi.as_basic_value(), then_vt))
            }

            ExprKind::Grouping { expr } => self.compile_expr(expr, function),

            // Type assertion and satisfies — erased at codegen, compile inner expr
            ExprKind::TypeAssertion { expr, .. } | ExprKind::Satisfies { expr, .. } => {
                self.compile_expr(expr, function)
            }

            ExprKind::PostfixUpdate { target, op } => {
                self.compile_update(target, *op, true, function, &expr.span)
            }

            ExprKind::PrefixUpdate { target, op } => {
                self.compile_update(target, *op, false, function, &expr.span)
            }

            ExprKind::ArrowFunction {
                params,
                return_type,
                body,
                is_async,
            } => {
                // Generate unique function name
                let fn_name = format!("__arrow_{}", self.arrow_counter);
                self.arrow_counter += 1;

                // Convert arrow body to statement list
                let body_stmts = match body {
                    ArrowBody::Expr(e) => {
                        vec![Statement {
                            kind: StmtKind::Return {
                                value: Some(*e.clone()),
                            },
                            span: e.span.clone(),
                        }]
                    }
                    ArrowBody::Block(stmts) => stmts.clone(),
                };

                // Find captured variables from outer scopes
                let captures = self.find_captures(&body_stmts, params);

                // Compile as a closure (all arrows use closure convention)
                self.compile_closure(
                    &fn_name,
                    params,
                    return_type,
                    &body_stmts,
                    captures,
                    function,
                )
            }

            ExprKind::ObjectLiteral { properties } => {
                self.compile_object_literal(properties, function, &expr.span)
            }

            ExprKind::This => {
                if let Some((this_ptr, this_vt)) = &self.current_this {
                    let llvm_type = self.var_type_to_llvm(this_vt);
                    let val = self
                        .builder
                        .build_load(llvm_type, *this_ptr, "this")
                        .unwrap();
                    Ok((val, this_vt.clone()))
                } else {
                    Err(CompileError::error(
                        "'this' is not available in this context",
                        expr.span.clone(),
                    ))
                }
            }

            ExprKind::MemberAssignment {
                object,
                property,
                value,
            } => self.compile_member_assignment(object, property, value, function, &expr.span),

            ExprKind::NewExpr { class_name, args } => {
                self.compile_new_expr(class_name, args, function, &expr.span)
            }

            ExprKind::Await { expr: inner } => self.compile_await_expr(inner, function, &expr.span),
        }
    }

    /// Try to resolve a computed property key expression to a string at compile time.
    /// Handles: string enum member access (EnumName.Member → string value), string literals.
    /// Falls back to a sequential placeholder for non-constant expressions.
    fn try_resolve_computed_key(&mut self, expr: &Expr, counter: &mut usize) -> String {
        match &expr.kind {
            ExprKind::Member { object, property } => {
                if let ExprKind::Identifier(enum_name) = &object.kind {
                    if let Some(members) = self.enum_string_values.get(enum_name.as_str()) {
                        if let Some(val) = members.get(property.as_str()) {
                            return val.clone();
                        }
                    }
                }
                let name = format!("__computed_{}__", counter);
                *counter += 1;
                name
            }
            ExprKind::StringLiteral(s) => s.clone(),
            _ => {
                let name = format!("__computed_{}__", counter);
                *counter += 1;
                name
            }
        }
    }

    fn compile_object_literal(
        &mut self,
        properties: &[ObjectProperty],
        function: FunctionValue<'ctx>,
        span: &Span,
    ) -> Result<(BasicValueEnum<'ctx>, VarType), CompileError> {
        // Compile all property values and determine their VarTypes
        let mut field_vals: Vec<(String, BasicValueEnum<'ctx>, VarType)> = Vec::new();

        let mut computed_counter: usize = 0;

        // First pass: compile non-method properties to determine field types
        for prop in properties {
            if prop.is_spread {
                // Spread: { ...expr } — extract all fields from the source struct
                let (spread_val, spread_vt) = self.compile_expr(&prop.value, function)?;
                if let VarType::Object { ref fields, .. } = spread_vt {
                    let struct_val = spread_val.into_struct_value();
                    for (i, (field_name, field_vt)) in fields.iter().enumerate() {
                        let extracted = self
                            .builder
                            .build_extract_value(struct_val, i as u32, field_name)
                            .unwrap();
                        field_vals.push((field_name.clone(), extracted, field_vt.clone()));
                    }
                }
            } else if let Some(ref key_expr) = prop.computed_key {
                // Computed key: { [expr]: value } — resolve key at compile time
                let key_expr = key_expr.clone();
                let key = self.try_resolve_computed_key(&key_expr, &mut computed_counter);
                let (val, vt) = self.compile_expr(&prop.value, function)?;
                field_vals.push((key, val, vt));
            } else if !prop.is_method {
                let (val, vt) = self.compile_expr(&prop.value, function)?;
                field_vals.push((prop.key.clone(), val, vt));
            } else {
                // Placeholder — will be filled in second pass
                let fn_name = format!("__method_{}_{}", self.arrow_counter, prop.key);
                self.arrow_counter += 1;
                let null_ptr = self.context.ptr_type(AddressSpace::default()).const_null();
                field_vals.push((
                    prop.key.clone(),
                    null_ptr.into(),
                    VarType::FunctionPtr {
                        fn_name: fn_name.clone(),
                    },
                ));
            }
        }

        // Build the VarType::Object so methods can use `this`
        let pre_fields: Vec<(String, VarType)> = field_vals
            .iter()
            .map(|(name, _, vt)| (name.clone(), vt.clone()))
            .collect();
        let pre_struct_name = format!("__obj_{}", self.arrow_counter);
        self.arrow_counter += 1;
        let pre_obj_vt = VarType::Object {
            struct_type_name: pre_struct_name.clone(),
            fields: pre_fields.clone(),
        };

        // Second pass: compile methods with `this` set up
        let mut method_idx = 0;
        for prop in properties {
            if prop.is_method {
                let fn_name = if let VarType::FunctionPtr { ref fn_name } = field_vals
                    .iter()
                    .find(|(k, _, _)| k == &prop.key)
                    .unwrap()
                    .2
                {
                    fn_name.clone()
                } else {
                    unreachable!()
                };

                let body_stmts = if let ExprKind::ArrowFunction { body, .. } = &prop.value.kind {
                    match body {
                        ArrowBody::Block(stmts) => stmts.clone(),
                        ArrowBody::Expr(e) => vec![Statement {
                            kind: StmtKind::Return {
                                value: Some(*e.clone()),
                            },
                            span: e.span.clone(),
                        }],
                    }
                } else {
                    return Err(CompileError::error("Invalid method body", span.clone()));
                };

                // Build method function with self pointer as first parameter
                let ptr_type = self.context.ptr_type(AddressSpace::default());
                let mut param_types: Vec<BasicMetadataTypeEnum> = vec![ptr_type.into()];
                let mut param_vts: Vec<VarType> = Vec::new();
                for param in &prop.params {
                    let vt = param
                        .type_ann
                        .as_ref()
                        .map(|ann| self.type_ann_to_var_type(ann))
                        .unwrap_or(VarType::Number);
                    param_types.push(self.var_type_to_llvm(&vt).into());
                    param_vts.push(vt);
                }

                let ret_vt = prop
                    .return_type
                    .as_ref()
                    .map(|ann| self.type_ann_to_var_type(ann))
                    .unwrap_or(VarType::Number);
                let ret_type = self.var_type_to_llvm(&ret_vt);
                let fn_type = ret_type.fn_type(&param_types, false);
                let method_fn = self.module.add_function(&fn_name, fn_type, None);

                let nounwind_kind = Attribute::get_named_enum_kind_id("nounwind");
                let nounwind = self.context.create_enum_attribute(nounwind_kind, 0);
                method_fn.add_attribute(AttributeLoc::Function, nounwind);

                self.functions.insert(fn_name.clone(), method_fn);

                let entry = self.context.append_basic_block(method_fn, "entry");
                let saved_block = self.builder.get_insert_block();
                self.builder.position_at_end(entry);

                let this_ptr = method_fn.get_nth_param(0).unwrap().into_pointer_value();
                let prev_this = self.current_this.clone();
                self.current_this = Some((this_ptr, pre_obj_vt.clone()));

                self.push_scope();

                for (i, param) in prop.params.iter().enumerate() {
                    let vt = param_vts[i].clone();
                    let param_val = method_fn.get_nth_param((i + 1) as u32).unwrap();
                    let alloca = self.create_alloca(method_fn, &vt, &param.name);
                    self.builder.build_store(alloca, param_val).unwrap();
                    self.set_variable(param.name.clone(), alloca, vt);
                }

                for stmt in &body_stmts {
                    self.compile_statement(stmt, method_fn)?;
                }

                if self
                    .builder
                    .get_insert_block()
                    .unwrap()
                    .get_terminator()
                    .is_none()
                {
                    let default_ret = self.default_value(&ret_vt);
                    self.builder.build_return(Some(&default_ret)).unwrap();
                }

                self.pop_scope();
                self.current_this = prev_this;

                if let Some(bb) = saved_block {
                    self.builder.position_at_end(bb);
                }

                // Update the field_vals with the actual function pointer
                let func = self.functions.get(&fn_name).copied().unwrap();
                let fn_ptr = func.as_global_value().as_pointer_value();
                for (key, val, _) in &mut field_vals {
                    if key == &prop.key {
                        *val = fn_ptr.into();
                        break;
                    }
                }

                method_idx += 1;
            }
        }
        let _ = method_idx;

        // Use the pre-computed field info
        let fields = pre_fields;
        let obj_vt = pre_obj_vt;

        // Build the LLVM struct type
        let field_types: Vec<BasicTypeEnum> = fields
            .iter()
            .map(|(_, vt)| self.var_type_to_llvm(vt))
            .collect();
        let struct_type = self.context.struct_type(&field_types, false);

        // Build the struct value
        let mut struct_val = struct_type.const_zero();
        for (i, (_, val, _)) in field_vals.iter().enumerate() {
            struct_val = self
                .builder
                .build_insert_value(struct_val, *val, i as u32, "obj.field")
                .unwrap()
                .into_struct_value();
        }

        // Discriminated union widening: if a registered merged-union struct is a
        // strict superset of our fields, automatically widen into it (zero-filling
        // any fields this literal doesn't provide).
        if let Some((merged_name, merged_fields)) = self.find_union_superset(&fields) {
            let (merged_struct_type, _, _) =
                self.class_struct_types.get(&merged_name).cloned().unwrap();
            let mut wide_val = merged_struct_type.const_zero();
            for (src_name, src_val, _) in &field_vals {
                if let Some(dst_idx) = merged_fields.iter().position(|(n, _)| n == src_name) {
                    wide_val = self
                        .builder
                        .build_insert_value(wide_val, *src_val, dst_idx as u32, "union.field")
                        .unwrap()
                        .into_struct_value();
                }
            }
            let wide_vt = VarType::Object {
                struct_type_name: merged_name,
                fields: merged_fields,
            };
            return Ok((wide_val.into(), wide_vt));
        }

        Ok((struct_val.into(), obj_vt))
    }

    /// Find a registered `__union_obj_*` struct whose field list is a STRICT superset
    /// of `current_fields`. Returns `(struct_name, all_fields)` if found, else `None`.
    fn find_union_superset(
        &self,
        current_fields: &[(String, VarType)],
    ) -> Option<(String, Vec<(String, VarType)>)> {
        // Only attempt widening when we actually have fewer fields than the candidate.
        for (struct_name, (_, candidate_fields, _)) in &self.class_struct_types {
            if !struct_name.starts_with("__union_obj_") {
                continue;
            }
            // candidate must have strictly more fields
            if candidate_fields.len() <= current_fields.len() {
                continue;
            }
            // every current field must exist in the candidate (by name)
            let all_present = current_fields
                .iter()
                .all(|(name, _)| candidate_fields.iter().any(|(n, _)| n == name));
            if all_present {
                return Some((struct_name.clone(), candidate_fields.clone()));
            }
        }
        None
    }

    /// Coerce `val` (of `VarType::Object`) to a target `VarType::Object` that is a superset,
    /// zero-filling any missing fields. No-op if types already match or target is not an Object.
    fn coerce_to_object_target(
        &mut self,
        val: BasicValueEnum<'ctx>,
        src_vt: &VarType,
        target_vt: &VarType,
    ) -> (BasicValueEnum<'ctx>, VarType) {
        if let (
            VarType::Object {
                fields: src_fields, ..
            },
            VarType::Object {
                struct_type_name: dst_name,
                fields: dst_fields,
            },
        ) = (src_vt, target_vt)
        {
            if dst_fields.len() > src_fields.len() {
                if let Some((dst_struct_type, _, _)) =
                    self.class_struct_types.get(dst_name).cloned()
                {
                    let src_struct = val.into_struct_value();
                    let mut wide_val = dst_struct_type.const_zero();
                    for (src_idx, (src_name, _)) in src_fields.iter().enumerate() {
                        if let Some(dst_idx) = dst_fields.iter().position(|(n, _)| n == src_name) {
                            let field_val = self
                                .builder
                                .build_extract_value(src_struct, src_idx as u32, src_name)
                                .unwrap();
                            wide_val = self
                                .builder
                                .build_insert_value(
                                    wide_val,
                                    field_val,
                                    dst_idx as u32,
                                    "wide.field",
                                )
                                .unwrap()
                                .into_struct_value();
                        }
                    }
                    return (wide_val.into(), target_vt.clone());
                }
            }
        }
        (val, src_vt.clone())
    }

    fn compile_member_assignment(
        &mut self,
        object: &Expr,
        property: &str,
        value: &Expr,
        function: FunctionValue<'ctx>,
        span: &Span,
    ) -> Result<(BasicValueEnum<'ctx>, VarType), CompileError> {
        let (new_val, new_vt) = self.compile_expr(value, function)?;

        // For `this.prop = value`, we need to store through the `this` pointer
        if matches!(object.kind, ExprKind::This) {
            if let Some((this_ptr, this_vt)) = self.current_this.clone() {
                if let VarType::Object { ref fields, .. } = this_vt {
                    for (i, (name, _)) in fields.iter().enumerate() {
                        if name == property {
                            let llvm_type = self.var_type_to_llvm(&this_vt);
                            let struct_type = llvm_type.into_struct_type();
                            let field_ptr = self
                                .builder
                                .build_struct_gep(
                                    struct_type,
                                    this_ptr,
                                    i as u32,
                                    &format!("this.{}", property),
                                )
                                .unwrap();
                            self.builder.build_store(field_ptr, new_val).unwrap();
                            return Ok((new_val, new_vt));
                        }
                    }
                }
            }
            return Err(CompileError::error(
                format!("Property '{}' not found on 'this'", property),
                span.clone(),
            ));
        }

        // For named variables: obj.prop = value
        if let ExprKind::Identifier(var_name) = &object.kind {
            let (ptr, vt) = self.get_variable(var_name).ok_or_else(|| {
                CompileError::error(format!("Undefined variable '{}'", var_name), span.clone())
            })?;
            if let VarType::Object { ref fields, .. } = vt {
                for (i, (name, _)) in fields.iter().enumerate() {
                    if name == property {
                        let llvm_type = self.var_type_to_llvm(&vt);
                        let struct_type = llvm_type.into_struct_type();
                        let field_ptr = self
                            .builder
                            .build_struct_gep(
                                struct_type,
                                ptr,
                                i as u32,
                                &format!("obj.{}", property),
                            )
                            .unwrap();
                        self.builder.build_store(field_ptr, new_val).unwrap();
                        return Ok((new_val, new_vt));
                    }
                }
            }
        }

        Err(CompileError::error(
            format!("Cannot assign to property '{}' in this context", property),
            span.clone(),
        ))
    }

    /// Compile a `++` / `--` update on any valid lvalue target.
    ///
    /// `is_postfix = true`  → return the *old* value (postfix `x++`)
    /// `is_postfix = false` → return the *new* value (prefix  `++x`)
    fn compile_update(
        &mut self,
        target: &Expr,
        op: UpdateOp,
        is_postfix: bool,
        function: FunctionValue<'ctx>,
        span: &Span,
    ) -> Result<(BasicValueEnum<'ctx>, VarType), CompileError> {
        match &target.kind {
            // ── Simple identifier: x++ / ++x ─────────────────────────────────
            ExprKind::Identifier(name) => {
                let (ptr, vt) = self.get_variable(name).ok_or_else(|| {
                    CompileError::error(format!("Undefined variable '{}'", name), span.clone())
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
                    let result = if is_postfix { old_int } else { new_val };
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
                    let result = if is_postfix { old_float } else { new_val };
                    Ok((result.into(), VarType::Number))
                }
            }

            // ── Index access: arr[i]++ or obj[key]++ ─────────────────────────
            ExprKind::IndexAccess { object, index } => {
                // We need the alloca of the base variable
                let ExprKind::Identifier(obj_name) = &object.kind else {
                    return Err(CompileError::error(
                        "++/-- requires a simple variable as base",
                        span.clone(),
                    ));
                };
                let (obj_ptr, obj_vt) = self.get_variable(obj_name).ok_or_else(|| {
                    CompileError::error(format!("Undefined variable '{}'", obj_name), span.clone())
                })?;

                match obj_vt.clone() {
                    // ── Array element: arr[i]++ ───────────────────────────────
                    VarType::Array => {
                        let arr_llvm = self.var_type_to_llvm(&VarType::Array);
                        let arr_val = self.builder.build_load(arr_llvm, obj_ptr, "arr").unwrap();
                        let data_ptr = self
                            .builder
                            .build_extract_value(arr_val.into_struct_value(), 0, "data")
                            .unwrap()
                            .into_pointer_value();

                        let (idx_val, idx_vt) = self.compile_expr(index, function)?;
                        let idx_i64 = match idx_vt {
                            VarType::Integer => idx_val.into_int_value(),
                            VarType::Number => self
                                .builder
                                .build_float_to_signed_int(
                                    idx_val.into_float_value(),
                                    self.context.i64_type(),
                                    "idx",
                                )
                                .unwrap(),
                            _ => {
                                return Err(CompileError::error(
                                    "Array index must be a number",
                                    span.clone(),
                                ))
                            }
                        };

                        let elem_ptr = unsafe {
                            self.builder
                                .build_gep(
                                    self.context.f64_type(),
                                    data_ptr,
                                    &[idx_i64],
                                    "elem_ptr",
                                )
                                .unwrap()
                        };
                        let old_float = self
                            .builder
                            .build_load(self.context.f64_type(), elem_ptr, "old")
                            .unwrap()
                            .into_float_value();
                        let one = self.context.f64_type().const_float(1.0);
                        let new_float = match op {
                            UpdateOp::Increment => {
                                self.builder.build_float_add(old_float, one, "inc").unwrap()
                            }
                            UpdateOp::Decrement => {
                                self.builder.build_float_sub(old_float, one, "dec").unwrap()
                            }
                        };
                        self.builder.build_store(elem_ptr, new_float).unwrap();
                        let result = if is_postfix { old_float } else { new_float };
                        Ok((result.into(), VarType::Number))
                    }

                    // ── Object field with dynamic string key: obj[key]++ ──────
                    VarType::Object {
                        ref fields,
                        ref struct_type_name,
                    } => {
                        let (idx_val, _idx_vt) = self.compile_expr(index, function)?;
                        let idx_str = idx_val.into_struct_value();
                        let idx_ptr = self
                            .builder
                            .build_extract_value(idx_str, 0, "idx_ptr")
                            .unwrap()
                            .into_pointer_value();
                        let idx_len = self
                            .builder
                            .build_extract_value(idx_str, 1, "idx_len")
                            .unwrap()
                            .into_int_value();

                        let memcmp_fn = self.module.get_function("memcmp").unwrap();
                        let f64_type = self.context.f64_type();
                        let obj_struct_type = self
                            .var_type_to_llvm(&VarType::Object {
                                struct_type_name: struct_type_name.clone(),
                                fields: fields.clone(),
                            })
                            .into_struct_type();

                        // Build one basic block per field + a no-match block + a merge block
                        let merge_bb = self.context.append_basic_block(function, "upd.merge");

                        // Collect field info (clone to avoid borrow issues)
                        let fields_snap: Vec<(String, VarType)> = fields.clone();
                        let n = fields_snap.len();

                        // Create check blocks and hit blocks
                        let check_bbs: Vec<_> = (0..n)
                            .map(|i| {
                                self.context
                                    .append_basic_block(function, &format!("upd.check{}", i))
                            })
                            .collect();
                        let hit_bbs: Vec<_> = (0..n)
                            .map(|i| {
                                self.context
                                    .append_basic_block(function, &format!("upd.hit{}", i))
                            })
                            .collect();
                        let nomatch_bb = self.context.append_basic_block(function, "upd.nomatch");

                        // Jump to first check
                        self.builder
                            .build_unconditional_branch(check_bbs[0])
                            .unwrap();

                        // Emit old-value storage for postfix semantics
                        // We use a stack slot so each hit block can write the old value
                        // and the merge phi can read it.
                        // (Simpler: just use phi across hit blocks + nomatch.)

                        let mut old_vals: Vec<(BasicValueEnum<'ctx>, _)> = Vec::new();

                        for (i, (field_name, _field_vt)) in fields_snap.iter().enumerate() {
                            // ── check block ──────────────────────────────────
                            self.builder.position_at_end(check_bbs[i]);

                            let field_bytes = field_name.as_bytes();
                            let field_len_val = self
                                .context
                                .i64_type()
                                .const_int(field_bytes.len() as u64, false);

                            // Length comparison: idx_len == field_len
                            let len_eq = self
                                .builder
                                .build_int_compare(
                                    inkwell::IntPredicate::EQ,
                                    idx_len,
                                    field_len_val,
                                    "len_eq",
                                )
                                .unwrap();

                            // Bytes comparison block
                            let bytes_bb = self
                                .context
                                .append_basic_block(function, &format!("upd.bytes{}", i));
                            let next_bb = if i + 1 < n {
                                check_bbs[i + 1]
                            } else {
                                nomatch_bb
                            };
                            self.builder
                                .build_conditional_branch(len_eq, bytes_bb, next_bb)
                                .unwrap();

                            // ── bytes block ───────────────────────────────────
                            self.builder.position_at_end(bytes_bb);
                            let lit_ptr = self
                                .builder
                                .build_global_string_ptr(field_name, "fk")
                                .unwrap()
                                .as_pointer_value();
                            let cmp_result = self
                                .builder
                                .build_call(
                                    memcmp_fn,
                                    &[idx_ptr.into(), lit_ptr.into(), field_len_val.into()],
                                    "memcmp",
                                )
                                .unwrap()
                                .try_as_basic_value()
                                .left()
                                .unwrap()
                                .into_int_value();
                            let zero_i32 = self.context.i32_type().const_int(0, false);
                            let bytes_eq = self
                                .builder
                                .build_int_compare(
                                    inkwell::IntPredicate::EQ,
                                    cmp_result,
                                    zero_i32,
                                    "bytes_eq",
                                )
                                .unwrap();
                            self.builder
                                .build_conditional_branch(bytes_eq, hit_bbs[i], next_bb)
                                .unwrap();

                            // ── hit block ─────────────────────────────────────
                            self.builder.position_at_end(hit_bbs[i]);
                            let field_ptr = self
                                .builder
                                .build_struct_gep(
                                    obj_struct_type,
                                    obj_ptr,
                                    i as u32,
                                    &format!("obj.{}", field_name),
                                )
                                .unwrap();
                            let old_f = self
                                .builder
                                .build_load(f64_type, field_ptr, "old")
                                .unwrap()
                                .into_float_value();
                            let one = f64_type.const_float(1.0);
                            let new_f = match op {
                                UpdateOp::Increment => {
                                    self.builder.build_float_add(old_f, one, "inc").unwrap()
                                }
                                UpdateOp::Decrement => {
                                    self.builder.build_float_sub(old_f, one, "dec").unwrap()
                                }
                            };
                            self.builder.build_store(field_ptr, new_f).unwrap();
                            self.builder.build_unconditional_branch(merge_bb).unwrap();
                            old_vals.push((old_f.into(), hit_bbs[i]));
                        }

                        // ── no-match block (silent no-op) ─────────────────────
                        self.builder.position_at_end(nomatch_bb);
                        let zero_f: BasicValueEnum<'ctx> = f64_type.const_float(0.0).into();
                        self.builder.build_unconditional_branch(merge_bb).unwrap();

                        // ── merge block ────────────────────────────────────────
                        self.builder.position_at_end(merge_bb);
                        let phi = self.builder.build_phi(f64_type, "upd.result").unwrap();
                        for (val, bb) in &old_vals {
                            phi.add_incoming(&[(val, *bb)]);
                        }
                        // no-match contributes 0.0 (postfix: returns 0, prefix: 0)
                        phi.add_incoming(&[(&zero_f, nomatch_bb)]);

                        let result = if is_postfix {
                            phi.as_basic_value() // old value
                        } else {
                            // prefix: we don't have a single "new value" phi across all fields.
                            // Return the old+1 value via a second phi.
                            // For simplicity use the postfix phi + constant 1 delta (same for all fields).
                            let one = f64_type.const_float(1.0);
                            match op {
                                UpdateOp::Increment => self
                                    .builder
                                    .build_float_add(
                                        phi.as_basic_value().into_float_value(),
                                        one,
                                        "pre_inc",
                                    )
                                    .unwrap()
                                    .into(),
                                UpdateOp::Decrement => self
                                    .builder
                                    .build_float_sub(
                                        phi.as_basic_value().into_float_value(),
                                        one,
                                        "pre_dec",
                                    )
                                    .unwrap()
                                    .into(),
                            }
                        };
                        Ok((result, VarType::Number))
                    }

                    _ => Err(CompileError::error(
                        "++/-- index target must be an array or object",
                        span.clone(),
                    )),
                }
            }

            // ── Member access: obj.prop++ ─────────────────────────────────────
            ExprKind::Member { object, property } => {
                // Resolve the object's alloca and struct layout
                let get_ptr_and_vt =
                    |cg: &mut Self| -> Result<(PointerValue<'ctx>, VarType), CompileError> {
                        if matches!(object.kind, ExprKind::This) {
                            if let Some((this_ptr, this_vt)) = cg.current_this.clone() {
                                return Ok((this_ptr, this_vt));
                            }
                            return Err(CompileError::error(
                                "Cannot use 'this' outside of a method",
                                span.clone(),
                            ));
                        }
                        if let ExprKind::Identifier(obj_name) = &object.kind {
                            if let Some((ptr, vt)) = cg.get_variable(obj_name) {
                                return Ok((ptr, vt));
                            }
                        }
                        Err(CompileError::error(
                            "++/-- member target must be a simple variable or 'this'",
                            span.clone(),
                        ))
                    };
                let (obj_ptr, obj_vt) = get_ptr_and_vt(self)?;
                if let VarType::Object { ref fields, .. } = obj_vt.clone() {
                    if let Some(i) = fields.iter().position(|(n, _)| n == property) {
                        let llvm_type = self.var_type_to_llvm(&obj_vt);
                        let struct_type = llvm_type.into_struct_type();
                        let field_ptr = self
                            .builder
                            .build_struct_gep(
                                struct_type,
                                obj_ptr,
                                i as u32,
                                &format!("obj.{}", property),
                            )
                            .unwrap();
                        let old_float = self
                            .builder
                            .build_load(self.context.f64_type(), field_ptr, "old")
                            .unwrap()
                            .into_float_value();
                        let one = self.context.f64_type().const_float(1.0);
                        let new_float = match op {
                            UpdateOp::Increment => {
                                self.builder.build_float_add(old_float, one, "inc").unwrap()
                            }
                            UpdateOp::Decrement => {
                                self.builder.build_float_sub(old_float, one, "dec").unwrap()
                            }
                        };
                        self.builder.build_store(field_ptr, new_float).unwrap();
                        let result = if is_postfix { old_float } else { new_float };
                        return Ok((result.into(), VarType::Number));
                    }
                }
                Err(CompileError::error(
                    format!("Property '{}' not found for ++/--", property),
                    span.clone(),
                ))
            }

            _ => Err(CompileError::error(
                "Invalid target for ++/--",
                span.clone(),
            )),
        }
    }

    /// Emit field initializers for a class and its parent chain (parent-first order).
    fn run_field_initializers(
        &mut self,
        class_name: &str,
        alloca: PointerValue<'ctx>,
        struct_type: StructType<'ctx>,
        all_fields: &[(String, VarType)],
        function: FunctionValue<'ctx>,
    ) -> Result<(), CompileError> {
        // Run parent initializers first
        let parent = self
            .class_struct_types
            .get(class_name)
            .and_then(|(_, _, p)| p.clone());
        if let Some(ref pname) = parent {
            let parent_data = self.class_struct_types.get(pname).cloned();
            if let Some((parent_struct, parent_fields, _)) = parent_data {
                self.run_field_initializers(
                    pname,
                    alloca,
                    parent_struct,
                    &parent_fields,
                    function,
                )?;
            }
        }

        // Emit this class's own field initializers
        let initializers = self
            .class_field_initializers
            .get(class_name)
            .cloned()
            .unwrap_or_default();
        for (field_name, init_expr) in initializers {
            let (init_val, _) = self.compile_expr(&init_expr, function)?;
            // Find the field index in the full struct layout
            if let Some(idx) = all_fields.iter().position(|(n, _)| n == &field_name) {
                let field_ptr = self
                    .builder
                    .build_struct_gep(
                        struct_type,
                        alloca,
                        idx as u32,
                        &format!("init_{}", field_name),
                    )
                    .unwrap();
                self.builder.build_store(field_ptr, init_val).unwrap();
            }
        }
        Ok(())
    }

    fn compile_new_expr(
        &mut self,
        class_name: &str,
        args: &[Expr],
        function: FunctionValue<'ctx>,
        span: &Span,
    ) -> Result<(BasicValueEnum<'ctx>, VarType), CompileError> {
        // new Date() / new Date(ms) — built-in
        if class_name == "Date" {
            let i64_type = self.context.i64_type();
            let ms_val: BasicValueEnum<'ctx> = match args.len() {
                0 => {
                    // new Date() — current time
                    let now_fn = self.module.get_function("tscc_date_now").unwrap();
                    self.builder
                        .build_call(now_fn, &[], "date_now")
                        .unwrap()
                        .try_as_basic_value()
                        .left()
                        .unwrap()
                }
                1 => {
                    // new Date(ms) — check the arg type
                    let (arg_val, arg_vt) = self.compile_expr(&args[0], function)?;
                    match &args[0].kind {
                        ExprKind::StringLiteral(_) => {
                            return Err(CompileError::error(
                                "new Date(string) is not implemented; use new Date() or new Date(milliseconds)",
                                span.clone(),
                            ));
                        }
                        _ => {}
                    }
                    // Coerce number → i64
                    match arg_vt {
                        VarType::Integer => arg_val,
                        VarType::Number => self
                            .builder
                            .build_float_to_signed_int(
                                arg_val.into_float_value(),
                                i64_type,
                                "ms_i64",
                            )
                            .unwrap()
                            .into(),
                        _ => {
                            return Err(CompileError::error(
                                "new Date() argument must be a number (milliseconds since epoch)",
                                span.clone(),
                            ));
                        }
                    }
                }
                _ => {
                    return Err(CompileError::error(
                        "new Date() takes 0 or 1 arguments",
                        span.clone(),
                    ));
                }
            };
            // Build { i64 } struct and insert the ms value
            let (date_struct_type, field_info, _) =
                self.class_struct_types.get("Date").cloned().unwrap();
            let mut struct_val = date_struct_type.const_zero();
            struct_val = self
                .builder
                .build_insert_value(struct_val, ms_val, 0, "date_struct")
                .unwrap()
                .into_struct_value();
            let date_vt = VarType::Object {
                struct_type_name: "Date".to_string(),
                fields: field_info,
            };
            return Ok((struct_val.into(), date_vt));
        }

        // new Map() — call tscc_map_alloc()
        if class_name == "Map" {
            let alloc_fn = self.module.get_function("tscc_map_alloc").unwrap();
            let map_ptr = self
                .builder
                .build_call(alloc_fn, &[], "map_new")
                .unwrap()
                .try_as_basic_value()
                .left()
                .unwrap();
            return Ok((
                map_ptr,
                VarType::Map {
                    val_vt: Box::new(VarType::Number),
                },
            ));
        }

        // new Promise(executor) — allocate a MgPromise, build a resolve-shim closure,
        // call the executor synchronously with that closure as its first argument,
        // and return the promise pointer.
        //
        // The resolve shim is a real LLVM function:
        //   void __promise_resolve_N(void* val, void* ctx)
        //     → tscc_promise_resolve((MgPromise*)ctx, val)
        //
        // The shim + promise_ptr form a standard {fn_ptr, env_ptr} closure struct that
        // the executor receives as its `resolve` parameter.  Because it follows the normal
        // closure ABI, it can be stored, passed to setTimeout, or called directly — no
        // special-casing at the call site.
        if class_name == "Promise" {
            if args.len() != 1 {
                return Err(CompileError::error(
                    "new Promise() requires exactly one executor function argument",
                    span.clone(),
                ));
            }

            let ptr_type = self.context.ptr_type(AddressSpace::default());

            // 1. Allocate the promise: MgPromise* promise = tscc_promise_new()
            let promise_new_fn = self.module.get_function("tscc_promise_new").unwrap();
            let promise_ptr = self
                .builder
                .build_call(promise_new_fn, &[], "promise_ptr")
                .unwrap()
                .try_as_basic_value()
                .left()
                .unwrap()
                .into_pointer_value();

            // 2. Emit the resolve shim function (once per call site, uniquely named).
            //    Signature: void __promise_resolve_N(void* val, void* ctx)
            //    Body: tscc_promise_resolve(ctx, val)
            let shim_name = format!("__promise_resolve_{}", self.arrow_counter);
            self.arrow_counter += 1;

            let void_type = self.context.void_type();
            let shim_fn_type = void_type.fn_type(&[ptr_type.into(), ptr_type.into()], false);
            let shim_fn = self.module.add_function(&shim_name, shim_fn_type, None);
            {
                let shim_bb = self.context.append_basic_block(shim_fn, "entry");
                let saved_bb = self.builder.get_insert_block();
                self.builder.position_at_end(shim_bb);

                let val_arg = shim_fn.get_nth_param(0).unwrap().into_pointer_value();
                let ctx_arg = shim_fn.get_nth_param(1).unwrap().into_pointer_value();

                let resolve_fn = self.module.get_function("tscc_promise_resolve").unwrap();
                self.builder
                    .build_call(
                        resolve_fn,
                        &[ctx_arg.into(), val_arg.into()],
                        "resolve_call",
                    )
                    .unwrap();
                self.builder.build_return(None).unwrap();

                // Restore insertion point
                if let Some(bb) = saved_bb {
                    self.builder.position_at_end(bb);
                }
            }

            // 3. Build the resolve closure struct { fn_ptr: shim, env_ptr: promise_ptr }
            //    This is identical to the closure layout produced by compile_closure().
            let shim_fn_ptr = shim_fn.as_global_value().as_pointer_value();
            let resolve_closure_val = {
                let zero = self.closure_type.const_zero();
                let with_fn = self
                    .builder
                    .build_insert_value(zero, shim_fn_ptr, 0, "resolve.fn")
                    .unwrap()
                    .into_struct_value();
                self.builder
                    .build_insert_value(with_fn, promise_ptr, 1, "resolve.env")
                    .unwrap()
                    .into_struct_value()
            };

            // Store the resolve closure into an alloca so compile_closure_call can load it.
            let resolve_alloca = self
                .builder
                .build_alloca(self.closure_type, "resolve_alloca")
                .unwrap();
            self.builder
                .build_store(resolve_alloca, resolve_closure_val)
                .unwrap();

            // 4. Compile the executor expression — must be an arrow / closure.
            let (exec_val, exec_vt) = self.compile_expr(&args[0], function)?;

            // The executor has signature: (resolve: (val) => void) => void
            // Its resolve parameter is a Closure<[Number] -> Number> (Number is the void stand-in).
            let resolve_param_vt = VarType::Closure {
                fn_name: shim_name.clone(),
                param_types: vec![VarType::Number],
                return_type: Box::new(VarType::Number),
            };

            match exec_vt {
                VarType::Closure {
                    ref param_types,
                    ref return_type,
                    ..
                } => {
                    // Store executor struct into an alloca to hand to compile_closure_call.
                    let exec_alloca = self
                        .builder
                        .build_alloca(self.closure_type, "executor_alloca")
                        .unwrap();
                    self.builder
                        .build_store(exec_alloca, exec_val.into_struct_value())
                        .unwrap();

                    // Build the LLVM call manually so we can pass an already-compiled arg
                    // (the resolve_alloca) rather than going through &[Expr].
                    let closure_struct = self
                        .builder
                        .build_load(self.closure_type, exec_alloca, "exec_closure")
                        .unwrap();
                    let exec_fn_ptr = self
                        .builder
                        .build_extract_value(closure_struct.into_struct_value(), 0, "exec_fn_ptr")
                        .unwrap()
                        .into_pointer_value();
                    let exec_env_ptr = self
                        .builder
                        .build_extract_value(closure_struct.into_struct_value(), 1, "exec_env_ptr")
                        .unwrap();

                    // Executor LLVM type: (ptr env, closure_type resolve) -> void
                    let exec_fn_type =
                        void_type.fn_type(&[ptr_type.into(), self.closure_type.into()], false);

                    // Load the resolve closure value to pass by value.
                    let resolve_val = self
                        .builder
                        .build_load(self.closure_type, resolve_alloca, "resolve_val")
                        .unwrap();

                    let _ = param_types; // acknowledged — executor takes one closure arg
                    let _ = return_type;

                    self.builder
                        .build_indirect_call(
                            exec_fn_type,
                            exec_fn_ptr,
                            &[exec_env_ptr.into(), resolve_val.into()],
                            "executor_call",
                        )
                        .unwrap();
                }
                _ => {
                    return Err(CompileError::error(
                        "new Promise() executor must be a function",
                        span.clone(),
                    ));
                }
            }

            let _ = resolve_param_vt; // used above for documentation; silences unused warning

            return Ok((
                promise_ptr.into(),
                VarType::Promise {
                    inner_vt: Box::new(VarType::Number),
                },
            ));
        }

        let (struct_type, field_info, parent_class) = self
            .class_struct_types
            .get(class_name)
            .cloned()
            .ok_or_else(|| {
                CompileError::error(format!("Undefined class '{}'", class_name), span.clone())
            })?;

        let obj_vt = VarType::Object {
            struct_type_name: class_name.to_string(),
            fields: field_info.clone(),
        };

        // Allocate the struct on the stack
        let alloca = self.create_alloca(function, &obj_vt, &format!("{}_inst", class_name));

        // Zero-initialize
        let zero = struct_type.const_zero();
        self.builder.build_store(alloca, zero).unwrap();

        // Run field initializers (parent hierarchy first, then own)
        let prev_this = self.current_this.clone();
        self.current_this = Some((alloca, obj_vt.clone()));
        self.run_field_initializers(class_name, alloca, struct_type, &field_info, function)?;
        self.current_this = prev_this;

        // Call constructor if it exists (check own constructor, then parent's)
        let ctor_name = format!("{}_constructor", class_name);
        let ctor_fn = self.functions.get(&ctor_name).copied();
        if let Some(ctor) = ctor_fn {
            let mut ctor_args: Vec<BasicMetadataValueEnum> = vec![alloca.into()];
            for arg in args {
                let (val, _) = self.compile_expr(arg, function)?;
                ctor_args.push(val.into());
            }
            self.builder.build_call(ctor, &ctor_args, "").unwrap();
        } else if let Some(ref pname) = parent_class {
            // No own constructor — call parent constructor
            let parent_ctor_name = format!("{}_constructor", pname);
            if let Some(parent_ctor) = self.functions.get(&parent_ctor_name).copied() {
                let mut ctor_args: Vec<BasicMetadataValueEnum> = vec![alloca.into()];
                for arg in args {
                    let (val, _) = self.compile_expr(arg, function)?;
                    ctor_args.push(val.into());
                }
                self.builder
                    .build_call(parent_ctor, &ctor_args, "")
                    .unwrap();
            }
        }

        // Load and return the struct
        let val = self.builder.build_load(struct_type, alloca, "obj").unwrap();

        Ok((val, obj_vt))
    }

    /// Compile an array literal as a tuple struct.
    /// Each element is compiled to its expected type and inserted into the struct.
    fn compile_tuple_literal(
        &mut self,
        elements: &[Expr],
        elem_types: &[VarType],
        function: FunctionValue<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, VarType), CompileError> {
        let tuple_vt = VarType::Tuple(elem_types.to_vec());
        let llvm_type = self.var_type_to_llvm(&tuple_vt).into_struct_type();
        let mut struct_val = llvm_type.const_zero();

        for (i, elem) in elements.iter().enumerate() {
            if i >= elem_types.len() {
                break;
            }
            let (val, _vt) = self.compile_expr(elem, function)?;
            struct_val = self
                .builder
                .build_insert_value(struct_val, val, i as u32, "tup.elem")
                .unwrap()
                .into_struct_value();
        }

        Ok((struct_val.into(), tuple_vt))
    }

    fn compile_class_decl(
        &mut self,
        name: &str,
        parent: &Option<String>,
        fields: &[ClassField],
        constructor: &Option<ClassConstructor>,
        methods: &[ClassMethod],
        function: FunctionValue<'ctx>,
    ) -> Result<(), CompileError> {
        let _ = function; // Class decl doesn't use the current function directly

        // Collect all fields (parent first, then own)
        let mut all_fields: Vec<(String, VarType)> = Vec::new();

        if let Some(parent_name) = parent {
            if let Some((_, parent_fields, _)) = self.class_struct_types.get(parent_name) {
                all_fields.extend(parent_fields.clone());
            }
        }

        // Add own value fields
        for field in fields {
            let vt = field
                .type_ann
                .as_ref()
                .map(|ann| self.type_ann_to_var_type(ann))
                .unwrap_or(VarType::Number);
            // Override parent field if same name
            all_fields.retain(|(n, _)| n != &field.name);
            all_fields.push((field.name.clone(), vt));
        }

        // Add method fields (as function pointers)
        for method in methods {
            all_fields.retain(|(n, _)| n != &method.name);
            let method_fn_name = format!("{}_{}", name, method.name);
            all_fields.push((
                method.name.clone(),
                VarType::FunctionPtr {
                    fn_name: method_fn_name,
                },
            ));
        }

        // Create the LLVM struct type
        let field_llvm_types: Vec<BasicTypeEnum> = all_fields
            .iter()
            .map(|(_, vt)| self.var_type_to_llvm(vt))
            .collect();
        let struct_type = self.context.struct_type(&field_llvm_types, false);

        self.class_struct_types.insert(
            name.to_string(),
            (struct_type, all_fields.clone(), parent.clone()),
        );

        // Collect field initializers for use in compile_new_expr
        let initializers: Vec<(String, Expr)> = fields
            .iter()
            .filter_map(|f| f.initializer.as_ref().map(|e| (f.name.clone(), e.clone())))
            .collect();
        if !initializers.is_empty() {
            self.class_field_initializers
                .insert(name.to_string(), initializers);
        }

        let obj_vt = VarType::Object {
            struct_type_name: name.to_string(),
            fields: all_fields.clone(),
        };

        // Compile constructor
        if let Some(ctor) = constructor {
            let ctor_name = format!("{}_constructor", name);
            let ptr_type = self.context.ptr_type(AddressSpace::default());

            // Constructor takes a pointer to the struct (self) + parameters
            let mut param_types: Vec<BasicMetadataTypeEnum> = vec![ptr_type.into()];
            for param in &ctor.params {
                let vt = param
                    .type_ann
                    .as_ref()
                    .map(|ann| self.type_ann_to_var_type(ann))
                    .unwrap_or(VarType::Number);
                param_types.push(self.var_type_to_llvm(&vt).into());
            }

            let fn_type = self.context.void_type().fn_type(&param_types, false);
            let ctor_fn = self.module.add_function(&ctor_name, fn_type, None);

            // Add nounwind attribute
            let nounwind_kind = Attribute::get_named_enum_kind_id("nounwind");
            let nounwind = self.context.create_enum_attribute(nounwind_kind, 0);
            ctor_fn.add_attribute(AttributeLoc::Function, nounwind);

            self.functions.insert(ctor_name.clone(), ctor_fn);

            let entry = self.context.append_basic_block(ctor_fn, "entry");
            let saved_block = self.builder.get_insert_block();
            self.builder.position_at_end(entry);

            // Set up `this` as the first parameter (pointer to struct)
            let this_ptr = ctor_fn.get_nth_param(0).unwrap().into_pointer_value();
            let prev_this = self.current_this.clone();
            self.current_this = Some((this_ptr, obj_vt.clone()));

            self.push_scope();

            // Register constructor parameters
            for (i, param) in ctor.params.iter().enumerate() {
                let vt = param
                    .type_ann
                    .as_ref()
                    .map(|ann| self.type_ann_to_var_type(ann))
                    .unwrap_or(VarType::Number);
                let param_val = ctor_fn.get_nth_param((i + 1) as u32).unwrap();
                let alloca = self.create_alloca(ctor_fn, &vt, &param.name);
                self.builder.build_store(alloca, param_val).unwrap();
                self.set_variable(param.name.clone(), alloca, vt);
            }

            for stmt in &ctor.body {
                self.compile_statement(stmt, ctor_fn)?;
            }

            // Ensure the constructor returns void
            if self
                .builder
                .get_insert_block()
                .unwrap()
                .get_terminator()
                .is_none()
            {
                self.builder.build_return(None).unwrap();
            }

            self.pop_scope();
            self.current_this = prev_this;

            if let Some(bb) = saved_block {
                self.builder.position_at_end(bb);
            }
        }

        // Compile methods
        for method in methods {
            let method_fn_name = format!("{}_{}", name, method.name);
            let ptr_type = self.context.ptr_type(AddressSpace::default());

            // Method takes self pointer + parameters
            let mut param_types: Vec<BasicMetadataTypeEnum> = vec![ptr_type.into()];
            let mut param_vts: Vec<VarType> = Vec::new();
            for param in &method.params {
                let vt = param
                    .type_ann
                    .as_ref()
                    .map(|ann| self.type_ann_to_var_type(ann))
                    .unwrap_or(VarType::Number);
                param_types.push(self.var_type_to_llvm(&vt).into());
                param_vts.push(vt);
            }

            let ret_vt = method
                .return_type
                .as_ref()
                .map(|ann| self.type_ann_to_var_type(ann))
                .unwrap_or(VarType::Number);

            let ret_type = self.var_type_to_llvm(&ret_vt);
            let fn_type = ret_type.fn_type(&param_types, false);
            let method_fn = self.module.add_function(&method_fn_name, fn_type, None);

            let nounwind_kind = Attribute::get_named_enum_kind_id("nounwind");
            let nounwind = self.context.create_enum_attribute(nounwind_kind, 0);
            method_fn.add_attribute(AttributeLoc::Function, nounwind);

            self.functions.insert(method_fn_name.clone(), method_fn);

            let entry = self.context.append_basic_block(method_fn, "entry");
            let saved_block = self.builder.get_insert_block();
            self.builder.position_at_end(entry);

            let this_ptr = method_fn.get_nth_param(0).unwrap().into_pointer_value();
            let prev_this = self.current_this.clone();
            self.current_this = Some((this_ptr, obj_vt.clone()));

            self.push_scope();

            for (i, param) in method.params.iter().enumerate() {
                let vt = param_vts[i].clone();
                let param_val = method_fn.get_nth_param((i + 1) as u32).unwrap();
                let alloca = self.create_alloca(method_fn, &vt, &param.name);
                self.builder.build_store(alloca, param_val).unwrap();
                self.set_variable(param.name.clone(), alloca, vt);
            }

            let saved_mode = self.number_mode.clone();
            // Methods are not integer-narrowed
            self.number_mode = VarType::Number;

            for stmt in &method.body {
                self.compile_statement(stmt, method_fn)?;
            }

            // If no terminator, add a default return
            if self
                .builder
                .get_insert_block()
                .unwrap()
                .get_terminator()
                .is_none()
            {
                let default_ret = self.default_value(&ret_vt);
                self.builder.build_return(Some(&default_ret)).unwrap();
            }

            self.number_mode = saved_mode;
            self.pop_scope();
            self.current_this = prev_this;

            if let Some(bb) = saved_block {
                self.builder.position_at_end(bb);
            }
        }

        Ok(())
    }

    fn compile_typeof(
        &mut self,
        operand: &Expr,
        function: FunctionValue<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, VarType), CompileError> {
        // Special case: typeof on a union-typed variable → runtime tag check
        if let ExprKind::Identifier(name) = &operand.kind {
            if let Some((ptr, VarType::Union(_))) = self.get_variable(name) {
                return self.compile_typeof_union(ptr, function);
            }
        }

        let (_val, vt) = self.compile_expr(operand, function)?;
        let type_str = match vt {
            VarType::Number | VarType::Integer => "number",
            VarType::String => "string",
            VarType::Boolean => "boolean",
            VarType::Array | VarType::StringArray | VarType::Object { .. } | VarType::Tuple(_) => {
                "object"
            }
            VarType::FunctionPtr { .. } | VarType::Closure { .. } => "function",
            VarType::Union(_) => "object", // fallback for non-identifier unions
            VarType::Map { .. } | VarType::ObjArray { .. } => "object",
            VarType::Promise { .. } => "object",
        };
        Ok((self.create_string_literal(type_str), VarType::String))
    }

    /// Compile `typeof` for a union-typed variable at runtime.
    /// Reads the tag from the union struct and returns the appropriate type string.
    fn compile_typeof_union(
        &mut self,
        union_ptr: PointerValue<'ctx>,
        function: FunctionValue<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, VarType), CompileError> {
        let union_llvm_type = self.get_union_llvm_type();
        let tag_ptr = self
            .builder
            .build_struct_gep(union_llvm_type, union_ptr, 0, "tag_ptr")
            .unwrap();
        let tag = self
            .builder
            .build_load(self.context.i8_type(), tag_ptr, "tag")
            .unwrap()
            .into_int_value();

        let number_bb = self.context.append_basic_block(function, "typeof_number");
        let string_bb = self.context.append_basic_block(function, "typeof_string");
        let boolean_bb = self.context.append_basic_block(function, "typeof_boolean");
        let merge_bb = self.context.append_basic_block(function, "typeof_merge");

        self.builder
            .build_switch(
                tag,
                string_bb, // default
                &[
                    (self.context.i8_type().const_int(0, false), number_bb),
                    (self.context.i8_type().const_int(1, false), string_bb),
                    (self.context.i8_type().const_int(2, false), boolean_bb),
                ],
            )
            .unwrap();

        self.builder.position_at_end(number_bb);
        let number_str = self.create_string_literal("number");
        self.builder.build_unconditional_branch(merge_bb).unwrap();
        let number_bb_end = self.builder.get_insert_block().unwrap();

        self.builder.position_at_end(string_bb);
        let string_str = self.create_string_literal("string");
        self.builder.build_unconditional_branch(merge_bb).unwrap();
        let string_bb_end = self.builder.get_insert_block().unwrap();

        self.builder.position_at_end(boolean_bb);
        let boolean_str = self.create_string_literal("boolean");
        self.builder.build_unconditional_branch(merge_bb).unwrap();
        let boolean_bb_end = self.builder.get_insert_block().unwrap();

        self.builder.position_at_end(merge_bb);
        let phi = self
            .builder
            .build_phi(self.string_type, "typeof_result")
            .unwrap();
        phi.add_incoming(&[
            (&number_str, number_bb_end),
            (&string_str, string_bb_end),
            (&boolean_str, boolean_bb_end),
        ]);

        Ok((phi.as_basic_value(), VarType::String))
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

        // Object member access
        let (obj_val, obj_vt) = self.compile_expr(object, function)?;

        // Map .size property
        if let VarType::Map { .. } = obj_vt {
            if property == "size" {
                let size_fn = self.module.get_function("tscc_map_size").unwrap();
                let count = self
                    .builder
                    .build_call(size_fn, &[obj_val.into_pointer_value().into()], "msize")
                    .unwrap()
                    .try_as_basic_value()
                    .left()
                    .unwrap();
                let count_f64 = self
                    .builder
                    .build_signed_int_to_float(
                        count.into_int_value(),
                        self.context.f64_type(),
                        "msizef",
                    )
                    .unwrap();
                return Ok((count_f64.into(), VarType::Number));
            }
        }

        // ObjArray .length
        if let VarType::ObjArray { .. } = obj_vt {
            if property == "length" {
                let len = self
                    .builder
                    .build_extract_value(obj_val.into_struct_value(), 1, "oa_len")
                    .unwrap()
                    .into_int_value();
                let len_f64 = self
                    .builder
                    .build_signed_int_to_float(len, self.context.f64_type(), "oa_lenf")
                    .unwrap();
                return Ok((len_f64.into(), VarType::Number));
            }
        }

        // Array .length
        if matches!(obj_vt, VarType::Array) && property == "length" {
            let len = self
                .builder
                .build_extract_value(obj_val.into_struct_value(), 1, "arrlen")
                .unwrap();
            let len_f64 = self
                .builder
                .build_signed_int_to_float(len.into_int_value(), self.context.f64_type(), "lenf")
                .unwrap();
            return Ok((len_f64.into(), VarType::Number));
        }

        // String .length
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

        // Object/class property access
        if let VarType::Object { ref fields, .. } = obj_vt {
            for (i, (name, field_vt)) in fields.iter().enumerate() {
                if name == property {
                    let val = self
                        .builder
                        .build_extract_value(
                            obj_val.into_struct_value(),
                            i as u32,
                            &format!("obj.{}", property),
                        )
                        .unwrap();
                    return Ok((val.into(), field_vt.clone()));
                }
            }
            return Err(CompileError::error(
                format!("Property '{}' does not exist on object", property),
                span.clone(),
            ));
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

            let concat_fn = self.module.get_function("tscc_string_concat").unwrap();
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

        // String equality / inequality: compare lengths then bytes with memcmp
        if matches!(left_vt, VarType::String) && matches!(right_vt, VarType::String) {
            if matches!(
                op,
                BinOp::Equal | BinOp::StrictEqual | BinOp::NotEqual | BinOp::StrictNotEqual
            ) {
                let ls = left_val.into_struct_value();
                let rs = right_val.into_struct_value();
                let lp = self
                    .builder
                    .build_extract_value(ls, 0, "lp")
                    .unwrap()
                    .into_pointer_value();
                let ll = self
                    .builder
                    .build_extract_value(ls, 1, "ll")
                    .unwrap()
                    .into_int_value();
                let rp = self
                    .builder
                    .build_extract_value(rs, 0, "rp")
                    .unwrap()
                    .into_pointer_value();
                let rl = self
                    .builder
                    .build_extract_value(rs, 1, "rl")
                    .unwrap()
                    .into_int_value();

                // First compare lengths
                let len_eq = self
                    .builder
                    .build_int_compare(IntPredicate::EQ, ll, rl, "seq_len")
                    .unwrap();

                // If lengths match, compare bytes with memcmp
                let memcmp_fn = self.module.get_function("memcmp").unwrap();
                let cmp_result = self
                    .builder
                    .build_call(memcmp_fn, &[lp.into(), rp.into(), ll.into()], "smemcmp")
                    .unwrap()
                    .try_as_basic_value()
                    .left()
                    .unwrap()
                    .into_int_value();
                let zero_i32 = self.context.i32_type().const_int(0, false);
                let bytes_eq = self
                    .builder
                    .build_int_compare(IntPredicate::EQ, cmp_result, zero_i32, "seq_bytes")
                    .unwrap();

                // Both length and bytes must match
                let str_eq = self.builder.build_and(len_eq, bytes_eq, "seq").unwrap();

                let result = match op {
                    BinOp::Equal | BinOp::StrictEqual => str_eq.into(),
                    _ => self.builder.build_not(str_eq, "sne").unwrap().into(),
                };
                return Ok((result, VarType::Boolean));
            }
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
                BinOp::Power => {
                    // Convert i64 → f64, call pow, convert back
                    let lf = self
                        .builder
                        .build_signed_int_to_float(li, self.context.f64_type(), "l2f")
                        .unwrap();
                    let rf = self
                        .builder
                        .build_signed_int_to_float(ri, self.context.f64_type(), "r2f")
                        .unwrap();
                    let pow_fn = self.module.get_function("tscc_math_pow").unwrap();
                    let result = self
                        .builder
                        .build_call(pow_fn, &[lf.into(), rf.into()], "pow")
                        .unwrap()
                        .try_as_basic_value()
                        .left()
                        .unwrap();
                    self.builder
                        .build_float_to_signed_int(
                            result.into_float_value(),
                            self.context.i64_type(),
                            "f2i",
                        )
                        .unwrap()
                        .into()
                }
                BinOp::NullishCoalescing => unreachable!("?? handled before compile_binary"),
            };

            let result_type = match op {
                BinOp::Add
                | BinOp::Subtract
                | BinOp::Multiply
                | BinOp::Divide
                | BinOp::Modulo
                | BinOp::Power => VarType::Integer,
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
                BinOp::Power => {
                    let pow_fn = self.module.get_function("tscc_math_pow").unwrap();
                    self.builder
                        .build_call(pow_fn, &[lf.into(), rf.into()], "pow")
                        .unwrap()
                        .try_as_basic_value()
                        .left()
                        .unwrap()
                }
                BinOp::NullishCoalescing => unreachable!("?? handled before compile_binary"),
            };

            let result_type = match op {
                BinOp::Add
                | BinOp::Subtract
                | BinOp::Multiply
                | BinOp::Divide
                | BinOp::Modulo
                | BinOp::Power => VarType::Number,
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
                // Number static methods
                if name == "Number" {
                    return self.compile_number_static_call(property, args, function, span);
                }
                // Date.now() — static method
                if name == "Date" {
                    if property == "now" {
                        if !args.is_empty() {
                            return Err(CompileError::error(
                                "Date.now() takes no arguments",
                                span.clone(),
                            ));
                        }
                        let f = self.module.get_function("tscc_date_now").unwrap();
                        let i64_result = self
                            .builder
                            .build_call(f, &[], "date_now")
                            .unwrap()
                            .try_as_basic_value()
                            .left()
                            .unwrap()
                            .into_int_value();
                        let f64_result = self
                            .builder
                            .build_signed_int_to_float(
                                i64_result,
                                self.context.f64_type(),
                                "date_now_f64",
                            )
                            .unwrap();
                        return Ok((f64_result.into(), VarType::Number));
                    }
                    return Err(CompileError::error(
                        format!("Date.{} is not implemented", property),
                        span.clone(),
                    ));
                }

                // crypto.*
                if name == "crypto" {
                    if property == "randomUUID" {
                        if !args.is_empty() {
                            return Err(CompileError::error(
                                "crypto.randomUUID() takes no arguments",
                                span.clone(),
                            ));
                        }
                        let f = self.module.get_function("tscc_crypto_random_uuid").unwrap();
                        let result = self
                            .builder
                            .build_call(f, &[], "uuid")
                            .unwrap()
                            .try_as_basic_value()
                            .left()
                            .unwrap();
                        return Ok((result, VarType::String));
                    }
                    return Err(CompileError::error(
                        format!("crypto.{} is not implemented", property),
                        span.clone(),
                    ));
                }
            }

            // String methods: object.method(args)
            let (obj_val, obj_vt) = self.compile_expr(object, function)?;
            if matches!(obj_vt, VarType::String) {
                return self.compile_string_method(obj_val, property, args, function, span);
            }

            // Number methods: value.toFixed(digits)
            if matches!(obj_vt, VarType::Number | VarType::Integer) {
                return self
                    .compile_number_method(obj_val, &obj_vt, property, args, function, span);
            }

            // Array methods: object.method(args)
            if matches!(obj_vt, VarType::Array) {
                return self.compile_array_method(object, obj_val, property, args, function, span);
            }

            // ObjArray methods: length, forEach
            if let VarType::ObjArray { ref elem_vt } = obj_vt {
                let elem_vt = elem_vt.clone();
                return self
                    .compile_obj_array_method(obj_val, &elem_vt, property, args, function, span);
            }

            // Map methods: set, get, has, delete, size, values
            if let VarType::Map { ref val_vt } = obj_vt {
                let val_vt = val_vt.clone();
                return self
                    .compile_map_method(object, obj_val, &val_vt, property, args, function, span);
            }

            // Date instance method calls
            if let VarType::Object {
                ref struct_type_name,
                ..
            } = obj_vt
            {
                if struct_type_name == "Date" {
                    return self.compile_date_method(obj_val, property, args, span);
                }
            }

            // Object/class method calls
            if let VarType::Object {
                ref fields,
                ref struct_type_name,
                ..
            } = obj_vt
            {
                for (_, (fname, fvt)) in fields.iter().enumerate() {
                    if fname == property {
                        if let VarType::FunctionPtr { fn_name } = fvt {
                            // Call the method function, passing `self` pointer as first arg
                            let method_fn =
                                self.functions.get(fn_name).copied().ok_or_else(|| {
                                    CompileError::error(
                                        format!("Method '{}' not compiled", property),
                                        span.clone(),
                                    )
                                })?;

                            // We need to get a pointer to the object for `this`
                            // If the object is a variable, use its alloca
                            let obj_ptr = if let ExprKind::Identifier(var_name) = &object.kind {
                                let (ptr, _) = self.get_variable(var_name).unwrap();
                                ptr
                            } else {
                                // Object is a temporary — store it in an alloca
                                let alloca = self.create_alloca(function, &obj_vt, "tmp_obj");
                                self.builder.build_store(alloca, obj_val).unwrap();
                                alloca
                            };

                            let mut call_args: Vec<BasicMetadataValueEnum> = vec![obj_ptr.into()];
                            for arg in args {
                                let (val, _) = self.compile_expr(arg, function)?;
                                call_args.push(val.into());
                            }

                            let result = self
                                .builder
                                .build_call(method_fn, &call_args, "method_call")
                                .unwrap();

                            if let Some(val) = result.try_as_basic_value().left() {
                                // Determine return type from the method's return type
                                let ret_type = method_fn.get_type().get_return_type();
                                let ret_vt = if let Some(rt) = ret_type {
                                    if rt.is_float_type() {
                                        VarType::Number
                                    } else if rt.is_int_type() {
                                        let bw = rt.into_int_type().get_bit_width();
                                        if bw == 1 {
                                            VarType::Boolean
                                        } else {
                                            VarType::Integer
                                        }
                                    } else if rt.is_struct_type() {
                                        // Check if it's a string type
                                        let st = rt.into_struct_type();
                                        if st.count_fields() == 2 {
                                            VarType::String
                                        } else if let Some((_, fi, _)) =
                                            self.class_struct_types.get(struct_type_name)
                                        {
                                            VarType::Object {
                                                struct_type_name: struct_type_name.clone(),
                                                fields: fi.clone(),
                                            }
                                        } else {
                                            VarType::String
                                        }
                                    } else {
                                        VarType::Number
                                    }
                                } else {
                                    VarType::Number
                                };
                                return Ok((val, ret_vt));
                            } else {
                                return Ok((
                                    self.context.f64_type().const_float(0.0).into(),
                                    VarType::Number,
                                ));
                            }
                        }
                    }
                }
            }
        }

        // Global functions: parseInt, parseFloat
        if let ExprKind::Identifier(name) = &callee.kind {
            if name == "parseInt" || name == "parseFloat" {
                return self.compile_global_func(name, args, function, span);
            }

            // Check if it's a closure variable first
            if let Some((var_ptr, var_vt)) = self.get_variable(name) {
                if let VarType::Closure {
                    ref param_types,
                    ref return_type,
                    ..
                } = var_vt
                {
                    return self.compile_closure_call(
                        var_ptr,
                        param_types,
                        return_type,
                        args,
                        function,
                        span,
                    );
                }
            }

            // Check if this is a call to a generic function — monomorphize on demand
            if self.generic_templates.contains_key(name.as_str()) {
                return self.compile_generic_call(name, args, function, span);
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

            // Check if function has a rest parameter
            let rest_idx = self.function_rest_param_index.get(name).copied();
            if let Some(rest_idx) = rest_idx {
                // Compile regular args 0..rest_idx
                for arg in args.iter().take(rest_idx) {
                    let (val, vt) = self.compile_expr(arg, function)?;
                    let val =
                        if is_target_integer && !caller_is_integer && matches!(vt, VarType::Number)
                        {
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
                // Pack rest args into an array
                let rest_args = &args[rest_idx.min(args.len())..];
                let f64_type = self.context.f64_type();
                let i64_type = self.context.i64_type();
                let ptr_type = self.context.ptr_type(AddressSpace::default());
                let rest_count = rest_args.len() as u64;
                let capacity = rest_count.max(4);
                let malloc_fn = self.module.get_function("malloc").unwrap_or_else(|| {
                    self.module.add_function(
                        "malloc",
                        ptr_type.fn_type(&[i64_type.into()], false),
                        None,
                    )
                });
                let alloc_size = i64_type.const_int(capacity * 8, false);
                let data_ptr = self
                    .builder
                    .build_call(malloc_fn, &[alloc_size.into()], "rest_data")
                    .unwrap()
                    .try_as_basic_value()
                    .left()
                    .unwrap()
                    .into_pointer_value();
                for (i, arg) in rest_args.iter().enumerate() {
                    let (val, vt) = self.compile_expr(arg, function)?;
                    let float_val: BasicValueEnum = match vt {
                        VarType::Integer => self
                            .builder
                            .build_signed_int_to_float(val.into_int_value(), f64_type, "i2f")
                            .unwrap()
                            .into(),
                        _ => val,
                    };
                    let idx = i64_type.const_int(i as u64, false);
                    let elem_ptr = unsafe {
                        self.builder
                            .build_gep(f64_type, data_ptr, &[idx], "rp")
                            .unwrap()
                    };
                    self.builder.build_store(elem_ptr, float_val).unwrap();
                }
                let rest_arr = self.array_type.const_zero();
                let rest_arr = self
                    .builder
                    .build_insert_value(rest_arr, data_ptr, 0, "rest.data")
                    .unwrap();
                let rest_arr = self
                    .builder
                    .build_insert_value(
                        rest_arr.into_struct_value(),
                        i64_type.const_int(rest_count, false),
                        1,
                        "rest.len",
                    )
                    .unwrap();
                let rest_arr = self
                    .builder
                    .build_insert_value(
                        rest_arr.into_struct_value(),
                        i64_type.const_int(capacity, false),
                        2,
                        "rest.cap",
                    )
                    .unwrap();
                compiled_args.push(rest_arr.into_struct_value().into());
            } else {
                let target_param_vts = self.function_param_var_types.get(name).cloned();
                for (arg_idx, arg) in args.iter().enumerate() {
                    let (val, vt) = self.compile_expr(arg, function)?;

                    // Check if the target parameter is a union type
                    let is_union_param = target_param_vts
                        .as_ref()
                        .and_then(|pvts| pvts.get(arg_idx))
                        .map(|pvt| matches!(pvt, VarType::Union(_)))
                        .unwrap_or(false);

                    let val = if is_union_param && !matches!(vt, VarType::Union(_)) {
                        // Wrap concrete value in union struct for union-typed parameter
                        let union_ptr = self.wrap_in_union(val, &vt, function);
                        let union_type = self.get_union_llvm_type();
                        self.builder
                            .build_load(union_type, union_ptr, "union_arg")
                            .unwrap()
                    } else if is_target_integer
                        && !caller_is_integer
                        && matches!(vt, VarType::Number)
                    {
                        // Convert f64 → i64 when calling an integer function from float context
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
            }

            // Fill in default parameter values for missing arguments
            if let Some(defaults) = self.function_param_defaults.get(name).cloned() {
                let expected = func.count_params() as usize;
                for i in args.len()..expected {
                    if let Some(Some(ref default_expr)) = defaults.get(i) {
                        let (val, vt) = self.compile_expr(default_expr, function)?;
                        let val = if is_target_integer
                            && !caller_is_integer
                            && matches!(vt, VarType::Number)
                        {
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
                }
            }

            let result = self
                .builder
                .build_call(func, &compiled_args, "call")
                .unwrap();

            if let Some(val) = result.try_as_basic_value().left() {
                // Use stored return type if available (more accurate than LLVM type inference)
                let ret_vt = if let Some(stored_vt) = self.function_return_types.get(name) {
                    stored_vt.clone()
                } else if let Some(ret_type) = func.get_type().get_return_type() {
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
                "tscc_eprint_number",
                "tscc_eprint_string",
                "tscc_eprint_boolean",
                "tscc_eprint_newline",
            )
        } else {
            (
                "tscc_print_number",
                "tscc_print_string",
                "tscc_print_boolean",
                "tscc_print_newline",
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
                VarType::Array => {
                    let data_ptr = self
                        .builder
                        .build_extract_value(val.into_struct_value(), 0, "arr_data")
                        .unwrap();
                    let length = self
                        .builder
                        .build_extract_value(val.into_struct_value(), 1, "arr_len")
                        .unwrap();
                    let print_arr_name = if is_stderr {
                        "tscc_eprint_array"
                    } else {
                        "tscc_print_array"
                    };
                    let f = self.module.get_function(print_arr_name).unwrap();
                    self.builder
                        .build_call(f, &[data_ptr.into(), length.into()], "")
                        .unwrap();
                }
                VarType::StringArray => {
                    let data_ptr = self
                        .builder
                        .build_extract_value(val.into_struct_value(), 0, "sarr_data")
                        .unwrap();
                    let length = self
                        .builder
                        .build_extract_value(val.into_struct_value(), 1, "sarr_len")
                        .unwrap();
                    let f = self.module.get_function("tscc_print_string_array").unwrap();
                    self.builder
                        .build_call(f, &[data_ptr.into(), length.into()], "")
                        .unwrap();
                }
                VarType::FunctionPtr { .. } | VarType::Closure { .. } => {
                    // Print [Function] for function values
                    let s = self.create_string_literal("[Function]");
                    let ptr = self
                        .builder
                        .build_extract_value(s.into_struct_value(), 0, "p")
                        .unwrap();
                    let len = self
                        .builder
                        .build_extract_value(s.into_struct_value(), 1, "l")
                        .unwrap();
                    let f = self.module.get_function(print_str).unwrap();
                    self.builder
                        .build_call(f, &[ptr.into(), len.into()], "")
                        .unwrap();
                }
                VarType::Object { ref fields, .. } => {
                    // Print object as { key: value, key2: value2 }
                    self.print_string_literal("{ ", print_str);
                    for (fi, (fname, fvt)) in fields.iter().enumerate() {
                        if fi > 0 {
                            self.print_string_literal(", ", print_str);
                        }
                        // Print field name + ": "
                        self.print_string_literal(&format!("{}: ", fname), print_str);
                        // Extract field value from struct
                        let field_val = self
                            .builder
                            .build_extract_value(
                                val.into_struct_value(),
                                fi as u32,
                                &format!("obj.{}", fname),
                            )
                            .unwrap();
                        // Print the value based on its type
                        match fvt {
                            VarType::Number => {
                                let f = self.module.get_function(print_num).unwrap();
                                self.builder.build_call(f, &[field_val.into()], "").unwrap();
                            }
                            VarType::Integer => {
                                let float_val = self
                                    .builder
                                    .build_signed_int_to_float(
                                        field_val.into_int_value(),
                                        self.context.f64_type(),
                                        "i2f",
                                    )
                                    .unwrap();
                                let f = self.module.get_function(print_num).unwrap();
                                self.builder.build_call(f, &[float_val.into()], "").unwrap();
                            }
                            VarType::String => {
                                // Wrap string values in single quotes
                                self.print_string_literal("'", print_str);
                                let sp = self
                                    .builder
                                    .build_extract_value(field_val.into_struct_value(), 0, "sp")
                                    .unwrap();
                                let sl = self
                                    .builder
                                    .build_extract_value(field_val.into_struct_value(), 1, "sl")
                                    .unwrap();
                                let f = self.module.get_function(print_str).unwrap();
                                self.builder
                                    .build_call(f, &[sp.into(), sl.into()], "")
                                    .unwrap();
                                self.print_string_literal("'", print_str);
                            }
                            VarType::Boolean => {
                                let f = self.module.get_function(print_bool).unwrap();
                                self.builder.build_call(f, &[field_val.into()], "").unwrap();
                            }
                            _ => {
                                self.print_string_literal("[complex]", print_str);
                            }
                        }
                    }
                    self.print_string_literal(" }", print_str);
                }
                VarType::Union(_) => {
                    // Unions should be narrowed before printing; fallback to [union]
                    self.print_string_literal("[union]", print_str);
                }
                VarType::Tuple(_) => {
                    // Tuples shouldn't reach here — elements are indexed individually
                    self.print_string_literal("[tuple]", print_str);
                }
                VarType::Map { .. } => {
                    self.print_string_literal("[Map]", print_str);
                }
                VarType::ObjArray { .. } => {
                    self.print_string_literal("[Array]", print_str);
                }
                VarType::Promise { .. } => {
                    self.print_string_literal("[Promise]", print_str);
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
        let func_name = format!("tscc_math_{}", method);
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
                let func_name = format!("tscc_string_{}", method);
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
                let func = self.module.get_function("tscc_string_charAt").unwrap();
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
                let func = self.module.get_function("tscc_string_indexOf").unwrap();
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
                let func = self.module.get_function("tscc_string_includes").unwrap();
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
                let func_name = format!("tscc_string_{}", method);
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
            "startsWith" | "endsWith" => {
                let (needle, _) = self.compile_expr(&args[0], function)?;
                let np = self
                    .builder
                    .build_extract_value(needle.into_struct_value(), 0, "np")
                    .unwrap();
                let nl = self
                    .builder
                    .build_extract_value(needle.into_struct_value(), 1, "nl")
                    .unwrap();
                let func_name = format!("tscc_string_{}", method);
                let func = self.module.get_function(&func_name).unwrap();
                let result = self
                    .builder
                    .build_call(
                        func,
                        &[ptr.into(), len.into(), np.into(), nl.into()],
                        method,
                    )
                    .unwrap()
                    .try_as_basic_value()
                    .left()
                    .unwrap();
                Ok((result, VarType::Boolean))
            }
            "repeat" => {
                let (count, _) = self.compile_expr(&args[0], function)?;
                let func = self.module.get_function("tscc_string_repeat").unwrap();
                let result = self
                    .builder
                    .build_call(func, &[ptr.into(), len.into(), count.into()], "repeat")
                    .unwrap()
                    .try_as_basic_value()
                    .left()
                    .unwrap();
                Ok((result, VarType::String))
            }
            "replace" => {
                let (search_val, _) = self.compile_expr(&args[0], function)?;
                let (replace_val, _) = self.compile_expr(&args[1], function)?;
                let sp = self
                    .builder
                    .build_extract_value(search_val.into_struct_value(), 0, "sp")
                    .unwrap();
                let sl = self
                    .builder
                    .build_extract_value(search_val.into_struct_value(), 1, "sl")
                    .unwrap();
                let rp = self
                    .builder
                    .build_extract_value(replace_val.into_struct_value(), 0, "rp")
                    .unwrap();
                let rl = self
                    .builder
                    .build_extract_value(replace_val.into_struct_value(), 1, "rl")
                    .unwrap();
                let func = self.module.get_function("tscc_string_replace").unwrap();
                let result = self
                    .builder
                    .build_call(
                        func,
                        &[
                            ptr.into(),
                            len.into(),
                            sp.into(),
                            sl.into(),
                            rp.into(),
                            rl.into(),
                        ],
                        "replace",
                    )
                    .unwrap()
                    .try_as_basic_value()
                    .left()
                    .unwrap();
                Ok((result, VarType::String))
            }
            "padStart" => {
                let (target_len, _) = self.compile_expr(&args[0], function)?;
                let (pad_str, _) = self.compile_expr(&args[1], function)?;
                let pad_ptr = self
                    .builder
                    .build_extract_value(pad_str.into_struct_value(), 0, "pad_ptr")
                    .unwrap();
                let pad_len = self
                    .builder
                    .build_extract_value(pad_str.into_struct_value(), 1, "pad_len")
                    .unwrap();
                let func = self.module.get_function("tscc_string_padStart").unwrap();
                let result = self
                    .builder
                    .build_call(
                        func,
                        &[
                            ptr.into(),
                            len.into(),
                            target_len.into(),
                            pad_ptr.into(),
                            pad_len.into(),
                        ],
                        "padStart",
                    )
                    .unwrap()
                    .try_as_basic_value()
                    .left()
                    .unwrap();
                Ok((result, VarType::String))
            }
            "split" => {
                let (sep, _) = self.compile_expr(&args[0], function)?;
                let sep_ptr = self
                    .builder
                    .build_extract_value(sep.into_struct_value(), 0, "sep_ptr")
                    .unwrap();
                let sep_len = self
                    .builder
                    .build_extract_value(sep.into_struct_value(), 1, "sep_len")
                    .unwrap();

                let ptr_type = self.context.ptr_type(AddressSpace::default());
                let i64_type = self.context.i64_type();

                // Allocate out-parameters on stack
                let out_data = self
                    .builder
                    .build_alloca(ptr_type, "split.out_data")
                    .unwrap();
                let out_len = self
                    .builder
                    .build_alloca(i64_type, "split.out_len")
                    .unwrap();

                let func = self.module.get_function("tscc_string_split").unwrap();
                self.builder
                    .build_call(
                        func,
                        &[
                            ptr.into(),
                            len.into(),
                            sep_ptr.into(),
                            sep_len.into(),
                            out_data.into(),
                            out_len.into(),
                        ],
                        "",
                    )
                    .unwrap();

                // Load results
                let data_val = self
                    .builder
                    .build_load(ptr_type, out_data, "split.data")
                    .unwrap();
                let len_val = self
                    .builder
                    .build_load(i64_type, out_len, "split.len")
                    .unwrap();

                // Build a string array struct: { ptr, len, capacity }
                let arr_struct = self.array_type.const_zero();
                let arr_struct = self
                    .builder
                    .build_insert_value(arr_struct, data_val, 0, "sa.ptr")
                    .unwrap()
                    .into_struct_value();
                let arr_struct = self
                    .builder
                    .build_insert_value(arr_struct, len_val, 1, "sa.len")
                    .unwrap()
                    .into_struct_value();
                let arr_struct = self
                    .builder
                    .build_insert_value(arr_struct, len_val, 2, "sa.cap")
                    .unwrap()
                    .into_struct_value();

                Ok((arr_struct.into(), VarType::StringArray))
            }
            _ => Err(CompileError::error(
                format!("Unknown string method '{}'", method),
                span.clone(),
            )),
        }
    }

    fn compile_number_static_call(
        &mut self,
        method: &str,
        args: &[Expr],
        function: FunctionValue<'ctx>,
        span: &Span,
    ) -> Result<(BasicValueEnum<'ctx>, VarType), CompileError> {
        match method {
            "isFinite" | "isInteger" | "isNaN" => {
                let (arg_val, _) = self.compile_expr(&args[0], function)?;
                let func_name = format!("tscc_number_{}", method);
                let func = self.module.get_function(&func_name).unwrap();
                let result = self
                    .builder
                    .build_call(func, &[arg_val.into()], method)
                    .unwrap()
                    .try_as_basic_value()
                    .left()
                    .unwrap();
                // Runtime returns f64 (1.0 or 0.0), convert to boolean
                let bool_val = self
                    .builder
                    .build_float_compare(
                        FloatPredicate::ONE,
                        result.into_float_value(),
                        self.context.f64_type().const_float(0.0),
                        "tobool",
                    )
                    .unwrap();
                Ok((bool_val.into(), VarType::Boolean))
            }
            _ => Err(CompileError::error(
                format!("Unknown Number method '{}'", method),
                span.clone(),
            )),
        }
    }

    fn compile_number_method(
        &mut self,
        obj_val: BasicValueEnum<'ctx>,
        obj_vt: &VarType,
        method: &str,
        args: &[Expr],
        function: FunctionValue<'ctx>,
        span: &Span,
    ) -> Result<(BasicValueEnum<'ctx>, VarType), CompileError> {
        match method {
            "toFixed" => {
                let (digits, _) = self.compile_expr(&args[0], function)?;

                // If obj is integer, convert to f64 first
                let num_val = if matches!(obj_vt, VarType::Integer) {
                    self.builder
                        .build_signed_int_to_float(
                            obj_val.into_int_value(),
                            self.context.f64_type(),
                            "itof",
                        )
                        .unwrap()
                        .into()
                } else {
                    obj_val
                };

                let ptr_type = self.context.ptr_type(AddressSpace::default());
                let i64_type = self.context.i64_type();

                let out_data = self
                    .builder
                    .build_alloca(ptr_type, "toFixed.out_data")
                    .unwrap();
                let out_len = self
                    .builder
                    .build_alloca(i64_type, "toFixed.out_len")
                    .unwrap();

                let func = self.module.get_function("tscc_number_toFixed").unwrap();
                self.builder
                    .build_call(
                        func,
                        &[
                            num_val.into(),
                            digits.into(),
                            out_data.into(),
                            out_len.into(),
                        ],
                        "",
                    )
                    .unwrap();

                let data = self
                    .builder
                    .build_load(ptr_type, out_data, "tf.data")
                    .unwrap();
                let len = self
                    .builder
                    .build_load(i64_type, out_len, "tf.len")
                    .unwrap();

                let str_struct = self.string_type.const_zero();
                let str_struct = self
                    .builder
                    .build_insert_value(str_struct, data, 0, "tf.s0")
                    .unwrap()
                    .into_struct_value();
                let str_struct = self
                    .builder
                    .build_insert_value(str_struct, len, 1, "tf.s1")
                    .unwrap()
                    .into_struct_value();

                Ok((str_struct.into(), VarType::String))
            }
            _ => Err(CompileError::error(
                format!("Unknown number method '{}'", method),
                span.clone(),
            )),
        }
    }

    /// Returns the byte size of an LLVM type as an i64 IntValue (using LLVM's sizeof).
    fn llvm_sizeof(&self, llvm_type: BasicTypeEnum<'ctx>) -> IntValue<'ctx> {
        match llvm_type {
            BasicTypeEnum::IntType(t) => t.size_of(),
            BasicTypeEnum::FloatType(t) => t.size_of(),
            BasicTypeEnum::StructType(t) => t.size_of().expect("struct should be sized"),
            BasicTypeEnum::PointerType(t) => t.size_of(),
            BasicTypeEnum::ArrayType(t) => t.size_of().expect("array should be sized"),
            BasicTypeEnum::VectorType(t) => t.size_of().expect("vector should be sized"),
        }
    }

    /// Compile a method call on a Date instance.
    /// `obj_val` is the Date struct value `{ i64 }`.
    fn compile_date_method(
        &mut self,
        obj_val: BasicValueEnum<'ctx>,
        property: &str,
        args: &[Expr],
        span: &Span,
    ) -> Result<(BasicValueEnum<'ctx>, VarType), CompileError> {
        if !args.is_empty() && property != "setTime" {
            // All getter methods take no arguments
            return Err(CompileError::error(
                format!("Date.{}() takes no arguments", property),
                span.clone(),
            ));
        }

        // Extract ms (field 0) from the struct value
        let ms_val = self
            .builder
            .build_extract_value(obj_val.into_struct_value(), 0, "date_ms")
            .unwrap();

        let fn_name = match property {
            "getTime" => "tscc_date_get_time",
            "getFullYear" => "tscc_date_get_full_year",
            "getMonth" => "tscc_date_get_month",
            "getDate" => "tscc_date_get_date",
            "getHours" => "tscc_date_get_hours",
            "getMinutes" => "tscc_date_get_minutes",
            "getSeconds" => "tscc_date_get_seconds",
            "getMilliseconds" => "tscc_date_get_milliseconds",
            "getUTCFullYear" => "tscc_date_get_utc_full_year",
            "getUTCMonth" => "tscc_date_get_utc_month",
            "getUTCDate" => "tscc_date_get_utc_date",
            "getUTCHours" => "tscc_date_get_utc_hours",
            "getUTCMinutes" => "tscc_date_get_utc_minutes",
            "getUTCSeconds" => "tscc_date_get_utc_seconds",
            "getUTCMilliseconds" => "tscc_date_get_utc_milliseconds",
            "toISOString" => {
                let f = self.module.get_function("tscc_date_to_iso_string").unwrap();
                let result = self
                    .builder
                    .build_call(f, &[ms_val.into()], "date_iso")
                    .unwrap()
                    .try_as_basic_value()
                    .left()
                    .unwrap();
                return Ok((result, VarType::String));
            }
            other => {
                return Err(CompileError::error(
                    format!("Date.{}() is not implemented", other),
                    span.clone(),
                ));
            }
        };

        let f = self.module.get_function(fn_name).unwrap();
        let i64_result = self
            .builder
            .build_call(f, &[ms_val.into()], "date_result")
            .unwrap()
            .try_as_basic_value()
            .left()
            .unwrap()
            .into_int_value();
        // Convert i64 → f64 so Date methods return `number` (matching JavaScript semantics)
        let f64_result = self
            .builder
            .build_signed_int_to_float(i64_result, self.context.f64_type(), "date_num")
            .unwrap();
        Ok((f64_result.into(), VarType::Number))
    }

    fn compile_map_method(
        &mut self,
        _object_expr: &Expr,
        map_val: BasicValueEnum<'ctx>,
        val_vt: &VarType,
        method: &str,
        args: &[Expr],
        function: FunctionValue<'ctx>,
        span: &Span,
    ) -> Result<(BasicValueEnum<'ctx>, VarType), CompileError> {
        let map_ptr = map_val.into_pointer_value();
        let ptr_type = self.context.ptr_type(AddressSpace::default());

        // Extract {data_ptr, len} from a string value (VarType::String)
        macro_rules! str_parts {
            ($sval:expr) => {{
                let sv = $sval.into_struct_value();
                let dp = self
                    .builder
                    .build_extract_value(sv, 0, "kp")
                    .unwrap()
                    .into_pointer_value();
                let dl = self
                    .builder
                    .build_extract_value(sv, 1, "kl")
                    .unwrap()
                    .into_int_value();
                (dp, dl)
            }};
        }

        match method {
            "set" => {
                if args.len() < 2 {
                    return Err(CompileError::error(
                        "map.set() requires 2 arguments",
                        span.clone(),
                    ));
                }
                let (key_val, _) = self.compile_expr(&args[0], function)?;
                let (key_ptr, key_len) = str_parts!(key_val);

                let (val_val, actual_vt) = self.compile_expr(&args[1], function)?;
                let val_llvm = self.var_type_to_llvm(&actual_vt);
                let val_alloca = self.builder.build_alloca(val_llvm, "mset_val").unwrap();
                self.builder.build_store(val_alloca, val_val).unwrap();
                let val_size = self.llvm_sizeof(val_llvm);

                let set_fn = self.module.get_function("tscc_map_set").unwrap();
                self.builder
                    .build_call(
                        set_fn,
                        &[
                            map_ptr.into(),
                            key_ptr.into(),
                            key_len.into(),
                            val_alloca.into(),
                            val_size.into(),
                        ],
                        "",
                    )
                    .unwrap();

                // Return the map itself (enables chaining)
                Ok((
                    map_val,
                    VarType::Map {
                        val_vt: Box::new(actual_vt),
                    },
                ))
            }
            "get" => {
                if args.is_empty() {
                    return Err(CompileError::error(
                        "map.get() requires 1 argument",
                        span.clone(),
                    ));
                }
                let (key_val, _) = self.compile_expr(&args[0], function)?;
                let (key_ptr, key_len) = str_parts!(key_val);

                let get_fn = self.module.get_function("tscc_map_get").unwrap();
                let raw_ptr = self
                    .builder
                    .build_call(
                        get_fn,
                        &[map_ptr.into(), key_ptr.into(), key_len.into()],
                        "mget_ptr",
                    )
                    .unwrap()
                    .try_as_basic_value()
                    .left()
                    .unwrap()
                    .into_pointer_value();

                // If NULL → return default zero value; else load the value
                let val_llvm = self.var_type_to_llvm(val_vt);
                let null_ptr = ptr_type.const_null();
                let is_null = self
                    .builder
                    .build_int_compare(inkwell::IntPredicate::EQ, raw_ptr, null_ptr, "map_is_null")
                    .unwrap();

                let found_bb = self.context.append_basic_block(function, "map_found");
                let miss_bb = self.context.append_basic_block(function, "map_miss");
                let merge_bb = self.context.append_basic_block(function, "map_merge");

                self.builder
                    .build_conditional_branch(is_null, miss_bb, found_bb)
                    .unwrap();

                self.builder.position_at_end(found_bb);
                let loaded = self
                    .builder
                    .build_load(val_llvm, raw_ptr, "mget_val")
                    .unwrap();
                self.builder.build_unconditional_branch(merge_bb).unwrap();
                let found_bb_end = self.builder.get_insert_block().unwrap();

                self.builder.position_at_end(miss_bb);
                let zero_val = self.default_value(val_vt);
                self.builder.build_unconditional_branch(merge_bb).unwrap();
                let miss_bb_end = self.builder.get_insert_block().unwrap();

                self.builder.position_at_end(merge_bb);
                let phi = self.builder.build_phi(val_llvm, "mget_result").unwrap();
                phi.add_incoming(&[(&loaded, found_bb_end), (&zero_val, miss_bb_end)]);

                Ok((phi.as_basic_value(), val_vt.clone()))
            }
            "has" => {
                if args.is_empty() {
                    return Err(CompileError::error(
                        "map.has() requires 1 argument",
                        span.clone(),
                    ));
                }
                let (key_val, _) = self.compile_expr(&args[0], function)?;
                let (key_ptr, key_len) = str_parts!(key_val);

                let has_fn = self.module.get_function("tscc_map_has").unwrap();
                let result = self
                    .builder
                    .build_call(
                        has_fn,
                        &[map_ptr.into(), key_ptr.into(), key_len.into()],
                        "mhas",
                    )
                    .unwrap()
                    .try_as_basic_value()
                    .left()
                    .unwrap();
                Ok((result, VarType::Boolean))
            }
            "delete" => {
                if args.is_empty() {
                    return Err(CompileError::error(
                        "map.delete() requires 1 argument",
                        span.clone(),
                    ));
                }
                let (key_val, _) = self.compile_expr(&args[0], function)?;
                let (key_ptr, key_len) = str_parts!(key_val);

                let del_fn = self.module.get_function("tscc_map_delete").unwrap();
                let result = self
                    .builder
                    .build_call(
                        del_fn,
                        &[map_ptr.into(), key_ptr.into(), key_len.into()],
                        "mdel",
                    )
                    .unwrap()
                    .try_as_basic_value()
                    .left()
                    .unwrap();
                Ok((result, VarType::Boolean))
            }
            "size" => {
                let size_fn = self.module.get_function("tscc_map_size").unwrap();
                let count = self
                    .builder
                    .build_call(size_fn, &[map_ptr.into()], "msize")
                    .unwrap()
                    .try_as_basic_value()
                    .left()
                    .unwrap();
                Ok((count, VarType::Integer))
            }
            "values" => {
                // Returns ObjArray: {void**, count, cap}
                let i64_type = self.context.i64_type();
                let count_alloca = self.builder.build_alloca(i64_type, "mv_count").unwrap();
                let vals_fn = self.module.get_function("tscc_map_values_alloc").unwrap();
                let void_star_star = self
                    .builder
                    .build_call(vals_fn, &[map_ptr.into(), count_alloca.into()], "mv_ptrs")
                    .unwrap()
                    .try_as_basic_value()
                    .left()
                    .unwrap();
                let count = self
                    .builder
                    .build_load(i64_type, count_alloca, "mv_len")
                    .unwrap();

                // Build the ObjArray struct: {ptr, i64, i64}
                let mut arr_struct = self.array_type.const_zero();
                arr_struct = self
                    .builder
                    .build_insert_value(arr_struct, void_star_star, 0, "oa.ptr")
                    .unwrap()
                    .into_struct_value();
                arr_struct = self
                    .builder
                    .build_insert_value(arr_struct, count, 1, "oa.len")
                    .unwrap()
                    .into_struct_value();
                arr_struct = self
                    .builder
                    .build_insert_value(arr_struct, count, 2, "oa.cap")
                    .unwrap()
                    .into_struct_value();

                Ok((
                    arr_struct.into(),
                    VarType::ObjArray {
                        elem_vt: Box::new(val_vt.clone()),
                    },
                ))
            }
            _ => Err(CompileError::error(
                format!("Unknown Map method '{}'", method),
                span.clone(),
            )),
        }
    }

    fn compile_obj_array_method(
        &mut self,
        arr_val: BasicValueEnum<'ctx>,
        elem_vt: &VarType,
        method: &str,
        args: &[Expr],
        function: FunctionValue<'ctx>,
        span: &Span,
    ) -> Result<(BasicValueEnum<'ctx>, VarType), CompileError> {
        let i64_type = self.context.i64_type();
        let f64_type = self.context.f64_type();
        let ptr_type = self.context.ptr_type(AddressSpace::default());

        match method {
            "length" => {
                let len = self
                    .builder
                    .build_extract_value(arr_val.into_struct_value(), 1, "oa_len")
                    .unwrap()
                    .into_int_value();
                let len_f64 = self
                    .builder
                    .build_signed_int_to_float(len, f64_type, "oa_len_f")
                    .unwrap();
                Ok((len_f64.into(), VarType::Number))
            }
            "forEach" => {
                if args.is_empty() {
                    return Err(CompileError::error(
                        "forEach requires a callback",
                        span.clone(),
                    ));
                }
                let (cb_val, cb_vt) = self.compile_expr(&args[0], function)?;
                let (cb_fn_ptr, cb_env_ptr) = self.extract_closure_parts(cb_val)?;

                let data_ptr = self
                    .builder
                    .build_extract_value(arr_val.into_struct_value(), 0, "oa_data")
                    .unwrap()
                    .into_pointer_value();
                let length = self
                    .builder
                    .build_extract_value(arr_val.into_struct_value(), 1, "oa_len")
                    .unwrap()
                    .into_int_value();
                let elem_llvm = self.var_type_to_llvm(elem_vt);

                let pre_bb = self.builder.get_insert_block().unwrap();
                let header_bb = self.context.append_basic_block(function, "oafe.header");
                let body_bb = self.context.append_basic_block(function, "oafe.body");
                let exit_bb = self.context.append_basic_block(function, "oafe.exit");
                self.builder.build_unconditional_branch(header_bb).unwrap();

                self.builder.position_at_end(header_bb);
                let i_phi = self.builder.build_phi(i64_type, "oafe_i").unwrap();
                i_phi.add_incoming(&[(&i64_type.const_int(0, false), pre_bb)]);
                let i = i_phi.as_basic_value().into_int_value();
                let cond = self
                    .builder
                    .build_int_compare(IntPredicate::SLT, i, length, "oafe_cond")
                    .unwrap();
                self.builder
                    .build_conditional_branch(cond, body_bb, exit_bb)
                    .unwrap();

                self.builder.position_at_end(body_bb);
                // Load void* at data[i], then load the element
                let elem_ptr_ptr = unsafe {
                    self.builder
                        .build_gep(ptr_type, data_ptr, &[i], "oafe_pp")
                        .unwrap()
                };
                let elem_ptr = self
                    .builder
                    .build_load(ptr_type, elem_ptr_ptr, "oafe_ep")
                    .unwrap()
                    .into_pointer_value();
                let elem_val = self
                    .builder
                    .build_load(elem_llvm, elem_ptr, "oafe_elem")
                    .unwrap();
                let i_f64 = self
                    .builder
                    .build_signed_int_to_float(i, f64_type, "oafe_if")
                    .unwrap();

                // Determine callback function type from VarType
                let cb_ret_vt = if let VarType::Closure {
                    ref return_type, ..
                } = cb_vt
                {
                    (**return_type).clone()
                } else {
                    VarType::Number
                };
                let elem_llvm_meta: BasicMetadataTypeEnum = elem_llvm.into();
                let cb_fn_type = match cb_ret_vt {
                    VarType::Number => {
                        f64_type.fn_type(&[ptr_type.into(), elem_llvm_meta, f64_type.into()], false)
                    }
                    VarType::Boolean => self
                        .context
                        .bool_type()
                        .fn_type(&[ptr_type.into(), elem_llvm_meta, f64_type.into()], false),
                    _ => self
                        .context
                        .void_type()
                        .fn_type(&[ptr_type.into(), elem_llvm_meta, f64_type.into()], false),
                };
                self.builder
                    .build_indirect_call(
                        cb_fn_type,
                        cb_fn_ptr,
                        &[cb_env_ptr.into(), elem_val.into(), i_f64.into()],
                        "",
                    )
                    .unwrap();

                let i_next = self
                    .builder
                    .build_int_add(i, i64_type.const_int(1, false), "oafe_n")
                    .unwrap();
                i_phi.add_incoming(&[(&i_next, self.builder.get_insert_block().unwrap())]);
                self.builder.build_unconditional_branch(header_bb).unwrap();

                self.builder.position_at_end(exit_bb);
                Ok((f64_type.const_float(0.0).into(), VarType::Number))
            }
            _ => Err(CompileError::error(
                format!("Unknown ObjArray method '{}'", method),
                span.clone(),
            )),
        }
    }

    fn compile_array_method(
        &mut self,
        object_expr: &Expr,
        obj_val: BasicValueEnum<'ctx>,
        method: &str,
        args: &[Expr],
        function: FunctionValue<'ctx>,
        span: &Span,
    ) -> Result<(BasicValueEnum<'ctx>, VarType), CompileError> {
        match method {
            "push" => {
                let (arg_val, arg_vt) = self.compile_expr(&args[0], function)?;
                // Convert to f64 if integer
                let float_val: BasicValueEnum = match arg_vt {
                    VarType::Integer => self
                        .builder
                        .build_signed_int_to_float(
                            arg_val.into_int_value(),
                            self.context.f64_type(),
                            "i2f",
                        )
                        .unwrap()
                        .into(),
                    _ => arg_val,
                };

                // Get the alloca pointer for the array variable
                let arr_ptr = if let ExprKind::Identifier(name) = &object_expr.kind {
                    self.get_variable(name).map(|(ptr, _)| ptr)
                } else {
                    None
                };

                let arr_ptr = arr_ptr.ok_or_else(|| {
                    CompileError::error("push requires a variable target", span.clone())
                })?;

                // Call tscc_array_push(arr_ptr, value) — modifies in place
                let push_fn = self.module.get_function("tscc_array_push").unwrap();
                self.builder
                    .build_call(push_fn, &[arr_ptr.into(), float_val.into()], "")
                    .unwrap();

                // Reload the array to get updated length
                let updated = self
                    .builder
                    .build_load(self.array_type, arr_ptr, "arr_updated")
                    .unwrap();
                let new_len = self
                    .builder
                    .build_extract_value(updated.into_struct_value(), 1, "new_len")
                    .unwrap();
                let len_f64 = self
                    .builder
                    .build_signed_int_to_float(
                        new_len.into_int_value(),
                        self.context.f64_type(),
                        "lenf",
                    )
                    .unwrap();
                Ok((len_f64.into(), VarType::Number))
            }
            "pop" => {
                let data_ptr = self
                    .builder
                    .build_extract_value(obj_val.into_struct_value(), 0, "data")
                    .unwrap()
                    .into_pointer_value();
                let length = self
                    .builder
                    .build_extract_value(obj_val.into_struct_value(), 1, "len")
                    .unwrap()
                    .into_int_value();

                // new_length = length - 1
                let one = self.context.i64_type().const_int(1, false);
                let new_len = self.builder.build_int_sub(length, one, "new_len").unwrap();

                // Load the last element: data[new_length]
                let elem_ptr = unsafe {
                    self.builder
                        .build_gep(self.context.f64_type(), data_ptr, &[new_len], "pop_ptr")
                        .unwrap()
                };
                let popped = self
                    .builder
                    .build_load(self.context.f64_type(), elem_ptr, "popped")
                    .unwrap();

                // Update array struct with new length
                let capacity = self
                    .builder
                    .build_extract_value(obj_val.into_struct_value(), 2, "cap")
                    .unwrap();
                let new_arr = self.array_type.const_zero();
                let new_arr = self
                    .builder
                    .build_insert_value(new_arr, data_ptr, 0, "arr.data")
                    .unwrap();
                let new_arr = self
                    .builder
                    .build_insert_value(new_arr.into_struct_value(), new_len, 1, "arr.len")
                    .unwrap();
                let new_arr = self
                    .builder
                    .build_insert_value(new_arr.into_struct_value(), capacity, 2, "arr.cap")
                    .unwrap();

                // Store updated array back
                if let ExprKind::Identifier(name) = &object_expr.kind {
                    if let Some((ptr, _)) = self.get_variable(name) {
                        self.builder
                            .build_store(ptr, new_arr.into_struct_value())
                            .unwrap();
                    }
                }

                Ok((popped, VarType::Number))
            }
            "forEach" => {
                // forEach(callback): call callback(element) for each element
                let (cb_val, cb_vt) = self.compile_expr(&args[0], function)?;
                let (cb_fn_ptr, cb_env_ptr) = self.extract_closure_parts(cb_val)?;

                // Determine callback return type from its VarType
                let cb_ret_vt = if let VarType::Closure {
                    ref return_type, ..
                } = cb_vt
                {
                    (**return_type).clone()
                } else {
                    VarType::Number
                };

                let data_ptr = self
                    .builder
                    .build_extract_value(obj_val.into_struct_value(), 0, "data")
                    .unwrap()
                    .into_pointer_value();
                let length = self
                    .builder
                    .build_extract_value(obj_val.into_struct_value(), 1, "len")
                    .unwrap()
                    .into_int_value();

                let i64_type = self.context.i64_type();
                let f64_type = self.context.f64_type();
                let ptr_type = self.context.ptr_type(AddressSpace::default());

                let header_bb = self.context.append_basic_block(function, "forEach.header");
                let body_bb = self.context.append_basic_block(function, "forEach.body");
                let exit_bb = self.context.append_basic_block(function, "forEach.exit");

                let pre_bb = self.builder.get_insert_block().unwrap();
                self.builder.build_unconditional_branch(header_bb).unwrap();
                self.builder.position_at_end(header_bb);

                let i_phi = self.builder.build_phi(i64_type, "i").unwrap();
                i_phi.add_incoming(&[(&i64_type.const_int(0, false), pre_bb)]);
                let i = i_phi.as_basic_value().into_int_value();
                let cmp = self
                    .builder
                    .build_int_compare(IntPredicate::SLT, i, length, "cmp")
                    .unwrap();
                self.builder
                    .build_conditional_branch(cmp, body_bb, exit_bb)
                    .unwrap();

                self.builder.position_at_end(body_bb);
                let elem_ptr = unsafe {
                    self.builder
                        .build_gep(f64_type, data_ptr, &[i], "elem_ptr")
                        .unwrap()
                };
                let elem = self.builder.build_load(f64_type, elem_ptr, "elem").unwrap();

                // Call callback(env, elem) — use actual return type to match function signature
                let cb_fn_type = match cb_ret_vt {
                    VarType::Number => f64_type.fn_type(&[ptr_type.into(), f64_type.into()], false),
                    VarType::Boolean => self
                        .context
                        .bool_type()
                        .fn_type(&[ptr_type.into(), f64_type.into()], false),
                    _ => self
                        .context
                        .void_type()
                        .fn_type(&[ptr_type.into(), f64_type.into()], false),
                };
                self.builder
                    .build_indirect_call(
                        cb_fn_type,
                        cb_fn_ptr,
                        &[cb_env_ptr.into(), elem.into()],
                        "",
                    )
                    .unwrap();

                let i_next = self
                    .builder
                    .build_int_add(i, i64_type.const_int(1, false), "i_next")
                    .unwrap();
                i_phi.add_incoming(&[(&i_next, body_bb)]);
                self.builder.build_unconditional_branch(header_bb).unwrap();

                self.builder.position_at_end(exit_bb);
                Ok((f64_type.const_float(0.0).into(), VarType::Number))
            }

            "map" => {
                // map(callback): create new array with callback(element) for each
                let (cb_val, _cb_vt) = self.compile_expr(&args[0], function)?;
                let (cb_fn_ptr, cb_env_ptr) = self.extract_closure_parts(cb_val)?;

                let data_ptr = self
                    .builder
                    .build_extract_value(obj_val.into_struct_value(), 0, "data")
                    .unwrap()
                    .into_pointer_value();
                let length = self
                    .builder
                    .build_extract_value(obj_val.into_struct_value(), 1, "len")
                    .unwrap()
                    .into_int_value();

                let i64_type = self.context.i64_type();
                let f64_type = self.context.f64_type();
                let ptr_type = self.context.ptr_type(AddressSpace::default());

                // Allocate new array: malloc(length * 8)
                let malloc_fn = self.module.get_function("malloc").unwrap_or_else(|| {
                    self.module.add_function(
                        "malloc",
                        ptr_type.fn_type(&[i64_type.into()], false),
                        None,
                    )
                });
                let alloc_size = self
                    .builder
                    .build_int_mul(length, i64_type.const_int(8, false), "alloc_size")
                    .unwrap();
                let new_data = self
                    .builder
                    .build_call(malloc_fn, &[alloc_size.into()], "map_data")
                    .unwrap()
                    .try_as_basic_value()
                    .left()
                    .unwrap()
                    .into_pointer_value();

                let header_bb = self.context.append_basic_block(function, "map.header");
                let body_bb = self.context.append_basic_block(function, "map.body");
                let exit_bb = self.context.append_basic_block(function, "map.exit");

                let pre_bb = self.builder.get_insert_block().unwrap();
                self.builder.build_unconditional_branch(header_bb).unwrap();
                self.builder.position_at_end(header_bb);

                let i_phi = self.builder.build_phi(i64_type, "i").unwrap();
                i_phi.add_incoming(&[(&i64_type.const_int(0, false), pre_bb)]);
                let i = i_phi.as_basic_value().into_int_value();
                let cmp = self
                    .builder
                    .build_int_compare(IntPredicate::SLT, i, length, "cmp")
                    .unwrap();
                self.builder
                    .build_conditional_branch(cmp, body_bb, exit_bb)
                    .unwrap();

                self.builder.position_at_end(body_bb);
                let elem_ptr = unsafe {
                    self.builder
                        .build_gep(f64_type, data_ptr, &[i], "elem_ptr")
                        .unwrap()
                };
                let elem = self.builder.build_load(f64_type, elem_ptr, "elem").unwrap();

                // Call callback(env, elem) -> f64
                let cb_fn_type = f64_type.fn_type(&[ptr_type.into(), f64_type.into()], false);
                let result = self
                    .builder
                    .build_indirect_call(
                        cb_fn_type,
                        cb_fn_ptr,
                        &[cb_env_ptr.into(), elem.into()],
                        "mapped",
                    )
                    .unwrap()
                    .try_as_basic_value()
                    .left()
                    .unwrap();

                // Store result in new array
                let dst_ptr = unsafe {
                    self.builder
                        .build_gep(f64_type, new_data, &[i], "dst_ptr")
                        .unwrap()
                };
                self.builder.build_store(dst_ptr, result).unwrap();

                let i_next = self
                    .builder
                    .build_int_add(i, i64_type.const_int(1, false), "i_next")
                    .unwrap();
                i_phi.add_incoming(&[(&i_next, body_bb)]);
                self.builder.build_unconditional_branch(header_bb).unwrap();

                self.builder.position_at_end(exit_bb);

                // Build result array struct
                let arr = self.array_type.const_zero();
                let arr = self
                    .builder
                    .build_insert_value(arr, new_data, 0, "arr.data")
                    .unwrap();
                let arr = self
                    .builder
                    .build_insert_value(arr.into_struct_value(), length, 1, "arr.len")
                    .unwrap();
                let arr = self
                    .builder
                    .build_insert_value(arr.into_struct_value(), length, 2, "arr.cap")
                    .unwrap();
                Ok((arr.into_struct_value().into(), VarType::Array))
            }

            "filter" => {
                // filter(callback): create new array with elements where callback returns true
                let (cb_val, _cb_vt) = self.compile_expr(&args[0], function)?;
                let (cb_fn_ptr, cb_env_ptr) = self.extract_closure_parts(cb_val)?;

                let data_ptr = self
                    .builder
                    .build_extract_value(obj_val.into_struct_value(), 0, "data")
                    .unwrap()
                    .into_pointer_value();
                let length = self
                    .builder
                    .build_extract_value(obj_val.into_struct_value(), 1, "len")
                    .unwrap()
                    .into_int_value();

                let i64_type = self.context.i64_type();
                let f64_type = self.context.f64_type();
                let ptr_type = self.context.ptr_type(AddressSpace::default());

                // Allocate new array with same capacity as original
                let malloc_fn = self.module.get_function("malloc").unwrap_or_else(|| {
                    self.module.add_function(
                        "malloc",
                        ptr_type.fn_type(&[i64_type.into()], false),
                        None,
                    )
                });
                let alloc_size = self
                    .builder
                    .build_int_mul(length, i64_type.const_int(8, false), "alloc_size")
                    .unwrap();
                let new_data = self
                    .builder
                    .build_call(malloc_fn, &[alloc_size.into()], "filter_data")
                    .unwrap()
                    .try_as_basic_value()
                    .left()
                    .unwrap()
                    .into_pointer_value();

                let header_bb = self.context.append_basic_block(function, "filter.header");
                let body_bb = self.context.append_basic_block(function, "filter.body");
                let store_bb = self.context.append_basic_block(function, "filter.store");
                let cont_bb = self.context.append_basic_block(function, "filter.cont");
                let exit_bb = self.context.append_basic_block(function, "filter.exit");

                let pre_bb = self.builder.get_insert_block().unwrap();
                self.builder.build_unconditional_branch(header_bb).unwrap();
                self.builder.position_at_end(header_bb);

                let i_phi = self.builder.build_phi(i64_type, "i").unwrap();
                i_phi.add_incoming(&[(&i64_type.const_int(0, false), pre_bb)]);
                let out_idx_phi = self.builder.build_phi(i64_type, "out_idx").unwrap();
                out_idx_phi.add_incoming(&[(&i64_type.const_int(0, false), pre_bb)]);

                let i = i_phi.as_basic_value().into_int_value();
                let out_idx = out_idx_phi.as_basic_value().into_int_value();
                let cmp = self
                    .builder
                    .build_int_compare(IntPredicate::SLT, i, length, "cmp")
                    .unwrap();
                self.builder
                    .build_conditional_branch(cmp, body_bb, exit_bb)
                    .unwrap();

                self.builder.position_at_end(body_bb);
                let elem_ptr = unsafe {
                    self.builder
                        .build_gep(f64_type, data_ptr, &[i], "elem_ptr")
                        .unwrap()
                };
                let elem = self.builder.build_load(f64_type, elem_ptr, "elem").unwrap();

                // Call callback(env, elem) -> i1 (boolean)
                let cb_fn_type = self
                    .context
                    .bool_type()
                    .fn_type(&[ptr_type.into(), f64_type.into()], false);
                let result = self
                    .builder
                    .build_indirect_call(
                        cb_fn_type,
                        cb_fn_ptr,
                        &[cb_env_ptr.into(), elem.into()],
                        "pred",
                    )
                    .unwrap()
                    .try_as_basic_value()
                    .left()
                    .unwrap();

                // The result is i1 (boolean) — branch directly
                let is_true = result.into_int_value();
                self.builder
                    .build_conditional_branch(is_true, store_bb, cont_bb)
                    .unwrap();

                // Store element in output array
                self.builder.position_at_end(store_bb);
                let dst_ptr = unsafe {
                    self.builder
                        .build_gep(f64_type, new_data, &[out_idx], "dst_ptr")
                        .unwrap()
                };
                self.builder.build_store(dst_ptr, elem).unwrap();
                let out_next = self
                    .builder
                    .build_int_add(out_idx, i64_type.const_int(1, false), "out_next")
                    .unwrap();
                self.builder.build_unconditional_branch(cont_bb).unwrap();

                // Continue to next iteration
                self.builder.position_at_end(cont_bb);
                let out_phi = self.builder.build_phi(i64_type, "out_merged").unwrap();
                out_phi.add_incoming(&[(&out_next, store_bb), (&out_idx, body_bb)]);

                let i_next = self
                    .builder
                    .build_int_add(i, i64_type.const_int(1, false), "i_next")
                    .unwrap();
                i_phi.add_incoming(&[(&i_next, cont_bb)]);
                out_idx_phi.add_incoming(&[(&out_phi.as_basic_value(), cont_bb)]);
                self.builder.build_unconditional_branch(header_bb).unwrap();

                self.builder.position_at_end(exit_bb);

                // Build result array struct with actual count
                let final_len = out_idx; // phi value at exit
                let arr = self.array_type.const_zero();
                let arr = self
                    .builder
                    .build_insert_value(arr, new_data, 0, "arr.data")
                    .unwrap();
                let arr = self
                    .builder
                    .build_insert_value(arr.into_struct_value(), final_len, 1, "arr.len")
                    .unwrap();
                let arr = self
                    .builder
                    .build_insert_value(arr.into_struct_value(), length, 2, "arr.cap")
                    .unwrap();
                Ok((arr.into_struct_value().into(), VarType::Array))
            }

            "reduce" => {
                // reduce(callback, initial): fold array with callback(acc, elem)
                if args.len() != 2 {
                    return Err(CompileError::error(
                        format!("reduce expects 2 arguments, got {}", args.len()),
                        span.clone(),
                    ));
                }
                let (cb_val, _cb_vt) = self.compile_expr(&args[0], function)?;
                let (cb_fn_ptr, cb_env_ptr) = self.extract_closure_parts(cb_val)?;

                let (init_val, init_vt) = self.compile_expr(&args[1], function)?;
                let init_f64 = match init_vt {
                    VarType::Integer => self
                        .builder
                        .build_signed_int_to_float(
                            init_val.into_int_value(),
                            self.context.f64_type(),
                            "i2f",
                        )
                        .unwrap()
                        .into(),
                    _ => init_val,
                };

                let data_ptr = self
                    .builder
                    .build_extract_value(obj_val.into_struct_value(), 0, "data")
                    .unwrap()
                    .into_pointer_value();
                let length = self
                    .builder
                    .build_extract_value(obj_val.into_struct_value(), 1, "len")
                    .unwrap()
                    .into_int_value();

                let i64_type = self.context.i64_type();
                let f64_type = self.context.f64_type();
                let ptr_type = self.context.ptr_type(AddressSpace::default());

                let header_bb = self.context.append_basic_block(function, "reduce.header");
                let body_bb = self.context.append_basic_block(function, "reduce.body");
                let exit_bb = self.context.append_basic_block(function, "reduce.exit");

                let pre_bb = self.builder.get_insert_block().unwrap();
                self.builder.build_unconditional_branch(header_bb).unwrap();
                self.builder.position_at_end(header_bb);

                let i_phi = self.builder.build_phi(i64_type, "i").unwrap();
                i_phi.add_incoming(&[(&i64_type.const_int(0, false), pre_bb)]);
                let acc_phi = self.builder.build_phi(f64_type, "acc").unwrap();
                acc_phi.add_incoming(&[(&init_f64, pre_bb)]);

                let i = i_phi.as_basic_value().into_int_value();
                let acc = acc_phi.as_basic_value().into_float_value();
                let cmp = self
                    .builder
                    .build_int_compare(IntPredicate::SLT, i, length, "cmp")
                    .unwrap();
                self.builder
                    .build_conditional_branch(cmp, body_bb, exit_bb)
                    .unwrap();

                self.builder.position_at_end(body_bb);
                let elem_ptr = unsafe {
                    self.builder
                        .build_gep(f64_type, data_ptr, &[i], "elem_ptr")
                        .unwrap()
                };
                let elem = self.builder.build_load(f64_type, elem_ptr, "elem").unwrap();

                // Call callback(env, acc, elem) -> f64
                let cb_fn_type =
                    f64_type.fn_type(&[ptr_type.into(), f64_type.into(), f64_type.into()], false);
                let new_acc = self
                    .builder
                    .build_indirect_call(
                        cb_fn_type,
                        cb_fn_ptr,
                        &[cb_env_ptr.into(), acc.into(), elem.into()],
                        "new_acc",
                    )
                    .unwrap()
                    .try_as_basic_value()
                    .left()
                    .unwrap();

                let i_next = self
                    .builder
                    .build_int_add(i, i64_type.const_int(1, false), "i_next")
                    .unwrap();
                i_phi.add_incoming(&[(&i_next, body_bb)]);
                acc_phi.add_incoming(&[(&new_acc, body_bb)]);
                self.builder.build_unconditional_branch(header_bb).unwrap();

                self.builder.position_at_end(exit_bb);
                Ok((acc.into(), VarType::Number))
            }

            _ => Err(CompileError::error(
                format!("Unknown array method '{}'", method),
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

        let func_name = format!("tscc_{}", name);
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

    /// Helper: emit calls to print a string constant via the given print function name.
    fn print_string_literal(&self, s: &str, print_fn_name: &str) {
        let str_val = self.create_string_literal(s);
        let ptr = self
            .builder
            .build_extract_value(str_val.into_struct_value(), 0, "p")
            .unwrap();
        let len = self
            .builder
            .build_extract_value(str_val.into_struct_value(), 1, "l")
            .unwrap();
        let f = self.module.get_function(print_fn_name).unwrap();
        self.builder
            .build_call(f, &[ptr.into(), len.into()], "")
            .unwrap();
    }

    fn to_string(
        &self,
        val: BasicValueEnum<'ctx>,
        vt: &VarType,
    ) -> Result<BasicValueEnum<'ctx>, CompileError> {
        match vt {
            VarType::String => Ok(val),
            VarType::Number => {
                let f = self.module.get_function("tscc_number_to_string").unwrap();
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
                let f = self.module.get_function("tscc_number_to_string").unwrap();
                Ok(self
                    .builder
                    .build_call(f, &[float_val.into()], "numstr")
                    .unwrap()
                    .try_as_basic_value()
                    .left()
                    .unwrap())
            }
            VarType::Boolean => {
                let f = self.module.get_function("tscc_boolean_to_string").unwrap();
                Ok(self
                    .builder
                    .build_call(f, &[val.into()], "boolstr")
                    .unwrap()
                    .try_as_basic_value()
                    .left()
                    .unwrap())
            }
            VarType::Array | VarType::StringArray => {
                // Arrays don't have a to_string yet; return "[object Array]"
                Ok(self.create_string_literal("[object Array]"))
            }
            VarType::FunctionPtr { .. } | VarType::Closure { .. } => {
                Ok(self.create_string_literal("[Function]"))
            }
            VarType::Object { .. } => Ok(self.create_string_literal("[object Object]")),
            VarType::Union(_) => {
                // Unions should be narrowed before to_string is called; fallback
                Ok(self.create_string_literal("[union]"))
            }
            VarType::Tuple(_) => Ok(self.create_string_literal("[tuple]")),
            VarType::Map { .. } => Ok(self.create_string_literal("[Map]")),
            VarType::ObjArray { .. } => Ok(self.create_string_literal("[Array]")),
            VarType::Promise { .. } => Ok(self.create_string_literal("[Promise]")),
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
            VarType::Array | VarType::StringArray | VarType::ObjArray { .. } => {
                self.array_type.into()
            }
            VarType::Map { .. } => self.context.ptr_type(AddressSpace::default()).into(),
            VarType::FunctionPtr { .. } => self.context.ptr_type(AddressSpace::default()).into(),
            VarType::Closure { .. } => self.closure_type.into(),
            VarType::Object {
                struct_type_name,
                fields,
            } => {
                // Look up or create a named struct type for this object shape
                if let Some((st, _, _)) = self.class_struct_types.get(struct_type_name) {
                    (*st).into()
                } else {
                    // Create an anonymous struct type from field types
                    let field_types: Vec<BasicTypeEnum> = fields
                        .iter()
                        .map(|(_, fvt)| self.var_type_to_llvm(fvt))
                        .collect();
                    self.context.struct_type(&field_types, false).into()
                }
            }
            VarType::Union(_) => self.get_union_llvm_type().into(),
            VarType::Tuple(ref elements) => {
                let field_types: Vec<BasicTypeEnum> = elements
                    .iter()
                    .map(|vt| self.var_type_to_llvm(vt))
                    .collect();
                self.context.struct_type(&field_types, false).into()
            }
            // Promise is a pointer to MgPromise
            VarType::Promise { .. } => self.context.ptr_type(AddressSpace::default()).into(),
        }
    }

    fn type_ann_to_var_type(&mut self, ann: &TypeAnnotation) -> VarType {
        match &ann.kind {
            TypeAnnKind::Number => self.number_mode.clone(),
            TypeAnnKind::String => VarType::String,
            TypeAnnKind::Boolean => VarType::Boolean,
            TypeAnnKind::Void | TypeAnnKind::Null | TypeAnnKind::Undefined => {
                self.number_mode.clone()
            }
            TypeAnnKind::Array(_) => VarType::Array,
            TypeAnnKind::Object { fields } => {
                let field_vts: Vec<(String, VarType)> = fields
                    .iter()
                    .map(|(name, ann)| (name.clone(), self.type_ann_to_var_type(ann)))
                    .collect();
                VarType::Object {
                    struct_type_name: format!("__anon_obj_{}", field_vts.len()),
                    fields: field_vts,
                }
            }
            TypeAnnKind::Named(name) => {
                // Check type parameter substitutions first (generics)
                if let Some(vt) = self.type_substitutions.get(name) {
                    return vt.clone();
                }
                // Look up class struct types (includes pre-registered built-ins like Date)
                if let Some((_, field_info, _)) = self.class_struct_types.get(name) {
                    return VarType::Object {
                        struct_type_name: name.clone(),
                        fields: field_info.clone(),
                    };
                }
                // Resolve non-generic type aliases (e.g. `type Result = A | B`)
                if let Some(alias_ann) = self.type_aliases_for_codegen.get(name).cloned() {
                    return self.type_ann_to_var_type(&alias_ann);
                }
                // Unknown named type — treat as number for now
                self.number_mode.clone()
            }
            TypeAnnKind::Typeof(_) => {
                // typeof is resolved by the type checker; at codegen the variable's
                // actual type is used from its initializer, so this is only hit
                // for uninitialized variables — fall back to number.
                self.number_mode.clone()
            }
            TypeAnnKind::StringLiteral(_) => VarType::String,
            TypeAnnKind::NumberLiteral(_) => self.number_mode.clone(),
            TypeAnnKind::BooleanLiteral(_) => VarType::Boolean,
            TypeAnnKind::Union(variants) => {
                let var_types: Vec<VarType> = variants
                    .iter()
                    .map(|v| self.type_ann_to_var_type(v))
                    .collect();
                if var_types.is_empty() {
                    return self.number_mode.clone();
                }
                // If ALL variants are Object types, merge them into a single
                // struct with the union of all fields (discriminated union pattern).
                // This matches JavaScript's runtime behaviour: no tag needed,
                // just a plain object with all possible fields.
                let all_objects = var_types
                    .iter()
                    .all(|v| matches!(v, VarType::Object { .. }));
                if all_objects {
                    let mut merged_fields: Vec<(String, VarType)> = Vec::new();
                    for vt in &var_types {
                        if let VarType::Object { fields, .. } = vt {
                            for (fname, fvt) in fields {
                                if !merged_fields.iter().any(|(n, _)| n == fname) {
                                    merged_fields.push((fname.clone(), fvt.clone()));
                                }
                            }
                        }
                    }
                    // Build a deterministic struct name from the field list
                    let struct_name = format!(
                        "__union_obj_{}",
                        merged_fields
                            .iter()
                            .map(|(n, _)| n.as_str())
                            .collect::<Vec<_>>()
                            .join("_")
                    );
                    // Register in class_struct_types if not already present
                    if !self.class_struct_types.contains_key(&struct_name) {
                        let field_llvm_types: Vec<BasicTypeEnum> = merged_fields
                            .iter()
                            .map(|(_, vt)| self.var_type_to_llvm(vt))
                            .collect();
                        let struct_type = self.context.struct_type(&field_llvm_types, false);
                        self.class_struct_types.insert(
                            struct_name.clone(),
                            (struct_type, merged_fields.clone(), None),
                        );
                    }
                    return VarType::Object {
                        struct_type_name: struct_name,
                        fields: merged_fields,
                    };
                }
                VarType::Union(var_types)
            }
            TypeAnnKind::Intersection(variants) => {
                // Intersection merges object fields — build combined object type
                let mut all_fields: Vec<(String, VarType)> = Vec::new();
                for v in variants {
                    if let VarType::Object { fields, .. } = self.type_ann_to_var_type(v) {
                        for (name, vt) in fields {
                            if !all_fields.iter().any(|(n, _)| n == &name) {
                                all_fields.push((name, vt));
                            }
                        }
                    }
                }
                if all_fields.is_empty() {
                    self.number_mode.clone()
                } else {
                    VarType::Object {
                        struct_type_name: format!("__intersection_{}", all_fields.len()),
                        fields: all_fields,
                    }
                }
            }
            TypeAnnKind::Keyof(_) => {
                // keyof resolves to string literal union — at codegen it's just a string
                VarType::String
            }
            TypeAnnKind::FunctionType {
                params,
                return_type,
            } => {
                let param_types: Vec<VarType> = params
                    .iter()
                    .map(|p| self.type_ann_to_var_type(p))
                    .collect();
                let ret_vt = self.type_ann_to_var_type(return_type);
                VarType::Closure {
                    fn_name: String::new(),
                    param_types,
                    return_type: Box::new(ret_vt),
                }
            }
            TypeAnnKind::Tuple(elements) => {
                let element_types: Vec<VarType> = elements
                    .iter()
                    .map(|e| self.type_ann_to_var_type(e))
                    .collect();
                VarType::Tuple(element_types)
            }
            TypeAnnKind::Generic { name, type_args } => {
                // Promise<T> → VarType::Promise { inner_vt: T }
                if name == "Promise" {
                    let inner_vt = type_args
                        .first()
                        .map(|a| self.type_ann_to_var_type(a))
                        .unwrap_or(VarType::Number);
                    return VarType::Promise {
                        inner_vt: Box::new(inner_vt),
                    };
                }
                // Map<K, V> → VarType::Map { val_vt }
                if name == "Map" && type_args.len() >= 2 {
                    let val_vt = self.type_ann_to_var_type(&type_args[1]);
                    return VarType::Map {
                        val_vt: Box::new(val_vt),
                    };
                }
                // Resolve generic type alias by substituting type args
                // For codegen, look up the alias body and substitute
                if let Some(alias_ann) = self.type_aliases_for_codegen.get(name).cloned() {
                    // Build substitution map from type param names
                    if let Some((tp_names, _)) = self.generic_alias_params.get(name).cloned() {
                        // Resolve type args first to avoid borrow conflict
                        let resolved_args: Vec<VarType> = type_args
                            .iter()
                            .map(|arg| self.type_ann_to_var_type(arg))
                            .collect();
                        let prev_subs = self.type_substitutions.clone();
                        for (tp_name, vt) in tp_names.iter().zip(resolved_args.into_iter()) {
                            self.type_substitutions.insert(tp_name.clone(), vt);
                        }
                        let result = self.type_ann_to_var_type(&alias_ann);
                        self.type_substitutions = prev_subs;
                        return result;
                    }
                }
                self.number_mode.clone()
            }
            TypeAnnKind::Conditional {
                check_type,
                extends_type,
                true_type,
                false_type,
            } => {
                // Evaluate conditional type at codegen time
                let check = self.type_ann_to_var_type(check_type);
                let extends = self.type_ann_to_var_type(extends_type);
                // Simple check: if the types match categories
                if Self::var_types_compatible(&check, &extends) {
                    self.type_ann_to_var_type(true_type)
                } else {
                    self.type_ann_to_var_type(false_type)
                }
            }
            TypeAnnKind::Mapped { .. } => {
                // Mapped types are type-only — no runtime representation
                self.number_mode.clone()
            }
            TypeAnnKind::IndexedAccess { .. } => {
                // Indexed access types are type-only — fallback
                self.number_mode.clone()
            }
        }
    }

    fn default_value(&self, vt: &VarType) -> BasicValueEnum<'ctx> {
        match vt {
            VarType::Number => self.context.f64_type().const_float(0.0).into(),
            VarType::Integer => self.context.i64_type().const_int(0, false).into(),
            VarType::String => self.create_string_literal(""),
            VarType::Boolean => self.context.bool_type().const_int(0, false).into(),
            VarType::Array | VarType::StringArray => self.array_type.const_zero().into(),
            VarType::FunctionPtr { .. } => self
                .context
                .ptr_type(AddressSpace::default())
                .const_null()
                .into(),
            VarType::Closure { .. } => self.closure_type.const_zero().into(),
            VarType::Object { fields, .. } => {
                let llvm_type = self.var_type_to_llvm(vt);
                let st = llvm_type.into_struct_type();
                let mut val = st.const_zero();
                for (i, (_, field_vt)) in fields.iter().enumerate() {
                    let default = self.default_value(field_vt);
                    val = self
                        .builder
                        .build_insert_value(val, default, i as u32, "obj.init")
                        .unwrap()
                        .into_struct_value();
                }
                val.into()
            }
            VarType::Union(_) => self.get_union_llvm_type().const_zero().into(),
            VarType::Tuple(ref elements) => {
                let llvm_type = self.var_type_to_llvm(vt);
                let st = llvm_type.into_struct_type();
                let mut val = st.const_zero();
                for (i, elem_vt) in elements.iter().enumerate() {
                    let default = self.default_value(elem_vt);
                    val = self
                        .builder
                        .build_insert_value(val, default, i as u32, "tup.init")
                        .unwrap()
                        .into_struct_value();
                }
                val.into()
            }
            VarType::Map { .. } => self
                .context
                .ptr_type(AddressSpace::default())
                .const_null()
                .into(),
            VarType::ObjArray { .. } => self.array_type.const_zero().into(),
            VarType::Promise { .. } => self
                .context
                .ptr_type(AddressSpace::default())
                .const_null()
                .into(),
        }
    }

    // --- Tagged union support ---

    /// LLVM struct type for tagged unions: { i8 tag, double num_slot, ptr str_ptr, i64 aux }
    fn get_union_llvm_type(&self) -> StructType<'ctx> {
        self.context.struct_type(
            &[
                self.context.i8_type().into(),                         // tag
                self.context.f64_type().into(),                        // number slot
                self.context.ptr_type(AddressSpace::default()).into(), // string data ptr
                self.context.i64_type().into(),                        // string len / bool
            ],
            false,
        )
    }

    /// Map a concrete VarType to a union tag constant.
    fn union_tag_for_var_type(vt: &VarType) -> u8 {
        match vt {
            VarType::Number | VarType::Integer => 0,
            VarType::String => 1,
            VarType::Boolean => 2,
            _ => 3,
        }
    }

    /// Map a typeof string like "number" to a VarType.
    fn type_string_to_var_type(&self, s: &str) -> VarType {
        match s {
            "number" => VarType::Number,
            "string" => VarType::String,
            "boolean" => VarType::Boolean,
            _ => VarType::Number,
        }
    }

    /// Wrap a concrete value into a tagged union struct, stored at an alloca.
    /// Returns the alloca pointer (NOT the loaded struct value).
    fn wrap_in_union(
        &self,
        value: BasicValueEnum<'ctx>,
        value_vt: &VarType,
        function: FunctionValue<'ctx>,
    ) -> PointerValue<'ctx> {
        let union_type = self.get_union_llvm_type();
        let alloca = self.create_alloca(function, &VarType::Union(vec![]), "union_wrap");

        // Store tag
        let tag_ptr = self
            .builder
            .build_struct_gep(union_type, alloca, 0, "tag_ptr")
            .unwrap();
        let tag = Self::union_tag_for_var_type(value_vt);
        self.builder
            .build_store(tag_ptr, self.context.i8_type().const_int(tag as u64, false))
            .unwrap();

        // Store value into appropriate slot
        match value_vt {
            VarType::Number => {
                let num_ptr = self
                    .builder
                    .build_struct_gep(union_type, alloca, 1, "num_ptr")
                    .unwrap();
                self.builder.build_store(num_ptr, value).unwrap();
            }
            VarType::Integer => {
                // Convert i64 → f64 before storing in number slot
                let float_val = self
                    .builder
                    .build_signed_int_to_float(
                        value.into_int_value(),
                        self.context.f64_type(),
                        "i2f",
                    )
                    .unwrap();
                let num_ptr = self
                    .builder
                    .build_struct_gep(union_type, alloca, 1, "num_ptr")
                    .unwrap();
                self.builder.build_store(num_ptr, float_val).unwrap();
            }
            VarType::String => {
                let str_val = value.into_struct_value();
                let data = self
                    .builder
                    .build_extract_value(str_val, 0, "str_data")
                    .unwrap();
                let len = self
                    .builder
                    .build_extract_value(str_val, 1, "str_len")
                    .unwrap();
                let str_ptr_slot = self
                    .builder
                    .build_struct_gep(union_type, alloca, 2, "str_ptr_slot")
                    .unwrap();
                self.builder.build_store(str_ptr_slot, data).unwrap();
                let str_len_slot = self
                    .builder
                    .build_struct_gep(union_type, alloca, 3, "str_len_slot")
                    .unwrap();
                self.builder.build_store(str_len_slot, len).unwrap();
            }
            VarType::Boolean => {
                let bool_i64 = self
                    .builder
                    .build_int_z_extend(value.into_int_value(), self.context.i64_type(), "b2i")
                    .unwrap();
                let aux_ptr = self
                    .builder
                    .build_struct_gep(union_type, alloca, 3, "aux_ptr")
                    .unwrap();
                self.builder.build_store(aux_ptr, bool_i64).unwrap();
            }
            _ => {} // other types not supported in unions yet
        }

        alloca
    }

    /// Extract a concrete value from a tagged union alloca, assuming the tag matches `target_vt`.
    fn extract_from_union(
        &self,
        union_ptr: PointerValue<'ctx>,
        target_vt: &VarType,
    ) -> BasicValueEnum<'ctx> {
        let union_type = self.get_union_llvm_type();
        match target_vt {
            VarType::Number => {
                let num_ptr = self
                    .builder
                    .build_struct_gep(union_type, union_ptr, 1, "num_ptr")
                    .unwrap();
                self.builder
                    .build_load(self.context.f64_type(), num_ptr, "num_val")
                    .unwrap()
            }
            VarType::String => {
                let str_ptr_slot = self
                    .builder
                    .build_struct_gep(union_type, union_ptr, 2, "str_ptr_slot")
                    .unwrap();
                let data = self
                    .builder
                    .build_load(
                        self.context.ptr_type(AddressSpace::default()),
                        str_ptr_slot,
                        "str_data",
                    )
                    .unwrap();
                let str_len_slot = self
                    .builder
                    .build_struct_gep(union_type, union_ptr, 3, "str_len_slot")
                    .unwrap();
                let len = self
                    .builder
                    .build_load(self.context.i64_type(), str_len_slot, "str_len")
                    .unwrap();
                // Build the string struct { ptr, i64 }
                let mut struct_val = self.string_type.const_zero();
                struct_val = self
                    .builder
                    .build_insert_value(struct_val, data, 0, "str.ptr")
                    .unwrap()
                    .into_struct_value();
                struct_val = self
                    .builder
                    .build_insert_value(struct_val, len, 1, "str.len")
                    .unwrap()
                    .into_struct_value();
                struct_val.into()
            }
            VarType::Boolean => {
                let aux_ptr = self
                    .builder
                    .build_struct_gep(union_type, union_ptr, 3, "aux_ptr")
                    .unwrap();
                let i64_val = self
                    .builder
                    .build_load(self.context.i64_type(), aux_ptr, "bool_i64")
                    .unwrap();
                self.builder
                    .build_int_truncate(
                        i64_val.into_int_value(),
                        self.context.bool_type(),
                        "bool_val",
                    )
                    .unwrap()
                    .into()
            }
            _ => self.context.f64_type().const_float(0.0).into(),
        }
    }

    /// Detect `typeof x === "type"` or `typeof x !== "type"` pattern in a condition.
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

    // --- Closure support ---

    /// Infer the VarType of an expression from its AST structure (without compiling).
    /// Used to determine return types of unannotated arrow functions.
    fn infer_expr_var_type(expr: &Expr) -> VarType {
        match &expr.kind {
            ExprKind::NumberLiteral(_) => VarType::Number,
            ExprKind::StringLiteral(_) => VarType::String,
            ExprKind::BooleanLiteral(_) => VarType::Boolean,
            ExprKind::Binary {
                op, left, right, ..
            } => match op {
                BinOp::Less
                | BinOp::Greater
                | BinOp::LessEqual
                | BinOp::GreaterEqual
                | BinOp::Equal
                | BinOp::StrictEqual
                | BinOp::NotEqual
                | BinOp::StrictNotEqual
                | BinOp::And
                | BinOp::Or => VarType::Boolean,
                BinOp::Add => {
                    // If either side is a string, result is string
                    let l = Self::infer_expr_var_type(left);
                    let r = Self::infer_expr_var_type(right);
                    if matches!(l, VarType::String) || matches!(r, VarType::String) {
                        VarType::String
                    } else {
                        VarType::Number
                    }
                }
                BinOp::NullishCoalescing => Self::infer_expr_var_type(right),
                _ => VarType::Number,
            },
            ExprKind::Unary { op, .. } => match op {
                UnaryOp::Not => VarType::Boolean,
                UnaryOp::Negate => VarType::Number,
            },
            ExprKind::Conditional { consequent, .. } => Self::infer_expr_var_type(consequent),
            ExprKind::Grouping { expr } => Self::infer_expr_var_type(expr),
            // Default to number for identifiers, calls, etc.
            _ => VarType::Number,
        }
    }

    /// Extract fn_ptr and env_ptr from a closure value ({ ptr, ptr } struct).
    fn extract_closure_parts(
        &self,
        closure_val: BasicValueEnum<'ctx>,
    ) -> Result<(PointerValue<'ctx>, BasicValueEnum<'ctx>), CompileError> {
        let sv = closure_val.into_struct_value();
        let fn_ptr = self
            .builder
            .build_extract_value(sv, 0, "fn_ptr")
            .unwrap()
            .into_pointer_value();
        let env_ptr = self.builder.build_extract_value(sv, 1, "env_ptr").unwrap();
        Ok((fn_ptr, env_ptr))
    }

    /// Collect all identifier names referenced in a list of statements.
    fn collect_idents_in_stmts(stmts: &[Statement], out: &mut HashSet<String>) {
        for s in stmts {
            Self::collect_idents_in_stmt(s, out);
        }
    }

    fn collect_idents_in_stmt(stmt: &Statement, out: &mut HashSet<String>) {
        match &stmt.kind {
            StmtKind::VariableDecl { initializer, .. } => {
                if let Some(e) = initializer {
                    Self::collect_idents_in_expr(e, out);
                }
            }
            StmtKind::Expression { expr } => Self::collect_idents_in_expr(expr, out),
            StmtKind::Return { value } => {
                if let Some(e) = value {
                    Self::collect_idents_in_expr(e, out);
                }
            }
            StmtKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                Self::collect_idents_in_expr(condition, out);
                Self::collect_idents_in_stmts(then_branch, out);
                if let Some(eb) = else_branch {
                    Self::collect_idents_in_stmts(eb, out);
                }
            }
            StmtKind::While { condition, body } => {
                Self::collect_idents_in_expr(condition, out);
                Self::collect_idents_in_stmts(body, out);
            }
            StmtKind::DoWhile { body, condition } => {
                Self::collect_idents_in_stmts(body, out);
                Self::collect_idents_in_expr(condition, out);
            }
            StmtKind::Switch {
                discriminant,
                cases,
            } => {
                Self::collect_idents_in_expr(discriminant, out);
                for case in cases {
                    if let Some(test) = &case.test {
                        Self::collect_idents_in_expr(test, out);
                    }
                    Self::collect_idents_in_stmts(&case.body, out);
                }
            }
            StmtKind::For {
                init,
                condition,
                update,
                body,
            } => {
                if let Some(i) = init {
                    Self::collect_idents_in_stmt(i, out);
                }
                if let Some(c) = condition {
                    Self::collect_idents_in_expr(c, out);
                }
                if let Some(u) = update {
                    Self::collect_idents_in_expr(u, out);
                }
                Self::collect_idents_in_stmts(body, out);
            }
            StmtKind::ForOf { iterable, body, .. } => {
                Self::collect_idents_in_expr(iterable, out);
                Self::collect_idents_in_stmts(body, out);
            }
            StmtKind::ArrayDestructure { initializer, .. } => {
                Self::collect_idents_in_expr(initializer, out);
            }
            StmtKind::ObjectDestructure { initializer, .. } => {
                Self::collect_idents_in_expr(initializer, out);
            }
            StmtKind::Block { statements } => Self::collect_idents_in_stmts(statements, out),
            _ => {}
        }
    }

    fn collect_idents_in_expr(expr: &Expr, out: &mut HashSet<String>) {
        match &expr.kind {
            ExprKind::Identifier(name) => {
                out.insert(name.clone());
            }
            ExprKind::Binary { left, right, .. } => {
                Self::collect_idents_in_expr(left, out);
                Self::collect_idents_in_expr(right, out);
            }
            ExprKind::Unary { operand, .. } | ExprKind::Typeof { operand } => {
                Self::collect_idents_in_expr(operand, out);
            }
            ExprKind::Call { callee, args } => {
                Self::collect_idents_in_expr(callee, out);
                for a in args {
                    Self::collect_idents_in_expr(a, out);
                }
            }
            ExprKind::Member { object, .. } | ExprKind::OptionalMember { object, .. } => {
                Self::collect_idents_in_expr(object, out);
            }
            ExprKind::Spread { expr: inner } => {
                Self::collect_idents_in_expr(inner, out);
            }
            ExprKind::Assignment { name, value } => {
                out.insert(name.clone());
                Self::collect_idents_in_expr(value, out);
            }
            ExprKind::MemberAssignment { object, value, .. } => {
                Self::collect_idents_in_expr(object, out);
                Self::collect_idents_in_expr(value, out);
            }
            ExprKind::PostfixUpdate { target, .. } | ExprKind::PrefixUpdate { target, .. } => {
                Self::collect_idents_in_expr(target, out);
            }
            ExprKind::Conditional {
                condition,
                consequent,
                alternate,
            } => {
                Self::collect_idents_in_expr(condition, out);
                Self::collect_idents_in_expr(consequent, out);
                Self::collect_idents_in_expr(alternate, out);
            }
            ExprKind::ArrowFunction { body, .. } => match body {
                ArrowBody::Expr(e) => Self::collect_idents_in_expr(e, out),
                ArrowBody::Block(stmts) => Self::collect_idents_in_stmts(stmts, out),
            },
            ExprKind::ObjectLiteral { properties } => {
                for prop in properties {
                    Self::collect_idents_in_expr(&prop.value, out);
                }
            }
            ExprKind::ArrayLiteral { elements } => {
                for e in elements {
                    Self::collect_idents_in_expr(e, out);
                }
            }
            ExprKind::IndexAccess { object, index } => {
                Self::collect_idents_in_expr(object, out);
                Self::collect_idents_in_expr(index, out);
            }
            ExprKind::NewExpr { args, .. } => {
                for a in args {
                    Self::collect_idents_in_expr(a, out);
                }
            }
            ExprKind::Grouping { expr } => Self::collect_idents_in_expr(expr, out),
            // Literals and This don't contain variable identifiers
            _ => {}
        }
    }

    /// Find variables captured by an arrow function body.
    /// Returns (name, pointer_in_outer_scope, var_type) for each capture.
    fn find_captures(
        &self,
        body_stmts: &[Statement],
        params: &[Parameter],
    ) -> Vec<(String, PointerValue<'ctx>, VarType)> {
        let param_names: HashSet<String> = params.iter().map(|p| p.name.clone()).collect();
        let mut referenced = HashSet::new();
        Self::collect_idents_in_stmts(body_stmts, &mut referenced);

        let mut captures = Vec::new();
        for name in &referenced {
            if param_names.contains(name) {
                continue;
            }
            // Check if it's a variable in scope (not a function name)
            if let Some((ptr, vt)) = self.get_variable(name) {
                // Don't capture function pointers that are in self.functions
                // (they're globally accessible LLVM functions)
                if matches!(vt, VarType::FunctionPtr { .. }) {
                    continue;
                }
                captures.push((name.clone(), ptr, vt));
            }
        }
        captures
    }

    /// Compile an arrow function as a closure with an environment struct.
    /// All arrow functions use the closure convention: { fn_ptr, env_ptr } struct.
    /// The LLVM function always takes ptr %env as its first parameter.
    fn compile_closure(
        &mut self,
        fn_name: &str,
        params: &[Parameter],
        return_type: &Option<TypeAnnotation>,
        body_stmts: &[Statement],
        captures: Vec<(String, PointerValue<'ctx>, VarType)>,
        _caller_function: FunctionValue<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, VarType), CompileError> {
        let ptr_type = self.context.ptr_type(AddressSpace::default());

        // --- Determine parameter types ---
        let param_vts: Vec<VarType> = params
            .iter()
            .map(|p| {
                p.type_ann
                    .as_ref()
                    .map(|ann| self.type_ann_to_var_type(ann))
                    .unwrap_or_else(|| self.number_mode.clone())
            })
            .collect();

        // LLVM param types: [ptr (env)] + declared params
        let mut llvm_param_types: Vec<BasicMetadataTypeEnum<'ctx>> = vec![ptr_type.into()];
        for vt in &param_vts {
            llvm_param_types.push(self.var_type_to_llvm(vt).into());
        }

        let mut ret_vt = return_type
            .as_ref()
            .map(|ann| self.type_ann_to_var_type(ann));

        // Infer return type from body if not annotated
        if ret_vt.is_none() {
            if let Some(Statement {
                kind:
                    StmtKind::Return {
                        value: Some(ret_expr),
                    },
                ..
            }) = body_stmts.first()
            {
                ret_vt = Some(Self::infer_expr_var_type(ret_expr));
            }
        }

        let fn_type = match &ret_vt {
            Some(vt) => self.var_type_to_llvm(vt).fn_type(&llvm_param_types, false),
            None => self.context.void_type().fn_type(&llvm_param_types, false),
        };

        let arrow_fn = self.module.add_function(fn_name, fn_type, None);
        self.functions.insert(fn_name.to_string(), arrow_fn);

        let nounwind_id = Attribute::get_named_enum_kind_id("nounwind");
        arrow_fn.add_attribute(
            AttributeLoc::Function,
            self.context.create_enum_attribute(nounwind_id, 0),
        );

        let entry = self.context.append_basic_block(arrow_fn, "entry");
        let saved_bb = self.builder.get_insert_block();

        self.builder.position_at_end(entry);

        // --- Save and isolate scope ---
        let saved_scopes = std::mem::take(&mut self.variables);
        self.push_scope();

        // --- Set up captured variables from env struct ---
        let env_ptr = arrow_fn.get_nth_param(0).unwrap().into_pointer_value();

        if !captures.is_empty() {
            // Build the env struct type from captured variable types
            let env_field_types: Vec<BasicTypeEnum<'ctx>> = captures
                .iter()
                .map(|(_, _, vt)| self.var_type_to_llvm(vt))
                .collect();
            let env_struct_type = self.context.struct_type(&env_field_types, false);

            // GEP into env struct for each captured variable
            for (i, (name, _, vt)) in captures.iter().enumerate() {
                let field_ptr = self
                    .builder
                    .build_struct_gep(env_struct_type, env_ptr, i as u32, &format!("env.{}", name))
                    .unwrap();
                self.set_variable(name.clone(), field_ptr, vt.clone());
            }
        }

        // --- Set up declared parameters (index +1 because env is first) ---
        for (i, (param, vt)) in params.iter().zip(param_vts.iter()).enumerate() {
            let param_val = arrow_fn.get_nth_param((i + 1) as u32).unwrap();
            let alloca = self.create_alloca(arrow_fn, vt, &param.name);
            self.builder.build_store(alloca, param_val).unwrap();
            self.set_variable(param.name.clone(), alloca, vt.clone());
        }

        // --- Compile body ---
        for stmt in body_stmts {
            self.compile_statement(stmt, arrow_fn)?;
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

        // --- Restore scope ---
        self.pop_scope();
        self.variables = saved_scopes;

        if let Some(bb) = saved_bb {
            self.builder.position_at_end(bb);
        }

        // --- Create environment struct (in the caller's context) ---
        let env_alloc = if !captures.is_empty() {
            let env_field_types: Vec<BasicTypeEnum<'ctx>> = captures
                .iter()
                .map(|(_, _, vt)| self.var_type_to_llvm(vt))
                .collect();
            let env_struct_type = self.context.struct_type(&env_field_types, false);

            // malloc the environment
            let malloc_fn = self.module.get_function("malloc").unwrap_or_else(|| {
                self.module.add_function(
                    "malloc",
                    ptr_type.fn_type(&[self.context.i64_type().into()], false),
                    None,
                )
            });

            let env_size = env_struct_type.size_of().unwrap();
            let env_malloc = self
                .builder
                .build_call(malloc_fn, &[env_size.into()], "env_alloc")
                .unwrap()
                .try_as_basic_value()
                .left()
                .unwrap()
                .into_pointer_value();

            // Copy captured variable values into the env struct
            for (i, (_, src_ptr, vt)) in captures.iter().enumerate() {
                let llvm_type = self.var_type_to_llvm(vt);
                let val = self
                    .builder
                    .build_load(llvm_type, *src_ptr, "cap_val")
                    .unwrap();
                let dst_ptr = self
                    .builder
                    .build_struct_gep(
                        env_struct_type,
                        env_malloc,
                        i as u32,
                        &format!("env.store.{}", i),
                    )
                    .unwrap();
                self.builder.build_store(dst_ptr, val).unwrap();
            }

            env_malloc
        } else {
            // No captures: null env pointer
            ptr_type.const_null()
        };

        // --- Build closure struct: { fn_ptr, env_ptr } ---
        let fn_ptr = arrow_fn.as_global_value().as_pointer_value();
        let closure_val = self.closure_type.const_zero();
        let closure_val = self
            .builder
            .build_insert_value(closure_val, fn_ptr, 0, "closure.fn")
            .unwrap()
            .into_struct_value();
        let closure_val = self
            .builder
            .build_insert_value(closure_val, env_alloc, 1, "closure.env")
            .unwrap()
            .into_struct_value();

        let closure_vt = VarType::Closure {
            fn_name: fn_name.to_string(),
            param_types: param_vts,
            return_type: Box::new(ret_vt.unwrap_or(VarType::Number)),
        };

        Ok((closure_val.into(), closure_vt))
    }

    /// Monomorphize and call a generic function.
    /// Infers type parameters from compiled argument types, generates a specialized
    /// function if not already compiled, then calls it.
    fn compile_generic_call(
        &mut self,
        name: &str,
        args: &[Expr],
        function: FunctionValue<'ctx>,
        span: &Span,
    ) -> Result<(BasicValueEnum<'ctx>, VarType), CompileError> {
        // Compile arguments first to determine concrete types
        let mut compiled_args: Vec<(BasicValueEnum<'ctx>, VarType)> = Vec::new();
        for arg in args {
            compiled_args.push(self.compile_expr(arg, function)?);
        }

        // Look up the generic template
        let (tp_names, params, return_type, body) =
            self.generic_templates.get(name).cloned().ok_or_else(|| {
                CompileError::error(
                    format!("Generic template '{}' not found", name),
                    span.clone(),
                )
            })?;

        // Infer type parameters from argument VarTypes
        let mut substitutions: HashMap<String, VarType> = HashMap::new();
        for (i, param) in params.iter().enumerate() {
            if let Some(ref ann) = param.type_ann {
                if let TypeAnnKind::Named(ref type_name) = ann.kind {
                    if tp_names.contains(type_name) {
                        if let Some((_, ref arg_vt)) = compiled_args.get(i) {
                            substitutions.insert(type_name.clone(), arg_vt.clone());
                        }
                    }
                }
            }
        }

        // Generate mangled specialization name
        let suffix: String = tp_names
            .iter()
            .map(|tp| {
                substitutions
                    .get(tp)
                    .map(Self::var_type_suffix)
                    .unwrap_or("u")
            })
            .collect::<Vec<_>>()
            .join("_");
        let mangled_name = format!("{}${}", name, suffix);

        // Compile specialization if not already done
        if self.module.get_function(&mangled_name).is_none() {
            // Save current state
            let prev_subs = std::mem::replace(&mut self.type_substitutions, substitutions.clone());
            let prev_mode = self.number_mode.clone();

            // Compile the specialized function
            self.compile_function_decl(&mangled_name, &params, &return_type, &body)?;

            // Restore state
            self.type_substitutions = prev_subs;
            self.number_mode = prev_mode;
        }

        // Call the specialized function
        let spec_func = self.module.get_function(&mangled_name).ok_or_else(|| {
            CompileError::error(
                format!("Failed to compile specialization '{}'", mangled_name),
                span.clone(),
            )
        })?;

        let call_args: Vec<BasicMetadataValueEnum> =
            compiled_args.iter().map(|(v, _)| (*v).into()).collect();

        let ret = self
            .builder
            .build_call(spec_func, &call_args, "generic_call")
            .unwrap();

        let ret_vt = self
            .function_return_types
            .get(&mangled_name)
            .cloned()
            .unwrap_or(VarType::Number);

        let ret_val = ret
            .try_as_basic_value()
            .left()
            .unwrap_or_else(|| self.context.f64_type().const_float(0.0).into());

        Ok((ret_val, ret_vt))
    }

    /// Check if two VarTypes are in the same category (for conditional type evaluation).
    fn var_types_compatible(a: &VarType, b: &VarType) -> bool {
        matches!(
            (a, b),
            (
                VarType::Number | VarType::Integer,
                VarType::Number | VarType::Integer
            ) | (VarType::String, VarType::String)
                | (VarType::Boolean, VarType::Boolean)
                | (VarType::Array, VarType::Array)
        )
    }

    /// Short suffix for a VarType, used in mangled specialization names.
    fn var_type_suffix(vt: &VarType) -> &'static str {
        match vt {
            VarType::Number => "n",
            VarType::Integer => "i",
            VarType::String => "s",
            VarType::Boolean => "b",
            VarType::Array | VarType::StringArray => "a",
            VarType::FunctionPtr { .. } | VarType::Closure { .. } => "f",
            VarType::Object { .. } => "o",
            VarType::Union(_) => "u",
            VarType::Tuple(_) => "t",
            VarType::Map { .. } => "m",
            VarType::ObjArray { .. } => "oa",
            VarType::Promise { .. } => "p",
        }
    }

    /// Call a closure variable: extract fn_ptr + env_ptr, indirect call with env as first arg.
    fn compile_closure_call(
        &mut self,
        closure_ptr: PointerValue<'ctx>,
        param_types: &[VarType],
        return_type: &VarType,
        args: &[Expr],
        function: FunctionValue<'ctx>,
        span: &Span,
    ) -> Result<(BasicValueEnum<'ctx>, VarType), CompileError> {
        let ptr_type = self.context.ptr_type(AddressSpace::default());

        // Load the closure struct { fn_ptr, env_ptr }
        let closure_val = self
            .builder
            .build_load(self.closure_type, closure_ptr, "closure")
            .unwrap();
        let fn_ptr = self
            .builder
            .build_extract_value(closure_val.into_struct_value(), 0, "fn_ptr")
            .unwrap()
            .into_pointer_value();
        let env_ptr = self
            .builder
            .build_extract_value(closure_val.into_struct_value(), 1, "env_ptr")
            .unwrap();

        // Build the function type for indirect call: (ptr env, ...params) -> ret
        let mut llvm_param_types: Vec<BasicMetadataTypeEnum<'ctx>> = vec![ptr_type.into()];
        for vt in param_types {
            llvm_param_types.push(self.var_type_to_llvm(vt).into());
        }

        let fn_type = match return_type {
            VarType::Number => self.context.f64_type().fn_type(&llvm_param_types, false),
            VarType::Integer => self.context.i64_type().fn_type(&llvm_param_types, false),
            VarType::String => self.string_type.fn_type(&llvm_param_types, false),
            VarType::Boolean => self.context.bool_type().fn_type(&llvm_param_types, false),
            VarType::Array => self.array_type.fn_type(&llvm_param_types, false),
            VarType::Closure { .. } => self.closure_type.fn_type(&llvm_param_types, false),
            _ => self.context.void_type().fn_type(&llvm_param_types, false),
        };

        // Compile arguments
        let mut call_args: Vec<BasicMetadataValueEnum<'ctx>> = vec![env_ptr.into()];
        if args.len() != param_types.len() {
            return Err(CompileError::error(
                format!(
                    "Expected {} arguments, got {}",
                    param_types.len(),
                    args.len()
                ),
                span.clone(),
            ));
        }
        for (arg, expected_vt) in args.iter().zip(param_types.iter()) {
            let (val, vt) = self.compile_expr(arg, function)?;
            // Convert number types if needed
            let val = if matches!(expected_vt, VarType::Number) && matches!(vt, VarType::Integer) {
                self.builder
                    .build_signed_int_to_float(val.into_int_value(), self.context.f64_type(), "i2f")
                    .unwrap()
                    .into()
            } else if matches!(expected_vt, VarType::Integer) && matches!(vt, VarType::Number) {
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
            call_args.push(val.into());
        }

        let result = self
            .builder
            .build_indirect_call(fn_type, fn_ptr, &call_args, "closure_call")
            .unwrap();

        if let Some(val) = result.try_as_basic_value().left() {
            Ok((val, return_type.clone()))
        } else {
            Ok((
                self.context.f64_type().const_float(0.0).into(),
                VarType::Number,
            ))
        }
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
