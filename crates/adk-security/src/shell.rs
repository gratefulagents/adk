use adk_core::{AccessMode, Error, ErrorCategory};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandClass {
    ReadOnly,
    Mutating,
}

#[derive(Debug, PartialEq, Eq)]
enum Token {
    Word(String),
    Separator(bool),
    Redirect(bool),
}

fn blocked(message: &'static str) -> Error {
    Error::new(ErrorCategory::PermissionDenied, message)
}

// This is a recognizer for a literal shell subset, not a shell emulator. Rejecting
// expansion before classification avoids guessing the command's runtime argv.
fn tokenize(input: &str) -> Result<Vec<Token>, Error> {
    if input.len() > 65_536 {
        return Err(blocked("command exceeds classifier limit"));
    }
    let mut chars = input.chars().peekable();
    let mut tokens = Vec::new();
    let mut word = String::new();
    let mut started = false;
    let mut quote = None;
    let mut literal = true;
    while let Some(c) = chars.next() {
        if c == '\0'
            || (c.is_control() && !matches!(c, '\n' | '\t'))
            || matches!(c, '\u{200b}'..='\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2060}'..='\u{206f}' | '\u{feff}')
        {
            return Err(blocked("control characters in shell command"));
        }
        if quote == Some('\'') {
            if c == '\'' {
                quote = None;
            } else {
                word.push(c);
            }
            continue;
        }
        if c == '\\' {
            literal = false;
            let next = chars
                .next()
                .ok_or_else(|| blocked("incomplete shell escape"))?;
            if next.is_control() {
                return Err(blocked("escaped control character"));
            }
            if quote == Some('"') && !matches!(next, '$' | '`' | '"' | '\\') {
                word.push('\\');
            }
            word.push(next);
            started = true;
            continue;
        }
        if matches!(c, '$' | '`') {
            return Err(blocked("shell expansion is not statically authorized"));
        }
        if quote == Some('"') {
            if c == '"' {
                quote = None;
            } else {
                word.push(c);
            }
            continue;
        }
        match c {
            '\'' | '"' => {
                quote = Some(c);
                literal = false;
                started = true;
            }
            ' ' | '\t' => {
                if started {
                    tokens.push(Token::Word(std::mem::take(&mut word)));
                    started = false;
                }
                literal = true;
            }
            '\n' | ';' | '|' | '&' => {
                if started {
                    tokens.push(Token::Word(std::mem::take(&mut word)));
                    started = false;
                }
                literal = true;
                if matches!(c, '|' | '&') && chars.peek() == Some(&c) {
                    chars.next();
                } else if c == '&' {
                    return Err(blocked("background shell execution is unsupported"));
                }
                tokens.push(Token::Separator(matches!(c, ';' | '\n')));
            }
            '>' | '<' => {
                if started {
                    // An unquoted adjacent numeric word is an IO descriptor, not argv.
                    if !literal || !word.chars().all(|c| c.is_ascii_digit()) {
                        tokens.push(Token::Word(std::mem::take(&mut word)));
                    }
                    word.clear();
                    started = false;
                }
                literal = true;
                if c == '>' && chars.peek() == Some(&'>') {
                    chars.next();
                }
                if matches!(chars.peek(), Some('<' | '>' | '&' | '|' | '(')) {
                    return Err(blocked("unsupported shell redirection"));
                }
                tokens.push(Token::Redirect(c == '>'));
            }
            '(' | ')' | '{' | '}' | '*' | '?' | '[' | ']' | '~' | '#' => {
                return Err(blocked("unsupported or dynamic shell syntax"));
            }
            _ => {
                word.push(c);
                started = true;
            }
        }
        if tokens.len() > 2048 {
            return Err(blocked("too many shell tokens"));
        }
    }
    if quote.is_some() {
        return Err(blocked("unterminated shell quote"));
    }
    if started {
        tokens.push(Token::Word(word));
    }
    if tokens.len() > 2048 {
        return Err(blocked("too many shell tokens"));
    }
    Ok(tokens)
}

/// Inspect argv in the bounded literal grammar, removing IO descriptors and
/// redirect targets. This is syntax inspection, NOT program authorization or
/// filesystem confinement; callers must apply their own command policy.
pub fn inspect_literal_commands(command: &str) -> Result<Vec<Vec<String>>, Error> {
    let tokens = tokenize(command)?;
    let mut commands = Vec::new();
    let mut argv = Vec::new();
    let mut iter = tokens.into_iter().peekable();
    while let Some(token) = iter.next() {
        match token {
            Token::Word(word) => argv.push(word),
            Token::Redirect(_) => {
                if !matches!(iter.next(), Some(Token::Word(_))) {
                    return Err(blocked("missing redirect target"));
                }
            }
            Token::Separator(terminal) => {
                if !terminal && iter.peek().is_none() {
                    return Err(blocked("incomplete shell operator"));
                }
                if argv.is_empty() {
                    return Err(blocked("empty shell statement"));
                }
                commands.push(std::mem::take(&mut argv));
            }
        }
    }
    if !argv.is_empty() {
        commands.push(argv);
    }
    if commands.is_empty() {
        return Err(blocked("empty shell command"));
    }
    Ok(commands)
}

/// Classify a bounded literal grammar. Unknown programs, options and dynamic
/// syntax fail closed at every access level; sandbox availability is irrelevant.
pub fn classify_command(
    command: &str,
    access: AccessMode,
    git_remote_writes: bool,
) -> Result<CommandClass, Error> {
    let tokens = tokenize(command)?;
    let mut argv = Vec::new();
    let mut mutated = false;
    let mut any = false;
    let mut iter = tokens.iter().peekable();
    while let Some(token) = iter.next() {
        match token {
            Token::Word(word) => argv.push(word.as_str()),
            Token::Redirect(write) => {
                let Some(Token::Word(path)) = iter.next() else {
                    return Err(blocked("missing redirect target"));
                };
                if *write && path != "/dev/null" {
                    if protected_path(path) {
                        return Err(blocked("redirect to protected path"));
                    }
                    mutated = true;
                }
            }
            Token::Separator(terminal) => {
                if !terminal && iter.peek().is_none() {
                    return Err(blocked("incomplete shell operator"));
                }
                if argv.is_empty() {
                    return Err(blocked("empty shell statement"));
                }
                mutated |= classify_argv(&argv, git_remote_writes)? == CommandClass::Mutating;
                any = true;
                argv.clear();
            }
        }
    }
    if !argv.is_empty() {
        mutated |= classify_argv(&argv, git_remote_writes)? == CommandClass::Mutating;
        any = true;
    }
    if !any {
        return Err(blocked("empty shell command"));
    }
    if mutated && access == AccessMode::ReadOnly {
        return Err(blocked("command mutates in read-only mode"));
    }
    Ok(if mutated {
        CommandClass::Mutating
    } else {
        CommandClass::ReadOnly
    })
}

fn protected_path(path: &str) -> bool {
    if path.split('/').any(|component| component == "..") {
        return true;
    }
    if !path.starts_with('/') {
        return false;
    }
    let mut components = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                components.pop();
            }
            _ => components.push(part),
        }
    }
    components.is_empty()
        || matches!(
            components[0],
            "etc"
                | "usr"
                | "bin"
                | "sbin"
                | "lib"
                | "lib64"
                | "var"
                | "boot"
                | "root"
                | "home"
                | "opt"
                | "dev"
                | "proc"
                | "sys"
        )
}

fn options(args: &[&str], shorts: &str, longs: &[&str]) -> bool {
    for arg in args {
        if *arg == "--" {
            return true;
        }
        if let Some(long) = arg.strip_prefix("--") {
            if !longs.contains(&long) {
                return false;
            }
        } else if let Some(short) = arg.strip_prefix('-') {
            if !short.chars().all(|c| shorts.contains(c)) {
                return false;
            }
        }
    }
    true
}

fn classify_argv(argv: &[&str], remote: bool) -> Result<CommandClass, Error> {
    let head = argv[0];
    // Arbitrary paths could name workspace programs masquerading as safe tools.
    let head = head
        .strip_prefix("/usr/bin/")
        .or_else(|| head.strip_prefix("/bin/"))
        .unwrap_or(head);
    let args = &argv[1..];
    let read_only = match head {
        "true" | "false" => args.is_empty(),
        "echo" => true,
        "pwd" => options(args, "LP", &[]),
        "cat" => options(
            args,
            "AbenstuvET",
            &["show-all", "number", "number-nonblank", "squeeze-blank"],
        ),
        "ls" => options(
            args,
            "aAbBcCdDfFghHiIklLmnpqQrRstTuUvwxX1",
            &[
                "all",
                "almost-all",
                "directory",
                "human-readable",
                "numeric-uid-gid",
                "recursive",
                "reverse",
            ],
        ),
        "wc" => options(
            args,
            "clmwL",
            &["bytes", "chars", "lines", "words", "max-line-length"],
        ),
        "grep" => options(
            args,
            "EFGPeifvwxclLnhHsqrRoaAbBC0123456789",
            &[
                "fixed-strings",
                "extended-regexp",
                "ignore-case",
                "line-number",
                "recursive",
                "files-with-matches",
                "count",
            ],
        ),
        "head" | "tail" => options(args, "cnqv0123456789", &[]),
        "git" => return classify_git(args, remote),
        "rm" => {
            if !options(args, "rfRivd", &["recursive", "force", "verbose", "dir"]) {
                return Err(blocked("unsupported rm option"));
            }
            if args.iter().any(|arg| protected_path(arg)) {
                return Err(blocked("removal of protected paths"));
            }
            return Ok(CommandClass::Mutating);
        }
        "mkdir" => {
            if !options(args, "pv", &["parents", "verbose"]) {
                return Err(blocked("unsupported mkdir option"));
            }
            if args.iter().any(|arg| protected_path(arg)) {
                return Err(blocked("mutation of protected paths"));
            }
            return Ok(CommandClass::Mutating);
        }
        _ => return Err(blocked("unknown or indirect-execution program")),
    };
    if !read_only {
        return Err(blocked("unsupported command arguments"));
    }
    Ok(CommandClass::ReadOnly)
}

fn classify_git(args: &[&str], remote: bool) -> Result<CommandClass, Error> {
    let args = args.strip_prefix(&["--no-pager"]).unwrap_or(args);
    let Some((sub, rest)) = args.split_first() else {
        return Err(blocked("missing git subcommand"));
    };
    let safe = match *sub {
        "status" => options(
            rest,
            "sbzuno",
            &[
                "short",
                "branch",
                "porcelain",
                "porcelain=v1",
                "porcelain=v2",
                "untracked-files=no",
                "untracked-files=all",
            ],
        ),
        "diff" | "show" | "log" => {
            let flags = rest.split(|arg| *arg == "--").next().unwrap_or_default();
            if !flags.contains(&"--no-ext-diff") || !flags.contains(&"--no-textconv") {
                return Err(blocked("git diff helpers must be explicitly disabled"));
            }
            options(
                rest,
                "pUsw0123456789",
                &[
                    "stat",
                    "numstat",
                    "shortstat",
                    "name-only",
                    "name-status",
                    "oneline",
                    "no-patch",
                    "no-ext-diff",
                    "no-textconv",
                    "cached",
                    "staged",
                    "all",
                    "decorate",
                    "reverse",
                ],
            )
        }
        "rev-parse" => options(
            rest,
            "",
            &[
                "verify",
                "short",
                "show-toplevel",
                "is-inside-work-tree",
                "abbrev-ref",
            ],
        ),
        "ls-files" => options(
            rest,
            "zcmdos",
            &[
                "cached",
                "modified",
                "deleted",
                "others",
                "exclude-standard",
            ],
        ),
        "push" => {
            if !remote {
                return Err(blocked("git remote writes are disabled"));
            }
            validate_git_push(rest)?;
            return Ok(CommandClass::Mutating);
        }
        // Local mutations still need an enforcing workspace sandbox.
        "add" => {
            if !options(rest, "Auv", &["all", "update", "verbose"]) {
                return Err(blocked("unsupported git add option"));
            }
            return Ok(CommandClass::Mutating);
        }
        _ => false,
    };
    if !safe {
        return Err(blocked("unsupported git subcommand or options"));
    }
    Ok(CommandClass::ReadOnly)
}

/// Validate arguments after `git push`: an explicit remote and one literal,
/// nonprotected branch destination, optionally preceded by `-u`.
/// This does not grant remote-write permission or authorize Git configuration.
pub fn validate_git_push(rest: &[&str]) -> Result<(), Error> {
    let rest = if matches!(rest.first(), Some(&"-u" | &"--set-upstream")) {
        &rest[1..]
    } else {
        rest
    };
    if rest.len() != 2 || !simple_ref(rest[0]) {
        return Err(blocked("push requires explicit remote and single refspec"));
    }
    let spec = rest[1];
    let (source, dest) = spec.split_once(':').unwrap_or((spec, spec));
    if !simple_ref(source) || !simple_ref(dest) {
        return Err(blocked("unsupported push refspec"));
    }
    let branch = dest.strip_prefix("refs/heads/").unwrap_or(dest);
    if matches!(branch, "main" | "master" | "HEAD")
        || (dest.starts_with("refs/") && !dest.starts_with("refs/heads/"))
    {
        return Err(blocked("push to protected or ambiguous ref"));
    }
    Ok(())
}

fn simple_ref(value: &str) -> bool {
    !value.is_empty()
        && !value.starts_with('-')
        && !value.starts_with('/')
        && !value.contains("..")
        && !value.contains("//")
        && !value.ends_with('/')
        && !value.ends_with(".lock")
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'/' | b'_' | b'-' | b'.'))
}
