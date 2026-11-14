use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

const KEYWORDS: &[&str] = &[
    "False", "None", "True", "and", "as", "assert", "async", "await", "break", "class", "continue",
    "def", "del", "elif", "else", "except", "finally", "for", "from", "global", "if", "import",
    "in", "is", "lambda", "nonlocal", "not", "or", "pass", "raise", "return", "try", "while",
    "with", "yield", "self", "kwargs",
];

pub fn identifier(name: &str) -> String {
    let mut out: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if out.is_empty() {
        out.push('_');
    }
    if out.as_bytes()[0].is_ascii_digit() {
        out.insert(0, '_');
    }
    if KEYWORDS.contains(&out.as_str()) || (out.starts_with("__") && out.ends_with("__")) {
        out.push('_');
    }
    out
}

pub fn names<'a>(originals: impl IntoIterator<Item = &'a str>) -> BTreeMap<String, String> {
    let mut used = BTreeSet::new();
    let mut result = BTreeMap::new();
    let originals: BTreeSet<_> = originals.into_iter().collect();
    for original in originals {
        let base = identifier(original);
        let mut name = base.clone();
        let mut suffix = 2;
        while !used.insert(name.clone()) {
            name = format!("{base}_{suffix}");
            suffix += 1;
        }
        result.insert(original.to_owned(), name);
    }
    result
}

pub struct Stubs {
    pub brief: String,
    pub full: String,
}

pub fn generate(path: &str, description: &str, input: &Value, output: Option<&Value>) -> Stubs {
    let mut generator = Generator {
        root: input,
        definitions: Vec::new(),
        active: BTreeSet::new(),
        used: BTreeSet::new(),
    };
    let mut args = Vec::new();
    if let Some(properties) = input.get("properties").and_then(Value::as_object) {
        let required = input.get("required").and_then(Value::as_array);
        let aliases = names(properties.keys().map(String::as_str));
        for required_first in [true, false] {
            for (name, schema) in properties {
                let is_required =
                    required.is_some_and(|r| r.iter().any(|v| v.as_str() == Some(name)));
                if is_required != required_first {
                    continue;
                }
                let hint =
                    generator.hint(schema, &format!("{}_{}", path.replace('.', "_"), name), 0);
                args.push(format!(
                    "{}: {hint}{}",
                    aliases[name],
                    if is_required { "" } else { " = None" }
                ));
            }
        }
    }
    let result = if let Some(schema) = output {
        generator.root = schema;
        generator.hint(schema, &format!("{}_result", path.replace('.', "_")), 0)
    } else {
        "Any".into()
    };
    let arguments = if args.is_empty() {
        String::new()
    } else {
        format!("*, {}", args.join(", "))
    };
    let signature = format!("async def tools.{path}({arguments}) -> {result}: ...");
    let first_line = description.lines().next().unwrap_or_default();
    let brief = format!("{signature}\n# {first_line}");
    let full = format!(
        "from typing import Any, Literal, NotRequired, TypedDict\n\n{}\n{signature}\n# {}",
        generator.definitions.join("\n\n"),
        description.replace('\n', "\n# ")
    );
    Stubs { brief, full }
}

struct Generator<'a> {
    root: &'a Value,
    definitions: Vec<String>,
    active: BTreeSet<String>,
    used: BTreeSet<String>,
}

impl Generator<'_> {
    fn hint(&mut self, schema: &Value, label: &str, depth: usize) -> String {
        if depth > 32 {
            return "Any".into();
        }
        if let Some(reference) = schema.get("$ref").and_then(Value::as_str) {
            if !reference.starts_with("#/") || !self.active.insert(reference.to_owned()) {
                return "Any".into();
            }
            let result = self
                .root
                .pointer(&reference[1..])
                .cloned()
                .map(|s| self.hint(&s, label, depth + 1))
                .unwrap_or_else(|| "Any".into());
            self.active.remove(reference);
            return result;
        }
        if let Some(values) = schema.get("enum").and_then(Value::as_array) {
            let values: Option<Vec<_>> = values.iter().map(literal).collect();
            return values
                .filter(|v| !v.is_empty())
                .map(|v| format!("Literal[{}]", v.join(", ")))
                .unwrap_or_else(|| "Any".into());
        }
        if let Some(value) = schema.get("const") {
            return literal(value)
                .map(|s| format!("Literal[{s}]"))
                .unwrap_or_else(|| "Any".into());
        }
        if let Some(union) = schema
            .get("anyOf")
            .or_else(|| schema.get("oneOf"))
            .and_then(Value::as_array)
        {
            let parts: BTreeSet<_> = union
                .iter()
                .map(|s| self.hint(s, label, depth + 1))
                .collect();
            return if parts.is_empty() {
                "Any".into()
            } else {
                parts.into_iter().collect::<Vec<_>>().join(" | ")
            };
        }
        if let Some(types) = schema.get("type").and_then(Value::as_array) {
            return types
                .iter()
                .map(|t| {
                    let mut s = schema.clone();
                    s["type"] = t.clone();
                    self.hint(&s, label, depth + 1)
                })
                .collect::<Vec<_>>()
                .join(" | ");
        }
        match schema.get("type").and_then(Value::as_str) {
            Some("string") => "str".into(),
            Some("integer") => "int".into(),
            Some("number") => "float".into(),
            Some("boolean") => "bool".into(),
            Some("null") => "None".into(),
            Some("array") => format!(
                "list[{}]",
                schema
                    .get("items")
                    .map(|s| self.hint(s, label, depth + 1))
                    .unwrap_or_else(|| "Any".into())
            ),
            Some("object") | None if schema.get("properties").is_some() => {
                let Some(properties) = schema.get("properties").and_then(Value::as_object) else {
                    return "dict[str, Any]".into();
                };
                let base = identifier(label);
                let mut name = base.clone();
                let mut suffix = 2;
                while !self.used.insert(name.clone()) {
                    name = format!("{base}_{suffix}");
                    suffix += 1;
                }
                let mut fields = Vec::new();
                for (key, value) in properties {
                    let hint = self.hint(value, &format!("{name}_{key}"), depth + 1);
                    let required = schema
                        .get("required")
                        .and_then(Value::as_array)
                        .is_some_and(|r| r.iter().any(|v| v.as_str() == Some(key)));
                    fields.push(format!(
                        "{}: {}",
                        quote(key),
                        if required {
                            hint
                        } else {
                            format!("NotRequired[{hint}]")
                        }
                    ));
                }
                self.definitions.push(format!(
                    "{name} = TypedDict({}, {{{}}})",
                    quote(&name),
                    fields.join(", ")
                ));
                name
            }
            Some("object") => "dict[str, Any]".into(),
            _ => "Any".into(),
        }
    }
}

fn literal(value: &Value) -> Option<String> {
    match value {
        Value::String(s) => Some(quote(s)),
        Value::Bool(b) => Some(if *b { "True" } else { "False" }.into()),
        Value::Null => Some("None".into()),
        Value::Number(n) if n.is_i64() || n.is_u64() => Some(n.to_string()),
        _ => None,
    }
}

pub fn quote(value: &str) -> String {
    // JSON string escapes are also valid Python string escapes; no executable names enter generated source.
    serde_json::to_string(value).unwrap_or_else(|_| "\"\"".into())
}

pub fn search_words(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 16);
    let mut previous_lower = false;
    let mut previous_upper = false;
    let mut chars = value.chars().peekable();
    while let Some(c) = chars.next() {
        if c.is_uppercase()
            && (previous_lower
                || previous_upper && chars.peek().is_some_and(|next| next.is_lowercase()))
        {
            out.push(' ');
        }
        if c.is_alphanumeric() {
            out.push(c);
            previous_lower = c.is_lowercase();
            previous_upper = c.is_uppercase();
        } else {
            out.push(' ');
            previous_lower = false;
            previous_upper = false;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn aliases_are_safe_deterministic_and_distinct() {
        assert_eq!(identifier("__init__"), "__init___");
        assert_eq!(identifier("__dict__"), "__dict___");
        let aliases = names(["class", "foo-bar", "foo_bar", "foo_bar_2", "2x", "x\npass"]);
        assert_eq!(aliases["class"], "class_");
        assert_eq!(aliases["2x"], "_2x");
        assert_eq!(aliases.values().collect::<BTreeSet<_>>().len(), 6);
        assert_eq!(
            search_words("listIssues.foo_bar-baz"),
            "list Issues foo bar baz"
        );
        assert_eq!(
            search_words("getHTTPResponse.XMLHttpRequest"),
            "get HTTP Response XML Http Request"
        );
    }
    #[test]
    fn schemas_include_optional_nested_union_and_recursive_fallback() {
        let schema = json!({"type":"object","properties":{"class":{"type":"string","enum":["a","b"]},"data":{"$ref":"#/$defs/Node"}},"required":["class"],"$defs":{"Node":{"type":"object","properties":{"count":{"type":["integer","null"]},"child":{"$ref":"#/$defs/Node"}}}}});
        let stub = generate("example.run", "description", &schema, None);
        assert!(stub.brief.contains("class_: Literal[\"a\", \"b\"]"));
        assert!(stub.brief.contains("data: example_run_data = None"));
        assert!(stub.full.contains("NotRequired[int | None]"));
        assert!(stub.full.contains("\"child\": NotRequired[Any]"));
    }
    #[test]
    fn real_trial_schema_corpus_has_stable_signatures() {
        let corpus: Value =
            serde_json::from_str(include_str!("../../../../../docs/trial-schemas.json")).unwrap();
        let mut count = 0;
        let mut golden = String::new();
        for (source, tools) in corpus.as_object().unwrap() {
            let aliases = names(
                tools
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|tool| tool["name"].as_str().unwrap()),
            );
            for tool in tools.as_array().unwrap() {
                let name = tool["name"].as_str().unwrap();
                let stub = generate(
                    &format!("{source}.{}", aliases[name]),
                    tool["description"].as_str().unwrap_or_default(),
                    &tool["inputSchema"],
                    tool.get("outputSchema"),
                );
                assert!(stub.brief.starts_with("async def tools."));
                assert!(!stub.brief.contains(" | null"));
                golden.push_str(&format!(
                    "## {source}.{name}\n{}\n{}\n\n",
                    stub.brief, stub.full
                ));
                count += 1;
            }
        }
        assert_eq!(count, 61);
        assert_eq!(
            golden,
            include_str!("../../../../../docs/trial-stubs.golden")
        );
    }
}
