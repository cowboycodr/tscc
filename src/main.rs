mod codegen;
mod diagnostics;
mod lexer;
mod parser;
mod types;

use std::path::{Path, PathBuf};
use std::process::Command;

use clap::{Parser as ClapParser, Subcommand};
use inkwell::context::Context;

use crate::codegen::llvm::Codegen;
use crate::diagnostics::report_error;
use crate::lexer::scanner::Scanner;
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

        /// Output executable name (defaults to input filename without extension)
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
                eprintln!("{}", e);
                std::process::exit(1);
            }
        }
        Commands::Run { file } => {
            let output_name = format!(
                "/tmp/mango_{}",
                Path::new(&file).file_stem().unwrap().to_string_lossy()
            );
            if let Err(e) = compile(&file, &output_name, false) {
                eprintln!("{}", e);
                std::process::exit(1);
            }
            let status = Command::new(&output_name)
                .status()
                .expect("Failed to run compiled program");
            std::process::exit(status.code().unwrap_or(1));
        }
    }
}

fn compile(input: &str, output: &str, emit_ir: bool) -> Result<(), String> {
    // Read source
    let source =
        std::fs::read_to_string(input).map_err(|e| format!("Error reading '{}': {}", input, e))?;

    // Lex
    let tokens = Scanner::new(&source).scan_tokens().map_err(|e| {
        report_error(&source, &e);
        format!("Lexer error")
    })?;

    // Parse
    let mut parser = Parser::new(tokens);
    let program = parser.parse().map_err(|e| {
        report_error(&source, &e);
        format!("Parse error")
    })?;

    // Type check
    let mut checker = TypeChecker::new();
    checker.check(&program).map_err(|e| {
        report_error(&source, &e);
        format!("Type error")
    })?;

    // Codegen
    let context = Context::create();
    let mut codegen = Codegen::new(&context, input);
    codegen.compile(&program).map_err(|e| {
        report_error(&source, &e);
        format!("Codegen error")
    })?;

    if emit_ir {
        println!("{}", codegen.print_ir());
        return Ok(());
    }

    // Write object file
    let obj_path = PathBuf::from(format!("{}.o", output));
    codegen
        .write_object_file(&obj_path)
        .map_err(|e| format!("Failed to write object file: {}", e))?;

    // Compile runtime
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

    // Link
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

    // Cleanup temp files
    let _ = std::fs::remove_file(&obj_path);
    let _ = std::fs::remove_file(&runtime_obj);

    eprintln!("Compiled {} -> {}", input, output);
    Ok(())
}

fn find_runtime() -> Result<PathBuf, String> {
    // Look for runtime.c relative to the executable
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()));

    let candidates = [
        // Next to the mango binary
        exe_dir
            .as_ref()
            .map(|d| d.join("runtime").join("runtime.c")),
        exe_dir.as_ref().map(|d| d.join("runtime.c")),
        // In the current working directory
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
