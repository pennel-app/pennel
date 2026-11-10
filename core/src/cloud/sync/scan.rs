use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use unicode_normalization::UnicodeNormalization;

use crate::error::PennelError;
use crate::meetings::valid_uid;

pub const TEMPLATES_DIR: &str = ".templates";
pub const MEETING_FILE: &str = "meeting.md";

const AUDIO: &[&str] = &[
    "m4a", "m4b", "mp3", "wav", "aac", "caf", "aif", "aiff", "flac", "ogg", "oga", "opus", "wma",
    "webm", "amr",
];
const SYSTEM_FILES: &[&str] = &[".ds_store", "desktop.ini", "thumbs.db"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalFile {
    pub key: String,
    pub rel_path: PathBuf,
    pub folder: Option<String>,
    pub hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MeetingFolder {
    /// The name on disk, which may be NFD on macOS.
    pub name: String,
    pub uid: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Key {
    Meeting { uid: String, name: String },
    Template { name: String },
}

pub fn meeting_key(uid: &str, name: &str) -> String {
    format!("meeting/{uid}/{}", nfc(name))
}

pub fn template_key(name: &str) -> String {
    format!("templates/{}", nfc(name))
}

/// Keys come out of decrypted payloads; anything that could escape its folder is refused.
pub fn parse_key(key: &str) -> Option<Key> {
    let mut parts = key.split('/');
    let parsed = match (parts.next()?, parts.next()?, parts.next()) {
        ("templates", name, None) if is_template_name(name) => Key::Template {
            name: name.to_string(),
        },
        ("meeting", uid, Some(name)) if valid_uid(uid) && is_syncable(name) => Key::Meeting {
            uid: uid.to_string(),
            name: name.to_string(),
        },
        _ => return None,
    };
    parts.next().is_none().then_some(parsed)
}

pub fn nfc(text: &str) -> String {
    text.nfc().collect()
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

pub fn hash_file(path: &Path) -> Option<String> {
    if !path.is_file() {
        return None;
    }
    std::fs::read(path).ok().map(|bytes| sha256_hex(&bytes))
}

pub fn is_syncable(name: &str) -> bool {
    let lower = name.to_lowercase();
    let audio = Path::new(&lower)
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| AUDIO.contains(&e));
    !name.is_empty()
        && !name.starts_with('.')
        && !name.contains(['/', '\\', '\0'])
        && !lower.ends_with(".tmp")
        && !SYSTEM_FILES.contains(&lower.as_str())
        && !audio
}

pub fn is_template_name(name: &str) -> bool {
    is_syncable(name) && name.to_lowercase().ends_with(".md")
}

fn refused_char(c: char) -> bool {
    matches!(c, '/' | '\\' | ':' | '<' | '>' | '"' | '|' | '?' | '*') || c.is_control()
}

/// Windows refuses these as a name stem, whatever the extension.
fn reserved_stem(name: &str) -> bool {
    let stem = name
        .split('.')
        .next()
        .unwrap_or(name)
        .trim_end()
        .to_ascii_lowercase();
    matches!(stem.as_str(), "con" | "prn" | "aux" | "nul")
        || ((stem.starts_with("com") || stem.starts_with("lpt"))
            && stem.len() == 4
            && stem.as_bytes()[3].is_ascii_digit()
            && stem.as_bytes()[3] != b'0')
}

/// The on-disk name of a synced file. What Windows refuses becomes `_` on every platform, so
/// all devices write the same name; the sync key keeps the original.
pub fn portable_name(name: &str) -> String {
    let replaced: String = name
        .chars()
        .map(|c| if refused_char(c) { '_' } else { c })
        .collect();
    let kept = replaced.trim_end_matches(['.', ' ']);
    let portable = format!("{kept}{}", "_".repeat(replaced.len() - kept.len()));
    if reserved_stem(&portable) {
        format!("_{portable}")
    } else {
        portable
    }
}

/// Where the file for a synced `name` is: under that name when it exists, else under its
/// portable name.
pub fn file_slot(dir: &Path, name: &str) -> Slot {
    let portable = portable_name(name);
    if portable == name {
        return slot(dir, name);
    }
    match slot(dir, name) {
        Slot::Existing(path) => Slot::Existing(path),
        _ => slot(dir, &portable),
    }
}

/// A folder name from another device made safe for every platform; `None` when nothing usable is left.
pub fn clean_folder(name: &str) -> Option<String> {
    let kept: String = nfc(name).chars().filter(|c| !refused_char(*c)).collect();
    let trimmed = kept.trim().trim_end_matches(['.', ' ']).trim();
    if trimmed.is_empty() || trimmed.starts_with('.') {
        return None;
    }
    Some(if reserved_stem(trimmed) {
        format!("_{trimmed}")
    } else {
        trimmed.to_string()
    })
}

fn utf8_entries(dir: &Path) -> Vec<(String, PathBuf)> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut names: Vec<(String, PathBuf)> = entries
        .filter_map(Result::ok)
        .filter_map(|e| Some((e.file_name().to_str()?.to_string(), e.path())))
        .collect();
    names.sort();
    names
}

pub const UNREADABLE_LIBRARY: &str = "The meeting folder could not be read.";

fn unreadable() -> PennelError {
    PennelError::Cloud {
        code: "library_unreadable".into(),
        message: UNREADABLE_LIBRARY.into(),
    }
}

/// Like `utf8_entries`, but a listing that fails halfway is an error rather than fewer files:
/// a missing file would otherwise be pushed as a delete. A folder that is gone lists as empty.
fn strict_entries(dir: &Path) -> Result<Vec<(String, PathBuf)>, PennelError> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(_) => return Err(unreadable()),
    };
    let mut names = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|_| unreadable())?;
        if let Some(name) = entry.file_name().to_str() {
            names.push((name.to_string(), entry.path()));
        }
    }
    names.sort();
    Ok(names)
}

pub fn meeting_folders(root: &Path) -> Result<Vec<MeetingFolder>, PennelError> {
    std::fs::read_dir(root).map_err(|_| unreadable())?;
    let store = crate::meetings::MeetingStore::new(root);
    Ok(strict_entries(root)?
        .into_iter()
        .filter(|(name, path)| !name.starts_with('.') && path.join(MEETING_FILE).is_file())
        .map(|(name, _)| MeetingFolder {
            uid: store.uid_of(&name),
            name,
        })
        .collect())
}

/// The existing folder that `name` refers to: exact, then Unicode-equivalent, then differing in case only.
pub fn find_dir(root: &Path, name: &str) -> Result<Option<String>, PennelError> {
    std::fs::read_dir(root).map_err(|_| unreadable())?;
    let wanted = nfc(name);
    let dirs: Vec<String> = strict_entries(root)?
        .into_iter()
        .filter(|(_, path)| path.is_dir())
        .map(|(n, _)| n)
        .collect();
    Ok(dirs
        .iter()
        .find(|n| *n == name)
        .or_else(|| dirs.iter().find(|n| nfc(n) == wanted))
        .or_else(|| {
            dirs.iter()
                .find(|n| nfc(n).to_lowercase() == wanted.to_lowercase())
        })
        .cloned())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Slot {
    /// The file this key refers to, under its on-disk name.
    Existing(PathBuf),
    Free(PathBuf),
    /// Another file differs only in case; on a case-insensitive disk writing would replace it.
    Taken,
}

pub fn slot(dir: &Path, name: &str) -> Slot {
    let wanted = nfc(name);
    let mut equivalent = None;
    let mut taken = false;
    for (existing, path) in utf8_entries(dir) {
        let normalised = nfc(&existing);
        if existing == name || normalised == wanted {
            if path.is_dir() {
                taken = true;
            } else if existing == name {
                return Slot::Existing(path);
            } else {
                equivalent.get_or_insert(path);
            }
        } else if normalised.to_lowercase() == wanted.to_lowercase() {
            taken = true;
        }
    }
    match equivalent {
        Some(path) => Slot::Existing(path),
        None if taken => Slot::Taken,
        None => Slot::Free(dir.join(name)),
    }
}

fn files_in(dir: &Path, accept: fn(&str) -> bool) -> Result<Vec<(String, PathBuf)>, PennelError> {
    let mut seen = HashSet::new();
    Ok(strict_entries(dir)?
        .into_iter()
        .filter(|(name, path)| accept(name) && path.is_file())
        // Two names that normalise to the same key: only the first is synced.
        .filter(|(name, _)| seen.insert(nfc(name)))
        .collect())
}

/// `folders` maps uid to the on-disk folder name, recording meetings already left out.
pub fn scan(root: &Path, folders: &HashMap<String, String>) -> Result<Vec<LocalFile>, PennelError> {
    let mut files = Vec::new();
    let mut add =
        |key: String, rel_path: PathBuf, folder: Option<String>| -> Result<(), PennelError> {
            // A file that disappears mid-scan is simply not there this pass.
            if let Some(hash) = hash_file(&root.join(&rel_path)) {
                files.push(LocalFile {
                    key,
                    rel_path,
                    folder,
                    hash,
                });
            }
            Ok(())
        };
    let mut ordered: Vec<(&String, &String)> = folders.iter().collect();
    ordered.sort_by(|a, b| a.1.cmp(b.1));
    for (uid, folder) in ordered {
        for (name, _) in files_in(&root.join(folder), is_syncable)? {
            add(
                meeting_key(uid, &name),
                Path::new(folder).join(&name),
                Some(nfc(folder)),
            )?;
        }
    }
    for (name, _) in files_in(&root.join(TEMPLATES_DIR), is_template_name)? {
        add(
            template_key(&name),
            Path::new(TEMPLATES_DIR).join(&name),
            None,
        )?;
    }
    Ok(files)
}

/// False when `dir` exists but cannot be listed: its files must not be taken for deleted.
pub fn listable(dir: &Path) -> bool {
    match std::fs::symlink_metadata(dir) {
        Err(error) => error.kind() == std::io::ErrorKind::NotFound,
        Ok(_) => std::fs::read_dir(dir).is_ok(),
    }
}

/// The files sync carries in `dir`, under their on-disk names.
pub fn syncable_files(dir: &Path) -> Result<Vec<(String, PathBuf)>, PennelError> {
    files_in(dir, is_syncable)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exclusions_follow_the_sync_rules() {
        for name in [
            "notes.md",
            "transcript.md",
            "Ideeën.txt",
            "foto.png",
            "pennel.summary.structure.0123456789abcdef.json",
        ] {
            assert!(is_syncable(name), "{name}");
        }
        for name in [
            "ik.m4a",
            "anderen.M4A",
            "x.wav",
            "notes.tmp",
            ".DS_Store",
            "desktop.ini",
            "Thumbs.db",
            ".hidden.md",
            "",
        ] {
            assert!(!is_syncable(name), "{name}");
        }
        assert!(is_template_name("sales.md"));
        assert!(!is_template_name("sales.txt"));
    }

    #[test]
    fn parse_key_refuses_escapes() {
        assert_eq!(
            parse_key("meeting/0190-ab/notes.md"),
            Some(Key::Meeting {
                uid: "0190-ab".into(),
                name: "notes.md".into()
            })
        );
        assert_eq!(
            parse_key("templates/a.md"),
            Some(Key::Template {
                name: "a.md".into()
            })
        );
        for bad in [
            "meeting/../notes.md",
            "meeting/u/../x",
            "meeting/u/.x",
            "meeting/u/a/b.md",
            "templates/../x.md",
            "templates/x.txt",
            "other/x",
            "meeting/u/x.m4a",
        ] {
            assert_eq!(parse_key(bad), None, "{bad}");
        }
    }

    #[test]
    fn scan_normalises_names_and_skips_unsynced_files() {
        let dir = tempfile::tempdir().unwrap();
        let folder = "2026-10-25 1400 Cafe\u{301}";
        std::fs::create_dir_all(dir.path().join(folder)).unwrap();
        for name in [
            "meeting.md",
            "notes.md",
            "ik.m4a",
            "notes.tmp",
            "Re\u{301}sume\u{301}.md",
        ] {
            std::fs::write(dir.path().join(folder).join(name), name).unwrap();
        }
        std::fs::create_dir_all(dir.path().join(".templates")).unwrap();
        std::fs::write(dir.path().join(".templates/sales.md"), "s").unwrap();
        std::fs::write(dir.path().join(".templates/.x.md"), "s").unwrap();
        let folders = HashMap::from([("u1".to_string(), folder.to_string())]);
        let files = scan(dir.path(), &folders).unwrap();
        let keys: Vec<&str> = files.iter().map(|f| f.key.as_str()).collect();
        assert_eq!(
            keys,
            vec![
                "meeting/u1/R\u{e9}sum\u{e9}.md",
                "meeting/u1/meeting.md",
                "meeting/u1/notes.md",
                "templates/sales.md"
            ]
        );
        assert_eq!(
            files[1].folder.as_deref(),
            Some("2026-10-25 1400 Caf\u{e9}")
        );
        assert_eq!(files[1].hash, sha256_hex(b"meeting.md"));
        assert_eq!(files[1].rel_path, Path::new(folder).join("meeting.md"));
    }

    #[test]
    fn portable_names_avoid_what_windows_refuses() {
        assert_eq!(portable_name("notes.md"), "notes.md");
        assert_eq!(portable_name("a:b?.md"), "a_b_.md");
        assert_eq!(portable_name("x<y>|\"*\u{1}.md"), "x_y_____.md");
        assert_eq!(portable_name("CON.md"), "_CON.md");
        assert_eq!(portable_name("lpt1.txt"), "_lpt1.txt");
        assert_eq!(portable_name("console.md"), "console.md");
        assert_eq!(portable_name("notes.md. "), "notes.md__");
        assert_eq!(clean_folder("CON"), Some("_CON".into()));
        assert_eq!(clean_folder("Aux.x"), Some("_Aux.x".into()));
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a_b.md"), "a").unwrap();
        assert_eq!(
            file_slot(dir.path(), "a:b.md"),
            Slot::Existing(dir.path().join("a_b.md"))
        );
        assert_eq!(
            file_slot(dir.path(), "c:d.md"),
            Slot::Free(dir.path().join("c_d.md"))
        );
    }

    #[test]
    fn slot_detects_equivalent_and_case_collisions() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Notes.md"), "a").unwrap();
        assert_eq!(
            slot(dir.path(), "Notes.md"),
            Slot::Existing(dir.path().join("Notes.md"))
        );
        assert_eq!(slot(dir.path(), "notes.md"), Slot::Taken);
        assert_eq!(
            slot(dir.path(), "other.md"),
            Slot::Free(dir.path().join("other.md"))
        );
        assert_eq!(clean_folder("a:b/c?"), Some("abc".into()));
        assert_eq!(clean_folder(" .. "), None);
        assert_eq!(clean_folder(".hidden"), None);
    }
}
