use super::Error;
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;

pub(super) fn resolve(
    document: &Value,
    schema: &Value,
    budget: &mut usize,
) -> Result<Value, Error> {
    resolve_for(document, schema, budget, false)
}
pub(super) fn input(document: &Value, schema: &Value, budget: &mut usize) -> Result<Value, Error> {
    resolve_for(document, schema, budget, true)
}
fn resolve_for(
    document: &Value,
    schema: &Value,
    budget: &mut usize,
    request: bool,
) -> Result<Value, Error> {
    let mut resolver = Resolver {
        document,
        request,
        references: BTreeMap::new(),
        definitions: Map::new(),
        budget,
    };
    let mut result = resolver.schema(schema, 0)?;
    if !resolver.definitions.is_empty() {
        result
            .as_object_mut()
            .ok_or(Error::Invalid("referenced schema must be an object"))?
            .insert("$defs".into(), Value::Object(resolver.definitions));
    }
    Ok(result)
}

struct Resolver<'a, 'b> {
    document: &'a Value,
    request: bool,
    references: BTreeMap<String, String>,
    definitions: Map<String, Value>,
    budget: &'b mut usize,
}
impl Resolver<'_, '_> {
    fn node(&mut self, depth: usize) -> Result<(), Error> {
        if depth > 64 || *self.budget == 0 {
            return Err(Error::Invalid(
                "OpenAPI schemas exceed the complexity limit",
            ));
        }
        *self.budget -= 1;
        Ok(())
    }
    fn data(&mut self, value: &Value, depth: usize) -> Result<Value, Error> {
        self.node(depth)?;
        match value {
            Value::Object(fields) => fields
                .iter()
                .map(|(key, value)| Ok((key.clone(), self.data(value, depth + 1)?)))
                .collect::<Result<Map<_, _>, Error>>()
                .map(Value::Object),
            Value::Array(values) => values
                .iter()
                .map(|v| self.data(v, depth + 1))
                .collect::<Result<Vec<_>, _>>()
                .map(Value::Array),
            _ => Ok(value.clone()),
        }
    }
    fn read_only<'a>(&'a self, mut value: &'a Value) -> bool {
        for _ in 0..64 {
            if value.get("readOnly") == Some(&Value::Bool(true)) {
                return true;
            }
            let Some(reference) = value
                .get("$ref")
                .and_then(Value::as_str)
                .and_then(|v| v.strip_prefix('#'))
            else {
                return false;
            };
            let Some(target) = self.document.pointer(reference) else {
                return false;
            };
            value = target;
        }
        false
    }
    fn schema(&mut self, value: &Value, depth: usize) -> Result<Value, Error> {
        self.node(depth)?;
        if value.is_boolean() {
            return Ok(value.clone());
        }
        let fields = value
            .as_object()
            .ok_or(Error::Invalid("invalid OpenAPI JSON Schema"))?;
        let mut result = Map::new();
        for (key, value) in fields {
            let value = match key.as_str() {
                "$id" | "nullable" => continue,
                "$dynamicRef" => {
                    return Err(Error::Invalid(
                        "dynamic JSON Schema references are unsupported",
                    ));
                }
                "$ref" => {
                    let reference = value
                        .as_str()
                        .ok_or(Error::Invalid("invalid schema reference"))?;
                    let pointer = reference
                        .strip_prefix('#')
                        .filter(|p| p.starts_with('/'))
                        .ok_or(Error::Invalid(
                            "only local JSON-pointer references are supported",
                        ))?;
                    let name = if let Some(name) = self.references.get(reference) {
                        name.clone()
                    } else {
                        let target = self
                            .document
                            .pointer(pointer)
                            .ok_or(Error::Invalid("schema reference does not exist"))?;
                        let name = format!("ref{}", self.references.len());
                        self.references.insert(reference.into(), name.clone());
                        self.definitions.insert(name.clone(), Value::Bool(true));
                        let resolved = self.schema(target, depth + 1)?;
                        self.definitions.insert(name.clone(), resolved);
                        name
                    };
                    Value::String(format!("#/$defs/{name}"))
                }
                "properties" | "patternProperties" | "dependentSchemas" | "$defs"
                | "definitions" => {
                    let fields = value
                        .as_object()
                        .ok_or(Error::Invalid("invalid schema map"))?;
                    Value::Object(
                        fields
                            .iter()
                            .map(|(key, value)| Ok((key.clone(), self.schema(value, depth + 1)?)))
                            .collect::<Result<Map<_, _>, Error>>()?,
                    )
                }
                "allOf" | "anyOf" | "oneOf" | "prefixItems" => Value::Array(
                    value
                        .as_array()
                        .ok_or(Error::Invalid("invalid schema alternatives"))?
                        .iter()
                        .map(|v| self.schema(v, depth + 1))
                        .collect::<Result<Vec<_>, _>>()?,
                ),
                "items"
                | "additionalItems"
                | "additionalProperties"
                | "unevaluatedProperties"
                | "unevaluatedItems"
                | "contains"
                | "if"
                | "then"
                | "else"
                | "not"
                | "propertyNames"
                | "contentSchema" => self.schema(value, depth + 1)?,
                _ => self.data(value, depth + 1)?,
            };
            result.insert(key.clone(), value);
        }
        for (exclusive, inclusive) in [
            ("exclusiveMinimum", "minimum"),
            ("exclusiveMaximum", "maximum"),
        ] {
            if let Some(Value::Bool(enabled)) = result.get(exclusive) {
                let enabled = *enabled;
                result.remove(exclusive);
                if enabled {
                    let boundary = result
                        .remove(inclusive)
                        .ok_or(Error::Invalid("exclusive schema bound lacks its value"))?;
                    result.insert(exclusive.into(), boundary);
                }
            }
        }
        if self.request {
            if let (Some(properties), Some(Value::Array(required))) = (
                fields.get("properties").and_then(Value::as_object),
                result.get_mut("required"),
            ) {
                required.retain(|name| {
                    name.as_str()
                        .and_then(|name| properties.get(name))
                        .is_none_or(|property| !self.read_only(property))
                });
            }
        }
        let result = Value::Object(result);
        Ok(if fields.get("nullable") == Some(&Value::Bool(true)) {
            json!({"anyOf":[result,{"type":"null"}]})
        } else {
            result
        })
    }
}
