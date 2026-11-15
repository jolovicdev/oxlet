/// How well `query` matches `text`, or `None` when its characters don't all appear in order.
/// Higher is better: matches at word starts and runs of consecutive characters score more, and
/// an earlier first match beats a later one. Case is ignored.
pub fn score(query: &str, text: &str) -> Option<i32> {
    let query: Vec<char> = query
        .chars()
        .filter(|c| !c.is_whitespace())
        .flat_map(char::to_lowercase)
        .collect();
    if query.is_empty() {
        return Some(0);
    }
    let chars: Vec<char> = text.chars().collect();
    let lower: Vec<char> = chars
        .iter()
        .map(|c| c.to_lowercase().next().unwrap_or(*c))
        .collect();
    let mut score = 0;
    let mut qi = 0;
    let mut previous: Option<usize> = None;
    let mut first = None;
    for (i, &c) in lower.iter().enumerate() {
        if qi == query.len() {
            break;
        }
        if c != query[qi] {
            continue;
        }
        let word_start = i == 0
            || !chars[i - 1].is_alphanumeric()
            || (chars[i].is_uppercase() && chars[i - 1].is_lowercase());
        score += 1;
        if word_start {
            score += 8;
        }
        if previous.is_some_and(|p| p + 1 == i) {
            score += 5;
        }
        first.get_or_insert(i);
        previous = Some(i);
        qi += 1;
    }
    (qi == query.len()).then(|| score - first.unwrap_or(0).min(20) as i32)
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
