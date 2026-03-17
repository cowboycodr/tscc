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

Written in Rust. Single crate. ~15,000 lines of Rust + ~546 lines of C runtime.

The runtime (`runtime/runtime.c`) provides print functions, string operations, math functions, and array support. It is compiled once at `cargo build` time and embedded directly into the `tscc` binary — no C toolchain is required on the user's machine to compile TypeScript files.

## Status

**294 tests passing, 63 pending.** The goal is drop-in compatibility with existing TypeScript projects.

## TypeScript Feature Coverage

**294 passing** / **63 pending** — run `cargo test` to see current counts.

:white_check_mark: = implemented and correct  :warning: = compiles but known-incorrect behavior  :x: = not yet implemented

| Category | Feature | Status | Notes |
|---|---|---|---|
| **Literals** | Integer | :white_check_mark: | `console.log(42)` |
| | Float | :white_check_mark: | `console.log(3.14)` |
| | Negative number | :white_check_mark: | `console.log(-7)` |
| | String (double/single quotes) | :white_check_mark: | `"hello"`, `'world'` |
| | Boolean | :white_check_mark: | `true`, `false` |
| | `null` / `undefined` | :warning: | compile as `0` — print as `0` not `null`/`undefined` |
| | BigInt | :x: | `9007199254740993n` |
| **Variables** | `let` / `const` | :white_check_mark: | `let x = 10`, `const y = 99` |
| | Type annotations | :white_check_mark: | `let x: number = 5` |
| | Reassignment | :white_check_mark: | `x = 2` |
| | Uninitialized `let` | :white_check_mark: | `let x: number` (defaults to 0) |
| | Optional semicolons | :white_check_mark: | `let x = 42` |
| | `var` | :warning: | treated as `let` — no hoisting |
| **Operators** | Arithmetic `+ - * / % **` | :white_check_mark: | `2 + 3 * 4` |
| | Comparison `< > <= >= == != === !==` | :white_check_mark: | `5 === 5` |
| | Logical `&& \|\| !` | :white_check_mark: | `true && !false` |
| | Nullish coalescing `??` | :white_check_mark: | `null ?? 42` |
| | Optional chaining `?.` | :white_check_mark: | `obj?.a` |
| | Compound assignment `+= -= *= /=` | :white_check_mark: | `x += 3` |
| | Postfix `++` / `--` | :white_check_mark: | `x++`, `arr[i]++`, `obj[key]++` |
| | Prefix `++` / `--` | :white_check_mark: | `++x`, `++arr[i]` |
| | Unary negate | :white_check_mark: | `-x` |
| | Ternary `? :` | :white_check_mark: | `x > 0 ? "pos" : "neg"` |
| | `typeof` | :warning: | `typeof null` → `"number"` (should be `"object"`); `typeof function` → `"object"` |
| | Loose equality `==` | :warning: | treated as `===` — no type coercion |
| **Strings** | Concatenation | :white_check_mark: | `"hello" + " " + 42` |
| | Template literals | :white_check_mark: | `` `value is ${x}` `` |
| | `.length` | :white_check_mark: | `"hello".length` |
| | `.toUpperCase()` / `.toLowerCase()` | :white_check_mark: | `"hi".toUpperCase()` |
| | `.trim()` / `.trimStart()` / `.trimEnd()` | :white_check_mark: | `"  hi  ".trim()` |
| | `.includes()` / `.startsWith()` / `.endsWith()` | :white_check_mark: | `"hello".includes("ell")` |
| | `.indexOf()` | :white_check_mark: | `"hello world".indexOf("world")` |
| | `.slice()` / `.substring()` | :white_check_mark: | `"hello".slice(1, 3)` |
| | `.split()` | :white_check_mark: | `"a,b,c".split(",")` |
| | `.replace()` | :white_check_mark: | `"hello".replace("l", "r")` |
| | `.repeat()` | :white_check_mark: | `"ab".repeat(3)` |
| | `.padStart()` / `.padEnd()` | :white_check_mark: | `"5".padStart(3, "0")` |
| | `.charAt()` | :white_check_mark: | `"hello".charAt(1)` |
| | Chained methods | :white_check_mark: | `"  Hello  ".trim().toLowerCase()` |
| **Control Flow** | `if` / `else if` / `else` | :white_check_mark: | `if (x > 0) { ... }` |
| | `while` | :white_check_mark: | `while (i < 5) { ... }` |
| | `for` | :white_check_mark: | `for (let i = 0; i < n; i++)` |
| | `for...of` | :white_check_mark: | `for (const x of arr)` |
| | `for...in` | :white_check_mark: | `for (const key in obj)` |
| | `do...while` | :white_check_mark: | `do { i++ } while (i < 5)` |
| | `switch` / `case` | :white_check_mark: | `switch (x) { case 1: ... }` |
| | `break` / `continue` | :white_check_mark: | `if (i == 5) break` |
| | Labeled statements | :white_check_mark: | `outer: for (...)` |
| | Block scoping | :white_check_mark: | `{ let y = 2 }` |
| **Functions** | Declarations | :white_check_mark: | `function add(a, b) { return a + b }` |
| | Arrow functions | :white_check_mark: | `(a, b) => a + b` |
| | Arrow (block body) | :white_check_mark: | `(x) => { return x * 2 }` |
| | Function expressions | :white_check_mark: | `let f = function() {}` |
| | Default parameters | :white_check_mark: | `function f(x = 10)` |
| | Rest parameters | :white_check_mark: | `function f(...args: number[])` |
| | Closures | :white_check_mark: | capturing outer variables |
| | Recursion | :white_check_mark: | `function fib(n) { ... fib(n-1) }` |
| | Function hoisting | :white_check_mark: | call before declaration |
| **Arrays** | Literals | :white_check_mark: | `[1, 2, 3]` |
| | Index access | :white_check_mark: | `arr[1]` |
| | Spread | :white_check_mark: | `[...arr, 4, 5]` |
| | `.length` | :white_check_mark: | `arr.length` |
| | `.push()` / `.pop()` | :white_check_mark: | `arr.push(4)` |
| | `.map()` / `.filter()` / `.reduce()` | :white_check_mark: | `arr.map(x => x * 2)` |
| | `.forEach()` | :white_check_mark: | `arr.forEach(x => console.log(x))` |
| | String array printing | :warning: | `["a","b"]` prints as garbage IEEE754 numbers |
| | Nested array printing | :warning: | `[[1,2],[3,4]]` prints as garbage pointer values |
| | Empty array printing | :warning: | `[]` prints as `[ ]` with extra spaces |
| **Objects & Classes** | Object literals | :white_check_mark: | `{ x: 1, y: 2 }` |
| | Property / bracket access | :white_check_mark: | `obj.x`, `obj["x"]` |
| | Shorthand properties | :white_check_mark: | `{ x, y }` |
| | Object spread | :white_check_mark: | `{ ...a, ...b }` |
| | Computed property keys | :white_check_mark: | `{ [Status.Todo]: 0 }` |
| | Object methods | :white_check_mark: | `{ greet() { ... } }` |
| | Object destructuring | :white_check_mark: | `let { x, y } = point` |
| | Array destructuring | :white_check_mark: | `let [a, b] = [1, 2]` |
| | Class declarations | :white_check_mark: | `class Point { x: number; ... }` |
| | `new` + constructor | :white_check_mark: | `new Point(3, 4)` |
| | Class methods | :white_check_mark: | `p.toString()` |
| | Class inheritance | :white_check_mark: | `class Dog extends Animal` |
| | Class field initializers | :white_check_mark: | `class Foo { x = 5 }` |
| | Interfaces | :white_check_mark: | `interface Point { x: number }` |
| | Interface inheritance | :white_check_mark: | `interface B extends A` |
| | Class instance printing | :warning: | `console.log(new Point(3,4))` omits class name prefix |
| | Nested object printing | :warning: | `console.log({a:{b:1}})` prints `{ a: [complex] }` |
| | Object/class string field printing | :warning: | string values printed unquoted in objects/classes |
| **Type System** | Type annotations & inference | :white_check_mark: | `let x: number`, `let y = 42` |
| | Union types | :white_check_mark: | `string \| number` |
| | Intersection types | :white_check_mark: | `Named & Aged` |
| | Type aliases | :white_check_mark: | `type ID = string \| number` |
| | Enums (numeric & string) | :white_check_mark: | `enum Dir { Up = "UP" }` |
| | Generics | :white_check_mark: | `function identity<T>(x: T): T` |
| | Generic constraints | :white_check_mark: | `<T extends { length: number }>` |
| | Tuple types | :white_check_mark: | `let t: [number, string]` |
| | Type assertions | :white_check_mark: | `x as string` |
| | Type narrowing | :white_check_mark: | `if (typeof x === "string")` |
| | String literal types | :white_check_mark: | `type Dir = "up" \| "down"` |
| | Boolean literal types | :white_check_mark: | `type T = \| { success: true; data: T }` |
| | `readonly` | :white_check_mark: | `readonly id: string` |
| | `keyof` | :white_check_mark: | `keyof Point` |
| | Conditional types | :white_check_mark: | `T extends number ? "yes" : "no"` |
| | Mapped types | :white_check_mark: | `{ [P in keyof T]: T[P] }` |
| | `typeof` in type position | :white_check_mark: | `let y: typeof x` |
| | Type predicates | :white_check_mark: | `x is SomeType` |
| | `satisfies` | :white_check_mark: | `"red" satisfies Colors` |
| | `as const` | :white_check_mark: | `[1, 2, 3] as const` |
| **Modules** | Named exports / imports | :white_check_mark: | `export function f()`, `import { f }` |
| | Import aliasing | :white_check_mark: | `import { add as sum }` |
| | Default export/import | :x: | `export default 42` |
| | `import * as` | :x: | `import * as math from "./math"` |
| | Re-exports | :x: | `export { foo } from "./bar"` |
| **Error Handling** | `try` / `catch` / `finally` | :white_check_mark: | `try { ... } catch (e) { ... }` |
| | `throw` | :white_check_mark: | `throw "message"` |
| **Async** | `async` / `await` | :white_check_mark: | `async function f() { await g() }` |
| | `Promise` (basic) | :white_check_mark: | async functions return `Promise<T>` |
| | `Promise.resolve()` / `.reject()` | :x: | static methods not yet wired up |
| | `.then()` / `.catch()` | :x: | promise chaining not yet implemented |
| | `setTimeout` | :x: | runtime exists, codegen not wired up |
| **Date** | `new Date()` / `new Date(ms)` | :white_check_mark: | `new Date(0).getTime()` |
| | `Date.now()` | :white_check_mark: | returns ms since epoch |
| | `getFullYear/Month/Date/Hours/...` | :white_check_mark: | local and UTC variants |
| | `toISOString()` | :white_check_mark: | `"2000-01-01T11:30:45.678Z"` |
| **Built-ins** | `console.log()` / `.error()` / `.warn()` | :white_check_mark: | `console.log("hello")` |
| | `parseInt()` / `parseFloat()` | :white_check_mark: | `parseInt("42")` |
| | `Math.*` (floor/ceil/round/abs/max/min/pow/sqrt/log/random/PI) | :white_check_mark: | `Math.floor(1.9)` |
| | `Math.trunc` / `Math.sign` / `Math.log2` / `Math.log10` / `Math.hypot` | :x: | not yet implemented |
| | `Number.isInteger()` / `.isFinite()` / `.isNaN()` | :white_check_mark: | `Number.isInteger(42)` |
| | `.toFixed()` | :white_check_mark: | `(3.14).toFixed(2)` |
| | `Map` | :white_check_mark: | `new Map()`, `.set()`, `.get()`, `.has()`, `.delete()`, `.values()` |
| | `JSON.stringify()` | :x: | `JSON.stringify({ a: 1 })` |
| | `Set` | :x: | `new Set([1, 2, 3])` |
| | `RegExp` | :x: | `/hello/.test("hello world")` |
| **Advanced** | Namespaces | :x: | `namespace Util { ... }` |
| | Decorators | :x: | `@log` |
| | Symbols | :x: | `Symbol("foo")` |
| | Generators | :x: | `function* range()` |

## Parity Test Suite

The parity suite (`mod parity` in `tests/integration.rs`) tracks output correctness against Node.js.
Each test asserts the exact output Node.js produces. Tests marked **ignored** compile successfully today but produce wrong output — they auto-pass once the underlying bug is fixed.

Run the full parity backlog: `cargo test parity -- --ignored`

### Passing parity tests (correct Node.js output confirmed)

| Area | Test | Expected output |
|---|---|---|
| **console.log** | multiple args space-separated | `1 hi true` |
| | number array format | `[ 1, 2, 3 ]` |
| | object with number values | `{ x: 1, y: 2 }` |
| | boolean in console.log | `true` / `false` |
| **typeof** | `typeof 42` | `"number"` |
| | `typeof "hello"` | `"string"` |
| | `typeof true` | `"boolean"` |
| | `typeof {}` | `"object"` |
| | `typeof [1,2,3]` | `"object"` |
| **Number formatting** | negative float `-3.14` | `-3.14` |
| | integer division `10 / 2` | `5` |
| | non-integer division `7 / 2` | `3.5` |
| **String coercions** | `"" + true` | `"true"` |
| | `"" + false` | `"false"` |
| | `"val=" + 42` | `"val=42"` |
| | `"val=" + 3.14` | `"val=3.14"` |
| **Equality** | `1 === 1` | `true` |
| | `"a" === "a"` | `true` |
| | `true === true` | `true` |
| **Math** | `Math.floor(1.9)` | `1` |
| | `Math.floor(-1.5)` | `-2` |
| | `Math.ceil(1.1)` | `2` |
| | `Math.ceil(-1.5)` | `-1` |
| | `Math.round(0.5)` | `1` |
| | `Math.abs(-5)` | `5` |
| | `Math.max(3, 7)` | `7` |
| | `Math.min(3, 7)` | `3` |
| | `Math.pow(2, 10)` | `1024` |
| | `Math.sqrt(9)` | `3` |
| **Arrays** | `[1, 2, 3]` | `[ 1, 2, 3 ]` |
| | `[42]` | `[ 42 ]` |
| | array length after push | `4` |
| **Objects** | `{ x: 1, y: 2 }` | `{ x: 1, y: 2 }` |
| | `{ flag: true }` | `{ flag: true }` |

### Ignored parity tests (compile OK, wrong output — backlog)

| Area | Test | tscc output | Expected |
|---|---|---|---|
| **console.log** | `console.log(null)` | `0` | `null` |
| | `console.log(undefined)` | `0` | `undefined` |
| | `console.log(NaN)` | `nan` | `NaN` |
| | `console.log(Infinity)` | `inf` | `Infinity` |
| | `console.log(-Infinity)` | `-inf` | `-Infinity` |
| | `console.log(-0)` | `0` | `-0` |
| | `console.log([])` | `[  ]` | `[]` |
| | `console.log({})` | `{  }` | `{}` |
| | `console.log(["a","b","c"])` | garbage IEEE754 | `[ 'a', 'b', 'c' ]` |
| | `console.log([[1,2],[3,4]])` | garbage pointers | `[ [ 1, 2 ], [ 3, 4 ] ]` |
| | `console.log({a:{b:1}})` | `{ a: [complex] }` | `{ a: { b: 1 } }` |
| | `console.log({name:"alice"})` | `{ name: alice }` | `{ name: 'alice' }` |
| **Class printing** | `console.log(new Point(3,4))` | `{ x: 3, y: 4 }` | `Point { x: 3, y: 4 }` |
| | class with string field | unquoted value | `Person { name: 'Alice' }` |
| | inherited class instance | no class name | `Dog { name: 'Rex' }` |
| | class with mixed fields | no class name, no quotes | `Item { id: 1, label: 'foo', active: true }` |
| **typeof** | `typeof null` | `"number"` | `"object"` |
| | `typeof undefined` | `"number"` | `"undefined"` |
| | `typeof (() => {})` | `"object"` | `"function"` |
| | `typeof function` | `"object"` | `"function"` |
| **Number formatting** | `0.1 + 0.2` | `0.3` | `0.30000000000000004` |
| | `1 / 0` | `inf` | `Infinity` |
| | `-1 / 0` | `-inf` | `-Infinity` |
| | `NaN === NaN` | `true` | `false` |
| | `9007199254740992` | scientific notation | `9007199254740992` |
| **String coercions** | `"" + null` | `0` | `null` |
| | `"" + undefined` | `0` | `undefined` |
| | `"" + NaN` | `nan` | `NaN` |
| | `"" + Infinity` | `inf` | `Infinity` |
| **Equality** | `NaN === NaN` | `true` | `false` |
| | `null === undefined` | `true` | `false` |
| | `null == undefined` | `false` | `true` |
| | `0 == ""` | `false` | `true` |
| **Math** | `Math.round(-0.5)` | `-1` | `0` |
| | `Math.trunc(1.9)` | compile error / missing | `1` |
| | `Math.trunc(-1.9)` | compile error / missing | `-1` |
| | `Math.sign(5)` | missing | `1` |
| | `Math.sign(-5)` | missing | `-1` |
| | `Math.sign(0)` | missing | `0` |
| | `Math.log2(8)` | missing | `3` |
| | `Math.log10(1000)` | missing | `3` |
| | `Math.hypot(3,4)` | missing | `5` |
| | `Math.clz32(1)` | missing | `31` |
| **Arrays** | `console.log([])` | `[  ]` | `[]` |
| | `console.log(["a","b","c"])` | garbage | `[ 'a', 'b', 'c' ]` |
| | `console.log([[1,2],[3,4]])` | garbage | `[ [ 1, 2 ], [ 3, 4 ] ]` |
| | `console.log([true,false,true])` | unknown | `[ true, false, true ]` |
| **Objects** | `console.log({})` | `{  }` | `{}` |
| | `console.log({name:"hello"})` | unquoted value | `{ name: 'hello' }` |
| | `console.log({a:{b:1}})` | `{ a: [complex] }` | `{ a: { b: 1 } }` |

## License

MIT
