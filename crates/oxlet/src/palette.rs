use oxlet_core::fuzzy;
use oxlet_core::model::ThemeMode;
use oxlet_core::slash::{self, Command};
use rune::{Color, El, Icon, Key, TextEdit, Tx, col, icon, row, text};
use rune_kit::Kit;
use rune_theme_opencode::icons;

use crate::actions;
use crate::state::{App, DockTab, Screen, Section};
use crate::theme;

const SHOWN: usize = 10;

#[derive(Clone, Debug, PartialEq)]
pub enum Item {
    NewThread,
    Thread(String),
    Project(String),
    Go(Screen),
    Dock(DockTab),
    Theme(ThemeMode),
    Slash(Command),
}

pub struct Palette {
    pub query: TextEdit,
    pub active: usize,
}

struct Entry {
    item: Item,
    title: String,
    detail: String,
    glyph: &'static Icon,
}

pub fn open(s: &mut App) {
    let mut query = TextEdit::new("");
    query.focused = true;
    s.palette = Some(Palette { query, active: 0 });
}

fn entries(s: &App) -> Vec<Entry> {
    let mut out = vec![Entry {
        item: Item::NewThread,
        title: "New thread".into(),
        detail: "Action".into(),
        glyph: &icons::NEW_SESSION,
    }];
    for t in s.ws.recent() {
        let project = s.ws.project(&t.project).map_or("", |p| p.name.as_str());
        out.push(Entry {
            item: Item::Thread(t.id.clone()),
            title: t.title.clone(),
            detail: project.to_owned(),
            glyph: theme::provider_icon(t.provider),
        });
    }
    for p in &s.ws.projects {
        out.push(Entry {
            item: Item::Project(p.id.clone()),
            title: format!("New thread in {}", p.name),
            detail: "Project".into(),
            glyph: &icons::FOLDER,
        });
    }
    let go: [(&str, &'static Icon, Screen); 6] = [
        ("Board", &icons::DOT_GRID, Screen::Kanban),
        ("Pull requests", &icons::BRANCH, Screen::PullRequests),
        ("Automations", &icons::STATUS, Screen::Automations),
        (
            "Settings",
            &icons::SETTINGS_GEAR,
            Screen::Settings(Section::General),
        ),
        (
            "Appearance",
            &icons::APPEARANCE,
            Screen::Settings(Section::Appearance),
        ),
        (
            "Agent providers",
            &icons::PROVIDERS,
            Screen::Settings(Section::Providers),
        ),
    ];
    for (title, glyph, screen) in go {
        out.push(Entry {
            item: Item::Go(screen),
            title: format!("Open {title}"),
            detail: "Go to".into(),
            glyph,
        });
    }
    for tab in DockTab::ALL {
        out.push(Entry {
            item: Item::Dock(tab),
            title: format!("Show {}", tab.title()),
            detail: "Review panel".into(),
            glyph: &icons::LAYOUT_RIGHT,
        });
    }
    for (mode, title) in [
        (ThemeMode::Dark, "Dark"),
        (ThemeMode::Light, "Light"),
        (ThemeMode::System, "System"),
    ] {
        out.push(Entry {
            item: Item::Theme(mode),
            title: format!("Use {title} appearance"),
            detail: "Appearance".into(),
            glyph: &icons::APPEARANCE,
        });
    }
    if s.selected.is_some() {
        for spec in slash::COMMANDS {
            out.push(Entry {
                item: Item::Slash(spec.command),
                title: spec.title.into(),
                detail: format!("/{}", spec.name),
                glyph: &icons::PROMPT,
            });
        }
    }
    out
}

/// The entries matching `query`, best first. With no query, recent threads lead.
fn ranked(s: &App, query: &str) -> Vec<Entry> {
    let mut scored: Vec<(i32, usize, Entry)> = entries(s)
        .into_iter()
        .enumerate()
        .filter_map(|(i, e)| fuzzy::score(query, &e.title).map(|sc| (sc, i, e)))
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    scored.into_iter().map(|(_, _, e)| e).take(SHOWN).collect()
}

fn run(s: &mut App, item: Item, tx: &mut Tx) {
    s.palette = None;
    s.popup = None;
    match item {
        Item::NewThread => actions::new_thread(s),
        Item::Thread(id) => actions::select_thread(s, &id, tx),
        Item::Project(id) => {
            s.draft_project = Some(id);
            actions::new_thread(s);
        }
        Item::Go(screen) => actions::go(s, screen),
        Item::Dock(tab) => {
            actions::go(s, Screen::Thread);
            s.dock_tab = tab;
            s.dock.set_open(true);
            actions::refresh_repo(s);
        }
        Item::Theme(mode) => {
            s.ws.settings.theme = mode;
            tx.color_motion(rune::tok::motion::CALM);
            actions::mark_dirty(s, tx);
        }
        // A command acts on the open thread, so it is shown; the goal is its argument, so that
        // command waits in the composer for one.
        Item::Slash(command) => {
            actions::go(s, Screen::Thread);
            if command == Command::Goal {
                s.composer = TextEdit::new(format!("/{} ", slash::spec(command).name));
                s.composer.focused = true;
            } else {
                actions::run_command(s, command, "", tx);
            }
        }
    }
}

fn on_key(s: &mut App, ev: &rune::KeyEvent, _: &mut Tx) -> bool {
    if ev.key == Key::Escape {
        s.palette = None;
        return true;
    }
    let Some(query) = s.palette.as_ref().map(|p| p.query.text.clone()) else {
        return false;
    };
    let count = ranked(s, &query).len().max(1);
    let Some(p) = s.palette.as_mut() else {
        return false;
    };
    match ev.key {
        Key::Down => p.active = (p.active + 1) % count,
        Key::Up => p.active = (p.active + count - 1) % count,
        _ => return false,
    }
    true
}

fn submit(s: &mut App, tx: &mut Tx) {
    let Some(p) = s.palette.as_ref() else { return };
    let active = p.active;
    if let Some(e) = ranked(s, &p.query.text).into_iter().nth(active) {
        run(s, e.item, tx);
    }
}

pub fn view(s: &App, k: &Kit) -> Option<El> {
    let p = s.palette.as_ref()?;
    let c = &k.c;
    let results = ranked(s, &p.query.text);
    let field = rune::text_input(&p.query, theme::sized(k.ty.ui, 15.0))
        .single_line()
        .key("palette-field")
        .grow(1.0)
        .color(c.text_base)
        .placeholder("Search threads, projects and actions", c.text_faint)
        .caret(c.text_base, k.selection())
        .label("Command palette")
        .on_edit(|s: &mut App| &mut s.palette.as_mut().expect("the palette is open").query)
        .on_input_change(|s: &mut App, _| {
            if let Some(p) = s.palette.as_mut() {
                p.active = 0;
            }
        })
        .on_input_key(on_key)
        .on_submit(submit);
    let mut list = col()
        .key("palette-results")
        .w_full()
        .px(6.0)
        .pb(6.0)
        .role("listbox");
    if results.is_empty() {
        list = list.child(
            text("Nothing matches")
                .font(k.ty.ui)
                .color(c.text_faint)
                .padding(14.0),
        );
    }
    for (i, e) in results.into_iter().enumerate() {
        let on = i == p.active.min(SHOWN - 1);
        let item = e.item.clone();
        list = list.child(
            row()
                .key(format!("palette-{i}"))
                .w_full()
                .h(36.0)
                .px(10.0)
                .gap(10.0)
                .center_y()
                .radius(8.0)
                .fill(if on { c.layer_02 } else { Color::TRANSPARENT })
                .hover(c.hover)
                .cursor(rune::Cursor::Pointer)
                .role("option")
                .checked(on)
                .on_press_tx(move |s: &mut App, tx| run(s, item.clone(), tx))
                .child(icon(e.glyph, 15.0).color(if on { c.icon_base } else { c.icon_muted }))
                .child(
                    text(e.title)
                        .font(k.ty.ui)
                        .color(c.text_base)
                        .truncate()
                        .grow(1.0),
                )
                .child(text(e.detail).font(k.ty.small).color(c.text_faint)),
        );
    }
    let panel = col()
        .key("palette")
        .w(600.0)
        .radius(14.0)
        .fill(c.bg_base)
        .border(1.0, c.border_base)
        .shadow(
            18.0,
            48.0,
            Color {
                r: 0.0,
                g: 0.0,
                b: 0.0,
                a: if k.dark { 0.5 } else { 0.18 },
            },
        )
        .modal(true, |s: &mut App| s.palette = None)
        .child(
            row()
                .w_full()
                .h(52.0)
                .px(16.0)
                .gap(10.0)
                .center_y()
                .child(icon(&icons::MAGNIFYING_GLASS, 16.0).color(c.icon_muted))
                .child(field),
        )
        .child(rune::rule(1.0, c.border_weak))
        .child(list.pt(6.0));
    Some(
        col()
            .key("palette-layer")
            .absolute(0.0, 0.0)
            .w_full()
            .h_full()
            .items_center()
            .pt(96.0)
            .child(
                rune_kit::scrim("palette-scrim", |s: &mut App| s.palette = None).fill(Color {
                    r: 0.0,
                    g: 0.0,
                    b: 0.0,
                    a: if k.dark { 0.45 } else { 0.18 },
                }),
            )
            .child(panel),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typing_ranks_actions_and_threads_together() {
        let mut app = App::new(
            crate::demo::workspace(oxlet_core::time::now_ms(), "."),
            std::path::PathBuf::from("/nonexistent"),
            0,
        );
        app.demo = true;
        let titles: Vec<String> = ranked(&app, "board").into_iter().map(|e| e.title).collect();
        assert_eq!(titles[0], "Open Board");
        let first = ranked(&app, "context").into_iter().next().unwrap();
        assert!(matches!(first.item, Item::Thread(_)));
        assert!(ranked(&app, "").len() <= SHOWN);
    }
}
