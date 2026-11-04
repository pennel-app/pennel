use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags};
use serde::{Deserialize, Serialize};

use crate::PennelError;

const FORMAT_VERSION: u32 = 1;
const JOURNAL: &str = "migration.json";
const BACKUP: &str = "migration-backup-v1";

/// Private app data survives an iOS container move; the container's temporary UUID is not identity.
pub(crate) fn private_library_key(root: &Path) -> Result<String, PennelError> {
    #[cfg(target_os = "ios")]
    let home = std::env::var_os("HOME").map(PathBuf::from);
    #[cfg(not(target_os = "ios"))]
    let home: Option<PathBuf> = None;
    canonical_private_library_key(root, home.as_deref(), cfg!(target_os = "ios"))
}

fn canonical_private_library_key(
    root: &Path,
    home: Option<&Path>,
    mobile: bool,
) -> Result<String, PennelError> {
    let root = root.canonicalize().map_err(PennelError::storage)?;
    let home = if mobile {
        home.map(Path::canonicalize)
            .transpose()
            .map_err(PennelError::storage)?
    } else {
        None
    };
    Ok(private_library_key_for_paths(
        &root,
        home.as_deref(),
        mobile,
    ))
}

/// Pure namespace selection over canonical paths, with an explicit platform for portable fixtures.
fn private_library_key_for_paths(root: &Path, home: Option<&Path>, mobile: bool) -> String {
    let namespace = if mobile
        && let Some(home) = home
        && let Ok(relative) = root.strip_prefix(home)
    {
        let relative = relative
            .components()
            .map(|component| component.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            .join("/");
        format!("ios-sandbox:{relative}")
    } else {
        root.to_string_lossy().into_owned()
    };
    crate::evidence_revision(&namespace)
}

#[derive(Debug, Serialize, Deserialize)]
struct Journal {
    version: u32,
    complete: bool,
    snapshot: bool,
}

#[derive(Debug, Serialize, Deserialize)]
struct Manifest {
    version: u32,
    files: Vec<String>,
}

#[derive(Debug)]
pub(crate) struct DirectoryLease {
    _file: File,
}

impl DirectoryLease {
    pub(crate) fn acquire(dir: &Path) -> Result<Self, PennelError> {
        Self::acquire_named(dir, ".pennel.lock")
    }

    pub(crate) fn acquire_library(dir: &Path) -> Result<Self, PennelError> {
        Self::acquire_named(dir, ".pennel-library.lock")
    }

    fn acquire_named(dir: &Path, name: &str) -> Result<Self, PennelError> {
        fs::create_dir_all(dir).map_err(PennelError::storage)?;
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(dir.join(name))
            .map_err(PennelError::storage)?;
        file.try_lock().map_err(|_| PennelError::Storage {
            message: "this data folder is already being used by another Pennel app".into(),
        })?;
        Ok(Self { _file: file })
    }
}

pub(crate) struct StartupStorage {
    _lease: DirectoryLease,
    dir: PathBuf,
    journal: Journal,
}

impl StartupStorage {
    /// Must run before opening stores, normalizing settings, or applying retention.
    pub(crate) fn prepare(dir: &Path) -> Result<Self, PennelError> {
        let lease = DirectoryLease::acquire(dir)?;
        let journal_path = dir.join(JOURNAL);
        let previous: Option<Journal> = match fs::read(&journal_path) {
            Ok(bytes) => Some(serde_json::from_slice(&bytes).map_err(PennelError::storage)?),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(PennelError::storage(error)),
        };
        if let Some(journal) = previous {
            if journal.version != FORMAT_VERSION {
                return Err(PennelError::Storage {
                    message: "this Pennel data requires a different app version".into(),
                });
            }
            if !journal.complete && journal.snapshot {
                ensure_snapshot(dir)?;
            }
            return Ok(Self {
                _lease: lease,
                dir: dir.to_owned(),
                journal,
            });
        }
        let snapshot = [
            "settings.json",
            crate::dictation_pad::PAD_FILE,
            "history.sqlite",
            "index.sqlite",
            "failed-audio",
        ]
        .iter()
        .any(|name| dir.join(name).exists());
        let journal = Journal {
            version: FORMAT_VERSION,
            complete: false,
            snapshot,
        };
        write_json(&journal_path, &journal)?;
        if snapshot {
            ensure_snapshot(dir)?;
        }
        Ok(Self {
            _lease: lease,
            dir: dir.to_owned(),
            journal,
        })
    }

    pub(crate) fn finish(&mut self) -> Result<(), PennelError> {
        if !self.journal.complete {
            self.journal.complete = true;
            write_json(&self.dir.join(JOURNAL), &self.journal)?;
        }
        Ok(())
    }
}

fn write_json(path: &Path, value: &impl Serialize) -> Result<(), PennelError> {
    let bytes = serde_json::to_vec_pretty(value).map_err(PennelError::storage)?;
    let temp = path.with_extension("json.tmp");
    let mut file = File::create(&temp).map_err(PennelError::storage)?;
    file.write_all(&bytes).map_err(PennelError::storage)?;
    file.sync_all().map_err(PennelError::storage)?;
    drop(file);
    fs::rename(temp, path).map_err(PennelError::storage)?;
    sync_directory(path.parent().expect("file parent"))
}

fn ensure_snapshot(dir: &Path) -> Result<(), PennelError> {
    let backup = dir.join(BACKUP);
    if backup.exists() {
        let manifest: Manifest = serde_json::from_slice(
            &fs::read(backup.join("manifest.json")).map_err(PennelError::storage)?,
        )
        .map_err(PennelError::storage)?;
        if manifest.version != FORMAT_VERSION
            || manifest.files.iter().any(|name| {
                let relative = Path::new(name);
                relative.is_absolute()
                    || relative
                        .components()
                        .any(|component| !matches!(component, std::path::Component::Normal(_)))
                    || !backup.join(relative).is_file()
            })
        {
            return Err(PennelError::Storage {
                message:
                    "de migratieback-up is onvolledig; herstel de gegevens voordat Pennel start"
                        .into(),
            });
        }
        return Ok(());
    }
    let staging = dir.join(format!("{BACKUP}.incomplete-{}", uuid::Uuid::now_v7()));
    fs::create_dir(&staging).map_err(PennelError::storage)?;
    private_directory(&staging)?;
    let mut files = Vec::new();
    for name in [
        "settings.json",
        crate::dictation_pad::PAD_FILE,
        "history.sqlite",
        "index.sqlite",
    ] {
        let source = dir.join(name);
        if !source.exists() {
            continue;
        }
        if source
            .symlink_metadata()
            .map_err(PennelError::storage)?
            .file_type()
            .is_symlink()
        {
            return Err(PennelError::Storage {
                message: "a data file is a symbolic link; make a regular local copy first".into(),
            });
        }
        let target = staging.join(name);
        if name.ends_with(".sqlite") {
            // VACUUM INTO reads a consistent SQLite snapshot, including committed WAL pages.
            let connection = Connection::open_with_flags(&source, OpenFlags::SQLITE_OPEN_READ_ONLY)
                .map_err(PennelError::storage)?;
            connection
                .busy_timeout(std::time::Duration::from_secs(5))
                .map_err(PennelError::storage)?;
            connection
                .execute("VACUUM INTO ?1", [target.to_string_lossy().as_ref()])
                .map_err(PennelError::storage)?;
        } else {
            fs::copy(&source, &target).map_err(PennelError::storage)?;
        }
        sync_file(&target)?;
        files.push(name.to_owned());
    }
    if dir.join("failed-audio").exists() {
        copy_tree(dir, &staging, Path::new("failed-audio"), &mut files)?;
    }
    write_json(
        &staging.join("manifest.json"),
        &Manifest {
            version: FORMAT_VERSION,
            files,
        },
    )?;
    sync_directory(&staging)?;
    fs::rename(staging, backup).map_err(PennelError::storage)?;
    sync_directory(dir)
}

fn copy_tree(
    root: &Path,
    target_root: &Path,
    relative: &Path,
    files: &mut Vec<String>,
) -> Result<(), PennelError> {
    let source = root.join(relative);
    let metadata = source.symlink_metadata().map_err(PennelError::storage)?;
    if metadata.file_type().is_symlink() {
        return Err(PennelError::Storage {
            message: "an audio file is a symbolic link; make a regular local copy first".into(),
        });
    }
    let target = target_root.join(relative);
    if metadata.is_dir() {
        fs::create_dir(&target).map_err(PennelError::storage)?;
        private_directory(&target)?;
        for entry in fs::read_dir(&source).map_err(PennelError::storage)? {
            let entry = entry.map_err(PennelError::storage)?;
            copy_tree(root, target_root, &relative.join(entry.file_name()), files)?;
        }
    } else if metadata.is_file() {
        fs::copy(source, &target).map_err(PennelError::storage)?;
        sync_file(&target)?;
        files.push(relative.to_string_lossy().into_owned());
    } else {
        return Err(PennelError::Storage {
            message: "onbekend bestandstype in de audiomap".into(),
        });
    }
    Ok(())
}

#[cfg(unix)]
fn private_directory(path: &Path) -> Result<(), PennelError> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).map_err(PennelError::storage)
}

#[cfg(not(unix))]
fn private_directory(_path: &Path) -> Result<(), PennelError> {
    Ok(())
}

fn sync_file(path: &Path) -> Result<(), PennelError> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    options.write(true);
    options
        .open(path)
        .and_then(|file| file.sync_all())
        .map_err(PennelError::storage)
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> Result<(), PennelError> {
    File::open(path)
        .and_then(|file| file.sync_all())
        .map_err(PennelError::storage)
}

#[cfg(not(unix))]
fn sync_directory(_path: &Path) -> Result<(), PennelError> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mobile_library_keys_survive_sandbox_relocation_but_keep_profiles_and_external_roots_separate()
     {
        let first = Path::new("/private/var/mobile/Containers/Data/Application/AAA");
        let second = Path::new("/private/var/mobile/Containers/Data/Application/BBB");
        let first_root = first.join("Documents/QA/pilot/Meetings");
        let second_root = second.join("Documents/QA/pilot/Meetings");
        let original = private_library_key_for_paths(&first_root, Some(first), true);
        assert_eq!(
            original,
            "a09cacd1aedb5f81a91127488e62127c272a8bc2bcabaa6f6e1a1b96a9e38648"
        );
        assert_eq!(
            original,
            private_library_key_for_paths(&second_root, Some(second), true)
        );
        assert_ne!(
            original,
            private_library_key_for_paths(
                &second.join("Documents/QA/another/Meetings"),
                Some(second),
                true
            )
        );
        assert_ne!(
            private_library_key_for_paths(Path::new("/Volumes/One/Meetings"), Some(first), true),
            private_library_key_for_paths(Path::new("/Volumes/Two/Meetings"), Some(second), true)
        );
        assert_eq!(
            private_library_key_for_paths(Path::new("/Volumes/Notes/Meetings"), Some(first), true),
            "6023fffc1261ac647b812611cdc1d5fc46c9e42d1bf6bd61909de9f0fd169146"
        );
    }

    #[test]
    fn desktop_library_keys_keep_the_previous_absolute_path_namespace() {
        assert_eq!(
            private_library_key_for_paths(
                Path::new("/Users/lander/Pennel"),
                Some(Path::new("/Users/lander")),
                false
            ),
            "69e00598e10ed7d61e16a71108cb7b93cc9fe5ac2cf4610d4b989d44c9f04626"
        );
    }

    #[cfg(unix)]
    #[test]
    fn mobile_library_namespace_canonicalizes_both_root_and_home_aliases() {
        let directory = tempfile::tempdir().unwrap();
        let home = directory.path().join("sandbox-home");
        let meetings = home.join("Documents/QA/pilot/Meetings");
        fs::create_dir_all(&meetings).unwrap();
        let home_alias = directory.path().join("sandbox-alias");
        std::os::unix::fs::symlink(&home, &home_alias).unwrap();
        let expected = "a09cacd1aedb5f81a91127488e62127c272a8bc2bcabaa6f6e1a1b96a9e38648";
        assert_eq!(
            canonical_private_library_key(&meetings, Some(&home_alias), true).unwrap(),
            expected
        );
        assert_eq!(
            canonical_private_library_key(
                &home_alias.join("Documents/QA/pilot/Meetings"),
                Some(&home),
                true
            )
            .unwrap(),
            expected
        );
        let external = directory.path().join("sandbox-home-external/Meetings");
        fs::create_dir_all(&external).unwrap();
        assert_eq!(
            canonical_private_library_key(&external, Some(&home), true).unwrap(),
            canonical_private_library_key(&external, Some(&home), false).unwrap()
        );
    }

    #[test]
    fn directory_lease_excludes_another_owner_and_releases_on_drop() {
        let dir = tempfile::tempdir().unwrap();
        let first = DirectoryLease::acquire(dir.path()).unwrap();
        assert!(DirectoryLease::acquire(dir.path()).is_err());
        drop(first);
        assert!(DirectoryLease::acquire(dir.path()).is_ok());
    }

    #[test]
    fn snapshot_includes_wal_rows_and_failed_audio_before_pruning() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("settings.json"), "original").unwrap();
        fs::write(
            dir.path().join(crate::dictation_pad::PAD_FILE),
            "Eigen concept",
        )
        .unwrap();
        fs::create_dir(dir.path().join("failed-audio")).unwrap();
        fs::write(dir.path().join("failed-audio/one.wav"), b"private audio").unwrap();
        let connection = Connection::open(dir.path().join("history.sqlite")).unwrap();
        connection.execute_batch("PRAGMA journal_mode=WAL; CREATE TABLE items (value TEXT); INSERT INTO items VALUES ('committed');").unwrap();
        let mut storage = StartupStorage::prepare(dir.path()).unwrap();
        let saved = Connection::open(dir.path().join(BACKUP).join("history.sqlite")).unwrap();
        assert_eq!(
            saved
                .query_row("SELECT value FROM items", [], |row| row.get::<_, String>(0))
                .unwrap(),
            "committed"
        );
        assert_eq!(
            fs::read(dir.path().join(BACKUP).join("failed-audio/one.wav")).unwrap(),
            b"private audio"
        );
        assert_eq!(
            fs::read_to_string(dir.path().join(BACKUP).join(crate::dictation_pad::PAD_FILE))
                .unwrap(),
            "Eigen concept"
        );
        storage.finish().unwrap();
    }

    #[test]
    fn interrupted_migration_reuses_original_snapshot() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("settings.json"), "original").unwrap();
        drop(StartupStorage::prepare(dir.path()).unwrap());
        fs::write(dir.path().join("settings.json"), "repaired").unwrap();
        let mut storage = StartupStorage::prepare(dir.path()).unwrap();
        storage.finish().unwrap();
        assert_eq!(
            fs::read_to_string(dir.path().join(BACKUP).join("settings.json")).unwrap(),
            "original"
        );
    }

    #[test]
    fn an_incomplete_snapshot_does_not_allow_startup() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("settings.json"), "original").unwrap();
        drop(StartupStorage::prepare(dir.path()).unwrap());
        fs::remove_file(dir.path().join(BACKUP).join("settings.json")).unwrap();
        assert!(StartupStorage::prepare(dir.path()).is_err());
    }

    #[test]
    fn future_migration_versions_are_not_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        write_json(
            &dir.path().join(JOURNAL),
            &Journal {
                version: 99,
                complete: true,
                snapshot: false,
            },
        )
        .unwrap();
        assert!(StartupStorage::prepare(dir.path()).is_err());
        let journal: Journal =
            serde_json::from_slice(&fs::read(dir.path().join(JOURNAL)).unwrap()).unwrap();
        assert_eq!(journal.version, 99);
    }
}
