//! Gives the program the commit it is built from (`UNIFYTERM_COMMIT`). On
//! Windows, puts the ConPTY that `tools/fetch-conpty.ps1` fetched beside
//! the built program, where the pty library looks for it before it falls
//! back to Windows' own (older) one, and puts the icon into the program.

use std::path::Path;

/// `conpty.dll` starts `OpenConsole.exe` from its own directory.
const FILES: [&str; 2] = ["conpty.dll", "OpenConsole.exe"];

include!("../../build/commit.rs");

/// The icon and the name Windows shows, in the program file itself. The
/// window, the taskbar, Alt-Tab and Explorer all take the icon from the
/// program (GPUI loads the icon resource numbered 1): an installer's
/// shortcut has an icon of its own, and nothing else has. A build without
/// it fails, so that a program without an icon is never released.
#[cfg(windows)]
fn embed_icon() {
    let icon = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packaging/icons/unifyterm.ico");
    println!("cargo:rerun-if-changed={}", icon.display());
    let mut resource = winresource::WindowsResource::new();
    resource.set_icon_with_id(&icon.to_string_lossy(), "1");
    resource.set("ProductName", "UnifyTerm");
    resource.set("FileDescription", "UnifyTerm");
    resource.compile().expect("the icon goes into the program (needs rc.exe, which comes with the Windows SDK)");
}

fn main() {
    println!("cargo:rustc-env=UNIFYTERM_COMMIT={}", commit());
    println!("cargo:rerun-if-changed=../../vendor/conpty/VERSION");
    #[cfg(windows)]
    embed_icon();
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
