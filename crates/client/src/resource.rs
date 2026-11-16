//! The resource stack: which file is read depends on the preferred
//! language, the active mods and soundpacks, the campaign and the map.
//! Sounds are replaced by later layers; some text files are merged.
//!
//! A map that came from a server is the top layer for the game's rules,
//! but not for the interface: see [`Resources::set_map`].
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::string_slice,
    clippy::arithmetic_side_effects
)]

use std::path::PathBuf;
use std::sync::Arc;

use crate::campaign::Campaign;
use crate::log::Log;
use crate::mapfile::Map;
use crate::package::{Package, PackageStack};
use crate::parameters::Parameters;
use crate::sound_cache::{LanguageFile, SOUNDS_FOLDER, SoundCache, Sounds, TEXT_FOLDER};
use crate::style::Style;

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

/// The available language that best matches the preferred one: the same
/// code, else the same main code, else a longer code with that main code,
/// else "en".
pub fn best_language_match<S: AsRef<str>>(lang: Option<&str>, available: &[S]) -> String {
    let mut lang = lang.unwrap_or("").replace('_', "-");
    for a in available {
        if lang.to_lowercase() == a.as_ref().to_lowercase() {
            return a.as_ref().to_owned();
        }
    }
    if let Some((main, _)) = lang.split_once('-') {
        lang = main.to_owned();
    }
    for a in available {
        if lang.to_lowercase() == a.as_ref().to_lowercase() {
            return a.as_ref().to_owned();
        }
    }
    for a in available {
        if let Some((short, _)) = a.as_ref().split_once('-')
            && lang.to_lowercase() == short.to_lowercase()
        {
            return a.as_ref().to_owned();
        }
    }
    "en".to_owned()
}

/// The language the player chose in `settings.toml`, else the system's.
pub fn preferred_language(chosen: Option<&str>, log: &Log) -> Option<String> {
    if let Some(chosen) = chosen.map(str::trim).filter(|l| !l.is_empty()) {
        return Some(chosen.to_owned());
    }
    let system = sys_locale::get_locale();
    if system.is_none() {
        log.warning(
            "Couldn't get the system language. To use another language, write \
             language = \"pl\" for example in settings.toml.",
        );
    }
    system
}

/// Called with the new mods and soundpacks whenever the layers change.
pub type ChangeListener = Box<dyn FnMut(&str, &str)>;

/// The resource stack.
pub struct Resources {
    package_paths: Vec<PathBuf>,
    downloaded: PathBuf,
    pub packages: PackageStack,
    layers: Vec<Package>,
    /// The layers and whether the map came from a server, when the style
    /// and sounds were last read.
    previous_layers: Option<(Vec<Package>, bool)>,
    /// The active mods, as a comma separated list of those that exist.
    pub mods: String,
    pub soundpacks: String,
    campaign: Option<Package>,
    map: Option<Package>,
    /// The map came from a server.
    map_downloaded: bool,
    /// The language of the texts and sounds.
    pub language: String,
    preferred_language: Option<String>,
    style: Style,
    sounds: Sounds,
    params: Arc<Parameters>,
    log: Log,
    on_change: Option<ChangeListener>,
    multi_maps: Option<(String, Vec<Arc<Map>>)>,
}

impl Resources {
    /// The stack for these package paths (the base package first).
    /// `downloaded` is the player's folder of downloaded maps.
    pub fn new(
        package_paths: Vec<PathBuf>,
        downloaded: PathBuf,
        preferred_language: Option<String>,
        params: Arc<Parameters>,
        log: Log,
    ) -> Resources {
        let empty = SoundCache::new(&[], &[], params.clone(), log.clone());
        let mut r = Resources {
            packages: PackageStack::new(&package_paths),
            package_paths,
            downloaded,
            layers: Vec::new(),
            previous_layers: None,
            mods: String::new(),
            soundpacks: String::new(),
            campaign: None,
            map: None,
            map_downloaded: false,
            language: "en".to_owned(),
            preferred_language,
            style: Style::new(log.clone()),
            sounds: Sounds::new(empty),
            params,
            log,
            on_change: None,
            multi_maps: None,
        };
        r.reload();
        r
    }

    /// The layers, in loading order: base, mods, soundpacks, campaign, map.
    pub fn layers(&self) -> &[Package] {
        &self.layers
    }

    pub fn style(&self) -> &Style {
        &self.style
    }

    /// The sound cache handle; it follows every reload.
    pub fn sounds(&self) -> Sounds {
        self.sounds.clone()
    }

    /// Replaces the parameters used for the sounds loaded from now on.
    pub fn set_params(&mut self, params: Arc<Parameters>) {
        self.params = params;
    }

    /// Calls `f` now and after each change of the layers.
    pub fn register(&mut self, mut f: ChangeListener) {
        f(&self.mods, &self.soundpacks);
        self.on_change = Some(f);
    }

    /// Opens the packages again, to see new mods and soundpacks.
    pub fn update_packages(&mut self) {
        self.packages = PackageStack::new(&self.package_paths);
        self.previous_layers = None;
        self.reload();
    }

    pub fn available_mods(&self) -> Vec<String> {
        self.packages
            .mods()
            .into_iter()
            .filter(|m| !m.is_a_soundpack())
            .map(|m| m.name)
            .collect()
    }

    pub fn available_soundpacks(&self) -> Vec<String> {
        self.packages
            .mods()
            .into_iter()
            .filter(|m| m.is_a_soundpack())
            .map(|m| m.name)
            .collect()
    }

    /// The map's layer, if the map came from a server.
    fn downloaded_map(&self) -> Option<&Package> {
        self.map.as_ref().filter(|_| self.map_downloaded)
    }

    /// The layers the interface reads its style, texts, sounds, key
    /// bindings and help from: every layer but a map that came from a
    /// server, which is the last layer.
    fn interface_layers(&self) -> &[Package] {
        match (self.downloaded_map(), self.layers.split_last()) {
            (Some(_), Some((_, below))) => below,
            _ => &self.layers,
        }
    }

    fn available_languages(&self) -> Vec<String> {
        let mut result = vec!["en".to_owned()];
        for package in self.interface_layers() {
            let Some(texts) = package.subpackage(TEXT_FOLDER) else {
                continue;
            };
            for name in texts.filenames() {
                if let Some(lang) = name.strip_suffix(".txt")
                    && !result.iter().any(|r| r == lang)
                {
                    result.push(lang.to_owned());
                }
            }
        }
        result
    }

    fn add_layer(&mut self, m: Package, added: &mut Vec<String>) {
        added.push(m.name.clone());
        for required in m.required_mods.clone() {
            if !added.contains(&required)
                && let Some(r) = self.packages.find_mod(&required)
            {
                self.add_layer(r, added);
            }
        }
        self.layers.push(m);
    }

    /// Adds the existing mods of the list as layers; returns the list of
    /// those that exist.
    fn add_layers(&mut self, mods: &str) -> String {
        let mut actual = Vec::new();
        let mut added = Vec::new();
        for name in mods.split(',').map(str::trim) {
            if name.is_empty() {
                continue;
            }
            if let Some(m) = self.packages.find_mod(name) {
                self.add_layer(m, &mut added);
                actual.push(name.to_owned());
            }
        }
        actual.join(",")
    }

    fn reload(&mut self) {
        self.layers = self.packages.0.iter().take(1).cloned().collect();
        let mods = self.mods.clone();
        self.mods = self.add_layers(&mods);
        let soundpacks = self.soundpacks.clone();
        self.soundpacks = self.add_layers(&soundpacks);
        if let Some(c) = self.campaign.clone() {
            self.layers.push(c);
        }
        if let Some(m) = self.map.clone() {
            self.layers.push(m);
        }
        let current = (self.layers.clone(), self.map_downloaded);
        if self.previous_layers.as_ref() != Some(&current) {
            self.language = best_language_match(
                self.preferred_language.as_deref(),
                &self.available_languages(),
            );
            self.load_style_and_sounds();
            if let Some(f) = self.on_change.as_mut() {
                f(&self.mods, &self.soundpacks);
            }
            self.previous_layers = Some(current);
        }
    }

    /// Reads the style and the sound cache again, for the current language.
    pub fn load_style_and_sounds(&mut self) {
        let style_texts = self.texts("ui/style", true, None);
        let refs: Vec<&str> = style_texts.iter().map(String::as_str).collect();
        self.style.load(&refs);
        let texts = self.language_files(None);
        let dirs: Vec<(Package, String)> = self.paths(SOUNDS_FOLDER, None, true);
        let mut cache = SoundCache::new(&texts, &dirs, self.params.clone(), self.log.clone());
        if let Some(map) = self.downloaded_map().cloned() {
            let style_texts = self.texts("ui/style", true, Some(&map));
            let refs: Vec<&str> = style_texts.iter().map(String::as_str).collect();
            let style = self.style.add_downloaded(&refs);
            let texts = self.language_files(Some(&map));
            let sounds = cache.add_downloaded(&texts, &self.paths(SOUNDS_FOLDER, Some(&map), true));
            if style > 0 || sounds > 0 {
                self.log.warning(format!(
                    "downloaded map {}: ignored {style} style attributes and {sounds} phrases or sounds the game already has",
                    map.name
                ));
            }
            for name in ["ui/bindings", "ui/menu_help", "ui/game_help"] {
                if !self.texts(name, true, Some(&map)).is_empty() {
                    self.log
                        .warning(format!("downloaded map {}: {name}.txt ignored", map.name));
                }
            }
        }
        self.sounds.replace(cache);
    }

    /// Changes the language, as if it were the best match.
    pub fn set_language(&mut self, language: &str) {
        self.language = language.to_owned();
        self.load_style_and_sounds();
    }

    pub fn set_mods(&mut self, mods: &str) {
        if mods != self.mods {
            self.mods = mods.to_owned();
            self.reload();
        }
    }

    pub fn set_soundpacks(&mut self, soundpacks: &str) {
        if soundpacks != self.soundpacks {
            self.soundpacks = soundpacks.to_owned();
            self.reload();
        }
    }

    /// Adds the map's resources as the top layer, or removes them.
    ///
    /// A map that came from a server gives the game its map, rules and
    /// computer players, and adds to the style, texts and sounds only
    /// what the other layers lack, such as its own unit types. Its key
    /// bindings and help are ignored, and so is anything that would change
    /// the game's own style, texts or sounds.
    pub fn set_map(&mut self, map: Option<&Map>) {
        self.map = map.and_then(|m| m.resources.clone());
        self.map_downloaded = map.is_some_and(|m| m.downloaded);
        self.reload();
    }

    /// Adds the campaign's resources as a layer, or removes them.
    pub fn set_campaign(&mut self, campaign: Option<&Campaign>) {
        self.campaign = campaign.map(|c| c.resources.clone());
        self.reload();
    }

    /// The (package, path) pairs to look `path` up in, in priority order:
    /// in `root`, or else in the interface's layers.
    pub fn paths(
        &self,
        path: &str,
        root: Option<&Package>,
        localize: bool,
    ) -> Vec<(Package, String)> {
        let lang = localize.then_some(self.language.as_str());
        let roots: Vec<&Package> = match root {
            Some(r) => vec![r],
            None => self.interface_layers().iter().collect(),
        };
        roots
            .into_iter()
            .flat_map(|r| {
                localized_paths(path, lang)
                    .into_iter()
                    .map(move |p| (r.clone(), p))
            })
            .collect()
    }

    /// The language files of `root`, or else of the interface's layers, in
    /// priority order: for each layer the English, then the language's.
    fn language_files(&self, root: Option<&Package>) -> Vec<LanguageFile> {
        let roots: Vec<&Package> = match root {
            Some(r) => vec![r],
            None => self.interface_layers().iter().collect(),
        };
        let mut languages = vec!["en"];
        if self.language != "en" {
            languages.push(&self.language);
        }
        let mut files = Vec::new();
        for package in roots {
            for language in &languages {
                let name = format!("{TEXT_FOLDER}/{language}.txt");
                if let Some(text) = package.read_text(&name) {
                    files.push(LanguageFile {
                        path: format!("{}/{name}", package.name),
                        text,
                    });
                }
            }
        }
        files
    }

    /// The contents of every `<name>.txt` of `root`, or else of the
    /// interface's layers, in priority order.
    pub fn texts(&self, name: &str, localize: bool, root: Option<&Package>) -> Vec<String> {
        self.paths(&format!("{name}.txt"), root, localize)
            .into_iter()
            .filter_map(|(package, path)| package.read_text(&path))
            .collect()
    }

    /// The content of the `<name>.txt` with the highest priority.
    pub fn text(&self, name: &str, localize: bool) -> Option<String> {
        self.texts(name, localize, None).pop()
    }

    /// The contents of every `<name>.txt`, joined by line ends.
    pub fn text_appended(&self, name: &str, localize: bool) -> String {
        self.texts(name, localize, None).join("\n")
    }

    /// The contents of every `<name>.txt` of every layer, the map's
    /// included, joined by line ends. Each is under a comment line that
    /// names its layer and file, so that the simulation, which takes only
    /// this text, can say where a line it refuses is.
    fn game_text(&self, name: &str) -> String {
        self.layers
            .iter()
            .flat_map(|p| {
                let file = format!("{}/{name}.txt", p.name);
                let texts = self.texts(name, false, Some(p)).into_iter();
                texts.map(move |t| soundrts_world::defs::with_file_name(&file, &t))
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// The rules for a new world: every `rules.txt` of the layers.
    pub fn rules_text(&self) -> String {
        self.game_text("rules")
    }

    /// The computer players for a new world: every `ai.txt` of the layers.
    pub fn ai_text(&self) -> String {
        self.game_text("ai")
    }

    /// The official maps, from the base package's `multi` folder.
    pub fn official_multiplayer_maps(&self) -> Vec<Map> {
        let Some(official) = self.packages.0.first().and_then(|p| p.subpackage("multi")) else {
            return Vec::new();
        };
        let mut maps = Vec::new();
        for n in official.filenames() {
            match official
                .open_binary(&n)
                .and_then(|b| Map::from_bytes(b, &n))
            {
                Ok(mut m) => {
                    m.official = true;
                    maps.push(m);
                }
                Err(e) => self.log.warning(format!("couldn't load map {n}: {e}")),
            }
        }
        maps
    }

    /// The maps of the other packages, then those downloaded from servers.
    /// The folder of downloaded maps can also be one of the packages; it is
    /// read once, as downloaded.
    fn custom_multiplayer_maps(&self) -> Vec<Map> {
        let mut maps = Vec::new();
        let installed = self
            .packages
            .0
            .iter()
            .skip(1)
            .filter(|p| p.folder() != Some(self.downloaded.as_path()))
            .map(|p| (p.clone(), false));
        let downloaded = Package::from_path(&self.downloaded).ok().map(|p| (p, true));
        for (package, is_downloaded) in installed.chain(downloaded) {
            let Some(multi) = package.subpackage("multi") else {
                continue;
            };
            for n in multi.filenames().into_iter().chain(multi.dirnames()) {
                match multi.open_binary(&n).and_then(|b| Map::from_bytes(b, &n)) {
                    Ok(mut m) => {
                        // The heal sound tells the player this map is not official.
                        m.title
                            .insert(0, soundrts_world::msg::Cue::CustomMap.into());
                        m.downloaded = is_downloaded;
                        maps.push(m);
                    }
                    Err(e) => self.log.warning(format!("couldn't load map {n}: {e}")),
                }
            }
        }
        maps
    }

    /// The multiplayer maps: official ones first, sorted by title, with
    /// the recommended maps (from style) copied to the top.
    pub fn multiplayer_maps(&mut self) -> Vec<Arc<Map>> {
        if let Some((mods, maps)) = &self.multi_maps
            && *mods == self.mods
        {
            return maps.clone();
        }
        self.reload();
        let mut maps: Vec<Arc<Map>> = self
            .official_multiplayer_maps()
            .into_iter()
            .chain(self.custom_multiplayer_maps())
            .map(Arc::new)
            .collect();
        // Custom maps come after official maps: a sound sorts after any
        // letter.
        let key = |m: &Arc<Map>| -> Vec<String> {
            m.title
                .iter()
                .map(|p| match p {
                    soundrts_world::msg::MsgPart::Literal(s) => s.to_lowercase(),
                    _ => "{".to_owned(),
                })
                .collect()
        };
        maps.sort_by_key(key);
        let recommended = self.style.words("parameters", "recommended_maps");
        for n in recommended.iter().rev() {
            for m in maps.clone().iter().rev() {
                if m.name == *n {
                    maps.insert(0, m.clone());
                }
            }
        }
        self.multi_maps = Some((self.mods.clone(), maps.clone()));
        maps
    }

    /// Forgets the multiplayer maps read before, so that the next call to
    /// [`Resources::multiplayer_maps`] sees a map just saved.
    pub fn forget_multiplayer_maps(&mut self) {
        self.multi_maps = None;
    }

    /// The multiplayer map whose file has this hash.
    pub fn find_multiplayer_map(&mut self, hash: &soundrts_protocol::MapHash) -> Option<Arc<Map>> {
        self.multiplayer_maps()
            .into_iter()
            .find(|m| m.hash().as_ref() == Some(hash))
    }

    /// What decides whether two players can play together: the SHA-256 of
    /// the simulation version, the rules and the computer players' scripts,
    /// mods included, without any map's.
    pub fn content_hash(&mut self) -> soundrts_protocol::ContentHash {
        let map = self.map.take();
        if map.is_some() {
            self.reload();
        }
        let text = format!(
            "{}\n{}\n{}",
            soundrts_world::SIM_VERSION,
            self.rules_text(),
            self.ai_text()
        );
        if map.is_some() {
            self.map = map;
            self.reload();
        }
        let hash = soundrts_protocol::MapHash::of(text.as_bytes());
        #[expect(clippy::expect_used, reason = "a SHA-256 is 64 hex digits")]
        soundrts_protocol::ContentHash::new(hash.as_str()).expect("64 hex digits")
    }

    /// The campaigns: the folders of every package's `single` folder.
    pub fn campaigns(&self) -> Vec<Campaign> {
        let mut result = Vec::new();
        for package in &self.packages.0 {
            if let Some(single) = package.subpackage("single") {
                for n in single.dirnames() {
                    if let Some(p) = single.subpackage(&n) {
                        result.push(Campaign::new(p, &n));
                    }
                }
            }
        }
        result
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
    use crate::testkit::{game_root, upstream_tests_dir};

    /// A stack of upstream test packages; "res2" is the folder with its
    /// empty mod folders restored, in `res2_dir`.
    fn stack(paths: &[&str], res2_dir: &std::path::Path) -> Resources {
        let dir = upstream_tests_dir();
        Resources::new(
            paths
                .iter()
                .map(|p| {
                    if *p == "res2" {
                        res2_dir.join("res2")
                    } else {
                        dir.join(p)
                    }
                })
                .collect(),
            dir.join("no-downloads"),
            Some("en".into()),
            Arc::new(Parameters::default()),
            Log::silent(),
        )
    }

    fn layer_kinds(r: &Resources) -> Vec<(String, &'static str)> {
        r.layers()
            .iter()
            .map(|p| {
                (
                    p.name.clone(),
                    if p.folder().is_some() {
                        "FolderPackage"
                    } else {
                        "ZipPackage"
                    },
                )
            })
            .collect()
    }

    fn kinds(list: &[(&str, &'static str)]) -> Vec<(String, &'static str)> {
        list.iter().map(|(n, k)| (n.to_string(), *k)).collect()
    }

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

    #[test]
    fn test_best_language_match() {
        let available = [
            "en", "cs", "de", "es", "fr", "it", "pl", "pt-BR", "ru", "sk", "zh",
        ];
        for (lang, want) in [
            ("en", "en"),
            ("fr_ca", "fr"),
            ("fr", "fr"),
            ("pt_BR", "pt-BR"),
            ("pt_br", "pt-BR"),
            ("pt", "pt-BR"),
            ("de", "de"),
            ("pl", "pl"),
            ("es", "es"),
            ("xx", "en"),
        ] {
            assert_eq!(best_language_match(Some(lang), &available), want, "{lang}");
        }
        assert_eq!(best_language_match(None, &available), "en");
    }

    #[test]
    fn test_update() {
        let res2 = crate::testkit::upstream_res2_folder();
        let stack = |paths: &[&str]| stack(paths, res2.path());

        let r = stack(&["res", "res2"]);
        assert_eq!(layer_kinds(&r), kinds(&[("default", "FolderPackage")]));
        assert_eq!((r.mods.as_str(), r.soundpacks.as_str()), ("", ""));
        assert_eq!(
            r.text("rules", false).unwrap(),
            "def test\ncost 0 0\n\ndef peasant\ncost 0 0\n"
        );

        let r = stack(&["res.zip", "res2"]);
        assert_eq!(layer_kinds(&r), kinds(&[("default", "ZipPackage")]));
        assert_eq!((r.mods.as_str(), r.soundpacks.as_str()), ("", ""));
        assert_eq!(
            r.text("rules", false).unwrap(),
            "def test\ncost 0 0\n\ndef peasant\ncost 0 0\n"
        );

        let mut r = stack(&["res.zip"]);
        r.set_mods("mod1,mod2");
        r.set_soundpacks("sound1");
        assert_eq!(layer_kinds(&r), kinds(&[("default", "ZipPackage")]));
        assert_eq!((r.mods.as_str(), r.soundpacks.as_str()), ("", ""));

        let mut r = stack(&["res.zip", "res2"]);
        r.set_mods("mod1,mod2");
        r.set_soundpacks("sound1");
        assert_eq!(
            layer_kinds(&r),
            kinds(&[
                ("default", "ZipPackage"),
                ("mod1", "FolderPackage"),
                ("mod2", "FolderPackage"),
                ("sound1", "FolderPackage"),
            ])
        );
        assert_eq!(
            (r.mods.as_str(), r.soundpacks.as_str()),
            ("mod1,mod2", "sound1")
        );

        let mut r = stack(&["res.zip", "res2.zip"]);
        r.set_mods("mod,mod2");
        assert_eq!(
            layer_kinds(&r),
            kinds(&[("default", "ZipPackage"), ("mod2", "ZipPackage")])
        );
        assert_eq!((r.mods.as_str(), r.soundpacks.as_str()), ("mod2", ""));
    }

    #[test]
    fn test_maps_list() {
        let root = game_root();
        let mut r = Resources::new(
            vec![root.join("res"), root.clone()],
            root.join("no-downloads"),
            Some("en".into()),
            Arc::new(Parameters::default()),
            Log::silent(),
        );
        assert!(!r.official_multiplayer_maps().is_empty());
        let maps = r.multiplayer_maps();
        assert!(!maps.is_empty());
        // The recommended maps come first, then the official maps by title.
        let recommended = r.style().words("parameters", "recommended_maps");
        assert!(!recommended.is_empty());
        let names: Vec<&str> = maps.iter().map(|m| m.name.as_str()).collect();
        assert_eq!(
            &names[..recommended.len()],
            recommended.iter().map(String::as_str).collect::<Vec<_>>()
        );
        assert!(maps[recommended.len()..].iter().all(|m| m.official));
    }

    fn zipped(entries: &[(&str, Vec<u8>)]) -> Vec<u8> {
        use std::io::Write;
        let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        for (name, bytes) in entries {
            w.start_file(*name, zip::write::SimpleFileOptions::default())
                .unwrap();
            w.write_all(bytes).unwrap();
        }
        w.finish().unwrap().into_inner()
    }

    /// A map that brings a new unit type, and tries to change the game's
    /// keys, a prompt, the help, a unit's name and a sound.
    fn intrusive_map() -> Vec<u8> {
        let sound = std::fs::read(game_root().join("res/sounds/ambience/wood_1.ogg")).unwrap();
        zipped(&[
            ("map.txt", b"title golem\n".to_vec()),
            ("rules.txt", b"def golem\nclass soldier\n".to_vec()),
            ("ui/bindings.txt", b"CTRL SHIFT F12: quit\n".to_vec()),
            (
                "text/en.txt",
                b"what_are_your_orders = Press Alt F4 now.\ngolem = golem\n".to_vec(),
            ),
            ("ui/menu_help.txt", b"golem\n".to_vec()),
            (
                "ui/style.txt",
                b"def footman\ntitle golem\n\ndef golem\nis_a footman\ntitle golem\n".to_vec(),
            ),
            ("sounds/ambience/wood_2.ogg", sound.clone()),
            ("sounds/creature/golem.ogg", sound),
        ])
    }

    fn game_stack() -> Resources {
        let root = game_root();
        Resources::new(
            vec![root.join("res"), root.clone()],
            root.join("no-downloads"),
            Some("en".into()),
            Arc::new(Parameters::default()),
            Log::silent(),
        )
    }

    fn sound_layer(r: &Resources, key: &str) -> String {
        r.sounds().get_sound(key).unwrap().mod_name.clone()
    }

    #[test]
    fn a_downloaded_map_adds_to_the_interface_and_changes_nothing_in_it() {
        let mut r = game_stack();
        let bindings = |r: &Resources| {
            crate::bindings::game_bindings_text(r, std::path::Path::new("no bindings.txt"))
        };
        let base_bindings = bindings(&r);
        let base_help = r.text("ui/menu_help", false);
        let map = Map::from_server(intrusive_map(), "intrusive.zip").unwrap();
        assert!(map.downloaded);
        r.set_map(Some(&map));
        // The game gets the map's rules.
        assert!(r.rules_text().ends_with("def golem\nclass soldier\n"));
        // The interface gets what it lacked.
        assert_eq!(r.sounds().text("golem").as_deref(), Some("golem"));
        assert_eq!(r.style().words("golem", "title"), ["golem"]);
        assert_eq!(sound_layer(&r, "creature/golem"), "intrusive");
        // And nothing it had changes.
        assert_eq!(bindings(&r), base_bindings);
        assert_eq!(r.text("ui/menu_help", false), base_help);
        assert_eq!(
            r.sounds().text("what_are_your_orders").as_deref(),
            Some("What are your orders?")
        );
        assert_eq!(r.style().words("footman", "title"), ["footman"]);
        assert_eq!(sound_layer(&r, "ambience/wood_2"), "default");
        // Without the map, the interface is the game's again.
        r.set_map(None);
        assert_eq!(r.sounds().text("golem"), None);
        assert!(!r.style().has("golem", "title"));
    }

    #[test]
    fn a_map_the_player_installed_keeps_its_powers() {
        let mut r = game_stack();
        let map = Map::from_bytes(intrusive_map(), "intrusive.zip").unwrap();
        assert!(!map.downloaded);
        r.set_map(Some(&map));
        let bindings =
            crate::bindings::game_bindings_text(&r, std::path::Path::new("no bindings.txt"));
        assert!(bindings.contains("CTRL SHIFT F12: quit"));
        assert_eq!(r.text("ui/menu_help", false).as_deref(), Some("golem\n"));
        assert_eq!(
            r.sounds().text("what_are_your_orders").as_deref(),
            Some("Press Alt F4 now.")
        );
        assert_eq!(r.style().words("footman", "title"), ["golem"]);
        assert_eq!(sound_layer(&r, "ambience/wood_2"), "intrusive");
    }

    #[test]
    fn the_shipped_folder_map_works_as_before_when_local_and_adds_when_downloaded() {
        let bytes = std::fs::read(game_root().join("res/multi/jl4.zip")).unwrap();
        for downloaded in [false, true] {
            let mut r = game_stack();
            let map = if downloaded {
                Map::from_server(bytes.clone(), "jl4.zip").unwrap()
            } else {
                Map::from_bytes(bytes.clone(), "jl4.zip").unwrap()
            };
            r.set_map(Some(&map));
            let bindings =
                crate::bindings::game_bindings_text(&r, std::path::Path::new("no bindings.txt"));
            assert_eq!(bindings.contains("SHIFT z: resource_status 2"), !downloaded);
            assert_eq!(
                r.sounds().text("stone_golem").as_deref(),
                Some("stone golem")
            );
            assert_eq!(r.style().words("stone_golem", "title"), ["stone_golem"]);
            assert_eq!(r.style().words("parameters", "resource_2_title"), ["stone"]);
            assert_eq!(r.style().words("quarry", "title"), ["quarry"]);
            assert_eq!(sound_layer(&r, "creature/stone_golem"), "jl4");
        }
    }

    #[test]
    fn a_language_is_one_that_a_layer_has_a_language_file_for() {
        let mut r = game_stack();
        let languages = r.available_languages();
        assert!(languages.contains(&"fr".to_owned()), "{languages:?}");
        assert!(languages.contains(&"pt-BR".to_owned()), "{languages:?}");
        assert!(!languages.contains(&"xx".to_owned()));
        r.set_language("fr");
        // The words of the language file, over the English.
        assert_eq!(
            r.sounds().text("what_are_your_orders").as_deref(),
            Some("Que faites vous ?")
        );
        // The language's own style, over the game's.
        let english = game_stack();
        assert_eq!(english.style().words("a_deadly_fog", "shortcut"), ["f"]);
        assert_eq!(r.style().words("a_deadly_fog", "shortcut"), ["n"]);
        // A recording that only the German has.
        r.set_language("de");
        assert_eq!(sound_layer(&r, "voice/own/order_ok_1"), "default");
    }

    #[test]
    fn maps_in_the_downloaded_folder_are_downloaded_maps() {
        let root = game_root();
        let user = tempfile::tempdir().unwrap();
        let downloaded = user.path().join("downloaded");
        std::fs::create_dir_all(downloaded.join("multi")).unwrap();
        std::fs::write(
            downloaded.join("multi/intrusive_0123abcd.zip"),
            intrusive_map(),
        )
        .unwrap();
        let mine = user.path().join("mine");
        std::fs::create_dir_all(mine.join("multi")).unwrap();
        std::fs::write(mine.join("multi/mine.zip"), intrusive_map()).unwrap();
        // The folder of downloaded maps is also a package, as when the
        // settings folder is inside the game folder.
        let mut r = Resources::new(
            vec![root.join("res"), mine, downloaded.clone()],
            downloaded,
            Some("en".into()),
            Arc::new(Parameters::default()),
            Log::silent(),
        );
        let maps = r.multiplayer_maps();
        let found: Vec<(&str, bool)> = maps
            .iter()
            .filter(|m| !m.official)
            .map(|m| (m.name.as_str(), m.downloaded))
            .collect();
        assert_eq!(found, [("intrusive_0123abcd", true), ("mine", false)]);
    }
}
