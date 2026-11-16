//! Port of upstream `tests/test_cache.py`: sounds, texts, style, rules and
//! computer players of the resource layers, with maps and campaigns.
//!
//! The tests read the original's small test packages once they are
//! imported. There, the number 0 is the phrase `nothing`, 1 is
//! `you_are_capitalized`, 5 is `what_are_your_orders`, and 9998 and 9999
//! are the sounds `silence/short` and `silence/long`. A number that names
//! nothing in the original, such as 6, stays a number.
//!
//! Rules and computer players belong to the simulation, which takes them
//! as text; these tests check the text the resource stack hands over.

use std::sync::Arc;

use soundrts::campaign::{Campaign, Chapter};
use soundrts::log::Log;
use soundrts::mapfile::Map;
use soundrts::package::Package;
use soundrts::parameters::Parameters;
use soundrts::resource::Resources;
use soundrts::testkit::{game_root, imported_tests_dir};
use soundrts_world::msg::MsgPart;

fn stack(path: std::path::PathBuf) -> Resources {
    Resources::new(
        vec![path],
        imported_tests_dir().join("no-downloads"),
        Some("en".into()),
        Arc::new(Parameters::default()),
        Log::silent(),
    )
}

fn test_stack() -> Resources {
    stack(imported_tests_dir().join("res"))
}

fn maps() -> Vec<Map> {
    ["single/map1", "single/map1.zip"]
        .iter()
        .map(|p| Map::from_path(&imported_tests_dir().join(p)).unwrap())
        .collect()
}

fn campaigns() -> Vec<Campaign> {
    ["single/campaign1", "single/campaign1.zip"]
        .iter()
        .map(|p| {
            Campaign::new(
                Package::from_path(&imported_tests_dir().join(p)).unwrap(),
                "campaign1",
            )
        })
        .collect()
}

/// The words of the attribute that the original's test styles call `test`.
/// The game has no attribute of that name, so the imported test packages
/// give `shortcut` instead, whose words name nothing (see
/// [`imported_tests_dir`]).
fn test_attribute(res: &Resources) -> Vec<String> {
    res.style()
        .get("peasant")
        .shortcut
        .clone()
        .unwrap_or_default()
}

fn text0(res: &Resources) -> Option<String> {
    res.sounds().text("nothing")
}

// Each layer's text is under a line that names the layer and the file.
const BASE_RULES: &str = "; file: default/rules.txt\ndef test\ncost 0 0\n\ndef peasant\ncost 0 0\n";
const BASE_AI: &str = "; file: default/ai.txt\ndef easy\nget 1 peasant\ngoto -1\n";

#[test]
fn test_get_sound_with_default_config() {
    let mut res = stack(game_root().join("res"));
    res.set_language("fr");
    assert!(res.sounds().get_sound("silence/long").is_some());
    assert!(res.sounds().get_sound("fdsgedtgf").is_none());
}

#[test]
fn test_get_sound() {
    let res = test_stack();
    let s = res.sounds();
    assert!(s.get_sound("ambience/path").is_none());
    assert!(s.get_sound("fdsgedtgf").is_none());
    assert!(s.get_sound("silence/short").is_some());
    assert!(s.get_sound("silence/long").is_none());
    assert!(s.get_sound("unnamed/sound_1028").is_none());
}

#[test]
fn test_get_sound_with_locale() {
    let mut res = test_stack();
    res.set_language("fr");
    let s = res.sounds();
    assert!(s.get_sound("ambience/path").is_none());
    assert!(s.get_sound("fdsgedtgf").is_none());
    assert!(s.get_sound("silence/short").is_some());
    assert!(s.get_sound("silence/long").is_some());
    // A sound can be stored in a subfolder.
    assert!(s.get_sound("unnamed/sound_1028").is_some());
}

#[test]
fn test_text() {
    let res = test_stack();
    let s = res.sounds();
    assert_eq!(s.text("nothing").as_deref(), Some("yes"));
    assert_eq!(s.text("you_are_capitalized").as_deref(), Some("no"));
    assert_eq!(s.text("in_front_of_you"), None);
}

#[test]
fn test_text_with_locale() {
    let mut res = test_stack();
    res.set_language("fr");
    let s = res.sounds();
    assert_eq!(s.text("nothing").as_deref(), Some("oui"));
    assert_eq!(s.text("you_are_capitalized").as_deref(), Some("no"));
    assert_eq!(s.text("in_front_of_you"), None);
}

#[test]
fn test_get_style() {
    let res = test_stack();
    assert_eq!(test_attribute(&res), ["nothing"]);
}

#[test]
fn test_get_style_with_locale() {
    let mut res = test_stack();
    res.set_language("fr");
    assert_eq!(test_attribute(&res), ["you_are_capitalized"]);
}

#[test]
fn test_get_rules_and_ai() {
    let res = test_stack();
    assert_eq!(res.rules_text(), BASE_RULES);
    assert_eq!(res.ai_text(), BASE_AI);
}

#[test]
fn test_folder_map() {
    let mut res = test_stack();
    for m in maps() {
        assert_eq!(m.name, "map1");
        assert_eq!(m.definition, "title 11 22\n");
        assert_ne!(text0(&res).as_deref(), Some("map1"));
        res.set_map(Some(&m));
        assert_eq!(
            res.rules_text(),
            format!(
                "{BASE_RULES}\n; file: map1/rules.txt\ndef peasant\ncost 6 0\n\ndef faction1\nclass faction\n\ndef faction2\nclass faction\n\ndef parameters\n"
            )
        );
        assert_eq!(
            res.ai_text(),
            format!("{BASE_AI}\n; file: map1/ai.txt\ndef easy\nget 6 peasant\ngoto -1\n")
        );
        assert_eq!(test_attribute(&res), ["6"]);
        assert_eq!(text0(&res).as_deref(), Some("map1"));
        res.set_map(None);
        assert_eq!(res.rules_text(), BASE_RULES);
        assert_eq!(res.ai_text(), BASE_AI);
        assert_eq!(test_attribute(&res), ["nothing"]);
        assert_eq!(text0(&res).as_deref(), Some("yes"));
    }
}

#[test]
fn test_campaign() {
    let mut res = test_stack();
    for c in campaigns() {
        assert_eq!(text0(&res).as_deref(), Some("yes"));
        res.set_campaign(Some(&c));
        assert_eq!(text0(&res).as_deref(), Some("campaign1"));
        res.set_campaign(None);
        assert_eq!(text0(&res).as_deref(), Some("yes"));
    }
}

#[test]
fn test_campaign_cutscene() {
    let mut res = test_stack();
    for c in campaigns() {
        res.set_campaign(Some(&c));
        let Chapter::CutScene {
            title, sequence, ..
        } = &c.chapters[0]
        else {
            panic!("chapter 0 is a cut scene");
        };
        assert_eq!(title, &vec![MsgPart::Name("prologue".into())]);
        assert_eq!(sequence, &vec![MsgPart::Count(7500)]);
        assert_eq!(text0(&res).as_deref(), Some("campaign1"));
        res.set_campaign(None);
    }
}

#[test]
fn test_campaign_map() {
    let mut res = test_stack();
    for campaign in campaigns() {
        res.set_campaign(Some(&campaign));
        let chapter = &campaign.chapters[2];
        let Chapter::Mission { map, .. } = chapter else {
            panic!("chapter 2 is a mission");
        };
        assert_eq!(map.name, "campaign1/2");
        assert_eq!(
            chapter.title(),
            vec![MsgPart::Count(11), MsgPart::Count(22)]
        );
        assert!(map.resources.is_none());
        res.set_map(Some(map));
        assert_eq!(
            res.rules_text(),
            format!("{BASE_RULES}\n; file: campaign1/rules.txt\ndef peasant\ncost 5 0\n")
        );
        assert_eq!(
            res.ai_text(),
            format!("{BASE_AI}\n; file: campaign1/ai.txt\ndef easy\nget 5 peasant\ngoto -1\n")
        );
        assert_eq!(test_attribute(&res), ["what_are_your_orders"]);
        assert_eq!(text0(&res).as_deref(), Some("campaign1"));
        res.set_map(None);
        res.set_campaign(None);
    }
}

#[test]
fn test_campaign_map_with_special_rules() {
    let mut res = test_stack();
    for campaign in campaigns() {
        res.set_campaign(Some(&campaign));
        let Chapter::Mission { map, .. } = &campaign.chapters[1] else {
            panic!("chapter 1 is a mission");
        };
        assert_eq!(map.name, "campaign1/1");
        res.set_map(Some(map));
        assert!(
            res.rules_text().ends_with("def peasant\ncost 7 0\n"),
            "{}",
            res.rules_text()
        );
        assert!(
            res.ai_text()
                .ends_with("def easy\nget 7 peasant\ngoto -1\n"),
            "{}",
            res.ai_text()
        );
        assert_eq!(test_attribute(&res), ["7"]);
        assert_eq!(text0(&res).as_deref(), Some("campaign1 map1"));
        res.set_map(None);
        res.set_campaign(None);
    }
}
