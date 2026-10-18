//! Governed SDK role labels and selection metadata, not execution authorization.

use crate::dto::null_default;
use serde::{Deserialize, Serialize};
use std::borrow::Cow;

#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Kind(#[serde(deserialize_with = "null_default")] pub Cow<'static, str>);

impl Kind {
    pub const EXPLORE: Self = Self(Cow::Borrowed("explore"));
    pub const ANALYST: Self = Self(Cow::Borrowed("analyst"));
    pub const PLANNER: Self = Self(Cow::Borrowed("planner"));
    pub const ARCHITECT: Self = Self(Cow::Borrowed("architect"));
    pub const DEBUGGER: Self = Self(Cow::Borrowed("debugger"));
    pub const EXECUTOR: Self = Self(Cow::Borrowed("executor"));
    pub const CRITIC: Self = Self(Cow::Borrowed("critic"));
    pub const SECURITY_REVIEWER: Self = Self(Cow::Borrowed("security-reviewer"));
    pub const DEPENDENCY_EXPERT: Self = Self(Cow::Borrowed("dependency-expert"));
    pub const DESIGNER: Self = Self(Cow::Borrowed("designer"));
    pub const WRITER: Self = Self(Cow::Borrowed("writer"));
}
impl std::ops::Deref for Kind {
    type Target = str;
    fn deref(&self) -> &str {
        self.0.as_ref()
    }
}
impl std::fmt::Display for Kind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(&self.0, f)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Definition {
    pub kind: Kind,
    #[serde(rename = "readOnly")]
    pub read_only: bool,
    #[serde(skip_serializing_if = "no_aliases")]
    pub aliases: Option<Vec<String>>,
}

fn no_aliases(value: &Option<Vec<String>>) -> bool {
    value.as_ref().is_none_or(Vec::is_empty)
}

const BUILT_INS: &[(Kind, bool, &[&str])] = &[
    (Kind::EXPLORE, true, &["Explore"]),
    (Kind::ANALYST, false, &[]),
    (Kind::PLANNER, true, &["Plan"]),
    (Kind::ARCHITECT, false, &[]),
    (Kind::DEBUGGER, false, &[]),
    (Kind::EXECUTOR, false, &["general-purpose"]),
    (Kind::CRITIC, false, &[]),
    (Kind::SECURITY_REVIEWER, true, &[]),
    (Kind::DEPENDENCY_EXPERT, false, &[]),
    (Kind::DESIGNER, false, &[]),
    (Kind::WRITER, false, &[]),
];

pub fn built_in_kinds() -> Vec<Kind> {
    BUILT_INS.iter().map(|(kind, _, _)| kind.clone()).collect()
}

/// Return owned metadata; changing aliases cannot mutate the governed catalog.
pub fn definitions() -> Vec<Definition> {
    BUILT_INS
        .iter()
        .map(|(kind, read_only, aliases)| Definition {
            kind: kind.clone(),
            read_only: *read_only,
            aliases: (!aliases.is_empty())
                .then(|| aliases.iter().map(|alias| (*alias).to_owned()).collect()),
        })
        .collect()
}

pub fn parse_kind(value: &str) -> Option<Kind> {
    resolve_kind(value).map(|definition| definition.kind)
}

pub fn resolve_kind(value: &str) -> Option<Definition> {
    let normalized: String = value
        .trim_matches(|c| {
            matches!(
                c,
                '\u{0009}'..='\u{000d}' | '\u{0020}' | '\u{0085}' | '\u{00a0}'
                    | '\u{1680}' | '\u{2000}'..='\u{200a}' | '\u{2028}' | '\u{2029}'
                    | '\u{202f}' | '\u{205f}' | '\u{3000}'
            )
        })
        .chars()
        .map(|c| match c {
            // Only these non-ASCII simple-lowercase mappings can reach ASCII labels.
            '\u{0130}' => 'i',
            '\u{212a}' => 'k',
            _ => c.to_ascii_lowercase(),
        })
        .collect();
    definitions().into_iter().find(|definition| {
        definition.kind.0 == normalized
            || definition.aliases.as_ref().is_some_and(|aliases| {
                aliases
                    .iter()
                    .any(|alias| alias.eq_ignore_ascii_case(&normalized))
            })
    })
}

/// Empty lists allow every label; otherwise only resolved canonical kinds match.
/// The `kind` argument itself is intentionally not normalized.
pub fn is_allowed(kind: &Kind, allowed: &[String]) -> bool {
    allowed.is_empty()
        || allowed
            .iter()
            .filter_map(|candidate| resolve_kind(candidate))
            .any(|definition| definition.kind == *kind)
}
