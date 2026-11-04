//! Summary documents and explicitly scoped output for the shared workspace.

use std::sync::Arc;

use pennel_core::{
    InterfaceLanguage, MeetingExport, MeetingOutputScope, MeetingSummaryCandidate,
    MeetingSummaryDocument, MeetingSummaryRecord, MeetingSummaryRevision, ShareableMeetingSummary,
};
use tauri::{AppHandle, Emitter, State};

use crate::{commands, meeting_runtime, state::Shared};

type Result<T> = std::result::Result<T, String>;

async fn background<T: Send + 'static>(
    work: impl FnOnce() -> Result<T> + Send + 'static,
) -> Result<T> {
    tauri::async_runtime::spawn_blocking(work)
        .await
        .map_err(|error| commands::localize_error_message(error.to_string()))?
}

fn error(error: impl std::fmt::Display) -> String {
    commands::localize_error_message(error.to_string())
}

fn changed(app: &AppHandle, id: &str) {
    let _ = app.emit("meeting-updated", id);
    let _ = app.emit("library-changed", ());
    crate::cloud::poke(app);
}

#[tauri::command]
pub async fn meeting_summary(
    id: String,
    shared: State<'_, Arc<Shared>>,
) -> Result<MeetingSummaryDocument> {
    let shared = shared.inner().clone();
    background(move || {
        commands::checked_meeting_dir(&shared.engine, &id)?;
        shared.engine.meeting_summary(id).map_err(error)
    })
    .await
}

#[tauri::command]
pub async fn meeting_summary_prompt_dismissed(
    id: String,
    shared: State<'_, Arc<Shared>>,
) -> Result<bool> {
    let shared = shared.inner().clone();
    background(move || {
        shared
            .engine
            .meeting_summary_prompt_dismissed(id)
            .map_err(error)
    })
    .await
}

#[tauri::command]
pub async fn dismiss_meeting_summary_prompt(
    id: String,
    expected_revision: String,
    expected_source_revision: String,
    app: AppHandle,
    shared: State<'_, Arc<Shared>>,
) -> Result<MeetingSummaryDocument> {
    let shared = shared.inner().clone();
    let updated_id = id.clone();
    let result = background(move || {
        let _job = meeting_runtime::LibraryJob::new(&shared)?;
        commands::checked_meeting_dir(&shared.engine, &id)?;
        commands::require_inactive_meeting(&shared, &id)?;
        shared
            .engine
            .dismiss_meeting_summary_prompt(id, expected_revision, expected_source_revision)
            .map_err(error)
    })
    .await?;
    changed(&app, &updated_id);
    Ok(result)
}

#[tauri::command]
pub async fn meeting_suggestion_prompt_dismissed(
    id: String,
    shared: State<'_, Arc<Shared>>,
) -> Result<bool> {
    let shared = shared.inner().clone();
    background(move || {
        shared
            .engine
            .meeting_suggestion_prompt_dismissed(id)
            .map_err(error)
    })
    .await
}

#[tauri::command]
pub async fn dismiss_meeting_suggestion_prompt(
    id: String,
    expected_revision: String,
    expected_source_revision: String,
    app: AppHandle,
    shared: State<'_, Arc<Shared>>,
) -> Result<MeetingSummaryDocument> {
    let shared = shared.inner().clone();
    let updated_id = id.clone();
    let result = background(move || {
        let _job = meeting_runtime::LibraryJob::new(&shared)?;
        commands::checked_meeting_dir(&shared.engine, &id)?;
        commands::require_inactive_meeting(&shared, &id)?;
        shared
            .engine
            .dismiss_meeting_suggestion_prompt(id, expected_revision, expected_source_revision)
            .map_err(error)
    })
    .await?;
    changed(&app, &updated_id);
    Ok(result)
}

#[tauri::command]
pub async fn meeting_transcript_provenance(
    id: String,
    shared: State<'_, Arc<Shared>>,
) -> Result<Option<pennel_core::MeetingTranscriptProvenance>> {
    let shared = shared.inner().clone();
    background(move || {
        commands::checked_meeting_dir(&shared.engine, &id)?;
        shared
            .engine
            .meeting_transcript_provenance(id)
            .map_err(error)
    })
    .await
}

#[tauri::command]
pub async fn meeting_speaker_timeline(
    id: String,
    shared: State<'_, Arc<Shared>>,
) -> Result<pennel_core::MeetingSpeakerTimeline> {
    let shared = shared.inner().clone();
    background(move || {
        commands::checked_meeting_dir(&shared.engine, &id)?;
        shared.engine.meeting_speaker_timeline(id).map_err(error)
    })
    .await
}

#[tauri::command]
pub async fn save_meeting_summary(
    id: String,
    expected_revision: String,
    expected_source_revision: String,
    markdown: String,
    app: AppHandle,
    shared: State<'_, Arc<Shared>>,
) -> Result<MeetingSummaryDocument> {
    let shared = shared.inner().clone();
    let updated_id = id.clone();
    let result = background(move || {
        let _job = meeting_runtime::LibraryJob::new(&shared)?;
        commands::checked_meeting_dir(&shared.engine, &id)?;
        commands::require_inactive_meeting(&shared, &id)?;
        shared
            .engine
            .save_meeting_summary(id, expected_revision, expected_source_revision, markdown)
            .map_err(error)
    })
    .await?;
    changed(&app, &updated_id);
    Ok(result)
}

#[tauri::command]
pub async fn accept_meeting_summary(
    id: String,
    expected_revision: String,
    expected_source_revision: String,
    markdown: String,
    record: Option<MeetingSummaryRecord>,
    app: AppHandle,
    shared: State<'_, Arc<Shared>>,
) -> Result<MeetingSummaryDocument> {
    let shared = shared.inner().clone();
    let updated_id = id.clone();
    let result = background(move || {
        let _job = meeting_runtime::LibraryJob::new(&shared)?;
        commands::checked_meeting_dir(&shared.engine, &id)?;
        commands::require_inactive_meeting(&shared, &id)?;
        shared
            .engine
            .accept_meeting_summary_record(
                id,
                expected_revision,
                expected_source_revision,
                markdown,
                record,
            )
            .map_err(error)
    })
    .await?;
    changed(&app, &updated_id);
    Ok(result)
}

#[tauri::command]
#[allow(clippy::too_many_arguments)] // Preserve named wire fields plus Tauri-injected app/state.
pub async fn update_meeting_summary_action(
    id: String,
    expected_revision: String,
    expected_source_revision: String,
    action_index: u32,
    done: bool,
    owner: Option<String>,
    app: AppHandle,
    shared: State<'_, Arc<Shared>>,
) -> Result<MeetingSummaryDocument> {
    let shared = shared.inner().clone();
    let updated_id = id.clone();
    let result = background(move || {
        let _job = meeting_runtime::LibraryJob::new(&shared)?;
        commands::checked_meeting_dir(&shared.engine, &id)?;
        commands::require_inactive_meeting(&shared, &id)?;
        shared
            .engine
            .update_meeting_summary_action(
                id,
                expected_revision,
                expected_source_revision,
                action_index,
                done,
                owner,
            )
            .map_err(error)
    })
    .await?;
    changed(&app, &updated_id);
    Ok(result)
}

#[tauri::command]
pub async fn meeting_summary_history(
    id: String,
    shared: State<'_, Arc<Shared>>,
) -> Result<Vec<MeetingSummaryRevision>> {
    let shared = shared.inner().clone();
    background(move || {
        commands::checked_meeting_dir(&shared.engine, &id)?;
        shared.engine.meeting_summary_history(id).map_err(error)
    })
    .await
}

#[tauri::command]
pub async fn restore_meeting_summary(
    id: String,
    revision: String,
    expected_revision: String,
    expected_source_revision: String,
    app: AppHandle,
    shared: State<'_, Arc<Shared>>,
) -> Result<MeetingSummaryDocument> {
    let shared = shared.inner().clone();
    let updated_id = id.clone();
    let result = background(move || {
        let _job = meeting_runtime::LibraryJob::new(&shared)?;
        commands::checked_meeting_dir(&shared.engine, &id)?;
        commands::require_inactive_meeting(&shared, &id)?;
        shared
            .engine
            .restore_meeting_summary(id, revision, expected_revision, expected_source_revision)
            .map_err(error)
    })
    .await?;
    changed(&app, &updated_id);
    Ok(result)
}

#[tauri::command]
pub async fn generate_meeting_summary(
    id: String,
    template_id: Option<String>,
    app: AppHandle,
    shared: State<'_, Arc<Shared>>,
) -> Result<MeetingSummaryCandidate> {
    let shared = shared.inner().clone();
    background(move || {
        let _job = meeting_runtime::LibraryJob::new(&shared)?;
        commands::checked_meeting_dir(&shared.engine, &id)?;
        commands::require_inactive_meeting(&shared, &id)?;
        let template = template_id.unwrap_or_else(|| shared.settings().default_template);
        let keys = shared.keys_for_task(pennel_core::ProviderTask::Llm)?;
        let progress = crate::meeting_postprocess::SummaryProgress::start(&app, &shared, &id);
        let result = shared
            .engine
            .generate_meeting_summary(id, template, keys);
        if let Some(progress) = progress {
            progress.complete(&result);
        }
        result.map_err(error)
    })
    .await
}

#[tauri::command]
pub async fn shareable_meeting_summary(
    id: String,
    shared: State<'_, Arc<Shared>>,
) -> Result<Option<ShareableMeetingSummary>> {
    let shared = shared.inner().clone();
    background(move || {
        commands::checked_meeting_dir(&shared.engine, &id)?;
        shared.engine.shareable_meeting_summary(id).map_err(error)
    })
    .await
}

#[tauri::command]
pub async fn generate_shareable_meeting_summary(
    id: String,
    template_id: Option<String>,
    app: AppHandle,
    shared: State<'_, Arc<Shared>>,
) -> Result<ShareableMeetingSummary> {
    let shared = shared.inner().clone();
    let updated_id = id.clone();
    let result = background(move || {
        let _job = meeting_runtime::LibraryJob::new(&shared)?;
        commands::checked_meeting_dir(&shared.engine, &id)?;
        commands::require_inactive_meeting(&shared, &id)?;
        let template = template_id.unwrap_or_else(|| shared.settings().default_template);
        let keys = shared.keys_for_task(pennel_core::ProviderTask::Llm)?;
        shared
            .engine
            .generate_shareable_meeting_summary(id, template, keys)
            .map_err(error)
    })
    .await?;
    changed(&app, &updated_id);
    Ok(result)
}

#[tauri::command]
pub async fn preview_meeting_export(
    id: String,
    scope: MeetingOutputScope,
    include_transcript: bool,
    language: Option<String>,
    shared: State<'_, Arc<Shared>>,
) -> Result<MeetingExport> {
    let shared = shared.inner().clone();
    background(move || {
        let _job = meeting_runtime::LibraryJob::new(&shared)?;
        commands::checked_meeting_dir(&shared.engine, &id)?;
        let language = match language.as_deref() {
            Some("en") => InterfaceLanguage::English,
            Some("nl") => InterfaceLanguage::Dutch,
            None => shared.interface_language(),
            Some(_) => return Err(error("Unsupported interface language.")),
        };
        shared
            .engine
            .preview_meeting_export(id, scope, include_transcript, language)
            .map_err(error)
    })
    .await
}
