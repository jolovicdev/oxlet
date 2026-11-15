use std::path::{Path, PathBuf};

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

/// Reads the workspace. A missing file is an empty workspace; an unreadable one is kept aside.
/// No turn outlives the app, so a thread saved mid-turn loads idle.
pub fn load(dir: &Path) -> Workspace {
    let file = state_file(dir);
    let Ok(text) = std::fs::read_to_string(&file) else {
        return Workspace::default();
    };
    match serde_json::from_str::<Workspace>(&text) {
        Ok(mut w) => {
            for t in w
                .threads
                .iter_mut()
                .filter(|t| t.status == ThreadStatus::Working)
            {
                t.status = ThreadStatus::Idle;
            }
            w
        }
        Err(_) => {
            let _ = std::fs::rename(&file, dir.join("workspace.corrupt.json"));
            Workspace::default()
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::ProviderId;

    #[test]
    fn save_then_load_and_quarantine_corruption() {
        let dir = std::env::temp_dir().join(format!("oxlet-store-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(load(&dir), Workspace::default());
        let mut w = Workspace::default();
        let p = w.add_project("/tmp/x");
        w.create_thread(&p, ProviderId::Codex, "", 7);
        save(&dir, &w).unwrap();
        assert_eq!(load(&dir), w);
        std::fs::write(state_file(&dir), "{oops").unwrap();
        assert_eq!(load(&dir), Workspace::default());
        assert!(dir.join("workspace.corrupt.json").exists());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_thread_saved_mid_turn_loads_idle() {
        let dir = std::env::temp_dir().join(format!("oxlet-store-working-{}", std::process::id()));
        let mut w = Workspace::default();
        let p = w.add_project("/tmp/x");
        let t = w.create_thread(&p, ProviderId::Claude, "", 1);
        w.thread_mut(&t).unwrap().status = ThreadStatus::Working;
        save(&dir, &w).unwrap();
        assert_eq!(load(&dir).thread(&t).unwrap().status, ThreadStatus::Idle);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
