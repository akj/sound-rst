//! Packages: folders or zip files of game data, read the same way.
//!
//! A map package can come from a server, so every read is bounded: a zip
//! file by [`MAX_PACKAGE_BYTES`], [`MAX_ENTRIES`] and the sizes its entries
//! state, each file by [`MAX_FILE_BYTES`]. A name read in a folder package
//! stays inside the folder, and links are never followed.
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::string_slice,
    clippy::arithmetic_side_effects
)]

use std::fmt;
use std::io::{Cursor, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use zip::ZipArchive;

/// The largest zip file read into memory, and the most its entries may
/// decompress to together, as their sizes state. The whole original game
/// is 13 MB in 1,101 files.
pub const MAX_PACKAGE_BYTES: u64 = 256 * 1024 * 1024;

/// The most bytes one file of a package may hold, decompressed. The
/// original's largest file is a sound of 1.2 MB.
pub const MAX_FILE_BYTES: u64 = 64 * 1024 * 1024;

/// The most entries a zip file may have, and the most files a folder map
/// is packed with. The whole original game has 1,101 files. A 32-bit end
/// record that states 65,535 entries, the count that defers to a 64-bit
/// record, is not held to this limit, so the room a reader reserves before
/// it refuses such a file is bounded by 65,535 entries rather than by this.
pub const MAX_ENTRIES: usize = 20_000;

/// How deep folders inside a package are looked into. The original's
/// deepest folder is 5 below the game folder.
const MAX_FOLDER_DEPTH: usize = 32;

/// A text file of a layer: where it is, for a message about one of its
/// lines, and its text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextFile {
    pub path: String,
    pub text: String,
}

/// A zip file held in memory, with its entry names in archive order.
struct ZipData {
    filename: String,
    names: Vec<String>,
    archive: Mutex<ZipArchive<Cursor<Vec<u8>>>>,
    /// How many zip files this one is inside.
    depth: u32,
}

/// Where the fields of a zip end record lie: its size, the offsets of its
/// two disk numbers and of its two entry counts (on this disk, in all), and
/// the size of each.
struct EndRecord {
    size: usize,
    disks: [usize; 2],
    disk_len: usize,
    counts: [usize; 2],
    count_len: usize,
}

const END_32: EndRecord = EndRecord {
    size: 22,
    disks: [4, 6],
    disk_len: 2,
    counts: [8, 10],
    count_len: 2,
};

const END_64: EndRecord = EndRecord {
    size: 56,
    disks: [16, 20],
    disk_len: 4,
    counts: [24, 32],
    count_len: 8,
};

/// The entries an end record at `at` states, if a reader could use it: the
/// whole record is in the file, both disk numbers are 0 and both counts
/// agree. A reader skips any other record before it reserves room for its
/// entries (zip 9.5.1, `find_central_directory_end` and
/// `read_central_header`).
fn stated_entries(bytes: &[u8], at: usize, record: &EndRecord) -> Option<u64> {
    let field = |offset: usize, len: usize| -> Option<u64> {
        let start = at.checked_add(offset)?;
        let b = bytes.get(start..start.checked_add(len)?)?;
        Some(b.iter().rev().fold(0, |n, &x| (n << 8) | u64::from(x)))
    };
    bytes.get(at..at.checked_add(record.size)?)?;
    let [d1, d2] = record.disks;
    if field(d1, record.disk_len)? != 0 || field(d2, record.disk_len)? != 0 {
        return None;
    }
    let [c1, c2] = record.counts;
    let on_this_disk = field(c1, record.count_len)?;
    (on_this_disk == field(c2, record.count_len)?).then_some(on_this_disk)
}

/// Whether a zip file can have more than [`MAX_ENTRIES`] entries, checked
/// before an index of them is built, whichever directory a reader picks:
/// - each entry of a central directory starts with its signature, so the
///   signatures anywhere in the file are at least the entries read, those
///   that share a name included;
/// - a reader reserves room for the count an end record states, so no end
///   record a reader could use, anywhere in the file, may state more. A
///   32-bit record stating 0xFFFF defers to a 64-bit record and is let
///   through, which bounds what a reader reserves at 65,535 entries.
///
/// The signature of an end record can also turn up inside an entry's data
/// or the file's comment; there it is held to the limit only if it reads
/// as a record a reader could use.
pub(crate) fn too_many_entries(bytes: &[u8]) -> bool {
    let limit = u64::try_from(MAX_ENTRIES).unwrap_or(u64::MAX);
    let mut entries = 0_usize;
    for (at, w) in bytes.windows(4).enumerate() {
        let stated = match w {
            b"PK\x01\x02" => {
                entries = entries.saturating_add(1);
                None
            }
            b"PK\x05\x06" => stated_entries(bytes, at, &END_32).filter(|&n| n != 0xFFFF),
            b"PK\x06\x06" => stated_entries(bytes, at, &END_64),
            _ => None,
        };
        if entries > MAX_ENTRIES || stated.is_some_and(|n| n > limit) {
            return true;
        }
    }
    false
}

fn too_large(what: &str, limit: u64) -> std::io::Error {
    std::io::Error::other(format!("{what} holds more than {limit} bytes"))
}

/// Reads a file of at most `limit` bytes.
pub fn read_file(path: &Path, limit: u64) -> std::io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take(limit.saturating_add(1))
        .read_to_end(&mut bytes)?;
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > limit {
        return Err(too_large(&path.display().to_string(), limit));
    }
    Ok(bytes)
}

impl ZipData {
    /// Opens a zip file, if it is within [`MAX_PACKAGE_BYTES`] and
    /// [`MAX_ENTRIES`], its entries state sizes within
    /// [`MAX_PACKAGE_BYTES`] together, and it is inside at most
    /// [`MAX_ZIP_DEPTH`] other zip files.
    fn new(filename: String, bytes: Vec<u8>, depth: u32) -> std::io::Result<ZipData> {
        if depth > MAX_ZIP_DEPTH {
            return Err(std::io::Error::other(format!(
                "{filename}: zip files nested too deep"
            )));
        }
        if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAX_PACKAGE_BYTES {
            return Err(too_large(&filename, MAX_PACKAGE_BYTES));
        }
        if too_many_entries(&bytes) {
            return Err(std::io::Error::other(format!(
                "{filename} has more than {MAX_ENTRIES} entries"
            )));
        }
        let mut archive = ZipArchive::new(Cursor::new(bytes)).map_err(std::io::Error::other)?;
        let mut stated: u64 = 0;
        for i in 0..archive.len() {
            let size = archive
                .by_index_raw(i)
                .map_err(std::io::Error::other)?
                .size();
            stated = stated.saturating_add(size);
        }
        if stated > MAX_PACKAGE_BYTES {
            return Err(too_large(
                &format!("{filename} decompressed"),
                MAX_PACKAGE_BYTES,
            ));
        }
        let names = archive
            .file_names()
            .filter_map(Result::ok)
            .map(|n| n.into_owned())
            .collect();
        Ok(ZipData {
            filename,
            names,
            archive: Mutex::new(archive),
            depth,
        })
    }

    /// The content of an entry. It may not decompress to more than the
    /// size it states, which keeps a whole reading of the zip file within
    /// [`MAX_PACKAGE_BYTES`], nor to more than [`MAX_FILE_BYTES`].
    fn read(&self, name: &str) -> std::io::Result<Vec<u8>> {
        let mut archive = self.archive.lock().unwrap_or_else(|p| p.into_inner());
        let mut file = archive.by_name(name).map_err(|e| match e {
            zip::result::ZipError::FileNotFound => {
                std::io::Error::new(std::io::ErrorKind::NotFound, name.to_owned())
            }
            e => std::io::Error::other(e),
        })?;
        let limit = file.size().min(MAX_FILE_BYTES);
        let mut bytes = Vec::new();
        (&mut file)
            .take(limit.saturating_add(1))
            .read_to_end(&mut bytes)?;
        if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > limit {
            return Err(too_large(&format!("{} in {}", name, self.filename), limit));
        }
        Ok(bytes)
    }
}

#[derive(Clone)]
enum Kind {
    Folder(PathBuf),
    Zip {
        data: Arc<ZipData>,
        subdir: Option<String>,
    },
}

/// A virtual folder: a real folder, a zip file, or a folder inside a zip
/// file. Paths inside a package use `/`.
#[derive(Clone)]
pub struct Package {
    /// The layer name: "default", or the mod, map or campaign name.
    pub name: String,
    /// The mods this mod requires, from its `mod.txt`.
    pub required_mods: Vec<String>,
    kind: Kind,
}

impl PartialEq for Package {
    fn eq(&self, other: &Self) -> bool {
        match (&self.kind, &other.kind) {
            (Kind::Folder(a), Kind::Folder(b)) => a == b,
            (Kind::Zip { data: a, subdir: s }, Kind::Zip { data: b, subdir: t }) => {
                Arc::ptr_eq(a, b) && s == t
            }
            _ => false,
        }
    }
}

impl fmt::Debug for Package {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.kind {
            Kind::Folder(p) => write!(f, "<FolderPackage {} name={}>", p.display(), self.name),
            Kind::Zip { data, subdir } => write!(
                f,
                "<ZipPackage filename={} subdir={subdir:?} name={}>",
                data.filename, self.name
            ),
        }
    }
}

/// `name` as a path under `root`: each part a plain name (not `..`, `.`,
/// a drive or a stream), and no part below `root` a link. `None` when the
/// name would leave the folder.
pub(crate) fn confined(root: &Path, name: &str) -> Option<PathBuf> {
    let mut path = root.to_path_buf();
    let mut exists = true;
    for part in name.split(['/', '\\']).filter(|p| !p.is_empty()) {
        if part.trim_end_matches(['.', ' ']).is_empty() || part.contains(':') {
            return None;
        }
        path.push(part);
        if exists {
            match std::fs::symlink_metadata(&path) {
                Ok(m) if m.is_symlink() => return None,
                Ok(_) => {}
                Err(_) => exists = false,
            }
        }
    }
    Some(path)
}

fn first_component(name: &str) -> Option<&str> {
    name.split('/').find(|part| !part.is_empty())
}

/// Converts `\r\n` and `\r` line ends to `\n`, as Python's text files did.
pub fn universal_newlines(s: &str) -> String {
    if s.contains('\r') {
        s.replace("\r\n", "\n").replace('\r', "\n")
    } else {
        s.to_owned()
    }
}

/// Decodes a text file of the game data.
pub fn decode_text(bytes: &[u8]) -> String {
    universal_newlines(&String::from_utf8_lossy(bytes))
}

impl Package {
    fn new(kind: Kind) -> Package {
        Package {
            name: "default".to_owned(),
            required_mods: Vec::new(),
            kind,
        }
    }

    /// A folder, or a zip file if the path ends with `.zip` or `.pkg`.
    pub fn from_path(path: &Path) -> std::io::Result<Package> {
        let is_zip = matches!(
            path.extension().and_then(|e| e.to_str()),
            Some("zip") | Some("pkg")
        );
        if is_zip {
            let bytes = read_file(path, MAX_PACKAGE_BYTES)?;
            Self::from_zip_bytes(path.display().to_string(), bytes)
        } else {
            Ok(Package::new(Kind::Folder(path.to_path_buf())))
        }
    }

    /// A zip file held in memory.
    pub fn from_zip_bytes(filename: String, bytes: Vec<u8>) -> std::io::Result<Package> {
        Self::from_zip_bytes_at(filename, bytes, 0)
    }

    /// A zip file held in memory, found inside `depth` other zip files.
    fn from_zip_bytes_at(filename: String, bytes: Vec<u8>, depth: u32) -> std::io::Result<Package> {
        let data = Arc::new(ZipData::new(filename, bytes, depth)?);
        Ok(Package::new(Kind::Zip { data, subdir: None }))
    }

    /// The folder on disk, for a folder package.
    pub fn folder(&self) -> Option<&Path> {
        match &self.kind {
            Kind::Folder(p) => Some(p),
            Kind::Zip { .. } => None,
        }
    }

    fn zip_path(subdir: &Option<String>, name: &str) -> String {
        match subdir {
            Some(s) if !s.is_empty() => format!("{s}/{name}"),
            _ => name.to_owned(),
        }
    }

    /// The names of the entries, relative to this package.
    fn zip_names<'a>(data: &'a ZipData, subdir: &'a Option<String>) -> Vec<&'a str> {
        match subdir {
            None => data.names.iter().map(String::as_str).collect(),
            Some(s) => {
                let start = format!("{s}/");
                data.names
                    .iter()
                    .filter_map(|n| n.strip_prefix(start.as_str()))
                    .collect()
            }
        }
    }

    /// The content of a file. A folder is returned zipped, as a map folder
    /// in `multi` is read as a zipped map.
    pub fn open_binary(&self, name: &str) -> std::io::Result<Vec<u8>> {
        match &self.kind {
            Kind::Folder(root) => {
                let path = confined(root, name).ok_or_else(|| outside(name))?;
                if path.is_dir() {
                    zip_folder(&path)
                } else {
                    read_file(&path, MAX_FILE_BYTES)
                }
            }
            Kind::Zip { data, subdir } => data.read(&Self::zip_path(subdir, name)),
        }
    }

    /// The text of a file, decoded and with `\n` line ends, or `None` if it
    /// cannot be read.
    pub fn read_text(&self, name: &str) -> Option<String> {
        let bytes = self.open_binary(name).ok()?;
        Some(decode_text(&bytes))
    }

    pub fn isfile(&self, name: &str) -> bool {
        match &self.kind {
            Kind::Folder(root) => confined(root, name).is_some_and(|p| p.is_file()),
            Kind::Zip { data, subdir } => {
                let full = Self::zip_path(subdir, name);
                data.names.contains(&full)
            }
        }
    }

    pub fn isdir(&self, name: &str) -> bool {
        match &self.kind {
            Kind::Folder(root) => confined(root, name).is_some_and(|p| p.is_dir()),
            Kind::Zip { data, subdir } => {
                let prefix = format!("{name}/");
                Self::zip_names(data, subdir)
                    .iter()
                    .any(|n| n.starts_with(&prefix))
            }
        }
    }

    fn folder_entries(root: &Path, want_dirs: bool) -> Vec<String> {
        let Ok(entries) = std::fs::read_dir(root) else {
            return Vec::new();
        };
        let mut names: Vec<String> = entries
            .flatten()
            .take(MAX_ENTRIES)
            .filter(|e| {
                e.file_type()
                    .is_ok_and(|t| !t.is_symlink() && t.is_dir() == want_dirs)
            })
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    fn zip_top_level(
        &self,
        data: &ZipData,
        subdir: &Option<String>,
        want_dirs: bool,
    ) -> Vec<String> {
        let mut result: Vec<String> = Vec::new();
        for name in Self::zip_names(data, subdir) {
            if let Some(first) = first_component(name)
                && self.isfile(first) != want_dirs
                && !result.iter().any(|r| r == first)
            {
                result.push(first.to_owned());
            }
        }
        result
    }

    /// The folders at the top level.
    pub fn dirnames(&self) -> Vec<String> {
        match &self.kind {
            Kind::Folder(root) => Self::folder_entries(root, true),
            Kind::Zip { data, subdir } => self.zip_top_level(data, subdir, true),
        }
    }

    /// The files at the top level.
    pub fn filenames(&self) -> Vec<String> {
        match &self.kind {
            Kind::Folder(root) => Self::folder_entries(root, false),
            Kind::Zip { data, subdir } => self.zip_top_level(data, subdir, false),
        }
    }

    /// The paths, relative to this package, of every file under `subdir`.
    pub fn relative_paths_of_files_in_subtree(&self, subdir: &str) -> Vec<String> {
        match &self.kind {
            Kind::Folder(root) => {
                let mut result = Vec::new();
                if let Some(dir) = confined(root, subdir) {
                    walk(root, &dir, &mut result, 0);
                }
                result.truncate(MAX_ENTRIES);
                result
            }
            Kind::Zip { data, subdir: s } => {
                let prefix = format!("{subdir}/");
                Self::zip_names(data, s)
                    .into_iter()
                    .filter(|n| n.starts_with(&prefix))
                    .map(str::to_owned)
                    .collect()
            }
        }
    }

    /// The package for a folder inside this one (or a zip file inside a
    /// zip file), if it exists.
    pub fn subpackage(&self, subdir: &str) -> Option<Package> {
        match &self.kind {
            Kind::Folder(root) => {
                let path = confined(root, subdir)?;
                if path.is_dir() {
                    Package::from_path(&path).ok()
                } else {
                    None
                }
            }
            Kind::Zip { data, subdir: s } => {
                let names = Self::zip_names(data, s);
                if subdir.ends_with(".zip") && names.contains(&subdir) {
                    let bytes = self.open_binary(subdir).ok()?;
                    let depth = data.depth.saturating_add(1);
                    return Package::from_zip_bytes_at(subdir.to_owned(), bytes, depth).ok();
                }
                let prefix = format!("{subdir}/");
                if names.iter().any(|n| n.starts_with(&prefix)) {
                    Some(Package::new(Kind::Zip {
                        data: data.clone(),
                        subdir: Some(Self::zip_path(s, subdir)),
                    }))
                } else {
                    None
                }
            }
        }
    }

    /// A package without `rules.txt` or `ai.txt` only changes sounds.
    pub fn is_a_soundpack(&self) -> bool {
        !(self.isfile("rules.txt") || self.isfile("ai.txt"))
    }

    /// Names this package as a layer of the resource stack.
    pub fn with_name(mut self, name: &str) -> Package {
        self.name = name.to_owned();
        self
    }
}

fn outside(name: &str) -> std::io::Error {
    std::io::Error::new(
        std::io::ErrorKind::NotFound,
        format!("{name}: not a plain path inside the package"),
    )
}

/// Adds to `out` the paths, relative to `root`, of the files under `dir`,
/// at most [`MAX_ENTRIES`] and one more, and no deeper than
/// [`MAX_FOLDER_DEPTH`]. Links are skipped.
fn walk(root: &Path, dir: &Path, out: &mut Vec<String>, depth: usize) {
    if depth > MAX_FOLDER_DEPTH {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = entries
        .flatten()
        .take(MAX_ENTRIES.saturating_add(1))
        .collect();
    entries.sort_by_key(|e| e.file_name());
    let mut subdirs = Vec::new();
    for e in entries {
        if out.len() > MAX_ENTRIES {
            return;
        }
        let Ok(kind) = e.file_type() else {
            continue;
        };
        let path = e.path();
        if kind.is_symlink() {
            continue;
        } else if kind.is_dir() {
            subdirs.push(path);
        } else if let Ok(rel) = path.strip_prefix(root) {
            let rel: Vec<String> = rel
                .components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect();
            out.push(rel.join("/"));
        }
    }
    for d in subdirs {
        walk(root, &d, out, depth.saturating_add(1));
    }
}

/// How much a zip file from elsewhere may hold once decompressed.
#[derive(Clone, Copy, Debug)]
pub struct ExpansionLimits {
    /// The most bytes one entry may decompress to.
    pub entry: u64,
    /// The most bytes all entries together may decompress to, those of
    /// zip files inside it included.
    pub total: u64,
}

/// The limits of a map package another player sends: those of every
/// package.
pub const NETWORK_MAP_LIMITS: ExpansionLimits = ExpansionLimits {
    entry: MAX_FILE_BYTES,
    total: MAX_PACKAGE_BYTES,
};

/// How deep zip files inside zip files are looked into.
const MAX_ZIP_DEPTH: u32 = 3;

/// Checks that a zip file decompresses within `limits` by decompressing
/// it, without keeping what it decompresses (the sizes the zip file states
/// may lie). Zip files inside it are checked too, deeper ones refused.
pub fn check_expansion(bytes: &[u8], limits: ExpansionLimits) -> Result<(), String> {
    let mut total = 0;
    check_zip(bytes, limits, &mut total, 0)
}

fn check_zip(
    bytes: &[u8],
    limits: ExpansionLimits,
    total: &mut u64,
    depth: u32,
) -> Result<(), String> {
    if depth > MAX_ZIP_DEPTH {
        return Err("zip files nested too deep".into());
    }
    if too_many_entries(bytes) {
        return Err(format!("more than {MAX_ENTRIES} entries"));
    }
    let mut archive = ZipArchive::new(Cursor::new(bytes)).map_err(|e| e.to_string())?;
    for i in 0..archive.len() {
        let mut file = archive.by_index(i).map_err(|e| e.to_string())?;
        let name = file.name().map(|n| n.into_owned()).unwrap_or_default();
        let nested = name.to_lowercase().ends_with(".zip");
        let left = limits.entry.min(limits.total.saturating_sub(*total));
        let mut kept = Vec::new();
        let mut reader = (&mut file).take(left.saturating_add(1));
        let n = if nested {
            reader.read_to_end(&mut kept).map_err(|e| e.to_string())? as u64
        } else {
            std::io::copy(&mut reader, &mut std::io::sink()).map_err(|e| e.to_string())?
        };
        if n > left {
            return Err(format!("{name} decompresses to too many bytes"));
        }
        *total = total.saturating_add(n);
        if nested {
            check_zip(&kept, limits, total, depth.saturating_add(1))?;
        }
    }
    Ok(())
}

/// A folder as an uncompressed zip file, in memory, within the limits of
/// a zip file. Links in it are left out, so that only the folder's own
/// files can be sent to other players.
fn zip_folder(dir: &Path) -> std::io::Result<Vec<u8>> {
    let mut names = Vec::new();
    walk(dir, dir, &mut names, 0);
    if names.len() > MAX_ENTRIES {
        return Err(std::io::Error::other(format!(
            "{} has more than {MAX_ENTRIES} files",
            dir.display()
        )));
    }
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    let mut total: u64 = 0;
    for name in names {
        let bytes = read_file(&dir.join(&name), MAX_FILE_BYTES)?;
        total = total.saturating_add(u64::try_from(bytes.len()).unwrap_or(u64::MAX));
        if total > MAX_PACKAGE_BYTES {
            return Err(too_large(&dir.display().to_string(), MAX_PACKAGE_BYTES));
        }
        writer
            .start_file(name.as_str(), options)
            .map_err(std::io::Error::other)?;
        writer.write_all(&bytes)?;
    }
    Ok(writer.finish().map_err(std::io::Error::other)?.into_inner())
}

/// The packages of the game, in loading order, starting with the base
/// package.
#[derive(Clone, Debug, Default)]
pub struct PackageStack(pub Vec<Package>);

impl PackageStack {
    /// Opens every path that can be opened, in order.
    pub fn new(paths: &[PathBuf]) -> PackageStack {
        PackageStack(
            paths
                .iter()
                .filter_map(|p| Package::from_path(p).ok())
                .collect(),
        )
    }

    /// The mod called `name` in the last package that has it.
    pub fn find_mod(&self, name: &str) -> Option<Package> {
        let subdir = format!("mods/{name}");
        let package = self.0.iter().rev().find_map(|p| p.subpackage(&subdir))?;
        let mut package = package.with_name(name);
        if package.isfile("mod.txt")
            && let Some(s) = package.read_text("mod.txt")
            && let Some(rest) = s.strip_prefix("mods ")
        {
            // The line end stays on the last name, as it did upstream.
            package.required_mods = rest.split(',').map(str::to_owned).collect();
        }
        Some(package)
    }

    /// Every mod and soundpack of every package.
    pub fn mods(&self) -> Vec<Package> {
        let mut names: Vec<String> = Vec::new();
        for package in self.0.iter().rev() {
            if let Some(mods) = package.subpackage("mods") {
                for name in mods.dirnames() {
                    if !names.contains(&name) {
                        names.push(name);
                    }
                }
            }
        }
        names.sort();
        names.iter().filter_map(|n| self.find_mod(n)).collect()
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

    fn zipped(entries: &[(&str, Vec<u8>)]) -> Vec<u8> {
        let mut w = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        for (name, bytes) in entries {
            w.start_file(*name, options).unwrap();
            w.write_all(bytes).unwrap();
        }
        w.finish().unwrap().into_inner()
    }

    const SMALL: ExpansionLimits = ExpansionLimits {
        entry: 1000,
        total: 1500,
    };

    #[test]
    fn a_zip_file_within_the_limits_passes() {
        let z = zipped(&[("map.txt", vec![b'a'; 1000]), ("a.ogg", vec![0; 500])]);
        assert_eq!(check_expansion(&z, SMALL), Ok(()));
    }

    #[test]
    fn a_zip_file_that_expands_too_much_is_refused() {
        // One entry too large, small once compressed.
        let z = zipped(&[("map.txt", vec![0; 1001])]);
        assert!(z.len() < 200);
        assert!(check_expansion(&z, SMALL).is_err());
        // Entries each within the limit, too large together.
        let z = zipped(&[("a", vec![0; 1000]), ("b", vec![0; 501])]);
        assert!(check_expansion(&z, SMALL).is_err());
        // Too large inside a zip file inside the zip file.
        let inner = zipped(&[("map.txt", vec![0; 1001])]);
        let z = zipped(&[("inner.zip", inner)]);
        assert!(check_expansion(&z, SMALL).is_err());
        // Not a zip file.
        assert!(check_expansion(b"title 1", SMALL).is_err());
    }
    use crate::testkit::upstream_tests_dir;

    fn check_res2(p: &Package) {
        assert!(p.subpackage("mods/mod1").is_some());
        assert!(p.subpackage("mods/mod").is_none());
        assert!(p.subpackage("mods/mod11").is_none());
        assert!(p.relative_paths_of_files_in_subtree("mods")[0].starts_with("mods"));
        assert_eq!(p.dirnames(), vec!["mods".to_owned()]);
        assert!(p.isdir("mods"));
        assert!(!p.isfile("mods"));
        assert!(p.isfile("mods/mod1/rules.txt"));
        assert!(!p.isdir("mods/mod1/rules.txt"));
        assert!(p.isfile("readme.txt"));
        assert!(!p.isdir("readme.txt"));
    }

    #[test]
    fn test_zip_package() {
        check_res2(&Package::from_path(&upstream_tests_dir().join("res2.zip")).unwrap());
    }

    #[test]
    fn test_folder_package() {
        check_res2(&Package::from_path(&upstream_tests_dir().join("res2")).unwrap());
    }

    #[test]
    fn test_res_folder_package() {
        let p = Package::from_path(&upstream_tests_dir().join("res")).unwrap();
        assert!(p.subpackage("ui").is_some());
        assert!(p.subpackage("u").is_none());
        assert!(p.subpackage("ui1").is_none());
        assert!(p.relative_paths_of_files_in_subtree("ui")[0].starts_with("ui"));
        let mut dirs = p.dirnames();
        dirs.sort();
        assert_eq!(dirs, ["multi", "ui", "ui-fr"]);
    }

    #[test]
    fn test_subpackage_dirnames() {
        let folder = crate::testkit::upstream_res2_folder();
        for path in [
            upstream_tests_dir().join("res2.zip"),
            folder.path().join("res2"),
        ] {
            let p = Package::from_path(&path).unwrap();
            let mods = p.subpackage("mods").unwrap();
            let mut names = mods.dirnames();
            names.sort();
            assert_eq!(names, ["mod1", "mod2", "sound1"]);
            assert!(mods.subpackage("sound1").unwrap().is_a_soundpack());
            assert!(!mods.subpackage("mod1").unwrap().is_a_soundpack());
            assert_eq!(PackageStack::new(&[path]).mods().len(), 3);
        }
    }

    #[test]
    fn a_folder_reads_back_as_a_zip_with_the_same_files() {
        let p = Package::from_path(&upstream_tests_dir().join("single")).unwrap();
        let zipped =
            Package::from_zip_bytes("map1".into(), p.open_binary("map1").unwrap()).unwrap();
        assert_eq!(zipped.read_text("map.txt").unwrap(), "title 11 22\n");
        assert!(zipped.isfile("ui/9998.ogg"));
    }

    #[test]
    fn text_gets_unix_line_ends() {
        assert_eq!(decode_text(b"a\r\nb\rc"), "a\nb\nc");
    }

    /// `bytes` with the uncompressed size of every entry, in its local and
    /// central headers, replaced by `size`.
    fn stating_size(mut bytes: Vec<u8>, size: u32) -> Vec<u8> {
        let mut i = 0;
        while i + 4 <= bytes.len() {
            let at = match &bytes[i..i + 4] {
                b"PK\x03\x04" => Some(i + 22),
                b"PK\x01\x02" => Some(i + 24),
                _ => None,
            };
            if let Some(at) = at {
                bytes[at..at + 4].copy_from_slice(&size.to_le_bytes());
            }
            i += 1;
        }
        bytes
    }

    #[test]
    fn an_entry_that_decompresses_to_more_than_it_states_is_refused() {
        let honest = zipped(&[("map.txt", vec![b'a'; 5000])]);
        let p = Package::from_zip_bytes("honest.zip".into(), honest.clone()).unwrap();
        assert_eq!(p.open_binary("map.txt").unwrap().len(), 5000);
        let lying = Package::from_zip_bytes("lying.zip".into(), stating_size(honest, 10)).unwrap();
        assert!(lying.open_binary("map.txt").is_err());
        assert_eq!(lying.read_text("map.txt"), None);
    }

    #[test]
    fn a_zip_file_whose_entries_state_too_much_is_refused() {
        let z = zipped(&[("a", vec![0; 10]), ("b", vec![0; 10])]);
        let half = u32::try_from(MAX_PACKAGE_BYTES / 2 + 1).unwrap();
        assert!(Package::from_zip_bytes("z.zip".into(), z.clone()).is_ok());
        assert!(Package::from_zip_bytes("z.zip".into(), stating_size(z, half)).is_err());
    }

    #[test]
    fn a_zip_file_with_too_many_entries_is_refused() {
        let names: Vec<String> = (0..=MAX_ENTRIES).map(|i| i.to_string()).collect();
        let entries: Vec<(&str, Vec<u8>)> =
            names.iter().map(|n| (n.as_str(), Vec::new())).collect();
        let z = zipped(&entries);
        assert!(Package::from_zip_bytes("many.zip".into(), z.clone()).is_err());
        assert!(check_expansion(&z, NETWORK_MAP_LIMITS).is_err());
        let z = zipped(&entries[1..]);
        assert!(Package::from_zip_bytes("enough.zip".into(), z).is_ok());
    }

    #[test]
    fn entries_that_share_a_name_count_each() {
        // The writer refuses a name twice: write distinct names of the same
        // length, then give them one name in the central directory, which
        // comes after the entries.
        let names: Vec<String> = (0..=MAX_ENTRIES).map(|i| format!("{i:05}")).collect();
        let entries: Vec<(&str, Vec<u8>)> =
            names.iter().map(|n| (n.as_str(), Vec::new())).collect();
        let mut z = zipped(&entries);
        let mut at = z.windows(4).position(|w| w == b"PK\x01\x02").unwrap();
        for name in &names {
            at += z[at..]
                .windows(5)
                .position(|w| w == name.as_bytes())
                .unwrap();
            z[at..at + 5].copy_from_slice(b"same.");
        }
        let archive = ZipArchive::new(Cursor::new(z.clone())).unwrap();
        assert!(archive.len() < 10);
        assert!(Package::from_zip_bytes("same.zip".into(), z.clone()).is_err());
        assert!(check_expansion(&z, NETWORK_MAP_LIMITS).is_err());
        // The same, followed by an end record of one entry that points at
        // nothing valid, so that readers may disagree on the directory.
        let end = z.windows(4).rposition(|w| w == b"PK\x05\x06").unwrap();
        let mut trailing = z[end..end + 22].to_vec();
        trailing[8..10].copy_from_slice(&1_u16.to_le_bytes());
        trailing[10..12].copy_from_slice(&1_u16.to_le_bytes());
        z.extend(trailing);
        assert!(Package::from_zip_bytes("same.zip".into(), z.clone()).is_err());
        assert!(check_expansion(&z, NETWORK_MAP_LIMITS).is_err());
    }

    #[test]
    fn a_zip_file_that_states_too_many_entries_is_refused() {
        // One entry, with an end record that states 20,001.
        let mut z = zipped(&[("map.txt", b"title 1".to_vec())]);
        assert!(Package::from_zip_bytes("one.zip".into(), z.clone()).is_ok());
        let end = z.windows(4).rposition(|w| w == b"PK\x05\x06").unwrap();
        let stated = u16::try_from(MAX_ENTRIES + 1).unwrap().to_le_bytes();
        z[end + 8..end + 10].copy_from_slice(&stated);
        z[end + 10..end + 12].copy_from_slice(&stated);
        assert!(Package::from_zip_bytes("one.zip".into(), z.clone()).is_err());
        assert!(check_expansion(&z, NETWORK_MAP_LIMITS).is_err());
    }

    /// A zip file of one entry whose end records are 64-bit ones stating
    /// `entries`.
    fn zip64_stating(entries: u64) -> Vec<u8> {
        let z = zipped(&[("map.txt", b"title 1".to_vec())]);
        let end = z.windows(4).rposition(|w| w == b"PK\x05\x06").unwrap();
        let mut out = z[..end].to_vec();
        out.extend(b"PK\x06\x06");
        out.extend(44_u64.to_le_bytes());
        out.extend([45, 0, 45, 0]);
        out.extend([0; 8]);
        out.extend(entries.to_le_bytes());
        out.extend(entries.to_le_bytes());
        out.extend(
            u64::from(u32::from_le_bytes(
                z[end + 12..end + 16].try_into().unwrap(),
            ))
            .to_le_bytes(),
        );
        out.extend(
            u64::from(u32::from_le_bytes(
                z[end + 16..end + 20].try_into().unwrap(),
            ))
            .to_le_bytes(),
        );
        out.extend(b"PK\x06\x07");
        out.extend([0; 4]);
        out.extend(u64::try_from(end).unwrap().to_le_bytes());
        out.extend(1_u32.to_le_bytes());
        out.extend(b"PK\x05\x06");
        out.extend([0; 4]);
        out.extend([0xff; 12]);
        out.extend([0; 2]);
        out
    }

    #[test]
    fn a_zip64_file_that_states_too_many_entries_is_refused() {
        let z = zip64_stating(1);
        let p = Package::from_zip_bytes("one.zip".into(), z).unwrap();
        assert_eq!(p.read_text("map.txt").unwrap(), "title 1");
        let z = zip64_stating(u64::try_from(MAX_ENTRIES + 1).unwrap());
        assert!(too_many_entries(&z));
        assert!(Package::from_zip_bytes("one.zip".into(), z.clone()).is_err());
        assert!(check_expansion(&z, NETWORK_MAP_LIMITS).is_err());
    }

    /// Bytes that start like an end record whose counts are large, as
    /// they can by chance in any data.
    fn record_lookalike(signature: &[u8]) -> Vec<u8> {
        let mut bytes = b"data ".to_vec();
        bytes.extend(signature);
        bytes.extend(b"z".repeat(60));
        bytes
    }

    #[test]
    fn data_that_looks_like_an_end_record_does_not_count() {
        for signature in [b"PK\x05\x06", b"PK\x06\x06"] {
            let data = record_lookalike(signature);
            for z in [stored(&data), deflated_as_is(&data)] {
                assert!(z.windows(4).any(|w| w == signature.as_slice()));
                let p = Package::from_zip_bytes("map.zip".into(), z.clone()).unwrap();
                assert_eq!(p.open_binary("map.txt").unwrap(), data);
                assert_eq!(check_expansion(&z, NETWORK_MAP_LIMITS), Ok(()));
            }
        }
    }

    /// A zip file of `map.txt` holding `data`, stored.
    fn stored(data: &[u8]) -> Vec<u8> {
        let mut w = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        w.start_file("map.txt", options).unwrap();
        w.write_all(data).unwrap();
        w.finish().unwrap().into_inner()
    }

    /// A zip file of `map.txt` holding `data`, deflated as one stored
    /// block, so that `data` appears as it is in the file. The zip writer
    /// makes no such block, so the stored file's headers are changed.
    fn deflated_as_is(data: &[u8]) -> Vec<u8> {
        let len = u16::try_from(data.len()).unwrap();
        let mut stream = vec![1];
        stream.extend(len.to_le_bytes());
        stream.extend((!len).to_le_bytes());
        stream.extend(data);
        let mut z = stored(&stream);
        let crc = crc32(data).to_le_bytes();
        let size = u32::from(len).to_le_bytes();
        for (signature, method, crc_at, size_at) in
            [(b"PK\x03\x04", 8, 14, 22), (b"PK\x01\x02", 10, 16, 24)]
        {
            let at = z.windows(4).position(|w| w == signature).unwrap();
            z[at + method..at + method + 2].copy_from_slice(&8_u16.to_le_bytes());
            z[at + crc_at..at + crc_at + 4].copy_from_slice(&crc);
            z[at + size_at..at + size_at + 4].copy_from_slice(&size);
        }
        z
    }

    fn crc32(data: &[u8]) -> u32 {
        let mut crc = !0_u32;
        for &byte in data {
            crc ^= u32::from(byte);
            for _ in 0..8 {
                crc = if crc & 1 != 0 {
                    (crc >> 1) ^ 0xedb8_8320
                } else {
                    crc >> 1
                };
            }
        }
        !crc
    }

    #[test]
    fn a_comment_that_looks_like_an_end_record_does_not_count() {
        let mut w = zip::ZipWriter::new(Cursor::new(Vec::new()));
        w.set_comment(String::from_utf8(record_lookalike(b"PK\x05\x06")).unwrap())
            .unwrap();
        w.start_file("map.txt", zip::write::SimpleFileOptions::default())
            .unwrap();
        w.write_all(b"title 1").unwrap();
        let z = w.finish().unwrap().into_inner();
        let p = Package::from_zip_bytes("map.zip".into(), z.clone()).unwrap();
        assert_eq!(p.read_text("map.txt").unwrap(), "title 1");
        assert_eq!(check_expansion(&z, NETWORK_MAP_LIMITS), Ok(()));
    }

    #[test]
    fn zip_files_inside_zip_files_are_opened_only_so_deep() {
        let mut z = zipped(&[("map.txt", b"title 1".to_vec())]);
        for _ in 0..MAX_ZIP_DEPTH {
            z = zipped(&[("in.zip", z)]);
        }
        let mut p = Package::from_zip_bytes("out.zip".into(), z.clone()).unwrap();
        for _ in 0..MAX_ZIP_DEPTH {
            p = p.subpackage("in.zip").unwrap();
        }
        assert_eq!(p.read_text("map.txt").unwrap(), "title 1");
        let z = zipped(&[("in.zip", z)]);
        let mut p = Package::from_zip_bytes("out.zip".into(), z).unwrap();
        for _ in 0..MAX_ZIP_DEPTH {
            p = p.subpackage("in.zip").unwrap();
        }
        assert!(p.subpackage("in.zip").is_none());
    }

    #[test]
    fn a_file_over_the_limit_is_not_read() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("f");
        std::fs::write(&path, [0; 11]).unwrap();
        assert_eq!(read_file(&path, 11).unwrap().len(), 11);
        assert!(read_file(&path, 10).is_err());
    }

    #[test]
    fn a_mod_cannot_require_a_folder_outside_its_package() {
        let dir = tempfile::tempdir().unwrap();
        let package = dir.path().join("package");
        std::fs::create_dir_all(package.join("mods/m")).unwrap();
        std::fs::write(package.join("mods/m/mod.txt"), "mods ../../outside").unwrap();
        std::fs::create_dir_all(dir.path().join("outside")).unwrap();
        std::fs::write(dir.path().join("outside/rules.txt"), "def secret\n").unwrap();
        let stack = PackageStack::new(std::slice::from_ref(&package));
        let m = stack.find_mod("m").unwrap();
        assert_eq!(m.required_mods, ["../../outside"]);
        assert!(stack.find_mod("../../outside").is_none());
        let p = Package::from_path(&package).unwrap();
        for name in [
            "../outside/rules.txt",
            "mods/../../outside/rules.txt",
            "mods\\..\\..\\outside\\rules.txt",
            "mods/m/.../rules.txt",
        ] {
            assert!(p.open_binary(name).is_err(), "{name}");
            assert!(!p.isfile(name), "{name}");
        }
        assert!(p.subpackage("..").is_none());
        assert!(p.relative_paths_of_files_in_subtree("..").is_empty());
        let absolute = dir.path().join("outside/rules.txt");
        assert!(p.open_binary(&absolute.display().to_string()).is_err());
    }

    /// Makes `link` a link to the folder `target`.
    fn link_folder(target: &Path, link: &Path) {
        #[cfg(windows)]
        if std::os::windows::fs::symlink_dir(target, link).is_err() {
            // A junction needs no privilege.
            let status = std::process::Command::new("cmd")
                .args(["/C", "mklink", "/J"])
                .arg(link)
                .arg(target)
                .stdout(std::process::Stdio::null())
                .status()
                .unwrap();
            assert!(status.success());
        }
        #[cfg(unix)]
        std::os::unix::fs::symlink(target, link).unwrap();
    }

    #[test]
    fn a_folder_map_leaves_out_its_links() {
        let dir = tempfile::tempdir().unwrap();
        let map = dir.path().join("multi/m");
        std::fs::create_dir_all(map.join("ui")).unwrap();
        std::fs::write(map.join("map.txt"), "title 1").unwrap();
        std::fs::write(map.join("ui/tts.txt"), "9000 golem").unwrap();
        let secret = dir.path().join("secret");
        std::fs::create_dir_all(&secret).unwrap();
        std::fs::write(secret.join("password.txt"), "hunter2").unwrap();
        link_folder(&secret, &map.join("ui/leak"));
        link_folder(&secret, &dir.path().join("multi/linked"));

        let multi = Package::from_path(&dir.path().join("multi")).unwrap();
        let zipped = Package::from_zip_bytes("m".into(), multi.open_binary("m").unwrap()).unwrap();
        let mut names = zipped.relative_paths_of_files_in_subtree("ui");
        names.sort();
        assert_eq!(names, ["ui/tts.txt"]);
        assert!(multi.open_binary("m/ui/leak/password.txt").is_err());
        assert!(multi.open_binary("linked").is_err());
        assert!(multi.subpackage("linked").is_none());
        assert_eq!(multi.dirnames(), ["m"]);
        assert!(multi.filenames().is_empty());
        let m = multi.subpackage("m").unwrap();
        assert_eq!(m.relative_paths_of_files_in_subtree("ui"), ["ui/tts.txt"]);
    }
}
