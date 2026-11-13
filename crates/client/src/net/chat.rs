//! The chat lines heard on a server, for the player to read again.

use std::collections::VecDeque;

use soundrts_protocol::{Channel, ChatLine};
use soundrts_world::msg::{Msg, msg};
use soundrts_world::msgparts as mp;

use crate::sound_cache::literal;

/// The sound before a chat line: the alert of the conversion ability,
/// rarely heard in a game.
pub const CHAT_SOUND: u64 = 6114;

/// The most lines kept.
const CAPACITY: usize = 100;

/// The lines, oldest first, and the one the player last read again.
#[derive(Default)]
pub struct ChatLog {
    lines: VecDeque<ChatLine>,
    /// The line read last with [`ChatLog::previous`] or [`ChatLog::following`].
    cursor: Option<usize>,
}

impl ChatLog {
    /// Adds a line. Reading again starts from the newest one.
    pub fn push(&mut self, line: ChatLine) {
        if self.lines.len() == CAPACITY {
            self.lines.pop_front();
        }
        self.lines.push_back(line);
        self.cursor = None;
    }

    /// The line before the one read last (the newest one at first); the
    /// oldest one stays.
    pub fn previous(&mut self) -> Option<&ChatLine> {
        let i = match self.cursor {
            None => self.lines.len().checked_sub(1)?,
            Some(i) => i.saturating_sub(1),
        };
        self.cursor = Some(i);
        self.lines.get(i)
    }

    /// The line after the one read last; the newest one stays.
    pub fn following(&mut self) -> Option<&ChatLine> {
        let last = self.lines.len().checked_sub(1)?;
        let i = match self.cursor {
            None => last,
            Some(i) => (i + 1).min(last),
        };
        self.cursor = Some(i);
        self.lines.get(i)
    }

    pub fn len(&self) -> usize {
        self.lines.len()
    }

    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }
}

/// How a chat line is said: who says what, "allied" first for a line to
/// the allies.
pub fn line_msg(line: &ChatLine) -> Msg {
    let mut m = Vec::new();
    if line.channel == Channel::Allies {
        m.extend(msg(mp::ALLY));
    }
    m.push(literal(line.from.as_str()));
    m.extend(msg(mp::SAYS));
    m.push(literal(line.text.as_str()));
    m
}

#[cfg(test)]
mod tests {
    use super::*;
    use soundrts_protocol::{ChatText, Login};

    fn line(s: &str) -> ChatLine {
        ChatLine {
            channel: Channel::Lobby,
            from: Login::new("a").unwrap(),
            text: ChatText::clean(s).unwrap(),
        }
    }

    fn text(l: Option<&ChatLine>) -> Option<&str> {
        l.map(|l| l.text.as_str())
    }

    #[test]
    fn reading_again_goes_back_from_the_newest_line_and_stops_at_the_ends() {
        let mut log = ChatLog::default();
        assert_eq!(text(log.previous()), None);
        for s in ["one", "two", "three"] {
            log.push(line(s));
        }
        assert_eq!(text(log.previous()), Some("three"));
        assert_eq!(text(log.previous()), Some("two"));
        assert_eq!(text(log.previous()), Some("one"));
        assert_eq!(text(log.previous()), Some("one"));
        assert_eq!(text(log.following()), Some("two"));
        assert_eq!(text(log.following()), Some("three"));
        assert_eq!(text(log.following()), Some("three"));
        log.push(line("four"));
        assert_eq!(text(log.previous()), Some("four"));
    }

    #[test]
    fn the_oldest_lines_go_when_the_log_is_full() {
        let mut log = ChatLog::default();
        for i in 0..CAPACITY + 5 {
            log.push(line(&format!("line {i}")));
        }
        assert_eq!(log.len(), CAPACITY);
        for _ in 0..CAPACITY + 1 {
            log.previous();
        }
        assert_eq!(text(log.previous()), Some("line 5"));
    }
}
