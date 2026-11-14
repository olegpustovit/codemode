use super::catalogue::Definition;
use apollo_parser::cst::{self, CstNode};
use serde::{
    Deserialize,
    de::{self, SeqAccess, Visitor},
};
use serde_json::{Value, value::RawValue};
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    time::Duration,
};

const DOCUMENT_BYTES: usize = 64 * 1024;
const RESULT_BYTES: usize = 8 * 1024 * 1024;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    Query,
    Mutation,
}
impl Kind {
    fn name(self) -> &'static str {
        match self {
            Self::Query => "query",
            Self::Mutation => "mutation",
        }
    }
    pub(crate) fn definition(self, namespace: &str) -> Definition {
        let name = self.name();
        let description = format!(
            "Run a GraphQL {name}. Pass a complete document in query, optional variables, and operationName when the document has multiple operations. Use query with introspection to discover the schema. Only the selected {name} and its fragments are sent. Subscriptions are unsupported. Mutations require approval by default. Returns the data object; GraphQL errors raise ToolError, including partial-data responses."
        );
        let input_schema = serde_json::json!({"type":"object","properties":{
            "query":{"type":"string","minLength":1,"maxLength":DOCUMENT_BYTES,"description":"Complete GraphQL document, at most 64 KiB"},
            "variables":{"type":["object","null"],"description":"GraphQL variable values"},
            "operationName":{"type":["string","null"],"maxLength":1024,"description":"Selected operation name"}
        },"required":["query"],"additionalProperties":false});
        Definition {
            upstream_name: name.into(),
            path: format!("{namespace}.{name}"),
            description,
            input_schema,
            output_schema: None,
            read_only: self == Self::Query,
            destructive: self == Self::Mutation,
        }
    }
    pub(crate) fn document(self, args: &Value) -> Result<String, String> {
        let query = args
            .get("query")
            .and_then(Value::as_str)
            .ok_or("query must be a GraphQL document")?;
        if query.is_empty() || query.len() > DOCUMENT_BYTES {
            return Err("GraphQL document must contain 1..65536 bytes".into());
        }
        let tree = apollo_parser::Parser::new(query)
            .recursion_limit(64)
            .token_limit(16384)
            .parse();
        if let Some(error) = tree.errors().next() {
            // Parser diagnostics may quote a credential from a literal; return only its location.
            return Err(format!(
                "invalid GraphQL syntax or parser limit at byte {}",
                error.index()
            ));
        }
        let mut operations = Vec::new();
        let mut names = BTreeSet::new();
        let mut fragments = BTreeMap::new();
        for definition in tree.document().definitions() {
            match definition {
                cst::Definition::OperationDefinition(op) => {
                    let name = op.name().map(|n| n.text().to_string());
                    if !names.insert(name) {
                        return Err("GraphQL operation names must be unique".into());
                    }
                    operations.push(op);
                }
                cst::Definition::FragmentDefinition(fragment) => {
                    let name = fragment
                        .fragment_name()
                        .ok_or("GraphQL fragment needs a name")?
                        .source_string();
                    if fragments.insert(name, fragment).is_some() {
                        return Err("GraphQL fragment names must be unique".into());
                    }
                }
                _ => {
                    return Err(
                        "GraphQL document must contain only operations and fragments".into(),
                    );
                }
            }
        }
        if operations.is_empty() {
            return Err("GraphQL document needs an operation".into());
        }
        if operations.len() > 1 && names.contains(&None) {
            return Err("an anonymous GraphQL operation must stand alone".into());
        }
        let name = args.get("operationName").and_then(Value::as_str);
        let selected = match name {
            Some(name) => operations
                .iter()
                .find(|op| op.name().is_some_and(|n| n.text() == name))
                .ok_or("GraphQL operationName does not match an operation")?,
            None if operations.len() == 1 => &operations[0],
            None => return Err("operationName is required for multiple GraphQL operations".into()),
        };
        let kind = selected.operation_type();
        if kind
            .as_ref()
            .is_some_and(|k| k.subscription_token().is_some())
        {
            return Err("GraphQL subscriptions are unsupported".into());
        }
        let selected_kind = if kind.is_some_and(|k| k.mutation_token().is_some()) {
            Self::Mutation
        } else {
            Self::Query
        };
        if selected_kind != self {
            return Err(format!(
                "selected operation is a {}; call the {} tool",
                selected_kind.name(),
                selected_kind.name()
            ));
        }
        let mut document = selected.source_string();
        let spreads = |node: &apollo_parser::SyntaxNode| {
            node.descendants()
                .filter_map(cst::FragmentSpread::cast)
                .filter_map(|spread| spread.fragment_name())
                .map(|name| name.source_string())
                .collect::<Vec<_>>()
        };
        let mut pending = spreads(selected.syntax());
        let mut used = BTreeSet::new();
        while let Some(name) = pending.pop() {
            if !used.insert(name.clone()) {
                continue;
            }
            let fragment = fragments
                .get(&name)
                .ok_or("GraphQL fragment is not defined")?;
            pending.extend(spreads(fragment.syntax()));
            document.push('\n');
            document.push_str(&fragment.source_string());
        }
        Ok(document)
    }
}

pub(crate) fn client() -> Result<reqwest::Client, reqwest::Error> {
    reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .retry(reqwest::retry::never())
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(60))
        .user_agent("codemode-gateway/0.1")
        .build()
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum Error {
    #[error("GraphQL request failed; request was not retried")]
    Request,
    #[error("GraphQL server returned HTTP {0}; request was not retried")]
    Status(u16),
    #[error("GraphQL response exceeds 8 MiB")]
    Size,
    #[error("GraphQL server returned an invalid JSON response")]
    Invalid,
    #[error("{0}")]
    Tool(String),
}
pub(crate) async fn call(
    client: &reqwest::Client,
    request: reqwest::Request,
) -> Result<(Vec<u8>, u16), Error> {
    let mut response = client.execute(request).await.map_err(|_| Error::Request)?;
    let status = response.status();
    if !status.is_success() && status.as_u16() != 400 {
        return Err(Error::Status(status.as_u16()));
    }
    if response
        .content_length()
        .is_some_and(|n| n > RESULT_BYTES as u64)
    {
        return Err(Error::Size);
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| Error::Request)? {
        if bytes.len() + chunk.len() > RESULT_BYTES {
            return Err(Error::Size);
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok((bytes, status.as_u16()))
}
fn present<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<&'de RawValue>, D::Error> {
    <&RawValue>::deserialize(deserializer).map(Some)
}
#[derive(Deserialize)]
struct Envelope<'a> {
    #[serde(default, borrow, deserialize_with = "present")]
    data: Option<&'a RawValue>,
    #[serde(borrow)]
    errors: Option<&'a RawValue>,
}
struct Errors(Option<String>);
impl<'de> Deserialize<'de> for Errors {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct First;
        impl<'de> Visitor<'de> for First {
            type Value = Errors;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a GraphQL error array")
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Errors, A::Error> {
                #[derive(Deserialize)]
                struct Entry {
                    message: String,
                }
                let first = seq.next_element::<Entry>()?.map(|e| e.message);
                while seq.next_element::<de::IgnoredAny>()?.is_some() {}
                Ok(Errors(first))
            }
        }
        d.deserialize_seq(First)
    }
}
pub(crate) fn result(bytes: &[u8], status: u16) -> Result<Box<RawValue>, Error> {
    let envelope: Envelope<'_> = serde_json::from_slice(bytes).map_err(|_| Error::Invalid)?;
    if let Some(errors) = envelope.errors {
        let errors: Errors = serde_json::from_str(errors.get()).map_err(|_| Error::Invalid)?;
        if let Some(message) = errors.0 {
            return Err(Error::Tool(message));
        }
    }
    if !(200..300).contains(&status) {
        return Err(Error::Status(status));
    }
    envelope.data.map(RawValue::to_owned).ok_or(Error::Invalid)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn selects_kind_and_only_reachable_fragments_without_rewriting_literals() {
        let args = json!({"query":"query Read($id: ID!) { item(id:$id) { ...A } } mutation Write { deleteItem } fragment Unused on Mutation { deleteItem } fragment A on Item { value(text:\"mutation { danger }\", n:1.234567890123456789) ...B } fragment B on Item { id }","operationName":"Read","variables":{"id":"one"}});
        let document = Kind::Query.document(&args).unwrap();
        assert!(document.contains("1.234567890123456789"));
        assert!(document.contains("fragment A"));
        assert!(document.contains("fragment B"));
        assert!(!document.contains("fragment Unused"));
        assert!(!document.contains("mutation Write"));
        assert!(Kind::Mutation.document(&args).is_err());
        assert!(
            Kind::Query
                .document(&json!({"query":"mutation { deleteItem }"}))
                .is_err()
        );
        assert!(
            Kind::Mutation
                .document(&json!({"query":"mutation { deleteItem }"}))
                .is_ok()
        );
        assert!(
            Kind::Query
                .document(&json!({"query":"{ viewer { id } }"}))
                .is_ok()
        );
    }
    #[test]
    fn rejects_ambiguous_malformed_oversized_or_non_executable_documents() {
        for args in [
            json!({"query":"query A { x } query B { x }"}),
            json!({"query":"query A { x } query A { y }","operationName":"A"}),
            json!({"query":"{ x } query B { x }","operationName":"B"}),
            json!({"query":"query A { x }","operationName":"Missing"}),
            json!({"query":"subscription { x }"}),
            json!({"query":"type Thing { x: Int }"}),
            json!({"query":"{ ...Missing }"}),
            json!({"query":"query { field(credential:\"do-not-echo\")"}),
            json!({"query":" ".repeat(DOCUMENT_BYTES+1)}),
            json!({"query":format!("{} x {}","{ a ".repeat(100),"}".repeat(100))}),
        ] {
            let error = Kind::Query.document(&args).unwrap_err();
            assert!(!error.contains("do-not-echo"));
        }
    }
    #[test]
    fn preserves_raw_data_and_raises_graphql_errors_even_with_partial_data() {
        let body =
            br#"{"data":{"big":123456789012345678901,"x":[true,null]},"extensions":{"x":1}}"#;
        let allocations = allocation_counter::measure(|| {
            assert_eq!(
                result(body, 200).unwrap().get(),
                r#"{"big":123456789012345678901,"x":[true,null]}"#
            )
        });
        assert_eq!(
            allocations.count_total, 2,
            "measured GraphQL data pass-through budget: raw output and JSON parser scratch; no value tree"
        );
        assert_eq!(result(br#"{"data":null}"#, 200).unwrap().get(), "null");
        assert!(
            matches!(result(br#"{"data":{"partial":true},"errors":[{"message":"precise failure"}]}"#,400),Err(Error::Tool(message)) if message=="precise failure")
        );
        for invalid in [
            b"{}".as_slice(),
            br#"{"errors":{}}"#,
            br#"{"errors":[{}]}"#,
            b"not json",
        ] {
            assert!(result(invalid, 200).is_err());
        }
    }
}
