//! Foundation inspection binary; deliberately not a production worker.
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args == ["--version"] {
        println!(
            "adk-agent {} (foundation only; no runner)",
            env!("CARGO_PKG_VERSION")
        );
        ExitCode::SUCCESS
    } else {
        eprintln!("adk-agent is a foundation binary, not a worker. Supported: --version");
        ExitCode::from(2)
    }
}
