use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use oxlet_core::model::{Palette, Settings, ThemeMode};
use oxlet_core::provider::ProviderId;
use rune::{Color, El, Family, Icon, Shadow, TextStyle};
use rune_kit::{Kit, PaletteSeed};
use rune_theme_opencode::{icons, provider_icons};

/// Oxlet's mark: a solid O with a shell prompt cut out of it.
pub static MARK: Icon = Icon {
    name: "oxlet-mark",
    view: 24.0,
    body: r#"<path fill="currentColor" fill-rule="evenodd" d="M12 2.4a9.6 9.6 0 1 0 0 19.2a9.6 9.6 0 0 0 0-19.2zM9.213 7.587L12.813 11.187A1.15 1.15 0 0 1 12.813 12.813L9.213 16.413A1.15 1.15 0 0 1 7.587 14.787L10.374 12L7.587 9.213A1.15 1.15 0 0 1 9.213 7.587zM13.4 14.45h3a1.15 1.15 0 0 1 0 2.3h-3a1.15 1.15 0 0 1 0-2.3z"/>"#,
};

pub fn is_dark(settings: &Settings, system_dark: bool) -> bool {
    match settings.theme {
        ThemeMode::System => system_dark,
        ThemeMode::Light => false,
        ThemeMode::Dark => true,
    }
}

fn color(hex: &str, fallback: &str) -> Color {
    parse_hex(hex)
        .or_else(|| parse_hex(fallback))
        .unwrap_or(Color::WHITE)
}

/// `#rgb` or `#rrggbb`. Rune slices the digits by byte, so anything but ASCII is refused first.
pub fn parse_hex(hex: &str) -> Option<Color> {
    let h = hex.trim();
    (h.is_ascii() && (h.len() == 7 || h.len() == 4))
        .then(|| rune::color::hex(h))
        .flatten()
}

thread_local! {
    static KIT: RefCell<Option<(String, Rc<Kit>)>> = const { RefCell::new(None) };
    static SCALE: Cell<f32> = const { Cell::new(1.0) };
    static NAMES: RefCell<HashMap<String, &'static str>> = RefCell::new(HashMap::new());
}

/// The system interface font on each platform, as a family list; Inter fills in where none is found.
const SYSTEM_UI: &str =
    "SF Pro Text, SF Pro, Segoe UI Variable Text, Segoe UI, Ubuntu, Cantarell, Noto Sans";

/// Family names live as long as the app: a style holds a `&'static str`. Each distinct name is
/// leaked once, however often the kit is rebuilt.
fn family_name(name: &str) -> &'static str {
    NAMES.with(|names| {
        *names
            .borrow_mut()
            .entry(name.to_owned())
            .or_insert_with(|| Box::leak(name.to_owned().into_boxed_str()))
    })
}

fn ui_family(setting: &str) -> Family {
    match setting.trim() {
        "" => Family::Sans,
        "system" => Family::Named(SYSTEM_UI),
        name => Family::Named(family_name(name)),
    }
}

fn code_family(setting: &str) -> Family {
    match setting.trim() {
        "" => Family::Mono,
        "system" => Family::SystemMono,
        name => Family::Named(family_name(name)),
    }
}

/// The kit for the active palette, fonts and text size, rebuilt only when one of them changes.
pub fn kit(settings: &Settings, dark: bool) -> Rc<Kit> {
    let p: &Palette = if dark {
        &settings.dark
    } else {
        &settings.light
    };
    let scale = settings.text_scale.clamp(0.8, 1.4);
    let key = format!(
        "{dark}{}{}{}{}{}{}{scale}",
        p.accent,
        p.background,
        p.foreground,
        settings.ui_font,
        settings.code_font,
        rune::theme::blends_srgb()
    );
    SCALE.with(|s| s.set(scale));
    KIT.with(|cell| {
        let mut cell = cell.borrow_mut();
        if let Some((k, kit)) = cell.as_ref()
            && *k == key
        {
            return kit.clone();
        }
        let fallback = Palette::preset(&oxlet_core::model::PRESETS[0], dark);
        let mut kit = Kit::from_palette(PaletteSeed {
            bg: color(&p.background, &fallback.background),
            fg: color(&p.foreground, &fallback.foreground),
            accent: color(&p.accent, &fallback.accent),
        });
        kit.c.bg_deep = rail_of(kit.c.bg_base, dark);
        kit.c.bg_icon_button_contrast = kit.c.bg_accent;
        kit.c.submit_fg = Color::WHITE;
        kit.c.submit_icon = Color::WHITE;
        kit.c.submit_hover = Color::WHITE.alpha(0.14);
        kit.c.border_focus = kit.c.bg_accent;
        kit.c.scrim = kit.dark_a(if dark { 0.45 } else { 0.18 });
        glaze(&mut kit, dark);
        kit.radius.sm = 6.0;
        kit.radius.md = 8.0;
        kit.radius.lg = 12.0;
        let (ui, code) = (
            ui_family(&settings.ui_font),
            code_family(&settings.code_font),
        );
        let t = &mut kit.ty;
        for style in [
            &mut t.ui,
            &mut t.ui_medium,
            &mut t.ui_compact,
            &mut t.small,
            &mut t.small_medium,
            &mut t.body,
            &mut t.body_medium,
            &mut t.text_small,
            &mut t.text_small_medium,
            &mut t.text_xsmall,
            &mut t.text_xsmall_medium,
            &mut t.mono,
        ] {
            style.family = if style.family == Family::Mono {
                code
            } else {
                ui
            };
            style.size *= scale;
            style.line_height *= scale;
        }
        let kit = Rc::new(kit);
        *cell = Some((key, kit.clone()));
        kit
    })
}

/// Gives the kit's buttons the app's control material: a translucent tint of the ink, a hairline
/// ring and a lit top edge, with no drop shadow (it would show through the tint). The accent
/// takes the same edge in place of its sheen. Text fields keep the kit's ring and drop: they
/// are wells, not glass.
fn glaze(kit: &mut Kit, dark: bool) {
    // A pale page shows white only against the tint under it, so light themes need more of it.
    let (tint, lit) = if dark { (0.06, 0.07) } else { (0.045, 0.7) };
    kit.c.bg_button_neutral = kit.alpha(kit.c.text_base, tint);
    kit.c.submit_highlight = Color::TRANSPARENT;
    let edge = |color| Shadow {
        y: 1.0,
        color,
        inset: true,
        ..Shadow::default()
    };
    let input = kit.elevation.button_neutral.clone();
    let ring = input.iter().copied().filter(|s| s.blur == 0.0 && !s.inset);
    let neutral = ring.chain([edge(kit.light(lit))]).collect();
    let accent = vec![
        Shadow {
            spread: 0.5,
            color: kit.dark_a(0.12),
            ..Shadow::default()
        },
        edge(kit.light_on(0.22, kit.c.bg_accent)),
    ];
    kit.elevation = std::mem::take(&mut kit.elevation).with_input(input);
    kit.elevation.button_neutral = neutral;
    kit.elevation.button_contrast = accent;
    // The lit edge is a zero-blur layer too, so rings keep the colours they are built with
    // rather than all taking `border_base`.
    kit.ring = None;
    // The kit holds secondary ink to 4.5:1 (muted) and 3:1 (faint) on the page. The darkest
    // surface text sits on is a selected row on the rail, so the floors hold there too.
    let floor = over_srgb(kit.c.pressed, kit.c.bg_deep);
    let held = Kit::from_palette(PaletteSeed {
        bg: floor,
        fg: kit.c.text_base,
        accent: kit.c.bg_accent,
    });
    let c = &mut kit.c;
    if contrast(c.text_muted, floor) < 4.5 {
        (c.text_muted, c.text_weak, c.icon_muted) =
            (held.c.text_muted, held.c.text_weak, held.c.icon_muted);
    }
    if contrast(c.text_faint, floor) < 3.0 {
        (c.text_faint, c.icon_faint) = (held.c.text_faint, held.c.icon_faint);
    }
}

/// `top` painted over opaque `bottom` as the window blends by default, in sRGB.
fn over_srgb(top: Color, bottom: Color) -> Color {
    use rune::theme::{srgb_decode as dec, srgb_encode as enc};
    let mix = |t: f32, b: f32| dec(enc(t) * top.a + enc(b) * (1.0 - top.a));
    Color {
        r: mix(top.r, bottom.r),
        g: mix(top.g, bottom.g),
        b: mix(top.b, bottom.b),
        a: 1.0,
    }
}

fn contrast(a: Color, b: Color) -> f32 {
    let (a, b) = (a.luminance(), b.luminance());
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

/// Blur of a frosted surface's backdrop, in points.
const FROST_BLUR: f32 = 24.0;
/// A frosted surface's tint opacity: enough that its muted text keeps 4.5:1 over sharp ink,
/// where the blur is missing (past the per-frame blur budget).
const FROST: f32 = 0.9;

/// `e` in the control material, over `fill`: the resting tint (`c.bg_button_neutral`), the
/// selected one (`c.pressed`) or a status role's `bg`.
pub fn glass(k: &Kit, e: El, fill: Color) -> El {
    k.elevate(e.fill(fill), &k.elevation.button_neutral)
}

/// A hand-built control: glass over `fill` when given, bare until hovered when not, and the
/// kit's hover and press tints either way.
pub fn pressable(k: &Kit, e: El, fill: Option<Color>) -> El {
    k.interactive(match fill {
        Some(f) => glass(k, e, f),
        None => e,
    })
}

/// A floating surface (popup, palette, toast): the raised layer, translucent over a blur of
/// what is behind it.
pub fn frost(k: &Kit, e: El) -> El {
    e.backdrop_blur(FROST_BLUR)
        .fill(k.alpha(k.c.layer_01, FROST))
}

/// `e` with a blur of what is behind it, under its own fill: a frosted scrim.
pub fn blur_behind(e: El) -> El {
    e.backdrop_blur(FROST_BLUR)
}

/// The window behind the framed main pane: one perceptual step from the page toward the ink.
fn rail_of(bg: Color, dark: bool) -> Color {
    let [l, a, b, alpha] = bg.to_oklab();
    Color::from_oklab([l + if dark { 0.05 } else { -0.045 }, a, b, alpha])
}

pub fn rail(k: &Kit) -> Color {
    k.c.bg_deep
}

pub fn provider_icon(id: ProviderId) -> &'static Icon {
    let name = match id {
        ProviderId::Claude => "anthropic",
        ProviderId::Codex => "openai",
        ProviderId::Gemini => "google",
        ProviderId::OpenCode => "opencode",
        ProviderId::Cursor => "cursor",
        ProviderId::Droid => "factory",
    };
    provider_icons::named(name).unwrap_or(&icons::PROVIDERS)
}

pub fn file_icon(path: &str) -> &'static Icon {
    rune_kit::file_icon(path.rsplit('/').next().unwrap_or(path))
}

/// `style` at `size` points before the user's text size is applied.
pub fn sized(style: TextStyle, size: f32) -> TextStyle {
    let scale = SCALE.with(Cell::get);
    TextStyle {
        size: size * scale,
        line_height: style.line_height / style.size.max(1.0) * size * scale,
        ..style
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_fields_refuse_text_that_is_not_ascii() {
        assert_eq!(parse_hex("#€"), None);
        assert_eq!(parse_hex("#é1"), None);
        assert!(parse_hex("#F6F").is_some() && parse_hex(" #16788C ").is_some());
    }
}
