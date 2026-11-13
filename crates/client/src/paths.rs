//! Where the game data and the player's files live.

use std::ffi::OsString;
use std::fmt;
use std::path::{Path, PathBuf};

use crate::log::Log;

/// The file whose presence marks a directory as the game root.
const ROOT_MARKER: &str = "res/rules.txt";

/// The game root could not be found, or the given one holds no game data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RootError {
    pub tried: Vec<PathBuf>,
}

impl fmt::Display for RootError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(
            f,
            "SoundRTS cannot find its game data (the folder that holds res/rules.txt)."
        )?;
        for path in &self.tried {
            writeln!(f, "  not found in: {}", path.display())?;
        }
        write!(
            f,
            "Pass --root <folder>, or set the SOUNDRTS_ROOT environment variable, or put the \
             res, cfg, mods and single folders next to the program. In a development checkout, \
             run scripts/fetch-upstream.ps1, then use --root vendor/soundrts-upstream."
        )
    }
}

impl std::error::Error for RootError {}

fn is_root(dir: &Path) -> bool {
    dir.join(ROOT_MARKER).is_file()
}

/// Finds the game root: `--root`, else `SOUNDRTS_ROOT`, else the
/// executable's folder, else the current folder. An explicitly given
/// folder must hold the game data too.
pub fn find_root(
    cli: Option<&Path>,
    env: Option<OsString>,
    exe_dir: Option<&Path>,
    cwd: &Path,
) -> Result<PathBuf, RootError> {
    let explicit = cli
        .map(Path::to_path_buf)
        .or_else(|| env.filter(|v| !v.is_empty()).map(PathBuf::from));
    if let Some(dir) = explicit {
        let dir = if dir.is_absolute() {
            dir
        } else {
            cwd.join(dir)
        };
        return if is_root(&dir) {
            Ok(dir)
        } else {
            Err(RootError { tried: vec![dir] })
        };
    }
    let mut tried = Vec::new();
    for dir in exe_dir.into_iter().chain([cwd]) {
        if is_root(dir) {
            return Ok(dir.to_path_buf());
        }
        tried.push(dir.to_path_buf());
    }
    Err(RootError { tried })
}

/// The player's folder: `<root>/user` if it exists, else
/// `%APPDATA%\SoundRTS`, else `~/.SoundRTS`, else `<root>/user`.
pub fn find_user_dir(root: &Path, appdata: Option<OsString>, home: Option<OsString>) -> PathBuf {
    let local = root.join("user");
    if local.exists() {
        local
    } else if let Some(appdata) = appdata {
        PathBuf::from(appdata).join("SoundRTS")
    } else if let Some(home) = home {
        PathBuf::from(home).join(".SoundRTS")
    } else {
        local
    }
}

/// The game root and the player's folder, with the files inside them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Paths {
    pub root: PathBuf,
    pub user: PathBuf,
}

impl Paths {
    /// Finds both folders from the process environment.
    pub fn from_environment(cli_root: Option<&Path>) -> Result<Paths, RootError> {
        let cwd = std::env::current_dir().unwrap_or_default();
        let exe = std::env::current_exe().ok();
        let exe_dir = exe.as_deref().and_then(Path::parent);
        let root = find_root(cli_root, std::env::var_os("SOUNDRTS_ROOT"), exe_dir, &cwd)?;
        let user = find_user_dir(&root, std::env::var_os("APPDATA"), std::env::var_os("HOME"));
        Ok(Paths { root, user })
    }

    pub fn tmp(&self) -> PathBuf {
        self.user.join("tmp")
    }
    pub fn replays(&self) -> PathBuf {
        self.user.join("replays")
    }
    pub fn downloaded(&self) -> PathBuf {
        self.user.join("downloaded")
    }
    pub fn client_log(&self) -> PathBuf {
        self.tmp().join("client.log")
    }
    pub fn config_file(&self) -> PathBuf {
        self.user.join("SoundRTS.ini")
    }
    pub fn campaigns_config(&self) -> PathBuf {
        self.user.join("campaigns.ini")
    }
    pub fn savegame(&self) -> PathBuf {
        self.user.join("savegame")
    }
    pub fn custom_bindings(&self) -> PathBuf {
        self.user.join("bindings.txt")
    }
    pub fn language_file(&self) -> PathBuf {
        self.root.join("cfg").join("language.txt")
    }
    pub fn parameters_file(&self) -> PathBuf {
        self.root.join("cfg").join("parameters.toml")
    }

    /// Creates the player's folders, as the game expects them. A folder
    /// that cannot be made is reported and skipped.
    pub fn create_user_folders(&self, log: &Log) {
        let downloaded = self.downloaded();
        let dirs = [
            self.user.clone(),
            self.tmp(),
            self.replays(),
            downloaded.clone(),
            self.user.join("single"),
            self.user.join("multi"),
            self.user.join("mods"),
            self.user.join("packages"),
            downloaded.join("multi"),
            downloaded.join("packages"),
        ];
        for dir in dirs {
            if !dir.exists()
                && let Err(e) = std::fs::create_dir(&dir)
            {
                log.warning(format!("cannot make dir: {} ({e})", dir.display()));
            }
        }
    }

    /// The package folders and zip files after the base package, in
    /// loading order: each additional path, then the entries of its
    /// `packages` folder.
    pub fn additional_package_paths(&self, base: &str, additional: &[String]) -> Vec<PathBuf> {
        let base = self.root.join(base);
        let mut result: Vec<PathBuf> = additional.iter().map(|p| self.root.join(p)).collect();
        for rp in additional {
            let pp = self.root.join(rp).join("packages");
            let Ok(entries) = std::fs::read_dir(&pp) else {
                continue;
            };
            let mut names: Vec<OsString> = entries.flatten().map(|e| e.file_name()).collect();
            names.sort();
            for name in names {
                let p = pp.join(name);
                if p != base {
                    result.push(p);
                }
            }
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_explicit_root_wins_and_must_hold_the_data() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("res")).unwrap();
        std::fs::write(dir.path().join("res/rules.txt"), "").unwrap();
        let other = tempfile::tempdir().unwrap();
        assert_eq!(
            find_root(Some(dir.path()), None, Some(other.path()), other.path()),
            Ok(dir.path().to_path_buf())
        );
        assert_eq!(
            find_root(Some(other.path()), None, Some(dir.path()), dir.path()),
            Err(RootError {
                tried: vec![other.path().to_path_buf()]
            })
        );
        assert_eq!(
            find_root(None, Some(dir.path().into()), None, other.path()),
            Ok(dir.path().to_path_buf())
        );
    }

    #[test]
    fn the_executable_folder_comes_before_the_current_folder() {
        let exe = tempfile::tempdir().unwrap();
        let cwd = tempfile::tempdir().unwrap();
        for d in [exe.path(), cwd.path()] {
            std::fs::create_dir_all(d.join("res")).unwrap();
            std::fs::write(d.join("res/rules.txt"), "").unwrap();
        }
        assert_eq!(
            find_root(None, None, Some(exe.path()), cwd.path()),
            Ok(exe.path().to_path_buf())
        );
        std::fs::remove_file(exe.path().join("res/rules.txt")).unwrap();
        assert_eq!(
            find_root(None, None, Some(exe.path()), cwd.path()),
            Ok(cwd.path().to_path_buf())
        );
    }

    #[test]
    fn a_missing_root_error_says_how_to_get_the_data() {
        let empty = tempfile::tempdir().unwrap();
        let error = find_root(None, None, None, empty.path()).unwrap_err();
        assert!(error.to_string().contains("scripts/fetch-upstream.ps1"));
    }

    #[test]
    fn the_user_folder_follows_upstream_precedence() {
        let root = tempfile::tempdir().unwrap();
        assert_eq!(
            find_user_dir(root.path(), Some("A".into()), Some("H".into())),
            PathBuf::from("A").join("SoundRTS")
        );
        assert_eq!(
            find_user_dir(root.path(), None, Some("H".into())),
            PathBuf::from("H").join(".SoundRTS")
        );
        std::fs::create_dir(root.path().join("user")).unwrap();
        assert_eq!(
            find_user_dir(root.path(), Some("A".into()), None),
            root.path().join("user")
        );
    }
}
