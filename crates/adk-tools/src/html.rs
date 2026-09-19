use html5ever::{
    tendril::StrTendril,
    tokenizer::{
        BufferQueue, TagKind, Token, TokenSink, TokenSinkResult, Tokenizer, states::RawKind,
    },
};
use std::cell::RefCell;
#[derive(Default)]
struct Text {
    output: String,
    pending: String,
    skipped: String,
    depth: usize,
    anchors: Vec<String>,
}
impl Text {
    fn flush(&mut self) {
        if self.depth == 0 && !self.pending.trim().is_empty() {
            self.output.push_str(self.pending.trim());
            self.output.push(' ');
        }
        self.pending.clear();
    }
}
#[derive(Default)]
struct Sink(RefCell<Text>);
impl TokenSink for Sink {
    type Handle = ();
    fn process_token(&self, token: Token, _: u64) -> TokenSinkResult<()> {
        let mut state = self.0.borrow_mut();
        match token {
            Token::CharacterTokens(text) => {
                state.pending.push_str(&text);
                return TokenSinkResult::Continue;
            }
            Token::NullCharacterToken => {
                // The SDK tokenizer preserves literal NUL text.
                state.pending.push('\0');
                return TokenSinkResult::Continue;
            }
            Token::ParseError(_) => return TokenSinkResult::Continue,
            _ => state.flush(),
        }
        let Token::TagToken(tag) = token else {
            return TokenSinkResult::Continue;
        };
        let name = tag.name.as_ref();
        let start = tag.kind == TagKind::StartTag;
        let result = if start {
            match name {
                "script" => TokenSinkResult::RawData(RawKind::ScriptData),
                "style" | "xmp" | "iframe" | "noembed" | "noframes" | "noscript" => {
                    TokenSinkResult::RawData(RawKind::Rawtext)
                }
                "title" | "textarea" => TokenSinkResult::RawData(RawKind::Rcdata),
                "plaintext" => TokenSinkResult::Plaintext,
                _ => TokenSinkResult::Continue,
            }
        } else {
            TokenSinkResult::Continue
        };
        if state.depth > 0 {
            if name == state.skipped {
                if start {
                    state.depth += 1;
                } else {
                    state.depth -= 1;
                }
            }
            return result;
        }
        if start && matches!(name, "script" | "style" | "nav" | "footer") {
            state.skipped = name.into();
            state.depth = 1;
            return result;
        }
        if start {
            match name {
                "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => state.output.push_str("\n\n# "),
                "p" | "div" | "section" | "article" => state.output.push_str("\n\n"),
                "br" => state.output.push('\n'),
                "li" => state.output.push_str("\n- "),
                "ul" | "ol" => state.output.push('\n'),
                "code" | "pre" => state.output.push('`'),
                "a" => {
                    let href = tag
                        .attrs
                        .iter()
                        .find(|attribute| attribute.name.local == html5ever::local_name!("href"))
                        .map(|attribute| attribute.value.to_string())
                        .unwrap_or_default();
                    if !href.is_empty() {
                        state.output.push('[');
                    }
                    state.anchors.push(href);
                }
                _ => {}
            }
        } else {
            match name {
                "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "p" | "div" | "section" | "article" => {
                    state.output.push('\n')
                }
                "code" | "pre" => state.output.push('`'),
                "a" => {
                    if let Some(href) = state.anchors.pop().filter(|href| !href.is_empty()) {
                        state.output.push_str(&format!("]({href}) "));
                    }
                }
                _ => {}
            }
        }
        result
    }
}
pub(crate) fn to_text(html: &str) -> String {
    let input = BufferQueue::default();
    input.push_back(StrTendril::from(html));
    let tokenizer = Tokenizer::new(Sink::default(), Default::default());
    let _ = tokenizer.feed(&input);
    tokenizer.end();
    let mut text = tokenizer.sink.0.into_inner();
    text.flush();
    let mut lines = Vec::new();
    let mut blanks = 0;
    for line in text.output.split('\n') {
        let line = line.trim();
        if line.is_empty() {
            blanks += 1;
            if blanks <= 1 {
                lines.push("");
            }
        } else {
            blanks = 0;
            lines.push(line);
        }
    }
    lines.join("\n").trim().to_owned()
}
