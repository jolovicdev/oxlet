/// How well `query` matches `text`, or `None` when its characters don't all appear in order.
/// Higher is better: matches at word starts and runs of consecutive characters score more, and
/// an earlier first match beats a later one. Case is ignored.
pub fn score(query: &str, text: &str) -> Option<i32> {
    let mut query = query
        .chars()
        .filter(|c| !c.is_whitespace())
        .flat_map(char::to_lowercase)
        .peekable();
    if query.peek().is_none() {
        return Some(0);
    }
    let mut score = 0;
    let mut previous: Option<usize> = None;
    let mut first = None;
    let mut before: Option<char> = None;
    for (i, c) in text.chars().enumerate() {
        let Some(&wanted) = query.peek() else {
            break;
        };
        if c.to_lowercase().next().unwrap_or(c) == wanted {
            let word_start = before
                .is_none_or(|b| !b.is_alphanumeric() || (c.is_uppercase() && b.is_lowercase()));
            score += 1;
            if word_start {
                score += 8;
            }
            if previous.is_some_and(|p| p + 1 == i) {
                score += 5;
            }
            first.get_or_insert(i);
            previous = Some(i);
            query.next();
        }
        before = Some(c);
    }
    query
        .peek()
        .is_none()
        .then(|| score - first.unwrap_or(0).min(20) as i32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requires_every_character_in_order() {
        assert!(score("ntd", "New thread").is_some());
        assert!(score("dtn", "New thread").is_none());
        assert_eq!(score("", "anything"), Some(0));
    }

    #[test]
    fn prefers_word_starts_and_runs() {
        let starts = score("pr", "Pull requests").unwrap();
        let scattered = score("pr", "Import").unwrap();
        assert!(starts > scattered);
        let run = score("set", "Settings").unwrap();
        let spread = score("set", "Show the editor").unwrap();
        assert!(run > spread);
    }

    #[test]
    fn ignores_case_and_spaces_in_the_query() {
        assert_eq!(score("NEW T", "new thread"), score("newt", "New Thread"));
    }
}
