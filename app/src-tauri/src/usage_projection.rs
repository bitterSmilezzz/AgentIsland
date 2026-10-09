//! Select the existing ledger fields while validating the whole JSON record.
//! Unselected strings/containers are visited, never retained in a Value tree.
use serde::de::{self, DeserializeSeed, MapAccess, SeqAccess, Visitor};
use serde::Deserialize;
use serde_json::{Map, Value};
use std::borrow::Cow;
use std::fmt;

#[derive(Clone, Copy)]
enum Node {
    UsageRoot,
    ContextRoot,
    ContextPayload,
    Message,
    Payload,
    Response,
    Model,
    Provider,
    Usage,
    RawUsage,
    Creation,
    Details,
    DetailRow,
    Text,
    Number,
    Timestamp,
    Skip,
}
impl Node {
    fn field(self, key: &str) -> Self {
        use Node::*;
        match (self, key) {
            (ContextRoot, "type") => Text,
            (ContextRoot, "payload") => ContextPayload,
            (ContextPayload, "id" | "session_id" | "turn_id" | "model_provider" | "model") => Text,
            (
                UsageRoot,
                "type" | "id" | "uuid" | "requestId" | "requestModelName" | "status" | "role",
            ) => Text,
            (
                UsageRoot,
                "timestamp" | "completedAt" | "completed_at" | "createdAt" | "created_at",
            ) => Timestamp,
            (UsageRoot, "message") => Message,
            (UsageRoot, "payload") => Payload,
            (UsageRoot, "response") => Response,
            (UsageRoot, "model") => Model,
            (UsageRoot, "providerData") => Provider,
            (Message, "id" | "model" | "status" | "role") => Text,
            (Message | Payload | Response | Provider, "usage") => Usage,
            (Payload, "thread_id" | "session_id" | "turn_id" | "response_id") => Text,
            (Response, "responseId") => Text,
            (Model, "modelId") => Text,
            (Provider, "requestModelId" | "requestModelName" | "model") => Text,
            (Provider, "rawUsage") => RawUsage,
            (
                Usage,
                "input_tokens"
                | "inputTokens"
                | "output_tokens"
                | "outputTokens"
                | "cached_input_tokens"
                | "cacheReadTokens"
                | "cache_read_input_tokens"
                | "cache_creation_input_tokens"
                | "cachedInputTokens",
            ) => Number,
            (Usage, "cache_creation") => Creation,
            (Usage, "input_details" | "inputDetails" | "inputTokensDetails") => Details,
            (
                RawUsage,
                "prompt_tokens"
                | "completion_tokens"
                | "prompt_cache_hit_tokens"
                | "cache_read_input_tokens"
                | "prompt_cache_miss_tokens",
            ) => Number,
            (RawUsage, "prompt_tokens_details") => Details,
            (Creation, "ephemeral_5m_input_tokens" | "ephemeral_1h_input_tokens") => Number,
            (Details | DetailRow, "cached_tokens" | "cachedTokens") => Number,
            _ => Skip,
        }
    }
    fn retains_map(self) -> bool {
        !matches!(
            self,
            Self::Skip | Self::Text | Self::Number | Self::Timestamp
        )
    }
}

// Borrow ordinary keys; escaped keys still decode exactly, including duplicates.
struct Key<'a>(Cow<'a, str>);
impl<'de> Deserialize<'de> for Key<'de> {
    fn deserialize<D: de::Deserializer<'de>>(decoder: D) -> Result<Self, D::Error> {
        struct KeyVisitor;
        impl<'de> Visitor<'de> for KeyVisitor {
            type Value = Key<'de>;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("an object key")
            }
            fn visit_borrowed_str<E: de::Error>(self, s: &'de str) -> Result<Self::Value, E> {
                Ok(Key(Cow::Borrowed(s)))
            }
            fn visit_str<E: de::Error>(self, s: &str) -> Result<Self::Value, E> {
                Ok(Key(Cow::Owned(s.into())))
            }
            fn visit_string<E: de::Error>(self, s: String) -> Result<Self::Value, E> {
                Ok(Key(Cow::Owned(s)))
            }
        }
        decoder.deserialize_str(KeyVisitor)
    }
}
struct Projection(Node);
impl<'de> DeserializeSeed<'de> for Projection {
    type Value = Value;
    fn deserialize<D: de::Deserializer<'de>>(self, decoder: D) -> Result<Value, D::Error> {
        decoder.deserialize_any(self)
    }
}
impl<'de> Visitor<'de> for Projection {
    type Value = Value;
    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("a JSON value")
    }
    fn visit_unit<E: de::Error>(self) -> Result<Value, E> {
        Ok(Value::Null)
    }
    fn visit_bool<E: de::Error>(self, _: bool) -> Result<Value, E> {
        Ok(Value::Null)
    }
    fn visit_i64<E: de::Error>(self, n: i64) -> Result<Value, E> {
        Ok(if matches!(self.0, Node::Number | Node::Timestamp) {
            n.into()
        } else {
            Value::Null
        })
    }
    fn visit_u64<E: de::Error>(self, n: u64) -> Result<Value, E> {
        Ok(if matches!(self.0, Node::Number | Node::Timestamp) {
            n.into()
        } else {
            Value::Null
        })
    }
    fn visit_f64<E: de::Error>(self, _: f64) -> Result<Value, E> {
        Ok(Value::Null)
    }
    fn visit_str<E: de::Error>(self, s: &str) -> Result<Value, E> {
        Ok(if matches!(self.0, Node::Text | Node::Timestamp) {
            Value::String(s.into())
        } else {
            Value::Null
        })
    }
    fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<Value, M::Error> {
        let retain = self.0.retains_map();
        let mut kept = Map::new();
        while let Some(Key(key)) = map.next_key()? {
            let child = self.0.field(&key);
            // Unlike IgnoredAny, this visitor still validates discarded numbers,
            // escaped strings and nesting with the same serde_json decoder.
            let value = map.next_value_seed(Projection(child))?;
            if !matches!(child, Node::Skip) {
                kept.insert(key.into_owned(), value);
            }
        }
        Ok(if retain {
            Value::Object(kept)
        } else {
            Value::Null
        })
    }
    fn visit_seq<S: SeqAccess<'de>>(self, mut seq: S) -> Result<Value, S::Error> {
        let details = matches!(self.0, Node::Details);
        let child = if details { Node::DetailRow } else { Node::Skip };
        let mut kept = Vec::new();
        while let Some(value) = seq.next_element_seed(Projection(child))? {
            if details {
                kept.push(value);
            }
        }
        Ok(if details {
            Value::Array(kept)
        } else {
            Value::Null
        })
    }
}
fn parse(line: &str, node: Node) -> serde_json::Result<Value> {
    let mut decoder = serde_json::Deserializer::from_str(line);
    let value = Projection(node).deserialize(&mut decoder)?;
    decoder.end()?;
    Ok(value)
}
pub fn usage(line: &str) -> serde_json::Result<Value> {
    parse(line, Node::UsageRoot)
}
pub fn context(line: &str) -> serde_json::Result<Value> {
    parse(line, Node::ContextRoot)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn discarded_values_still_require_valid_numbers_unicode_and_nesting() {
        for bad in [
            r#"{"message":{"usage":{"input_tokens":5}},"body":1e400}"#,
            r#"{"message":{"usage":{"input_tokens":5}},"body":"\uD800"}"#,
            r#"{"message":{"usage":{"input_tokens":5}},"body":[1,]}"#,
            r#"{"message":{"usage":{"input_tokens":5}}}{}"#,
        ] {
            assert!(serde_json::from_str::<Value>(bad).is_err());
            assert!(usage(bad).is_err());
        }
        let deep = format!("{{\"body\":{}0{}}}", "[".repeat(130), "]".repeat(130));
        assert!(serde_json::from_str::<Value>(&deep).is_err());
        assert!(usage(&deep).is_err());
    }
    #[test]
    fn escaped_keys_duplicate_fields_and_null_presence_preserve_precedence() {
        let doc = usage(r#"{"messa\u0067e":{"usage":{"input_tokens":2,"input_tokens":3}},"message":null,"payload":{"response_id":"r","usage":{"input_tokens":9}},"providerData":null}"#).unwrap();
        assert_eq!(doc["message"], Value::Null);
        assert!(doc.get("providerData").is_some());
        assert_eq!(
            usage(r#"{"messa\u0067e":{"usage":{"input_tokens":2,"input_tokens":3}}}"#).unwrap()
                ["message"]["usage"]["input_tokens"],
            3
        );
    }
}
