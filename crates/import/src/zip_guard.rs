//! A check on a zip file from a stranger, made before a reader builds an
//! index of its entries.
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::string_slice,
    clippy::arithmetic_side_effects
)]

/// Where the fields of a zip end record lie: its size, the offsets of its
/// two disk numbers and of its two entry counts (on this disk, in all), and
/// the size of each.
struct EndRecord {
    size: usize,
    disks: [usize; 2],
    disk_len: usize,
    counts: [usize; 2],
    count_len: usize,
}

const END_32: EndRecord = EndRecord {
    size: 22,
    disks: [4, 6],
    disk_len: 2,
    counts: [8, 10],
    count_len: 2,
};

const END_64: EndRecord = EndRecord {
    size: 56,
    disks: [16, 20],
    disk_len: 4,
    counts: [24, 32],
    count_len: 8,
};

/// The entries an end record at `at` states, if a reader could use it: the
/// whole record is in the file, both disk numbers are 0 and both counts
/// agree. A reader skips any other record before it reserves room for its
/// entries (zip 9.5.1, `find_central_directory_end` and
/// `read_central_header`).
fn stated_entries(bytes: &[u8], at: usize, record: &EndRecord) -> Option<u64> {
    let field = |offset: usize, len: usize| -> Option<u64> {
        let start = at.checked_add(offset)?;
        let b = bytes.get(start..start.checked_add(len)?)?;
        Some(b.iter().rev().fold(0, |n, &x| (n << 8) | u64::from(x)))
    };
    bytes.get(at..at.checked_add(record.size)?)?;
    let [d1, d2] = record.disks;
    if field(d1, record.disk_len)? != 0 || field(d2, record.disk_len)? != 0 {
        return None;
    }
    let [c1, c2] = record.counts;
    let on_this_disk = field(c1, record.count_len)?;
    (on_this_disk == field(c2, record.count_len)?).then_some(on_this_disk)
}

/// Whether a zip file can have more than `max` entries, checked
/// before an index of them is built, whichever directory a reader picks:
/// - each entry of a central directory starts with its signature, so the
///   signatures anywhere in the file are at least the entries read, those
///   that share a name included;
/// - a reader reserves room for the count an end record states, so no end
///   record a reader could use, anywhere in the file, may state more. A
///   32-bit record stating 0xFFFF defers to a 64-bit record and is let
///   through, which bounds what a reader reserves at 65,535 entries.
///
/// The signature of an end record can also turn up inside an entry's data
/// or the file's comment; there it is held to the limit only if it reads
/// as a record a reader could use.
pub fn too_many_entries(bytes: &[u8], max: usize) -> bool {
    let limit = u64::try_from(max).unwrap_or(u64::MAX);
    let mut entries = 0_usize;
    for (at, w) in bytes.windows(4).enumerate() {
        let stated = match w {
            b"PK\x01\x02" => {
                entries = entries.saturating_add(1);
                None
            }
            b"PK\x05\x06" => stated_entries(bytes, at, &END_32).filter(|&n| n != 0xFFFF),
            b"PK\x06\x06" => stated_entries(bytes, at, &END_64),
            _ => None,
        };
        if entries > max || stated.is_some_and(|n| n > limit) {
            return true;
        }
    }
    false
}
