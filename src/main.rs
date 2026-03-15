use std::path::Path;
use std::process::Command;

use clap::{Parser as ClapParser, Subcommand};

#[derive(ClapParser)]
#[command(name = "tscc")]
#[command(about = "tscc - TypeScript compiled to native machine code")]
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

        /// Skip LLVM optimizations (faster compile, slower binary)
        #[arg(long)]
        debug: bool,
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

        /// Skip LLVM optimizations (faster compile, slower binary)
        #[arg(long)]
        debug: bool,
    },
}

fn main() {
    let cli = Cli::parse();

    match cli.command {
        Commands::Build {
            file,
            output,
            emit_ir,
            debug,
        } => {
            let output_name = output.unwrap_or_else(|| {
                Path::new(&file)
                    .file_stem()
                    .unwrap()
                    .to_string_lossy()
                    .to_string()
            });
            let optimize = !debug;
            if let Err(e) = tscc::compile_file(&file, &output_name, emit_ir, optimize) {
                eprintln!("\x1b[1;31merror\x1b[0m: {}", e);
                std::process::exit(1);
            }
        }
        Commands::Run {
            file,
            output,
            benchmark,
            debug,
        } => {
            let output_name = output.unwrap_or_else(|| {
                let stem = Path::new(&file)
                    .file_stem()
                    .unwrap()
                    .to_string_lossy()
                    .to_string();
                format!("./{}", stem)
            });
            let optimize = !debug;
            if let Err(e) = tscc::compile_file(&file, &output_name, false, optimize) {
                eprintln!("\x1b[1;31merror\x1b[0m: {}", e);
                std::process::exit(1);
            }

            let status = if benchmark {
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
