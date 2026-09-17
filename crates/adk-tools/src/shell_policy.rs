use adk_core::AccessMode;

/// Tool-layer defense in depth; filesystem enforcement must come from the executor.
/// `filesystem_enforced` is trusted host metadata, never a model argument.
///
/// Restricted modes intentionally tighten the SDK oracle: filesystem enforcement
/// does not authorize dynamic/compound shell syntax, indirect Git execution,
/// config aliases, or pushes with implicit/protected destinations. Ordinary build
/// programs remain supported; this is not a program allowlist or a confinement
/// boundary for code executed by Cargo, npm, interpreters, or repository hooks.
pub fn command_blocked(
    access: AccessMode,
    git_remote_writes: bool,
    command: &str,
    filesystem_enforced: bool,
) -> Option<String> {
    if !git_remote_writes && !filesystem_enforced {
        return Some("Command blocked: GitRemoteWrites=disabled requires an enforcing command sandbox so Git credentials remain unavailable to subprocesses".into());
    }
    check(access, git_remote_writes, command, filesystem_enforced, 0).or_else(|| {
        if access == AccessMode::FullAccess {
            return None;
        }
        literal_check(command, access, git_remote_writes)
            .err()
            .map(|reason| format!("Command blocked in restricted mode: {reason}"))
    })
}

fn literal_check(command: &str, access: AccessMode, remote: bool) -> Result<(), String> {
    let commands = adk_security::inspect_literal_commands(command).map_err(|e| e.to_string())?;
    for words in commands {
        let mut argv = words.as_slice();
        loop {
            let head = base(&argv[0]);
            if head == "env" {
                let mut i = 1;
                while argv
                    .get(i)
                    .is_some_and(|s| matches!(s.as_str(), "-u" | "--unset"))
                {
                    i += 2;
                }
                if i >= argv.len() || argv[i].starts_with('-') {
                    return Err("unsupported env invocation".into());
                }
                argv = &argv[i..];
            } else if head == "timeout" {
                let duration = argv.get(1).map(String::as_str).unwrap_or("");
                let number = duration.trim_end_matches(['s', 'm', 'h', 'd']);
                if argv.len() < 3 || number.parse::<f64>().is_err() || duration.starts_with('-') {
                    return Err("unsupported timeout invocation".into());
                }
                argv = &argv[2..];
            } else {
                break;
            }
        }
        let head = base(&argv[0]);
        if head == "gh" {
            return Err("the gh CLI is not allowed; use the built-in GitHub tools".into());
        }
        if assignment(&argv[0])
            || shell(head)
            || matches!(
                head,
                "if" | "then"
                    | "else"
                    | "elif"
                    | "fi"
                    | "for"
                    | "while"
                    | "until"
                    | "do"
                    | "done"
                    | "case"
                    | "esac"
                    | "select"
                    | "in"
                    | "!"
                    | "time"
                    | "function"
                    | "coproc"
                    | "eval"
                    | "source"
                    | "."
                    | "alias"
                    | "sudo"
                    | "doas"
                    | "nice"
                    | "ionice"
                    | "stdbuf"
                    | "exec"
                    | "command"
                    | "nohup"
                    | "setsid"
                    | "builtin"
                    | "xargs"
                    | "parallel"
                    | "export"
                    | "unset"
                    | "readonly"
                    | "declare"
                    | "typeset"
                    | "local"
                    | "let"
                    | "set"
                    | "enable"
                    | "unalias"
                    | "bind"
                    | "trap"
                    | "read"
                    | "mapfile"
                    | "readarray"
                    | "hash"
                    | "busybox"
                    | "toybox"
            )
            || head.starts_with("git-")
        {
            return Err(
                "compound, dynamic, or indirect execution cannot be authorized statically".into(),
            );
        }
        if head == "find"
            && argv
                .iter()
                .any(|a| matches!(a.as_str(), "-exec" | "-execdir" | "-ok" | "-okdir"))
        {
            return Err("indirect execution through find is unsupported".into());
        }
        if head != "git" {
            continue;
        }
        if argv.len() != words.len() {
            return Err("indirect git invocation is unsupported".into());
        }
        let mut i = 1;
        while let Some(option) = argv.get(i) {
            match option.as_str() {
                "--no-pager" => i += 1,
                "-C" if argv.get(i + 1).is_some() => i += 2,
                _ => break,
            }
        }
        let sub = argv.get(i).map(String::as_str).unwrap_or("");
        if access == AccessMode::ReadOnly
            && matches!(
                sub,
                "push"
                    | "commit"
                    | "reset"
                    | "remote"
                    | "merge"
                    | "rebase"
                    | "cherry-pick"
                    | "add"
                    | "rm"
                    | "clean"
                    | "tag"
                    | "branch"
                    | "stash"
            )
        {
            return Err("git mutation is not allowed in read-only mode".into());
        }
        match sub {
            "push" => {
                if !remote {
                    return Err("git push is disabled by the GitRemoteWrites policy".into());
                }
                let args: Vec<_> = argv[i + 1..].iter().map(String::as_str).collect();
                adk_security::validate_git_push(&args).map_err(|e| e.to_string())?;
            }
            "status" | "diff" | "show" | "log" | "rev-parse" | "ls-files" | "add" | "commit"
            | "reset" | "merge" | "rebase" | "cherry-pick" | "rm" | "clean" | "tag" | "branch"
            | "stash" | "checkout" | "switch" | "restore" | "fetch" | "pull" | "clone" | "init"
            | "revert" => {}
            _ => return Err("unsupported git subcommand, configuration, or alias".into()),
        }
    }
    Ok(())
}
fn check(
    access: AccessMode,
    remote: bool,
    command: &str,
    enforced: bool,
    depth: usize,
) -> Option<String> {
    if depth > 64 {
        return Some("Command blocked: shell nesting cannot be authorized statically".into());
    }
    let restricted = access != AccessMode::FullAccess;
    let mode = match access {
        AccessMode::ReadOnly => "read-only",
        AccessMode::WorkspaceWrite => "workspace-write",
        AccessMode::FullAccess => "danger-full-access",
    };
    let commands = parse(command);
    let dynamic = syntax_reason(command).or_else(|| {
        commands.iter().find_map(|cmd| {
            if let Some(word) = cmd.argv.iter().find(|w| assignment(w)) {
                return Some(format!(
                    "shell assignment {} cannot be authorized statically",
                    word.split('=').next().unwrap()
                ));
            }
            let argv = unwrap(&cmd.argv);
            let head = argv.first().map(|s| base(s)).unwrap_or_default();
            if matches!(head, "eval" | "source" | "." | "alias") {
                Some(format!("{head} cannot be authorized statically"))
            } else if head == "function" || head.ends_with("()") || head.starts_with(":()") {
                Some("function definitions cannot be authorized statically".into())
            } else {
                None
            }
        })
    });
    if let Some(reason) = &dynamic
        && (access == AccessMode::ReadOnly || access == AccessMode::WorkspaceWrite && !enforced)
    {
        return Some(format!(
            "Command blocked in {mode} mode: {reason} — write the command with only literal arguments, or split it into separate Bash calls"
        ));
    }
    for (index, cmd) in commands.iter().enumerate() {
        let mut argv = unwrap(&cmd.argv);
        if argv.len() == 1
            && base(&argv[0]) == "git"
            && let Some(next) = commands.get(index + 1)
        {
            argv.extend(unwrap(&next.argv));
        }
        if restricted && !enforced {
            if let Some(reason) = destructive(&argv, &cmd.redirects) {
                return Some(format!("Command blocked in {mode} mode: {reason}"));
            }
            if cmd.piped && argv.first().is_some_and(|s| shell(base(s))) {
                return Some(format!(
                    "Command blocked in {mode} mode: piping into {:?} is not allowed",
                    argv[0]
                ));
            }
        }
        let Some(head) = argv.first() else {
            continue;
        };
        let head = base(head);
        if head == "git" {
            let sub_index = git_sub(&argv);
            let sub = sub_index.map(|i| argv[i].as_str()).unwrap_or("");
            if !remote && (sub == "push" || alias_push(&argv, sub)) {
                return Some(
                    "Command blocked: git push is disabled by the GitRemoteWrites policy".into(),
                );
            }
            if access == AccessMode::WorkspaceWrite && dynamic.is_some() && sub == "push" {
                return Some(format!(
                    "Command blocked in {mode} mode: dynamic git push arguments cannot be authorized statically"
                ));
            }
            if (access == AccessMode::ReadOnly
                && matches!(
                    sub,
                    "push"
                        | "commit"
                        | "reset"
                        | "remote"
                        | "merge"
                        | "rebase"
                        | "cherry-pick"
                        | "add"
                        | "rm"
                        | "clean"
                        | "tag"
                        | "branch"
                        | "stash"
                ))
                || (access == AccessMode::WorkspaceWrite && sub == "remote")
            {
                let hint = if sub == "remote" {
                    " — remote configuration is platform-managed; use git_status or the built-in git/GitHub tools instead"
                } else {
                    " — this session cannot mutate the repository"
                };
                return Some(format!(
                    "Command blocked in {mode} mode: git {sub} is not allowed{hint}"
                ));
            }
            if restricted
                && sub == "push"
                && argv[sub_index.unwrap() + 1..]
                    .iter()
                    .any(|s| protected_ref(s))
            {
                return Some(format!(
                    "Command blocked in {mode} mode: push to main/master is not allowed"
                ));
            }
        }
        if restricted && head == "gh" {
            return Some(format!(
                "Command blocked in {mode} mode: the gh CLI is not allowed; use the built-in GitHub tools (create_pull_request, get_pull_request, list_review_threads, submit_pull_request_review, create_github_issue, ...) instead"
            ));
        }
        let mut nested = cmd.substitutions.clone();
        if shell(head) {
            for pair in argv[1..].windows(2) {
                if pair[0].starts_with('-') && pair[0].contains('c') {
                    nested.push(pair[1].clone());
                }
            }
            for (op, body) in &cmd.redirects {
                if op.starts_with("<<") {
                    nested.push(body.clone());
                }
            }
            if cmd.piped && index > 0 {
                let previous = unwrap(&commands[index - 1].argv);
                if previous
                    .first()
                    .is_some_and(|s| matches!(base(s), "echo" | "printf"))
                {
                    nested.push(previous[1..].join(" "));
                }
            }
        }
        if head == "eval" {
            nested.push(argv[1..].join(" "));
        }
        for body in nested {
            if let Some(reason) = check(access, remote, &body, enforced, depth + 1) {
                return Some(reason);
            }
        }
    }
    None
}
fn syntax_reason(command: &str) -> Option<String> {
    let bytes = command.as_bytes();
    let (mut single, mut double, mut escaped) = (false, false, false);
    for (i, &c) in bytes.iter().enumerate() {
        if escaped {
            escaped = false;
            continue;
        }
        if c == b'\\' && !single {
            escaped = true;
            continue;
        }
        if c == b'\'' && !double {
            single = !single;
            continue;
        }
        if single {
            continue;
        }
        if c == b'"' {
            double = !double;
            continue;
        }
        let next = bytes.get(i + 1).copied().unwrap_or_default();
        let reason = if c == b'`' {
            "backtick substitution cannot be authorized statically"
        } else if c == b'$' && next == b'\'' {
            "ANSI-C shell quoting cannot be authorized statically"
        } else if c == b'$' && (next.is_ascii_alphanumeric() || b"({_@*#?$!-".contains(&next)) {
            "shell substitution cannot be authorized statically"
        } else if !double && matches!(c, b'<' | b'>') && next == b'(' {
            "process substitution cannot be authorized statically"
        } else if !double && c == b'<' && next == b'<' {
            "heredocs and here-strings cannot be authorized statically"
        } else {
            continue;
        };
        return Some(reason.into());
    }
    None
}
fn base(s: &str) -> &str {
    s.rsplit('/').next().unwrap_or(s)
}
fn shell(s: &str) -> bool {
    matches!(s, "sh" | "bash" | "zsh" | "ksh" | "dash" | "ash")
}
fn assignment(s: &str) -> bool {
    s.split_once('=').is_some_and(|(name, _)| {
        !name.is_empty()
            && name
                .bytes()
                .enumerate()
                .all(|(i, c)| c == b'_' || c.is_ascii_alphabetic() || i > 0 && c.is_ascii_digit())
    })
}
fn unwrap(words: &[String]) -> Vec<String> {
    let mut words = words.to_vec();
    loop {
        while words.first().is_some_and(|s| assignment(s)) {
            words.remove(0);
        }
        let Some(head) = words.first().map(|s| base(s).to_owned()) else {
            return words;
        };
        let operands: &[&str] = match head.as_str() {
            "sudo" => &[
                "-u",
                "--user",
                "-g",
                "--group",
                "-h",
                "--host",
                "-p",
                "--prompt",
                "-C",
                "--close-from",
                "-D",
                "--chdir",
                "-R",
                "--chroot",
                "-r",
                "--role",
                "-t",
                "--type",
                "-T",
                "--command-timeout",
                "-U",
                "--other-user",
            ],
            "doas" => &["-C", "-u"],
            "env" => &["-u", "--unset", "-C", "--chdir"],
            "nice" => &["-n", "--adjustment"],
            "ionice" => &["-c", "--class", "-n", "--classdata", "-p", "--pid"],
            "stdbuf" => &["-i", "--input", "-o", "--output", "-e", "--error"],
            "exec" => &["-a"],
            "timeout" => &["-k", "--kill-after", "-s", "--signal"],
            "command" | "nohup" | "setsid" | "builtin" => &[],
            _ => return words,
        };
        let mut i = 1;
        let mut splice = None;
        while i < words.len() {
            let a = &words[i];
            if a.starts_with('-') {
                let (name, value) = a
                    .split_once('=')
                    .map_or((a.as_str(), None), |(n, v)| (n, Some(v)));
                if head == "env" && matches!(name, "-S" | "--split-string") {
                    let value = value
                        .or_else(|| words.get(i + 1).map(String::as_str))
                        .unwrap_or("");
                    let mut split: Vec<_> = value.split_whitespace().map(str::to_owned).collect();
                    split.extend_from_slice(
                        &words[(i + if a.contains('=') { 1 } else { 2 }).min(words.len())..],
                    );
                    splice = Some(split);
                    break;
                }
                i += if value.is_none() && operands.contains(&name) {
                    2
                } else {
                    1
                };
            } else if matches!(head.as_str(), "env" | "sudo" | "doas") && assignment(a) {
                i += 1;
            } else {
                break;
            }
        }
        if let Some(split) = splice {
            words = split;
            continue;
        }
        if head == "timeout" && i < words.len() {
            i += 1;
        }
        words.drain(..i.min(words.len()));
    }
}
fn git_sub(argv: &[String]) -> Option<usize> {
    let mut i = 1;
    while i < argv.len() {
        if matches!(
            argv[i].as_str(),
            "-c" | "-C" | "--git-dir" | "--work-tree" | "--namespace" | "--config-env"
        ) {
            i += 2;
        } else if argv[i].starts_with('-') {
            i += 1;
        } else {
            return Some(i);
        }
    }
    None
}
fn alias_push(argv: &[String], sub: &str) -> bool {
    argv.windows(2).any(|pair| {
        pair[0] == "-c"
            && pair[1].split_once('=').is_some_and(|(key, body)| {
                key.trim().eq_ignore_ascii_case(&format!("alias.{sub}")) && {
                    let fields: Vec<_> = body
                        .trim()
                        .trim_start_matches('!')
                        .split_whitespace()
                        .collect();
                    matches!(fields.first(), Some(&"push" | &"git-push"))
                        || fields.starts_with(&["git", "push"])
                }
            })
    })
}
fn protected_ref(s: &str) -> bool {
    let s = s.trim_start_matches('+');
    matches!(
        s,
        "main" | "master" | "refs/heads/main" | "refs/heads/master"
    ) || s.starts_with("main:")
        || s.starts_with("master:")
        || [":main", ":master", ":refs/heads/main", ":refs/heads/master"]
            .iter()
            .any(|end| s.ends_with(end))
}
fn forbidden(path: &str) -> bool {
    if matches!(
        path,
        "/dev/null"
            | "/dev/zero"
            | "/dev/full"
            | "/dev/tty"
            | "/dev/stdin"
            | "/dev/stdout"
            | "/dev/stderr"
            | "/dev/random"
            | "/dev/urandom"
    ) || path.starts_with("/dev/fd/")
    {
        return false;
    }
    ["/etc", "/dev", "/sys", "/proc", "/boot"]
        .iter()
        .any(|p| path == *p || path.starts_with(&format!("{p}/")))
}
fn destructive(argv: &[String], redirects: &[(String, String)]) -> Option<String> {
    for (op, target) in redirects {
        if op.starts_with('>') && forbidden(target) {
            return Some(format!(
                "redirect to protected path {target:?} is not allowed"
            ));
        }
    }
    let head = base(argv.first()?);
    let args = &argv[1..];
    if head == "rm"
        && args.iter().any(|a| {
            a == "--recursive"
                || a.starts_with('-') && !a.starts_with("--") && a.contains(['r', 'R'])
        })
        && args.iter().any(|a| {
            a.starts_with("/*")
                || matches!(
                    a.trim_end_matches('/'),
                    "" | "/etc"
                        | "/usr"
                        | "/bin"
                        | "/sbin"
                        | "/lib"
                        | "/lib64"
                        | "/var"
                        | "/boot"
                        | "/root"
                        | "/home"
                        | "/opt"
                        | "/dev"
                        | "/proc"
                        | "/sys"
                )
        })
    {
        return Some("recursive removal of root paths is not allowed".into());
    }
    if matches!(head, "chmod" | "chown")
        && args
            .iter()
            .any(|a| a == "--recursive" || a.starts_with('-') && a.contains('R'))
        && args.iter().any(|a| a == "/" || a.starts_with("/*"))
    {
        return Some(format!("{head} recursive at root is not allowed"));
    }
    if head == "dd" && args.iter().any(|a| a.starts_with("of=/dev/")) {
        return Some("dd to block device is not allowed".into());
    }
    if head.starts_with("mkfs") {
        return Some(format!("{head} is not allowed"));
    }
    if head == "tee"
        && let Some(path) = args.iter().find(|a| forbidden(a))
    {
        return Some(format!("tee to protected path {path:?} is not allowed"));
    }
    if head.starts_with(":()") {
        return Some("fork bomb pattern is not allowed".into());
    }
    if matches!(head, "python" | "python3" | "perl" | "ruby" | "node")
        && args.windows(2).any(|p| {
            p[0] == "-c"
                && ["/etc/passwd", "/etc/shadow", "/etc/hosts", "/etc/sudoers"]
                    .iter()
                    .any(|s| p[1].contains(s))
        })
    {
        return Some(format!("{head} -c writing to system files is not allowed"));
    }
    None
}

#[derive(Default)]
struct Command {
    piped: bool,
    argv: Vec<String>,
    redirects: Vec<(String, String)>,
    substitutions: Vec<String>,
}
#[derive(Default)]
struct Token {
    text: String,
    op: bool,
    substitutions: Vec<String>,
}
fn parse(input: &str) -> Vec<Command> {
    let tokens = tokenize(input);
    let mut commands = Vec::new();
    let mut cmd = Command::default();
    let mut i = 0;
    while i < tokens.len() {
        let token = &tokens[i];
        if !token.op {
            cmd.argv.push(token.text.clone());
            cmd.substitutions.extend(token.substitutions.clone());
        } else if matches!(token.text.as_str(), ">" | ">>" | "<" | "<<" | "<<<") {
            let mut target = String::new();
            if let Some(next) = tokens.get(i + 1).filter(|t| !t.op) {
                target = next.text.clone();
                cmd.substitutions.extend(next.substitutions.clone());
                i += 1;
            }
            cmd.redirects.push((token.text.clone(), target));
        } else {
            commands.push(std::mem::take(&mut cmd));
            cmd.piped = token.text == "|";
        }
        i += 1;
    }
    commands.push(cmd);
    commands
}
fn tokenize(input: &str) -> Vec<Token> {
    let r: Vec<char> = input.chars().collect();
    let mut out = Vec::new();
    let mut cur = Token::default();
    let (mut word, mut quote, mut i) = (false, '\0', 0);
    fn flush(out: &mut Vec<Token>, cur: &mut Token, word: &mut bool) {
        if *word {
            out.push(std::mem::take(cur));
            *word = false;
        }
    }
    while i < r.len() {
        let c = r[i];
        if quote == '\'' {
            if c == '\'' {
                quote = '\0';
            } else {
                cur.text.push(c);
            }
            i += 1;
            continue;
        }
        if c == '\\' && i + 1 < r.len() && (quote == '\0' || "\\\"$`\n".contains(r[i + 1])) {
            if r[i + 1] != '\n' {
                cur.text.push(r[i + 1]);
                word = true;
            }
            i += 2;
            continue;
        }
        if c == quote && quote != '\0' {
            quote = '\0';
            i += 1;
            continue;
        }
        if quote == '\0' && matches!(c, '\'' | '"') {
            quote = c;
            word = true;
            i += 1;
            continue;
        }
        if c == '$' && i + 1 < r.len() {
            if r[i + 1] == '(' {
                let start = i + 2;
                i += 2;
                let mut depth = 1;
                while i < r.len() {
                    if r[i] == '(' {
                        depth += 1;
                    } else if r[i] == ')' {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    i += 1;
                }
                cur.substitutions.push(r[start..i].iter().collect());
                word = true;
                i = (i + 1).min(r.len());
                continue;
            }
            if r[i + 1] == '\'' {
                word = true;
                i += 2;
                while i < r.len() && r[i] != '\'' {
                    if r[i] == '\\' && i + 1 < r.len() {
                        i += 1;
                        let escape = r[i];
                        if escape == 'x' || escape.is_digit(8) {
                            let radix = if escape == 'x' {
                                i += 1;
                                16
                            } else {
                                8
                            };
                            let mut n = 0;
                            let mut count = 0;
                            while i < r.len() && count < if radix == 16 { 2 } else { 3 } {
                                let Some(v) = r[i].to_digit(radix) else {
                                    break;
                                };
                                n = n * radix + v;
                                count += 1;
                                i += 1;
                            }
                            if let Some(c) = char::from_u32(n) {
                                cur.text.push(c);
                            }
                            continue;
                        }
                        cur.text.push(match escape {
                            'n' => '\n',
                            'r' => '\r',
                            't' => '\t',
                            'e' | 'E' => '\x1b',
                            'a' => '\x07',
                            'b' => '\x08',
                            'f' => '\x0c',
                            'v' => '\x0b',
                            other => other,
                        });
                    } else {
                        cur.text.push(r[i]);
                    }
                    i += 1;
                }
                i = (i + 1).min(r.len());
                continue;
            }
            if r[i + 1] == '{' || r[i + 1] == '_' || r[i + 1].is_ascii_alphabetic() {
                let braced = r[i + 1] == '{';
                i += if braced { 2 } else { 1 };
                let start = i;
                while i < r.len()
                    && if braced {
                        r[i] != '}'
                    } else {
                        r[i] == '_' || r[i].is_ascii_alphanumeric()
                    }
                {
                    i += 1;
                }
                let name: String = r[start..i].iter().collect();
                if braced {
                    i = (i + 1).min(r.len());
                }
                if name == "IFS" {
                    if quote == '"' {
                        cur.text.push(' ');
                    } else {
                        flush(&mut out, &mut cur, &mut word);
                    }
                } else {
                    word = true;
                }
                continue;
            }
        }
        if c == '`' {
            let start = i + 1;
            i += 1;
            while i < r.len() && r[i] != '`' {
                i += 1;
            }
            cur.substitutions.push(r[start..i].iter().collect());
            word = true;
            i = (i + 1).min(r.len());
            continue;
        }
        if quote == '\0' && c == '#' && !word {
            while i < r.len() && r[i] != '\n' {
                i += 1;
            }
            continue;
        }
        if quote == '\0' && (";|&<>\n".contains(c) || c.is_whitespace()) {
            flush(&mut out, &mut cur, &mut word);
            if !c.is_whitespace() || c == '\n' {
                let mut op = c.to_string();
                if "|&<>".contains(c) {
                    while i + 1 < r.len()
                        && r[i + 1] == c
                        && op.len() < if c == '<' { 3 } else { 2 }
                    {
                        op.push(c);
                        i += 1;
                    }
                }
                out.push(Token {
                    text: op,
                    op: true,
                    substitutions: Vec::new(),
                });
            }
            i += 1;
            continue;
        }
        cur.text.push(c);
        word = true;
        i += 1;
    }
    flush(&mut out, &mut cur, &mut word);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn guards_wrappers_and_remote_side_effects() {
        for command in [
            "git push origin HEAD:main",
            "env -u GIT_ASKPASS git push origin main",
            "sudo -u deploy git push origin master",
            "nice -n 5 git push origin main",
            "git remote -v",
            "bash -c 'gh pr list'",
            "env -S 'gh pr list'",
            "echo x | gh issue create",
            "printf 'git push origin main' | bash",
            "git\npush origin main",
        ] {
            assert!(
                command_blocked(AccessMode::WorkspaceWrite, true, command, true).is_some(),
                "{command}"
            );
        }
        for command in [
            "echo gh",
            "git status",
            "git push origin feature",
            "env -u GITHUB_TOKEN ls",
            "timeout 30 make test",
        ] {
            assert!(
                command_blocked(AccessMode::WorkspaceWrite, true, command, true).is_none(),
                "{command}"
            );
        }
    }
    #[test]
    fn dynamic_read_only_and_disabled_credentials() {
        for command in [
            "echo $HOME",
            "X=1 git status",
            "cat <<< hello",
            "echo `ls`",
            "eval ls",
            "alias foo=ls",
            "foo() { ls; }",
            "printf $'hi'",
            "source file",
            "cat <(ls)",
        ] {
            assert!(
                command_blocked(AccessMode::ReadOnly, true, command, true).is_some(),
                "{command}"
            );
        }
        for command in [
            "printf '%s' \"$HOME\"",
            "wc -l $(find . -name '*.go')",
            "git push origin $(printf main)",
            "BR=main; git push origin $BR",
        ] {
            assert!(
                command_blocked(AccessMode::WorkspaceWrite, true, command, true).is_some(),
                "enforcement must not authorize dynamic syntax: {command}"
            );
        }
        assert!(
            command_blocked(AccessMode::FullAccess, false, "echo safe", false)
                .unwrap()
                .contains("enforcing command sandbox")
        );
        for command in [
            "git push origin feature",
            "git -c alias.publish=push publish origin feature",
            "git -c alias.publish='!git push' publish origin feature",
        ] {
            assert!(
                command_blocked(AccessMode::FullAccess, false, command, true)
                    .unwrap()
                    .contains("GitRemoteWrites")
            );
        }
    }
    #[test]
    fn destructive_advisory_and_safe_devices() {
        for command in [
            "rm -fr /",
            "sudo rm -r /*",
            "echo x >/etc/hosts",
            "dd if=a of=/dev/sda",
            "mkfs.ext4 /tmp/foo",
            "tee /sys/foo",
        ] {
            assert!(
                command_blocked(AccessMode::WorkspaceWrite, true, command, false).is_some(),
                "{command}"
            );
        }
        for command in [
            "ls 2>/dev/null",
            "echo x >/dev/stderr",
            "echo x | tee /dev/stdout",
        ] {
            assert!(
                command_blocked(AccessMode::WorkspaceWrite, true, command, false).is_none(),
                "{command}"
            );
        }
    }
}
