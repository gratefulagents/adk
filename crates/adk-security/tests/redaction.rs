use adk_security::{check_secrets, redact_secrets};

#[test]
fn diagnostic_redaction_preserves_surroundings_without_weakening_enforcement() {
    let token = format!("ghp_{}", "x".repeat(36));
    let input = format!("before {token} after {token}");
    let (output, kinds, count) = redact_secrets(&input);
    assert_eq!(count, 2);
    assert_eq!(kinds.len(), 1);
    assert!(output.starts_with("before [REDACTED:"));
    assert!(output.contains("] after [REDACTED:"));
    assert!(!output.contains(&token));
    assert!(check_secrets(&input).is_err());
    assert_eq!(
        redact_secrets("ordinary diagnostic"),
        ("ordinary diagnostic".into(), vec![], 0)
    );
}
