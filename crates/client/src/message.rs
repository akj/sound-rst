//! News: what the game tells the player without being asked, sorted by
//! category, with what it is about, and what the player chose to hear of
//! each category.

use serde::{Deserialize, Serialize};
use soundrts_protocol::SlotIndex;
use soundrts_world::events::Reason;
use soundrts_world::msg::Msg;
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

    /// The categories whose news is said before the other news waiting,
    /// in this order: what players say, news of the game or the connection
    /// itself, then the story.
    pub const AHEAD: [Category; 3] = [Category::Chat, Category::System, Category::Story];

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

/// What a piece of news is about. Newer news about a subject replaces the
/// news about it the player has not heard yet.
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

/// A message the game volunteers.
#[derive(Clone, Debug, PartialEq)]
pub struct News {
    pub category: Category,
    pub parts: Msg,
    pub subject: Option<Subject>,
    /// Where it happened.
    pub square: Option<SquareId>,
}

impl News {
    pub fn new(category: Category, parts: Msg) -> News {
        News {
            category,
            parts,
            subject: None,
            square: None,
        }
    }

    pub fn about(mut self, subject: Subject) -> News {
        self.subject = Some(subject);
        self
    }

    pub fn at(mut self, square: Option<SquareId>) -> News {
        self.square = square;
        self
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
        match sounds.translate(part) {
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
