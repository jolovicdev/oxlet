use oxlet_core::model::{Thread, ThreadStatus};
use oxlet_core::time::{self, Bucket};
use rune::{El, TextEdit, Tx, icon, row, text};
use rune_kit::shell::{RowStatus, Sidebar, SidebarRow, SidebarSection};
use rune_kit::{IconButton, IconButtonVariant, Kit, MenuItem, TextInput, context_menu};
use rune_theme_opencode::icons;

use crate::actions::{self, mark_dirty};
use crate::state::{App, Screen, Section};
use crate::theme;

fn compact(s: &App) -> bool {
    s.ws.settings.density == oxlet_core::model::Density::Compact
}

fn row_h(s: &App) -> f32 {
    if compact(s) { 38.0 } else { 45.0 }
}

fn nav_h(s: &App) -> f32 {
    if compact(s) { 28.0 } else { 32.0 }
}
const EARLIER_CAP: usize = 150;

pub fn view(s: &App, k: &Kit, layer: &mut Vec<El>) -> El {
    let mut sb = Sidebar::new()
        .label("Threads")
        .inset(0.0)
        .header_gap(2.0)
        .header_padding(0.0, 12.0)
        .row_gap(2.0);
    if s.searching {
        sb = sb.header(
            TextInput::new(&s.search)
                .width(None)
                .placeholder("Search threads")
                .leading_icon(&icons::MAGNIFYING_GLASS)
                .on_edit(|s: &mut App| &mut s.search)
                .on_key(|s: &mut App, ev, _| {
                    if ev.key == rune::Key::Escape {
                        s.searching = false;
                        s.search = TextEdit::new("");
                        return true;
                    }
                    false
                })
                .build(k)
                .key("search-field"),
        );
    }
    sb = sb.header(primary_row(s, k)).header(destinations(s, k));

    sb = sb.section(projects(s, k));

    let query = s.search.text.trim().to_lowercase();
    let now = actions::now();
    let mut pinned = Vec::new();
    let mut today = Vec::new();
    let mut yesterday = Vec::new();
    let mut earlier = Vec::new();
    for t in s.ws.recent() {
        if !query.is_empty() && !t.title.to_lowercase().contains(&query) {
            continue;
        }
        if t.pinned {
            pinned.push(t);
            continue;
        }
        match time::bucket(t.updated, now, s.offset) {
            Bucket::Today => today.push(t),
            Bucket::Yesterday => yesterday.push(t),
            Bucket::Earlier => earlier.push(t),
        }
    }
    let earlier_total = earlier.len();
    earlier.truncate(EARLIER_CAP);
    for (key, title, list) in [
        ("pinned", "Pinned", pinned),
        ("recent", "Recent", today),
        ("yesterday", "Yesterday", yesterday),
        ("earlier", "Earlier", earlier),
    ] {
        if list.is_empty() {
            continue;
        }
        let count = if key == "earlier" {
            earlier_total
        } else {
            list.len()
        };
        let rows: Vec<El> = list.iter().map(|t| thread_row(s, t, k, layer)).collect();
        sb = sb.section(
            SidebarSection::new(title)
                .key(format!("section-{key}"))
                .count(count)
                .collapsed(s.is_collapsed(key))
                .on_toggle(move |s: &mut App, _| s.toggle_collapsed(key))
                .rows(rows),
        );
    }
    sb.footer(
        SidebarRow::new("Settings")
            .icon(&icons::SETTINGS_GEAR)
            .height(nav_h(s))
            .detail(shortcut(","))
            .on_select(|s: &mut App, _| s.screen = Screen::Settings(Section::General))
            .build(k)
            .key("nav-settings"),
    )
    .build(k)
    .on_drop_files(|s: &mut App, paths: &[std::path::PathBuf], tx| {
        actions::drop_projects(s, paths, tx)
    })
}

fn shortcut(key: &str) -> String {
    if cfg!(target_os = "macos") {
        format!("⌘{key}")
    } else {
        format!("Ctrl+{key}")
    }
}

/// Oxlet's mark in ink: brand colour is kept for actions.
pub fn mark(k: &Kit, size: f32) -> El {
    icon(&theme::MARK, size).color(k.c.text_strong)
}

/// New thread, the sidebar's primary action, with search beside it.
fn primary_row(s: &App, k: &Kit) -> El {
    let c = &k.c;
    let new = row()
        .key("nav-new")
        .grow(1.0)
        .h(34.0)
        .px(10.0)
        .gap(8.0)
        .center_y()
        .radius(9.0)
        .fill(c.bg_base)
        .border(1.0, c.border_weak)
        .hover(c.hover)
        .press(c.pressed)
        .cursor(rune::Cursor::Pointer)
        .role("button")
        .label("New thread")
        .focusable(true)
        .on_press(actions::new_thread)
        .child(icon(&icons::NEW_SESSION, 15.0).color(c.icon_base))
        .child(
            text("New thread")
                .font(k.ty.ui_medium)
                .color(c.text_base)
                .grow(1.0),
        )
        .child(text(shortcut("N")).font(k.ty.small).color(c.text_faint));
    let search = row()
        .key("brand-search")
        .size(34.0, 34.0)
        .center()
        .radius(9.0)
        .fill(if s.searching { c.layer_02 } else { c.bg_base })
        .border(1.0, c.border_weak)
        .hover(c.hover)
        .press(c.pressed)
        .cursor(rune::Cursor::Pointer)
        .role("button")
        .label("Search threads")
        .help(format!("Search threads  {}", shortcut("K")))
        .focusable(true)
        .on_press(|s: &mut App| {
            s.searching = !s.searching;
            s.search = TextEdit::new("");
            s.search.focused = s.searching;
        })
        .child(icon(&icons::MAGNIFYING_GLASS, 15.0).color(c.icon_muted));
    row().w_full().gap(6.0).child(new).child(search)
}

/// The other screens as one row of icons: less height than a list, so more threads fit.
fn destinations(s: &App, k: &Kit) -> El {
    let c = &k.c;
    let automations = s.ws.automations.iter().filter(|a| a.enabled).count();
    let items: [(&str, &'static rune::Icon, Screen); 4] = [
        ("Threads", &icons::BUBBLE_5, Screen::Thread),
        ("Board", &icons::DOT_GRID, Screen::Kanban),
        ("Pull requests", &icons::BRANCH, Screen::PullRequests),
        ("Automations", &icons::STATUS, Screen::Automations),
    ];
    let mut bar = row()
        .key("destinations")
        .w_full()
        .gap(2.0)
        .pt(6.0)
        .role("tablist");
    for (label, glyph, screen) in items {
        let on = s.screen == screen;
        let mut cell = row()
            .key(format!("nav-{}", label.to_lowercase().replace(' ', "-")))
            .grow(1.0)
            .h(30.0)
            .center()
            .gap(5.0)
            .radius(8.0)
            .fill(if on {
                c.layer_02
            } else {
                rune::Color::TRANSPARENT
            })
            .hover(c.hover)
            .cursor(rune::Cursor::Pointer)
            .role("tab")
            .label(label)
            .checked(on)
            .focusable(true)
            .help(label)
            .on_press(move |s: &mut App| {
                s.screen = screen;
                if screen == Screen::PullRequests {
                    actions::load_prs(s);
                }
            })
            .child(icon(glyph, 15.0).color(if on { c.icon_base } else { c.icon_muted }));
        if screen == Screen::Automations && automations > 0 {
            cell = cell.child(
                text(automations.to_string())
                    .font(k.ty.small_medium)
                    .color(c.text_faint),
            );
        }
        bar = bar.child(cell);
    }
    bar
}

fn projects(s: &App, k: &Kit) -> SidebarSection {
    let current = s.current_project().map(|p| p.id.clone());
    let add = IconButton::new(&icons::PLUS_SMALL)
        .variant(IconButtonVariant::GhostMuted)
        .label("Add project")
        .on_press(|s: &mut App| {
            s.adding_project = !s.adding_project;
            s.add_project = TextEdit::new("");
            s.add_project.focused = s.adding_project;
        })
        .build(k)
        .key("project-add");
    let mut rows: Vec<El> = Vec::new();
    if s.adding_project {
        rows.push(
            TextInput::new(&s.add_project)
                .width(None)
                .placeholder("/path/to/project")
                .leading_icon(&icons::FOLDER)
                .on_edit(|s: &mut App| &mut s.add_project)
                .on_submit(|s: &mut App, tx| {
                    let path = s.add_project.text.clone();
                    actions::add_project(s, &path, tx);
                })
                .build(k)
                .key("project-path"),
        );
    }
    for p in &s.ws.projects {
        let id = p.id.clone();
        let busy =
            s.ws.threads
                .iter()
                .filter(|t| t.project == p.id && t.status == ThreadStatus::Working)
                .count();
        let mut r = SidebarRow::new(p.name.clone())
            .icon(&icons::FOLDER)
            .height(nav_h(s))
            .selected(
                s.screen == Screen::Thread
                    && s.selected.is_none()
                    && current.as_deref() == Some(&p.id),
            )
            .on_select(move |s: &mut App, _| {
                s.draft_project = Some(id.clone());
                actions::new_thread(s);
            });
        if busy > 0 {
            r = r.status(RowStatus::Busy);
        }
        rows.push(r.build(k).key(format!("project-{}", p.id)));
    }
    SidebarSection::new("Projects")
        .key("section-projects")
        .collapsed(s.is_collapsed("projects"))
        .on_toggle(|s: &mut App, _| s.toggle_collapsed("projects"))
        .action(add)
        .rows(rows)
}

fn subtitle(s: &App, t: &Thread) -> String {
    let project = s.ws.project(&t.project).map_or("", |p| p.name.as_str());
    let mut parts = vec![project.to_owned()];
    if let Some(pr) = t.pull_request {
        parts.push(format!("#{pr}"));
    }
    parts.extend(t.branch.clone());
    parts.join(" · ")
}

fn thread_row(s: &App, t: &Thread, k: &Kit, layer: &mut Vec<El>) -> El {
    let id = t.id.clone();
    let avatar = icon(theme::provider_icon(t.provider), 14.0).color(k.c.icon_muted);
    let mut r = SidebarRow::new(t.title.clone())
        .height(row_h(s))
        .avatar(row().size(16.0, 16.0).center().child(avatar))
        .subtitle(subtitle(s, t))
        .selected(s.screen == Screen::Thread && s.selected.as_deref() == Some(&t.id))
        .on_select({
            let id = id.clone();
            move |s: &mut App, tx| actions::select_thread(s, &id, tx)
        })
        .on_double_click({
            let id = id.clone();
            move |s: &mut App, _| start_rename(s, &id)
        });
    if t.status != ThreadStatus::Working {
        r = r.detail(time::ago(t.updated, actions::now()));
    }
    r = match (t.status, t.unread) {
        (ThreadStatus::Working, _) => r.status(RowStatus::Busy),
        (ThreadStatus::NeedsInput | ThreadStatus::Failed, _) => r.status(RowStatus::Attention),
        (_, true) => r.status(RowStatus::Unread),
        _ => r,
    };
    if s.renaming.as_deref() == Some(t.id.as_str()) {
        r = r.renaming(
            &s.rename,
            |s: &mut App| &mut s.rename,
            commit_rename,
            |s: &mut App, _| s.renaming = None,
        );
    }
    let built = r
        .build(k)
        .key(format!("thread-{}", t.id))
        .on_secondary_press(move |s: &mut App, p, _| s.thread_menu = Some((id.clone(), p)));
    match &s.thread_menu {
        Some((menu_id, point)) if *menu_id == t.id => thread_menu(t, built, *point, k, layer),
        _ => built,
    }
}

fn thread_menu(
    t: &Thread,
    trigger: El,
    point: rune::PointerPress,
    k: &Kit,
    layer: &mut Vec<El>,
) -> El {
    let id = t.id.clone();
    let act = move |f: fn(&mut App, &str, &mut Tx)| {
        let id = id.clone();
        move |s: &mut App, tx: &mut Tx| {
            s.thread_menu = None;
            f(s, &id, tx);
            mark_dirty(s, tx);
        }
    };
    context_menu("thread-menu", trigger, Some(point))
        .item(
            MenuItem::new(if t.pinned { "Unpin" } else { "Pin" }).on_select_tx(act(|s, id, _| {
                if let Some(t) = s.ws.thread_mut(id) {
                    t.pinned = !t.pinned;
                }
            })),
        )
        .item(MenuItem::new("Rename").on_select_tx(act(|s, id, _| start_rename(s, id))))
        .item(
            MenuItem::new("Fork")
                .icon(&icons::FORK)
                .on_select_tx(act(|s, id, tx| {
                    if let Some(new) = s.ws.fork_thread(id, actions::now()) {
                        actions::select_thread(s, &new, tx);
                    }
                })),
        )
        .item(
            MenuItem::new(if t.unread {
                "Mark as read"
            } else {
                "Mark as unread"
            })
            .on_select_tx(act(|s, id, _| {
                if let Some(t) = s.ws.thread_mut(id) {
                    t.unread = !t.unread;
                }
            })),
        )
        .item(
            MenuItem::new("Mark as done")
                .icon(&icons::CHECK)
                .on_select_tx(act(|s, id, _| {
                    if let Some(t) = s.ws.thread_mut(id) {
                        t.status = ThreadStatus::Done;
                    }
                })),
        )
        .separator()
        .item(
            MenuItem::new("Archive")
                .icon(&icons::ARCHIVE)
                .on_select_tx(act(archive)),
        )
        .on_close(|s: &mut App| s.thread_menu = None)
        .build(k, layer)
}

pub fn archive(s: &mut App, id: &str, tx: &mut Tx) {
    actions::stop_turn(s, id);
    if let Some(t) = s.ws.thread_mut(id) {
        t.archived = true;
        t.pinned = false;
    }
    if s.selected.as_deref() == Some(id) {
        s.selected = s.ws.recent().first().map(|t| t.id.clone());
        actions::refresh_repo(s);
    }
    mark_dirty(s, tx);
}

fn start_rename(s: &mut App, id: &str) {
    if let Some(t) = s.ws.thread(id) {
        s.rename = TextEdit::new(t.title.clone());
        s.rename.anchor = 0;
        s.rename.focused = true;
        s.renaming = Some(id.to_owned());
    }
}

fn commit_rename(s: &mut App, tx: &mut Tx) {
    let title = s.rename.text.trim().to_owned();
    if let Some(id) = s.renaming.take()
        && !title.is_empty()
        && let Some(t) = s.ws.thread_mut(&id)
    {
        t.title = title;
        mark_dirty(s, tx);
    }
}
