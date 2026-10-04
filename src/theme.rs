use eframe::egui;
use std::fs;
use std::path::PathBuf;
use std::sync::mpsc::Sender;
use std::thread;
use std::time::Duration;

use crate::models::BackgroundMessage;

#[derive(Clone, Debug)]
pub struct ThemeColors {
    pub mode: String,
    pub background: egui::Color32,
    pub dark_background: egui::Color32,
    pub lighter_background: egui::Color32,
    pub foreground: egui::Color32,
    pub dark_foreground: egui::Color32,
    pub accent: egui::Color32,
    pub roku_purple: egui::Color32,
}

impl Default for ThemeColors {
    fn default() -> Self {
        Self {
            mode: "dark".to_string(),
            background: egui::Color32::from_rgb(26, 26, 32),
            dark_background: egui::Color32::from_rgb(34, 34, 42),
            lighter_background: egui::Color32::from_rgb(45, 45, 56),
            foreground: egui::Color32::from_rgb(240, 240, 245),
            dark_foreground: egui::Color32::from_rgb(160, 160, 175),
            accent: egui::Color32::from_rgb(102, 45, 145),
            roku_purple: egui::Color32::from_rgb(102, 45, 145),
        }
    }
}

pub fn parse_hex_color(hex: &str) -> Option<egui::Color32> {
    let clean = hex.trim().trim_matches('"').trim_start_matches('#');
    if clean.len() == 6 {
        let r = u8::from_str_radix(&clean[0..2], 16).ok()?;
        let g = u8::from_str_radix(&clean[2..4], 16).ok()?;
        let b = u8::from_str_radix(&clean[4..6], 16).ok()?;
        return Some(egui::Color32::from_rgb(r, g, b));
    } else if clean.len() == 8 {
        let r = u8::from_str_radix(&clean[0..2], 16).ok()?;
        let g = u8::from_str_radix(&clean[2..4], 16).ok()?;
        let b = u8::from_str_radix(&clean[4..6], 16).ok()?;
        let a = u8::from_str_radix(&clean[6..8], 16).ok()?;
        return Some(egui::Color32::from_rgba_unmultiplied(r, g, b, a));
    }
    None
}

pub fn load_omarchy_theme() -> ThemeColors {
    let mut theme = ThemeColors::default();
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    let candidates = vec![
        PathBuf::from(&home).join(".local/state/omarchy/current/theme/colors.toml"),
        PathBuf::from(&home).join(".config/omarchy/themes/current/colors.toml"),
    ];

    for path in candidates {
        if let Ok(content) = fs::read_to_string(&path) {
            for line in content.lines() {
                let trimmed = line.trim();
                if trimmed.starts_with('#') || !trimmed.contains('=') {
                    continue;
                }
                let mut parts = trimmed.splitn(2, '=');
                let key = parts.next().unwrap_or("").trim();
                let val = parts.next().unwrap_or("").trim().trim_matches('"');

                match key {
                    "mode" => theme.mode = val.to_string(),
                    "background" => {
                        if let Some(c) = parse_hex_color(val) { theme.background = c; }
                    }
                    "dark_background" => {
                        if let Some(c) = parse_hex_color(val) { theme.dark_background = c; }
                    }
                    "lighter_background" => {
                        if let Some(c) = parse_hex_color(val) { theme.lighter_background = c; }
                    }
                    "foreground" => {
                        if let Some(c) = parse_hex_color(val) { theme.foreground = c; }
                    }
                    "dark_foreground" => {
                        if let Some(c) = parse_hex_color(val) { theme.dark_foreground = c; }
                    }
                    "accent" => {
                        if let Some(c) = parse_hex_color(val) { theme.accent = c; }
                    }
                    _ => {}
                }
            }
            break;
        }
    }

    theme
}

pub fn apply_theme(ctx: &egui::Context, theme: &ThemeColors) {
    let is_dark = theme.mode == "dark";
    let mut visuals = if is_dark {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };

    visuals.panel_fill = theme.background;
    visuals.window_fill = theme.background;
    visuals.override_text_color = Some(theme.foreground);

    visuals.window_rounding = egui::Rounding::same(12.0);
    visuals.widgets.noninteractive.rounding = egui::Rounding::same(8.0);
    visuals.widgets.inactive.rounding = egui::Rounding::same(8.0);
    visuals.widgets.hovered.rounding = egui::Rounding::same(8.0);
    visuals.widgets.active.rounding = egui::Rounding::same(8.0);
    visuals.widgets.open.rounding = egui::Rounding::same(8.0);

    visuals.widgets.noninteractive.fg_stroke = egui::Stroke::new(1.0f32, theme.foreground);
    visuals.widgets.noninteractive.bg_fill = theme.background;

    visuals.widgets.inactive.bg_fill = theme.dark_background;
    visuals.widgets.inactive.fg_stroke = egui::Stroke::new(1.0f32, theme.foreground);
    visuals.widgets.inactive.bg_stroke = egui::Stroke::new(1.0f32, theme.lighter_background);

    visuals.widgets.hovered.bg_fill = theme.lighter_background;
    visuals.widgets.hovered.fg_stroke = egui::Stroke::new(1.0f32, theme.foreground);
    visuals.widgets.hovered.bg_stroke = egui::Stroke::new(1.5f32, theme.accent);

    visuals.widgets.active.bg_fill = theme.accent;
    visuals.widgets.active.fg_stroke = egui::Stroke::new(1.0f32, egui::Color32::WHITE);

    visuals.widgets.open.bg_fill = theme.lighter_background;
    visuals.widgets.open.fg_stroke = egui::Stroke::new(1.0f32, theme.foreground);

    visuals.selection.bg_fill = theme.accent;
    visuals.selection.stroke = egui::Stroke::new(1.0f32, egui::Color32::WHITE);

    ctx.set_visuals(visuals);
}

pub fn start_theme_watcher(
    tx: Sender<BackgroundMessage>,
    ctx: egui::Context,
    is_active: std::sync::Arc<std::sync::atomic::AtomicBool>,
) {
    thread::spawn(move || {
        let mut current = load_omarchy_theme();
        loop {
            thread::sleep(Duration::from_secs(3));
            if !is_active.load(std::sync::atomic::Ordering::Relaxed) {
                continue;
            }
            let next = load_omarchy_theme();
            if next.background != current.background || next.accent != current.accent {
                current = next.clone();
                let _ = tx.send(BackgroundMessage::ThemeUpdated(next));
                ctx.request_repaint();
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_hex_color() {
        assert_eq!(parse_hex_color("#316ac5"), Some(egui::Color32::from_rgb(0x31, 0x6a, 0xc5)));
        assert_eq!(parse_hex_color("316ac5"), Some(egui::Color32::from_rgb(0x31, 0x6a, 0xc5)));
        assert_eq!(parse_hex_color("#316ac5ff"), Some(egui::Color32::from_rgba_unmultiplied(0x31, 0x6a, 0xc5, 0xff)));
        assert_eq!(parse_hex_color("invalid"), None);
        assert_eq!(parse_hex_color(""), None);
    }
}
