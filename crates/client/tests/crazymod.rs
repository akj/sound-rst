//! CrazyMod, the mod that comes with the game in `data/mods/crazymod`,
//! loaded through the client's real resource stack: the game, then the mod,
//! in English, French and Spanish. These check the names the style gives
//! and the words they resolve to, not what a screen reader says.

use soundrts::config::Config;
use soundrts::testkit::{Harness, Script};
use soundrts_world::Rules;
use soundrts_world::msg::MsgPart;

/// The types without a title, as in the original: the base types of three
/// races, which no map or menu shows, and five effects, three of them the
/// game's own and untitled in its style as well.
const UNTITLED: [&str; 8] = [
    "instant_death",
    "quicksand",
    "exorcism",
    "robot",
    "undead_unit",
    "soul_absorption",
    "corpse_explosion",
    "vermin_building",
];

/// A client speaking `language` with CrazyMod on.
fn with_crazymod(language: &str) -> Harness {
    let config = Config {
        mods: "crazymod".to_owned(),
        ..Config::default()
    };
    let h = Harness::with_config(language, Script::new(), config);
    assert_eq!(h.client.res.mods, "crazymod");
    assert_eq!(h.client.res.language, language);
    h
}

/// The warnings and errors of the client's log.
fn complaints(h: &Harness) -> Vec<String> {
    h.log_text()
        .lines()
        .filter(|l| l.contains(" WARNING: ") || l.contains(" ERROR: "))
        .map(str::to_owned)
        .collect()
}

/// The words of `name`, a phrase: the phrase itself, not a sound.
fn words(h: &Harness, name: &str) -> Option<String> {
    h.client.sounds.text(name).filter(|t| !t.trim().is_empty())
}

/// The client's log has no complaint, and every type of the mod's rules has
/// a title that resolves to words in `h`'s language.
fn check(h: &Harness) {
    let language = h.client.res.language.clone();
    // The style is refused each name that is no phrase and no sound, and the
    // language files each line that is not `name = words`.
    assert_eq!(complaints(h), Vec::<String>::new(), "{language}");

    let rules = Rules::load(&[h.client.res.rules_text()]);
    assert_eq!(rules.warnings, Vec::<String>::new());
    let style = h.client.res.style();
    let (mut untitled, mut unresolved) = (Vec::new(), Vec::new());
    for name in rules
        .classnames()
        .into_iter()
        .map(str::to_owned)
        .chain(rules.factions())
    {
        let title = style.get(&name).title_msg();
        let resolved = title.iter().all(|part| match part {
            MsgPart::Name(n) => words(h, n).is_some(),
            _ => true,
        });
        if title.is_empty() {
            untitled.push(name);
        } else if !resolved {
            unresolved.push(format!("{name}: {title:?}"));
        }
    }
    assert_eq!(untitled, UNTITLED, "{language}");
    assert_eq!(unresolved, Vec::<String>::new(), "{language}");
}

#[test]
fn crazymod_loads_in_english_with_every_title_and_sound() {
    let h = with_crazymod("en");
    assert_eq!(h.text("traditional"), "traditional");
    assert_eq!(h.text("bomber_airship"), "bomber airship");
    check(&h);

    // Every recording of the mod decodes. The recordings are the same in
    // every language, so one language checks them.
    let sounds =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/mods/crazymod/sounds");
    let mut names = Vec::new();
    for folder in std::fs::read_dir(&sounds).unwrap().flatten() {
        for file in std::fs::read_dir(folder.path()).unwrap().flatten() {
            let file = file.file_name().to_string_lossy().into_owned();
            let stem = file.strip_suffix(".ogg").expect("only .ogg files");
            names.push(format!("{}/{stem}", folder.file_name().to_string_lossy()));
        }
    }
    assert!(names.len() > 500, "{} recordings", names.len());
    let missing: Vec<_> = names
        .iter()
        .filter(|n| h.client.sounds.get_sound(n).is_none())
        .collect();
    assert_eq!(missing, Vec::<&String>::new());
    assert_eq!(complaints(&h), Vec::<String>::new());
}

#[test]
fn crazymod_loads_in_french_with_the_authors_words() {
    let h = with_crazymod("fr");
    assert_eq!(h.text("traditional"), "traditionnel");
    assert_eq!(h.text("bomber_airship"), "bombardier dirigeable");
    assert_eq!(h.text("powder_magazine"), "poudrière");
    check(&h);
}

/// Spanish has a style of its own, which gives other shortcut keys.
#[test]
fn crazymod_loads_in_spanish_with_its_own_style() {
    let h = with_crazymod("es");
    assert_eq!(h.text("bomber_airship"), "avión bombardero");
    // The author's plural, where the game's own Spanish says "orco".
    assert_eq!(h.text("orc"), "orcos");
    // The Spanish style gives the lumbermill the shortcut c, not the game's l.
    let shortcut = h.client.res.style().get("lumbermill").shortcut.clone();
    assert_eq!(shortcut, Some(vec!["c".to_owned()]));
    check(&h);
}

#[test]
fn crazymod_loads_in_german_and_italian_with_the_authors_words() {
    let h = with_crazymod("de");
    // The author's capital, where the game's own German says "ork".
    assert_eq!(h.text("orc"), "Ork");
    assert_eq!(h.text("earth_elemental"), "Erdelement");
    check(&h);
    let h = with_crazymod("it");
    assert_eq!(h.text("bomber_airship"), "dirigibile bombardiere");
    check(&h);
}
