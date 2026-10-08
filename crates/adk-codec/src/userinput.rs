//! Public SDK user-input helpers. These inspect records; they do not execute tools.
use crate::dto::{RunItem, RunItemType};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

fn go_utf8(mut input: &[u8]) -> String {
    let mut text = String::new();
    while !input.is_empty() {
        match std::str::from_utf8(input) {
            Ok(rest) => {
                text.push_str(rest);
                break;
            }
            Err(error) => {
                let (valid, rest) = input.split_at(error.valid_up_to());
                text.push_str(std::str::from_utf8(valid).expect("validated prefix"));
                let invalid = error.error_len().unwrap_or(rest.len());
                text.extend(std::iter::repeat_n('\u{fffd}', invalid));
                input = &rest[invalid..];
            }
        }
    }
    text
}

fn decode<T: serde::de::DeserializeOwned>(input: &[u8]) -> Result<T, serde_json::Error> {
    let mut bytes = go_utf8(input).into_bytes();
    let mut inside = false;
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'"' {
            inside = !inside;
        } else if inside && bytes[i] == b'\\' {
            if bytes.get(i + 1) == Some(&b'u') {
                let hex = |start: usize| {
                    bytes
                        .get(start..start + 4)
                        .and_then(|digits| std::str::from_utf8(digits).ok())
                        .and_then(|digits| u16::from_str_radix(digits, 16).ok())
                };
                if let Some(code) = hex(i + 2) {
                    if (0xd800..=0xdbff).contains(&code)
                        && bytes.get(i + 6..i + 8) == Some(b"\\u")
                        && hex(i + 8).is_some_and(|low| (0xdc00..=0xdfff).contains(&low))
                    {
                        i += 12;
                        continue;
                    }
                    // encoding/json replaces unpaired surrogates instead of rejecting the string.
                    if (0xd800..=0xdfff).contains(&code) {
                        bytes[i + 2..i + 6].copy_from_slice(b"fffd");
                    }
                    i += 6;
                    continue;
                }
            }
            i += 2;
            continue;
        }
        i += 1;
    }
    serde_json::from_slice(&bytes)
}

// Go retains backing-array elements across repeated nonempty slice fields.
#[derive(Default)]
struct GoSlice<T> {
    slots: Vec<T>,
    len: Option<usize>,
}
impl<T> GoSlice<T> {
    fn into_values(self) -> Option<Vec<T>> {
        self.len
            .map(|len| self.slots.into_iter().take(len).collect())
    }
}
struct ElementSeed<'a, T>(&'a mut T);
impl<'de, T: Deserialize<'de>> serde::de::DeserializeSeed<'de> for ElementSeed<'_, T> {
    type Value = ();
    fn deserialize<D: serde::Deserializer<'de>>(self, d: D) -> Result<(), D::Error> {
        d.deserialize_option(self)
    }
}
impl<'de, T: Deserialize<'de>> serde::de::Visitor<'de> for ElementSeed<'_, T> {
    type Value = ();
    fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("a nullable SDK field")
    }
    fn visit_none<E: serde::de::Error>(self) -> Result<(), E> {
        Ok(())
    }
    fn visit_some<D: serde::Deserializer<'de>>(self, d: D) -> Result<(), D::Error> {
        T::deserialize_in_place(d, self.0)
    }
}
impl<'de, T: Deserialize<'de> + Default> Deserialize<'de> for GoSlice<T> {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let mut out = Self::default();
        Self::deserialize_in_place(d, &mut out)?;
        Ok(out)
    }
    fn deserialize_in_place<D: serde::Deserializer<'de>>(
        d: D,
        out: &mut Self,
    ) -> Result<(), D::Error> {
        struct Slice<'a, T>(&'a mut GoSlice<T>);
        impl<'de, T: Deserialize<'de> + Default> serde::de::Visitor<'de> for Slice<'_, T> {
            type Value = ();
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("an SDK slice or null")
            }
            fn visit_unit<E: serde::de::Error>(self) -> Result<(), E> {
                *self.0 = GoSlice::default();
                Ok(())
            }
            fn visit_seq<A: serde::de::SeqAccess<'de>>(self, mut seq: A) -> Result<(), A::Error> {
                let mut len = 0;
                loop {
                    if len == self.0.slots.len() {
                        self.0.slots.push(T::default());
                    }
                    if seq
                        .next_element_seed(ElementSeed(&mut self.0.slots[len]))?
                        .is_none()
                    {
                        break;
                    }
                    len += 1;
                }
                if len == 0 {
                    self.0.slots.clear();
                }
                self.0.len = Some(len);
                Ok(())
            }
        }
        d.deserialize_any(Slice(out))
    }
}

macro_rules! read_field {
    ($map:ident, $out:expr, scalar) => {
        $map.next_value_seed(ElementSeed(&mut $out))?;
    };
    ($map:ident, $out:expr, slice) => {
        struct SliceSeed<'a, T>(&'a mut GoSlice<T>);
        impl<'de, T: Deserialize<'de> + Default> serde::de::DeserializeSeed<'de> for SliceSeed<'_, T> {
            type Value = ();
            fn deserialize<D: serde::Deserializer<'de>>(self, d: D) -> Result<(), D::Error> {
                GoSlice::deserialize_in_place(d, self.0)
            }
        }
        $map.next_value_seed(SliceSeed(&mut $out))?;
    };
}

macro_rules! go_object {
    ($ty:ident, $( $key:literal => $field:ident : $kind:ident ),* $(,)?) => {
        impl<'de> Deserialize<'de> for $ty {
            fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                let mut out = Self::default();
                Self::deserialize_in_place(d, &mut out)?;
                Ok(out)
            }
            fn deserialize_in_place<D: serde::Deserializer<'de>>(d: D, out: &mut Self) -> Result<(), D::Error> {
                struct Object<'a>(&'a mut $ty);
                impl<'de> serde::de::Visitor<'de> for Object<'_> {
                    type Value = ();
                    fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                        f.write_str("an SDK input object")
                    }
                    fn visit_unit<E: serde::de::Error>(self) -> Result<(), E> { Ok(()) }
                    fn visit_map<M: serde::de::MapAccess<'de>>(self, mut map: M) -> Result<(), M::Error> {
                        let out = self.0;
                        while let Some(key) = map.next_key::<String>()? {
                            let key: String = key.chars().map(|c| match c {
                                'K' => 'k', 'ſ' => 's', _ => c.to_ascii_lowercase(),
                            }).collect();
                            match key.as_str() {
                                $( $key => { read_field!(map, out.$field, $kind); } )*
                                _ => { map.next_value::<serde::de::IgnoredAny>()?; }
                            }
                        }
                        Ok(())
                    }
                }
                d.deserialize_any(Object(out))
            }
        }
    };
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, JsonSchema)]
pub struct QuickAction {
    pub id: String,
    pub label: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub style: String,
}

go_object!(QuickAction, "id" => id: scalar, "label" => label: scalar, "style" => style: scalar);

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UserInputPause {
    pub requested: bool,
    pub plan_review: bool,
    pub question: String,
    pub actions: Option<Vec<u8>>,
}

/// `None` preserves a nil Go slice (`null`); an empty slice encodes as `[]`.
pub fn marshal_quick_actions(actions: Option<&[QuickAction]>) -> Vec<u8> {
    crate::snapshots::to_go_json(&actions).expect("quick actions contain only strings")
}

pub fn extract_ask_user_choices(input: &[u8]) -> Option<Vec<u8>> {
    #[derive(Default)]
    struct Input {
        choices: GoSlice<String>,
    }
    go_object!(Input, "choices" => choices: slice);
    let input = decode::<Option<Input>>(input).ok()??;
    let choices = input.choices.into_values().unwrap_or_default();
    if choices.is_empty() {
        return None;
    }
    let actions: Vec<_> = choices
        .into_iter()
        .enumerate()
        .map(|(i, label)| QuickAction {
            id: format!("choice_{i}"),
            label,
            style: if i == 0 { "primary" } else { "secondary" }.into(),
        })
        .collect();
    Some(marshal_quick_actions(Some(&actions)))
}

pub fn extract_present_plan_data(input: &[u8]) -> (String, Option<Vec<u8>>) {
    #[derive(Default)]
    struct Input {
        summary: String,
        actions: GoSlice<QuickAction>,
        recommended: String,
    }
    go_object!(Input, "summary" => summary: scalar, "actions" => actions: slice, "recommended" => recommended: scalar);
    let Ok(input) = decode::<Option<Input>>(input) else {
        return (String::new(), None);
    };
    let input = input.unwrap_or_default();
    let actions = input.actions.into_values();
    (
        input.summary,
        Some(marshal_quick_actions(actions.as_deref())),
    )
}

pub fn extract_ask_user_question(input: &[u8]) -> String {
    #[derive(Default)]
    struct Choice {
        label: String,
        description: String,
    }
    #[derive(Default)]
    struct Question {
        question: String,
        header: String,
        options: GoSlice<Choice>,
    }
    #[derive(Default)]
    struct Structured {
        questions: GoSlice<Question>,
    }
    #[derive(Default)]
    struct Simple {
        question: String,
    }
    go_object!(Choice, "label" => label: scalar, "description" => description: scalar);
    go_object!(Question, "question" => question: scalar, "header" => header: scalar, "options" => options: slice);
    go_object!(Structured, "questions" => questions: slice);
    go_object!(Simple, "question" => question: scalar);
    if let Ok(Some(value)) = decode::<Option<Structured>>(input) {
        let questions = value.questions.into_values().unwrap_or_default();
        if !questions.is_empty() {
            return questions
                .into_iter()
                .map(|q| {
                    let mut text = q.question;
                    let options = q.options.into_values().unwrap_or_default();
                    if !options.is_empty() {
                        text.push_str(" Options: ");
                        text.push_str(
                            &options
                                .into_iter()
                                .map(|o| {
                                    if o.description.is_empty() {
                                        o.label
                                    } else {
                                        format!("{}: {}", o.label, o.description)
                                    }
                                })
                                .collect::<Vec<_>>()
                                .join(" | "),
                        );
                    }
                    text
                })
                .collect::<Vec<_>>()
                .join("\n");
        }
    }
    if let Ok(Some(value)) = decode::<Option<Simple>>(input) {
        if !value.question.is_empty() {
            return value.question;
        }
    }
    go_utf8(input)
}

pub fn detect_user_input_pause(items: &[RunItem], final_text: &str) -> UserInputPause {
    for item in items {
        if item.kind != RunItemType(1) {
            continue;
        }
        let Some(call) = &item.tool_call else {
            continue;
        };
        let input = match &call.input {
            crate::dto::RawJson::Missing => Vec::new(),
            crate::dto::RawJson::Encoded(raw) => raw.get().as_bytes().to_vec(),
            crate::dto::RawJson::Present(value) => {
                crate::snapshots::to_go_json(value).expect("JSON value")
            }
        };
        let (mut question, actions, plan_review) = match call.name.as_str() {
            "AskUserQuestion" => (
                extract_ask_user_question(&input),
                extract_ask_user_choices(&input),
                false,
            ),
            "present_plan" => {
                let (question, actions) = extract_present_plan_data(&input);
                (question, actions, true)
            }
            _ => continue,
        };
        let trimmed = question.trim();
        if question.is_empty()
            || (trimmed.starts_with('{') && trimmed.ends_with('}') && !final_text.trim().is_empty())
        {
            question = final_text.trim().into();
        }
        if question.is_empty() {
            question = "The agent needs your input to continue.".into();
        }
        return UserInputPause {
            requested: true,
            plan_review,
            question,
            actions,
        };
    }
    UserInputPause::default()
}

pub fn build_auto_turn_cap_prompt(max_turns: i64) -> String {
    format!("Auto mode global turn cap ({max_turns}) reached.")
}
