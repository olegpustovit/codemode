use super::Error;
use serde_json::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub(super) enum Location {
    Path,
    Query,
    Header,
}
#[derive(Clone, serde::Serialize)]
pub(super) struct Parameter {
    pub argument: String,
    pub name: String,
    pub location: Location,
    pub style: String,
    pub explode: bool,
    pub allow_reserved: bool,
}
impl Parameter {
    pub fn validate(&self) -> Result<(), Error> {
        let valid = match self.location {
            Location::Path => matches!(self.style.as_str(), "simple" | "label" | "matrix"),
            Location::Query => matches!(
                self.style.as_str(),
                "form" | "spaceDelimited" | "pipeDelimited" | "deepObject"
            ),
            Location::Header => self.style == "simple",
        };
        if valid {
            Ok(())
        } else {
            Err(Error::Invalid("unsupported OpenAPI parameter style"))
        }
    }
    pub fn query(&self, value: &Value) -> Result<Vec<(String, String)>, Error> {
        Ok(match value {
            Value::Array(values) => {
                let values = values.iter().map(scalar).collect::<Result<Vec<_>, _>>()?;
                if self.style == "deepObject" {
                    return Err(Error::Arguments("deepObject needs an object"));
                }
                if self.explode {
                    values.into_iter().map(|v| (self.name.clone(), v)).collect()
                } else {
                    vec![(
                        self.name.clone(),
                        values.join(match self.style.as_str() {
                            "spaceDelimited" => " ",
                            "pipeDelimited" => "|",
                            _ => ",",
                        }),
                    )]
                }
            }
            Value::Object(fields) => {
                let values = fields
                    .iter()
                    .map(|(k, v)| Ok((k.clone(), scalar(v)?)))
                    .collect::<Result<Vec<_>, Error>>()?;
                if self.style == "deepObject" {
                    values
                        .into_iter()
                        .map(|(k, v)| (format!("{}[{k}]", self.name), v))
                        .collect()
                } else if self.explode {
                    values
                } else {
                    vec![(
                        self.name.clone(),
                        values
                            .into_iter()
                            .flat_map(|(k, v)| [k, v])
                            .collect::<Vec<_>>()
                            .join(","),
                    )]
                }
            }
            _ => vec![(self.name.clone(), scalar(value)?)],
        })
    }
    pub fn path(&self, value: &Value) -> Result<String, Error> {
        let encode = |v: &Value| -> Result<String, Error> {
            let raw = scalar(v)?;
            if raw == "." || raw == ".." {
                return Err(Error::Arguments("path arguments cannot be dot segments"));
            }
            Ok(percent(&raw, false))
        };
        let name = percent(&self.name, false);
        let delimiter = if self.style == "label" && self.explode {
            "."
        } else {
            ","
        };
        let value = match value {
            Value::Array(values) => {
                let values = values.iter().map(encode).collect::<Result<Vec<_>, _>>()?;
                if self.style == "matrix" && self.explode {
                    return Ok(values
                        .into_iter()
                        .map(|v| format!(";{name}={v}"))
                        .collect::<Vec<_>>()
                        .join(""));
                }
                values.join(delimiter)
            }
            Value::Object(fields) => {
                let mut values = Vec::new();
                for (k, v) in fields {
                    let k = percent(k, false);
                    let v = encode(v)?;
                    if self.style == "matrix" && self.explode {
                        values.push(format!(";{k}={v}"));
                    } else if self.explode {
                        values.push(format!("{k}={v}"));
                    } else {
                        values.extend([k, v]);
                    }
                }
                if self.style == "matrix" && self.explode {
                    return Ok(values.join(""));
                }
                values.join(delimiter)
            }
            _ => encode(value)?,
        };
        Ok(match self.style.as_str() {
            "label" => format!(".{value}"),
            "matrix" => format!(";{name}={value}"),
            _ => value,
        })
    }
    pub fn header(&self, value: &Value) -> Result<String, Error> {
        Ok(match value {
            Value::Array(values) => values
                .iter()
                .map(scalar)
                .collect::<Result<Vec<_>, _>>()?
                .join(","),
            Value::Object(fields) => {
                let mut values = Vec::new();
                for (k, v) in fields {
                    let v = scalar(v)?;
                    if self.explode {
                        values.push(format!("{k}={v}"));
                    } else {
                        values.extend([k.clone(), v]);
                    }
                }
                values.join(",")
            }
            _ => scalar(value)?,
        })
    }
}
fn scalar(value: &Value) -> Result<String, Error> {
    match value {
        Value::String(value) => Ok(value.clone()),
        Value::Null => Ok(String::new()),
        Value::Bool(_) | Value::Number(_) => Ok(value.to_string()),
        _ => Err(Error::Arguments(
            "parameter collections must contain scalar values",
        )),
    }
}
pub(super) fn percent(value: &str, reserved: bool) -> String {
    use std::fmt::Write;
    let mut output = String::new();
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric()
            || b"-._~".contains(&byte)
            || (reserved && b":/?@!$'()*;,".contains(&byte))
        {
            output.push(char::from(byte));
        } else {
            let _ = write!(output, "%{byte:02X}");
        }
    }
    output
}
pub(crate) fn controlled_header(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "host"
            | "authorization"
            | "proxy-authorization"
            | "cookie"
            | "connection"
            | "content-length"
            | "transfer-encoding"
            | "te"
            | "trailer"
            | "upgrade"
            | "expect"
    ) || name.to_ascii_lowercase().starts_with("proxy-")
}
