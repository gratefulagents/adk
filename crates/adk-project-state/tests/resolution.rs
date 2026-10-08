#![cfg(unix)]

use adk_project_state::{
    Error, FilesystemOptions, FilesystemResolutionHost, StoreOptions, resolve_filesystem_options,
};
use std::path::PathBuf;

fn host() -> FilesystemResolutionHost {
    FilesystemResolutionHost {
        cwd: "/".into(),
        home: Some("/host/home".into()),
    }
}

#[test]
fn pinned_go_resolution_observations() {
    // Observed by executing the unmodified resolution prefix and helpers from
    // sdk 1dc92b73900fac74dc357a938e4b5eee6392b418 filesystem.go, with cwd=/ and
    // HOME=/host/home. Execution stopped before backend construction or IO.
    let cases = [
        (
            "",
            "",
            "",
            "project-98f54143",
            "/host/home/.gratefulagents/projects/project-98f54143/state",
            "",
        ),
        (
            "",
            "",
            " . ",
            "project-42099b4a",
            "/host/home/.gratefulagents/projects/project-42099b4a/state",
            "/",
        ),
        (
            "",
            "",
            "repo/a/../My Project//",
            "my-project-53961fa2",
            "/host/home/.gratefulagents/projects/my-project-53961fa2/state",
            "/repo/My Project",
        ),
        (
            " ../state/./x/../ ",
            " Team / ALPHA ",
            "repo/work",
            "team-alpha",
            "/state",
            "/repo/work",
        ),
        (
            " /explicit/../state// ",
            "***",
            " /repo/work/../app ",
            "app-0632a276",
            "/state",
            "/repo/app",
        ),
        (
            "",
            "",
            "/",
            "project-42099b4a",
            "/host/home/.gratefulagents/projects/project-42099b4a/state",
            "/",
        ),
        (
            "",
            "",
            "/../../repo/./app",
            "app-0632a276",
            "/host/home/.gratefulagents/projects/app-0632a276/state",
            "/repo/app",
        ),
        (
            "",
            "",
            "/repo/!!!",
            "-227ac891",
            "/host/home/.gratefulagents/projects/227ac891/state",
            "/repo/!!!",
        ),
        (
            "",
            "",
            "/repo/   /child/..",
            "repo-9feece9c",
            "/host/home/.gratefulagents/projects/repo-9feece9c/state",
            "/repo/   ",
        ),
        (
            "",
            "İTeam",
            "repo/İApp",
            "iteam",
            "/host/home/.gratefulagents/projects/iteam/state",
            "/repo/İApp",
        ),
        (
            "",
            "",
            "repo/İApp",
            "iapp-e220c17a",
            "/host/home/.gratefulagents/projects/iapp-e220c17a/state",
            "/repo/İApp",
        ),
        (
            "",
            "",
            "\u{2003}repo/Ｃafé\u{2003}",
            "af-b47caa81",
            "/host/home/.gratefulagents/projects/af-b47caa81/state",
            "/repo/Ｃafé",
        ),
    ];
    for (state, id, work, expected_id, expected_state, expected_work) in cases {
        let resolved = resolve_filesystem_options(&host(), state, id, work).unwrap();
        assert_eq!(
            resolved.project_id, expected_id,
            "{state:?} {id:?} {work:?}"
        );
        assert_eq!(resolved.state_dir, PathBuf::from(expected_state));
        assert_eq!(resolved.work_dir, PathBuf::from(expected_work));
    }
}

#[test]
fn explicit_state_directory_needs_no_home_and_uses_host_cwd() {
    let host = FilesystemResolutionHost {
        cwd: "/host/parent/../cwd".into(),
        home: None,
    };
    for (state, expected) in [
        (" /absolute/./state/../stored ", "/absolute/stored"),
        (" .state/../stored ", "/host/cwd/stored"),
    ] {
        let resolved = resolve_filesystem_options(&host, state, " Project ", "work/tree").unwrap();
        let options = FilesystemOptions {
            state_dir: resolved.state_dir,
            store: StoreOptions {
                project_id: resolved.project_id,
                work_dir: resolved.work_dir,
                ..Default::default()
            },
            ..Default::default()
        };
        assert_eq!(options.state_dir, PathBuf::from(expected));
        assert!(options.state_dir.is_absolute());
        assert_eq!(options.store.work_dir, PathBuf::from("/host/cwd/work/tree"));
        assert_eq!(options.store.project_id, "project");
    }
    assert!(matches!(
        resolve_filesystem_options(&host, " \n ", "project", ""),
        Err(Error::Invalid(message)) if message.contains("home is required")
    ));
}

#[test]
fn host_paths_are_validated_even_for_absolute_configuration() {
    for invalid in ["", ".", "relative/path", "/absolute/\0path"] {
        let bad_cwd = FilesystemResolutionHost {
            cwd: invalid.into(),
            ..host()
        };
        assert!(matches!(
            resolve_filesystem_options(&bad_cwd, "/state", "project", "/work"),
            Err(Error::Invalid(message)) if message.contains("host cwd")
        ));
        let bad_home = FilesystemResolutionHost {
            home: Some(invalid.into()),
            ..host()
        };
        assert!(matches!(
            resolve_filesystem_options(&bad_home, "/state", "project", "/work"),
            Err(Error::Invalid(message)) if message.contains("host home")
        ));
    }
}

#[test]
fn non_utf8_host_paths_are_not_lossily_hashed() {
    use std::{ffi::OsString, os::unix::ffi::OsStringExt};
    let invalid = PathBuf::from(OsString::from_vec(b"/host/\xff".to_vec()));
    for host in [
        FilesystemResolutionHost {
            cwd: invalid.clone(),
            home: None,
        },
        FilesystemResolutionHost {
            cwd: "/".into(),
            home: Some(invalid),
        },
    ] {
        assert!(matches!(
            resolve_filesystem_options(&host, "/state", "", "work"),
            Err(Error::Invalid(_))
        ));
    }
}

#[test]
fn nul_in_configured_paths_is_rejected() {
    for (state, work) in [("/state\0", "work"), ("/state", "work\0")] {
        assert!(matches!(
            resolve_filesystem_options(&host(), state, "project", work),
            Err(Error::Invalid(_))
        ));
    }
}

#[test]
fn nonexistent_paths_are_cleaned_lexically_without_opening_a_store() {
    let host = FilesystemResolutionHost {
        cwd: "/not-required-to-exist/host/child/..".into(),
        home: Some("/not-required-to-exist/home/child/..".into()),
    };
    let resolved = resolve_filesystem_options(&host, "", "project", "./a/../work//").unwrap();
    assert_eq!(
        resolved.work_dir,
        PathBuf::from("/not-required-to-exist/host/work")
    );
    assert_eq!(
        resolved.state_dir,
        PathBuf::from("/not-required-to-exist/home/.gratefulagents/projects/project/state")
    );
}

#[test]
fn normalized_workdir_controls_identity_but_blank_workdir_does_not_use_cwd() {
    let first = resolve_filesystem_options(&host(), "/state", "", "repo/app").unwrap();
    let same = resolve_filesystem_options(&host(), "/state", "", "/repo/a/../app/.").unwrap();
    let other = resolve_filesystem_options(&host(), "/state", "", "/other/app").unwrap();
    assert_eq!(first.project_id, same.project_id);
    assert_ne!(first.project_id, other.project_id);
    assert!(other.project_id.starts_with("app-"));
    let elsewhere = FilesystemResolutionHost {
        cwd: "/elsewhere".into(),
        ..host()
    };
    let blank = resolve_filesystem_options(&elsewhere, "/state", "", " \n\t ").unwrap();
    assert_eq!(blank.project_id, "project-98f54143");
    assert!(blank.work_dir.as_os_str().is_empty());
    let dot = resolve_filesystem_options(&elsewhere, "/state", "", ".").unwrap();
    assert_ne!(blank.project_id, dot.project_id);
    assert_eq!(dot.work_dir, PathBuf::from("/elsewhere"));
}

#[test]
fn resolved_options_preserve_identity_when_the_store_is_opened() {
    for (id, work, expected) in [
        ("", "/repo/!!!", "-227ac891"),
        ("", "/repo/İApp", "iapp-e220c17a"),
        ("İTeam", "/repo/work", "iteam"),
        ("***", "", "project-98f54143"),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let options = FilesystemOptions {
            state_dir: directory.path().join("state"),
            store: StoreOptions {
                project_id: id.into(),
                work_dir: work.into(),
                actor: " actor ".into(),
                run_id: " run ".into(),
            },
            ..Default::default()
        }
        .resolve(&host())
        .unwrap();
        assert_eq!(options.store.project_id, id);
        let store = adk_project_state::ProjectStore::filesystem(options.clone()).unwrap();
        assert_eq!(store.project_id(), expected);
        assert_eq!(store.state_dir(), options.state_dir);
        assert_eq!(store.state().unwrap().project.project_id, expected);
        drop(store);
        let reopened = adk_project_state::ProjectStore::filesystem(options).unwrap();
        assert_eq!(reopened.project_id(), expected);
    }
}
