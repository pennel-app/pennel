//! Imports the pre-Pennel data without moving the only copy or replacing existing Pennel stores.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use pennel_core::PennelEngine;
use serde::{Deserialize, Serialize};

use crate::secrets::Secrets;

const VERSION: u32 = 1;
const MARKER: &str = "migrated-from-musli";
const STAGING: &str = ".pennel-musli-import";
const STAGE_OWNER: &str = ".pennel-legacy-stage.json";

#[derive(Debug)]
pub enum MigrationError {
    Io(std::io::Error),
    InvalidMarker,
    Busy,
    SourceChanged,
    UnsupportedEntry,
    Credentials(String),
    Settings(String),
}

impl std::fmt::Display for MigrationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "Could not import Musli data: {error}"),
            Self::InvalidMarker => formatter.write_str("The Musli migration is incomplete or requires another app version. The original data has been preserved."),
            Self::Busy => formatter.write_str("Another Pennel app is importing the Musli data. Close it and try again."),
            Self::SourceChanged => formatter.write_str("The Musli data changed while being copied. Close Musli and try again. Your original data has been kept."),
            Self::UnsupportedEntry => formatter.write_str("The Musli folder contains a linked or special file. Your data is unchanged; import the folder manually."),
            Self::Credentials(message) | Self::Settings(message) => formatter.write_str(message),
        }
    }
}

impl std::error::Error for MigrationError {}

impl From<std::io::Error> for MigrationError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct Marker {
    version: u32,
    source: PathBuf,
    destination: PathBuf,
    legacy_default_meetings: Option<PathBuf>,
}

#[derive(Debug, PartialEq, Eq)]
struct Stamp {
    path: PathBuf,
    length: u64,
    modified: SystemTime,
}

fn nonce() -> String {
    format!(
        "{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    )
}

fn marker(source: &Path, destination: &Path, home: &Path) -> Marker {
    let meetings = home.join("Musli");
    Marker {
        version: VERSION,
        source: source.to_owned(),
        destination: destination.to_owned(),
        legacy_default_meetings: (meetings.is_dir() && !home.join("Pennel").exists())
            .then_some(meetings),
    }
}

fn write_marker(path: &Path, marker: &Marker) -> Result<(), MigrationError> {
    let bytes = serde_json::to_vec_pretty(marker).map_err(|_| MigrationError::InvalidMarker)?;
    let temporary = path.with_file_name(format!(".{MARKER}-{}.tmp", nonce()));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    drop(file);
    fs::rename(temporary, path)?;
    Ok(())
}

fn read_marker(path: &Path) -> Result<Option<Marker>, MigrationError> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let marker: Marker =
        serde_json::from_slice(&bytes).map_err(|_| MigrationError::InvalidMarker)?;
    if marker.version != VERSION {
        return Err(MigrationError::InvalidMarker);
    }
    Ok(Some(marker))
}

fn validate_marker(
    state: &Marker,
    source: &Path,
    destination: &Path,
    home: &Path,
) -> Result<(), MigrationError> {
    if state.source != source
        || state.destination != destination
        || state
            .legacy_default_meetings
            .as_ref()
            .is_some_and(|directory| directory != &home.join("Musli"))
    {
        return Err(MigrationError::InvalidMarker);
    }
    Ok(())
}

fn has_pennel_data(directory: &Path) -> Result<bool, MigrationError> {
    if !directory.exists() {
        return Ok(false);
    }
    if fs::symlink_metadata(directory)?.file_type().is_symlink() {
        return Ok(true);
    }
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        if entry.file_name() != "logs" && entry.file_name() != ".pennel.lock" {
            return Ok(true);
        }
    }
    Ok(false)
}

fn stamps(root: &Path) -> Result<Vec<Stamp>, MigrationError> {
    fn visit(root: &Path, directory: &Path, result: &mut Vec<Stamp>) -> Result<(), MigrationError> {
        for entry in fs::read_dir(directory)? {
            let entry = entry?;
            let kind = entry.file_type()?;
            if kind.is_dir() {
                visit(root, &entry.path(), result)?;
            } else if kind.is_file() {
                let metadata = entry.metadata()?;
                result.push(Stamp {
                    path: entry
                        .path()
                        .strip_prefix(root)
                        .map_err(|_| MigrationError::UnsupportedEntry)?
                        .to_owned(),
                    length: metadata.len(),
                    modified: metadata.modified()?,
                });
            } else {
                return Err(MigrationError::UnsupportedEntry);
            }
        }
        Ok(())
    }
    let mut result = Vec::new();
    visit(root, root, &mut result)?;
    result.sort_by(|first, second| first.path.cmp(&second.path));
    Ok(result)
}

fn copy_tree(source: &Path, destination: &Path, root: bool) -> Result<(), MigrationError> {
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        if root && (entry.file_name() == STAGE_OWNER || entry.file_name() == MARKER) {
            continue;
        }
        let target = destination.join(entry.file_name());
        let kind = entry.file_type()?;
        if kind.is_dir() {
            fs::create_dir(&target)?;
            copy_tree(&entry.path(), &target, false)?;
        } else if kind.is_file() {
            fs::copy(entry.path(), &target)?;
            OpenOptions::new().write(true).open(target)?.sync_all()?;
        } else {
            return Err(MigrationError::UnsupportedEntry);
        }
    }
    Ok(())
}

/// Runs before logging or the core creates the canonical data directory. The caller first checks
/// that neither a previous Pennel app nor Musli is running.
pub fn prepare(data_dir: &Path, home: &Path) -> Result<(), MigrationError> {
    let parent = data_dir.parent().ok_or(MigrationError::UnsupportedEntry)?;
    let source = parent.join("Musli");
    let marker_path = data_dir.join(MARKER);
    if !source.exists() && !marker_path.exists() {
        return Ok(());
    }
    fs::create_dir_all(parent)?;
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(parent.join(".pennel-musli-migration.lock"))?;
    lock.try_lock().map_err(|_| MigrationError::Busy)?;

    if marker_path.exists() {
        // The native migration used an empty marker; upgrade its retry state before core startup.
        if fs::metadata(&marker_path)?.len() == 0 {
            write_marker(&marker_path, &marker(&source, data_dir, home))?;
        } else {
            let state = read_marker(&marker_path)?.ok_or(MigrationError::InvalidMarker)?;
            validate_marker(&state, &source, data_dir, home)?;
        }
        return Ok(());
    }
    if !source.is_dir() || has_pennel_data(data_dir)? {
        return Ok(());
    }
    if fs::symlink_metadata(&source)?.file_type().is_symlink() {
        return Err(MigrationError::UnsupportedEntry);
    }

    let state = marker(&source, data_dir, home);
    let staging = parent.join(STAGING);
    if staging.exists() {
        if fs::symlink_metadata(&staging)?.file_type().is_symlink() {
            return Err(MigrationError::InvalidMarker);
        }
        let owner =
            read_marker(&staging.join(STAGE_OWNER))?.ok_or(MigrationError::InvalidMarker)?;
        if owner.source != source || owner.destination != data_dir {
            return Err(MigrationError::InvalidMarker);
        }
        fs::remove_dir_all(&staging)?;
    }
    fs::create_dir(&staging)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&staging, fs::Permissions::from_mode(0o700))?;
    }
    write_marker(&staging.join(STAGE_OWNER), &state)?;
    let before = stamps(&source)?;
    copy_tree(&source, &staging, true)?;
    if stamps(&source)? != before {
        return Err(MigrationError::SourceChanged);
    }
    write_marker(&staging.join(MARKER), &state)?;

    let mut backup = None;
    let mut canonical_lock = None;
    if data_dir.exists() {
        if has_pennel_data(data_dir)? {
            fs::remove_dir_all(staging)?;
            return Ok(());
        }
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(data_dir.join(".pennel.lock"))?;
        file.try_lock().map_err(|_| MigrationError::Busy)?;
        canonical_lock = Some(file);
        let previous = parent.join(format!(".Pennel-before-musli-{}", nonce()));
        // Windows refuses to rename a directory that contains an open file handle. The lease was
        // free just now; a process that opens it after this release makes the rename fail instead.
        #[cfg(windows)]
        drop(canonical_lock.take());
        fs::rename(data_dir, &previous)?;
        backup = Some(previous);
    }
    if let Err(error) = fs::rename(&staging, data_dir) {
        if let Some(previous) = backup {
            let _ = fs::rename(previous, data_dir);
        }
        return Err(error.into());
    }
    fs::remove_file(data_dir.join(STAGE_OWNER))?;
    drop(canonical_lock);
    drop(lock);
    Ok(())
}

/// A denied/locked Keychain leaves the marker for a later retry; neither existing nor legacy keys
/// are deleted. The native helper inserts missing entries atomically instead of updating them.
pub fn finish(
    engine: &PennelEngine,
    secrets: &Secrets,
    data_dir: &Path,
    home: &Path,
) -> Result<(), MigrationError> {
    let path = data_dir.join(MARKER);
    let Some(state) = read_marker(&path)? else {
        return Ok(());
    };
    let source = data_dir
        .parent()
        .ok_or(MigrationError::InvalidMarker)?
        .join("Musli");
    validate_marker(&state, &source, data_dir, home)?;
    let mut settings = engine.settings();
    let legacy = state.legacy_default_meetings.or_else(|| {
        let old = home.join("Musli");
        (old.is_dir() && !home.join("Pennel").exists()).then_some(old)
    });
    if settings.meetings_dir.is_empty()
        && let Some(legacy) = legacy.filter(|directory| directory.is_dir())
    {
        settings.meetings_dir = legacy.to_string_lossy().into_owned();
        engine
            .update_settings(settings.clone())
            .map_err(|error| MigrationError::Settings(error.to_string()))?;
    }
    secrets
        .migrate_legacy(&settings.providers)
        .map_err(MigrationError::Credentials)?;
    fs::remove_file(path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn legacy(root: &Path) -> (PathBuf, PathBuf, PathBuf) {
        let support = root.join("support");
        let home = root.join("home");
        let old = support.join("Musli");
        fs::create_dir_all(old.join("models")).unwrap();
        fs::create_dir_all(home.join("Musli")).unwrap();
        fs::write(old.join("settings.json"), b"legacy settings").unwrap();
        fs::write(old.join("history.sqlite"), b"legacy database").unwrap();
        fs::write(old.join("history.sqlite-wal"), b"legacy WAL").unwrap();
        fs::write(old.join("models/model.gguf"), b"model data").unwrap();
        (old, support.join("Pennel"), home)
    }

    #[test]
    fn import_preserves_original_settings_database_wal_and_models() {
        let root = tempfile::tempdir().unwrap();
        let (source, destination, home) = legacy(root.path());
        prepare(&destination, &home).unwrap();
        for name in [
            "settings.json",
            "history.sqlite",
            "history.sqlite-wal",
            "models/model.gguf",
        ] {
            assert_eq!(
                fs::read(source.join(name)).unwrap(),
                fs::read(destination.join(name)).unwrap()
            );
        }
        let state = read_marker(&destination.join(MARKER)).unwrap().unwrap();
        assert_eq!(state.legacy_default_meetings, Some(home.join("Musli")));
        prepare(&destination, &home).unwrap();
        assert!(source.join("models/model.gguf").exists());
    }

    #[test]
    fn existing_pennel_data_is_not_merged_or_replaced() {
        let root = tempfile::tempdir().unwrap();
        let (source, destination, home) = legacy(root.path());
        fs::create_dir_all(&destination).unwrap();
        fs::write(destination.join("settings.json"), b"new settings").unwrap();
        prepare(&destination, &home).unwrap();
        assert_eq!(
            fs::read(destination.join("settings.json")).unwrap(),
            b"new settings"
        );
        assert!(!destination.join(MARKER).exists());
        assert!(source.exists());
    }

    #[test]
    fn retry_only_removes_a_stage_owned_by_this_migration() {
        let root = tempfile::tempdir().unwrap();
        let (source, destination, home) = legacy(root.path());
        let staging = source.parent().unwrap().join(STAGING);
        fs::create_dir_all(&staging).unwrap();
        fs::write(staging.join("unrelated"), b"keep").unwrap();
        assert!(prepare(&destination, &home).is_err());
        assert_eq!(fs::read(staging.join("unrelated")).unwrap(), b"keep");
        write_marker(
            &staging.join(STAGE_OWNER),
            &marker(&source, &destination, &home),
        )
        .unwrap();
        prepare(&destination, &home).unwrap();
        assert!(destination.join("history.sqlite").exists());
        assert!(source.join("history.sqlite").exists());
    }

    #[test]
    fn preexisting_logs_are_preserved_in_a_backup_directory() {
        let root = tempfile::tempdir().unwrap();
        let (_source, destination, home) = legacy(root.path());
        fs::create_dir_all(destination.join("logs")).unwrap();
        fs::write(destination.join("logs/pennel.log"), b"existing log").unwrap();
        prepare(&destination, &home).unwrap();
        let backup = fs::read_dir(destination.parent().unwrap())
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .find(|path| {
                path.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with(".Pennel-before-musli-")
            })
            .unwrap();
        assert_eq!(
            fs::read(backup.join("logs/pennel.log")).unwrap(),
            b"existing log"
        );
        assert!(destination.join("settings.json").exists());
    }

    #[test]
    fn open_canonical_lease_prevents_promotion_without_changing_originals() {
        let root = tempfile::tempdir().unwrap();
        let (source, destination, home) = legacy(root.path());
        fs::create_dir_all(destination.join("logs")).unwrap();
        fs::write(destination.join("logs/pennel.log"), b"existing log").unwrap();
        let foreign = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(destination.join(".pennel.lock"))
            .unwrap();
        foreign.try_lock().unwrap();

        assert!(matches!(
            prepare(&destination, &home),
            Err(MigrationError::Busy)
        ));
        assert_eq!(
            fs::read(destination.join("logs/pennel.log")).unwrap(),
            b"existing log"
        );
        let staging = source.parent().unwrap().join(STAGING);
        for name in [
            "settings.json",
            "history.sqlite",
            "history.sqlite-wal",
            "models/model.gguf",
        ] {
            assert_eq!(
                fs::read(source.join(name)).unwrap(),
                fs::read(staging.join(name)).unwrap()
            );
            assert!(!destination.join(name).exists());
        }
        let state = read_marker(&staging.join(STAGE_OWNER)).unwrap().unwrap();
        validate_marker(&state, &source, &destination, &home).unwrap();
        assert!(staging.join(MARKER).exists());
        assert!(!destination.join(MARKER).exists());
        drop(foreign);
        let lease = OpenOptions::new()
            .read(true)
            .write(true)
            .open(destination.join(".pennel.lock"))
            .unwrap();
        lease.try_lock().unwrap();
    }

    #[test]
    fn native_empty_marker_gets_retryable_state_before_core_opens_stores() {
        let root = tempfile::tempdir().unwrap();
        let (_source, destination, home) = legacy(root.path());
        fs::create_dir_all(&destination).unwrap();
        fs::write(destination.join(MARKER), b"").unwrap();
        prepare(&destination, &home).unwrap();
        assert_eq!(
            read_marker(&destination.join(MARKER))
                .unwrap()
                .unwrap()
                .legacy_default_meetings,
            Some(home.join("Musli"))
        );
    }

    fn imported_engine(root: &Path) -> (std::sync::Arc<PennelEngine>, PathBuf, PathBuf) {
        let (source, destination, home) = legacy(root);
        fs::remove_file(source.join("history.sqlite")).unwrap();
        fs::remove_file(source.join("history.sqlite-wal")).unwrap();
        let mut settings = pennel_core::default_settings();
        settings.providers.clear();
        // An imported test engine must never claim the running user's default library.
        settings.meetings_dir = home.join("Musli").to_string_lossy().into_owned();
        fs::write(
            source.join("settings.json"),
            serde_json::to_vec(&settings).unwrap(),
        )
        .unwrap();
        prepare(&destination, &home).unwrap();
        let engine = PennelEngine::new(destination.to_string_lossy().into_owned()).unwrap();
        (engine, destination, home)
    }

    #[test]
    fn finish_keeps_the_legacy_meeting_folder_and_is_idempotent_without_credentials() {
        let root = tempfile::tempdir().unwrap();
        let (engine, destination, home) = imported_engine(root.path());
        let secrets = Secrets::new(false);
        finish(&engine, &secrets, &destination, &home).unwrap();
        assert_eq!(
            engine.settings().meetings_dir,
            home.join("Musli").to_string_lossy()
        );
        assert!(!destination.join(MARKER).exists());
        finish(&engine, &secrets, &destination, &home).unwrap();
    }

    #[test]
    fn deferred_credentials_keep_the_marker_and_every_original_file() {
        let root = tempfile::tempdir().unwrap();
        let (engine, destination, home) = imported_engine(root.path());
        let preview_credentials = Secrets::new(true);
        assert!(matches!(
            finish(&engine, &preview_credentials, &destination, &home),
            Err(MigrationError::Credentials(_))
        ));
        assert!(destination.join(MARKER).exists());
        assert!(
            destination
                .parent()
                .unwrap()
                .join("Musli/settings.json")
                .exists()
        );
    }

    #[test]
    fn a_concurrent_import_is_rejected_before_any_promotion() {
        let root = tempfile::tempdir().unwrap();
        let (source, destination, home) = legacy(root.path());
        let lease = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(
                source
                    .parent()
                    .unwrap()
                    .join(".pennel-musli-migration.lock"),
            )
            .unwrap();
        lease.try_lock().unwrap();
        assert!(matches!(
            prepare(&destination, &home),
            Err(MigrationError::Busy)
        ));
        assert!(!destination.exists());
    }

    #[test]
    fn foreign_marker_paths_are_rejected_before_core_startup() {
        let root = tempfile::tempdir().unwrap();
        let (source, destination, home) = legacy(root.path());
        fs::create_dir_all(&destination).unwrap();
        let mut state = marker(&source, &destination, &home);
        state.legacy_default_meetings = Some(root.path().join("unrelated"));
        write_marker(&destination.join(MARKER), &state).unwrap();
        assert!(matches!(
            prepare(&destination, &home),
            Err(MigrationError::InvalidMarker)
        ));
        assert!(!destination.join("settings.json").exists());
    }

    #[cfg(unix)]
    #[test]
    fn linked_files_do_not_escape_the_source_or_replace_the_destination() {
        let root = tempfile::tempdir().unwrap();
        let (source, destination, home) = legacy(root.path());
        let external = root.path().join("private");
        fs::write(&external, b"keep private").unwrap();
        std::os::unix::fs::symlink(&external, source.join("linked")).unwrap();
        assert!(matches!(
            prepare(&destination, &home),
            Err(MigrationError::UnsupportedEntry)
        ));
        assert_eq!(fs::read(external).unwrap(), b"keep private");
        assert!(!destination.exists());
    }
}
