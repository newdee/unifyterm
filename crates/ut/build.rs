//! Gives the program the commit it is built from (`UNIFYTERM_COMMIT`), and
//! tells the Microsoft linker not to make an import library for `ut.exe`.
//!
//! A linked crate exports a symbol, and for a program that exports anything
//! the linker writes `ut.lib` and `ut.exp` beside it and says so on every
//! build. Nothing links against `ut`, so neither file is wanted.

include!("../../build/commit.rs");

fn main() {
    println!("cargo:rustc-env=UNIFYTERM_COMMIT={}", commit());
    println!("cargo:rerun-if-changed=build.rs");
    let msvc = std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc");
    if msvc {
        println!("cargo:rustc-link-arg-bins=/NOIMPLIB");
        println!("cargo:rustc-link-arg-bins=/NOEXP");
    }
}
