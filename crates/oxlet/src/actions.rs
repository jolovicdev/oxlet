use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use oxlet_core::model::{ChatMode, FileChange, Id, Part, Project, Role, ThreadStatus};
use oxlet_core::process::{self, Line};
use oxlet_core::provider::{AgentEvent, PROVIDERS, ProviderId, TurnRequest};
use oxlet_core::slash::{self, Command};
use oxlet_core::target::{self, Place, Target};
use oxlet_core::{export, git, schedule, store, time};
use rune::{FileDialogOptions, TextEdit, Tx};
use rune_kit::diff::{self, DiffState, FileDiff};
use rune_kit::{FileNode, TreeModel};

use crate::state::{
    AddMode, App, DockTab, Event, Pending, Placement, RepoSnapshot, Screen, Toast, Turn, file_tree,
};

const SAVE_DELAY: f64 = 0.4;
const MAX_TERMINAL_LINES: usize = 4000;
const MAX_UNTRACKED_PREVIEW: u64 = 256 * 1024;
/// Untracked files past these limits are listed by name only. Previews are diffed on the UI
/// thread when the read lands, about 20 ms for 512 KB in a debug build, and their bytes cross
/// the connection to a remote target.
const MAX_UNTRACKED_FILES: usize = 200;
const MAX_UNTRACKED_BYTES: u64 = 512 * 1024;

pub fn now() -> u64 {
    time::now_ms()
}

pub fn mark_dirty(s: &mut App, tx: &mut Tx) {
    if !s.dirty {
        s.dirty = true;
        s.save_due = tx.clock() + SAVE_DELAY;
        tx.wake_after(std::time::Duration::from_secs_f64(SAVE_DELAY));
    }
}

pub fn notice(s: &mut App, tx: &mut Tx, text: impl Into<String>) {
    s.toast = Some(Toast {
        text: text.into(),
        until: tx.clock() + 3.5,
    });
    tx.wake_after(std::time::Duration::from_millis(3600));
}

pub fn detect_providers(s: &App) {
    let overrides: Vec<(ProviderId, String)> = s.ws.settings.provider_paths.clone();
    s.bridge.spawn(move |b| {
        let found = PROVIDERS
            .iter()
            .map(|p| {
                let custom = overrides
                    .iter()
                    .find(|(id, _)| *id == p.id)
                    .map(|(_, path)| store::expand_home(path));
                (
                    p.id,
                    oxlet_core::provider::locate(p.binary, custom.as_deref(), None),
                )
            })
            .collect();
        b.send(Event::Providers(Target::Local, found));
        b.send(Event::Editors(oxlet_core::open::editors(None)));
        b.send(Event::Distros(target::wsl_distros()));
    });
}

/// Looks for the agent CLIs on a WSL or SSH target the first time it is used.
fn ensure_providers(s: &mut App, on: &Target) {
    if *on == Target::Local || s.providers.contains_key(on) {
        return;
    }
    s.providers.insert(on.clone(), Vec::new());
    let on = on.clone();
    s.bridge.spawn(move |b| {
        let binaries: Vec<&str> = PROVIDERS.iter().map(|p| p.binary).collect();
        match target::find_programs(&on, &binaries) {
            Ok(found) => {
                let found = PROVIDERS
                    .iter()
                    .zip(found)
                    .map(|(p, path)| (p.id, path.map(PathBuf::from)))
                    .collect();
                b.send(Event::Providers(on, found));
            }
            Err(e) => b.send(Event::Notice(e)),
        }
    });
}

/// Shows `screen`, closing whatever picker was open on the one before.
pub fn go(s: &mut App, screen: Screen) {
    s.screen = screen;
    s.popup = None;
    if screen == Screen::PullRequests {
        load_prs(s);
    }
}

pub fn select_thread(s: &mut App, id: &str, tx: &mut Tx) {
    if s.editing.as_ref().is_some_and(|(t, _)| t != id) {
        cancel_edit(s);
    }
    s.screen = Screen::Thread;
    s.selected = Some(id.to_owned());
    s.popup = None;
    if let Some(t) = s.ws.thread_mut(id)
        && t.unread
    {
        t.unread = false;
        mark_dirty(s, tx);
    }
    s.composer.focused = true;
    refresh_repo(s);
}

pub fn new_thread(s: &mut App) {
    s.screen = Screen::Thread;
    s.selected = None;
    s.popup = None;
    s.editing = None;
    s.composer = TextEdit::new("");
    s.composer.focused = true;
    s.placement = if s.ws.settings.worktrees_by_default {
        Placement::Worktree
    } else {
        Placement::Local
    };
    refresh_repo(s);
}

/// Asks for a folder to add as a project, starting in `start`.
pub fn pick_folder(s: &mut App, start: Option<PathBuf>, tx: &mut Tx) {
    s.popup = None;
    let options = FileDialogOptions {
        title: "Add project".into(),
        directory: start,
        ..Default::default()
    };
    tx.open_folder(options, |s: &mut App, result, tx| match result {
        Ok(Some(path)) => add_project(s, &path.to_string_lossy(), tx),
        Ok(None) => {}
        // WSLg usually has no desktop portal for a Linux dialog; Windows has its own.
        Err(_) if target::inside_wsl() => s
            .bridge
            .spawn(|b| b.send(Event::PickedFolder(target::windows_folder_dialog()))),
        Err(e) => type_folder(s, &e.to_string(), tx),
    });
}

pub fn picked_folder(s: &mut App, result: Result<Option<String>, String>, tx: &mut Tx) {
    match result {
        Ok(Some(path)) => add_project(s, &path, tx),
        Ok(None) => {}
        Err(e) => type_folder(s, &e, tx),
    }
}

fn type_folder(s: &mut App, why: &str, tx: &mut Tx) {
    start_adding(s, AddMode::Path);
    notice(
        s,
        tx,
        format!("No folder dialog is available ({why}). Type the folder's path instead."),
    );
}

/// Opens the add-project field, for a path or an SSH location.
pub fn start_adding(s: &mut App, mode: AddMode) {
    s.popup = None;
    s.adding_project = Some(mode);
    s.add_project = TextEdit::new("");
    s.add_project.focused = true;
    if mode == AddMode::Ssh {
        s.bridge
            .spawn(|b| b.send(Event::SshHosts(target::ssh_hosts())));
    }
}

/// Adds a typed or picked folder. A WSL or SSH folder is checked on its target first.
pub fn add_project(s: &mut App, input: &str, tx: &mut Tx) {
    let (on, path) = target::parse_location(input);
    if path.is_empty() {
        return;
    }
    if on != Target::Local {
        notice(
            s,
            tx,
            format!("Opening {path} on {}", on.label().unwrap_or_default()),
        );
        let at = Place::new(on, path);
        s.bridge.spawn(move |b| {
            let result = target::resolve_folder(&at);
            b.send(Event::FolderChecked {
                target: at.target,
                result,
            });
        });
        return;
    }
    let path = store::expand_home(&path);
    if !Path::new(&path).is_dir() {
        notice(s, tx, format!("{path} is not a folder"));
        return;
    }
    added(s, Target::Local, &path, tx);
}

pub fn folder_checked(s: &mut App, on: Target, result: Result<String, String>, tx: &mut Tx) {
    match result {
        Ok(path) => {
            s.toast = None;
            added(s, on, &path, tx);
        }
        Err(e) => notice(s, tx, e),
    }
}

fn added(s: &mut App, on: Target, path: &str, tx: &mut Tx) {
    let id = s.ws.add_project_on(on, path);
    s.draft_project = Some(id);
    s.adding_project = None;
    s.add_project = TextEdit::new("");
    new_thread(s);
    mark_dirty(s, tx);
}

/// Sends the composer's text: a slash command, or a message that starts a turn.
pub fn submit(s: &mut App, tx: &mut Tx) {
    let text = s.composer.text.trim().to_owned();
    if text.is_empty() {
        return;
    }
    if !s.slash_dismissed
        && let Some(hits) = slash::suggestions(&text)
        && let Some(spec) = hits.get(s.slash_active.min(hits.len().saturating_sub(1)))
        && slash::parse(&text).is_none()
    {
        s.composer = TextEdit::new(format!("/{} ", spec.name));
        s.composer.focused = true;
        s.slash_active = 0;
        return;
    }
    if let Some((command, arg)) = slash::parse(&text) {
        let arg = arg.to_owned();
        s.composer = TextEdit::new("");
        s.composer.focused = true;
        run_command(s, command, &arg, tx);
        return;
    }
    s.composer = TextEdit::new("");
    s.composer.focused = true;
    send_prompt(s, &text, tx);
}

pub fn send_prompt(s: &mut App, text: &str, tx: &mut Tx) {
    if let Some((thread, message)) = s.editing.take()
        && s.selected.as_deref() == Some(thread.as_str())
    {
        rewind(s, &thread, &message);
    }
    let Some(thread) = ensure_thread(s, tx) else {
        return;
    };
    let busy = s.waiting_worktree.contains(&thread) || s.turns.contains_key(&thread);
    // A stopped turn leaves its queue; what was queued first still goes first.
    if busy || s.queued.get(&thread).is_some_and(|q| !q.is_empty()) {
        if let Some(t) = s.ws.thread_mut(&thread)
            && t.messages.is_empty()
        {
            t.title = oxlet_core::model::title_from_text(text);
        }
        s.queued
            .entry(thread.clone())
            .or_default()
            .push_back(text.to_owned());
        if !busy {
            start_next(s, &thread, tx);
        }
    } else {
        begin(s, &thread, text, tx);
    }
    mark_dirty(s, tx);
}

/// Adds `text` to the transcript and starts the turn that answers it.
fn begin(s: &mut App, thread: &str, text: &str, tx: &mut Tx) {
    s.ws.push_message(
        thread,
        Role::User,
        vec![Part::Text {
            text: text.to_owned(),
        }],
        now(),
    );
    start_turn(s, thread, text, tx);
}

fn start_next(s: &mut App, thread: &str, tx: &mut Tx) {
    if let Some(next) = s.queued.get_mut(thread).and_then(|q| q.pop_front()) {
        begin(s, thread, &next, tx);
    }
}

fn ensure_thread(s: &mut App, tx: &mut Tx) -> Option<String> {
    if let Some(id) = s.selected.clone() {
        return Some(id);
    }
    let Some(project) = s.current_project().cloned() else {
        notice(s, tx, "Add a project first");
        s.adding_project = Some(AddMode::Path);
        return None;
    };
    let provider = default_provider(s, &project);
    let model = match s.draft_model.take() {
        Some((p, m)) if p == provider => m,
        _ => String::new(),
    };
    let id = s.ws.create_thread(&project.id, provider, &model, now());
    if let Some(t) = s.ws.thread_mut(&id) {
        t.effort = std::mem::take(&mut s.draft_effort);
        t.mode = std::mem::take(&mut s.draft_mode);
    }
    s.selected = Some(id.clone());
    if s.placement == Placement::Worktree {
        let slug = format!("oxlet/{id}");
        let root = store::expand_home(&s.ws.settings.worktree_root);
        let path = format!("{root}/{}/{id}", project.name);
        s.waiting_worktree.insert(id.clone());
        let (repo, thread) = (project.at(&project.path), id.clone());
        let name = project.name.clone();
        s.bridge.spawn(move |b| {
            // The worktree root setting is a local folder; WSL and SSH projects keep theirs in
            // the target user's home.
            let path = if repo.target == Target::Local {
                std::fs::create_dir_all(Path::new(&path).parent().unwrap_or(Path::new(&path)))
                    .map(|_| path)
                    .map_err(|e| e.to_string())
            } else {
                target::make_home_dir(&repo.target, &format!(".oxlet/worktrees/{name}"))
                    .map(|dir| format!("{dir}/{thread}"))
            };
            let result =
                path.and_then(|path| git::worktree_add(&repo, &path, &slug).map(|_| (path, slug)));
            b.send(Event::WorktreeReady { thread, result });
        });
    } else if let Some(t) = s.ws.thread_mut(&id) {
        t.branch = None;
    }
    Some(id)
}

/// The provider a new thread in `project` starts with.
pub fn default_provider(s: &App, project: &Project) -> ProviderId {
    project
        .default_provider
        .or(s.ws.settings.default_provider)
        .unwrap_or_else(|| first_installed(s, &project.target))
}

fn first_installed(s: &App, on: &Target) -> ProviderId {
    s.providers
        .get(on)
        .and_then(|found| found.iter().find(|(_, b)| b.is_some()))
        .map_or(ProviderId::Claude, |(id, _)| *id)
}

/// Drops the thread's agent session, so its next turn starts a fresh one. A turn still running
/// may report the old session afterwards; the epoch tells it apart.
pub fn reset_session(s: &mut App, thread: &str) {
    if let Some(t) = s.ws.thread_mut(thread) {
        t.session = None;
    }
    *s.epochs.entry(thread.to_owned()).or_default() += 1;
}

fn epoch(s: &App, thread: &str) -> u64 {
    s.epochs.get(thread).copied().unwrap_or_default()
}

pub fn start_turn(s: &mut App, thread: &str, prompt: &str, tx: &mut Tx) {
    let Some(t) = s.ws.thread(thread).cloned() else {
        return;
    };
    let Some(project) = s.ws.project(&t.project).cloned() else {
        return;
    };
    let message =
        s.ws.push_message(thread, Role::Assistant, Vec::new(), now())
            .unwrap_or_default();
    if let Some(m) =
        s.ws.thread_mut(thread)
            .and_then(|t| t.messages.iter_mut().find(|m| m.id == message))
    {
        m.agent = Some((t.provider, t.model.clone()));
    }
    // Matched on the text: a queued compaction is a plain queued message.
    let compact = prompt == slash::COMPACT_PROMPT;
    let info = t.provider.info();
    let program = match (
        s.binary(&project.target, t.provider),
        project.target.label(),
    ) {
        (Some(Some(path)), _) => path,
        // Until detection there answers, the target's login shell finds the CLI itself.
        (None, Some(_)) => PathBuf::from(info.binary),
        (_, place) => {
            let error = match place {
                None => format!(
                    "{} is not installed. Install {} and make sure `{}` is on your PATH, or set its path in Settings → Agent providers.",
                    info.name, info.runtime, info.binary
                ),
                Some(place) => format!(
                    "{} is not installed on {place}. Install {} there and make sure `{}` is on the login shell's PATH.",
                    info.name, info.runtime, info.binary
                ),
            };
            finish_message(s, thread, &message, Some(error), None, false, false);
            mark_dirty(s, tx);
            return;
        }
    };
    let mut prompt = match &t.goal {
        Some(goal) => format!("{prompt}\n\n(Thread goal: {goal})"),
        None => prompt.to_owned(),
    };
    if t.session.is_none()
        && let Some(context) = handoff(&t.messages[..t.messages.len().saturating_sub(1)])
    {
        prompt = format!("{context}\n\n---\n\n{prompt}");
    }
    let req = TurnRequest {
        prompt,
        model: t.model.clone(),
        effort: t.effort,
        access: t.access,
        mode: t.mode,
        session: t.session.clone(),
        place: project.at(t.cwd(&project)),
    };
    let bridge = s.bridge.clone();
    let id = thread.to_owned();
    match process::spawn_turn(t.provider, &program, &req, move |event| {
        bridge.send(Event::Agent {
            thread: id.clone(),
            event,
        })
    }) {
        Ok(running) => {
            s.turns.insert(
                thread.to_owned(),
                Turn {
                    running,
                    started: now(),
                    message,
                    edited: BTreeSet::new(),
                    live: None,
                    epoch: epoch(s, thread),
                    compact,
                },
            );
            if let Some(t) = s.ws.thread_mut(thread) {
                t.status = ThreadStatus::Working;
            }
        }
        Err(e) => finish_message(
            s,
            thread,
            &message,
            Some(format!("Could not start {}: {e}", info.binary)),
            None,
            false,
            false,
        ),
    }
    mark_dirty(s, tx);
}

const HANDOFF_CHARS: usize = 12_000;

/// The conversation since the last fresh start, for a turn that has no session to resume: the
/// first turn after switching provider, or after a fork. Newest messages are kept when it is long;
/// the newest is cut to fit rather than left out.
pub fn handoff(history: &[oxlet_core::model::Message]) -> Option<String> {
    let start = history
        .iter()
        .rposition(|m| m.role == Role::System)
        .map_or(0, |i| i + 1);
    let mut lines: Vec<String> = Vec::new();
    let mut size = 0;
    for m in history[start..].iter().rev() {
        let text = m.text();
        if text.trim().is_empty() || m.role == Role::System {
            continue;
        }
        let who = if m.role == Role::User {
            "User"
        } else {
            "Assistant"
        };
        let mut entry = format!("{who}: {text}");
        if lines.is_empty() && entry.len() > HANDOFF_CHARS {
            entry.truncate(entry.floor_char_boundary(HANDOFF_CHARS));
            entry.push('…');
        } else if size + entry.len() > HANDOFF_CHARS {
            break;
        }
        size += entry.len();
        lines.push(entry);
    }
    if !lines.iter().any(|l| l.starts_with("Assistant:")) {
        return None;
    }
    lines.reverse();
    Some(format!(
        "You are continuing a conversation that began in another session. Here it is:\n\n{}",
        lines.join("\n\n")
    ))
}

/// Loads a sent message into the composer; sending it again rewinds the thread to that point.
pub fn edit_message(s: &mut App, thread: &str, message: &str) {
    let Some(text) =
        s.ws.thread(thread)
            .and_then(|t| t.messages.iter().find(|m| m.id == message))
            .map(|m| m.text())
    else {
        return;
    };
    s.composer = TextEdit::new(text);
    s.composer.focused = true;
    s.editing = Some((thread.to_owned(), message.to_owned()));
}

pub fn cancel_edit(s: &mut App) {
    if s.editing.take().is_some() {
        s.composer = TextEdit::new("");
        s.composer.focused = true;
    }
}

/// Drops `message` and everything after it. The agent's session remembers the dropped turns, so
/// the next turn starts a fresh one, handed the history that remains.
fn rewind(s: &mut App, thread: &str, message: &str) {
    if let Some(t) = s.ws.thread_mut(thread)
        && let Some(i) = t.messages.iter().position(|m| m.id == message)
    {
        t.messages.truncate(i);
        t.status = ThreadStatus::Idle;
        reset_session(s, thread);
    }
}

/// Runs the last prompt again in place of the latest reply, in a fresh session: the old one
/// holds the reply being replaced.
pub fn retry(s: &mut App, thread: &str, tx: &mut Tx) {
    if s.turns.contains_key(thread) {
        return;
    }
    let Some(t) = s.ws.thread_mut(thread) else {
        return;
    };
    let Some(user) = t.messages.iter().rposition(|m| m.role == Role::User) else {
        return;
    };
    let prompt = t.messages[user].text();
    t.messages.truncate(user + 1);
    reset_session(s, thread);
    start_turn(s, thread, &prompt, tx);
}

pub fn open_in_editor(s: &mut App, index: usize, tx: &mut Tx) {
    s.popup = None;
    let (Some(editor), Some(at)) = (s.editors.get(index).cloned(), s.place()) else {
        return;
    };
    if let Err(e) = oxlet_core::open::in_editor(&editor, &at) {
        notice(s, tx, format!("Could not open {}: {e}", editor.name));
    }
}

pub fn reveal(s: &mut App, tx: &mut Tx) {
    s.popup = None;
    let Some(path) = s.place().and_then(|at| at.local_path()) else {
        return;
    };
    if let Err(e) = oxlet_core::open::reveal(&path.to_string_lossy()) {
        notice(
            s,
            tx,
            format!("Could not open {}: {e}", oxlet_core::open::file_manager()),
        );
    }
}

/// Mentions dropped files in the composer. Dropped files are local, so only local projects
/// take them.
pub fn drop_files(s: &mut App, paths: &[PathBuf]) {
    let Some(root) = s
        .place()
        .filter(|at| at.target == Target::Local)
        .map(|at| at.cwd)
    else {
        return;
    };
    let mut text = s.composer.text.trim_end().to_owned();
    if !text.is_empty() {
        text.push(' ');
    }
    text.push_str(&oxlet_core::open::mentions(paths, &root));
    text.push(' ');
    s.composer = TextEdit::new(text);
    s.composer.focused = true;
}

/// Adds every dropped folder as a project.
pub fn drop_projects(s: &mut App, paths: &[PathBuf], tx: &mut Tx) {
    for dir in paths.iter().filter(|p| p.is_dir()) {
        add_project(s, &dir.to_string_lossy(), tx);
    }
}

pub fn stop_turn(s: &mut App, thread: &str) {
    if let Some(turn) = s.turns.get(thread) {
        turn.running.stop();
    }
}

/// Ends a reply. `edited` is whether this turn changed files; a turn the user stopped posts no
/// notification.
fn finish_message(
    s: &mut App,
    thread: &str,
    message: &str,
    error: Option<String>,
    worked: Option<u64>,
    edited: bool,
    stopped: bool,
) {
    let selected = s.selected.as_deref() == Some(thread);
    let Some(t) = s.ws.thread_mut(thread) else {
        return;
    };
    if let Some(m) = t.messages.iter_mut().find(|m| m.id == message) {
        if let Some(e) = &error {
            m.parts.push(Part::Error { message: e.clone() });
        }
        m.worked_ms = worked;
    }
    t.status = match (&error, t.status) {
        (Some(_), _) => ThreadStatus::Failed,
        _ if edited => ThreadStatus::Review,
        _ => ThreadStatus::Idle,
    };
    t.updated = now();
    t.unread = !selected;
    let (title, failed) = (t.title.clone(), t.status == ThreadStatus::Failed);
    if s.ws.settings.notify_on_finish && !s.demo && !stopped && (!selected || !s.window_focused) {
        let body = if failed {
            "Stopped with an error"
        } else {
            "Finished"
        };
        rune_notify::post(
            &rune_notify::Notification::new(format!("oxlet-{thread}"), title)
                .body(body)
                .info("thread", thread),
        );
    }
}

pub fn apply_agent(s: &mut App, thread: &str, event: AgentEvent, tx: &mut Tx) {
    let Some(message) = s.turns.get(thread).map(|t| t.message.clone()) else {
        return;
    };
    match event {
        AgentEvent::Session(id) => {
            let current = s
                .turns
                .get(thread)
                .is_some_and(|t| t.epoch == epoch(s, thread));
            if current && let Some(t) = s.ws.thread_mut(thread) {
                t.session = Some(id);
            }
        }
        AgentEvent::Notice(text) => notice(s, tx, text),
        AgentEvent::Delta(piece) => {
            let live = s.turns.get(thread).and_then(|t| t.live);
            let mut opened = None;
            with_message(s, thread, &message, |parts| {
                match live.and_then(|i| parts.get_mut(i)) {
                    Some(Part::Text { text }) => text.push_str(&piece),
                    _ => {
                        parts.push(Part::Text { text: piece });
                        opened = Some(parts.len() - 1);
                    }
                }
            });
            if let (Some(i), Some(turn)) = (opened, s.turns.get_mut(thread)) {
                turn.live = Some(i);
            }
            return;
        }
        AgentEvent::Text(text) => {
            let live = s.turns.get_mut(thread).and_then(|t| t.live.take());
            with_message(s, thread, &message, |parts| {
                if text.trim().is_empty() {
                    // A block of only whitespace settles its deltas by dropping them.
                    if let Some(i) =
                        live.filter(|&i| matches!(parts.get(i), Some(Part::Text { .. })))
                    {
                        parts.remove(i);
                    }
                } else if let Some(slot) = live.and_then(|i| parts.get_mut(i)) {
                    *slot = Part::Text { text };
                } else if let Some(Part::Text { text: last }) = parts.last_mut() {
                    last.push_str("\n\n");
                    last.push_str(&text);
                } else {
                    parts.push(Part::Text { text });
                }
            })
        }
        AgentEvent::Tool { name, summary } => with_message(s, thread, &message, |parts| {
            parts.push(Part::Tool {
                name,
                summary,
                output: String::new(),
            })
        }),
        AgentEvent::ToolOutput(out) => with_message(s, thread, &message, |parts| {
            if let Some(Part::Tool { output, .. }) = parts
                .iter_mut()
                .rev()
                .find(|p| matches!(p, Part::Tool { .. }))
            {
                *output = out.chars().take(8000).collect();
            }
        }),
        AgentEvent::Usage(usage) => {
            if usage.tokens() > 0
                && let Some(m) =
                    s.ws.thread_mut(thread)
                        .and_then(|t| t.messages.iter_mut().find(|m| m.id == message))
            {
                m.usage = Some(usage);
            }
        }
        AgentEvent::Edited(path) => {
            if let Some(turn) = s.turns.get_mut(thread) {
                turn.edited.insert(path);
            }
        }
        AgentEvent::Done { duration_ms, error } => {
            let Some(turn) = s.turns.remove(thread) else {
                return;
            };
            let worked = duration_ms.unwrap_or_else(|| now().saturating_sub(turn.started));
            let stopped = turn.running.was_stopped();
            let error = if stopped { None } else { error };
            if turn.compact && error.is_none() && !stopped {
                compacted(s, thread, &message);
            }
            let edited = !turn.edited.is_empty();
            finish_message(s, thread, &message, error, Some(worked), edited, stopped);
            if edited {
                collect_edits(s, thread, &message, turn.edited);
            }
            if s.selected.as_deref() == Some(thread) {
                refresh_repo(s);
            }
            if !stopped {
                start_next(s, thread, tx);
            }
        }
    }
    mark_dirty(s, tx);
}

/// After a compaction: the next turn starts a fresh session, handed the conversation from the
/// marker on, which is the summary and the prompt that asked for it.
fn compacted(s: &mut App, thread: &str, reply: &str) {
    if let Some(t) = s.ws.thread_mut(thread)
        && let Some(i) = t.messages.iter().position(|m| m.id == reply)
    {
        t.messages.insert(
            i.saturating_sub(1),
            system_message(
                "compact",
                "Compacted. The next message starts a fresh session from this summary.",
            ),
        );
    }
    reset_session(s, thread);
}

fn system_message(kind: &str, text: &str) -> oxlet_core::model::Message {
    oxlet_core::model::Message {
        id: format!("{kind}{}", now()),
        role: Role::System,
        parts: vec![Part::Text { text: text.into() }],
        at: now(),
        worked_ms: None,
        pinned: false,
        agent: None,
        usage: None,
    }
}

fn with_message(s: &mut App, thread: &str, message: &str, f: impl FnOnce(&mut Vec<Part>)) {
    if let Some(m) =
        s.ws.thread_mut(thread)
            .and_then(|t| t.messages.iter_mut().find(|m| m.id == message))
    {
        f(&mut m.parts);
    }
}

fn collect_edits(s: &App, thread: &str, message: &str, edited: BTreeSet<String>) {
    let Some(at) =
        s.ws.thread(thread)
            .and_then(|t| s.ws.project(&t.project).map(|p| p.at(t.cwd(p))))
    else {
        return;
    };
    let (thread, message) = (thread.to_owned(), message.to_owned());
    s.bridge.spawn(move |b| {
        let slash = |p: &str| p.replace('\\', "/");
        let prefix = format!("{}/", slash(&at.cwd).trim_end_matches('/'));
        let wanted: BTreeSet<String> = edited
            .iter()
            .map(|p| {
                let p = slash(p);
                p.strip_prefix(&prefix).map(str::to_owned).unwrap_or(p)
            })
            .collect();
        let mut files: Vec<FileChange> = git::numstat(&at)
            .unwrap_or_default()
            .into_iter()
            .filter(|c| wanted.contains(&c.path))
            .collect();
        let missing: Vec<String> = wanted
            .into_iter()
            .filter(|path| !files.iter().any(|f| &f.path == path))
            .collect();
        // A tracked file with no change against HEAD was committed during the turn, or put
        // back: it is not new.
        let tracked = git::tracked(&at, &missing).unwrap_or_default();
        let new: Vec<String> = missing
            .into_iter()
            .filter(|path| !tracked.contains(path))
            .collect();
        let texts = target::read_texts(&at, &new, u64::from(u32::MAX), u64::from(u32::MAX));
        for (path, text) in new.into_iter().zip(texts) {
            files.push(FileChange {
                path,
                added: text.lines().count() as u32,
                removed: 0,
            });
        }
        b.send(Event::TurnEdits {
            thread,
            message,
            files,
        });
    });
}

pub fn apply_edits(s: &mut App, thread: &str, message: &str, files: Vec<FileChange>, tx: &mut Tx) {
    with_message(s, thread, message, |parts| {
        parts.push(Part::Edits { files })
    });
    if let Some(t) = s.ws.thread_mut(thread)
        && t.status == ThreadStatus::Idle
    {
        t.status = ThreadStatus::Review;
    }
    mark_dirty(s, tx);
}

pub fn worktree_ready(
    s: &mut App,
    thread: &str,
    result: Result<(String, String), String>,
    tx: &mut Tx,
) {
    s.waiting_worktree.remove(thread);
    match result {
        Ok((path, branch)) => {
            if let Some(t) = s.ws.thread_mut(thread) {
                t.worktree = Some(path);
                t.branch = Some(branch);
            }
        }
        Err(e) => notice(
            s,
            tx,
            format!("Worktree failed, running in the project folder: {e}"),
        ),
    }
    start_next(s, thread, tx);
    refresh_repo(s);
    mark_dirty(s, tx);
}

pub fn run_command(s: &mut App, command: Command, arg: &str, tx: &mut Tx) {
    let id = s.selected.clone();
    let needs_thread = !matches!(
        command,
        Command::Automation | Command::Plan | Command::Debug | Command::Default | Command::Review
    );
    if needs_thread && id.is_none() {
        notice(
            s,
            tx,
            format!("/{} needs an open thread", slash::spec(command).name),
        );
        return;
    }
    let id = id.unwrap_or_default();
    match command {
        Command::Fork | Command::Side => {
            if let Some(new) = s.ws.fork_thread(&id, now()) {
                if command == Command::Side
                    && let Some(t) = s.ws.thread_mut(&new)
                {
                    t.title = format!("Side: {}", t.title.trim_end_matches(" (fork)"));
                }
                select_thread(s, &new, tx);
            }
        }
        Command::Export => {
            if let Some(t) = s.ws.thread(&id).cloned() {
                let dir = s.dir.join("exports");
                s.bridge.spawn(move |b| {
                    let text = match export::write(&t, &dir) {
                        Ok(p) => format!("Exported to {}", p.display()),
                        Err(e) => format!("Export failed: {e}"),
                    };
                    b.send(Event::Notice(text));
                });
            }
        }
        Command::Goal => {
            if let Some(t) = s.ws.thread_mut(&id) {
                t.goal = (!arg.is_empty()).then(|| arg.to_owned());
            }
            notice(
                s,
                tx,
                if arg.is_empty() {
                    "Goal cleared".to_owned()
                } else {
                    format!("Goal set: {arg}")
                },
            );
        }
        Command::Plan | Command::Debug | Command::Default => {
            let mode = match command {
                Command::Plan => ChatMode::Plan,
                Command::Debug => ChatMode::Debug,
                _ => ChatMode::Default,
            };
            match s.selected.clone().and_then(|id| s.ws.thread_mut(&id)) {
                Some(t) => t.mode = mode,
                None => s.draft_mode = mode,
            }
            notice(s, tx, format!("{} on", slash::spec(command).title));
            if !arg.is_empty() {
                send_prompt(s, arg, tx);
            }
        }
        Command::Review => send_prompt(
            s,
            if arg.is_empty() {
                slash::REVIEW_PROMPT
            } else {
                arg
            },
            tx,
        ),
        Command::Compact => send_prompt(s, slash::COMPACT_PROMPT, tx),
        Command::Clear => {
            reset_session(s, &id);
            if let Some(t) = s.ws.thread_mut(&id) {
                t.messages.push(system_message(
                    "clear",
                    "Session cleared. The next message starts fresh.",
                ));
            }
        }
        Command::Model => s.popup = Some(crate::state::Popup::Model),
        Command::Status => {
            if let Some(t) = s.ws.thread(&id) {
                let text = format!(
                    "{} · {} · {} · session {}",
                    t.provider.info().name,
                    t.provider.model_label(&t.model),
                    t.access.label(),
                    t.session.as_deref().unwrap_or("not started")
                );
                notice(s, tx, text);
            }
        }
        Command::Rename => {
            if !arg.is_empty() {
                if let Some(t) = s.ws.thread_mut(&id) {
                    t.title = arg.to_owned();
                }
            } else if let Some(t) = s.ws.thread(&id) {
                s.rename = TextEdit::new(t.title.clone());
                s.rename.focused = true;
                s.renaming = Some(id.clone());
            }
        }
        Command::Automation => {
            s.auto_prompt = TextEdit::new(arg.to_owned());
            s.auto_project = s.current_project().map(|p| p.id.clone());
            go(s, Screen::Automations);
        }
    }
    mark_dirty(s, tx);
}

/// Marks the repository as changed, and reads it again if it is on screen.
pub fn refresh_repo(s: &mut App) {
    let generation = s.repo.generation;
    let Some(cwd) = s.place() else {
        s.repo = crate::state::Repo {
            generation,
            ..Default::default()
        };
        return;
    };
    ensure_providers(s, &cwd.target);
    if s.repo.cwd != cwd {
        s.repo = crate::state::Repo {
            cwd,
            generation,
            ..Default::default()
        };
    }
    s.repo.fresh = false;
    if !sync_repo(s) && !s.repo.reading {
        s.repo.loading = false;
    }
}

/// What the screen shows of the repository: `Some(true)` for the dock's changes, files or git,
/// `Some(false)` for only the branch on the new-thread screen.
fn repo_shown(s: &App) -> Option<bool> {
    if s.screen != Screen::Thread {
        return None;
    }
    let dock =
        s.dock.is_open() && matches!(s.dock_tab, DockTab::Changes | DockTab::Files | DockTab::Git);
    (dock || s.selected.is_none()).then_some(dock)
}

/// Starts a read of the repository when what it holds is stale or less than the screen shows,
/// and none runs. Returns whether it started one.
pub fn sync_repo(s: &mut App) -> bool {
    let Some(full) = repo_shown(s) else {
        return false;
    };
    if s.repo.fresh && (s.repo.full || !full) || s.repo.reading || s.place().is_none() {
        return false;
    }
    s.repo.fresh = true;
    s.repo.full = full;
    s.repo.reading = true;
    s.repo.loading = true;
    s.repo.generation += 1;
    let (cwd, generation) = (s.repo.cwd.clone(), s.repo.generation);
    let c = cwd.clone();
    s.bridge.spawn(move |b| {
        let result = snapshot(&c, full);
        b.send(Event::Repo {
            cwd: c,
            generation,
            result,
        });
    });
    if full {
        s.bridge.spawn(move |b| {
            let result = git::files(&cwd).map(|files| (files.len(), file_tree(files)));
            b.send(Event::Files {
                cwd,
                generation,
                result,
            });
        });
    }
    true
}

/// The repository's status, and with `full` what the dock shows of it.
fn snapshot(cwd: &Place, full: bool) -> Result<RepoSnapshot, String> {
    if !git::is_repo(cwd)? {
        return Err("Not a git repository".into());
    }
    let status = git::status(cwd)?;
    if !full {
        return Ok(RepoSnapshot {
            status,
            changes: Vec::new(),
            patch: String::new(),
            untracked: Vec::new(),
            log: Vec::new(),
            branches: Vec::new(),
        });
    }
    let patch = git::diff(cwd)?;
    let mut changes = git::numstat(cwd)?;
    let mut untracked = Vec::new();
    let paths: Vec<String> = status
        .entries
        .iter()
        .filter(|e| e.untracked())
        .map(|e| e.path.clone())
        .collect();
    let shown = paths.len().min(MAX_UNTRACKED_FILES);
    let mut texts = target::read_texts(
        cwd,
        &paths[..shown],
        MAX_UNTRACKED_PREVIEW,
        MAX_UNTRACKED_BYTES,
    );
    texts.resize(paths.len(), String::new());
    for (path, text) in paths.into_iter().zip(texts) {
        changes.push(FileChange {
            path: path.clone(),
            added: text.lines().count() as u32,
            removed: 0,
        });
        untracked.push((path, text));
    }
    Ok(RepoSnapshot {
        status,
        changes,
        patch,
        untracked,
        log: git::log(cwd, 30).unwrap_or_default(),
        branches: git::branches(cwd).unwrap_or_default(),
    })
}

pub fn apply_repo(
    s: &mut App,
    cwd: Place,
    generation: u64,
    result: Result<RepoSnapshot, String>,
    tx: &mut Tx,
) {
    if s.repo.cwd != cwd || s.repo.generation != generation {
        return;
    }
    s.repo.reading = false;
    s.repo.loading = false;
    match result {
        Ok(snap) if !s.repo.full => {
            s.repo.error = None;
            s.repo.status = snap.status;
        }
        Ok(snap) => {
            s.repo.error = None;
            s.repo.status = snap.status;
            s.repo.changes = snap.changes;
            s.repo.log = snap.log;
            s.repo.branches = snap.branches;
            let mode = s.repo.diff.mode();
            let mut files = diff::parse(&snap.patch);
            files.extend(
                snap.untracked
                    .iter()
                    .map(|(path, text)| FileDiff::from_texts(path.clone(), "", text)),
            );
            s.repo.diff = DiffState::new(files);
            s.repo.diff.set_mode(mode);
        }
        Err(e) => s.repo.error = Some(e),
    }
    if s.repo.fresh
        && let Some(path) = s.repo.reveal.take()
        && let Some(i) = s.repo.diff.file_index(&path)
    {
        s.repo.diff.scroll_to_file(i, tx);
    }
}

pub fn apply_files(
    s: &mut App,
    cwd: Place,
    generation: u64,
    result: Result<(usize, TreeModel<FileNode<String>>), String>,
) {
    if s.repo.cwd != cwd || s.repo.generation != generation {
        return;
    }
    if let Ok((count, files)) = result {
        s.repo.file_count = count;
        s.repo.files = files;
    }
}

pub fn git_action(s: &mut App, what: &'static str, tx: &mut Tx) {
    let Some(cwd) = s.place() else { return };
    let message = s.commit_message.text.trim().to_owned();
    let commits = what.starts_with("commit");
    if commits && message.is_empty() {
        notice(s, tx, "Write a commit message first");
        return;
    }
    s.repo.loading = true;
    s.bridge.spawn(move |b| {
        let commit = || git::commit_all(&cwd, &message);
        let (result, committed) = match what {
            "commit" => {
                let r = commit().map(|h| format!("Committed {h}"));
                let ok = r.is_ok();
                (r, ok)
            }
            "commit-push" => match commit() {
                Ok(h) => (
                    git::push(&cwd)
                        .map(|_| format!("Committed {h} and pushed"))
                        .map_err(|e| format!("Committed {h}, but the push failed: {e}")),
                    true,
                ),
                Err(e) => (Err(e), false),
            },
            "push" => (git::push(&cwd).map(|_| "Pushed".to_owned()), false),
            "pr" => (git::create_pull_request(&cwd), false),
            _ => (Ok(String::new()), false),
        };
        b.send(Event::GitDone {
            cwd,
            result,
            committed,
        });
    });
}

pub fn checkout(s: &mut App, branch: String, tx: &mut Tx) {
    let Some(cwd) = s.place() else { return };
    s.bridge.spawn(move |b| {
        let result = git::checkout(&cwd, &branch, false).map(|_| format!("Switched to {branch}"));
        b.send(Event::GitDone {
            cwd,
            result,
            committed: false,
        });
    });
    s.popup = None;
    let _ = tx;
}

pub fn discard(s: &mut App, path: String) {
    let Some(cwd) = s.place() else { return };
    let entry = s
        .repo
        .status
        .entries
        .iter()
        .find(|e| e.path == path)
        .cloned()
        .unwrap_or(git::Entry {
            path,
            code: " M".into(),
            from: None,
        });
    s.bridge.spawn(move |b| {
        let result = git::discard(&cwd, &entry).map(|_| format!("Discarded {}", entry.path));
        b.send(Event::GitDone {
            cwd,
            result,
            committed: false,
        });
    });
}

/// Runs the action the confirmation dialog asked about, judged on the state now.
pub fn confirmed(s: &mut App, tx: &mut Tx) {
    let Some(pending) = s.confirm.take() else {
        return;
    };
    match pending {
        Pending::Discard(path) => discard(s, path),
        Pending::RemoveWorktree(thread) => {
            let Some((repo, path)) = s.ws.thread(&thread).and_then(|t| {
                let p = s.ws.project(&t.project)?;
                Some((p.at(&p.path), t.worktree.clone()?))
            }) else {
                return;
            };
            if let Some(t) = s.ws.threads.iter().find(|t| {
                t.worktree.as_deref() == Some(path.as_str())
                    && (s.turns.contains_key(&t.id) || s.waiting_worktree.contains(&t.id))
            }) {
                let text = format!(
                    "Stop \u{201c}{}\u{201d} before removing its worktree",
                    t.title
                );
                notice(s, tx, text);
                return;
            }
            remove_worktree(s, repo, path);
        }
        Pending::DeleteThread(id) => delete_threads(s, &[id], tx),
        Pending::DeleteAutomation(id) => {
            s.ws.automations.retain(|a| a.id != id);
            mark_dirty(s, tx);
        }
        Pending::RestoreAppearance => {
            crate::settings::restore_appearance(s);
            mark_dirty(s, tx);
        }
        Pending::RemoveProject(id) => remove_project(s, &id, tx),
    }
}

/// Removes a managed worktree; the threads in it move to the project folder once git is done.
/// Its branch stays, with any commits on it.
fn remove_worktree(s: &App, repo: Place, path: String) {
    s.bridge.spawn(move |b| {
        let result = git::worktree_remove(&repo, &path);
        b.send(Event::WorktreeRemoved { path, result });
    });
}

pub fn worktree_removed(s: &mut App, path: &str, result: Result<(), String>, tx: &mut Tx) {
    if let Err(e) = result {
        notice(s, tx, format!("Could not remove worktree: {e}"));
        return;
    }
    for t in
        s.ws.threads
            .iter_mut()
            .filter(|t| t.worktree.as_deref() == Some(path))
    {
        let t = std::sync::Arc::make_mut(t);
        t.worktree = None;
        t.branch = None;
    }
    notice(s, tx, format!("Removed {path}"));
    refresh_repo(s);
    mark_dirty(s, tx);
}

/// Deletes threads for good: their turns stop, and a worktree only they used is removed.
fn delete_threads(s: &mut App, ids: &[Id], tx: &mut Tx) {
    for id in ids {
        if let Some(turn) = s.turns.remove(id) {
            turn.running.stop();
        }
        s.queued.remove(id);
        s.waiting_worktree.remove(id);
    }
    let gone = |id: &Id| ids.contains(id);
    let mut worktrees: Vec<(Place, String)> = Vec::new();
    for t in s.ws.threads.iter().filter(|t| gone(&t.id)) {
        if let (Some(path), Some(p)) = (&t.worktree, s.ws.project(&t.project))
            && !s
                .ws
                .threads
                .iter()
                .any(|o| !gone(&o.id) && o.worktree.as_ref() == Some(path))
            && !worktrees.iter().any(|(_, w)| w == path)
        {
            worktrees.push((p.at(&p.path), path.clone()));
        }
    }
    s.ws.threads.retain(|t| !gone(&t.id));
    for a in &mut s.ws.automations {
        a.runs.retain(|r| !gone(r));
    }
    if s.editing.as_ref().is_some_and(|(t, _)| gone(t)) {
        cancel_edit(s);
    }
    if s.renaming.as_ref().is_some_and(gone) {
        s.renaming = None;
    }
    if s.notes_for.as_ref().is_some_and(gone) {
        s.notes_for = None;
    }
    if s.thread_menu.as_ref().is_some_and(|(t, _)| gone(t)) {
        s.thread_menu = None;
    }
    if s.selected.as_ref().is_some_and(gone) {
        s.selected = s.ws.recent().first().map(|t| t.id.clone());
        refresh_repo(s);
    }
    for (repo, path) in worktrees {
        remove_worktree(s, repo, path);
    }
    mark_dirty(s, tx);
}

/// Forgets a project, its threads and its automations. Its folder stays as it is.
fn remove_project(s: &mut App, id: &str, tx: &mut Tx) {
    let threads: Vec<Id> =
        s.ws.threads
            .iter()
            .filter(|t| t.project == id)
            .map(|t| t.id.clone())
            .collect();
    delete_threads(s, &threads, tx);
    s.ws.projects.retain(|p| p.id != id);
    s.ws.automations.retain(|a| a.project != id);
    if s.draft_project.as_deref() == Some(id) {
        s.draft_project = s.ws.projects.first().map(|p| p.id.clone());
    }
    if s.auto_project.as_deref() == Some(id) {
        s.auto_project = None;
    }
    if s.renaming.as_deref() == Some(id) {
        s.renaming = None;
    }
    s.project_menu = None;
    refresh_repo(s);
}

pub fn load_prs(s: &mut App) {
    s.prs = None;
    let Some(cwd) = s.place() else {
        s.prs = Some(Err("Add a project to see its pull requests".into()));
        return;
    };
    s.bridge
        .spawn(move |b| b.send(Event::Prs(git::pull_requests(&cwd))));
}

pub fn run_terminal(s: &mut App, tx: &mut Tx) {
    let command = s.terminal.input.text.trim().to_owned();
    if command.is_empty() {
        return;
    }
    let Some(cwd) = s.place() else {
        notice(s, tx, "Add a project first");
        return;
    };
    if let Some(r) = s.terminal.running.take() {
        r.stop();
    }
    s.terminal.input = TextEdit::new("");
    s.terminal.input.focused = true;
    s.terminal.history.push(command.clone());
    s.terminal.history_at = None;
    s.terminal.lines.push((false, format!("$ {command}")));
    s.terminal.run += 1;
    let (bridge, run) = (s.bridge.clone(), s.terminal.run);
    match process::spawn_shell(&command, &cwd, move |l| bridge.send(Event::Term(run, l))) {
        Ok(r) => s.terminal.running = Some(r),
        Err(e) => s.terminal.lines.push((true, e.to_string())),
    }
}

pub fn apply_terminal(s: &mut App, run: u64, line: Line) {
    if run != s.terminal.run {
        return;
    }
    match line {
        Line::Out(t) => s.terminal.lines.push((false, t)),
        Line::Err(t) => s.terminal.lines.push((true, t)),
        Line::Exit(code) => {
            s.terminal.running = None;
            if code != Some(0) {
                s.terminal.lines.push((
                    true,
                    format!("exit {}", code.map_or("signal".into(), |c| c.to_string())),
                ));
            }
        }
    }
    let excess = s.terminal.lines.len().saturating_sub(MAX_TERMINAL_LINES);
    s.terminal.lines.drain(..excess);
}

pub fn run_due_automations(s: &mut App, tx: &mut Tx) {
    let now = now();
    let due: Vec<usize> = (0..s.ws.automations.len())
        .filter(|&i| schedule::is_due(&s.ws.automations[i], now, s.offset))
        .collect();
    for i in due {
        run_automation(s, i, tx);
    }
}

pub fn run_automation(s: &mut App, i: usize, tx: &mut Tx) {
    let a = s.ws.automations[i].clone();
    if s.ws.project(&a.project).is_none() {
        return;
    }
    let id = s.ws.create_thread(&a.project, a.provider, &a.model, now());
    s.ws.push_message(
        &id,
        Role::User,
        vec![Part::Text {
            text: a.prompt.clone(),
        }],
        now(),
    );
    if let Some(t) = s.ws.thread_mut(&id) {
        t.title = a.name.clone();
    }
    s.ws.automations[i].last_run = Some(now());
    s.ws.automations[i].runs.push(id.clone());
    start_turn(s, &id, &a.prompt, tx);
    mark_dirty(s, tx);
}

pub fn save_now(s: &mut App) {
    s.dirty = false;
    if s.demo {
        return;
    }
    s.saver.save(s.ws.clone());
}
