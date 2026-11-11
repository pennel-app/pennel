use std::sync::Arc;

use pennel_core::PennelError;
use serde_json::Value;
use tauri::{AppHandle, Emitter, State};

use crate::state::Shared;

type Result<T> = std::result::Result<T, String>;

fn keys_after_admission<A, K, E>(
    admission: std::result::Result<A, E>,
    keys: impl FnOnce() -> K,
) -> std::result::Result<(A, K), E> {
    Ok((admission?, keys()))
}

pub(crate) fn metadata(value: &Value) -> Value {
    let mut output = serde_json::Map::new();
    for key in [
        "meeting_uid",
        "revision",
        "job_id",
        "state",
        "physical_running",
    ] {
        if let Some(value) = value.get(key) {
            output.insert(key.into(), value.clone());
        }
    }
    Value::Object(output)
}

pub(crate) fn publish(app: &AppHandle, value: &Value) {
    let _ = app.emit_to("main", "meeting-insights-changed", metadata(value));
}

async fn text(
    shared: Arc<Shared>,
    work: impl FnOnce(&Shared) -> std::result::Result<String, PennelError> + Send + 'static,
) -> Result<String> {
    tauri::async_runtime::spawn_blocking(move || {
        work(&shared).map_err(|error| crate::commands::localize_error_message(error.to_string()))
    })
    .await
    .map_err(|_| "The meeting review worker stopped unexpectedly.".to_string())?
}

async fn response(
    shared: Arc<Shared>,
    work: impl FnOnce(&Shared) -> std::result::Result<String, PennelError> + Send + 'static,
) -> Result<Value> {
    serde_json::from_str(&text(shared, work).await?)
        .map_err(|_| "The meeting review response could not be read.".into())
}

#[tauri::command]
pub async fn insights_snapshot(id: String, shared: State<'_, Arc<Shared>>) -> Result<Value> {
    response(shared.inner().clone(), move |shared| {
        shared.engine.meeting_insights_json(id)
    })
    .await
}

#[tauri::command]
pub async fn insights_availability(
    id: String,
    language_policy: String,
    shared: State<'_, Arc<Shared>>,
) -> Result<Value> {
    response(shared.inner().clone(), move |shared| {
        shared
            .engine
            .meeting_insight_availability_json(id, language_policy)
    })
    .await
}

#[tauri::command]
pub async fn begin_insights_analysis(
    id: String,
    language_policy: String,
    expected_configuration_fingerprint: String,
    app: AppHandle,
    shared: State<'_, Arc<Shared>>,
) -> Result<Value> {
    let value = response(shared.inner().clone(), move |shared| {
        let (admission, keys) = keys_after_admission(
            shared
                .engine
                .clone()
                .admit_meeting_insight_analysis_checked(
                    id,
                    language_policy,
                    expected_configuration_fingerprint,
                ),
            || shared.keys(),
        )?;
        shared
            .engine
            .clone()
            .begin_meeting_insight_analysis_json(admission, keys)
    })
    .await?;
    publish(&app, &value);
    Ok(value)
}

#[tauri::command]
pub async fn insights_job(
    job_id: String,
    app: AppHandle,
    shared: State<'_, Arc<Shared>>,
) -> Result<Value> {
    let value = response(shared.inner().clone(), move |shared| {
        shared.engine.meeting_insight_job_json(job_id)
    })
    .await?;
    publish(&app, &value);
    Ok(value)
}

#[tauri::command]
pub async fn cancel_insights_analysis(
    job_id: String,
    app: AppHandle,
    shared: State<'_, Arc<Shared>>,
) -> Result<Value> {
    let value = response(shared.inner().clone(), move |shared| {
        shared.engine.cancel_meeting_insight_analysis_json(job_id)
    })
    .await?;
    publish(&app, &value);
    Ok(value)
}

async fn mutation(
    app: AppHandle,
    shared: Arc<Shared>,
    work: impl FnOnce(&Shared) -> std::result::Result<String, PennelError> + Send + 'static,
) -> Result<Value> {
    let value = response(shared, work).await?;
    publish(&app, value.get("snapshot").unwrap_or(&value));
    crate::cloud::poke(&app);
    Ok(value)
}

#[tauri::command]
pub async fn review_insight(
    id: String,
    request: Value,
    app: AppHandle,
    shared: State<'_, Arc<Shared>>,
) -> Result<Value> {
    mutation(app, shared.inner().clone(), move |shared| {
        shared
            .engine
            .review_meeting_insight_json(id, request.to_string())
    })
    .await
}

#[tauri::command]
pub async fn claim_insight_edit(
    id: String,
    request: Value,
    app: AppHandle,
    shared: State<'_, Arc<Shared>>,
) -> Result<Value> {
    mutation(app, shared.inner().clone(), move |shared| {
        shared
            .engine
            .claim_meeting_insight_edit_json(id, request.to_string())
    })
    .await
}

#[tauri::command]
pub async fn prepare_insight_insertion(
    id: String,
    request: Value,
    app: AppHandle,
    shared: State<'_, Arc<Shared>>,
) -> Result<Value> {
    mutation(app, shared.inner().clone(), move |shared| {
        shared
            .engine
            .prepare_meeting_insight_insertion_json(id, request.to_string())
    })
    .await
}

#[tauri::command]
pub async fn reconcile_insight_insertions(
    id: String,
    store_revision: u64,
    app: AppHandle,
    shared: State<'_, Arc<Shared>>,
) -> Result<Value> {
    mutation(app, shared.inner().clone(), move |shared| {
        shared
            .engine
            .reconcile_meeting_insight_insertions_json(id, store_revision)
    })
    .await
}

#[tauri::command]
pub async fn resolve_insight_insertion(
    id: String,
    request: Value,
    app: AppHandle,
    shared: State<'_, Arc<Shared>>,
) -> Result<Value> {
    mutation(app, shared.inner().clone(), move |shared| {
        shared
            .engine
            .resolve_meeting_insight_insertion_json(id, request.to_string())
    })
    .await
}

#[tauri::command]
pub async fn reassociate_insight_source(
    id: String,
    request: Value,
    app: AppHandle,
    shared: State<'_, Arc<Shared>>,
) -> Result<Value> {
    mutation(app, shared.inner().clone(), move |shared| {
        shared
            .engine
            .reassociate_meeting_insight_json(id, request.to_string())
    })
    .await
}

#[tauri::command]
pub async fn insights_export(id: String, shared: State<'_, Arc<Shared>>) -> Result<String> {
    text(shared.inner().clone(), move |shared| {
        shared.engine.export_meeting_insights_markdown(id)
    })
    .await
}

#[tauri::command]
pub async fn insight_markdown(
    id: String,
    item_id: String,
    item_revision: u64,
    shared: State<'_, Arc<Shared>>,
) -> Result<String> {
    text(shared.inner().clone(), move |shared| {
        shared
            .engine
            .export_meeting_insight_markdown(id, item_id, item_revision)
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::cell::Cell;

    #[test]
    fn rejected_or_unqualified_admission_never_reads_credentials() {
        let reads = Cell::new(0);
        let result: std::result::Result<((), ()), &str> =
            keys_after_admission(Err("unqualified"), || reads.set(reads.get() + 1));
        assert_eq!(result, Err("unqualified"));
        assert_eq!(reads.get(), 0);
        assert_eq!(
            keys_after_admission::<_, _, &str>(Ok(7), || {
                reads.set(reads.get() + 1);
                11
            }),
            Ok((7, 11))
        );
        assert_eq!(reads.get(), 1);
    }
    #[test]
    fn invalidation_never_broadcasts_proposals_notes_or_source_text() {
        let value = json!({"meeting_uid":"owned","revision":3,"items":[{"text":"private sentinel"}],"notes":"private sentinel", "state":"running"});
        let event = metadata(&value);
        assert_eq!(
            event,
            json!({"meeting_uid":"owned","revision":3,"state":"running"})
        );
        assert!(!event.to_string().contains("sentinel"));
    }
}
