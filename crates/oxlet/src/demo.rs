use oxlet_core::model::{FileChange, Message, Part, Role, Schedule, ThreadStatus, Workspace};
use oxlet_core::provider::ProviderId;

const MIN: u64 = 60_000;
const HOUR: u64 = 60 * MIN;
const DAY: u64 = 24 * HOUR;

/// A populated workspace for headless scenes and screenshots. `repo` is a real checkout, so the
/// dock's git views have something to read.
pub fn workspace(now: u64, repo: &str) -> Workspace {
    let mut w = Workspace::default();
    let oxlet = w.add_project(repo);
    let api = w.add_project("/srv/atlas-api");
    let site = w.add_project("/srv/marketing-site");

    let thread =
        |w: &mut Workspace, project: &str, provider, model: &str, title: &str, age: u64, status| {
            let id = w.create_thread(project, provider, model, now - age);
            let t = w.thread_mut(&id).unwrap();
            t.title = title.to_owned();
            t.status = status;
            t.updated = now - age;
            id
        };
    let main = thread(
        &mut w,
        &oxlet,
        ProviderId::Claude,
        "claude-opus-5-5",
        "Fix context switching and the context meter",
        3 * MIN,
        ThreadStatus::Review,
    );
    {
        let t = w.thread_mut(&main).unwrap();
        t.branch = Some("oxlet/context-switch".into());
        t.pull_request = Some(1260);
        t.messages = vec![
            message(
                "m1",
                Role::User,
                now - 40 * MIN,
                vec![text(
                    "Switching the model mid-thread keeps the old context budget. Make the switch safe and show the real runtime budget in the composer.",
                )],
            ),
            Message {
                id: "m2".into(),
                agent: Some((ProviderId::Claude, "claude-opus-5-5".into())),
                usage: Some(oxlet_core::model::Usage {
                    input: 48_200,
                    output: 3_900,
                    cost_usd: Some(0.41),
                }),
                role: Role::Assistant,
                at: now - 36 * MIN,
                worked_ms: Some(4 * MIN + 20_000),
                pinned: false,
                parts: vec![
                    Part::Tool {
                        name: "Grep".into(),
                        summary: "contextWindow".into(),
                        output: String::new(),
                    },
                    Part::Tool {
                        name: "Read".into(),
                        summary: "src/composer/model_picker.rs".into(),
                        output: String::new(),
                    },
                    Part::Tool {
                        name: "Bash".into(),
                        summary: "cargo test -p composer".into(),
                        output: "test result: ok".into(),
                    },
                    Part::Tool {
                        name: "Edit".into(),
                        summary: "src/composer/model_picker.rs".into(),
                        output: String::new(),
                    },
                    text(
                        "The budget now follows the runtime of the selected model, including **Auto**.\n\n\
                         - A model switch restarts an idle runtime and resumes the same conversation.\n\
                         - Busy runtimes finish their turn first; nothing in flight is dropped.\n\
                         - The meter reads fresh usage for both the numerator and the denominator.\n\n\
                         Checks: `cargo test` 46 passed, clippy clean.",
                    ),
                    Part::Edits {
                        files: vec![
                            change("src/composer/model_picker.rs", 75, 0),
                            change("src/composer/model_menu.rs", 13, 0),
                            change("src/composer/budget.rs", 2, 0),
                            change("src/chat/view.rs", 9, 0),
                            change("src/runtime/context_window.rs", 47, 0),
                            change("src/runtime/session.rs", 31, 4),
                            change("docs/context-switch.md", 11, 0),
                        ],
                    },
                ],
            },
            message(
                "m3",
                Role::User,
                now - 12 * MIN,
                vec![text(
                    "Review it again and open a pull request if everything holds up.",
                )],
            ),
            Message {
                id: "m4".into(),
                agent: Some((ProviderId::Codex, "gpt-6.1-sol".into())),
                usage: Some(oxlet_core::model::Usage {
                    input: 21_000,
                    output: 1_200,
                    cost_usd: None,
                }),
                role: Role::Assistant,
                at: now - 3 * MIN,
                worked_ms: Some(MIN + 13_000),
                pinned: true,
                parts: vec![
                    Part::Tool {
                        name: "Bash".into(),
                        summary: "git push -u origin oxlet/context-switch".into(),
                        output: String::new(),
                    },
                    text(
                        "Opened **PR #1260** from `oxlet/context-switch`.\n\n\
                         | Check | Result |\n|---|---|\n| Unit tests | 46 passed |\n| Lint | 0 errors |\n| Type check | clean |\n\n\
                         ```rust\nlet budget = runtime.context_window(model).unwrap_or(DEFAULT_WINDOW);\nmeter.set(usage.tokens, budget);\n```",
                    ),
                ],
            },
        ];
    }
    let sidebar = thread(
        &mut w,
        &oxlet,
        ProviderId::Codex,
        "gpt-6.1-sol",
        "Remove the sidebar block",
        50 * MIN,
        ThreadStatus::Working,
    );
    w.thread_mut(&sidebar).unwrap().messages = vec![message(
        "s1",
        Role::User,
        now - 50 * MIN,
        vec![text(
            "The sidebar shows an empty block under Pinned. Remove it.",
        )],
    )];
    thread(
        &mut w,
        &api,
        ProviderId::Claude,
        "claude-sonnet-5-5",
        "Compaction keeps tool results",
        2 * HOUR,
        ThreadStatus::NeedsInput,
    );
    thread(
        &mut w,
        &api,
        ProviderId::Codex,
        "",
        "Fix pull request rate limits",
        3 * HOUR,
        ThreadStatus::Idle,
    );
    thread(
        &mut w,
        &site,
        ProviderId::Gemini,
        "",
        "Minimise the effort slider",
        5 * HOUR,
        ThreadStatus::Done,
    );
    let y = thread(
        &mut w,
        &oxlet,
        ProviderId::Claude,
        "claude-opus-5-5",
        "Import projects and voice shortcut",
        DAY + 2 * HOUR,
        ThreadStatus::Review,
    );
    w.thread_mut(&y).unwrap().pull_request = Some(1259);
    thread(
        &mut w,
        &api,
        ProviderId::Cursor,
        "",
        "Retry flaky webhook tests",
        DAY + 5 * HOUR,
        ThreadStatus::Done,
    );
    thread(
        &mut w,
        &site,
        ProviderId::OpenCode,
        "",
        "Pricing page copy pass",
        4 * DAY,
        ThreadStatus::Idle,
    );
    let pinned = thread(
        &mut w,
        &oxlet,
        ProviderId::Claude,
        "claude-opus-5-5",
        "Persistent goals across providers",
        2 * DAY,
        ThreadStatus::Idle,
    );
    w.thread_mut(&pinned).unwrap().pinned = true;
    w.thread_mut(&sidebar).unwrap().unread = false;
    w.thread_mut(&y).unwrap().unread = true;

    w.automations.push(oxlet_core::model::Automation {
        id: "a1".into(),
        project: api.clone(),
        name: "Nightly dependency audit".into(),
        prompt: "Check for outdated or vulnerable dependencies and open a pull request with safe updates.".into(),
        provider: ProviderId::Codex,
        model: String::new(),
        schedule: Schedule::Weekdays { hour: 2 },
        enabled: true,
        last_run: Some(now - 20 * HOUR),
        runs: vec![],
    });
    w.automations.push(oxlet_core::model::Automation {
        id: "a2".into(),
        project: oxlet,
        name: "Triage new issues".into(),
        prompt: "Label and summarise issues opened since the last run.".into(),
        provider: ProviderId::Claude,
        model: "claude-sonnet-5-5".into(),
        schedule: Schedule::Hourly,
        enabled: false,
        last_run: None,
        runs: vec![],
    });
    w
}

fn text(t: &str) -> Part {
    Part::Text { text: t.to_owned() }
}

fn change(path: &str, added: u32, removed: u32) -> FileChange {
    FileChange {
        path: path.into(),
        added,
        removed,
    }
}

fn message(id: &str, role: Role, at: u64, parts: Vec<Part>) -> Message {
    Message {
        id: id.into(),
        role,
        parts,
        at,
        worked_ms: None,
        pinned: false,
        agent: None,
        usage: None,
    }
}
