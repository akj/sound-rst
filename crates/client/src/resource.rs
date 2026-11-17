//! The resources: which file is read depends on the preferred language,
//! the active mods and soundpacks, the campaign and the map. The layers
//! and which of their files apply are in [`crate::layers`].
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::string_slice,
    clippy::arithmetic_side_effects
)]

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::campaign::Campaign;
use crate::layers::{GameFile, InterfaceFile, Layer, Layers};
use crate::log::Log;
use crate::mapfile::Map;
use crate::package::{Package, PackageStack, TextFile};
use crate::parameters::Parameters;
use crate::sound_cache::{SoundCache, Sounds, TEXT_FOLDER};
use crate::style::Style;

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
    /// The player's folder, whose language files come after every layer's.
    player: PathBuf,
    /// The player's folder of maps that came from a server.
    downloaded: PathBuf,
    pub packages: PackageStack,
    layers: Layers,
    /// The layers when the style and sounds were last read.
    previous_layers: Option<Layers>,
    /// The active mods, as a comma separated list of those that exist.
    pub mods: String,
    pub soundpacks: String,
    campaign: Option<Package>,
    /// The map's layer: [`Layer::Map`] or [`Layer::MapFromServer`].
    map: Option<Layer>,
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
    /// `player` is the player's folder: its `downloaded` folder holds the
    /// maps that came from a server, and its `text` folder the player's
    /// own language files.
    pub fn new(
        package_paths: Vec<PathBuf>,
        player: PathBuf,
        preferred_language: Option<String>,
        params: Arc<Parameters>,
        log: Log,
    ) -> Resources {
        let empty = SoundCache::new(&[], &[], params.clone(), log.clone());
        let mut r = Resources {
            packages: PackageStack::new(&package_paths),
            package_paths,
            downloaded: player.join("downloaded"),
            player,
            layers: Layers::new(Vec::new(), None),
            previous_layers: None,
            mods: String::new(),
            soundpacks: String::new(),
            campaign: None,
            map: None,
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

    /// The packages of the layers, in loading order: base, mods,
    /// soundpacks, campaign, map.
    pub fn layers(&self) -> Vec<&Package> {
        self.layers.packages().collect()
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

    /// The languages that have a language file: in a layer, among the
    /// port's own, or in the player's folder.
    pub fn available_languages(&self) -> Vec<String> {
        self.layers.languages()
    }

    fn add_layer(&self, m: Package, added: &mut Vec<String>, layers: &mut Vec<Layer>) {
        added.push(m.name.clone());
        for required in m.required_mods.clone() {
            if !added.contains(&required)
                && let Some(r) = self.packages.find_mod(&required)
            {
                self.add_layer(r, added, layers);
            }
        }
        layers.push(Layer::Mod(m));
    }

    /// Adds the existing mods of the list as layers; returns the list of
    /// those that exist.
    fn add_layers(&self, mods: &str, layers: &mut Vec<Layer>) -> String {
        let mut actual = Vec::new();
        let mut added = Vec::new();
        for name in mods.split(',').map(str::trim) {
            if name.is_empty() {
                continue;
            }
            if let Some(m) = self.packages.find_mod(name) {
                self.add_layer(m, &mut added, layers);
                actual.push(name.to_owned());
            }
        }
        actual.join(",")
    }

    fn reload(&mut self) {
        let mut layers: Vec<Layer> = self
            .packages
            .0
            .iter()
            .take(1)
            .cloned()
            .map(Layer::Game)
            .collect();
        self.mods = self.add_layers(&self.mods.clone(), &mut layers);
        self.soundpacks = self.add_layers(&self.soundpacks.clone(), &mut layers);
        layers.extend(self.campaign.clone().map(Layer::Campaign));
        layers.extend(self.map.clone());
        let player = Package::from_path(&self.player).ok().map(|p| {
            let name = self.player.display().to_string();
            p.with_name(&name)
        });
        self.layers = Layers::new(layers, player);
        if self.previous_layers.as_ref() != Some(&self.layers) {
            self.language = best_language_match(
                self.preferred_language.as_deref(),
                &self.available_languages(),
            );
            self.load_style_and_sounds();
            if let Some(f) = self.on_change.as_mut() {
                f(&self.mods, &self.soundpacks);
            }
            self.previous_layers = Some(self.layers.clone());
        }
    }

    /// Reads the style and the sound cache again, for the current language.
    pub fn load_style_and_sounds(&mut self) {
        let language = self.language.as_str();
        let languages = if language == "en" {
            vec!["en"]
        } else {
            vec!["en", language]
        };
        let phrases = self.layers.phrases(&languages);
        let sounds = self.layers.sounds(language);
        let style = self.layers.style(language);
        // The phrases and the sounds come first: the style is refused each
        // word that names none of them. A layer that only adds names adds
        // them after, so the style below it names only its own.
        let mut cache = SoundCache::new(
            &phrases.replace,
            &sounds.replace,
            self.params.clone(),
            self.log.clone(),
        );
        self.style.load(&style.replace, &|name| cache.has(name));
        let ignored_sounds = cache.add_downloaded(&phrases.add_only, &sounds.add_only);
        let ignored_style = self
            .style
            .add_downloaded(&style.add_only, &|name| cache.has(name));
        if ignored_style > 0 || ignored_sounds > 0 {
            let map = self.map.as_ref().and_then(Layer::package);
            let map = map.map_or("", |m| m.name.as_str());
            self.log.warning(format!(
                "downloaded map {map}: ignored {ignored_style} style attributes and {ignored_sounds} phrases or sounds the game already has"
            ));
        }
        for (layer, file) in self.layers.refused(language) {
            self.log
                .warning(format!("{}: {} ignored", layer.describe(), file.path));
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

    /// Adds the map's resources as the top layer, or removes them. What a
    /// map that came from a server gives is in [`Layer::power`].
    pub fn set_map(&mut self, map: Option<&Map>) {
        self.map = map.and_then(|m| {
            let package = m.resources.clone()?;
            Some(if m.downloaded {
                Layer::MapFromServer(package)
            } else {
                Layer::Map(package)
            })
        });
        self.reload();
        if let Some(map) = map {
            self.check_map(map);
        }
    }

    /// Reports each word of a map that should name a phrase or a sound and
    /// names none, with the map and the line. Such a word says nothing in
    /// the game.
    fn check_map(&self, map: &Map) {
        let mut reports = 0_usize;
        for (line, word) in soundrts_import::words_a_map_says(&map.definition) {
            let what = match soundrts_world::msg::MsgPart::named(&word) {
                Some(soundrts_world::msg::MsgPart::Name(n)) if !self.sounds.has(&n) => {
                    "names no phrase and no sound"
                }
                Some(_) => continue,
                None => "is not a name or a count",
            };
            reports = reports.saturating_add(1);
            if reports > 20 {
                break;
            }
            self.log
                .error(format!("map {}, line {line}: {word} {what}", map.name));
        }
    }

    /// Adds the campaign's resources as a layer, or removes them.
    pub fn set_campaign(&mut self, campaign: Option<&Campaign>) {
        self.campaign = campaign.map(|c| c.resources.clone());
        self.reload();
    }

    /// The interface's files of one language, in priority order: the
    /// port's own, each layer's, then the player's. A map from a server
    /// adds phrases but translates none of the game's, so its files are
    /// left out.
    pub fn files_of_language(&self, language: &str) -> Vec<TextFile> {
        self.layers.phrases(&[language]).replace
    }

    /// The player's language file of `language`, which may not exist yet.
    pub fn player_language_file(&self, language: &str) -> PathBuf {
        self.player
            .join(TEXT_FOLDER)
            .join(format!("{language}.txt"))
    }

    /// The interface's files `file`, in priority order, each with its
    /// layer in its path for a message.
    pub fn interface_files(&self, file: InterfaceFile) -> Vec<TextFile> {
        self.layers.interface(file, &self.language)
    }

    /// The files of the key bindings, in the order they are read, the
    /// player's file at `custom` last (see [`Layers::bindings`]).
    pub fn bindings_files(&self, custom: &Path) -> Vec<TextFile> {
        self.layers.bindings(&self.language, custom)
    }

    /// The content of the interface's `<name>.txt` with the highest
    /// priority.
    #[cfg(test)]
    pub fn text(&self, name: &'static str, localized: bool) -> Option<String> {
        let file = InterfaceFile::named(name, localized);
        self.interface_files(file).pop().map(|f| f.text)
    }

    /// The rules for a new world: every `rules.txt` of the layers.
    pub fn rules_text(&self) -> String {
        self.layers.game_text(GameFile::Rules)
    }

    /// The computer players for a new world: every `ai.txt` of the layers.
    pub fn ai_text(&self) -> String {
        self.layers.game_text(GameFile::Ai)
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
        let recommended = self.style.get("parameters").recommended_maps.clone();
        let recommended = recommended.unwrap_or_default();
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
    pub fn content_hash(&self) -> soundrts_protocol::ContentHash {
        let layers = self.layers.without_map();
        let text = format!(
            "{}\n{}\n{}",
            soundrts_world::SIM_VERSION,
            layers.game_text(GameFile::Rules),
            layers.game_text(GameFile::Ai)
        );
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
            dir.join("no-player"),
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
            r.rules_text(),
            "; file: default/rules.txt\ndef test\ncost 0 0\n\ndef peasant\ncost 0 0\n"
        );

        let r = stack(&["res.zip", "res2"]);
        assert_eq!(layer_kinds(&r), kinds(&[("default", "ZipPackage")]));
        assert_eq!((r.mods.as_str(), r.soundpacks.as_str()), ("", ""));
        assert_eq!(
            r.rules_text(),
            "; file: default/rules.txt\ndef test\ncost 0 0\n\ndef peasant\ncost 0 0\n"
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
            root.join("no-player"),
            Some("en".into()),
            Arc::new(Parameters::default()),
            Log::silent(),
        );
        assert!(!r.official_multiplayer_maps().is_empty());
        let maps = r.multiplayer_maps();
        assert!(!maps.is_empty());
        // The recommended maps come first, then the official maps by title.
        let recommended = r.style().get("parameters").recommended_maps.clone();
        let recommended = recommended.unwrap_or_default();
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
            root.join("no-player"),
            Some("en".into()),
            Arc::new(Parameters::default()),
            Log::silent(),
        )
    }

    fn name(s: &str) -> soundrts_world::msg::MsgPart {
        soundrts_world::msg::MsgPart::Name(s.to_owned())
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
        // The game gets the map's rules, under the same line as the host
        // of the game, whose file of the map has another name.
        assert!(
            r.rules_text()
                .ends_with("\n; file: map/rules.txt\ndef golem\nclass soldier\n")
        );
        let mut host = game_stack();
        host.set_map(Some(
            &Map::from_bytes(intrusive_map(), "my map.zip").unwrap(),
        ));
        assert_eq!(host.rules_text(), r.rules_text());
        assert_eq!(host.ai_text(), r.ai_text());
        // The interface gets what it lacked.
        assert_eq!(r.sounds().text("golem").as_deref(), Some("golem"));
        assert_eq!(r.style().get("golem").title_msg(), [name("golem")]);
        assert_eq!(sound_layer(&r, "creature/golem"), "intrusive");
        // And nothing it had changes.
        assert_eq!(bindings(&r), base_bindings);
        assert_eq!(r.text("ui/menu_help", false), base_help);
        assert_eq!(
            r.sounds().text("what_are_your_orders").as_deref(),
            Some("What are your orders?")
        );
        assert_eq!(r.style().get("footman").title_msg(), [name("footman")]);
        assert_eq!(sound_layer(&r, "ambience/wood_2"), "default");
        // Without the map, the interface is the game's again.
        r.set_map(None);
        assert_eq!(r.sounds().text("golem"), None);
        assert!(!r.style().has("golem"));
    }

    #[test]
    fn a_map_the_player_installed_keeps_its_powers() {
        let mut r = game_stack();
        let map = Map::from_bytes(intrusive_map(), "intrusive.zip").unwrap();
        assert!(!map.downloaded);
        r.set_map(Some(&map));
        let bindings =
            crate::bindings::game_bindings_text(&r, std::path::Path::new("no bindings.txt"));
        assert!(
            bindings
                .iter()
                .any(|f| f.text.contains("CTRL SHIFT F12: quit"))
        );
        assert_eq!(r.text("ui/menu_help", false).as_deref(), Some("golem\n"));
        assert_eq!(
            r.sounds().text("what_are_your_orders").as_deref(),
            Some("Press Alt F4 now.")
        );
        assert_eq!(r.style().get("footman").title_msg(), [name("golem")]);
        assert_eq!(sound_layer(&r, "ambience/wood_2"), "intrusive");
    }

    #[test]
    fn a_refused_line_of_a_mods_bindings_is_logged_with_the_mods_file_and_its_line() {
        let root = game_root();
        let user = tempfile::tempdir().unwrap();
        let mods = user.path().join("mine");
        let keys = mods.join("mods/keys/ui");
        std::fs::create_dir_all(&keys).unwrap();
        std::fs::write(
            keys.join("bindings.txt"),
            "; my keys\n\nF11: say_time\nF10: no_such_command\n",
        )
        .unwrap();
        let log_path = user.path().join("client.log");
        let log = Log::file_only(&log_path);
        let mut r = Resources::new(
            vec![root.join("res"), mods],
            root.join("no-player"),
            Some("en".into()),
            Arc::new(Parameters::default()),
            log.clone(),
        );
        r.set_mods("keys");
        let files = crate::bindings::game_bindings_text(&r, std::path::Path::new("no file"));
        let mut b = crate::bindings::Bindings::new();
        b.load(&files, &crate::game::command::GameCommand::parse, &log);
        let logged = std::fs::read_to_string(&log_path).unwrap();
        let refused = "keys/ui/bindings.txt line 4 is ignored \
                       ('no_such_command' is not a command): F10: no_such_command";
        assert!(logged.contains(refused), "{logged}");
    }

    #[test]
    fn the_content_hash_leaves_out_the_map_and_counts_the_mods() {
        let mut r = game_stack();
        let plain = r.content_hash();
        for map in [
            Map::from_bytes(intrusive_map(), "intrusive.zip").unwrap(),
            Map::from_server(intrusive_map(), "intrusive.zip").unwrap(),
        ] {
            r.set_map(Some(&map));
            assert!(r.rules_text().contains("def golem"));
            assert_eq!(r.content_hash(), plain);
        }
        r.set_mods("orc");
        assert_ne!(r.content_hash(), plain);
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
            let resource = |f: &TextFile| f.text.contains("SHIFT z: resource_status 2");
            assert_eq!(bindings.iter().any(resource), !downloaded);
            assert_eq!(
                r.sounds().text("stone_golem").as_deref(),
                Some("stone golem")
            );
            let style = r.style();
            assert_eq!(style.get("stone_golem").title_msg(), [name("stone_golem")]);
            let stone = style.get("parameters").resource_title_msg(2);
            assert_eq!(stone, [name("stone")]);
            assert_eq!(style.get("quarry").title_msg(), [name("quarry")]);
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
        let shortcut = |r: &Resources| r.style().get("a_deadly_fog").shortcut.clone();
        assert_eq!(shortcut(&english), Some(vec!["f".to_owned()]));
        assert_eq!(shortcut(&r), Some(vec!["n".to_owned()]));
        // A recording that only the German has.
        r.set_language("de");
        assert_eq!(sound_layer(&r, "voice/own/order_ok_1"), "default");
    }

    #[test]
    fn the_game_its_mods_and_its_campaigns_load_without_a_refused_line_in_any_language() {
        let root = game_root();
        let log = Log::silent();
        log.keep_errors();
        let mut r = Resources::new(
            vec![root.join("res"), root.clone()],
            root.join("no-player"),
            Some("en".into()),
            Arc::new(Parameters::default()),
            log.clone(),
        );
        let languages = r.available_languages();
        assert!(languages.len() >= 12, "{languages:?}");
        let mods = r.available_mods();
        assert!(mods.contains(&"orc".to_owned()), "{mods:?}");
        for language in &languages {
            r.set_language(language);
            for mods in [String::new(), mods.join(",")] {
                r.set_mods(&mods);
                r.set_soundpacks("soundpack");
                for campaign in r.campaigns() {
                    r.set_campaign(Some(&campaign));
                    assert!(r.style().has("peasant"));
                    for chapter in &campaign.chapters {
                        if let crate::campaign::Chapter::Mission { map, .. } = chapter {
                            r.set_map(Some(map));
                        }
                    }
                    r.set_map(None);
                }
                r.set_campaign(None);
                r.set_soundpacks("");
            }
            r.set_mods("");
            assert_eq!(log.take_errors(), Vec::<String>::new(), "{language}");
        }
        // Every map of the game names only what the game has.
        r.set_language("en");
        let maps = r.multiplayer_maps();
        assert_eq!(maps.len(), 89 + 4);
        for map in &maps {
            r.set_map(Some(map));
        }
        assert_eq!(log.take_errors(), Vec::<String>::new());
    }

    #[test]
    fn a_word_of_a_map_that_names_nothing_is_reported_with_its_line() {
        let log = Log::silent();
        log.keep_errors();
        let root = game_root();
        let mut r = Resources::new(
            vec![root.join("res")],
            root.join("no-player"),
            Some("en".into()),
            Arc::new(Parameters::default()),
            log.clone(),
        );
        let text = "title numeral_1 no_such_title\n; a comment\nobjective neutralized Enemy\n\
                    trigger player1 (timer 0) (add_objective 1 build one_f farmm)\n\
                    trigger player1 (has farm) (cut_scene nicely_done 3)\n";
        let map = Map::from_bytes(text.as_bytes().to_vec(), "typo.txt").unwrap();
        r.set_map(Some(&map));
        assert_eq!(
            log.take_errors(),
            [
                "map typo, line 1: no_such_title names no phrase and no sound",
                "map typo, line 3: Enemy is not a name or a count",
                "map typo, line 4: farmm names no phrase and no sound",
                "map typo, line 5: nicely_done names no phrase and no sound",
            ]
        );
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
            vec![root.join("res"), mine, downloaded],
            user.path().to_path_buf(),
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
