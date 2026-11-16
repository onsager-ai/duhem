//! Manifest policy must survive run -> bundle -> server ingest (#673).
use std::collections::BTreeMap;
use std::sync::Arc;

use duhem_evidence::{EventPayload, RunBundle, SqliteStore, Trace, replay};
use duhem_judge::{InconclusiveCause, VerdictState};
use duhem_runtime::Engine;
use duhem_schema::{InconclusivePolicy, ManifestDefaults, VerificationDefinition};

const LEAF: &str = r#"
verification: replay effective policy
criteria:
  - id: AC-2
    description: Replay preserves the effective manifest policy.
    checks:
      - id: AC-2.1
        steps:
          - id: command
            uses: cli/invoke
            with:
              command: [sh, -c, 'printf OK']
        assertions:
          - $steps.command.outputs.exit_code == 0
          - matches:
              value: $steps.command.outputs.stdout
              pattern: 'OK'
"#;

#[tokio::test]
async fn effective_policy_survives_bundle_and_ingest_without_softening_failures() {
    let tmp = tempfile::tempdir().unwrap();
    let source = Arc::new(
        SqliteStore::open(tmp.path().join("source.db"))
            .await
            .unwrap(),
    );
    let target = SqliteStore::open(tmp.path().join("target.db"))
        .await
        .unwrap();
    let missing = VerdictState::Inconclusive(InconclusiveCause::MissingObservation);
    // Same construct as the nested defaults-example run: cli/invoke with
    // an unavailable command. AC-2 of the outer CLI suite merely validates
    // that definition; AC-5 runs it.
    for policy in [
        InconclusivePolicy::Block,
        InconclusivePolicy::Warn,
        InconclusivePolicy::Pass,
    ] {
        for (assertion, raw) in [
            ("exit_code == 0", VerdictState::Pass),
            ("exit_code == 1", VerdictState::Fail),
            ("exit_code == 0", missing),
        ] {
            let mut yaml = LEAF.replace("exit_code == 0", assertion);
            if raw == missing {
                yaml = yaml.replace(
                    "[sh, -c, 'printf OK']",
                    "[/duhem-regression-673/nonexistent-command]",
                );
            }
            let def = VerificationDefinition::from_yaml_str(&yaml).unwrap();
            duhem_schema::validate_with_action_catalog(&def, &|uses| {
                duhem_actions::contract_for(uses)
                    .map(|c| c.outputs.iter().map(|o| o.to_string()).collect())
            })
            .unwrap();
            let outcome = Engine::new()
                .with_store(source.clone())
                .with_defaults(&ManifestDefaults {
                    inconclusive_policy: Some(policy),
                    ..Default::default()
                })
                .run_with_metadata(&def, BTreeMap::new())
                .await
                .unwrap();
            let expected = if raw == missing && policy != InconclusivePolicy::Block {
                VerdictState::Pass
            } else {
                raw
            };
            assert_eq!(outcome.verdict.state, expected);
            assert_eq!(outcome.verdict.criteria[0].checks[0].state, raw);
            assert_eq!(
                outcome.warnings.len(),
                usize::from(raw == missing && policy == InconclusivePolicy::Warn)
            );
            let bundle = RunBundle::from_store(source.as_ref(), &outcome.run_id)
                .await
                .unwrap();
            // Exercise the wire boundary rather than sharing runtime state.
            let bundle: RunBundle = serde_json::from_slice(&bundle.wire_bytes().unwrap()).unwrap();
            let trace = Trace::from_events(bundle.events.clone()).unwrap();
            assert_eq!(replay(&trace).unwrap().run, outcome.verdict);
            target
                .ingest_bundle(&bundle, &bundle.content_hash().unwrap())
                .await
                .unwrap();
            let ingested = Trace::from_store(&target, &outcome.run_id).await.unwrap();
            assert_eq!(replay(&ingested).unwrap().run, outcome.verdict);

            // Legacy headers omit the policy. Never infer a permissive
            // policy from a recorded passing verdict.
            let mut legacy_wire = serde_json::to_value(&bundle).unwrap();
            legacy_wire["events"][0]
                .as_object_mut()
                .unwrap()
                .remove("inconclusive_policy");
            let legacy: RunBundle = serde_json::from_value(legacy_wire).unwrap();
            let legacy_trace = Trace::from_events(legacy.events.clone()).unwrap();
            if raw == missing && policy != InconclusivePolicy::Block {
                assert!(replay(&legacy_trace).is_err());
                let rejected = SqliteStore::open(tmp.path().join(format!("legacy-{policy:?}.db")))
                    .await
                    .unwrap();
                assert!(
                    rejected
                        .ingest_bundle(&legacy, &legacy.content_hash().unwrap())
                        .await
                        .is_err()
                );
            } else {
                assert_eq!(replay(&legacy_trace).unwrap().run, outcome.verdict);
            }

            // Ingest must still re-judge and reject an altered criterion.
            let mut tampered = bundle;
            for event in &mut tampered.events {
                if let EventPayload::CriterionFinished { verdict, .. } = &mut event.payload {
                    *verdict = if expected == VerdictState::Fail {
                        VerdictState::Pass
                    } else {
                        VerdictState::Fail
                    };
                }
            }
            assert!(
                target
                    .ingest_bundle(&tampered, &tampered.content_hash().unwrap())
                    .await
                    .is_err()
            );
        }
    }
}
