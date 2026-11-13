//! Which counts are worth saying as they change.

/// Whether a count is worth announcing: every count up to 10, then every
/// ten up to 100, then every hundred.
pub fn must_be_said(nb: i64) -> bool {
    if nb <= 10 {
        true
    } else if nb <= 100 {
        nb % 10 == 0
    } else {
        nb % 100 == 0
    }
}

#[cfg(test)]
mod tests {
    use super::must_be_said;

    #[test]
    fn small_counts_then_round_counts_are_said() {
        let said: Vec<i64> = (-2..=1000).filter(|&n| must_be_said(n)).collect();
        let mut expected: Vec<i64> = (-2..=10).collect();
        expected.extend((20..=100).step_by(10));
        expected.extend((200..=1000).step_by(100));
        assert_eq!(said, expected);
    }
}
