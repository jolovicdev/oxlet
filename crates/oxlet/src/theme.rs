use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use oxlet_core::model::{Palette, Settings, ThemeMode};
use oxlet_core::provider::ProviderId;
use rune::{Color, Family, Icon, TextStyle};
use rune_kit::{Kit, PaletteSeed};
use rune_theme_opencode::{icons, provider_icons};

/// Oxlet's mark: an open ring with a bar turned inward, a G that is also a prompt.
pub static MARK: Icon = Icon {
    name: "oxlet-mark",
    view: 24.0,
    body: r#"<path d="M18.93 8A8 8 0 1 0 20 12H13" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round" fill="none"/>"#,
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
