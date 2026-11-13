//! The resource stack: which file is read depends on the preferred
//! language, the active mods and soundpacks, the campaign and the map.
//! Sounds are replaced by later layers; some text files are merged.

use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use crate::campaign::Campaign;
use crate::log::Log;
use crate::mapfile::Map;
use crate::package::{Package, PackageStack};
use crate::parameters::Parameters;
use crate::sound_cache::{SoundCache, Sounds, TXT_FILE};
use crate::style::Style;

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// The path for this language: `ui` becomes `ui-fr`, when `ui` is a whole
/// word of the path.
pub fn localized_path(path: &str, lang: &str) -> String {
    let mut out = String::with_capacity(path.len() + lang.len() + 1);
    let mut rest = path;
    let mut previous: Option<char> = None;
    while let Some(i) = rest.find("ui") {
        let before = rest[..i].chars().next_back().or(previous);
        let after = rest[i + 2..].chars().next();
        out.push_str(&rest[..i]);
        if !before.is_some_and(is_word) && !after.is_some_and(is_word) {
            out.push_str("ui-");
            out.push_str(lang);
        } else {
            out.push_str("ui");
        }
        previous = Some('i');
        rest = &rest[i + 2..];
    }
    out.push_str(rest);
    out
}

/// The path, then its translation if a language is given.
pub fn localized_paths(path: &str, language: Option<&str>) -> Vec<String> {
    let mut result = vec![path.to_owned()];
    if let Some(lang) = language.filter(|l| !l.is_empty()) {
        result.push(localized_path(path, lang));
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

/// The language the player asked for in `cfg/language.txt`, else the
/// system's.
pub fn preferred_language(language_file: &std::path::Path, log: &Log) -> Option<String> {
    let Ok(bytes) = std::fs::read(language_file) else {
        log.warning("couldn't read cfg/language.txt");
        return Some("en".to_owned());
    };
    let cfg = String::from_utf8_lossy(&bytes).trim().to_owned();
    if !cfg.is_empty() {
        return Some(cfg);
    }
    let system = sys_locale::get_locale();
    if system.is_none() {
        log.warning(
            "Couldn't get the system language. To use another language, edit \
             'cfg/language.txt' and write 'pl' for example.",
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
    previous_layers: Option<Vec<Package>>,
    /// The active mods, as a comma separated list of those that exist.
    pub mods: String,
    pub soundpacks: String,
    campaign: Option<Package>,
    map: Option<Package>,
    /// The language of the texts and sounds.
    pub language: String,
    preferred_language: Option<String>,
    style: Style,
    sounds: Sounds,
    params: Rc<Parameters>,
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
        params: Rc<Parameters>,
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

    pub fn params(&self) -> Rc<Parameters> {
        self.params.clone()
    }

    /// Replaces the parameters used for the sounds loaded from now on.
    pub fn set_params(&mut self, params: Rc<Parameters>) {
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

    fn available_languages(&self) -> Vec<String> {
        let mut result = vec!["en".to_owned()];
        for package in &self.layers {
            for name in package.dirnames() {
                if let Some(lang) = name.strip_prefix("ui-")
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
        let style_texts = self.texts("ui/style", true, None);
        let refs: Vec<&str> = style_texts.iter().map(String::as_str).collect();
        self.style.load(&refs);
        let texts = self.texts(TXT_FILE, true, None);
        let dirs: Vec<(Package, String)> = self.paths("ui", None, true);
        self.sounds.replace(SoundCache::new(
            &texts,
            &dirs,
            self.params.clone(),
            self.log.clone(),
        ));
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
    pub fn set_map(&mut self, map: Option<&Map>) {
        self.map = map.and_then(|m| m.resources.clone());
        self.reload();
    }

    /// Adds the campaign's resources as a layer, or removes them.
    pub fn set_campaign(&mut self, campaign: Option<&Campaign>) {
        self.campaign = campaign.map(|c| c.resources.clone());
        self.reload();
    }

    /// The (package, path) pairs to look `path` up in, in priority order.
    pub fn paths(
        &self,
        path: &str,
        root: Option<&Package>,
        localize: bool,
    ) -> Vec<(Package, String)> {
        let lang = localize.then_some(self.language.as_str());
        let roots: Vec<&Package> = match root {
            Some(r) => vec![r],
            None => self.layers.iter().collect(),
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

    /// The contents of every `<name>.txt` of the layers, in priority order.
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

    /// The rules for a new world: every `rules.txt` of the layers.
    pub fn rules_text(&self) -> String {
        self.text_appended("rules", false)
    }

    /// The computer players for a new world: every `ai.txt` of the layers.
    pub fn ai_text(&self) -> String {
        self.text_appended("ai", false)
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

    fn custom_multiplayer_maps(&self) -> Vec<Map> {
        let mut maps = Vec::new();
        let downloaded = Package::from_path(&self.downloaded).ok();
        for package in self.packages.0.iter().skip(1).chain(downloaded.iter()) {
            let Some(multi) = package.subpackage("multi") else {
                continue;
            };
            for n in multi.filenames().into_iter().chain(multi.dirnames()) {
                match multi.open_binary(&n).and_then(|b| Map::from_bytes(b, &n)) {
                    Ok(mut m) => {
                        // The heal sound tells the player this map is not official.
                        m.title.insert(0, 1097.into());
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
        // Custom maps come after official maps: a sound number sorts after
        // any letter.
        let key = |m: &Arc<Map>| -> Vec<String> {
            m.title
                .iter()
                .map(|p| match p {
                    soundrts_world::msg::MsgPart::Text(s) => s.to_lowercase(),
                    soundrts_world::msg::MsgPart::Num(_) => "{".to_owned(),
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
mod tests {
    use super::*;
    use crate::testkit::{upstream_root, upstream_tests_dir};

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
            Rc::new(Parameters::default()),
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
    fn test_localize_path() {
        assert_eq!(localized_path("/ui", "fr"), "/ui-fr");
        assert_eq!(localized_path("/ui/", "fr"), "/ui-fr/");
        assert_eq!(localized_path("/uii", "fr"), "/uii");
        assert_eq!(localized_path("/oui", "fr"), "/oui");
        assert_eq!(localized_path("/ui/i", "fr"), "/ui-fr/i");
        assert_eq!(localized_path("/oui/i", "fr"), "/oui/i");
        assert_eq!(
            localized_path("ui/style.txt", "pt-BR"),
            "ui-pt-BR/style.txt"
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
        assert_eq!(localized_path("ui", "en"), "ui-en");
        assert_eq!(localized_paths("ui", Some("en")), ["ui", "ui-en"]);

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
        let root = upstream_root();
        let mut r = Resources::new(
            vec![root.join("res"), root.clone()],
            root.join("no-downloads"),
            Some("en".into()),
            Rc::new(Parameters::default()),
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
}
