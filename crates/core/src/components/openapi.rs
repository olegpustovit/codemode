mod parameters;
mod schema;
use super::catalogue::{Definition, stubs};
pub(crate) use parameters::controlled_header;
use parameters::{Location, Parameter, percent};
use serde_json::{Map, Value, json};
use std::{collections::BTreeSet, time::Duration};
use url::Url;

static ANY_SCHEMA: Value = Value::Bool(true);
const DOCUMENT_BYTES: usize = 2 * 1024 * 1024;
const RESULT_BYTES: usize = 8 * 1024 * 1024;
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Invalid(&'static str),
    #[error("{0}")]
    Arguments(&'static str),
    #[error("OpenAPI document could not be fetched; redirects are disabled")]
    Fetch,
    #[error("OpenAPI request failed")]
    Request,
    #[error("OpenAPI server returned HTTP {0}; request was not retried")]
    Status(u16),
}
pub(crate) struct Loaded {
    pub server: String,
    pub client: reqwest::Client,
    pub operations: Vec<Operation>,
}
#[derive(Clone)]
pub(crate) struct Operation {
    pub definition: Definition,
    pub validator: std::sync::Arc<jsonschema::Validator>,
    method: reqwest::Method,
    path: String,
    server: Url,
    parameters: Vec<Parameter>,
    body: Option<(String, String)>,
}

pub(crate) fn server_url(value: &str) -> Result<Url, Error> {
    let url = Url::parse(value).map_err(|_| Error::Invalid("invalid API server URL"))?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(Error::Invalid(
            "API server URL must use HTTP(S) without credentials, query or fragment",
        ));
    }
    Ok(url)
}
pub(crate) fn load(
    url: String,
    fixed_server: Option<String>,
    namespace: String,
    permit: tokio::sync::OwnedSemaphorePermit,
) -> futures_util::future::BoxFuture<'static, Result<Loaded, Error>> {
    // Keep HTTP discovery's large future out of source CRUD handler instantiations.
    Box::pin(async move {
        let client = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(60))
            .user_agent("codemode-gateway/0.1")
            .build()
            .map_err(|_| Error::Fetch)?;
        let mut response = client
            .get(&url)
            .timeout(Duration::from_secs(10))
            .send()
            .await
            .map_err(|_| Error::Fetch)?;
        if !response.status().is_success() {
            return Err(Error::Fetch);
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| Error::Fetch)? {
            if bytes.len() + chunk.len() > DOCUMENT_BYTES {
                return Err(Error::Invalid("OpenAPI document exceeds 2 MiB"));
            }
            bytes.extend_from_slice(&chunk);
        }
        let (server, operations) = tokio::task::spawn_blocking(move || {
            // A cancelled request must not release its slot while detached compilation is still running.
            let _permit = permit;
            parse(&bytes, &url, fixed_server.as_deref(), &namespace)
        })
        .await
        .map_err(|_| Error::Invalid("OpenAPI compiler failed"))??;
        Ok(Loaded {
            server,
            client,
            operations,
        })
    })
}
fn parse(
    bytes: &[u8],
    document_url: &str,
    fixed_server: Option<&str>,
    namespace: &str,
) -> Result<(String, Vec<Operation>), Error> {
    if bytes.len() > DOCUMENT_BYTES {
        return Err(Error::Invalid("OpenAPI document exceeds 2 MiB"));
    }
    let text =
        std::str::from_utf8(bytes).map_err(|_| Error::Invalid("OpenAPI document must be UTF-8"))?;
    let mut budget = serde_saphyr::Budget::default();
    budget.max_depth = 64;
    budget.max_nodes = 100_000;
    budget.max_events = 200_000;
    budget.max_aliases = 1000;
    budget.max_total_scalar_bytes = 4 * DOCUMENT_BYTES;
    let mut options = serde_saphyr::Options::default();
    options.budget = Some(budget);
    let root: Value = serde_saphyr::from_str_with_options(text, options)
        .map_err(|_| Error::Invalid("invalid or over-budget OpenAPI JSON/YAML document"))?;
    let version = root
        .get("openapi")
        .and_then(Value::as_str)
        .ok_or(Error::Invalid("OpenAPI 3.x is required"))?;
    if !version.starts_with("3.") {
        return Err(Error::Invalid("OpenAPI 3.x is required"));
    }
    let mut server = fixed_server.map(server_url).transpose()?;
    let paths = root
        .get("paths")
        .and_then(Value::as_object)
        .ok_or(Error::Invalid("OpenAPI paths are required"))?;
    let mut operations = Vec::new();
    let mut names = BTreeSet::new();
    let mut budget = 200_000;
    for (path, item) in paths {
        if path.starts_with("x-") {
            continue;
        }
        if !path.starts_with('/')
            || path.contains(['?', '#'])
            || path
                .split('/')
                .any(|s| s == "." || s == ".." || s.to_ascii_lowercase().contains("%2e"))
        {
            return Err(Error::Invalid(
                "OpenAPI paths must be absolute paths without dot segments, query or fragment",
            ));
        }
        let item = dereference(&root, item)?;
        for method in [
            "get", "put", "post", "delete", "options", "head", "patch", "trace", "query",
        ] {
            let Some(operation) = item.get(method) else {
                continue;
            };
            if operations.len() >= 1000 {
                return Err(Error::Invalid("maximum 1000 OpenAPI operations per source"));
            }
            let operation = dereference(&root, operation)?;
            let operation_server = if fixed_server.is_some() {
                server
                    .clone()
                    .ok_or(Error::Invalid("API server is not pinned"))?
            } else {
                let owner = if operation.get("servers").is_some() {
                    operation
                } else if item.get("servers").is_some() {
                    item
                } else {
                    &root
                };
                let advertised = default_server(owner, document_url)?;
                if server.as_ref().is_some_and(|pinned| pinned != &advertised) {
                    return Err(Error::Invalid(
                        "operations use different API servers; enter one API server address to override them",
                    ));
                }
                server = Some(advertised.clone());
                advertised
            };
            let name = operation
                .get("operationId")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .unwrap_or_else(|| format!("{method}_{path}"));
            if name.is_empty() || name.len() > 512 || !names.insert(name.clone()) {
                return Err(Error::Invalid(
                    "operation IDs must be nonempty, unique and at most 512 bytes",
                ));
            }
            let mut parameters = Vec::<Parameter>::new();
            let mut inputs = Map::new();
            let mut required = Vec::new();
            let mut seen = BTreeSet::new();
            let mut merged = std::collections::BTreeMap::new();
            for parent in [item, operation] {
                if let Some(values) = parent.get("parameters") {
                    for parameter in values
                        .as_array()
                        .ok_or(Error::Invalid("invalid OpenAPI parameter list"))?
                    {
                        let parameter = dereference(&root, parameter)?;
                        let name = string(parameter, "name")?;
                        let location = string(parameter, "in")?;
                        merged.insert((location.to_owned(), name.to_owned()), parameter);
                    }
                }
            }
            for ((location, name), parameter) in merged {
                let location = match location.as_str() {
                    "path" => Location::Path,
                    "query" => Location::Query,
                    "header" => Location::Header,
                    _ => {
                        return Err(Error::Invalid(
                            "only path, query and header OpenAPI parameters are supported",
                        ));
                    }
                };
                if location == Location::Header
                    && (controlled_header(&name)
                        || matches!(
                            name.to_ascii_lowercase().as_str(),
                            "accept" | "content-type"
                        ))
                {
                    continue;
                }
                let mut argument = name.clone();
                if !seen.insert(argument.clone()) {
                    argument = format!(
                        "{}_{name}",
                        match location {
                            Location::Path => "path",
                            Location::Query => "query",
                            Location::Header => "header",
                        }
                    );
                    while !seen.insert(argument.clone()) {
                        argument.insert(0, '_');
                    }
                }
                let style = parameter
                    .get("style")
                    .and_then(Value::as_str)
                    .unwrap_or(if location == Location::Query {
                        "form"
                    } else {
                        "simple"
                    })
                    .to_owned();
                let parameter_schema = parameter
                    .get("schema")
                    .ok_or(Error::Invalid("OpenAPI parameters need a schema"))?;
                inputs.insert(argument.clone(), parameter_schema.clone());
                if location == Location::Path
                    || parameter.get("required") == Some(&Value::Bool(true))
                {
                    required.push(Value::String(argument.clone()));
                }
                let binding = Parameter {
                    argument,
                    name,
                    location,
                    explode: parameter
                        .get("explode")
                        .and_then(Value::as_bool)
                        .unwrap_or(style == "form"),
                    style,
                    allow_reserved: parameter.get("allowReserved") == Some(&Value::Bool(true)),
                };
                binding.validate()?;
                parameters.push(binding);
            }
            let body = if let Some(body) = operation.get("requestBody") {
                let body = dereference(&root, body)?;
                let (content_type, body_schema) = json_content(body)?.ok_or(Error::Invalid(
                    "only JSON OpenAPI request bodies are supported",
                ))?;
                let mut argument = "body".to_owned();
                while !seen.insert(argument.clone()) {
                    argument.insert(0, '_');
                }
                inputs.insert(argument.clone(), body_schema.clone());
                if body.get("required") == Some(&Value::Bool(true)) {
                    required.push(Value::String(argument.clone()));
                }
                Some((argument, content_type.to_owned()))
            } else {
                None
            };
            let input_schema = schema::input(
                &root,
                &json!({"type":"object","properties":inputs,"required":required,"additionalProperties":false}),
                &mut budget,
            )?;
            let validator = std::sync::Arc::new(
                jsonschema::options()
                    .offline()
                    .build(&input_schema)
                    .map_err(|_| Error::Invalid("invalid OpenAPI input schema"))?,
            );
            let mut output_schema = None;
            if let Some(responses) = operation.get("responses").and_then(Value::as_object) {
                for (status, response) in responses {
                    if status.starts_with('2') {
                        if let Some((_, value)) = json_content(dereference(&root, response)?)? {
                            output_schema = Some(schema::resolve(&root, value, &mut budget)?);
                            break;
                        }
                    }
                }
            }
            let description = operation
                .get("description")
                .or_else(|| operation.get("summary"))
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned();
            let method = reqwest::Method::from_bytes(method.to_uppercase().as_bytes())
                .map_err(|_| Error::Invalid("invalid operation method"))?;
            operations.push(Operation {
                definition: Definition {
                    upstream_name: name,
                    path: String::new(),
                    description,
                    input_schema,
                    output_schema,
                    read_only: matches!(method, reqwest::Method::GET | reqwest::Method::HEAD),
                    destructive: method == reqwest::Method::DELETE
                        || operation.get("x-destructive") == Some(&Value::Bool(true)),
                },
                validator,
                method,
                path: path.clone(),
                server: operation_server,
                parameters,
                body,
            });
        }
    }
    let aliases = stubs::names(
        operations
            .iter()
            .map(|op| op.definition.upstream_name.as_str()),
    );
    for operation in &mut operations {
        let alias = aliases
            .get(&operation.definition.upstream_name)
            .ok_or(Error::Invalid("operation name could not be compiled"))?;
        operation.definition.path = format!("{namespace}.{alias}");
        // Validate all placeholders before any request or credential can be sent.
        let mut path = operation.path.clone();
        for p in operation
            .parameters
            .iter()
            .filter(|p| p.location == Location::Path)
        {
            path = path.replace(&format!("{{{}}}", p.name), "value");
        }
        if path.contains(['{', '}']) {
            return Err(Error::Invalid(
                "path template has no matching path parameter",
            ));
        }
    }
    let server = match server {
        Some(server) => server,
        None => default_server(&root, document_url)?,
    };
    Ok((server.to_string(), operations))
}
fn string<'a>(value: &'a Value, key: &str) -> Result<&'a str, Error> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or(Error::Invalid(
            "OpenAPI parameter needs a name and location",
        ))
}
fn dereference<'a>(root: &'a Value, mut value: &'a Value) -> Result<&'a Value, Error> {
    let mut seen = BTreeSet::new();
    while let Some(reference) = value.get("$ref") {
        let reference = reference
            .as_str()
            .ok_or(Error::Invalid("invalid OpenAPI reference"))?;
        if !seen.insert(reference) {
            return Err(Error::Invalid("cyclic OpenAPI object reference"));
        }
        let pointer = reference
            .strip_prefix('#')
            .filter(|p| p.starts_with('/'))
            .ok_or(Error::Invalid(
                "only local JSON-pointer references are supported",
            ))?;
        value = root
            .pointer(pointer)
            .ok_or(Error::Invalid("OpenAPI reference does not exist"))?;
    }
    Ok(value)
}
fn default_server(root: &Value, document_url: &str) -> Result<Url, Error> {
    let document = server_url(document_url)?;
    let Some(server) = root
        .get("servers")
        .and_then(Value::as_array)
        .and_then(|v| v.first())
    else {
        return server_url(
            document
                .join("/")
                .map_err(|_| Error::Invalid("invalid API server"))?
                .as_str(),
        );
    };
    let mut value = server
        .get("url")
        .and_then(Value::as_str)
        .ok_or(Error::Invalid("API server needs a URL"))?
        .to_owned();
    if let Some(variables) = server.get("variables").and_then(Value::as_object) {
        for (name, variable) in variables {
            let default = variable
                .get("default")
                .and_then(Value::as_str)
                .ok_or(Error::Invalid("API server variable needs a default"))?;
            value = value.replace(&format!("{{{name}}}"), default);
        }
    }
    if value.contains(['{', '}']) {
        return Err(Error::Invalid("unresolved API server variable"));
    }
    server_url(
        document
            .join(&value)
            .map_err(|_| Error::Invalid("invalid API server URL"))?
            .as_str(),
    )
}
fn json_content(value: &Value) -> Result<Option<(&str, &Value)>, Error> {
    let Some(content) = value.get("content").and_then(Value::as_object) else {
        return Ok(None);
    };
    let content = content.get_key_value("application/json").or_else(|| {
        content.iter().find(|(name, _)| {
            let mime = name.split(';').next().unwrap_or_default().trim();
            mime.eq_ignore_ascii_case("application/json") || mime.ends_with("+json")
        })
    });
    content
        .map(|(name, value)| Ok((name.as_str(), value.get("schema").unwrap_or(&ANY_SCHEMA))))
        .transpose()
}

impl Operation {
    pub(crate) fn request(
        &self,
        client: &reqwest::Client,
        args: &Map<String, Value>,
    ) -> Result<reqwest::Request, Error> {
        let mut path = self.path.clone();
        let mut query = Vec::new();
        let mut headers = reqwest::header::HeaderMap::new();
        for parameter in &self.parameters {
            let Some(value) = args.get(&parameter.argument) else {
                continue;
            };
            match parameter.location {
                Location::Path => {
                    path = path.replace(&format!("{{{}}}", parameter.name), &parameter.path(value)?)
                }
                Location::Query => query.extend(
                    parameter
                        .query(value)?
                        .into_iter()
                        .map(|(k, v)| (k, v, parameter.allow_reserved)),
                ),
                Location::Header => {
                    let name = reqwest::header::HeaderName::from_bytes(parameter.name.as_bytes())
                        .map_err(|_| Error::Arguments("invalid header parameter name"))?;
                    let value = reqwest::header::HeaderValue::from_str(&parameter.header(value)?)
                        .map_err(|_| Error::Arguments("invalid header parameter value"))?;
                    headers.insert(name, value);
                }
            }
        }
        if path.contains(['{', '}']) {
            return Err(Error::Arguments("required path parameter is missing"));
        }
        let mut url = server_url(&format!(
            "{}{path}",
            self.server.as_str().trim_end_matches('/')
        ))?;
        if url.origin() != self.server.origin()
            || !url
                .path()
                .starts_with(self.server.path().trim_end_matches('/'))
        {
            return Err(Error::Arguments("request would escape its API server"));
        }
        if !query.is_empty() {
            url.set_query(Some(
                &query
                    .into_iter()
                    .map(|(key, value, reserved)| {
                        format!("{}={}", percent(&key, false), percent(&value, reserved))
                    })
                    .collect::<Vec<_>>()
                    .join("&"),
            ));
        }
        let mut request = client.request(self.method.clone(), url).headers(headers);
        if let Some((argument, content_type)) = &self.body {
            if let Some(value) = args.get(argument) {
                request = request
                    .header(reqwest::header::CONTENT_TYPE, content_type)
                    .body(
                        serde_json::to_vec(value)
                            .map_err(|_| Error::Arguments("invalid JSON request body"))?,
                    );
            }
        }
        request.build().map_err(|_| Error::Request)
    }
    pub(crate) async fn call(
        &self,
        client: &reqwest::Client,
        request: reqwest::Request,
    ) -> Result<Box<serde_json::value::RawValue>, Error> {
        let mut response = client.execute(request).await.map_err(|_| Error::Request)?;
        if !response.status().is_success() {
            return Err(Error::Status(response.status().as_u16()));
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| Error::Request)? {
            if bytes.len() + chunk.len() > RESULT_BYTES {
                return Err(Error::Invalid("OpenAPI response exceeds 8 MiB"));
            }
            bytes.extend_from_slice(&chunk);
        }
        decode_result(bytes)
    }
}

// Budget: three allocations after collecting the response buffer, independent of JSON node count.
fn decode_result(bytes: Vec<u8>) -> Result<Box<serde_json::value::RawValue>, Error> {
    if bytes.is_empty() {
        return serde_json::value::RawValue::from_string("null".into()).map_err(|_| Error::Request);
    }
    let text =
        String::from_utf8(bytes).map_err(|_| Error::Invalid("OpenAPI response is not UTF-8"))?;
    if serde_json::from_str::<&serde_json::value::RawValue>(&text).is_ok() {
        serde_json::value::RawValue::from_string(text).map_err(|_| Error::Request)
    } else {
        serde_json::value::to_raw_value(&text).map_err(|_| Error::Request)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn compile(value: Value) -> Vec<Operation> {
        parse(
            &serde_json::to_vec(&value).unwrap(),
            "https://docs.example/spec.json",
            None,
            "api",
        )
        .unwrap()
        .1
    }
    #[test]
    fn yaml_references_parameters_bodies_and_schema_validation() {
        let yaml = r##"openapi: 3.0.3
servers: [{url: https://api.example/v1}]
components:
  schemas:
    Node:
      type: object
      properties:
        name: {type: string}
        id: {type: integer, readOnly: true}
        child: {$ref: '#/components/schemas/Node'}
      additionalProperties: false
      required: [id]
  parameters:
    Id: {name: id, in: path, required: true, schema: {type: string}}
paths:
  /nodes/{id}:
    parameters: [{$ref: '#/components/parameters/Id'}]
    put:
      operationId: replace-node
      parameters:
        - {name: tags, in: query, schema: {type: array, items: {type: string}}}
        - {name: X-Revision, in: header, schema: {type: integer}}
      requestBody:
        required: true
        content:
          application/json:
            schema: {$ref: '#/components/schemas/Node'}
      responses:
        '200':
          content:
            application/json:
              schema: {$ref: '#/components/schemas/Node'}
"##;
        let (server, mut operations) = parse(
            yaml.as_bytes(),
            "https://docs.example/openapi.yaml",
            None,
            "api",
        )
        .unwrap();
        assert_eq!(server, "https://api.example/v1");
        let operation = operations.remove(0);
        assert_eq!(operation.definition.path, "api.replace_node");
        let args = json!({"id":"a/b","tags":["one","two"],"X-Revision":2,"body":{"name":"parent","child":{"name":"child"}}});
        let validator = jsonschema::options()
            .offline()
            .build(&operation.definition.input_schema)
            .unwrap();
        assert!(validator.is_valid(&args));
        assert!(!validator.is_valid(&json!({"id":"x","body":{"child":{"name":123}}})));
        let request = operation
            .request(&reqwest::Client::new(), args.as_object().unwrap())
            .unwrap();
        assert_eq!(
            request.url().as_str(),
            "https://api.example/v1/nodes/a%2Fb?tags=one&tags=two"
        );
        assert_eq!(request.headers()["X-Revision"], "2");
        assert_eq!(
            serde_json::from_slice::<Value>(request.body().unwrap().as_bytes().unwrap()).unwrap(),
            args["body"]
        );
        assert!(operation.definition.output_schema.is_some());
    }
    #[test]
    fn pinned_server_fallback_names_overrides_nullable_and_literal_examples() {
        let document = json!({"openapi":"3.1.0","servers":[{"url":"https://changed.example"}],"paths":{
            "/rows/{id}":{"parameters":[{"name":"id","in":"path","schema":{"type":"integer"}}],
                "get":{"parameters":[{"name":"id","in":"path","schema":{"type":"string"}},
                    {"name":"optional","in":"query","schema":{"type":"string","nullable":true,"example":{"$ref":"literal data"}}}],"responses":{}}}}});
        let (server, operations) = parse(
            &serde_json::to_vec(&document).unwrap(),
            "https://docs.example/spec",
            Some("https://pinned.example/api"),
            "api",
        )
        .unwrap();
        assert_eq!(server, "https://pinned.example/api");
        let operation = &operations[0];
        assert_eq!(operation.definition.path, "api.get__rows__id_");
        let validator = jsonschema::options()
            .offline()
            .build(&operation.definition.input_schema)
            .unwrap();
        assert!(validator.is_valid(&json!({"id":"abc","optional":null})));
        assert!(!validator.is_valid(&json!({"id":3})));
        assert!(operation.definition.read_only);
        assert_eq!(
            operation
                .request(
                    &reqwest::Client::new(),
                    json!({"id":"abc"}).as_object().unwrap()
                )
                .unwrap()
                .url()
                .host_str(),
            Some("pinned.example")
        );
    }
    #[test]
    fn malformed_remote_and_cyclic_references_and_complex_documents_are_rejected() {
        for document in [
            json!({"openapi":"2.0","paths":{}}),
            json!({"openapi":"3.1.0","paths":{"/x":{"$ref":"https://evil.example/paths"}}}),
            json!({"openapi":"3.1.0","paths":{"/x":{"$ref":"#/paths/~1x"}}}),
            json!({"openapi":"3.1.0","paths":{"/x/{id}":{"get":{"responses":{}}}}}),
            json!({"openapi":"3.1.0","paths":{"/../private":{"get":{"responses":{}}}}}),
            json!({"openapi":"3.1.0","paths":{"/x":{"get":{"parameters":[{"name":"x","in":"query","schema":{"$ref":"https://evil.example/schema"}}]}}}}),
        ] {
            assert!(
                parse(
                    &serde_json::to_vec(&document).unwrap(),
                    "https://docs.example/spec",
                    None,
                    "api"
                )
                .is_err()
            );
        }
        assert!(
            parse(
                &vec![b'x'; DOCUMENT_BYTES + 1],
                "https://docs.example/spec",
                None,
                "api"
            )
            .is_err()
        );
        assert!(
            parse(
                b"openapi: 3.1.0\npaths: {}\npaths: {}",
                "https://docs.example/spec",
                None,
                "api"
            )
            .is_err()
        );
        let root = json!({"components":{"schemas":{"Loop":{"$ref":"#/components/schemas/Loop"}}}});
        let value =
            schema::resolve(&root, &json!({"$ref":"#/components/schemas/Loop"}), &mut 10).unwrap();
        // Preserve recursive validation rather than replacing cycles with an unconstrained schema.
        assert_eq!(value["$defs"]["ref0"]["$ref"], "#/$defs/ref0");
        assert!(
            schema::resolve(
                &Value::Null,
                &json!({"type":"object","properties":{"a":{"type":"string"}}}),
                &mut 2
            )
            .is_err()
        );
    }
    #[test]
    fn effective_operation_servers_are_pinned_or_require_an_explicit_override() {
        let mut document = json!({"openapi":"3.1.0","servers":[{"url":"https://root.example"}],"paths":{
            "/x":{"servers":[{"url":"https://path.example/v1"}],"get":{"servers":[{"url":"https://operation.example/v2"}],"responses":{}}}
        }});
        let parse_document = |document: &Value, pinned: Option<&str>| {
            parse(
                &serde_json::to_vec(document).unwrap(),
                "https://docs.example/spec",
                pinned,
                "api",
            )
        };
        assert_eq!(
            parse_document(&document, None).unwrap().0,
            "https://operation.example/v2"
        );
        document["paths"]["/y"] = json!({"get":{"responses":{}}});
        assert!(parse_document(&document, None).is_err());
        let (_, operations) =
            parse_document(&document, Some("https://pinned.example/api")).unwrap();
        assert!(
            operations
                .iter()
                .all(|operation| operation.server.as_str() == "https://pinned.example/api")
        );
    }
    #[test]
    fn raw_results_do_not_allocate_a_tree_and_handle_text_and_empty_bodies() {
        let entries = r#"{"value":1},"#.repeat(1000);
        let document = format!("{{\"items\":[{}]}}", entries.trim_end_matches(','));
        let mut bytes = Vec::with_capacity(document.len() + 4096);
        bytes.extend_from_slice(document.as_bytes());
        let mut output = None;
        let allocations = allocation_counter::measure(|| {
            output = Some(decode_result(bytes).unwrap());
        });
        assert!(
            allocations.count_total <= 3,
            "{} allocations exceed the measured three-allocation OpenAPI result budget",
            allocations.count_total
        );
        assert_eq!(output.unwrap().get(), document);
        assert_eq!(decode_result(Vec::new()).unwrap().get(), "null");
        assert_eq!(
            decode_result(b"plain text".to_vec()).unwrap().get(),
            r#""plain text""#
        );
        assert!(decode_result(vec![255]).is_err());
    }
    #[test]
    fn parameter_styles_encode_values_and_reject_header_injection_and_dot_segments() {
        use parameters::*;
        let parameter = Parameter {
            argument: "v".into(),
            name: "v".into(),
            location: Location::Query,
            style: "deepObject".into(),
            explode: true,
            allow_reserved: false,
        };
        assert_eq!(
            parameter.query(&json!({"a":"1","b":"2"})).unwrap(),
            vec![("v[a]".into(), "1".into()), ("v[b]".into(), "2".into())]
        );
        let parameter = Parameter {
            location: Location::Path,
            style: "matrix".into(),
            ..parameter
        };
        assert_eq!(
            parameter.path(&json!(["x/y", "a,b"])).unwrap(),
            ";v=x%2Fy;v=a%2Cb"
        );
        assert_eq!(parameter.path(&json!({"a":"x"})).unwrap(), ";a=x");
        assert!(parameter.path(&json!("..")).is_err());
        let operations = compile(
            json!({"openapi":"3.1.0","paths":{"/x":{"get":{"parameters":[
                {"name":"Host","in":"header","schema":{"type":"string"}},
                {"name":"X-Test","in":"header","schema":{"type":"string"}}
            ]}}}}),
        );
        assert!(
            operations[0].definition.input_schema["properties"]
                .get("Host")
                .is_none()
        );
        assert!(
            operations[0]
                .request(
                    &reqwest::Client::new(),
                    json!({"X-Test":"safe\r\nHost: evil"}).as_object().unwrap()
                )
                .is_err()
        );
        assert_eq!(percent("x&y=+#", true), "x%26y%3D%2B%23");
    }
}
