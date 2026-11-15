use std::borrow::Cow;
use std::rc::Rc;

use oxlet_core::model::{FileChange, Message, Part, Role, Thread};
use oxlet_core::time;
use rune::{El, GrowthSpec, ScrollSpec, col, icon, row, text};
use rune_kit::{
    Button, ButtonVariant, ControlSize, IconButton, IconButtonVariant, Kit, Markdown, Spinner,
};
use rune_theme_opencode::icons;

use crate::actions;
use crate::state::{App, DockTab};
use crate::theme;

pub const COLUMN: f32 = 760.0;
const TOOLS_SHOWN: usize = 3;
const FILES_SHOWN: usize = 5;

pub fn transcript(s: &App, t: &Thread, k: &Rc<Kit>) -> El {
    let kit = k.clone();
    let id = t.id.clone();
    let count = t.messages.len();
    let list = col()
        .key("messages")
        .w_full()
        .growth(GrowthSpec::default())
        .virtual_rows(
            count,
            120.0,
            |s: &App, i| {
                Cow::Owned(
                    s.thread()
                        .and_then(|t| t.messages.get(i))
                        .map_or_else(|| i.to_string(), |m| m.id.clone()),
                )
            },
            move |s: &App, i| match s
                .ws
                .thread(&id)
                .and_then(|t| t.messages.get(i).map(|m| (t, m)))
            {
                Some((t, m)) => message_row(s, t, m, &kit),
                None => col(),
            },
        )
        .virtual_rows_version((t.id.as_str(), count, t.updated, s.turns.contains_key(&t.id)));
    col()
        .key("transcript")
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .scroll(ScrollSpec { stick_bottom: true })
        .scroll_edge_fade(12.0)
        .child(list)
}

fn centred(child: El) -> El {
    row()
        .w_full()
        .justify_center()
        .px(24.0)
        .child(col().w_full().max_w(COLUMN).child(child))
}

fn message_row(s: &App, t: &Thread, m: &Message, k: &Kit) -> El {
    let body = match m.role {
        Role::User => user_message(s, t, m, k),
        Role::System => text(m.text()).font(k.ty.small).color(k.c.text_faint).wrap(),
        Role::Assistant => assistant_message(s, t, m, k),
    };
    centred(body).py(10.0).key(m.id.clone())
}

fn user_message(s: &App, t: &Thread, m: &Message, k: &Kit) -> El {
    let c = &k.c;
    let bubble = col()
        .max_w(COLUMN * 0.78)
        .px(14.0)
        .py(10.0)
        .radius(16.0)
        .fill(if s.editing.as_ref().is_some_and(|(_, id)| *id == m.id) {
            c.bg_accent.alpha(0.16)
        } else {
            c.layer_02
        })
        .child(
            text(m.text())
                .font(k.ty.body)
                .color(c.text_base)
                .wrap()
                .selectable(true),
        );
    let copy = m.text();
    let (tid, mid) = (t.id.clone(), m.id.clone());
    let small = |glyph: &'static rune::Icon, label: &str| {
        IconButton::new(glyph)
            .variant(IconButtonVariant::GhostMuted)
            .control_size(ControlSize::Small)
            .label(label)
    };
    let mut tools = row().gap(2.0).center_y();
    if !s.turns.contains_key(&t.id) {
        tools = tools.child(
            small(&icons::EDIT, "Edit and resend")
                .on_press(move |s: &mut App| actions::edit_message(s, &tid, &mid))
                .build(k)
                .key(format!("{}-edit", m.id)),
        );
    }
    tools = tools.child(
        small(&icons::COPY, "Copy")
            .on_press_tx(move |_: &mut App, tx| tx.copy(copy.clone()))
            .build(k)
            .key(format!("{}-copy", m.id)),
    );
    col()
        .w_full()
        .items_end()
        .gap(4.0)
        .hover_group()
        .child(bubble)
        .child(tools.group_opacity(0.0, 1.0))
}

fn assistant_message(s: &App, t: &Thread, m: &Message, k: &Kit) -> El {
    let c = &k.c;
    let live = s.turns.get(&t.id).is_some_and(|turn| turn.message == m.id);
    let mut out = col().w_full().gap(12.0);
    out = out.child(reply_header(s, t, m, k));
    let mut tools: Vec<(&str, &str)> = Vec::new();
    // The index of the group's first part keys it: stable while tools arrive, and unique.
    let mut first = 0;
    let flush = |out: El, tools: &mut Vec<(&str, &str)>, first: usize| {
        if tools.is_empty() {
            return out;
        }
        let el = tool_group(s, m, first, tools, k);
        tools.clear();
        out.child(el)
    };
    for (i, part) in m.parts.iter().enumerate() {
        if let Part::Tool { name, summary, .. } = part {
            if tools.is_empty() {
                first = i;
            }
            tools.push((name, summary));
            continue;
        }
        out = flush(out, &mut tools, first);
        out = match part {
            Part::Text { text } => out.child(
                Markdown::new(text.clone())
                    .selectable(true)
                    .streaming(live && i + 1 == m.parts.len())
                    .build(k)
                    .key(format!("{}-md{i}", m.id)),
            ),
            Part::Edits { files } => out.child(edits_card(m, files, s, k)),
            Part::Error { message } => out.child(
                row()
                    .w_full()
                    .gap(8.0)
                    .px(12.0)
                    .py(10.0)
                    .radius(k.radius.md)
                    .fill(c.danger.bg)
                    .border(1.0, c.danger.border)
                    .child(icon(&icons::CIRCLE_EXCLAMATION, 14.0).color(c.danger.icon))
                    .child(
                        text(message.clone())
                            .font(k.ty.ui)
                            .color(c.text_base)
                            .wrap()
                            .grow(1.0)
                            .selectable(true),
                    ),
            ),
            Part::Tool { .. } => out,
        };
    }
    out = flush(out, &mut tools, first);
    if live {
        out = out.child(
            row()
                .gap(8.0)
                .center_y()
                .child(Spinner::new().size(14.0).build(k))
                .child(text("Working").font(k.ty.small).color(c.text_muted)),
        );
    } else {
        out = out.child(footer(s, t, m, k));
    }
    out
}

fn tool_group(s: &App, m: &Message, first: usize, tools: &[(&str, &str)], k: &Kit) -> El {
    let c = &k.c;
    let key = format!("{}-tools{first}", m.id);
    let open = s.expanded.contains(&key);
    let hidden = tools.len().saturating_sub(TOOLS_SHOWN);
    let shown = if open { tools } else { &tools[hidden..] };
    let mut list = col().w_full().gap(4.0);
    if hidden > 0 {
        let k2 = key.clone();
        list = list.child(
            row()
                .key(format!("{key}-toggle"))
                .gap(4.0)
                .center_y()
                .cursor(rune::Cursor::Pointer)
                .on_press(move |s: &mut App| {
                    if !s.expanded.remove(&k2) {
                        s.expanded.insert(k2.clone());
                    }
                })
                .child(
                    icon(
                        if open {
                            &icons::CHEVRON_DOWN
                        } else {
                            &icons::CHEVRON_RIGHT
                        },
                        12.0,
                    )
                    .color(c.icon_faint),
                )
                .child(
                    text(if open {
                        "Hide earlier steps".to_owned()
                    } else {
                        format!("{hidden} earlier steps")
                    })
                    .font(k.ty.small)
                    .color(c.text_faint),
                ),
        );
    }
    for (name, summary) in shown {
        list = list.child(
            row()
                .w_full()
                .gap(8.0)
                .center_y()
                .child(icon(tool_icon(name), 13.0).color(c.icon_faint))
                .child(
                    text((*name).to_owned())
                        .font(k.ty.small_medium)
                        .color(c.text_muted),
                )
                .child(
                    text((*summary).to_owned())
                        .font(theme::sized(k.ty.mono, 12.0))
                        .color(c.text_faint)
                        .truncate()
                        .grow(1.0),
                ),
        );
    }
    list
}

fn tool_icon(name: &str) -> &'static rune::Icon {
    match name {
        "Bash" | "Shell" => &icons::CONSOLE,
        "Edit" | "Write" | "MultiEdit" | "NotebookEdit" => &icons::PENCIL_LINE,
        "Read" => &icons::FILE_DOCUMENT,
        "Grep" | "Glob" | "web_search" | "WebSearch" => &icons::MAGNIFYING_GLASS,
        "WebFetch" => &icons::GLOBE,
        "Task" => &icons::TASK,
        _ => &icons::MCP,
    }
}

fn edits_card(m: &Message, files: &[FileChange], s: &App, k: &Kit) -> El {
    let c = &k.c;
    let (added, removed) = files
        .iter()
        .fold((0, 0), |(a, r), f| (a + f.added, r + f.removed));
    let key = format!("{}-files", m.id);
    let open = s.expanded.contains(&key);
    let review = Button::new("Review")
        .variant(ButtonVariant::Neutral)
        .control_size(ControlSize::Small)
        .on_press(|s: &mut App| {
            s.dock_tab = DockTab::Changes;
            s.dock.set_open(true);
            actions::refresh_repo(s);
        })
        .build(k)
        .key(format!("{}-review", m.id));
    let header = row()
        .w_full()
        .px(14.0)
        .py(10.0)
        .gap(10.0)
        .center_y()
        .child(icon(&icons::REVIEW, 15.0).color(c.icon_muted))
        .child(
            col()
                .grow(1.0)
                .gap(2.0)
                .child(
                    text(format!(
                        "Edited {} file{}",
                        files.len(),
                        if files.len() == 1 { "" } else { "s" }
                    ))
                    .font(k.ty.ui_medium)
                    .color(c.text_base),
                )
                .child(counts(added, removed, k)),
        )
        .child(review);
    let mut card = col()
        .w_full()
        .radius(k.radius.lg)
        .border(1.0, c.border_weak)
        .fill(c.bg_base)
        .child(header);
    let shown = if open {
        files.len()
    } else {
        files.len().min(FILES_SHOWN)
    };
    for f in &files[..shown] {
        let path = f.path.clone();
        card = card.child(
            row()
                .key(format!("{}-file-{}", m.id, f.path))
                .w_full()
                .h(32.0)
                .px(14.0)
                .gap(10.0)
                .center_y()
                .border_sides([1.0, 0.0, 0.0, 0.0], c.border_weak)
                .hover(c.hover)
                .cursor(rune::Cursor::Pointer)
                .on_press_tx(move |s: &mut App, tx| {
                    s.dock_tab = DockTab::Changes;
                    s.dock.set_open(true);
                    if let Some(i) = s.repo.diff.file_index(&path) {
                        s.repo.diff.scroll_to_file(i, tx);
                    }
                })
                .child(icon(theme::file_icon(&f.path), 13.0).color(c.icon_muted))
                .child(
                    text(f.path.clone())
                        .font(theme::sized(k.ty.mono, 12.5))
                        .color(c.text_base)
                        .truncate()
                        .grow(1.0),
                )
                .child(counts(f.added, f.removed, k)),
        );
    }
    if files.len() > FILES_SHOWN {
        let hidden = files.len() - FILES_SHOWN;
        card = card.child(
            row()
                .key(format!("{key}-more"))
                .w_full()
                .px(14.0)
                .py(9.0)
                .gap(6.0)
                .center_y()
                .border_sides([1.0, 0.0, 0.0, 0.0], c.border_weak)
                .cursor(rune::Cursor::Pointer)
                .on_press(move |s: &mut App| {
                    if !s.expanded.remove(&key) {
                        s.expanded.insert(key.clone());
                    }
                })
                .child(
                    icon(
                        if open {
                            &icons::CHEVRON_DOWN
                        } else {
                            &icons::CHEVRON_RIGHT
                        },
                        12.0,
                    )
                    .color(c.icon_faint),
                )
                .child(
                    text(if open {
                        "Show fewer files".to_owned()
                    } else {
                        format!("Show {hidden} more files")
                    })
                    .font(k.ty.small)
                    .color(c.text_muted),
                ),
        );
    }
    card
}

pub fn counts(added: u32, removed: u32, k: &Kit) -> El {
    row()
        .gap(6.0)
        .child(
            text(format!("+{added}"))
                .font(k.ty.small_medium)
                .color(k.c.success.icon),
        )
        .child(
            text(format!("-{removed}"))
                .font(k.ty.small_medium)
                .color(k.c.danger.icon),
        )
}

fn footer(s: &App, t: &Thread, m: &Message, k: &Kit) -> El {
    let latest = t.messages.last().is_some_and(|last| last.id == m.id);
    let copy = m.text();
    let (tid, mid) = (t.id.clone(), m.id.clone());
    let small = |glyph: &'static rune::Icon, label: &str| {
        IconButton::new(glyph)
            .variant(IconButtonVariant::GhostMuted)
            .control_size(ControlSize::Small)
            .label(label)
    };
    row()
        .gap(2.0)
        .center_y()
        .child(
            small(&icons::COPY, "Copy")
                .on_press_tx(move |_: &mut App, tx| tx.copy(copy.clone()))
                .build(k)
                .key(format!("{}-copy", m.id)),
        )
        .child(
            small(&icons::FORK, "Fork from here")
                .on_press_tx({
                    let (tid, mid) = (tid.clone(), mid.clone());
                    move |s: &mut App, tx| fork_at(s, &tid, &mid, tx)
                })
                .build(k)
                .key(format!("{}-fork", m.id)),
        )
        .child(
            small(
                if m.pinned {
                    &rune_kit::glyphs::STAR_FILLED
                } else {
                    &rune_kit::glyphs::STAR
                },
                if m.pinned { "Unpin" } else { "Pin" },
            )
            .on_press_tx(move |s: &mut App, tx| {
                if let Some(m) =
                    s.ws.thread_mut(&tid)
                        .and_then(|t| t.messages.iter_mut().find(|x| x.id == mid))
                {
                    m.pinned = !m.pinned;
                }
                actions::mark_dirty(s, tx);
            })
            .build(k)
            .key(format!("{}-pin", m.id)),
        )
        .children((latest && !s.turns.contains_key(&t.id)).then(|| {
            let tid = t.id.clone();
            small(&icons::REFRESH, "Retry")
                .on_press_tx(move |s: &mut App, tx| actions::retry(s, &tid, tx))
                .build(k)
                .key(format!("{}-retry", m.id))
        }))
}

/// Who wrote a reply, how long it took and when: a thread can pass between agents.
fn reply_header(s: &App, t: &Thread, m: &Message, k: &Kit) -> El {
    let c = &k.c;
    let (provider, model) = m
        .agent
        .clone()
        .unwrap_or_else(|| (t.provider, t.model.clone()));
    let name = provider.model_name(&model);
    let mut meta = time::stamp(m.at, actions::now(), s.offset, s.ws.settings.clock_24h);
    if let Some(ms) = m.worked_ms {
        meta = format!("Worked {} · {meta}", time::duration(ms));
    }
    if let Some(u) = m.usage {
        meta = format!("{meta} · {}", u.label());
    }
    row()
        .w_full()
        .gap(8.0)
        .center_y()
        .child(icon(theme::provider_icon(provider), 14.0).color(c.icon_muted))
        .child(text(name).font(k.ty.ui_medium).color(c.text_base))
        .child(text(meta).font(k.ty.small).color(c.text_faint))
        .children(
            m.pinned
                .then(|| icon(&rune_kit::glyphs::STAR_FILLED, 11.0).color(c.icon_accent)),
        )
}

fn fork_at(s: &mut App, thread: &str, message: &str, tx: &mut rune::Tx) {
    let Some(new) = s.ws.fork_thread(thread, actions::now()) else {
        return;
    };
    if let Some(t) = s.ws.thread_mut(&new)
        && let Some(i) = t.messages.iter().position(|m| m.id == message)
    {
        t.messages.truncate(i + 1);
    }
    actions::select_thread(s, &new, tx);
    actions::mark_dirty(s, tx);
}
