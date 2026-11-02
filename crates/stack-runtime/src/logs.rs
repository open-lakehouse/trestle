//! Log lines, parsed from `docker compose logs --timestamps --no-color`.

use serde::Serialize;

/// One line a container logged.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct LogLine {
    /// The container's short name as Compose prefixes it (e.g. `web-1`).
    pub container: String,
    /// The RFC 3339 timestamp Docker recorded, when the line carried one.
    pub timestamp: Option<String>,
    /// The text.
    pub text: String,
}

/// Parse one line of `docker compose logs --timestamps --no-color`, e.g.
/// `web-1  | 2026-11-02T19:55:16.807260250Z tick 1`. A line without the `<container> | `
/// prefix is kept whole as the text, with no container.
pub fn parse_log_line(line: &str) -> LogLine {
    let Some((container, rest)) = line.split_once(" | ").or_else(|| line.split_once(" |")) else {
        return LogLine {
            container: String::new(),
            timestamp: None,
            text: line.to_owned(),
        };
    };
    let (timestamp, text) = match rest.split_once(' ') {
        Some((ts, text)) if looks_like_timestamp(ts) => (Some(ts.to_owned()), text),
        _ if looks_like_timestamp(rest) => (Some(rest.to_owned()), ""),
        _ => (None, rest),
    };
    LogLine {
        container: container.trim().to_owned(),
        timestamp,
        text: text.to_owned(),
    }
}

/// `2026-11-02T19:55:16.807260250Z`: digits, then a `T`, ending in `Z` or an offset.
fn looks_like_timestamp(s: &str) -> bool {
    s.len() >= 20 && s.as_bytes()[..4].iter().all(u8::is_ascii_digit) && s.as_bytes()[10] == b'T'
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_without_a_timestamp_keeps_its_text() {
        let line = parse_log_line("web-1  | hello there");
        assert_eq!(line.container, "web-1");
        assert_eq!(line.timestamp, None);
        assert_eq!(line.text, "hello there");
    }

    #[test]
    fn a_bare_line_has_no_container() {
        let line = parse_log_line("no prefix here");
        assert_eq!(line.container, "");
        assert_eq!(line.text, "no prefix here");
    }
}
