#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    Clear,
    Compact,
    Model,
    Plan,
    Debug,
    Default,
    Review,
    Fork,
    Side,
    Status,
    Export,
    Goal,
    Rename,
    Automation,
}

pub struct Spec {
    pub command: Command,
    pub name: &'static str,
    pub title: &'static str,
    pub description: &'static str,
}

pub const COMMANDS: &[Spec] = &[
    Spec {
        command: Command::Fork,
        name: "fork",
        title: "Fork",
        description: "Fork this thread into a new thread",
    },
    Spec {
        command: Command::Side,
        name: "side",
        title: "Sidechat",
        description: "Open a side thread from this one",
    },
    Spec {
        command: Command::Export,
        name: "export",
        title: "Export",
        description: "Save this thread as Markdown and JSON",
    },
    Spec {
        command: Command::Goal,
        name: "goal",
        title: "Goal",
        description: "Set or clear this thread's persistent goal",
    },
    Spec {
        command: Command::Plan,
        name: "plan",
        title: "Plan Mode",
        description: "Plan before changing anything",
    },
    Spec {
        command: Command::Debug,
        name: "debug",
        title: "Debug Mode",
        description: "Switch to evidence-first debugging",
    },
    Spec {
        command: Command::Default,
        name: "default",
        title: "Default Mode",
        description: "Switch back to normal chat",
    },
    Spec {
        command: Command::Review,
        name: "review",
        title: "Review",
        description: "Ask the agent to review the working changes",
    },
    Spec {
        command: Command::Compact,
        name: "compact",
        title: "Compact",
        description: "Summarise the thread, then continue in a fresh session",
    },
    Spec {
        command: Command::Clear,
        name: "clear",
        title: "Clear",
        description: "Start a fresh session in this thread",
    },
    Spec {
        command: Command::Model,
        name: "model",
        title: "Model",
        description: "Change the model for this thread",
    },
    Spec {
        command: Command::Status,
        name: "status",
        title: "Status",
        description: "Show the thread's provider, model and session",
    },
    Spec {
        command: Command::Rename,
        name: "rename",
        title: "Rename",
        description: "Rename this thread",
    },
    Spec {
        command: Command::Automation,
        name: "automation",
        title: "Automation",
        description: "Create a scheduled automation from this prompt",
    },
];

pub fn spec(command: Command) -> &'static Spec {
    COMMANDS
        .iter()
        .find(|s| s.command == command)
        .expect("every command has a spec")
}

/// The command a composer's text starts with, and the rest of the text.
pub fn parse(input: &str) -> Option<(Command, &str)> {
    let body = input.trim_start().strip_prefix('/')?;
    let (name, rest) = body.split_once(char::is_whitespace).unwrap_or((body, ""));
    let name = name.to_ascii_lowercase();
    COMMANDS
        .iter()
        .find(|s| s.name == name)
        .map(|s| (s.command, rest.trim()))
}

/// The commands to offer while the composer holds `/query` and nothing after it.
pub fn suggestions(input: &str) -> Option<Vec<&'static Spec>> {
    let query = input.trim_start().strip_prefix('/')?;
    if query.contains(char::is_whitespace) {
        return None;
    }
    let q = query.to_ascii_lowercase();
    let mut hits: Vec<&Spec> = COMMANDS.iter().filter(|s| s.name.starts_with(&q)).collect();
    hits.extend(
        COMMANDS
            .iter()
            .filter(|s| !s.name.starts_with(&q) && s.title.to_ascii_lowercase().contains(&q)),
    );
    Some(hits)
}

pub const REVIEW_PROMPT: &str = "Review the uncommitted changes in this repository. Report concrete defects first, \
with file and line, then risky patterns. Do not modify files.";

pub const COMPACT_PROMPT: &str = "Summarise this conversation so far: the goal, decisions made, files touched and \
what remains. Be concise; the summary will seed a fresh session.";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_name_and_argument_case_insensitively() {
        assert_eq!(parse("  /GOAL ship it "), Some((Command::Goal, "ship it")));
        assert_eq!(parse("/fork"), Some((Command::Fork, "")));
        assert_eq!(parse("/nope x"), None);
        assert_eq!(parse("hello /fork"), None);
    }

    #[test]
    fn suggestions_rank_prefixes_before_title_matches_and_stop_at_a_space() {
        let names: Vec<_> = suggestions("/de").unwrap().iter().map(|s| s.name).collect();
        assert_eq!(names[..2], ["debug", "default"]);
        assert_eq!(suggestions("/").unwrap().len(), COMMANDS.len());
        assert!(suggestions("/goal x").is_none());
        assert!(suggestions("goal").is_none());
        assert_eq!(suggestions("  /cle").unwrap()[0].name, "clear");
    }
}
