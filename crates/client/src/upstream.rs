//! The data of the original game (rules, maps, sounds, translations),
//! downloaded at the pinned commit when the game finds none.
//!
//! It is not part of this program, so the game fetches GitHub's zip of the
//! commit into the player's folder. The zip comes from outside: every read
//! is bounded, and only the four folders the game reads are extracted.
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::string_slice,
    clippy::arithmetic_side_effects
)]

use std::fs::{self, File, OpenOptions, TryLockError};
use std::io::{Cursor, ErrorKind, Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use zip::ZipArchive;

use crate::log::Log;
use crate::notice;
use crate::package::{confined, too_many_entries};
use crate::paths::{self, Paths};
use crate::tts::{SpeechBackend, screen_reader_speech};

/// The commit of the original game whose data the port reads.
pub const COMMIT: &str = include_str!("../../../scripts/upstream-commit.txt").trim_ascii();

/// The folders of the zip that the game reads. The zip also has a `user`
/// folder, which would become the settings folder inside the game root.
const FOLDERS: [&str; 4] = ["res", "cfg", "mods", "single"];

/// The file that marks a folder as the game root.
const ROOT_MARKER: &str = "res/rules.txt";

/// The most parts an entry's path may have, top folder included. The data
/// has at most 6. It bounds the folders one entry can make.
const MAX_PATH_PARTS: usize = 16;

/// The names Windows gives to devices, whatever follows a dot: a file of
/// such a name is the device.
const DEVICES: [&str; 4] = ["con", "prn", "aux", "nul"];

/// A connection that sends nothing for this long has stalled.
const STALL_TIMEOUT: Duration = Duration::from_secs(30);

/// The longest the whole download may take.
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(15 * 60);

/// Spoken progress comes at most this often.
const PROGRESS_EVERY: Duration = Duration::from_secs(5);

/// The question asked when the game starts without its data.
const QUESTION: &str = "SoundRTS needs the data of the original game: its rules, maps and \
                        sounds. They are not part of this program.\n\nDownload them now from \
                        the original game's repository on GitHub? The download is about 10 \
                        megabytes.";

/// The most a download may hold and expand to.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    /// Bytes of the zip file.
    pub download: u64,
    /// Entries of the zip file, all folders included.
    pub entries: usize,
    /// Bytes of one extracted file.
    pub file: u64,
    /// Bytes of all extracted files.
    pub total: u64,
}

/// The limits of a real download, several times what the pinned commit
/// needs (10 MB zipped, 876 files, 10 MB extracted).
pub const LIMITS: Limits = Limits {
    download: 64 * 1024 * 1024,
    entries: 10_000,
    file: 32 * 1024 * 1024,
    total: 256 * 1024 * 1024,
};

/// Where the downloaded data lives in the player's folder.
pub fn data_dir(player: &Path) -> PathBuf {
    player.join("data").join(COMMIT)
}

/// GitHub's zip of the pinned commit.
pub fn archive_url() -> String {
    format!("https://github.com/soundmud/soundrts/archive/{COMMIT}.zip")
}

/// Downloads the data into the player's folder and returns the new game
/// root. `progress` gets the bytes received so far. Only over HTTPS: a
/// redirect to anything else is refused.
pub fn fetch(player: &Path, progress: &mut dyn FnMut(u64)) -> Result<PathBuf, String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .https_only(true)
        .timeout_resolve(Some(STALL_TIMEOUT))
        .timeout_connect(Some(STALL_TIMEOUT))
        .timeout_send_request(Some(STALL_TIMEOUT))
        .timeout_recv_response(Some(STALL_TIMEOUT))
        .timeout_per_read(Some(STALL_TIMEOUT))
        .timeout_global(Some(DOWNLOAD_TIMEOUT))
        .build()
        .into();
    let url = archive_url();
    let mut response = agent
        .get(&url)
        .call()
        .map_err(|e| format!("cannot download {url}: {e}"))?;
    install(
        response.body_mut().as_reader(),
        &player.join("data"),
        COMMIT,
        &LIMITS,
        progress,
    )
}

/// Installs the zip that `reader` gives as `<data>/<commit>` and returns
/// that folder. It is extracted into `<data>/<commit>.partial` first, which
/// is removed on any failure, so the data folder is complete or absent.
/// Safe to rerun, also after a run that stopped halfway.
///
/// One game installs at a time: a second is refused while the first works,
/// and finds the data there afterwards. Data that is complete is never
/// replaced: a running game may be reading it. The data of an older pin is
/// left for the same reason.
pub fn install(
    reader: impl Read,
    data: &Path,
    commit: &str,
    limits: &Limits,
    progress: &mut dyn FnMut(u64),
) -> Result<PathBuf, String> {
    let target = data.join(commit);
    let partial = data.join(format!("{commit}.partial"));
    fs::create_dir_all(data).map_err(|e| format!("cannot make {}: {e}", data.display()))?;
    let _lock = lock(data)?;
    if target.join(ROOT_MARKER).is_file() {
        return Ok(target);
    }
    remove_dir(&partial)?;
    let result = read_limited(reader, limits.download, progress)
        .and_then(|bytes| extract(bytes, &partial, limits))
        .and_then(|()| {
            if !partial.join(ROOT_MARKER).is_file() {
                return Err(format!("the download holds no {ROOT_MARKER}"));
            }
            // At most what an install that failed left: it has no rules.
            remove_dir(&target)?;
            fs::rename(&partial, &target)
                .map_err(|e| format!("cannot move the data to {}: {e}", target.display()))
        });
    if let Err(e) = result {
        let _ = fs::remove_dir_all(&partial);
        return Err(e);
    }
    Ok(target)
}

/// The right to install into `data`, until the file is closed or the
/// program ends, however it ends.
fn lock(data: &Path) -> Result<File, String> {
    let path = data.join("install.lock");
    let cannot = |e: std::io::Error| format!("cannot lock {}: {e}", path.display());
    let file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(&path)
        .map_err(cannot)?;
    match file.try_lock() {
        Ok(()) => Ok(file),
        Err(TryLockError::WouldBlock) => Err(
            "another SoundRTS is downloading the game data. Try again when it has finished."
                .to_owned(),
        ),
        Err(TryLockError::Error(e)) => Err(cannot(e)),
    }
}

/// Removes a folder if it is there.
fn remove_dir(dir: &Path) -> Result<(), String> {
    match fs::remove_dir_all(dir) {
        Err(e) if e.kind() != ErrorKind::NotFound => {
            Err(format!("cannot remove {}: {e}", dir.display()))
        }
        _ => Ok(()),
    }
}

/// Reads all of `reader`, at most `limit` bytes.
fn read_limited(
    mut reader: impl Read,
    limit: u64,
    progress: &mut dyn FnMut(u64),
) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    let mut buffer = vec![0_u8; 64 * 1024];
    loop {
        let n = match reader.read(&mut buffer) {
            Ok(0) => return Ok(bytes),
            Ok(n) => n,
            Err(e) if e.kind() == ErrorKind::Interrupted => continue,
            Err(e) => return Err(format!("the download stopped: {e}")),
        };
        bytes.extend_from_slice(buffer.get(..n).unwrap_or_default());
        let received = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
        if received > limit {
            return Err(format!("the download is larger than {limit} bytes"));
        }
        progress(received);
    }
}

/// Whether a part of an entry's path names a file or folder of its own on
/// Windows: not `.` or `..`, not a device (`NUL`, `COM1.txt`), and with no
/// dot or space at its end, which Windows drops.
fn plain(part: &str) -> bool {
    let stem = part.split('.').next().unwrap_or(part).trim_end();
    let stem = stem.to_ascii_lowercase();
    let port = ["com", "lpt"].iter().any(|p| {
        stem.strip_prefix(p)
            .is_some_and(|n| n.chars().count() == 1 && n != "0")
    });
    !(part.ends_with(['.', ' ']) || DEVICES.contains(&stem.as_str()) || port)
}

/// The first part of an entry's name, if the name is safe: relative, with
/// no drive and no backslash, at most [`MAX_PATH_PARTS`] parts, and every
/// part [`plain`]. A folder's name ends with one `/`.
fn top_of(name: &str) -> Option<&str> {
    let path = name.strip_suffix('/').unwrap_or(name);
    let mut parts = path.split('/');
    let top = parts.next().filter(|p| !p.is_empty())?;
    let safe = !name.contains(['\\', ':'])
        && path.split('/').count() <= MAX_PATH_PARTS
        && [top]
            .into_iter()
            .chain(parts)
            .all(|p| !p.is_empty() && plain(p));
    safe.then_some(top)
}

/// Extracts the entries under `<top folder>/{res,cfg,mods,single}/` into
/// `dest`, without the top folder. The limits hold for what is actually
/// extracted, whatever sizes the zip states.
fn extract(bytes: Vec<u8>, dest: &Path, limits: &Limits) -> Result<(), String> {
    let too_many = || format!("the download has more than {} entries", limits.entries);
    if too_many_entries(&bytes) {
        return Err(too_many());
    }
    let mut archive = ZipArchive::new(Cursor::new(bytes))
        .map_err(|e| format!("the download is not a zip file that can be read: {e}"))?;
    if archive.len() > limits.entries {
        return Err(too_many());
    }
    let mut names = Vec::with_capacity(archive.len());
    for i in 0..archive.len() {
        let entry = archive.by_index_raw(i).map_err(|e| e.to_string())?;
        names.push(entry.name().map_err(|e| e.to_string())?.into_owned());
    }
    let mut top = None;
    for name in &names {
        let this = top_of(name).ok_or_else(|| format!("unsafe path in the download: {name}"))?;
        if *top.get_or_insert(this) != this {
            return Err("the download has more than one top folder".to_owned());
        }
    }
    let top = top.ok_or("the download is empty")?;
    fs::create_dir_all(dest).map_err(|e| format!("cannot make {}: {e}", dest.display()))?;
    let mut total: u64 = 0;
    for (i, name) in names.iter().enumerate() {
        let Some(rest) = name
            .strip_prefix(top)
            .and_then(|r| r.strip_prefix('/'))
            .filter(|r| r.split('/').next().is_some_and(|f| FOLDERS.contains(&f)))
        else {
            continue;
        };
        let path =
            confined(dest, rest).ok_or_else(|| format!("unsafe path in the download: {name}"))?;
        let mut entry = archive.by_index(i).map_err(|e| e.to_string())?;
        let make = |dir: &Path| {
            fs::create_dir_all(dir).map_err(|e| format!("cannot make {}: {e}", dir.display()))
        };
        if entry.is_dir() {
            make(&path)?;
            continue;
        }
        if let Some(parent) = path.parent() {
            make(parent)?;
        }
        let mut file =
            File::create(&path).map_err(|e| format!("cannot write {}: {e}", path.display()))?;
        copy_limited(&mut entry, &mut file, name, limits, &mut total)?;
    }
    Ok(())
}

/// Copies one entry, within the limit of a file and of the whole download.
fn copy_limited(
    entry: &mut impl Read,
    file: &mut File,
    name: &str,
    limits: &Limits,
    total: &mut u64,
) -> Result<(), String> {
    let mut buffer = vec![0_u8; 64 * 1024];
    let mut written: u64 = 0;
    loop {
        let n = match entry.read(&mut buffer) {
            Ok(0) => return Ok(()),
            Ok(n) => n,
            Err(e) if e.kind() == ErrorKind::Interrupted => continue,
            Err(e) => return Err(format!("cannot extract {name}: {e}")),
        };
        let n64 = u64::try_from(n).unwrap_or(u64::MAX);
        written = written.saturating_add(n64);
        *total = total.saturating_add(n64);
        if written > limits.file {
            return Err(format!("{name} is larger than {} bytes", limits.file));
        }
        if *total > limits.total {
            return Err(format!(
                "the download expands to more than {} bytes",
                limits.total
            ));
        }
        file.write_all(buffer.get(..n).unwrap_or_default())
            .map_err(|e| format!("cannot write {name}: {e}"))?;
    }
}

/// What a normal start does when it finds no data.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MissingData {
    /// Ask the player whether to download it into this player's folder.
    Ask(PathBuf),
    /// Fail with the message that says how to get it.
    Fail,
}

/// The game asks only when no root was given explicitly (an explicit root
/// never falls back), there is a player's folder to download to, and the
/// player can be asked (see [`notice::can_ask`]).
pub fn on_missing_data(explicit_root: bool, player: Option<PathBuf>, can_ask: bool) -> MissingData {
    match player {
        Some(p) if !explicit_root && can_ask => MissingData::Ask(p),
        _ => MissingData::Fail,
    }
}

/// What the player hears of the download's progress.
pub fn spoken_progress(bytes: u64) -> String {
    match bytes / 1_000_000 {
        1 => "1 megabyte".to_owned(),
        n => format!("{n} megabytes"),
    }
}

/// Asks the player whether to download the data, then downloads it while
/// speaking its progress. A failure is told with Retry and Cancel. The new
/// game root, or `None` if the player said No or Cancel.
pub fn ask_and_fetch(player: &Path) -> Option<PathBuf> {
    if !notice::ask_yes_no(QUESTION) {
        return None;
    }
    loop {
        match fetch_speaking(player) {
            Ok(root) => return Some(root),
            Err(e) => {
                let text = format!("SoundRTS could not download the game data.\n\n{e}");
                if !notice::ask_retry(&text) {
                    return None;
                }
            }
        }
    }
}

/// [`fetch`], with progress spoken by the screen reader. The settings are
/// not loaded yet, so speech takes the plain route, and the words are
/// English because the translations come with the data. Without speech the
/// download goes on silently.
fn fetch_speaking(player: &Path) -> Result<PathBuf, String> {
    let mut speech = screen_reader_speech(false, Log::silent());
    let mut say = |text: &str| {
        let _ = speech.speak(text);
    };
    say("Downloading the game data.");
    let mut last = Instant::now();
    let result = fetch(player, &mut |bytes| {
        if last.elapsed() >= PROGRESS_EVERY {
            last = Instant::now();
            say(&spoken_progress(bytes));
        }
    });
    // The failure is also said: its message box can open behind the
    // program the player went to while waiting.
    say(match result {
        Ok(_) => "The game data is ready.",
        Err(_) => "The game data could not be downloaded.",
    });
    // The game's own speech starts next: this must have been said by then.
    speech.finish();
    result
}

/// `--fetch-data`: downloads the data unless the game already finds some,
/// and prints what it does. No speech, no window, no dialog. Whether the
/// game has its data now.
pub fn fetch_data(cli_root: Option<&Path>) -> bool {
    let error = match Paths::from_environment(cli_root) {
        Ok(paths) => {
            notice::print(&format!(
                "The game data is already in {}\n",
                paths.root.display()
            ));
            return true;
        }
        Err(e) => e,
    };
    let player = paths::player_dir_from_environment();
    let player = match on_missing_data(paths::root_is_explicit(cli_root), player, true) {
        MissingData::Ask(p) => p,
        MissingData::Fail => {
            notice::print(&format!("{error}\n"));
            return false;
        }
    };
    notice::print(&format!(
        "Downloading the game data from {}\ninto {}\n",
        archive_url(),
        data_dir(&player).display()
    ));
    match fetch(&player, &mut |_| {}) {
        Ok(root) => {
            notice::print(&format!("The game data is ready in {}\n", root.display()));
            true
        }
        Err(e) => {
            notice::print(&format!("Cannot download the game data: {e}\n"));
            false
        }
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]
mod tests {
    use super::*;

    const SMALL: Limits = Limits {
        download: 1024 * 1024,
        entries: 100,
        file: 1024,
        total: 4096,
    };

    /// A zip of these entries; a name ending in `/` is a folder.
    fn zip_of(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut w = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default();
        for (name, content) in entries {
            if name.ends_with('/') {
                w.add_directory(*name, options).unwrap();
            } else {
                w.start_file(*name, options).unwrap();
                w.write_all(content).unwrap();
            }
        }
        w.finish().unwrap().into_inner()
    }

    fn game_zip() -> Vec<u8> {
        zip_of(&[
            ("soundrts-c0ffee/", b""),
            ("soundrts-c0ffee/README.md", b"readme"),
            ("soundrts-c0ffee/res/", b""),
            ("soundrts-c0ffee/res/rules.txt", b"rules"),
            ("soundrts-c0ffee/res/ui/bindings.txt", b"keys"),
            ("soundrts-c0ffee/cfg/language.txt", b"en"),
            ("soundrts-c0ffee/mods/orc/rules.txt", b"orc"),
            ("soundrts-c0ffee/single/campaign/0.txt", b"map"),
            ("soundrts-c0ffee/user/SoundRTS.ini", b"[general]"),
            ("soundrts-c0ffee/soundrts/clientmain.py", b"python"),
            ("soundrts-c0ffee/resources/x.txt", b"not res"),
        ])
    }

    /// Every file under `dir`, as `/`-separated relative paths, sorted.
    fn files(dir: &Path) -> Vec<String> {
        let mut found = Vec::new();
        let mut stack = vec![dir.to_path_buf()];
        while let Some(d) = stack.pop() {
            for e in fs::read_dir(&d).unwrap().flatten() {
                if e.file_type().unwrap().is_dir() {
                    stack.push(e.path());
                } else {
                    let rel = e.path().strip_prefix(dir).unwrap().to_path_buf();
                    let parts: Vec<String> = rel
                        .components()
                        .map(|c| c.as_os_str().to_string_lossy().into_owned())
                        .collect();
                    found.push(parts.join("/"));
                }
            }
        }
        found.sort();
        found
    }

    /// The names in `dir`, sorted; none if it does not exist.
    fn entries(dir: &Path) -> Vec<String> {
        let Ok(read) = fs::read_dir(dir) else {
            return Vec::new();
        };
        let mut names: Vec<String> = read
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    fn install_bytes(bytes: &[u8], data: &Path, limits: &Limits) -> Result<PathBuf, String> {
        install(bytes, data, "c0ffee", limits, &mut |_| {})
    }

    #[test]
    fn only_the_four_folders_are_installed_without_the_top_folder() {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("data");
        let bytes = game_zip();
        let mut received = Vec::new();
        let root = install(&bytes[..], &data, "c0ffee", &SMALL, &mut |n| {
            received.push(n)
        })
        .unwrap();
        assert_eq!(root, data.join("c0ffee"));
        assert_eq!(
            files(&root),
            [
                "cfg/language.txt",
                "mods/orc/rules.txt",
                "res/rules.txt",
                "res/ui/bindings.txt",
                "single/campaign/0.txt",
            ]
        );
        assert_eq!(fs::read(root.join("res/rules.txt")).unwrap(), b"rules");
        assert_eq!(entries(&data), ["c0ffee", "install.lock"]);
        assert_eq!(received.last(), Some(&(bytes.len() as u64)));
        // The installed folder is what the game finds.
        assert_eq!(
            paths::find_root(None, None, None, dir.path(), Some(&root)),
            Ok(root.clone())
        );
    }

    #[test]
    fn unsafe_paths_are_refused() {
        for name in [
            "soundrts-c0ffee/res/../../evil.txt",
            "/soundrts-c0ffee/res/evil.txt",
            "C:/soundrts-c0ffee/res/evil.txt",
            "soundrts-c0ffee/res/..\\..\\evil.txt",
        ] {
            let dir = tempfile::tempdir().unwrap();
            let data = dir.path().join("data");
            let bytes = zip_of(&[("soundrts-c0ffee/res/rules.txt", b"rules"), (name, b"evil")]);
            let error = install_bytes(&bytes, &data, &SMALL).unwrap_err();
            assert!(error.contains("unsafe path"), "{name}: {error}");
            assert_eq!(entries(&data), ["install.lock"], "{name}");
            assert!(!dir.path().join("evil.txt").exists());
        }
    }

    #[test]
    fn a_download_past_its_limit_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("data");
        let bytes = game_zip();
        let limits = Limits {
            download: bytes.len() as u64 - 1,
            ..SMALL
        };
        let error = install_bytes(&bytes, &data, &limits).unwrap_err();
        assert!(error.contains("larger than"), "{error}");
        assert_eq!(entries(&data), ["install.lock"]);
    }

    #[test]
    fn what_is_extracted_is_held_to_the_limits_whatever_the_zip_states() {
        // Zeros compress well: each file is small in the zip.
        let zeros = vec![0_u8; 3000];
        let one_big = zip_of(&[
            ("soundrts-c0ffee/res/rules.txt", b"rules"),
            ("soundrts-c0ffee/res/big.ogg", &zeros),
        ]);
        let many = zip_of(&[
            ("soundrts-c0ffee/res/rules.txt", b"rules"),
            ("soundrts-c0ffee/res/a", &zeros[..1000]),
            ("soundrts-c0ffee/res/b", &zeros[..1000]),
            ("soundrts-c0ffee/res/c", &zeros[..1000]),
        ]);
        let entries_limit = Limits {
            entries: 3,
            ..SMALL
        };
        let total_limit = Limits {
            total: 2500,
            ..SMALL
        };
        for (bytes, limits, says) in [
            (&one_big, &SMALL, "big.ogg is larger than 1024 bytes"),
            (&many, &total_limit, "expands to more than 2500 bytes"),
            (&many, &entries_limit, "more than 3 entries"),
        ] {
            assert!(bytes.len() < 1000, "{}", bytes.len());
            let dir = tempfile::tempdir().unwrap();
            let data = dir.path().join("data");
            let error = install_bytes(bytes, &data, limits).unwrap_err();
            assert!(error.contains(says), "{error}");
            assert_eq!(entries(&data), ["install.lock"]);
        }
    }

    /// Gives the first half of its bytes, then fails.
    struct Broken {
        bytes: Vec<u8>,
        left: usize,
    }

    impl Read for Broken {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            if self.left == 0 {
                return Err(std::io::Error::other("connection reset"));
            }
            let n = buf.len().min(self.left);
            buf[..n].copy_from_slice(&self.bytes[..n]);
            self.bytes.drain(..n);
            self.left -= n;
            Ok(n)
        }
    }

    #[test]
    fn a_download_that_stops_halfway_leaves_nothing_and_a_rerun_works() {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("data");
        let bytes = game_zip();
        let left = bytes.len() / 2;
        let error =
            install(Broken { bytes, left }, &data, "c0ffee", &SMALL, &mut |_| {}).unwrap_err();
        assert!(error.contains("connection reset"), "{error}");
        assert_eq!(entries(&data), ["install.lock"]);

        // What a run that died while extracting leaves behind.
        fs::create_dir_all(data.join("c0ffee.partial/res")).unwrap();
        fs::write(data.join("c0ffee.partial/res/stale.txt"), "stale").unwrap();
        let root = install_bytes(&game_zip(), &data, &SMALL).unwrap();
        assert_eq!(entries(&data), ["c0ffee", "install.lock"]);
        assert!(!root.join("res/stale.txt").exists());
        assert!(root.join("res/rules.txt").is_file());
    }

    #[test]
    fn a_download_without_rules_is_not_installed() {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("data");
        let bytes = zip_of(&[("soundrts-c0ffee/cfg/language.txt", b"en")]);
        let error = install_bytes(&bytes, &data, &SMALL).unwrap_err();
        assert!(error.contains("res/rules.txt"), "{error}");
        assert_eq!(entries(&data), ["install.lock"]);
    }

    #[test]
    fn a_failed_install_of_this_pin_is_replaced_and_an_older_pin_is_left() {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("data");
        fs::create_dir_all(data.join("0ld/res")).unwrap();
        fs::write(data.join("0ld/res/rules.txt"), "old").unwrap();
        // A folder of this pin without its marker.
        fs::create_dir_all(data.join("c0ffee/res")).unwrap();
        fs::write(data.join("c0ffee/res/broken.txt"), "").unwrap();
        install_bytes(&game_zip(), &data, &SMALL).unwrap();
        assert_eq!(entries(&data), ["0ld", "c0ffee", "install.lock"]);
        assert!(!data.join("c0ffee/res/broken.txt").exists());
        assert_eq!(fs::read(data.join("0ld/res/rules.txt")).unwrap(), b"old");
    }

    #[test]
    fn data_that_is_complete_is_never_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("data");
        // Another game finished installing while this one was asking.
        fs::create_dir_all(data.join("c0ffee/res")).unwrap();
        fs::write(data.join("c0ffee/res/rules.txt"), "theirs").unwrap();
        let root = install_bytes(&game_zip(), &data, &SMALL).unwrap();
        assert_eq!(root, data.join("c0ffee"));
        assert_eq!(fs::read(root.join("res/rules.txt")).unwrap(), b"theirs");
        assert_eq!(files(&root), ["res/rules.txt"]);
    }

    #[test]
    fn a_second_install_at_the_same_time_is_refused_and_touches_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("data");
        // What the first install has extracted so far.
        fs::create_dir_all(data.join("c0ffee.partial/res")).unwrap();
        fs::write(data.join("c0ffee.partial/res/rules.txt"), "half").unwrap();
        let first = lock(&data).unwrap();
        let error = install_bytes(&game_zip(), &data, &SMALL).unwrap_err();
        assert!(error.contains("another SoundRTS is downloading"), "{error}");
        assert_eq!(entries(&data), ["c0ffee.partial", "install.lock"]);
        assert_eq!(
            fs::read(data.join("c0ffee.partial/res/rules.txt")).unwrap(),
            b"half"
        );
        // When the first has ended, the next install goes through.
        drop(first);
        install_bytes(&game_zip(), &data, &SMALL).unwrap();
        assert_eq!(entries(&data), ["c0ffee", "install.lock"]);
    }

    #[test]
    fn names_that_windows_reads_as_something_else_are_refused() {
        for name in [
            "soundrts-c0ffee/res/NUL",
            "soundrts-c0ffee/res/nul.txt",
            "soundrts-c0ffee/res/Com1.ogg",
            "soundrts-c0ffee/res/aux/x.txt",
            "soundrts-c0ffee/res/LPT9",
            "soundrts-c0ffee/res/rules.txt.",
            "soundrts-c0ffee/res/rules.txt ",
            "soundrts-c0ffee/res//x.txt",
            // 17 parts.
            "soundrts-c0ffee/res/1/2/3/4/5/6/7/8/9/10/11/12/13/14/x.txt",
        ] {
            let dir = tempfile::tempdir().unwrap();
            let data = dir.path().join("data");
            let bytes = zip_of(&[("soundrts-c0ffee/res/rules.txt", b"rules"), (name, b"x")]);
            let error = install_bytes(&bytes, &data, &SMALL).unwrap_err();
            assert!(error.contains("unsafe path"), "{name}: {error}");
            assert_eq!(entries(&data), ["install.lock"], "{name}");
        }
        // Names that only look like those are files like any other.
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("data");
        let bytes = zip_of(&[
            ("soundrts-c0ffee/res/rules.txt", b"rules"),
            ("soundrts-c0ffee/res/console.txt", b"a"),
            ("soundrts-c0ffee/res/com10.txt", b"b"),
            (
                "soundrts-c0ffee/res/1/2/3/4/5/6/7/8/9/10/11/12/13/x.txt",
                b"c",
            ),
        ]);
        let root = install_bytes(&bytes, &data, &SMALL).unwrap();
        assert_eq!(
            files(&root),
            [
                "res/1/2/3/4/5/6/7/8/9/10/11/12/13/x.txt",
                "res/com10.txt",
                "res/console.txt",
                "res/rules.txt"
            ]
        );
    }

    #[test]
    fn the_game_asks_only_when_started_from_the_desktop_with_nowhere_else_to_look() {
        let player = || Some(PathBuf::from("P"));
        assert_eq!(
            on_missing_data(false, player(), true),
            MissingData::Ask(PathBuf::from("P"))
        );
        assert_eq!(on_missing_data(false, player(), false), MissingData::Fail);
        assert_eq!(on_missing_data(true, player(), true), MissingData::Fail);
        assert_eq!(on_missing_data(false, None, true), MissingData::Fail);
    }

    #[test]
    fn progress_is_said_in_whole_megabytes() {
        assert_eq!(spoken_progress(999_999), "0 megabytes");
        assert_eq!(spoken_progress(1_500_000), "1 megabyte");
        assert_eq!(spoken_progress(4_200_000), "4 megabytes");
    }

    #[test]
    fn the_pinned_commit_is_a_full_commit_id() {
        assert_eq!(COMMIT.len(), 40);
        assert!(COMMIT.bytes().all(|b| b.is_ascii_hexdigit()));
        assert_eq!(
            archive_url(),
            format!("https://github.com/soundmud/soundrts/archive/{COMMIT}.zip")
        );
    }
}
