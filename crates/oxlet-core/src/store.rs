use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};

use crate::model::{ThreadStatus, Workspace};

/// Oxlet's data folder: `$OXLET_HOME`, else `~/.oxlet`.
pub fn home() -> PathBuf {
    if let Some(dir) = std::env::var_os("OXLET_HOME").filter(|d| !d.is_empty()) {
        return PathBuf::from(dir);
    }
    user_home()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".oxlet")
}

fn user_home() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .filter(|h| !h.is_empty())
        .map(PathBuf::from)
}

/// Expands a leading `~/` (or `~\` on Windows) to the user's home folder.
pub fn expand_home(path: &str) -> String {
    let rest = path.strip_prefix("~/").or_else(|| path.strip_prefix("~\\"));
    match (rest, user_home()) {
        (Some(rest), Some(home)) => home.join(rest).to_string_lossy().into_owned(),
        _ => path.to_owned(),
    }
}

pub fn state_file(dir: &Path) -> PathBuf {
    dir.join("workspace.json")
}

/// Reads the workspace. A missing file is an empty workspace. One that does not parse is moved
/// aside, and its new path is returned: the next save would otherwise write over it.
/// No turn outlives the app, so a thread saved mid-turn loads idle.
pub fn load(dir: &Path) -> (Workspace, Option<PathBuf>) {
    let file = state_file(dir);
    let Ok(text) = std::fs::read_to_string(&file) else {
        return (Workspace::default(), None);
    };
    match serde_json::from_str::<Workspace>(&text) {
        Ok(mut w) => {
            for t in w
                .threads
                .iter_mut()
                .filter(|t| t.status == ThreadStatus::Working)
            {
                Arc::make_mut(t).status = ThreadStatus::Idle;
            }
            (w, None)
        }
        Err(_) => {
            let aside = dir.join("workspace.corrupt.json");
            let moved = std::fs::rename(&file, &aside).is_ok();
            (Workspace::default(), moved.then_some(aside))
        }
    }
}

/// Writes the workspace through a temporary file, so a crash never leaves half a file.
pub fn save(dir: &Path, workspace: &Workspace) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let tmp = dir.join("workspace.json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(workspace)?)?;
    std::fs::rename(tmp, state_file(dir))
}

/// Writes workspaces one at a time on a thread of its own, so two writes never share the
/// temporary file and the newest workspace is always the last one written. One handed over
/// while another is being written replaces any still waiting: a write can take longer than the
/// gap between saves.
pub struct Saver {
    shared: Arc<(Mutex<Slot>, Condvar)>,
}

#[derive(Default)]
struct Slot {
    next: Option<Workspace>,
    /// How many workspaces were handed over, and which of them was written last.
    handed: u64,
    written: u64,
    closed: bool,
}

impl Saver {
    pub fn new(mut write: impl FnMut(&Workspace) + Send + 'static) -> Saver {
        let shared = Arc::new((Mutex::new(Slot::default()), Condvar::new()));
        let worker = shared.clone();
        std::thread::spawn(move || {
            let (lock, changed) = &*worker;
            loop {
                let (workspace, n) = {
                    let mut slot = changed
                        .wait_while(lock.lock().unwrap(), |s| s.next.is_none() && !s.closed)
                        .unwrap();
                    match slot.next.take() {
                        Some(w) => (w, slot.handed),
                        None => return,
                    }
                };
                write(&workspace);
                lock.lock().unwrap().written = n;
                changed.notify_all();
            }
        });
        Saver { shared }
    }

    pub fn save(&self, workspace: Workspace) {
        let (lock, changed) = &*self.shared;
        let mut slot = lock.lock().unwrap();
        slot.next = Some(workspace);
        slot.handed += 1;
        changed.notify_all();
    }

    /// Waits until the last workspace handed over has been written.
    pub fn flush(&self) {
        let (lock, changed) = &*self.shared;
        drop(
            changed
                .wait_while(lock.lock().unwrap(), |s| s.written < s.handed)
                .unwrap(),
        );
    }
}

/// The thread finishes the write it has, and one still waiting, then ends.
impl Drop for Saver {
    fn drop(&mut self) {
        let (lock, changed) = &*self.shared;
        lock.lock().unwrap().closed = true;
        changed.notify_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::ProviderId;

    #[test]
    fn save_then_load_and_quarantine_corruption() {
        let dir = std::env::temp_dir().join(format!("oxlet-store-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(load(&dir), (Workspace::default(), None));
        let mut w = Workspace::default();
        let p = w.add_project("/tmp/x");
        w.create_thread(&p, ProviderId::Codex, "", 7);
        save(&dir, &w).unwrap();
        assert_eq!(load(&dir), (w, None));
        std::fs::write(state_file(&dir), "{oops").unwrap();
        let aside = dir.join("workspace.corrupt.json");
        assert_eq!(load(&dir), (Workspace::default(), Some(aside.clone())));
        assert_eq!(std::fs::read_to_string(aside).unwrap(), "{oops");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn saves_never_overlap_and_the_newest_is_written_last() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::mpsc;
        let (started_tx, started) = mpsc::channel();
        let (release, gate) = mpsc::channel::<()>();
        let gate = Mutex::new(gate);
        let (writing, most) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));
        let written = Arc::new(Mutex::new(Vec::new()));
        let (busy, peak, log) = (writing.clone(), most.clone(), written.clone());
        let saver = Saver::new(move |w: &Workspace| {
            peak.fetch_max(busy.fetch_add(1, Ordering::SeqCst) + 1, Ordering::SeqCst);
            started_tx.send(w.next_id).unwrap();
            gate.lock().unwrap().recv().unwrap();
            log.lock().unwrap().push(w.next_id);
            busy.fetch_sub(1, Ordering::SeqCst);
        });
        let numbered = |n| Workspace {
            next_id: n,
            ..Workspace::default()
        };
        saver.save(numbered(1));
        assert_eq!(started.recv().unwrap(), 1);
        saver.save(numbered(2));
        saver.save(numbered(3));
        release.send(()).unwrap();
        assert_eq!(started.recv().unwrap(), 3);
        release.send(()).unwrap();
        saver.flush();
        assert_eq!(*written.lock().unwrap(), [1, 3]);
        assert_eq!(most.load(Ordering::SeqCst), 1, "two writes at once");
    }

    #[test]
    fn a_thread_saved_mid_turn_loads_idle() {
        let dir = std::env::temp_dir().join(format!("oxlet-store-working-{}", std::process::id()));
        let mut w = Workspace::default();
        let p = w.add_project("/tmp/x");
        let t = w.create_thread(&p, ProviderId::Claude, "", 1);
        w.thread_mut(&t).unwrap().status = ThreadStatus::Working;
        save(&dir, &w).unwrap();
        assert_eq!(load(&dir).0.thread(&t).unwrap().status, ThreadStatus::Idle);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
