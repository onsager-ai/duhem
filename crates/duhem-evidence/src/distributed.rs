//! Stable identities for distributed plans; no machine-local input is hashed.
use sha2::{Digest, Sha256};

pub fn manifest_digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

/// Namespaced hash encoded as a valid ULID. It identifies a logical run, so its
/// timestamp portion is opaque, rather than the start time of any one machine.
pub fn logical_run_id(identity: &str) -> String {
    let mut hash = Sha256::new();
    hash.update(b"duhem:distributed-run:v1\0");
    hash.update(identity.as_bytes());
    let digest = hash.finalize();
    let mut bytes = [0; 16];
    bytes.copy_from_slice(&digest[..16]);
    ulid::Ulid::from_bytes(bytes).to_string()
}

/// Sort stable unit IDs and assign round-robin. This balances unit counts to
/// within one without depending on process hashes, manifest glob order or paths
/// outside the manifest root. IDs must be unique within the selected run.
pub fn partition(
    units: &[String],
    index: usize,
    total: usize,
) -> std::collections::BTreeSet<String> {
    assert!(index > 0 && index <= total);
    let ordered: std::collections::BTreeSet<_> = units.iter().cloned().collect();
    ordered
        .into_iter()
        .enumerate()
        .filter_map(|(offset, unit)| (offset % total == index - 1).then_some(unit))
        .collect()
}
