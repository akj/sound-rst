//! Copies NVDA's controller client (`scripts/fetch-nvda-controller.ps1`)
//! next to the program when it has been fetched. Without it, the game
//! speaks through Tolk only.

use std::path::PathBuf;

fn main() {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("manifest dir"));
    let vendor = manifest.join("../../vendor");
    // The whole folder, so a fetch made after a build is noticed.
    println!("cargo:rerun-if-changed={}", vendor.display());
    println!("cargo:rerun-if-changed=build.rs");
    let dll = vendor.join("nvda-controller/nvdaControllerClient.dll");
    if std::env::var("CARGO_CFG_WINDOWS").is_err() || !dll.is_file() {
        return;
    }
    // OUT_DIR is target/<profile>/build/<crate>/out: the program is three
    // folders up.
    let out = PathBuf::from(std::env::var("OUT_DIR").expect("out dir"));
    let Some(profile) = out.ancestors().nth(3) else {
        return;
    };
    if let Err(e) = std::fs::copy(&dll, profile.join("nvdaControllerClient.dll")) {
        println!("cargo:warning=cannot copy {}: {e}", dll.display());
    }
}
