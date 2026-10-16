//! Bounded decoding of untyped SDK JSON values.

use serde::{Deserialize, Deserializer};
use serde_json::{Map, Value, value::RawValue};

/// Decode with Go string replacement and finite float64 number semantics.
///
/// Duplicate object entries are validated before later entries replace them.
/// Returned numbers use Go's JSON spelling, including negative zero. This retains
/// the native 127-container nesting limit; it does not claim Go's deeper limit.
pub fn decode(input: &[u8]) -> Result<Value, serde_json::Error> {
    let normalized = crate::go_decode::normalize_strings(input);
    let raw: &RawValue = serde_json::from_slice(&normalized)?;
    decode_value(raw, 0)
}

fn decode_value(raw: &RawValue, depth: usize) -> Result<Value, serde_json::Error> {
    let text = raw.get();
    let first = text.as_bytes()[0];
    if matches!(first, b'{' | b'[') && depth == 127 {
        return Err(serde::de::Error::custom("recursion limit exceeded"));
    }
    match first {
        b'{' => {
            let entries: Object<'_> = serde_json::from_str(text)?;
            let mut object = Map::new();
            for (key, raw) in entries.0 {
                object.insert(key, decode_value(raw, depth + 1)?);
            }
            Ok(Value::Object(object))
        }
        b'[' => {
            let entries: Vec<&RawValue> = serde_json::from_str(text)?;
            entries
                .into_iter()
                .map(|raw| decode_value(raw, depth + 1))
                .collect::<Result<Vec<_>, _>>()
                .map(Value::Array)
        }
        b'"' => serde_json::from_str(text).map(Value::String),
        b't' => Ok(Value::Bool(true)),
        b'f' => Ok(Value::Bool(false)),
        b'n' => Ok(Value::Null),
        _ => {
            let number: f64 = serde_json::from_str(text)?;
            let encoded = crate::snapshots::to_go_json(&number)?;
            // Re-parsing this validated spelling would turn "-0" into "0".
            let spelling = String::from_utf8(encoded).expect("JSON float spelling is ASCII");
            Ok(Value::Number(serde_json::Number::from_string_unchecked(
                spelling,
            )))
        }
    }
}

struct Object<'a>(Vec<(String, &'a RawValue)>);
impl<'de> Deserialize<'de> for Object<'de> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Visitor;
        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = Object<'de>;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("a JSON object")
            }
            fn visit_map<M: serde::de::MapAccess<'de>>(
                self,
                mut map: M,
            ) -> Result<Self::Value, M::Error> {
                let mut entries = Vec::new();
                while let Some(entry) = map.next_entry()? {
                    entries.push(entry);
                }
                Ok(Object(entries))
            }
        }
        deserializer.deserialize_map(Visitor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nesting_bound_matches_existing_native_value_decoder() {
        for depth in [0, 1, 126, 127, 128, 129] {
            for (open, close) in [("[", "]"), ("{\"k\":", "}")] {
                let input = format!("{}0{}", open.repeat(depth), close.repeat(depth));
                assert_eq!(
                    decode(input.as_bytes()).is_ok(),
                    serde_json::from_str::<Value>(&input).is_ok(),
                    "depth={depth} open={open}"
                );
            }
        }
    }

    #[test]
    fn duplicate_overflow_is_not_hidden_by_a_later_valid_value() {
        for input in [br#"{"n":1e400,"n":1}"#.as_slice(), br#"[1,1e400]"#] {
            assert!(decode(input).is_err());
        }
        assert_eq!(
            decode(br#"{"n":2,"n":1}"#).unwrap(),
            serde_json::json!({"n":1})
        );
    }

    #[test]
    fn private_serde_marker_names_are_ordinary_object_keys() {
        for key in [
            "$serde_json::private::Number",
            "$serde_json::private::RawValue",
        ] {
            let input = format!("{{\"{key}\":\"1\"}}");
            let decoded = decode(input.as_bytes()).unwrap();
            assert_eq!(decoded.as_object().unwrap()[key].as_str(), Some("1"));
        }
    }
}
