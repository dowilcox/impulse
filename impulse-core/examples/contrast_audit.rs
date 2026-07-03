//! WCAG 2.1 contrast audit for every built-in theme.
//!
//! Loads each theme through the real `resolve_theme` pipeline and computes
//! exact relative-luminance contrast ratios for the color pairs that actually
//! matter for readability and accessibility. Prints a per-theme report plus a
//! ranked summary of the worst offenders.
//!
//! Run with: `cargo run -p impulse-core --example contrast_audit`
//! JSON mode:  `cargo run -p impulse-core --example contrast_audit -- --json`

use impulse_core::theme::{builtin_theme, builtin_theme_names, theme_display_name, ResolvedTheme};

// ---------------------------------------------------------------------------
// Color math (WCAG 2.1)
// ---------------------------------------------------------------------------

fn hex_to_rgba(hex: &str) -> (f64, f64, f64, f64) {
    let h = hex.trim_start_matches('#');
    let parse = |s: &str| u8::from_str_radix(s, 16).unwrap_or(0) as f64 / 255.0;
    if h.len() >= 8 {
        (
            parse(&h[0..2]),
            parse(&h[2..4]),
            parse(&h[4..6]),
            parse(&h[6..8]),
        )
    } else if h.len() >= 6 {
        (parse(&h[0..2]), parse(&h[2..4]), parse(&h[4..6]), 1.0)
    } else {
        (0.0, 0.0, 0.0, 1.0)
    }
}

/// Composite a possibly-translucent foreground hex over an opaque background.
fn composite(fg: &str, bg: &str) -> (f64, f64, f64) {
    let (fr, fg_, fb, fa) = hex_to_rgba(fg);
    let (br, bg_, bb, _) = hex_to_rgba(bg);
    (
        fr * fa + br * (1.0 - fa),
        fg_ * fa + bg_ * (1.0 - fa),
        fb * fa + bb * (1.0 - fa),
    )
}

fn linearize(c: f64) -> f64 {
    if c <= 0.03928 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

fn relative_luminance(r: f64, g: f64, b: f64) -> f64 {
    0.2126 * linearize(r) + 0.7152 * linearize(g) + 0.0722 * linearize(b)
}

fn bg_luminance(bg: &str) -> f64 {
    let (r, g, b, _) = hex_to_rgba(bg);
    relative_luminance(r, g, b)
}

// ---------------------------------------------------------------------------
// HSL helpers + hue-preserving AA nudge (suggester)
// ---------------------------------------------------------------------------

struct Hsl {
    h: f64,
    s: f64,
    l: f64,
}

fn rgb_to_hsl(r: f64, g: f64, b: f64) -> Hsl {
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) / 2.0;
    if (max - min).abs() < 1e-9 {
        return Hsl { h: 0.0, s: 0.0, l };
    }
    let d = max - min;
    let s = if l > 0.5 {
        d / (2.0 - max - min)
    } else {
        d / (max + min)
    };
    let h = if (max - r).abs() < 1e-9 {
        let mut h = (g - b) / d;
        if g < b {
            h += 6.0;
        }
        h
    } else if (max - g).abs() < 1e-9 {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    };
    Hsl { h: h * 60.0, s, l }
}

fn hsl_to_hex(hsl: &Hsl) -> String {
    let (r, g, b) = if hsl.s.abs() < 1e-9 {
        (hsl.l, hsl.l, hsl.l)
    } else {
        let q = if hsl.l < 0.5 {
            hsl.l * (1.0 + hsl.s)
        } else {
            hsl.l + hsl.s - hsl.l * hsl.s
        };
        let p = 2.0 * hsl.l - q;
        let h = hsl.h / 360.0;
        let f = |mut t: f64| -> f64 {
            if t < 0.0 {
                t += 1.0;
            }
            if t > 1.0 {
                t -= 1.0;
            }
            if t < 1.0 / 6.0 {
                p + (q - p) * 6.0 * t
            } else if t < 1.0 / 2.0 {
                q
            } else if t < 2.0 / 3.0 {
                p + (q - p) * (2.0 / 3.0 - t) * 6.0
            } else {
                p
            }
        };
        (f(h + 1.0 / 3.0), f(h), f(h - 1.0 / 3.0))
    };
    format!(
        "#{:02x}{:02x}{:02x}",
        (r * 255.0).round() as u8,
        (g * 255.0).round() as u8,
        (b * 255.0).round() as u8
    )
}

fn min_contrast(hex: &str, bgs: &[String]) -> f64 {
    bgs.iter()
        .map(|bg| contrast(hex, bg))
        .fold(f64::INFINITY, f64::min)
}

/// Minimal hue-preserving adjustment of `hex` so it reaches `target` contrast
/// against every background in `bgs`. Moves lightness toward white (dark bgs)
/// or black (light bgs); only reduces saturation if lightness alone can't get
/// there. Returns the adjusted hex (unchanged if already compliant).
fn nudge_to_aa(hex: &str, bgs: &[String], target: f64) -> String {
    if min_contrast(hex, bgs) >= target {
        return hex.to_string();
    }
    let (r, g, b, _) = hex_to_rgba(hex);
    let base = rgb_to_hsl(r, g, b);
    let avg_bg = bgs.iter().map(|b| bg_luminance(b)).sum::<f64>() / bgs.len() as f64;
    let lighten = avg_bg < 0.5;

    // Try progressively lower saturation only if pure lightness can't reach it.
    let mut s = base.s;
    for _ in 0..12 {
        let mut lo;
        let mut hi;
        if lighten {
            lo = base.l;
            hi = 1.0;
            // f(L) increasing in L on dark bg → find smallest L meeting target.
            if min_contrast(
                &hsl_to_hex(&Hsl {
                    h: base.h,
                    s,
                    l: hi,
                }),
                bgs,
            ) >= target
            {
                for _ in 0..40 {
                    let mid = (lo + hi) / 2.0;
                    if min_contrast(
                        &hsl_to_hex(&Hsl {
                            h: base.h,
                            s,
                            l: mid,
                        }),
                        bgs,
                    ) >= target
                    {
                        hi = mid;
                    } else {
                        lo = mid;
                    }
                }
                return hsl_to_hex(&Hsl {
                    h: base.h,
                    s,
                    l: hi,
                });
            }
        } else {
            lo = 0.0;
            hi = base.l;
            // f(L) decreasing in L on light bg → find largest L meeting target.
            if min_contrast(
                &hsl_to_hex(&Hsl {
                    h: base.h,
                    s,
                    l: lo,
                }),
                bgs,
            ) >= target
            {
                for _ in 0..40 {
                    let mid = (lo + hi) / 2.0;
                    if min_contrast(
                        &hsl_to_hex(&Hsl {
                            h: base.h,
                            s,
                            l: mid,
                        }),
                        bgs,
                    ) >= target
                    {
                        lo = mid;
                    } else {
                        hi = mid;
                    }
                }
                return hsl_to_hex(&Hsl {
                    h: base.h,
                    s,
                    l: lo,
                });
            }
        }
        s *= 0.8; // desaturate and retry
    }
    // Last resort: pure black or white (guaranteed max contrast on its side).
    if lighten {
        "#ffffff".to_string()
    } else {
        "#000000".to_string()
    }
}

/// WCAG contrast ratio between a (possibly translucent) fg and opaque bg.
fn contrast(fg: &str, bg: &str) -> f64 {
    let (r, g, b) = composite(fg, bg);
    let (br, bg_, bb, _) = hex_to_rgba(bg);
    let l1 = relative_luminance(r, g, b);
    let l2 = relative_luminance(br, bg_, bb);
    let (hi, lo) = if l1 >= l2 { (l1, l2) } else { (l2, l1) };
    (hi + 0.05) / (lo + 0.05)
}

// ---------------------------------------------------------------------------
// Check definitions
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq)]
enum Sev {
    /// Real readable text. WCAG AA normal-text threshold 4.5:1.
    Text,
    /// Large/secondary text or important UI glyphs. AA large-text 3.0:1.
    Large,
    /// Non-text UI component (border, separator). 3.0:1 desired, decorative.
    Ui,
}

impl Sev {
    fn threshold(self) -> f64 {
        match self {
            Sev::Text => 4.5,
            Sev::Large => 3.0,
            Sev::Ui => 3.0,
        }
    }
    fn label(self) -> &'static str {
        match self {
            Sev::Text => "text",
            Sev::Large => "large",
            Sev::Ui => "ui",
        }
    }
}

struct Check {
    name: &'static str,
    fg: String,
    bg: String,
    bg_name: &'static str,
    sev: Sev,
    ratio: f64,
}

fn check(name: &'static str, fg: &str, bg: &str, bg_name: &'static str, sev: Sev) -> Check {
    Check {
        name,
        fg: fg.to_string(),
        bg: bg.to_string(),
        bg_name,
        sev,
        ratio: contrast(fg, bg),
    }
}

/// Build the full battery of contrast checks for a resolved theme.
fn checks_for(t: &ResolvedTheme) -> Vec<Check> {
    let bg = &t.bg; // editor / terminal surface
    let bgd = &t.bg_dark; // sidebar / chrome
    let bgs = &t.bg_surface; // panels / cards / popovers
    let mut v = vec![
        // --- Core body text on each surface ---
        check("fg / bg", &t.fg, bg, "bg", Sev::Text),
        check("fg / bg_dark", &t.fg, bgd, "bg_dark", Sev::Text),
        check("fg / bg_surface", &t.fg, bgs, "bg_surface", Sev::Text),
        // --- Muted text (status bar, secondary labels) ---
        check("fg_muted / bg", &t.fg_muted, bg, "bg", Sev::Text),
        check("fg_muted / bg_dark", &t.fg_muted, bgd, "bg_dark", Sev::Text),
        check(
            "fg_muted / bg_surface",
            &t.fg_muted,
            bgs,
            "bg_surface",
            Sev::Text,
        ),
        // --- Comments: real text but conventionally dim. Hold to large-text. ---
        check("fg_comment / bg", &t.fg_comment, bg, "bg", Sev::Large),
        check(
            "fg_comment / bg_dark",
            &t.fg_comment,
            bgd,
            "bg_dark",
            Sev::Large,
        ),
        // --- Accent used as text (links, active labels, chips) ---
        check("accent / bg", &t.accent, bg, "bg", Sev::Text),
        check("accent / bg_dark", &t.accent, bgd, "bg_dark", Sev::Text),
        check(
            "accent / bg_surface",
            &t.accent,
            bgs,
            "bg_surface",
            Sev::Text,
        ),
        // --- Border visibility against adjacent surfaces ---
        check("border / bg", &t.border, bg, "bg", Sev::Ui),
        check("border / bg_dark", &t.border, bgd, "bg_dark", Sev::Ui),
    ];

    // --- Git status colors: shown as file-name text + badges in the sidebar
    //     (bg_dark) and gutter (bg). These are meaningful text. ---
    for (n, c) in [
        ("git_added", &t.git_added),
        ("git_modified", &t.git_modified),
        ("git_deleted", &t.git_deleted),
        ("git_renamed", &t.git_renamed),
        ("git_conflict", &t.git_conflict),
    ] {
        v.push(Check {
            name: leak(format!("{n} / bg_dark")),
            fg: c.clone(),
            bg: bgd.clone(),
            bg_name: "bg_dark",
            sev: Sev::Text,
            ratio: contrast(c, bgd),
        });
    }

    // --- Syntax colors on the editor bg. Code text → AA 4.5 ideal, but the
    //     industry norm treats these as acceptable at large-text 3.0. We hold
    //     them to Text and let the report show how far each lands. ---
    for (n, c) in [
        ("syn keyword", &t.syntax_keyword),
        ("syn function", &t.syntax_function),
        ("syn type", &t.syntax_type),
        ("syn string", &t.syntax_string),
        ("syn number", &t.syntax_number),
        ("syn constant", &t.syntax_constant),
        ("syn comment", &t.syntax_comment),
        ("syn operator", &t.syntax_operator),
        ("syn variable", &t.syntax_variable),
    ] {
        v.push(Check {
            name: leak(format!("{n} / bg")),
            fg: c.clone(),
            bg: bg.clone(),
            bg_name: "bg",
            sev: Sev::Text,
            ratio: contrast(c, bg),
        });
    }

    // --- Terminal ANSI palette on the terminal bg. Indices 0..8 normal,
    //     8..16 bright. Index 0 (black) and 8 (bright black) are background-ish
    //     by design; everything else is foreground text that must be legible. ---
    let labels = [
        "term black",
        "term red",
        "term green",
        "term yellow",
        "term blue",
        "term magenta",
        "term cyan",
        "term white",
        "term br.black",
        "term br.red",
        "term br.green",
        "term br.yellow",
        "term br.blue",
        "term br.magenta",
        "term br.cyan",
        "term br.white",
    ];
    for (i, c) in t.terminal_palette.iter().enumerate() {
        // Background-ish palette slots are structural, not readable text:
        // on dark themes that's black/bright-black (0/8); on light themes the
        // white/bright-white slots (7/15) are the near-bg tones instead.
        let exempt = if t.is_light {
            i == 7 || i == 15
        } else {
            i == 0 || i == 8
        };
        let sev = if exempt { Sev::Ui } else { Sev::Text };
        v.push(Check {
            name: labels[i],
            fg: c.clone(),
            bg: t.terminal_bg.clone(),
            bg_name: "term_bg",
            sev,
            ratio: contrast(c, &t.terminal_bg),
        });
    }

    v
}

// Leak a formatted string to get a 'static str for the report (audit tool only).
fn leak(s: String) -> &'static str {
    Box::leak(s.into_boxed_str())
}

// ---------------------------------------------------------------------------
// Reporting
// ---------------------------------------------------------------------------

fn main() {
    let json = std::env::args().any(|a| a == "--json");
    let suggest = std::env::args().any(|a| a == "--suggest");
    let ids = builtin_theme_names();

    if suggest {
        print_suggestions(&ids);
        return;
    }
    if json {
        print_json(&ids);
        return;
    }

    let mut all_fails: Vec<(String, &'static str, &'static str, f64, Sev)> = Vec::new();

    for id in &ids {
        let t = match builtin_theme(id) {
            Some(t) => t,
            None => continue,
        };
        let name = theme_display_name(id);
        let variant = if t.is_light { "light" } else { "dark" };
        let checks = checks_for(&t);

        let fails: Vec<&Check> = checks
            .iter()
            .filter(|c| c.ratio < c.sev.threshold())
            .collect();
        let worst = checks.iter().map(|c| c.ratio).fold(f64::INFINITY, f64::min);

        println!(
            "\n\x1b[1m{name}\x1b[0m  ({variant}, id={id})  —  {} checks, {} fail, worst {:.2}:1",
            checks.len(),
            fails.len(),
            worst
        );
        // Print every failing check, sorted worst-first.
        let mut sorted = fails.clone();
        sorted.sort_by(|a, b| a.ratio.partial_cmp(&b.ratio).unwrap());
        for c in &sorted {
            let pad = " ".repeat(22usize.saturating_sub(c.name.len()));
            println!(
                "  \x1b[31mFAIL\x1b[0m {}{}  {:>5.2}:1  (need {:.1}, {} on {})  fg={} bg={}",
                c.name,
                pad,
                c.ratio,
                c.sev.threshold(),
                c.sev.label(),
                c.bg_name,
                c.fg,
                c.bg,
            );
            all_fails.push((
                format!("{name}: {}", c.name),
                c.bg_name,
                c.sev.label(),
                c.ratio,
                c.sev,
            ));
        }
        if fails.is_empty() {
            println!("  \x1b[32mAll checks pass.\x1b[0m");
        }
    }

    // Ranked global summary
    println!("\n\n\x1b[1m===== WORST OFFENDERS (all themes) =====\x1b[0m");
    all_fails.sort_by(|a, b| a.3.partial_cmp(&b.3).unwrap());
    for (label, bgn, sev, ratio, _) in all_fails.iter().take(50) {
        println!("  {:>5.2}:1  [{:5}] {}  (on {})", ratio, sev, label, bgn);
    }

    // Per-category fail counts
    println!("\n\x1b[1m===== FAIL COUNT BY THEME =====\x1b[0m");
    for id in &ids {
        if let Some(t) = builtin_theme(id) {
            let checks = checks_for(&t);
            let n = checks
                .iter()
                .filter(|c| c.ratio < c.sev.threshold())
                .count();
            let bar = "█".repeat(n);
            println!("  {:>20}  {:>2}  {}", theme_display_name(id), n, bar);
        }
    }
    println!(
        "\nTotal failing checks across all themes: {}",
        all_fails.len()
    );
}

/// Print deterministic, hue-preserving AA suggestions for every theme, grouped
/// by source color so a color shared across roles gets one coherent fix.
fn print_suggestions(ids: &[&str]) {
    const TARGET: f64 = 4.5;
    for id in ids {
        let t = match builtin_theme(id) {
            Some(t) => t,
            None => continue,
        };
        let checks = checks_for(&t);
        // Group failing text-role checks by source color.
        // value = (set of bgs, set of role names)
        let mut groups: std::collections::BTreeMap<String, (Vec<String>, Vec<String>)> =
            std::collections::BTreeMap::new();
        for c in &checks {
            // Skip decorative/structural roles (borders, background-ish ANSI
            // slots): those are Sev::Ui and not held to readable-text AA.
            if c.sev == Sev::Ui {
                continue;
            }
            if c.ratio >= TARGET {
                continue;
            }
            let e = groups.entry(c.fg.to_lowercase()).or_default();
            if !e.0.contains(&c.bg) {
                e.0.push(c.bg.clone());
            }
            e.1.push(c.name.to_string());
        }
        if groups.is_empty() {
            continue;
        }
        println!("\n### {} ({})", theme_display_name(id), id);
        for (fg, (bgs, roles)) in &groups {
            let suggestion = nudge_to_aa(fg, bgs, TARGET);
            let achieved = min_contrast(&suggestion, bgs);
            let before = min_contrast(fg, bgs);
            println!(
                "  {fg} -> {suggestion}   ({before:.2}:1 -> {achieved:.2}:1)   roles: {}",
                roles.join(", ")
            );
        }
    }
}

fn print_json(ids: &[&str]) {
    println!("[");
    for (ti, id) in ids.iter().enumerate() {
        let t = match builtin_theme(id) {
            Some(t) => t,
            None => continue,
        };
        let checks = checks_for(&t);
        println!("  {{");
        println!("    \"id\": \"{id}\",");
        println!("    \"name\": \"{}\",", theme_display_name(id));
        println!(
            "    \"variant\": \"{}\",",
            if t.is_light { "light" } else { "dark" }
        );
        println!("    \"checks\": [");
        for (ci, c) in checks.iter().enumerate() {
            let comma = if ci + 1 < checks.len() { "," } else { "" };
            println!(
                "      {{\"pair\": \"{}\", \"fg\": \"{}\", \"bg\": \"{}\", \"bg_name\": \"{}\", \"sev\": \"{}\", \"ratio\": {:.3}, \"threshold\": {:.1}, \"pass\": {}}}{}",
                c.name, c.fg, c.bg, c.bg_name, c.sev.label(), c.ratio, c.sev.threshold(),
                c.ratio >= c.sev.threshold(), comma
            );
        }
        println!("    ]");
        let comma = if ti + 1 < ids.len() { "," } else { "" };
        println!("  }}{comma}");
    }
    println!("]");
}
