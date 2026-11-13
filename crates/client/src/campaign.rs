//! Campaign packages, as the resource stack discovers them: title, mods and
//! chapters. Playing a campaign is the game interface's job.

use soundrts_world::msg::{Msg, MsgPart, nb2msg};

use crate::mapfile::Map;
use crate::package::Package;

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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testkit::upstream_tests_dir;

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
