use std::collections::HashMap;
use std::rc::Rc;

use rune::{El, Key, ScrollSpec, TextEdit, col, icon, row, text};
use rune_kit::diff::{DiffMode, DiffView};
use rune_kit::{
    Button, ButtonVariant, ControlSize, IconButton, IconButtonVariant, Kit, Spinner, TextArea,
    TextInput, Tree, TreeItem,
};
use rune_theme_opencode::icons;

use crate::actions::{self, mark_dirty};
use crate::state::{App, DockTab, Pending};
use crate::theme;
use crate::thread::counts;

pub fn view(s: &App, k: &Rc<Kit>) -> El {
    let c = &k.c;
    let mut tabs = row().key("dock-tabs").gap(2.0).center_y().role("tablist");
    for tab in DockTab::ALL {
        let on = tab == s.dock_tab;
        tabs = tabs.child(
            row()
                .key(format!("dock-tab-{}", tab.title()))
                .h(26.0)
                .px(10.0)
                .center_y()
                .radius(7.0)
                .role("tab")
                .label(tab.title())
                .checked(on)
                .focusable(true)
                .fill(if on {
                    c.layer_02
                } else {
                    rune::Color::TRANSPARENT
                })
                .hover(c.hover)
                .cursor(rune::Cursor::Pointer)
                .on_press(move |s: &mut App| {
                    s.dock_tab = tab;
                    if matches!(tab, DockTab::Changes | DockTab::Files | DockTab::Git) {
                        actions::refresh_repo(s);
                    }
                })
                .child(text(tab.title()).font(k.ty.ui).color(if on {
                    c.text_strong
                } else {
                    c.text_muted
                })),
        );
    }
    let refresh = IconButton::new(&icons::REFRESH)
        .variant(IconButtonVariant::GhostMuted)
        .control_size(ControlSize::Small)
        .label("Refresh")
        .on_press(actions::refresh_repo)
        .build(k)
        .key("dock-refresh");
    let header = row()
        .w_full()
        .px(10.0)
        .h(48.0)
        .gap(8.0)
        .center_y()
        .child(tabs)
        .child(rune::el().grow(1.0))
        .child(if s.repo.loading {
            Spinner::new().size(13.0).build(k)
        } else {
            refresh
        });
    let body = match s.dock_tab {
        DockTab::Changes => changes(s, k),
        DockTab::Files => files(s, k),
        DockTab::Terminal => terminal(s, k),
        DockTab::Git => git(s, k),
        DockTab::Notes => notes(s, k),
    };
    col()
        .key("dock")
        .w_full()
        .h_full()
        .min_h(0.0)
        .fill(c.bg_base)
        .child(header)
        .child(rune::rule(1.0, c.border_weak))
        .child(col().w_full().grow(1.0).min_h(0.0).child(body))
}

fn empty(k: &Kit, title: &str, detail: &str) -> El {
    col()
        .w_full()
        .h_full()
        .center()
        .gap(6.0)
        .padding(24.0)
        .child(
            text(title.to_owned())
                .font(k.ty.ui_medium)
                .color(k.c.text_muted),
        )
        .child(
            text(detail.to_owned())
                .font(k.ty.small)
                .color(k.c.text_faint)
                .wrap()
                .align(rune::Align::Center),
        )
}

fn changes(s: &App, k: &Rc<Kit>) -> El {
    let c = &k.c;
    if let Some(e) = &s.repo.error {
        return empty(k, "No changes to show", e);
    }
    if s.repo.changes.is_empty() {
        return empty(
            k,
            "Working tree clean",
            "Changes the agent makes show up here for review.",
        );
    }
    let (added, removed) = s
        .repo
        .changes
        .iter()
        .fold((0, 0), |(a, r), f| (a + f.added, r + f.removed));
    let split = s.repo.diff.mode() == DiffMode::Split;
    let summary = row()
        .w_full()
        .px(12.0)
        .py(8.0)
        .gap(10.0)
        .center_y()
        .child(
            text(format!("{} files", s.repo.changes.len()))
                .font(k.ty.ui_medium)
                .color(c.text_base),
        )
        .child(counts(added, removed, k))
        .child(rune::el().grow(1.0))
        .child(
            IconButton::new(if split {
                &icons::UNIFIED
            } else {
                &icons::SPLIT
            })
            .variant(IconButtonVariant::GhostMuted)
            .control_size(ControlSize::Small)
            .label(if split { "Unified view" } else { "Split view" })
            .on_press_tx(|s: &mut App, tx| {
                let mode = if s.repo.diff.mode() == DiffMode::Split {
                    DiffMode::Unified
                } else {
                    DiffMode::Split
                };
                tx.motion(rune::tok::motion::DEFAULT);
                s.repo.diff.set_mode(mode);
            })
            .build(k)
            .key("diff-mode"),
        );
    let kit = k.clone();
    let diff = DiffView::new(|s: &App| &s.repo.diff, |s: &mut App| &mut s.repo.diff)
        .header_actions(move |s: &App, i| {
            let path = s
                .repo
                .diff
                .files()
                .get(i)
                .map(|f| f.path().to_owned())
                .unwrap_or_default();
            IconButton::new(&icons::OUTLINE_UNDO)
                .variant(IconButtonVariant::GhostMuted)
                .control_size(ControlSize::Small)
                .label("Discard changes")
                .on_press(move |s: &mut App| s.confirm = Some(Pending::Discard(path.clone())))
                .build(&kit)
        })
        .build(s, k);
    let commit = commit_box(s, k, s.repo.changes.len());
    col()
        .w_full()
        .h_full()
        .min_h(0.0)
        .child(summary)
        .child(col().w_full().grow(1.0).min_h(0.0).child(diff))
        .child(commit)
}

/// The message and the two ways to commit. Commit is the one primary action; it stays disabled
/// until there is a message, and it says how many files it takes.
fn commit_box(s: &App, k: &Kit, files: usize) -> El {
    let c = &k.c;
    let ready = !s.commit_message.text.trim().is_empty();
    let field = rune::auto_grow(
        rune::text_input(&s.commit_message, k.ty.ui)
            .key("commit-message")
            .w_full()
            .color(c.text_base)
            .placeholder("Describe the change", c.text_faint)
            .caret(c.text_base, k.selection())
            .label("Commit message")
            .on_edit(|s: &mut App| &mut s.commit_message)
            .on_submit(|s: &mut App, tx| actions::git_action(s, "commit", tx)),
        1,
        6,
    );
    let label = format!("Commit {files} file{}", if files == 1 { "" } else { "s" });
    let actions_row = row()
        .w_full()
        .gap(8.0)
        .center_y()
        .child(
            text(format!(
                "on {}",
                s.repo.status.branch.as_deref().unwrap_or("detached")
            ))
            .font(k.ty.small)
            .color(c.text_faint)
            .truncate()
            .grow(1.0),
        )
        .child(
            Button::new("Commit & push")
                .variant(ButtonVariant::Ghost)
                .control_size(ControlSize::Small)
                .disabled(!ready)
                .on_press_tx(|s: &mut App, tx| actions::git_action(s, "commit-push", tx))
                .build(k)
                .key("commit-push"),
        )
        .child(
            Button::new(label)
                .variant(if ready {
                    ButtonVariant::Submit
                } else {
                    ButtonVariant::Neutral
                })
                .control_size(ControlSize::Small)
                .disabled(!ready)
                .on_press_tx(|s: &mut App, tx| actions::git_action(s, "commit", tx))
                .build(k)
                .key("commit"),
        );
    col().key("commit-box").w_full().padding(12.0).child(
        col()
            .w_full()
            .gap(10.0)
            .padding(12.0)
            .radius(12.0)
            .fill(c.bg_base)
            .border(1.0, if ready { c.border_base } else { c.border_weak })
            .child(field)
            .child(actions_row),
    )
}

fn files(s: &App, k: &Kit) -> El {
    if s.repo.files.is_empty() {
        return empty(k, "No files", "Add a project folder to browse its files.");
    }
    let changed: HashMap<String, (u32, u32)> = s
        .repo
        .changes
        .iter()
        .map(|c| (c.path.clone(), (c.added, c.removed)))
        .collect();
    let tree = Tree::new(
        |s: &App| &s.repo.files,
        |s: &mut App| &mut s.repo.files,
        |n| n.label(),
    )
    .row(move |node, r| {
        let item = TreeItem::file(node, r);
        match node.data.as_ref().and_then(|p| changed.get(p)) {
            Some(&(a, d)) => item.changes(a, d),
            None => item,
        }
    })
    .on_open(|s: &mut App, id, _| {
        let Some(path) = s.repo.files.get(id).and_then(|n| n.data.clone()) else {
            return;
        };
        let mut t = s.composer.text.clone();
        if !t.is_empty() && !t.ends_with(' ') {
            t.push(' ');
        }
        t.push_str(&format!("@{path} "));
        s.composer = TextEdit::new(t);
        s.composer.focused = true;
    })
    .build(s, k);
    col()
        .w_full()
        .h_full()
        .min_h(0.0)
        .child(
            text(format!(
                "{} files · open one to mention it",
                s.repo.file_count
            ))
            .font(k.ty.small)
            .color(k.c.text_faint)
            .px(12.0)
            .py(6.0),
        )
        .child(col().w_full().grow(1.0).min_h(0.0).child(tree))
}

fn terminal(s: &App, k: &Kit) -> El {
    let c = &k.c;
    let mono = theme::sized(k.ty.mono, 12.0);
    let lines = col()
        .key("terminal-lines")
        .w_full()
        .growth(rune::GrowthSpec::default())
        .virtual_rows(
            s.terminal.lines.len(),
            17.0,
            |_: &App, i| std::borrow::Cow::Owned(i.to_string()),
            {
                let (text_c, err_c) = (c.text_base, c.danger.icon);
                move |s: &App, i| {
                    let (err, line) = &s.terminal.lines[i];
                    text(line.clone())
                        .font(mono)
                        .color(if *err { err_c } else { text_c })
                        .wrap_anywhere()
                        .selectable(true)
                        .px(12.0)
                        .py(1.0)
                }
            },
        )
        .virtual_rows_version((
            s.terminal.lines.len(),
            s.terminal.lines.last().map(|l| l.1.len()),
        ));
    let input = row()
        .w_full()
        .px(12.0)
        .py(8.0)
        .gap(8.0)
        .center_y()
        .child(text("$").font(mono).color(c.text_faint))
        .child(
            TextInput::new(&s.terminal.input)
                .width(None)
                .plain()
                .placeholder(s.place().map_or_else(String::new, |at| {
                    format!(
                        "{}  (each command starts here; cd does not carry over)",
                        at.cwd
                    )
                }))
                .text_style(mono)
                .on_edit(|s: &mut App| &mut s.terminal.input)
                .on_key(|s: &mut App, ev, _| {
                    let t = &mut s.terminal;
                    if t.history.is_empty() {
                        return false;
                    }
                    let at = match (&ev.key, t.history_at) {
                        (Key::Up, None) => t.history.len().checked_sub(1),
                        (Key::Up, Some(i)) => Some(i.saturating_sub(1)),
                        (Key::Down, Some(i)) if i + 1 < t.history.len() => Some(i + 1),
                        (Key::Down, Some(_)) => None,
                        _ => return false,
                    };
                    t.history_at = at;
                    t.input = TextEdit::new(at.map_or("", |i| t.history[i].as_str()));
                    t.input.focused = true;
                    true
                })
                .on_submit(actions::run_terminal)
                .build(k)
                .grow(1.0)
                .key("terminal-input"),
        )
        .child(if s.terminal.running.is_some() {
            IconButton::new(&icons::STOP)
                .variant(IconButtonVariant::GhostMuted)
                .control_size(ControlSize::Small)
                .label("Stop")
                .on_press(|s: &mut App| {
                    if let Some(r) = &s.terminal.running {
                        r.stop();
                    }
                })
                .build(k)
                .key("terminal-stop")
        } else {
            IconButton::new(&icons::OUTLINE_TRASH)
                .variant(IconButtonVariant::GhostMuted)
                .control_size(ControlSize::Small)
                .label("Clear")
                .on_press(|s: &mut App| s.terminal.lines.clear())
                .build(k)
                .key("terminal-clear")
        });
    col()
        .w_full()
        .h_full()
        .min_h(0.0)
        .fill(c.bg_deep)
        .child(
            col()
                .w_full()
                .grow(1.0)
                .min_h(0.0)
                .pt(8.0)
                .scroll(ScrollSpec { stick_bottom: true })
                .child(lines),
        )
        .child(rune::rule(1.0, c.border_weak))
        .child(input)
}

fn git(s: &App, k: &Kit) -> El {
    let c = &k.c;
    if let Some(e) = &s.repo.error {
        return empty(k, "Git unavailable", e);
    }
    let st = &s.repo.status;
    let branch = st.branch.clone().unwrap_or_else(|| "detached".into());
    let mut sync = branch;
    if let Some(up) = &st.upstream {
        sync.push_str(&format!(" → {up}"));
    }
    let actions_row = row()
        .gap(6.0)
        .child(
            Button::new(format!(
                "Push{}",
                if st.ahead > 0 {
                    format!(" {}", st.ahead)
                } else {
                    String::new()
                }
            ))
            .icon(&icons::ARROW_UP)
            .control_size(ControlSize::Small)
            .on_press_tx(|s: &mut App, tx| actions::git_action(s, "push", tx))
            .build(k)
            .key("git-push"),
        )
        .child(
            Button::new("Create pull request")
                .icon(&icons::BRANCH_OUT)
                .control_size(ControlSize::Small)
                .on_press_tx(|s: &mut App, tx| actions::git_action(s, "pr", tx))
                .build(k)
                .key("git-pr"),
        );
    let head = col()
        .w_full()
        .gap(8.0)
        .padding(12.0)
        .child(
            row()
                .gap(8.0)
                .center_y()
                .child(icon(&icons::BRANCH, 14.0).color(c.icon_muted))
                .child(text(sync).font(k.ty.ui_medium).color(c.text_base)),
        )
        .child(
            text(format!(
                "{} ahead · {} behind · {} changed",
                st.ahead,
                st.behind,
                st.entries.len()
            ))
            .font(k.ty.small)
            .color(c.text_faint),
        )
        .child(actions_row);
    let section = |title: &str| {
        text(title.to_owned())
            .font(k.ty.small_medium)
            .color(c.text_faint)
            .px(12.0)
            .pt(10.0)
            .pb(4.0)
    };
    let mut list = col().w_full().child(head).child(section("Branches"));
    for b in &s.repo.branches {
        let current = st.branch.as_deref() == Some(b.as_str());
        let name = b.clone();
        list = list.child(
            row()
                .key(format!("branch-{b}"))
                .w_full()
                .px(12.0)
                .py(6.0)
                .gap(8.0)
                .center_y()
                .hover(c.hover)
                .cursor(rune::Cursor::Pointer)
                .on_press_tx(move |s: &mut App, tx| actions::checkout(s, name.clone(), tx))
                .child(
                    icon(
                        if current {
                            &icons::CHECK_SMALL
                        } else {
                            &icons::BRANCH
                        },
                        13.0,
                    )
                    .color(if current { c.icon_accent } else { c.icon_faint }),
                )
                .child(
                    text(b.clone())
                        .font(k.ty.ui)
                        .color(if current { c.text_strong } else { c.text_base })
                        .truncate(),
                ),
        );
    }
    list = list.child(section("History"));
    for commit in &s.repo.log {
        list = list.child(
            row()
                .w_full()
                .px(12.0)
                .py(6.0)
                .gap(10.0)
                .child(
                    text(commit.hash.clone())
                        .font(theme::sized(k.ty.mono, 12.0))
                        .color(c.text_faint),
                )
                .child(
                    col()
                        .grow(1.0)
                        .gap(2.0)
                        .child(
                            text(commit.subject.clone())
                                .font(k.ty.ui)
                                .color(c.text_base)
                                .truncate(),
                        )
                        .child(
                            text(format!("{} · {}", commit.author, commit.when))
                                .font(k.ty.small)
                                .color(c.text_faint),
                        ),
                ),
        );
    }
    col()
        .w_full()
        .h_full()
        .min_h(0.0)
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .child(list.pb(12.0))
}

fn notes(s: &App, k: &Kit) -> El {
    let c = &k.c;
    let Some(t) = s.thread() else {
        return empty(
            k,
            "No thread",
            "Open a thread to keep notes and a goal for it.",
        );
    };
    let goal = TextInput::new(&s.goal)
        .width(None)
        .placeholder(
            t.goal
                .clone()
                .unwrap_or_else(|| "What should this thread achieve?".into()),
        )
        .leading_icon(&icons::STATUS_ACTIVE)
        .on_edit(|s: &mut App| &mut s.goal)
        .on_submit(|s: &mut App, tx| {
            let g = s.goal.text.trim().to_owned();
            actions::run_command(s, oxlet_core::slash::Command::Goal, &g, tx);
            s.goal = TextEdit::new("");
        })
        .build(k)
        .key("goal-input");
    let edit = if s.notes_for.as_deref() == Some(t.id.as_str()) {
        s.notes.clone()
    } else {
        TextEdit::new(t.notes.clone())
    };
    let notes = TextArea::new(&edit)
        .width(None)
        .placeholder("Notes for this thread")
        .auto_grow(8, 30)
        .on_edit(|s: &mut App| {
            // One editor serves every thread; it takes up the open thread's notes on its first edit.
            if s.notes_for != s.selected {
                let text = s.thread().map(|t| t.notes.clone()).unwrap_or_default();
                s.notes = TextEdit::new(text);
                s.notes_for = s.selected.clone();
            }
            &mut s.notes
        })
        .on_change(|s: &mut App, tx| {
            if let Some(id) = s.notes_for.clone()
                && let Some(t) = s.ws.thread_mut(&id)
            {
                t.notes = s.notes.text.clone();
                mark_dirty(s, tx);
            }
        })
        .build(k)
        .key("notes-input");
    let mut out = col()
        .w_full()
        .gap(10.0)
        .padding(12.0)
        .child(
            row()
                .w_full()
                .gap(8.0)
                .center_y()
                .child(
                    text("This thread")
                        .font(k.ty.small_medium)
                        .color(c.text_faint),
                )
                .child(rune::el().grow(1.0))
                .child(
                    text(
                        t.usage()
                            .map_or("No usage reported yet".into(), |u| u.label()),
                    )
                    .font(k.ty.small)
                    .color(c.text_muted),
                ),
        )
        .child(text("Goal").font(k.ty.small_medium).color(c.text_faint))
        .child(goal);
    if let Some(g) = &t.goal {
        out = out.child(text(g.clone()).font(k.ty.ui).color(c.text_base).wrap());
    }
    let pinned: Vec<String> = t
        .messages
        .iter()
        .filter(|m| m.pinned)
        .map(|m| m.text())
        .collect();
    if !pinned.is_empty() {
        out = out.child(
            text("Pinned")
                .font(k.ty.small_medium)
                .color(c.text_faint)
                .pt(6.0),
        );
        for p in pinned {
            out = out.child(
                text(p.lines().next().unwrap_or_default().to_owned())
                    .font(k.ty.ui)
                    .color(c.text_base)
                    .truncate()
                    .px(10.0)
                    .py(6.0)
                    .radius(k.radius.sm)
                    .fill(c.layer_01),
            );
        }
    }
    out.child(
        text("Notes")
            .font(k.ty.small_medium)
            .color(c.text_faint)
            .pt(6.0),
    )
    .child(notes)
}
