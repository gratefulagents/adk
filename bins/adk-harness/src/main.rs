//! Offline baseline replay; never contacts models, tools, or platform services.
use serde_json::{Map, Value};
use std::io::{self, Read};

fn replay(operation: &str, input: &Value) -> Result<Value, Box<dyn std::error::Error>> {
    Ok(adk_platform::codec::replay(operation, input)?)
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let output = if args == ["--fixtures"] {
        let mut outputs = Map::new();
        for text in [
            include_str!("../../../fixtures/sdk.json"),
            include_str!("../../../fixtures/platform.json"),
        ] {
            let document: Value = serde_json::from_str(text)?;
            for case in document["cases"].as_array().ok_or("missing cases")? {
                let name = case["name"].as_str().ok_or("missing name")?;
                let operation = case["operation"].as_str().ok_or("missing operation")?;
                let result = replay(operation, &case["input"])?;
                if outputs.insert(name.to_owned(), result).is_some() {
                    return Err("duplicate fixture case".into());
                }
            }
        }
        Value::Object(outputs)
    } else if args.is_empty() {
        let mut input = String::new();
        // Bound the subprocess protocol, not arbitrary user input in memory.
        io::stdin()
            .take(8 * 1024 * 1024 + 1)
            .read_to_string(&mut input)?;
        if input.len() > 8 * 1024 * 1024 {
            return Err("input exceeds 8 MiB".into());
        }
        let request: Value = serde_json::from_str(&input)?;
        let operation = request["operation"].as_str().ok_or("missing operation")?;
        replay(operation, request.get("input").ok_or("missing input")?)?
    } else {
        return Err(
            "usage: adk-harness [--fixtures]; otherwise reads {operation,input} JSON on stdin"
                .into(),
        );
    };
    println!("{}", serde_json::to_string(&output)?);
    Ok(())
}

fn main() -> std::process::ExitCode {
    match run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("adk-harness: {error}");
            std::process::ExitCode::from(1)
        }
    }
}
