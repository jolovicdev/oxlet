use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::model::{AccessMode, ChatMode, Effort};
use crate::target::Place;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ProviderId {
    Claude,
    Codex,
    Cursor,
    OpenCode,
    Gemini,
    Droid,
}

/// How a provider's stdout is read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Protocol {
    /// Claude Code's `--output-format stream-json`, which Cursor's agent also speaks.
    ClaudeStream,
    /// `codex exec --json`.
    CodexJson,
    /// Plain text: the whole of stdout is the reply.
    Plain,
}

pub struct Provider {
    pub id: ProviderId,
    pub name: &'static str,
    pub runtime: &'static str,
    pub binary: &'static str,
    pub protocol: Protocol,
    /// `(model id, label)`, as the CLI accepts them. An empty id runs the CLI's own default model;
    /// Settings adds any other id the CLI knows.
    pub models: &'static [(&'static str, &'static str)],
    pub supports_effort: bool,
}

pub const PROVIDERS: &[Provider] = &[
    Provider {
        id: ProviderId::Claude,
        name: "Claude",
        runtime: "Claude Code",
        binary: "claude",
        protocol: Protocol::ClaudeStream,
        models: &[
            ("", "Default"),
            ("claude-opus-5-5", "Claude Opus 5.5"),
            ("claude-sonnet-5-5", "Claude Sonnet 5.5"),
            ("claude-haiku-5-5", "Claude Haiku 5.5"),
            ("claude-fable-5-2", "Claude Fable 5.2"),
        ],
        supports_effort: false,
    },
    Provider {
        id: ProviderId::Codex,
        name: "Codex",
        runtime: "Codex CLI",
        binary: "codex",
        protocol: Protocol::CodexJson,
        models: &[
            ("", "Default"),
            ("gpt-6.1-sol", "GPT-6.1 Sol"),
            ("gpt-6.1-terra", "GPT-6.1 Terra"),
            ("gpt-6-astra", "GPT-6 Astra"),
            ("gpt-6-luna", "GPT-6 Luna"),
        ],
        supports_effort: true,
    },
    Provider {
        id: ProviderId::Cursor,
        name: "Cursor",
        runtime: "Cursor agent",
        binary: "cursor-agent",
        protocol: Protocol::ClaudeStream,
        models: &[("", "Default")],
        supports_effort: false,
    },
    Provider {
        id: ProviderId::OpenCode,
        name: "OpenCode",
        runtime: "OpenCode",
        binary: "opencode",
        protocol: Protocol::Plain,
        models: &[("", "Default")],
        supports_effort: false,
    },
    Provider {
        id: ProviderId::Gemini,
        name: "Gemini",
        runtime: "Gemini CLI",
        binary: "gemini",
        protocol: Protocol::Plain,
        models: &[("", "Default")],
        supports_effort: false,
    },
    Provider {
        id: ProviderId::Droid,
        name: "Droid",
        runtime: "Factory Droid",
        binary: "droid",
        protocol: Protocol::Plain,
        models: &[("", "Default")],
        supports_effort: false,
    },
];

impl ProviderId {
    pub fn info(self) -> &'static Provider {
        PROVIDERS
            .iter()
            .find(|p| p.id == self)
            .expect("every id has an entry")
    }

    /// How a reply names its model: the listed label, the provider for its default, else the id.
    pub fn model_name(self, model: &str) -> String {
        match self.info().models.iter().find(|(id, _)| *id == model) {
            Some(("", _)) | None if model.is_empty() => self.info().name.to_owned(),
            Some((_, label)) => (*label).to_owned(),
            None => model.to_owned(),
        }
    }

    pub fn model_label(self, model: &str) -> &'static str {
        let info = self.info();
        info.models
            .iter()
            .find(|(id, _)| *id == model)
            .map_or(info.models[0].1, |(_, l)| l)
    }
}

/// Finds `binary` on `PATH`, or uses `override_path` when it names a file.
pub fn locate(
    binary: &str,
    override_path: Option<&str>,
    path_var: Option<&str>,
) -> Option<PathBuf> {
    if let Some(p) = override_path.filter(|p| !p.is_empty()) {
        return Path::new(p).is_file().then(|| PathBuf::from(p));
    }
    let path_var = path_var
        .map(str::to_owned)
        .or_else(|| std::env::var("PATH").ok())?;
    let names: &[String] = &if cfg!(windows) {
        ["exe", "cmd", "bat"]
            .iter()
            .map(|ext| format!("{binary}.{ext}"))
            .collect::<Vec<_>>()
    } else {
        vec![binary.to_owned()]
    };
    std::env::split_paths(&path_var)
        .flat_map(|dir| names.iter().map(move |n| dir.join(n)))
        .find(|candidate| candidate.is_file())
}

#[derive(Clone, Debug, PartialEq)]
pub struct TurnRequest {
    pub prompt: String,
    pub model: String,
    pub effort: Effort,
    pub access: AccessMode,
    pub mode: ChatMode,
    pub session: Option<String>,
    pub place: Place,
}

/// The program arguments for one turn, not including the program itself.
pub fn arguments(id: ProviderId, req: &TurnRequest) -> Vec<String> {
    let mut a: Vec<String> = Vec::new();
    let prompt = framed_prompt(req);
    let model = (!req.model.is_empty()).then(|| req.model.clone());
    match id {
        ProviderId::Claude | ProviderId::Cursor => {
            a.extend(["-p".into(), "--output-format".into(), "stream-json".into()]);
            if id == ProviderId::Claude {
                a.extend(["--verbose".into(), "--include-partial-messages".into()]);
                let permission = match (req.mode, req.access) {
                    (ChatMode::Plan, _) | (_, AccessMode::ReadOnly) => "plan",
                    (_, AccessMode::AutoEdit) => "acceptEdits",
                    (_, AccessMode::FullAccess) => "bypassPermissions",
                };
                a.extend(["--permission-mode".into(), permission.into()]);
            } else if req.access == AccessMode::FullAccess {
                a.push("--force".into());
            }
            if let Some(m) = model {
                a.extend(["--model".into(), m]);
            }
            if let Some(s) = &req.session {
                a.extend(["--resume".into(), s.clone()]);
            }
            // So a prompt that starts with `-` is not read as an option.
            if id == ProviderId::Claude {
                a.push("--".into());
            }
            a.push(prompt);
        }
        ProviderId::Codex => {
            a.extend([
                "exec".into(),
                "--json".into(),
                "--skip-git-repo-check".into(),
            ]);
            let sandbox = match (req.mode, req.access) {
                (ChatMode::Plan, _) | (_, AccessMode::ReadOnly) => "read-only",
                (_, AccessMode::AutoEdit) => "workspace-write",
                (_, AccessMode::FullAccess) => "danger-full-access",
            };
            a.extend(["--sandbox".into(), sandbox.into()]);
            if let Some(m) = model {
                a.extend(["-m".into(), m]);
            }
            a.extend([
                "-c".into(),
                format!("model_reasoning_effort=\"{}\"", req.effort.id()),
            ]);
            if let Some(s) = &req.session {
                a.extend(["resume".into(), s.clone()]);
            }
            a.extend(["--".into(), prompt]);
        }
        ProviderId::OpenCode => {
            a.push("run".into());
            if let Some(m) = model {
                a.extend(["-m".into(), m]);
            }
            a.extend(["--".into(), prompt]);
        }
        ProviderId::Gemini => {
            if let Some(m) = model {
                a.extend(["-m".into(), m]);
            }
            if req.access == AccessMode::FullAccess {
                a.push("--yolo".into());
            }
            a.extend(["-p".into(), prompt]);
        }
        ProviderId::Droid => {
            a.push("exec".into());
            if let Some(m) = model {
                a.extend(["-m".into(), m]);
            }
            a.push(prompt);
        }
    }
    a
}

fn framed_prompt(req: &TurnRequest) -> String {
    match req.mode {
        ChatMode::Default => req.prompt.clone(),
        ChatMode::Plan => format!(
            "Plan only. Investigate and propose a step-by-step plan; do not modify files.\n\n{}",
            req.prompt
        ),
        ChatMode::Debug => format!(
            "Debug mode. Reproduce the problem, gather evidence, and state the root cause before changing code.\n\n{}",
            req.prompt
        ),
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum AgentEvent {
    Session(String),
    /// A piece of text still being written; the complete block follows as `Text`.
    Delta(String),
    Text(String),
    Tool {
        name: String,
        summary: String,
    },
    ToolOutput(String),
    Usage(crate::model::Usage),
    Edited(String),
    /// Something to tell the user that does not end the turn, such as a reconnect.
    Notice(String),
    Done {
        duration_ms: Option<u64>,
        error: Option<String>,
    },
}

/// Parses one line of a provider's stdout. Lines that carry nothing Oxlet shows yield nothing.
pub fn parse_line(protocol: Protocol, line: &str) -> Vec<AgentEvent> {
    let line = line.trim();
    if line.is_empty() {
        return Vec::new();
    }
    match protocol {
        Protocol::Plain => vec![AgentEvent::Text(line.to_owned())],
        Protocol::ClaudeStream => serde_json::from_str(line)
            .map(|v| claude_event(&v))
            .unwrap_or_default(),
        Protocol::CodexJson => serde_json::from_str(line)
            .map(|v| codex_event(&v))
            .unwrap_or_default(),
    }
}

fn str_at<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(Value::as_str)
}

fn claude_event(v: &Value) -> Vec<AgentEvent> {
    let mut out = Vec::new();
    match str_at(v, "type") {
        Some("stream_event") => {
            let event = v.get("event").unwrap_or(&Value::Null);
            if str_at(event, "type") == Some("content_block_delta")
                && let Some(delta) = event
                    .get("delta")
                    .filter(|d| str_at(d, "type") == Some("text_delta"))
                && let Some(t) = str_at(delta, "text")
            {
                out.push(AgentEvent::Delta(t.to_owned()));
            }
        }
        Some("system") => {
            if let Some(s) = str_at(v, "session_id") {
                out.push(AgentEvent::Session(s.to_owned()));
            }
        }
        Some("assistant") => {
            let content = v.pointer("/message/content").and_then(Value::as_array);
            for block in content.into_iter().flatten() {
                match str_at(block, "type") {
                    Some("text") => {
                        if let Some(t) = str_at(block, "text") {
                            out.push(AgentEvent::Text(t.to_owned()));
                        }
                    }
                    Some("tool_use") => {
                        let name = str_at(block, "name").unwrap_or("tool").to_owned();
                        let input = block.get("input").cloned().unwrap_or(Value::Null);
                        if matches!(
                            name.as_str(),
                            "Edit" | "Write" | "MultiEdit" | "NotebookEdit"
                        ) && let Some(path) =
                            str_at(&input, "file_path").or_else(|| str_at(&input, "notebook_path"))
                        {
                            out.push(AgentEvent::Edited(path.to_owned()));
                        }
                        out.push(AgentEvent::Tool {
                            summary: tool_summary(&name, &input),
                            name,
                        });
                    }
                    _ => {}
                }
            }
        }
        Some("user") => {
            let content = v.pointer("/message/content").and_then(Value::as_array);
            for block in content
                .into_iter()
                .flatten()
                .filter(|b| str_at(b, "type") == Some("tool_result"))
            {
                let text = match block.get("content") {
                    Some(Value::String(s)) => s.clone(),
                    Some(Value::Array(items)) => items
                        .iter()
                        .filter_map(|i| str_at(i, "text"))
                        .collect::<Vec<_>>()
                        .join("\n"),
                    _ => String::new(),
                };
                out.push(AgentEvent::ToolOutput(text));
            }
        }
        Some("result") => {
            if let Some(s) = str_at(v, "session_id") {
                out.push(AgentEvent::Session(s.to_owned()));
            }
            if let Some(u) = v.get("usage") {
                let n = |k: &str| u.get(k).and_then(Value::as_u64).unwrap_or(0);
                out.push(AgentEvent::Usage(crate::model::Usage {
                    input: n("input_tokens")
                        + n("cache_creation_input_tokens")
                        + n("cache_read_input_tokens"),
                    output: n("output_tokens"),
                    cost_usd: v.get("total_cost_usd").and_then(Value::as_f64),
                }));
            }
            let failed = v.get("is_error").and_then(Value::as_bool).unwrap_or(false);
            out.push(AgentEvent::Done {
                duration_ms: v.get("duration_ms").and_then(Value::as_u64),
                error: failed.then(|| str_at(v, "result").unwrap_or("The turn failed").to_owned()),
            });
        }
        _ => {}
    }
    out
}

fn tool_summary(name: &str, input: &Value) -> String {
    let field = [
        "command",
        "file_path",
        "path",
        "pattern",
        "url",
        "query",
        "description",
    ]
    .iter()
    .find_map(|k| str_at(input, k));
    match field {
        Some(f) => first_line(f, 120),
        None => name.to_owned(),
    }
}

fn first_line(s: &str, max: usize) -> String {
    let line = s.lines().next().unwrap_or_default();
    if line.chars().count() > max {
        let mut cut: String = line.chars().take(max).collect();
        cut.push('…');
        cut
    } else {
        line.to_owned()
    }
}

fn codex_event(v: &Value) -> Vec<AgentEvent> {
    let mut out = Vec::new();
    match str_at(v, "type") {
        Some("thread.started") => {
            if let Some(id) = str_at(v, "thread_id") {
                out.push(AgentEvent::Session(id.to_owned()));
            }
        }
        Some("item.completed") => {
            let Some(item) = v.get("item") else {
                return out;
            };
            match str_at(item, "type") {
                Some("agent_message") => {
                    if let Some(t) = str_at(item, "text") {
                        out.push(AgentEvent::Text(t.to_owned()));
                    }
                }
                Some("command_execution") => {
                    let command = str_at(item, "command").unwrap_or_default();
                    out.push(AgentEvent::Tool {
                        name: "Shell".into(),
                        summary: first_line(command, 120),
                    });
                    out.push(AgentEvent::ToolOutput(
                        str_at(item, "aggregated_output")
                            .unwrap_or_default()
                            .to_owned(),
                    ));
                }
                Some("file_change") => {
                    for change in item
                        .get("changes")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                    {
                        if let Some(path) = str_at(change, "path") {
                            out.push(AgentEvent::Edited(path.to_owned()));
                        }
                    }
                }
                Some("mcp_tool_call") | Some("web_search") => {
                    let name = str_at(item, "tool")
                        .or_else(|| str_at(item, "type"))
                        .unwrap_or("tool");
                    let summary = str_at(item, "query").unwrap_or(name);
                    out.push(AgentEvent::Tool {
                        name: name.to_owned(),
                        summary: first_line(summary, 120),
                    });
                }
                _ => {}
            }
        }
        Some("turn.completed") => {
            if let Some(u) = v.get("usage") {
                let n = |k: &str| u.get(k).and_then(Value::as_u64).unwrap_or(0);
                out.push(AgentEvent::Usage(crate::model::Usage {
                    input: n("input_tokens"),
                    output: n("output_tokens"),
                    cost_usd: None,
                }));
            }
            out.push(AgentEvent::Done {
                duration_ms: None,
                error: None,
            });
        }
        Some("turn.failed") => {
            let message = v
                .pointer("/error/message")
                .and_then(Value::as_str)
                .or_else(|| str_at(v, "message"))
                .unwrap_or("The turn failed");
            out.push(AgentEvent::Done {
                duration_ms: None,
                error: Some(message.to_owned()),
            });
        }
        // Codex keeps going after an `error` (it retries); `turn.failed` or its exit ends the turn.
        Some("error") => {
            if let Some(message) = str_at(v, "message") {
                out.push(AgentEvent::Notice(message.to_owned()));
            }
        }
        _ => {}
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req() -> TurnRequest {
        TurnRequest {
            prompt: "fix it".into(),
            model: "claude-sonnet-5-5".into(),
            effort: Effort::High,
            access: AccessMode::FullAccess,
            mode: ChatMode::Default,
            session: Some("abc".into()),
            place: Place::local("/x"),
        }
    }

    #[test]
    fn claude_arguments_resume_with_permissions() {
        let a = arguments(ProviderId::Claude, &req());
        let joined = a.join(" ");
        assert!(joined.starts_with(
            "-p --output-format stream-json --verbose --include-partial-messages --permission-mode bypassPermissions"
        ));
        assert!(joined.contains("--model claude-sonnet-5-5"));
        assert!(joined.contains("--resume abc"));
        assert!(joined.ends_with("-- fix it"));
        let mut dash = req();
        dash.prompt = "-v is broken".into();
        for id in [ProviderId::Claude, ProviderId::Codex, ProviderId::OpenCode] {
            let a = arguments(id, &dash);
            assert_eq!(a[a.len() - 2..], ["--", "-v is broken"], "{id:?}");
        }
    }

    #[test]
    fn codex_plan_mode_is_read_only_and_default_model_adds_no_flag() {
        let mut r = req();
        r.mode = ChatMode::Plan;
        r.model.clear();
        r.session = None;
        let a = arguments(ProviderId::Codex, &r);
        assert!(a.windows(2).any(|w| w == ["--sandbox", "read-only"]));
        assert!(!a.iter().any(|x| x == "-m"));
        assert!(a.last().unwrap().starts_with("Plan only."));
        r.effort = Effort::ExtraHigh;
        r.session = Some("th".into());
        let a = arguments(ProviderId::Codex, &r);
        assert!(a.contains(&"model_reasoning_effort=\"xhigh\"".to_owned()));
        assert!(a.join(" ").contains("resume th -- Plan only."));
    }

    #[test]
    fn claude_stream_yields_session_text_tools_edits_and_result() {
        let lines = [
            r#"{"type":"system","subtype":"init","session_id":"s1"}"#,
            r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Looking."},{"type":"tool_use","name":"Edit","input":{"file_path":"src/a.rs"}}]}}"#,
            r#"{"type":"user","message":{"content":[{"type":"tool_result","content":[{"type":"text","text":"ok"}]}]}}"#,
            r#"{"type":"result","subtype":"success","is_error":false,"duration_ms":4200,"result":"Done","session_id":"s1"}"#,
        ];
        let events: Vec<_> = lines
            .iter()
            .flat_map(|l| parse_line(Protocol::ClaudeStream, l))
            .collect();
        assert_eq!(
            events,
            vec![
                AgentEvent::Session("s1".into()),
                AgentEvent::Text("Looking.".into()),
                AgentEvent::Edited("src/a.rs".into()),
                AgentEvent::Tool {
                    name: "Edit".into(),
                    summary: "src/a.rs".into()
                },
                AgentEvent::ToolOutput("ok".into()),
                AgentEvent::Session("s1".into()),
                AgentEvent::Done {
                    duration_ms: Some(4200),
                    error: None
                },
            ]
        );
    }

    #[test]
    fn both_protocols_report_usage_before_done() {
        let claude = r#"{"type":"result","is_error":false,"total_cost_usd":0.042,"usage":{"input_tokens":10,"cache_read_input_tokens":2000,"output_tokens":300}}"#;
        let events = parse_line(Protocol::ClaudeStream, claude);
        assert_eq!(
            events[0],
            AgentEvent::Usage(crate::model::Usage {
                input: 2010,
                output: 300,
                cost_usd: Some(0.042)
            })
        );
        assert!(matches!(events[1], AgentEvent::Done { .. }));
        let codex = r#"{"type":"turn.completed","usage":{"input_tokens":900,"cached_input_tokens":800,"output_tokens":50}}"#;
        let events = parse_line(Protocol::CodexJson, codex);
        assert_eq!(
            events[0],
            AgentEvent::Usage(crate::model::Usage {
                input: 900,
                output: 50,
                cost_usd: None
            })
        );
    }

    #[test]
    fn claude_partial_messages_yield_text_deltas() {
        let line = r#"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hel"}}}"#;
        assert_eq!(
            parse_line(Protocol::ClaudeStream, line),
            vec![AgentEvent::Delta("Hel".into())]
        );
        let tool = r#"{"type":"stream_event","event":{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{"}}}"#;
        assert!(parse_line(Protocol::ClaudeStream, tool).is_empty());
    }

    #[test]
    fn codex_json_yields_messages_commands_and_failures() {
        let lines = [
            r#"{"type":"thread.started","thread_id":"th"}"#,
            r#"{"type":"item.completed","item":{"type":"command_execution","command":"cargo test","aggregated_output":"ok","exit_code":0}}"#,
            r#"{"type":"item.completed","item":{"type":"file_change","changes":[{"path":"a.rs","kind":"update"}]}}"#,
            r#"{"type":"item.completed","item":{"type":"agent_message","text":"All green."}}"#,
            r#"{"type":"error","message":"Reconnecting... 2/5"}"#,
            r#"{"type":"turn.failed","error":{"message":"quota"}}"#,
            "not json",
        ];
        let events: Vec<_> = lines
            .iter()
            .flat_map(|l| parse_line(Protocol::CodexJson, l))
            .collect();
        assert_eq!(events[0], AgentEvent::Session("th".into()));
        assert_eq!(
            events[1],
            AgentEvent::Tool {
                name: "Shell".into(),
                summary: "cargo test".into()
            }
        );
        assert_eq!(events[3], AgentEvent::Edited("a.rs".into()));
        assert_eq!(events[4], AgentEvent::Text("All green.".into()));
        assert_eq!(events[5], AgentEvent::Notice("Reconnecting... 2/5".into()));
        assert_eq!(
            events[6],
            AgentEvent::Done {
                duration_ms: None,
                error: Some("quota".into())
            }
        );
        assert_eq!(events.len(), 7);
    }

    #[test]
    fn locate_respects_override_and_path() {
        let dir = std::env::temp_dir().join(format!("oxlet-locate-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let bin = dir.join("fakeagent");
        std::fs::write(&bin, "").unwrap();
        let path_var = dir.to_string_lossy().into_owned();
        assert_eq!(
            locate("fakeagent", None, Some(&path_var)),
            Some(bin.clone())
        );
        assert_eq!(locate("missing", None, Some(&path_var)), None);
        assert_eq!(
            locate("x", Some(bin.to_str().unwrap()), Some("")),
            Some(bin)
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
}
