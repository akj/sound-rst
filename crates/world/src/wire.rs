//! How a few types are sent where serde's derived form will not do: a map
//! whose keys are not text, and the remembered entity an order can aim at,
//! whose own orders can aim at another, without end.
//!
//! What is read here comes from a stranger: nothing panics, and a list is
//! never allocated beyond what the bytes read so far hold.
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use std::cell::Cell;

use serde::de::Error as _;
use serde::ser::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::known::KnownEntity;

/// A map sent as a list of key and value pairs, keys in order. Text
/// formats such as JSON take only text as a map's key; pairs take any. A
/// key out of order or twice is refused: the sender never writes one.
pub(crate) mod pairs {
    use std::collections::BTreeMap;

    use super::*;

    pub(crate) fn serialize<K, V, S>(map: &BTreeMap<K, V>, s: S) -> Result<S::Ok, S::Error>
    where
        K: Serialize,
        V: Serialize,
        S: Serializer,
    {
        s.collect_seq(map)
    }

    pub(crate) fn deserialize<'de, K, V, D>(d: D) -> Result<BTreeMap<K, V>, D::Error>
    where
        K: Deserialize<'de> + Ord,
        V: Deserialize<'de>,
        D: Deserializer<'de>,
    {
        let mut map = BTreeMap::new();
        for (k, v) in Vec::<(K, V)>::deserialize(d)? {
            if map.last_key_value().is_some_and(|(last, _)| *last >= k) {
                return Err(D::Error::custom("map keys out of order"));
            }
            map.insert(k, v);
        }
        Ok(map)
    }
}

/// How deep remembered entities may nest in order targets: an order aims
/// at a remembered entity, whose own orders aim at another... The world
/// makes one only for a unit the seat controls and remembers, so more than
/// one level is rare. Both sides refuse a deeper one, so that the sender
/// never writes what the receiver refuses, and the receiver's stack is
/// bounded: postcard has no limit of its own, and JSON's is 128 levels.
pub(crate) const MAX_NESTED_ENTITIES: u32 = 8;

thread_local! {
    /// The remembered entities being written or read on this thread, one
    /// inside the other.
    static DEPTH: Cell<u32> = const { Cell::new(0) };
}

/// One more level of [`DEPTH`] while it lives, if there is room.
struct Nested;

impl Nested {
    fn enter() -> Option<Nested> {
        let depth = DEPTH.get();
        (depth < MAX_NESTED_ENTITIES).then(|| {
            DEPTH.set(depth.saturating_add(1));
            Nested
        })
    }
}

impl Drop for Nested {
    fn drop(&mut self) {
        DEPTH.set(DEPTH.get().saturating_sub(1));
    }
}

const TOO_DEEP: &str = "remembered entities nested deeper than MAX_NESTED_ENTITIES";

/// [`KnownTarget::Remembered`](crate::KnownTarget::Remembered), at most
/// [`MAX_NESTED_ENTITIES`] deep.
pub(crate) mod nested_entity {
    use super::*;

    pub(crate) fn serialize<S: Serializer>(e: &KnownEntity, s: S) -> Result<S::Ok, S::Error> {
        let _level = Nested::enter().ok_or_else(|| S::Error::custom(TOO_DEEP))?;
        e.serialize(s)
    }

    pub(crate) fn deserialize<'de, D: Deserializer<'de>>(
        d: D,
    ) -> Result<Box<KnownEntity>, D::Error> {
        let _level = Nested::enter().ok_or_else(|| D::Error::custom(TOO_DEEP))?;
        KnownEntity::deserialize(d).map(Box::new)
    }
}
