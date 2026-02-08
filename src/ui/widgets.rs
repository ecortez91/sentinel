use ratatui::{buffer::Buffer, layout::Rect, style::Style, widgets::Widget};

use super::theme::Theme;

/// A beautiful sparkline-style bar gauge with gradient coloring.
pub struct GradientGauge<'a> {
    pub percent: f32,
    pub label: String,
    pub show_value: bool,
    pub theme: &'a Theme,
}

impl<'a> GradientGauge<'a> {
    pub fn new(percent: f32, label: &str, theme: &'a Theme) -> Self {
        Self {
            percent: percent.clamp(0.0, 100.0),
            label: label.to_string(),
            show_value: true,
            theme,
        }
    }
}

impl Widget for GradientGauge<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.width < 4 || area.height < 1 {
            return;
        }

        let label_width = self.label.len() as u16 + 1;
        let value_width = if self.show_value { 7 } else { 0 };
        let bar_width = area.width.saturating_sub(label_width + value_width);

        if bar_width < 2 {
            return;
        }

        // Render label
        let label_style = Style::default().fg(self.theme.text_dim);
        buf.set_string(area.x, area.y, &self.label, label_style);

        // Render bar
        let bar_x = area.x + label_width;
        let filled = ((self.percent / 100.0) * bar_width as f32) as u16;
        let color = self.theme.usage_color(self.percent);

        // Block characters for smooth bar: ░ ▒ ▓ █
        for i in 0..bar_width {
            let (ch, style) = if i < filled {
                ('█', Style::default().fg(color))
            } else if i == filled {
                // Partial fill for smooth transition
                let frac = (self.percent / 100.0) * bar_width as f32 - filled as f32;
                let partial = if frac > 0.75 {
                    '▓'
                } else if frac > 0.5 {
                    '▒'
                } else if frac > 0.25 {
                    '░'
                } else {
                    '░'
                };
                (partial, Style::default().fg(color))
            } else {
                ('░', Style::default().fg(self.theme.gauge_bg))
            };
            buf.set_string(bar_x + i, area.y, ch.to_string(), style);
        }

        // Render percentage value
        if self.show_value {
            let val_str = format!("{:>5.1}%", self.percent);
            let val_style = Style::default().fg(color);
            buf.set_string(bar_x + bar_width + 1, area.y, &val_str, val_style);
        }
    }
}

/// A mini sparkline showing CPU per-core usage as a bar chart.
pub struct CpuMiniChart<'a> {
    pub usages: &'a [f32],
    pub theme: &'a Theme,
}

impl<'a> CpuMiniChart<'a> {
    pub fn new(usages: &'a [f32], theme: &'a Theme) -> Self {
        Self { usages, theme }
    }
}

impl Widget for CpuMiniChart<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.height < 1 || area.width < 1 {
            return;
        }

        // Each core gets vertical bar characters: ▁▂▃▄▅▆▇█
        let bar_chars = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];

        let cores_to_show = (area.width as usize).min(self.usages.len());

        for (i, &usage) in self.usages.iter().take(cores_to_show).enumerate() {
            let idx = ((usage / 100.0) * 7.0).round() as usize;
            let idx = idx.min(7);
            let ch = bar_chars[idx];
            let color = self.theme.usage_color(usage);
            buf.set_string(
                area.x + i as u16,
                area.y,
                ch.to_string(),
                Style::default().fg(color),
            );
        }
    }
}
