pub(crate) fn title(code: &str, paths: &[String]) -> String {
    let comment = code.lines().find_map(|line| {
        let text = line.trim_start().strip_prefix('#')?.trim();
        (!text.is_empty()).then_some(text)
    });
    let fallback;
    let text = match comment {
        Some(comment) => comment,
        None if !paths.is_empty() => {
            fallback = paths.join(" → ");
            &fallback
        }
        None => "Python execution",
    };
    text.chars().take(160).collect()
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn first_nonempty_comment_wins_otherwise_paths_stay_in_call_order() {
        let paths = vec!["sentry.list_issues".into(), "linear.create_issue".into()];
        assert_eq!(
            title("#\n  # Find today's issues\n# ignored", &paths),
            "Find today's issues"
        );
        assert_eq!(
            title("return '# not a comment'", &paths),
            "sentry.list_issues → linear.create_issue"
        );
        assert_eq!(title("return 42", &[]), "Python execution");
        assert_eq!(
            title(&format!("# {}", "雪".repeat(200)), &[]),
            "雪".repeat(160)
        );
    }
}
