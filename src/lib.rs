pub mod codegen;
pub mod diagnostics;
pub mod lexer;
pub mod modules;
pub mod parser;
pub mod types;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use inkwell::context::Context;

use crate::codegen::llvm::Codegen;
use crate::diagnostics::report_error;
use crate::lexer::scanner::Scanner;
use crate::modules::ModuleGraph;
use crate::parser::parser::Parser;
use crate::types::checker::TypeChecker;

/// The pre-compiled runtime static library, embedded at cargo build time by build.rs.
/// build.rs compiles runtime/runtime.c once via the `cc` crate and writes the path
/// of the resulting libtscc_runtime.a into the RUNTIME_LIB_PATH env var.
const RUNTIME_LIB: &[u8] = include_bytes!(env!("RUNTIME_LIB_PATH"));

/// Compile a source string directly (no file read needed).
/// Used by tests and programmatic callers.
pub fn compile_source(source: &str, output: &str, optimize: bool) -> Result<(), String> {
    compile_single_file("<source>", source, output, false, optimize)
}

/// Compile a .ts file to a native executable.
pub fn compile_file(
    input: &str,
    output: &str,
    emit_ir: bool,
    optimize: bool,
) -> Result<(), String> {
    let input_path = Path::new(input);

    let source =
        std::fs::read_to_string(input).map_err(|e| format!("Error reading '{}': {}", input, e))?;

    let has_imports = source.contains("import ");

    if has_imports {
        compile_multi_file(input_path, output, emit_ir, optimize)
    } else {
        compile_single_file(input, &source, output, emit_ir, optimize)
    }
}

pub fn compile_single_file(
    filename: &str,
    source: &str,
    output: &str,
    emit_ir: bool,
    release: bool,
) -> Result<(), String> {
    // Lex
    let tokens = Scanner::new(source).scan_tokens().map_err(|e| {
        report_error(source, filename, &e);
        format!("Lexer error: {}", e.message)
    })?;

    // Parse
    let mut parser = Parser::new(tokens);
    let program = parser.parse().map_err(|e| {
        report_error(source, filename, &e);
        format!("Parse error: {}", e.message)
    })?;

    // Type check
    let mut checker = TypeChecker::new();
    checker.check(&program).map_err(|e| {
        report_error(source, filename, &e);
        format!("Type error: {}", e.message)
    })?;

    // Codegen
    let context = Context::create();
    let mut codegen = Codegen::new(&context, filename);
    codegen.compile(&program).map_err(|e| {
        report_error(source, filename, &e);
        format!("Codegen error: {}", e.message)
    })?;

    if emit_ir {
        if release {
            codegen
                .optimize()
                .map_err(|e| format!("Optimization error: {}", e))?;
        }
        println!("{}", codegen.print_ir());
        return Ok(());
    }

    if release {
        codegen
            .optimize()
            .map_err(|e| format!("Optimization error: {}", e))?;
    }

    link_and_output(&codegen, output)
}

pub fn compile_multi_file(
    entry_path: &Path,
    output: &str,
    emit_ir: bool,
    release: bool,
) -> Result<(), String> {
    let graph = ModuleGraph::build(entry_path)?;

    let mut all_exports: HashMap<PathBuf, HashMap<String, crate::types::ty::Type>> = HashMap::new();

    for module in &graph.modules {
        let mut checker = TypeChecker::new();

        let parent_dir = module.path.parent().unwrap_or(Path::new("."));
        for stmt in &module.program.statements {
            if let crate::parser::ast::StmtKind::Import { specifiers, source } = &stmt.kind {
                let dep_path = resolve_import_path(parent_dir, source)?;
                if let Some(dep_exports) = all_exports.get(&dep_path) {
                    for spec in specifiers {
                        if let Some(ty) = dep_exports.get(&spec.imported) {
                            checker
                                .imported_symbols
                                .insert(spec.local.clone(), ty.clone());
                        }
                    }
                }
            }
        }

        let filename = module.path.to_string_lossy().to_string();
        checker.check(&module.program).map_err(|e| {
            report_error(&module.source, &filename, &e);
            format!("Type error in '{}'", filename)
        })?;

        all_exports.insert(module.path.clone(), checker.exported_symbols);
    }

    let entry_idx = graph.entry_index();
    let entry = &graph.modules[entry_idx];

    let context = Context::create();
    let mut codegen = Codegen::new(&context, &entry.path.to_string_lossy());

    for (i, module) in graph.modules.iter().enumerate() {
        if i == entry_idx {
            continue;
        }
        for stmt in &module.program.statements {
            if let crate::parser::ast::StmtKind::FunctionDecl {
                name,
                params,
                return_type,
                body,
                is_exported,
                ..
            } = &stmt.kind
            {
                if *is_exported {
                    codegen
                        .compile_exported_function(name, params, return_type, body)
                        .map_err(|e| format!("Codegen error: {}", e.message))?;
                }
            }
        }
    }

    codegen.compile(&entry.program).map_err(|e| {
        let filename = entry.path.to_string_lossy().to_string();
        report_error(&entry.source, &filename, &e);
        "Codegen error".to_string()
    })?;

    if emit_ir {
        if release {
            codegen
                .optimize()
                .map_err(|e| format!("Optimization error: {}", e))?;
        }
        println!("{}", codegen.print_ir());
        return Ok(());
    }

    if release {
        codegen
            .optimize()
            .map_err(|e| format!("Optimization error: {}", e))?;
    }

    link_and_output(&codegen, output)
}

pub fn link_and_output(codegen: &Codegen, output: &str) -> Result<(), String> {
    // Module verification available via codegen.verify_module() for debugging.
    let obj_path = PathBuf::from(format!("{}.o", output));
    codegen
        .write_object_file(&obj_path)
        .map_err(|e| format!("Failed to write object file: {}", e))?;

    // Write the pre-compiled runtime static library to a temp path next to the output.
    // The .a was compiled once at `cargo build` time by build.rs and embedded as bytes —
    // no C toolchain is required on the user's machine at compile time.
    let runtime_lib_path = PathBuf::from(format!("{}_runtime.a", output));
    std::fs::write(&runtime_lib_path, RUNTIME_LIB)
        .map_err(|e| format!("Failed to write runtime library: {}", e))?;

    let link_status = Command::new("cc")
        .args([
            obj_path.to_str().unwrap(),
            runtime_lib_path.to_str().unwrap(),
            "-o",
            output,
            "-lm",
            "-Wl,-w",
        ])
        .status()
        .map_err(|e| format!("Failed to link: {}", e))?;

    let _ = std::fs::remove_file(&obj_path);
    let _ = std::fs::remove_file(&runtime_lib_path);

    if !link_status.success() {
        return Err("Failed to link executable".to_string());
    }

    Ok(())
}

pub fn resolve_import_path(parent_dir: &Path, source: &str) -> Result<PathBuf, String> {
    let mut target = parent_dir.join(source);
    if target.extension().is_none() {
        target.set_extension("ts");
    }
    std::fs::canonicalize(&target).map_err(|e| format!("Cannot resolve '{}': {}", source, e))
}
