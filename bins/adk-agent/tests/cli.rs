use std::process::Command;

#[test]
fn advertises_foundation_and_rejects_worker_invocations() {
    let version = Command::new(env!("CARGO_BIN_EXE_adk-agent"))
        .arg("--version")
        .output()
        .unwrap();
    assert!(version.status.success());
    assert!(
        String::from_utf8(version.stdout)
            .unwrap()
            .contains("foundation only; no runner")
    );
    for args in [vec![], vec!["run"], vec!["slack"], vec!["desktop-bridge"]] {
        let result = Command::new(env!("CARGO_BIN_EXE_adk-agent"))
            .args(args)
            .output()
            .unwrap();
        assert_eq!(result.status.code(), Some(2));
    }
}
