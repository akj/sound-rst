//! Campaigns (`campaign.py`): the packages the resource stack discovers
//! (title, mods, chapters), the chapter menu, the sequence of missions and
//! cut scenes, and the bookmark of the last chapter unlocked.

use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;

use soundrts_world::msg::{Msg, MsgPart, msg, nb2msg};
use soundrts_world::msgparts as mp;

use crate::client::Client;
use crate::mapfile::Map;
use crate::menu::{Action, Exit, Flow, Menu};
use crate::package::Package;
use crate::session::Session;

/// A numbered step of a campaign.
#[derive(Clone, Debug)]
pub enum Chapter {
    /// A game on a map.
    Mission { number: usize, map: Map },
    /// Sounds and texts said in order, between missions.
    CutScene {
        number: usize,
        title: Msg,
        sequence: Vec<String>,
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
        let title = match header_line(&text, "title", words) {
            Some(t) => t.split(' ').map(MsgPart::from).collect(),
            None => vec![MsgPart::Text(name.clone())],
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
                    let digits = |c: char| c.is_ascii_digit() || c == ' ';
                    let title = match header_line(&text, "title", digits) {
                        Some(t) => t
                            .split(' ')
                            .filter_map(|x| x.parse().ok())
                            .map(MsgPart::Num)
                            .collect(),
                        None => nb2msg(number),
                    };
                    let sequence = header_line(&text, "sequence", digits)
                        .map(|s| s.split(' ').map(str::to_owned).collect())
                        .unwrap_or_default();
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

    /// The number of the last chapter unlocked (0 at first).
    pub fn bookmark(&self, ini: &Path) -> usize {
        let text = std::fs::read_to_string(ini).unwrap_or_default();
        read_ini(&text)
            .into_iter()
            .find(|(s, _)| *s == self.ini_id())
            .and_then(|(_, keys)| {
                keys.into_iter()
                    .find(|(k, _)| k == "chapter")
                    .and_then(|(_, v)| v.parse().ok())
            })
            .unwrap_or(0)
    }

    fn set_bookmark(&self, ini: &Path, number: usize) -> std::io::Result<()> {
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
        match keys.iter_mut().find(|(k, _)| k == "chapter") {
            Some(kv) => kv.1 = number.to_string(),
            None => keys.push(("chapter".into(), number.to_string())),
        }
        std::fs::write(ini, write_ini(&sections))
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
    let result = campaign_menu(cx, &campaign).and_then(|mut m| m.run(cx));
    cx.res.set_campaign(None);
    result
}

fn chapter_action(campaign: &Rc<Campaign>, number: usize) -> Action {
    let campaign = campaign.clone();
    Action::run(move |cx| run_chapter(cx, &campaign, number).map(|()| Flow::Stay))
}

/// The menu of the chapters unlocked: "continue" with the last one, then
/// each chapter, then back.
fn campaign_menu(cx: &Client, campaign: &Rc<Campaign>) -> Result<Menu, Exit> {
    let ini = cx.paths.campaigns_config();
    let mut menu = Menu::new(campaign.title.clone(), Vec::new());
    let available = campaign.available_chapters(&ini);
    if available.len() > 1 {
        let last = &available[available.len() - 1];
        menu.append(
            [msg(mp::CONTINUE), last.title()].concat(),
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
    menu.append(msg(mp::BACK), Action::Nothing, None);
    Ok(menu)
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
            let mut game = Session::mission(cx, Arc::new(map.clone()));
            game.run(cx)?;
            if game.has_victory {
                campaign.unlock_next(cx, number);
                if number + 1 < campaign.chapters.len() {
                    let mut menu = Menu::new(Vec::new(), Vec::new());
                    menu.append(
                        msg(mp::CONTINUE),
                        chapter_action(campaign, number + 1),
                        None,
                    );
                    menu.append(msg(mp::QUIT), Action::Nothing, None);
                    menu.run(cx)?;
                }
            } else {
                let mut menu = Menu::new(Vec::new(), Vec::new());
                menu.append(msg(mp::RESTART), chapter_action(campaign, number), None);
                menu.append(msg(mp::QUIT), Action::Nothing, None);
                menu.run(cx)?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testkit::upstream_tests_dir;

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
    fn a_campaign_lists_its_chapters_from_a_folder_or_a_zip() {
        for path in ["single/campaign1", "single/campaign1.zip"] {
            let package = Package::from_path(&upstream_tests_dir().join(path)).unwrap();
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
                vec![MsgPart::Num(11), MsgPart::Num(22)]
            );
            assert!(map.resources.is_none());
            let Chapter::Mission { map, .. } = &c.chapters[1] else {
                panic!("chapter 1 is a mission");
            };
            assert_eq!(map.name, "campaign1/1");
        }
    }
}
