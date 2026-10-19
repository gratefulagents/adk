//! Schema-bounded raw transport for analysis snapshots.
//! Tokens are local to one call; ordinary Go-style decoding retains duplicate
//! pointer/object and slice-slot semantics before raw values are restored.
use super::{append_raw, fold_key, normalize_strings, raw_end, string_end};
use crate::dto::{RawJson, ResponseSnapshot, RunItemSnapshot};
use crate::snapshots::RequestSnapshot;

#[derive(Clone, Copy)]
pub(super) enum Node {
    Request,
    Response,
    Items,
    Tools,
    Calls,
    Tool,
    Schema,
    Item,
    Input,
    Raw,
    Other,
}

fn child(node: Node, key: &str) -> Node {
    match (node, key) {
        (Node::Request, "input_items") | (Node::Response, "items") => Node::Items,
        (Node::Request, "tools") => Node::Tools,
        (Node::Request, "output_schema") => Node::Schema,
        (Node::Response, "tool_calls") => Node::Calls,
        (Node::Response, "raw") | (Node::Tool, "input_schema") | (Node::Schema, "schema") => {
            Node::Raw
        }
        (Node::Item, "tool_call" | "tool_approval") => Node::Input,
        (Node::Input, "input") => Node::Raw,
        _ => Node::Other,
    }
}

fn skip_space(input: &[u8], mut i: usize) -> usize {
    while input.get(i).is_some_and(u8::is_ascii_whitespace) {
        i += 1;
    }
    i
}

// Callers validate syntax first. Recursion follows this finite schema only;
// unknown values and raw fragments are scanned, not recursively traversed.
fn rewrite(
    input: &[u8],
    node: Node,
    raw: &mut impl FnMut(&[u8]) -> Result<Vec<u8>, serde_json::Error>,
) -> Result<Vec<u8>, serde_json::Error> {
    let input = input.trim_ascii();
    if matches!(node, Node::Raw) {
        return raw(input);
    }
    let element = match node {
        Node::Items => Some(Node::Item),
        Node::Tools => Some(Node::Tool),
        Node::Calls => Some(Node::Input),
        _ => None,
    };
    if let Some(element) = element {
        if input.first() != Some(&b'[') {
            return Ok(normalize_strings(input));
        }
        let mut out = vec![b'['];
        let mut i = skip_space(input, 1);
        while input[i] != b']' {
            let end = raw_end(input, i);
            out.extend(rewrite(&input[i..end], element, raw)?);
            i = skip_space(input, end);
            if input[i] == b',' {
                out.push(b',');
                i = skip_space(input, i + 1);
            }
        }
        out.push(b']');
        return Ok(out);
    }
    if matches!(node, Node::Other) || input.first() != Some(&b'{') {
        return Ok(normalize_strings(input));
    }
    let mut out = vec![b'{'];
    let mut i = skip_space(input, 1);
    while input[i] != b'}' {
        let end = string_end(input, i);
        let key = normalize_strings(&input[i..end]);
        let name: String = serde_json::from_slice(&key)?;
        out.extend(key);
        out.push(b':');
        i = skip_space(input, skip_space(input, end) + 1);
        let end = raw_end(input, i);
        out.extend(rewrite(&input[i..end], child(node, &fold_key(&name)), raw)?);
        i = skip_space(input, end);
        if input[i] == b',' {
            out.push(b',');
            i = skip_space(input, i + 1);
        }
    }
    out.push(b'}');
    Ok(out)
}

fn visit_raw(
    item: &mut RunItemSnapshot,
    mut visit: impl FnMut(&mut RawJson) -> Result<(), serde_json::Error>,
) -> Result<(), serde_json::Error> {
    if let Some(call) = &mut item.tool_call {
        visit(&mut call.input)?;
    }
    if let Some(approval) = &mut item.tool_approval {
        visit(&mut approval.input)?;
    }
    Ok(())
}

fn original_raw(bytes: &[u8]) -> Result<RawJson, serde_json::Error> {
    match std::str::from_utf8(bytes) {
        Ok(text) => Ok(RawJson::Encoded(serde_json::value::RawValue::from_string(
            text.to_owned(),
        )?)),
        Err(_) => Ok(RawJson::EncodedBytes(bytes.to_vec())),
    }
}

pub(super) trait Document: Clone + serde::Serialize + serde::de::DeserializeOwned {
    const NODE: Node;
    fn visit_raw(
        &mut self,
        visit: &mut impl FnMut(&mut RawJson) -> Result<(), serde_json::Error>,
    ) -> Result<(), serde_json::Error>;
}

impl Document for RunItemSnapshot {
    const NODE: Node = Node::Item;
    fn visit_raw(
        &mut self,
        visit: &mut impl FnMut(&mut RawJson) -> Result<(), serde_json::Error>,
    ) -> Result<(), serde_json::Error> {
        visit_raw(self, visit)
    }
}

impl Document for RequestSnapshot {
    const NODE: Node = Node::Request;
    fn visit_raw(
        &mut self,
        visit: &mut impl FnMut(&mut RawJson) -> Result<(), serde_json::Error>,
    ) -> Result<(), serde_json::Error> {
        for item in self.input_items.iter_mut().flatten() {
            visit_raw(item, &mut *visit)?;
        }
        for tool in self.tools.iter_mut().flatten() {
            visit(&mut tool.input_schema)?;
        }
        if let Some(schema) = &mut self.output_schema {
            visit(&mut schema.schema)?;
        }
        Ok(())
    }
}

impl Document for ResponseSnapshot {
    const NODE: Node = Node::Response;
    fn visit_raw(
        &mut self,
        visit: &mut impl FnMut(&mut RawJson) -> Result<(), serde_json::Error>,
    ) -> Result<(), serde_json::Error> {
        for item in self.items.iter_mut().flatten() {
            visit_raw(item, &mut *visit)?;
        }
        for call in self.tool_calls.iter_mut().flatten() {
            visit(&mut call.input)?;
        }
        visit(&mut self.raw)
    }
}

pub(super) fn decode<T: Document>(input: &[u8]) -> Result<T, serde_json::Error> {
    // Validate every occurrence before replacing raw values, including values
    // that a later null or duplicate key will discard.
    let normalized = normalize_strings(input);
    let _: &serde_json::value::RawValue = serde_json::from_slice(&normalized)?;
    let mut slots = Vec::new();
    let rewritten = rewrite(input, T::NODE, &mut |bytes| {
        let index = slots.len();
        slots.push(bytes.to_vec());
        Ok(index.to_string().into_bytes())
    })?;
    let mut item: T = serde_json::from_slice(&rewritten)?;
    item.visit_raw(&mut |raw| {
        if raw.bytes().is_empty() {
            return Ok(());
        }
        let index: usize = serde_json::from_slice(&raw.bytes())?;
        let bytes = slots
            .get(index)
            .ok_or_else(|| serde::de::Error::custom("invalid snapshot raw slot"))?;
        *raw = original_raw(bytes)?;
        Ok(())
    })?;
    Ok(item)
}

pub(super) fn encode<T: Document>(item: &T) -> Result<Vec<u8>, serde_json::Error> {
    let mut item = item.clone();
    let mut slots = Vec::new();
    item.visit_raw(&mut |raw| {
        if raw.bytes().is_empty() {
            *raw = RawJson::Missing;
        } else {
            let mut bytes = Vec::new();
            append_raw(&mut bytes, raw)?;
            let index = slots.len();
            slots.push(bytes);
            *raw = original_raw(index.to_string().as_bytes())?;
        }
        Ok(())
    })?;
    let encoded = crate::snapshots::to_go_json(&item)?;
    rewrite(&encoded, T::NODE, &mut |bytes| {
        let index: usize = serde_json::from_slice(bytes)?;
        slots
            .get(index)
            .cloned()
            .ok_or_else(|| serde::de::Error::custom("invalid snapshot raw slot"))
    })
}
