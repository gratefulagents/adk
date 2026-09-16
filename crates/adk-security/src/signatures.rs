pub(super) const SIGNATURES: &[(&str, &str)] = &[
    ("AWS access key", r#"\b(?:AKIA|ASIA)[0-9A-Z]{16}\b"#),
    ("GitHub token", r#"\bgh[psuro]_[A-Za-z0-9]{30,255}\b"#),
    (
        "GitHub fine-grained PAT",
        r#"\bgithub_pat_[A-Za-z0-9_]{36,255}\b"#,
    ),
    ("OpenAI project key", r#"\bsk-proj-[A-Za-z0-9_\-]{20,}\b"#),
    ("OpenAI API key", r#"\bsk-[A-Za-z0-9_\-]{40,}\b"#),
    (
        "Anthropic API key",
        r#"\bsk-ant-(?:api\d{2}-)?[A-Za-z0-9_\-]{20,}\b"#,
    ),
    ("Slack token", r#"\bxox[baprs]-[A-Za-z0-9-]{10,}\b"#),
    ("npm token", r#"\bnpm_[A-Za-z0-9]{30,}\b"#),
    (
        "JWT",
        r#"\beyJ[A-Za-z0-9_\-]{8,}\.eyJ[A-Za-z0-9_\-]{8,}\.[A-Za-z0-9_\-+/=.]{4,}"#,
    ),
    (
        "private key",
        r#"-----BEGIN (?:RSA |EC |DSA |OPENSSH |PGP |ENCRYPTED )?PRIVATE KEY-----"#,
    ),
    (
        "Authorization Bearer header",
        r#"(?i)Authorization\s*:\s*Bearer\s+[A-Za-z0-9._\-+/=]{16,}"#,
    ),
    ("Bearer token", r#"\bBearer\s+[A-Za-z0-9._\-+/=]{24,}\b"#),
    (
        "GCP service-account key",
        r#""type"\s*:\s*"service_account""#,
    ),
    (
        "generic credential assignment",
        r#"(?i)(api[_-]?key|secret[_-]?key|password)\s*[:=]\s*["'][^"']{8,}["']"#,
    ),
];
