// benchmark.ts — Recursive Fibonacci
// tscc compiles this with integer narrowing, matching native Rust performance.
// Compile: tscc build examples/benchmark.ts
// Run:     tscc run examples/benchmark.ts --benchmark

function fib(n: number): number {
    if (n <= 1) {
        return n
    }
    return fib(n - 1) + fib(n - 2)
}

console.log(fib(50))
