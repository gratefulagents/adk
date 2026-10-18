//! models.dev catalog data, independent of fetching and cache ownership.
use crate::go_decode::{ElementSeed, fold_key, go_object, normalize_strings, read_field};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct ModelLimits {
    #[serde(rename = "context")]
    pub context_tokens: i64,
    #[serde(rename = "output")]
    pub output_tokens: i64,
}
go_object!(ModelLimits,
    "context" => context_tokens: integer,
    "output" => output_tokens: integer
);

pub type Catalog = BTreeMap<String, BTreeMap<String, ModelLimits>>;

#[derive(Default)]
struct Provider {
    models: Option<BTreeMap<String, Model>>,
}
go_object!(Provider, "models" => models: map);

#[derive(Default)]
struct Model {
    limit: ModelLimits,
}
go_object!(Model, "limit" => limit: scalar);

/// Decode the catalog, retaining only nonempty model IDs with positive windows.
/// Normalized key collisions are resolved in source-key order, rather than Go's
/// unspecified map iteration order.
pub fn parse_catalog(body: &[u8]) -> Result<Catalog, serde_json::Error> {
    crate::payload_decode::check_depth(body)?;
    let raw: Option<BTreeMap<String, Provider>> = serde_json::from_slice(&normalize_strings(body))?;
    let mut catalog = Catalog::new();
    for (provider, entry) in raw.into_iter().flatten() {
        let models: BTreeMap<_, _> = entry
            .models
            .into_iter()
            .flatten()
            .filter_map(|(id, model)| {
                let id = normalize_id(&id);
                (!id.is_empty() && model.limit.context_tokens > 0).then_some((id, model.limit))
            })
            .collect();
        if !models.is_empty() {
            catalog.insert(normalize_id(&provider), models);
        }
    }
    if catalog.is_empty() {
        return Err(serde::de::Error::custom(
            "models.dev catalog contained no models",
        ));
    }
    Ok(catalog)
}

fn normalize_id(value: &str) -> String {
    value
        .trim_matches(|c| {
            matches!(c,
                '\u{0009}'..='\u{000d}' | '\u{0020}' | '\u{0085}' | '\u{00a0}' |
                '\u{1680}' | '\u{2000}'..='\u{200a}' | '\u{2028}' | '\u{2029}' |
                '\u{202f}' | '\u{205f}' | '\u{3000}'
            )
        })
        .chars()
        .map(|c| match c {
            // Go baseline uses Unicode15; these mappings first appeared in Unicode16.
            '\u{1c89}'
            | '\u{a7cb}'
            | '\u{a7cc}'
            | '\u{a7da}'
            | '\u{a7dc}'
            | '\u{10d50}'..='\u{10d65}' => c,
            _ => c.to_lowercase().next().expect("lowercase is nonempty"),
        })
        .collect()
}

/// Resolve provider aliases and prefer the longest compatible catalog model ID.
pub fn lookup(catalog: &Catalog, provider: &str, model: &str) -> Option<ModelLimits> {
    let provider = normalize_id(provider);
    let provider = match provider.as_str() {
        "copilot" => "github-copilot",
        "anthropic-oauth" => "anthropic",
        value => value,
    };
    let models = catalog.get(provider)?;
    let model = normalize_id(model);
    let model = normalize_id(model.split_once('/').map_or(model.as_str(), |(_, id)| id));
    if model.is_empty() {
        return None;
    }
    if let Some(limits) = models
        .get(&model)
        .filter(|limits| limits.context_tokens > 0)
    {
        return Some(*limits);
    }
    models
        .iter()
        .filter(|(id, limits)| {
            !id.is_empty()
                && limits.context_tokens > 0
                && (model.starts_with(id.as_str()) || id.starts_with(&model))
        })
        .max_by_key(|(id, _)| id.len())
        .map(|(_, limits)| *limits)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_parser_matches_pinned_go() {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../fixtures/modelsdev/observations.json"
        ))
        .unwrap();
        for case in fixture["parsed"].as_array().unwrap() {
            let result = parse_catalog(case["input"].as_str().unwrap().as_bytes());
            assert_eq!(result.is_ok(), case["ok"].as_bool().unwrap(), "{case}");
            if let Ok(catalog) = result {
                assert_eq!(
                    serde_json::to_value(catalog).unwrap(),
                    case["catalog"],
                    "{case}"
                );
            }
        }
        let catalog: Catalog = serde_json::from_value(fixture["catalog"].clone()).unwrap();
        for case in fixture["resolved"].as_array().unwrap() {
            let result = lookup(
                &catalog,
                case["provider"].as_str().unwrap(),
                case["model"].as_str().unwrap(),
            );
            assert_eq!(result.is_some(), case["ok"].as_bool().unwrap(), "{case}");
            if let Some(limits) = result {
                assert_eq!(
                    serde_json::to_value(limits).unwrap(),
                    case["limits"],
                    "{case}"
                );
            }
        }
    }

    #[test]
    fn catalog_case_normalization_matches_every_go_unicode_scalar() {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../fixtures/modelsdev/observations.json"
        ))
        .unwrap();
        let mut mismatches = Vec::new();
        for c in (0..=0x10ffff).filter_map(char::from_u32) {
            let value = c.to_string();
            let expected = fixture["lower"]
                .get(&value)
                .and_then(|v| v.as_str())
                .unwrap_or(&value);
            if normalize_id(&format!("x{c}x")) != format!("x{expected}x") {
                mismatches.push(format!("U+{:X}", u32::from(c)));
            }
        }
        assert!(mismatches.is_empty(), "{mismatches:?}");
    }
}
