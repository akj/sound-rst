//! `--probe`: reports what the client found, without opening a window,
//! speaking, playing, or writing a file.

use std::fmt::Write;
use std::rc::Rc;

use crate::client::{app_title, open_resources};
use crate::config::Config;
use crate::log::Log;
use crate::options::Options;
use crate::parameters::Parameters;
use crate::paths::Paths;
use crate::resource::preferred_language;

/// The probe report.
pub fn report(options: &Options) -> Result<String, String> {
    let paths = Paths::from_environment(options.root.as_deref()).map_err(|e| e.to_string())?;
    let log = Log::stderr();
    let params = Rc::new(
        Parameters::load(&paths.parameters_file(), &log)
            .map_err(|e| format!("cannot read {}: {e}", paths.parameters_file().display()))?,
    );
    let config_file = paths.config_file();
    let config = match std::fs::read(&config_file) {
        Ok(bytes) => Config::from_ini(&String::from_utf8_lossy(&bytes), &log).0,
        Err(_) => Config::default(),
    };
    let mods = options.mods.clone().unwrap_or_else(|| config.mods.clone());
    let soundpacks = options
        .soundpacks
        .clone()
        .unwrap_or_else(|| config.soundpacks.clone());
    let preferred = preferred_language(&paths.language_file(), &log);
    let res = open_resources(&paths, params, &mods, &soundpacks, preferred.clone(), &log);

    let mut out = String::new();
    let _ = writeln!(out, "{}", app_title(&res.mods, &res.soundpacks).trim_end());
    let _ = writeln!(out, "game root: {}", paths.root.display());
    let exists = if paths.user.exists() {
        ""
    } else {
        " (not created yet)"
    };
    let _ = writeln!(out, "user folder: {}{exists}", paths.user.display());
    let _ = writeln!(
        out,
        "settings: {}{}",
        config_file.display(),
        if config_file.exists() {
            ""
        } else {
            " (not created yet)"
        }
    );
    let _ = writeln!(
        out,
        "language: {} (preferred: {})",
        res.language,
        preferred.as_deref().unwrap_or("unknown")
    );
    let _ = writeln!(
        out,
        "mods: {:?}, soundpacks: {:?}",
        res.mods, res.soundpacks
    );
    match crate::rodio_output::describe_default_device() {
        Ok(device) => {
            let _ = writeln!(out, "audio output: {device}");
        }
        Err(e) => {
            let _ = writeln!(out, "audio output: none ({e})");
        }
    }
    let speech = crate::tts::probe_speech();
    let _ = writeln!(out, "speech backend: {}", speech.backend);
    let _ = writeln!(
        out,
        "speaking through: {}",
        speech
            .output
            .as_deref()
            .unwrap_or("nothing (no screen reader and no system voice)")
    );
    for note in speech.notes {
        let _ = writeln!(out, "  {note}");
    }
    Ok(out)
}
