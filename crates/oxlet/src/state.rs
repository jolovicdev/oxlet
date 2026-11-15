use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};

use oxlet_core::git::{self, Commit, PullRequest};
use oxlet_core::model::{ChatMode, Effort, FileChange, Id, Workspace};
use oxlet_core::process::Running;
use oxlet_core::provider::{AgentEvent, ProviderId};
use oxlet_core::target::{Place, Target};
use rune::{TextEdit, Waker};
use rune_kit::diff::DiffState;
use rune_kit::shell::PaneState;
use rune_kit::{FileNode, FileTree, TreeModel};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Screen {
    Thread,
    Kanban,
    PullRequests,
    Automations,
    Settings(Section),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Section {
    General,
    Appearance,
    Providers,
    Worktrees,
    Keybindings,
    Archived,
}

impl Section {
    pub fn title(self) -> &'static str {
        match self {
            Section::General => "General",
            Section::Appearance => "Appearance",
            Section::Providers => "Agent providers",
            Section::Worktrees => "Managed worktrees",
            Section::Keybindings => "Keybindings",
            Section::Archived => "Archived threads",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DockTab {
    Changes,
    Files,
    Terminal,
    Git,
    Notes,
}

impl DockTab {
    pub const ALL: [DockTab; 5] = [
        DockTab::Changes,
        DockTab::Files,
        DockTab::Terminal,
        DockTab::Git,
        DockTab::Notes,
    ];

    pub fn title(self) -> &'static str {
        match self {
            DockTab::Changes => "Changes",
            DockTab::Files => "Files",
            DockTab::Terminal => "Terminal",
            DockTab::Git => "Git",
            DockTab::Notes => "Notes",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Popup {
    Access,
    Model,
    Effort,
    DefaultProvider,
    DefaultAccess,
    Project,
    Workspace,
    NewAutomationProject,
    NewAutomationSchedule,
    OpenIn,
    /// The other ways to add a project, from the menu keyed by this name.
    AddProject(&'static str),
}

/// What the add-project field takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AddMode {
    Path,
    Ssh,
}

/// A destructive action waiting for the user to confirm it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Pending {
    /// A changed file in the dock's folder, by path.
    Discard(String),
    /// The worktree a thread works in.
    RemoveWorktree(Id),
    DeleteThread(Id),
    DeleteAutomation(Id),
    RestoreAppearance,
    RemoveProject(Id),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Placement {
    Local,
    Worktree,
}

#[derive(Default)]
pub struct Repo {
    pub cwd: Place,
    /// The last read started; results of any other are stale. It carries over to a new `cwd`.
    pub generation: u64,
    /// A read is running. One runs at a time: a request meanwhile is left to the next one.
    pub reading: bool,
    /// Nothing changed since the last read started, so the screen needs no other.
    pub fresh: bool,
    /// The last read started covers the dock, not only the branch.
    pub full: bool,
    /// A changed file to scroll the diff to once the read running or wanted lands.
    pub reveal: Option<String>,
    pub loading: bool,
    pub error: Option<String>,
    pub status: git::Status,
    pub changes: Vec<FileChange>,
    pub diff: DiffState,
    pub log: Vec<Commit>,
    pub branches: Vec<String>,
    pub files: TreeModel<FileNode<String>>,
    pub file_count: usize,
}

pub struct Terminal {
    pub lines: Vec<(bool, String)>,
    pub input: TextEdit,
    pub running: Option<Running>,
    /// Which command `running` is: lines from an earlier one are dropped.
    pub run: u64,
    pub history: Vec<String>,
    /// The history entry Up and Down have put in the input.
    pub history_at: Option<usize>,
}

pub struct Turn {
    pub running: Running,
    pub started: u64,
    pub message: Id,
    pub edited: BTreeSet<String>,
    /// The text part deltas are being written into, until its complete block arrives.
    pub live: Option<usize>,
    /// The thread's session epoch when the turn started; its session id is stale after a reset.
    pub epoch: u64,
    /// The turn summarises the thread for a fresh session to continue from.
    pub compact: bool,
}

/// What a worker reads from a repository. Diff models are built on the UI thread: they hold `Rc`s.
pub struct RepoSnapshot {
    pub status: git::Status,
    pub changes: Vec<FileChange>,
    pub patch: String,
    pub untracked: Vec<(String, String)>,
    pub log: Vec<Commit>,
    pub branches: Vec<String>,
}

pub enum Event {
    Agent {
        thread: Id,
        event: AgentEvent,
    },
    TurnEdits {
        thread: Id,
        message: Id,
        files: Vec<FileChange>,
    },
    Repo {
        cwd: Place,
        generation: u64,
        result: Result<RepoSnapshot, String>,
    },
    /// The project's files: how many, and as a tree.
    Files {
        cwd: Place,
        generation: u64,
        result: Result<(usize, TreeModel<FileNode<String>>), String>,
    },
    Term(u64, oxlet_core::process::Line),
    Prs(Result<Vec<PullRequest>, String>),
    Providers(Target, Vec<(ProviderId, Option<PathBuf>)>),
    Distros(Vec<String>),
    SshHosts(Vec<String>),
    /// A WSL or SSH folder to add as a project: its absolute path, or why it can't be one.
    FolderChecked {
        target: Target,
        result: Result<String, String>,
    },
    /// The Windows folder dialog's answer, when Oxlet runs inside WSL.
    PickedFolder(Result<Option<String>, String>),
    Editors(Vec<oxlet_core::open::Editor>),
    WorktreeReady {
        thread: Id,
        result: Result<(String, String), String>,
    },
    Notice(String),
    GitDone {
        cwd: Place,
        result: Result<String, String>,
        committed: bool,
    },
    WorktreeRemoved {
        path: String,
        result: Result<(), String>,
    },
}

/// Hands events from worker threads to the UI thread and wakes the window.
#[derive(Clone)]
pub struct Bridge {
    tx: Sender<Event>,
    waker: Arc<Mutex<Option<Waker>>>,
}

impl Bridge {
    pub fn send(&self, e: Event) {
        if self.tx.send(e).is_ok()
            && let Some(w) = self.waker.lock().unwrap().as_ref()
        {
            w();
        }
    }

    pub fn set_waker(&self, w: Waker) {
        *self.waker.lock().unwrap() = Some(w);
    }

    pub fn spawn(&self, f: impl FnOnce(&Bridge) + Send + 'static) {
        let b = self.clone();
        std::thread::spawn(move || f(&b));
    }
}

pub struct Toast {
    pub text: String,
    pub until: f64,
}

pub struct App {
    pub ws: Workspace,
    pub dir: PathBuf,
    pub offset: i64,
    pub demo: bool,

    pub screen: Screen,
    pub selected: Option<Id>,
    pub draft_project: Option<Id>,
    pub placement: Placement,
    /// Choices made on the new-thread screen, for the thread its first message creates.
    pub draft_model: Option<(ProviderId, String)>,
    pub draft_effort: Effort,
    pub draft_mode: ChatMode,
    pub sidebar: PaneState,
    pub dock: PaneState,
    pub dock_tab: DockTab,
    pub popup: Option<Popup>,
    pub thread_menu: Option<(Id, rune::PointerPress)>,
    pub project_menu: Option<(Id, rune::PointerPress)>,
    pub confirm: Option<Pending>,
    pub collapsed: Vec<&'static str>,
    pub expanded: HashSet<String>,

    pub composer: TextEdit,
    pub slash_active: usize,
    pub slash_dismissed: bool,
    /// The user message the composer is rewriting: `(thread, message)`.
    pub editing: Option<(Id, Id)>,
    pub palette: Option<crate::palette::Palette>,
    pub search: TextEdit,
    pub searching: bool,
    pub renaming: Option<Id>,
    pub rename: TextEdit,
    pub add_project: TextEdit,
    pub adding_project: Option<AddMode>,
    pub commit_message: TextEdit,
    pub notes: TextEdit,
    pub notes_for: Option<Id>,
    pub goal: TextEdit,

    pub auto_name: TextEdit,
    pub auto_prompt: TextEdit,
    pub auto_project: Option<Id>,
    pub auto_schedule: usize,

    pub hex_fields: HashMap<&'static str, TextEdit>,
    pub provider_path: HashMap<ProviderId, TextEdit>,
    pub model_fields: HashMap<ProviderId, TextEdit>,

    /// The agent CLIs found on each target; an empty list while detection there runs.
    pub providers: HashMap<Target, Vec<(ProviderId, Option<PathBuf>)>>,
    pub editors: Vec<oxlet_core::open::Editor>,
    pub distros: Vec<String>,
    pub ssh_hosts: Vec<String>,
    pub window_focused: bool,
    pub turns: HashMap<Id, Turn>,
    /// Bumped per thread whenever its session is dropped.
    pub epochs: HashMap<Id, u64>,
    pub queued: HashMap<Id, VecDeque<String>>,
    pub waiting_worktree: HashSet<Id>,
    pub repo: Repo,
    pub terminal: Terminal,
    pub prs: Option<Result<Vec<PullRequest>, String>>,
    pub toast: Option<Toast>,

    pub bridge: Bridge,
    pub inbox: Receiver<Event>,
    pub saver: oxlet_core::store::Saver,
    pub dirty: bool,
    pub save_due: f64,
    pub next_automation_check: f64,
}

impl App {
    pub fn new(ws: Workspace, dir: PathBuf, offset: i64) -> App {
        let (tx, inbox) = channel();
        let bridge = Bridge {
            tx,
            waker: Arc::new(Mutex::new(None)),
        };
        let saver = {
            let (dir, bridge) = (dir.clone(), bridge.clone());
            oxlet_core::store::Saver::new(move |ws| {
                if let Err(e) = oxlet_core::store::save(&dir, ws) {
                    eprintln!("Could not save the workspace: {e}");
                    bridge.send(Event::Notice(format!("Could not save: {e}")));
                }
            })
        };
        let mut app = App {
            ws,
            dir,
            offset,
            demo: false,
            screen: Screen::Thread,
            selected: None,
            draft_project: None,
            placement: Placement::Local,
            draft_model: None,
            draft_effort: Effort::default(),
            draft_mode: ChatMode::default(),
            sidebar: PaneState::new(300.0).range(200.0, 480.0),
            dock: PaneState::new(520.0).range(360.0, 900.0).collapsed(),
            dock_tab: DockTab::Changes,
            popup: None,
            thread_menu: None,
            project_menu: None,
            confirm: None,
            collapsed: vec!["earlier"],
            expanded: HashSet::new(),
            composer: TextEdit::new(""),
            slash_active: 0,
            slash_dismissed: false,
            editing: None,
            palette: None,
            search: TextEdit::new(""),
            searching: false,
            renaming: None,
            rename: TextEdit::new(""),
            add_project: TextEdit::new(""),
            adding_project: None,
            commit_message: TextEdit::new(""),
            notes: TextEdit::new(""),
            notes_for: None,
            goal: TextEdit::new(""),
            auto_name: TextEdit::new(""),
            auto_prompt: TextEdit::new(""),
            auto_project: None,
            auto_schedule: 1,
            hex_fields: HashMap::new(),
            provider_path: HashMap::new(),
            model_fields: HashMap::new(),
            providers: HashMap::new(),
            editors: Vec::new(),
            distros: Vec::new(),
            ssh_hosts: Vec::new(),
            window_focused: true,
            turns: HashMap::new(),
            epochs: HashMap::new(),
            queued: HashMap::new(),
            waiting_worktree: HashSet::new(),
            repo: Repo::default(),
            terminal: Terminal {
                lines: Vec::new(),
                input: TextEdit::new(""),
                running: None,
                run: 0,
                history: Vec::new(),
                history_at: None,
            },
            prs: None,
            toast: None,
            bridge,
            inbox,
            saver,
            dirty: false,
            save_due: 0.0,
            next_automation_check: 0.0,
        };
        app.composer.focused = true;
        app.selected = app
            .ws
            .threads
            .iter()
            .filter(|t| !t.archived)
            .max_by_key(|t| t.updated)
            .map(|t| t.id.clone());
        app.draft_project = app.ws.projects.first().map(|p| p.id.clone());
        app.placement = if app.ws.settings.worktrees_by_default {
            Placement::Worktree
        } else {
            Placement::Local
        };
        app
    }

    pub fn thread(&self) -> Option<&oxlet_core::model::Thread> {
        self.selected.as_deref().and_then(|id| self.ws.thread(id))
    }

    /// The project a new thread would start in.
    pub fn current_project(&self) -> Option<&oxlet_core::model::Project> {
        match self.thread() {
            Some(t) => self.ws.project(&t.project),
            None => self
                .draft_project
                .as_deref()
                .and_then(|p| self.ws.project(p))
                .or(self.ws.projects.first()),
        }
    }

    /// The folder the dock's tools work in: the thread's worktree, else its project.
    pub fn place(&self) -> Option<Place> {
        let project = self.current_project()?;
        Some(match self.thread() {
            Some(t) => project.at(t.cwd(project)),
            None => project.at(&project.path),
        })
    }

    /// Where `id`'s CLI is on `target`; `None` until detection there has answered.
    pub fn binary(&self, target: &Target, id: ProviderId) -> Option<Option<PathBuf>> {
        self.providers
            .get(target)?
            .iter()
            .find(|(p, _)| *p == id)
            .map(|(_, b)| b.clone())
    }

    /// Project names for a picker, with the path added where two share a name.
    pub fn project_labels(&self) -> Vec<String> {
        let projects = &self.ws.projects;
        projects
            .iter()
            .map(|p| {
                if projects.iter().filter(|q| q.name == p.name).count() < 2 {
                    p.name.clone()
                } else if let Some(place) = p.target.label() {
                    format!("{} · {place}:{}", p.name, p.path)
                } else {
                    format!("{} · {}", p.name, p.path)
                }
            })
            .collect()
    }

    pub fn is_collapsed(&self, key: &str) -> bool {
        self.collapsed.contains(&key)
    }

    pub fn toggle_collapsed(&mut self, key: &'static str) {
        if let Some(i) = self.collapsed.iter().position(|k| *k == key) {
            self.collapsed.remove(i);
        } else {
            self.collapsed.push(key);
        }
    }
}

pub fn file_tree(paths: Vec<String>) -> TreeModel<FileNode<String>> {
    FileTree::new()
        .key("files")
        .build(paths.into_iter().map(|p| (p.clone(), p)))
}
