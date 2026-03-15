# tscc Roadmap

**Current status: 199 tests passing, 40 pending — 83% of test suite**

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
- [ ] Import aliasing fix — `import { add as sum }` *(known codegen bug — 1-line fix)*

---

## Tier 2 — Medium Effort

Each requires a new AST node and coordinated changes across parser → checker → codegen.

- [ ] **Enums (numeric)** — `enum Color { Red, Green, Blue }`
- [ ] **Enums (string)** — `enum Direction { Up = "UP", Down = "DOWN" }`
- [ ] **Union types** — `string | number` *(type checker; codegen uses widest type)*
- [ ] **`try`/`catch`** — `try { ... } catch (e) { ... }` *(setjmp-based or LLVM landingpad)*
- [ ] **`try`/`finally`** — `try { ... } finally { ... }`
- [ ] **Function hoisting** — calling a function before its declaration *(pre-scan pass in checker)*
- [ ] **Default exports** — `export default 42` / `import foo from "./foo"`
- [ ] **`import * as`** — `import * as math from "./math"`
- [ ] **Re-exports** — `export { foo } from "./bar"`
- [ ] **Labeled statements** — `outer: for (...) { break outer }`
- [ ] **Tuple types** — `[number, string]` *(fixed-length array with typed positions)*
- [ ] **`JSON.stringify()`** — `JSON.stringify({ a: 1 })` *(runtime C function)*
- [ ] **Intersection types** — `Named & Aged`
- [ ] **String literal types** — `type Dir = "up" | "down"`
- [ ] **Type narrowing** — `if (typeof val === "string") { ... }`

---

## Tier 3 — Large Features

Significant design work. Each could be a multi-session effort.

- [ ] **Generics** — `function identity<T>(x: T): T` *(type parameter substitution throughout pipeline)*
- [ ] **Generic constraints** — `<T extends { length: number }>`
- [ ] **`Map`** — `new Map<string, number>()` *(runtime hash map)*
- [ ] **`Set`** — `new Set([1, 2, 3])` *(runtime hash set)*
- [ ] **`RegExp`** — `/hello/.test("hello world")` *(link against PCRE or re2)*
- [ ] **`keyof`** — `keyof Point`
- [ ] **Conditional types** — `T extends number ? "yes" : "no"`
- [ ] **Mapped types** — `{ [P in keyof T]: T[P] }`

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

## Known Bugs

- **Import aliasing codegen** — `import { add as sum }` maps alias in checker but codegen looks up original name. Fix: thread alias→original map into codegen.
- **Function hoisting** — type checker rejects calls to functions declared later in the file. Fix: two-pass check (pre-scan declarations before checking bodies).
