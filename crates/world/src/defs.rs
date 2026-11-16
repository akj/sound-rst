//! The text conventions shared by the definition files: `rules.txt`,
//! `ai.txt`, `style.txt` and the maps, and the limits on the size of the
//! rules, scripts and maps, which may come from a server.
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::string_slice,
    clippy::arithmetic_side_effects
)]

/// The largest rules, scripts or map text, all layers together, in bytes.
/// The original's largest is a map of 15 KB.
pub const MAX_TEXT_BYTES: usize = 256 * 1024;

/// The most lines a rules, scripts or map text may have, all layers
/// together. The original's longest, `rules.txt` with a mod's, has 861.
pub const MAX_TEXT_LINES: usize = 10_000;

/// The longest line, once lines ending with a backslash are joined, in
/// bytes. The original's longest is a map line of 667 bytes.
pub const MAX_LINE_BYTES: usize = 4096;

/// The largest number, either way from zero, a rules or map text may hold
/// once read, decimals multiplied by [`crate::nofloat::PRECISION`]. The
/// original's largest is `inf`, read as 2147483000. The simulation adds and
/// multiplies these numbers in 64 bits, which hold the product of any two.
pub const MAX_NUMBER: i64 = i32::MAX as i64;

/// `n`, if it is within [`MAX_NUMBER`].
pub fn bounded(n: i64) -> Option<i64> {
    (n.unsigned_abs() <= MAX_NUMBER.unsigned_abs()).then_some(n)
}

/// Refuses a text of more than [`MAX_TEXT_BYTES`] bytes.
pub fn check_size(bytes: usize) -> Result<(), String> {
    if bytes > MAX_TEXT_BYTES {
        return Err(format!("{bytes} bytes, more than {MAX_TEXT_BYTES}"));
    }
    Ok(())
}

/// [`preprocess`], for a text that must stay within [`MAX_TEXT_BYTES`],
/// [`MAX_TEXT_LINES`] and [`MAX_LINE_BYTES`]. The error names the limit and
/// quotes the start of a line too long.
pub fn preprocess_within_limits(s: &str) -> Result<String, String> {
    check_size(s.len())?;
    let lines = s.split('\n').count();
    if lines > MAX_TEXT_LINES {
        return Err(format!("{lines} lines, more than {MAX_TEXT_LINES}"));
    }
    let out = preprocess(s);
    if let Some(line) = out.split('\n').find(|l| l.len() > MAX_LINE_BYTES) {
        let start: String = line.chars().take(40).collect();
        return Err(format!(
            "a line of {} bytes, more than {MAX_LINE_BYTES} (in \"{start}...\")",
            line.len()
        ));
    }
    Ok(out)
}

/// Prepares a definition file for line by line reading.
///
/// Removes `;` comments, then lines left blank, then joins each line ending
/// with a backslash to the next one, with a space in place of the backslash.
/// A comment line between two joined lines does not break the join.
///
/// Expects `\n` line ends. Whoever reads the file converts them first.
pub fn preprocess(s: &str) -> String {
    let (lines, (_, last)) = kept_lines(s);
    let mut out = String::with_capacity(s.len());
    for (_, line) in lines {
        match joined(line) {
            Some(joined) => {
                out.push_str(joined);
                out.push(' ');
            }
            None => {
                out.push_str(line);
                out.push('\n');
            }
        }
    }
    out.push_str(last);
    out
}

/// For each line of [`preprocess`] of `s`, the number of the line of `s`
/// it starts on, counted from 1.
pub fn source_lines(s: &str) -> Vec<usize> {
    let (lines, (last_number, _)) = kept_lines(s);
    let mut numbers = Vec::new();
    let mut start = None;
    for (n, line) in lines {
        let first = *start.get_or_insert(n);
        if joined(line).is_none() {
            numbers.push(first);
            start = None;
        }
    }
    numbers.push(start.unwrap_or(last_number));
    numbers
}

/// The lines of `s` without their comments, each with its number from 1,
/// without the blank ones; and apart, the last piece of the text, which has
/// no line end. A blank line goes together with its line end, so the last
/// piece stays.
fn kept_lines(s: &str) -> (Vec<(usize, &str)>, (usize, &str)) {
    let mut lines: Vec<(usize, &str)> = s
        .split('\n')
        .map(|line| line.split_once(';').map_or(line, |(code, _)| code))
        .enumerate()
        .map(|(i, line)| (i.saturating_add(1), line))
        .collect();
    let last = lines.pop().unwrap_or((1, ""));
    lines.retain(|(_, line)| !line.chars().all(|c| c == ' ' || c == '\t'));
    (lines, last)
}

/// A line ending with a backslash, without it: it joins the next one.
fn joined(line: &str) -> Option<&str> {
    line.trim_end_matches([' ', '\t']).strip_suffix('\\')
}

#[cfg(test)]
mod tests {
    use super::{MAX_TEXT_BYTES, MAX_TEXT_LINES, preprocess};

    #[test]
    fn a_comment_is_removed_up_to_the_line_end() {
        assert_eq!(preprocess(";aaa"), "");
        assert_eq!(preprocess("eeeee;aaa\nzzzzz"), "eeeee\nzzzzz");
    }

    #[test]
    fn a_blank_line_is_removed() {
        assert_eq!(preprocess("a\n \t\n\nb\n"), "a\nb\n");
    }

    #[test]
    fn a_line_ending_with_a_backslash_joins_the_next_one() {
        assert_eq!(preprocess("a\\\na"), "a a");
        assert_eq!(preprocess("a\\   \na"), "a a");
    }

    #[test]
    fn a_comment_line_between_joined_lines_does_not_count() {
        assert_eq!(
            preprocess("on\\\n; comment\nthe same line"),
            "on the same line"
        );
    }

    #[test]
    fn a_text_over_a_limit_is_an_error() {
        use super::preprocess_within_limits as p;
        assert_eq!(p("a\\\nb"), Ok("a b".to_owned()));
        assert_eq!(
            p(&" ".repeat(MAX_TEXT_BYTES + 1)),
            Err("262145 bytes, more than 262144".to_owned())
        );
        assert_eq!(
            p(&"\n".repeat(MAX_TEXT_LINES)),
            Err("10001 lines, more than 10000".to_owned())
        );
        // Lines joined by backslashes count as one.
        assert_eq!(
            p(&"é \\\n".repeat(2000)),
            Err("a line of 8000 bytes, more than 4096 (in \"é  é  é  é  é  é  é  é  é  é  é  é  é  é...\")".to_owned())
        );
    }

    #[test]
    fn each_line_left_knows_the_line_it_starts_on() {
        let s = "a\n; comment\n\nb \\\n; between\nc\nd";
        assert_eq!(preprocess(s), "a\nb  c\nd");
        assert_eq!(super::source_lines(s), [1, 4, 7]);
        assert_eq!(super::source_lines("a\n"), [1, 2]);
        assert_eq!(super::source_lines("a\\\n"), [1]);
    }

    #[test]
    fn a_backslash_inside_a_line_stays() {
        assert_eq!(preprocess("a\\b\nc"), "a\\b\nc");
    }
}
