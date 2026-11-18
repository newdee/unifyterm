//! Gives the program the commit it is built from (`UNI_TERM_COMMIT`). On
//! Windows, puts the ConPTY that `tools/fetch-conpty.ps1` fetched beside
//! the built program, where the pty library looks for it before it falls
//! back to Windows' own (older) one.

use std::path::Path;

/// `conpty.dll` starts `OpenConsole.exe` from its own directory.
const FILES: [&str; 2] = ["conpty.dll", "OpenConsole.exe"];

include!("../../build/commit.rs");

fn main() {
    println!("cargo:rustc-env=UNI_TERM_COMMIT={}", commit());
    println!("cargo:rerun-if-changed=../../vendor/conpty/VERSION");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let arch = match std::env::var("CARGO_CFG_TARGET_ARCH").as_deref() {
        Ok("x86_64") => "x64",
        Ok("aarch64") => "arm64",
        _ => return,
    };
    let vendor = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../vendor/conpty").join(arch);
    // OUT_DIR is target/<profile>/build/<package>-<hash>/out.
    let out_dir = std::env::var("OUT_DIR").expect("cargo sets OUT_DIR");
    let Some(profile) = Path::new(&out_dir).ancestors().nth(3) else { return };
    for name in FILES {
        let source = vendor.join(name);
        if !source.is_file() {
            println!(
                "cargo:warning=vendor/conpty/{arch}/{name} is missing: Windows' own ConPTY will be used. Run tools/fetch-conpty.ps1."
            );
            return;
        }
        // Beside the program, and beside the test programs (target/<profile>/deps).
        for dir in [profile.to_path_buf(), profile.join("deps")] {
            if dir.is_dir()
                && let Err(error) = std::fs::copy(&source, dir.join(name))
            {
                println!("cargo:warning=copy {name} to {}: {error}", dir.display());
            }
        }
    }
}
