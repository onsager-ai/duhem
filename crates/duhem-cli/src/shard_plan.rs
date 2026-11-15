//! CLI projection of a distributed plan onto the existing selection machinery.
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::Arc;

use duhem_evidence::{EvidenceWriter, RunLineage, RunOrigin, Store};
use duhem_runtime::CheckFilter;
use duhem_schema::{Loaded, LoadedLeaf, NativeRun};
use duhem_summary::{DistributedRun, ExpectedPartition, ManifestSelection};

use crate::filter::{CliCheckFilter, native_selection};
use crate::run_scope::Scope;
use crate::shard::{Detection, partition};

pub struct Plan {
    pub metadata: DistributedRun,
    owned: BTreeSet<String>,
}

/// JSON encoding frames identifiers without relying on an escaping convention.
fn criterion_id(leaf: &str, criterion: &str) -> String {
    serde_json::to_string(&("criterion", leaf, criterion)).expect("unit serializes")
}
fn native_id(leaf: &str, native: &str) -> String {
    serde_json::to_string(&("native", leaf, native)).expect("unit serializes")
}
fn leaf_key(path: &Path, root: &Path) -> String {
    // Glob expansion may return canonical absolute paths even when the CLI
    // target was relative. Resolve both sides before dropping the root.
    let path = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let root = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    path.strip_prefix(&root)
        .unwrap_or(&path)
        .to_string_lossy()
        .replace('\\', "/")
}

/// Hash parsed definitions as loaded, before selection and before machine-local
/// input resolution. Absolute source paths and runtime inputs are not included.
pub fn digest(loaded: &Loaded) -> Result<String, serde_json::Error> {
    let value = match loaded {
        Loaded::Leaf { definition, .. } => serde_json::to_value(definition)?,
        Loaded::Manifest {
            manifest,
            manifest_path,
            leaves,
            ..
        } => {
            let root = manifest_path.parent().unwrap_or(Path::new("."));
            let mut definitions = BTreeMap::new();
            for leaf in leaves {
                definitions.insert(
                    leaf_key(&leaf.path, root),
                    serde_json::to_value(&leaf.definition)?,
                );
            }
            serde_json::json!({"manifest": manifest, "leaves": definitions})
        }
    };
    Ok(duhem_evidence::distributed::manifest_digest(
        &serde_json::to_vec(&value)?,
    ))
}

impl Plan {
    pub fn new(
        detection: Detection,
        digest: String,
        leaves: &[LoadedLeaf],
        scope: &Scope,
        filter: Option<&CliCheckFilter>,
    ) -> Self {
        let mut units = Vec::new();
        for leaf in leaves {
            let name = crate::run_cmd::leaf_name(&leaf.path);
            let key = Self::key(&leaf.path, scope);
            let criteria = filter
                .and_then(|f| f.for_criteria())
                .and_then(|f| f.for_verification(&name));
            for criterion in &leaf.definition.criteria {
                if criterion.checks.iter().any(|check| match filter {
                    None => true,
                    Some(_) => criteria
                        .as_ref()
                        .is_some_and(|f| f.matches(&criterion.id, &check.id)),
                }) {
                    units.push(criterion_id(&key, &criterion.id));
                }
            }
            let (native, _) = native_selection(filter, &leaf.definition.native);
            units.extend(native.iter().map(|run| native_id(&key, &run.id)));
        }
        if let Scope::Manifest { native, .. } = scope {
            let (native, _) = native_selection(filter, native);
            units.extend(native.iter().map(|run| native_id("$manifest", &run.id)));
        }
        let owned = partition(&units, detection.shard.index, detection.shard.total);
        let metadata = DistributedRun {
            origin: Some("shard".into()),
            parent_run_id: Some(duhem_evidence::distributed::logical_run_id(
                &detection.identity,
            )),
            shard: Some(detection.shard),
            manifest_digest: Some(digest),
            expected_partition: Some(ExpectedPartition {
                units: owned.iter().cloned().collect(),
                total_units: units.len(),
                excluded: units
                    .iter()
                    .filter(|unit| !owned.contains(*unit))
                    .map(|unit| duhem_summary::ExcludedUnit {
                        unit: unit.clone(),
                        reason: "shard".into(),
                    })
                    .collect(),
            }),
        };
        Self { metadata, owned }
    }

    pub fn key(path: &Path, scope: &Scope) -> String {
        match scope {
            Scope::Manifest { manifest_dir, .. } => {
                leaf_key(path, manifest_dir.as_deref().unwrap_or(Path::new(".")))
            }
            Scope::SingleLeaf => "$leaf".into(),
        }
    }
    pub fn owns_criterion(&self, key: &str, criterion: &str) -> bool {
        self.owned.contains(&criterion_id(key, criterion))
    }
    pub fn criteria(&self, key: &str, ids: impl Iterator<Item = String>) -> BTreeSet<String> {
        ids.filter(|id| self.owned.contains(&criterion_id(key, id)))
            .collect()
    }
    pub fn native(&self, key: &str, runs: &mut Vec<NativeRun>, excluded: &mut Vec<String>) {
        runs.retain(|run| {
            if self.owned.contains(&native_id(key, &run.id)) {
                true
            } else {
                excluded.push(run.id.clone());
                false
            }
        });
    }
    pub fn narrow_selection(
        &self,
        leaves: &[LoadedLeaf],
        scope: &Scope,
        selection: &mut Option<ManifestSelection>,
    ) {
        if let Some(selection) = selection {
            selection.distributed = self.metadata.clone();
            for leaf in leaves {
                let key = Self::key(&leaf.path, scope);
                let owns = self.owned.iter().any(|id| {
                    let parts: Vec<String> = serde_json::from_str(id).expect("generated unit id");
                    parts[1] == key
                });
                let path = leaf.path.display().to_string();
                if !owns
                    && !selection
                        .excluded
                        .iter()
                        .any(|excluded| excluded.verification == path)
                {
                    selection.selected -= 1;
                    selection.excluded.push(duhem_summary::ExcludedLeaf {
                        verification: path,
                        reason: "shard".into(),
                    });
                }
            }
        }
    }

    /// A local unjudged logical parent satisfies the SQLite FK. It stays open:
    /// only the later server aggregation can judge/finish the whole run.
    pub async fn ensure_parent(
        &self,
        store: Arc<dyn Store>,
        path: &Path,
    ) -> Result<(), duhem_evidence::WriterError> {
        let id = self
            .metadata
            .parent_run_id
            .as_deref()
            .expect("shard parent");
        if store.get_run(id).await?.is_none() {
            let mut writer =
                EvidenceWriter::begin(store, id, &path.to_string_lossy(), BTreeMap::new()).await?;
            writer
                .append(duhem_evidence::run_started(
                    path.to_string_lossy(),
                    BTreeMap::new(),
                ))
                .await?;
            writer.finish().await?;
        }
        Ok(())
    }
    pub fn lineage(&self) -> RunLineage {
        RunLineage {
            parent_run_id: self.metadata.parent_run_id.clone(),
            origin: Some(RunOrigin::Shard),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn selection_partition_covers_varied_loaded_manifests() {
        for leaves_count in [1, 2, 5] {
            let leaves: Vec<_> = (0..leaves_count).map(|i| LoadedLeaf {
                path: format!("leaf-{i}.yml").into(),
                definition: duhem_schema::VerificationDefinition::from_yaml_str(
                    "verification: partition\ncriteria:\n  - id: A\n    description: first\n    checks: [{id: a1, assertions: ['true']}, {id: a2, assertions: ['true']}]\n  - id: B\n    description: second\n    checks: [{id: b1, assertions: ['true']}]\n").unwrap(),
            }).collect();
            let scope = Scope::Manifest {
                warnings: Vec::new(),
                provision: None,
                native: Vec::new(),
                manifest_dir: Some(".".into()),
                defaults: None,
                project: None,
                inputs: BTreeMap::new(),
            };
            for filter in [
                None,
                Some(CliCheckFilter::parse(&["A::a2".into()]).unwrap()),
            ] {
                let expected = leaves_count * if filter.is_some() { 1 } else { 2 };
                for n in 1..=7 {
                    let mut union = BTreeSet::new();
                    for k in 1..=n {
                        let detection =
                            crate::shard::detect(Some(&format!("{k}/{n}")), Some("shared"), |_| {
                                None
                            })
                            .unwrap()
                            .unwrap();
                        let plan =
                            Plan::new(detection, "digest".into(), &leaves, &scope, filter.as_ref());
                        let partition = plan.metadata.expected_partition.unwrap();
                        assert_eq!(partition.total_units, expected);
                        for unit in partition.units {
                            assert!(union.insert(unit));
                        }
                    }
                    assert_eq!(union.len(), expected);
                }
            }
        }
    }

    #[test]
    fn digest_and_unit_paths_are_stable_across_checkout_roots_and_globs() {
        let mut digests = Vec::new();
        let mut plans = Vec::new();
        for _ in 0..2 {
            let tmp = tempfile::tempdir().unwrap();
            std::fs::write(tmp.path().join("leaf.yml"), "verification: stable\ncriteria:\n  - id: AC\n    description: stable\n    checks: [{id: body, assertions: ['true']}]\n").unwrap();
            let path = tmp.path().join("duhem.yml");
            std::fs::write(
                &path,
                "manifest_version: 1\nverifications: [{glob: 'leaf*.yml'}]\n",
            )
            .unwrap();
            let loaded = duhem_schema::load_for_run(&path).unwrap();
            digests.push(digest(&loaded).unwrap());
            let Loaded::Manifest { leaves, .. } = loaded else {
                panic!("manifest")
            };
            let scope = Scope::Manifest {
                warnings: Vec::new(),
                provision: None,
                native: Vec::new(),
                manifest_dir: Some(tmp.path().into()),
                defaults: None,
                project: None,
                inputs: BTreeMap::new(),
            };
            let detection = crate::shard::detect(Some("1/2"), Some("shared"), |_| None)
                .unwrap()
                .unwrap();
            let plan = Plan::new(
                detection,
                digests.last().unwrap().clone(),
                &leaves,
                &scope,
                None,
            );
            plans.push(plan.metadata.expected_partition.unwrap());
            std::fs::write(tmp.path().join("leaf.yml"), "verification: changed\ncriteria:\n  - id: AC\n    description: changed\n    checks: [{id: body, assertions: ['true']}]\n").unwrap();
            assert_ne!(
                digest(&duhem_schema::load_for_run(&path).unwrap()).unwrap(),
                *digests.last().unwrap()
            );
        }
        assert_eq!(digests[0], digests[1]);
        assert_eq!(plans[0], plans[1]);
        assert!(plans[0].units[0].contains("leaf.yml"));
    }
}

/// Conjunction preserves the user's check selection inside each owned criterion.
#[derive(Clone)]
pub struct CriterionFilter {
    pub original: Option<CliCheckFilter>,
    pub owned: BTreeSet<String>,
}
impl CheckFilter for CriterionFilter {
    fn matches(&self, criterion: &str, check: &str) -> bool {
        self.owned.contains(criterion)
            && self
                .original
                .as_ref()
                .is_none_or(|filter| filter.matches(criterion, check))
    }
    fn includes_criterion(&self, criterion: &str) -> bool {
        self.owned.contains(criterion)
    }
    fn exclusion_reason(&self, criterion: &str, check: &str) -> &'static str {
        if self
            .original
            .as_ref()
            .is_some_and(|filter| !filter.matches(criterion, check))
        {
            "check_filter"
        } else {
            "shard"
        }
    }
}
