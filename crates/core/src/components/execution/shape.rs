use serde::Deserialize;
use serde::de::{DeserializeSeed, Deserializer, IgnoredAny, MapAccess, SeqAccess, Visitor};
use std::fmt;

pub(super) fn summarize(raw: &str) -> String {
    let mut nodes = 0;
    Shape {
        depth: 0,
        nodes: &mut nodes,
    }
    .deserialize(&mut serde_json::Deserializer::from_str(raw))
    .unwrap_or_else(|_| "unknown".into())
}

struct Shape<'a> {
    depth: usize,
    nodes: &'a mut usize,
}
impl<'de> DeserializeSeed<'de> for Shape<'_> {
    type Value = String;
    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<String, D::Error> {
        if self.depth >= 6 || *self.nodes >= 32 {
            IgnoredAny::deserialize(deserializer)?;
            return Ok("…".into());
        }
        *self.nodes += 1;
        deserializer.deserialize_any(self)
    }
}
impl<'de> Visitor<'de> for Shape<'_> {
    type Value = String;
    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("JSON")
    }
    fn visit_bool<E>(self, _: bool) -> Result<String, E> {
        Ok("bool".into())
    }
    fn visit_i64<E>(self, _: i64) -> Result<String, E> {
        Ok("int".into())
    }
    fn visit_u64<E>(self, _: u64) -> Result<String, E> {
        Ok("int".into())
    }
    fn visit_f64<E>(self, _: f64) -> Result<String, E> {
        Ok("float".into())
    }
    fn visit_str<E>(self, _: &str) -> Result<String, E> {
        Ok("str".into())
    }
    fn visit_unit<E>(self) -> Result<String, E> {
        Ok("None".into())
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<String, A::Error> {
        let first = seq.next_element_seed(Shape {
            depth: self.depth + 1,
            nodes: self.nodes,
        })?;
        let Some(first) = first else {
            return Ok("[]".into());
        };
        let mut count = 1;
        while seq.next_element::<IgnoredAny>()?.is_some() {
            count += 1;
        }
        Ok(format!("[{count}× (first: {first})]"))
    }
    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<String, A::Error> {
        let mut fields = Vec::new();
        while fields.len() < 6 && *self.nodes < 32 {
            let Some(key) = map.next_key_seed(Key)? else {
                return Ok(format!("{{{}}}", fields.join(",")));
            };
            let value = map.next_value_seed(Shape {
                depth: self.depth + 1,
                nodes: self.nodes,
            })?;
            fields.push(format!("{key}:{value}"));
        }
        let mut omitted = false;
        while map.next_key::<IgnoredAny>()?.is_some() {
            map.next_value::<IgnoredAny>()?;
            omitted = true;
        }
        if omitted {
            fields.push("…".into());
        }
        Ok(format!("{{{}}}", fields.join(",")))
    }
}

struct Key;
impl<'de> DeserializeSeed<'de> for Key {
    type Value = String;
    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<String, D::Error> {
        deserializer.deserialize_str(self)
    }
}
impl Visitor<'_> for Key {
    type Value = String;
    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("object key")
    }
    fn visit_str<E>(self, key: &str) -> Result<String, E> {
        let short: String = key.chars().take(32).collect();
        Ok(serde_json::to_string(&short).unwrap_or_else(|_| "?".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shape_keeps_keys_and_array_counts_without_values() {
        let rows = vec![serde_json::json!({"id":"private-id","title":"private-title"}); 36];
        let raw =
            serde_json::json!({"data":{"items":rows,"nextCursor":"private-cursor"}}).to_string();
        let shape = summarize(&raw);
        assert_eq!(
            shape,
            r#"{"data":{"items":[36× (first: {"id":str,"title":str})],"nextCursor":str}}"#
        );
        assert!(!shape.contains("private"));
        assert_eq!(summarize("[]"), "[]");
        assert_eq!(summarize("null"), "None");
    }
    #[test]
    fn shape_bounds_depth_keys_and_field_counts() {
        let mut value = serde_json::json!({"x".repeat(1000): [1,2,3]});
        for _ in 0..10 {
            value = serde_json::json!({"nested":value});
        }
        let shape = summarize(&value.to_string());
        assert!(shape.contains('…'));
        assert!(shape.len() < 100);
        let mut fields = serde_json::Map::new();
        for i in 0..100 {
            fields.insert(format!("key{i}"), serde_json::json!(true));
        }
        assert!(summarize(&serde_json::Value::Object(fields).to_string()).len() < 120);
    }
}
