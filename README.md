# Mango

A compiler that takes TypeScript syntax and compiles it to native machine code via LLVM.

Not a transpiler. Not a runtime. Mango produces standalone native binaries from `.ts` files.

```
mango run examples/hello.ts
```

## Benchmarks

Recursive Fibonacci (`fib(40)`) on Apple Silicon (M3):

| Runtime | Time | vs Mango |
|---|---|---|
| **Mango** | **0.28s** | 1.0x |
| Rust (`rustc -O`, i64) | 0.29s | 1.0x |
| Rust (`rustc -O`, f64) | 0.43s | 1.5x slower |
| Bun 1.3 | 0.51s | 1.8x slower |
| Node 23 | 0.79s | 2.8x slower |

Mango matches native Rust performance through integer narrowing — an analysis pass that detects when `number` values can safely compile as `i64` instead of `f64`, enabling LLVM to apply integer-specific optimizations.

Compilation takes ~100ms with O3 optimization, ~30ms in debug mode.

## Install

Requires LLVM 18 and Rust.

```sh
# Install LLVM 18
brew install llvm@18

# Set environment variables (add to your shell profile)
export LLVM_SYS_180_PREFIX=/opt/homebrew/opt/llvm@18
export LIBRARY_PATH="/opt/homebrew/lib:$LIBRARY_PATH"

# Build Mango
cargo install --path .
```

## Usage

```sh
# Compile and run
mango run file.ts

# Compile to binary
mango build file.ts            # outputs ./file
mango build file.ts -o output  # custom output path

# Flags
mango run file.ts --benchmark  # time execution
mango build file.ts --debug    # skip optimization (faster compile)
mango build file.ts --emit-ir  # print LLVM IR
```

## Examples

```ts
// hello.ts
function greet(name: string): string {
    return `Hello, ${name}!`
}

console.log(greet("World"))

let numbers = [10, 20, 30]
console.log("Second:", numbers[1])

let double = (x: number): number => x * 2
console.log(double(21))  // 42
```

```sh
$ mango run examples/hello.ts
Hello, World!
Second: 20
42
```

See [`examples/`](examples/) for more.

## What works

**Types & variables** — `let`, `const`, type annotations, type inference

**Functions** — declarations, arrow functions, recursion, multi-file `import`/`export`

**Control flow** — `if`/`else`, `while`, `for`, `break`, `continue`, ternary (`? :`)

**Operators** — arithmetic, comparison, logical, `++`/`--`, `+=`/`-=`/`*=`/`/=`, `**`, `typeof`

**Strings** — literals, template literals, concatenation, `.length`, `.toUpperCase()`, `.toLowerCase()`, `.charAt()`, `.indexOf()`, `.includes()`, `.substring()`, `.slice()`, `.trim()`

**Arrays** — literals, index access, `.length`, `.push()`, `.pop()`

**Math** — `Math.floor`, `ceil`, `round`, `abs`, `sqrt`, `pow`, `min`, `max`, `sin`, `cos`, `tan`, `log`, `exp`, `random`, `PI`, `E`

**Globals** — `console.log`, `console.error`, `console.warn`, `parseInt`, `parseFloat`

## Architecture

```
TypeScript source
    │
    ├─ Lexer ──────── Hand-written scanner
    ├─ Parser ─────── Recursive descent + Pratt precedence
    ├─ Type Checker ── Structural typing, inference
    ├─ Codegen ────── LLVM IR via inkwell
    ├─ Optimizer ──── LLVM O3 + native CPU targeting
    └─ Linker ─────── Clang (links with C runtime)
        │
    Native binary
```

Written in Rust. Single crate. ~4,000 lines.

The C runtime (`runtime/runtime.c`) provides print functions, string operations, math functions, and array support. It's compiled and linked into every binary.

## Status

Early stage. The goal is drop-in compatibility with existing TypeScript projects. Currently covers the core language features needed for compute-heavy programs.

Not yet implemented: closures, objects, classes, interfaces, generics, union types, enums, `async`/`await`, and many other TypeScript features. See the test suite for exact coverage.

## License

MIT
