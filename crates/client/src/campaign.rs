//! Campaigns (`campaign.py`): the packages the resource stack discovers
//! (title, mods, chapters), the chapter menu, the sequence of missions and
//! cut scenes, and the bookmark of the last chapter unlocked.

use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;

use soundrts_world::Skill;
use soundrts_world::msg::{Msg, MsgPart, Phrase, comma, msg, nb2msg};

use crate::client::Client;
use crate::mapfile::Map;
use crate::menu::{Action, Exit, Flow, Menu};
use crate::package::Package;
use crate::saves::Saved;
use crate::session::{Session, SessionKind};

/// A numbered step of a campaign.
#[derive(Clone, Debug)]
pub enum Chapter {
    /// A game on a map.
    Mission { number: usize, map: Map },
    /// Sounds and texts said in order, between missions.
    CutScene {
        number: usize,
        title: Msg,
        sequence: Msg,
    },
}

impl Chapter {
    pub fn number(&self) -> usize {
        match self {
            Chapter::Mission { number, .. } | Chapter::CutScene { number, .. } => *number,
        }
    }

    /// What the menus say for this chapter.
    pub fn title(&self) -> Msg {
        match self {
            Chapter::Mission { map, .. } => map.title[1..].to_vec(),
            Chapter::CutScene { title, .. } => title.clone(),
        }
    }
}

/// A campaign: a folder of numbered chapters with its own resources.
#[derive(Clone, Debug)]
pub struct Campaign {
    pub name: String,
    pub resources: Package,
    pub title: Msg,
    /// The mods the campaign switches to; `None` keeps the player's mods.
    pub mods: Option<String>,
    pub chapters: Vec<Chapter>,
}

/// The words after `keyword` on a line made only of `allowed` characters.
fn header_line<'a>(
    text: &'a str,
    keyword: &str,
    allowed: impl Fn(char) -> bool,
) -> Option<&'a str> {
    text.split('\n').find_map(|line| {
        let rest = line.strip_prefix(keyword)?;
        let value = rest.trim_start_matches([' ', '\t']);
        (value.len() < rest.len() && !value.is_empty() && value.chars().all(&allowed))
            .then_some(value)
    })
}

/// Whether a line can hold `c` and name phrases, sounds and counts.
pub fn is_name_char(c: char) -> bool {
    c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '_' | '/' | ' ')
}

impl Campaign {
    /// Reads the campaign in `package`, named after the last part of `path`.
    pub fn new(package: Package, path: &str) -> Campaign {
        let base = path.rsplit(['/', '\\']).next().unwrap_or(path);
        let name = base
            .rsplit_once('.')
            .map_or(base, |(stem, _)| stem)
            .to_owned();
        let resources = package.with_name(&name);
        let text = if resources.isfile("campaign.txt") {
            resources.read_text("campaign.txt").unwrap_or_default()
        } else {
            String::new()
        };
        let words = |c: char| c.is_ascii_alphanumeric() || c == ' ';
        let title = match header_line(&text, "title", is_name_char) {
            Some(t) => t.split(' ').filter_map(MsgPart::named).collect(),
            None => vec![MsgPart::literal(name.as_str())],
        };
        let mut mods = match header_line(&text, "mods", words) {
            Some(m) => Some(m.to_owned()),
            None if text.split('\n').any(|l| l == "mods") => Some(String::new()),
            None => None,
        };
        if resources.isfile("mods.txt") {
            mods = resources.read_text("mods.txt");
        }
        let mut campaign = Campaign {
            name,
            resources,
            title,
            mods,
            chapters: Vec::new(),
        };
        campaign.chapters = campaign.read_chapters();
        campaign
    }

    fn read_chapters(&self) -> Vec<Chapter> {
        let mut chapters = Vec::new();
        for number in 0.. {
            let mut filename = format!("{number}.txt");
            if !self.resources.isfile(&filename) {
                filename = format!("{number}.zip");
                if !self.resources.isfile(&filename) {
                    break;
                }
            }
            let cutscene = filename
                .ends_with(".txt")
                .then(|| self.resources.read_text(&filename))
                .flatten();
            match cutscene {
                Some(text) if text.split_inclusive('\n').next() == Some("cut_scene_chapter\n") => {
                    let named = |keyword| -> Option<Msg> {
                        let line = header_line(&text, keyword, is_name_char)?;
                        Some(line.split(' ').filter_map(MsgPart::named).collect())
                    };
                    let title = named("title").unwrap_or_else(|| nb2msg(number));
                    let sequence = named("sequence").unwrap_or_default();
                    chapters.push(Chapter::CutScene {
                        number: number as usize,
                        title,
                        sequence,
                    });
                }
                _ => {
                    let Ok(bytes) = self.resources.open_binary(&filename) else {
                        break;
                    };
                    let Ok(mut map) = Map::from_bytes(bytes, &filename) else {
                        break;
                    };
                    map.name = format!("{}/{number}", self.name);
                    chapters.push(Chapter::Mission {
                        number: number as usize,
                        map,
                    });
                }
            }
        }
        chapters
    }

    /// The name in `campaigns.ini`: the campaign's name with only letters,
    /// digits and underscores.
    fn ini_id(&self) -> String {
        self.name
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
            .collect()
    }

    /// The value of `key` in the campaign's section of `ini`.
    fn ini_value(&self, ini: &Path, key: &str) -> Option<String> {
        let text = std::fs::read_to_string(ini).unwrap_or_default();
        read_ini(&text)
            .into_iter()
            .find(|(s, _)| *s == self.ini_id())
            .and_then(|(_, keys)| keys.into_iter().find(|(k, _)| k == key))
            .map(|(_, v)| v)
    }

    /// Sets `key` to `value` in the campaign's section of `ini`, keeping
    /// the rest of the file.
    fn set_ini_value(&self, ini: &Path, key: &str, value: String) -> std::io::Result<()> {
        let text = std::fs::read_to_string(ini).unwrap_or_default();
        let mut sections = read_ini(&text);
        let id = self.ini_id();
        let i = match sections.iter().position(|(s, _)| *s == id) {
            Some(i) => i,
            None => {
                sections.push((id, Vec::new()));
                sections.len() - 1
            }
        };
        let keys = &mut sections[i].1;
        match keys.iter_mut().find(|(k, _)| k == key) {
            Some(kv) => kv.1 = value,
            None => keys.push((key.into(), value)),
        }
        std::fs::write(ini, write_ini(&sections))
    }

    /// The number of the last chapter unlocked (0 at first).
    pub fn bookmark(&self, ini: &Path) -> usize {
        self.ini_value(ini, "chapter")
            .and_then(|v| v.parse().ok())
            .unwrap_or(0)
    }

    fn set_bookmark(&self, ini: &Path, number: usize) -> std::io::Result<()> {
        self.set_ini_value(ini, "chapter", number.to_string())
    }

    /// The level of the computers of its missions: the normal level unless
    /// the player chose another.
    pub fn skill(&self, ini: &Path) -> Skill {
        self.ini_value(ini, "level")
            .and_then(|v| v.parse().ok())
            .and_then(Skill::from_number)
            .unwrap_or(Skill::NORMAL)
    }

    /// Chooses the level of the computers of its missions, from the next
    /// mission on.
    pub fn set_skill(&self, cx: &Client, skill: Skill) {
        let ini = cx.paths.campaigns_config();
        if let Err(e) = self.set_ini_value(&ini, "level", skill.number().to_string()) {
            cx.log
                .warning(format!("cannot write {}: {e}", ini.display()));
        }
    }

    /// The chapters up to the bookmark.
    pub fn available_chapters(&self, ini: &Path) -> &[Chapter] {
        let n = (self.bookmark(ini) + 1).min(self.chapters.len());
        &self.chapters[..n]
    }

    /// Unlocks the chapter after `number`, if `number` is the last one
    /// unlocked.
    pub fn unlock_next(&self, cx: &Client, number: usize) {
        let ini = cx.paths.campaigns_config();
        if self.bookmark(&ini) == number
            && number + 1 < self.chapters.len()
            && let Err(e) = self.set_bookmark(&ini, number + 1)
        {
            cx.log
                .warning(format!("cannot write {}: {e}", ini.display()));
        }
    }
}

/// The sections of an INI file as Python's `configparser` reads them, in
/// file order, with lower case keys.
fn read_ini(text: &str) -> Vec<(String, Vec<(String, String)>)> {
    let mut sections: Vec<(String, Vec<(String, String)>)> = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            sections.push((name.to_owned(), Vec::new()));
        } else if let Some((k, v)) = line.split_once(['=', ':'])
            && let Some((_, keys)) = sections.last_mut()
        {
            keys.push((k.trim().to_lowercase(), v.trim().to_owned()));
        }
    }
    sections
}

/// The sections as `configparser` writes them.
fn write_ini(sections: &[(String, Vec<(String, String)>)]) -> String {
    let mut out = String::new();
    for (name, keys) in sections {
        out.push_str(&format!("[{name}]\n"));
        for (k, v) in keys {
            out.push_str(&format!("{k} = {v}\n"));
        }
        out.push('\n');
    }
    out
}

/// Plays a campaign: switches to its mods (they stay after), adds its
/// resources, and runs its chapter menu once.
pub fn run_campaign(cx: &mut Client, campaign: Rc<Campaign>) -> Result<(), Exit> {
    if let Some(mods) = &campaign.mods {
        cx.res.set_mods(mods);
    }
    cx.res.set_campaign(Some(&campaign));
    let result = (|| {
        let mut menu = campaign_menu(cx, &campaign);
        loop {
            menu.run(cx)?;
            if menu.ended() {
                return Ok(());
            }
            // The level changed: its choices say the new one.
            let new = campaign_menu(cx, &campaign);
            menu.update_menu(cx, new);
        }
    })();
    cx.res.set_campaign(None);
    result
}

/// Plays a chapter, then leaves the campaign's menu.
fn chapter_action(campaign: &Rc<Campaign>, number: usize) -> Action {
    let campaign = campaign.clone();
    Action::run(move |cx| run_chapter(cx, &campaign, number).map(|()| Flow::CloseMenu))
}

/// Plays a chapter from the menu after a mission, which is run once.
fn next_chapter_action(campaign: &Rc<Campaign>, number: usize) -> Action {
    let campaign = campaign.clone();
    Action::run(move |cx| run_chapter(cx, &campaign, number).map(|()| Flow::Stay))
}

/// The choices of the level of the campaign's computers, one for each level
/// it does not have: "set the computers, level 3, to level 2". The menu
/// stays open.
fn level_choices(menu: &mut Menu, campaign: &Rc<Campaign>, current: Skill) {
    for skill in Skill::ALL.into_iter().filter(|s| *s != current) {
        let label = [
            msg(Phrase::Set),
            msg(Phrase::TheComputers),
            comma(),
            msg(Phrase::Level),
            nb2msg(current.number().into()),
            comma(),
            msg(Phrase::SetTo),
            msg(Phrase::Level),
            nb2msg(skill.number().into()),
        ]
        .concat();
        let campaign = campaign.clone();
        let action = Action::run(move |cx| {
            campaign.set_skill(cx, skill);
            Ok(Flow::Stay)
        });
        menu.append(label, action, None);
    }
}

/// The menu of the chapters unlocked: "continue" with the last one, then
/// each chapter, then the level of the computers, then back.
fn campaign_menu(cx: &Client, campaign: &Rc<Campaign>) -> Menu {
    let ini = cx.paths.campaigns_config();
    let mut menu = Menu::new(campaign.title.clone(), Vec::new());
    let available = campaign.available_chapters(&ini);
    if available.len() > 1 {
        let last = &available[available.len() - 1];
        menu.append(
            [msg(Phrase::Continue), last.title()].concat(),
            chapter_action(campaign, last.number()),
            None,
        );
    }
    for chapter in available {
        let prefix = if chapter.number() > 0 {
            nb2msg(chapter.number() as i64)
        } else {
            Vec::new()
        };
        menu.append(
            [prefix, chapter.title()].concat(),
            chapter_action(campaign, chapter.number()),
            None,
        );
    }
    level_choices(&mut menu, campaign, campaign.skill(&ini));
    menu.append(msg(Phrase::Back), Action::CloseMenu, None);
    menu
}

/// Plays a chapter: a cut scene goes on to the next chapter; a mission
/// asks to continue after a victory, to restart after a defeat.
pub fn run_chapter(cx: &mut Client, campaign: &Rc<Campaign>, number: usize) -> Result<(), Exit> {
    let Some(chapter) = campaign.chapters.get(number) else {
        return Ok(());
    };
    cx.voice.important(&chapter.title());
    match chapter {
        Chapter::CutScene { sequence, .. } => {
            cx.play_sequence(sequence);
            campaign.unlock_next(cx, number);
            if number + 1 < campaign.chapters.len() {
                run_chapter(cx, campaign, number + 1)?;
            }
        }
        Chapter::Mission { map, .. } => {
            let skill = campaign.skill(&cx.paths.campaigns_config());
            let game = Session::mission(cx, Arc::new(map.clone()), campaign, number, skill);
            play_mission(cx, campaign, number, game)?;
        }
    }
    Ok(())
}

/// Plays a mission of chapter `number`, then asks to continue after a
/// victory, which unlocks the next chapter, or to restart after a defeat.
fn play_mission(
    cx: &mut Client,
    campaign: &Rc<Campaign>,
    number: usize,
    mut game: Session,
) -> Result<(), Exit> {
    game.run(cx)?;
    if game.has_victory {
        campaign.unlock_next(cx, number);
        if number + 1 < campaign.chapters.len() {
            let mut menu = Menu::new(Vec::new(), Vec::new());
            menu.append(
                msg(Phrase::Continue),
                next_chapter_action(campaign, number + 1),
                None,
            );
            menu.append(msg(Phrase::Quit), Action::Nothing, None);
            menu.run(cx)?;
        }
    } else {
        let mut menu = Menu::new(Vec::new(), Vec::new());
        menu.append(
            msg(Phrase::Restart),
            next_chapter_action(campaign, number),
            None,
        );
        menu.append(msg(Phrase::Quit), Action::Nothing, None);
        menu.run(cx)?;
    }
    Ok(())
}

/// Plays on a mission saved in chapter `number` of `campaign`: switches
/// to the campaign's mods and resources, as [`run_campaign`] does, replays
/// the save, and plays it as the chapter's mission. A save of a mission
/// the campaign no longer has plays alone, on its own map.
pub fn resume_mission(
    cx: &mut Client,
    campaign: Rc<Campaign>,
    number: usize,
    saved: Saved,
) -> Result<(), Exit> {
    if let Some(mods) = &campaign.mods {
        cx.res.set_mods(mods);
    }
    cx.res.set_campaign(Some(&campaign));
    let result = (|| {
        let mission = matches!(campaign.chapters.get(number), Some(Chapter::Mission { .. }));
        let map = match campaign.chapters.get(number) {
            Some(Chapter::Mission { map, .. }) if map.definition == saved.record.setup().map => {
                Some(Arc::new(map.clone()))
            }
            _ => crate::saves::load::map_of(&saved),
        };
        let Some(map) = map else {
            return crate::saves::load::cannot_load(cx);
        };
        let Some(game) = crate::saves::load::replayed(cx, SessionKind::Mission, map, saved)? else {
            return Ok(());
        };
        if mission {
            play_mission(cx, &campaign, number, game)
        } else {
            let mut game = game;
            game.run(cx)
        }
    })();
    cx.res.set_campaign(None);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testkit::imported_tests_dir;

    #[test]
    fn the_bookmark_file_reads_and_writes_as_configparser_does() {
        let text = "[campaign1]\nchapter = 2\n\n[Other_one]\nChapter: 5\n";
        let sections = read_ini(text);
        assert_eq!(
            sections,
            vec![
                ("campaign1".into(), vec![("chapter".into(), "2".into())]),
                ("Other_one".into(), vec![("chapter".into(), "5".into())]),
            ]
        );
        assert_eq!(
            write_ini(&sections),
            "[campaign1]\nchapter = 2\n\n[Other_one]\nchapter = 5\n\n"
        );
    }

    #[test]
    fn the_level_is_normal_unless_one_was_chosen_and_is_kept_beside_the_bookmark() {
        let package = Package::from_path(&upstream_tests_dir().join("single/campaign1")).unwrap();
        let c = Campaign::new(package, "campaign1");
        let dir = tempfile::tempdir().unwrap();
        let ini = dir.path().join("campaigns.ini");
        let skill_in = |text: &str| {
            std::fs::write(&ini, text).unwrap();
            c.skill(&ini)
        };
        assert_eq!(c.skill(&ini), Skill::NORMAL, "no file");
        assert_eq!(skill_in("[campaign1]\nchapter = 2\n"), Skill::NORMAL);
        for bad in ["0", "4", "-1", "level 2", "", "99999999999999999999"] {
            let text = format!("[campaign1]\nlevel = {bad}\n");
            assert_eq!(skill_in(&text), Skill::NORMAL, "{bad:?}");
        }
        assert_eq!(skill_in("[other]\nlevel = 1\n"), Skill::NORMAL);
        assert_eq!(skill_in("[campaign1]\nLevel: 2\n"), Skill::Level2);
        // Writing the level keeps the bookmark and the other campaigns.
        std::fs::write(&ini, "[campaign1]\nchapter = 2\n\n[other]\nlevel = 1\n").unwrap();
        c.set_ini_value(&ini, "level", Skill::Level2.number().to_string())
            .unwrap();
        assert_eq!(c.skill(&ini), Skill::Level2);
        assert_eq!(c.bookmark(&ini), 2);
        assert_eq!(
            std::fs::read_to_string(&ini).unwrap(),
            "[campaign1]\nchapter = 2\nlevel = 2\n\n[other]\nlevel = 1\n\n"
        );
    }

    #[test]
    fn a_campaign_lists_its_chapters_from_a_folder_or_a_zip() {
        for path in ["single/campaign1", "single/campaign1.zip"] {
            let package = Package::from_path(&imported_tests_dir().join(path)).unwrap();
            let c = Campaign::new(package, "campaign1");
            assert_eq!(c.name, "campaign1");
            assert_eq!(c.resources.name, "campaign1");
            assert_eq!(c.chapters.len(), 3);
            let Chapter::Mission { map, .. } = &c.chapters[2] else {
                panic!("chapter 2 is a mission");
            };
            assert_eq!(map.name, "campaign1/2");
            assert_eq!(
                c.chapters[2].title(),
                vec![MsgPart::Count(11), MsgPart::Count(22)]
            );
            assert!(map.resources.is_none());
            let Chapter::Mission { map, .. } = &c.chapters[1] else {
                panic!("chapter 1 is a mission");
            };
            assert_eq!(map.name, "campaign1/1");
        }
    }
}
