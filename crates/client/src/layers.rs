//! The layers of resources, and which of their files apply.
//!
//! The layers, lowest first: the port's own phrases, the game, its mods and
//! soundpacks, the campaign, the map, and the player's folder. For a kind
//! of file, [`Layers::texts`] and [`Layers::folders`] give the files that
//! apply, in priority order, each with its layer and what it may do with a
//! name that a lower layer gave. [`Layer::power`] is the one table of what
//! each layer gives: a map that came from a server adds phrases, sounds
//! and style to the interface and never replaces one, and its other files
//! of the interface do not apply.
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::string_slice,
    clippy::arithmetic_side_effects
)]

use crate::package::{Package, TextFile};
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

/// A kind of file that the layers hold.
#[derive(Clone, Copy, Debug)]
pub enum FileKind<'a> {
    /// The language files `text/<language>.txt`, one for each language, in
    /// this order.
    Phrases(&'a [&'a str]),
    /// The recordings under `sounds/`, then the language's own.
    Sounds,
    /// `ui/style.txt`, then the language's own.
    Style,
    /// Another file of the interface, `<name>.txt`, then the language's
    /// own if it has one.
    Interface { name: &'a str, localized: bool },
    /// A file of the game's rules, `<name>.txt`: `rules` or `ai`.
    Game(&'a str),
}

/// The key bindings of the layers.
pub const BINDINGS: FileKind<'static> = FileKind::Interface {
    name: "ui/bindings",
    localized: true,
};
/// The help of the menus.
pub const MENU_HELP: FileKind<'static> = FileKind::Interface {
    name: "ui/menu_help",
    localized: false,
};
/// The help of a game.
pub const GAME_HELP: FileKind<'static> = FileKind::Interface {
    name: "ui/game_help",
    localized: false,
};

impl FileKind<'_> {
    /// The paths of this kind in a layer, lowest priority first, for the
    /// player's `language`.
    fn paths(self, language: &str) -> Vec<String> {
        match self {
            FileKind::Phrases(languages) => languages
                .iter()
                .map(|l| format!("{TEXT_FOLDER}/{l}.txt"))
                .collect(),
            FileKind::Sounds => localized_paths(SOUNDS_FOLDER, Some(language)),
            FileKind::Style => localized_paths("ui/style.txt", Some(language)),
            FileKind::Interface { name, localized } => {
                localized_paths(&format!("{name}.txt"), localized.then_some(language))
            }
            FileKind::Game(name) => vec![format!("{name}.txt")],
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
    pub fn power(&self, kind: FileKind) -> Option<Power> {
        use FileKind as K;
        match (self, kind) {
            // The port and the player's folder hold only language files.
            (Layer::Port | Layer::Player(_), K::Phrases(_)) => Some(Power::Replace),
            (Layer::Port | Layer::Player(_), _) => None,
            (Layer::MapFromServer(_), K::Phrases(_) | K::Sounds | K::Style) => Some(Power::AddOnly),
            (Layer::MapFromServer(_), K::Interface { .. }) => None,
            // The game's rules are the host's, which reads the same map.
            (Layer::MapFromServer(_), K::Game(_)) => Some(Power::Replace),
            (Layer::Game(_) | Layer::Mod(_) | Layer::Campaign(_) | Layer::Map(_), _) => {
                Some(Power::Replace)
            }
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

/// A file that applies, with its layer and what it may do with a name.
pub struct Found<'a, T> {
    pub layer: &'a Layer,
    pub power: Power,
    pub file: T,
}

/// The files of the layers that may replace a name, then those of the
/// layers that only add names, each in priority order.
pub fn by_power<T>(found: Vec<Found<'_, T>>) -> (Vec<T>, Vec<T>) {
    let mut replace = Vec::new();
    let mut add_only = Vec::new();
    for f in found {
        match f.power {
            Power::Replace => replace.push(f.file),
            Power::AddOnly => add_only.push(f.file),
        }
    }
    (replace, add_only)
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

    /// The packages of the game, its mods and soundpacks, the campaign and
    /// the map, lowest first.
    pub fn packages(&self) -> impl Iterator<Item = &Package> {
        self.0
            .iter()
            .filter(|l| !matches!(l, Layer::Port | Layer::Player(_)))
            .filter_map(Layer::package)
    }

    /// The same layers without the map.
    pub fn without_map(&self) -> Layers {
        let layers = self
            .0
            .iter()
            .filter(|l| !matches!(l, Layer::Map(_) | Layer::MapFromServer(_)));
        Layers(layers.cloned().collect())
    }

    /// Each place of a file of `kind`, in priority order, with its layer
    /// and its power.
    fn places<'a>(
        &'a self,
        kind: FileKind<'a>,
        language: &'a str,
    ) -> impl Iterator<Item = (&'a Layer, Power, String)> + 'a {
        self.0.iter().flat_map(move |layer| {
            let paths = match layer.power(kind) {
                Some(power) => kind
                    .paths(language)
                    .into_iter()
                    .map(|path| (layer, power, path))
                    .collect(),
                None => Vec::new(),
            };
            paths.into_iter()
        })
    }

    /// The text files of `kind` that apply, in priority order.
    pub fn texts<'a>(&'a self, kind: FileKind<'a>, language: &'a str) -> Vec<Found<'a, TextFile>> {
        self.places(kind, language)
            .filter_map(|(layer, power, path)| {
                let file = layer.read_text(&path)?;
                Some(Found { layer, power, file })
            })
            .collect()
    }

    /// The folders of `kind` that apply, in priority order, each as its
    /// package and its path there.
    pub fn folders<'a>(
        &'a self,
        kind: FileKind<'a>,
        language: &'a str,
    ) -> Vec<Found<'a, (Package, String)>> {
        self.places(kind, language)
            .filter_map(|(layer, power, path)| {
                let file = (layer.package()?.clone(), path);
                Some(Found { layer, power, file })
            })
            .collect()
    }

    /// The text files of `kind` that a package has and that do not apply.
    pub fn refused(&self, kind: FileKind, language: &str) -> Vec<TextFile> {
        self.0
            .iter()
            .filter(|l| !matches!(l, Layer::Port | Layer::Player(_)))
            .filter(|l| l.power(kind).is_none())
            .flat_map(|l| {
                let paths = kind.paths(language).into_iter();
                paths.filter_map(|path| l.read_text(&path))
            })
            .collect()
    }

    /// The languages that a layer gives a language file for, "en" first.
    /// The port's own come last, so that a language a package has is
    /// matched before one that only the port translates.
    pub fn languages(&self) -> Vec<String> {
        let giving = self
            .0
            .iter()
            .filter(|l| l.power(FileKind::Phrases(&[])) == Some(Power::Replace));
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
    fn package(root: &std::path::Path, name: &str, files: &[&str]) -> Package {
        let dir = root.join(name);
        for file in files {
            let path = dir.join(file);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, format!("{name} {file}\n")).unwrap();
        }
        Package::from_path(&dir).unwrap().with_name(name)
    }

    fn found<T>(found: Vec<Found<'_, T>>, path: impl Fn(&T) -> String) -> Vec<(String, Power)> {
        found.iter().map(|f| (path(&f.file), f.power)).collect()
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
        let layers = Layers::new(
            vec![
                Layer::Game(package(root.path(), "game", &all)),
                Layer::Mod(package(root.path(), "orc", &["text/en.txt", "rules.txt"])),
                Layer::Campaign(package(root.path(), "campaign", &["ui/bindings.txt"])),
                Layer::MapFromServer(package(root.path(), "map", &all)),
            ],
            Some(package(root.path(), "player", &all)),
        );
        let texts = |kind| found(layers.texts(kind, "fr"), |f: &TextFile| f.path.clone());
        let (replace, add) = (Power::Replace, Power::AddOnly);
        let at = |path: &str, power| (path.to_owned(), power);
        assert_eq!(
            texts(FileKind::Phrases(&["en", "fr"])),
            [
                at("sound-rst/text/fr.txt", replace),
                at("game/text/en.txt", replace),
                at("game/text/fr.txt", replace),
                at("orc/text/en.txt", replace),
                at("map/text/en.txt", add),
                at("map/text/fr.txt", add),
                at("player/text/en.txt", replace),
                at("player/text/fr.txt", replace),
            ]
        );
        assert_eq!(
            texts(FileKind::Style),
            [
                at("game/ui/style.txt", replace),
                at("game/lang/fr/ui/style.txt", replace),
                at("map/ui/style.txt", add),
                at("map/lang/fr/ui/style.txt", add),
            ]
        );
        assert_eq!(
            texts(BINDINGS),
            [
                at("game/ui/bindings.txt", replace),
                at("campaign/ui/bindings.txt", replace),
            ]
        );
        assert_eq!(texts(MENU_HELP), [at("game/ui/menu_help.txt", replace)]);
        let refused = |kind| -> Vec<String> {
            let files = layers.refused(kind, "fr");
            files.into_iter().map(|f| f.path).collect()
        };
        assert_eq!(refused(BINDINGS), ["map/ui/bindings.txt"]);
        assert_eq!(refused(MENU_HELP), ["map/ui/menu_help.txt"]);
        assert_eq!(refused(FileKind::Style), Vec::<String>::new());
        // The game's rules are the host's, the map's included.
        assert_eq!(
            texts(FileKind::Game("rules")),
            [
                at("game/rules.txt", replace),
                at("orc/rules.txt", replace),
                at("map/rules.txt", replace),
            ]
        );
        let folders = found(layers.folders(FileKind::Sounds, "fr"), |(p, path)| {
            format!("{}/{path}", p.name)
        });
        assert_eq!(
            folders,
            [
                at("game/sounds", replace),
                at("game/lang/fr/sounds", replace),
                at("orc/sounds", replace),
                at("orc/lang/fr/sounds", replace),
                at("campaign/sounds", replace),
                at("campaign/lang/fr/sounds", replace),
                at("map/sounds", add),
                at("map/lang/fr/sounds", add),
            ]
        );
        let packages: Vec<&str> = layers.packages().map(|p| p.name.as_str()).collect();
        assert_eq!(packages, ["game", "orc", "campaign", "map"]);
        let packages: Vec<String> = layers
            .without_map()
            .packages()
            .map(|p| p.name.clone())
            .collect();
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
