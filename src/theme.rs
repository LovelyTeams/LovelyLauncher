//! Theme tokens, generated from a user-chosen accent color and optionally
//! overridden token-by-token from `config.json`.

use std::collections::BTreeMap;

use gpui::{App, Global, Hsla, Rgba, WindowAppearance, hsla, rgb};

use crate::config::{DEFAULT_ACCENT, Settings, ThemeMode};

#[derive(Clone, Debug)]
pub struct Theme {
    pub dark: bool,
    pub window_bg: Hsla,
    pub panel: Hsla,
    pub panel_hover: Hsla,
    pub surface: Hsla,
    pub border: Hsla,
    pub border_strong: Hsla,
    pub text: Hsla,
    pub text_muted: Hsla,
    pub text_faint: Hsla,
    pub accent: Hsla,
    pub accent_soft: Hsla,
    pub accent_text: Hsla,
    pub input_bg: Hsla,
    pub selection: Hsla,
    pub kbd_bg: Hsla,
    pub kbd_border: Hsla,
    pub row_hover: Hsla,
    pub row_active: Hsla,
    pub overlay: Hsla,
    pub danger: Hsla,
    pub banner_bg: Hsla,
    pub banner_text: Hsla,
    pub folder: Hsla,
}

impl Global for Theme {}

/// Accent presets offered in settings: (name, hex).
pub const ACCENT_PRESETS: &[(&str, &str)] = &[
    ("草莓粉", DEFAULT_ACCENT),
    ("樱桃红", "#E5484D"),
    ("蜜橘橙", "#F27A1A"),
    ("柠檬黄", "#E0A400"),
    ("薄荷绿", "#22A06B"),
    ("湖水青", "#0FA5A5"),
    ("天空蓝", "#2F80ED"),
    ("薰衣紫", "#8B5CF6"),
    ("石墨灰", "#5B6577"),
];

/// Parse "#RGB", "#RRGGBB" or "#RRGGBBAA" (the '#' is optional).
pub fn parse_hex(s: &str) -> Option<Hsla> {
    let hex = s.trim().trim_start_matches('#');
    if !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let value = match hex.len() {
        3 => {
            let v = u32::from_str_radix(hex, 16).ok()?;
            let (r, g, b) = ((v >> 8) & 0xf, (v >> 4) & 0xf, v & 0xf);
            ((r * 17) << 24) | ((g * 17) << 16) | ((b * 17) << 8) | 0xff
        }
        6 => (u32::from_str_radix(hex, 16).ok()? << 8) | 0xff,
        8 => u32::from_str_radix(hex, 16).ok()?,
        _ => return None,
    };
    Some(Rgba::from(gpui::rgba(value)).into())
}

impl Theme {
    pub fn from_settings(settings: &Settings, appearance: WindowAppearance) -> Self {
        let dark = match settings.theme {
            ThemeMode::Light => false,
            ThemeMode::Dark => true,
            ThemeMode::System => matches!(
                appearance,
                WindowAppearance::Dark | WindowAppearance::VibrantDark
            ),
        };
        let accent = parse_hex(&settings.accent)
            .or_else(|| parse_hex(DEFAULT_ACCENT))
            .unwrap();
        let opacity = if settings.blur_background {
            (settings.opacity.clamp(40, 100) as f32) / 100.
        } else {
            1.
        };
        let mut theme = Self::generate(accent, dark, settings.tint_surfaces, opacity);
        let overrides = if dark {
            &settings.theme_overrides.dark
        } else {
            &settings.theme_overrides.light
        };
        theme.apply_overrides(overrides);
        theme
    }

    /// Derive every token from the accent's hue.
    pub fn generate(accent: Hsla, dark: bool, tint: bool, opacity: f32) -> Self {
        let h = accent.h;
        // Saturation of neutral surfaces: a hint of the accent hue, or none.
        let ns = |s: f32| if tint { s } else { 0. };
        if dark {
            let accent = hsla(h, accent.s, accent.l.max(0.68), 1.);
            Self {
                dark,
                window_bg: hsla(h, ns(0.10), 0.115, opacity.max(0.6)),
                panel: hsla(h, ns(0.08), 0.165, 0.93),
                panel_hover: hsla(h, ns(0.08), 0.195, 1.),
                surface: hsla(h, ns(0.08), 0.175, 1.),
                border: hsla(h, ns(0.08), 0.24, 0.8),
                border_strong: hsla(h, ns(0.08), 0.29, 1.),
                text: hsla(h, ns(0.15), 0.94, 1.),
                text_muted: hsla(h, ns(0.08), 0.70, 1.),
                text_faint: hsla(h, ns(0.06), 0.48, 1.),
                accent,
                accent_soft: hsla(h, accent.s * 0.35, 0.22, 1.),
                accent_text: hsla(h, 0.5, 0.12, 1.),
                input_bg: hsla(h, ns(0.08), 0.165, 1.),
                selection: hsla(h, accent.s, accent.l, 0.32),
                kbd_bg: hsla(h, ns(0.08), 0.21, 1.),
                kbd_border: hsla(h, ns(0.08), 0.27, 1.),
                row_hover: hsla(h, ns(0.10), 0.23, 0.6),
                row_active: hsla(h, accent.s * 0.35, 0.22, 1.),
                overlay: hsla(0., 0., 0., 0.45),
                danger: rgb(0xff6b7d).into(),
                banner_bg: hsla(h, accent.s * 0.3, 0.19, 1.),
                banner_text: hsla(h, accent.s.min(0.9), 0.80, 1.),
                folder: rgb(0xf2b43c).into(),
            }
        } else {
            Self {
                dark,
                window_bg: hsla(h, ns(0.40), 0.975, opacity),
                panel: hsla(0., 0., 1., 0.93),
                panel_hover: hsla(0., 0., 1., 1.),
                surface: hsla(h, ns(0.60), 0.99, 1.),
                border: hsla(h, ns(0.30), 0.90, 0.8),
                border_strong: hsla(h, ns(0.25), 0.85, 1.),
                text: hsla(h, ns(0.15), 0.16, 1.),
                text_muted: hsla(h, ns(0.06), 0.44, 1.),
                text_faint: hsla(h, ns(0.06), 0.63, 1.),
                accent,
                accent_soft: hsla(h, accent.s * 0.9, 0.95, 1.),
                accent_text: hsla(0., 0., 1., 1.),
                input_bg: hsla(0., 0., 1., 1.),
                selection: hsla(h, accent.s, accent.l, 0.25),
                kbd_bg: hsla(h, ns(0.30), 0.955, 1.),
                kbd_border: hsla(h, ns(0.22), 0.89, 1.),
                row_hover: hsla(h, ns(0.35), 0.95, 0.8),
                row_active: hsla(h, accent.s * 0.9, 0.95, 1.),
                overlay: hsla(h, ns(0.15), 0.16, 0.25),
                danger: rgb(0xdc3d4f).into(),
                banner_bg: hsla(h, accent.s * 0.85, 0.955, 1.),
                banner_text: hsla(h, accent.s * 0.6, 0.45, 1.),
                folder: rgb(0xf5b83d).into(),
            }
        }
    }

    fn apply_overrides(&mut self, overrides: &BTreeMap<String, String>) {
        for (name, value) in overrides {
            let Some(color) = parse_hex(value) else { continue };
            let slot = match name.as_str() {
                "window_bg" => &mut self.window_bg,
                "panel" => &mut self.panel,
                "panel_hover" => &mut self.panel_hover,
                "surface" => &mut self.surface,
                "border" => &mut self.border,
                "border_strong" => &mut self.border_strong,
                "text" => &mut self.text,
                "text_muted" => &mut self.text_muted,
                "text_faint" => &mut self.text_faint,
                "accent" => &mut self.accent,
                "accent_soft" => &mut self.accent_soft,
                "accent_text" => &mut self.accent_text,
                "input_bg" => &mut self.input_bg,
                "selection" => &mut self.selection,
                "kbd_bg" => &mut self.kbd_bg,
                "kbd_border" => &mut self.kbd_border,
                "row_hover" => &mut self.row_hover,
                "row_active" => &mut self.row_active,
                "overlay" => &mut self.overlay,
                "danger" => &mut self.danger,
                "banner_bg" => &mut self.banner_bg,
                "banner_text" => &mut self.banner_text,
                "folder" => &mut self.folder,
                _ => continue,
            };
            *slot = color;
        }
    }

    pub fn get(cx: &App) -> &Self {
        cx.global::<Theme>()
    }

    /// (foreground, background) for a named accent color of a tool tile.
    pub fn tint(&self, name: &str) -> (Hsla, Hsla) {
        let (fg, light_bg, dark_bg, dark_fg): (u32, u32, u32, Option<u32>) = match name {
            "blue" => (0x3b82f6, 0xe8f0fe, 0x1f2a44, Some(0x7fb0ff)),
            "sky" => (0x0ea5e9, 0xe0f2fe, 0x13303f, Some(0x5cc8f5)),
            "purple" => (0x8b5cf6, 0xefe9fe, 0x2c2348, Some(0xb39bff)),
            "green" => (0x22a35a, 0xe3f6ea, 0x17352a, Some(0x5fd28f)),
            "orange" => (0xf59e0b, 0xfef3dc, 0x3a2e14, None),
            "teal" => (0x14b8a6, 0xddf6f3, 0x123632, None),
            "red" => (0xef4444, 0xfde8e8, 0x3f1d1d, Some(0xff8080)),
            "slate" => (0x475569, 0xeef1f5, 0x2c3039, Some(0xcbd5e1)),
            _ => (0xee6585, 0xfde7ed, 0x48293a, Some(0xff8aa6)), // pink
        };
        if self.dark {
            (rgb(dark_fg.unwrap_or(fg)).into(), rgb(dark_bg).into())
        } else {
            (rgb(fg).into(), rgb(light_bg).into())
        }
    }
}

pub const TINTS: &[&str] = &[
    "pink", "red", "orange", "green", "teal", "sky", "blue", "purple", "slate",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex() {
        assert!(parse_hex("#EE6585").is_some());
        assert!(parse_hex("ee6585").is_some());
        assert!(parse_hex("#fff").is_some());
        assert!(parse_hex("#ee658580").is_some());
        assert!(parse_hex("#ee65").is_none());
        assert!(parse_hex("#gg0000").is_none());
    }
}
