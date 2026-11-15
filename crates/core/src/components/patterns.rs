/// Glob terms are anchored; ordinary words are substrings. SQL metacharacters stay literal.
pub fn sql(query: &str) -> Vec<String> {
    query
        .split_whitespace()
        .map(|term| {
            let glob = term.contains(['*', '?']);
            let mut pattern = String::new();
            if !glob {
                pattern.push('%');
            }
            for character in term.chars() {
                match character {
                    '*' => pattern.push('%'),
                    '?' => pattern.push('_'),
                    '%' | '_' | '\\' => {
                        pattern.push('\\');
                        pattern.push(character);
                    }
                    other => pattern.push(other),
                }
            }
            if !glob {
                pattern.push('%');
            }
            pattern
        })
        .collect()
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn escapes_sql_wildcards_and_anchors_globs() {
        assert_eq!(
            sql("github.* pull 100% a_b a\\b"),
            ["github.%", "%pull%", "%100\\%%", "%a\\_b%", "%a\\\\b%"]
        );
        assert_eq!(sql("*.delete_*"), ["%.delete\\_%"]);
        assert!(sql(" ").is_empty());
    }
}
