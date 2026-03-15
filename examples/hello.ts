// Mango example: compiled TypeScript

function add(a: number, b: number): number {
    return a + b;
}

function greet(name: string): string {
    return "Hello, " + name + "!";
}

// Arithmetic
let result: number = add(3, 4);
console.log(result);

let product: number = result * 6;
console.log(product);

// String operations
let message: string = greet("Mango");
console.log(message);

// Control flow
if (result > 5) {
    console.log("result is greater than 5");
} else {
    console.log("result is 5 or less");
}

// While loop
let count: number = 0;
while (count < 5) {
    console.log(count);
    count = count + 1;
}

// For loop
for (let i: number = 0; i < 3; i++) {
    console.log("iteration: " + i);
}

// Boolean expressions
let flag: boolean = true;
console.log(flag);
console.log(!flag);

// Multiple args to console.log
console.log("sum:", add(10, 20));
