use oxlet_core::model::{AccessMode, Density, Palette, ThemeMode};
use oxlet_core::provider::{PROVIDERS, ProviderId};
use oxlet_core::target::Target;
use std::collections::HashMap;
use std::hash::Hash;

use oxlet_core::store;
use rune::{Color, El, ScrollSpec, TextEdit, Tx, col, icon, row, text};
use rune_kit::shell::SidebarRow;
use rune_kit::{
    Button, ButtonVariant, ControlSize, IconButton, IconButtonVariant, Kit, SegmentedControl,
    Select, Switch, TextInput,
};
use rune_theme_opencode::icons;

use crate::actions::{self, mark_dirty, notice};
use crate::state::{App, Pending, Placement, Popup, Screen, Section};
use crate::theme;

const GROUPS: [(&str, &[Section]); 4] = [
    ("Personal", &[Section::General, Section::Appearance]),
    ("Coding", &[Section::Providers, Section::Worktrees]),
    ("System", &[Section::Keybindings]),
    ("Archived", &[Section::Archived]),
];

pub fn view(s: &App, section: Section, k: &Kit, inset: f32, layer: &mut Vec<El>) -> El {
    let c = &k.c;
    let mut nav = col()
        .w(248.0)
        .h_full()
        .pt(inset.max(12.0) + 36.0)
        .px(10.0)
        .gap(2.0)
        .fill(theme::rail(k));
    nav = nav.child(
        SidebarRow::new("Back to app")
            .icon(&icons::ARROW_LEFT)
            .height(32.0)
            .on_select(|s: &mut App, _| actions::go(s, Screen::Thread))
            .build(k)
            .key("settings-back"),
    );
    for (group, sections) in GROUPS {
        nav = nav.child(
            text(group)
                .font(k.ty.small)
                .color(c.text_faint)
                .px(8.0)
                .pt(14.0)
                .pb(4.0),
        );
        for &sec in sections {
            nav = nav.child(
                {
                    let row = SidebarRow::new(sec.title())
                        .icon(section_icon(sec))
                        .height(32.0)
                        .selected(sec == section)
                        .on_select(move |s: &mut App, _| actions::go(s, Screen::Settings(sec)))
                        .build(k);
                    if sec == section {
                        theme::glass(k, row, c.pressed)
                    } else {
                        row
                    }
                }
                .key(format!("settings-{}", sec.title())),
            );
        }
    }
    let content = match section {
        Section::General => general(s, k, layer),
        Section::Appearance => appearance(s, k),
        Section::Providers => providers(s, k),
        Section::Worktrees => worktrees(s, k),
        Section::Keybindings => keybindings(k),
        Section::Archived => archived(s, k),
    };
    let page = col()
        .w_full()
        .max_w(560.0)
        .gap(18.0)
        .pt(inset.max(12.0) + 24.0)
        .pb(40.0)
        .child(content);
    row()
        .key("settings")
        .w_full()
        .h_full()
        .fill(c.bg_base)
        .child(nav)
        .child(
            col()
                .grow(1.0)
                .h_full()
                .min_h(0.0)
                .scroll(ScrollSpec {
                    stick_bottom: false,
                })
                .child(row().w_full().justify_center().px(24.0).child(page)),
        )
}

fn section_icon(s: Section) -> &'static rune::Icon {
    match s {
        Section::General => &icons::SETTINGS_GEAR,
        Section::Appearance => &icons::APPEARANCE,
        Section::Providers => &icons::PROVIDERS,
        Section::Worktrees => &icons::OUTLINE_WORKTREE,
        Section::Keybindings => &icons::KEYBOARD,
        Section::Archived => &icons::ARCHIVE,
    }
}

fn heading(title: &str, detail: &str, k: &Kit) -> El {
    col()
        .gap(4.0)
        .child(
            text(title.to_owned())
                .font(theme::sized(k.ty.body_medium, 20.0))
                .color(k.c.text_strong),
        )
        .child(
            text(detail.to_owned())
                .font(k.ty.ui)
                .color(k.c.text_muted)
                .wrap(),
        )
}

fn group(k: &Kit, rows: Vec<El>) -> El {
    let mut g = col()
        .w_full()
        .radius(k.radius.lg)
        .border(1.0, k.c.border_weak);
    for (i, r) in rows.into_iter().enumerate() {
        let r = if i == 0 {
            r
        } else {
            r.border_sides([1.0, 0.0, 0.0, 0.0], k.c.border_weak)
        };
        g = g.child(r);
    }
    g
}

fn setting(k: &Kit, title: &str, detail: Option<&str>, control: El) -> El {
    let mut label = col()
        .grow(1.0)
        .gap(2.0)
        .child(text(title.to_owned()).font(k.ty.ui).color(k.c.text_base));
    if let Some(d) = detail {
        label = label.child(
            text(d.to_owned())
                .font(k.ty.small)
                .color(k.c.text_faint)
                .wrap(),
        );
    }
    row()
        .w_full()
        .min_h(48.0)
        .px(14.0)
        .py(8.0)
        .gap(12.0)
        .center_y()
        .child(label)
        .child(control)
}

/// The default-provider choice that leaves it to whichever CLI is installed.
const FIRST_INSTALLED: &str = "First installed";

fn general(s: &App, k: &Kit, layer: &mut Vec<El>) -> El {
    let st = &s.ws.settings;
    let names: Vec<&str> = std::iter::once(FIRST_INSTALLED)
        .chain(PROVIDERS.iter().map(|p| p.name))
        .collect();
    let provider = Select::new("default-provider", &names)
        .label("Default provider")
        .current(Some(
            st.default_provider
                .map_or(FIRST_INSTALLED, |p| p.info().name),
        ))
        .open(s.popup == Some(Popup::DefaultProvider))
        .on_toggle(|s: &mut App| {
            s.popup = if s.popup == Some(Popup::DefaultProvider) {
                None
            } else {
                Some(Popup::DefaultProvider)
            }
        })
        .on_close(|s: &mut App| s.popup = None)
        .on_choose(|s: &mut App, v: &str, tx| {
            s.ws.settings.default_provider = PROVIDERS.iter().find(|p| p.name == v).map(|p| p.id);
            s.popup = None;
            mark_dirty(s, tx);
        })
        .build(k, layer);
    let access_labels: Vec<&str> = AccessMode::ALL.iter().map(|m| m.label()).collect();
    let access = Select::new("default-access", &access_labels)
        .label("Default access")
        .current(Some(st.default_access.label()))
        .open(s.popup == Some(Popup::DefaultAccess))
        .on_toggle(|s: &mut App| {
            s.popup = if s.popup == Some(Popup::DefaultAccess) {
                None
            } else {
                Some(Popup::DefaultAccess)
            }
        })
        .on_close(|s: &mut App| s.popup = None)
        .on_choose(|s: &mut App, v: &str, tx| {
            if let Some(m) = AccessMode::from_label(v) {
                s.ws.settings.default_access = m;
            }
            s.popup = None;
            mark_dirty(s, tx);
        })
        .build(k, layer);
    col()
        .gap(18.0)
        .child(heading("General", "Defaults for new threads.", k))
        .child(group(
            k,
            vec![
                setting(
                    k,
                    "Default provider",
                    Some("The agent new threads start with."),
                    provider,
                ),
                setting(
                    k,
                    "Default access",
                    Some("What agents may do without asking."),
                    access,
                ),
                setting(
                    k,
                    "Use worktrees for new threads",
                    Some("Each new thread works in its own managed worktree and branch."),
                    Switch::new(st.worktrees_by_default)
                        .on_toggle(|s: &mut App| {
                            let on = !s.ws.settings.worktrees_by_default;
                            s.ws.settings.worktrees_by_default = on;
                            s.placement = if on {
                                Placement::Worktree
                            } else {
                                Placement::Local
                            };
                            s.dirty = true;
                        })
                        .build(k)
                        .label("Use worktrees"),
                ),
                setting(
                    k,
                    "Notify when a turn finishes",
                    Some("A system notification when a thread you are not looking at finishes."),
                    Switch::new(st.notify_on_finish)
                        .on_toggle(|s: &mut App| {
                            s.ws.settings.notify_on_finish = !s.ws.settings.notify_on_finish;
                            s.dirty = true;
                        })
                        .build(k)
                        .label("Notify when a turn finishes"),
                ),
            ],
        ))
}

fn appearance(s: &App, k: &Kit) -> El {
    let c = &k.c;
    let st = &s.ws.settings;
    let restore = Button::new("Restore defaults")
        .icon(&icons::RESET)
        .variant(ButtonVariant::Ghost)
        .control_size(ControlSize::Small)
        .on_press(|s: &mut App| s.confirm = Some(Pending::RestoreAppearance))
        .build(k)
        .key("appearance-restore");
    let mut cards = row().w_full().gap(12.0);
    for (mode, label) in [
        (ThemeMode::System, "System"),
        (ThemeMode::Light, "Light"),
        (ThemeMode::Dark, "Dark"),
    ] {
        let on = st.theme == mode;
        cards = cards.child(
            col()
                .key(format!("theme-{label}"))
                .grow(1.0)
                .gap(8.0)
                .items_center()
                .cursor(rune::Cursor::Pointer)
                .role("radio")
                .label(label)
                .checked(on)
                .focusable(true)
                .on_press_tx(move |s: &mut App, tx| {
                    s.ws.settings.theme = mode;
                    tx.color_motion(rune::tok::motion::CALM);
                    mark_dirty(s, tx);
                })
                .child(theme_preview(mode, on, k))
                .child(text(label).font(k.ty.small).color(if on {
                    c.text_strong
                } else {
                    c.text_muted
                })),
        );
    }
    let density = SegmentedControl::new(
        &["Compact", "Comfortable"],
        Some(if st.density == Density::Compact { 0 } else { 1 }),
    )
    .label("Density")
    .on_pick(|s: &mut App, i, tx| {
        s.ws.settings.density = if i == 0 {
            Density::Compact
        } else {
            Density::Comfortable
        };
        mark_dirty(s, tx);
    })
    .build(k)
    .key("density");
    let clock = Switch::new(st.clock_24h)
        .on_toggle(|s: &mut App| {
            s.ws.settings.clock_24h = !s.ws.settings.clock_24h;
            s.dirty = true;
        })
        .build(k)
        .label("24-hour time");
    col()
        .gap(18.0)
        .child(
            row()
                .w_full()
                .items_start()
                .child(col().grow(1.0).child(heading(
                    "Appearance",
                    "Theme, colours, fonts, text size, density and time format. Restore defaults resets all of it.",
                    k,
                )))
                .child(restore),
        )
        .child(section(k, "Mode", None, cards))
        .child(section(
            k,
            "Palette",
            Some("A palette sets the colours of both the dark and the light theme."),
            presets(s, k),
        ))
        .child(section(k, "Typography", None, fonts(s, k)))
        .child(section(k, "Colours", None, palette_group(s, k)))
        .child(section(
            k,
            "Layout",
            None,
            group(
                k,
                vec![
                    setting(k, "Density", None, density),
                    setting(k, "24-hour time", None, clock),
                ],
            ),
        ))
}

/// One captioned part of a settings page.
fn section(k: &Kit, title: &str, detail: Option<&str>, body: El) -> El {
    let mut caption = col()
        .gap(2.0)
        .child(text(title).font(k.ty.small).color(k.c.text_faint));
    if let Some(detail) = detail {
        caption = caption.child(text(detail).font(k.ty.small).color(k.c.text_muted).wrap());
    }
    col().w_full().gap(8.0).child(caption).child(body)
}

pub fn restore_appearance(s: &mut App) {
    let st = &mut s.ws.settings;
    st.dark = Palette::default_dark();
    st.light = Palette::default_light();
    st.theme = ThemeMode::System;
    st.density = Density::Comfortable;
    st.ui_font.clear();
    st.code_font.clear();
    st.text_scale = 1.0;
    st.clock_24h = true;
    s.hex_fields.clear();
}

/// The editor a settings field types into, holding `saved` whenever the field is not focused:
/// text typed but never applied goes when the field loses focus.
fn draft<'a, K: Eq + Hash>(
    drafts: &'a mut HashMap<K, TextEdit>,
    key: K,
    saved: &str,
) -> &'a mut TextEdit {
    let edit = drafts.entry(key).or_insert_with(|| TextEdit::new(saved));
    if !edit.focused {
        *edit = TextEdit::new(saved);
    }
    edit
}

/// What a draft field shows: its draft while focused, else the saved value.
fn field_edit<K: Eq + Hash>(drafts: &HashMap<K, TextEdit>, key: &K, saved: &str) -> TextEdit {
    drafts
        .get(key)
        .filter(|e| e.focused)
        .cloned()
        .unwrap_or_else(|| TextEdit::new(saved))
}

fn presets(s: &App, k: &Kit) -> El {
    let c = &k.c;
    let current = oxlet_core::model::current_preset(&s.ws.settings).map(|p| p.name);
    let dark = k.dark;
    let mut list = row().w_full().gap(10.0);
    for preset in &oxlet_core::model::PRESETS {
        let on = current == Some(preset.name);
        let [bg, fg, accent] = if dark { preset.dark } else { preset.light };
        let hex = |h: &str| rune::color::hex(h).unwrap_or(Color::WHITE);
        let swatch = row()
            .w_full()
            .h(44.0)
            .radius(9.0)
            .fill(hex(bg))
            .border(1.0, c.border_weak)
            .px(10.0)
            .gap(6.0)
            .center_y()
            .child(rune::el().size(14.0, 14.0).radius(7.0).fill(hex(accent)))
            .child(
                rune::el()
                    .h(3.0)
                    .w(28.0)
                    .radius(1.5)
                    .fill(hex(fg).alpha(0.55)),
            );
        list = list.child(
            col()
                .key(format!("preset-{}", preset.name))
                .grow(1.0)
                .gap(6.0)
                .items_center()
                .cursor(rune::Cursor::Pointer)
                .role("radio")
                .label(preset.name)
                .checked(on)
                .focusable(true)
                .on_press_tx(move |s: &mut App, tx| {
                    s.ws.settings.dark = Palette::preset(preset, true);
                    s.ws.settings.light = Palette::preset(preset, false);
                    s.hex_fields.clear();
                    tx.color_motion(rune::tok::motion::CALM);
                    mark_dirty(s, tx);
                })
                .child(swatch.outline(if on { 2.0 } else { 0.0 }, 2.0, c.bg_accent))
                .child(text(preset.name).font(k.ty.small).color(if on {
                    c.text_strong
                } else {
                    c.text_muted
                })),
        );
    }
    if current.is_none() {
        let p = if dark {
            &s.ws.settings.dark
        } else {
            &s.ws.settings.light
        };
        let colour = |h: &str| theme::parse_hex(h).unwrap_or(Color::WHITE);
        let swatch = row()
            .w_full()
            .h(44.0)
            .radius(9.0)
            .fill(colour(&p.background))
            .border(1.0, c.border_weak)
            .px(10.0)
            .gap(6.0)
            .center_y()
            .child(
                rune::el()
                    .size(14.0, 14.0)
                    .radius(7.0)
                    .fill(colour(&p.accent)),
            )
            .child(
                rune::el()
                    .h(3.0)
                    .w(28.0)
                    .radius(1.5)
                    .fill(colour(&p.foreground).alpha(0.55)),
            );
        list = list.child(
            col()
                .key("preset-Custom")
                .grow(1.0)
                .gap(6.0)
                .items_center()
                .role("radio")
                .label("Custom")
                .checked(true)
                .child(swatch.outline(2.0, 2.0, c.bg_accent))
                .child(text("Custom").font(k.ty.small).color(c.text_strong)),
        );
    }
    list
}

const TEXT_SIZES: [(&str, f32); 4] = [
    ("Small", 0.92),
    ("Default", 1.0),
    ("Large", 1.08),
    ("Larger", 1.16),
];

fn fonts(s: &App, k: &Kit) -> El {
    let st = &s.ws.settings;
    let size = TEXT_SIZES
        .iter()
        .position(|(_, v)| (v - st.text_scale).abs() < 0.01)
        .unwrap_or(1);
    let names: Vec<&str> = TEXT_SIZES.iter().map(|(n, _)| *n).collect();
    let text_size = SegmentedControl::new(&names, Some(size))
        .label("Text size")
        .fit()
        .on_pick(|s: &mut App, i, tx| {
            s.ws.settings.text_scale = TEXT_SIZES[i].1;
            mark_dirty(s, tx);
        })
        .build(k)
        .key("text-size");
    group(
        k,
        vec![
            setting(
                k,
                "Interface font",
                Some("Built in, system, or any installed font"),
                font_field(s, k, false),
            ),
            setting(
                k,
                "Code font",
                Some("Built in, system, or any installed font"),
                font_field(s, k, true),
            ),
            setting(k, "Text size", None, text_size),
        ],
    )
}

fn font_field(s: &App, k: &Kit, code: bool) -> El {
    let c = &k.c;
    let (key, current, builtin) = if code {
        (
            "code-font",
            s.ws.settings.code_font.as_str(),
            "Cascadia Code",
        )
    } else {
        ("ui-font", s.ws.settings.ui_font.as_str(), "Inter")
    };
    let choice = |label: &'static str, value: &'static str| {
        let on = current == value;
        let chip = row()
            .key(format!("{key}-{label}"))
            .h(28.0)
            .px(10.0)
            .center_y()
            .radius(7.0);
        theme::pressable(
            k,
            chip,
            Some(if on { c.pressed } else { c.bg_button_neutral }),
        )
        .cursor(rune::Cursor::Pointer)
        .role("radio")
        .label(label)
        .checked(on)
        .focusable(true)
        .on_press_tx(move |s: &mut App, tx| {
            set_font(s, code, value);
            s.hex_fields.remove(key);
            mark_dirty(s, tx);
        })
        .child(text(label).font(k.ty.small).color(if on {
            c.text_strong
        } else {
            c.text_muted
        }))
    };
    let custom =
        !current.is_empty() && current != "system" && !(!code && current == "Cascadia Code");
    let saved = if custom {
        current.to_owned()
    } else {
        String::new()
    };
    let edit = field_edit(&s.hex_fields, &key, &saved);
    let field = TextInput::new(&edit)
        .width(Some(132.0))
        .placeholder("Font name")
        .on_edit(move |s: &mut App| draft(&mut s.hex_fields, key, &saved))
        .on_submit(move |s: &mut App, tx| {
            let name = s
                .hex_fields
                .get(key)
                .map(|e| e.text.trim().to_owned())
                .unwrap_or_default();
            set_font(s, code, &name);
            mark_dirty(s, tx);
        })
        .build(k)
        .shrink()
        .min_w(72.0)
        .key(format!("{key}-name"));
    let mut choices = row()
        .gap(6.0)
        .center_y()
        .shrink()
        .min_w(0.0)
        .child(choice(builtin, ""))
        .child(choice("System", "system"));
    if !code {
        choices = choices.child(choice("Cascadia", "Cascadia Code"));
    }
    choices.child(field)
}

fn set_font(s: &mut App, code: bool, value: &str) {
    let slot = if code {
        &mut s.ws.settings.code_font
    } else {
        &mut s.ws.settings.ui_font
    };
    *slot = value.to_owned();
}

fn theme_preview(mode: ThemeMode, on: bool, k: &Kit) -> El {
    let half = |dark: bool| {
        let (bg, panel, line) = if dark {
            (
                rune::color::hex("#2A2A2A").unwrap(),
                rune::color::hex("#3A3A3A").unwrap(),
                rune::color::hex("#5C5C5C").unwrap(),
            )
        } else {
            (
                rune::color::hex("#E9E9E9").unwrap(),
                Color::WHITE,
                rune::color::hex("#CFCFCF").unwrap(),
            )
        };
        let bar = |w: f32| rune::el().h(3.0).w(w).radius(1.5).fill(line);
        col()
            .grow(1.0)
            .h_full()
            .fill(bg)
            .pt(12.0)
            .items_center()
            .gap(8.0)
            .child(bar(48.0))
            .child(
                col()
                    .w(80.0)
                    .grow(1.0)
                    .radius(6.0)
                    .fill(panel)
                    .padding(10.0)
                    .gap(9.0)
                    .child(bar(40.0))
                    .child(bar(54.0))
                    .child(bar(46.0)),
            )
    };
    let inner = match mode {
        ThemeMode::System => row().w_full().h_full().child(half(false)).child(half(true)),
        ThemeMode::Light => row().w_full().h_full().child(half(false)),
        ThemeMode::Dark => row().w_full().h_full().child(half(true)),
    };
    let preview = col()
        .w_full()
        .h(104.0)
        .radius(10.0)
        .border(1.0, k.c.border_weak)
        .clip_rounded(10.0)
        .child(inner);
    // Outside the clip, which would cut the ring off.
    col()
        .w_full()
        .radius(10.0)
        .outline(if on { 2.0 } else { 0.0 }, 2.0, k.c.bg_accent)
        .child(preview)
}

fn palette_group(s: &App, k: &Kit) -> El {
    let c = &k.c;
    let st = &s.ws.settings;
    let shown = theme::is_dark(st, k.dark);
    let dark = s.colours_dark.unwrap_or(shown);
    let p = if dark { &st.dark } else { &st.light };
    let which = SegmentedControl::new(&["Dark", "Light"], Some(if dark { 0 } else { 1 }))
        .label("Theme to edit")
        .on_pick(|s: &mut App, i, _| s.colours_dark = Some(i == 0))
        .build(k)
        .key("colours-theme");
    let note = match (dark == shown, dark) {
        (true, _) => "On screen now.",
        (false, true) => "Used when the app is dark.",
        (false, false) => "Used when the app is light.",
    };
    let reset = Button::new("Reset")
        .variant(ButtonVariant::GhostMuted)
        .control_size(ControlSize::Small)
        .on_press_tx(move |s: &mut App, tx| {
            if dark {
                s.ws.settings.dark = Palette::default_dark();
            } else {
                s.ws.settings.light = Palette::default_light();
            }
            s.hex_fields.clear();
            mark_dirty(s, tx);
        })
        .build(k)
        .key("colours-reset");
    let header = row()
        .w_full()
        .px(14.0)
        .py(10.0)
        .gap(10.0)
        .center_y()
        .child(which)
        .child(text(note).font(k.ty.small).color(c.text_faint).grow(1.0))
        .child(reset);
    let fields: [(&'static str, &str, &str); 3] = if dark {
        [
            ("dark-accent", "Accent", &p.accent),
            ("dark-background", "Background", &p.background),
            ("dark-foreground", "Foreground", &p.foreground),
        ]
    } else {
        [
            ("light-accent", "Accent", &p.accent),
            ("light-background", "Background", &p.background),
            ("light-foreground", "Foreground", &p.foreground),
        ]
    };
    let mut rows = vec![header];
    for (key, label, value) in fields {
        rows.push(setting(k, label, None, hex_field(s, key, value, k)));
    }
    group(k, rows)
}

fn hex_field(s: &App, key: &'static str, value: &str, k: &Kit) -> El {
    let edit = field_edit(&s.hex_fields, &key, value);
    let saved = value.to_owned();
    let swatch = theme::parse_hex(&edit.text)
        .or_else(|| theme::parse_hex(value))
        .unwrap_or(Color::WHITE);
    row()
        .w(180.0)
        .gap(8.0)
        .center_y()
        .child(
            rune::el()
                .size(18.0, 18.0)
                .radius(9.0)
                .fill(swatch)
                .border(1.0, k.c.border_base),
        )
        .child(
            TextInput::new(&edit)
                .width(None)
                .text_style(theme::sized(k.ty.mono, 12.0))
                .on_edit(move |s: &mut App| draft(&mut s.hex_fields, key, &saved))
                .on_change(move |s: &mut App, tx| apply_hex(s, key, false, tx))
                .on_submit(move |s: &mut App, tx| apply_hex(s, key, true, tx))
                .build(k)
                .grow(1.0)
                .key(format!("hex-{key}")),
        )
}

/// Applies a typed colour. While typing only a whole `#RRGGBB` counts: `#F6F` on the way to
/// `#F6F6F3` is a colour of its own.
fn apply_hex(s: &mut App, key: &'static str, submitted: bool, tx: &mut Tx) {
    let Some(text) = s.hex_fields.get(key).map(|e| e.text.trim().to_uppercase()) else {
        return;
    };
    if theme::parse_hex(&text).is_none() || !(submitted || text.len() == 7) {
        return;
    }
    let st = &mut s.ws.settings;
    let slot = match key {
        "dark-accent" => &mut st.dark.accent,
        "dark-background" => &mut st.dark.background,
        "dark-foreground" => &mut st.dark.foreground,
        "light-accent" => &mut st.light.accent,
        "light-background" => &mut st.light.background,
        _ => &mut st.light.foreground,
    };
    *slot = text;
    tx.color_motion(rune::tok::motion::CALM);
    mark_dirty(s, tx);
}

fn providers(s: &App, k: &Kit) -> El {
    let c = &k.c;
    let mut rows = Vec::new();
    for p in PROVIDERS {
        let found = s.binary(&Target::Local, p.id).flatten();
        let shown = !s.ws.settings.hidden_providers.contains(&p.id);
        let id = p.id;
        let status = match &found {
            Some(path) => text(path.display().to_string())
                .font(theme::sized(k.ty.mono, 11.0))
                .color(c.text_faint)
                .truncate(),
            None => text(format!("`{}` not found on PATH", p.binary))
                .font(k.ty.small)
                .color(c.warning.icon),
        };
        let saved =
            s.ws.settings
                .provider_paths
                .iter()
                .find(|(x, _)| *x == id)
                .map(|(_, v)| v.clone())
                .unwrap_or_default();
        let edit = field_edit(&s.provider_path, &id, &saved);
        let path = TextInput::new(&edit)
            .width(None)
            .placeholder(format!("Custom path to {}", p.binary))
            .text_style(theme::sized(k.ty.mono, 12.0))
            .on_edit(move |s: &mut App| draft(&mut s.provider_path, id, &saved))
            .on_submit(move |s: &mut App, tx| {
                let v = s
                    .provider_path
                    .get(&id)
                    .map(|e| e.text.trim().to_owned())
                    .unwrap_or_default();
                s.ws.settings.provider_paths.retain(|(x, _)| *x != id);
                if !v.is_empty() {
                    s.ws.settings.provider_paths.push((id, v));
                }
                actions::detect_providers(s);
                mark_dirty(s, tx);
            })
            .build(k)
            .key(format!("provider-path-{}", p.binary));
        rows.push(
            col()
                .w_full()
                .px(14.0)
                .py(12.0)
                .gap(8.0)
                .child(
                    row()
                        .w_full()
                        .gap(10.0)
                        .center_y()
                        .child(icon(theme::provider_icon(p.id), 18.0).color(c.icon_base))
                        .child(
                            col()
                                .grow(1.0)
                                .gap(2.0)
                                .child(
                                    text(format!("{} · {}", p.name, p.runtime))
                                        .font(k.ty.ui_medium)
                                        .color(c.text_base),
                                )
                                .child(status),
                        )
                        .child(
                            Switch::new(shown)
                                .on_toggle(move |s: &mut App| {
                                    let h = &mut s.ws.settings.hidden_providers;
                                    if let Some(i) = h.iter().position(|x| *x == id) {
                                        h.remove(i);
                                    } else {
                                        h.push(id);
                                    }
                                    s.dirty = true;
                                })
                                .build(k)
                                .label("Show in model picker"),
                        ),
                )
                .child(
                    row()
                        .w_full()
                        .gap(8.0)
                        .child(path.grow(1.0))
                        .child(more_models(s, k, id).grow(1.0)),
                ),
        );
    }
    let detect = Button::new("Detect again")
        .icon(&icons::REFRESH)
        .control_size(ControlSize::Small)
        .on_press_tx(|s: &mut App, tx| {
            actions::detect_providers(s);
            notice(s, tx, "Looking for agent CLIs");
        })
        .build(k)
        .key("providers-detect");
    col()
        .gap(18.0)
        .child(
            row()
                .w_full()
                .items_start()
                .child(col().grow(1.0).child(heading(
                    "Agent providers",
                    "Oxlet runs the agent CLIs installed and signed in on this machine.",
                    k,
                )))
                .child(detect),
        )
        .child(group(k, rows))
}

/// Model ids the CLI accepts beyond Oxlet's list, comma-separated; they join the model picker.
fn more_models(s: &App, k: &Kit, id: ProviderId) -> El {
    let saved: Vec<&str> =
        s.ws.settings
            .custom_models
            .iter()
            .filter(|(p, _)| *p == id)
            .map(|(_, m)| m.as_str())
            .collect();
    let saved = saved.join(", ");
    let edit = field_edit(&s.model_fields, &id, &saved);
    TextInput::new(&edit)
        .width(None)
        .placeholder("More models, comma-separated")
        .text_style(theme::sized(k.ty.mono, 12.0))
        .on_edit(move |s: &mut App| draft(&mut s.model_fields, id, &saved))
        .on_submit(move |s: &mut App, tx| {
            let text = s
                .model_fields
                .get(&id)
                .map(|e| e.text.clone())
                .unwrap_or_default();
            let models = &mut s.ws.settings.custom_models;
            models.retain(|(p, _)| *p != id);
            for m in text.split(',').map(str::trim).filter(|m| !m.is_empty()) {
                if !models.iter().any(|(p, x)| *p == id && x == m) {
                    models.push((id, m.to_owned()));
                }
            }
            notice(s, tx, "Models saved");
            mark_dirty(s, tx);
        })
        .build(k)
        .key(format!("provider-models-{}", id.info().binary))
}

fn worktrees(s: &App, k: &Kit) -> El {
    let c = &k.c;
    let saved = s.ws.settings.worktree_root.clone();
    let root = field_edit(&s.hex_fields, &"worktree-root", &saved);
    let root_field = TextInput::new(&root)
        .width(None)
        .text_style(theme::sized(k.ty.mono, 12.0))
        .on_edit(move |s: &mut App| draft(&mut s.hex_fields, "worktree-root", &saved))
        .on_submit(|s: &mut App, tx| {
            if let Some(v) = s
                .hex_fields
                .get("worktree-root")
                .map(|e| e.text.trim().to_owned())
                .filter(|v| !v.is_empty())
            {
                s.ws.settings.worktree_root = v;
                mark_dirty(s, tx);
            }
        })
        .build(k)
        .w(260.0)
        .key("worktree-root");
    let mut list = Vec::new();
    for t in s.ws.threads.iter().filter(|t| t.worktree.is_some()) {
        let path = t.worktree.clone().unwrap_or_default();
        let id = t.id.clone();
        list.push(
            row()
                .w_full()
                .px(14.0)
                .py(10.0)
                .gap(10.0)
                .center_y()
                .child(icon(&icons::OUTLINE_WORKTREE, 14.0).color(c.icon_muted))
                .child(
                    col()
                        .grow(1.0)
                        .gap(2.0)
                        .child(
                            text(t.title.clone())
                                .font(k.ty.ui)
                                .color(c.text_base)
                                .truncate(),
                        )
                        .child(
                            text(format!(
                                "{} · {}",
                                t.branch.clone().unwrap_or_default(),
                                path
                            ))
                            .font(k.ty.small)
                            .color(c.text_faint)
                            .truncate(),
                        ),
                )
                .child(
                    Button::new("Remove")
                        .variant(ButtonVariant::Ghost)
                        .control_size(ControlSize::Small)
                        .on_press(move |s: &mut App| {
                            s.confirm = Some(Pending::RemoveWorktree(id.clone()))
                        })
                        .build(k)
                        .key(format!("worktree-remove-{}", t.id)),
                ),
        );
    }
    if list.is_empty() {
        list.push(
            text("No managed worktrees")
                .font(k.ty.ui)
                .color(c.text_faint)
                .px(14.0)
                .py(12.0),
        );
    }
    col()
        .gap(18.0)
        .child(heading(
            "Managed worktrees",
            "Threads in a worktree work on their own branch, apart from your checkout.",
            k,
        ))
        .child(group(
            k,
            vec![setting(
                k,
                "Location",
                Some(&store::expand_home(&s.ws.settings.worktree_root)),
                root_field,
            )],
        ))
        .child(group(k, list))
}

fn keybindings(k: &Kit) -> El {
    let m = if cfg!(target_os = "macos") {
        "⌘"
    } else {
        "Ctrl+"
    };
    let rows = [
        ("New thread", format!("{m}N")),
        ("Settings", format!("{m},")),
        ("Command palette", format!("{m}K")),
        ("Open thread 1 to 9", format!("{m}1 … {m}9")),
        (
            "Previous or next thread",
            if cfg!(target_os = "macos") {
                "⌥↑ ⌥↓".into()
            } else {
                "Alt+↑ Alt+↓".into()
            },
        ),
        (
            "Toggle sidebar",
            if cfg!(target_os = "macos") {
                "⇧⌘B".into()
            } else {
                "Ctrl+Shift+B".into()
            },
        ),
        ("Toggle review panel in a thread", format!("{m}J")),
        ("Open the terminal", format!("{m}`")),
        ("Stop the running turn", "Esc".into()),
        ("Send message", "Return".into()),
        ("New line", "Shift+Return".into()),
        ("Slash commands", "/".into()),
    ];
    let rows: Vec<El> = rows
        .into_iter()
        .map(|(label, keys)| setting(k, label, None, rune_kit::Keybind::new(&[keys]).build(k)))
        .collect();
    col()
        .gap(18.0)
        .child(heading("Keybindings", "Keyboard shortcuts in Oxlet.", k))
        .child(group(k, rows))
}

fn archived(s: &App, k: &Kit) -> El {
    let c = &k.c;
    let mut rows = Vec::new();
    for t in s.ws.threads.iter().filter(|t| t.archived) {
        let (a, b) = (t.id.clone(), t.id.clone());
        rows.push(
            row()
                .w_full()
                .px(14.0)
                .py(10.0)
                .gap(10.0)
                .center_y()
                .child(icon(theme::provider_icon(t.provider), 14.0).color(c.icon_muted))
                .child(
                    text(t.title.clone())
                        .font(k.ty.ui)
                        .color(c.text_base)
                        .truncate()
                        .grow(1.0),
                )
                .child(
                    Button::new("Restore")
                        .variant(ButtonVariant::Ghost)
                        .control_size(ControlSize::Small)
                        .on_press_tx(move |s: &mut App, tx| {
                            if let Some(t) = s.ws.thread_mut(&a) {
                                t.archived = false;
                            }
                            mark_dirty(s, tx);
                        })
                        .build(k)
                        .key(format!("restore-{}", t.id)),
                )
                .child(
                    IconButton::new(&icons::OUTLINE_TRASH)
                        .variant(IconButtonVariant::GhostMuted)
                        .control_size(ControlSize::Small)
                        .label("Delete permanently")
                        .on_press(move |s: &mut App| {
                            s.confirm = Some(Pending::DeleteThread(b.clone()))
                        })
                        .build(k)
                        .key(format!("delete-{}", t.id)),
                ),
        );
    }
    if rows.is_empty() {
        rows.push(
            text("Nothing archived")
                .font(k.ty.ui)
                .color(c.text_faint)
                .px(14.0)
                .py(12.0),
        );
    }
    col()
        .gap(18.0)
        .child(heading(
            "Archived threads",
            "Restore a thread to the sidebar, or delete it for good.",
            k,
        ))
        .child(group(k, rows))
}
