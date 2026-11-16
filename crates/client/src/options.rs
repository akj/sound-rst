//! Command line options.

use std::ffi::OsString;
use std::path::PathBuf;

/// What the command line asks for. Unknown words are ignored, as upstream's
/// option parser left positional arguments alone.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Options {
    pub root: Option<PathBuf>,
    /// Overrides the mods of `settings.toml` for this run.
    pub mods: Option<String>,
    /// Overrides the soundpacks of `settings.toml` for this run.
    pub soundpacks: Option<String>,
    pub probe: bool,
    /// Downloads the game data if the game finds none, then exits.
    pub fetch_data: bool,
    /// Replays a game's record or a save, says how it went, then exits.
    pub check_record: Option<PathBuf>,
}

/// A command line the program cannot follow.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsageError(pub String);

impl std::fmt::Display for UsageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}\nusage: soundrts [--root <folder>] [-m|--mods <mods>] [-s|--soundpacks <soundpacks>] [--probe] [--fetch-data] [--check-record <file>]",
            self.0
        )
    }
}

impl Options {
    /// Parses the arguments after the program name.
    pub fn parse(args: impl IntoIterator<Item = OsString>) -> Result<Options, UsageError> {
        let mut options = Options::default();
        let mut args = args.into_iter();
        while let Some(arg) = args.next() {
            let arg = arg.to_string_lossy().into_owned();
            let (name, inline) = match arg.split_once('=') {
                Some((n, v)) if n.starts_with("--") => (n.to_owned(), Some(v.to_owned())),
                _ => (arg.clone(), None),
            };
            let mut value = |name: &str| -> Result<OsString, UsageError> {
                match &inline {
                    Some(v) => Ok(v.into()),
                    None => args
                        .next()
                        .ok_or_else(|| UsageError(format!("{name} needs a value"))),
                }
            };
            match name.as_str() {
                "--root" => options.root = Some(PathBuf::from(value("--root")?)),
                "-m" | "--mods" => {
                    options.mods = Some(value("--mods")?.to_string_lossy().into_owned())
                }
                "-s" | "--soundpacks" => {
                    options.soundpacks = Some(value("--soundpacks")?.to_string_lossy().into_owned())
                }
                "--probe" => options.probe = true,
                "--fetch-data" => options.fetch_data = true,
                "--check-record" => {
                    options.check_record = Some(PathBuf::from(value("--check-record")?))
                }
                _ => {}
            }
        }
        Ok(options)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Options, UsageError> {
        Options::parse(args.iter().map(OsString::from))
    }

    #[test]
    fn options_take_separate_or_inline_values() {
        assert_eq!(
            parse(&[
                "--root",
                "r",
                "-m",
                "orc",
                "--soundpacks=sp",
                "--probe",
                "x"
            ]),
            Ok(Options {
                root: Some("r".into()),
                mods: Some("orc".into()),
                soundpacks: Some("sp".into()),
                probe: true,
                fetch_data: false,
                check_record: None,
            })
        );
        assert_eq!(
            parse(&["--mods=a b"]).map(|o| o.mods),
            Ok(Some("a b".into()))
        );
        assert!(parse(&["--mods"]).is_err());
        assert_eq!(
            parse(&["--check-record", "game.jsonl"]).map(|o| o.check_record),
            Ok(Some("game.jsonl".into()))
        );
        assert!(parse(&["--check-record"]).is_err());
        assert_eq!(
            parse(&["--fetch-data"]),
            Ok(Options {
                fetch_data: true,
                ..Options::default()
            })
        );
    }
}
