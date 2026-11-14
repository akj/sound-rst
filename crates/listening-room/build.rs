//! With the `binaural` feature: fetches Steam Audio's library, links it,
//! and copies `phonon.dll` next to the program, as tolk-sys does for
//! Tolk's DLLs. Without the feature it does nothing.

fn main() {
    println!("cargo::rerun-if-changed=build.rs");
    #[cfg(feature = "binaural")]
    steam_audio::install();
}

#[cfg(feature = "binaural")]
mod steam_audio {
    use std::fs;
    use std::io::Read;
    use std::path::{Path, PathBuf};
    use std::process::Command;

    use sha2::{Digest, Sha256};

    /// The release audionimbus 0.17 binds to, and its published SHA-256.
    const VERSION: &str = "4.8.1";
    const SHA256: &str = "4a0aa5ec1176f38f0b0993a37c2259d9e86f27e22d5e24f83ec4c3cb9a1d5449";

    pub fn install() {
        let target = std::env::var("TARGET").unwrap();
        if target != "x86_64-pc-windows-msvc" {
            panic!("The binaural feature is set up for x86_64-pc-windows-msvc only, not {target}.");
        }
        let out_dir = PathBuf::from(std::env::var("OUT_DIR").unwrap());
        // OUT_DIR is <target>/<profile>/build/<crate>-<hash>/out.
        let profile_dir = out_dir.ancestors().nth(3).unwrap().to_path_buf();
        let cache = profile_dir.parent().unwrap().join("steamaudio-cache");
        let zip_path = cache.join(format!("steamaudio_{VERSION}.zip"));
        if !has_hash(&zip_path) {
            download(&zip_path);
        }
        let lib_dir = out_dir.join("lib");
        fs::create_dir_all(&lib_dir).unwrap();
        let mut archive = zip::ZipArchive::new(fs::File::open(&zip_path).unwrap()).unwrap();
        for name in ["phonon.dll", "phonon.lib"] {
            let mut entry = archive
                .by_name(&format!("steamaudio/lib/windows-x64/{name}"))
                .unwrap_or_else(|e| panic!("{name} is not in {}: {e}", zip_path.display()));
            let mut bytes = Vec::new();
            entry.read_to_end(&mut bytes).unwrap();
            fs::write(lib_dir.join(name), &bytes).unwrap();
        }
        // The program and its tests find the DLL beside them, or through
        // this search path when Cargo runs them.
        fs::copy(lib_dir.join("phonon.dll"), profile_dir.join("phonon.dll")).unwrap();
        // The licences of the code inside phonon.dll travel with it.
        let mut notices = String::new();
        archive
            .by_name("steamaudio/THIRDPARTY.md")
            .unwrap()
            .read_to_string(&mut notices)
            .unwrap();
        fs::write(profile_dir.join("phonon-THIRDPARTY.md"), notices).unwrap();
        println!("cargo::rustc-link-search=native={}", lib_dir.display());
    }

    fn has_hash(path: &Path) -> bool {
        let Ok(bytes) = fs::read(path) else {
            return false;
        };
        let digest = Sha256::digest(&bytes);
        let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
        hex == SHA256
    }

    fn download(path: &Path) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let url = format!(
            "https://github.com/ValveSoftware/steam-audio/releases/download/v{VERSION}/steamaudio_{VERSION}.zip"
        );
        let part = path.with_extension("part");
        println!("cargo::warning=Downloading Steam Audio {VERSION} (about 180 MB, once).");
        let status = Command::new("curl")
            .args(["-L", "-f", "-s", "-S", "--retry", "3", "-o"])
            .arg(&part)
            .arg(&url)
            .status()
            .unwrap_or_else(|e| panic!("cannot run curl to download {url}: {e}"));
        assert!(status.success(), "curl could not download {url}");
        fs::rename(&part, path).unwrap();
        assert!(
            has_hash(path),
            "{} does not have the published SHA-256 {SHA256}",
            path.display()
        );
    }
}
