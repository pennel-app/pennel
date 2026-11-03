pub mod api;
#[cfg(test)]
mod engine_tests;
pub mod scan;
pub mod state;
pub mod trash;

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use api::{ChangePage, PutResult, SyncApi};
use chrono::{Local, TimeZone};
use scan::{
    Key, LocalFile, MEETING_FILE, Slot, TEMPLATES_DIR, clean_folder, file_slot, find_dir,
    hash_file, nfc, parse_key, portable_name, sha256_hex, slot,
};
use state::{Entry, SyncState};

use crate::cloud::crypto::{self, Payload, VaultKey};
use crate::error::PennelError;
use crate::meetings::MeetingStore;

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum SyncStatus {
    Off,
    SignedOut,
    NeedsSetup,
    /// The server has a vault key and this device has none.
    NeedsRecoveryCode,
    /// The vault was restarted on another device; this device forgot its key.
    VaultReset,
    /// Ready to sync, but no pass has finished since Pennel started.
    Pending,
    Synced,
    Offline,
    StorageFull {
        message: String,
    },
    Error {
        message: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SyncReport {
    pub status: SyncStatus,
    pub uploaded: u32,
    pub downloaded: u32,
    pub conflicts: u32,
    pub last_success_ms: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum SyncItemState {
    Synced,
    Pending,
    Conflict,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct MeetingSyncState {
    pub id: String,
    pub state: SyncItemState,
}

pub struct PassInput<'a> {
    pub api: &'a dyn SyncApi,
    pub state: &'a SyncState,
    pub key: &'a VaultKey,
    pub root: &'a Path,
    pub data_dir: &'a Path,
    pub device: &'a str,
    pub recording: &'a [String],
}

const PAGE: u32 = 500;
const TRASH_DAYS: u32 = 30;
/// The server's per-object limit (5 MiB), checked on the sealed blob before uploading. Content
/// travels base64-encoded inside the sealed JSON, so files above roughly 3.75 MB do not fit.
const MAX_OBJECT_BYTES: usize = 5_242_880;
const MISSING_ROOT: &str = "The meeting folder was not found.";
const TOO_LARGE: &str = "This file is too large for Pennel Cloud (approximately 3.5 MB maximum).";
const UNKNOWN_DEVICE: &str = "other device";
const UNEXPECTED_RESPONSE: &str = "Pennel Cloud returned an unexpected response.";
const CHANGED_DURING_SYNC: &str =
    "A meeting file changed during sync; Pennel will try again shortly.";
pub const MASS_DELETE: &str = "Many meetings disappeared from your folder. Sync has deleted nothing; confirm under Settings → Account if this was intentional.";

struct Stop(SyncStatus);

#[cfg(test)]
thread_local! {
    /// Simulates the process dying while a downloaded file is being applied.
    static CRASH_IN_APPLY: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

impl From<PennelError> for Stop {
    fn from(error: PennelError) -> Self {
        Stop(status_for(error))
    }
}

pub(crate) fn status_for(error: PennelError) -> SyncStatus {
    match error {
        PennelError::Network { .. } => SyncStatus::Offline,
        PennelError::Cloud { code, .. } if code == "rate_limited" => SyncStatus::Offline,
        PennelError::InvalidResponse { .. } => SyncStatus::Error {
            message: UNEXPECTED_RESPONSE.into(),
        },
        PennelError::NotSignedIn => SyncStatus::SignedOut,
        PennelError::VaultLocked => SyncStatus::NeedsRecoveryCode,
        PennelError::Cloud { code, message } if code == "storage_full" => {
            SyncStatus::StorageFull { message }
        }
        PennelError::Cloud { code, .. } if code == "generation" => SyncStatus::VaultReset,
        PennelError::Cloud { message, .. } => SyncStatus::Error { message },
        PennelError::Storage { message } if message == crate::meetings::UID_RACE => {
            SyncStatus::Error {
                message: CHANGED_DURING_SYNC.into(),
            }
        }
        PennelError::Storage { .. } => SyncStatus::Error {
            message: scan::UNREADABLE_LIBRARY.into(),
        },
        _ => SyncStatus::Error {
            message: UNEXPECTED_RESPONSE.into(),
        },
    }
}

#[derive(Default)]
struct Counts {
    uploaded: u32,
    downloaded: u32,
    conflicts: u32,
}

struct Pass<'a> {
    api: &'a dyn SyncApi,
    state: &'a SyncState,
    key: &'a VaultKey,
    generation: u32,
    root: PathBuf,
    data_dir: &'a Path,
    device: &'a str,
    recording: &'a [String],
    store: MeetingStore,
    /// uid → folder name on disk, including meetings that are recording.
    folders: HashMap<String, String>,
    counts: Counts,
    /// A problem that did not stop the pass, reported once it finishes.
    problem: Option<String>,
    /// Objects deferred during this pass, which must stay deferred.
    deferred_again: HashSet<String>,
}

pub fn run(input: PassInput<'_>) -> SyncReport {
    let mut pass = Pass {
        api: input.api,
        state: input.state,
        key: input.key,
        generation: 0,
        root: input.root.to_path_buf(),
        data_dir: input.data_dir,
        device: input.device,
        recording: input.recording,
        store: MeetingStore::new(input.root),
        folders: HashMap::new(),
        counts: Counts::default(),
        problem: None,
        deferred_again: HashSet::new(),
    };
    let status = match pass.execute() {
        Ok(()) => pass
            .problem
            .take()
            .map_or(SyncStatus::Synced, |message| SyncStatus::Error { message }),
        Err(Stop(status)) => status,
    };
    SyncReport {
        status,
        uploaded: pass.counts.uploaded,
        downloaded: pass.counts.downloaded,
        conflicts: pass.counts.conflicts,
        last_success_ms: None,
    }
}

fn conflict_name(name: &str, payload: &Payload) -> (String, String) {
    let (stem, ext) = match name.rfind('.') {
        Some(dot) if dot > 0 => (&name[..dot], &name[dot..]),
        _ => (name, ""),
    };
    let device = clean_folder(&payload.device).unwrap_or_else(|| UNKNOWN_DEVICE.to_string());
    let when = Local
        .timestamp_millis_opt(payload.modified_ms)
        .single()
        .unwrap_or_else(Local::now)
        .format("%Y-%m-%d %H%M");
    (
        format!("{stem} (conflict {device} {when})"),
        ext.to_string(),
    )
}

fn modified_ms(path: &Path) -> i64 {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_millis() as i64)
}

fn temp_path(dir: &Path) -> PathBuf {
    dir.join(format!(".pennel-sync-{}.tmp", uuid::Uuid::now_v7()))
}

/// Replaces `target` only if it still has the content the decision was based on (`expected`,
/// `None` = absent). Returns false, leaving `target` alone, when it changed in the meantime.
fn write_checked(
    dir: &Path,
    target: &Path,
    content: &[u8],
    expected: Option<&str>,
) -> Result<bool, PennelError> {
    let tmp = temp_path(dir);
    std::fs::write(&tmp, content).map_err(PennelError::storage)?;
    if hash_file(target).as_deref() != expected || (expected.is_none() && target.exists()) {
        let _ = std::fs::remove_file(&tmp);
        return Ok(false);
    }
    std::fs::rename(&tmp, target).map_err(|error| {
        let _ = std::fs::remove_file(&tmp);
        PennelError::storage(error)
    })?;
    Ok(true)
}

impl Pass<'_> {
    fn execute(&mut self) -> Result<(), Stop> {
        if !self.root.is_dir() {
            return Err(Stop(SyncStatus::Error {
                message: MISSING_ROOT.into(),
            }));
        }
        if let Ok(canonical) = self.root.canonicalize() {
            self.root = canonical;
            self.store = MeetingStore::new(&self.root);
        }
        trash::purge(self.data_dir, TRASH_DAYS);
        let root_id = self.root.to_string_lossy().into_owned();
        match self.state.root()? {
            Some(previous) if previous == root_id => {}
            // Another meetings folder: compare everything again instead of treating the
            // old folder's files as deleted.
            Some(_) => {
                self.state.forget_progress()?;
                self.state.set_root(&root_id)?;
            }
            None => self.state.set_root(&root_id)?,
        }
        let vault = self.api.vault()?;
        let check = crypto::vault_check(self.key);
        // A key kept from before a sign-out may belong to another account's vault.
        if let Some(expected) = vault.kdf.as_ref().and_then(|kdf| kdf.key_check.as_deref())
            && expected != check
        {
            let reset = self
                .state
                .generation()?
                .is_some_and(|local| local < vault.generation);
            return Err(Stop(if reset {
                SyncStatus::VaultReset
            } else {
                SyncStatus::NeedsRecoveryCode
            }));
        }
        self.state.use_vault(&check)?;
        match self.state.generation()? {
            None => self.state.set_generation(vault.generation)?,
            Some(local) if local < vault.generation => return Err(Stop(SyncStatus::VaultReset)),
            Some(local) if local > vault.generation => {
                return Err(Stop(status_for(crypto::vault_invalid())));
            }
            Some(_) => {}
        }
        if vault.wrapped.is_none() {
            return Err(Stop(SyncStatus::NeedsSetup));
        }
        self.generation = vault.generation;
        self.folders = self.resolve_folders()?;
        self.pull()?;
        self.assign_uids()?;
        self.push()
    }

    fn is_recording(&self, folder: &str) -> bool {
        let folder = nfc(folder);
        self.recording.iter().any(|id| nfc(id) == folder)
    }

    /// One folder per uid. A copied folder shares its uid with the original; the one this device
    /// synced before keeps it.
    fn resolve_folders(&self) -> Result<HashMap<String, String>, PennelError> {
        let mut by_uid: HashMap<String, Vec<String>> = HashMap::new();
        for folder in scan::meeting_folders(&self.root)? {
            if let Some(uid) = folder.uid {
                by_uid.entry(uid).or_default().push(folder.name);
            }
        }
        let mut owners = HashMap::new();
        for (uid, names) in by_uid {
            let known = self.state.folder_of_uid(&uid)?;
            let owner = names
                .iter()
                .find(|name| known.as_deref() == Some(nfc(name).as_str()))
                .unwrap_or(&names[0])
                .clone();
            owners.insert(uid, owner);
        }
        Ok(owners)
    }

    fn assign_uids(&mut self) -> Result<(), Stop> {
        let mut owners = self.resolve_folders()?;
        for folder in scan::meeting_folders(&self.root)? {
            if self.is_recording(&folder.name) {
                continue;
            }
            let owned = folder
                .uid
                .as_ref()
                .is_some_and(|uid| owners.get(uid) == Some(&folder.name));
            if owned {
                continue;
            }
            let known = self
                .state
                .uid_for_folder(&nfc(&folder.name))?
                .filter(|uid| !owners.contains_key(uid));
            let assigned = match (known, folder.uid.is_none()) {
                (None, true) => self.store.ensure_uid(&folder.name).ok(),
                (known, _) => {
                    let uid = known.unwrap_or_else(|| uuid::Uuid::now_v7().to_string());
                    self.store.set_uid(&folder.name, &uid).ok().map(|()| uid)
                }
            };
            if let Some(uid) = assigned {
                owners.insert(uid, folder.name);
            }
        }
        for (uid, folder) in &owners {
            self.state.set_meeting_folder(uid, &nfc(folder))?;
        }
        self.folders = owners;
        Ok(())
    }

    fn defer(&mut self, id: &str) -> Result<(), PennelError> {
        self.deferred_again.insert(id.to_string());
        self.state.defer(id)
    }

    fn note_problem(&mut self, message: String) {
        self.problem.get_or_insert(message);
    }

    fn pull(&mut self) -> Result<(), Stop> {
        for id in self.state.deferred()? {
            let entry = self.state.entry_by_object(&id)?;
            let before = entry.as_ref().map(|e| self.local_hash(&e.key));
            // The item leaves the deferred list only once it is applied, so a crash keeps it.
            let fetched = self.api.get(&id)?;
            self.deferred_again.remove(&id);
            let applied = match fetched {
                Some((version, blob)) => self.apply_remote(&id, version, &blob, entry, before),
                None => match entry {
                    Some(entry) => {
                        let version = entry.server_version;
                        self.apply_tombstones(vec![(entry, version)])
                    }
                    None => Ok(()),
                },
            };
            applied?;
            if !self.deferred_again.contains(&id) {
                self.state.undefer(&id)?;
            }
        }
        let mut cursor = self.state.cursor()?;
        let mut full: Option<HashSet<String>> = None;
        loop {
            let (changes, next, more, generation) = match self.api.changes(cursor, PAGE)? {
                ChangePage::CursorExpired if full.is_none() => {
                    full = Some(HashSet::new());
                    cursor = 0;
                    continue;
                }
                ChangePage::CursorExpired => {
                    return Err(Stop(SyncStatus::Error {
                        message: UNEXPECTED_RESPONSE.into(),
                    }));
                }
                ChangePage::Ok {
                    changes,
                    cursor,
                    more,
                    generation,
                } => (changes, cursor, more, generation),
            };
            if generation != self.generation {
                return Err(Stop(SyncStatus::VaultReset));
            }
            let mut tombstones = Vec::new();
            for change in changes {
                if let Some(seen) = full.as_mut() {
                    seen.insert(change.id.clone());
                }
                let entry = self.state.entry_by_object(&change.id)?;
                if entry
                    .as_ref()
                    .is_some_and(|e| e.server_version >= change.version)
                {
                    continue;
                }
                if change.deleted {
                    if let Some(entry) = entry {
                        tombstones.push((entry, change.version));
                    }
                    continue;
                }
                // Hashed before the download, so an edit made while it runs is noticed.
                let before = entry.as_ref().map(|e| self.local_hash(&e.key));
                if let Some((version, blob)) = self.api.get(&change.id)? {
                    self.apply_remote(&change.id, version, &blob, entry, before)?;
                }
            }
            self.apply_tombstones(tombstones)?;
            self.state.set_cursor(next)?;
            cursor = next;
            if !more {
                break;
            }
        }
        // After a full listing, objects missing from it were purged from the server long ago;
        // forgetting them makes the local files upload again instead of being deleted.
        if let Some(seen) = full {
            for entry in self.state.entries()? {
                if !seen.contains(&entry.object_id) {
                    self.state.remove(&entry.key)?;
                }
            }
        }
        Ok(())
    }

    /// The directory a key lives in and its file name; `None` for a meeting not on this device.
    fn location(&self, key: &str) -> Option<(PathBuf, Option<String>, String)> {
        match parse_key(key)? {
            Key::Template { name } => Some((self.root.join(TEMPLATES_DIR), None, name)),
            Key::Meeting { uid, name } => {
                let folder = self.folders.get(&uid)?.clone();
                Some((self.root.join(&folder), Some(folder), name))
            }
        }
    }

    fn local_hash(&self, key: &str) -> Option<String> {
        let (dir, _, name) = self.location(key)?;
        match file_slot(&dir, &name) {
            Slot::Existing(path) => hash_file(&path),
            _ => None,
        }
    }

    /// The folder for a meeting uid, adopting a same-named folder without uid (the same
    /// meeting from before sync) or creating one.
    fn folder_for(&mut self, uid: &str, wanted: Option<&str>) -> Result<String, Stop> {
        if let Some(folder) = self.folders.get(uid) {
            return Ok(folder.clone());
        }
        let wanted = wanted
            .and_then(clean_folder)
            .unwrap_or_else(|| uid.to_string());
        let mut candidate = wanted.clone();
        for n in 2.. {
            match find_dir(&self.root, &candidate)? {
                None => {
                    std::fs::create_dir_all(self.root.join(&candidate))
                        .map_err(PennelError::storage)?;
                    self.folders.insert(uid.to_string(), candidate.clone());
                    return Ok(candidate);
                }
                Some(existing) => {
                    let owned = self.folders.values().any(|f| *f == existing)
                        || self.store.uid_of(&existing).is_some();
                    if !owned {
                        if !self.is_recording(&existing)
                            && self.root.join(&existing).join(MEETING_FILE).is_file()
                        {
                            let _ = self.store.set_uid(&existing, uid);
                        }
                        self.folders.insert(uid.to_string(), existing.clone());
                        return Ok(existing);
                    }
                }
            }
            candidate = format!("{wanted} ({n})");
        }
        unreachable!("unbounded range")
    }

    fn rename_folder(&mut self, uid: &str, current: &str, wanted: &str) -> Option<String> {
        if self.is_recording(current) {
            return None;
        }
        let mut candidate = wanted.to_string();
        for n in 2.. {
            match find_dir(&self.root, &candidate).ok()? {
                Some(existing) if existing != current => candidate = format!("{wanted} ({n})"),
                _ => break,
            }
        }
        if candidate == current {
            return None;
        }
        std::fs::rename(self.root.join(current), self.root.join(&candidate)).ok()?;
        self.folders.insert(uid.to_string(), candidate.clone());
        let _ = self.state.set_meeting_folder(uid, &nfc(&candidate));
        Some(candidate)
    }

    /// Writes the remote version beside `name`; an identical copy already there is reused.
    fn conflict_copy(&mut self, dir: &Path, name: &str, payload: &Payload) -> Result<(), Stop> {
        let (base, ext) = conflict_name(&portable_name(name), payload);
        let content_hash = sha256_hex(&payload.content);
        for n in 1.. {
            let candidate = if n == 1 {
                format!("{base}{ext}")
            } else {
                format!("{base} ({n}){ext}")
            };
            match slot(dir, &candidate) {
                Slot::Existing(path) if hash_file(&path).as_deref() == Some(&content_hash) => {
                    return Ok(());
                }
                Slot::Free(path) if write_checked(dir, &path, &payload.content, None)? => {
                    self.counts.conflicts += 1;
                    return Ok(());
                }
                _ => {}
            }
        }
        unreachable!("unbounded range")
    }

    /// `before`: for a known object, the local hash taken before the download started.
    fn apply_remote(
        &mut self,
        id: &str,
        version: i64,
        blob: &[u8],
        entry: Option<Entry>,
        before: Option<Option<String>>,
    ) -> Result<(), Stop> {
        #[cfg(test)]
        if CRASH_IN_APPLY.with(std::cell::Cell::get) {
            panic!("simulated crash while applying");
        }
        let payload = match crypto::open(self.key, id, self.generation, blob) {
            Ok(payload) => payload,
            Err(error) => {
                self.note_problem(error.to_string());
                return Ok(());
            }
        };
        let Some(parsed) = parse_key(&payload.key) else {
            self.note_problem(crypto::object_unreadable().to_string());
            return Ok(());
        };
        let (mut dir, folder, name, uid) = match parsed {
            Key::Template { name } => {
                let dir = self.root.join(TEMPLATES_DIR);
                std::fs::create_dir_all(&dir).map_err(PennelError::storage)?;
                (dir, None, name, None)
            }
            Key::Meeting { uid, name } => {
                let folder = self.folder_for(&uid, payload.folder.as_deref())?;
                if self.is_recording(&folder) {
                    self.defer(id)?;
                    return Ok(());
                }
                (self.root.join(&folder), Some(folder), name, Some(uid))
            }
        };
        let target = match file_slot(&dir, &name) {
            Slot::Taken => {
                // Writing would replace a different file on a case-insensitive disk.
                return self.conflict_copy(&dir, &name, &payload);
            }
            Slot::Existing(path) | Slot::Free(path) => path,
        };
        let local = before.unwrap_or_else(|| hash_file(&target));
        let remote_hash = sha256_hex(&payload.content);
        let base_hash = entry
            .as_ref()
            .map(|e| e.synced_hash.clone())
            .unwrap_or_default();
        let record = |hash: String, folder: Option<String>| Entry {
            key: payload.key.clone(),
            object_id: id.to_string(),
            server_version: version,
            synced_hash: hash,
            folder: folder.map(|f| nfc(&f)),
        };
        if local.as_deref() == Some(remote_hash.as_str()) {
            self.state.put(&record(remote_hash, folder))?;
            return Ok(());
        }
        let unchanged_here = local.is_none()
            || entry
                .as_ref()
                .is_some_and(|e| local.as_deref() == Some(e.synced_hash.as_str()));
        // Also when meeting.md conflicts: the folder name follows the other device either way,
        // so a rename there never reverts and no old folder is left behind.
        let mut folder = folder;
        let mut target = target;
        if name == MEETING_FILE
            && let (Some(uid), Some(current)) = (uid.as_deref(), folder.clone())
            && let Some(wanted) = payload.folder.as_deref().and_then(clean_folder)
            && nfc(&current) != wanted
            && let Some(renamed) = self.rename_folder(uid, &current, &wanted)
        {
            dir = self.root.join(&renamed);
            target = dir.join(target.file_name().expect("file path"));
            folder = Some(renamed);
        }
        if !unchanged_here {
            self.conflict_copy(&dir, &name, &payload)?;
            self.state.put(&record(base_hash, folder))?;
            return Ok(());
        }
        if write_checked(&dir, &target, &payload.content, local.as_deref())? {
            self.counts.downloaded += 1;
            self.state.put(&record(remote_hash, folder))?;
        } else {
            self.conflict_copy(&dir, &name, &payload)?;
            self.state.put(&record(base_hash, folder))?;
        }
        Ok(())
    }

    /// True when deleting this meeting would lose something: a file never synced or changed since.
    fn meeting_has_local_work(&self, uid: &str, dir: &Path) -> Result<bool, PennelError> {
        for (name, path) in scan::syncable_files(dir)? {
            if name == MEETING_FILE {
                continue;
            }
            let entry = tracked_entry(self.state, &scan::meeting_key(uid, &name))?;
            if entry.is_none_or(|e| hash_file(&path).as_deref() != Some(e.synced_hash.as_str())) {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn apply_tombstones(&mut self, tombstones: Vec<(Entry, i64)>) -> Result<(), Stop> {
        let (meeting_files, others): (Vec<_>, Vec<_>) = tombstones.into_iter().partition(|(e, _)| {
            matches!(parse_key(&e.key), Some(Key::Meeting { name, .. }) if name == MEETING_FILE)
        });
        let mut whole_without_local_work = HashSet::new();
        for (entry, _) in &meeting_files {
            if let Some(Key::Meeting { uid, .. }) = parse_key(&entry.key)
                && let Some((dir, folder, _)) = self.location(&entry.key)
                && !folder
                    .as_deref()
                    .is_some_and(|folder| self.is_recording(folder))
                && !self.meeting_has_local_work(&uid, &dir)?
            {
                whole_without_local_work.insert(uid);
            }
        }
        for (entry, version) in others.into_iter().chain(meeting_files) {
            let Some((dir, folder, name)) = self.location(&entry.key) else {
                self.state.remove(&entry.key)?;
                continue;
            };
            if folder.as_deref().is_some_and(|f| self.is_recording(f)) {
                self.defer(&entry.object_id)?;
                continue;
            }
            let uid = match parse_key(&entry.key) {
                Some(Key::Meeting { uid, .. }) => Some(uid),
                _ => None,
            };
            // A local conflict manifest can still need a part retired by the other device.
            if uid
                .as_ref()
                .is_some_and(|uid| !whole_without_local_work.contains(uid))
                && crate::meeting_utterances::part_is_referenced(&dir, &name)
            {
                self.state.put(&Entry {
                    server_version: version,
                    synced_hash: String::new(),
                    ..entry
                })?;
                continue;
            }
            // Edits made here to other files of a deleted meeting keep the meeting alive.
            if name == MEETING_FILE
                && let Some(uid) = uid.as_deref()
                && self.meeting_has_local_work(uid, &dir)?
            {
                self.state.put(&Entry {
                    server_version: version,
                    synced_hash: String::new(),
                    ..entry
                })?;
                continue;
            }
            match file_slot(&dir, &name) {
                Slot::Existing(path) if hash_file(&path).as_deref() == Some(&entry.synced_hash) => {
                    let rel = path.strip_prefix(&self.root).unwrap_or(&path).to_path_buf();
                    trash::move_to_trash(self.data_dir, &self.root, &rel)?;
                    self.state.remove(&entry.key)?;
                }
                Slot::Existing(_) => {
                    self.state.put(&Entry {
                        server_version: version,
                        ..entry
                    })?;
                }
                _ => self.state.remove(&entry.key)?,
            }
            if let (Some(uid), Some(folder)) = (uid, folder) {
                self.clean_up_folder(&uid, &folder)?;
            }
        }
        Ok(())
    }

    /// A meeting folder left with nothing sync carries (only audio, say) follows its meeting
    /// into the trash.
    fn clean_up_folder(&mut self, uid: &str, folder: &str) -> Result<(), PennelError> {
        let dir = self.root.join(folder);
        if !dir.is_dir() || !scan::listable(&dir) || !scan::syncable_files(&dir)?.is_empty() {
            return Ok(());
        }
        let empty = std::fs::read_dir(&dir)
            .map(|mut d| d.next().is_none())
            .unwrap_or(false);
        if empty {
            std::fs::remove_dir(&dir).map_err(PennelError::storage)?;
        } else {
            trash::move_to_trash(self.data_dir, &self.root, Path::new(folder))?;
        }
        self.folders.remove(uid);
        Ok(())
    }

    fn push(&mut self) -> Result<(), Stop> {
        let live: HashMap<String, String> = self
            .folders
            .iter()
            .filter(|(_, folder)| !self.is_recording(folder))
            .map(|(uid, folder)| (uid.clone(), folder.clone()))
            .collect();
        let mut files = scan::scan(&self.root, &live)?;
        for file in &mut files {
            if let Some(entry) = tracked_entry(self.state, &file.key)? {
                file.key = entry.key;
            }
        }
        let keys: HashSet<String> = files.iter().map(|f| f.key.clone()).collect();
        for file in &files {
            let entry = self.state.entry(&file.key)?;
            if entry.as_ref().is_some_and(|e| e.synced_hash == file.hash) {
                continue;
            }
            self.push_file(file, entry)?;
        }
        self.push_deletes(&keys)
    }

    /// Whole meetings that vanished outside Pennel are held back when they are suspiciously
    /// many (an emptied or swapped folder), until the user confirms.
    fn push_deletes(&mut self, keys: &HashSet<String>) -> Result<(), Stop> {
        let entries = self.state.entries()?;
        let tracked: HashSet<String> = entries
            .iter()
            .filter_map(|e| match parse_key(&e.key) {
                Some(Key::Meeting { uid, .. }) => Some(uid),
                _ => None,
            })
            .collect();
        let on_purpose = self.state.deleted_on_purpose()?;
        let mut single = Vec::new();
        let mut whole: HashMap<String, Vec<Entry>> = HashMap::new();
        for entry in entries {
            if keys.contains(&entry.key) || !self.vanished(&entry)? {
                continue;
            }
            match parse_key(&entry.key) {
                Some(Key::Meeting { uid, .. }) if !self.folders.contains_key(&uid) => {
                    whole.entry(uid).or_default().push(entry);
                }
                _ => single.push(entry),
            }
        }
        let unexpected = whole
            .keys()
            .filter(|uid| !on_purpose.contains(*uid))
            .count();
        // Meetings deleted in Pennel do not count: what matters is how much of the rest vanished.
        let kept = tracked.difference(&on_purpose).count();
        let confirmed = self.state.deletes_confirmed()?;
        let brake = !confirmed
            && unexpected > 0
            && ((unexpected == kept && tracked.len() >= 2)
                || (unexpected * 2 > kept && unexpected > 5));
        for entry in single {
            self.push_delete(entry)?;
        }
        for (uid, entries) in whole {
            if brake && !on_purpose.contains(&uid) {
                continue;
            }
            for entry in entries {
                self.push_delete(entry)?;
            }
        }
        if brake {
            self.problem = Some(MASS_DELETE.into());
        } else if confirmed {
            self.state.clear_confirmation()?;
        }
        let remaining: Vec<String> = self.state.entries()?.into_iter().map(|e| e.key).collect();
        for uid in on_purpose {
            let prefix = format!("meeting/{uid}/");
            if !remaining.iter().any(|key| key.starts_with(&prefix)) {
                self.state.unmark_deleted(&uid)?;
            }
        }
        Ok(())
    }

    fn push_file(&mut self, file: &LocalFile, entry: Option<Entry>) -> Result<(), Stop> {
        let path = self.root.join(&file.rel_path);
        let Ok(content) = std::fs::read(&path) else {
            return Ok(());
        };
        let hash = sha256_hex(&content);
        if entry.as_ref().is_some_and(|e| e.synced_hash == hash) {
            return Ok(());
        }
        let id = crypto::object_id(self.key, &file.key);
        let payload = Payload {
            key: file.key.clone(),
            folder: file.folder.clone(),
            content,
            modified_ms: modified_ms(&path),
            device: self.device.to_string(),
        };
        let blob = crypto::seal(self.key, &id, self.generation, &payload);
        if blob.len() > MAX_OBJECT_BYTES {
            self.note_problem(TOO_LARGE.into());
            return Ok(());
        }
        let dir = path.parent().unwrap_or(&self.root).to_path_buf();
        let name = path
            .file_name()
            .map(|n| nfc(&n.to_string_lossy()))
            .unwrap_or_default();
        let mut base = entry.as_ref().map_or(0, |e| e.server_version);
        // A conflict is resolved and retried once; anything left over waits for the next pass.
        for _ in 0..3 {
            let result = match self.api.put(&id, base, self.generation, &blob) {
                Err(PennelError::Cloud { code, .. }) if code == "object_too_large" => {
                    self.note_problem(TOO_LARGE.into());
                    return Ok(());
                }
                other => other?,
            };
            match result {
                PutResult::Ok(version) => {
                    self.state.put(&Entry {
                        key: file.key.clone(),
                        object_id: id,
                        server_version: version,
                        synced_hash: hash,
                        folder: file.folder.clone(),
                    })?;
                    self.counts.uploaded += 1;
                    return Ok(());
                }
                PutResult::GenerationChanged => return Err(Stop(SyncStatus::VaultReset)),
                PutResult::Conflict {
                    current,
                    deleted: true,
                } => base = current,
                PutResult::Conflict { current, .. } => {
                    let Some((version, remote)) = self.api.get(&id)? else {
                        base = current;
                        continue;
                    };
                    let remote = match crypto::open(self.key, &id, self.generation, &remote) {
                        Ok(remote) => remote,
                        Err(error) => {
                            self.note_problem(error.to_string());
                            return Ok(());
                        }
                    };
                    if sha256_hex(&remote.content) == hash {
                        self.state.put(&Entry {
                            key: file.key.clone(),
                            object_id: id,
                            server_version: version,
                            synced_hash: hash,
                            folder: file.folder.clone(),
                        })?;
                        return Ok(());
                    }
                    self.conflict_copy(&dir, &name, &remote)?;
                    base = version;
                }
            }
        }
        Ok(())
    }

    /// Only a file whose meeting is still here, or a meeting folder that is gone entirely,
    /// counts as deleted. An unreadable `meeting.md` or a recording meeting never does.
    fn vanished(&self, entry: &Entry) -> Result<bool, PennelError> {
        let absent = |dir: &Path, name: &str| {
            scan::listable(dir) && !matches!(file_slot(dir, name), Slot::Existing(_))
        };
        Ok(match parse_key(&entry.key) {
            // A missing `.templates` folder (moved, or not synced into this root yet) is not
            // a request to delete every template.
            Some(Key::Template { name }) => {
                let dir = self.root.join(TEMPLATES_DIR);
                dir.is_dir() && absent(&dir, &name)
            }
            Some(Key::Meeting { uid, name }) => match self.folders.get(&uid) {
                // A folder renamed while this pass runs is found again next pass.
                Some(folder) => {
                    let dir = self.root.join(folder);
                    !self.is_recording(folder) && dir.is_dir() && absent(&dir, &name)
                }
                None => match entry.folder.as_deref() {
                    Some(folder) if !self.is_recording(folder) => {
                        find_dir(&self.root, folder)?.is_none()
                    }
                    _ => false,
                },
            },
            None => false,
        })
    }

    fn push_delete(&mut self, entry: Entry) -> Result<(), Stop> {
        match self
            .api
            .delete(&entry.object_id, entry.server_version, self.generation)?
        {
            PutResult::Ok(_) | PutResult::Conflict { deleted: true, .. } => {
                self.state.remove(&entry.key)?;
            }
            PutResult::GenerationChanged => return Err(Stop(SyncStatus::VaultReset)),
            // Changed on another device after this one deleted it: bring that version back.
            PutResult::Conflict { .. } => {
                self.state.remove(&entry.key)?;
                if let Some((version, blob)) = self.api.get(&entry.object_id)? {
                    self.apply_remote(&entry.object_id, version, &blob, None, None)?;
                }
            }
        }
        Ok(())
    }
}

/// The entry for the file a key from disk names. A download whose name was not portable lives
/// under its portable name but keeps the key it was synced under.
fn tracked_entry(state: &SyncState, key: &str) -> Result<Option<Entry>, PennelError> {
    if let Some(entry) = state.entry(key)? {
        return Ok(Some(entry));
    }
    let Some((prefix, name)) = key.rsplit_once('/') else {
        return Ok(None);
    };
    if !name.contains('_') {
        return Ok(None);
    }
    let prefix = format!("{prefix}/");
    Ok(state
        .entries_with_prefix(&prefix)?
        .into_iter()
        .find(|entry| {
            entry.key[prefix.len()..] != *name && portable_name(&entry.key[prefix.len()..]) == name
        }))
}

fn has_conflict_copy(dir: &Path) -> bool {
    std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .any(|entry| {
            Path::new(&entry.file_name())
                .file_stem()
                .is_some_and(|stem| stem.to_string_lossy().contains(" (conflict "))
        })
}

/// Per meeting: a conflict copy is present, something is not uploaded yet, or all is synced.
pub fn item_states(root: &Path, state: &SyncState) -> Result<Vec<MeetingSyncState>, PennelError> {
    let mut states = Vec::new();
    for folder in scan::meeting_folders(root)? {
        let dir = root.join(&folder.name);
        let item = if has_conflict_copy(&dir) {
            SyncItemState::Conflict
        } else {
            let mut pending = folder.uid.is_none();
            if let Some(uid) = folder.uid.as_deref() {
                for (name, path) in scan::syncable_files(&dir)? {
                    let entry = tracked_entry(state, &scan::meeting_key(uid, &name))?;
                    if entry.is_none_or(|e| hash_file(&path).as_deref() != Some(&e.synced_hash)) {
                        pending = true;
                        break;
                    }
                }
            }
            if pending {
                SyncItemState::Pending
            } else {
                SyncItemState::Synced
            }
        };
        states.push(MeetingSyncState {
            id: folder.name,
            state: item,
        });
    }
    Ok(states)
}

fn provider_of(segment: &str) -> Option<&'static str> {
    if segment == "Dropbox" || segment.starts_with("Dropbox-") || segment.starts_with("Dropbox (") {
        Some("Dropbox")
    } else if segment.starts_with("OneDrive") {
        Some("OneDrive")
    } else if segment == "iCloudDrive" || segment == "Mobile Documents" {
        Some("iCloud")
    } else if segment == "Google Drive" || segment.starts_with("GoogleDrive") {
        Some("Google Drive")
    } else {
        None
    }
}

/// Warns when the meetings folder already lives in a cloud-synced folder.
pub fn folder_warning(root: &Path, home: &Path) -> Option<String> {
    let segments: Vec<String> = root
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    let in_library = |sub: &str| root.starts_with(home.join("Library").join(sub));
    let name = if in_library("Mobile Documents") {
        Some("iCloud")
    } else if in_library("CloudStorage") {
        let provider = root
            .strip_prefix(home.join("Library").join("CloudStorage"))
            .ok()
            .and_then(|rest| rest.components().next())
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .unwrap_or_default();
        Some(provider_of(&provider).unwrap_or("iCloud"))
    } else {
        segments.iter().find_map(|segment| provider_of(segment))
    };
    name.map(|name| {
        format!(
            "This folder already syncs through {name}. Using two sync services can cause conflicts."
        )
    })
}

#[cfg(test)]
mod tests {
    use super::api::fake::FakeApi;
    use super::*;
    use crate::cloud::crypto;
    use crate::meetings::MeetingStore;
    use chrono::{Local, TimeZone};
    use std::path::PathBuf;

    struct Device {
        _data: tempfile::TempDir,
        data_dir: PathBuf,
        root: PathBuf,
        state: SyncState,
        key: String,
        name: String,
    }

    fn shared_key() -> String {
        VaultKey::generate().to_b64()
    }

    impl Device {
        fn new(name: &str, key: &str) -> Self {
            let data = tempfile::tempdir().unwrap();
            let data_dir = data.path().join("data");
            let root = data.path().join("Meetings");
            std::fs::create_dir_all(&data_dir).unwrap();
            std::fs::create_dir_all(&root).unwrap();
            let state = SyncState::open(&data_dir).unwrap();
            state.set_generation(1).unwrap();
            Self {
                _data: data,
                data_dir,
                root,
                state,
                key: key.to_string(),
                name: name.to_string(),
            }
        }

        fn sync(&self, api: &FakeApi) -> SyncReport {
            self.sync_recording(api, &[])
        }

        fn sync_recording(&self, api: &FakeApi, recording: &[String]) -> SyncReport {
            let key = VaultKey::from_b64(&self.key).unwrap();
            run(PassInput {
                api,
                state: &self.state,
                key: &key,
                root: &self.root,
                data_dir: &self.data_dir,
                device: &self.name,
                recording,
            })
        }

        fn store(&self) -> MeetingStore {
            MeetingStore::new(&self.root)
        }

        fn engine_states(&self) -> Vec<SyncItemState> {
            item_states(&self.root, &self.state)
                .unwrap()
                .into_iter()
                .map(|s| s.state)
                .collect()
        }

        fn path(&self, rel: &str) -> PathBuf {
            self.root.join(rel)
        }

        fn read(&self, rel: &str) -> Option<String> {
            std::fs::read_to_string(self.path(rel)).ok()
        }

        fn write(&self, rel: &str, text: &str) {
            let path = self.path(rel);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }

        fn names_in(&self, folder: &str) -> Vec<String> {
            let mut names: Vec<String> = std::fs::read_dir(self.path(folder))
                .map(|entries| {
                    entries
                        .filter_map(Result::ok)
                        .map(|e| e.file_name().to_string_lossy().into_owned())
                        .collect()
                })
                .unwrap_or_default();
            names.sort();
            names
        }

        fn folders(&self) -> Vec<String> {
            self.names_in("")
                .into_iter()
                .filter(|n| !n.starts_with('.'))
                .collect()
        }

        fn conflict_copies(&self, folder: &str) -> Vec<String> {
            self.names_in(folder)
                .into_iter()
                .filter(|n| n.contains(" (conflict "))
                .collect()
        }

        fn trash_files(&self) -> Vec<String> {
            fn walk(dir: &Path, out: &mut Vec<String>) {
                for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
                    if entry.path().is_dir() {
                        walk(&entry.path(), out);
                    } else {
                        out.push(entry.file_name().to_string_lossy().into_owned());
                    }
                }
            }
            let mut out = Vec::new();
            walk(&self.data_dir.join(trash::TRASH_DIR), &mut out);
            out.sort();
            out
        }
    }

    fn setup() -> (FakeApi, Device, Device) {
        let api = FakeApi::new();
        let key = shared_key();
        let wrapped_key = VaultKey::from_b64(&key).unwrap();
        let code = crypto::new_recovery_code();
        let (wrapped, kdf) = crypto::wrap(&wrapped_key, &code).unwrap();
        api.put_vault(1, &wrapped, &kdf, true).unwrap();
        (api, Device::new("Mac", &key), Device::new("Laptop", &key))
    }

    fn start() -> chrono::DateTime<Local> {
        Local.with_ymd_and_hms(2026, 10, 25, 14, 0, 0).unwrap()
    }

    fn synced(report: &SyncReport) {
        assert_eq!(report.status, SyncStatus::Synced, "{report:?}");
    }

    fn insight_fixture(
        device: &Device,
    ) -> (
        String,
        crate::MeetingTranscriptSnapshot,
        crate::InsightSnapshot,
    ) {
        let id = device
            .store()
            .create("Owned reviewed insights", start())
            .unwrap();
        let row = crate::MeetingUtterance::new(
            crate::Segment {
                start_ms: 1_000,
                end_ms: 5_000,
                speaker: crate::Speaker::Others,
                text: "Jan will send the report on Friday.".into(),
                failed: false,
                voice: None,
            },
            Some("room_microphone".into()),
        );
        let source = device
            .store()
            .write_transcript_snapshot(&id, Some(0), &[row])
            .unwrap();
        let proposal = crate::meeting_insights::tests::candidate(&source);
        let insights = device
            .store()
            .merge_insight_candidates(
                &id,
                0,
                &[proposal],
                &source,
                &crate::InferenceCancellation::new(),
            )
            .unwrap();
        (id, source, insights)
    }

    #[test]
    fn portable_review_store_near_its_limit_syncs_encrypted_without_becoming_transcript_text() {
        let (api, a, b) = setup();
        let (id, _, insights) = insight_fixture(&a);
        let edited = format!(
            "INSIGHT_ONLY_SENTINEL {}",
            "x".repeat(crate::MAX_INSIGHTS_BYTES - 8_000)
        );
        let item = &insights.items[0];
        let reviewed = a
            .store()
            .update_insight_review(
                &id,
                insights.revision,
                &item.id,
                item.revision,
                &crate::InsightReviewUpdate {
                    review_state: Some(crate::InsightReviewState::Accepted),
                    edited_text: Some(edited.clone()),
                    ..Default::default()
                },
            )
            .unwrap();
        let raw = std::fs::read(a.path(&format!("{id}/insights.json"))).unwrap();
        assert!(raw.len() <= crate::MAX_INSIGHTS_BYTES);
        synced(&a.sync(&api));
        synced(&b.sync(&api));
        assert_eq!(b.store().read_insights(&id).unwrap(), reviewed);
        assert_eq!(
            std::fs::read(b.path(&format!("{id}/insights.json"))).unwrap(),
            raw
        );
        assert!(
            b.store()
                .export_insights_markdown(&id)
                .unwrap()
                .starts_with(&edited)
        );
        assert!(
            !b.store()
                .read_transcript(&id)
                .iter()
                .any(|segment| segment.text.contains("INSIGHT_ONLY_SENTINEL"))
        );
        let key = crypto::object_id(
            &VaultKey::from_b64(&a.key).unwrap(),
            &format!("meeting/{}/insights.json", reviewed.meeting_uid),
        );
        let (_, sealed) = api.get(&key).unwrap().unwrap();
        assert!(sealed.len() <= MAX_OBJECT_BYTES);
        assert!(
            !sealed
                .windows(b"INSIGHT_ONLY_SENTINEL".len())
                .any(|window| window == b"INSIGHT_ONLY_SENTINEL")
        );
    }

    #[test]
    fn concurrent_review_edits_survive_sync_as_the_current_file_and_a_conflict_copy() {
        let (api, a, b) = setup();
        let (id, _, initial) = insight_fixture(&a);
        synced(&a.sync(&api));
        synced(&b.sync(&api));
        let item = &initial.items[0];
        for (device, text) in [
            (&a, "User edit from the first device."),
            (&b, "User edit from the second device."),
        ] {
            device
                .store()
                .update_insight_review(
                    &id,
                    initial.revision,
                    &item.id,
                    item.revision,
                    &crate::InsightReviewUpdate {
                        review_state: Some(crate::InsightReviewState::Accepted),
                        edited_text: Some(text.into()),
                        ..Default::default()
                    },
                )
                .unwrap();
        }
        synced(&a.sync(&api));
        synced(&b.sync(&api));
        let files: Vec<_> = b
            .names_in(&id)
            .into_iter()
            .filter(|name| {
                name == "insights.json"
                    || name.starts_with("insights (conflict ") && name.ends_with(").json")
            })
            .collect();
        assert_eq!(files.len(), 2);
        let edits: HashSet<_> = files
            .iter()
            .map(|name| {
                let value: serde_json::Value =
                    serde_json::from_str(&b.read(&format!("{id}/{name}")).unwrap()).unwrap();
                value["items"][0]["edited_text"]
                    .as_str()
                    .unwrap()
                    .to_owned()
            })
            .collect();
        assert!(edits.contains("User edit from the first device."));
        assert!(edits.contains("User edit from the second device."));
        assert_eq!(
            b.store().read_insights(&id).unwrap().items[0].review_state,
            crate::InsightReviewState::Accepted
        );
    }

    #[test]
    fn canonical_transcript_parts_upload_within_sealed_limit_and_reopen_on_another_device() {
        use crate::meeting_utterances::{METADATA_PART_BYTES, MeetingUtterance, TimedTextSpan};
        let (api, a, b) = setup();
        let id = a.store().create("Portable rapid turns", start()).unwrap();
        let rows: Vec<_> = (0..3_600)
            .map(|turn| {
                let mut row = MeetingUtterance::new(
                    crate::Segment {
                        start_ms: turn * 500,
                        end_ms: turn * 500 + 400,
                        speaker: crate::Speaker::Others,
                        text: "Ja. Yes.".into(),
                        failed: false,
                        voice: Some(2),
                    },
                    Some("microphone".into()),
                );
                row.metadata.spans = vec![TimedTextSpan(0, 3, turn * 500, turn * 500 + 200)];
                row
            })
            .collect();
        let original = a
            .store()
            .write_transcript_snapshot(&id, Some(0), &rows)
            .unwrap();
        synced(&a.sync(&api));
        let parts: Vec<_> = a
            .names_in(&id)
            .into_iter()
            .filter(|name| name.starts_with("transcript.meta.") && name != "transcript.meta.json")
            .collect();
        assert!(parts.len() > 1);
        let vault_key = VaultKey::from_b64(&a.key).unwrap();
        for name in &parts {
            assert!(
                std::fs::metadata(a.path(&format!("{id}/{name}")))
                    .unwrap()
                    .len()
                    <= METADATA_PART_BYTES as u64
            );
            let object_id = crypto::object_id(
                &vault_key,
                &format!("meeting/{}/{name}", original.meeting_uid),
            );
            let (_, sealed) = api
                .get(&object_id)
                .unwrap()
                .expect("metadata part uploaded");
            assert!(sealed.len() <= MAX_OBJECT_BYTES);
        }
        synced(&b.sync(&api));
        let reopened = b.store().read_transcript_snapshot(&id).unwrap();
        assert_eq!(reopened, original);
        assert_eq!(reopened.utterances, rows);
        assert_eq!(
            b.names_in(&id)
                .into_iter()
                .filter(|name| parts.contains(name))
                .count(),
            parts.len()
        );
    }

    #[test]
    fn hour_of_transcript_revisions_bounds_live_sync_parts_without_triggering_meeting_delete_brake()
    {
        use crate::meeting_utterances::MeetingUtterance;
        let (api, a, b) = setup();
        let id = a.store().create("Hour of revisions", start()).unwrap();
        let mut rows = Vec::new();
        for chunk in 0..240 {
            rows.push(MeetingUtterance::new(
                crate::Segment {
                    start_ms: chunk * 15_000,
                    end_ms: chunk * 15_000 + 4_000,
                    speaker: crate::Speaker::Others,
                    text: "A finalized Dutch and English acknowledgement.".into(),
                    failed: false,
                    voice: Some(2),
                },
                Some("microphone".into()),
            ));
            let snapshot = a
                .store()
                .write_transcript_snapshot(&id, None, &rows)
                .unwrap();
            if chunk % 10 == 9 {
                synced(&a.sync(&api));
                synced(&b.sync(&api));
                let parts = a
                    .names_in(&id)
                    .into_iter()
                    .filter(|name| {
                        name.starts_with("transcript.meta.") && name != "transcript.meta.json"
                    })
                    .count();
                assert!(api.live_objects() <= parts + 4);
                assert_eq!(b.store().read_transcript_snapshot(&id).unwrap(), snapshot);
            }
        }
        assert_eq!(
            b.store().read_transcript_snapshot(&id).unwrap().utterances,
            rows
        );
    }

    #[test]
    fn metadata_part_retirement_keeps_parts_needed_by_a_synced_conflict_manifest() {
        use crate::meeting_utterances::MeetingUtterance;
        let (api, a, b) = setup();
        let id = a.store().create("Metadata conflict", start()).unwrap();
        let row = MeetingUtterance::new(
            crate::Segment {
                start_ms: 0,
                end_ms: 400,
                speaker: crate::Speaker::Others,
                text: "Original source.".into(),
                failed: false,
                voice: Some(1),
            },
            Some("microphone".into()),
        );
        a.store()
            .write_transcript_snapshot(&id, Some(0), std::slice::from_ref(&row))
            .unwrap();
        synced(&a.sync(&api));
        synced(&b.sync(&api));
        let manifest = b.read(&format!("{id}/transcript.meta.json")).unwrap();
        let old_part = b
            .names_in(&id)
            .into_iter()
            .find(|name| name.starts_with("transcript.meta.") && name != "transcript.meta.json")
            .unwrap();
        b.write(
            &format!("{id}/transcript.meta (conflict Mac 2026-10-25 1400).json"),
            &manifest,
        );
        let mut changed = row;
        changed.segment.text = "New source revision.".into();
        a.store()
            .write_transcript_snapshot(&id, Some(1), &[changed])
            .unwrap();
        assert!(!a.path(&format!("{id}/{old_part}")).exists());
        synced(&a.sync(&api));
        synced(&b.sync(&api));
        assert!(
            b.path(&format!("{id}/{old_part}")).exists(),
            "Remote retirement removed conflict recovery evidence"
        );
        let restored_manifest: serde_json::Value = serde_json::from_str(&manifest).unwrap();
        assert!(
            restored_manifest["parts"]
                .as_array()
                .unwrap()
                .iter()
                .all(|part| {
                    b.path(&format!(
                        "{id}/transcript.meta.{}.json",
                        part["sha256"].as_str().unwrap()
                    ))
                    .exists()
                })
        );
    }

    #[test]
    fn deleting_a_whole_canonical_meeting_does_not_resurrect_unmodified_metadata_parts() {
        use crate::meeting_utterances::MeetingUtterance;
        let (api, a, b) = setup();
        let id = a
            .store()
            .create("Explicit metadata deletion", start())
            .unwrap();
        let row = MeetingUtterance::new(
            crate::Segment {
                start_ms: 0,
                end_ms: 400,
                speaker: crate::Speaker::Others,
                text: "Owned source.".into(),
                failed: false,
                voice: Some(1),
            },
            Some("microphone".into()),
        );
        a.store()
            .write_transcript_snapshot(&id, Some(0), &[row])
            .unwrap();
        synced(&a.sync(&api));
        synced(&b.sync(&api));
        a.store().delete(&id).unwrap();
        synced(&a.sync(&api));
        synced(&b.sync(&api));
        assert!(b.store().list().is_empty());
        assert_eq!(api.live_objects(), 0);
        assert!(
            b.trash_files()
                .iter()
                .any(|name| name == "transcript.meta.json")
        );
    }

    /// A meeting with notes, transcript and audio, synced from `a` to `b`.
    fn shared_meeting(api: &FakeApi, a: &Device, b: &Device) -> String {
        let id = a.store().create("Klant", start()).unwrap();
        a.write(&format!("{id}/notes.md"), "notities\n");
        a.write(&format!("{id}/transcript.md"), "[00:01] **Ik:** Hallo\n");
        a.write(&format!("{id}/ik.m4a"), "audio");
        synced(&a.sync(api));
        synced(&b.sync(api));
        id
    }

    fn assert_conflict_name(name: &str, stem: &str, ext: &str) {
        let inner = name
            .strip_prefix(&format!("{stem} (conflict "))
            .and_then(|rest| rest.strip_suffix(&format!("){ext}")))
            .unwrap_or_else(|| panic!("unexpected conflict name {name}"));
        let (device, stamp) = inner.split_at(inner.len() - "2026-10-25 1400".len());
        assert!(!device.trim().is_empty(), "{name}");
        assert!(device.ends_with(' '), "{name}");
        assert!(
            chrono::NaiveDateTime::parse_from_str(stamp, "%Y-%m-%d %H%M").is_ok(),
            "{name}"
        );
    }

    #[test]
    fn new_meeting_reaches_second_device() {
        let (api, a, b) = setup();
        let id = a.store().create("Klant", start()).unwrap();
        a.write(&format!("{id}/notes.md"), "notities\n");
        a.write(&format!("{id}/transcript.md"), "[00:01] **Ik:** Hallo\n");
        a.write(&format!("{id}/ik.m4a"), "audio");
        let report = a.sync(&api);
        synced(&report);
        assert_eq!(report.uploaded, 3);
        let report = b.sync(&api);
        synced(&report);
        assert_eq!(report.downloaded, 3);
        assert_eq!(
            b.names_in(&id),
            vec!["meeting.md", "notes.md", "transcript.md"]
        );
        for name in ["meeting.md", "notes.md", "transcript.md"] {
            assert_eq!(
                b.read(&format!("{id}/{name}")),
                a.read(&format!("{id}/{name}"))
            );
        }
        assert_eq!(b.store().uid_of(&id), a.store().uid_of(&id));
        let again = b.sync(&api);
        assert_eq!((again.uploaded, again.downloaded), (0, 0));
    }

    #[test]
    fn edit_roundtrips_back() {
        let (api, a, b) = setup();
        let id = shared_meeting(&api, &a, &b);
        b.write(&format!("{id}/notes.md"), "aangevuld op laptop\n");
        assert_eq!(b.sync(&api).uploaded, 1);
        let report = a.sync(&api);
        synced(&report);
        assert_eq!(report.downloaded, 1);
        assert_eq!(
            a.read(&format!("{id}/notes.md")).as_deref(),
            Some("aangevuld op laptop\n")
        );
        assert!(a.conflict_copies(&id).is_empty());
        assert!(b.conflict_copies(&id).is_empty());
    }

    #[test]
    fn concurrent_edit_creates_conflict_copy_and_keeps_local() {
        let (api, a, b) = setup();
        let id = shared_meeting(&api, &a, &b);
        a.write(&format!("{id}/notes.md"), "versie A\n");
        synced(&a.sync(&api));
        b.write(&format!("{id}/notes.md"), "versie B\n");
        let report = b.sync(&api);
        synced(&report);
        assert_eq!(report.conflicts, 1);
        assert_eq!(
            b.read(&format!("{id}/notes.md")).as_deref(),
            Some("versie B\n")
        );
        let copies = b.conflict_copies(&id);
        assert_eq!(copies.len(), 1, "{copies:?}");
        assert_conflict_name(&copies[0], "notes", ".md");
        assert!(copies[0].contains("(conflict Mac "), "{copies:?}");
        assert_eq!(
            b.read(&format!("{id}/{}", copies[0])).as_deref(),
            Some("versie A\n")
        );

        synced(&a.sync(&api));
        assert_eq!(
            a.read(&format!("{id}/notes.md")).as_deref(),
            Some("versie B\n")
        );
        assert_eq!(
            a.read(&format!("{id}/{}", copies[0])).as_deref(),
            Some("versie A\n")
        );
        let settle = b.sync(&api);
        assert_eq!(
            (settle.uploaded, settle.downloaded, settle.conflicts),
            (0, 0, 0)
        );
    }

    #[test]
    fn delete_moves_to_trash_on_other_device() {
        let (api, a, b) = setup();
        let id = shared_meeting(&api, &a, &b);
        b.write(&format!("{id}/ik.m4a"), "audio op laptop");
        a.store().delete(&id).unwrap();
        synced(&a.sync(&api));
        assert_eq!(api.live_objects(), 0);
        synced(&b.sync(&api));
        assert!(!b.path(&id).exists());
        assert_eq!(
            b.trash_files(),
            vec!["ik.m4a", "meeting.md", "notes.md", "transcript.md"]
        );
        let again = a.sync(&api);
        synced(&again);
        assert!(!a.path(&id).exists());
    }

    #[test]
    fn locally_modified_file_survives_remote_delete() {
        let (api, a, b) = setup();
        let id = shared_meeting(&api, &a, &b);
        a.store().delete(&id).unwrap();
        synced(&a.sync(&api));
        b.write(&format!("{id}/notes.md"), "nog bezig op laptop\n");
        synced(&b.sync(&api));
        assert_eq!(
            b.read(&format!("{id}/notes.md")).as_deref(),
            Some("nog bezig op laptop\n")
        );
        assert!(b.path(&format!("{id}/meeting.md")).exists());
        assert_eq!(b.trash_files(), vec!["transcript.md"]);
        synced(&a.sync(&api));
        assert_eq!(
            a.read(&format!("{id}/notes.md")).as_deref(),
            Some("nog bezig op laptop\n")
        );
        assert!(a.path(&format!("{id}/meeting.md")).exists());
    }

    #[test]
    fn rename_on_a_edit_on_b() {
        let (api, a, b) = setup();
        let id = shared_meeting(&api, &a, &b);
        let renamed = a.store().rename(&id, "Nieuwe naam").unwrap();
        assert_ne!(renamed, id);
        synced(&a.sync(&api));
        b.write(&format!("{id}/notes.md"), "edit op laptop\n");
        synced(&b.sync(&api));
        assert_eq!(b.folders(), vec![renamed.clone()]);
        assert_eq!(
            b.read(&format!("{renamed}/notes.md")).as_deref(),
            Some("edit op laptop\n")
        );
        assert_eq!(
            b.read(&format!("{renamed}/meeting.md")),
            a.read(&format!("{renamed}/meeting.md"))
        );
        synced(&a.sync(&api));
        assert_eq!(
            a.read(&format!("{renamed}/notes.md")).as_deref(),
            Some("edit op laptop\n")
        );
        assert!(a.conflict_copies(&renamed).is_empty());
        assert!(b.conflict_copies(&renamed).is_empty());
    }

    #[test]
    fn first_sync_matches_identical_files_without_conflicts() {
        let (api, a, b) = setup();
        let legacy = "2026-01-05 0900 Oud overleg";
        let meeting = "---\ntitle: Oud overleg\nstart: 2026-01-05T09:00:00+01:00\nduration_min: 30\ntemplate: \n---\n\nSamenvatting\n";
        let shared = a.store().create("Gedeeld", start()).unwrap();
        a.write(&format!("{shared}/notes.md"), "gedeeld\n");
        for device in [&a, &b] {
            device.write(&format!("{legacy}/meeting.md"), meeting);
            device.write(&format!("{legacy}/notes.md"), "oude notities\n");
            device.write(".templates/sales.md", "# Sales\n");
        }
        for name in ["meeting.md", "notes.md"] {
            b.write(
                &format!("{shared}/{name}"),
                &a.read(&format!("{shared}/{name}")).unwrap(),
            );
        }
        synced(&a.sync(&api));
        let report = b.sync(&api);
        synced(&report);
        assert_eq!(report.conflicts, 0);
        assert_eq!(report.uploaded, 0);
        let mut expected = vec![legacy.to_string(), shared.clone()];
        expected.sort();
        assert_eq!(b.folders(), expected);
        for folder in [legacy, shared.as_str()] {
            assert!(b.conflict_copies(folder).is_empty());
            for name in ["meeting.md", "notes.md"] {
                assert_eq!(
                    b.read(&format!("{folder}/{name}")),
                    a.read(&format!("{folder}/{name}"))
                );
            }
        }
        assert!(b.store().uid_of(legacy).is_some());
        let again = a.sync(&api);
        assert_eq!(
            (again.uploaded, again.downloaded, again.conflicts),
            (0, 0, 0)
        );
    }

    #[test]
    fn external_edit_during_pull_is_kept() {
        let (api, a, b) = setup();
        let id = shared_meeting(&api, &a, &b);
        a.write(&format!("{id}/notes.md"), "van de server\n");
        synced(&a.sync(&api));
        let target = b.path(&format!("{id}/notes.md"));
        let mut fired = false;
        api.on_get(move |_| {
            if !fired {
                fired = true;
                std::fs::write(&target, "getypt in Obsidian\n").unwrap();
            }
        });
        let report = b.sync(&api);
        synced(&report);
        assert_eq!(
            b.read(&format!("{id}/notes.md")).as_deref(),
            Some("getypt in Obsidian\n")
        );
        let copies = b.conflict_copies(&id);
        assert_eq!(copies.len(), 1, "{copies:?}");
        assert_eq!(
            b.read(&format!("{id}/{}", copies[0])).as_deref(),
            Some("van de server\n")
        );
    }

    #[test]
    fn cursor_expired_does_full_compare_without_deleting() {
        let (api, a, b) = setup();
        let first = shared_meeting(&api, &a, &b);
        let second = a.store().create("Tweede", start()).unwrap();
        a.write(&format!("{second}/notes.md"), "v1\n");
        synced(&a.sync(&api));
        synced(&b.sync(&api));
        a.store().delete(&first).unwrap();
        a.write(&format!("{second}/notes.md"), "v2\n");
        synced(&a.sync(&api));
        api.purge_tombstones();

        let report = b.sync(&api);
        synced(&report);
        assert_eq!(
            b.read(&format!("{second}/notes.md")).as_deref(),
            Some("v2\n")
        );
        for name in ["meeting.md", "notes.md", "transcript.md"] {
            assert!(b.path(&format!("{first}/{name}")).exists(), "{name}");
        }
        assert!(b.trash_files().is_empty());
        synced(&a.sync(&api));
        assert!(a.path(&format!("{first}/notes.md")).exists());
    }

    #[test]
    fn recording_meeting_is_skipped() {
        let (api, a, b) = setup();
        let recording = a.store().create("Opname", start()).unwrap();
        let other = a.store().create("Andere", start()).unwrap();
        a.write(&format!("{recording}/transcript.md"), "[00:01] **Ik:** a\n");
        synced(&a.sync_recording(&api, std::slice::from_ref(&recording)));
        synced(&b.sync(&api));
        assert_eq!(b.folders(), vec![other.clone()]);
        synced(&a.sync(&api));
        synced(&b.sync(&api));
        assert!(b.path(&format!("{recording}/transcript.md")).exists());

        a.write(&format!("{recording}/notes.md"), "na afloop\n");
        synced(&a.sync(&api));
        synced(&b.sync_recording(&api, std::slice::from_ref(&recording)));
        assert!(!b.path(&format!("{recording}/notes.md")).exists());
        synced(&b.sync(&api));
        assert_eq!(
            b.read(&format!("{recording}/notes.md")).as_deref(),
            Some("na afloop\n")
        );

        std::fs::remove_dir_all(b.path(&recording)).unwrap();
        b.store().create("Opname", start()).unwrap();
        synced(&b.sync_recording(&api, std::slice::from_ref(&recording)));
        assert_eq!(api.live_objects(), 4);
    }

    #[test]
    fn templates_sync() {
        let (api, a, b) = setup();
        a.write(".templates/sales.md", "# Sales\n");
        a.write(".templates/notes.txt", "geen sjabloon");
        synced(&a.sync(&api));
        synced(&b.sync(&api));
        assert_eq!(b.read(".templates/sales.md").as_deref(), Some("# Sales\n"));
        assert!(b.read(".templates/notes.txt").is_none());
        b.write(".templates/sales.md", "# Sales v2\n");
        synced(&b.sync(&api));
        synced(&a.sync(&api));
        assert_eq!(
            a.read(".templates/sales.md").as_deref(),
            Some("# Sales v2\n")
        );
        std::fs::remove_file(a.path(".templates/sales.md")).unwrap();
        synced(&a.sync(&api));
        synced(&b.sync(&api));
        assert!(b.read(".templates/sales.md").is_none());
        assert_eq!(b.trash_files(), vec!["sales.md"]);
    }

    #[test]
    fn vault_reset_on_other_device_reports_vault_reset() {
        let (api, a, b) = setup();
        shared_meeting(&api, &a, &b);
        api.reset_vault(1).unwrap();
        let report = b.sync(&api);
        assert_eq!(report.status, SyncStatus::VaultReset);
    }

    #[test]
    fn storage_full_reported_with_server_message() {
        let (api, a, _) = setup();
        api.set_storage_limit(Some(10));
        a.store().create("Groot", start()).unwrap();
        assert_eq!(
            a.sync(&api).status,
            SyncStatus::StorageFull {
                message: "Your cloud storage is full (1 GB). Local features keep working.".into()
            }
        );
    }

    #[test]
    fn network_error_reports_offline() {
        let (api, a, _) = setup();
        a.store().create("Offline", start()).unwrap();
        api.set_offline(true);
        assert_eq!(a.sync(&api).status, SyncStatus::Offline);
        api.set_offline(false);
        assert_eq!(a.sync(&api).uploaded, 1);
    }

    #[test]
    fn existing_meeting_gets_uid_once() {
        let (api, a, _) = setup();
        let original = "---\ntitle: Oud\nstart: 2026-01-01T10:00:00+01:00\n---\n\nInhoud\n";
        a.write("2026-01-01 1000 Oud/meeting.md", original);
        synced(&a.sync(&api));
        let uid = a
            .store()
            .uid_of("2026-01-01 1000 Oud")
            .expect("uid assigned");
        let text = a.read("2026-01-01 1000 Oud/meeting.md").unwrap();
        assert_eq!(
            text,
            format!(
                "---\ntitle: Oud\nstart: 2026-01-01T10:00:00+01:00\npennel_id: {uid}\n---\n\nInhoud\n"
            )
        );
        let again = a.sync(&api);
        assert_eq!(again.uploaded, 0);
        assert_eq!(a.read("2026-01-01 1000 Oud/meeting.md").unwrap(), text);
    }

    #[test]
    fn folder_warning_detects_cloud_paths() {
        let home = Path::new("/Users/els");
        let message = |name: &str| {
            Some(format!(
                "This folder already syncs through {name}. Using two sync services can cause conflicts."
            ))
        };
        let cases = [
            (
                "/Users/els/Library/Mobile Documents/com~apple~CloudDocs/Pennel",
                message("iCloud"),
            ),
            (
                "/Users/els/Library/CloudStorage/OneDrive-Persoonlijk/Pennel",
                message("OneDrive"),
            ),
            (
                "/Users/els/Library/CloudStorage/GoogleDrive-els@x.be/My Drive/Pennel",
                message("Google Drive"),
            ),
            (
                "/Users/els/Library/CloudStorage/Dropbox/Pennel",
                message("Dropbox"),
            ),
            ("/Users/els/Dropbox/Pennel", message("Dropbox")),
            ("C:/Users/els/OneDrive - AE/Pennel", message("OneDrive")),
            ("C:/Users/els/iCloudDrive/Pennel", message("iCloud")),
            ("G:/Google Drive/Pennel", message("Google Drive")),
            ("/Users/els/Pennel", None),
            ("/Users/els/Documents/DropboxBackup/Pennel", None),
        ];
        for (path, expected) in cases {
            assert_eq!(folder_warning(Path::new(path), home), expected, "{path}");
        }
    }

    #[test]
    fn missing_meetings_root_reports_error_and_deletes_nothing() {
        let (api, a, _) = setup();
        let id = a.store().create("Extern", start()).unwrap();
        a.write(&format!("{id}/notes.md"), "n\n");
        synced(&a.sync(&api));
        std::fs::remove_dir_all(&a.root).unwrap();
        assert!(matches!(a.sync(&api).status, SyncStatus::Error { .. }));
        assert_eq!(api.live_objects(), 2);
    }

    #[test]
    fn unreadable_meeting_md_deletes_nothing_remotely() {
        let (api, a, b) = setup();
        let id = shared_meeting(&api, &a, &b);
        a.write(&format!("{id}/meeting.md"), "kapotte frontmatter");
        synced(&a.sync(&api));
        assert_eq!(api.live_objects(), 3);
        assert!(b.path(&format!("{id}/notes.md")).exists());
    }

    #[test]
    fn case_variant_local_file_is_not_overwritten() {
        let (api, a, b) = setup();
        let id = shared_meeting(&api, &a, &b);
        a.write(&format!("{id}/Ideeen.md"), "van A\n");
        synced(&a.sync(&api));
        b.write(&format!("{id}/IDEEEN.md"), "van B\n");
        synced(&b.sync(&api));
        assert_eq!(
            b.read(&format!("{id}/IDEEEN.md")).as_deref(),
            Some("van B\n")
        );
        let copies = b.conflict_copies(&id);
        assert_eq!(copies.len(), 1, "{copies:?}");
        assert_conflict_name(&copies[0], "Ideeen", ".md");
        assert_eq!(
            b.read(&format!("{id}/{}", copies[0])).as_deref(),
            Some("van A\n")
        );
    }

    #[test]
    fn unreadable_object_is_skipped_and_reported() {
        let (api, a, b) = setup();
        let id = a.store().create("Leesbaar", start()).unwrap();
        a.write(&format!("{id}/notes.md"), "ok\n");
        synced(&a.sync(&api));
        let bogus = crypto::object_id(&VaultKey::generate(), "templates/x.md");
        assert!(matches!(
            api.put(&bogus, 0, 1, b"garbage").unwrap(),
            api::PutResult::Ok(_)
        ));
        let report = b.sync(&api);
        assert_eq!(
            report.status,
            SyncStatus::Error {
                message: "A file from Pennel Cloud could not be decrypted.".into()
            }
        );
        assert_eq!(b.read(&format!("{id}/notes.md")).as_deref(), Some("ok\n"));
    }

    #[cfg(unix)]
    #[test]
    fn unreadable_templates_folder_deletes_nothing() {
        use std::os::unix::fs::PermissionsExt;
        let (api, a, _) = setup();
        a.write(".templates/sales.md", "# Sales\n");
        synced(&a.sync(&api));
        let dir = a.path(".templates");
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o000)).unwrap();
        let report = a.sync(&api);
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(
            report.status,
            SyncStatus::Error {
                message: scan::UNREADABLE_LIBRARY.into()
            }
        );
        assert_eq!(api.live_objects(), 1);
    }

    fn put_remote(api: &FakeApi, device: &Device, key: &str, folder: Option<&str>, text: &str) {
        let vault_key = VaultKey::from_b64(&device.key).unwrap();
        let id = crypto::object_id(&vault_key, key);
        let payload = Payload {
            key: key.into(),
            folder: folder.map(str::to_string),
            content: text.as_bytes().to_vec(),
            modified_ms: 0,
            device: "Linux".into(),
        };
        let blob = crypto::seal(&vault_key, &id, 1, &payload);
        assert!(matches!(
            api.put(&id, 0, 1, &blob).unwrap(),
            PutResult::Ok(_)
        ));
    }

    #[test]
    fn names_windows_refuses_are_written_portably_and_keep_their_key() {
        let (api, a, b) = setup();
        let id = shared_meeting(&api, &a, &b);
        let uid = a.store().uid_of(&id).unwrap();
        let odd = format!("meeting/{uid}/a:b?.md");
        put_remote(&api, &a, &odd, Some(&id), "raar\n");
        put_remote(&api, &a, "templates/con.md", None, "# Con\n");
        let live = api.live_objects();

        let report = b.sync(&api);
        synced(&report);
        assert_eq!(report.downloaded, 2);
        assert_eq!(b.read(&format!("{id}/a_b_.md")).as_deref(), Some("raar\n"));
        assert_eq!(b.read(".templates/_con.md").as_deref(), Some("# Con\n"));
        assert!(b.state.entry(&odd).unwrap().is_some());

        let report = b.sync(&api);
        synced(&report);
        assert_eq!(report.uploaded, 0);
        assert_eq!(api.live_objects(), live);

        b.write(&format!("{id}/a_b_.md"), "aangepast\n");
        let report = b.sync(&api);
        synced(&report);
        assert_eq!(report.uploaded, 1);
        assert_eq!(api.live_objects(), live);
        synced(&a.sync(&api));
        assert_eq!(
            a.read(&format!("{id}/a_b_.md")).as_deref(),
            Some("aangepast\n")
        );
        assert_eq!(b.engine_states(), vec![SyncItemState::Synced]);

        std::fs::remove_file(b.path(&format!("{id}/a_b_.md"))).unwrap();
        synced(&b.sync(&api));
        assert_eq!(api.live_objects(), live - 1);
        assert!(b.state.entry(&odd).unwrap().is_none());
    }

    #[test]
    fn file_too_large_once_sealed_is_skipped_with_the_real_limit() {
        let (api, a, _) = setup();
        let id = a.store().create("Groot", start()).unwrap();
        // Fits under 5 MiB as a file, not once base64-encoded inside the sealed payload.
        let content: Vec<u8> = (0..4_000_000u32).map(|n| (n % 251) as u8).collect();
        std::fs::write(a.path(&format!("{id}/bijlage.bin")), content).unwrap();
        let report = a.sync(&api);
        assert_eq!(
            report.status,
            SyncStatus::Error {
                message: "This file is too large for Pennel Cloud (approximately 3.5 MB maximum)."
                    .into()
            }
        );
        assert_eq!(report.uploaded, 1);
        assert_eq!(api.live_objects(), 1);
    }

    #[test]
    fn missing_templates_folder_deletes_no_templates() {
        let (api, a, _) = setup();
        a.store().create("Blijft", start()).unwrap();
        a.write(".templates/sales.md", "# Sales\n");
        a.write(".templates/intake.md", "# Intake\n");
        synced(&a.sync(&api));
        assert_eq!(api.live_objects(), 3);
        std::fs::remove_dir_all(a.path(".templates")).unwrap();
        synced(&a.sync(&api));
        assert_eq!(api.live_objects(), 3);
    }

    const BRAKE_MESSAGE: &str = "Many meetings disappeared from your folder. Sync has deleted nothing; confirm under Settings → Account if this was intentional.";

    fn meetings(device: &Device, count: usize) -> Vec<String> {
        (0..count)
            .map(|n| {
                device
                    .store()
                    .create(&format!("Meeting {n}"), start())
                    .unwrap()
            })
            .collect()
    }

    #[test]
    fn emptied_root_pushes_no_deletes() {
        let (api, a, _) = setup();
        for id in meetings(&a, 2) {
            a.write(&format!("{id}/notes.md"), "n\n");
        }
        synced(&a.sync(&api));
        for folder in a.folders() {
            std::fs::remove_dir_all(a.path(&folder)).unwrap();
        }
        assert_eq!(
            a.sync(&api).status,
            SyncStatus::Error {
                message: BRAKE_MESSAGE.into()
            }
        );
        assert_eq!(api.live_objects(), 4);
        assert_eq!(
            a.sync(&api).status,
            SyncStatus::Error {
                message: BRAKE_MESSAGE.into()
            }
        );
        a.state.confirm_deletes().unwrap();
        synced(&a.sync(&api));
        assert_eq!(api.live_objects(), 0);
    }

    #[cfg(unix)]
    #[test]
    fn unreadable_root_reports_error_and_deletes_nothing() {
        use std::os::unix::fs::PermissionsExt;
        let (api, a, _) = setup();
        meetings(&a, 3);
        synced(&a.sync(&api));
        std::fs::set_permissions(&a.root, std::fs::Permissions::from_mode(0o000)).unwrap();
        let report = a.sync(&api);
        std::fs::set_permissions(&a.root, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(
            report.status,
            SyncStatus::Error {
                message: "The meeting folder could not be read.".into()
            }
        );
        assert_eq!(api.live_objects(), 3);
    }

    #[test]
    fn partial_mass_delete_is_held_back() {
        let (api, a, _) = setup();
        let ids = meetings(&a, 8);
        synced(&a.sync(&api));
        for id in &ids[..6] {
            std::fs::remove_dir_all(a.path(id)).unwrap();
        }
        assert_eq!(
            a.sync(&api).status,
            SyncStatus::Error {
                message: BRAKE_MESSAGE.into()
            }
        );
        assert_eq!(api.live_objects(), 8);
    }

    #[test]
    fn a_few_deleted_meetings_are_pushed() {
        let (api, a, _) = setup();
        let ids = meetings(&a, 3);
        synced(&a.sync(&api));
        std::fs::remove_dir_all(a.path(&ids[0])).unwrap();
        synced(&a.sync(&api));
        assert_eq!(api.live_objects(), 2);
    }

    #[test]
    fn intended_deletes_bypass_the_brake() {
        let (api, a, _) = setup();
        let ids = meetings(&a, 2);
        synced(&a.sync(&api));
        for id in &ids {
            let uid = a.store().uid_of(id).unwrap();
            a.state.mark_deleted(&uid).unwrap();
            std::fs::remove_dir_all(a.path(id)).unwrap();
        }
        synced(&a.sync(&api));
        assert_eq!(api.live_objects(), 0);
    }

    #[test]
    fn folder_renamed_during_push_is_not_deleted() {
        let (api, a, b) = setup();
        let id = shared_meeting(&api, &a, &b);
        std::fs::remove_file(a.path(&format!("{id}/transcript.md"))).unwrap();
        a.write(&format!("{id}/notes.md"), "nieuw\n");
        let from = a.path(&id);
        let to = a.path(&format!("{id} hernoemd"));
        let mut fired = false;
        api.on_put(move |_| {
            if !fired {
                fired = true;
                std::fs::rename(&from, &to).unwrap();
            }
        });
        synced(&a.sync(&api));
        assert_eq!(api.live_objects(), 3);
        synced(&a.sync(&api));
        assert_eq!(api.live_objects(), 2);
    }

    #[test]
    fn deferred_change_survives_a_failed_download() {
        let (api, a, b) = setup();
        let id = shared_meeting(&api, &a, &b);
        a.write(&format!("{id}/notes.md"), "tijdens opname\n");
        synced(&a.sync(&api));
        synced(&b.sync_recording(&api, std::slice::from_ref(&id)));
        api.fail_gets(true);
        assert_eq!(b.sync(&api).status, SyncStatus::Offline);
        api.fail_gets(false);
        synced(&b.sync(&api));
        assert_eq!(
            b.read(&format!("{id}/notes.md")).as_deref(),
            Some("tijdens opname\n")
        );
    }

    #[test]
    fn meeting_md_conflict_still_follows_the_rename() {
        let (api, a, b) = setup();
        let id = shared_meeting(&api, &a, &b);
        let renamed = a.store().rename(&id, "Nieuwe titel").unwrap();
        synced(&a.sync(&api));
        b.store().set_attendees(&id, &["Els".into()]).unwrap();
        let local_edit = b.read(&format!("{id}/meeting.md")).unwrap();
        let report = b.sync(&api);
        synced(&report);
        assert_eq!(b.folders(), vec![renamed.clone()]);
        let live = b.read(&format!("{renamed}/meeting.md")).unwrap();
        let copies = b.conflict_copies(&renamed);
        let kept = live == local_edit
            || copies
                .iter()
                .any(|c| b.read(&format!("{renamed}/{c}")).as_deref() == Some(&local_edit));
        assert!(kept, "local meeting.md edit lost");
        synced(&a.sync(&api));
        assert_eq!(a.folders(), vec![renamed.clone()]);
        synced(&b.sync(&api));
        assert_eq!(b.folders(), vec![renamed]);
    }

    #[test]
    fn transient_and_unexpected_errors_preserve_server_messages() {
        assert_eq!(
            status_for(PennelError::Cloud {
                code: "rate_limited".into(),
                message: "Too many attempts. Wait a moment and try again.".into()
            }),
            SyncStatus::Offline
        );
        assert_eq!(
            status_for(PennelError::InvalidResponse {
                message: "unexpected changes response".into()
            }),
            SyncStatus::Error {
                message: "Pennel Cloud returned an unexpected response.".into()
            }
        );
    }

    #[test]
    fn brake_compares_against_meetings_not_deleted_on_purpose() {
        let (api, a, _) = setup();
        let ids = meetings(&a, 2);
        synced(&a.sync(&api));
        let uid = a.store().uid_of(&ids[0]).unwrap();
        a.state.mark_deleted(&uid).unwrap();
        for id in &ids {
            std::fs::remove_dir_all(a.path(id)).unwrap();
        }
        assert_eq!(
            a.sync(&api).status,
            SyncStatus::Error {
                message: BRAKE_MESSAGE.into()
            }
        );
        assert_eq!(api.live_objects(), 1);
    }

    #[test]
    fn deferred_item_survives_a_crash_while_applying() {
        let (api, a, b) = setup();
        let id = shared_meeting(&api, &a, &b);
        a.write(&format!("{id}/notes.md"), "tijdens opname\n");
        synced(&a.sync(&api));
        synced(&b.sync_recording(&api, std::slice::from_ref(&id)));
        assert_eq!(b.state.deferred().unwrap().len(), 1);
        CRASH_IN_APPLY.with(|crash| crash.set(true));
        let crashed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| b.sync(&api)));
        CRASH_IN_APPLY.with(|crash| crash.set(false));
        assert!(crashed.is_err());
        assert_eq!(b.state.deferred().unwrap().len(), 1);
        synced(&b.sync(&api));
        assert!(b.state.deferred().unwrap().is_empty());
        assert_eq!(
            b.read(&format!("{id}/notes.md")).as_deref(),
            Some("tijdens opname\n")
        );
    }

    #[cfg(unix)]
    #[test]
    fn root_unreadable_during_push_deletes_nothing() {
        use std::os::unix::fs::PermissionsExt;
        let (api, a, _) = setup();
        let ids = meetings(&a, 3);
        synced(&a.sync(&api));
        std::fs::remove_dir_all(a.path(&ids[0])).unwrap();
        a.write(&format!("{}/notes.md", ids[1]), "bewerkt\n");
        let root = a.root.clone();
        let mut fired = false;
        api.on_put(move |_| {
            if !fired {
                fired = true;
                std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o000)).unwrap();
            }
        });
        let report = a.sync(&api);
        std::fs::set_permissions(&a.root, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(
            report.status,
            SyncStatus::Error {
                message: scan::UNREADABLE_LIBRARY.into()
            }
        );
        assert_eq!(api.live_objects(), 4);
    }

    #[test]
    fn storage_errors_use_canonical_english() {
        assert_eq!(
            status_for(PennelError::Storage {
                message: crate::meetings::UID_RACE.into()
            }),
            SyncStatus::Error {
                message: "A meeting file changed during sync; Pennel will try again shortly."
                    .into()
            }
        );
        assert_eq!(
            status_for(PennelError::Storage {
                message: "Permission denied (os error 13)".into()
            }),
            SyncStatus::Error {
                message: scan::UNREADABLE_LIBRARY.into()
            }
        );
        assert_eq!(
            status_for(PennelError::NotFound),
            SyncStatus::Error {
                message: UNEXPECTED_RESPONSE.into()
            }
        );
    }

    #[test]
    fn copied_meeting_folder_gets_its_own_uid() {
        let (api, a, b) = setup();
        let id = a.store().create("Origineel", start()).unwrap();
        synced(&a.sync(&api));
        let copy = format!("{id} kopie");
        a.write(
            &format!("{copy}/meeting.md"),
            &a.read(&format!("{id}/meeting.md")).unwrap(),
        );
        synced(&a.sync(&api));
        assert_ne!(a.store().uid_of(&id), a.store().uid_of(&copy));
        synced(&b.sync(&api));
        assert_eq!(b.store().uid_of(&id), a.store().uid_of(&id));
        assert_eq!(b.store().uid_of(&copy), a.store().uid_of(&copy));
        let mut expected = vec![id.clone(), copy.clone()];
        expected.sort();
        assert_eq!(b.folders(), expected);
    }
}
