mod codegen;
mod diagnostics;
mod lexer;
mod modules;
mod parser;
mod types;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use clap::{Parser as ClapParser, Subcommand};
use inkwell::context::Context;

use crate::codegen::llvm::Codegen;
use crate::diagnostics::report_error;
use crate::lexer::scanner::Scanner;
use crate::modules::ModuleGraph;
use crate::parser::parser::Parser;
use crate::types::checker::TypeChecker;

#[derive(ClapParser)]
#[command(name = "mango")]
#[command(about = "Mango - TypeScript compiled to native machine code")]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Compile a TypeScript file to a native executable
    Build {
        /// Input .ts file
        file: String,

        /// Output executable name (defaults to filename without .ts extension)
        #[arg(short, long)]
        output: Option<String>,

        /// Print LLVM IR instead of compiling
        #[arg(long)]
        emit_ir: bool,
    },

    /// Compile and immediately run a TypeScript file
    Run {
        /// Input .ts file
        file: String,

        /// Output executable name (defaults to filename without .ts extension)
        #[arg(short, long)]
        output: Option<String>,

        /// Time the execution (equivalent to prefixing with `time`)
        #[arg(long)]
        benchmark: bool,
    },
}

fn main() {
    let cli = Cli::parse();

    match cli.command {
        Commands::Build {
            file,
            output,
            emit_ir,
        } => {
            let output_name = output.unwrap_or_else(|| {
                Path::new(&file)
                    .file_stem()
                    .unwrap()
                    .to_string_lossy()
                    .to_string()
            });
            if let Err(e) = compile(&file, &output_name, emit_ir) {
                eprintln!("\x1b[1;31merror\x1b[0m: {}", e);
                std::process::exit(1);
            }
        }
        Commands::Run {
            file,
            output,
            benchmark,
        } => {
            let output_name = output.unwrap_or_else(|| {
                let stem = Path::new(&file)
                    .file_stem()
                    .unwrap()
                    .to_string_lossy()
                    .to_string();
                format!("./{}", stem)
            });
            if let Err(e) = compile(&file, &output_name, false) {
                eprintln!("\x1b[1;31merror\x1b[0m: {}", e);
                std::process::exit(1);
            }

            let status = if benchmark {
                // Use `time` to measure execution
                let start = std::time::Instant::now();
                let result = Command::new(&output_name)
                    .status()
                    .expect("Failed to run compiled program");
                let elapsed = start.elapsed();
                eprintln!(
                    "\n\x1b[1;36m{:.3}s\x1b[0m ({}ms)",
                    elapsed.as_secs_f64(),
                    elapsed.as_millis()
                );
                result
            } else {
                Command::new(&output_name)
                    .status()
                    .expect("Failed to run compiled program")
            };

            std::process::exit(status.code().unwrap_or(1));
        }
    }
}

fn compile(input: &str, output: &str, emit_ir: bool) -> Result<(), String> {
    let input_path = Path::new(input);

    // Check if the file has imports (needs multi-file compilation)
    let source =
        std::fs::read_to_string(input).map_err(|e| format!("Error reading '{}': {}", input, e))?;

    let has_imports = source.contains("import ");

    if has_imports {
        compile_multi_file(input_path, output, emit_ir)
    } else {
        compile_single_file(input, &source, output, emit_ir)
    }
}

fn compile_single_file(
    filename: &str,
    source: &str,
    output: &str,
    emit_ir: bool,
) -> Result<(), String> {
    // Lex
    let tokens = Scanner::new(source).scan_tokens().map_err(|e| {
        report_error(source, filename, &e);
        "Lexer error".to_string()
    })?;

    // Parse
    let mut parser = Parser::new(tokens);
    let program = parser.parse().map_err(|e| {
        report_error(source, filename, &e);
        "Parse error".to_string()
    })?;

    // Type check
    let mut checker = TypeChecker::new();
    checker.check(&program).map_err(|e| {
        report_error(source, filename, &e);
        "Type error".to_string()
    })?;

    // Codegen
    let context = Context::create();
    let mut codegen = Codegen::new(&context, filename);
    codegen.compile(&program).map_err(|e| {
        report_error(source, filename, &e);
        "Codegen error".to_string()
    })?;

    if emit_ir {
        println!("{}", codegen.print_ir());
        return Ok(());
    }

    link_and_output(&codegen, output)
}

fn compile_multi_file(entry_path: &Path, output: &str, emit_ir: bool) -> Result<(), String> {
    // Build the module graph
    let graph = ModuleGraph::build(entry_path)?;

    // Type check all modules in dependency order, collecting exports
    let mut all_exports: HashMap<PathBuf, HashMap<String, crate::types::ty::Type>> = HashMap::new();

    for module in &graph.modules {
        let mut checker = TypeChecker::new();

        // Resolve imports: find what this module imports and from where
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

    // Codegen: compile the entry module (single-file for now, functions from imports
    // are linked via separate object files)
    let entry_idx = graph.entry_index();
    let entry = &graph.modules[entry_idx];

    let context = Context::create();
    let mut codegen = Codegen::new(&context, &entry.path.to_string_lossy());

    // For multi-file: compile imported module functions first
    for (i, module) in graph.modules.iter().enumerate() {
        if i == entry_idx {
            continue;
        }
        // Compile exported functions from dependency modules into the same LLVM module
        for stmt in &module.program.statements {
            if let crate::parser::ast::StmtKind::FunctionDecl {
                name,
                params,
                return_type,
                body,
                is_exported,
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
        println!("{}", codegen.print_ir());
        return Ok(());
    }

    link_and_output(&codegen, output)
}

fn link_and_output(codegen: &Codegen, output: &str) -> Result<(), String> {
    let obj_path = PathBuf::from(format!("{}.o", output));
    codegen
        .write_object_file(&obj_path)
        .map_err(|e| format!("Failed to write object file: {}", e))?;

    let runtime_src = find_runtime()?;
    let runtime_obj = PathBuf::from(format!("{}_runtime.o", output));
    let cc_status = Command::new("cc")
        .args([
            "-c",
            "-O2",
            runtime_src.to_str().unwrap(),
            "-o",
            runtime_obj.to_str().unwrap(),
        ])
        .status()
        .map_err(|e| format!("Failed to compile runtime: {}", e))?;

    if !cc_status.success() {
        return Err("Failed to compile runtime".to_string());
    }

    let link_status = Command::new("cc")
        .args([
            obj_path.to_str().unwrap(),
            runtime_obj.to_str().unwrap(),
            "-o",
            output,
            "-lm",
        ])
        .status()
        .map_err(|e| format!("Failed to link: {}", e))?;

    if !link_status.success() {
        return Err("Failed to link executable".to_string());
    }

    let _ = std::fs::remove_file(&obj_path);
    let _ = std::fs::remove_file(&runtime_obj);

    eprintln!("Compiled -> {}", output);
    Ok(())
}

fn resolve_import_path(parent_dir: &Path, source: &str) -> Result<PathBuf, String> {
    let mut target = parent_dir.join(source);
    if target.extension().is_none() {
        target.set_extension("ts");
    }
    std::fs::canonicalize(&target).map_err(|e| format!("Cannot resolve '{}': {}", source, e))
}

fn find_runtime() -> Result<PathBuf, String> {
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()));

    let candidates = [
        exe_dir
            .as_ref()
            .map(|d| d.join("runtime").join("runtime.c")),
        exe_dir.as_ref().map(|d| d.join("runtime.c")),
        Some(PathBuf::from("runtime/runtime.c")),
        Some(PathBuf::from("runtime.c")),
    ];

    for candidate in candidates.iter().flatten() {
        if candidate.exists() {
            return Ok(candidate.clone());
        }
    }

    Err("Could not find runtime.c. Make sure it's in the runtime/ directory.".to_string())
}
