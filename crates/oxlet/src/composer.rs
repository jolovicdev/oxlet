use oxlet_core::model::{AccessMode, ChatMode, Effort};
use oxlet_core::provider::{PROVIDERS, ProviderId};
use oxlet_core::slash;
use rune::{El, Key, Tx, col, icon, row, text};
use rune_kit::{IconButton, IconButtonVariant, Kit, PopupItem, PopupList, Select, Spinner};
use rune_theme_opencode::icons;

use crate::actions::{self, mark_dirty};
use crate::state::{App, Placement, Popup};
use crate::theme;

const PLACEHOLDER: &str = "Ask anything, @tag files/folders, or use / to show available commands";

/// `(provider, model id, label)` for every model the picker offers.
fn models(s: &App) -> Vec<(ProviderId, String, String)> {
    let custom = &s.ws.settings.custom_models;
    PROVIDERS
        .iter()
        .filter(|p| !s.ws.settings.hidden_providers.contains(&p.id))
        .flat_map(|p| {
            let listed = p.models.iter().map(move |(id, label)| {
                let label = match label.strip_prefix(p.name).map(str::trim_start) {
                    _ if id.is_empty() => format!("{} default", p.name),
                    Some(short) => short.to_owned(),
                    None => (*label).to_owned(),
                };
                (p.id, (*id).to_owned(), label)
            });
            let extra = custom
                .iter()
                .filter(move |(pid, _)| *pid == p.id)
                .map(|(pid, id)| (*pid, id.clone(), id.clone()));
            listed.chain(extra)
        })
        .collect()
}

pub fn view(s: &App, k: &Kit, layer: &mut Vec<El>) -> El {
    let c = &k.c;
    let thread = s.thread();
    let working = thread.is_some_and(|t| s.turns.contains_key(&t.id));
    let field = rune::auto_grow(
        rune::text_input(&s.composer, k.ty.body)
            .key("composer-field")
            .w_full()
            .color(c.text_base)
            .placeholder(PLACEHOLDER, c.text_faint)
            .caret(c.text_base, k.selection())
            .label("Message")
            .on_edit(|s: &mut App| &mut s.composer)
            .on_input_change(|s: &mut App, _| {
                s.slash_active = 0;
                s.slash_dismissed = false;
            })
            .on_input_key(on_key)
            .on_submit(actions::submit),
        2,
        12,
    );

    let (provider, model, effort, access, mode) = match thread {
        Some(t) => (t.provider, t.model.as_str(), t.effort, t.access, t.mode),
        None => (
            s.current_project()
                .and_then(|p| p.default_provider)
                .or(s.ws.settings.default_provider)
                .unwrap_or(ProviderId::Claude),
            "",
            Effort::default(),
            s.ws.settings.default_access,
            ChatMode::Default,
        ),
    };

    let access_labels: Vec<&str> = AccessMode::ALL.iter().map(|m| m.label()).collect();
    let access_details: Vec<Option<&str>> =
        AccessMode::ALL.iter().map(|m| Some(m.detail())).collect();
    let access_select = Select::new("access", &access_labels)
        .details(&access_details)
        .label("Access")
        .current(Some(access.label()))
        .open(s.popup == Some(Popup::Access))
        .on_toggle(|s: &mut App| toggle(s, Popup::Access))
        .on_close(|s: &mut App| s.popup = None)
        .on_choose(|s: &mut App, v: &str, tx| {
            if let Some(m) = AccessMode::from_label(v) {
                match s.selected.clone() {
                    Some(id) => {
                        if let Some(t) = s.ws.thread_mut(&id) {
                            t.access = m;
                        }
                    }
                    None => s.ws.settings.default_access = m,
                }
                mark_dirty(s, tx);
            }
            s.popup = None;
        })
        .build(k, layer);
    let access_glyph = match access {
        AccessMode::FullAccess => icon(&icons::WARNING, 13.0).color(c.warning.icon),
        AccessMode::AutoEdit => icon(&icons::SHIELD, 13.0).color(c.icon_muted),
        AccessMode::ReadOnly => icon(&icons::EYE, 13.0).color(c.icon_muted),
    };

    let all = models(s);
    let grouped: Vec<(&str, &str)> = all
        .iter()
        .map(|(p, _, l)| (p.info().name, l.as_str()))
        .collect();
    let current_label = all
        .iter()
        .find(|(p, m, _)| *p == provider && m == model)
        .map(|(_, _, l)| l.clone());
    let model_select = Select::grouped("model", &grouped)
        .label("Model")
        .current(current_label.as_deref())
        .open(s.popup == Some(Popup::Model))
        .on_toggle(|s: &mut App| toggle(s, Popup::Model))
        .on_close(|s: &mut App| s.popup = None)
        .on_choose(|s: &mut App, v: &str, tx| {
            if let Some((p, m, _)) = models(s).into_iter().find(|(_, _, l)| l == v) {
                choose_model(s, p, &m, tx);
            }
            s.popup = None;
        })
        .build(k, layer);

    let mut tools = row().w_full().gap(6.0).center_y().px(8.0).pb(8.0);
    tools = tools
        .child(
            IconButton::new(&icons::PLUS)
                .variant(IconButtonVariant::GhostMuted)
                .label("Attach files from the project")
                .on_press(|s: &mut App| {
                    s.dock_tab = crate::state::DockTab::Files;
                    s.dock.set_open(true);
                    actions::refresh_repo(s);
                })
                .build(k)
                .key("composer-attach"),
        )
        .child(
            row()
                .gap(4.0)
                .center_y()
                .child(access_glyph)
                .child(access_select),
        );
    if mode != ChatMode::Default {
        let label = if mode == ChatMode::Plan {
            "Plan"
        } else {
            "Debug"
        };
        tools = tools.child(
            row()
                .key("composer-mode")
                .px(8.0)
                .h(22.0)
                .center_y()
                .radius(11.0)
                .fill(c.bg_accent.alpha(0.12))
                .cursor(rune::Cursor::Pointer)
                .help("Back to default mode")
                .on_press_tx(|s: &mut App, tx| {
                    actions::run_command(s, slash::Command::Default, "", tx)
                })
                .child(text(label).font(k.ty.small_medium).color(c.text_accent)),
        );
    }
    if thread.is_none() && s.current_project().is_some() {
        let options = ["Local", "Worktree"];
        tools = tools.child(
            Select::new("placement", &options)
                .label("Where the thread works")
                .current(Some(if s.placement == Placement::Local {
                    "Local"
                } else {
                    "Worktree"
                }))
                .open(s.popup == Some(Popup::Workspace))
                .on_toggle(|s: &mut App| toggle(s, Popup::Workspace))
                .on_close(|s: &mut App| s.popup = None)
                .on_choose(|s: &mut App, v: &str, _| {
                    s.placement = if v == "Worktree" {
                        Placement::Worktree
                    } else {
                        Placement::Local
                    };
                    s.popup = None;
                })
                .build(k, layer),
        );
    }
    tools = tools.child(el_spacer());
    if working {
        tools = tools.child(Spinner::new().size(14.0).build(k));
    }
    tools = tools.child(
        row()
            .gap(4.0)
            .center_y()
            .child(icon(theme::provider_icon(provider), 13.0).color(c.icon_base))
            .child(model_select),
    );
    if provider.info().supports_effort {
        let labels: Vec<&str> = Effort::ALL.iter().map(|e| e.label()).collect();
        tools = tools.child(
            Select::new("effort", &labels)
                .label("Reasoning effort")
                .current(Some(effort.label()))
                .open(s.popup == Some(Popup::Effort))
                .on_toggle(|s: &mut App| toggle(s, Popup::Effort))
                .on_close(|s: &mut App| s.popup = None)
                .on_choose(|s: &mut App, v: &str, tx| {
                    if let Some(e) = Effort::ALL.into_iter().find(|e| e.label() == v)
                        && let Some(t) = s.selected.clone().and_then(|id| s.ws.thread_mut(&id))
                    {
                        t.effort = e;
                        mark_dirty(s, tx);
                    }
                    s.popup = None;
                })
                .build(k, layer),
        );
    }
    let send = if working && s.composer.text.trim().is_empty() {
        IconButton::new(&icons::STOP)
            .variant(IconButtonVariant::Contrast)
            .label("Stop")
            .on_press(|s: &mut App| {
                if let Some(id) = s.selected.clone() {
                    actions::stop_turn(s, &id);
                }
            })
    } else {
        let empty = s.composer.text.trim().is_empty();
        IconButton::new(&icons::ARROW_UP)
            .variant(if empty {
                IconButtonVariant::Neutral
            } else {
                IconButtonVariant::Submit
            })
            .label(if working { "Queue message" } else { "Send" })
            .disabled(empty)
            .on_press_tx(actions::submit)
    };
    tools = tools.child(send.build(k).key("composer-send"));

    slash_popup(s, k, layer);

    col()
        .key("composer")
        .on_drop_files(|s: &mut App, paths: &[std::path::PathBuf], _| actions::drop_files(s, paths))
        .w_full()
        .radius(18.0)
        .fill(c.bg_base)
        .border(1.0, c.border_base)
        .shadow(
            4.0,
            16.0,
            rune::Color {
                r: 0.0,
                g: 0.0,
                b: 0.0,
                a: if k.dark { 0.35 } else { 0.06 },
            },
        )
        .children(s.editing.is_some().then(|| editing_banner(k)))
        .child(queued(s, k))
        .child(col().w_full().px(16.0).pt(16.0).pb(8.0).child(field))
        .child(tools)
}

fn editing_banner(k: &Kit) -> El {
    let c = &k.c;
    row()
        .key("editing")
        .w_full()
        .px(16.0)
        .pt(10.0)
        .gap(8.0)
        .center_y()
        .child(icon(&icons::EDIT, 12.0).color(c.icon_accent))
        .child(
            text("Editing a sent message. Sending replaces it and everything after it.")
                .font(k.ty.small)
                .color(c.text_muted)
                .truncate()
                .grow(1.0),
        )
        .child(
            rune_kit::Button::new("Cancel")
                .variant(rune_kit::ButtonVariant::Ghost)
                .control_size(rune_kit::ControlSize::Small)
                .on_press(actions::cancel_edit)
                .build(k)
                .key("editing-cancel"),
        )
}

/// Messages waiting for the running turn, shown above the field until they start.
fn queued(s: &App, k: &Kit) -> El {
    let c = &k.c;
    let mut list = col().key("queued").w_full();
    let Some(id) = s.selected.clone() else {
        return list;
    };
    let Some(queue) = s.queued.get(&id).filter(|q| !q.is_empty()) else {
        return list;
    };
    for (i, text_) in queue.iter().enumerate() {
        let id = id.clone();
        list = list.child(
            row()
                .key(format!("queued-{i}"))
                .w_full()
                .px(16.0)
                .pt(10.0)
                .gap(8.0)
                .center_y()
                .child(icon(&icons::ENTER, 12.0).color(c.icon_faint))
                .child(text("Queued").font(k.ty.small_medium).color(c.text_faint))
                .child(
                    text(text_.lines().next().unwrap_or_default().to_owned())
                        .font(k.ty.small)
                        .color(c.text_muted)
                        .truncate()
                        .grow(1.0),
                )
                .child(
                    IconButton::new(&icons::CLOSE_SMALL)
                        .variant(IconButtonVariant::GhostMuted)
                        .control_size(rune_kit::ControlSize::Mini)
                        .label("Remove from queue")
                        .on_press(move |s: &mut App| {
                            if let Some(q) = s.queued.get_mut(&id)
                                && i < q.len()
                            {
                                q.remove(i);
                            }
                        })
                        .build(k)
                        .key(format!("queued-remove-{i}")),
                ),
        );
    }
    list
}

fn el_spacer() -> El {
    rune::el().grow(1.0)
}

fn toggle(s: &mut App, p: Popup) {
    s.popup = if s.popup == Some(p) { None } else { Some(p) };
}

fn choose_model(s: &mut App, provider: ProviderId, model: &str, tx: &mut Tx) {
    match s.selected.clone() {
        Some(id) => {
            if let Some(t) = s.ws.thread_mut(&id) {
                if t.provider != provider {
                    t.session = None;
                }
                t.provider = provider;
                t.model = model.to_owned();
            }
        }
        None => {
            s.ws.settings.default_provider = Some(provider);
            if let Some(p) = s
                .draft_project
                .clone()
                .and_then(|id| s.ws.projects.iter_mut().find(|p| p.id == id))
            {
                p.default_provider = Some(provider);
            }
        }
    }
    mark_dirty(s, tx);
}

fn on_key(s: &mut App, ev: &rune::KeyEvent, _: &mut Tx) -> bool {
    if s.slash_dismissed {
        return false;
    }
    let Some(hits) = slash::suggestions(&s.composer.text).filter(|h| !h.is_empty()) else {
        return false;
    };
    match ev.key {
        Key::Up => s.slash_active = (s.slash_active + hits.len() - 1) % hits.len(),
        Key::Down => s.slash_active = (s.slash_active + 1) % hits.len(),
        Key::Tab => {
            let name = hits[s.slash_active.min(hits.len() - 1)].name;
            s.composer = rune::TextEdit::new(format!("/{name} "));
            s.composer.focused = true;
        }
        Key::Escape => s.slash_dismissed = true,
        _ => return false,
    }
    true
}

fn slash_popup(s: &App, k: &Kit, layer: &mut Vec<El>) {
    let Some(hits) = slash::suggestions(&s.composer.text) else {
        return;
    };
    let items: Vec<PopupItem> = hits
        .iter()
        .map(|spec| {
            PopupItem::new(spec.name, spec.title)
                .description(spec.description)
                .trailing(
                    text(format!("/{}", spec.name))
                        .font(k.ty.small)
                        .color(k.c.text_faint),
                )
        })
        .collect();
    PopupList::new("slash", "composer")
        .label("Commands")
        .placement(rune_kit::Placement::TopStart)
        .open(true)
        .width(480.0)
        .max_height(320.0)
        .items(items)
        .empty_message("No matching command")
        .active(Some(s.slash_active.min(hits.len().saturating_sub(1))))
        .on_active(|s: &mut App, i| s.slash_active = i)
        .on_choose(|s: &mut App, id: &str, _| {
            s.composer = rune::TextEdit::new(format!("/{id} "));
            s.composer.focused = true;
        })
        .on_close(|s: &mut App| s.slash_dismissed = true)
        .build(k, layer);
}
