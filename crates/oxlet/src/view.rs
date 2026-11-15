use std::rc::Rc;
use std::time::Duration;

use rune::{Cx, El, Spring, col, icon, row, text};
use rune_kit::shell::{PaneMotion, SidebarPeek, SidebarToggle, SplitView, Toolbar};
use rune_kit::{Button, ButtonVariant, ControlSize, IconButton, IconButtonVariant, Kit, Select};
use rune_theme_opencode::icons;

use crate::state::{App, DockTab, Popup, Screen};
use crate::{actions, composer, dock, pages, settings, sidebar, theme, thread};

pub fn view(s: &App, cx: &mut Cx<App>) -> El {
    let dark = theme::is_dark(&s.ws.settings, cx.system_dark());
    let k = theme::kit(&s.ws.settings, dark);
    let mut layer = Vec::new();
    let inset = cx.titlebar_inset();
    let base = match s.screen {
        Screen::Settings(section) => settings::view(s, section, &k, inset, &mut layer),
        screen => {
            let (main, toolbar) = match screen {
                Screen::Kanban => (pages::kanban(s, &k), Toolbar::new().title("Kanban")),
                Screen::PullRequests => (
                    pages::pull_requests(s, &k),
                    Toolbar::new()
                        .title("Pull requests")
                        .trailing(pr_refresh(&k)),
                ),
                Screen::Automations => (
                    pages::automations(s, &k, &mut layer),
                    Toolbar::new().title("Automations"),
                ),
                _ => (
                    thread_screen(s, &k, &mut layer),
                    thread_toolbar(s, &k, &mut layer),
                ),
            };
            let mut split = SplitView::new(main).sidebar(
                &s.sidebar,
                |s: &mut App| &mut s.sidebar,
                sidebar::view(s, &k, &mut layer),
            );
            if screen == Screen::Thread {
                split = split.inspector(&s.dock, |s: &mut App| &mut s.dock, dock::view(s, &k));
            }
            split
                .toolbar(toolbar)
                .titlebar_inset(inset)
                .frame_inset(8.0)
                .toolbar_in_panel()
                .sidebar_padding(10.0, 10.0)
                .sidebar_toggle(
                    SidebarToggle::new()
                        .size(ControlSize::Large)
                        .past_lights(16.0)
                        .edge(18.0)
                        .gap(4.0)
                        .in_header()
                        .unread(any_unread(s)),
                )
                .pane_motion(
                    PaneMotion::new()
                        .spring(Spring::new(0.3, 0.0))
                        .scale_with_width(480.0, 0.15, 0.3)
                        .fade_tail(0.15),
                )
                .sidebar_peek(
                    SidebarPeek::new()
                        .edge(16.0, ms(40))
                        .toggle_delay(ms(80))
                        .below(56.0)
                        .hold(15.0, ms(100))
                        .spring(Spring::new(0.18, 0.0)),
                )
                .reduced_motion(cx.is_reduced_motion())
                .build(&k)
        }
    };
    layer.extend(crate::palette::view(s, &k));
    if let Some(t) = &s.toast {
        layer.push(toast(&t.text, &k));
    }
    rune_kit::root(base, layer)
        .on_window_focus(|s: &mut App, focused, _| s.window_focused = focused)
}

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

fn any_unread(s: &App) -> bool {
    s.ws.threads.iter().any(|t| t.unread && !t.archived)
}

fn toast(message: &str, k: &Kit) -> El {
    let c = &k.c;
    col()
        .key("toast-layer")
        .absolute(0.0, 0.0)
        .w_full()
        .h_full()
        .justify_end()
        .items_center()
        .pb(28.0)
        .inert()
        .child(
            row()
                .key("toast")
                .max_w(520.0)
                .px(14.0)
                .py(9.0)
                .radius(10.0)
                .fill(c.bg_inverse)
                .shadow(
                    6.0,
                    20.0,
                    rune::Color {
                        r: 0.0,
                        g: 0.0,
                        b: 0.0,
                        a: 0.25,
                    },
                )
                .child(
                    text(message.to_owned())
                        .font(k.ty.ui)
                        .color(c.text_inverse)
                        .wrap(),
                ),
        )
}

fn pr_refresh(k: &Kit) -> El {
    IconButton::new(&icons::REFRESH)
        .variant(IconButtonVariant::GhostMuted)
        .label("Refresh pull requests")
        .on_press(|s: &mut App| {
            s.prs = None;
            actions::load_prs(s);
        })
        .build(k)
        .key("prs-refresh")
}

fn thread_toolbar(s: &App, k: &Kit, layer: &mut Vec<El>) -> Toolbar {
    let c = &k.c;
    let tb = Toolbar::new();
    let title = match s.thread() {
        Some(t) => row()
            .gap(8.0)
            .center_y()
            .child(icon(theme::provider_icon(t.provider), 14.0).color(c.icon_muted))
            .child(
                text(t.title.clone())
                    .font(k.ty.ui_medium)
                    .color(c.text_base)
                    .truncate(),
            ),
        None => text("New thread").font(k.ty.ui_medium).color(c.text_base),
    };
    let button = |glyph: &'static rune::Icon, label: &str| {
        IconButton::new(glyph)
            .variant(IconButtonVariant::GhostMuted)
            .label(label)
    };
    let mut trailing = row().gap(2.0).center_y();
    if s.selected.is_some() {
        trailing = trailing.child(
            button(&icons::FORK, "Fork thread")
                .on_press_tx(|s: &mut App, tx| {
                    actions::run_command(s, oxlet_core::slash::Command::Fork, "", tx)
                })
                .build(k)
                .key("toolbar-fork"),
        );
    }
    if s.current_project().is_some() {
        trailing = trailing.child(open_in(s, k, layer));
    }
    trailing = trailing
        .child(
            button(&icons::PLUS, "New thread")
                .on_press(actions::new_thread)
                .build(k)
                .key("toolbar-new"),
        )
        .child(
            button(&icons::TERMINAL, "Terminal")
                .on_press(|s: &mut App| {
                    s.dock_tab = DockTab::Terminal;
                    s.dock.set_open(true);
                    s.terminal.input.focused = true;
                })
                .build(k)
                .key("toolbar-terminal"),
        )
        .child(
            button(
                if s.dock.is_open() {
                    &icons::LAYOUT_RIGHT_FULL
                } else {
                    &icons::LAYOUT_RIGHT
                },
                "Toggle review panel",
            )
            .expanded(s.dock.is_open())
            .on_press(|s: &mut App| {
                s.dock.toggle();
                if s.dock.is_open() {
                    actions::refresh_repo(s);
                }
            })
            .build(k)
            .key("toolbar-dock"),
        );
    tb.leading(title).trailing(trailing)
}

/// The project in an editor or the file manager.
fn open_in(s: &App, k: &Kit, layer: &mut Vec<El>) -> El {
    let button = IconButton::new(&icons::SQUARE_ARROW_TOP_RIGHT)
        .variant(IconButtonVariant::GhostMuted)
        .label("Open in")
        .expanded(s.popup == Some(Popup::OpenIn))
        .on_press(|s: &mut App| {
            s.popup = if s.popup == Some(Popup::OpenIn) {
                None
            } else {
                Some(Popup::OpenIn)
            }
        })
        .build(k)
        .key("toolbar-open");
    let mut menu = rune_kit::Menu::new("open-in")
        .trigger(button)
        .open(s.popup == Some(Popup::OpenIn))
        .min_width(220.0);
    let place = s.place();
    let remote = place
        .as_ref()
        .is_some_and(|at| at.target != oxlet_core::target::Target::Local);
    let mut editors = 0;
    for (i, editor) in s.editors.iter().enumerate() {
        if remote && !editor.remote {
            continue;
        }
        editors += 1;
        menu = menu.item(
            rune_kit::MenuItem::new(format!("Open in {}", editor.name))
                .icon(&icons::CODE)
                .on_select_tx(move |s: &mut App, tx| actions::open_in_editor(s, i, tx)),
        );
    }
    if editors > 0 {
        menu = menu.separator();
    }
    if place.and_then(|at| at.local_path()).is_some() {
        menu = menu.item(
            rune_kit::MenuItem::new(format!("Reveal in {}", oxlet_core::open::file_manager()))
                .icon(&icons::FOLDER)
                .on_select_tx(actions::reveal),
        );
    }
    menu.item(
        rune_kit::MenuItem::new("Copy path")
            .icon(&icons::COPY)
            .on_select_tx(|s: &mut App, tx| {
                s.popup = None;
                if let Some(at) = s.place() {
                    tx.copy(at.cwd);
                }
            }),
    )
    .on_close(|s: &mut App| s.popup = None)
    .build(k, layer)
}

fn thread_screen(s: &App, k: &Rc<Kit>, layer: &mut Vec<El>) -> El {
    let composer = row()
        .w_full()
        .justify_center()
        .px(24.0)
        .pb(16.0)
        .pt(4.0)
        .child(
            col()
                .w_full()
                .max_w(thread::COLUMN)
                .child(composer::view(s, k, layer)),
        );
    let body = match s.thread() {
        Some(t) => thread::transcript(s, t, k),
        None => hero(s, k, layer),
    };
    col()
        .key("thread-screen")
        .w_full()
        .h_full()
        .min_h(0.0)
        .child(body)
        .child(composer)
}

fn hero(s: &App, k: &Kit, layer: &mut Vec<El>) -> El {
    let c = &k.c;
    let mut out = col()
        .key("hero")
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .center()
        .gap(14.0)
        .padding(24.0);
    out = out.child(crate::sidebar::mark(k, 40.0).mb(16.0)).child(
        text("What should we build?")
            .font(rune::TextStyle {
                weight: 600.0,
                tracking: -0.4,
                ..theme::sized(k.ty.body_medium, 24.0)
            })
            .color(c.text_strong),
    );
    if s.ws.projects.is_empty() {
        let more = Button::new("Other locations")
            .variant(ButtonVariant::Ghost)
            .expanded(s.popup == Some(Popup::AddProject("hero")))
            .on_press(|s: &mut App| sidebar::toggle_add_menu(s, "hero"))
            .build(k)
            .key("hero-more");
        out = out
            .child(
                text("Add a project folder to start a thread in it.")
                    .font(k.ty.body)
                    .color(c.text_muted),
            )
            .child(
                row()
                    .gap(8.0)
                    .center_y()
                    .child(
                        Button::new("Add project")
                            .variant(ButtonVariant::Submit)
                            .icon(&icons::FOLDER_ADD_LEFT)
                            .on_press_tx(|s: &mut App, tx| actions::pick_folder(s, None, tx))
                            .build(k)
                            .key("hero-add"),
                    )
                    .child(sidebar::add_menu(s, k, layer, "hero", more)),
            );
        if let Some(mode) = s.adding_project {
            out = out.child(
                col()
                    .w_full()
                    .max_w(460.0)
                    .gap(2.0)
                    .children(sidebar::add_field(s, k, mode, "hero-path")),
            );
        }
        return out;
    }
    let names: Vec<&str> = s.ws.projects.iter().map(|p| p.name.as_str()).collect();
    let current = s.current_project().map(|p| p.name.as_str());
    let picker = Select::new("project", &names)
        .label("Project")
        .current(current)
        .open(s.popup == Some(Popup::Project))
        .on_toggle(|s: &mut App| {
            s.popup = if s.popup == Some(Popup::Project) {
                None
            } else {
                Some(Popup::Project)
            }
        })
        .on_close(|s: &mut App| s.popup = None)
        .on_choose(|s: &mut App, v: &str, _| {
            s.draft_project =
                s.ws.projects
                    .iter()
                    .find(|p| p.name == v)
                    .map(|p| p.id.clone());
            s.popup = None;
            actions::refresh_repo(s);
        })
        .build(k, layer);
    let branch = s.repo.status.branch.clone();
    let mut meta = row()
        .gap(10.0)
        .center_y()
        .child(text("in").font(k.ty.body).color(c.text_muted))
        .child(picker);
    if let Some(b) = branch {
        meta = meta.child(
            row()
                .gap(4.0)
                .center_y()
                .child(icon(&icons::BRANCH, 13.0).color(c.icon_faint))
                .child(text(b).font(k.ty.ui).color(c.text_faint)),
        );
    }
    out.child(meta).child(starters(k))
}

const STARTERS: [(&str, &str, &str); 4] = [
    (
        "Map this codebase",
        "The architecture and where to start reading",
        "Give me a tour of this codebase: the architecture, the main modules, how they connect, and where to start reading.",
    ),
    (
        "Find a bug",
        "Reproduce a defect and propose a fix",
        "Look for a real bug in this project. Reproduce it, explain the root cause, and propose a minimal fix.",
    ),
    (
        "Write tests",
        "Cover the riskiest untested code",
        "Find the most important code that has no tests and write focused tests for it.",
    ),
    (
        "Review my changes",
        "Check the working tree before a commit",
        oxlet_core::slash::REVIEW_PROMPT,
    ),
];

fn starters(k: &Kit) -> El {
    let c = &k.c;
    let glyphs: [&'static rune::Icon; 4] = [
        &icons::FILE_TREE,
        &icons::MAGNIFYING_GLASS,
        &icons::CHECKLIST,
        &icons::REVIEW,
    ];
    let card = |i: usize| {
        let (title, detail, prompt) = STARTERS[i];
        col()
            .key(format!("starter-{i}"))
            .grow(1.0)
            .min_w(0.0)
            .gap(4.0)
            .padding(14.0)
            .radius(12.0)
            .fill(c.layer_01)
            .border(1.0, c.border_weak)
            .hover(c.hover)
            .press(c.pressed)
            .cursor(rune::Cursor::Pointer)
            .role("button")
            .label(title)
            .focusable(true)
            .on_press(move |s: &mut App| {
                s.composer = rune::TextEdit::new(prompt);
                s.composer.focused = true;
            })
            .child(icon(glyphs[i], 16.0).color(c.icon_accent).mb(6.0))
            .child(text(title).font(k.ty.ui_medium).color(c.text_base))
            .child(text(detail).font(k.ty.small).color(c.text_faint).wrap())
    };
    col()
        .key("starters")
        .w_full()
        .max_w(640.0)
        .gap(10.0)
        .pt(18.0)
        .child(row().w_full().gap(10.0).child(card(0)).child(card(1)))
        .child(row().w_full().gap(10.0).child(card(2)).child(card(3)))
}
