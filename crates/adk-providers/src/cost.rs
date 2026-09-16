//! Baseline standard-tier USD prices, derived from SDK internal/{openai,anthropic}/cost.go.
//! Not current billing quotes; gateways may report different negotiated prices.
use adk_core::Usage;

/// Unknown OpenAI model prices remain unknown, rather than silently free.
pub fn openai(model: &str, usage: &Usage) -> Option<f64> {
    let model = model.trim();
    let model = model.strip_prefix("openai/").unwrap_or(model).trim();
    let long = usage.input_tokens > 272_000;
    let (input, read, write, output) = match model {
        "gpt-4" | "gpt-4.1" => (2.0, 0.5, 0.0, 8.0),
        "gpt-4.1-mini" => (0.4, 0.1, 0.0, 1.6),
        "gpt-4.1-nano" => (0.1, 0.025, 0.0, 0.4),
        "gpt-4o" => (2.5, 1.25, 0.0, 10.0),
        "gpt-4o-mini" => (0.15, 0.075, 0.0, 0.6),
        "gpt-5" | "gpt-5-codex" | "gpt-5.1" | "gpt-5.1-codex" | "gpt-5.1-codex-max" => {
            (1.25, 0.125, 0.0, 10.0)
        }
        "gpt-5-mini" | "gpt-5.1-codex-mini" => (0.25, 0.025, 0.0, 2.0),
        "gpt-5-nano" => (0.05, 0.005, 0.0, 0.4),
        "gpt-5.2" | "gpt-5.2-codex" | "gpt-5.3-codex" => (1.75, 0.175, 0.0, 14.0),
        "gpt-5.3-codex-spark" => (0.4, 0.0, 0.0, 1.6),
        "gpt-5.4" => (2.5, 0.25, 0.0, 15.0),
        "gpt-5.4-mini" => (0.75, 0.075, 0.0, 4.5),
        "gpt-5.4-nano" => (0.2, 0.02, 0.0, 1.25),
        "gpt-5.5" => (5.0, 0.5, 0.0, 30.0),
        "gpt-6-astra" if long => (20.0, 2.0, 25.0, 75.0),
        "gpt-6-astra" => (10.0, 1.0, 12.5, 50.0),
        "gpt-5.6" | "gpt-5.6-sol" | "daybreak-blue-latest" | "gpt-daybreak-blue-latest" if long => {
            (8.0, 0.8, 10.0, 30.0)
        }
        "gpt-5.6" | "gpt-5.6-sol" | "daybreak-blue-latest" | "gpt-daybreak-blue-latest" => {
            (4.0, 0.4, 5.0, 20.0)
        }
        "gpt-5.6-cyber" | "daybreak-red-latest" | "gpt-daybreak-red-latest" => {
            (12.5, 1.25, 15.625, 75.0)
        }
        "gpt-5.6-terra" => (2.5, 0.25, 3.125, 15.0),
        "gpt-5.6-luna" => (1.0, 0.1, 1.25, 6.0),
        _ => return None,
    };
    let reads = usage.cache_read_tokens.min(usage.input_tokens);
    let writes = usage.cache_creation_tokens.min(usage.input_tokens - reads);
    let uncached = usage.input_tokens
        - if read > 0.0 { reads } else { 0 }
        - if write > 0.0 { writes } else { 0 };
    Some(
        (uncached as f64 * input
            + reads as f64 * read
            + writes as f64 * write
            + usage.output_tokens as f64 * output)
            / 1_000_000.0,
    )
}
/// Baseline Anthropic fallback uses Sonnet 4.6 pricing for unrecognized models.
/// Cache reads/writes are additional to Anthropic's uncached input counter.
pub fn anthropic(model: &str, usage: &Usage) -> f64 {
    let model = model.trim().to_lowercase();
    let mut model = model
        .split_once('/')
        .filter(|(_, bare)| !bare.is_empty())
        .map_or(model.as_str(), |(_, bare)| bare)
        .replace('.', "-");
    if let Some((prefix, date)) = model.rsplit_once('-')
        && date.len() == 8
        && date.bytes().all(|b| b.is_ascii_digit())
    {
        model = prefix.to_owned();
    }
    if model == "claude-3-5-haiku" {
        model = "claude-haiku-3-5".into();
    }
    let (input, output, read, write) = match model.as_str() {
        "claude-fable-5" | "claude-mythos-5" => (10.0, 50.0, 1.0, 12.5),
        "claude-opus-4" | "claude-opus-4-1" => (15.0, 75.0, 1.5, 18.75),
        "claude-sonnet-5" => (2.0, 10.0, 0.2, 2.5),
        "claude-haiku-3-5" => (0.8, 4.0, 0.08, 1.0),
        m if m.contains("fable") || m.contains("mythos") => (10.0, 50.0, 0.25, 12.5),
        m if m.contains("opus") => (5.0, 25.0, 0.5, 6.25),
        m if m.contains("haiku") => (1.0, 5.0, 0.1, 1.25),
        _ => (3.0, 15.0, 0.3, 3.75),
    };
    (usage.input_tokens as f64 * input
        + usage.output_tokens as f64 * output
        + usage.cache_read_tokens as f64 * read
        + usage.cache_creation_tokens as f64 * write)
        / 1_000_000.0
}
