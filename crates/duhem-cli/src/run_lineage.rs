//! Nested-run process context and evidence-store resolution (#348).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use duhem_evidence::{RunScope, SqliteStore, Store};

pub(crate) struct InvocationParent {
    pub run_id: String,
    pub scope: RunScope,
}

pub(crate) struct RunStore {
    pub db_path: PathBuf,
    pub store: Arc<dyn Store>,
    pub parent: Option<InvocationParent>,
}

pub(crate) async fn open(explicit_db: Option<&Path>) -> Result<RunStore, String> {
    let parent_id = std::env::var(duhem_runtime::PARENT_RUN_ID_ENV)
        .ok()
        .filter(|value| !value.is_empty());
    let parent_db = parent_id.as_ref().and_then(|_| {
        std::env::var_os(duhem_runtime::PARENT_DB_PATH_ENV)
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
    });
    // Nested runs cannot detach from their parent's verification act:
    // the inherited path wins over an invoked argv's explicit --db.
    let db_path = match (parent_db, explicit_db) {
        (Some(path), _) => path,
        (None, Some(path)) => path.to_path_buf(),
        (None, None) => {
            let cwd = std::env::current_dir()
                .map_err(|e| format!("cannot determine current directory: {e}"))?;
            duhem_evidence::project_db_path(&cwd).map_err(|e| format!("resolve store: {e}"))?
        }
    };
    let store: Arc<dyn Store> = Arc::new(
        SqliteStore::open(&db_path)
            .await
            .map_err(|e| format!("open store {}: {e}", db_path.display()))?,
    );
    let parent = match parent_id {
        Some(run_id) => {
            let record = store
                .get_run(&run_id)
                .await
                .map_err(|e| format!("read parent run `{run_id}`: {e}"))?
                .ok_or_else(|| {
                    format!(
                        "{} names unknown parent run `{run_id}` in {}",
                        duhem_runtime::PARENT_RUN_ID_ENV,
                        db_path.display()
                    )
                })?;
            Some(InvocationParent {
                run_id,
                scope: record.scope,
            })
        }
        None => None,
    };
    Ok(RunStore {
        db_path,
        store,
        parent,
    })
}
