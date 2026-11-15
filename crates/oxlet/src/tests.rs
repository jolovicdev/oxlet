use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use oxlet_core::model::{Part, Role, ThreadStatus, Workspace};
use oxlet_core::provider::ProviderId;
use rune::{Env, Runtime, TextEdit};

use crate::state::App;

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
        oxlet_core::git::commit_all(repo.to_str().unwrap(), "init").unwrap();
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
    app.providers = providers;
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

fn click(rt: &mut Runtime<App>, key: &str) {
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
    let mut app = App::new(
        crate::demo::workspace(
            oxlet_core::time::now_ms(),
            Path::new(".").to_str().unwrap(),
        ),
        PathBuf::from("/nonexistent"),
        0,
    );
    app.demo = true;
    let mut rt = Runtime::new(
        app,
        crate::view::view,
        (1400.0, 900.0),
        Env {
            system_dark: false,
            scale: 1.0,
        },
    );
    rt.frame(0.0);
    for (_, scene) in crate::SCENES {
        rt.update(|s, _| scene(s));
        for _ in 0..3 {
            rt.frame(1.0 / 60.0);
        }
    }
    assert!(rt.find("settings").is_some() || rt.find("transcript").is_some());
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
