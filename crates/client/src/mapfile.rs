//! Map files: a text file, or a package with `map.txt` and its own
//! resources. The client reads the header; the simulation reads the text.
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::string_slice,
    clippy::arithmetic_side_effects
)]

use std::path::Path;
use std::sync::Arc;

use soundrts_protocol::{MAX_TITLE_PARTS, MapData, MapFileName, MapHash, MapInfo};
use soundrts_world::msg::{Msg, MsgPart};

use crate::package::{MAX_PACKAGE_BYTES, Package, decode_text, read_file};

/// A map, as the menus and the resource stack see it.
#[derive(Clone, Debug)]
pub struct Map {
    /// The file name without extension, restricted to `A-Za-z0-9._-`.
    pub name: String,
    /// The text of the map, with `\n` line ends.
    pub definition: String,
    /// The map's own resources, for a map package.
    pub resources: Option<Package>,
    /// The name, then what the `title` line names.
    pub title: Msg,
    pub nb_players_min: i64,
    pub nb_players_max: i64,
    /// Official maps come from the base package.
    pub official: bool,
    /// The map came from a server, now or in an earlier game: its
    /// resources may add to the interface but not change it (see
    /// [`crate::resource::Resources::set_map`]).
    pub downloaded: bool,
    /// The name with its extension (`.txt` or `.zip`) for a map file; just
    /// the name for a folder.
    pub file_name: String,
    /// The bytes of the map file; `None` for a folder, which cannot be sent
    /// to other players.
    pub bytes: Option<Arc<Vec<u8>>>,
}

/// The title of a map from what a host said of it: its name, said as
/// written, then what its `title` line names.
pub fn title_of(info: &MapInfo) -> Msg {
    let mut title = vec![MsgPart::literal(name_from_path(info.file_name.as_str()))];
    title.extend(info.title.iter().cloned());
    title
}

/// The map name from a file name.
pub fn name_from_path(path: &str) -> String {
    let base = path.rsplit(['/', '\\']).next().unwrap_or(path);
    let stem = match base.rsplit_once('.') {
        Some((stem, _)) if !stem.is_empty() => stem,
        _ => base,
    };
    stem.chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
        .collect()
}

/// The words after `keyword` on a line holding only names, counts and
/// spaces.
fn find_a_line_with<'a>(definition: &'a str, keyword: &str) -> Option<&'a str> {
    definition.split('\n').find_map(|line| {
        let rest = line.strip_prefix(keyword)?;
        let value = rest.trim_start_matches([' ', '\t']);
        let valid = value.len() < rest.len()
            && !value.is_empty()
            && value.chars().all(crate::campaign::is_name_char);
        valid.then_some(value)
    })
}

impl Map {
    /// Reads a map from a file or folder on disk.
    pub fn from_path(path: &Path) -> std::io::Result<Map> {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        if path.is_dir() {
            let mut m = Self::from_package(Package::from_path(path)?, name_from_path(&name))?;
            m.file_name = name;
            Ok(m)
        } else {
            Self::from_bytes(read_file(path, MAX_PACKAGE_BYTES)?, &name)
        }
    }

    /// Reads a map that came from a server, as [`Map::from_bytes`].
    pub fn from_server(buffer: Vec<u8>, name_with_ext: &str) -> std::io::Result<Map> {
        let mut map = Self::from_bytes(buffer, name_with_ext)?;
        map.downloaded = true;
        Ok(map)
    }

    /// Reads a map from its bytes and its file name: text if the name ends
    /// with `.txt`, else a zipped package.
    pub fn from_bytes(buffer: Vec<u8>, name_with_ext: &str) -> std::io::Result<Map> {
        let name = name_from_path(name_with_ext);
        let bytes = Arc::new(buffer.clone());
        let mut map = if name_with_ext.ends_with(".txt") {
            let definition = decode_text(&buffer);
            Self::from_definition(definition, name, None)
        } else {
            let package = Package::from_zip_bytes(name_with_ext.to_owned(), buffer)?;
            Self::from_package(package, name)?
        };
        let base = name_with_ext
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or(name_with_ext);
        map.file_name = base.to_owned();
        map.bytes = Some(bytes);
        Ok(map)
    }

    /// The SHA-256 of the map file; `None` for a folder.
    pub fn hash(&self) -> Option<MapHash> {
        self.bytes.as_deref().map(|b| MapHash::of(b))
    }

    /// The file name other players get: the name and the extension.
    fn wire_file_name(&self) -> Option<MapFileName> {
        let ext = if self.file_name.ends_with(".txt") {
            ".txt"
        } else {
            ".zip"
        };
        MapFileName::new(format!("{}{ext}", self.name)).ok()
    }

    /// What a host tells the server and the other players about this map;
    /// `None` for a map that cannot be sent (a folder, or a name too long).
    pub fn info(&self) -> Option<MapInfo> {
        // What the `title` line names: not the map's name, and not the
        // sound that marks a map that is not official.
        let title = self
            .title
            .iter()
            .filter(|p| matches!(p, MsgPart::Name(_) | MsgPart::Count(_)))
            .cloned()
            .collect();
        Some(MapInfo {
            file_name: self.wire_file_name()?,
            hash: self.hash()?,
            title,
            nb_players_min: u8::try_from(self.nb_players_min).ok()?,
            nb_players_max: u8::try_from(self.nb_players_max).ok()?,
        })
    }

    /// The map file, to send to the server or a player.
    pub fn data(&self) -> Option<MapData> {
        let bytes = soundrts_protocol::MapBytes::new(self.bytes.as_deref()?.clone()).ok()?;
        Some(MapData {
            file_name: self.wire_file_name()?,
            bytes,
        })
    }

    fn from_package(package: Package, name: String) -> std::io::Result<Map> {
        let resources = package.with_name(&name);
        let definition = resources.read_text("map.txt").ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::NotFound, format!("{name}: map.txt"))
        })?;
        Ok(Self::from_definition(definition, name, Some(resources)))
    }

    fn from_definition(definition: String, name: String, resources: Option<Package>) -> Map {
        let mut title: Msg = vec![MsgPart::literal(name.as_str())];
        if let Some(line) = find_a_line_with(&definition, "title") {
            title.extend(
                line.split(' ')
                    .filter_map(MsgPart::named)
                    .take(MAX_TITLE_PARTS),
            );
        }
        let int = |keyword| {
            find_a_line_with(&definition, keyword)
                .and_then(|l| l.trim().parse().ok())
                .unwrap_or(1)
        };
        Map {
            nb_players_min: int("nb_players_min"),
            nb_players_max: int("nb_players_max"),
            name,
            definition,
            resources,
            title,
            official: false,
            downloaded: false,
            file_name: String::new(),
            bytes: None,
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
    use crate::testkit::imported_tests_dir;

    #[test]
    fn a_text_map_has_its_name_title_and_player_counts() {
        let m = Map::from_path(&imported_tests_dir().join("jl1.txt")).unwrap();
        assert_eq!(m.name, "jl1");
        assert_eq!(
            m.title,
            vec![MsgPart::literal("jl1"), MsgPart::Name("numeral_1".into())]
        );
        assert_eq!((m.nb_players_min, m.nb_players_max), (2, 2));
    }

    #[test]
    fn a_map_package_brings_its_resources() {
        for path in ["single/map1", "single/map1.zip"] {
            let m = Map::from_path(&imported_tests_dir().join(path)).unwrap();
            assert_eq!(m.name, "map1");
            assert_eq!(m.definition, "title 11 22\n");
            assert_eq!(
                m.title,
                vec![
                    MsgPart::literal("map1"),
                    MsgPart::Count(11),
                    MsgPart::Count(22)
                ]
            );
            assert_eq!(m.resources.as_ref().unwrap().name, "map1");
        }
    }

    #[test]
    fn a_name_keeps_only_safe_characters() {
        assert_eq!(name_from_path("multi/My map (2).txt"), "Mymap2");
        assert_eq!(name_from_path("jl4.zip"), "jl4");
    }

    #[test]
    fn a_title_keeps_at_most_the_parts_a_title_may_have() {
        let line = "numeral_1 ".repeat(MAX_TITLE_PARTS + 1);
        let m = Map::from_bytes(format!("title {line}\n").into_bytes(), "t.txt").unwrap();
        assert_eq!(m.title.len(), 1 + MAX_TITLE_PARTS);
    }

    #[test]
    fn a_title_names_phrases_and_sounds_and_the_host_tells_the_other_players() {
        let text = "title chapter 2 voice/own/attack\nnb_players_min 2\nnb_players_max 2\n";
        let m = Map::from_bytes(text.as_bytes().to_vec(), "t.txt").unwrap();
        let named = [
            MsgPart::Name("chapter".into()),
            MsgPart::Count(2),
            MsgPart::Name("voice/own/attack".into()),
        ];
        assert_eq!(
            m.title,
            [vec![MsgPart::literal("t")], named.to_vec()].concat()
        );
        let info = m.info().unwrap();
        assert_eq!(info.title, named);
        assert!(info.title_is_valid());
        assert_eq!(title_of(&info), m.title);
        // A line with a word that is no name and no count is no title.
        let m = Map::from_bytes(b"title Chapter 2\n".to_vec(), "t.txt").unwrap();
        assert_eq!(m.title, [MsgPart::literal("t")]);
    }
}
