# AGENTS.md

Instructions for AI coding agents working on the Mango compiler.

## Build & Test

**Required environment variables** (must be set before any cargo command):
```sh
export LLVM_SYS_180_PREFIX=/opt/homebrew/opt/llvm@18
export LIBRARY_PATH="/opt/homebrew/lib:$LIBRARY_PATH"
```

**Commands:**
```sh
cargo build                          # dev build
cargo build --release                # optimized build
cargo test                           # all tests (~170 pass, ~70 ignored)
cargo test test_name                 # single test by name
cargo test module::test_name         # e.g. cargo test variables::let_with_number
cargo test -- --ignored              # run only ignored (unimplemented) tests
```

No linter/formatter config exists. Standard `rustfmt` and `clippy` conventions apply.

## Architecture

Mango compiles TypeScript (`.ts` files) to native binaries via LLVM 18. Not a transpiler.

**Pipeline:** Source → Lexer → Parser → Type Checker → LLVM Codegen → Linker → Native binary

```
src/
├── lib.rs              # Public API: compile_source(), compile_file()
├── main.rs             # CLI (clap): mango build/run
├── diagnostics.rs      # CompileError, Severity, report_error()
├── modules.rs          # Multi-file import resolution, topological sort
├── lexer/
│   ├── token.rs        # Token enum (~90 variants), Span, SpannedToken
│   └── scanner.rs      # Hand-written character-by-character scanner
├── parser/
│   ├── ast.rs          # StmtKind, ExprKind, BinOp, TypeAnnotation, etc.
│   └── parser.rs       # Recursive descent + Pratt precedence climbing
├── types/
│   ├── ty.rs           # Type enum: Number, String, Boolean, Array, Function, ...
│   └── checker.rs      # Structural type checking, scope stack, symbol tables
└── codegen/
    └── llvm.rs         # LLVM IR generation via inkwell (~2400 lines, largest file)
runtime/
└── runtime.c           # C runtime linked into every binary (print, string, math, arrays)
tests/
└── integration.rs      # End-to-end tests: compile TS source → run binary → check stdout
```

## Adding a New Feature

Every feature touches multiple pipeline stages. Follow this order:

1. **token.rs** — Add new `Token` variant if new syntax
2. **scanner.rs** — Recognize the token in `scan_token()` match
3. **ast.rs** — Add `StmtKind`/`ExprKind` variant (or `BinOp`, etc.)
4. **parser.rs** — Parse the syntax; respect precedence chain:
   `assignment → ternary → logical_or → ... → multiplicative → exponentiation → unary → postfix → call → primary`
5. **checker.rs** — Type-check in `check_statement()` or `check_expr()`
6. **llvm.rs** — Emit LLVM IR in `compile_statement()` or `compile_expr()`
7. **runtime.c** — Add C functions if needed, then declare them in `declare_runtime_functions()`
8. **integration.rs** — Add or un-ignore tests

Rust's exhaustive matching will error on any missed match arm — use compiler errors as a checklist.

## Code Style

**Imports** — Three groups, separated by blank lines:
```rust
use std::collections::HashMap;        // 1. std

use inkwell::context::Context;        // 2. external crates

use crate::diagnostics::CompileError; // 3. internal
use crate::parser::ast::*;
```

**Naming:**
- Types/enums: `PascalCase` — `Token`, `StmtKind`, `VarType`, `CompileError`
- Functions/methods: `snake_case` — `compile_expr`, `scan_tokens`, `check_statement`
- Variables: `snake_case`, short names common — `vt` (var type), `bb` (basic block), `fn_name`
- Private by default; `pub` only on API boundaries (`lib.rs`, module `mod.rs` re-exports)

**Error handling:**
```rust
// Create errors with span for source location
Err(CompileError::error("message", expr.span.clone()))
Err(CompileError::error("msg", span.clone()).with_hint("try this"))

// Propagate with ?; convert with map_err at API boundaries
let tokens = Scanner::new(src).scan_tokens().map_err(|e| {
    report_error(src, filename, &e);
    format!("Lexer error: {}", e.message)
})?;
```

Every `CompileError` carries a `Span` (start, end, line, column) for source-location error display.

**Match expressions** — Always exhaustive. When adding a new enum variant, fix every match site.

## Key Codegen Patterns

**VarType** tracks LLVM representation at compile time:
- `Number` = f64, `Integer` = i64, `String` = `{i8*, i64}`, `Boolean` = i1
- `Array` = `{double*, i64, i64}` (data, length, capacity)
- `FunctionPtr { fn_name }` = opaque ptr (arrow functions)

**Scope stack** for variables: `Vec<HashMap<String, (PointerValue, VarType)>>`
— push on block entry, pop on exit, walk in reverse for lookup.

**Integer narrowing** — Analysis pass (`analyze_integer_functions`) detects functions where all
number ops are integer-safe (no division, no floats). These compile as i64 instead of f64,
enabling LLVM to produce faster code (matches native Rust on fib(40)).

**Loop context stack** — `Vec<LoopContext>` with `exit_bb` and `continue_bb` for break/continue.
After break/continue, create a dead basic block to absorb unreachable code.

**Two-pass function compilation** — First pass: declare all functions (so they can call each other).
Second pass: compile top-level code (main function).

**Runtime C functions** — Must be declared in both `runtime.c` AND `declare_runtime_functions()`
in llvm.rs with matching signatures. Struct returns >16 bytes on aarch64 use sret convention;
prefer passing pointers to structs instead (see `mango_array_push`).

## Testing Conventions

Tests are end-to-end integration tests in `tests/integration.rs`:
```rust
#[test]
fn feature_name() {
    assert_eq!(run_ts("console.log(1 + 2)"), "3\n");
}
```

- `run_ts(source)` — compile + execute, return stdout as String
- `run_ts_full(source)` — return `(stdout, stderr)` tuple
- `assert_compile_fails(source)` — verify compilation produces an error
- Tests run in parallel; each gets a unique temp directory via atomic counter
- Unimplemented features: `#[ignore = "reason"]` in the `not_yet_implemented` module
- Output format: numbers print as integers when whole (`42`), floats with `%.15g` (`3.14`);
  booleans as `true`/`false`; arrays as `[ 1, 2, 3 ]`; multiple console.log args space-separated
- Don't use `r#"..."#` with `\n` — it becomes literal `\n`, not newline. Use regular strings.

## Important Gotchas

- **Semicolons are optional** in Mango (lenient parsing, like TypeScript)
- **Function hoisting is NOT supported** — type checker doesn't pre-scan declarations
- **Import aliasing (`as`) is broken in codegen** — alias maps in checker but codegen uses original name
- `inkwell` `AggregateValueEnum` doesn't impl `Into<BasicValueEnum>` — call `.into_struct_value().into()`
- Template literals are desugared to string concatenation in the scanner (not a parser feature)
- LLVM contexts are safe to create per-thread; each test gets its own
- The `find_runtime()` helper works in tests because `cargo test` runs from project root
