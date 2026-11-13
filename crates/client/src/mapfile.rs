//! Map files: a text file, or a package with `map.txt` and its own
//! resources. The client reads the header; the simulation reads the text.

use std::path::Path;

use soundrts_world::msg::{Msg, MsgPart};

use crate::package::{Package, decode_text};

/// A map, as the menus and the resource stack see it.
#[derive(Clone, Debug)]
pub struct Map {
    /// The file name without extension, restricted to `A-Za-z0-9._-`.
    pub name: String,
    /// The text of the map, with `\n` line ends.
    pub definition: String,
    /// The map's own resources, for a map package.
    pub resources: Option<Package>,
    /// The name, then the sound numbers of the `title` line.
    pub title: Msg,
    pub nb_players_min: i64,
    pub nb_players_max: i64,
    /// Official maps come from the base package.
    pub official: bool,
}

/// The map name from a file name.
pub fn name_from_path(path: &str) -> String {
    let base = path.rsplit(['/', '\\']).next().unwrap_or(path);
    let stem = match base.rfind('.') {
        Some(i) if i > 0 => &base[..i],
        _ => base,
    };
    stem.chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
        .collect()
}

/// The numbers after `keyword` on a line holding only digits and spaces.
fn find_a_line_with<'a>(definition: &'a str, keyword: &str) -> Option<&'a str> {
    definition.split('\n').find_map(|line| {
        let rest = line.strip_prefix(keyword)?;
        let value = rest.trim_start_matches([' ', '\t']);
        let valid = value.len() < rest.len()
            && !value.is_empty()
            && value.bytes().all(|b| b.is_ascii_digit() || b == b' ');
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
            Self::from_package(Package::from_path(path)?, name_from_path(&name))
        } else {
            Self::from_bytes(std::fs::read(path)?, &name)
        }
    }

    /// Reads a map from its bytes and its file name: text if the name ends
    /// with `.txt`, else a zipped package.
    pub fn from_bytes(buffer: Vec<u8>, name_with_ext: &str) -> std::io::Result<Map> {
        let name = name_from_path(name_with_ext);
        if name_with_ext.ends_with(".txt") {
            let definition = decode_text(&buffer, name_with_ext);
            Ok(Self::from_definition(definition, name, None))
        } else {
            let package = Package::from_zip_bytes(name_with_ext.to_owned(), buffer)?;
            Self::from_package(package, name)
        }
    }

    fn from_package(package: Package, name: String) -> std::io::Result<Map> {
        let resources = package.with_name(&name);
        let definition = resources.read_text("map.txt").ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::NotFound, format!("{name}: map.txt"))
        })?;
        Ok(Self::from_definition(definition, name, Some(resources)))
    }

    fn from_definition(definition: String, name: String, resources: Option<Package>) -> Map {
        let mut title: Msg = vec![MsgPart::Text(name.clone())];
        if let Some(line) = find_a_line_with(&definition, "title") {
            title.extend(
                line.split(' ')
                    .filter_map(|x| x.parse().ok())
                    .map(MsgPart::Num),
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
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testkit::upstream_tests_dir;

    #[test]
    fn a_text_map_has_its_name_title_and_player_counts() {
        let m = Map::from_path(&upstream_tests_dir().join("jl1.txt")).unwrap();
        assert_eq!(m.name, "jl1");
        assert_eq!(m.title, vec![MsgPart::from("jl1"), MsgPart::Num(3001)]);
        assert_eq!((m.nb_players_min, m.nb_players_max), (2, 2));
    }

    #[test]
    fn a_map_package_brings_its_resources() {
        for path in ["single/map1", "single/map1.zip"] {
            let m = Map::from_path(&upstream_tests_dir().join(path)).unwrap();
            assert_eq!(m.name, "map1");
            assert_eq!(m.definition, "title 11 22\n");
            assert_eq!(
                m.title,
                vec![MsgPart::from("map1"), MsgPart::Num(11), MsgPart::Num(22)]
            );
            assert_eq!(m.resources.as_ref().unwrap().name, "map1");
        }
    }

    #[test]
    fn a_name_keeps_only_safe_characters() {
        assert_eq!(name_from_path("multi/My map (2).txt"), "Mymap2");
        assert_eq!(name_from_path("jl4.zip"), "jl4");
    }
}
