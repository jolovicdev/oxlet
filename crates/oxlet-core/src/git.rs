use serde::Deserialize;

use crate::model::FileChange;
use crate::process::output;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Status {
    pub branch: Option<String>,
    pub upstream: Option<String>,
    pub ahead: u32,
    pub behind: u32,
    pub entries: Vec<Entry>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub path: String,
    pub code: String,
}

impl Entry {
    pub fn untracked(&self) -> bool {
        self.code == "??"
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Commit {
    pub hash: String,
    pub subject: String,
    pub author: String,
    pub when: String,
}

fn git(cwd: &str, args: &[&str]) -> Result<String, String> {
    output("git", args, cwd)
}

pub fn is_repo(cwd: &str) -> bool {
    git(cwd, &["rev-parse", "--is-inside-work-tree"]).is_ok_and(|s| s.trim() == "true")
}

pub fn status(cwd: &str) -> Result<Status, String> {
    Ok(parse_status(&git(
        cwd,
        &["status", "--porcelain=v1", "-b", "--untracked-files=all"],
    )?))
}

pub fn parse_status(text: &str) -> Status {
    let mut s = Status::default();
    for line in text.lines() {
        if let Some(head) = line.strip_prefix("## ") {
            let (names, counts) = head.split_once(" [").unwrap_or((head, ""));
            let (branch, upstream) = names
                .split_once("...")
                .map_or((names, None), |(b, u)| (b, Some(u)));
            let branch = branch.strip_prefix("No commits yet on ").unwrap_or(branch);
            s.branch = (branch != "HEAD (no branch)").then(|| branch.to_owned());
            s.upstream = upstream.map(str::to_owned);
            for part in counts.trim_end_matches(']').split(", ") {
                if let Some(n) = part.strip_prefix("ahead ") {
                    s.ahead = n.parse().unwrap_or(0);
                } else if let Some(n) = part.strip_prefix("behind ") {
                    s.behind = n.parse().unwrap_or(0);
                }
            }
        } else if line.len() > 3 {
            let path = &line[3..];
            let path = path.split_once(" -> ").map_or(path, |(_, to)| to);
            s.entries.push(Entry {
                path: path.trim_matches('"').to_owned(),
                code: line[..2].to_owned(),
            });
        }
    }
    s
}

/// Tracked changes against HEAD, as a unified patch.
pub fn diff(cwd: &str) -> Result<String, String> {
    match git(cwd, &["diff", "HEAD", "--no-color", "--no-ext-diff"]) {
        Ok(d) => Ok(d),
        Err(_) => git(cwd, &["diff", "--cached", "--no-color", "--no-ext-diff"]),
    }
}

pub fn numstat(cwd: &str) -> Result<Vec<FileChange>, String> {
    let text = git(cwd, &["diff", "HEAD", "--numstat"]).unwrap_or_default();
    Ok(parse_numstat(&text))
}

pub fn parse_numstat(text: &str) -> Vec<FileChange> {
    text.lines()
        .filter_map(|l| {
            let mut it = l.splitn(3, '\t');
            let added = it.next()?.parse().unwrap_or(0);
            let removed = it.next()?.parse().unwrap_or(0);
            Some(FileChange {
                path: it.next()?.to_owned(),
                added,
                removed,
            })
        })
        .collect()
}

pub fn branches(cwd: &str) -> Result<Vec<String>, String> {
    Ok(git(cwd, &["branch", "--format=%(refname:short)"])?
        .lines()
        .map(str::to_owned)
        .collect())
}

pub fn log(cwd: &str, limit: usize) -> Result<Vec<Commit>, String> {
    let n = format!("-{limit}");
    let text = git(cwd, &["log", &n, "--pretty=format:%h%x1f%s%x1f%an%x1f%cr"])?;
    Ok(text
        .lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.split('\x1f').collect();
            (f.len() == 4).then(|| Commit {
                hash: f[0].into(),
                subject: f[1].into(),
                author: f[2].into(),
                when: f[3].into(),
            })
        })
        .collect())
}

pub fn checkout(cwd: &str, branch: &str, create: bool) -> Result<(), String> {
    let args: &[&str] = if create {
        &["checkout", "-b", branch]
    } else {
        &["checkout", branch]
    };
    git(cwd, args).map(drop)
}

pub fn commit_all(cwd: &str, message: &str) -> Result<String, String> {
    git(cwd, &["add", "-A"])?;
    git(cwd, &["commit", "-m", message])?;
    Ok(git(cwd, &["rev-parse", "--short", "HEAD"])?
        .trim()
        .to_owned())
}

pub fn push(cwd: &str) -> Result<(), String> {
    git(cwd, &["push", "-u", "origin", "HEAD"]).map(drop)
}

pub fn discard(cwd: &str, path: &str, untracked: bool) -> Result<(), String> {
    if untracked {
        git(cwd, &["clean", "-f", "--", path]).map(drop)
    } else {
        git(cwd, &["checkout", "HEAD", "--", path]).map(drop)
    }
}

pub fn worktree_add(repo: &str, path: &str, branch: &str) -> Result<(), String> {
    git(repo, &["worktree", "add", "-b", branch, path]).map(drop)
}

pub fn worktree_remove(repo: &str, path: &str) -> Result<(), String> {
    git(repo, &["worktree", "remove", "--force", path]).map(drop)
}

pub fn worktrees(repo: &str) -> Result<Vec<(String, Option<String>)>, String> {
    let text = git(repo, &["worktree", "list", "--porcelain"])?;
    let mut out = Vec::new();
    for block in text.split("\n\n") {
        let path = block.lines().find_map(|l| l.strip_prefix("worktree "));
        let branch = block
            .lines()
            .find_map(|l| l.strip_prefix("branch refs/heads/"));
        if let Some(p) = path {
            out.push((p.to_owned(), branch.map(str::to_owned)));
        }
    }
    Ok(out)
}

/// Files in the project: tracked and untracked, without ignored ones.
pub fn files(cwd: &str) -> Result<Vec<String>, String> {
    Ok(git(cwd, &["ls-files", "-co", "--exclude-standard"])?
        .lines()
        .map(str::to_owned)
        .collect())
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct PullRequest {
    pub number: u32,
    pub title: String,
    #[serde(rename = "headRefName")]
    pub branch: String,
    pub state: String,
    #[serde(default)]
    pub author: Option<Author>,
    #[serde(rename = "isDraft", default)]
    pub draft: bool,
    #[serde(default)]
    pub url: String,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct Author {
    pub login: String,
}

/// Pull requests through the GitHub CLI, which must be installed and signed in.
pub fn pull_requests(cwd: &str) -> Result<Vec<PullRequest>, String> {
    let text = output(
        "gh",
        &[
            "pr",
            "list",
            "--state",
            "all",
            "--limit",
            "50",
            "--json",
            "number,title,headRefName,state,author,isDraft,url",
        ],
        cwd,
    )?;
    serde_json::from_str(&text).map_err(|e| e.to_string())
}

pub fn create_pull_request(cwd: &str) -> Result<String, String> {
    Ok(output("gh", &["pr", "create", "--fill"], cwd)?
        .trim()
        .to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_reads_branch_counts_renames_and_untracked() {
        let s = parse_status(
            "## main...origin/main [ahead 2, behind 1]\n M src/a.rs\nR  old.rs -> new.rs\n?? notes.md\n",
        );
        assert_eq!(s.branch.as_deref(), Some("main"));
        assert_eq!(s.upstream.as_deref(), Some("origin/main"));
        assert_eq!((s.ahead, s.behind), (2, 1));
        let paths: Vec<_> = s.entries.iter().map(|e| e.path.as_str()).collect();
        assert_eq!(paths, ["src/a.rs", "new.rs", "notes.md"]);
        assert!(s.entries[2].untracked());
    }

    #[test]
    fn status_on_an_unborn_branch() {
        let s = parse_status("## No commits yet on main\n");
        assert_eq!(s.branch.as_deref(), Some("main"));
        assert!(s.upstream.is_none());
    }

    #[test]
    fn numstat_counts_binary_as_zero() {
        let c = parse_numstat("3\t1\tsrc/a.rs\n-\t-\timg.png\n");
        assert_eq!(
            c[0],
            FileChange {
                path: "src/a.rs".into(),
                added: 3,
                removed: 1
            }
        );
        assert_eq!((c[1].added, c[1].removed), (0, 0));
    }

    #[test]
    fn real_repository_round_trip() {
        let dir = std::env::temp_dir().join(format!("oxlet-git-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let cwd = dir.to_str().unwrap();
        for args in [
            &["init", "-q", "-b", "main"][..],
            &["config", "user.email", "g@example.com"],
            &["config", "user.name", "G"],
        ] {
            git(cwd, args).unwrap();
        }
        std::fs::write(dir.join("a.txt"), "one\n").unwrap();
        assert!(is_repo(cwd));
        commit_all(cwd, "first").unwrap();
        std::fs::write(dir.join("a.txt"), "one\ntwo\n").unwrap();
        assert_eq!(
            numstat(cwd).unwrap(),
            vec![FileChange {
                path: "a.txt".into(),
                added: 1,
                removed: 0
            }]
        );
        assert!(diff(cwd).unwrap().contains("+two"));
        assert_eq!(log(cwd, 5).unwrap()[0].subject, "first");
        discard(cwd, "a.txt", false).unwrap();
        assert!(status(cwd).unwrap().entries.is_empty());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
