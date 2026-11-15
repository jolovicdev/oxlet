use std::borrow::Cow;
use std::rc::Rc;

use oxlet_core::model::{Automation, Id, Schedule, Thread, ThreadStatus};
use oxlet_core::time;
use rune::{El, GrowthSpec, ScrollSpec, TextEdit, col, icon, row, text};
use rune_kit::{
    Badge, BadgeVariant, Button, ButtonVariant, ControlSize, IconButton, IconButtonVariant, Kit,
    Select, Spinner, Switch, TextArea, TextInput,
};
use rune_theme_opencode::icons;

use crate::actions::{self, mark_dirty, notice};
use crate::state::{App, Pending, Popup};
use crate::theme;

const COLUMNS: [(&str, &[ThreadStatus]); 5] = [
    ("Backlog", &[ThreadStatus::Idle]),
    ("In progress", &[ThreadStatus::Working]),
    (
        "Needs you",
        &[ThreadStatus::NeedsInput, ThreadStatus::Failed],
    ),
    ("Review", &[ThreadStatus::Review]),
    ("Done", &[ThreadStatus::Done]),
];

pub fn kanban(s: &App, k: &Rc<Kit>) -> El {
    let c = &k.c;
    let mut board = row()
        .w_full()
        .h_full()
        .min_h(0.0)
        .gap(12.0)
        .padding(16.0)
        .items_start();
    let recent = s.ws.recent();
    for (title, statuses) in COLUMNS {
        let cards: Rc<[Id]> = recent
            .iter()
            .filter(|t| statuses.contains(&t.status))
            .map(|t| t.id.clone())
            .collect();
        let column = if cards.is_empty() {
            text("Nothing here")
                .font(k.ty.small)
                .color(c.text_faint)
                .padding(10.0)
        } else {
            let (keys, kit) = (cards.clone(), k.clone());
            let ids = cards.clone();
            col()
                .w_full()
                .growth(GrowthSpec::default())
                .virtual_rows(
                    cards.len(),
                    86.0,
                    move |_: &App, i| Cow::Owned(keys[i].clone()),
                    move |s: &App, i| match s.ws.thread(&ids[i]) {
                        // Virtual rows take no gap from their list.
                        Some(t) => col()
                            .w_full()
                            .pb(if i + 1 < ids.len() { 8.0 } else { 0.0 })
                            .child(card(s, t, &kit)),
                        None => col(),
                    },
                )
                .virtual_rows_version(&cards)
        };
        board = board.child(
            col()
                .key(format!("kanban-{title}"))
                .grow(1.0)
                .w(180.0)
                .h_full()
                .min_h(0.0)
                .child(
                    row()
                        .px(4.0)
                        .py(10.0)
                        .gap(8.0)
                        .center_y()
                        .child(text(title).font(k.ty.ui_medium).color(c.text_base))
                        .child(
                            text(cards.len().to_string())
                                .font(k.ty.small)
                                .color(c.text_faint),
                        ),
                )
                .child(
                    col()
                        .w_full()
                        .grow(1.0)
                        .min_h(0.0)
                        .px(8.0)
                        .pb(8.0)
                        .scroll(ScrollSpec {
                            stick_bottom: false,
                        })
                        .child(column),
                ),
        );
    }
    // Five columns do not fit a narrow window: they scroll sideways instead of clipping.
    rune::el()
        .key("kanban")
        .w_full()
        .h_full()
        .min_h(0.0)
        .scroll_x()
        .child(board)
}

fn card(s: &App, t: &Thread, k: &Kit) -> El {
    let c = &k.c;
    let id = t.id.clone();
    let project = s.ws.project(&t.project).map_or("", |p| p.name.as_str());
    let done = t.status == ThreadStatus::Done;
    let toggle = IconButton::new(if done { &icons::RESET } else { &icons::CHECK })
        .variant(IconButtonVariant::GhostMuted)
        .control_size(ControlSize::Small)
        .label(if done { "Reopen" } else { "Mark done" })
        .on_press_tx({
            let id = id.clone();
            move |s: &mut App, tx| {
                if let Some(t) = s.ws.thread_mut(&id) {
                    t.status = if t.status == ThreadStatus::Done {
                        ThreadStatus::Idle
                    } else {
                        ThreadStatus::Done
                    };
                }
                mark_dirty(s, tx);
            }
        })
        .build(k)
        .key(format!("card-toggle-{}", t.id));
    let card = col()
        .key(format!("card-{}", t.id))
        .w_full()
        .gap(6.0)
        .padding(10.0)
        .radius(k.radius.md);
    theme::pressable(k, card, Some(c.bg_button_neutral))
        .cursor(rune::Cursor::Pointer)
        .role("button")
        .label(t.title.clone())
        .focusable(true)
        .on_press_tx(move |s: &mut App, tx| actions::select_thread(s, &id, tx))
        .child(
            row()
                .w_full()
                .gap(8.0)
                .items_start()
                .child(
                    text(t.title.clone())
                        .font(k.ty.ui_medium)
                        .color(c.text_base)
                        .wrap()
                        .max_lines(3)
                        .grow(1.0),
                )
                .child(toggle),
        )
        .child(
            row()
                .gap(6.0)
                .center_y()
                .child(icon(theme::provider_icon(t.provider), 12.0).color(c.icon_faint))
                .child(
                    text(project.to_owned())
                        .font(k.ty.small)
                        .color(c.text_faint),
                )
                .child(rune::el().grow(1.0))
                .child(
                    text(time::ago(t.updated, actions::now()))
                        .font(k.ty.small)
                        .color(c.text_faint),
                ),
        )
}

pub fn pull_requests(s: &App, k: &Kit) -> El {
    let c = &k.c;
    let list = match &s.prs {
        None => {
            return col()
                .w_full()
                .h_full()
                .center()
                .child(Spinner::new().size(18.0).build(k));
        }
        Some(Err(e)) => {
            return col()
                .w_full()
                .h_full()
                .center()
                .gap(6.0)
                .padding(24.0)
                .child(
                    text("Pull requests unavailable")
                        .font(k.ty.ui_medium)
                        .color(c.text_muted),
                )
                .child(
                    text(format!(
                        "{e}\nOxlet reads them through the GitHub CLI (`gh`), signed in."
                    ))
                    .font(k.ty.small)
                    .color(c.text_faint)
                    .wrap()
                    .align(rune::Align::Center),
                );
        }
        Some(Ok(list)) => list,
    };
    let mut out = col().w_full().max_w(880.0).gap(2.0);
    if list.is_empty() {
        out = out.child(
            text("No pull requests in this repository")
                .font(k.ty.ui)
                .color(c.text_faint)
                .padding(16.0),
        );
    }
    // The list is the current project's repository: only its threads can match.
    let project = s.current_project().map(|p| p.id.as_str());
    for pr in list {
        let url = pr.url.clone();
        let thread = s
            .ws
            .threads
            .iter()
            .filter(|t| !t.archived && Some(t.project.as_str()) == project)
            .find(|t| t.branch.as_deref() == Some(&pr.branch) || t.pull_request == Some(pr.number))
            .map(|t| t.id.clone());
        let (label, variant) = match (pr.state.as_str(), pr.draft) {
            ("OPEN", true) => ("Draft", BadgeVariant::Subtle),
            ("OPEN", _) => ("Open", BadgeVariant::Accent),
            ("MERGED", _) => ("Merged", BadgeVariant::Neutral),
            _ => ("Closed", BadgeVariant::Subtle),
        };
        let mut r = row()
            .key(format!("pr-{}", pr.number))
            .w_full()
            .px(14.0)
            .py(10.0)
            .gap(12.0)
            .center_y()
            .radius(k.radius.md)
            .hover(c.hover)
            .cursor(rune::Cursor::Pointer)
            .role("link")
            .label(pr.title.clone())
            .focusable(true)
            .on_press_tx(move |_: &mut App, tx| {
                let _ = tx.open_url(&url);
            })
            .child(icon(&icons::BRANCH_OUT, 15.0).color(c.icon_muted))
            .child(
                col()
                    .grow(1.0)
                    .gap(2.0)
                    .child(
                        text(pr.title.clone())
                            .font(k.ty.ui_medium)
                            .color(c.text_base)
                            .truncate(),
                    )
                    .child(
                        text(format!(
                            "#{} · {} · {}",
                            pr.number,
                            pr.branch,
                            pr.author.as_ref().map_or("", |a| a.login.as_str())
                        ))
                        .font(k.ty.small)
                        .color(c.text_faint)
                        .truncate(),
                    ),
            )
            .child(Badge::new(label).variant(variant).build(k));
        if let Some(id) = thread {
            r = r.child(
                Button::new("Open thread")
                    .variant(ButtonVariant::Ghost)
                    .control_size(ControlSize::Small)
                    .on_press_tx(move |s: &mut App, tx| actions::select_thread(s, &id, tx))
                    .build(k)
                    .key(format!("pr-thread-{}", pr.number)),
            );
        }
        out = out.child(r);
    }
    col()
        .w_full()
        .h_full()
        .min_h(0.0)
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .child(row().w_full().justify_center().padding(16.0).child(out))
}

const SCHEDULES: [&str; 4] = [
    "Every 15 minutes",
    "Every hour",
    "Daily at 09:00",
    "Weekdays at 09:00",
];

fn schedule_at(i: usize) -> Schedule {
    match i {
        0 => Schedule::EveryMinutes { minutes: 15 },
        1 => Schedule::Hourly,
        2 => Schedule::Daily { hour: 9 },
        _ => Schedule::Weekdays { hour: 9 },
    }
}

pub fn automations(s: &App, k: &Kit, layer: &mut Vec<El>) -> El {
    let c = &k.c;
    let mut out = col().w_full().max_w(720.0).gap(16.0);
    out = out.child(
        col()
            .gap(4.0)
            .child(text("Automations").font(theme::sized(k.ty.body_medium, 20.0)).color(c.text_strong))
            .child(text("Recurring agent runs. Each run opens a thread in its project while Oxlet is running.").font(k.ty.ui).color(c.text_muted).wrap()),
    );
    out = out.child(form(s, k, layer));
    for (i, a) in s.ws.automations.iter().enumerate() {
        out = out.child(automation_row(s, i, a, k));
    }
    col()
        .w_full()
        .h_full()
        .min_h(0.0)
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .child(row().w_full().justify_center().padding(24.0).child(out))
}

fn form(s: &App, k: &Kit, layer: &mut Vec<El>) -> El {
    let c = &k.c;
    let labels = s.project_labels();
    let current = s
        .auto_project
        .as_deref()
        .and_then(|id| s.ws.projects.iter().position(|p| p.id == id))
        .or((!s.ws.projects.is_empty()).then_some(0))
        .map(|i| labels[i].as_str());
    let project_select = Select::new("auto-project", &labels)
        .label("Project")
        .placeholder("Project")
        .current(current)
        .open(s.popup == Some(Popup::NewAutomationProject))
        .on_toggle(|s: &mut App| {
            s.popup = if s.popup == Some(Popup::NewAutomationProject) {
                None
            } else {
                Some(Popup::NewAutomationProject)
            }
        })
        .on_close(|s: &mut App| s.popup = None)
        .on_choose(|s: &mut App, v: &str, _| {
            if let Some(i) = s.project_labels().iter().position(|l| l == v) {
                s.auto_project = Some(s.ws.projects[i].id.clone());
            }
            s.popup = None;
        })
        .build(k, layer);
    let schedule_select = Select::new("auto-schedule", &SCHEDULES)
        .label("Schedule")
        .current(Some(SCHEDULES[s.auto_schedule]))
        .open(s.popup == Some(Popup::NewAutomationSchedule))
        .on_toggle(|s: &mut App| {
            s.popup = if s.popup == Some(Popup::NewAutomationSchedule) {
                None
            } else {
                Some(Popup::NewAutomationSchedule)
            }
        })
        .on_close(|s: &mut App| s.popup = None)
        .on_choose(|s: &mut App, v: &str, _| {
            s.auto_schedule = SCHEDULES.iter().position(|x| *x == v).unwrap_or(1);
            s.popup = None;
        })
        .build(k, layer);
    col()
        .key("automation-form")
        .w_full()
        .gap(10.0)
        .padding(14.0)
        .radius(k.radius.lg)
        .border(1.0, c.border_weak)
        .child(
            TextInput::new(&s.auto_name)
                .width(None)
                .placeholder("Name, e.g. Nightly dependency check")
                .on_edit(|s: &mut App| &mut s.auto_name)
                .build(k)
                .key("auto-name"),
        )
        .child(
            TextArea::new(&s.auto_prompt)
                .width(None)
                .placeholder("What should the agent do each time?")
                .auto_grow(3, 10)
                .on_edit(|s: &mut App| &mut s.auto_prompt)
                .build(k)
                .key("auto-prompt"),
        )
        .child(
            row()
                .w_full()
                .gap(10.0)
                .center_y()
                .child(project_select)
                .child(schedule_select)
                .child(rune::el().grow(1.0))
                .child(
                    Button::new("Create automation")
                        .variant(ButtonVariant::Submit)
                        .on_press_tx(create)
                        .build(k)
                        .key("auto-create"),
                ),
        )
}

fn create(s: &mut App, tx: &mut rune::Tx) {
    let (name, prompt) = (
        s.auto_name.text.trim().to_owned(),
        s.auto_prompt.text.trim().to_owned(),
    );
    let Some(project) = s
        .auto_project
        .clone()
        .or_else(|| s.ws.projects.first().map(|p| p.id.clone()))
    else {
        notice(s, tx, "Add a project first");
        return;
    };
    if name.is_empty() || prompt.is_empty() {
        notice(s, tx, "Give the automation a name and a prompt");
        return;
    }
    let Some(provider) =
        s.ws.project(&project)
            .map(|p| actions::default_provider(s, p))
    else {
        return;
    };
    let id = s.ws.new_id("a");
    s.ws.automations.push(Automation {
        id,
        project,
        name,
        prompt,
        provider,
        model: String::new(),
        schedule: schedule_at(s.auto_schedule),
        enabled: true,
        last_run: None,
        runs: Vec::new(),
    });
    s.auto_name = TextEdit::new("");
    s.auto_prompt = TextEdit::new("");
    mark_dirty(s, tx);
}

fn automation_row(s: &App, i: usize, a: &Automation, k: &Kit) -> El {
    let c = &k.c;
    let project =
        s.ws.project(&a.project)
            .map_or("missing project", |p| p.name.as_str());
    let last = a.last_run.map_or("Never run".to_owned(), |t| {
        match time::ago(t, actions::now()) {
            ago if ago == "now" => "Last run just now".to_owned(),
            ago => format!("Last run {ago} ago"),
        }
    });
    let card = row()
        .key(format!("automation-{}", a.id))
        .w_full()
        .gap(12.0)
        .padding(14.0)
        .radius(k.radius.lg);
    theme::glass(k, card, c.bg_button_neutral)
        .center_y()
        .child(icon(theme::provider_icon(a.provider), 16.0).color(c.icon_muted))
        .child(
            col()
                .grow(1.0)
                .gap(3.0)
                .child(text(a.name.clone()).font(k.ty.ui_medium).color(c.text_base))
                .child(
                    text(format!(
                        "{} · {} · {} · {} run{}",
                        a.schedule.label(),
                        project,
                        last,
                        a.runs.len(),
                        if a.runs.len() == 1 { "" } else { "s" }
                    ))
                    .font(k.ty.small)
                    .color(c.text_faint)
                    .truncate(),
                ),
        )
        .child(
            Button::new("Run now")
                .variant(ButtonVariant::Ghost)
                .control_size(ControlSize::Small)
                .on_press_tx(move |s: &mut App, tx| actions::run_automation(s, i, tx))
                .build(k)
                .key(format!("automation-run-{}", a.id)),
        )
        .child(
            Switch::new(a.enabled)
                .on_toggle(move |s: &mut App| {
                    if let Some(a) = s.ws.automations.get_mut(i) {
                        a.enabled = !a.enabled;
                    }
                    s.dirty = true;
                })
                .build(k)
                .label("Enabled"),
        )
        .child(
            IconButton::new(&icons::OUTLINE_TRASH)
                .variant(IconButtonVariant::GhostMuted)
                .control_size(ControlSize::Small)
                .label("Delete automation")
                .on_press({
                    let id = a.id.clone();
                    move |s: &mut App| s.confirm = Some(Pending::DeleteAutomation(id.clone()))
                })
                .build(k)
                .key(format!("automation-delete-{}", a.id)),
        )
}
