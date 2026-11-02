//! Colors and metrics for Impulse's own chrome (titlebar, docks, panels,
//! status bar), ported from `ChromePalette` and `Metrics` in the Swift app's
//! `DesignSystem/ChromePalette.swift`. Pure color math, so every platform
//! derives the same chrome from a theme.

use crate::theme::ResolvedTheme;

/// An sRGB color with alpha, components in 0...1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rgba {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Rgba {
    pub const BLACK: Rgba = Rgba::new(0.0, 0.0, 0.0, 1.0);
    pub const WHITE: Rgba = Rgba::new(1.0, 1.0, 1.0, 1.0);

    pub const fn new(r: f32, g: f32, b: f32, a: f32) -> Self {
        Self { r, g, b, a }
    }

    /// `#rrggbb` or `#rrggbbaa`; anything else is opaque black, as in the
    /// Swift `NSColor(hex:)`.
    pub fn from_hex(hex: &str) -> Self {
        let trimmed = hex.trim_matches('#');
        let valid = (trimmed.len() == 6 || trimmed.len() == 8)
            && trimmed.chars().all(|c| c.is_ascii_hexdigit());
        let Some(value) = valid
            .then(|| u32::from_str_radix(trimmed, 16).ok())
            .flatten()
        else {
            return Self::BLACK;
        };
        let channel = |shift: u32| ((value >> shift) & 0xFF) as f32 / 255.0;
        if trimmed.len() == 8 {
            Self::new(channel(24), channel(16), channel(8), channel(0))
        } else {
            Self::new(channel(16), channel(8), channel(0), 1.0)
        }
    }

    pub fn opacity(self, alpha: f32) -> Self {
        Self { a: alpha, ..self }
    }

    /// Linear blend in sRGB; the result is opaque.
    pub fn mix(self, toward: Rgba, amount: f32) -> Self {
        let t = amount.clamp(0.0, 1.0);
        Self::new(
            self.r + (toward.r - self.r) * t,
            self.g + (toward.g - self.g) * t,
            self.b + (toward.b - self.b) * t,
            1.0,
        )
    }

    /// Black or white, whichever reads better on `self` (WCAG luminance).
    pub fn readable_text(self) -> Self {
        fn lin(v: f32) -> f32 {
            if v <= 0.03928 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        }
        let l = 0.2126 * lin(self.r) + 0.7152 * lin(self.g) + 0.0722 * lin(self.b);
        if l > 0.4 {
            Self::new(0.08, 0.08, 0.1, 1.0)
        } else {
            Self::WHITE
        }
    }
}

/// The chrome's colors, derived from the theme's seeds:
///
/// - chrome surfaces sit a step darker (dark themes) or lighter (light themes)
///   than the editor/terminal background, so content leads and chrome recedes;
/// - interactive states are tints of the foreground, not system materials;
/// - status colors come from the theme's ANSI-ish palette, separate from the
///   accent.
#[derive(Clone, Debug, PartialEq)]
pub struct ChromePalette {
    // Surfaces
    pub window: Rgba,
    pub chrome: Rgba,
    pub panel: Rgba,
    pub content: Rgba,
    pub raised: Rgba,
    pub overlay: Rgba,
    pub hairline: Rgba,
    pub hairline_strong: Rgba,
    // Interaction
    pub hover: Rgba,
    pub pressed: Rgba,
    pub selection: Rgba,
    pub selection_strong: Rgba,
    pub focus_ring: Rgba,
    // Text
    pub text: Rgba,
    pub text_secondary: Rgba,
    pub text_tertiary: Rgba,
    pub text_on_accent: Rgba,
    // Accent + semantic
    pub accent: Rgba,
    pub accent_soft: Rgba,
    pub success: Rgba,
    pub warning: Rgba,
    pub danger: Rgba,
    pub info: Rgba,
    pub working: Rgba,
    pub attention: Rgba,
    // Git
    pub git_added: Rgba,
    pub git_modified: Rgba,
    pub git_deleted: Rgba,
    pub git_renamed: Rgba,
    pub git_conflict: Rgba,
    pub git_untracked: Rgba,
    pub is_light: bool,
}

impl ChromePalette {
    /// `increase_contrast` is the platform's Increase Contrast setting:
    /// firmer edges, stronger selection, and secondary text closer to the
    /// foreground.
    pub fn new(theme: &ResolvedTheme, increase_contrast: bool) -> Self {
        let bg = Rgba::from_hex(&theme.bg);
        let fg = Rgba::from_hex(&theme.fg);
        let is_light = theme.is_light;
        let boost = if increase_contrast { 1.0 } else { 0.0 };
        let light_or = |light: f32, dark: f32| if is_light { light } else { dark };

        let chrome = bg.mix(Rgba::BLACK, light_or(0.035, 0.22));
        let accent = Rgba::from_hex(&theme.accent);
        let muted = Rgba::from_hex(&theme.fg_muted);
        let comment = Rgba::from_hex(&theme.fg_comment);
        let git_added = Rgba::from_hex(&theme.git_added);

        Self {
            window: chrome,
            chrome,
            panel: bg.mix(Rgba::BLACK, light_or(0.02, 0.14)),
            content: bg,
            raised: bg.mix(fg, light_or(0.06, 0.07)),
            overlay: bg.mix(fg, light_or(0.02, 0.05)),
            hairline: bg.mix(fg, 0.12 + 0.2 * boost),
            hairline_strong: bg.mix(fg, 0.2 + 0.25 * boost),
            hover: fg.opacity(light_or(0.06, 0.07) + 0.06 * boost),
            pressed: fg.opacity(light_or(0.1, 0.12) + 0.08 * boost),
            selection: accent.opacity(light_or(0.14, 0.18) + 0.14 * boost),
            selection_strong: accent.opacity(light_or(0.24, 0.3) + 0.15 * boost),
            focus_ring: accent.opacity(0.75 + 0.25 * boost),
            text: fg,
            text_secondary: if increase_contrast {
                muted.mix(fg, 0.5)
            } else {
                muted
            },
            text_tertiary: if increase_contrast { muted } else { comment },
            text_on_accent: accent.readable_text(),
            accent,
            accent_soft: accent.opacity(0.16 + 0.12 * boost),
            success: Rgba::from_hex(&theme.green),
            warning: Rgba::from_hex(&theme.yellow),
            danger: Rgba::from_hex(&theme.red),
            info: Rgba::from_hex(&theme.blue),
            working: Rgba::from_hex(&theme.magenta),
            attention: Rgba::from_hex(&theme.orange),
            git_added,
            git_modified: Rgba::from_hex(&theme.git_modified),
            git_deleted: Rgba::from_hex(&theme.git_deleted),
            git_renamed: Rgba::from_hex(&theme.git_renamed),
            git_conflict: Rgba::from_hex(&theme.git_conflict),
            git_untracked: git_added.opacity(0.8),
            is_light,
        }
    }
}

/// Spacing and sizes for the chrome, in points. One place so density stays
/// even.
pub mod metrics {
    /// Height of the titlebar band (window controls + tabs + actions).
    pub const TITLEBAR_HEIGHT: f32 = 40.0;
    /// Leading inset reserved for macOS's traffic lights (not full screen).
    pub const TRAFFIC_LIGHT_INSET: f32 = 78.0;
    pub const STATUS_BAR_HEIGHT: f32 = 24.0;
    pub const PANE_HEADER_HEIGHT: f32 = 28.0;
    pub const ROW_HEIGHT: f32 = 24.0;
    pub const TAB_HEIGHT: f32 = 28.0;

    pub const RADIUS_SMALL: f32 = 4.0;
    pub const RADIUS: f32 = 6.0;
    pub const RADIUS_LARGE: f32 = 8.0;

    pub const ICON_SMALL: f32 = 12.0;
    pub const ICON: f32 = 14.0;
    pub const ICON_LARGE: f32 = 16.0;

    pub const LEFT_DOCK_DEFAULT_WIDTH: f32 = 260.0;
}

/// How tall a sidebar section that scrolls (the Workspaces list) is drawn.
///
/// `content` is its rows' whole height, padding included, and `rows` each
/// row's top and height from the section's top. A list that fits under
/// `limit` gets its whole height. One that doesn't ends halfway through the
/// lowest row it can, so it plainly goes on and is never left to scroll by
/// a few points of padding.
pub fn section_height(content: f32, rows: &[(f32, f32)], limit: f32) -> f32 {
    if content <= limit {
        return content;
    }
    rows.iter()
        .map(|(top, height)| top + height / 2.0)
        .take_while(|middle| *middle <= limit)
        .last()
        .unwrap_or(limit)
}

/// A height set by hand (the section's handle dragged), let out to the
/// whole list when that is no more than half a row taller and within
/// `max`: a list left to scroll by a few points feels broken.
pub fn snapped_section_height(content: f32, height: f32, max: f32) -> f32 {
    if content > height && content - height <= metrics::ROW_HEIGHT / 2.0 && content <= max {
        content
    } else {
        height
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_section_that_fits_gets_its_whole_height() {
        assert_eq!(section_height(246.0, &[(6.0, 26.0)], 280.0), 246.0);
        assert_eq!(section_height(280.0, &[(6.0, 26.0)], 280.0), 280.0);
    }

    #[test]
    fn an_overflowing_section_ends_halfway_through_a_row() {
        // Rows 26 tall and 1 apart below 6 of padding: 11 of them are 302.
        let rows: Vec<(f32, f32)> = (0..11).map(|i| (6.0 + 27.0 * i as f32, 26.0)).collect();
        // The tenth row (top 249) is cut at its middle rather than the
        // eleventh by 9 points.
        assert_eq!(section_height(302.0, &rows, 280.0), 262.0);
        // No row's middle under the limit: the limit.
        assert_eq!(section_height(302.0, &rows, 10.0), 10.0);
    }

    #[test]
    fn a_dragged_height_just_short_of_the_list_lets_it_out() {
        assert_eq!(snapped_section_height(300.0, 292.0, 600.0), 300.0);
        assert_eq!(snapped_section_height(300.0, 250.0, 600.0), 250.0);
        assert_eq!(snapped_section_height(300.0, 320.0, 600.0), 320.0);
        assert_eq!(snapped_section_height(300.0, 292.0, 295.0), 292.0);
    }

    #[test]
    fn parses_hex_like_nscolor() {
        assert_eq!(Rgba::from_hex("#ff0000"), Rgba::new(1.0, 0.0, 0.0, 1.0));
        assert_eq!(Rgba::from_hex("3959a640").a, 0x40 as f32 / 255.0);
        assert_eq!(Rgba::from_hex("#xyz"), Rgba::BLACK);
    }

    #[test]
    fn dark_chrome_recedes_and_light_text_reads_on_dark_accent() {
        let theme = crate::theme::builtin_theme("nord").unwrap();
        let palette = ChromePalette::new(&theme, false);
        let lum = |c: Rgba| c.r + c.g + c.b;
        assert!(lum(palette.chrome) < lum(palette.content));
        assert!(lum(palette.panel) < lum(palette.content));
        assert!(!palette.is_light);
    }

    #[test]
    fn increase_contrast_firms_hairlines() {
        let theme = crate::theme::builtin_theme("harbor").unwrap();
        let normal = ChromePalette::new(&theme, false);
        let firm = ChromePalette::new(&theme, true);
        assert_ne!(normal.hairline, firm.hairline);
        assert!(firm.selection.a > normal.selection.a);
    }
}
