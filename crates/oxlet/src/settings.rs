use oxlet_core::model::{AccessMode, Density, Palette, ThemeMode};
use oxlet_core::provider::{PROVIDERS, ProviderId};
use oxlet_core::{git, store};
use rune::{Color, El, ScrollSpec, TextEdit, Tx, col, icon, row, text};
use rune_kit::shell::SidebarRow;
use rune_kit::{
    Button, ButtonVariant, ControlSize, IconButton, IconButtonVariant, Kit, SegmentedControl,
    Select, Switch, TextInput,
};
use rune_theme_opencode::icons;

use crate::actions::{self, mark_dirty, notice};
use crate::state::{App, Event, Popup, Screen, Section};
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
            .on_select(|s: &mut App, _| s.screen = Screen::Thread)
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
                SidebarRow::new(sec.title())
                    .icon(section_icon(sec))
                    .height(32.0)
                    .selected(sec == section)
                    .on_select(move |s: &mut App, _| s.screen = Screen::Settings(sec))
                    .build(k)
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

fn general(s: &App, k: &Kit, layer: &mut Vec<El>) -> El {
    let st = &s.ws.settings;
    let names: Vec<&str> = PROVIDERS.iter().map(|p| p.name).collect();
    let provider = Select::new("default-provider", &names)
        .label("Default provider")
        .placeholder("Choose")
        .current(st.default_provider.map(|p| p.info().name))
        .open(s.popup == Some(Popup::Model))
        .on_toggle(|s: &mut App| {
            s.popup = if s.popup == Some(Popup::Model) {
                None
            } else {
                Some(Popup::Model)
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
        .open(s.popup == Some(Popup::Access))
        .on_toggle(|s: &mut App| {
            s.popup = if s.popup == Some(Popup::Access) {
                None
            } else {
                Some(Popup::Access)
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
                            s.ws.settings.worktrees_by_default =
                                !s.ws.settings.worktrees_by_default;
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
        .on_press_tx(|s: &mut App, tx| {
            s.ws.settings.dark = Palette::default_dark();
            s.ws.settings.light = Palette::default_light();
            s.ws.settings.theme = ThemeMode::System;
            s.ws.settings.density = Density::Comfortable;
            s.ws.settings.ui_font.clear();
            s.ws.settings.code_font.clear();
            s.ws.settings.text_scale = 1.0;
            s.hex_fields.clear();
            mark_dirty(s, tx);
        })
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
                    "Theme, colours, fonts, text size, density and time format.",
                    k,
                )))
                .child(restore),
        )
        .child(cards)
        .child(presets(s, k))
        .child(fonts(s, k))
        .child(palette_group(s, k, true))
        .child(palette_group(s, k, false))
        .child(group(
            k,
            vec![
                setting(k, "Density", None, density),
                setting(k, "24-hour time", None, clock),
            ],
        ))
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
        row()
            .key(format!("{key}-{label}"))
            .h(28.0)
            .px(10.0)
            .center_y()
            .radius(7.0)
            .fill(if on { c.layer_02 } else { Color::TRANSPARENT })
            .border(1.0, if on { c.border_base } else { c.border_weak })
            .hover(c.hover)
            .cursor(rune::Cursor::Pointer)
            .role("radio")
            .label(label)
            .checked(on)
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
    let edit = s
        .hex_fields
        .get(key)
        .cloned()
        .unwrap_or_else(|| TextEdit::new(if custom { current } else { "" }));
    let field = TextInput::new(&edit)
        .width(Some(132.0))
        .placeholder("Font name")
        .on_edit(move |s: &mut App| s.hex_fields.entry(key).or_insert_with(|| TextEdit::new("")))
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
        .key(format!("{key}-name"));
    let mut choices = row()
        .gap(6.0)
        .center_y()
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
    col()
        .w_full()
        .h(104.0)
        .radius(10.0)
        .border(
            if on { 2.0 } else { 1.0 },
            if on { k.c.text_strong } else { k.c.border_weak },
        )
        .clip_rounded(10.0)
        .child(inner)
}

fn palette_group(s: &App, k: &Kit, dark: bool) -> El {
    let c = &k.c;
    let st = &s.ws.settings;
    let active = theme::is_dark(st, k.dark) == dark;
    let p = if dark { &st.dark } else { &st.light };
    let title = if dark { "Dark theme" } else { "Light theme" };
    let note = match (st.theme, active) {
        (ThemeMode::System, _) if active => "Active now; follows the system.",
        (ThemeMode::System, _) => "Used when the system switches appearance.",
        (_, true) => "This is the active theme right now.",
        _ => "Inactive while the app is locked to the other appearance.",
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
        .key(format!("{title}-reset"));
    let header = col()
        .w_full()
        .px(14.0)
        .py(10.0)
        .gap(4.0)
        .child(
            row()
                .gap(8.0)
                .center_y()
                .child(text(title).font(k.ty.ui_medium).color(c.text_base))
                .child(reset),
        )
        .child(text(note).font(k.ty.small).color(c.text_faint));
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
    let edit = s
        .hex_fields
        .get(key)
        .cloned()
        .unwrap_or_else(|| TextEdit::new(value.to_owned()));
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
                .on_edit(move |s: &mut App| {
                    s.hex_fields.entry(key).or_insert_with(|| TextEdit::new(""))
                })
                .on_change(move |s: &mut App, tx| apply_hex(s, key, tx))
                .build(k)
                .grow(1.0)
                .key(format!("hex-{key}")),
        )
}

fn apply_hex(s: &mut App, key: &'static str, tx: &mut Tx) {
    let Some(text) = s.hex_fields.get(key).map(|e| e.text.trim().to_uppercase()) else {
        return;
    };
    if theme::parse_hex(&text).is_none() {
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
        let found = s.binary(p.id);
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
        let edit = s.provider_path.get(&id).cloned().unwrap_or_else(|| {
            TextEdit::new(
                s.ws.settings
                    .provider_paths
                    .iter()
                    .find(|(x, _)| *x == id)
                    .map(|(_, v)| v.clone())
                    .unwrap_or_default(),
            )
        });
        let path = TextInput::new(&edit)
            .width(None)
            .placeholder(format!("Custom path to {}", p.binary))
            .text_style(theme::sized(k.ty.mono, 12.0))
            .on_edit(move |s: &mut App| {
                s.provider_path
                    .entry(id)
                    .or_insert_with(|| TextEdit::new(""))
            })
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
    let edit = s
        .model_fields
        .get(&id)
        .cloned()
        .unwrap_or_else(|| TextEdit::new(saved.join(", ")));
    TextInput::new(&edit)
        .width(None)
        .placeholder("More models, comma-separated")
        .text_style(theme::sized(k.ty.mono, 12.0))
        .on_edit(move |s: &mut App| {
            s.model_fields
                .entry(id)
                .or_insert_with(|| TextEdit::new(""))
        })
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
    let root = s
        .hex_fields
        .get("worktree-root")
        .cloned()
        .unwrap_or_else(|| TextEdit::new(s.ws.settings.worktree_root.clone()));
    let root_field = TextInput::new(&root)
        .width(None)
        .text_style(theme::sized(k.ty.mono, 12.0))
        .on_edit(|s: &mut App| {
            s.hex_fields
                .entry("worktree-root")
                .or_insert_with(|| TextEdit::new(""))
        })
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
        let (id, repo) = (
            t.id.clone(),
            s.ws.project(&t.project)
                .map(|p| p.path.clone())
                .unwrap_or_default(),
        );
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
                        .on_press_tx(move |s: &mut App, tx| {
                            if let Some(t) = s.ws.thread_mut(&id) {
                                t.worktree = None;
                            }
                            let (repo, path) = (repo.clone(), path.clone());
                            s.bridge.spawn(move |b| {
                                let text = match git::worktree_remove(&repo, &path) {
                                    Ok(()) => format!("Removed {path}"),
                                    Err(e) => format!("Could not remove worktree: {e}"),
                                };
                                b.send(Event::Notice(text));
                            });
                            mark_dirty(s, tx);
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
        ("Toggle review panel", format!("{m}J")),
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
        .child(heading(
            "Keybindings",
            "Shortcuts available everywhere in Oxlet.",
            k,
        ))
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
                        .on_press_tx(move |s: &mut App, tx| {
                            s.ws.threads.retain(|t| t.id != b);
                            mark_dirty(s, tx);
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
