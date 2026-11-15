use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::provider::ProviderId;
use crate::target::{Place, Target};

pub type Id = String;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Project {
    pub id: Id,
    pub name: String,
    /// The path as seen on `target`.
    pub path: String,
    #[serde(default)]
    pub target: Target,
    #[serde(default)]
    pub default_provider: Option<ProviderId>,
}

impl Project {
    /// `cwd`, a folder on this project's target.
    pub fn at(&self, cwd: &str) -> Place {
        Place::new(self.target.clone(), cwd)
    }
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ThreadStatus {
    #[default]
    Idle,
    Working,
    NeedsInput,
    Review,
    Done,
    Failed,
}

impl ThreadStatus {
    pub fn label(self) -> &'static str {
        match self {
            ThreadStatus::Idle => "Idle",
            ThreadStatus::Working => "Working",
            ThreadStatus::NeedsInput => "Needs input",
            ThreadStatus::Review => "In review",
            ThreadStatus::Done => "Done",
            ThreadStatus::Failed => "Failed",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AccessMode {
    ReadOnly,
    #[default]
    AutoEdit,
    FullAccess,
}

impl AccessMode {
    pub const ALL: [AccessMode; 3] = [
        AccessMode::ReadOnly,
        AccessMode::AutoEdit,
        AccessMode::FullAccess,
    ];

    pub fn label(self) -> &'static str {
        match self {
            AccessMode::ReadOnly => "Read only",
            AccessMode::AutoEdit => "Auto-edit",
            AccessMode::FullAccess => "Full access",
        }
    }

    pub fn detail(self) -> &'static str {
        match self {
            AccessMode::ReadOnly => "Reads and plans; changes nothing",
            AccessMode::AutoEdit => "Edits files; risky commands are blocked",
            AccessMode::FullAccess => "Runs anything without asking",
        }
    }

    pub fn from_label(label: &str) -> Option<AccessMode> {
        AccessMode::ALL.into_iter().find(|m| m.label() == label)
    }
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ChatMode {
    #[default]
    Default,
    Plan,
    Debug,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Effort {
    Low,
    Medium,
    #[default]
    High,
    #[serde(rename = "xhigh")]
    ExtraHigh,
}

impl Effort {
    pub const ALL: [Effort; 4] = [Effort::Low, Effort::Medium, Effort::High, Effort::ExtraHigh];

    pub fn label(self) -> &'static str {
        match self {
            Effort::Low => "Low",
            Effort::Medium => "Medium",
            Effort::High => "High",
            Effort::ExtraHigh => "Extra high",
        }
    }

    /// The value Codex's `model_reasoning_effort` takes.
    pub fn id(self) -> &'static str {
        match self {
            Effort::Low => "low",
            Effort::Medium => "medium",
            Effort::High => "high",
            Effort::ExtraHigh => "xhigh",
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    User,
    Assistant,
    System,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct FileChange {
    pub path: String,
    pub added: u32,
    pub removed: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Part {
    Text {
        text: String,
    },
    Tool {
        name: String,
        summary: String,
        output: String,
    },
    Edits {
        files: Vec<FileChange>,
    },
    Error {
        message: String,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Message {
    pub id: Id,
    pub role: Role,
    pub parts: Vec<Part>,
    pub at: u64,
    #[serde(default)]
    pub worked_ms: Option<u64>,
    #[serde(default)]
    pub pinned: bool,
    /// The agent that wrote an assistant message; a thread can change provider between turns.
    #[serde(default)]
    pub agent: Option<(ProviderId, String)>,
    #[serde(default)]
    pub usage: Option<Usage>,
}

/// Tokens a turn used, and its cost when the provider reports one.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Usage {
    pub input: u64,
    pub output: u64,
    #[serde(default)]
    pub cost_usd: Option<f64>,
}

impl Usage {
    pub fn tokens(&self) -> u64 {
        self.input + self.output
    }

    pub fn add(&mut self, other: &Usage) {
        self.input += other.input;
        self.output += other.output;
        self.cost_usd = match (self.cost_usd, other.cost_usd) {
            (None, None) => None,
            (a, b) => Some(a.unwrap_or(0.0) + b.unwrap_or(0.0)),
        };
    }

    /// `12.4k tokens · $0.08`.
    pub fn label(&self) -> String {
        let t = self.tokens();
        let tokens = if t >= 1_000_000 {
            format!("{:.1}M tokens", t as f64 / 1e6)
        } else if t >= 1_000 {
            format!("{:.1}k tokens", t as f64 / 1e3)
        } else {
            format!("{t} tokens")
        };
        match self.cost_usd {
            Some(c) if c >= 0.01 => format!("{tokens} · ${c:.2}"),
            Some(c) if c > 0.0 => format!("{tokens} · <$0.01"),
            _ => tokens,
        }
    }
}

impl Message {
    pub fn text(&self) -> String {
        let mut out = String::new();
        for part in &self.parts {
            if let Part::Text { text } = part {
                if !out.is_empty() {
                    out.push_str("\n\n");
                }
                out.push_str(text);
            }
        }
        out
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Thread {
    pub id: Id,
    pub project: Id,
    pub title: String,
    pub provider: ProviderId,
    pub model: String,
    #[serde(default)]
    pub effort: Effort,
    #[serde(default)]
    pub access: AccessMode,
    #[serde(default)]
    pub mode: ChatMode,
    #[serde(default)]
    pub status: ThreadStatus,
    #[serde(default)]
    pub branch: Option<String>,
    #[serde(default)]
    pub worktree: Option<String>,
    #[serde(default)]
    pub pull_request: Option<u32>,
    #[serde(default)]
    pub goal: Option<String>,
    #[serde(default)]
    pub notes: String,
    #[serde(default)]
    pub session: Option<String>,
    #[serde(default)]
    pub parent: Option<Id>,
    #[serde(default)]
    pub pinned: bool,
    #[serde(default)]
    pub archived: bool,
    #[serde(default)]
    pub unread: bool,
    pub created: u64,
    pub updated: u64,
    #[serde(default)]
    pub messages: Vec<Message>,
}

impl Thread {
    pub fn cwd<'a>(&'a self, project: &'a Project) -> &'a str {
        self.worktree.as_deref().unwrap_or(&project.path)
    }

    pub fn usage(&self) -> Option<Usage> {
        let mut total: Option<Usage> = None;
        for u in self.messages.iter().filter_map(|m| m.usage.as_ref()) {
            total.get_or_insert_with(Usage::default).add(u);
        }
        total
    }

    pub fn last_change(&self) -> Option<&[FileChange]> {
        self.messages
            .iter()
            .rev()
            .flat_map(|m| m.parts.iter().rev())
            .find_map(|p| match p {
                Part::Edits { files } => Some(files.as_slice()),
                _ => None,
            })
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Schedule {
    Hourly,
    Daily { hour: u8 },
    Weekdays { hour: u8 },
    EveryMinutes { minutes: u32 },
}

impl Schedule {
    pub fn label(&self) -> String {
        match self {
            Schedule::Hourly => "Every hour".into(),
            Schedule::Daily { hour } => format!("Daily at {hour:02}:00"),
            Schedule::Weekdays { hour } => format!("Weekdays at {hour:02}:00"),
            Schedule::EveryMinutes { minutes } => format!("Every {minutes} min"),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Automation {
    pub id: Id,
    pub project: Id,
    pub name: String,
    pub prompt: String,
    pub provider: ProviderId,
    pub model: String,
    pub schedule: Schedule,
    #[serde(default = "yes")]
    pub enabled: bool,
    #[serde(default)]
    pub last_run: Option<u64>,
    #[serde(default)]
    pub runs: Vec<Id>,
}

fn yes() -> bool {
    true
}

fn one() -> f32 {
    1.0
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ThemeMode {
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Density {
    Compact,
    #[default]
    Comfortable,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Palette {
    pub accent: String,
    pub background: String,
    pub foreground: String,
}

/// A named pair of palettes, one per appearance.
pub struct Preset {
    pub name: &'static str,
    pub dark: [&'static str; 3],
    pub light: [&'static str; 3],
}

/// `[background, foreground, accent]` per appearance. Accents keep white text above 4.5:1.
pub const PRESETS: [Preset; 4] = [
    Preset {
        name: "Patina",
        dark: ["#111314", "#E6E8E5", "#16788C"],
        light: ["#F6F6F3", "#16191A", "#0B6E86"],
    },
    Preset {
        name: "Kiln",
        dark: ["#141210", "#ECE6DE", "#B5481A"],
        light: ["#F7F4EE", "#1D1A16", "#A3421A"],
    },
    Preset {
        name: "Cobalt",
        dark: ["#101218", "#E4E7EE", "#3461DE"],
        light: ["#F5F6F8", "#141821", "#2B54C9"],
    },
    Preset {
        name: "Graphite",
        dark: ["#121212", "#E8E8E8", "#4A4A4A"],
        light: ["#F7F7F7", "#151515", "#2E2E2E"],
    },
];

impl Palette {
    fn from(v: [&str; 3]) -> Palette {
        Palette {
            background: v[0].into(),
            foreground: v[1].into(),
            accent: v[2].into(),
        }
    }

    pub fn default_dark() -> Palette {
        Palette::from(PRESETS[0].dark)
    }

    pub fn default_light() -> Palette {
        Palette::from(PRESETS[0].light)
    }

    pub fn preset(preset: &Preset, dark: bool) -> Palette {
        Palette::from(if dark { preset.dark } else { preset.light })
    }
}

/// The preset both palettes match, if any.
pub fn current_preset(settings: &Settings) -> Option<&'static Preset> {
    PRESETS.iter().find(|p| {
        settings.dark == Palette::preset(p, true) && settings.light == Palette::preset(p, false)
    })
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Settings {
    #[serde(default)]
    pub theme: ThemeMode,
    #[serde(default = "Palette::default_dark")]
    pub dark: Palette,
    #[serde(default = "Palette::default_light")]
    pub light: Palette,
    #[serde(default)]
    pub density: Density,
    /// An installed font family for the interface; empty uses the built-in Inter, `system` the
    /// platform's own.
    #[serde(default)]
    pub ui_font: String,
    /// An installed font family for code; empty uses the built-in Cascadia Code, `system` the
    /// platform's monospaced font.
    #[serde(default)]
    pub code_font: String,
    #[serde(default = "one")]
    pub text_scale: f32,
    #[serde(default = "yes")]
    pub clock_24h: bool,
    #[serde(default)]
    pub default_provider: Option<ProviderId>,
    #[serde(default)]
    pub default_access: AccessMode,
    #[serde(default)]
    pub worktrees_by_default: bool,
    #[serde(default = "yes")]
    pub notify_on_finish: bool,
    #[serde(default = "default_worktree_root")]
    pub worktree_root: String,
    #[serde(default)]
    pub provider_paths: Vec<(ProviderId, String)>,
    #[serde(default)]
    pub hidden_providers: Vec<ProviderId>,
    /// Model ids a CLI accepts beyond the ones Oxlet lists, per provider.
    #[serde(default)]
    pub custom_models: Vec<(ProviderId, String)>,
}

fn default_worktree_root() -> String {
    "~/.oxlet/worktrees".into()
}

impl Default for Settings {
    fn default() -> Self {
        serde_json::from_str("{}").expect("every settings field has a default")
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Workspace {
    #[serde(default)]
    pub projects: Vec<Project>,
    /// Shared, so a snapshot for saving costs a pointer per thread; writing to a thread through
    /// [`Workspace::thread_mut`] copies it first while a snapshot still holds it.
    #[serde(default)]
    pub threads: Vec<Arc<Thread>>,
    #[serde(default)]
    pub automations: Vec<Automation>,
    #[serde(default)]
    pub settings: Settings,
    #[serde(default)]
    pub next_id: u64,
}

impl Workspace {
    pub fn new_id(&mut self, prefix: &str) -> Id {
        self.next_id += 1;
        format!("{prefix}{}", self.next_id)
    }

    pub fn project(&self, id: &str) -> Option<&Project> {
        self.projects.iter().find(|p| p.id == id)
    }

    pub fn thread(&self, id: &str) -> Option<&Thread> {
        self.threads.iter().find(|t| t.id == id).map(|t| &**t)
    }

    pub fn thread_mut(&mut self, id: &str) -> Option<&mut Thread> {
        self.threads
            .iter_mut()
            .find(|t| t.id == id)
            .map(Arc::make_mut)
    }

    pub fn add_project(&mut self, path: &str) -> Id {
        self.add_project_on(Target::Local, path)
    }

    pub fn add_project_on(&mut self, target: Target, path: &str) -> Id {
        let path = match path.trim_end_matches('/') {
            "" if path.starts_with('/') => "/".to_owned(),
            trimmed => trimmed.to_owned(),
        };
        if let Some(p) = self
            .projects
            .iter()
            .find(|p| p.path == path && p.target == target)
        {
            return p.id.clone();
        }
        let name = path
            .rsplit('/')
            .next()
            .filter(|n| !n.is_empty())
            .unwrap_or("project")
            .to_owned();
        let id = self.new_id("p");
        self.projects.push(Project {
            id: id.clone(),
            name,
            path,
            target,
            default_provider: None,
        });
        id
    }

    pub fn create_thread(
        &mut self,
        project: &str,
        provider: ProviderId,
        model: &str,
        now: u64,
    ) -> Id {
        let id = self.new_id("t");
        let access = self.settings.default_access;
        self.threads.push(Arc::new(Thread {
            id: id.clone(),
            project: project.to_owned(),
            title: "New thread".into(),
            provider,
            model: model.to_owned(),
            effort: Effort::default(),
            access,
            mode: ChatMode::default(),
            status: ThreadStatus::Idle,
            branch: None,
            worktree: None,
            pull_request: None,
            goal: None,
            notes: String::new(),
            session: None,
            parent: None,
            pinned: false,
            archived: false,
            unread: false,
            created: now,
            updated: now,
            messages: Vec::new(),
        }));
        id
    }

    /// A copy of thread `id` that continues in the same folder. Its pull request stays the
    /// source's.
    pub fn fork_thread(&mut self, id: &str, now: u64) -> Option<Id> {
        let source = self.thread(id)?.clone();
        let new = self.new_id("t");
        self.threads.push(Arc::new(Thread {
            id: new.clone(),
            title: format!("{} (fork)", source.title),
            parent: Some(source.id.clone()),
            session: None,
            status: ThreadStatus::Idle,
            pull_request: None,
            archived: false,
            pinned: false,
            unread: false,
            created: now,
            updated: now,
            ..source
        }));
        Some(new)
    }

    pub fn push_message(
        &mut self,
        thread: &str,
        role: Role,
        parts: Vec<Part>,
        now: u64,
    ) -> Option<Id> {
        let id = self.new_id("m");
        let t = self.thread_mut(thread)?;
        if role == Role::User && t.messages.iter().all(|m| m.role != Role::User) {
            t.title = title_from(&parts);
        }
        t.messages.push(Message {
            id: id.clone(),
            role,
            parts,
            at: now,
            worked_ms: None,
            pinned: false,
            agent: None,
            usage: None,
        });
        t.updated = now;
        Some(id)
    }

    /// Threads not archived, most recently updated first, pinned ones ahead of the rest.
    pub fn recent(&self) -> Vec<&Thread> {
        let mut out: Vec<&Thread> = self
            .threads
            .iter()
            .map(|t| &**t)
            .filter(|t| !t.archived)
            .collect();
        out.sort_by(|a, b| b.pinned.cmp(&a.pinned).then(b.updated.cmp(&a.updated)));
        out
    }
}

fn title_from(parts: &[Part]) -> String {
    parts
        .iter()
        .find_map(|p| match p {
            Part::Text { text } => Some(title_from_text(text)),
            _ => None,
        })
        .unwrap_or_else(|| "New thread".into())
}

/// A thread title from the first non-empty line of a message, cut at 60 characters.
pub fn title_from_text(text: &str) -> String {
    let line = text
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("New thread");
    let mut title: String = line.chars().take(60).collect();
    if line.chars().count() > 60 {
        title.push('…');
    }
    title
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_user_message_titles_the_thread() {
        let mut w = Workspace::default();
        let p = w.add_project("/src/app/");
        let t = w.create_thread(&p, ProviderId::Claude, "claude-sonnet-5-5", 1);
        w.push_message(
            &t,
            Role::User,
            vec![Part::Text {
                text: "\n  Fix the login flow\nmore".into(),
            }],
            2,
        );
        w.push_message(
            &t,
            Role::User,
            vec![Part::Text {
                text: "second".into(),
            }],
            3,
        );
        let thread = w.thread(&t).unwrap();
        assert_eq!(thread.title, "Fix the login flow");
        assert_eq!(thread.updated, 3);
        assert_eq!(w.project(&p).unwrap().name, "app");
    }

    #[test]
    fn adding_the_same_path_twice_keeps_one_project() {
        let mut w = Workspace::default();
        assert_eq!(w.add_project("/a/b"), w.add_project("/a/b/"));
        assert_eq!(w.projects.len(), 1);
        let host = Target::Ssh {
            host: "devbox".into(),
        };
        assert_ne!(
            w.add_project_on(host.clone(), "/a/b"),
            w.add_project("/a/b")
        );
        assert_eq!(w.projects[1].target, host);
        let root = w.add_project("/");
        assert_eq!(w.project(&root).unwrap().path, "/");
    }

    #[test]
    fn projects_saved_before_targets_load_as_local() {
        let p: Project =
            serde_json::from_str(r#"{"id":"p1","name":"app","path":"/src/app"}"#).unwrap();
        assert_eq!(p.target, Target::Local);
    }

    #[test]
    fn recent_puts_pinned_first_and_hides_archived() {
        let mut w = Workspace::default();
        let p = w.add_project("/x");
        let a = w.create_thread(&p, ProviderId::Codex, "m", 1);
        let b = w.create_thread(&p, ProviderId::Codex, "m", 2);
        let c = w.create_thread(&p, ProviderId::Codex, "m", 3);
        w.thread_mut(&a).unwrap().pinned = true;
        w.thread_mut(&c).unwrap().archived = true;
        let ids: Vec<_> = w.recent().iter().map(|t| t.id.clone()).collect();
        assert_eq!(ids, vec![a, b]);
    }

    #[test]
    fn fork_copies_history_and_links_parent() {
        let mut w = Workspace::default();
        let p = w.add_project("/x");
        let t = w.create_thread(&p, ProviderId::Claude, "m", 1);
        w.push_message(&t, Role::User, vec![Part::Text { text: "hi".into() }], 2);
        let source = w.thread_mut(&t).unwrap();
        (source.pull_request, source.archived) = (Some(7), true);
        let f = w.fork_thread(&t, 5).unwrap();
        let fork = w.thread(&f).unwrap();
        assert_eq!(fork.parent.as_deref(), Some(t.as_str()));
        assert_eq!(fork.messages.len(), 1);
        assert!(fork.session.is_none());
        assert_eq!((fork.pull_request, fork.archived), (None, false));
    }

    #[test]
    fn usage_adds_up_and_reads_compactly() {
        let mut u = Usage {
            input: 12_000,
            output: 400,
            cost_usd: Some(0.05),
        };
        u.add(&Usage {
            input: 1_000,
            output: 100,
            cost_usd: None,
        });
        assert_eq!(u.tokens(), 13_500);
        assert_eq!(u.label(), "13.5k tokens · $0.05");
        assert_eq!(
            Usage {
                input: 10,
                output: 2,
                cost_usd: Some(0.001)
            }
            .label(),
            "12 tokens · <$0.01"
        );
        assert_eq!(
            Usage {
                input: 2_000_000,
                output: 0,
                cost_usd: None
            }
            .label(),
            "2.0M tokens"
        );
    }

    #[test]
    fn settings_round_trip_with_defaults() {
        let s = Settings::default();
        assert_eq!(s.dark.accent, "#16788C");
        assert_eq!(current_preset(&s).map(|p| p.name), Some("Patina"));
        let json = serde_json::to_string(&s).unwrap();
        assert_eq!(serde_json::from_str::<Settings>(&json).unwrap(), s);
    }
}
