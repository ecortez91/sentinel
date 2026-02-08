//! Shared rendering helpers: text truncation, status badges, scrollbar, centered rect.

use ratatui::{
    layout::{Margin, Rect},
    style::Style,
    text::Span,
    widgets::{Scrollbar, ScrollbarOrientation, ScrollbarState},
    Frame,
};

use crate::ui::theme::Theme;

/// Truncate a string to `max_len` characters, appending "..." if truncated.
pub fn truncate_str(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}...", &s[..max_len.saturating_sub(3)])
    }
}

/// Format bytes-per-tick as a human-readable rate.
/// Since sysinfo `received()`/`transmitted()` return bytes since last refresh (~1s), treat as /s.
pub fn format_rate(bytes_per_sec: u64) -> String {
    if bytes_per_sec >= 1_073_741_824 {
        format!("{:.1} GiB", bytes_per_sec as f64 / 1_073_741_824.0)
    } else if bytes_per_sec >= 1_048_576 {
        format!("{:.1} MiB", bytes_per_sec as f64 / 1_048_576.0)
    } else if bytes_per_sec >= 1024 {
        format!("{:.1} KiB", bytes_per_sec as f64 / 1024.0)
    } else {
        format!("{} B", bytes_per_sec)
    }
}

/// Render a styled process status badge.
pub fn status_badge<'a>(status: &crate::models::ProcessStatus, t: &Theme) -> Span<'a> {
    use crate::models::ProcessStatus;
    match status {
        ProcessStatus::Running => Span::styled(
            t!("status.running").to_string(),
            Style::default().fg(t.success),
        ),
        ProcessStatus::Sleeping => Span::styled(
            t!("status.sleeping").to_string(),
            Style::default().fg(t.text_dim),
        ),
        ProcessStatus::Stopped => Span::styled(
            t!("status.stopped").to_string(),
            Style::default().fg(t.warning),
        ),
        ProcessStatus::Zombie => Span::styled(
            t!("status.zombie").to_string(),
            Style::default().fg(t.danger),
        ),
        ProcessStatus::Dead => {
            Span::styled(t!("status.dead").to_string(), Style::default().fg(t.danger))
        }
        ProcessStatus::Unknown => Span::styled(
            t!("status.unknown").to_string(),
            Style::default().fg(t.text_muted),
        ),
    }
}

/// Render a vertical scrollbar on the right side of `area`.
///
/// Only renders if `total > visible_height`.
pub fn render_scrollbar(frame: &mut Frame, area: Rect, total: usize, position: usize) {
    let visible_height = area.height as usize;
    if total <= visible_height {
        return;
    }
    let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
        .begin_symbol(Some("▲"))
        .end_symbol(Some("▼"));
    let mut scrollbar_state = ScrollbarState::new(total).position(position);
    frame.render_stateful_widget(
        scrollbar,
        area.inner(Margin {
            vertical: 0,
            horizontal: 0,
        }),
        &mut scrollbar_state,
    );
}

/// Render a vertical scrollbar inside a bordered area (1px vertical margin).
pub fn render_scrollbar_bordered(frame: &mut Frame, area: Rect, total: usize, position: usize) {
    let visible_height = area.height.saturating_sub(2) as usize;
    if total <= visible_height {
        return;
    }
    let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
        .begin_symbol(Some("▲"))
        .end_symbol(Some("▼"));
    let mut scrollbar_state = ScrollbarState::new(total).position(position);
    frame.render_stateful_widget(
        scrollbar,
        area.inner(Margin {
            vertical: 1,
            horizontal: 0,
        }),
        &mut scrollbar_state,
    );
}

/// Return a `Rect` centered within `area` with the given dimensions.
pub fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    Rect {
        x,
        y,
        width: width.min(area.width),
        height: height.min(area.height),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── truncate_str (renderer variant: uses saturating_sub) ──────

    #[test]
    fn truncate_str_short_unchanged() {
        assert_eq!(truncate_str("hello", 10), "hello");
    }

    #[test]
    fn truncate_str_exact_fit() {
        assert_eq!(truncate_str("hello", 5), "hello");
    }

    #[test]
    fn truncate_str_adds_ellipsis() {
        assert_eq!(truncate_str("hello world", 8), "hello...");
    }

    #[test]
    fn truncate_str_max_3() {
        // saturating_sub(3) = 0, so "..." appended to empty prefix
        assert_eq!(truncate_str("abcdef", 3), "...");
    }

    #[test]
    fn truncate_str_max_2() {
        // saturating_sub(3) = 0 when max_len < 3, so still "..."
        // This documents the behavior difference from utils::truncate_str
        assert_eq!(truncate_str("abcdef", 2), "...");
    }

    #[test]
    fn truncate_str_max_0() {
        assert_eq!(truncate_str("abcdef", 0), "...");
    }

    #[test]
    fn truncate_str_empty_input() {
        assert_eq!(truncate_str("", 5), "");
        assert_eq!(truncate_str("", 0), "");
    }

    // ── format_rate ───────────────────────────────────────────────

    #[test]
    fn format_rate_zero() {
        assert_eq!(format_rate(0), "0 B");
    }

    #[test]
    fn format_rate_bytes() {
        assert_eq!(format_rate(512), "512 B");
        assert_eq!(format_rate(1023), "1023 B");
    }

    #[test]
    fn format_rate_kib() {
        assert_eq!(format_rate(1024), "1.0 KiB");
        assert_eq!(format_rate(10 * 1024), "10.0 KiB");
    }

    #[test]
    fn format_rate_mib() {
        assert_eq!(format_rate(1_048_576), "1.0 MiB");
        assert_eq!(format_rate(100 * 1_048_576), "100.0 MiB");
    }

    #[test]
    fn format_rate_gib() {
        assert_eq!(format_rate(1_073_741_824), "1.0 GiB");
    }

    // ── centered_rect ─────────────────────────────────────────────

    #[test]
    fn centered_rect_normal() {
        let area = Rect::new(0, 0, 100, 50);
        let r = centered_rect(40, 20, area);
        assert_eq!(r.x, 30);
        assert_eq!(r.y, 15);
        assert_eq!(r.width, 40);
        assert_eq!(r.height, 20);
    }

    #[test]
    fn centered_rect_larger_than_area() {
        let area = Rect::new(0, 0, 20, 10);
        let r = centered_rect(40, 30, area);
        // Width/height clamped to area
        assert_eq!(r.width, 20);
        assert_eq!(r.height, 10);
        assert_eq!(r.x, 0);
        assert_eq!(r.y, 0);
    }

    #[test]
    fn centered_rect_with_offset_area() {
        let area = Rect::new(10, 5, 80, 40);
        let r = centered_rect(20, 10, area);
        assert_eq!(r.x, 40); // 10 + (80 - 20) / 2
        assert_eq!(r.y, 20); // 5 + (40 - 10) / 2
        assert_eq!(r.width, 20);
        assert_eq!(r.height, 10);
    }

    #[test]
    fn centered_rect_exact_fit() {
        let area = Rect::new(0, 0, 50, 25);
        let r = centered_rect(50, 25, area);
        assert_eq!(r.x, 0);
        assert_eq!(r.y, 0);
        assert_eq!(r.width, 50);
        assert_eq!(r.height, 25);
    }
}
