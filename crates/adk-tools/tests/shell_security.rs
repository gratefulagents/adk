//! Pure authorization regressions: none of these shell payloads are executed.
//! The enforced SDK used to permit dynamic syntax and ambiguous pushes. Those
//! unsafe behaviors are deliberately tightened rather than copied from fixtures.
use adk_core::AccessMode;
use adk_tools::shell::command_blocked;

#[test]
fn enforced_remote_write_bypasses_are_denied() {
    for command in [
        "if git push origin main; then :; fi",
        "2>/dev/null git push origin main",
        "echo \"${__ADK_REVIEW_UNSET:-$(git push origin main)}\"",
        "2>/dev/null git push origin feature",
        "git 2>/dev/null push origin feature",
        "git push 2>/dev/null origin feature",
        "0</dev/null 2>/dev/null git push origin feature",
    ] {
        assert!(
            command_blocked(AccessMode::WorkspaceWrite, false, command, true).is_some(),
            "remote writes disabled: {command}"
        );
    }
}

#[test]
fn restricted_shell_syntax_fails_closed_even_with_enforcement() {
    for command in [
        "if git status; then :; fi",
        "while true; do git status; done",
        "for x in main; do git push origin main; done",
        "case x in x) git push origin main;; esac",
        "(git push origin main)",
        "{ git push origin main; }",
        "echo \"${__ADK_REVIEW_UNSET:-$(git push origin main)}\"",
        "echo `git push origin main`",
        "printf '%s' \"$HOME\"",
        "wc -l $(find . -name '*.go')",
        "X=1 cargo test",
        "export GIT_CONFIG_COUNT=1",
        "f() { git push origin main; }; f",
        "eval 'git push origin main'",
        "source command-file",
        "cat <(git push origin main)",
        "cat <<EOF\ngit push origin main\nEOF",
        "printf $'git push'",
        "git push origin feat*",
        "git status &&",
        "git status |",
        "git status >",
        "echo 'unterminated",
    ] {
        for access in [AccessMode::ReadOnly, AccessMode::WorkspaceWrite] {
            for remote in [false, true] {
                for enforced in [false, true] {
                    assert!(
                        command_blocked(access, remote, command, enforced).is_some(),
                        "{access:?}, remote={remote}, enforced={enforced}: {command}"
                    );
                }
            }
        }
    }
}

#[test]
fn git_requires_direct_invocation_and_explicit_nonprotected_destination() {
    for command in [
        "git push origin --all",
        "git push origin HEAD",
        "git push",
        "git push origin",
        "git push --mirror origin",
        "git push origin --tags",
        "git push origin HEAD:refs/tags/release",
        "git push origin HEAD:refs/heads/HEAD",
        "git push origin HEAD:refs/heads/main",
        "git push origin HEAD:master",
        "git push origin :feature",
        "git push origin +HEAD:feature",
        "git push origin feature other",
        "git push --repo=origin HEAD:feature",
        "git push origin HEAD:feature --receive-pack=evil",
        "git push origin HEAD:feature..bad",
        "git -c alias.p=push p origin HEAD:main",
        "git -calias.p=push p origin HEAD:main",
        "git --config-env=alias.p=PAYLOAD p origin HEAD:main",
        "git -c push.default=matching push origin feature",
        "git config alias.p push; git p origin HEAD:main",
        "git p origin HEAD:main",
        "git --exec-path=custom push origin feature",
        "git-push origin HEAD:main",
        "env git push origin feature",
        "env -S 'git push origin main'",
        "timeout 30 git push origin feature",
        "command git push origin feature",
        "exec git push origin feature",
        "bash -c 'git push origin feature'",
        "sh script.sh",
        "printf 'git push origin feature' | bash",
        "printf main | xargs git push origin",
        "find . -exec git push origin main ';'",
        "2>/dev/null gh pr merge 5",
    ] {
        for enforced in [false, true] {
            assert!(
                command_blocked(AccessMode::WorkspaceWrite, true, command, enforced).is_some(),
                "enforced={enforced}: {command}"
            );
        }
    }
    for command in [
        "git push origin feature",
        "git push -u origin feature",
        "git push --set-upstream origin HEAD:feature",
        "git push origin HEAD:refs/heads/feature",
        "git --no-pager -C repo push origin HEAD:feature",
        "2>/dev/null git push origin HEAD:feature",
        "git 2>/dev/null push origin feature",
        "git push origin feature 2>/dev/null",
    ] {
        assert_eq!(
            command_blocked(AccessMode::WorkspaceWrite, true, command, true),
            None,
            "{command}"
        );
    }
}

#[test]
fn ordinary_build_and_test_commands_still_work_with_enforcement() {
    for command in [
        "cargo test --workspace --all-targets",
        "cargo build --release",
        "cargo fmt --all -- --check && cargo clippy --all-targets",
        "npm ci && npm run build && npm test -- --runInBand",
        "npm run lint",
        "npx tsc --noEmit",
        "pnpm test",
        "yarn build",
        "go test ./...",
        "python3 -m pytest tests -q",
        "make test",
        "test -f Cargo.toml && cargo test",
        "cd crates/adk-tools && cargo test --test shell_security",
        "cargo test 2>/dev/null | tee test.log",
        "2>/dev/null cargo test",
        "env -u GITHUB_TOKEN cargo test",
        "timeout 30 make test",
        "git --no-pager status; git diff --stat",
        "git add file && git commit -m 'local change'",
        "printf '%s' '$HOME $(literal)'",
    ] {
        for remote in [false, true] {
            assert_eq!(
                command_blocked(AccessMode::WorkspaceWrite, remote, command, true),
                None,
                "remote={remote}: {command}"
            );
        }
    }
}

#[test]
fn io_descriptors_are_not_command_arguments_but_quoted_numbers_are() {
    for command in ["2>/dev/null git status", "git 2>/dev/null status"] {
        assert_eq!(
            adk_security::inspect_literal_commands(command).unwrap(),
            vec![vec!["git", "status"]]
        );
    }
    for command in [
        "'2'>/dev/null git status",
        "\\2>/dev/null git status",
        "2''>/dev/null git status",
    ] {
        assert_eq!(
            adk_security::inspect_literal_commands(command).unwrap(),
            vec![vec!["2", "git", "status"]]
        );
    }
    for command in [
        "2>/dev/null git commit -m x",
        "2>/dev/null git push origin feature",
    ] {
        assert!(command_blocked(AccessMode::ReadOnly, true, command, true).is_some());
    }
}

#[test]
fn explicit_full_access_behavior_is_preserved() {
    for command in [
        "if git push origin main; then :; fi",
        "echo \"${__ADK_REVIEW_UNSET:-$(git push origin main)}\"",
        "git push origin --all",
        "git push origin HEAD",
        "git -c alias.p=push p origin HEAD:main",
        "X=1 cargo test",
        "bash -c 'git push origin main'",
    ] {
        for enforced in [false, true] {
            assert_eq!(
                command_blocked(AccessMode::FullAccess, true, command, enforced),
                None,
                "{command}"
            );
        }
    }
    assert!(command_blocked(AccessMode::FullAccess, false, "echo safe", false).is_some());
    assert!(
        command_blocked(
            AccessMode::FullAccess,
            false,
            "git push origin feature",
            true
        )
        .is_some()
    );
}
