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

/// Decodes ids into their text form. Fails on an empty item, as the
/// original does on two spaces in a row.
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
            result.extend((a..=b).map(|i| i.to_string()));
        } else {
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
}
