//! The layers of resources, and which of their files apply.
//!
//! The layers, lowest priority first: the port's own phrases, the game,
//! its mods and soundpacks, the campaign, the map, and the player's
//! folder. Every order in which the files of the layers are read is here:
//!
//! - [`Layers::phrases`], [`Layers::sounds`] and [`Layers::style`]: the
//!   files merged by name, each layer's file then its language's own. The
//!   files of the layers that may replace a name come apart from those of
//!   the layers that only add names.
//! - [`Layers::interface`]: a file of the interface read whole, such as
//!   the help, each layer's in turn; the last one is the one read.
//! - [`Layers::bindings`]: the key bindings, each layer's, then this
//!   client's own additions, then the player's file.
//! - [`Layers::game_text`]: the rules or the computer players, each
//!   layer's, the map's included, under a line that names the layer.
//! - [`Layers::languages`]: the languages, the port's own last.
//!
//! [`Layer::power`] and [`Layer::gives`] are the table of what each layer
//! gives. A map that came from a server adds phrases, sounds and style to
//! the interface and never replaces one, its other files of the interface
//! do not apply, and its rules and computer players do, as the host reads
//! them.
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::string_slice,
    clippy::arithmetic_side_effects
)]

use std::path::Path;

use crate::bindings::{CLIENT_BINDINGS, MAX_BINDINGS_BYTES};
use crate::package::{Package, TextFile, read_file, universal_newlines};
use crate::sound_cache::{SOUNDS_FOLDER, TEXT_FOLDER};
use crate::translation::{PORT_LANGUAGE_FILES, port_language_file};

/// The folder of a package that holds what differs in a language: its
/// own `ui/style.txt`, help files and recordings.
pub fn language_folder(language: &str) -> String {
    format!("lang/{language}")
}

/// The path, then its translation if a language is given.
pub fn localized_paths(path: &str, language: Option<&str>) -> Vec<String> {
    let mut result = vec![path.to_owned()];
    if let Some(lang) = language.filter(|l| !l.is_empty()) {
        result.push(format!("{}/{path}", language_folder(lang)));
    }
    result
}

/// Where a message says [`CLIENT_BINDINGS`] are.
const CLIENT_BINDINGS_PATH: &str = "sound-rst/ui/bindings.txt";

/// A kind of file whose layers are merged by name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Merged {
    Phrases,
    Sounds,
    Style,
}

/// Who reads a file that a layer gives whole.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Reader {
    Interface,
    Game,
}

/// A file of the interface that is read whole: `<name>.txt`, then the
/// language's own if the file has one.
#[derive(Clone, Copy, Debug)]
pub struct InterfaceFile {
    name: &'static str,
    localized: bool,
}

impl InterfaceFile {
    /// The help of the menus.
    pub const MENU_HELP: InterfaceFile = InterfaceFile {
        name: "ui/menu_help",
        localized: false,
    };
    /// The help of a game.
    pub const GAME_HELP: InterfaceFile = InterfaceFile {
        name: "ui/game_help",
        localized: false,
    };
    /// The key bindings: read only through [`Layers::bindings`], with what
    /// comes after them.
    const BINDINGS: InterfaceFile = InterfaceFile {
        name: "ui/bindings",
        localized: true,
    };
    /// Every file of the interface read whole.
    const ALL: [InterfaceFile; 3] = [Self::BINDINGS, Self::MENU_HELP, Self::GAME_HELP];

    /// Any file of the interface, for a test.
    #[cfg(test)]
    pub fn named(name: &'static str, localized: bool) -> InterfaceFile {
        InterfaceFile { name, localized }
    }

    fn paths(self, language: &str) -> Vec<String> {
        let path = format!("{}.txt", self.name);
        localized_paths(&path, self.localized.then_some(language))
    }
}

/// A file of the game's rules, which the simulation reads.
#[derive(Clone, Copy, Debug)]
pub enum GameFile {
    Rules,
    Ai,
}

impl GameFile {
    fn name(self) -> &'static str {
        match self {
            GameFile::Rules => "rules",
            GameFile::Ai => "ai",
        }
    }
}

/// What a layer's file may do with a name that a lower layer gave.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Power {
    Replace,
    AddOnly,
}

/// A layer of resources.
#[derive(Clone, Debug, PartialEq)]
pub enum Layer {
    /// The port's own phrases, built into the program.
    Port,
    /// The game, the base package.
    Game(Package),
    /// A mod or a soundpack.
    Mod(Package),
    /// The campaign being played.
    Campaign(Package),
    /// The map of the game, installed by the player or in a campaign.
    Map(Package),
    /// The map of the game, which came from a server.
    MapFromServer(Package),
    /// The player's folder.
    Player(Package),
}

impl Layer {
    /// What this layer's files of `kind` may do with the names below, or
    /// `None` if they do not apply.
    fn power(&self, kind: Merged) -> Option<Power> {
        match self {
            // The port and the player's folder hold only language files.
            Layer::Port | Layer::Player(_) => (kind == Merged::Phrases).then_some(Power::Replace),
            Layer::MapFromServer(_) => Some(Power::AddOnly),
            Layer::Game(_) | Layer::Mod(_) | Layer::Campaign(_) | Layer::Map(_) => {
                Some(Power::Replace)
            }
        }
    }

    /// Whether this layer's files that `reader` reads whole apply.
    fn gives(&self, reader: Reader) -> bool {
        match self {
            Layer::Port | Layer::Player(_) => false,
            // The game's rules are the host's, which reads the same map.
            Layer::MapFromServer(_) => reader == Reader::Game,
            Layer::Game(_) | Layer::Mod(_) | Layer::Campaign(_) | Layer::Map(_) => true,
        }
    }

    /// What the layer is, for a message.
    pub fn describe(&self) -> &'static str {
        match self {
            Layer::Port => "the port",
            Layer::Game(_) => "the game",
            Layer::Mod(_) => "a mod",
            Layer::Campaign(_) => "the campaign",
            Layer::Map(_) => "the map",
            Layer::MapFromServer(_) => "a map from a server",
            Layer::Player(_) => "the player's folder",
        }
    }

    /// The package of the layer; the port has none.
    pub fn package(&self) -> Option<&Package> {
        match self {
            Layer::Port => None,
            Layer::Game(p)
            | Layer::Mod(p)
            | Layer::Campaign(p)
            | Layer::Map(p)
            | Layer::MapFromServer(p)
            | Layer::Player(p) => Some(p),
        }
    }

    /// The text file at `path`, named by its layer for a message.
    fn read_text(&self, path: &str) -> Option<TextFile> {
        match self.package() {
            Some(package) => {
                let text = package.read_text(path)?;
                let path = format!("{}/{path}", package.name);
                Some(TextFile { path, text })
            }
            None => path
                .strip_prefix(TEXT_FOLDER)
                .and_then(|p| p.strip_prefix('/'))
                .and_then(|p| p.strip_suffix(".txt"))
                .and_then(port_language_file),
        }
    }

    /// The languages this layer has a language file for.
    fn languages(&self) -> Vec<String> {
        let Some(package) = self.package() else {
            return PORT_LANGUAGE_FILES
                .iter()
                .map(|(l, _)| (*l).to_owned())
                .collect();
        };
        let Some(texts) = package.subpackage(TEXT_FOLDER) else {
            return Vec::new();
        };
        texts
            .filenames()
            .iter()
            .filter_map(|name| name.strip_suffix(".txt"))
            .map(str::to_owned)
            .collect()
    }
}

/// The files of a kind merged by name, each part in priority order.
#[derive(Debug, PartialEq)]
pub struct ByPower<T> {
    /// Those of the layers that may replace a name.
    pub replace: Vec<T>,
    /// Those of the layers that only add names, which come above.
    pub add_only: Vec<T>,
}

impl<T> ByPower<T> {
    fn of(found: impl IntoIterator<Item = (Power, T)>) -> ByPower<T> {
        let mut result = ByPower {
            replace: Vec::new(),
            add_only: Vec::new(),
        };
        for (power, file) in found {
            match power {
                Power::Replace => result.replace.push(file),
                Power::AddOnly => result.add_only.push(file),
            }
        }
        result
    }
}

/// The layers, lowest priority first: the port, the packages, then the
/// player's folder.
#[derive(Clone, Debug, PartialEq)]
pub struct Layers(Vec<Layer>);

impl Layers {
    /// The port's phrases, then `packages` lowest first, then the player's
    /// folder.
    pub fn new(packages: Vec<Layer>, player: Option<Package>) -> Layers {
        let mut layers = vec![Layer::Port];
        layers.extend(packages);
        layers.extend(player.map(Layer::Player));
        Layers(layers)
    }

    /// The layers of the game, its mods and soundpacks, the campaign and
    /// the map, lowest first.
    fn package_layers(&self) -> impl Iterator<Item = &Layer> {
        self.0
            .iter()
            .filter(|l| !matches!(l, Layer::Port | Layer::Player(_)))
    }

    /// The packages of the game, its mods and soundpacks, the campaign and
    /// the map, lowest first.
    pub fn packages(&self) -> impl Iterator<Item = &Package> {
        self.package_layers().filter_map(Layer::package)
    }

    /// The same layers without the map.
    pub fn without_map(&self) -> Layers {
        let layers = self
            .0
            .iter()
            .filter(|l| !matches!(l, Layer::Map(_) | Layer::MapFromServer(_)));
        Layers(layers.cloned().collect())
    }

    /// The text files at `paths` of each layer that `applies` gives a
    /// value, in priority order, each with its layer and that value.
    fn read<P: Copy>(
        &self,
        paths: &[String],
        applies: impl Fn(&Layer) -> Option<P>,
    ) -> Vec<(&Layer, P, TextFile)> {
        let mut found = Vec::new();
        for layer in &self.0 {
            let Some(value) = applies(layer) else {
                continue;
            };
            for path in paths {
                found.extend(layer.read_text(path).map(|file| (layer, value, file)));
            }
        }
        found
    }

    fn merged(&self, kind: Merged, paths: &[String]) -> ByPower<TextFile> {
        let found = self.read(paths, |l| l.power(kind));
        ByPower::of(found.into_iter().map(|(_, power, file)| (power, file)))
    }

    /// The language files `text/<language>.txt` of each language, in this
    /// order, for each layer.
    pub fn phrases(&self, languages: &[&str]) -> ByPower<TextFile> {
        let paths: Vec<String> = languages
            .iter()
            .map(|l| format!("{TEXT_FOLDER}/{l}.txt"))
            .collect();
        self.merged(Merged::Phrases, &paths)
    }

    /// Each layer's `ui/style.txt`, then the language's own.
    pub fn style(&self, language: &str) -> ByPower<TextFile> {
        self.merged(
            Merged::Style,
            &localized_paths("ui/style.txt", Some(language)),
        )
    }

    /// Each layer's folder of recordings, then the language's own, as its
    /// package and its path there.
    pub fn sounds(&self, language: &str) -> ByPower<(Package, String)> {
        let paths = localized_paths(SOUNDS_FOLDER, Some(language));
        let mut found = Vec::new();
        for layer in &self.0 {
            let (Some(power), Some(package)) = (layer.power(Merged::Sounds), layer.package())
            else {
                continue;
            };
            for path in &paths {
                found.push((power, (package.clone(), path.clone())));
            }
        }
        ByPower::of(found)
    }

    /// The interface's files `file`, in priority order.
    pub fn interface(&self, file: InterfaceFile, language: &str) -> Vec<TextFile> {
        let paths = file.paths(language);
        let found = self.read(&paths, |l| l.gives(Reader::Interface).then_some(()));
        found.into_iter().map(|(_, (), file)| file).collect()
    }

    /// The files of the key bindings, in the order they are read: each
    /// layer's `ui/bindings.txt` and its language's own, this client's own
    /// additions, then the player's file at `custom` if there is one, so
    /// the player's lines win.
    pub fn bindings(&self, language: &str, custom: &Path) -> Vec<TextFile> {
        let mut files = self.interface(InterfaceFile::BINDINGS, language);
        files.push(TextFile {
            path: CLIENT_BINDINGS_PATH.to_owned(),
            text: CLIENT_BINDINGS.to_owned(),
        });
        let limit = u64::try_from(MAX_BINDINGS_BYTES).unwrap_or(u64::MAX);
        if let Ok(bytes) = read_file(custom, limit) {
            files.push(TextFile {
                path: custom.display().to_string(),
                text: universal_newlines(&String::from_utf8_lossy(&bytes)),
            });
        }
        files
    }

    /// The contents of every `<name>.txt` of the layers, the map's
    /// included, joined by line ends. Each is under a comment line that
    /// names its layer and file, so that the simulation, which takes only
    /// this text, can say where a line it refuses is. Players can play
    /// together only with the same text, so the map's layer is `map` and
    /// not its name: a map kept from a server has another file name.
    pub fn game_text(&self, file: GameFile) -> String {
        let name = file.name();
        let paths = [format!("{name}.txt")];
        self.read(&paths, |l| l.gives(Reader::Game).then_some(()))
            .into_iter()
            .filter_map(|(layer, (), found)| {
                let label = match layer {
                    Layer::Map(_) | Layer::MapFromServer(_) => "map",
                    other => other.package()?.name.as_str(),
                };
                let file = format!("{label}/{name}.txt");
                Some(soundrts_world::defs::with_file_name(&file, &found.text))
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// The files of the interface read whole that a package has and that
    /// do not apply, each with its layer.
    pub fn refused(&self, language: &str) -> Vec<(&Layer, TextFile)> {
        let mut found = Vec::new();
        for layer in self.package_layers() {
            if layer.gives(Reader::Interface) {
                continue;
            }
            for file in InterfaceFile::ALL {
                for path in file.paths(language) {
                    found.extend(layer.read_text(&path).map(|f| (layer, f)));
                }
            }
        }
        found
    }

    /// The languages that a layer gives a language file for, "en" first.
    /// The port's own come last, so that a language a package has is
    /// matched before one that only the port translates.
    pub fn languages(&self) -> Vec<String> {
        let giving = self
            .0
            .iter()
            .filter(|l| l.power(Merged::Phrases) == Some(Power::Replace));
        let (port, packages): (Vec<&Layer>, Vec<&Layer>) =
            giving.partition(|l| matches!(l, Layer::Port));
        let mut result = vec!["en".to_owned()];
        for language in packages.into_iter().chain(port).flat_map(Layer::languages) {
            if !result.contains(&language) {
                result.push(language);
            }
        }
        result
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    #[test]
    fn a_language_has_its_own_folder_in_a_package() {
        assert_eq!(localized_paths("ui/style.txt", None), ["ui/style.txt"]);
        assert_eq!(localized_paths("sounds", Some("")), ["sounds"]);
        assert_eq!(
            localized_paths("ui/style.txt", Some("pt-BR")),
            ["ui/style.txt", "lang/pt-BR/ui/style.txt"]
        );
        assert_eq!(
            localized_paths("sounds", Some("de")),
            ["sounds", "lang/de/sounds"]
        );
    }

    /// A folder package called `name`, with these files.
    fn package(root: &Path, name: &str, files: &[&str]) -> Package {
        let dir = root.join(name);
        for file in files {
            let path = dir.join(file);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, format!("{name} {file}\n")).unwrap();
        }
        Package::from_path(&dir).unwrap().with_name(name)
    }

    fn paths(files: &[TextFile]) -> Vec<&str> {
        files.iter().map(|f| f.path.as_str()).collect()
    }

    #[test]
    fn each_kind_of_file_has_one_order_and_a_map_from_a_server_only_adds_to_the_interface() {
        let root = tempfile::tempdir().unwrap();
        let all = [
            "text/en.txt",
            "text/fr.txt",
            "ui/style.txt",
            "lang/fr/ui/style.txt",
            "ui/bindings.txt",
            "ui/menu_help.txt",
            "rules.txt",
        ];
        let custom = root.path().join("my bindings.txt");
        std::fs::write(&custom, "F12: say_time\n").unwrap();
        let layers = Layers::new(
            vec![
                Layer::Game(package(root.path(), "game", &all)),
                Layer::Mod(package(root.path(), "orc", &["text/en.txt", "rules.txt"])),
                Layer::Campaign(package(root.path(), "campaign", &["ui/bindings.txt"])),
                Layer::MapFromServer(package(root.path(), "map", &all)),
            ],
            Some(package(root.path(), "player", &all)),
        );
        let phrases = layers.phrases(&["en", "fr"]);
        assert_eq!(
            paths(&phrases.replace),
            [
                "sound-rst/text/fr.txt",
                "game/text/en.txt",
                "game/text/fr.txt",
                "orc/text/en.txt",
                "player/text/en.txt",
                "player/text/fr.txt",
            ]
        );
        assert_eq!(
            paths(&phrases.add_only),
            ["map/text/en.txt", "map/text/fr.txt"]
        );
        let style = layers.style("fr");
        assert_eq!(
            paths(&style.replace),
            ["game/ui/style.txt", "game/lang/fr/ui/style.txt"]
        );
        assert_eq!(
            paths(&style.add_only),
            ["map/ui/style.txt", "map/lang/fr/ui/style.txt"]
        );
        let sounds = layers.sounds("fr");
        let folders = |found: &[(Package, String)]| -> Vec<String> {
            found
                .iter()
                .map(|(p, path)| format!("{}/{path}", p.name))
                .collect()
        };
        assert_eq!(
            folders(&sounds.replace),
            [
                "game/sounds",
                "game/lang/fr/sounds",
                "orc/sounds",
                "orc/lang/fr/sounds",
                "campaign/sounds",
                "campaign/lang/fr/sounds",
            ]
        );
        assert_eq!(
            folders(&sounds.add_only),
            ["map/sounds", "map/lang/fr/sounds"]
        );
        let custom_path = custom.display().to_string();
        assert_eq!(
            paths(&layers.bindings("fr", &custom)),
            [
                "game/ui/bindings.txt",
                "campaign/ui/bindings.txt",
                CLIENT_BINDINGS_PATH,
                custom_path.as_str(),
            ]
        );
        let help = layers.interface(InterfaceFile::MENU_HELP, "fr");
        assert_eq!(paths(&help), ["game/ui/menu_help.txt"]);
        let refused = layers.refused("fr");
        let refused: Vec<(&str, &str)> = refused
            .iter()
            .map(|(layer, file)| (layer.describe(), file.path.as_str()))
            .collect();
        assert_eq!(
            refused,
            [
                ("a map from a server", "map/ui/bindings.txt"),
                ("a map from a server", "map/ui/menu_help.txt"),
            ]
        );
        // The game's rules are the host's, the map's included.
        assert_eq!(
            layers.game_text(GameFile::Rules),
            "; file: game/rules.txt\ngame rules.txt\n\n\
             ; file: orc/rules.txt\norc rules.txt\n\n\
             ; file: map/rules.txt\nmap rules.txt\n"
        );
        let packages: Vec<&str> = layers.packages().map(|p| p.name.as_str()).collect();
        assert_eq!(packages, ["game", "orc", "campaign", "map"]);
        let without = layers.without_map();
        let packages: Vec<&str> = without.packages().map(|p| p.name.as_str()).collect();
        assert_eq!(packages, ["game", "orc", "campaign"]);
    }

    #[test]
    fn a_language_of_a_package_is_listed_before_one_only_the_port_translates() {
        let root = tempfile::tempdir().unwrap();
        let layers = Layers::new(
            vec![
                Layer::Game(package(root.path(), "game", &["text/pt-PT.txt"])),
                Layer::MapFromServer(package(root.path(), "map", &["text/xx.txt"])),
            ],
            Some(package(root.path(), "player", &["text/yy.txt"])),
        );
        let languages = layers.languages();
        assert_eq!(languages[..3], ["en", "pt-PT", "yy"]);
        assert!(languages.contains(&"pt-BR".to_owned()));
        assert!(!languages.contains(&"xx".to_owned()));
    }
}
