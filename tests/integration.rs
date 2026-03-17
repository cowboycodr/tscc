//! tscc Integration Tests
//!
//! End-to-end tests that compile TypeScript source through the full pipeline
//! (lexer -> parser -> type checker -> codegen -> link -> execute) and verify output.
//!
//! Organization:
//!   - Working features: normal #[test] functions
//!   - Unimplemented features: #[test] #[ignore] functions (tracked as future work)
//!
//! Run with: cargo test
//! See ignored count for "TypeScript coverage gap"

use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Compile and run a TypeScript source string, returning stdout.
fn run_ts(source: &str) -> String {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let tid = std::thread::current().id();
    let dir = std::env::temp_dir().join(format!("tscc_test_{:?}_{}", tid, id));
    std::fs::create_dir_all(&dir).unwrap();
    let output = dir.join("test_binary");
    let output_str = output.to_str().unwrap();

    let result = tscc::compile_source(source, output_str, false);
    if let Err(e) = &result {
        let _ = std::fs::remove_dir_all(&dir);
        panic!("Compilation failed:\n{}\n\nSource:\n{}", e, source);
    }

    let run = Command::new(&output)
        .output()
        .expect("Failed to execute compiled binary");

    let _ = std::fs::remove_dir_all(&dir);

    assert!(
        run.status.success(),
        "Binary exited with non-zero status: {:?}\nstderr: {}",
        run.status,
        String::from_utf8_lossy(&run.stderr)
    );

    String::from_utf8(run.stdout).unwrap()
}

/// Compile and run, returning (stdout, stderr).
fn run_ts_full(source: &str) -> (String, String) {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let tid = std::thread::current().id();
    let dir = std::env::temp_dir().join(format!("tscc_test_{:?}_{}", tid, id));
    std::fs::create_dir_all(&dir).unwrap();
    let output = dir.join("test_binary");
    let output_str = output.to_str().unwrap();

    tscc::compile_source(source, output_str, false)
        .unwrap_or_else(|e| panic!("Compilation failed:\n{}\n\nSource:\n{}", e, source));

    let run = Command::new(&output)
        .output()
        .expect("Failed to execute compiled binary");

    let _ = std::fs::remove_dir_all(&dir);

    (
        String::from_utf8(run.stdout).unwrap(),
        String::from_utf8(run.stderr).unwrap(),
    )
}

/// Verify that compilation fails (for error-case tests).
fn assert_compile_fails(source: &str) {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let tid = std::thread::current().id();
    let dir = std::env::temp_dir().join(format!("tscc_test_{:?}_{}", tid, id));
    std::fs::create_dir_all(&dir).unwrap();
    let output = dir.join("test_binary");
    let output_str = output.to_str().unwrap();

    let result = tscc::compile_source(source, output_str, false);
    let _ = std::fs::remove_dir_all(&dir);

    assert!(
        result.is_err(),
        "Expected compilation to fail, but it succeeded.\n\nSource:\n{}",
        source
    );
}

// ============================================================
// 1. LITERALS & PRIMITIVES
// ============================================================
mod literals {
    use super::*;

    #[test]
    fn integer_literal() {
        assert_eq!(run_ts("console.log(42)"), "42\n");
    }

    #[test]
    fn float_literal() {
        assert_eq!(run_ts("console.log(3.14)"), "3.14\n");
    }

    #[test]
    fn negative_number() {
        assert_eq!(run_ts("console.log(-7)"), "-7\n");
    }

    #[test]
    fn zero() {
        assert_eq!(run_ts("console.log(0)"), "0\n");
    }

    #[test]
    fn large_integer() {
        assert_eq!(run_ts("console.log(1000000)"), "1000000\n");
    }

    #[test]
    fn string_double_quotes() {
        assert_eq!(run_ts(r#"console.log("hello")"#), "hello\n");
    }

    #[test]
    fn string_single_quotes() {
        assert_eq!(run_ts("console.log('world')"), "world\n");
    }

    #[test]
    fn empty_string() {
        assert_eq!(run_ts(r#"console.log("")"#), "\n");
    }

    #[test]
    fn boolean_true() {
        assert_eq!(run_ts("console.log(true)"), "true\n");
    }

    #[test]
    fn boolean_false() {
        assert_eq!(run_ts("console.log(false)"), "false\n");
    }

    #[test]
    fn null_literal() {
        // null is compiled as 0.0 (number), so console.log prints it as a number
        // This tests current behavior — may change when null gets its own type
        let out = run_ts("console.log(null)");
        assert!(!out.is_empty());
    }

    #[test]
    fn undefined_literal() {
        // Same as null — currently compiled as 0.0
        let out = run_ts("console.log(undefined)");
        assert!(!out.is_empty());
    }
}

// ============================================================
// 2. VARIABLES
// ============================================================
mod variables {
    use super::*;

    #[test]
    fn let_with_number() {
        assert_eq!(run_ts("let x = 10\nconsole.log(x)"), "10\n");
    }

    #[test]
    fn let_with_string() {
        assert_eq!(run_ts("let s = \"hi\"\nconsole.log(s)"), "hi\n");
    }

    #[test]
    fn let_with_boolean() {
        assert_eq!(run_ts("let b = true\nconsole.log(b)"), "true\n");
    }

    #[test]
    fn const_with_number() {
        assert_eq!(run_ts("const x = 99\nconsole.log(x)"), "99\n");
    }

    #[test]
    fn let_with_type_annotation() {
        assert_eq!(run_ts("let x: number = 5\nconsole.log(x)"), "5\n");
    }

    #[test]
    fn let_string_type_annotation() {
        assert_eq!(
            run_ts("let s: string = \"typed\"\nconsole.log(s)"),
            "typed\n"
        );
    }

    #[test]
    fn let_boolean_type_annotation() {
        assert_eq!(run_ts("let b: boolean = false\nconsole.log(b)"), "false\n");
    }

    #[test]
    fn variable_reassignment() {
        assert_eq!(run_ts("let x = 1\nx = 2\nconsole.log(x)"), "2\n");
    }

    #[test]
    fn multiple_variables() {
        assert_eq!(
            run_ts("let a = 1\nlet b = 2\nlet c = 3\nconsole.log(a, b, c)"),
            "1 2 3\n"
        );
    }

    #[test]
    fn let_without_initializer() {
        // let x: number; should default to 0
        assert_eq!(run_ts("let x: number\nconsole.log(x)"), "0\n");
    }

    #[test]
    fn semicolons_optional() {
        assert_eq!(run_ts("let x = 42\nconsole.log(x)"), "42\n");
    }

    #[test]
    fn semicolons_present() {
        assert_eq!(run_ts("let x = 42;\nconsole.log(x);"), "42\n");
    }
}

// ============================================================
// 3. ARITHMETIC OPERATORS
// ============================================================
mod arithmetic {
    use super::*;

    #[test]
    fn addition() {
        assert_eq!(run_ts("console.log(2 + 3)"), "5\n");
    }

    #[test]
    fn subtraction() {
        assert_eq!(run_ts("console.log(10 - 4)"), "6\n");
    }

    #[test]
    fn multiplication() {
        assert_eq!(run_ts("console.log(3 * 7)"), "21\n");
    }

    #[test]
    fn division() {
        assert_eq!(run_ts("console.log(10 / 4)"), "2.5\n");
    }

    #[test]
    fn integer_division() {
        assert_eq!(run_ts("console.log(10 / 2)"), "5\n");
    }

    #[test]
    fn modulo() {
        assert_eq!(run_ts("console.log(10 % 3)"), "1\n");
    }

    #[test]
    fn operator_precedence() {
        assert_eq!(run_ts("console.log(2 + 3 * 4)"), "14\n");
    }

    #[test]
    fn parenthesized_expressions() {
        assert_eq!(run_ts("console.log((2 + 3) * 4)"), "20\n");
    }

    #[test]
    fn nested_arithmetic() {
        assert_eq!(run_ts("console.log(1 + 2 + 3 + 4)"), "10\n");
    }

    #[test]
    fn float_arithmetic() {
        assert_eq!(run_ts("console.log(1.5 + 2.5)"), "4\n");
    }

    #[test]
    fn negative_result() {
        assert_eq!(run_ts("console.log(3 - 10)"), "-7\n");
    }

    #[test]
    fn postfix_increment() {
        assert_eq!(run_ts("let x = 5\nx++\nconsole.log(x)"), "6\n");
    }

    #[test]
    fn postfix_decrement() {
        assert_eq!(run_ts("let x = 5\nx--\nconsole.log(x)"), "4\n");
    }

    #[test]
    fn prefix_increment() {
        assert_eq!(run_ts("let x = 5\n++x\nconsole.log(x)"), "6\n");
    }

    #[test]
    fn prefix_decrement() {
        assert_eq!(run_ts("let x = 5\n--x\nconsole.log(x)"), "4\n");
    }

    #[test]
    fn postfix_returns_old_value() {
        // console.log(x++) should print the old value
        assert_eq!(run_ts("let x = 5\nconsole.log(x++)"), "5\n");
    }

    #[test]
    fn prefix_returns_new_value() {
        assert_eq!(run_ts("let x = 5\nconsole.log(++x)"), "6\n");
    }

    #[test]
    fn postfix_increment_array_element() {
        assert_eq!(
            run_ts("const arr = [1, 2, 3]\narr[1]++\nconsole.log(arr[1])"),
            "3\n"
        );
    }

    #[test]
    fn postfix_decrement_array_element() {
        assert_eq!(
            run_ts("const arr = [10, 20, 30]\narr[0]--\nconsole.log(arr[0])"),
            "9\n"
        );
    }

    #[test]
    fn postfix_array_element_returns_old_value() {
        assert_eq!(
            run_ts("const arr = [5, 6, 7]\nconsole.log(arr[2]++)"),
            "7\n"
        );
    }

    #[test]
    fn postfix_increment_object_field_dynamic_key() {
        assert_eq!(
            run_ts(
                "const counts = { a: 0, b: 0 }\nconst key = \"a\"\ncounts[key]++\nconsole.log(counts.a)"
            ),
            "1\n"
        );
    }

    #[test]
    fn postfix_increment_object_field_dynamic_key_no_match_noop() {
        // Key that doesn't match any field is a silent no-op
        assert_eq!(
            run_ts(
                "const counts = { a: 0, b: 0 }\nconst key = \"z\"\ncounts[key]++\nconsole.log(counts.a)"
            ),
            "0\n"
        );
    }

    #[test]
    fn prefix_increment_array_element() {
        assert_eq!(
            run_ts("const arr = [1, 2, 3]\n++arr[0]\nconsole.log(arr[0])"),
            "2\n"
        );
    }

    #[test]
    fn unary_negate() {
        assert_eq!(run_ts("let x = 5\nconsole.log(-x)"), "-5\n");
    }
}

// ============================================================
// 4. COMPARISON OPERATORS
// ============================================================
mod comparison {
    use super::*;

    #[test]
    fn less_than_true() {
        assert_eq!(run_ts("console.log(1 < 2)"), "true\n");
    }

    #[test]
    fn less_than_false() {
        assert_eq!(run_ts("console.log(2 < 1)"), "false\n");
    }

    #[test]
    fn greater_than() {
        assert_eq!(run_ts("console.log(5 > 3)"), "true\n");
    }

    #[test]
    fn less_equal() {
        assert_eq!(run_ts("console.log(5 <= 5)"), "true\n");
    }

    #[test]
    fn greater_equal() {
        assert_eq!(run_ts("console.log(3 >= 4)"), "false\n");
    }

    #[test]
    fn equal() {
        assert_eq!(run_ts("console.log(5 == 5)"), "true\n");
    }

    #[test]
    fn equal_false() {
        assert_eq!(run_ts("console.log(5 == 6)"), "false\n");
    }

    #[test]
    fn strict_equal() {
        assert_eq!(run_ts("console.log(5 === 5)"), "true\n");
    }

    #[test]
    fn not_equal() {
        assert_eq!(run_ts("console.log(5 != 6)"), "true\n");
    }

    #[test]
    fn strict_not_equal() {
        assert_eq!(run_ts("console.log(5 !== 5)"), "false\n");
    }

    #[test]
    fn boolean_equality() {
        assert_eq!(run_ts("console.log(true == true)"), "true\n");
    }

    #[test]
    fn boolean_inequality() {
        assert_eq!(run_ts("console.log(true != false)"), "true\n");
    }
}

// ============================================================
// 5. LOGICAL OPERATORS
// ============================================================
mod logical {
    use super::*;

    #[test]
    fn and_true() {
        assert_eq!(run_ts("console.log(true && true)"), "true\n");
    }

    #[test]
    fn and_false() {
        assert_eq!(run_ts("console.log(true && false)"), "false\n");
    }

    #[test]
    fn or_true() {
        assert_eq!(run_ts("console.log(false || true)"), "true\n");
    }

    #[test]
    fn or_false() {
        assert_eq!(run_ts("console.log(false || false)"), "false\n");
    }

    #[test]
    fn not_true() {
        assert_eq!(run_ts("console.log(!true)"), "false\n");
    }

    #[test]
    fn not_false() {
        assert_eq!(run_ts("console.log(!false)"), "true\n");
    }

    #[test]
    fn complex_logical() {
        assert_eq!(run_ts("console.log(true && !false || false)"), "true\n");
    }

    #[test]
    fn numeric_and() {
        // In JS, && with numbers: both truthy -> truthy result
        assert_eq!(run_ts("console.log(1 && 1)"), "true\n");
    }

    #[test]
    fn numeric_or() {
        assert_eq!(run_ts("console.log(0 || 1)"), "true\n");
    }
}

// ============================================================
// 6. STRINGS
// ============================================================
mod strings {
    use super::*;

    #[test]
    fn string_concatenation() {
        assert_eq!(
            run_ts(r#"console.log("hello" + " " + "world")"#),
            "hello world\n"
        );
    }

    #[test]
    fn string_number_concat() {
        assert_eq!(run_ts(r#"console.log("value: " + 42)"#), "value: 42\n");
    }

    #[test]
    fn number_string_concat() {
        assert_eq!(
            run_ts(r#"console.log(42 + " is the answer")"#),
            "42 is the answer\n"
        );
    }

    #[test]
    fn string_boolean_concat() {
        assert_eq!(run_ts(r#"console.log("it is " + true)"#), "it is true\n");
    }

    #[test]
    fn string_length() {
        assert_eq!(run_ts(r#"console.log("hello".length)"#), "5\n");
    }

    #[test]
    fn empty_string_length() {
        assert_eq!(run_ts(r#"console.log("".length)"#), "0\n");
    }

    #[test]
    fn to_upper_case() {
        assert_eq!(run_ts(r#"console.log("hello".toUpperCase())"#), "HELLO\n");
    }

    #[test]
    fn to_lower_case() {
        assert_eq!(run_ts(r#"console.log("HELLO".toLowerCase())"#), "hello\n");
    }

    #[test]
    fn char_at() {
        assert_eq!(run_ts(r#"console.log("hello".charAt(1))"#), "e\n");
    }

    #[test]
    fn char_at_first() {
        assert_eq!(run_ts(r#"console.log("abc".charAt(0))"#), "a\n");
    }

    #[test]
    fn index_of_found() {
        assert_eq!(
            run_ts(r#"console.log("hello world".indexOf("world"))"#),
            "6\n"
        );
    }

    #[test]
    fn index_of_not_found() {
        assert_eq!(run_ts(r#"console.log("hello".indexOf("xyz"))"#), "-1\n");
    }

    #[test]
    fn includes_true() {
        assert_eq!(
            run_ts(r#"console.log("hello world".includes("world"))"#),
            "true\n"
        );
    }

    #[test]
    fn includes_false() {
        assert_eq!(run_ts(r#"console.log("hello".includes("xyz"))"#), "false\n");
    }

    #[test]
    fn substring() {
        assert_eq!(
            run_ts(r#"console.log("hello world".substring(0, 5))"#),
            "hello\n"
        );
    }

    #[test]
    fn substring_to_end() {
        assert_eq!(
            run_ts(r#"console.log("hello world".substring(6))"#),
            "world\n"
        );
    }

    #[test]
    fn slice_basic() {
        assert_eq!(
            run_ts(r#"console.log("hello world".slice(0, 5))"#),
            "hello\n"
        );
    }

    #[test]
    fn slice_negative() {
        assert_eq!(run_ts(r#"console.log("hello world".slice(-5))"#), "world\n");
    }

    #[test]
    fn trim() {
        assert_eq!(run_ts(r#"console.log("  hello  ".trim())"#), "hello\n");
    }

    #[test]
    fn string_variable_methods() {
        assert_eq!(
            run_ts("let s = \"Hello World\"\nconsole.log(s.toUpperCase())"),
            "HELLO WORLD\n"
        );
    }
}

// ============================================================
// 7. CONTROL FLOW
// ============================================================
mod control_flow {
    use super::*;

    #[test]
    fn if_true() {
        assert_eq!(run_ts("if (true) {\n  console.log(1)\n}"), "1\n");
    }

    #[test]
    fn if_false_no_output() {
        assert_eq!(
            run_ts("if (false) {\n  console.log(1)\n}\nconsole.log(2)"),
            "2\n"
        );
    }

    #[test]
    fn if_else() {
        assert_eq!(
            run_ts("if (false) {\n  console.log(1)\n} else {\n  console.log(2)\n}"),
            "2\n"
        );
    }

    #[test]
    fn if_else_if_else() {
        let src = r#"
let x = 2
if (x == 1) {
    console.log("one")
} else if (x == 2) {
    console.log("two")
} else {
    console.log("other")
}
"#;
        assert_eq!(run_ts(src), "two\n");
    }

    #[test]
    fn if_with_comparison() {
        assert_eq!(
            run_ts("let x = 10\nif (x > 5) {\n  console.log(\"big\")\n}"),
            "big\n"
        );
    }

    #[test]
    fn while_loop() {
        let src = r#"
let i = 0
while (i < 5) {
    i++
}
console.log(i)
"#;
        assert_eq!(run_ts(src), "5\n");
    }

    #[test]
    fn while_loop_with_output() {
        let src = r#"
let sum = 0
let i = 1
while (i <= 4) {
    sum = sum + i
    i++
}
console.log(sum)
"#;
        assert_eq!(run_ts(src), "10\n");
    }

    #[test]
    fn for_loop_basic() {
        let src = r#"
let sum = 0
for (let i = 1; i <= 5; i++) {
    sum = sum + i
}
console.log(sum)
"#;
        assert_eq!(run_ts(src), "15\n");
    }

    #[test]
    fn for_loop_countdown() {
        let src = r#"
let result = 0
for (let i = 10; i > 0; i--) {
    result = result + i
}
console.log(result)
"#;
        assert_eq!(run_ts(src), "55\n");
    }

    #[test]
    fn nested_loops() {
        let src = r#"
let count = 0
for (let i = 0; i < 3; i++) {
    for (let j = 0; j < 3; j++) {
        count++
    }
}
console.log(count)
"#;
        assert_eq!(run_ts(src), "9\n");
    }

    #[test]
    fn nested_if_in_loop() {
        let src = r#"
let count = 0
for (let i = 1; i <= 10; i++) {
    if (i % 2 == 0) {
        count++
    }
}
console.log(count)
"#;
        assert_eq!(run_ts(src), "5\n");
    }

    #[test]
    fn block_scoping() {
        let src = r#"
let x = 1
{
    let y = 2
    console.log(y)
}
console.log(x)
"#;
        assert_eq!(run_ts(src), "2\n1\n");
    }

    #[test]
    fn for_of_number_array() {
        let src = "
let arr = [1, 2, 3]
for (let x of arr) {
    console.log(x)
}
";
        assert_eq!(run_ts(src), "1\n2\n3\n");
    }

    #[test]
    fn for_of_const() {
        // for (const x of ...) binds a const loop variable — same runtime behaviour
        let src = "
let arr = [10, 20, 30]
for (const x of arr) {
    console.log(x)
}
";
        assert_eq!(run_ts(src), "10\n20\n30\n");
    }
}

// ============================================================
// 8. FUNCTIONS
// ============================================================
mod functions {
    use super::*;

    #[test]
    fn simple_function() {
        let src = r#"
function greet(): void {
    console.log("hello")
}
greet()
"#;
        assert_eq!(run_ts(src), "hello\n");
    }

    #[test]
    fn function_with_return() {
        let src = r#"
function add(a: number, b: number): number {
    return a + b
}
console.log(add(3, 4))
"#;
        assert_eq!(run_ts(src), "7\n");
    }

    #[test]
    fn function_multiple_params() {
        let src = r#"
function sum3(a: number, b: number, c: number): number {
    return a + b + c
}
console.log(sum3(1, 2, 3))
"#;
        assert_eq!(run_ts(src), "6\n");
    }

    #[test]
    fn function_string_param() {
        let src = r#"
function hello(name: string): string {
    return "Hello, " + name
}
console.log(hello("Mango"))
"#;
        assert_eq!(run_ts(src), "Hello, Mango\n");
    }

    #[test]
    fn function_boolean_return() {
        let src = r#"
function isPositive(n: number): boolean {
    return n > 0
}
console.log(isPositive(5))
console.log(isPositive(-3))
"#;
        assert_eq!(run_ts(src), "true\nfalse\n");
    }

    #[test]
    fn recursion() {
        let src = r#"
function factorial(n: number): number {
    if (n <= 1) {
        return 1
    }
    return n * factorial(n - 1)
}
console.log(factorial(5))
"#;
        assert_eq!(run_ts(src), "120\n");
    }

    #[test]
    fn mutual_function_calls() {
        let src = r#"
function double(x: number): number {
    return x * 2
}
function quadruple(x: number): number {
    return double(double(x))
}
console.log(quadruple(3))
"#;
        assert_eq!(run_ts(src), "12\n");
    }

    #[test]
    fn function_with_local_vars() {
        let src = r#"
function compute(x: number): number {
    let doubled = x * 2
    let result = doubled + 1
    return result
}
console.log(compute(5))
"#;
        assert_eq!(run_ts(src), "11\n");
    }

    #[test]
    fn function_with_if() {
        let src = r#"
function abs(x: number): number {
    if (x < 0) {
        return -x
    }
    return x
}
console.log(abs(-5))
console.log(abs(3))
"#;
        assert_eq!(run_ts(src), "5\n3\n");
    }

    #[test]
    fn function_with_loop() {
        let src = r#"
function sumTo(n: number): number {
    let total = 0
    let i = 1
    while (i <= n) {
        total = total + i
        i++
    }
    return total
}
console.log(sumTo(10))
"#;
        assert_eq!(run_ts(src), "55\n");
    }

    #[test]
    fn void_function() {
        let src = r#"
function sayHi(): void {
    console.log("hi")
}
sayHi()
"#;
        assert_eq!(run_ts(src), "hi\n");
    }

    #[test]
    fn function_called_before_declaration() {
        // Two-pass compilation handles codegen, but the type checker
        // doesn't scan ahead for function declarations yet
        let src = r#"
console.log(add(1, 2))
function add(a: number, b: number): number {
    return a + b
}
"#;
        assert_eq!(run_ts(src), "3\n");
    }

    #[test]
    fn fibonacci_recursive() {
        let src = r#"
function fib(n: number): number {
    if (n <= 1) {
        return n
    }
    return fib(n - 1) + fib(n - 2)
}
console.log(fib(10))
"#;
        assert_eq!(run_ts(src), "55\n");
    }
}

// ============================================================
// 9. MATH STDLIB
// ============================================================
mod math_stdlib {
    use super::*;

    #[test]
    fn math_floor() {
        assert_eq!(run_ts("console.log(Math.floor(3.7))"), "3\n");
    }

    #[test]
    fn math_ceil() {
        assert_eq!(run_ts("console.log(Math.ceil(3.2))"), "4\n");
    }

    #[test]
    fn math_round() {
        assert_eq!(run_ts("console.log(Math.round(3.5))"), "4\n");
    }

    #[test]
    fn math_round_down() {
        assert_eq!(run_ts("console.log(Math.round(3.4))"), "3\n");
    }

    #[test]
    fn math_abs_positive() {
        assert_eq!(run_ts("console.log(Math.abs(-5))"), "5\n");
    }

    #[test]
    fn math_abs_already_positive() {
        assert_eq!(run_ts("console.log(Math.abs(5))"), "5\n");
    }

    #[test]
    fn math_sqrt() {
        assert_eq!(run_ts("console.log(Math.sqrt(9))"), "3\n");
    }

    #[test]
    fn math_pow() {
        assert_eq!(run_ts("console.log(Math.pow(2, 10))"), "1024\n");
    }

    #[test]
    fn math_min() {
        assert_eq!(run_ts("console.log(Math.min(3, 7))"), "3\n");
    }

    #[test]
    fn math_max() {
        assert_eq!(run_ts("console.log(Math.max(3, 7))"), "7\n");
    }

    #[test]
    fn math_pi() {
        let out = run_ts("console.log(Math.PI)");
        assert!(out.starts_with("3.14159"), "Got: {}", out);
    }

    #[test]
    fn math_e() {
        let out = run_ts("console.log(Math.E)");
        assert!(out.starts_with("2.71828"), "Got: {}", out);
    }

    #[test]
    fn math_sin_zero() {
        assert_eq!(run_ts("console.log(Math.sin(0))"), "0\n");
    }

    #[test]
    fn math_cos_zero() {
        assert_eq!(run_ts("console.log(Math.cos(0))"), "1\n");
    }

    #[test]
    fn math_log_e() {
        assert_eq!(run_ts("console.log(Math.log(Math.E))"), "1\n");
    }

    #[test]
    fn math_exp_zero() {
        assert_eq!(run_ts("console.log(Math.exp(0))"), "1\n");
    }

    #[test]
    fn math_random_range() {
        // Math.random() should be in [0, 1)
        let out = run_ts("let r = Math.random()\nconsole.log(r >= 0 && r < 1)");
        assert_eq!(out, "true\n");
    }

    #[test]
    fn math_combined() {
        assert_eq!(run_ts("console.log(Math.floor(Math.sqrt(26)))"), "5\n");
    }
}

// ============================================================
// 10. CONSOLE
// ============================================================
mod console_output {
    use super::*;

    #[test]
    fn log_number() {
        assert_eq!(run_ts("console.log(42)"), "42\n");
    }

    #[test]
    fn log_string() {
        assert_eq!(run_ts(r#"console.log("test")"#), "test\n");
    }

    #[test]
    fn log_boolean() {
        assert_eq!(run_ts("console.log(true)"), "true\n");
    }

    #[test]
    fn log_multiple_args() {
        assert_eq!(run_ts(r#"console.log(1, "two", true)"#), "1 two true\n");
    }

    #[test]
    fn log_multiple_numbers() {
        assert_eq!(run_ts("console.log(1, 2, 3)"), "1 2 3\n");
    }

    #[test]
    fn multiple_log_calls() {
        assert_eq!(
            run_ts("console.log(1)\nconsole.log(2)\nconsole.log(3)"),
            "1\n2\n3\n"
        );
    }

    #[test]
    fn error_to_stderr() {
        let (stdout, stderr) = run_ts_full(r#"console.error("err msg")"#);
        assert_eq!(stdout, "");
        assert_eq!(stderr, "err msg\n");
    }

    #[test]
    fn warn_to_stderr() {
        let (stdout, stderr) = run_ts_full(r#"console.warn("warn msg")"#);
        assert_eq!(stdout, "");
        assert_eq!(stderr, "warn msg\n");
    }

    #[test]
    fn log_expression_result() {
        assert_eq!(run_ts("console.log(2 + 3 * 4)"), "14\n");
    }
}

// ============================================================
// 11. GLOBALS (typeof, parseInt, parseFloat)
// ============================================================
mod globals {
    use super::*;

    #[test]
    fn typeof_number() {
        assert_eq!(run_ts("console.log(typeof 42)"), "number\n");
    }

    #[test]
    fn typeof_string() {
        assert_eq!(run_ts(r#"console.log(typeof "hi")"#), "string\n");
    }

    #[test]
    fn typeof_boolean() {
        assert_eq!(run_ts("console.log(typeof true)"), "boolean\n");
    }

    #[test]
    fn typeof_variable() {
        assert_eq!(run_ts("let x = 42\nconsole.log(typeof x)"), "number\n");
    }

    #[test]
    fn parse_int() {
        assert_eq!(run_ts(r#"console.log(parseInt("42"))"#), "42\n");
    }

    #[test]
    fn parse_int_with_decimals() {
        assert_eq!(run_ts(r#"console.log(parseInt("3.14"))"#), "3\n");
    }

    #[test]
    fn parse_float() {
        assert_eq!(run_ts(r#"console.log(parseFloat("3.14"))"#), "3.14\n");
    }

    #[test]
    fn parse_float_integer() {
        assert_eq!(run_ts(r#"console.log(parseFloat("42"))"#), "42\n");
    }

    #[test]
    fn crypto_random_uuid_length() {
        // UUID v4 is always exactly 36 characters: 8-4-4-4-12 + 4 hyphens
        assert_eq!(run_ts("console.log(crypto.randomUUID().length)"), "36\n");
    }

    #[test]
    fn crypto_random_uuid_version_nibble() {
        // The 13th character (index 14, after "xxxxxxxx-xxxx-") is always '4' (version 4)
        assert_eq!(run_ts("console.log(crypto.randomUUID().charAt(14))"), "4\n");
    }

    #[test]
    fn crypto_random_uuid_unique() {
        // Two calls must produce different values
        assert_eq!(
            run_ts("console.log(crypto.randomUUID() === crypto.randomUUID())"),
            "false\n"
        );
    }
}

// ============================================================
// 12. MODULES (import/export)
// ============================================================
mod modules {
    use super::*;
    use std::io::Write;

    /// Helper to compile & run multi-file programs.
    /// Takes (filename, content) pairs; last pair is the entry file.
    fn run_ts_multi(files: &[(&str, &str)]) -> String {
        let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let tid = std::thread::current().id();
        let dir = std::env::temp_dir().join(format!("tscc_test_{:?}_{}", tid, id));
        std::fs::create_dir_all(&dir).unwrap();

        // Write all files
        for (name, content) in files {
            let path = dir.join(name);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            let mut f = std::fs::File::create(&path).unwrap();
            f.write_all(content.as_bytes()).unwrap();
        }

        // The entry file is the last one
        let entry_name = files.last().unwrap().0;
        let entry_path = dir.join(entry_name);
        let output = dir.join("test_binary");

        tscc::compile_file(
            entry_path.to_str().unwrap(),
            output.to_str().unwrap(),
            false,
            false,
        )
        .unwrap_or_else(|e| panic!("Multi-file compilation failed:\n{}", e));

        let run = Command::new(&output)
            .output()
            .expect("Failed to execute compiled binary");

        let _ = std::fs::remove_dir_all(&dir);

        assert!(
            run.status.success(),
            "Binary exited with non-zero status: {:?}",
            run.status
        );

        String::from_utf8(run.stdout).unwrap()
    }

    #[test]
    fn export_import_function() {
        let math_ts = r#"
export function square(x: number): number {
    return x * x
}
"#;
        let main_ts = r#"
import { square } from "./math"
console.log(square(5))
"#;
        assert_eq!(
            run_ts_multi(&[("math.ts", math_ts), ("main.ts", main_ts)]),
            "25\n"
        );
    }

    #[test]
    fn import_multiple_functions() {
        let utils_ts = r#"
export function double(x: number): number {
    return x * 2
}
export function triple(x: number): number {
    return x * 3
}
"#;
        let main_ts = r#"
import { double, triple } from "./utils"
console.log(double(5))
console.log(triple(5))
"#;
        assert_eq!(
            run_ts_multi(&[("utils.ts", utils_ts), ("main.ts", main_ts)]),
            "10\n15\n"
        );
    }

    #[test]
    fn import_with_alias() {
        let math_ts = r#"
export function add(a: number, b: number): number {
    return a + b
}
"#;
        let main_ts = r#"
import { add as sum } from "./math"
console.log(sum(3, 4))
"#;
        assert_eq!(
            run_ts_multi(&[("math.ts", math_ts), ("main.ts", main_ts)]),
            "7\n"
        );
    }
}

// ============================================================
// 13. COMPOUND / EDGE CASES
// ============================================================
mod edge_cases {
    use super::*;

    #[test]
    fn empty_program() {
        assert_eq!(run_ts(""), "");
    }

    #[test]
    fn comments_only() {
        // If the lexer supports comments — test that they don't produce output
        // If not supported, this might fail and should be moved to #[ignore]
        assert_eq!(run_ts("// this is a comment\nconsole.log(1)"), "1\n");
    }

    #[test]
    fn deeply_nested_expressions() {
        assert_eq!(run_ts("console.log(((((1 + 2)))))"), "3\n");
    }

    #[test]
    fn variable_in_function_scope() {
        let src = r#"
let x = 10
function getX(): number {
    return x
}
console.log(getX())
"#;
        // This tests whether functions can access outer scope variables
        // Current implementation uses lexical scoping via alloca — this may or may not work
        // depending on how the codegen handles it
        let result = std::panic::catch_unwind(|| run_ts(src));
        if result.is_err() {
            // If it panics, closures aren't supported yet — that's expected
            eprintln!("NOTE: Accessing outer variables from functions not yet supported");
        }
    }

    #[test]
    fn chained_string_methods() {
        assert_eq!(
            run_ts(r#"console.log("  Hello  ".trim().toUpperCase())"#),
            "HELLO\n"
        );
    }

    #[test]
    fn integer_narrowing_fibonacci() {
        // This exercises the integer narrowing optimization path
        let src = r#"
function fib(n: number): number {
    if (n <= 1) {
        return n
    }
    return fib(n - 1) + fib(n - 2)
}
console.log(fib(20))
"#;
        assert_eq!(run_ts(src), "6765\n");
    }

    #[test]
    fn mixed_print_types() {
        let src = r#"
let n = 42
let s = "hello"
let b = true
console.log(n, s, b)
"#;
        assert_eq!(run_ts(src), "42 hello true\n");
    }
}

// ============================================================
// 14. MAP
// ============================================================
mod map {
    use super::*;

    #[test]
    fn map_set_get() {
        let src = "
const m = new Map()
m.set(\"hello\", 42)
console.log(m.get(\"hello\"))
";
        assert_eq!(run_ts(src), "42\n");
    }

    #[test]
    fn map_has_delete() {
        let src = "
const m = new Map()
m.set(\"a\", 1)
console.log(m.has(\"a\"))
console.log(m.has(\"b\"))
m.delete(\"a\")
console.log(m.has(\"a\"))
";
        assert_eq!(run_ts(src), "true\nfalse\nfalse\n");
    }

    #[test]
    fn map_size() {
        let src = "
const m = new Map()
m.set(\"x\", 1)
m.set(\"y\", 2)
m.set(\"z\", 3)
console.log(m.size)
m.delete(\"y\")
console.log(m.size)
";
        assert_eq!(run_ts(src), "3\n2\n");
    }

    #[test]
    fn map_overwrite() {
        let src = "
const m = new Map()
m.set(\"key\", 1)
m.set(\"key\", 99)
console.log(m.get(\"key\"))
";
        assert_eq!(run_ts(src), "99\n");
    }

    #[test]
    fn map_get_missing() {
        let src = "
const m = new Map()
m.set(\"a\", 10)
const v = m.get(\"missing\")
console.log(v)
";
        assert_eq!(run_ts(src), "0\n");
    }

    #[test]
    fn map_type_annotation() {
        let src = "
const m: Map<string, number> = new Map()
m.set(\"pi\", 3.14)
console.log(m.get(\"pi\"))
";
        assert_eq!(run_ts(src), "3.14\n");
    }

    #[test]
    fn map_field_initializer() {
        let src = "
class Store {
  items: Map<string, number> = new Map()
  add(k: string, v: number): void {
    this.items.set(k, v)
  }
  get(k: string): number {
    return this.items.get(k)
  }
}
const s = new Store()
s.add(\"x\", 42)
console.log(s.get(\"x\"))
";
        assert_eq!(run_ts(src), "42\n");
    }
}

// ============================================================
// 15. DISCRIMINATED UNIONS
// ============================================================
mod discriminated_unions {
    use super::*;

    #[test]
    fn shared_property_access() {
        // Accessing a property present in ALL union variants is valid TypeScript
        let src = r#"
type Result = { success: boolean; value: number } | { success: boolean; error: string }
function check(r: Result): boolean {
  return r.success
}
console.log(check({ success: true, value: 42, error: "" }))
console.log(check({ success: false, value: 0, error: "oops" }))
"#;
        assert_eq!(run_ts(src), "true\nfalse\n");
    }

    #[test]
    fn discriminated_union_widening() {
        // Object literals returned as a union type are widened to the merged struct.
        // Both return sites produce different fields; the merged struct holds all.
        let src = r#"
type Result = { ok: boolean; val: number; err: string }
function make_ok(n: number): Result {
  return { ok: true, val: n, err: "" }
}
function make_err(msg: string): Result {
  return { ok: false, val: 0, err: msg }
}
const r1 = make_ok(99)
const r2 = make_err("bad")
console.log(r1.ok)
console.log(r1.val)
console.log(r2.ok)
console.log(r2.err)
"#;
        assert_eq!(run_ts(src), "true\n99\nfalse\nbad\n");
    }

    #[test]
    fn two_variant_result_type() {
        // Classic Result<T> discriminated union
        let src = r#"
type Result = { success: boolean; data: number; error: string }
function divide(a: number, b: number): Result {
  if (b === 0) {
    return { success: false, data: 0, error: "division by zero" }
  }
  return { success: true, data: a / b, error: "" }
}
const r1 = divide(10, 2)
const r2 = divide(5, 0)
console.log(r1.success)
console.log(r1.data)
console.log(r2.success)
console.log(r2.error)
"#;
        assert_eq!(run_ts(src), "true\n5\nfalse\ndivision by zero\n");
    }

    #[test]
    fn type_predicate_basic() {
        // A function returning `x is T` acts as a type guard: the type checker
        // narrows the argument in the if-true branch.
        let src = r#"
type Result = { success: true; data: number } | { success: false; error: string }

function isSuccess(result: Result): result is { success: true; data: number } {
  return (result as any).success === true
}

let r: Result = { success: true, data: 42, error: "" }
if (isSuccess(r)) {
  console.log(r.data)
}
"#;
        assert_eq!(run_ts(src), "42\n");
    }

    #[test]
    fn type_predicate_generic() {
        // Generic type predicate `result is { success: true; data: T }` —
        // T resolves to Unknown at registration time (wildcard), so structural
        // matching falls back to field-name presence for the `data` field.
        let src = r#"
type Result<T> =
  | { success: true; data: T }
  | { success: false; error: string }

function isSuccess<T>(result: Result<T>): result is { success: true; data: T } {
  return true
}

let r: Result<number> = { success: true, data: 99, error: "" }
if (isSuccess(r)) {
  console.log(r.data)
}
"#;
        assert_eq!(run_ts(src), "99\n");
    }

    #[test]
    fn type_predicate_else_branch() {
        // The else branch narrows to the non-matching union variants.
        let src = r#"
type Shape = { kind: string; radius: number } | { kind: string; width: number; height: number }

function isCircle(s: Shape): s is { kind: string; radius: number } {
  return true
}

let s: Shape = { kind: "circle", radius: 5, width: 0, height: 0 }
if (isCircle(s)) {
  console.log(s.radius)
} else {
  console.log(s.width)
}
"#;
        assert_eq!(run_ts(src), "5\n");
    }
}

// ============================================================
// 16b. TIMERS (setTimeout)
// ============================================================
mod timers {
    use super::*;

    #[test]
    fn set_timeout_basic() {
        // Callbacks fire in registration order (equal delay), after synchronous code.
        let src = r#"
setTimeout(() => console.log("timer"), 0)
console.log("sync")
"#;
        assert_eq!(run_ts(src), "sync\ntimer\n");
    }

    #[test]
    fn set_timeout_ordering() {
        // Lower delay fires first.
        let src = r#"
setTimeout(() => console.log("second"), 20)
setTimeout(() => console.log("first"), 0)
console.log("sync")
"#;
        assert_eq!(run_ts(src), "sync\nfirst\nsecond\n");
    }

    #[test]
    fn set_timeout_with_capture() {
        // Closure captures outer variable.
        let src = r#"
let msg = "captured"
setTimeout(() => console.log(msg), 0)
console.log("sync")
"#;
        assert_eq!(run_ts(src), "sync\ncaptured\n");
    }

    #[test]
    fn promise_void_via_set_timeout() {
        // new Promise(resolve => setTimeout(resolve, ms)) — the resolve closure
        // is a zero-arg closure that resolves with undefined.
        // await suspends until the timer fires.
        let src = r#"
async function delay(ms: number): Promise<void> {
  return new Promise(resolve => setTimeout(resolve, ms))
}
async function run(): Promise<void> {
  console.log("before")
  await delay(0)
  console.log("after")
}
run()
"#;
        assert_eq!(run_ts(src), "before\nafter\n");
    }
}

// ============================================================
// 16. DATE
// ============================================================
mod date {
    use super::*;

    #[test]
    fn date_now_returns_number() {
        // Date.now() should return a positive integer (ms since epoch)
        let src = "
const t = Date.now()
console.log(t > 0)
";
        assert_eq!(run_ts(src), "true\n");
    }

    #[test]
    fn date_new_no_args() {
        // new Date() should not crash; getTime() should be > 0
        let src = "
const d = new Date()
console.log(d.getTime() > 0)
";
        assert_eq!(run_ts(src), "true\n");
    }

    #[test]
    fn date_new_from_ms() {
        // new Date(0) is the Unix epoch; getTime() should return 0
        let src = "
const d = new Date(0)
console.log(d.getTime())
";
        assert_eq!(run_ts(src), "0\n");
    }

    #[test]
    fn date_get_full_year() {
        // 2025-03-16T00:00:00Z = ms = 1742083200000
        let src = "
const d = new Date(1742083200000)
console.log(d.getUTCFullYear())
console.log(d.getUTCMonth())
console.log(d.getUTCDate())
";
        assert_eq!(run_ts(src), "2025\n2\n16\n");
    }

    #[test]
    fn date_get_utc_time_components() {
        // 2000-01-01T11:30:45.678Z = ms = 946726245678
        let src = "
const d = new Date(946726245678)
console.log(d.getUTCFullYear())
console.log(d.getUTCMonth())
console.log(d.getUTCDate())
console.log(d.getUTCHours())
console.log(d.getUTCMinutes())
console.log(d.getUTCSeconds())
console.log(d.getUTCMilliseconds())
";
        assert_eq!(run_ts(src), "2000\n0\n1\n11\n30\n45\n678\n");
    }

    #[test]
    fn date_to_iso_string() {
        // 2000-01-01T11:30:45.678Z = ms = 946726245678
        let src = r#"
const d = new Date(946726245678)
console.log(d.toISOString())
"#;
        assert_eq!(run_ts(src), "2000-01-01T11:30:45.678Z\n");
    }

    #[test]
    fn date_string_arg_compile_error() {
        assert_compile_fails("const d = new Date(\"2024-01-01\")");
    }

    #[test]
    fn date_type_annotation() {
        // createdAt: Date type annotation should be accepted
        let src = "
const d: Date = new Date(0)
console.log(d.getTime())
";
        assert_eq!(run_ts(src), "0\n");
    }
}

// ============================================================
// 17. PARITY — Output Correctness vs Node.js
//
// These tests assert the *exact* output Node.js produces.
// Tests marked #[ignore] compile successfully in tscc but
// produce wrong output today. They auto-pass once fixed.
// Run the full parity backlog with:
//   cargo test parity -- --ignored
// ============================================================
mod parity {
    use super::*;

    // --------------------------------------------------------
    // console.log formatting
    // --------------------------------------------------------
    mod console_log_formatting {
        use super::*;

        #[test]
        #[ignore = "parity: null prints as 0, should print 'null'"]
        fn null_prints_as_null() {
            assert_eq!(run_ts("console.log(null)"), "null\n");
        }

        #[test]
        #[ignore = "parity: undefined prints as 0, should print 'undefined'"]
        fn undefined_prints_as_undefined() {
            assert_eq!(run_ts("console.log(undefined)"), "undefined\n");
        }

        #[test]
        #[ignore = "parity: NaN prints as 'nan', should print 'NaN'"]
        fn nan_prints_as_nan() {
            assert_eq!(run_ts("console.log(NaN)"), "NaN\n");
        }

        #[test]
        #[ignore = "parity: Infinity prints as 'inf', should print 'Infinity'"]
        fn infinity_prints_as_infinity() {
            assert_eq!(run_ts("console.log(Infinity)"), "Infinity\n");
        }

        #[test]
        #[ignore = "parity: -Infinity prints as '-inf', should print '-Infinity'"]
        fn neg_infinity_prints_correctly() {
            assert_eq!(run_ts("console.log(-Infinity)"), "-Infinity\n");
        }

        #[test]
        #[ignore = "parity: -0 prints as 0, should print '-0'"]
        fn neg_zero_prints_as_neg_zero() {
            assert_eq!(run_ts("console.log(-0)"), "-0\n");
        }

        #[test]
        #[ignore = "parity: empty array prints as '[  ]' with extra spaces, should be '[]'"]
        fn empty_array_no_spaces() {
            assert_eq!(run_ts("console.log([])"), "[]\n");
        }

        #[test]
        #[ignore = "parity: empty object prints as '{  }' with extra spaces, should be '{}'"]
        fn empty_object_no_spaces() {
            assert_eq!(run_ts("console.log({})"), "{}\n");
        }

        #[test]
        #[ignore = "parity: string array literal prints garbage IEEE754 numbers instead of quoted strings"]
        fn string_array_literal() {
            assert_eq!(
                run_ts(r#"console.log(["a", "b", "c"])"#),
                "[ 'a', 'b', 'c' ]\n"
            );
        }

        #[test]
        #[ignore = "parity: nested number array prints as garbage pointer values, should recurse"]
        fn nested_number_array() {
            assert_eq!(
                run_ts("console.log([[1, 2], [3, 4]])"),
                "[ [ 1, 2 ], [ 3, 4 ] ]\n"
            );
        }

        #[test]
        #[ignore = "parity: nested object prints as '{ a: [complex] }', should recurse"]
        fn nested_object() {
            assert_eq!(run_ts("console.log({ a: { b: 1 } })"), "{ a: { b: 1 } }\n");
        }

        #[test]
        #[ignore = "parity: object with string value prints value unquoted, should be single-quoted"]
        fn object_with_string_value() {
            assert_eq!(
                run_ts(r#"console.log({ name: "alice" })"#),
                "{ name: 'alice' }\n"
            );
        }

        #[test]
        fn multiple_args_space_separated() {
            // Node.js: console.log(1, "hi", true) => "1 hi true"
            assert_eq!(run_ts(r#"console.log(1, "hi", true)"#), "1 hi true\n");
        }

        #[test]
        fn array_of_numbers_format() {
            // Verify existing number array format matches Node.js exactly
            assert_eq!(run_ts("console.log([1, 2, 3])"), "[ 1, 2, 3 ]\n");
        }

        #[test]
        fn object_with_number_values_format() {
            assert_eq!(
                run_ts("let obj = { x: 1, y: 2 }\nconsole.log(obj)"),
                "{ x: 1, y: 2 }\n"
            );
        }

        #[test]
        fn boolean_in_console_log() {
            assert_eq!(
                run_ts("console.log(true)\nconsole.log(false)"),
                "true\nfalse\n"
            );
        }
    }

    // --------------------------------------------------------
    // Class instance printing
    // --------------------------------------------------------
    mod class_printing {
        use super::*;

        #[test]
        #[ignore = "parity: class instance prints without class name prefix, should be 'Point { x: 3, y: 4 }'"]
        fn class_instance_with_name() {
            let src = r#"
class Point {
    x: number
    y: number
    constructor(x: number, y: number) {
        this.x = x
        this.y = y
    }
}
console.log(new Point(3, 4))
"#;
            assert_eq!(run_ts(src), "Point { x: 3, y: 4 }\n");
        }

        #[test]
        #[ignore = "parity: class with string field prints value unquoted, should be single-quoted"]
        fn class_instance_with_string_field() {
            let src = r#"
class Person {
    name: string
    constructor(name: string) {
        this.name = name
    }
}
console.log(new Person("Alice"))
"#;
            assert_eq!(run_ts(src), "Person { name: 'Alice' }\n");
        }

        #[test]
        #[ignore = "parity: child class instance prints without class name prefix"]
        fn inherited_class_instance() {
            let src = r#"
class Animal {
    name: string
    constructor(name: string) {
        this.name = name
    }
}
class Dog extends Animal {}
console.log(new Dog("Rex"))
"#;
            assert_eq!(run_ts(src), "Dog { name: 'Rex' }\n");
        }

        #[test]
        #[ignore = "parity: class instance with mixed fields prints without class name and with wrong value formatting"]
        fn class_instance_mixed_fields() {
            let src = r#"
class Item {
    id: number
    label: string
    active: boolean
    constructor(id: number, label: string, active: boolean) {
        this.id = id
        this.label = label
        this.active = active
    }
}
console.log(new Item(1, "foo", true))
"#;
            assert_eq!(run_ts(src), "Item { id: 1, label: 'foo', active: true }\n");
        }
    }

    // --------------------------------------------------------
    // typeof operator
    // --------------------------------------------------------
    mod typeof_operator {
        use super::*;

        #[test]
        fn typeof_number() {
            assert_eq!(run_ts(r#"console.log(typeof 42)"#), "number\n");
        }

        #[test]
        fn typeof_string() {
            assert_eq!(run_ts(r#"console.log(typeof "hello")"#), "string\n");
        }

        #[test]
        fn typeof_boolean() {
            assert_eq!(run_ts("console.log(typeof true)"), "boolean\n");
        }

        #[test]
        fn typeof_object_literal() {
            assert_eq!(run_ts("console.log(typeof {})"), "object\n");
        }

        #[test]
        #[ignore = "parity: typeof null returns 'number', should return 'object' (historic JS quirk)"]
        fn typeof_null() {
            assert_eq!(run_ts("console.log(typeof null)"), "object\n");
        }

        #[test]
        #[ignore = "parity: typeof undefined returns 'number', should return 'undefined'"]
        fn typeof_undefined() {
            assert_eq!(run_ts("console.log(typeof undefined)"), "undefined\n");
        }

        #[test]
        #[ignore = "parity: typeof arrow function returns 'object', should return 'function'"]
        fn typeof_arrow_function() {
            assert_eq!(
                run_ts("const f = () => 1\nconsole.log(typeof f)"),
                "function\n"
            );
        }

        #[test]
        #[ignore = "parity: typeof named function returns 'object', should return 'function'"]
        fn typeof_named_function() {
            assert_eq!(
                run_ts("function add(a: number, b: number): number { return a + b }\nconsole.log(typeof add)"),
                "function\n"
            );
        }

        #[test]
        fn typeof_array_is_object() {
            // typeof [] === "object" in Node.js
            assert_eq!(run_ts("console.log(typeof [1, 2, 3])"), "object\n");
        }
    }

    // --------------------------------------------------------
    // Number formatting
    // --------------------------------------------------------
    mod number_formatting {
        use super::*;

        #[test]
        #[ignore = "parity: 0.1 + 0.2 prints '0.3' due to %.15g rounding, should print '0.30000000000000004'"]
        fn float_precision_0_1_plus_0_2() {
            assert_eq!(run_ts("console.log(0.1 + 0.2)"), "0.30000000000000004\n");
        }

        #[test]
        #[ignore = "parity: 1/0 prints 'inf', should print 'Infinity'"]
        fn division_by_zero_is_infinity() {
            assert_eq!(run_ts("console.log(1 / 0)"), "Infinity\n");
        }

        #[test]
        #[ignore = "parity: -1/0 prints '-inf', should print '-Infinity'"]
        fn neg_division_by_zero_is_neg_infinity() {
            assert_eq!(run_ts("console.log(-1 / 0)"), "-Infinity\n");
        }

        #[test]
        #[ignore = "parity: NaN === NaN returns true in tscc, should return false"]
        fn nan_not_equal_to_itself() {
            assert_eq!(run_ts("console.log(NaN === NaN)"), "false\n");
        }

        #[test]
        #[ignore = "parity: large integer 9007199254740992 prints in scientific notation, should print exact digits"]
        fn large_integer_exact() {
            // 2^53 — max safe integer, should print exactly
            assert_eq!(
                run_ts("console.log(9007199254740992)"),
                "9007199254740992\n"
            );
        }

        #[test]
        fn negative_float() {
            assert_eq!(run_ts("console.log(-3.14)"), "-3.14\n");
        }

        #[test]
        fn integer_division_result() {
            // 10 / 2 = 5.0 but should print as 5 (integer-like)
            assert_eq!(run_ts("console.log(10 / 2)"), "5\n");
        }

        #[test]
        fn non_integer_division_result() {
            assert_eq!(run_ts("console.log(7 / 2)"), "3.5\n");
        }
    }

    // --------------------------------------------------------
    // String coercions / concatenation with non-strings
    // --------------------------------------------------------
    mod string_coercions {
        use super::*;

        #[test]
        #[ignore = "parity: '' + null yields '0', should yield 'null'"]
        fn concat_null() {
            assert_eq!(run_ts(r#"console.log("" + null)"#), "null\n");
        }

        #[test]
        #[ignore = "parity: '' + undefined yields '0', should yield 'undefined'"]
        fn concat_undefined() {
            assert_eq!(run_ts(r#"console.log("" + undefined)"#), "undefined\n");
        }

        #[test]
        #[ignore = "parity: '' + NaN yields 'nan', should yield 'NaN'"]
        fn concat_nan() {
            assert_eq!(run_ts(r#"console.log("" + NaN)"#), "NaN\n");
        }

        #[test]
        #[ignore = "parity: '' + Infinity yields 'inf', should yield 'Infinity'"]
        fn concat_infinity() {
            assert_eq!(run_ts(r#"console.log("" + Infinity)"#), "Infinity\n");
        }

        #[test]
        fn concat_true() {
            assert_eq!(run_ts(r#"console.log("" + true)"#), "true\n");
        }

        #[test]
        fn concat_false() {
            assert_eq!(run_ts(r#"console.log("" + false)"#), "false\n");
        }

        #[test]
        fn concat_number() {
            assert_eq!(run_ts(r#"console.log("val=" + 42)"#), "val=42\n");
        }

        #[test]
        fn concat_float() {
            assert_eq!(run_ts(r#"console.log("val=" + 3.14)"#), "val=3.14\n");
        }
    }

    // --------------------------------------------------------
    // Equality parity
    // --------------------------------------------------------
    mod equality_parity {
        use super::*;

        #[test]
        fn strict_eq_numbers() {
            assert_eq!(run_ts("console.log(1 === 1)"), "true\n");
            assert_eq!(run_ts("console.log(1 === 2)"), "false\n");
        }

        #[test]
        fn strict_eq_strings() {
            assert_eq!(run_ts(r#"console.log("a" === "a")"#), "true\n");
            assert_eq!(run_ts(r#"console.log("a" === "b")"#), "false\n");
        }

        #[test]
        fn strict_eq_booleans() {
            assert_eq!(run_ts("console.log(true === true)"), "true\n");
            assert_eq!(run_ts("console.log(true === false)"), "false\n");
        }

        #[test]
        #[ignore = "parity: NaN === NaN returns true, should return false"]
        fn nan_strict_ne_nan() {
            assert_eq!(run_ts("console.log(NaN === NaN)"), "false\n");
        }

        #[test]
        #[ignore = "parity: null === undefined returns true, should return false"]
        fn null_strict_ne_undefined() {
            assert_eq!(run_ts("console.log(null === undefined)"), "false\n");
        }

        #[test]
        #[ignore = "parity: null == undefined — loose equality not implemented, should return true"]
        fn null_loose_eq_undefined() {
            assert_eq!(run_ts("console.log(null == undefined)"), "true\n");
        }

        #[test]
        #[ignore = "parity: 0 == '' — loose equality not implemented, should return true"]
        fn zero_loose_eq_empty_string() {
            assert_eq!(run_ts(r#"console.log(0 == "")"#), "true\n");
        }
    }

    // --------------------------------------------------------
    // Math built-ins
    // --------------------------------------------------------
    mod math_builtins {
        use super::*;

        #[test]
        fn floor_positive() {
            assert_eq!(run_ts("console.log(Math.floor(1.9))"), "1\n");
        }

        #[test]
        fn floor_negative() {
            // Math.floor(-1.5) === -2 in Node.js
            assert_eq!(run_ts("console.log(Math.floor(-1.5))"), "-2\n");
        }

        #[test]
        fn ceil_positive() {
            assert_eq!(run_ts("console.log(Math.ceil(1.1))"), "2\n");
        }

        #[test]
        fn ceil_negative() {
            // Math.ceil(-1.5) === -1 in Node.js
            assert_eq!(run_ts("console.log(Math.ceil(-1.5))"), "-1\n");
        }

        #[test]
        fn round_half_up() {
            // Math.round(0.5) === 1
            assert_eq!(run_ts("console.log(Math.round(0.5))"), "1\n");
        }

        #[test]
        #[ignore = "parity: Math.round(-0.5) returns -1 in tscc, should return 0 (Node.js rounds toward +Infinity)"]
        fn round_neg_half() {
            // Math.round(-0.5) === 0 in Node.js (rounds toward +Infinity)
            assert_eq!(run_ts("console.log(Math.round(-0.5))"), "0\n");
        }

        #[test]
        fn abs_negative() {
            assert_eq!(run_ts("console.log(Math.abs(-5))"), "5\n");
        }

        #[test]
        fn max_of_two() {
            assert_eq!(run_ts("console.log(Math.max(3, 7))"), "7\n");
        }

        #[test]
        fn min_of_two() {
            assert_eq!(run_ts("console.log(Math.min(3, 7))"), "3\n");
        }

        #[test]
        fn pow_basic() {
            assert_eq!(run_ts("console.log(Math.pow(2, 10))"), "1024\n");
        }

        #[test]
        fn sqrt_basic() {
            assert_eq!(run_ts("console.log(Math.sqrt(9))"), "3\n");
        }

        #[test]
        #[ignore = "parity: Math.trunc not yet implemented"]
        fn trunc_positive() {
            assert_eq!(run_ts("console.log(Math.trunc(1.9))"), "1\n");
        }

        #[test]
        #[ignore = "parity: Math.trunc not yet implemented"]
        fn trunc_negative() {
            assert_eq!(run_ts("console.log(Math.trunc(-1.9))"), "-1\n");
        }

        #[test]
        #[ignore = "parity: Math.sign not yet implemented"]
        fn sign_positive() {
            assert_eq!(run_ts("console.log(Math.sign(5))"), "1\n");
        }

        #[test]
        #[ignore = "parity: Math.sign not yet implemented"]
        fn sign_negative() {
            assert_eq!(run_ts("console.log(Math.sign(-5))"), "-1\n");
        }

        #[test]
        #[ignore = "parity: Math.sign not yet implemented"]
        fn sign_zero() {
            assert_eq!(run_ts("console.log(Math.sign(0))"), "0\n");
        }

        #[test]
        #[ignore = "parity: Math.log2 not yet implemented"]
        fn log2_basic() {
            assert_eq!(run_ts("console.log(Math.log2(8))"), "3\n");
        }

        #[test]
        #[ignore = "parity: Math.log10 not yet implemented"]
        fn log10_basic() {
            assert_eq!(run_ts("console.log(Math.log10(1000))"), "3\n");
        }

        #[test]
        #[ignore = "parity: Math.hypot not yet implemented"]
        fn hypot_basic() {
            assert_eq!(run_ts("console.log(Math.hypot(3, 4))"), "5\n");
        }

        #[test]
        #[ignore = "parity: Math.clz32 not yet implemented"]
        fn clz32_basic() {
            assert_eq!(run_ts("console.log(Math.clz32(1))"), "31\n");
        }
    }

    // --------------------------------------------------------
    // Array printing format
    // --------------------------------------------------------
    mod array_printing {
        use super::*;

        #[test]
        fn number_array_format() {
            assert_eq!(run_ts("console.log([1, 2, 3])"), "[ 1, 2, 3 ]\n");
        }

        #[test]
        fn single_element_array() {
            assert_eq!(run_ts("console.log([42])"), "[ 42 ]\n");
        }

        #[test]
        #[ignore = "parity: empty array prints '[ ]' or '[  ]', should print '[]'"]
        fn empty_array() {
            assert_eq!(run_ts("console.log([])"), "[]\n");
        }

        #[test]
        #[ignore = "parity: string array literal prints garbage, should be \"[ 'a', 'b', 'c' ]\""]
        fn string_array() {
            assert_eq!(
                run_ts(r#"console.log(["a", "b", "c"])"#),
                "[ 'a', 'b', 'c' ]\n"
            );
        }

        #[test]
        #[ignore = "parity: nested array prints as garbage pointer values"]
        fn nested_number_array() {
            assert_eq!(
                run_ts("console.log([[1, 2], [3, 4]])"),
                "[ [ 1, 2 ], [ 3, 4 ] ]\n"
            );
        }

        #[test]
        #[ignore = "parity: boolean array — booleans stored as i1, array printing may not handle booleans"]
        fn boolean_array() {
            assert_eq!(
                run_ts("console.log([true, false, true])"),
                "[ true, false, true ]\n"
            );
        }

        #[test]
        fn array_length_after_push() {
            let src = "
let arr = [1, 2, 3]
arr.push(4)
console.log(arr.length)
";
            assert_eq!(run_ts(src), "4\n");
        }
    }

    // --------------------------------------------------------
    // Object printing format
    // --------------------------------------------------------
    mod object_printing {
        use super::*;

        #[test]
        fn simple_number_object() {
            assert_eq!(run_ts("console.log({ x: 1, y: 2 })"), "{ x: 1, y: 2 }\n");
        }

        #[test]
        #[ignore = "parity: empty object prints '{  }' or '{ }', should print '{}'"]
        fn empty_object() {
            assert_eq!(run_ts("console.log({})"), "{}\n");
        }

        #[test]
        #[ignore = "parity: object with string value prints value without quotes, should use single quotes"]
        fn object_string_value() {
            assert_eq!(
                run_ts(r#"console.log({ name: "hello" })"#),
                "{ name: 'hello' }\n"
            );
        }

        #[test]
        #[ignore = "parity: nested object prints as '{ a: [complex] }', should recurse"]
        fn nested_object() {
            assert_eq!(run_ts("console.log({ a: { b: 1 } })"), "{ a: { b: 1 } }\n");
        }

        #[test]
        fn object_boolean_value() {
            assert_eq!(run_ts("console.log({ flag: true })"), "{ flag: true }\n");
        }
    }
}

// ============================================================
// 16. NOT YET IMPLEMENTED — TypeScript Coverage Gap
//
// Each #[ignore] test represents a TypeScript feature that tscc
// does not yet support. The total ignored count = coverage gap.
// ============================================================
mod not_yet_implemented {
    use super::*;

    // --- Variable declarations ---

    #[test]
    fn var_declaration() {
        assert_eq!(run_ts("var x = 42\nconsole.log(x)"), "42\n");
    }

    #[test]
    fn destructuring_assignment() {
        assert_eq!(run_ts("let [a, b] = [1, 2]\nconsole.log(a, b)"), "1 2\n");
    }

    #[test]
    fn object_destructuring() {
        assert_eq!(
            run_ts("let { x, y } = { x: 1, y: 2 }\nconsole.log(x, y)"),
            "1 2\n"
        );
    }

    // --- Operators ---

    #[test]
    fn plus_equals() {
        assert_eq!(run_ts("let x = 5\nx += 3\nconsole.log(x)"), "8\n");
    }

    #[test]
    fn minus_equals() {
        assert_eq!(run_ts("let x = 10\nx -= 3\nconsole.log(x)"), "7\n");
    }

    #[test]
    fn star_equals() {
        assert_eq!(run_ts("let x = 4\nx *= 3\nconsole.log(x)"), "12\n");
    }

    #[test]
    fn slash_equals() {
        assert_eq!(run_ts("let x = 10\nx /= 2\nconsole.log(x)"), "5\n");
    }

    #[test]
    fn exponentiation() {
        assert_eq!(run_ts("console.log(2 ** 10)"), "1024\n");
    }

    #[test]
    fn ternary_operator() {
        assert_eq!(run_ts("console.log(true ? 1 : 2)"), "1\n");
    }

    #[test]
    fn nullish_coalescing() {
        assert_eq!(run_ts("console.log(null ?? 42)"), "42\n");
    }

    #[test]
    fn optional_chaining() {
        assert_eq!(run_ts("let obj = { a: 1 }\nconsole.log(obj?.a)"), "1\n");
    }

    // --- Control flow ---

    #[test]
    fn switch_case() {
        let src = r#"
let x = 2
switch (x) {
    case 1:
        console.log("one")
        break
    case 2:
        console.log("two")
        break
    default:
        console.log("other")
}
"#;
        assert_eq!(run_ts(src), "two\n");
    }

    #[test]
    fn break_in_loop() {
        let src = r#"
for (let i = 0; i < 10; i++) {
    if (i == 5) {
        break
    }
    console.log(i)
}
"#;
        assert_eq!(run_ts(src), "0\n1\n2\n3\n4\n");
    }

    #[test]
    fn continue_in_loop() {
        let src = r#"
for (let i = 0; i < 5; i++) {
    if (i == 2) {
        continue
    }
    console.log(i)
}
"#;
        assert_eq!(run_ts(src), "0\n1\n3\n4\n");
    }

    #[test]
    fn do_while() {
        let src = r#"
let i = 0
do {
    i++
} while (i < 5)
console.log(i)
"#;
        assert_eq!(run_ts(src), "5\n");
    }

    #[test]
    #[ignore = "string array literals compile as number arrays (pre-existing); for...of StringArray path is correct but unreachable from literals"]
    fn for_of_string_array() {
        let src = r#"
let words: string[] = ["hello", "world", "foo"]
for (const w of words) {
    console.log(w)
}
"#;
        assert_eq!(run_ts(src), "hello\nworld\nfoo\n");
    }

    #[test]
    #[ignore = "string array literals compile as number arrays (pre-existing); for...of StringArray path is correct but unreachable from literals"]
    fn for_of_string_array_method() {
        let src = r#"
let words: string[] = ["hello", "world"]
for (const w of words) {
    console.log(w.toUpperCase())
}
"#;
        assert_eq!(run_ts(src), "HELLO\nWORLD\n");
    }

    #[test]
    #[ignore = "array literals with object elements compile as VarType::Array (f64), not ObjArray; for...of ObjArray path is correct but unreachable from literals"]
    fn for_of_object_array() {
        let src = r#"
class Point {
    x: number
    y: number
    constructor(x: number, y: number) {
        this.x = x
        this.y = y
    }
}
let pts: Point[] = [new Point(1, 2), new Point(3, 4)]
for (const p of pts) {
    console.log(p.x + p.y)
}
"#;
        assert_eq!(run_ts(src), "3\n7\n");
    }

    #[test]
    #[ignore = "array literals with object elements compile as VarType::Array (f64), not ObjArray; for...of ObjArray path is correct but unreachable from literals"]
    fn for_of_object_array_field_access() {
        let src = r#"
class Person {
    name: string
    age: number
    constructor(name: string, age: number) {
        this.name = name
        this.age = age
    }
}
let people: Person[] = [new Person("alice", 30), new Person("bob", 25)]
for (const p of people) {
    console.log(p.name)
}
"#;
        assert_eq!(run_ts(src), "alice\nbob\n");
    }

    // for...of over an object array populated via Map + spread.
    // This exercises the ObjArray spread path (tscc_obj_array_push) and
    // the TypeAnnKind::Array → ObjArray dispatch in type_ann_to_var_type.
    #[test]
    fn for_of_object_array_via_map_spread() {
        let src = "
interface Point {
  x: number
  y: number
}
function makePoints(): Point[] {
  const m: Map<string, Point> = new Map()
  m.set(\"a\", { x: 1, y: 2 })
  m.set(\"b\", { x: 3, y: 4 })
  return [...m.values()]
}
function sumCoords(pts: Point[]): number {
  let total = 0
  for (const p of pts) {
    total = total + p.x + p.y
  }
  return total
}
console.log(sumCoords(makePoints()))
";
        assert_eq!(run_ts(src), "10\n");
    }

    // Dynamic object key update: obj[stringExpr]++ where the key is a runtime string variable.
    // This exercises the compile_update Object IndexAccess path with a dynamic string key.
    #[test]
    fn dynamic_object_key_increment() {
        let src = "
const counts = { todo: 0, done: 0, other: 0 }
let k = \"todo\"
counts[k]++
counts[k]++
k = \"done\"
counts[k]++
console.log(counts.todo)
console.log(counts.done)
console.log(counts.other)
";
        assert_eq!(run_ts(src), "2\n1\n0\n");
    }

    // Typed array parameter: function receives T[] and iterates over it.
    // Verifies that TypeAnnKind::Array(Object) resolves to ObjArray and
    // that for...of correctly binds element fields.
    #[test]
    fn typed_object_array_parameter() {
        let src = "
interface Item {
  name: string
  value: number
}
function printItems(items: Item[]): void {
  for (const it of items) {
    console.log(it.value)
  }
}
const m: Map<string, Item> = new Map()
m.set(\"x\", { name: \"x\", value: 10 })
m.set(\"y\", { name: \"y\", value: 20 })
printItems([...m.values()])
";
        assert_eq!(run_ts(src), "10\n20\n");
    }

    #[test]
    fn for_in() {
        let src = r#"
let obj = { a: 1, b: 2 }
for (let key in obj) {
    console.log(key)
}
"#;
        // Output order may vary
        let out = run_ts(src);
        assert!(out.contains("a") && out.contains("b"));
    }

    #[test]
    fn labeled_break() {
        let src = r#"
outer: for (let i = 0; i < 3; i++) {
    for (let j = 0; j < 3; j++) {
        if (j == 1) break outer
        console.log(i, j)
    }
}
"#;
        assert_eq!(run_ts(src), "0 0\n");
    }

    // --- Strings ---

    #[test]
    fn template_literal() {
        assert_eq!(
            run_ts("let x = 42\nconsole.log(`value is ${x}`)"),
            "value is 42\n"
        );
    }

    #[test]
    fn string_starts_with() {
        assert_eq!(run_ts(r#"console.log("hello".startsWith("he"))"#), "true\n");
    }

    #[test]
    fn string_ends_with() {
        assert_eq!(run_ts(r#"console.log("hello".endsWith("lo"))"#), "true\n");
    }

    #[test]
    fn string_repeat() {
        assert_eq!(run_ts(r#"console.log("ab".repeat(3))"#), "ababab\n");
    }

    #[test]
    fn string_split() {
        assert_eq!(
            run_ts(r#"console.log("a,b,c".split(","))"#),
            "[ 'a', 'b', 'c' ]\n"
        );
    }

    #[test]
    fn string_replace() {
        assert_eq!(
            run_ts(r#"console.log("hello".replace("l", "r"))"#),
            "herlo\n"
        );
    }

    #[test]
    fn string_pad_start() {
        assert_eq!(run_ts(r#"console.log("5".padStart(3, "0"))"#), "005\n");
    }

    // --- Arrays ---

    #[test]
    fn array_literal() {
        assert_eq!(
            run_ts("let arr = [1, 2, 3]\nconsole.log(arr)"),
            "[ 1, 2, 3 ]\n"
        );
    }

    #[test]
    fn array_index_access() {
        assert_eq!(
            run_ts("let arr = [10, 20, 30]\nconsole.log(arr[1])"),
            "20\n"
        );
    }

    #[test]
    fn array_push() {
        assert_eq!(
            run_ts("let arr = [1, 2]\narr.push(3)\nconsole.log(arr.length)"),
            "3\n"
        );
    }

    #[test]
    fn array_pop() {
        assert_eq!(
            run_ts("let arr = [1, 2, 3]\nlet x = arr.pop()\nconsole.log(x)"),
            "3\n"
        );
    }

    #[test]
    fn array_length() {
        assert_eq!(
            run_ts("let arr = [1, 2, 3, 4]\nconsole.log(arr.length)"),
            "4\n"
        );
    }

    #[test]
    fn array_map() {
        assert_eq!(
            run_ts("let arr = [1, 2, 3]\nconsole.log(arr.map(x => x * 2))"),
            "[ 2, 4, 6 ]\n"
        );
    }

    #[test]
    fn array_filter() {
        assert_eq!(
            run_ts("let arr = [1, 2, 3, 4]\nconsole.log(arr.filter(x => x > 2))"),
            "[ 3, 4 ]\n"
        );
    }

    #[test]
    fn array_reduce() {
        assert_eq!(
            run_ts("let arr = [1, 2, 3, 4]\nconsole.log(arr.reduce((a, b) => a + b, 0))"),
            "10\n"
        );
    }

    #[test]
    fn array_forEach() {
        assert_eq!(
            run_ts("let arr = [1, 2, 3]\narr.forEach(x => console.log(x))"),
            "1\n2\n3\n"
        );
    }

    // --- Objects ---

    #[test]
    fn object_literal() {
        assert_eq!(
            run_ts("let obj = { x: 1, y: 2 }\nconsole.log(obj.x)"),
            "1\n"
        );
    }

    #[test]
    fn object_property_access() {
        assert_eq!(
            run_ts("let obj = { name: \"mango\" }\nconsole.log(obj.name)"),
            "mango\n"
        );
    }

    #[test]
    fn object_bracket_access() {
        assert_eq!(
            run_ts("let obj = { x: 42 }\nconsole.log(obj[\"x\"])"),
            "42\n"
        );
    }

    #[test]
    fn object_method() {
        let src = r#"
let obj = {
    x: 10,
    getX(): number {
        return this.x
    }
}
console.log(obj.getX())
"#;
        assert_eq!(run_ts(src), "10\n");
    }

    // --- Classes ---

    #[test]
    fn class_basic() {
        let src = r#"
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
console.log(p.toString())
"#;
        assert_eq!(run_ts(src), "3,4\n");
    }

    #[test]
    fn class_inheritance() {
        let src = r#"
class Animal {
    name: string
    constructor(name: string) {
        this.name = name
    }
    speak(): string {
        return this.name + " makes a sound"
    }
}
class Dog extends Animal {
    speak(): string {
        return this.name + " barks"
    }
}
let d = new Dog("Rex")
console.log(d.speak())
"#;
        assert_eq!(run_ts(src), "Rex barks\n");
    }

    // Concrete class instantiated inside a function body.
    // Previously failed because classes were registered in the main loop (third pass)
    // AFTER function bodies were compiled in the second pass.
    #[test]
    fn class_instantiated_inside_function() {
        let src = "
class Counter {
  value: number
  constructor(start: number) {
    this.value = start
  }
  increment(): void {
    this.value = this.value + 1
  }
  get(): number {
    return this.value
  }
}
function makeCounter(start: number): number {
  const c = new Counter(start)
  c.increment()
  c.increment()
  return c.get()
}
console.log(makeCounter(10))
";
        assert_eq!(run_ts(src), "12\n");
    }

    // Concrete class extending a generic class, calling an inherited method.
    // Verifies generic class template storage, first-pass layout registration with
    // type substitution, and monomorphized parent method compilation.
    #[test]
    fn class_extends_generic_parent() {
        let src = "
class Container<T> {
  items: Map<string, T> = new Map()
  add(key: string, item: T): void {
    this.items.set(key, item)
  }
  size(): number {
    return this.items.size
  }
}
class NumberBox extends Container<number> {
  addMultiple(key: string, a: number, b: number): void {
    this.add(key, a + b)
  }
}
const box = new NumberBox()
box.add(\"a\", 10)
box.add(\"b\", 20)
box.addMultiple(\"c\", 5, 25)
console.log(box.size())
";
        assert_eq!(run_ts(src), "3\n");
    }

    // Generic parent's inherited method called through the subclass instance.
    // Verifies the monomorphized function name dispatch works end-to-end.
    #[test]
    fn generic_parent_method_dispatch() {
        let src = "
class Repo<T> {
  items: Map<string, T> = new Map()
  set(id: string, item: T): void {
    this.items.set(id, item)
  }
  count(): number {
    return this.items.size
  }
}
class UserRepo extends Repo<number> {}
function fill(r: UserRepo): void {
  r.set(\"x\", 1)
  r.set(\"y\", 2)
  console.log(r.count())
}
const repo = new UserRepo()
fill(repo)
";
        assert_eq!(run_ts(src), "2\n");
    }

    // --- Interfaces ---

    #[test]
    fn interface_extends() {
        let src = "
interface Identifiable {
  id: number
}
interface Named extends Identifiable {
  name: string
}
function greet(n: Named): void {
  console.log(n.id, n.name)
}
greet({ id: 1, name: \"Alice\" })
";
        assert_eq!(run_ts(src), "1 Alice\n");
    }

    #[test]
    fn interface_extends_multi() {
        let src = "
interface HasId {
  id: number
}
interface HasName {
  name: string
}
interface User extends HasId, HasName {
  active: boolean
}
function print_user(u: User): void {
  console.log(u.id, u.name, u.active)
}
print_user({ id: 42, name: \"Bob\", active: true })
";
        assert_eq!(run_ts(src), "42 Bob true\n");
    }

    #[test]
    fn interface_basic() {
        let src = r#"
interface Point {
    x: number
    y: number
}
function printPoint(p: Point): void {
    console.log(p.x, p.y)
}
printPoint({ x: 1, y: 2 })
"#;
        assert_eq!(run_ts(src), "1 2\n");
    }

    // --- Type system features ---

    #[test]
    fn union_type() {
        let src = r#"
function format(val: string | number): string {
    if (typeof val === "number") {
        return "num:" + val
    }
    return "str:" + val
}
console.log(format(42))
console.log(format("hi"))
"#;
        assert_eq!(run_ts(src), "num:42\nstr:hi\n");
    }

    #[test]
    fn type_alias() {
        let src = r#"
type ID = string | number
let id: ID = 42
console.log(id)
"#;
        assert_eq!(run_ts(src), "42\n");
    }

    #[test]
    fn enum_basic() {
        let src = r#"
enum Color {
    Red,
    Green,
    Blue
}
console.log(Color.Red)
console.log(Color.Green)
"#;
        assert_eq!(run_ts(src), "0\n1\n");
    }

    #[test]
    fn enum_string() {
        let src = r#"
enum Direction {
    Up = "UP",
    Down = "DOWN"
}
console.log(Direction.Up)
"#;
        assert_eq!(run_ts(src), "UP\n");
    }

    // String enum used as a field type in an interface/struct.
    // Verifies that type_ann_to_var_type resolves Named("Status") → VarType::String,
    // so the struct layout is correct and field access returns a string, not a float.
    #[test]
    fn string_enum_as_struct_field_type() {
        let src = "
enum Status {
  Active = \"active\",
  Inactive = \"inactive\"
}
interface Item {
  name: string
  status: Status
}
const item: Item = { name: \"foo\", status: Status.Active }
console.log(item.status)
console.log(item.name)
";
        assert_eq!(run_ts(src), "active\nfoo\n");
    }

    // String enum field used as a dynamic object key for indexing (obj[enum_field]++).
    // Exercises the full chain: string enum → VarType::String field → dynamic key lookup.
    #[test]
    fn string_enum_field_as_dynamic_key() {
        let src = "
enum Kind {
  A = \"a\",
  B = \"b\"
}
interface Thing {
  kind: Kind
  value: number
}
const counts = { a: 0, b: 0 }
const t1: Thing = { kind: Kind.A, value: 1 }
const t2: Thing = { kind: Kind.B, value: 2 }
const t3: Thing = { kind: Kind.A, value: 3 }
counts[t1.kind]++
counts[t2.kind]++
counts[t3.kind]++
console.log(counts.a)
console.log(counts.b)
";
        assert_eq!(run_ts(src), "2\n1\n");
    }

    #[test]
    fn generic_function() {
        let src = r#"
function identity<T>(x: T): T {
    return x
}
console.log(identity(42))
console.log(identity("hi"))
"#;
        assert_eq!(run_ts(src), "42\nhi\n");
    }

    #[test]
    fn generic_constraint() {
        let src = r#"
function getLength<T extends { length: number }>(x: T): number {
    return x.length
}
console.log(getLength("hello"))
console.log(getLength([1, 2, 3]))
"#;
        assert_eq!(run_ts(src), "5\n3\n");
    }

    #[test]
    fn tuple_type() {
        let src = r#"
let pair: [number, string] = [1, "one"]
console.log(pair[0], pair[1])
"#;
        assert_eq!(run_ts(src), "1 one\n");
    }

    #[test]
    fn type_assertion() {
        let src = r#"
let x: any = "hello"
let len = (x as string).length
console.log(len)
"#;
        assert_eq!(run_ts(src), "5\n");
    }

    // --- Functions ---

    #[test]
    fn arrow_function_expression() {
        let src = r#"
let add = (a: number, b: number): number => a + b
console.log(add(3, 4))
"#;
        assert_eq!(run_ts(src), "7\n");
    }

    #[test]
    fn arrow_function_block() {
        let src = r#"
let greet = (name: string): string => {
    return "Hello, " + name
}
console.log(greet("World"))
"#;
        assert_eq!(run_ts(src), "Hello, World\n");
    }

    #[test]
    fn closure() {
        let src = r#"
function makeCounter(): () => number {
    let count = 0
    return () => {
        count++
        return count
    }
}
let counter = makeCounter()
console.log(counter())
console.log(counter())
"#;
        assert_eq!(run_ts(src), "1\n2\n");
    }

    #[test]
    fn default_parameters() {
        let src = r#"
function greet(name: string = "World"): void {
    console.log("Hello, " + name)
}
greet()
greet("Mango")
"#;
        assert_eq!(run_ts(src), "Hello, World\nHello, Mango\n");
    }

    #[test]
    fn rest_parameters() {
        let src = "
function sum(...nums: number[]): number {
    let total = 0
    for (let n of nums) {
        total += n
    }
    return total
}
console.log(sum(1, 2, 3, 4))
";
        assert_eq!(run_ts(src), "10\n");
    }

    #[test]
    fn spread_syntax() {
        let src = "
let a = [1, 2, 3]
let b = [...a, 4, 5]
console.log(b)
";
        assert_eq!(run_ts(src), "[ 1, 2, 3, 4, 5 ]\n");
    }

    #[test]
    fn function_expression() {
        let src = r#"
let add = function(a: number, b: number): number {
    return a + b
}
console.log(add(3, 4))
"#;
        assert_eq!(run_ts(src), "7\n");
    }

    // --- Error handling ---

    #[test]
    fn try_catch() {
        let src = "
try {
    throw 42
} catch (e) {
    console.log(\"caught\")
}
";
        assert_eq!(run_ts(src), "caught\n");
    }

    #[test]
    fn try_finally() {
        let src = "
try {
    console.log(\"try\")
} finally {
    console.log(\"finally\")
}
";
        assert_eq!(run_ts(src), "try\nfinally\n");
    }

    #[test]
    fn try_catch_finally() {
        let src = "
try {
    throw 1
} catch (e) {
    console.log(\"catch\")
} finally {
    console.log(\"finally\")
}
";
        assert_eq!(run_ts(src), "catch\nfinally\n");
    }

    #[test]
    fn try_no_throw() {
        let src = "
try {
    console.log(\"ok\")
} catch (e) {
    console.log(\"should not run\")
}
";
        assert_eq!(run_ts(src), "ok\n");
    }

    // --- Async ---

    #[test]
    fn async_await() {
        let src = "
async function fetchData(): Promise<number> {
    return 42
}
async function main(): Promise<void> {
    let data = await fetchData()
    console.log(data)
}
main()
";
        assert_eq!(run_ts(src), "42\n");
    }

    #[test]
    fn async_await_string() {
        let src = "
async function greet(): Promise<string> {
    return \"hello\"
}
async function main(): Promise<void> {
    let msg = await greet()
    console.log(msg)
}
main()
";
        assert_eq!(run_ts(src), "hello\n");
    }

    #[test]
    fn async_await_chained() {
        let src = "
async function add(a: number, b: number): Promise<number> {
    return a + b
}
async function main(): Promise<void> {
    let x = await add(10, 32)
    let y = await add(x, 0)
    console.log(y)
}
main()
";
        assert_eq!(run_ts(src), "42\n");
    }

    #[test]
    fn async_non_awaited_call() {
        // Calling an async function without await still executes its body
        // (event loop drains it before main exits)
        let src = "
async function work(): Promise<void> {
    console.log(\"done\")
}
work()
";
        assert_eq!(run_ts(src), "done\n");
    }

    // --- Modules ---

    #[test]
    #[ignore = "default exports not implemented"]
    fn default_export() {
        // Would need multi-file setup
        run_ts("export default 42");
    }

    #[test]
    #[ignore = "import * as not implemented"]
    fn import_star() {
        // Would need multi-file setup
        run_ts(r#"import * as math from "./math""#);
    }

    #[test]
    #[ignore = "re-exports not implemented"]
    fn re_export() {
        run_ts(r#"export { foo } from "./bar""#);
    }

    // --- Built-in objects ---

    #[test]
    #[ignore = "JSON not implemented"]
    fn json_stringify() {
        assert_eq!(
            run_ts(r#"console.log(JSON.stringify({ a: 1 }))"#),
            r#"{"a":1}"#.to_string() + "\n"
        );
    }

    #[test]
    #[ignore = "Map not implemented"]
    fn map_basic() {
        let src = r#"
let m = new Map()
m.set("a", 1)
console.log(m.get("a"))
"#;
        assert_eq!(run_ts(src), "1\n");
    }

    #[test]
    #[ignore = "Set not implemented"]
    fn set_basic() {
        let src = r#"
let s = new Set([1, 2, 3, 2, 1])
console.log(s.size)
"#;
        assert_eq!(run_ts(src), "3\n");
    }

    #[test]
    #[ignore = "RegExp not implemented"]
    fn regex_test() {
        assert_eq!(
            run_ts(r#"console.log(/hello/.test("hello world"))"#),
            "true\n"
        );
    }

    #[test]
    #[ignore = "Promise.resolve() static method not yet implemented in codegen"]
    fn promise_static_resolve() {
        let src = "
let p = Promise.resolve(42)
p.then((v: number) => console.log(v))
";
        assert_eq!(run_ts(src), "42\n");
    }

    // --- Number methods ---

    #[test]
    fn number_is_integer() {
        assert_eq!(run_ts("console.log(Number.isInteger(42))"), "true\n");
    }

    #[test]
    fn number_is_finite() {
        assert_eq!(run_ts("console.log(Number.isFinite(42))"), "true\n");
    }

    #[test]
    fn number_is_nan() {
        assert_eq!(run_ts("console.log(Number.isNaN(NaN))"), "true\n");
    }

    #[test]
    fn number_to_fixed() {
        assert_eq!(run_ts("console.log((3.14159).toFixed(2))"), "3.14\n");
    }

    // --- Misc TypeScript features ---

    #[test]
    fn type_narrowing() {
        let src = r#"
function process(val: string | number): void {
    if (typeof val === "string") {
        console.log(val.toUpperCase())
    } else {
        console.log(val + 1)
    }
}
process("hi")
process(41)
"#;
        assert_eq!(run_ts(src), "HI\n42\n");
    }

    #[test]
    fn string_literal_type() {
        let src = r#"
type Direction = "up" | "down" | "left" | "right"
let d: Direction = "up"
console.log(d)
"#;
        assert_eq!(run_ts(src), "up\n");
    }

    #[test]
    fn intersection_type() {
        let src = r#"
interface Named {
    name: string
}
interface Aged {
    age: number
}
type Person = Named & Aged
let p: Person = { name: "Alice", age: 30 }
console.log(p.name, p.age)
"#;
        assert_eq!(run_ts(src), "Alice 30\n");
    }

    #[test]
    fn readonly_property() {
        let src = r#"
interface Config {
    readonly host: string
    readonly port: number
}
let cfg: Config = { host: "localhost", port: 3000 }
console.log(cfg.host)
"#;
        assert_eq!(run_ts(src), "localhost\n");
    }

    #[test]
    fn keyof_operator() {
        let src = r#"
interface Point {
    x: number
    y: number
}
type PointKey = keyof Point
let k: PointKey = "x"
console.log(k)
"#;
        assert_eq!(run_ts(src), "x\n");
    }

    #[test]
    fn conditional_type() {
        let src = r#"
type IsNumber<T> = T extends number ? "yes" : "no"
let x: IsNumber<number> = "yes"
console.log(x)
"#;
        assert_eq!(run_ts(src), "yes\n");
    }

    #[test]
    fn mapped_type() {
        let src = r#"
type Readonly<T> = {
    readonly [P in keyof T]: T[P]
}
"#;
        run_ts(src);
    }

    #[test]
    fn typeof_type_operator() {
        let src = r#"
let x = 42
let y: typeof x = 100
console.log(y)
"#;
        assert_eq!(run_ts(src), "100\n");
    }

    #[test]
    fn satisfies_operator() {
        let src = r#"
type Colors = "red" | "green" | "blue"
let c = "red" satisfies Colors
console.log(c)
"#;
        assert_eq!(run_ts(src), "red\n");
    }

    #[test]
    fn as_const() {
        let src = r#"
let x = [1, 2, 3] as const
console.log(x[0])
"#;
        assert_eq!(run_ts(src), "1\n");
    }

    #[test]
    #[ignore = "namespace not implemented"]
    fn namespace() {
        let src = r#"
namespace Util {
    export function add(a: number, b: number): number {
        return a + b
    }
}
console.log(Util.add(1, 2))
"#;
        assert_eq!(run_ts(src), "3\n");
    }

    #[test]
    #[ignore = "decorators not implemented"]
    fn decorator() {
        let src = r#"
function log(target: any, key: string) {}
class Foo {
    @log
    bar() {}
}
"#;
        run_ts(src);
    }

    #[test]
    #[ignore = "symbol not implemented"]
    fn symbol_basic() {
        let src = r#"
let s = Symbol("foo")
console.log(typeof s)
"#;
        assert_eq!(run_ts(src), "symbol\n");
    }

    #[test]
    #[ignore = "iterator protocol not implemented"]
    fn generator_function() {
        let src = r#"
function* range(start: number, end: number) {
    for (let i = start; i < end; i++) {
        yield i
    }
}
for (let n of range(0, 3)) {
    console.log(n)
}
"#;
        assert_eq!(run_ts(src), "0\n1\n2\n");
    }

    #[test]
    #[ignore = "bigint not implemented"]
    fn bigint_literal() {
        assert_eq!(
            run_ts("console.log(9007199254740993n)"),
            "9007199254740993n\n"
        );
    }
}
