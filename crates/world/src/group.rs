//! The compact form of a list of ids in the `control` command
//! (`lib/group.py`): `1-3 7` means 1, 2, 3 and 7.

/// Encodes ids, joining runs of consecutive numbers.
pub fn encode(ids: &[i64]) -> String {
    let mut ranges: Vec<(i64, i64)> = Vec::new();
    for &i in ids {
        match ranges.last_mut() {
            Some(r) if i == r.1 + 1 => r.1 = i,
            _ => ranges.push((i, i)),
        }
    }
    ranges
        .iter()
        .map(|&(a, b)| {
            if a == b {
                a.to_string()
            } else {
                format!("{a}-{b}")
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// The most ids a list may hold: far more than a game has units, and few
/// enough that a list sent by another player cannot exhaust memory.
pub const MAX_IDS: usize = 10_000;

/// Decodes ids into their text form. Fails on an empty item, as the
/// original does on two spaces in a row, and on more than [`MAX_IDS`] ids.
pub fn decode(s: &str) -> Result<Vec<String>, String> {
    if s.is_empty() {
        return Ok(Vec::new());
    }
    let int = |x: &str| {
        crate::nofloat::py_int(x).map_err(|_| format!("invalid literal for int(): {x:?}"))
    };
    let mut result = Vec::new();
    for r in s.split(' ') {
        if r.contains('-') {
            let mut parts = r.split('-');
            let (Some(a), Some(b), None) = (parts.next(), parts.next(), parts.next()) else {
                return Err(format!("too many values to unpack: {r:?}"));
            };
            let (a, b) = (int(a)?, int(b)?);
            let len = (i128::from(b) - i128::from(a) + 1).max(0);
            if len > (MAX_IDS - result.len()) as i128 {
                return Err(format!("too many ids: {r:?}"));
            }
            result.extend((a..=b).map(|i| i.to_string()));
        } else {
            if result.len() == MAX_IDS {
                return Err(format!("too many ids: {r:?}"));
            }
            result.push(int(r)?.to_string());
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runs_are_encoded_as_ranges_and_decoded_back() {
        // The cases of the original's self test.
        let cases: [(&[i64], &str); 5] = [
            (&[0, 1, 2], "0-2"),
            (&[0, 1, 2, 3], "0-3"),
            (&[0], "0"),
            (&[1, 2, 4], "1-2 4"),
            (&[0, 1, 2, 7, 8, 10], "0-2 7-8 10"),
        ];
        for (ids, text) in cases {
            assert_eq!(encode(ids), text);
            let back: Vec<String> = ids.iter().map(i64::to_string).collect();
            assert_eq!(decode(text).unwrap(), back);
        }
        assert_eq!(encode(&[]), "");
        assert_eq!(decode("").unwrap(), Vec::<String>::new());
        assert!(decode("1  2").is_err());
    }

    #[test]
    fn a_list_of_too_many_ids_is_refused_without_being_expanded() {
        assert!(decode("1-9223372036854775807").is_err());
        assert!(decode("-9223372036854775808-9223372036854775807").is_err());
        assert_eq!(decode("1-10000").unwrap().len(), MAX_IDS);
        assert!(decode("1-10000 10001").is_err());
        assert!(decode("1-5000 6000-11000").is_err());
        // A reversed range is empty, as in the original.
        assert_eq!(decode("5-1").unwrap(), Vec::<String>::new());
    }
}
