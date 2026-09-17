//! Independent oracle: regenerate from the pinned Go SDK with
//! `cd repos/sdk && GOROOT=/usr/local/go GOTELEMETRY=off go run ../../fixtures/tools/schema-generate.go`.
//! The generator never reads the Rust manifest. JSON object order is immaterial;
//! descriptions, property types, required arrays and optional fields are exact.
use adk_core::{AccessMode, ToolDefinition};
use adk_tools::{Capability, capabilities, shell};
use serde::Deserialize;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Deserialize)]
struct Fixture {
    sdk_commit: String,
    control_flow_source: String,
    samples: Vec<Sample>,
}
#[derive(Deserialize)]
struct Sample {
    access: String,
    environment: BTreeMap<String, String>,
    definitions: Vec<Definition>,
}
#[derive(Deserialize)]
struct Definition {
    name: String,
    description: String,
    parameters: Value,
    read_only: bool,
    requires_approval: bool,
    control_flow: bool,
    timeout_seconds: u64,
}
fn fixture() -> Fixture {
    serde_json::from_str(include_str!(
        "../../../fixtures/tools/schema-definitions.json"
    ))
    .unwrap()
}
fn access(sample: &Sample) -> AccessMode {
    match sample.access.as_str() {
        "read_only" => AccessMode::ReadOnly,
        "workspace_write" => AccessMode::WorkspaceWrite,
        "full_access" => AccessMode::FullAccess,
        other => panic!("unknown SDK access mode {other}"),
    }
}
fn capability(sample: &Sample, expected: &Definition) -> &'static Capability {
    let candidates: Vec<_> = capabilities()
        .iter()
        .filter(|c| {
            c.name == expected.name
                && (c.mode == "any"
                    || c.mode == sample.access
                    || (c.mode == "write" && sample.access != "read_only"))
        })
        .collect();
    assert_eq!(candidates.len(), 1, "{} / {}", expected.name, sample.access);
    candidates[0]
}
fn actual(sample: &Sample, expected: &Definition) -> ToolDefinition {
    let c = capability(sample, expected);
    c.definition.clone().unwrap_or_else(|| {
        shell::definition(
            &expected.name,
            access(sample),
            &shell::Limits::from_environment(&sample.environment),
        )
        .expect("dynamic shell definition")
    })
}

#[test]
fn sdk_fixture_covers_every_catalog_variant_in_all_three_access_modes() {
    let fixture = fixture();
    assert_eq!(
        fixture.sdk_commit,
        "1dc92b73900fac74dc357a938e4b5eee6392b418"
    );
    assert!(
        fixture
            .control_flow_source
            .contains("isRegistryControlFlowTool")
    );
    assert_eq!(fixture.samples.len(), 12);
    let catalog: BTreeSet<_> = capabilities()
        .iter()
        .map(|c| (c.name.as_str(), c.mode.as_str()))
        .collect();
    assert_eq!(catalog.len(), 51);
    assert_eq!(
        capabilities()
            .iter()
            .map(|c| &c.name)
            .collect::<BTreeSet<_>>()
            .len(),
        46
    );
    let mut environments = BTreeSet::new();
    for sample in &fixture.samples {
        environments.insert(sample.environment.clone());
        let names: BTreeSet<_> = sample.definitions.iter().map(|d| &d.name).collect();
        assert_eq!(names.len(), sample.definitions.len(), "duplicate SDK tool");
    }
    assert_eq!(environments.len(), 4);
    assert!(
        environments.contains(&BTreeMap::new()),
        "default shell environment missing"
    );
    for environment in environments {
        let samples: Vec<_> = fixture
            .samples
            .iter()
            .filter(|s| s.environment == environment)
            .collect();
        assert_eq!(
            samples
                .iter()
                .map(|s| s.access.as_str())
                .collect::<BTreeSet<_>>(),
            BTreeSet::from(["read_only", "workspace_write", "full_access"])
        );
        let covered: BTreeSet<_> = samples
            .iter()
            .flat_map(|s| {
                s.definitions.iter().map(|d| {
                    let c = capability(s, d);
                    (c.name.as_str(), c.mode.as_str())
                })
            })
            .collect();
        assert_eq!(
            covered, catalog,
            "missing schema variant for {environment:?}"
        );
    }
}

// One contract dimension per test, with access/env/name in failure diagnostics.
macro_rules! compare_field {
    ($test:ident, $field:ident) => {
        #[test]
        fn $test() {
            for sample in fixture().samples {
                for expected in &sample.definitions {
                    assert_eq!(
                        actual(&sample, expected).$field,
                        expected.$field,
                        "{} / {} / {:?}",
                        expected.name,
                        sample.access,
                        sample.environment
                    );
                }
            }
        }
    };
}
compare_field!(names_match_actual_sdk_bundle_definitions, name);
compare_field!(
    descriptions_match_actual_sdk_bundle_definitions,
    description
);
compare_field!(
    read_only_flags_match_actual_sdk_bundle_definitions,
    read_only
);
compare_field!(
    approval_flags_match_actual_sdk_bundle_definitions,
    requires_approval
);

#[test]
fn parameter_schemas_match_actual_sdk_bundle_definitions() {
    for sample in fixture().samples {
        for expected in &sample.definitions {
            assert_eq!(
                serde_json::to_value(actual(&sample, expected).input_schema).unwrap(),
                expected.parameters,
                "{} / {} / {:?}",
                expected.name,
                sample.access,
                sample.environment
            );
        }
    }
}

#[test]
fn catalog_read_only_metadata_matches_actual_sdk_bundle_definitions() {
    for sample in fixture().samples {
        for expected in &sample.definitions {
            assert_eq!(
                capability(&sample, expected).read_only,
                expected.read_only,
                "{} / {}",
                expected.name,
                sample.access
            );
        }
    }
}

#[test]
fn control_flow_flags_match_pinned_sdk_registry_exemptions() {
    for sample in fixture().samples {
        for expected in &sample.definitions {
            assert_eq!(
                capability(&sample, expected).control_flow,
                expected.control_flow,
                "{} / {}",
                expected.name,
                sample.access
            );
        }
    }
}

#[test]
fn sdk_tool_timeouts_match_the_registry_zero_timeout_contract() {
    // ToolDefinition has no timeout member. Registry::build explicitly rejects
    // nonzero Tool::timeout(); verify this invariant against every Go instance.
    // These are tool-policy defaults, not Bash's parameter-level timeout default.
    for sample in fixture().samples {
        for expected in &sample.definitions {
            assert_eq!(
                expected.timeout_seconds, 0,
                "{} / {}",
                expected.name, sample.access
            );
        }
    }
}
