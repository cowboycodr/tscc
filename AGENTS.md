# AGENTS.md

Instructions for AI coding agents working on the tscc compiler.

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
cargo test                           # all tests (238 pass, 16 ignored)
cargo test test_name                 # single test by name substring
cargo test module::test_name         # e.g. cargo test variables::let_with_number
cargo test arithmetic::postfix       # e.g. all postfix tests
cargo test -- --ignored              # run only ignored (unimplemented) tests
```

No linter/formatter config exists. Standard `rustfmt` and `clippy` conventions apply.

## Architecture

tscc compiles TypeScript (`.ts` files) to native binaries via LLVM 18. Not a transpiler.

**Pipeline:** Source → Lexer → Parser → Type Checker → LLVM Codegen → Linker → Native binary

```
src/
├── lib.rs              # Public API: compile_source(), compile_file()
├── main.rs             # CLI (clap): tscc build/run
├── diagnostics.rs      # CompileError, Severity, report_error()
├── modules.rs          # Multi-file import resolution, topological sort
├── lexer/
│   ├── token.rs        # Token enum (~90 variants), Span, SpannedToken
│   └── scanner.rs      # Hand-written character-by-character scanner (~570 lines)
├── parser/
│   ├── ast.rs          # StmtKind, ExprKind, BinOp, TypeAnnotation, etc. (~410 lines)
│   └── parser.rs       # Recursive descent + Pratt precedence climbing (~2,420 lines)
├── types/
│   ├── ty.rs           # Type enum: Number, String, Boolean, Array, Function, ...
│   └── checker.rs      # Structural type checking, scope stack, symbol tables (~2,290 lines)
└── codegen/
    └── llvm.rs         # LLVM IR generation via inkwell (~7,700 lines, largest file)
runtime/
└── runtime.c           # C runtime linked into every binary (~546 lines)
tests/
└── integration.rs      # End-to-end tests: compile TS source → run binary → check stdout
```

## Adding a New Feature

Every feature touches multiple pipeline stages. Follow this order:

1. **token.rs** — Add new `Token` variant if new syntax is needed
2. **scanner.rs** — Recognize the token in `scan_token()` match
3. **ast.rs** — Add `StmtKind`/`ExprKind` variant (or extend an existing one)
4. **parser.rs** — Parse the syntax; respect precedence chain:
   `assignment → ternary → logical_or → ... → multiplicative → exponentiation → unary → postfix → call → primary`
5. **checker.rs** — Type-check in `check_statement()` or `check_expr()`
6. **llvm.rs** — Emit LLVM IR in `compile_statement()` or `compile_expr()`
7. **runtime.c** — Add C functions if needed, then declare them in `declare_runtime_functions()`
8. **integration.rs** — Add or un-ignore tests

Rust's exhaustive matching will flag every missed match arm — use compiler errors as a checklist.

## Code Style

**Imports** — Three groups separated by blank lines:
```rust
use std::collections::HashMap;        // 1. std

use inkwell::context::Context;        // 2. external crates

use crate::diagnostics::CompileError; // 3. internal
use crate::parser::ast::*;
```

**Naming:**
- Types/enums: `PascalCase` — `Token`, `StmtKind`, `VarType`, `CompileError`
- Functions/methods: `snake_case` — `compile_expr`, `scan_tokens`, `check_statement`
- Variables: `snake_case`, short names common in codegen — `vt` (var type), `bb` (basic block)
- Private by default; `pub` only on API boundaries (`lib.rs`, module re-exports)

**Error handling:**
```rust
// Always include a Span for source-location display
Err(CompileError::error("message", expr.span.clone()))
Err(CompileError::error("msg", span.clone()).with_hint("try this"))

// Propagate with ?; map_err at API boundaries only
let tokens = Scanner::new(src).scan_tokens().map_err(|e| {
    report_error(src, filename, &e);
    format!("Lexer error: {}", e.message)
})?;
```

**Match expressions** — Always exhaustive. Adding a new enum variant must fix every match site.

## Key Codegen Patterns

**VarType** tracks LLVM representation at compile time:
- `Number` = f64, `Integer` = i64, `String` = `{i8*, i64}`, `Boolean` = i1
- `Array` = `{double*, i64, i64}` (data, length, capacity)
- `ObjArray { elem_vt }` = `{void**, i64, i64}` (array of heap-allocated objects)
- `Map { val_vt }` = opaque pointer to `MgMap` C struct
- `FunctionPtr { fn_name }` = opaque ptr (arrow functions / closures)
- `Object { struct_type_name, fields }` = named LLVM struct
- `Class { name, fields }` = same layout, distinct semantic type

**Scope stack** for variables: `Vec<HashMap<String, (PointerValue, VarType)>>`
— push on block entry, pop on exit, walk in reverse for lookup.

**Integer narrowing** — `analyze_integer_functions()` detects functions where all number ops
are integer-safe (no division, no float literals). Those compile as i64 not f64, enabling
LLVM's integer optimizations (matches native Rust on `fib(40)`).

**Loop context stack** — `Vec<LoopContext>` with `exit_bb` and `continue_bb` for break/continue.
After break/continue, append a dead basic block to absorb any unreachable instructions.

**Two-pass function compilation** — First pass declares all functions so mutual calls work.
Second pass compiles top-level statements into `main`.

**compile_update()** — Handles `++`/`--` on any lvalue: `Identifier` (load/inc/dec/store),
`IndexAccess` on Array (GEP into data pointer), `IndexAccess` on Object (compile-time memcmp
comparison chain over all struct fields; silent no-op on no match), `Member` (struct_gep).

**Runtime C functions** — Must be declared in both `runtime.c` AND `declare_runtime_functions()`
in `llvm.rs` with matching signatures. Structs >16 bytes on aarch64 use sret; prefer passing
pointers instead (see `tscc_array_push`). `memcmp` from libc is also declared for string comparison.

**Class field initializers** — Stored in `class_field_initializers: HashMap<String, Vec<(String, Expr)>>`.
`run_field_initializers()` emits them (parent-first) at the start of each constructor.

## Testing Conventions

Tests in `tests/integration.rs` are end-to-end: compile TS source, run binary, check stdout.

```rust
#[test]
fn feature_name() {
    assert_eq!(run_ts("console.log(1 + 2)"), "3\n");
}
```

- `run_ts(source)` — compile + execute, return stdout as `String`
- `run_ts_full(source)` — return `(stdout, stderr)` tuple
- `assert_compile_fails(source)` — assert compilation errors out
- Tests run in parallel; each gets a unique temp dir via atomic counter
- Unimplemented features: `#[ignore = "reason"]` in the `not_yet_implemented` module
- Output format: whole numbers print as integers (`42`), floats with `%.15g` (`3.14`);
  booleans as `true`/`false`; arrays as `[ 1, 2, 3 ]`; multiple `console.log` args space-separated
- Don't use `r#"..."#` strings with `\n` — it becomes a literal backslash-n. Use `"\n"`.
- Tests are grouped in `mod` blocks by feature area (`arithmetic`, `variables`, `classes`, …)

## Important Gotchas

- **Semicolons are optional** — lenient `consume_semicolon()` never errors on a missing `;`
- **`async`/`await` are not keywords** — `async` scans as `Identifier("async")` and is silently
  ignored as a standalone expression statement; `await expr` parses as two statements
- **Function hoisting is NOT supported** — functions must be declared before use
- **Import aliasing** — `import { add as sum }` works for functions (`llvm.rs:1326`);
  not yet implemented for variables or classes
- `inkwell` `AggregateValueEnum` doesn't impl `Into<BasicValueEnum>` — use `.into_struct_value().into()`
- **Template literals** are desugared to string concatenation in the scanner, not the parser
- **Objects are LLVM structs** — property access is `extract_value` at a compile-time index;
  dynamic string key access generates a `memcmp` comparison chain over all known fields
- **Classes compile to structs** — methods are separate LLVM functions with an implicit `self`
  pointer as the first parameter; `new` allocates on the stack
- **Inheritance uses struct prefix layout** — child fields follow parent fields; if the child
  has no constructor, the parent constructor is called automatically
- **`this` in methods** is a pointer passed as the first argument; `this.prop` → `struct_gep` + load/store
- **The C runtime is embedded** at `cargo build` time via `include_str!()` and compiled on-demand
  during linking — tscc binaries need no source tree at runtime

## Implementation Philosophy

**The goal is TypeScript compatibility, not compilation success.**

A feature is either implemented correctly or it is not implemented. There is no middle ground.
A half-implementation that silently compiles but produces wrong runtime behaviour is strictly
worse than a missing feature that produces a clear compile error — it hides real gaps and
misleads users into thinking their code works when it doesn't.

Concretely:
- If a built-in (e.g. `Date`, `Promise`, `setTimeout`) cannot be implemented with correct
  TypeScript semantics, leave it as `:x:` (unimplemented). Do NOT stub it out with a fake
  version that looks like it works.
- If a language feature requires other features to work correctly (e.g. `await` requires
  `Promise` requires `setTimeout` requires an event loop), implement the whole system or
  none of it. Partial assembly of an interlocking system creates silent bugs.
- If you are unsure whether an implementation is fully correct, it is not ready to ship.
  Add it to Known Technical Debt with a clear description of what is wrong.
- Compiler errors are honest. Wrong output is not.

This means some programs will not compile today. That is fine and expected. The test suite's
`:x:` entries are a roadmap, not a backlog to clear with shortcuts.

## Known Technical Debt

Behaviours that compile silently but produce incorrect results at runtime:

- **`var` is block-scoped** — tscc treats `var` as `let`; no hoisting, no function scope
- **Unknown/unregistered generics silently become `f64`** — unresolved `Named` types fall
  through `type_ann_to_var_type()` (`llvm.rs:6503`) to `VarType::Number` with no diagnostic
- **`Type::Unknown` is universally assignable** — unresolved type references produce
  `Type::Unknown` which passes every assignability check, masking real errors
- **`x[i]++` / `x.prop++` on non-simple targets** — postfix/prefix update is fully supported
  for `Identifier`, `IndexAccess`, and `Member` targets; more complex lvalue expressions
  (e.g. `a.b.c++`) are silently treated as non-lvalue and the `++`/`--` is dropped
