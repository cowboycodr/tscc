# tscc Roadmap

**Current status: 232 tests passing, 16 pending — 93% of test suite**

Features are grouped by implementation effort. Items within each tier are roughly ordered by value/effort ratio.

---

## Tier 1 — Quick Wins

Small, self-contained changes. Each typically touches 1–3 files.

### String Methods
- [x] `.startsWith(s)` — `"hello".startsWith("he")`
- [x] `.endsWith(s)` — `"hello".endsWith("lo")`
- [x] `.repeat(n)` — `"ab".repeat(3)`
- [x] `.padStart(n, s)` — `"5".padStart(3, "0")`
- [x] `.replace(a, b)` — `"hello".replace("l", "r")`
- [ ] `.split(s)` — `"a,b,c".split(",")` *(requires string array support)*

### Number Methods
- [ ] `Number.isInteger(x)` — `Number.isInteger(42)`
- [ ] `Number.isFinite(x)` — `Number.isFinite(42)`
- [ ] `Number.isNaN(x)` — `Number.isNaN(NaN)`
- [ ] `.toFixed(n)` — `(3.14159).toFixed(2)`

### Type-Only Features
These require only parser/checker changes — zero LLVM codegen.
- [x] Type aliases — `type ID = string | number` *(parser + checker; test blocked on union types)*
- [x] Type assertions — `x as string` *(parse and discard; return inner expr)*
- [x] `satisfies` operator — `"red" satisfies Colors` *(type-check only; test blocked on union types)*
- [x] `as const` — `[1, 2, 3] as const` *(treat as identity)*
- [x] `readonly` modifier — `readonly host: string`
- [x] `typeof` in type position — `let y: typeof x`

### Small Language Features
- [ ] Function expressions — `let f = function(x) { return x }` *(same as arrow, different keyword)*
- [ ] `for...in` — `for (let key in obj)` *(iterate object field names as strings)*
- [x] Import aliasing — `import { add as sum }` *(functions only; variable/class imports with aliases not yet handled)*

---

## Tier 2 — Medium Effort

Each requires a new AST node and coordinated changes across parser → checker → codegen.

- [x] **Enums (numeric)** — `enum Color { Red, Green, Blue }`
- [x] **Enums (string)** — `enum Direction { Up = "UP", Down = "DOWN" }`
- [x] **Union types** — `string | number` *(type checker; codegen uses widest type)*
- [ ] **`try`/`catch`** — `try { ... } catch (e) { ... }` *(setjmp-based or LLVM landingpad)*
- [ ] **`try`/`finally`** — `try { ... } finally { ... }`
- [ ] **Function hoisting** — calling a function before its declaration *(pre-scan pass in checker)*
- [ ] **Default exports** — `export default 42` / `import foo from "./foo"`
- [ ] **`import * as`** — `import * as math from "./math"`
- [ ] **Re-exports** — `export { foo } from "./bar"`
- [ ] **Labeled statements** — `outer: for (...) { break outer }`
- [x] **Tuple types** — `[number, string]` *(fixed-length array with typed positions)*
- [ ] **`JSON.stringify()`** — `JSON.stringify({ a: 1 })` *(runtime C function)*
- [x] **Intersection types** — `Named & Aged`
- [x] **String literal types** — `type Dir = "up" | "down"`
- [x] **Type narrowing** — `if (typeof val === "string") { ... }`

---

## Tier 3 — Large Features

Significant design work. Each could be a multi-session effort.

- [x] **Generics** — `function identity<T>(x: T): T` *(monomorphization at call sites)*
- [x] **Generic constraints** — `<T extends { length: number }>`
- [x] **`Map`** — `new Map<string, number>()` *(runtime hash map)*
- [ ] **`Set`** — `new Set([1, 2, 3])` *(runtime hash set)*
- [ ] **`RegExp`** — `/hello/.test("hello world")` *(link against PCRE or re2)*
- [x] **`keyof`** — `keyof Point`
- [x] **Conditional types** — `T extends number ? "yes" : "no"`
- [x] **Mapped types** — `{ [P in keyof T]: T[P] }`

---

## Tier 4 — Deferred / Complex Runtime

These require substantial runtime infrastructure or are lower priority.

- [ ] **`async`/`await`** — needs an event loop or coroutine/fiber support
- [ ] **`Promise`** — `Promise.resolve(42).then(x => x + 1)`
- [ ] **`bigint`** — `9007199254740993n` *(128-bit or arbitrary-precision integers)*
- [ ] **`Symbol`** — `Symbol("foo")`
- [ ] **Decorators** — `@log`
- [ ] **Namespaces** — `namespace Util { ... }`
- [ ] **Iterator protocol** — `Symbol.iterator`, `for...of` on custom iterables
- [ ] **Generators** — `function* range(n)`

---

## Post-1.0 — Self-Hosting

These are long-term aspirations that require tscc to be substantially complete first.

- [ ] **Self-hosting runtime** — Rewrite `runtime/runtime.c` in TypeScript, compiled by tscc itself. Requires tscc to support all language features used by the runtime (string ops, math, I/O via syscalls or libc FFI).
- [ ] **Self-hosting compiler** — Rewrite tscc itself in TypeScript and compile it with tscc. The classic milestone for a mature language implementation.

---

## Known Technical Debt

These are things that currently **compile without error but produce wrong runtime behaviour**. They are not missing features — they are silent incorrectnesses. Fix these before claiming TypeScript compatibility.

### Class field initializers not compiled
`class Foo { x = 5 }` parses the `= 5` but discards it. The field is an uninitialized LLVM struct slot at runtime. `ClassField` has no `initializer` field in the AST.

**Fix:** Add `initializer: Option<Expr>` to `ClassField`. In codegen, emit initializer assignments at the top of the constructor body (before user-written constructor code).

### Unknown generic types silently become `f64`
Any type annotation for an unregistered generic — `Map<K,V>`, `Promise<T>`, `Set<T>`, etc. — hits the fallthrough in `type_ann_to_var_type` (`llvm.rs:5613`) and silently becomes `f64`. Code using these types compiles and produces garbage values.

**Fix:** Emit a hard compile error for unrecognised named/generic types instead of silently falling back.

### `var` is block-scoped, not function-scoped
tscc treats `var` identically to `let`. JavaScript `var` hoists to function scope. Code relying on `var`'s hoisting or cross-block visibility silently behaves differently.

**Fix:** In the parser/codegen, track `var`-declared variables separately and allocate them in the function's entry block rather than the current block.

### `Type::Unknown` is universally assignable
`Type::Unknown` (produced by unresolved type references) is accepted as a valid source and target in every assignability check in `checker.rs`. Type errors involving unknown types flow through silently.

**Fix:** Treat `Type::Unknown` as an error type that poisons any expression it touches, surfacing a diagnostic at the point of first unresolved reference.

### Postfix/prefix `++`/`--` only work on simple identifiers
`x[i]++` and `x.prop++` silently drop the `++` token. The `postfix()` parser only creates an update node for `ExprKind::Identifier`. The token is not consumed, causing it to be misinterpreted as a prefix operator on the next statement, which then fails.

**Fix:** Generalize `PostfixUpdate`/`PrefixUpdate` AST nodes to hold `target: Box<Expr>` instead of `name: String`. Handle `IndexAccess` and `Member` targets in codegen.

---

## Known Bugs

- **Function hoisting** — type checker rejects calls to functions declared later in the file. Fix: two-pass check (pre-scan declarations before checking bodies).
