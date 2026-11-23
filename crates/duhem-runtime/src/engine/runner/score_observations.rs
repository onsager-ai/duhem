//! Resolve only the observation sources explicitly authored on a score step.
use std::path::{Component, Path};

use duhem_evidence::EvidenceWriter;
use duhem_schema::score::{ObservationSource, observation_source};
use serde_yml::Value;

pub(super) async fn resolve(
    authored: &Value,
    resolved: &mut Value,
    writer: &EvidenceWriter,
) -> Result<(), String> {
    let authored = authored
        .get("observations")
        .and_then(Value::as_sequence)
        .ok_or("missing observations list")?;
    let Some(Value::Sequence(resolved)) = resolved
        .as_mapping_mut()
        .and_then(|map| map.get_mut(Value::String("observations".into())))
    else {
        return Err("missing observations list".into());
    };
    for (source, value) in authored.iter().zip(resolved) {
        let content = match observation_source(source).ok_or("invalid observation source")? {
            ObservationSource::StepOutput => continue,
            ObservationSource::File(path) => read_workspace_file(path).await?,
            ObservationSource::Evidence(sha) => {
                let store = writer.store();
                let events = store
                    .run_events(writer.run_id())
                    .await
                    .map_err(|_| "could not read current-run evidence")?;
                if !duhem_evidence::bundle::referenced_blobs(&events)
                    .iter()
                    .any(|hash| hash == sha)
                {
                    return Err("observation blob is not referenced by current-run evidence".into());
                }
                let bytes = store
                    .get_blob(sha)
                    .await
                    .map_err(|_| "could not read observation blob")?
                    .ok_or("observation blob is missing")?;
                String::from_utf8(bytes).map_err(|_| "observation blob is not UTF-8 text")?
            }
        };
        *value = Value::String(content);
    }
    Ok(())
}

async fn read_workspace_file(path: &str) -> Result<String, String> {
    let path = Path::new(path);
    if path
        .components()
        .any(|part| !matches!(part, Component::Normal(_) | Component::CurDir))
    {
        return Err("observation file must be relative to the working directory".into());
    }
    let root = std::env::current_dir().map_err(|_| "could not locate the working directory")?;
    let file = tokio::fs::canonicalize(root.join(path))
        .await
        .map_err(|_| "could not resolve observation file")?;
    if !file.starts_with(&root)
        || !tokio::fs::metadata(&file)
            .await
            .is_ok_and(|meta| meta.is_file())
    {
        return Err("observation file must be a regular file inside the working directory".into());
    }
    tokio::fs::read_to_string(file)
        .await
        .map_err(|_| "could not read UTF-8 observation file".into())
}
