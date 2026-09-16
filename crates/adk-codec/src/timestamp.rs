use schemars::{JsonSchema, Schema, SchemaGenerator};
use serde::{Deserialize, Deserializer, Serialize};
use std::borrow::Cow;

/// RFC3339 timestamps at nanosecond precision, without a floating-point conversion.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct GoTimestamp(String);

impl Default for GoTimestamp {
    fn default() -> Self {
        Self("0001-01-01T00:00:00Z".into())
    }
}

impl GoTimestamp {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    fn parse(s: &str) -> Option<Self> {
        let bytes = s.as_bytes();
        if bytes.len() < 20 || !s.is_ascii() {
            return None;
        }
        for (i, expected) in [(4, b'-'), (7, b'-'), (10, b'T'), (13, b':'), (16, b':')] {
            if bytes[i] != expected {
                return None;
            }
        }
        let number = |start: usize, end: usize| -> Option<u32> {
            let digits = &s[start..end];
            if !digits.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            digits.parse().ok()
        };
        let year = number(0, 4)?;
        let month = number(5, 7)?;
        let day = number(8, 10)?;
        let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
        let days = match month {
            2 => {
                if leap {
                    29
                } else {
                    28
                }
            }
            4 | 6 | 9 | 11 => 30,
            1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
            _ => return None,
        };
        if day == 0
            || day > days
            || number(11, 13)? > 23
            || number(14, 16)? > 59
            || number(17, 19)? > 59
        {
            return None;
        }
        let mut end = 19;
        let mut fraction = "";
        if bytes[end] == b'.' {
            end += 1;
            let start = end;
            while end < bytes.len() && bytes[end].is_ascii_digit() {
                end += 1;
            }
            if end == start || end - start > 9 {
                return None;
            }
            fraction = s[start..end].trim_end_matches('0');
        }
        let zone = &s[end..];
        if zone != "Z" {
            if zone.len() != 6
                || !matches!(zone.as_bytes()[0], b'+' | b'-')
                || zone.as_bytes()[3] != b':'
            {
                return None;
            }
            if number(end + 1, end + 3)? > 23 || number(end + 4, end + 6)? > 59 {
                return None;
            }
        }
        let zone = if matches!(zone, "+00:00" | "-00:00") {
            "Z"
        } else {
            zone
        };
        let mut canonical = s[..19].to_owned();
        if !fraction.is_empty() {
            canonical.push('.');
            canonical.push_str(fraction);
        }
        canonical.push_str(zone);
        Some(Self(canonical))
    }
}

impl<'de> Deserialize<'de> for GoTimestamp {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        match Option::<String>::deserialize(d)? {
            None => Ok(Self::default()),
            Some(s) => Self::parse(&s).ok_or_else(|| {
                serde::de::Error::custom(
                    "expected RFC3339 timestamp with at most nine fractional digits",
                )
            }),
        }
    }
}

impl JsonSchema for GoTimestamp {
    fn schema_name() -> Cow<'static, str> {
        "GoTimestamp".into()
    }
    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        schemars::json_schema!({"type": "string", "format": "date-time"})
    }
}
