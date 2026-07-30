use std::fs;
use std::path::Path;
use std::process::{Command, Output};

fn write(root: &Path, relative: &str, body: &str) {
    let path = root.join(relative);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, body).unwrap();
}

fn run(root: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_duhem"))
        .current_dir(root)
        .env("RESOLVE_TEST_SECRET", "credential-that-must-not-leak")
        .args(args)
        .output()
        .unwrap()
}

fn fixture() -> tempfile::TempDir {
    let temp = tempfile::tempdir().unwrap();
    write(
        temp.path(),
        "shared.yml",
        r#"
profiles:
  staging:
    base_url: https://shared.example
inputs:
  base_url:
    type: string
    default: https://shared-default.example
"#,
    );
    write(
        temp.path(),
        "duhem.yml",
        r#"
manifest_version: 1
includes:
  - shared.yml
defaults:
  timeout: 9s
profiles:
  staging:
    base_url: https://staging.example
  prod:
    base_url: https://prod.example
inputs:
  base_url:
    type: string
    default: https://root-default.example
verifications:
  - path: leaf/duhem.yml
"#,
    );
    write(
        temp.path(),
        "leaf/duhem.yml",
        r#"
verification: resolve fixture
provision:
  up: ./must-not-run.sh
inputs:
  base_url: { inherit: true }
  password:
    type: string
    env: RESOLVE_TEST_SECRET
    secret: true
  user:
    type: string
    default: fixture@example.com
criteria:
  - id: AC-1
    description: invalid on purpose, but still resolvable
    checks:
      - id: AC-1.1
        steps:
          - uses: cli/invoke
            with:
              command: [echo, $inputs.base_url, $inputs.password, '$runtime.upper("resolved")']
          - uses: cli/invoke
            with:
              command: [echo, '$runtime.format("{}", $inputs.missing)']
        assertions:
          - $inputs.never_declared == true
"#,
    );
    write(
        temp.path(),
        "leaf/must-not-run.sh",
        "#!/bin/sh\ntouch ../provision-ran\n",
    );
    temp
}

#[test]
fn resolves_includes_profiles_secrets_validation_and_provenance_without_side_effects() {
    let temp = fixture();
    let output = run(
        temp.path(),
        &[
            "resolve",
            ".",
            "--profile",
            "staging",
            "--format",
            "json",
            "--provenance",
        ],
    );
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(!text.contains("credential-that-must-not-leak"), "{text}");
    assert!(text.contains("••••••"), "{text}");
    assert!(text.contains("shared.yml"), "{text}");
    assert!(text.contains("duhem.yml"), "{text}");
    assert!(text.contains("\"rung\": \"profile staging\""), "{text}");
    assert!(!temp.path().join("provision-ran").exists());

    let value: serde_json::Value = serde_json::from_str(&text).unwrap();
    let leaf = &value["verifications"][0];
    assert_eq!(
        leaf["document"]["inputs"]["base_url"],
        "https://staging.example"
    );
    assert_eq!(leaf["document"]["inputs"]["password"], "••••••");
    assert_eq!(
        leaf["document"]["criteria"][0]["checks"][0]["steps"][0]["with"]["command"][2],
        "••••••"
    );
    assert_eq!(
        leaf["document"]["criteria"][0]["checks"][0]["steps"][0]["with"]["command"][3],
        "RESOLVED"
    );
    assert!(
        leaf["errors"].as_array().unwrap().iter().any(|error| {
            error["stage"] == "reference_resolution"
                && error["message"].as_str().is_some_and(|message| {
                    message.contains("unresolved `$inputs.missing`")
                        && message
                            .contains("(evaluating `$runtime.format(\"{}\", $inputs.missing)`)")
                })
        }),
        "{leaf}"
    );
    assert_eq!(
        leaf["document"]["criteria"][0]["checks"][0]["steps"][0]["with"]["timeout"],
        9000
    );
    let base_origin = &leaf["provenance"]["inputs.base_url"];
    assert!(
        base_origin["source"]
            .as_str()
            .unwrap()
            .ends_with("duhem.yml"),
        "{base_origin}"
    );
    assert!(
        base_origin["overridden"]
            .as_array()
            .unwrap()
            .iter()
            .any(|origin| origin["source"].as_str().unwrap().ends_with("shared.yml")),
        "{base_origin}"
    );
    assert_eq!(
        leaf["provenance"]["inputs.password"]["rung"],
        "env RESOLVE_TEST_SECRET"
    );
    assert!(
        leaf["errors"]
            .as_array()
            .unwrap()
            .iter()
            .any(|error| error["stage"] == "validation"),
        "{leaf}"
    );
}

#[test]
fn profiles_change_values_and_json_matches_yaml_structure() {
    let temp = fixture();
    let staging = run(
        temp.path(),
        &["resolve", ".", "--profile", "staging", "--format", "json"],
    );
    let prod = run(
        temp.path(),
        &["resolve", ".", "--profile", "prod", "--format", "json"],
    );
    let yaml = run(
        temp.path(),
        &["resolve", ".", "--profile", "staging", "--format", "yaml"],
    );
    assert!(staging.status.success());
    assert!(prod.status.success());
    assert!(yaml.status.success());
    let staging_json: serde_json::Value = serde_json::from_slice(&staging.stdout).unwrap();
    let prod_json: serde_json::Value = serde_json::from_slice(&prod.stdout).unwrap();
    let yaml_value: serde_yml::Value = serde_yml::from_slice(&yaml.stdout).unwrap();
    let yaml_json = serde_json::to_value(yaml_value).unwrap();
    assert_eq!(staging_json, yaml_json);
    assert_eq!(
        staging_json["verifications"][0]["document"]["inputs"]["base_url"],
        "https://staging.example"
    );
    assert_eq!(
        prod_json["verifications"][0]["document"]["inputs"]["base_url"],
        "https://prod.example"
    );
}

#[test]
fn load_failure_is_reported_in_the_document() {
    let temp = tempfile::tempdir().unwrap();
    write(
        temp.path(),
        "duhem.yml",
        "verification: broken\nunknown: true\ncriteria: []\n",
    );
    let output = run(temp.path(), &["resolve", "duhem.yml", "--format", "json"]);
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["errors"][0]["stage"], "load");
    assert!(
        value["errors"][0]["message"]
            .as_str()
            .unwrap()
            .contains("unknown field")
    );
}

#[test]
fn invalid_secret_override_is_not_echoed_in_an_error() {
    let temp = tempfile::tempdir().unwrap();
    write(
        temp.path(),
        "duhem.yml",
        r#"
verification: secret error
inputs:
  pin: { type: integer, secret: true }
criteria:
  - id: AC-1
    description: x
    checks:
      - id: AC-1.1
        assertions: ["true"]
"#,
    );
    let output = run(
        temp.path(),
        &[
            "resolve",
            "duhem.yml",
            "--inputs",
            "pin=leaky-invalid-pin",
            "--format",
            "json",
        ],
    );
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(!text.contains("leaky-invalid-pin"), "{text}");
    assert!(text.contains("secret value"), "{text}");
}

#[test]
fn input_file_override_reports_file_and_line_provenance() {
    let temp = fixture();
    write(
        temp.path(),
        "overrides.yml",
        "base_url: https://override.example\n",
    );
    let output = run(
        temp.path(),
        &[
            "resolve",
            ".",
            "--profile",
            "staging",
            "--inputs",
            "@overrides.yml",
            "--format",
            "json",
            "--provenance",
        ],
    );
    assert!(output.status.success());
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let leaf = &value["verifications"][0];
    assert_eq!(
        leaf["document"]["inputs"]["base_url"],
        "https://override.example"
    );
    let provenance = &leaf["provenance"]["inputs.base_url"];
    assert_eq!(provenance["rung"], "--inputs @file");
    assert!(
        provenance["source"]
            .as_str()
            .unwrap()
            .ends_with("overrides.yml"),
        "{provenance}"
    );
    assert_eq!(provenance["line"], 1);
    assert!(
        provenance["overridden"]
            .as_array()
            .unwrap()
            .iter()
            .any(|origin| origin["source"].as_str().unwrap().ends_with("duhem.yml")),
        "{provenance}"
    );
}
