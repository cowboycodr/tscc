use std::env;
use std::path::PathBuf;

fn main() {
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let runtime_lib = out_dir.join("libtscc_runtime.a");

    // Compile runtime/runtime.c into a static library once at cargo build time.
    // The resulting .a path is passed to lib.rs via the RUNTIME_LIB_PATH env var,
    // where it is embedded as bytes with include_bytes!() — no cc invocation needed
    // at user compile time.
    cc::Build::new()
        .file("runtime/runtime.c")
        .opt_level(2)
        .compile("tscc_runtime");

    // cc::Build::compile() writes libtscc_runtime.a into OUT_DIR and prints the
    // necessary cargo:rustc-link-* directives automatically.  We just need to
    // expose the path so lib.rs can embed it.
    println!("cargo:rustc-env=RUNTIME_LIB_PATH={}", runtime_lib.display());

    // Re-run this build script only when the runtime source changes.
    println!("cargo:rerun-if-changed=runtime/runtime.c");
}
