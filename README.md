# tscc

A compiler that takes TypeScript syntax and compiles it to native machine code via LLVM.

Not a transpiler. Not a runtime. tscc produces standalone native binaries from `.ts` files.

```
tscc run examples/hello.ts
```

## Performance

All benchmarks on Apple Silicon M3. Best of 3 runs.

### Recursive Fibonacci — `fib(40)`

| Runtime | Time | Relative | Memory |
|---|---|---|---|
| **tscc** | **0.28s** | **1.0x** | **1.2 MB** |
| C (`cc -O3`) | 0.28s | 1.0x | 1.2 MB |
| Rust (`rustc -O`, i64) | 0.28s | 1.0x | 1.4 MB |
| Rust (`rustc -O`, f64) | 0.41s | 1.5x slower | 1.4 MB |
| Bun 1.3 | 0.48s | 1.7x slower | 27 MB |
| Node 25 | 0.79s | 2.8x slower | 49 MB |

### Loop Sum — `sum(0..1_000_000_000)`

| Runtime | Time | Relative |
|---|---|---|
| **tscc** | **< 0.01s** | **--** |
| Rust (`rustc -O`) | < 0.01s | -- |
| Bun 1.3 | 0.88s | -- |
| Node 25 | 1.47s | -- |

Both tscc and Rust produce effectively instant results — LLVM optimizes the entire loop into a closed-form computation at compile time.

### Why is tscc fast?

- **LLVM O3** — Full optimization pipeline (loop vectorization, SLP vectorization, function inlining, dead code elimination)
- **Native CPU targeting** — Generates code tuned for the exact CPU (`-mcpu=native`)
- **Integer narrowing** — Analysis pass detects when `number` values can compile as `i64` instead of `f64`, enabling LLVM's integer-specific optimizations (accumulator transformation, strength reduction)
- **No runtime overhead** — No JIT warmup, no garbage collector, no event loop. Just a native binary
- **Tiny binaries** — `fib(40)` compiles to a 37 KB binary (vs 441 KB for Rust)

### Compilation Speed

| Mode | Time |
|---|---|
| Optimized (O3) | ~90ms |
| Debug (no optimization) | ~80ms |

## Install

Requires LLVM 18 and Rust.

```sh
# Install LLVM 18
brew install llvm@18

# Set environment variables (add to your shell profile)
export LLVM_SYS_180_PREFIX=/opt/homebrew/opt/llvm@18
export LIBRARY_PATH="/opt/homebrew/lib:$LIBRARY_PATH"

# Build tscc
cargo install --path .
```

## Usage

```sh
# Compile and run
tscc run file.ts

# Compile to binary
tscc build file.ts            # outputs ./file
tscc build file.ts -o output  # custom output path

# Flags
tscc run file.ts --benchmark  # time execution
tscc build file.ts --debug    # skip optimization (faster compile)
tscc build file.ts --emit-ir  # print LLVM IR
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
$ tscc run examples/hello.ts
Hello, World!
Second: 20
42
```

Objects and classes compile to zero-overhead LLVM structs:

```ts
class Point {
    x: number
    y: number
    constructor(x: number, y: number) {
        this.x = x
        this.y = y
    }
    toString(): string {
        return this.x + "," + this.y
    }
}

let p = new Point(3, 4)
console.log(p)            // { x: 3, y: 4, toString: [complex] }
console.log(p.toString()) // 3,4
```

See [`examples/`](examples/) for more.

## Architecture

```
TypeScript source
    |
    +- Lexer -------- Hand-written scanner
    +- Parser ------- Recursive descent + Pratt precedence
    +- Type Checker -- Structural typing, inference
    +- Codegen ------ LLVM IR via inkwell
    +- Optimizer ---- LLVM O3 + native CPU targeting
    +- Linker ------- Links with pre-compiled runtime
    |
    Native binary
```

Written in Rust. Single crate. ~12,600 lines of Rust + ~470 lines of C runtime.

The runtime (`runtime/runtime.c`) provides print functions, string operations, math functions, and array support. It is compiled once at `cargo build` time and embedded directly into the `tscc` binary — no C toolchain is required on the user's machine to compile TypeScript files.

## Status

Early stage. 225 tests passing, 16 pending. The goal is drop-in compatibility with existing TypeScript projects. Currently covers the core language features needed for compute-heavy programs.

## TypeScript Feature Coverage

**225 passing** / **16 not yet implemented** — 93% of test suite

### Literals & Primitives

| Feature | Status | Test |
|---|---|---|
| Integer literals | :white_check_mark: | `console.log(42)` |
| Float literals | :white_check_mark: | `console.log(3.14)` |
| Negative numbers | :white_check_mark: | `console.log(-7)` |
| Zero | :white_check_mark: | `console.log(0)` |
| Large integers | :white_check_mark: | `console.log(1000000)` |
| String (double quotes) | :white_check_mark: | `console.log("hello")` |
| String (single quotes) | :white_check_mark: | `console.log('world')` |
| Empty string | :white_check_mark: | `console.log("")` |
| Boolean `true` | :white_check_mark: | `console.log(true)` |
| Boolean `false` | :white_check_mark: | `console.log(false)` |
| `null` | :white_check_mark: | `console.log(null)` |
| `undefined` | :white_check_mark: | `console.log(undefined)` |
| BigInt literals | :x: | `9007199254740993n` |

### Variables

| Feature | Status | Test |
|---|---|---|
| `let` with number | :white_check_mark: | `let x = 10` |
| `let` with string | :white_check_mark: | `let s = "hi"` |
| `let` with boolean | :white_check_mark: | `let b = true` |
| `const` | :white_check_mark: | `const x = 99` |
| Type annotations | :white_check_mark: | `let x: number = 5` |
| String type annotation | :white_check_mark: | `let s: string = "typed"` |
| Boolean type annotation | :white_check_mark: | `let b: boolean = false` |
| Reassignment | :white_check_mark: | `x = 2` |
| Multiple variables | :white_check_mark: | `let a = 1; let b = 2` |
| Uninitialized `let` | :white_check_mark: | `let x: number` (defaults to 0) |
| Optional semicolons | :white_check_mark: | `let x = 42` |
| `var` declarations | :white_check_mark: | `var x = 42` |

### Arithmetic Operators

| Feature | Status | Test |
|---|---|---|
| Addition `+` | :white_check_mark: | `2 + 3` |
| Subtraction `-` | :white_check_mark: | `10 - 4` |
| Multiplication `*` | :white_check_mark: | `3 * 7` |
| Division `/` | :white_check_mark: | `10 / 4` |
| Modulo `%` | :white_check_mark: | `10 % 3` |
| Exponentiation `**` | :white_check_mark: | `2 ** 10` |
| Operator precedence | :white_check_mark: | `2 + 3 * 4` = 14 |
| Parenthesized expressions | :white_check_mark: | `(2 + 3) * 4` = 20 |
| Postfix `++` / `--` | :white_check_mark: | `x++`, `x--` |
| Prefix `++` / `--` | :white_check_mark: | `++x`, `--x` |
| Unary negate | :white_check_mark: | `-x` |
| `+=` `-=` `*=` `/=` | :white_check_mark: | `x += 3` |

### Comparison Operators

| Feature | Status | Test |
|---|---|---|
| `<` `>` `<=` `>=` | :white_check_mark: | `1 < 2`, `5 > 3` |
| `==` `!=` | :white_check_mark: | `5 == 5`, `5 != 6` |
| `===` `!==` | :white_check_mark: | `5 === 5`, `5 !== 5` |
| Boolean equality | :white_check_mark: | `true == true` |

### Logical Operators

| Feature | Status | Test |
|---|---|---|
| `&&` | :white_check_mark: | `true && true` |
| `\|\|` | :white_check_mark: | `false \|\| true` |
| `!` | :white_check_mark: | `!true` |
| Complex logical | :white_check_mark: | `true && !false \|\| false` |
| Numeric `&&` / `\|\|` | :white_check_mark: | `1 && 1`, `0 \|\| 1` |
| Nullish coalescing `??` | :white_check_mark: | `null ?? 42` |
| Optional chaining `?.` | :white_check_mark: | `obj?.a` |

### Strings

| Feature | Status | Test |
|---|---|---|
| Concatenation `+` | :white_check_mark: | `"hello" + " world"` |
| String + number | :white_check_mark: | `"value: " + 42` |
| String + boolean | :white_check_mark: | `"it is " + true` |
| `.length` | :white_check_mark: | `"hello".length` |
| `.toUpperCase()` | :white_check_mark: | `"hello".toUpperCase()` |
| `.toLowerCase()` | :white_check_mark: | `"HELLO".toLowerCase()` |
| `.charAt()` | :white_check_mark: | `"hello".charAt(1)` |
| `.indexOf()` | :white_check_mark: | `"hello world".indexOf("world")` |
| `.includes()` | :white_check_mark: | `"hello world".includes("world")` |
| `.substring()` | :white_check_mark: | `"hello world".substring(0, 5)` |
| `.slice()` | :white_check_mark: | `"hello world".slice(-5)` |
| `.trim()` | :white_check_mark: | `"  hello  ".trim()` |
| Template literals | :white_check_mark: | `` `value is ${x}` `` |
| Chained methods | :white_check_mark: | `"  Hello  ".trim().toUpperCase()` |
| `.startsWith()` | :white_check_mark: | `"hello".startsWith("he")` |
| `.endsWith()` | :white_check_mark: | `"hello".endsWith("lo")` |
| `.repeat()` | :white_check_mark: | `"ab".repeat(3)` |
| `.split()` | :white_check_mark: | `"a,b,c".split(",")` |
| `.replace()` | :white_check_mark: | `"hello".replace("l", "r")` |
| `.padStart()` | :white_check_mark: | `"5".padStart(3, "0")` |

### Control Flow

| Feature | Status | Test |
|---|---|---|
| `if` | :white_check_mark: | `if (true) { ... }` |
| `if`/`else` | :white_check_mark: | `if (x) { ... } else { ... }` |
| `if`/`else if`/`else` | :white_check_mark: | chained conditions |
| `while` | :white_check_mark: | `while (i < 5) { ... }` |
| `for` | :white_check_mark: | `for (let i = 0; i < n; i++)` |
| Nested loops | :white_check_mark: | `for { for { ... } }` |
| Block scoping | :white_check_mark: | `{ let y = 2 }` |
| `break` | :white_check_mark: | `if (i == 5) break` |
| `continue` | :white_check_mark: | `if (i == 2) continue` |
| Ternary `? :` | :white_check_mark: | `true ? 1 : 2` |
| `do...while` | :white_check_mark: | `do { i++ } while (i < 5)` |
| `switch`/`case` | :white_check_mark: | `switch (x) { case 1: ... }` |
| `for...of` | :white_check_mark: | `for (let x of arr)` |
| `for...in` | :white_check_mark: | `for (let key in obj)` |
| Labeled statements | :white_check_mark: | `outer: for (...)` |

### Functions

| Feature | Status | Test |
|---|---|---|
| Declarations | :white_check_mark: | `function add(a, b) { return a + b }` |
| Return values | :white_check_mark: | `return a + b` |
| Multiple params | :white_check_mark: | `function f(a, b, c)` |
| String params/returns | :white_check_mark: | `function hello(name: string): string` |
| Boolean returns | :white_check_mark: | `function isPositive(n): boolean` |
| Recursion | :white_check_mark: | `function fib(n) { ... fib(n-1) }` |
| Mutual calls | :white_check_mark: | `double(double(x))` |
| Local variables | :white_check_mark: | `let result = x * 2` |
| Void functions | :white_check_mark: | `function sayHi(): void` |
| Arrow (expression) | :white_check_mark: | `let add = (a, b) => a + b` |
| Arrow (block body) | :white_check_mark: | `let f = (x) => { return x }` |
| Function hoisting | :white_check_mark: | calling before declaration |
| Closures | :white_check_mark: | capturing outer variables |
| Default parameters | :white_check_mark: | `function f(x = 10)` |
| Rest parameters | :white_check_mark: | `function f(...args: number[])` |
| Spread syntax | :white_check_mark: | `[...arr, 4, 5]` |
| Function expressions | :white_check_mark: | `let f = function() {}` |

### Arrays

| Feature | Status | Test |
|---|---|---|
| Literals | :white_check_mark: | `[1, 2, 3]` |
| Index access | :white_check_mark: | `arr[1]` |
| `.length` | :white_check_mark: | `arr.length` |
| `.push()` | :white_check_mark: | `arr.push(4)` |
| `.pop()` | :white_check_mark: | `arr.pop()` |
| `.map()` | :white_check_mark: | `arr.map(x => x * 2)` |
| `.filter()` | :white_check_mark: | `arr.filter(x => x > 2)` |
| `.reduce()` | :white_check_mark: | `arr.reduce((a, b) => a + b, 0)` |
| `.forEach()` | :white_check_mark: | `arr.forEach(x => console.log(x))` |

### Math Standard Library

| Feature | Status | Test |
|---|---|---|
| `Math.floor()` | :white_check_mark: | `Math.floor(3.7)` |
| `Math.ceil()` | :white_check_mark: | `Math.ceil(3.2)` |
| `Math.round()` | :white_check_mark: | `Math.round(3.5)` |
| `Math.abs()` | :white_check_mark: | `Math.abs(-5)` |
| `Math.sqrt()` | :white_check_mark: | `Math.sqrt(9)` |
| `Math.pow()` | :white_check_mark: | `Math.pow(2, 10)` |
| `Math.min()` / `Math.max()` | :white_check_mark: | `Math.min(3, 7)` |
| `Math.sin()` / `Math.cos()` / `Math.tan()` | :white_check_mark: | trig functions |
| `Math.log()` / `Math.exp()` | :white_check_mark: | `Math.log(Math.E)` |
| `Math.random()` | :white_check_mark: | returns [0, 1) |
| `Math.PI` / `Math.E` | :white_check_mark: | constants |

### Console & Globals

| Feature | Status | Test |
|---|---|---|
| `console.log()` | :white_check_mark: | single and multiple args |
| `console.error()` | :white_check_mark: | writes to stderr |
| `console.warn()` | :white_check_mark: | writes to stderr |
| `typeof` | :white_check_mark: | `typeof 42` = "number" |
| `parseInt()` | :white_check_mark: | `parseInt("42")` |
| `parseFloat()` | :white_check_mark: | `parseFloat("3.14")` |

### Modules

| Feature | Status | Test |
|---|---|---|
| `export function` | :white_check_mark: | `export function square(x) {}` |
| `import { name }` | :white_check_mark: | `import { square } from "./math"` |
| Multiple imports | :white_check_mark: | `import { a, b } from "./utils"` |
| Import aliasing `as` | :white_check_mark: | `import { add as sum }` |
| Default export | :x: | `export default 42` |
| `import * as` | :x: | `import * as math from "./math"` |
| Re-exports | :x: | `export { foo } from "./bar"` |

### Objects & Classes

| Feature | Status | Test |
|---|---|---|
| Object literals | :white_check_mark: | `{ x: 1, y: 2 }` |
| Property access | :white_check_mark: | `obj.x` |
| Bracket access | :white_check_mark: | `obj["x"]` |
| Object methods | :white_check_mark: | `obj.getX()` |
| `console.log(obj)` | :white_check_mark: | `{ name: 'Kian', age: 19 }` |
| Class declarations | :white_check_mark: | `class Point { ... }` |
| `new` + constructor | :white_check_mark: | `new Point(3, 4)` |
| Class methods | :white_check_mark: | `p.toString()` |
| Class inheritance | :white_check_mark: | `class Dog extends Animal` |
| Interfaces | :white_check_mark: | `interface Point { x: number }` |
| Interface inheritance | :white_check_mark: | `interface B extends A` |
| Array destructuring | :white_check_mark: | `let [a, b] = [1, 2]` |
| Object destructuring | :white_check_mark: | `let { x, y } = { x: 1, y: 2 }` |

### Type System

| Feature | Status | Test |
|---|---|---|
| Type annotations | :white_check_mark: | `let x: number`, `function f(): string` |
| Type inference | :white_check_mark: | `let x = 42` (inferred as number) |
| Union types | :white_check_mark: | `string \| number` |
| Type aliases | :white_check_mark: | `type ID = string \| number` |
| Enums (numeric) | :white_check_mark: | `enum Color { Red, Green }` |
| Enums (string) | :white_check_mark: | `enum Direction { Up = "UP" }` |
| Generics | :white_check_mark: | `function identity<T>(x: T): T` |
| Generic constraints | :white_check_mark: | `<T extends { length: number }>` |
| Tuple types | :white_check_mark: | `[number, string]` |
| Type assertions | :white_check_mark: | `x as string` |
| Type narrowing | :white_check_mark: | `if (typeof val === "string")` |
| String literal types | :white_check_mark: | `type Dir = "up" \| "down"` |
| Intersection types | :white_check_mark: | `Named & Aged` |
| `readonly` | :white_check_mark: | `readonly host: string` |
| `keyof` | :white_check_mark: | `keyof Point` |
| Conditional types | :white_check_mark: | `T extends number ? "yes" : "no"` |
| Mapped types | :white_check_mark: | `{ [P in keyof T]: T[P] }` |
| `typeof` in type position | :white_check_mark: | `let y: typeof x` |
| `satisfies` | :white_check_mark: | `"red" satisfies Colors` |
| `as const` | :white_check_mark: | `[1, 2, 3] as const` |

### Error Handling & Async

| Feature | Status | Test |
|---|---|---|
| `try`/`catch` | :x: | `try { throw new Error() } catch (e) {}` |
| `try`/`finally` | :x: | `try { ... } finally { ... }` |
| `async`/`await` | :x: | `async function f() { await ... }` |
| Promises | :x: | `Promise.resolve(42)` |

### Built-in Objects

| Feature | Status | Test |
|---|---|---|
| `JSON.stringify()` | :x: | `JSON.stringify({ a: 1 })` |
| `Map` | :x: | `new Map()` |
| `Set` | :x: | `new Set([1, 2, 3])` |
| `RegExp` | :x: | `/hello/.test("hello world")` |
| `Number.isInteger()` | :white_check_mark: | `Number.isInteger(42)` |
| `Number.isFinite()` | :white_check_mark: | `Number.isFinite(42)` |
| `Number.isNaN()` | :white_check_mark: | `Number.isNaN(NaN)` |
| `.toFixed()` | :white_check_mark: | `(3.14).toFixed(2)` |

### Advanced Features

| Feature | Status | Test |
|---|---|---|
| Namespaces | :x: | `namespace Util { ... }` |
| Decorators | :x: | `@log` |
| Symbols | :x: | `Symbol("foo")` |
| Generators | :x: | `function* range()` |

## License

MIT
