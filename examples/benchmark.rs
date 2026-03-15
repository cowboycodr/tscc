fn fib(n: f64) -> f64 {
    if n <= 1.0 {
        return n;
    }
    fib(n - 1.0) + fib(n - 2.0)
}

fn main() {
    let result = fib(40.0);
    println!("{}", result);
}
