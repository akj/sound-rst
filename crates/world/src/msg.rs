//! Spoken messages, as sequences of parts.

use std::fmt;

pub use crate::phrases::{Cue, Pause, Phrase};

/// The longest name of the game's files, in bytes. The original's longest,
/// once named, has 43.
pub const MAX_NAME_BYTES: usize = 64;

/// Whether `s` has the shape of a name of the game's files: words of lower
/// case letters, digits and `_` that start with a letter, joined by `/`.
/// A name without a slash is a phrase: a line of a language file. A name
/// with one is a sound: its path under `sounds/`, without `.ogg`.
pub fn is_name(s: &str) -> bool {
    fn is_word(s: &str) -> bool {
        let mut chars = s.chars();
        chars.next().is_some_and(|c| c.is_ascii_lowercase())
            && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
    }
    s.len() <= MAX_NAME_BYTES && s.split('/').all(is_word)
}

/// One part of a spoken message.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum MsgPart {
    /// A phrase the code names.
    Phrase(Phrase),
    /// A sound the code plays by itself.
    Sound(Cue),
    /// A count, read aloud.
    Count(u64),
    /// A silence.
    Pause(Pause),
    /// Words said as written and never looked up: what a person wrote, such
    /// as a login or a chat line, and what the code computed, such as a
    /// speed.
    Literal(String),
    /// A name from the files of the game: a style, a map, a campaign, a
    /// help file. It names a phrase of a language file or a recording
    /// under `sounds/` (see [`is_name`]). Make one with [`MsgPart::named`].
    Name(String),
}

/// A spoken message. The parts are said in order.
pub type Msg = Vec<MsgPart>;

impl MsgPart {
    /// The part that a word of the game's files names: a count if the word
    /// is a whole number, a name if it has the shape of one. None for any
    /// other word.
    pub fn named(word: &str) -> Option<MsgPart> {
        if !word.is_empty() && word.bytes().all(|b| b.is_ascii_digit()) {
            return word.parse().ok().map(MsgPart::Count);
        }
        is_name(word).then(|| MsgPart::Name(word.to_owned()))
    }

    /// The parts that the words of a line of the game's files name. The
    /// error is the first word that names nothing.
    pub fn all_named<'a>(words: impl IntoIterator<Item = &'a str>) -> Result<Msg, &'a str> {
        words
            .into_iter()
            .map(|w| MsgPart::named(w).ok_or(w))
            .collect()
    }

    /// Words said as written.
    pub fn literal(words: impl Into<String>) -> MsgPart {
        MsgPart::Literal(words.into())
    }
}

/// The part in English, for a log or a test: the words of a phrase, the
/// name of a sound, a name of the game's files as written. It is not what
/// the player hears.
impl fmt::Display for MsgPart {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MsgPart::Phrase(p) => f.write_str(p.english()),
            MsgPart::Sound(c) => f.write_str(c.sound()),
            MsgPart::Count(n) => write!(f, "{n}"),
            MsgPart::Pause(Pause::Short) => f.write_str(","),
            MsgPart::Pause(Pause::Long) => f.write_str("."),
            MsgPart::Literal(s) | MsgPart::Name(s) => f.write_str(s),
        }
    }
}

impl From<Phrase> for MsgPart {
    fn from(p: Phrase) -> Self {
        MsgPart::Phrase(p)
    }
}

impl From<Cue> for MsgPart {
    fn from(c: Cue) -> Self {
        MsgPart::Sound(c)
    }
}

impl From<Pause> for MsgPart {
    fn from(p: Pause) -> Self {
        MsgPart::Pause(p)
    }
}

/// A phrase as a message.
pub fn msg(phrase: Phrase) -> Msg {
    vec![MsgPart::Phrase(phrase)]
}

/// A sound the code plays by itself as a message.
pub fn cue(cue: Cue) -> Msg {
    vec![MsgPart::Sound(cue)]
}

/// A short silence as a message.
pub fn comma() -> Msg {
    vec![MsgPart::Pause(Pause::Short)]
}

/// A long silence as a message.
pub fn period() -> Msg {
    vec![MsgPart::Pause(Pause::Long)]
}

/// The message that reads the count `n` aloud. A negative count says nothing.
pub fn nb2msg(n: i64) -> Msg {
    match u64::try_from(n) {
        Ok(n) => vec![MsgPart::Count(n)],
        Err(_) => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_negative_count_is_silent() {
        assert_eq!(nb2msg(12), vec![MsgPart::Count(12)]);
        assert_eq!(nb2msg(0), vec![MsgPart::Count(0)]);
        assert_eq!(nb2msg(-1), Vec::new());
    }

    #[test]
    fn a_word_of_a_file_is_a_name_or_a_count() {
        let name = |s: &str| Some(MsgPart::Name(s.to_owned()));
        assert_eq!(MsgPart::named("footman"), name("footman"));
        assert_eq!(MsgPart::named("footstep/grass_1"), name("footstep/grass_1"));
        assert_eq!(MsgPart::named("10"), Some(MsgPart::Count(10)));
        assert_eq!(MsgPart::named("007"), Some(MsgPart::Count(7)));
        for bad in [
            "",
            "Footman",
            "1st",
            "-1",
            "+1",
            "a b",
            "a/",
            "/a",
            "a//b",
            "a-b",
            "$1",
            "é",
            "99999999999999999999999",
        ] {
            assert_eq!(MsgPart::named(bad), None, "{bad}");
        }
        assert!(is_name(&"a".repeat(MAX_NAME_BYTES)));
        assert!(!is_name(&"a".repeat(MAX_NAME_BYTES + 1)));
        assert_eq!(
            MsgPart::all_named(["you_have", "2"]),
            Ok(vec![MsgPart::Name("you_have".into()), MsgPart::Count(2)])
        );
        assert_eq!(MsgPart::all_named(["you_have", "X"]), Err("X"));
    }

    #[test]
    fn every_phrase_has_an_id_and_english_words() {
        let mut ids = std::collections::BTreeSet::new();
        for p in Phrase::all() {
            let id = p.id();
            let mut chars = id.chars();
            assert!(chars.next().is_some_and(|c| c.is_ascii_lowercase()), "{id}");
            assert!(
                chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'),
                "{id}"
            );
            assert!(!p.english().trim().is_empty(), "{id}");
            assert!(ids.insert(id), "{id} twice");
        }
        assert_eq!(Phrase::AwaitingYourOrders.id(), "awaiting_your_orders");
        let found = Phrase::from_id("awaiting_your_orders");
        assert_eq!(found, Some(Phrase::AwaitingYourOrders));
        assert_eq!(Phrase::from_id("AwaitingYourOrders"), None);
        assert!(Phrase::all().all(|p| Phrase::from_id(p.id()) == Some(p)));
        assert_eq!(Phrase::AwaitingYourOrders.english(), "awaiting your orders");
        assert_eq!(Phrase::OutOf.note(), "As in \"2 of 3\".");
        assert_eq!(Phrase::IsReady.note(), "");
        assert_eq!(Phrase::letter(0), Some(Phrase::LetterA));
        assert_eq!(Phrase::letter(25), Some(Phrase::LetterZ));
        assert_eq!(Phrase::letter(26), None);
    }
}
