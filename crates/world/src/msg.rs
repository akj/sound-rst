//! Spoken messages, as sequences of numbered sounds and literal text.

use std::fmt;

/// Numbers from this value up are counts to read aloud, not sound names:
/// `NB_ENCODE_SHIFT + 12` is spoken as "12".
pub const NB_ENCODE_SHIFT: u64 = 1_000_000;

/// One part of a spoken message.
///
/// A number names a recorded sound or a line of `ui/tts.txt`. Text is either
/// such a name written as a string (map files do this) or words to speak as
/// written. The two forms of the same name are looked up the same way, by
/// [`MsgPart::key`]. A literal is what a person wrote, such as a login or a
/// chat line: it is said as written and never looked up.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum MsgPart {
    Num(u64),
    Text(String),
    Literal(String),
}

/// A spoken message. The parts are said in order.
pub type Msg = Vec<MsgPart>;

impl MsgPart {
    /// The name used to look this part up in the text table and the sounds.
    pub fn key(&self) -> String {
        self.to_string()
    }
}

impl fmt::Display for MsgPart {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MsgPart::Num(n) => write!(f, "{n}"),
            MsgPart::Text(s) | MsgPart::Literal(s) => f.write_str(s),
        }
    }
}

impl From<u64> for MsgPart {
    fn from(n: u64) -> Self {
        MsgPart::Num(n)
    }
}

impl From<&str> for MsgPart {
    fn from(s: &str) -> Self {
        MsgPart::Text(s.to_owned())
    }
}

impl From<String> for MsgPart {
    fn from(s: String) -> Self {
        MsgPart::Text(s)
    }
}

/// Builds a message from sound numbers, such as a constant of [`crate::msgparts`].
pub fn msg(numbers: &[u64]) -> Msg {
    numbers.iter().copied().map(MsgPart::Num).collect()
}

/// The message that reads the count `n` aloud. A negative count says nothing.
pub fn nb2msg(n: i64) -> Msg {
    match u64::try_from(n) {
        Ok(n) => vec![MsgPart::Num(NB_ENCODE_SHIFT + n)],
        Err(_) => Vec::new(),
    }
}

/// Packs a message into one word of the text command protocol.
pub fn encode_msg(msg: &[MsgPart]) -> String {
    msg.iter().map(MsgPart::key).collect::<Vec<_>>().join("***")
}

/// Unpacks a message packed by [`encode_msg`]. Every part comes back as text.
pub fn decode_msg(s: &str) -> Msg {
    s.split("***").map(MsgPart::from).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_count_is_shifted_and_a_negative_count_is_silent() {
        assert_eq!(nb2msg(12), vec![MsgPart::Num(1_000_012)]);
        assert_eq!(nb2msg(0), vec![MsgPart::Num(1_000_000)]);
        assert_eq!(nb2msg(-1), Vec::new());
    }

    #[test]
    fn a_number_and_its_text_form_share_a_key() {
        assert_eq!(MsgPart::Num(4053).key(), "4053");
        assert_eq!(MsgPart::from("4053").key(), "4053");
    }

    #[test]
    fn a_message_survives_packing_as_text_parts() {
        let packed = encode_msg(&[MsgPart::Num(150), MsgPart::from("bob")]);
        assert_eq!(packed, "150***bob");
        assert_eq!(
            decode_msg(&packed),
            vec![MsgPart::from("150"), MsgPart::from("bob")]
        );
    }
}
