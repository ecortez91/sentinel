//! Shared utility functions used across modules.

use crate::constants::SPINNER_CHARS;
use std::io::Write;

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

/// Detect whether the terminal can render CJK (double-width) characters.
///
/// Probes by writing a known CJK character and checking how many columns
/// the cursor advanced. Returns `true` if the terminal renders it as
/// 2 columns wide (proper CJK font), `false` if 1 or 0 (missing glyphs).
///
/// This must be called BEFORE entering the alternate screen.
pub fn detect_cjk_support() -> bool {
    // Try the terminal probe first, fall back to locale check
    if let Some(result) = probe_cjk_terminal() {
        return result;
    }
    // Fallback: check locale environment variables for CJK hints
    check_locale_cjk()
}

/// Probe the terminal by writing a CJK character and measuring cursor advance.
fn probe_cjk_terminal() -> Option<bool> {
    use crossterm::{
        cursor,
        terminal::{disable_raw_mode, enable_raw_mode},
    };

    // Enter raw mode to get cursor position reports
    enable_raw_mode().ok()?;

    let result = (|| -> Option<bool> {
        let mut stdout = std::io::stdout();

        // Save cursor position, move to a known column
        write!(stdout, "\x1B[s\x1B[999D").ok()?; // save + move to column 0
        stdout.flush().ok()?;

        // Get baseline position
        let (base_col, _) = cursor::position().ok()?;

        // Write a CJK character (日 = U+65E5, should be 2 columns wide)
        write!(stdout, "\u{65E5}").ok()?;
        stdout.flush().ok()?;

        // Check how far the cursor moved
        let (new_col, _) = cursor::position().ok()?;

        // Restore cursor and clear the test character
        write!(stdout, "\x1B[u\x1B[K").ok()?; // restore + clear to end of line
        stdout.flush().ok()?;

        let advance = new_col.saturating_sub(base_col);
        Some(advance >= 2)
    })();

    let _ = disable_raw_mode();
    result
}

/// Fallback: check locale/environment for CJK indicators.
fn check_locale_cjk() -> bool {
    for var in &["LANG", "LC_ALL", "LC_CTYPE"] {
        if let Ok(val) = std::env::var(var) {
            let lower = val.to_lowercase();
            if lower.contains("ja")
                || lower.contains("zh")
                || lower.contains("ko")
                || lower.contains("cjk")
                || lower.contains("utf-8")
                    && (lower.contains("japan")
                        || lower.contains("chinese")
                        || lower.contains("korean"))
            {
                return true;
            }
        }
    }
    false
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
