// stdlib.ts — Standard library showcase
// Compile: mango run examples/stdlib.ts

// Math
console.log("sqrt(144) =", Math.sqrt(144))
console.log("2 ** 10 =", 2 ** 10)
console.log("PI =", Math.PI)

// Strings
let s = "Hello, World!"
console.log("length:", s.length)
console.log("upper:", s.toUpperCase())
console.log("slice(0,5):", s.slice(0, 5))
console.log("includes('World'):", s.includes("World"))
console.log("indexOf('World'):", s.indexOf("World"))
console.log("trim:", "  spaces  ".trim())

// Template literals
let name = "Mango"
let version = 0.5
console.log(`${name} v${version} is fast!`)

// Ternary + typeof
let x = 42
console.log(typeof x)
console.log(x > 0 ? "positive" : "non-positive")

// parseInt / parseFloat
console.log("parseInt('42') =", parseInt("42"))
console.log("parseFloat('3.14') =", parseFloat("3.14"))

// Arrays with push/pop
let arr = [1, 2, 3]
arr.push(4)
let last = arr.pop()
console.log("popped:", last)
console.log("remaining:", arr)
