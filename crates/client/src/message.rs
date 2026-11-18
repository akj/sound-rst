//! News: what the game tells the player without being asked, sorted by
//! category, with what it is about, and what the player chose to hear of
//! each category.

use serde::{Deserialize, Serialize};
use soundrts_protocol::SlotIndex;
use soundrts_world::events::Reason;
use soundrts_world::msg::{Msg, Phrase, comma, msg, nb2msg};
use soundrts_world::{Place, SquareId};

use crate::sound_cache::{Sounds, Spoken};

/// The kind of news a message is. What the player asks for (moving the
/// cursor, a status key, an order's confirmation) is not news.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Category {
    /// The player is attacked, loses units or wins a fight.
    Alert,
    /// Units trained or added, buildings finished, research and upgrades
    /// done, production deferred or refused.
    Production,
    /// Resources, food, and deposits exhausted.
    Economy,
    /// Enemies and resources found.
    Scouting,
    /// What the other players say.
    Chat,
    /// Players arriving and leaving, the connection, the state of the game.
    System,
    /// Objectives and cut scenes.
    Story,
}

impl Category {
    pub const ALL: [Category; 7] = [
        Category::Alert,
        Category::Production,
        Category::Economy,
        Category::Scouting,
        Category::Chat,
        Category::System,
        Category::Story,
    ];

    /// The order in which the voice says the news waiting, a category's
    /// oldest first: news of the game or the connection itself, alerts,
    /// the story, what players say, then the rest.
    pub const ORDER: [Category; 7] = [
        Category::System,
        Category::Alert,
        Category::Story,
        Category::Chat,
        Category::Production,
        Category::Economy,
        Category::Scouting,
    ];

    /// The category after this one, the first after the last.
    pub fn next(self) -> Category {
        let i = Category::ALL.iter().position(|&c| c == self).unwrap_or(0);
        Category::ALL[(i + 1) % Category::ALL.len()]
    }
}

/// What the player hears of a category of news.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Policy {
    /// Said in turn, with its sound.
    Speak,
    /// Only its sound: the sound of the event, and the recorded sounds of
    /// the message in turn. Its words wait in the log, unheard.
    Sound,
    /// Kept in the log, unheard, without a sound.
    Log,
    /// Neither said, nor played, nor kept.
    Drop,
}

impl Policy {
    /// Whether the news comes with the sound of its event.
    pub fn plays_sound(self) -> bool {
        matches!(self, Policy::Speak | Policy::Sound)
    }
}

/// A policy for each category: the `[news]` table of `settings.toml`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Policies {
    alert: Policy,
    production: Policy,
    economy: Policy,
    scouting: Policy,
    chat: Policy,
    system: Policy,
    story: Policy,
}

impl Default for Policies {
    fn default() -> Self {
        let speak = Policy::Speak;
        Policies {
            alert: speak,
            production: speak,
            economy: speak,
            scouting: speak,
            chat: speak,
            system: speak,
            story: speak,
        }
    }
}

impl Policies {
    fn slot(&mut self, c: Category) -> &mut Policy {
        match c {
            Category::Alert => &mut self.alert,
            Category::Production => &mut self.production,
            Category::Economy => &mut self.economy,
            Category::Scouting => &mut self.scouting,
            Category::Chat => &mut self.chat,
            Category::System => &mut self.system,
            Category::Story => &mut self.story,
        }
    }

    pub fn of(&self, c: Category) -> Policy {
        match c {
            Category::Alert => self.alert,
            Category::Production => self.production,
            Category::Economy => self.economy,
            Category::Scouting => self.scouting,
            Category::Chat => self.chat,
            Category::System => self.system,
            Category::Story => self.story,
        }
    }

    pub fn set(&mut self, c: Category, p: Policy) {
        *self.slot(c) = p;
    }
}

/// What a room announcement of a server's lobby is about.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum RoomTopic {
    /// Who is in the room.
    Players,
    Faction(SlotIndex),
    Alliance(SlotIndex),
    /// A computer's level.
    Skill(SlotIndex),
    Income(SlotIndex),
    Ready(SlotIndex),
    /// Who organizes the game.
    Host,
    /// Whether this person watches.
    Watcher(soundrts_protocol::Login),
}

/// What a piece of news is about. Newer news about a subject joins the
/// news about it the player has not heard yet, as the newer news says.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Subject {
    /// The count of the resource with this index.
    Resource(usize),
    Food,
    /// The player's units attacked at a place.
    Attack(Place),
    /// The player's units lost at a place.
    Lost(Place),
    /// Units the player neutralized at a place.
    Neutralized(Place),
    /// The enemies where the player has just lost a fight.
    EnemiesAt(Place),
    /// Enemies seen for the first time at a place.
    EnemiesSighted(Place),
    /// The orders of a type of unit.
    Menu(String),
    /// An order refused for this reason.
    Refusal(Reason),
    ProductionDeferred,
    /// What the room's host did to the clock of a game on a server.
    Clock,
    Room(RoomTopic),
}

/// "3 footmen, 2 archers and 1 peasant": each kind with its count, in the
/// order they come. With `brief` and more than two kinds, only the count.
pub fn summary(group: &[Msg], brief: bool) -> Msg {
    let mut kinds: Vec<&Msg> = Vec::new();
    for m in group {
        if !kinds.contains(&m) {
            kinds.push(m);
        }
    }
    if brief && kinds.len() > 2 {
        return nb2msg(group.len() as i64);
    }
    let mut result = Vec::new();
    for (i, &kind) in kinds.iter().enumerate() {
        if i + 1 == kinds.len() && i > 0 {
            result.extend(msg(Phrase::And));
        } else if i > 0 {
            result.extend(comma());
        }
        result.extend(nb2msg(group.iter().filter(|&m| m == kind).count() as i64));
        result.extend(kind.iter().cloned());
    }
    result
}

/// Where a count is said among the counts of the same moment: the
/// smallest first, and those of the same rank in the order they came. The
/// game ranks them: by the kind of count, then by the place it counts.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Rank(pub u32, pub u32);

/// What a piece of news counts: one item for each thing, as its title,
/// said as a [`summary`], then the words that follow the count.
#[derive(Clone, Debug, PartialEq)]
pub struct Count {
    items: Vec<Msg>,
    brief: bool,
    then: Msg,
    rank: Rank,
}

impl Count {
    /// "3 footman and 1 archer lost at b 2" counts `items` (footman,
    /// footman, archer, footman), with `brief` as [`summary`] takes it,
    /// and `then` is "lost at b 2".
    pub fn new(items: Vec<Msg>, brief: bool, then: Msg, rank: Rank) -> Count {
        Count {
            items,
            brief,
            then,
            rank,
        }
    }

    fn words(&self) -> Msg {
        [summary(&self.items, self.brief), self.then.clone()].concat()
    }
}

/// How newer news about a subject joins the news about it that the player
/// has not heard yet. With none such, newer news is news of its own, except
/// an update.
#[derive(Clone, Debug, PartialEq)]
enum Join {
    /// Takes the older news's place.
    Replace,
    /// Takes the older news's place, and is not news without it: a count
    /// that only news worth saying on its own brings to the player.
    Update,
    /// Takes the older news's place, and counts what the older news
    /// counted too.
    Count(Count),
}

/// A message the game volunteers.
#[derive(Clone, Debug, PartialEq)]
pub struct News {
    pub category: Category,
    pub parts: Msg,
    about: Option<(Subject, Join)>,
    /// Where it happened.
    pub square: Option<SquareId>,
}

impl News {
    pub fn new(category: Category, parts: Msg) -> News {
        News {
            category,
            parts,
            about: None,
            square: None,
        }
    }

    /// News that counts things about `subject`, such as the units lost at
    /// a place. It waits until the news of its moment is complete, with
    /// the other counts about its subject; then what they count is added
    /// to the unheard news about it.
    pub fn counting(category: Category, subject: Subject, count: Count) -> News {
        News {
            category,
            parts: count.words(),
            about: Some((subject, Join::Count(count))),
            square: None,
        }
    }

    /// News about `subject`: it replaces the unheard news about it.
    pub fn about(mut self, subject: Subject) -> News {
        self.about = Some((subject, Join::Replace));
        self
    }

    /// News that brings the unheard news about `subject` up to date: it
    /// replaces that news, and is dropped when there is none.
    pub fn updating(mut self, subject: Subject) -> News {
        self.about = Some((subject, Join::Update));
        self
    }

    pub fn at(mut self, square: Option<SquareId>) -> News {
        self.square = square;
        self
    }

    pub fn subject(&self) -> Option<&Subject> {
        self.about.as_ref().map(|(s, _)| s)
    }

    /// The rank of a count, `None` for news that counts nothing.
    pub(crate) fn rank(&self) -> Option<Rank> {
        match &self.about {
            Some((_, Join::Count(c))) => Some(c.rank),
            _ => None,
        }
    }

    /// Newer news about its subject replaces it, without counting what it
    /// counted.
    pub(crate) fn stop_counting(&mut self) {
        if let Some((_, join @ Join::Count(_))) = &mut self.about {
            *join = Join::Replace;
        }
    }

    /// This news after `older`, the news about its subject that the player
    /// has not heard yet, if there is any: what takes its place, or `None`
    /// when this news is not news.
    pub(crate) fn join(self, older: Option<&News>) -> Option<News> {
        let older_count = older.and_then(|n| match &n.about {
            Some((_, Join::Count(c))) => Some(c),
            _ => None,
        });
        match (self.about, older_count) {
            (Some((_, Join::Update)), _) if older.is_none() => None,
            (Some((subject, Join::Count(new))), Some(old)) => {
                let count = Count {
                    items: [old.items.as_slice(), &new.items].concat(),
                    ..new
                };
                Some(News {
                    parts: count.words(),
                    about: Some((subject, Join::Count(count))),
                    ..self
                })
            }
            (about, _) => Some(News { about, ..self }),
        }
    }
}

/// Which parts of a message are played.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Parts {
    All,
    Texts,
    Sounds,
}

/// A message as texts and sounds, with neighbouring texts joined by a
/// space. Texts that a sound separated stay separate even without the
/// sounds.
pub fn translate_and_collapse(parts: &Msg, sounds: &Sounds, which: Parts) -> Vec<Spoken> {
    let mut result: Vec<Spoken> = Vec::new();
    let mut after_text = false;
    for part in parts {
        let Some(spoken) = sounds.translate(part) else {
            continue;
        };
        match spoken {
            Spoken::Sound(s) => {
                after_text = false;
                if which != Parts::Texts {
                    result.push(Spoken::Sound(s));
                }
            }
            Spoken::Text(_) if which == Parts::Sounds => {}
            Spoken::Text(t) => {
                match result.last_mut() {
                    Some(Spoken::Text(previous)) if after_text => {
                        previous.push(' ');
                        previous.push_str(&t);
                    }
                    _ => result.push(Spoken::Text(t)),
                }
                after_text = true;
            }
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use soundrts_world::msg::MsgPart;

    fn t(n: u64) -> Msg {
        vec![MsgPart::Count(n)]
    }

    #[test]
    fn the_order_of_the_news_holds_every_category_once() {
        for c in Category::ALL {
            let n = Category::ORDER.iter().filter(|&&o| o == c).count();
            assert_eq!(n, 1, "{c:?}");
        }
    }

    #[test]
    fn a_summary_counts_each_kind_in_order() {
        let n = |k| nb2msg(k);
        assert_eq!(summary(&[], false), Vec::new());
        assert_eq!(summary(&[t(1), t(1)], false), [n(2), t(1)].concat());
        assert_eq!(
            summary(&[t(1), t(2), t(1), t(3)], false),
            [
                n(2),
                t(1),
                comma(),
                n(1),
                t(2),
                msg(Phrase::And),
                n(1),
                t(3)
            ]
            .concat()
        );
        assert_eq!(
            summary(&[t(1), t(2)], true),
            [n(1), t(1), msg(Phrase::And), n(1), t(2)].concat()
        );
        assert_eq!(summary(&[t(1), t(2), t(3), t(3)], true), n(4));
    }
}
