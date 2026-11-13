//! The text conventions shared by the definition files: `rules.txt`,
//! `ai.txt`, `style.txt` and the maps.

/// Prepares a definition file for line by line reading.
///
/// Removes `;` comments, then lines left blank, then joins each line ending
/// with a backslash to the next one, with a space in place of the backslash.
/// A comment line between two joined lines does not break the join.
///
/// Expects `\n` line ends. Whoever reads the file converts them first.
pub fn preprocess(s: &str) -> String {
    let mut lines: Vec<&str> = s
        .split('\n')
        .map(|line| line.split_once(';').map_or(line, |(code, _)| code))
        .collect();

    // A blank line goes together with its line end, so the last piece of the
    // text, which has no line end, stays.
    let last = lines.pop().unwrap_or_default();
    lines.retain(|line| !line.chars().all(|c| c == ' ' || c == '\t'));
    lines.push(last);

    let mut out = String::with_capacity(s.len());
    let last_index = lines.len() - 1;
    for (i, line) in lines.iter().enumerate() {
        if i == last_index {
            out.push_str(line);
            break;
        }
        match line.trim_end_matches([' ', '\t']).strip_suffix('\\') {
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
    out
}

#[cfg(test)]
mod tests {
    use super::preprocess;

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
    fn a_backslash_inside_a_line_stays() {
        assert_eq!(preprocess("a\\b\nc"), "a\\b\nc");
    }
}
