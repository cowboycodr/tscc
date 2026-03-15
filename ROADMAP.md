# tscc Roadmap

**Current status: 190 tests passing, 49 pending — 80% of test suite**

Features are grouped by implementation effort. Items within each tier are roughly ordered by value/effort ratio.

---

## Tier 1 — Quick Wins

Small, self-contained changes. Each typically touches 1–3 files.

### String Methods
- [ ] `.startsWith(s)` — `"hello".startsWith("he")`
- [ ] `.endsWith(s)` — `"hello".endsWith("lo")`
- [ ] `.repeat(n)` — `"ab".repeat(3)`
- [ ] `.padStart(n, s)` — `"5".padStart(3, "0")`
- [ ] `.replace(a, b)` — `"hello".replace("l", "r")`
- [ ] `.split(s)` — `"a,b,c".split(",")` *(returns array — slightly more involved)*

### Number Methods
- [ ] `Number.isInteger(x)` — `Number.isInteger(42)`
- [ ] `Number.isFinite(x)` — `Number.isFinite(42)`
- [ ] `Number.isNaN(x)` — `Number.isNaN(NaN)`
- [ ] `.toFixed(n)` — `(3.14159).toFixed(2)`

### Type-Only Features
These require only parser/checker changes — zero LLVM codegen.
- [ ] Type aliases — `type ID = string | number`
- [ ] Type assertions — `x as string` *(parse and discard; return inner expr)*
- [ ] `satisfies` operator — `"red" satisfies Colors` *(type-check only)*
- [ ] `as const` — `[1, 2, 3] as const` *(treat as identity)*
- [ ] `readonly` modifier — `readonly host: string`
- [ ] `typeof` in type position — `let y: typeof x`

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

## Known Bugs

- **Import aliasing codegen** — `import { add as sum }` maps alias in checker but codegen looks up original name. Fix: thread alias→original map into codegen.
- **Function hoisting** — type checker rejects calls to functions declared later in the file. Fix: two-pass check (pre-scan declarations before checking bodies).
