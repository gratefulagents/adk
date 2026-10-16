use serde::{Deserialize, de::MapAccess};
use serde_json::value::RawValue;

type Result<T> = std::result::Result<T, ()>;

trait Merge {
    fn merge(&mut self, raw: &RawValue) -> Result<()>;
}

struct Fields(Vec<(String, Box<RawValue>)>);
impl<'de> Deserialize<'de> for Fields {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        struct Visitor;
        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = Fields;
            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("a model metadata object")
            }
            fn visit_map<M: MapAccess<'de>>(
                self,
                mut map: M,
            ) -> std::result::Result<Fields, M::Error> {
                let mut fields = Vec::new();
                while let Some(field) = map.next_entry()? {
                    fields.push(field);
                }
                Ok(Fields(fields))
            }
        }
        deserializer.deserialize_map(Visitor)
    }
}

impl Merge for Option<String> {
    fn merge(&mut self, raw: &RawValue) -> Result<()> {
        if raw.get() != "null" {
            *self = Some(serde_json::from_str(raw.get()).map_err(|_| ())?);
        }
        Ok(())
    }
}
impl Merge for Option<i64> {
    fn merge(&mut self, raw: &RawValue) -> Result<()> {
        if raw.get() != "null" {
            // Go accepts integer -0 but rejects fractional and exponent notation.
            *self = Some(raw.get().parse().map_err(|_| ())?);
        }
        Ok(())
    }
}

#[derive(Default)]
pub(super) struct Slice<T> {
    values: Vec<T>,
    len: usize,
}
impl<T> Slice<T> {
    pub(super) fn is_empty(&self) -> bool {
        self.len == 0
    }
    pub(super) fn into_iter(self) -> impl Iterator<Item = T> {
        self.values.into_iter().take(self.len)
    }
}
impl<T: Merge + Default> Merge for Slice<T> {
    fn merge(&mut self, raw: &RawValue) -> Result<()> {
        let entries: Option<Vec<&RawValue>> = serde_json::from_str(raw.get()).map_err(|_| ())?;
        let entries = entries.unwrap_or_default();
        if entries.is_empty() {
            self.values.clear();
        }
        for (index, entry) in entries.iter().enumerate() {
            if index == self.values.len() {
                self.values.push(T::default());
            }
            self.values[index].merge(entry)?;
        }
        // encoding/json reuses backing-array elements even after a shorter duplicate array.
        self.len = entries.len();
        Ok(())
    }
}

macro_rules! object {
    ($name:ident { $($field:ident: $kind:ty),* $(,)? }) => {
        #[derive(Default)]
        pub(super) struct $name {
            $(pub(super) $field: $kind),*
        }
        impl Merge for $name {
            fn merge(&mut self, raw: &RawValue) -> Result<()> {
                if raw.get() == "null" {
                    return Ok(());
                }
                let Fields(fields) = serde_json::from_str(raw.get()).map_err(|_| ())?;
                for (name, value) in fields {
                    let folded: String = name.chars().map(|c| match c {
                        'ſ' => 's',
                        'K' => 'k',
                        _ => c.to_ascii_lowercase(),
                    }).collect();
                    match folded.as_str() {
                        $(stringify!($field) => self.$field.merge(&value)?,)*
                        _ => {}
                    }
                }
                Ok(())
            }
        }
    };
}

object!(Limits {
    max_context_window_tokens: Option<i64>,
    max_prompt_tokens: Option<i64>,
    max_output_tokens: Option<i64>,
});
object!(Capabilities { limits: Limits });
object!(Effort { effort: Option<String> });
object!(Upgrade { model: Option<String> });
impl Merge for Option<Upgrade> {
    fn merge(&mut self, raw: &RawValue) -> Result<()> {
        if raw.get() == "null" {
            *self = None;
            Ok(())
        } else {
            self.get_or_insert_with(Upgrade::default).merge(raw)
        }
    }
}
object!(CodexEntry {
    slug: Option<String>,
    context_window: Option<i64>,
    max_context_window: Option<i64>,
    auto_compact_token_limit: Option<i64>,
    effective_context_window_percent: Option<i64>,
    display_name: Option<String>,
    description: Option<String>,
    visibility: Option<String>,
    priority: Option<i64>,
    default_reasoning_level: Option<String>,
    supported_reasoning_levels: Slice<Effort>,
    upgrade: Option<Upgrade>,
});
object!(DataEntry {
    id: Option<String>,
    capabilities: Capabilities,
});
object!(Catalog {
    models: Slice<CodexEntry>,
    data: Slice<DataEntry>,
});

pub(super) fn decode(body: &[u8]) -> Result<Catalog> {
    let mut normalized = Vec::with_capacity(body.len());
    for chunk in body.utf8_chunks() {
        normalized.extend_from_slice(chunk.valid().as_bytes());
        // Go replaces each invalid byte, not each maximal invalid UTF-8 sequence.
        for _ in chunk.invalid() {
            normalized.extend_from_slice("\u{fffd}".as_bytes());
        }
    }
    let mut quoted = false;
    let mut index = 0;
    while index < normalized.len() {
        if normalized[index] == b'"' {
            quoted = !quoted;
        } else if quoted && normalized[index] == b'\\' {
            if normalized.get(index + 1) == Some(&b'u') {
                let hex = |start| {
                    normalized
                        .get(start..start + 4)
                        .and_then(|digits| std::str::from_utf8(digits).ok())
                        .and_then(|digits| u16::from_str_radix(digits, 16).ok())
                };
                if let Some(code) = hex(index + 2) {
                    if (0xd800..=0xdbff).contains(&code)
                        && normalized.get(index + 6..index + 8) == Some(b"\\u")
                        && hex(index + 8).is_some_and(|low| (0xdc00..=0xdfff).contains(&low))
                    {
                        index += 12;
                        continue;
                    }
                    if (0xd800..=0xdfff).contains(&code) {
                        normalized[index + 2..index + 6].copy_from_slice(b"fffd");
                    }
                    index += 6;
                    continue;
                }
            }
            index += 2;
            continue;
        }
        index += 1;
    }
    let raw: Box<RawValue> = serde_json::from_slice(&normalized).map_err(|_| ())?;
    let mut depth = 0;
    let mut quoted = false;
    let mut escaped = false;
    for byte in raw.get().bytes() {
        if escaped {
            escaped = false;
        } else if quoted && byte == b'\\' {
            escaped = true;
        } else if byte == b'"' {
            quoted = !quoted;
        } else if !quoted {
            match byte {
                b'[' | b'{' => {
                    depth += 1;
                    if depth > 10000 {
                        return Err(());
                    }
                }
                b']' | b'}' => depth -= 1,
                _ => {}
            }
        }
    }
    let mut catalog = Catalog::default();
    catalog.merge(&raw)?;
    Ok(catalog)
}
