//! Packages: folders or zip files of game data, read the same way.

use std::fmt;
use std::io::{Cursor, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use zip::ZipArchive;

/// A zip file held in memory, with its entry names in archive order.
struct ZipData {
    filename: String,
    names: Vec<String>,
    archive: Mutex<ZipArchive<Cursor<Vec<u8>>>>,
}

impl ZipData {
    fn new(filename: String, bytes: Vec<u8>) -> std::io::Result<ZipData> {
        let archive = ZipArchive::new(Cursor::new(bytes)).map_err(std::io::Error::other)?;
        let names = archive
            .file_names()
            .filter_map(Result::ok)
            .map(|n| n.into_owned())
            .collect();
        Ok(ZipData {
            filename,
            names,
            archive: Mutex::new(archive),
        })
    }

    fn read(&self, name: &str) -> std::io::Result<Vec<u8>> {
        let mut archive = self.archive.lock().unwrap_or_else(|p| p.into_inner());
        let mut file = archive.by_name(name).map_err(|e| match e {
            zip::result::ZipError::FileNotFound => {
                std::io::Error::new(std::io::ErrorKind::NotFound, name.to_owned())
            }
            e => std::io::Error::other(e),
        })?;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)?;
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

/// Decodes a text file of the game data. `tts.txt` files may start with a
/// UTF-8 signature, which is skipped; the other files keep it, as upstream.
pub fn decode_text(bytes: &[u8], name: &str) -> String {
    let is_tts = name.rsplit(['/', '\\']).next() == Some("tts.txt");
    let bytes = match bytes.strip_prefix(b"\xef\xbb\xbf") {
        Some(rest) if is_tts => rest,
        _ => bytes,
    };
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
            let bytes = std::fs::read(path)?;
            Self::from_zip_bytes(path.display().to_string(), bytes)
        } else {
            Ok(Package::new(Kind::Folder(path.to_path_buf())))
        }
    }

    /// A zip file held in memory.
    pub fn from_zip_bytes(filename: String, bytes: Vec<u8>) -> std::io::Result<Package> {
        let data = Arc::new(ZipData::new(filename, bytes)?);
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
                let path = root.join(name);
                if path.is_dir() {
                    zip_folder(&path)
                } else {
                    std::fs::read(path)
                }
            }
            Kind::Zip { data, subdir } => data.read(&Self::zip_path(subdir, name)),
        }
    }

    /// The text of a file, decoded and with `\n` line ends, or `None` if it
    /// cannot be read.
    pub fn read_text(&self, name: &str) -> Option<String> {
        let bytes = self.open_binary(name).ok()?;
        Some(decode_text(&bytes, name))
    }

    pub fn isfile(&self, name: &str) -> bool {
        match &self.kind {
            Kind::Folder(root) => root.join(name).is_file(),
            Kind::Zip { data, subdir } => {
                let full = Self::zip_path(subdir, name);
                data.names.contains(&full)
            }
        }
    }

    pub fn isdir(&self, name: &str) -> bool {
        match &self.kind {
            Kind::Folder(root) => root.join(name).is_dir(),
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
            .filter(|e| e.file_type().is_ok_and(|t| t.is_dir() == want_dirs))
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
                walk(root, &root.join(subdir), &mut result);
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
                let path = root.join(subdir);
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
                    return Package::from_zip_bytes(subdir.to_owned(), bytes).ok();
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

fn walk(root: &Path, dir: &Path, out: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = entries.flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    let mut subdirs = Vec::new();
    for e in entries {
        let path = e.path();
        if path.is_dir() {
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
        walk(root, &d, out);
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

/// The limits of a map package another player sends: far above any real
/// map, far below what would exhaust memory.
pub const NETWORK_MAP_LIMITS: ExpansionLimits = ExpansionLimits {
    entry: 64 * 1024 * 1024,
    total: 256 * 1024 * 1024,
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
    let mut archive = ZipArchive::new(Cursor::new(bytes)).map_err(|e| e.to_string())?;
    for i in 0..archive.len() {
        let mut file = archive.by_index(i).map_err(|e| e.to_string())?;
        let name = file.name().map(|n| n.into_owned()).unwrap_or_default();
        let nested = name.to_lowercase().ends_with(".zip");
        let left = limits.entry.min(limits.total - *total);
        let mut kept = Vec::new();
        let mut reader = (&mut file).take(left + 1);
        let n = if nested {
            reader.read_to_end(&mut kept).map_err(|e| e.to_string())? as u64
        } else {
            std::io::copy(&mut reader, &mut std::io::sink()).map_err(|e| e.to_string())?
        };
        if n > left {
            return Err(format!("{name} decompresses to too many bytes"));
        }
        *total += n;
        if nested {
            check_zip(&kept, limits, total, depth + 1)?;
        }
    }
    Ok(())
}

/// A folder as an uncompressed zip file, in memory.
fn zip_folder(dir: &Path) -> std::io::Result<Vec<u8>> {
    let mut names = Vec::new();
    walk(dir, dir, &mut names);
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    for name in names {
        writer
            .start_file(name.as_str(), options)
            .map_err(std::io::Error::other)?;
        writer.write_all(&std::fs::read(dir.join(&name))?)?;
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
    fn text_gets_unix_line_ends_and_tts_files_lose_their_signature() {
        assert_eq!(
            decode_text(b"\xef\xbb\xbfa\r\nb\rc", "ui/tts.txt"),
            "a\nb\nc"
        );
        assert_eq!(decode_text(b"\xef\xbb\xbfa", "ui/style.txt"), "\u{feff}a");
    }
}
