use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use oxlet_core::model::{Part, Role, ThreadStatus, Workspace};
use oxlet_core::provider::ProviderId;
use oxlet_core::target::{Place, Target};
use rune::{Env, Runtime, TextEdit};

use crate::state::{App, Pending, Screen, Section};

struct Sandbox {
    root: PathBuf,
}

impl Sandbox {
    fn new(name: &str) -> Sandbox {
        let root = std::env::temp_dir().join(format!("oxlet-e2e-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("repo")).unwrap();
        std::fs::create_dir_all(root.join("bin")).unwrap();
        let repo = root.join("repo");
        for args in [
            &["init", "-q", "-b", "main"][..],
            &["config", "user.email", "t@example.com"],
            &["config", "user.name", "T"],
        ] {
            assert!(
                std::process::Command::new("git")
                    .args(args)
                    .current_dir(&repo)
                    .status()
                    .unwrap()
                    .success()
            );
        }
        std::fs::write(repo.join("README.md"), "hello\n").unwrap();
        oxlet_core::git::commit_all(&Place::local(repo.to_str().unwrap()), "init").unwrap();
        Sandbox { root }
    }

    fn repo(&self) -> String {
        self.root.join("repo").to_string_lossy().into_owned()
    }

    /// A stand-in for an agent CLI: records its arguments, then runs `body`.
    fn agent(&self, name: &str, body: &str) -> PathBuf {
        let path = self.root.join("bin").join(name);
        let log = self.root.join(format!("{name}.args"));
        let script = format!(
            "#!/bin/sh\nfor a in \"$@\"; do printf '%s\\n' \"$a\"; done >> '{}'\necho '<<turn>>' >> '{}'\n{body}\n",
            log.display(),
            log.display()
        );
        std::fs::write(&path, script).unwrap();
        std::process::Command::new("chmod")
            .args(["+x", path.to_str().unwrap()])
            .status()
            .unwrap();
        path
    }

    fn args(&self, name: &str) -> String {
        std::fs::read_to_string(self.root.join(format!("{name}.args"))).unwrap_or_default()
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn runtime(sandbox: &Sandbox, providers: Vec<(ProviderId, Option<PathBuf>)>) -> Runtime<App> {
    let mut ws = Workspace::default();
    ws.add_project(&sandbox.repo());
    let mut app = App::new(ws, sandbox.root.join("home"), 0);
    app.demo = true;
    app.providers.insert(Target::Local, providers);
    let mut rt = Runtime::new(
        app,
        crate::view::view,
        (1400.0, 900.0),
        Env {
            system_dark: true,
            scale: 1.0,
        },
    );
    rt.frame(0.0);
    rt
}

/// Clicks `key` once panes and popups have finished moving.
fn click(rt: &mut Runtime<App>, key: &str) {
    for _ in 0..30 {
        rt.frame(1.0 / 60.0);
    }
    let (x, y) = rt
        .find(key)
        .unwrap_or_else(|| panic!("no element keyed {key}"));
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    rt.frame(1.0 / 60.0);
}

fn send(rt: &mut Runtime<App>, text: &str) {
    rt.update(|s, tx| {
        s.composer = TextEdit::new(text);
        s.composer.focused = true;
        tx.focus("composer-field", false);
    });
    rt.frame(1.0 / 60.0);
    rt.key(rune::KeyEvent {
        key: rune::Key::Enter,
        mods: Default::default(),
    });
    rt.frame(1.0 / 60.0);
}

fn click_send(rt: &mut Runtime<App>, text: &str) {
    rt.update(|s, _| {
        s.composer = TextEdit::new(text);
        s.composer.focused = true;
    });
    rt.frame(1.0 / 60.0);
    click(rt, "composer-send");
}

fn press(rt: &mut Runtime<App>, key: rune::Key) {
    rt.key(rune::KeyEvent {
        key,
        mods: Default::default(),
    });
    rt.frame(1.0 / 60.0);
}

fn type_text(rt: &mut Runtime<App>, text: &str) {
    for c in text.chars() {
        press(rt, rune::Key::Text(c.to_string()));
    }
}

/// The arguments of each turn `name` ran, oldest first.
fn turns(sb: &Sandbox, name: &str) -> Vec<String> {
    sb.args(name)
        .split("<<turn>>\n")
        .filter(|t| !t.is_empty())
        .map(str::to_owned)
        .collect()
}

/// Runs frames and polls until `done` holds, as the window would while workers report back.
fn pump(rt: &mut Runtime<App>, done: impl Fn(&App) -> bool) {
    let start = Instant::now();
    let mut clock = 0.0;
    while !done(&rt.state) {
        assert!(
            start.elapsed() < Duration::from_secs(15),
            "timed out waiting for the app"
        );
        clock += 0.05;
        rt.update(|s, tx| {
            crate::poll(s, clock, tx);
        });
        rt.frame(1.0 / 60.0);
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn thread(rt: &Runtime<App>) -> &oxlet_core::model::Thread {
    rt.state.thread().expect("a thread is selected")
}

fn idle(s: &App) -> bool {
    s.thread()
        .is_some_and(|t| t.status != ThreadStatus::Working)
        && s.turns.is_empty()
}

#[test]
fn a_claude_turn_streams_text_tools_and_edits_then_resumes_its_session() {
    let sb = Sandbox::new("claude");
    let repo = sb.repo();
    let claude = sb.agent(
        "claude",
        &format!(
            r#"echo '{{"type":"system","subtype":"init","session_id":"sess-1"}}'
printf 'new\n' > '{repo}/added.txt'
echo '{{"type":"assistant","message":{{"content":[{{"type":"tool_use","name":"Write","input":{{"file_path":"{repo}/added.txt"}}}}]}}}}'
echo '{{"type":"user","message":{{"content":[{{"type":"tool_result","content":"ok"}}]}}}}'
echo '{{"type":"assistant","message":{{"content":[{{"type":"text","text":"Added **added.txt**."}}]}}}}'
echo '{{"type":"result","subtype":"success","is_error":false,"duration_ms":1500,"result":"done","session_id":"sess-1"}}'"#
        ),
    );
    let mut rt = runtime(&sb, vec![(ProviderId::Claude, Some(claude))]);

    click_send(&mut rt, "Add a file");
    pump(&mut rt, |s| {
        idle(s) && s.thread().is_some_and(|t| t.last_change().is_some())
    });

    let t = thread(&rt);
    assert_eq!(t.title, "Add a file");
    assert_eq!(t.session.as_deref(), Some("sess-1"));
    assert_eq!(t.status, ThreadStatus::Review);
    let reply = &t.messages[1];
    assert_eq!(reply.role, Role::Assistant);
    assert_eq!(reply.worked_ms, Some(1500));
    assert!(reply.parts.iter().any(
        |p| matches!(p, Part::Tool { name, output, .. } if name == "Write" && output == "ok")
    ));
    assert!(reply.text().contains("Added **added.txt**."));
    let edits = t.last_change().unwrap();
    assert_eq!(edits.len(), 1);
    assert_eq!(edits[0].path, "added.txt");
    assert_eq!(edits[0].added, 1);

    send(&mut rt, "Thanks");
    pump(&mut rt, |s| {
        idle(s) && s.thread().is_some_and(|t| t.messages.len() == 4)
    });
    let args = sb.args("claude");
    let turns: Vec<&str> = args.split("<<turn>>\n").filter(|t| !t.is_empty()).collect();
    assert_eq!(turns.len(), 2);
    assert!(turns[0].contains("stream-json") && !turns[0].contains("--resume"));
    assert!(turns[1].contains("--resume\nsess-1\n") && turns[1].trim_end().ends_with("Thanks"));
}

#[test]
fn a_codex_turn_reads_json_events_and_reports_failures() {
    let sb = Sandbox::new("codex");
    let codex = sb.agent(
        "codex",
        r#"echo '{"type":"thread.started","thread_id":"th-9"}'
echo '{"type":"error","message":"Reconnecting... 2/5"}'
echo '{"type":"item.completed","item":{"type":"command_execution","command":"cargo test","aggregated_output":"ok","exit_code":0}}'
echo '{"type":"item.completed","item":{"type":"agent_message","text":"Tests pass."}}'
echo '{"type":"turn.completed","usage":{}}'"#,
    );
    let mut rt = runtime(&sb, vec![(ProviderId::Codex, Some(codex.clone()))]);
    rt.update(|s, _| s.ws.settings.default_provider = Some(ProviderId::Codex));

    send(&mut rt, "Run the tests");
    pump(&mut rt, idle);
    let t = thread(&rt);
    assert_eq!(t.provider, ProviderId::Codex);
    assert_eq!(t.session.as_deref(), Some("th-9"));
    assert_eq!(t.status, ThreadStatus::Idle);
    assert!(t.messages[1].text().contains("Tests pass."));
    assert!(sb.args("codex").starts_with("exec\n--json\n"));

    std::fs::write(&codex, "#!/bin/sh\necho 'quota exceeded' >&2\nexit 2\n").unwrap();
    send(&mut rt, "Again");
    pump(&mut rt, |s| {
        s.thread().is_some_and(|t| t.status == ThreadStatus::Failed)
    });
    let last = thread(&rt).messages.last().unwrap();
    assert!(
        last.parts
            .iter()
            .any(|p| matches!(p, Part::Error { message } if message == "quota exceeded"))
    );
}

#[test]
fn messages_sent_while_working_are_queued_and_run_in_order() {
    let sb = Sandbox::new("queue");
    let claude = sb.agent(
        "claude",
        r#"sleep 0.4
for last; do :; done
printf '{"type":"assistant","message":{"content":[{"type":"text","text":"echo %s"}]}}\n' "$last"
echo '{"type":"result","is_error":false,"session_id":"q"}'"#,
    );
    let mut rt = runtime(&sb, vec![(ProviderId::Claude, Some(claude))]);
    send(&mut rt, "first");
    send(&mut rt, "second");
    assert_eq!(rt.state.queued.values().map(|q| q.len()).sum::<usize>(), 1);
    pump(&mut rt, |s| {
        idle(s) && s.thread().is_some_and(|t| t.messages.len() == 4)
    });
    let texts: Vec<String> = thread(&rt).messages.iter().map(|m| m.text()).collect();
    assert_eq!(texts, ["first", "echo first", "second", "echo second"]);
}

#[test]
fn a_missing_provider_explains_how_to_install_it() {
    let sb = Sandbox::new("missing");
    let mut rt = runtime(&sb, vec![(ProviderId::Claude, None)]);
    send(&mut rt, "hello");
    let t = thread(&rt);
    assert_eq!(t.status, ThreadStatus::Failed);
    assert!(
        t.messages[1]
            .parts
            .iter()
            .any(|p| matches!(p, Part::Error { message } if message.contains("Claude Code")))
    );
}

#[test]
fn escape_stops_a_running_turn() {
    let sb = Sandbox::new("stop");
    let claude = sb.agent("claude", "sleep 30");
    let mut rt = runtime(&sb, vec![(ProviderId::Claude, Some(claude))]);
    send(&mut rt, "long job");
    assert!(!rt.state.turns.is_empty());
    rt.update(|s, _| s.confirm = Some(Pending::Discard("README.md".into())));
    rt.frame(1.0 / 60.0);
    press(&mut rt, rune::Key::Escape);
    assert!(rt.state.confirm.is_none(), "Escape closes the dialog");
    assert!(!rt.state.turns.is_empty(), "and only the dialog");
    assert!(rt.state.composer.focused);
    press(&mut rt, rune::Key::Escape);
    pump(&mut rt, idle);
    let t = thread(&rt);
    assert_eq!(t.status, ThreadStatus::Idle);
    assert!(
        !t.messages
            .last()
            .unwrap()
            .parts
            .iter()
            .any(|p| matches!(p, Part::Error { .. }))
    );
}

#[test]
fn slash_commands_set_goals_switch_modes_and_fork() {
    let sb = Sandbox::new("slash");
    let mut rt = runtime(&sb, vec![(ProviderId::Claude, None)]);
    send(&mut rt, "start");
    let first = rt.state.selected.clone().unwrap();
    send(&mut rt, "/goal ship the release");
    assert_eq!(thread(&rt).goal.as_deref(), Some("ship the release"));
    send(&mut rt, "/plan");
    assert_eq!(thread(&rt).mode, oxlet_core::model::ChatMode::Plan);
    send(&mut rt, "/fork");
    assert_ne!(rt.state.selected.as_deref(), Some(first.as_str()));
    assert_eq!(thread(&rt).parent.as_deref(), Some(first.as_str()));
}

#[test]
fn a_partial_slash_command_completes_before_it_runs() {
    let sb = Sandbox::new("complete");
    let mut rt = runtime(&sb, vec![]);
    send(&mut rt, "/go");
    assert_eq!(rt.state.composer.text, "/goal ");
    assert!(rt.state.ws.threads.is_empty());
}

#[test]
fn every_screen_builds_with_a_populated_workspace() {
    for (name, scene) in crate::SCENES {
        let mut app = App::new(
            crate::demo::workspace(oxlet_core::time::now_ms(), Path::new(".").to_str().unwrap()),
            PathBuf::from("/nonexistent"),
            0,
        );
        app.demo = true;
        scene(&mut app);
        let mut rt = Runtime::new(
            app,
            crate::view::view,
            (1400.0, 900.0),
            Env {
                system_dark: false,
                scale: 1.0,
            },
        );
        for _ in 0..3 {
            rt.frame(1.0 / 60.0);
        }
        let shown = match *name {
            "dock" | "terminal" => "dock",
            "new" | "add" => "hero",
            "palette" => "palette",
            "kanban" => "kanban-Backlog",
            "automations" => "automation-form",
            "settings" | "providers" => "settings",
            "confirm" => "confirm-ok",
            _ => "transcript",
        };
        assert!(rt.find(shown).is_some(), "scene {name} shows no {shown}");
    }
}

#[test]
fn a_worktree_thread_runs_every_message_in_its_own_checkout() {
    let sb = Sandbox::new("worktree");
    let cwd_log = sb.root.join("cwd.log");
    let claude = sb.agent(
        "claude",
        &format!(
            r#"pwd >> '{}'
for last; do :; done
printf '{{"type":"assistant","message":{{"content":[{{"type":"text","text":"did %s"}}]}}}}\n' "$last"
echo '{{"type":"result","is_error":false,"session_id":"w"}}'"#,
            cwd_log.display()
        ),
    );
    let mut rt = runtime(&sb, vec![(ProviderId::Claude, Some(claude))]);
    let root = sb.root.join("worktrees").to_string_lossy().into_owned();
    rt.update(|s, _| {
        s.ws.settings.worktree_root = root.clone();
        s.placement = crate::state::Placement::Worktree;
    });
    send(&mut rt, "one");
    send(&mut rt, "two");
    pump(&mut rt, |s| {
        idle(s) && s.thread().is_some_and(|t| t.messages.len() == 4)
    });

    let t = thread(&rt);
    let worktree = t.worktree.clone().expect("the thread has a worktree");
    assert!(worktree.starts_with(&root));
    assert_eq!(
        t.branch.as_deref(),
        Some(format!("oxlet/{}", t.id).as_str())
    );
    let texts: Vec<String> = t.messages.iter().map(|m| m.text()).collect();
    assert_eq!(texts, ["one", "did one", "two", "did two"]);
    let dirs = std::fs::read_to_string(&cwd_log).unwrap();
    assert_eq!(
        dirs.lines().collect::<Vec<_>>(),
        [worktree.as_str(), worktree.as_str()]
    );
}

#[test]
fn switching_provider_hands_the_conversation_to_the_new_agent() {
    let sb = Sandbox::new("handoff");
    let reply = r#"echo '{"type":"assistant","message":{"content":[{"type":"text","text":"The cache key is the path."}]}}'
echo '{"type":"result","is_error":false,"session_id":"c1"}'"#;
    let claude = sb.agent("claude", reply);
    let codex = sb.agent("codex", r#"echo '{"type":"turn.completed"}'"#);
    let mut rt = runtime(
        &sb,
        vec![
            (ProviderId::Claude, Some(claude)),
            (ProviderId::Codex, Some(codex)),
        ],
    );
    send(&mut rt, "What is the cache key?");
    pump(&mut rt, idle);
    rt.update(|s, _| {
        let id = s.selected.clone().unwrap();
        let t = s.ws.thread_mut(&id).unwrap();
        t.provider = ProviderId::Codex;
        t.session = None;
    });
    send(&mut rt, "Use it in the loader");
    pump(&mut rt, |s| {
        idle(s) && s.thread().is_some_and(|t| t.messages.len() == 4)
    });
    let prompt = sb.args("codex");
    assert!(prompt.contains("User: What is the cache key?"));
    assert!(prompt.contains("Assistant: The cache key is the path."));
    assert!(prompt.contains("is the path.\n\n---\n\nUse it in the loader\n<<turn>>\n"));
    assert!(!sb.args("claude").contains("continuing a conversation"));
}

#[test]
fn claude_text_streams_in_as_deltas_then_settles_on_the_complete_block() {
    let sb = Sandbox::new("deltas");
    let claude = sb.agent(
        "claude",
        r#"for piece in 'Hel' 'lo, ' 'world'; do
  printf '{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"%s"}}}\n' "$piece"
done
echo '{"type":"assistant","message":{"content":[{"type":"text","text":"Hello, world"}]}}'
echo '{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":" "}}}'
echo '{"type":"assistant","message":{"content":[{"type":"text","text":" "}]}}'
echo '{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Bash","input":{"command":"ls"}}]}}'
echo '{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"Done"}}}'
echo '{"type":"assistant","message":{"content":[{"type":"text","text":"Done."}]}}'
echo '{"type":"result","is_error":false,"session_id":"d"}'"#,
    );
    let mut rt = runtime(&sb, vec![(ProviderId::Claude, Some(claude))]);
    send(&mut rt, "hi");
    pump(&mut rt, idle);
    let parts = &thread(&rt).messages[1].parts;
    assert_eq!(parts.len(), 3);
    assert!(matches!(&parts[0], Part::Text { text } if text == "Hello, world"));
    assert!(matches!(&parts[1], Part::Tool { name, .. } if name == "Bash"));
    assert!(matches!(&parts[2], Part::Text { text } if text == "Done."));
    assert!(sb.args("claude").contains("--include-partial-messages"));
}

fn echo_agent(sb: &Sandbox) -> PathBuf {
    sb.agent(
        "claude",
        r#"for last; do :; done
last=$(printf '%s' "$last" | tail -n 1)
printf '{"type":"assistant","message":{"content":[{"type":"text","text":"re: %s"}]}}\n' "$last"
echo '{"type":"result","is_error":false,"session_id":"e","total_cost_usd":0.02,"usage":{"input_tokens":1000,"output_tokens":200}}'"#,
    )
}

#[test]
fn replies_record_usage_and_threads_total_it() {
    let sb = Sandbox::new("usage");
    let mut rt = runtime(&sb, vec![(ProviderId::Claude, Some(echo_agent(&sb)))]);
    send(&mut rt, "one");
    pump(&mut rt, idle);
    send(&mut rt, "two");
    pump(&mut rt, |s| {
        idle(s) && s.thread().is_some_and(|t| t.messages.len() == 4)
    });
    let t = thread(&rt);
    assert_eq!(t.messages[1].usage.unwrap().tokens(), 1200);
    assert_eq!(t.usage().unwrap().label(), "2.4k tokens · $0.04");
}

#[test]
fn retry_replaces_the_latest_reply() {
    let sb = Sandbox::new("retry");
    let mut rt = runtime(&sb, vec![(ProviderId::Claude, Some(echo_agent(&sb)))]);
    send(&mut rt, "hello");
    pump(&mut rt, idle);
    let first = thread(&rt).messages[1].id.clone();
    let id = rt.state.selected.clone().unwrap();
    rt.update(|s, tx| crate::actions::retry(s, &id, tx));
    pump(&mut rt, |s| {
        idle(s)
            && s.thread()
                .is_some_and(|t| t.messages.len() == 2 && t.messages[1].id != first)
    });
    let texts: Vec<String> = thread(&rt).messages.iter().map(|m| m.text()).collect();
    assert_eq!(texts, ["hello", "re: hello"]);
    let turns = turns(&sb, "claude");
    assert!(
        !turns[1].contains("--resume"),
        "a retry starts a fresh session"
    );
}

#[test]
fn editing_a_message_rewinds_and_starts_a_fresh_session() {
    let sb = Sandbox::new("edit");
    let mut rt = runtime(&sb, vec![(ProviderId::Claude, Some(echo_agent(&sb)))]);
    send(&mut rt, "first");
    pump(&mut rt, idle);
    send(&mut rt, "second");
    pump(&mut rt, |s| {
        idle(s) && s.thread().is_some_and(|t| t.messages.len() == 4)
    });
    let (id, target) = (
        rt.state.selected.clone().unwrap(),
        thread(&rt).messages[2].id.clone(),
    );
    rt.update(|s, _| crate::actions::edit_message(s, &id, &target));
    assert_eq!(rt.state.composer.text, "second");
    send(&mut rt, "second, edited");
    pump(&mut rt, |s| {
        idle(s) && s.thread().is_some_and(|t| t.messages.len() == 4)
    });
    let texts: Vec<String> = thread(&rt).messages.iter().map(|m| m.text()).collect();
    assert_eq!(
        texts,
        ["first", "re: first", "second, edited", "re: second, edited"]
    );
    let last_turn = sb
        .args("claude")
        .rsplit("<<turn>>\n")
        .nth(1)
        .unwrap_or_default()
        .to_owned();
    assert!(
        !last_turn.contains("--resume"),
        "an edit starts a fresh session"
    );
    assert!(last_turn.contains("User: first") && !last_turn.contains("User: second\n"));
}

#[test]
fn dropped_files_become_mentions_in_the_composer() {
    let sb = Sandbox::new("drop");
    let mut rt = runtime(&sb, vec![]);
    let repo = PathBuf::from(sb.repo());
    rt.update(|s, _| {
        s.composer = TextEdit::new("Look at");
        crate::actions::drop_files(s, &[repo.join("README.md"), repo.join("src/lib.rs")]);
    });
    assert_eq!(rt.state.composer.text, "Look at @README.md @src/lib.rs ");
}

#[test]
fn add_project_picks_a_folder_in_the_dialog() {
    let sb = Sandbox::new("pick");
    let other = sb.root.join("other");
    std::fs::create_dir_all(&other).unwrap();
    let mut rt = runtime(&sb, vec![]);
    click(&mut rt, "project-add");
    assert_eq!(
        rt.file_dialog().map(|(kind, _)| kind),
        Some(rune::FileDialogKind::Folder)
    );
    rt.resolve_file_dialog(Some(other.clone())).unwrap();
    rt.frame(1.0 / 60.0);
    let added = rt.state.current_project().unwrap();
    assert_eq!(added.path, other.to_string_lossy());
    assert_eq!(added.target, Target::Local);
    assert_eq!(rt.state.ws.projects.len(), 2);
}

#[test]
fn a_cancelled_folder_dialog_changes_nothing() {
    let sb = Sandbox::new("pick-cancel");
    let mut rt = runtime(&sb, vec![]);
    let before = rt.state.ws.clone();
    click(&mut rt, "project-add");
    rt.resolve_file_dialog(None).unwrap();
    rt.frame(1.0 / 60.0);
    assert_eq!(rt.state.ws, before);
    assert!(rt.state.adding_project.is_none() && rt.state.toast.is_none());
}

/// A stand-in ssh client for every test in this run: it logs the host it was given, then runs
/// the remote command here, in a session of its own as sshd would, so stopping the client does
/// not reach it.
fn fake_ssh() -> PathBuf {
    static ONCE: std::sync::Once = std::sync::Once::new();
    let dir = std::env::temp_dir().join("oxlet-e2e-ssh");
    ONCE.call_once(|| {
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("home")).unwrap();
        let script = dir.join("ssh");
        std::fs::write(
            &script,
            "#!/bin/sh\nwhile [ \"$1\" != -- ]; do shift; done\nprintf '%s\\n' \"$2\" >> \"$0.log\"\nshift 2\nexport HOME=\"${0%/*}/home\" SHELL=/bin/sh\nexec setsid -w sh -c \"$*\"\n",
        )
        .unwrap();
        std::process::Command::new("chmod")
            .args(["+x", script.to_str().unwrap()])
            .status()
            .unwrap();
        // SAFETY: the tests read the environment only through std, whose lock orders this write
        // with their reads.
        unsafe { std::env::set_var("OXLET_SSH", &script) };
    });
    dir.join("ssh.log")
}

#[test]
fn an_ssh_project_runs_its_turns_and_git_through_the_host() {
    let sb = Sandbox::new("ssh");
    let repo = sb.repo();
    let log = fake_ssh();
    let host = Target::Ssh {
        host: "me@devbox-turn".into(),
    };
    let claude = sb.agent(
        "claude",
        &format!(
            r#"printf 'changed\n' >> '{repo}/README.md'
printf 'one\ntwo\nthree\n' > '{repo}/notes.md'
for piece in 'Done ' 'remotely'; do
  printf '{{"type":"stream_event","event":{{"type":"content_block_delta","delta":{{"type":"text_delta","text":"%s"}}}}}}\n' "$piece"
done
echo '{{"type":"assistant","message":{{"content":[{{"type":"tool_use","name":"Write","input":{{"file_path":"{repo}/notes.md"}}}}]}}}}'
echo '{{"type":"assistant","message":{{"content":[{{"type":"text","text":"Done remotely."}}]}}}}'
echo '{{"type":"result","is_error":false,"session_id":"r1"}}'"#
        ),
    );
    let mut rt = runtime(&sb, vec![]);
    rt.update(|s, tx| {
        s.ws.projects.clear();
        s.providers
            .insert(host.clone(), vec![(ProviderId::Claude, Some(claude))]);
        crate::actions::add_project(s, &format!("me@devbox-turn:{repo}"), tx);
    });
    pump(&mut rt, |s| !s.ws.projects.is_empty());
    let project = &rt.state.ws.projects[0];
    assert_eq!(
        (&project.target, project.path.as_str()),
        (&host, repo.as_str())
    );

    send(&mut rt, "Change something");
    pump(&mut rt, |s| {
        idle(s) && s.thread().is_some_and(|t| t.last_change().is_some())
    });
    let t = thread(&rt);
    assert_eq!(t.messages[1].text(), "Done remotely.");
    assert_eq!(t.last_change().unwrap()[0].path, "notes.md");
    assert_eq!(t.last_change().unwrap()[0].added, 3);

    pump(&mut rt, |s| !s.repo.loading && s.repo.changes.len() == 2);
    assert_eq!(rt.state.repo.cwd.target, host);
    assert_eq!(rt.state.repo.status.branch.as_deref(), Some("main"));
    let changes: Vec<(&str, u32)> = rt
        .state
        .repo
        .changes
        .iter()
        .map(|c| (c.path.as_str(), c.added))
        .collect();
    assert_eq!(changes, [("README.md", 1), ("notes.md", 3)]);
    assert!(
        std::fs::read_to_string(log)
            .unwrap()
            .contains("me@devbox-turn\n")
    );
}

#[test]
fn stopping_an_ssh_turn_ends_the_remote_program() {
    let sb = Sandbox::new("ssh-stop");
    fake_ssh();
    let pid_file = sb.root.join("agent.pid");
    let claude = sb.agent(
        "claude",
        &format!("echo $$ > '{}'\nexec sleep 30", pid_file.display()),
    );
    let host = Target::Ssh {
        host: "devbox-stop".into(),
    };
    let mut rt = runtime(&sb, vec![]);
    rt.update(|s, _| {
        s.ws.projects[0].target = host.clone();
        s.providers
            .insert(host.clone(), vec![(ProviderId::Claude, Some(claude))]);
    });
    send(&mut rt, "long job");
    pump(&mut rt, |_| {
        std::fs::read_to_string(&pid_file).is_ok_and(|p| p.ends_with('\n'))
    });
    let pid = std::fs::read_to_string(&pid_file)
        .unwrap()
        .trim()
        .to_owned();
    rt.update(|s, tx| {
        crate::on_key(
            s,
            &rune::KeyEvent {
                key: rune::Key::Escape,
                mods: Default::default(),
            },
            tx,
        );
    });
    pump(&mut rt, idle);
    let alive = || {
        std::process::Command::new("kill")
            .args(["-0", &pid])
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap()
            .success()
    };
    let start = Instant::now();
    while alive() && start.elapsed() < Duration::from_secs(5) {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(!alive(), "the remote agent outlived its stopped turn");
}

#[test]
fn discarding_a_file_waits_for_confirmation() {
    let sb = Sandbox::new("discard");
    let repo = PathBuf::from(sb.repo());
    std::fs::write(repo.join("README.md"), "changed\n").unwrap();
    let mut rt = runtime(&sb, vec![]);
    rt.update(|s, _| crate::actions::refresh_repo(s));
    pump(&mut rt, |s| s.repo.changes.len() == 1);
    rt.update(|s, _| s.confirm = Some(Pending::Discard("README.md".into())));
    rt.frame(1.0 / 60.0);
    assert!(rt.find("confirm-ok").is_some());
    assert_eq!(
        std::fs::read_to_string(repo.join("README.md")).unwrap(),
        "changed\n"
    );
    click(&mut rt, "confirm-ok");
    pump(&mut rt, |s| s.repo.changes.is_empty() && !s.repo.loading);
    assert_eq!(
        std::fs::read_to_string(repo.join("README.md")).unwrap(),
        "hello\n"
    );
}

/// A thread in its own worktree, after one finished turn.
fn worktree_thread(sb: &Sandbox, rt: &mut Runtime<App>) -> (String, String) {
    let root = sb.root.join("worktrees").to_string_lossy().into_owned();
    rt.update(|s, _| {
        s.ws.settings.worktree_root = root;
        s.placement = crate::state::Placement::Worktree;
    });
    send(rt, "one");
    pump(rt, |s| {
        idle(s) && s.thread().is_some_and(|t| t.worktree.is_some())
    });
    let t = thread(rt);
    (t.id.clone(), t.worktree.clone().unwrap())
}

fn branch_exists(sb: &Sandbox, branch: &str) -> bool {
    let out = std::process::Command::new("git")
        .args(["branch", "--list", branch])
        .current_dir(sb.repo())
        .output()
        .unwrap();
    !out.stdout.is_empty()
}

#[test]
fn removing_a_worktree_asks_then_moves_its_thread_to_the_project_folder() {
    let sb = Sandbox::new("wt-remove");
    let mut rt = runtime(&sb, vec![(ProviderId::Claude, Some(echo_agent(&sb)))]);
    let (id, path) = worktree_thread(&sb, &mut rt);
    rt.update(|s, _| crate::actions::go(s, Screen::Settings(Section::Worktrees)));
    rt.frame(1.0 / 60.0);
    click(&mut rt, &format!("worktree-remove-{id}"));
    assert_eq!(rt.state.confirm, Some(Pending::RemoveWorktree(id.clone())));
    assert!(Path::new(&path).exists() && rt.state.ws.thread(&id).unwrap().worktree.is_some());
    click(&mut rt, "confirm-ok");
    pump(&mut rt, |s| {
        s.ws.thread(&id).is_some_and(|t| t.worktree.is_none())
    });
    let t = rt.state.ws.thread(&id).unwrap();
    assert!(t.branch.is_none());
    assert!(!Path::new(&path).exists());
    assert!(
        branch_exists(&sb, &format!("oxlet/{id}")),
        "the branch stays"
    );
}

#[test]
fn deleting_an_archived_thread_takes_its_worktree_and_automation_runs() {
    let sb = Sandbox::new("delete");
    let mut rt = runtime(&sb, vec![(ProviderId::Claude, Some(echo_agent(&sb)))]);
    let (id, path) = worktree_thread(&sb, &mut rt);
    rt.update(|s, tx| {
        crate::sidebar::archive(s, &id, tx);
        s.ws.automations.push(oxlet_core::model::Automation {
            id: "a1".into(),
            project: s.ws.projects[0].id.clone(),
            name: "Nightly".into(),
            prompt: "check".into(),
            provider: ProviderId::Claude,
            model: String::new(),
            schedule: oxlet_core::model::Schedule::Hourly,
            enabled: false,
            last_run: None,
            runs: vec![id.clone()],
        });
        crate::actions::go(s, Screen::Settings(Section::Archived));
    });
    rt.frame(1.0 / 60.0);
    click(&mut rt, &format!("delete-{id}"));
    assert!(
        rt.state.ws.thread(&id).is_some(),
        "nothing goes before the confirmation"
    );
    click(&mut rt, "confirm-ok");
    assert!(rt.state.ws.thread(&id).is_none());
    assert!(rt.state.ws.automations[0].runs.is_empty());
    pump(&mut rt, |_| !Path::new(&path).exists());
}

#[test]
fn removing_a_project_stops_its_turns_and_leaves_its_folder() {
    let sb = Sandbox::new("project-remove");
    let pid_file = sb.root.join("agent.pid");
    let claude = sb.agent(
        "claude",
        &format!("echo $$ > '{}'\nexec sleep 30", pid_file.display()),
    );
    let mut rt = runtime(&sb, vec![(ProviderId::Claude, Some(claude))]);
    send(&mut rt, "long job");
    pump(&mut rt, |_| {
        std::fs::read_to_string(&pid_file).is_ok_and(|p| p.ends_with('\n'))
    });
    let pid = std::fs::read_to_string(&pid_file)
        .unwrap()
        .trim()
        .to_owned();
    let project = rt.state.ws.projects[0].id.clone();
    let row = format!("project-{project}");

    let (x, y) = rt.find(&row).unwrap();
    rt.secondary_down(x, y);
    rt.secondary_up(x, y);
    rt.frame(1.0 / 60.0);
    click(&mut rt, "project-menu-item0");
    type_text(&mut rt, "renamed");
    press(&mut rt, rune::Key::Enter);
    assert_eq!(rt.state.ws.projects[0].name, "renamed");

    let (x, y) = rt.find(&row).unwrap();
    rt.secondary_down(x, y);
    rt.secondary_up(x, y);
    rt.frame(1.0 / 60.0);
    click(&mut rt, "project-menu-item2");
    assert_eq!(rt.state.confirm, Some(Pending::RemoveProject(project)));
    click(&mut rt, "confirm-ok");
    assert!(rt.state.ws.projects.is_empty() && rt.state.ws.threads.is_empty());
    assert!(rt.state.turns.is_empty() && rt.state.selected.is_none());
    assert!(Path::new(&sb.repo()).join("README.md").exists());
    let alive = || {
        std::process::Command::new("kill")
            .args(["-0", &pid])
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap()
            .success()
    };
    let start = Instant::now();
    while alive() && start.elapsed() < Duration::from_secs(5) {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(!alive(), "the removed project's turn kept running");
}

#[test]
fn an_edit_left_behind_in_another_thread_rewinds_nothing() {
    let sb = Sandbox::new("edit-switch");
    let mut rt = runtime(&sb, vec![(ProviderId::Claude, Some(echo_agent(&sb)))]);
    send(&mut rt, "in a");
    pump(&mut rt, idle);
    let a = rt.state.selected.clone().unwrap();
    rt.update(|s, _| crate::actions::new_thread(s));
    send(&mut rt, "in b");
    pump(&mut rt, idle);
    let b = rt.state.selected.clone().unwrap();
    let first = rt.state.ws.thread(&a).unwrap().messages[0].id.clone();
    rt.update(|s, tx| {
        crate::actions::select_thread(s, &a, tx);
        crate::actions::edit_message(s, &a, &first);
    });
    rt.frame(1.0 / 60.0);
    click(&mut rt, &format!("thread-{b}"));
    send(&mut rt, "more in b");
    pump(&mut rt, |s| {
        idle(s) && s.thread().is_some_and(|t| t.messages.len() == 4)
    });
    assert_eq!(rt.state.ws.thread(&a).unwrap().messages.len(), 2);
    assert_eq!(rt.state.selected.as_deref(), Some(b.as_str()));
}

#[test]
fn notes_belong_to_their_thread() {
    let sb = Sandbox::new("notes");
    let mut rt = runtime(&sb, vec![(ProviderId::Claude, None)]);
    send(&mut rt, "a");
    let a = rt.state.selected.clone().unwrap();
    rt.update(|s, _| crate::actions::new_thread(s));
    send(&mut rt, "b");
    let b = rt.state.selected.clone().unwrap();
    rt.update(|s, tx| {
        crate::actions::select_thread(s, &a, tx);
        s.dock_tab = crate::state::DockTab::Notes;
        s.dock.set_open(true);
    });
    rt.frame(1.0 / 60.0);
    click(&mut rt, "notes-input");
    type_text(&mut rt, "for a");
    click(&mut rt, &format!("thread-{b}"));
    click(&mut rt, "notes-input");
    type_text(&mut rt, "for b");
    assert_eq!(rt.state.ws.thread(&a).unwrap().notes, "for a");
    assert_eq!(rt.state.ws.thread(&b).unwrap().notes, "for b");
}

#[test]
fn choices_on_the_new_thread_screen_carry_into_its_thread() {
    let sb = Sandbox::new("draft");
    let codex = sb.agent("codex", r#"echo '{"type":"turn.completed"}'"#);
    let mut rt = runtime(&sb, vec![(ProviderId::Codex, Some(codex))]);
    rt.update(|s, _| {
        s.ws.settings.default_provider = Some(ProviderId::Codex);
        s.ws.settings.hidden_providers = vec![ProviderId::Claude];
    });
    rt.frame(1.0 / 60.0);
    send(&mut rt, "/plan");
    click(&mut rt, "model-trigger");
    click(&mut rt, "option1");
    click(&mut rt, "effort-trigger");
    click(&mut rt, "option0");
    assert!(rt.state.ws.threads.is_empty());
    send(&mut rt, "do it");
    pump(&mut rt, idle);
    let t = thread(&rt);
    assert_eq!(
        (t.provider, t.model.as_str(), t.effort, t.mode),
        (
            ProviderId::Codex,
            "gpt-6.1-sol",
            oxlet_core::model::Effort::Low,
            oxlet_core::model::ChatMode::Plan
        )
    );
    let args = sb.args("codex");
    assert!(args.contains("--sandbox\nread-only\n") && args.contains("-m\ngpt-6.1-sol\n"));
}

#[test]
fn a_session_reported_after_clear_is_ignored() {
    let sb = Sandbox::new("clear");
    let claude = sb.agent(
        "claude",
        r#"sleep 0.5
echo '{"type":"system","subtype":"init","session_id":"old"}'
echo '{"type":"result","is_error":false,"session_id":"old"}'"#,
    );
    let mut rt = runtime(&sb, vec![(ProviderId::Claude, Some(claude))]);
    send(&mut rt, "long");
    send(&mut rt, "/clear");
    pump(&mut rt, idle);
    assert_eq!(thread(&rt).session, None);
}

#[test]
fn compact_hands_only_the_summary_to_a_fresh_session() {
    let sb = Sandbox::new("compact");
    let claude = sb.agent(
        "claude",
        r#"for last; do :; done
case "$last" in *Summarise*) reply="the summary";; *) reply="re: $(printf '%s' "$last" | tail -n 1)";; esac
sleep 0.3
printf '{"type":"assistant","message":{"content":[{"type":"text","text":"%s"}]}}\n' "$reply"
echo '{"type":"result","is_error":false,"session_id":"s"}'"#,
    );
    let mut rt = runtime(&sb, vec![(ProviderId::Claude, Some(claude))]);
    send(&mut rt, "first");
    send(&mut rt, "/compact");
    pump(&mut rt, |s| {
        idle(s) && s.thread().is_some_and(|t| t.messages.len() == 5)
    });
    assert_eq!(thread(&rt).session, None);
    send(&mut rt, "next");
    pump(&mut rt, |s| {
        idle(s) && s.thread().is_some_and(|t| t.messages.len() == 7)
    });
    let turns = turns(&sb, "claude");
    assert!(
        turns[1].contains("--resume\ns\n"),
        "the compaction reads the old session"
    );
    assert!(!turns[2].contains("--resume"));
    assert!(turns[2].contains("Assistant: the summary") && !turns[2].contains("User: first"));
}

#[test]
fn stopping_a_turn_keeps_its_queue_in_order() {
    let sb = Sandbox::new("stop-queue");
    let claude = sb.agent(
        "claude",
        r#"for last; do :; done
[ "$last" = first ] && exec sleep 30
printf '{"type":"assistant","message":{"content":[{"type":"text","text":"re: %s"}]}}\n' "$last"
echo '{"type":"result","is_error":false,"session_id":"q"}'"#,
    );
    let mut rt = runtime(&sb, vec![(ProviderId::Claude, Some(claude))]);
    send(&mut rt, "first");
    send(&mut rt, "second");
    let id = rt.state.selected.clone().unwrap();
    rt.update(|s, _| crate::actions::stop_turn(s, &id));
    pump(&mut rt, |s| s.turns.is_empty());
    assert_eq!(rt.state.queued[&id].len(), 1);
    send(&mut rt, "third");
    pump(&mut rt, |s| {
        idle(s) && s.thread().is_some_and(|t| t.messages.len() == 6)
    });
    let asked: Vec<String> = thread(&rt)
        .messages
        .iter()
        .filter(|m| m.role == Role::User)
        .map(|m| m.text())
        .collect();
    assert_eq!(asked, ["first", "second", "third"]);
}

#[test]
fn a_stopped_command_does_not_end_the_next_one() {
    let sb = Sandbox::new("terminal");
    let mut rt = runtime(&sb, vec![]);
    rt.update(|s, _| {
        s.dock_tab = crate::state::DockTab::Terminal;
        s.dock.set_open(true);
    });
    for command in ["sleep 5", "echo second; sleep 5"] {
        rt.update(|s, tx| {
            s.terminal.input = TextEdit::new(command);
            crate::actions::run_terminal(s, tx);
        });
    }
    pump(&mut rt, |s| {
        s.terminal.lines.iter().any(|(_, l)| l == "second")
    });
    let start = Instant::now();
    pump(&mut rt, |_| start.elapsed() > Duration::from_millis(500));
    assert!(rt.state.terminal.running.is_some());
    assert!(
        !rt.state
            .terminal
            .lines
            .iter()
            .any(|(_, l)| l.starts_with("exit"))
    );
    click(&mut rt, "terminal-input");
    let mut seen = Vec::new();
    for key in [
        rune::Key::Up,
        rune::Key::Up,
        rune::Key::Down,
        rune::Key::Down,
    ] {
        press(&mut rt, key);
        seen.push(rt.state.terminal.input.text.clone());
    }
    assert_eq!(
        seen,
        [
            "echo second; sleep 5",
            "sleep 5",
            "echo second; sleep 5",
            ""
        ]
    );
    if let Some(r) = &rt.state.terminal.running {
        r.stop();
    }
}

#[test]
fn a_failed_commit_keeps_its_message() {
    let sb = Sandbox::new("commit");
    let repo = PathBuf::from(sb.repo());
    let hook = repo.join(".git/hooks/pre-commit");
    std::fs::write(&hook, "#!/bin/sh\necho 'lint failed' >&2\nexit 1\n").unwrap();
    std::process::Command::new("chmod")
        .args(["+x", hook.to_str().unwrap()])
        .status()
        .unwrap();
    std::fs::write(repo.join("README.md"), "changed\n").unwrap();
    let mut rt = runtime(&sb, vec![]);
    rt.update(|s, _| {
        s.dock.set_open(true);
        crate::actions::refresh_repo(s);
    });
    pump(&mut rt, |s| s.repo.changes.len() == 1);
    rt.update(|s, _| s.commit_message = TextEdit::new("Change the readme"));
    rt.frame(1.0 / 60.0);
    click(&mut rt, "commit");
    pump(&mut rt, |s| {
        s.toast
            .as_ref()
            .is_some_and(|t| t.text.contains("lint failed"))
    });
    assert_eq!(rt.state.commit_message.text, "Change the readme");
    std::fs::remove_file(&hook).unwrap();
    click(&mut rt, "commit");
    pump(&mut rt, |s| s.commit_message.text.is_empty());
}

#[test]
fn a_long_newest_reply_still_reaches_the_next_agent() {
    let mut w = Workspace::default();
    let p = w.add_project("/x");
    let t = w.create_thread(&p, ProviderId::Claude, "", 1);
    let long = "é".repeat(10_000);
    w.push_message(&t, Role::User, vec![Part::Text { text: "q".into() }], 2);
    w.push_message(&t, Role::Assistant, vec![Part::Text { text: long }], 3);
    let context = crate::actions::handoff(&w.thread(&t).unwrap().messages).unwrap();
    assert!(context.contains("Assistant: éé") && context.len() < 12_200);
}

#[test]
fn automations_wait_for_provider_detection() {
    let sb = Sandbox::new("automation");
    let mut rt = runtime(&sb, vec![]);
    rt.update(|s, _| {
        s.demo = false;
        s.ws.settings.notify_on_finish = false;
        s.providers.clear();
        s.ws.automations.push(oxlet_core::model::Automation {
            id: "a1".into(),
            project: s.ws.projects[0].id.clone(),
            name: "Check".into(),
            prompt: "check".into(),
            provider: ProviderId::Claude,
            model: String::new(),
            schedule: oxlet_core::model::Schedule::EveryMinutes { minutes: 15 },
            enabled: true,
            last_run: None,
            runs: Vec::new(),
        });
    });
    rt.update(|s, tx| {
        crate::poll(s, 1.0, tx);
    });
    assert!(rt.state.ws.threads.is_empty());
    let claude = echo_agent(&sb);
    rt.update(|s, tx| {
        s.providers
            .insert(Target::Local, vec![(ProviderId::Claude, Some(claude))]);
        crate::poll(s, 2.0, tx);
    });
    assert_eq!(rt.state.ws.threads.len(), 1);
    assert_eq!(rt.state.ws.automations[0].runs.len(), 1);
    rt.update(|s, _| s.selected = s.ws.automations[0].runs.first().cloned());
    pump(&mut rt, idle);
}

#[test]
fn escape_closes_the_slash_menu_for_good() {
    let sb = Sandbox::new("slash-escape");
    let mut rt = runtime(&sb, vec![]);
    rt.update(|s, tx| {
        s.composer = TextEdit::new("/cl");
        tx.focus("composer-field", false);
    });
    rt.frame(1.0 / 60.0);
    assert!(rt.find("slash-content").is_some());
    press(&mut rt, rune::Key::Escape);
    rt.frame(1.0 / 60.0);
    assert!(rt.find("slash-content").is_none());
}

#[test]
fn quitting_stops_turns_and_saves() {
    let sb = Sandbox::new("quit");
    let pid_file = sb.root.join("agent.pid");
    let claude = sb.agent(
        "claude",
        &format!("echo $$ > '{}'\nexec sleep 30", pid_file.display()),
    );
    let mut rt = runtime(&sb, vec![(ProviderId::Claude, Some(claude))]);
    rt.update(|s, _| s.demo = false);
    send(&mut rt, "long job");
    pump(&mut rt, |_| {
        std::fs::read_to_string(&pid_file).is_ok_and(|p| p.ends_with('\n'))
    });
    let pid = std::fs::read_to_string(&pid_file)
        .unwrap()
        .trim()
        .to_owned();
    rt.update(crate::quit);
    let saved = oxlet_core::store::load(&sb.root.join("home"));
    assert_eq!(saved.threads[0].messages[0].text(), "long job");
    let alive = || {
        std::process::Command::new("kill")
            .args(["-0", &pid])
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap()
            .success()
    };
    let start = Instant::now();
    while alive() && start.elapsed() < Duration::from_secs(5) {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(!alive(), "the turn outlived the app");
}

#[test]
fn the_palette_hands_goal_to_the_composer_on_the_thread() {
    let sb = Sandbox::new("palette-goal");
    let mut rt = runtime(&sb, vec![(ProviderId::Claude, None)]);
    send(&mut rt, "start");
    rt.update(|s, tx| {
        crate::actions::go(s, Screen::Kanban);
        crate::palette::open(s);
        if let Some(p) = s.palette.as_mut() {
            p.query = TextEdit::new("Goal");
        }
        tx.focus("palette-field", false);
    });
    rt.frame(1.0 / 60.0);
    press(&mut rt, rune::Key::Enter);
    assert_eq!(rt.state.screen, Screen::Thread);
    assert_eq!(rt.state.composer.text, "/goal ");
}

#[test]
fn colours_apply_once_whole_and_unapplied_drafts_go_on_blur() {
    let sb = Sandbox::new("hex");
    let mut rt = runtime(&sb, vec![]);
    rt.update(|s, _| crate::actions::go(s, Screen::Settings(Section::Appearance)));
    rt.frame(1.0 / 60.0);
    click(&mut rt, "hex-light-background");
    press(&mut rt, rune::Key::Command('a'));
    type_text(&mut rt, "#F6F");
    assert_eq!(rt.state.ws.settings.light.background, "#F6F6F3");
    type_text(&mut rt, "6F4");
    assert_eq!(rt.state.ws.settings.light.background, "#F6F6F4");
    type_text(&mut rt, "#12");
    click(&mut rt, "settings-back");
    rt.update(|s, _| crate::actions::go(s, Screen::Settings(Section::Appearance)));
    rt.frame(1.0 / 60.0);
    click(&mut rt, "hex-light-background");
    assert_eq!(rt.state.hex_fields["light-background"].text, "#F6F6F4");
}

#[test]
fn pull_requests_match_live_threads_of_this_project_only() {
    let sb = Sandbox::new("prs");
    let mut rt = runtime(&sb, vec![]);
    rt.update(|s, _| {
        let here = s.ws.projects[0].id.clone();
        let there = s.ws.add_project("/elsewhere");
        for (project, archived) in [(here, true), (there, false)] {
            let id = s.ws.create_thread(&project, ProviderId::Claude, "", 1);
            let t = s.ws.thread_mut(&id).unwrap();
            (t.branch, t.archived) = (Some("feature".into()), archived);
        }
        s.screen = Screen::PullRequests;
        s.prs = Some(Ok(vec![oxlet_core::git::PullRequest {
            number: 5,
            title: "Feature".into(),
            branch: "feature".into(),
            state: "OPEN".into(),
            author: None,
            draft: false,
            url: String::new(),
        }]));
    });
    rt.frame(1.0 / 60.0);
    assert!(rt.find("pr-5").is_some() && rt.find("pr-thread-5").is_none());
    rt.update(|s, _| s.ws.threads[0].archived = false);
    rt.frame(1.0 / 60.0);
    assert!(rt.find("pr-thread-5").is_some());
}

#[test]
fn two_projects_with_one_name_can_each_be_picked() {
    let sb = Sandbox::new("same-name");
    let mut rt = runtime(&sb, vec![]);
    let other = rt.state.ws.projects[0].path.replace("/repo", "/other/repo");
    rt.update(|s, _| {
        s.ws.add_project(&other);
        crate::actions::go(s, Screen::Automations);
    });
    rt.frame(1.0 / 60.0);
    click(&mut rt, "auto-project-trigger");
    click(&mut rt, "option1");
    assert_eq!(
        rt.state.auto_project.as_ref(),
        Some(&rt.state.ws.projects[1].id)
    );
}
