// hello.ts — Your first Mango program
// Compile: mango build examples/hello.ts
// Run:     mango run examples/hello.ts

function greet(name: string): string {
    return `Hello, ${name}!`
}

let message = greet("World")
console.log(message)

// Arrays
let numbers = [10, 20, 30, 40, 50]
console.log("Numbers:", numbers)
console.log("Third element:", numbers[2])
console.log("Length:", numbers.length)

// Control flow
for (let i = 0; i < 5; i++) {
    if (i % 2 === 0) {
        console.log(i, "is even")
    }
}

// Arrow functions
let double = (x: number): number => x * 2
console.log("double(21) =", double(21))
