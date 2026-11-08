//! Preview-only profile controls and idle background analysis. Content never enters events or logs.

use std::sync::atomic::Ordering;
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use pennel_core::{PennelError, RecordingPhase};
use serde::Serialize;
use serde_json::Value;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::cloud::CloudError;
use crate::state::Shared;

pub const CHANGED: &str = "writing-profile-changed";
const INTERVAL: Duration = Duration::from_secs(60);

#[derive(Default)]
struct Signals {
    stop: bool,
    poked: bool,
}

pub struct ProfileRunner {
    signals: Arc<(Mutex<Signals>, Condvar)>,
}

fn idle(shared: &Shared) -> bool {
    !shared.quitting.load(Ordering::Acquire)
        && !shared.dictation_active.load(Ordering::Acquire)
        && shared.dictation_jobs.load(Ordering::Acquire) == 0
        && shared.library_jobs.load(Ordering::Acquire) == 0
        && !shared.live_questions.lock().unwrap().is_active()
        && shared.recording.lock().unwrap().snapshot().phase == RecordingPhase::Idle
}

impl ProfileRunner {
    pub fn start(shared: Arc<Shared>, app: AppHandle) -> Self {
        let signals = Arc::new((Mutex::new(Signals::default()), Condvar::new()));
        let retained = signals.clone();
        std::thread::spawn(move || {
            loop {
                if retained.0.lock().unwrap().stop {
                    return;
                }
                if shared.preview
                    && idle(&shared)
                    && shared.engine.writing_profile_learning_enabled()
                {
                    let _ = shared.engine.refresh_writing_profile();
                    let _ = app.emit(CHANGED, ());
                }
                let mut state = retained.0.lock().unwrap();
                if !state.stop && !state.poked {
                    state = retained.1.wait_timeout(state, INTERVAL).unwrap().0;
                }
                if state.stop {
                    return;
                }
                state.poked = false;
            }
        });
        Self { signals }
    }

    pub fn poke(&self) {
        self.signals.0.lock().unwrap().poked = true;
        self.signals.1.notify_all();
    }

    pub fn stop(&self) {
        self.signals.0.lock().unwrap().stop = true;
        self.signals.1.notify_all();
    }
}

fn changed(app: &AppHandle) {
    let _ = app.emit(CHANGED, ());
    if let Some(runner) = app.try_state::<ProfileRunner>() {
        runner.poke();
    }
    crate::claude_access::poke(app);
}

async fn profile_work(
    app: AppHandle,
    shared: Arc<Shared>,
    work: impl FnOnce(&Shared) -> Result<Value, PennelError> + Send + 'static,
) -> Result<Value, CloudError> {
    let value = tauri::async_runtime::spawn_blocking(move || work(&shared))
        .await
        .map_err(CloudError::other)?
        .map_err(CloudError::from)?;
    changed(&app);
    Ok(value)
}

pub(crate) fn profile_language(arguments: &str) -> Option<String> {
    if arguments.len() > 256 {
        return None;
    }
    let value: Value = serde_json::from_str(arguments).ok()?;
    let object = value.as_object()?;
    if object.len() != 1 {
        return None;
    }
    let language = object.get("language")?.as_str()?;
    let parts: Vec<_> = language.split('-').collect();
    if language.len() > 35
        || !(2..=3).contains(&parts[0].len())
        || !parts[0].bytes().all(|byte| byte.is_ascii_alphabetic())
        || parts.iter().skip(1).any(|part| {
            part.is_empty()
                || part.len() > 8
                || !part.bytes().all(|byte| byte.is_ascii_alphanumeric())
        })
    {
        return None;
    }
    Some(language.to_owned())
}

#[tauri::command]
pub async fn writing_profile_status(shared: State<'_, Arc<Shared>>) -> Result<Value, CloudError> {
    let shared = shared.inner().clone();
    tauri::async_runtime::spawn_blocking(move || shared.engine.writing_profile_status())
        .await
        .map_err(CloudError::other)?
        .map_err(CloudError::from)
}

#[tauri::command]
pub async fn set_writing_profile_learning(
    enabled: bool,
    app: AppHandle,
    shared: State<'_, Arc<Shared>>,
) -> Result<Value, CloudError> {
    profile_work(app, shared.inner().clone(), move |shared| {
        shared.engine.set_writing_profile_learning(enabled)
    })
    .await
}

#[tauri::command]
pub async fn refresh_writing_profile(
    app: AppHandle,
    shared: State<'_, Arc<Shared>>,
) -> Result<Value, CloudError> {
    profile_work(app, shared.inner().clone(), move |shared| {
        if !idle(shared) {
            return Err(PennelError::Cancelled);
        }
        shared.engine.refresh_writing_profile()
    })
    .await
}

#[tauri::command]
pub async fn set_writing_profile_source_excluded(
    source_id: String,
    excluded: bool,
    app: AppHandle,
    shared: State<'_, Arc<Shared>>,
) -> Result<Value, CloudError> {
    profile_work(app, shared.inner().clone(), move |shared| {
        shared
            .engine
            .set_writing_profile_source_excluded(&source_id, excluded)
    })
    .await
}

#[tauri::command]
pub async fn set_writing_profile_trait(
    language: String,
    trait_key: String,
    value: Option<String>,
    app: AppHandle,
    shared: State<'_, Arc<Shared>>,
) -> Result<Value, CloudError> {
    profile_work(app, shared.inner().clone(), move |shared| {
        shared
            .engine
            .set_writing_profile_trait(&language, &trait_key, value.as_deref())
    })
    .await
}

#[tauri::command]
pub async fn delete_writing_profile(
    app: AppHandle,
    shared: State<'_, Arc<Shared>>,
) -> Result<Value, CloudError> {
    profile_work(app, shared.inner().clone(), delete_profile_state).await
}

fn delete_profile_state(shared: &Shared) -> Result<Value, PennelError> {
    let (operation, value) = {
        let mut epoch = shared.writing_profile_sharing_epoch.lock().unwrap();
        *epoch = epoch.wrapping_add(1);
        (*epoch, shared.engine.delete_writing_profile()?)
    };
    let _source = shared.writing_profile_source_gate.lock().unwrap();
    if *shared.writing_profile_sharing_epoch.lock().unwrap() == operation
        && shared.engine.cloud_signed_in()
    {
        let _ = shared.engine.cloud_writing_style_source(Some(false));
    }
    Ok(value)
}

fn change_sharing(
    shared: &Shared,
    enabled: bool,
    select_source: impl FnOnce(bool) -> Result<bool, PennelError>,
) -> Result<(), PennelError> {
    if !shared.preview {
        return Err(PennelError::Cancelled);
    }
    let account = shared.engine.cloud_account_epoch();
    let operation = {
        let mut epoch = shared.writing_profile_sharing_epoch.lock().unwrap();
        *epoch = epoch.wrapping_add(1);
        // Disabling never waits for a pending designation network request.
        if !enabled {
            shared.engine.set_writing_profile_sharing(false)?;
        }
        *epoch
    };
    // Network mutations run in order; local disable/reset can invalidate a pending enable.
    let _source = shared.writing_profile_source_gate.lock().unwrap();
    if *shared.writing_profile_sharing_epoch.lock().unwrap() != operation {
        return Err(PennelError::Cancelled);
    }
    let selected = select_source(enabled);
    if !enabled {
        return Ok(());
    }
    if !selected? {
        return Err(PennelError::Cancelled);
    }
    let epoch = shared.writing_profile_sharing_epoch.lock().unwrap();
    if *epoch != operation || account != shared.engine.cloud_account_epoch() {
        return Err(PennelError::Cancelled);
    }
    shared
        .engine
        .enable_writing_profile_sharing_for_account(account)?;
    if account != shared.engine.cloud_account_epoch() {
        shared.engine.set_writing_profile_sharing(false)?;
        return Err(PennelError::Cancelled);
    }
    Ok(())
}

#[derive(Serialize)]
pub struct ProfileConnection {
    enabled: bool,
    signed_in: bool,
    designated: bool,
    mcp_url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<&'static str>,
}

fn connection(shared: &Shared) -> ProfileConnection {
    let signed_in = shared.engine.cloud_signed_in();
    let result = if shared.preview && signed_in {
        shared.engine.cloud_writing_style_source(None)
    } else {
        Ok(false)
    };
    ProfileConnection {
        enabled: shared.preview && shared.engine.writing_profile_sharing_enabled(),
        signed_in,
        designated: result.as_ref().copied().unwrap_or(false),
        mcp_url: shared.engine.cloud_writing_style_mcp_url(),
        error: result.is_err().then_some("connection_unavailable"),
    }
}

#[tauri::command]
pub async fn writing_profile_connection(
    shared: State<'_, Arc<Shared>>,
) -> Result<ProfileConnection, CloudError> {
    let shared = shared.inner().clone();
    tauri::async_runtime::spawn_blocking(move || connection(&shared))
        .await
        .map_err(CloudError::other)
}

#[tauri::command]
pub async fn set_writing_profile_sharing(
    enabled: bool,
    app: AppHandle,
    shared: State<'_, Arc<Shared>>,
) -> Result<ProfileConnection, CloudError> {
    let shared = shared.inner().clone();
    let value =
        tauri::async_runtime::spawn_blocking(move || -> Result<ProfileConnection, PennelError> {
            change_sharing(&shared, enabled, |enabled| {
                shared.engine.cloud_writing_style_source(Some(enabled))
            })?;
            Ok(connection(&shared))
        })
        .await
        .map_err(CloudError::other)?
        .map_err(CloudError::from)?;
    changed(&app);
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_arguments_are_bounded_and_cannot_select_accounts_or_sources() {
        assert_eq!(
            profile_language(r#"{"language":"nl-BE"}"#).as_deref(),
            Some("nl-BE")
        );
        assert_eq!(
            profile_language(r#"{"language":"es-419"}"#).as_deref(),
            Some("es-419")
        );
        for arguments in [
            r#"{}"#,
            r#"{"language":"en","account":"other"}"#,
            r#"{"language":"../private"}"#,
            r#"{"language":""}"#,
            r#"{"language":7}"#,
            r#"{"language":"en--US"}"#,
        ] {
            assert!(profile_language(arguments).is_none(), "{arguments}");
        }
    }

    #[test]
    fn background_work_yields_to_dictation_recording_and_shutdown() {
        let directory = tempfile::tempdir().unwrap();
        let shared = Shared::testing(directory.path().to_path_buf());
        assert!(idle(&shared));
        shared.dictation_active.store(true, Ordering::Release);
        assert!(!idle(&shared));
        shared.dictation_active.store(false, Ordering::Release);
        shared.recording.lock().unwrap().begin(None).unwrap();
        assert!(!idle(&shared));
    }

    #[test]
    fn delayed_enable_cannot_undo_a_later_disable_or_profile_deletion() {
        use std::sync::mpsc;
        use std::time::Instant;
        for delete in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let shared = Shared::testing(directory.path().to_path_buf());
            shared.engine.set_writing_profile_learning(true).unwrap();
            let (started, wait_started) = mpsc::channel();
            let (release, wait_release) = mpsc::channel();
            let enabling = shared.clone();
            let enable = std::thread::spawn(move || {
                change_sharing(&enabling, true, |_| {
                    started.send(()).unwrap();
                    wait_release.recv().unwrap();
                    Ok(true)
                })
            });
            wait_started.recv_timeout(Duration::from_secs(5)).unwrap();
            let disabling = shared.clone();
            let stop = std::thread::spawn(move || {
                if delete {
                    delete_profile_state(&disabling).map(|_| ())
                } else {
                    change_sharing(&disabling, false, |_| Ok(false))
                }
            });
            let deadline = Instant::now() + Duration::from_secs(5);
            while *shared.writing_profile_sharing_epoch.lock().unwrap() < 2 {
                assert!(Instant::now() < deadline);
                std::thread::sleep(Duration::from_millis(2));
            }
            assert!(!shared.engine.writing_profile_sharing_enabled());
            release.send(()).unwrap();
            assert!(enable.join().unwrap().is_err());
            stop.join().unwrap().unwrap();
            assert!(!shared.engine.writing_profile_sharing_enabled());
            if delete {
                assert!(!shared.engine.writing_profile_learning_enabled());
            }
        }
    }
}
