use std::path::{Path, PathBuf};
use std::process::Stdio;

use crate::process::command;
use crate::provider::locate;

/// A code editor Oxlet can open a folder or file in.
#[derive(Clone, Debug, PartialEq)]
pub struct Editor {
    pub name: &'static str,
    pub program: PathBuf,
}

/// Editors with a command-line launcher, in the order Oxlet offers them.
const EDITORS: [(&str, &str); 5] = [
    ("VS Code", "code"),
    ("Cursor", "cursor"),
    ("Zed", "zed"),
    ("Windsurf", "windsurf"),
    ("Sublime Text", "subl"),
];

/// The editors installed on `PATH` (or on `path_var`, for tests).
pub fn editors(path_var: Option<&str>) -> Vec<Editor> {
    EDITORS
        .iter()
        .filter_map(|(name, bin)| {
            locate(bin, None, path_var).map(|program| Editor { name, program })
        })
        .collect()
}

/// Opens `target` in `editor` without waiting for it.
pub fn in_editor(editor: &Editor, target: &str) -> std::io::Result<()> {
    command(&editor.program)
        .arg(target)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(drop)
}

/// What the file manager is called here, for menus.
pub fn file_manager() -> &'static str {
    if cfg!(target_os = "macos") {
        "Finder"
    } else if cfg!(windows) {
        "Explorer"
    } else {
        "Files"
    }
}

/// Shows `target` in the file manager, selected where the platform can select it.
pub fn reveal(target: &str) -> std::io::Result<()> {
    let path = Path::new(target);
    let mut cmd = if cfg!(target_os = "macos") {
        let mut c = command("open");
        c.arg("-R").arg(path);
        c
    } else if cfg!(windows) {
        let mut c = command("explorer");
        c.arg(format!("/select,{}", path.display()));
        c
    } else {
        let mut c = command("xdg-open");
        c.arg(if path.is_dir() {
            path
        } else {
            path.parent().unwrap_or(path)
        });
        c
    };
    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(drop)
}

/// Turns dropped paths into composer mentions: project-relative inside `root`, absolute outside.
pub fn mentions(paths: &[PathBuf], root: &str) -> String {
    let root = Path::new(root);
    paths
        .iter()
        .map(|p| {
            let shown = p
                .strip_prefix(root)
                .unwrap_or(p)
                .to_string_lossy()
                .replace('\\', "/");
            if shown.contains(' ') {
                format!("@\"{shown}\"")
            } else {
                format!("@{shown}")
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_only_installed_editors_in_order() {
        let dir = std::env::temp_dir().join(format!("oxlet-editors-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for bin in ["zed", "code"] {
            std::fs::write(dir.join(bin), "").unwrap();
        }
        let found: Vec<&str> = editors(Some(dir.to_str().unwrap()))
            .iter()
            .map(|e| e.name)
            .collect();
        assert_eq!(found, ["VS Code", "Zed"]);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn dropped_paths_become_relative_mentions() {
        let paths = [
            PathBuf::from("/repo/src/main.rs"),
            PathBuf::from("/repo/docs/My Notes.md"),
            PathBuf::from("/elsewhere/x"),
        ];
        assert_eq!(
            mentions(&paths, "/repo"),
            r#"@src/main.rs @"docs/My Notes.md" @/elsewhere/x"#
        );
    }
}
