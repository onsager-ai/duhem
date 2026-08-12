//! Process context for nested `duhem run` invocations (#348).

use std::collections::BTreeMap;
use std::path::Path;

/// Public lineage token consumed by `duhem run`.
pub const PARENT_RUN_ID_ENV: &str = "DUHEM_PARENT_RUN_ID";
/// Internal companion that keeps a nested run in the same append-only
/// store as its parent, making the parent foreign key resolvable.
pub const PARENT_DB_PATH_ENV: &str = "DUHEM_PARENT_DB_PATH";

pub(crate) fn child_process_env(run_id: &str, db_path: Option<&Path>) -> BTreeMap<String, String> {
    let mut env = BTreeMap::new();
    env.insert(PARENT_RUN_ID_ENV.to_string(), run_id.to_string());
    if let Some(path) = db_path {
        env.insert(
            PARENT_DB_PATH_ENV.to_string(),
            path.to_string_lossy().into_owned(),
        );
    }
    env
}
