use async_openai::types::responses::CreateResponse;
use serde_json::json;
fn main() {
    let source = json!({"model":"gpt-5.6","input":[
        {"type":"reasoning","id":"r","summary":[],"encrypted_content":"reasoning-opaque"},
        {"type":"compaction","id":"c","encrypted_content":"compact-opaque"}
    ],"instructions":"","store":false,"stream":true,
    "include":["reasoning.encrypted_content"],"reasoning":{"effort":"max","summary":"auto"},
    "prompt_cache_key":"synthetic","prompt_cache_retention":"24h"});
    let typed: CreateResponse = serde_json::from_value(source.clone()).unwrap();
    let wire = serde_json::to_value(typed).unwrap();
    for key in [
        "model",
        "input",
        "instructions",
        "store",
        "stream",
        "include",
        "reasoning",
        "prompt_cache_key",
        "prompt_cache_retention",
    ] {
        assert_eq!(wire[key], source[key], "field {key}");
    }
    println!("async-openai 0.42.0 typed Responses continuation/cache/reasoning roundtrip: PASS");
}
