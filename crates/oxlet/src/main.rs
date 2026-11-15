#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod actions;
mod composer;
mod demo;
mod dock;
mod pages;
mod palette;
mod settings;
mod sidebar;
mod state;
mod theme;
mod thread;
mod view;

use std::time::Duration;

use oxlet_core::{store, time};
use rune::{AppSpec, Chrome, KeyEvent, Tx, Waker};
use rune_kit::shell;

use crate::state::{App, DockTab, Event, Popup, Screen, Section};

const AUTOMATION_INTERVAL: f64 = 30.0;

/// Inter for the interface and Cascadia Code for code, both under the SIL Open Font License.
const FONTS: &[&[u8]] = &[
    rune_theme_opencode::FONTS[0],
    include_bytes!("../assets/fonts/CascadiaCode.ttf"),
];

fn init() -> App {
    let offset = time::local_offset_secs();
    let headless = rune::tooling::headless_run().is_some();
    if headless || rune::extra_args().iter().any(|a| a == "demo") {
        let repo = std::env::current_dir()
            .map(|d| d.to_string_lossy().into_owned())
            .unwrap_or_else(|_| "/".into());
        let mut app = App::new(
            demo::workspace(time::now_ms(), &repo),
            store::home(),
            offset,
        );
        app.demo = true;
        return app;
    }
    let dir = store::home();
    App::new(store::load(&dir), dir, offset)
}

fn setup(s: &mut App, waker: Waker) {
    s.bridge.set_waker(waker);
    actions::detect_providers(s);
    actions::refresh_repo(s);
}

fn poll(s: &mut App, clock: f64, tx: &mut Tx) -> bool {
    let mut changed = false;
    while let Ok(event) = s.inbox.try_recv() {
        changed = true;
        match event {
            Event::Agent { thread, event } => actions::apply_agent(s, &thread, event, tx),
            Event::TurnEdits {
                thread,
                message,
                files,
            } => actions::apply_edits(s, &thread, &message, files, tx),
            Event::Repo { cwd, result } => actions::apply_repo(s, cwd, result),
            Event::Files { cwd, result } => actions::apply_files(s, cwd, result),
            Event::Term(line) => actions::apply_terminal(s, line),
            Event::Prs(result) => s.prs = Some(result),
            Event::Providers(target, found) => {
                s.providers.insert(target, found);
            }
            Event::Editors(found) => s.editors = found,
            Event::Distros(found) => s.distros = found,
            Event::SshHosts(found) => s.ssh_hosts = found,
            Event::FolderChecked { target, result } => {
                actions::folder_checked(s, target, result, tx)
            }
            Event::PickedFolder(result) => actions::picked_folder(s, result, tx),
            Event::WorktreeReady { thread, result } => {
                actions::worktree_ready(s, &thread, result, tx)
            }
            Event::Notice(text) => actions::notice(s, tx, text),
            Event::GitDone { cwd, result } => {
                match result {
                    Ok(msg) if !msg.is_empty() => actions::notice(s, tx, msg),
                    Ok(_) => {}
                    Err(e) => actions::notice(s, tx, e),
                }
                if s.repo.cwd == cwd {
                    actions::refresh_repo(s);
                }
            }
        }
    }
    if s.dirty && clock >= s.save_due {
        actions::save_now(s);
    }
    if s.toast.as_ref().is_some_and(|t| clock >= t.until) {
        s.toast = None;
        changed = true;
    }
    if clock >= s.next_automation_check {
        s.next_automation_check = clock + AUTOMATION_INTERVAL;
        if !s.demo {
            actions::run_due_automations(s, tx);
        }
        tx.wake_after(Duration::from_secs_f64(AUTOMATION_INTERVAL));
    }
    changed
}

fn on_key(s: &mut App, ev: &KeyEvent, tx: &mut Tx) -> bool {
    if shell::is_sidebar_toggle(ev) {
        s.sidebar.toggle();
        return true;
    }
    if ev.is_command('n') {
        actions::new_thread(s);
        return true;
    }
    if ev.is_command(',') {
        s.screen = Screen::Settings(Section::General);
        return true;
    }
    if ev.is_command('k') {
        if s.palette.take().is_none() {
            palette::open(s);
        }
        return true;
    }
    if let rune::Key::Command(digit @ '1'..='9') = ev.key {
        let n = digit as usize - '1' as usize;
        if let Some(id) = s.ws.recent().get(n).map(|t| t.id.clone()) {
            actions::select_thread(s, &id, tx);
        }
        return true;
    }
    if ev.mods.alt && matches!(ev.key, rune::Key::Up | rune::Key::Down) {
        let ids: Vec<String> = s.ws.recent().iter().map(|t| t.id.clone()).collect();
        let at = s
            .selected
            .as_ref()
            .and_then(|id| ids.iter().position(|x| x == id));
        let down = ev.key == rune::Key::Down;
        let next = match at {
            Some(i) if down => (i + 1).min(ids.len().saturating_sub(1)),
            Some(i) => i.saturating_sub(1),
            None => 0,
        };
        if let Some(id) = ids.get(next).cloned() {
            actions::select_thread(s, &id, tx);
        }
        return true;
    }
    if ev.is_command('j') && s.screen == Screen::Thread {
        s.dock.toggle();
        if s.dock.is_open() {
            actions::refresh_repo(s);
        }
        return true;
    }
    if ev.is_command('`') {
        s.screen = Screen::Thread;
        s.dock_tab = DockTab::Terminal;
        s.dock.set_open(true);
        s.terminal.input.focused = true;
        return true;
    }
    if ev.key == rune::Key::Escape {
        if s.popup.take().is_some() || s.thread_menu.take().is_some() {
            return true;
        }
        if s.editing.is_some() {
            actions::cancel_edit(s);
            return true;
        }
        if matches!(s.screen, Screen::Settings(_)) {
            s.screen = Screen::Thread;
            return true;
        }
        if let Some(id) = s.selected.clone().filter(|id| s.turns.contains_key(id)) {
            actions::stop_turn(s, &id);
            return true;
        }
    }
    false
}

fn scene_dark(s: &mut App) {
    s.ws.settings.theme = oxlet_core::model::ThemeMode::Dark;
}

fn scene_light(s: &mut App) {
    s.ws.settings.theme = oxlet_core::model::ThemeMode::Light;
}

fn scene_dock(s: &mut App) {
    s.dock.set_open(true);
}

fn scene_new(s: &mut App) {
    s.selected = None;
}

fn scene_slash(s: &mut App) {
    s.composer = rune::TextEdit::new("/");
    s.composer.focused = true;
}

fn scene_model(s: &mut App) {
    s.popup = Some(Popup::Model);
}

fn scene_palette(s: &mut App) {
    palette::open(s);
    if let Some(p) = s.palette.as_mut() {
        p.query = rune::TextEdit::new("con");
        p.query.focused = true;
    }
}

fn scene_open(s: &mut App) {
    s.popup = Some(Popup::OpenIn);
    s.editors = vec![
        oxlet_core::open::Editor {
            name: "VS Code",
            program: "code".into(),
            remote: true,
        },
        oxlet_core::open::Editor {
            name: "Zed",
            program: "zed".into(),
            remote: false,
        },
    ];
}

fn scene_add(s: &mut App) {
    s.selected = None;
    s.popup = Some(Popup::AddProject("sidebar"));
    s.distros = vec!["Ubuntu-24.04".into()];
    if let Some(p) = s.ws.projects.get_mut(1) {
        p.target = oxlet_core::target::Target::Ssh {
            host: "me@devbox".into(),
        };
    }
}

fn scene_kanban(s: &mut App) {
    s.screen = Screen::Kanban;
}

fn scene_automations(s: &mut App) {
    s.screen = Screen::Automations;
}

fn scene_settings(s: &mut App) {
    s.screen = Screen::Settings(Section::Appearance);
}

fn scene_providers(s: &mut App) {
    s.screen = Screen::Settings(Section::Providers);
}

fn scene_terminal(s: &mut App) {
    s.dock.set_open(true);
    s.dock_tab = DockTab::Terminal;
    s.terminal.lines = vec![
        (false, "$ cargo test".into()),
        (false, "test result: ok. 23 passed".into()),
    ];
}

type Scene = (&'static str, fn(&mut App));

const SCENES: &[Scene] = &[
    ("light", scene_light),
    ("dark", scene_dark),
    ("dock", scene_dock),
    ("new", scene_new),
    ("slash", scene_slash),
    ("model", scene_model),
    ("palette", scene_palette),
    ("open", scene_open),
    ("add", scene_add),
    ("kanban", scene_kanban),
    ("automations", scene_automations),
    ("settings", scene_settings),
    ("providers", scene_providers),
    ("terminal", scene_terminal),
];

fn main() {
    #[cfg(windows)]
    {
        rune::set_app_icon_resource(1);
        let _ = rune_notify::register(&rune_notify::Identity::new("dev.oxlet.app", "Oxlet"));
    }
    let chrome = Chrome {
        full_size_content: true,
        traffic_lights: Some((16.0, 25.0)),
        drag_regions: true,
    };
    rune::App::new(AppSpec {
        min_size: Some((720.0, 480.0)),
        scenes: SCENES,
        fonts: FONTS,
        families: ("Inter", "Cascadia Code"),
        setup: Some(setup),
        poll: Some(poll),
        on_key: Some(on_key),
        ..AppSpec::new("Oxlet", (1280.0, 820.0), init, view::view)
    })
    .chrome(chrome)
    .run();
}

#[cfg(all(test, unix))]
mod tests;
