//! Shared utility functions used across modules.

use crate::constants::SPINNER_CHARS;

/// Truncate a string to `max_len` characters, appending "..." if truncated.
pub fn truncate_str(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else if max_len > 3 {
        format!("{}...", &s[..max_len - 3])
    } else {
        s[..max_len].to_string()
    }
}

/// Get the spinner character for the current tick.
pub fn spinner_char(tick: u64) -> &'static str {
    SPINNER_CHARS[(tick % SPINNER_CHARS.len() as u64) as usize]
}

/// Get animated loading dots for the current tick.
pub fn loading_dots(tick: u64) -> &'static str {
    match tick % 4 {
        0 => "",
        1 => ".",
        2 => "..",
        _ => "...",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── truncate_str ──────────────────────────────────────────────

    #[test]
    fn truncate_str_short_string_unchanged() {
        assert_eq!(truncate_str("hello", 10), "hello");
    }

    #[test]
    fn truncate_str_exact_length() {
        assert_eq!(truncate_str("hello", 5), "hello");
    }

    #[test]
    fn truncate_str_needs_truncation() {
        assert_eq!(truncate_str("hello world", 8), "hello...");
    }

    #[test]
    fn truncate_str_max_len_4() {
        assert_eq!(truncate_str("abcdef", 4), "a...");
    }

    #[test]
    fn truncate_str_max_len_3_or_less() {
        // When max_len <= 3, no room for "...", just hard-cut
        assert_eq!(truncate_str("abcdef", 3), "abc");
        assert_eq!(truncate_str("abcdef", 2), "ab");
        assert_eq!(truncate_str("abcdef", 1), "a");
    }

    #[test]
    fn truncate_str_max_len_zero() {
        assert_eq!(truncate_str("abcdef", 0), "");
    }

    #[test]
    fn truncate_str_empty_string() {
        assert_eq!(truncate_str("", 5), "");
        assert_eq!(truncate_str("", 0), "");
    }

    // ── spinner_char ──────────────────────────────────────────────

    #[test]
    fn spinner_char_cycles() {
        assert_eq!(spinner_char(0), "◐");
        assert_eq!(spinner_char(1), "◓");
        assert_eq!(spinner_char(2), "◑");
        assert_eq!(spinner_char(3), "◒");
        // Wraps around
        assert_eq!(spinner_char(4), "◐");
        assert_eq!(spinner_char(100), "◐"); // 100 % 4 == 0
    }

    // ── loading_dots ──────────────────────────────────────────────

    #[test]
    fn loading_dots_cycles() {
        assert_eq!(loading_dots(0), "");
        assert_eq!(loading_dots(1), ".");
        assert_eq!(loading_dots(2), "..");
        assert_eq!(loading_dots(3), "...");
        // Wraps around
        assert_eq!(loading_dots(4), "");
        assert_eq!(loading_dots(7), "...");
    }
}
