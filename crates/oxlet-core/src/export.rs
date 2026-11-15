use std::path::{Path, PathBuf};

use crate::model::{Part, Role, Thread};

pub fn markdown(thread: &Thread) -> String {
    let mut out = format!("# {}\n\n", thread.title);
    if let Some(goal) = &thread.goal {
        out.push_str(&format!("> Goal: {goal}\n\n"));
    }
    for m in &thread.messages {
        let who = match (m.role, &m.agent) {
            (Role::User, _) => "You",
            (Role::Assistant, Some((provider, _))) => provider.info().name,
            (Role::Assistant, None) => thread.provider.info().name,
            (Role::System, _) => "Oxlet",
        };
        out.push_str(&format!("## {who}\n\n"));
        for part in &m.parts {
            // A paragraph straight after a bullet would read as part of it.
            if !matches!(part, Part::Tool { .. }) && out.ends_with("\n") && !out.ends_with("\n\n") {
                out.push('\n');
            }
            match part {
                Part::Text { text } => out.push_str(&format!("{text}\n\n")),
                Part::Tool { name, summary, .. } => {
                    out.push_str(&format!("- `{name}` {summary}\n"))
                }
                Part::Edits { files } => {
                    for f in files {
                        out.push_str(&format!(
                            "- edited `{}` +{} -{}\n",
                            f.path, f.added, f.removed
                        ));
                    }
                    out.push('\n');
                }
                Part::Error { message } => out.push_str(&format!("**Error:** {message}\n\n")),
            }
        }
    }
    out
}

fn slug(title: &str) -> String {
    let s: String = title
        .chars()
        .map(|c| {
            if c.is_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let s = s
        .split('-')
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    if s.is_empty() {
        "thread".into()
    } else {
        s.chars().take(60).collect()
    }
}

/// Writes `<slug>.md` and `<slug>.json` into `dir` and returns the Markdown file's path.
pub fn write(thread: &Thread, dir: &Path) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let base = dir.join(format!("{}-{}", slug(&thread.title), thread.id));
    let md = base.with_extension("md");
    std::fs::write(&md, markdown(thread))?;
    std::fs::write(
        base.with_extension("json"),
        serde_json::to_vec_pretty(thread)?,
    )?;
    Ok(md)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replies_name_their_own_agent_and_text_after_tools_starts_a_paragraph() {
        use crate::model::Workspace;
        use crate::provider::ProviderId;
        let mut w = Workspace::default();
        let p = w.add_project("/x");
        let t = w.create_thread(&p, ProviderId::Codex, "", 1);
        let parts = vec![
            Part::Tool {
                name: "Read".into(),
                summary: "a.rs".into(),
                output: String::new(),
            },
            Part::Text {
                text: "Done.".into(),
            },
        ];
        let m = w.push_message(&t, Role::Assistant, parts, 2).unwrap();
        let thread = w.thread_mut(&t).unwrap();
        thread.messages[0].agent = Some((ProviderId::Claude, String::new()));
        assert_eq!(m, thread.messages[0].id);
        assert!(markdown(thread).contains("## Claude\n\n- `Read` a.rs\n\nDone.\n"));
    }

    #[test]
    fn slugs_are_file_safe() {
        assert_eq!(
            slug("Fix PR: fetching / rate limits!"),
            "fix-pr-fetching-rate-limits"
        );
        assert_eq!(slug("???"), "thread");
    }
}
