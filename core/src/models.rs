use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use sha2::{Digest, Sha256};

use crate::catalog::{self, CatalogModel};
use crate::error::PennelError;
use crate::http;

const PROGRESS_STEP: u64 = 1024 * 1024;
const CHUNK_BYTES: u64 = 16 * 1024 * 1024;
/// Per-chunk budget; 16 MB in 120 s still allows ~140 KB/s connections.
const CHUNK_TIMEOUT_SECS: u64 = 120;

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record, serde::Serialize)]
pub struct LocalModel {
    pub id: String,
    pub name: String,
    pub description: String,
    pub size_bytes: u64,
    pub parameters: Option<String>,
    pub speed: u8,
    pub accuracy: Option<u8>,
    pub languages: Vec<String>,
    pub recommended: bool,
    pub legacy: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum, serde::Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ModelState {
    NotDownloaded,
    Partial { bytes: u64 },
    Downloaded,
}

#[uniffi::export(with_foreign)]
pub trait DownloadListener: Send + Sync {
    fn on_progress(&self, downloaded: u64, total: u64);
}

#[derive(Debug, Clone)]
pub struct ModelEntry {
    pub id: String,
    pub name: String,
    pub description: String,
    pub size_bytes: u64,
    pub sha256: String,
    pub url: String,
    /// Benchmarks and languages; for legacy entries those of the catalog model they were quantized from.
    pub scores: Option<&'static CatalogModel>,
    pub legacy: bool,
    pub kind: ModelKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelKind {
    Speech,
    Diarizer,
    /// One of the two files of the speaker pipeline; the app offers them as one download.
    Speakers,
}

pub const DIARIZER_ID: &str = "diar_streaming_sortformer_4spk-v2.1";

/// Splits "Anderen" into voices; pinned from the catalog's `diarize` entry.
fn diarizer_entry() -> ModelEntry {
    ModelEntry {
        id: DIARIZER_ID.into(),
        name: "Sortformer (speakers)".into(),
        description: "Recognizes up to 4 speakers in a meeting".into(),
        size_bytes: 139_310_336,
        sha256: "a5dacdc650790266c7a362e54e6bf51952015487edaa606c4e11632bc32442a9".into(),
        url: "https://huggingface.co/handy-computer/diar_streaming_sortformer_4spk-v2.1-gguf/resolve/ae4afbb5c3d33b71cf2dbf600022b655ee706dd0/diar_streaming_sortformer_4spk-v2.1-Q8_0.gguf".into(),
        scores: None,
        legacy: false,
        kind: ModelKind::Diarizer,
    }
}

pub const SEGMENTATION_ID: &str = "pyannote-segmentation-3.0";
pub const EMBEDDING_ID: &str = "3dspeaker-campplus-zh-en-advanced";

/// pyannote segmentation-3.0 (MIT) and 3D-Speaker CAM++ (Apache-2.0), ONNX exports pinned by commit.
fn speaker_entries() -> Vec<ModelEntry> {
    vec![
        ModelEntry {
            id: SEGMENTATION_ID.into(),
            name: "pyannote segmentation 3.0".into(),
            description: "Detects who speaks when".into(),
            size_bytes: 5_992_913,
            sha256: "220ad67ca923bef2fa91f2390c786097bf305bceb5e261d4af67b38e938e1079".into(),
            url: "https://huggingface.co/csukuangfj/sherpa-onnx-pyannote-segmentation-3-0/resolve/9403a6902bb58e3d5ae8c7e77c3422de279db2e0/model.onnx".into(),
            scores: None,
            legacy: false,
            kind: ModelKind::Speakers,
        },
        ModelEntry {
            id: EMBEDDING_ID.into(),
            name: "CAM++ voiceprint".into(),
            description: "Recognizes voices across meetings".into(),
            size_bytes: 28_281_164,
            sha256: "aa3cfc16963a10586a9393f5035d6d6b57e98d358b347f80c2a30bf4f00ceba2".into(),
            url: "https://huggingface.co/csukuangfj/speaker-embedding-models/resolve/0743f301363dec56491a490f6d6cbc9d67f9a3bf/3dspeaker_speech_campplus_sv_zh_en_16k-common_advanced.onnx".into(),
            scores: None,
            legacy: false,
            kind: ModelKind::Speakers,
        },
    ]
}

impl ModelEntry {
    fn extension(&self) -> &str {
        if self.url.ends_with(".gguf") {
            "gguf"
        } else if self.url.ends_with(".onnx") {
            "onnx"
        } else {
            "bin"
        }
    }
}

/// whisper.cpp files offered alongside the catalog, including previously downloaded models.
fn legacy_entries() -> Vec<ModelEntry> {
    let hf = "https://huggingface.co/ggerganov/whisper.cpp/resolve/main";
    vec![
        ModelEntry {
            id: "whisper-large-v3-turbo-q5_0".into(),
            name: "Whisper large-v3 turbo (q5)".into(),
            description: catalog::find("whisper-large-v3-turbo")
                .map(|model| model.description.clone())
                .unwrap_or_default(),
            size_bytes: 574_041_195,
            sha256: "394221709cd5ad1f40c46e6031ca61bce88931e6e088c188294c6d5a55ffa7e2".into(),
            url: format!("{hf}/ggml-large-v3-turbo-q5_0.bin"),
            scores: catalog::find("whisper-large-v3-turbo"),
            legacy: false,
            kind: ModelKind::Speech,
        },
        ModelEntry {
            id: "whisper-small-q5_1".into(),
            name: "Whisper small (q5)".into(),
            description: "Previously downloaded version".into(),
            size_bytes: 190_085_487,
            sha256: "ae85e4a935d7a567bd102fe55afc16bb595bdb618e11b2fc7591bc08120411bb".into(),
            url: format!("{hf}/ggml-small-q5_1.bin"),
            scores: catalog::find("whisper-small"),
            legacy: true,
            kind: ModelKind::Speech,
        },
    ]
}

fn entries() -> Vec<ModelEntry> {
    let mut entries: Vec<ModelEntry> = catalog::catalog()
        .iter()
        .map(|m| ModelEntry {
            id: m.id.clone(),
            name: m.name.clone(),
            description: m.description.clone(),
            size_bytes: m.file.size,
            sha256: m.file.sha256.clone(),
            url: m.url.clone(),
            scores: Some(m),
            legacy: false,
            kind: ModelKind::Speech,
        })
        .collect();
    entries.extend(legacy_entries());
    entries.push(diarizer_entry());
    entries.extend(speaker_entries());
    entries
}

fn recommended_rank(id: &str) -> Option<usize> {
    catalog::RECOMMENDED.iter().position(|r| *r == id)
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub struct ModelStore {
    dir: PathBuf,
    catalog: Vec<ModelEntry>,
    chunk_bytes: u64,
}

impl ModelStore {
    pub fn new(data_dir: &Path) -> Self {
        Self::with_catalog(data_dir, entries())
    }

    pub fn with_catalog(data_dir: &Path, catalog: Vec<ModelEntry>) -> Self {
        Self {
            dir: data_dir.join("models"),
            catalog,
            chunk_bytes: CHUNK_BYTES,
        }
    }

    fn entry(&self, id: &str) -> Option<&ModelEntry> {
        self.catalog.iter().find(|e| e.id == id)
    }

    fn final_path(&self, id: &str) -> PathBuf {
        let extension = self.entry(id).map_or("bin", ModelEntry::extension);
        self.dir.join(format!("{id}.{extension}"))
    }

    fn part_path(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{id}.part"))
    }

    pub fn state(&self, id: &str) -> ModelState {
        if self.final_path(id).exists() {
            return ModelState::Downloaded;
        }
        match std::fs::metadata(self.part_path(id)) {
            Ok(meta) => ModelState::Partial { bytes: meta.len() },
            Err(_) => ModelState::NotDownloaded,
        }
    }

    pub fn path(&self, id: &str) -> Option<PathBuf> {
        let path = self.final_path(id);
        path.exists().then_some(path)
    }

    /// Recommended models first in their pinned order, then by accuracy in `language` (unmeasured last)
    /// and speed. Legacy files only appear once downloaded.
    pub fn local_models(&self, language: Option<&str>, supported_only: bool) -> Vec<LocalModel> {
        let language = language.unwrap_or(catalog::DEFAULT_SCORE_LANGUAGE);
        let mut models: Vec<LocalModel> = self
            .catalog
            .iter()
            .filter(|e| e.kind == ModelKind::Speech)
            .filter(|e| !e.legacy || self.final_path(&e.id).exists())
            .filter(|e| !supported_only || e.scores.is_none_or(|s| s.supports(language)))
            .map(|e| LocalModel {
                id: e.id.clone(),
                name: e.name.clone(),
                description: e.description.clone(),
                size_bytes: e.size_bytes,
                parameters: e.scores.and_then(|s| s.parameters.clone()),
                speed: e.scores.map_or(0, |s| s.speed),
                accuracy: e.scores.and_then(|s| s.accuracy(language)),
                languages: e.scores.map(|s| s.languages.clone()).unwrap_or_default(),
                recommended: recommended_rank(&e.id).is_some(),
                legacy: e.legacy,
            })
            .collect();
        models.sort_by(|a, b| {
            let rank = |m: &LocalModel| recommended_rank(&m.id).unwrap_or(usize::MAX);
            rank(a)
                .cmp(&rank(b))
                .then(b.accuracy.is_some().cmp(&a.accuracy.is_some()))
                .then(b.accuracy.cmp(&a.accuracy))
                .then(b.speed.cmp(&a.speed))
        });
        models
    }

    /// Recommended models first, so recovery after removing a provider stays multilingual.
    pub fn downloaded(&self) -> Vec<String> {
        let mut ids: Vec<&ModelEntry> = self
            .catalog
            .iter()
            .filter(|e| e.kind == ModelKind::Speech && self.final_path(&e.id).exists())
            .collect();
        ids.sort_by_key(|e| {
            recommended_rank(&e.id)
                .or_else(|| e.scores.and_then(|s| recommended_rank(&s.id)))
                .unwrap_or(usize::MAX)
        });
        ids.into_iter().map(|e| e.id.clone()).collect()
    }

    /// Downloads in bounded Range requests so a stalled connection times out instead of hanging,
    /// and cancel takes effect within one chunk.
    pub fn download(
        &self,
        id: &str,
        listener: &dyn DownloadListener,
        cancel: &AtomicBool,
    ) -> Result<(), PennelError> {
        let entry = self.entry(id).ok_or(PennelError::NotFound)?.clone();
        if self.path(id).is_some() {
            return Ok(());
        }
        std::fs::create_dir_all(&self.dir).map_err(PennelError::storage)?;
        let part = self.part_path(id);
        // Not append mode: Windows refuses set_len on a handle opened for appending only.
        let mut file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(false)
            .open(&part)
            .map_err(PennelError::storage)?;
        let mut have = file.seek(SeekFrom::End(0)).map_err(PennelError::storage)?;
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_connect(Some(Duration::from_secs(15)))
            .timeout_recv_response(Some(Duration::from_secs(30)))
            .timeout_recv_body(Some(Duration::from_secs(CHUNK_TIMEOUT_SECS)))
            .http_status_as_error(false)
            .build()
            .into();
        let mut restarted = false;
        let mut progress = Progress {
            listener,
            total: entry.size_bytes,
            next_report: have + PROGRESS_STEP,
        };

        while have < entry.size_bytes {
            if cancel.load(Ordering::SeqCst) {
                return Err(PennelError::Cancelled);
            }
            let range = range_header(have, entry.size_bytes, self.chunk_bytes);
            let mut response = agent
                .get(&entry.url)
                .header("Range", range)
                .call()
                .map_err(|e| PennelError::Network {
                    message: e.to_string(),
                })?;
            match response.status().as_u16() {
                206 => {}
                200 => {
                    have = truncate(&mut file)?;
                }
                416 if !restarted => {
                    restarted = true;
                    have = truncate(&mut file)?;
                    continue;
                }
                status => {
                    let body = response.body_mut().read_to_string().unwrap_or_default();
                    return Err(http::map_status(status, &body));
                }
            }
            let reader = response
                .into_body()
                .into_with_config()
                .limit(u64::MAX)
                .reader();
            have = append_body(reader, &mut file, have, &mut progress, cancel)?;
        }
        file.flush().map_err(PennelError::storage)?;
        drop(file);

        if sha256_file(&part)? != entry.sha256 {
            let _ = std::fs::remove_file(&part);
            return Err(PennelError::InvalidResponse {
                message: "checksum mismatch".into(),
            });
        }
        std::fs::rename(&part, self.final_path(id)).map_err(PennelError::storage)?;
        listener.on_progress(entry.size_bytes, entry.size_bytes);
        Ok(())
    }

    pub fn delete(&self, id: &str) -> Result<(), PennelError> {
        for path in [self.final_path(id), self.part_path(id)] {
            match std::fs::remove_file(path) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                    return Err(PennelError::storage(e));
                }
                _ => {}
            }
        }
        Ok(())
    }
}

fn truncate(file: &mut File) -> Result<u64, PennelError> {
    file.set_len(0).map_err(PennelError::storage)?;
    file.seek(SeekFrom::Start(0)).map_err(PennelError::storage)
}

pub(crate) fn range_header(have: u64, size: u64, chunk: u64) -> String {
    format!("bytes={have}-{}", (have + chunk).min(size) - 1)
}

struct Progress<'a> {
    listener: &'a dyn DownloadListener,
    total: u64,
    next_report: u64,
}

fn append_body(
    mut reader: impl Read,
    file: &mut File,
    mut have: u64,
    progress: &mut Progress,
    cancel: &AtomicBool,
) -> Result<u64, PennelError> {
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        if cancel.load(Ordering::SeqCst) {
            return Err(PennelError::Cancelled);
        }
        let n = reader.read(&mut buf).map_err(|e| PennelError::Network {
            message: e.to_string(),
        })?;
        if n == 0 {
            return Ok(have);
        }
        file.write_all(&buf[..n]).map_err(PennelError::storage)?;
        have += n as u64;
        if have >= progress.next_report {
            progress.listener.on_progress(have, progress.total);
            progress.next_report = have + PROGRESS_STEP;
        }
    }
}

fn sha256_file(path: &Path) -> Result<String, PennelError> {
    let mut file = File::open(path).map_err(PennelError::storage)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1024 * 1024];
    loop {
        let n = file.read(&mut buf).map_err(PennelError::storage)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex(&hasher.finalize()))
}

pub type CancelFlag = Arc<AtomicBool>;

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};
    use std::sync::Mutex;

    const MB: usize = 1024 * 1024;

    #[test]
    fn bundled_model_metadata_uses_english_with_stable_model_ids() {
        let diarizer = diarizer_entry();
        assert_eq!(diarizer.id, DIARIZER_ID);
        assert_eq!(diarizer.name, "Sortformer (speakers)");
        assert_eq!(
            diarizer.description,
            "Recognizes up to 4 speakers in a meeting"
        );
        let speakers = speaker_entries();
        assert_eq!(speakers[0].id, SEGMENTATION_ID);
        assert_eq!(speakers[0].name, "pyannote segmentation 3.0");
        assert_eq!(speakers[1].id, EMBEDDING_ID);
        assert_eq!(speakers[1].name, "CAM++ voiceprint");
        assert!(
            legacy_entries()
                .iter()
                .filter(|entry| entry.legacy)
                .all(|entry| entry.description == "Previously downloaded version")
        );
    }

    fn payload() -> Vec<u8> {
        (0..3 * MB).map(|i| (i % 251) as u8).collect()
    }

    fn store(dir: &tempfile::TempDir, url: String, bytes: &[u8]) -> ModelStore {
        ModelStore::with_catalog(
            dir.path(),
            vec![ModelEntry {
                id: "tiny".into(),
                name: "Tiny".into(),
                description: "test".into(),
                size_bytes: bytes.len() as u64,
                sha256: hex(&Sha256::digest(bytes)),
                url,
                scores: None,
                legacy: false,
                kind: ModelKind::Speech,
            }],
        )
    }

    #[derive(Default)]
    struct Recorder {
        calls: Mutex<Vec<(u64, u64)>>,
        cancel_on_first: Option<Arc<AtomicBool>>,
    }

    impl DownloadListener for Recorder {
        fn on_progress(&self, downloaded: u64, total: u64) {
            self.calls.lock().unwrap().push((downloaded, total));
            if let Some(flag) = &self.cancel_on_first {
                flag.store(true, Ordering::SeqCst);
            }
        }
    }

    fn ids(models: &[LocalModel]) -> Vec<&str> {
        models.iter().map(|m| m.id.as_str()).collect()
    }

    #[test]
    fn recommended_first_in_pinned_order() {
        let dir = tempfile::tempdir().unwrap();
        let models = ModelStore::new(dir.path()).local_models(Some("nl"), true);
        assert_eq!(ids(&models)[..5], catalog::RECOMMENDED);
        assert!(models[..5].iter().all(|m| m.recommended));
        assert!(!models[5].recommended);
    }

    #[test]
    fn auto_language_scores_dutch() {
        let dir = tempfile::tempdir().unwrap();
        let store = ModelStore::new(dir.path());
        assert_eq!(
            store.local_models(None, true),
            store.local_models(Some("nl"), true)
        );
        let parakeet = store
            .local_models(None, true)
            .into_iter()
            .find(|m| m.id == "parakeet-tdt-0.6b-v3")
            .unwrap();
        assert_eq!((parakeet.speed, parakeet.accuracy), (96, Some(60)));
        assert_eq!(parakeet.parameters.as_deref(), Some("627M"));
        assert!(parakeet.languages.contains(&"nl".to_string()));
    }

    #[test]
    fn supported_only_filters_language() {
        let dir = tempfile::tempdir().unwrap();
        let store = ModelStore::new(dir.path());
        assert!(!ids(&store.local_models(Some("nl"), true)).contains(&"parakeet-tdt-0.6b-v2"));
        assert!(ids(&store.local_models(Some("nl"), false)).contains(&"parakeet-tdt-0.6b-v2"));
    }

    #[test]
    fn unsupported_language_sorts_last() {
        let dir = tempfile::tempdir().unwrap();
        let models = ModelStore::new(dir.path()).local_models(Some("nl"), false);
        let rest = &models[catalog::RECOMMENDED.len()..];
        let first_unmeasured = rest.iter().position(|m| m.accuracy.is_none()).unwrap();
        assert!(
            rest[first_unmeasured..]
                .iter()
                .all(|m| m.accuracy.is_none())
        );
        assert!(
            rest[..first_unmeasured]
                .windows(2)
                .all(|w| w[0].accuracy >= w[1].accuracy)
        );
    }

    #[test]
    fn whisper_turbo_q5_is_available_before_download_and_keeps_existing_identity() {
        let dir = tempfile::tempdir().unwrap();
        let store = ModelStore::new(dir.path());
        let models = store.local_models(Some("nl"), true);
        let model = models
            .iter()
            .find(|model| model.id == "whisper-large-v3-turbo-q5_0")
            .expect("Q5 should be available to download");
        assert!(!model.legacy);
        assert_eq!(store.state(&model.id), ModelState::NotDownloaded);
        let entry = store
            .catalog
            .iter()
            .find(|entry| entry.id == model.id)
            .unwrap();
        assert!(entry.url.ends_with("ggml-large-v3-turbo-q5_0.bin"));
        assert_eq!(
            entry.sha256,
            "394221709cd5ad1f40c46e6031ca61bce88931e6e088c188294c6d5a55ffa7e2"
        );
        std::fs::create_dir_all(dir.path().join("models")).unwrap();
        std::fs::write(
            dir.path().join("models/whisper-large-v3-turbo-q5_0.bin"),
            b"x",
        )
        .unwrap();
        assert_eq!(store.state(&model.id), ModelState::Downloaded);
    }

    #[test]
    fn legacy_ids_stay_downloaded() {
        let dir = tempfile::tempdir().unwrap();
        let store = ModelStore::new(dir.path());
        assert!(!ids(&store.local_models(None, true)).contains(&"whisper-small-q5_1"));
        std::fs::create_dir_all(dir.path().join("models")).unwrap();
        std::fs::write(dir.path().join("models/whisper-small-q5_1.bin"), b"x").unwrap();
        assert_eq!(store.downloaded(), vec!["whisper-small-q5_1".to_string()]);
        assert_eq!(store.state("whisper-small-q5_1"), ModelState::Downloaded);
        let models = store.local_models(None, true);
        let legacy = models
            .iter()
            .find(|m| m.id == "whisper-small-q5_1")
            .unwrap();
        let small = models.iter().find(|m| m.id == "whisper-small").unwrap();
        assert!(legacy.legacy);
        assert_eq!(legacy.accuracy, small.accuracy);
    }

    #[test]
    fn downloaded_lists_recommended_first() {
        let dir = tempfile::tempdir().unwrap();
        let store = ModelStore::new(dir.path());
        std::fs::create_dir_all(dir.path().join("models")).unwrap();
        for file in [
            "parakeet-unified-en-0.6b.gguf",
            "whisper-small-q5_1.bin",
            "parakeet-tdt-0.6b-v3.gguf",
        ] {
            std::fs::write(dir.path().join("models").join(file), b"x").unwrap();
        }
        assert_eq!(
            store.downloaded(),
            [
                "parakeet-tdt-0.6b-v3",
                "whisper-small-q5_1",
                "parakeet-unified-en-0.6b"
            ]
        );
    }

    #[test]
    fn diarizer_not_listed_as_speech_model() {
        let dir = tempfile::tempdir().unwrap();
        let store = ModelStore::new(dir.path());
        std::fs::create_dir_all(dir.path().join("models")).unwrap();
        std::fs::write(dir.path().join(format!("models/{DIARIZER_ID}.gguf")), b"x").unwrap();
        assert_eq!(store.state(DIARIZER_ID), ModelState::Downloaded);
        assert!(store.downloaded().is_empty());
        assert!(!ids(&store.local_models(None, false)).contains(&DIARIZER_ID));
    }

    #[test]
    fn speaker_models_are_onnx_files_outside_the_speech_list() {
        let dir = tempfile::tempdir().unwrap();
        let store = ModelStore::new(dir.path());
        std::fs::create_dir_all(dir.path().join("models")).unwrap();
        for id in [SEGMENTATION_ID, EMBEDDING_ID] {
            assert_eq!(store.state(id), ModelState::NotDownloaded);
            std::fs::write(dir.path().join(format!("models/{id}.onnx")), b"x").unwrap();
            assert_eq!(store.state(id), ModelState::Downloaded);
            assert!(store.path(id).unwrap().ends_with(format!("{id}.onnx")));
        }
        assert!(store.downloaded().is_empty());
        let listed = store.local_models(None, false);
        assert!(!ids(&listed).contains(&SEGMENTATION_ID));
        assert!(!ids(&listed).contains(&EMBEDDING_ID));
    }

    #[test]
    fn diarizer_entry_matches_catalog() {
        let catalog: serde_json::Value =
            serde_json::from_str(include_str!("catalog.json")).unwrap();
        let model = catalog["models"]
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["id"] == DIARIZER_ID)
            .unwrap();
        let file = &model["files"][model["quant"].as_str().unwrap()];
        let entry = diarizer_entry();
        assert_eq!(file["sha256"], entry.sha256.as_str());
        assert_eq!(file["size"], entry.size_bytes);
        assert!(entry.url.contains(model["revision"].as_str().unwrap()));
    }

    #[test]
    fn gguf_final_path() {
        let dir = tempfile::tempdir().unwrap();
        let store = ModelStore::new(dir.path());
        std::fs::create_dir_all(dir.path().join("models")).unwrap();
        std::fs::write(dir.path().join("models/parakeet-tdt-0.6b-v3.gguf"), b"x").unwrap();
        assert_eq!(
            store.path("parakeet-tdt-0.6b-v3"),
            Some(dir.path().join("models/parakeet-tdt-0.6b-v3.gguf"))
        );
    }

    #[test]
    fn download_verifies_and_marks_downloaded() {
        let (dir, mut server) = (tempfile::tempdir().unwrap(), mockito::Server::new());
        let bytes = payload();
        server.mock("GET", "/m.bin").with_body(&bytes).create();
        let store = store(&dir, format!("{}/m.bin", server.url()), &bytes);
        assert_eq!(store.state("tiny"), ModelState::NotDownloaded);
        let listener = Recorder::default();
        store
            .download("tiny", &listener, &AtomicBool::new(false))
            .unwrap();
        assert_eq!(store.state("tiny"), ModelState::Downloaded);
        assert_eq!(std::fs::read(store.path("tiny").unwrap()).unwrap(), bytes);
        let total = bytes.len() as u64;
        assert_eq!(listener.calls.lock().unwrap().last(), Some(&(total, total)));
    }

    #[test]
    fn resumes_from_partial() {
        let (dir, mut server) = (tempfile::tempdir().unwrap(), mockito::Server::new());
        let bytes = payload();
        let store = store(&dir, format!("{}/m.bin", server.url()), &bytes);
        std::fs::create_dir_all(dir.path().join("models")).unwrap();
        std::fs::write(dir.path().join("models/tiny.part"), &bytes[..MB]).unwrap();
        assert_eq!(
            store.state("tiny"),
            ModelState::Partial { bytes: MB as u64 }
        );
        let mock = server
            .mock("GET", "/m.bin")
            .match_header("range", mockito::Matcher::Regex("^bytes=1048576-".into()))
            .with_status(206)
            .with_body(&bytes[MB..])
            .create();
        store
            .download("tiny", &Recorder::default(), &AtomicBool::new(false))
            .unwrap();
        mock.assert();
        assert_eq!(std::fs::read(store.path("tiny").unwrap()).unwrap(), bytes);
    }

    #[test]
    fn complete_part_is_verified_without_request() {
        let (dir, server) = (tempfile::tempdir().unwrap(), mockito::Server::new());
        let bytes = payload();
        let store = store(&dir, format!("{}/m.bin", server.url()), &bytes);
        std::fs::create_dir_all(dir.path().join("models")).unwrap();
        std::fs::write(dir.path().join("models/tiny.part"), &bytes).unwrap();
        store
            .download("tiny", &Recorder::default(), &AtomicBool::new(false))
            .unwrap();
        assert_eq!(store.state("tiny"), ModelState::Downloaded);
    }

    #[test]
    fn range_not_satisfiable_restarts_from_zero() {
        let (dir, mut server) = (tempfile::tempdir().unwrap(), mockito::Server::new());
        let bytes = payload();
        let store = store(&dir, format!("{}/m.bin", server.url()), &bytes);
        std::fs::create_dir_all(dir.path().join("models")).unwrap();
        std::fs::write(dir.path().join("models/tiny.part"), &bytes[..MB]).unwrap();
        server
            .mock("GET", "/m.bin")
            .match_header("range", mockito::Matcher::Regex("^bytes=1048576-".into()))
            .with_status(416)
            .create();
        server
            .mock("GET", "/m.bin")
            .match_header("range", mockito::Matcher::Regex("^bytes=0-".into()))
            .with_status(206)
            .with_body(&bytes)
            .create();
        store
            .download("tiny", &Recorder::default(), &AtomicBool::new(false))
            .unwrap();
        assert_eq!(std::fs::read(store.path("tiny").unwrap()).unwrap(), bytes);
    }

    #[test]
    fn ranges_are_bounded_chunks() {
        assert_eq!(range_header(0, 10, 4), "bytes=0-3");
        assert_eq!(range_header(8, 10, 4), "bytes=8-9");
    }

    #[test]
    fn checksum_mismatch_removes_part() {
        let (dir, mut server) = (tempfile::tempdir().unwrap(), mockito::Server::new());
        let bytes = payload();
        server
            .mock("GET", "/m.bin")
            .with_body(vec![0u8; bytes.len()])
            .create();
        let store = store(&dir, format!("{}/m.bin", server.url()), &bytes);
        let err = store
            .download("tiny", &Recorder::default(), &AtomicBool::new(false))
            .unwrap_err();
        assert!(
            matches!(err, PennelError::InvalidResponse { .. }),
            "{err:?}"
        );
        assert_eq!(store.state("tiny"), ModelState::NotDownloaded);
    }

    #[test]
    fn cancel_keeps_partial() {
        let (dir, mut server) = (tempfile::tempdir().unwrap(), mockito::Server::new());
        let bytes = payload();
        server.mock("GET", "/m.bin").with_body(&bytes).create();
        let store = store(&dir, format!("{}/m.bin", server.url()), &bytes);
        let cancel = Arc::new(AtomicBool::new(false));
        let listener = Recorder {
            cancel_on_first: Some(cancel.clone()),
            ..Default::default()
        };
        assert_eq!(
            store.download("tiny", &listener, &cancel).unwrap_err(),
            PennelError::Cancelled
        );
        assert!(matches!(store.state("tiny"), ModelState::Partial { .. }));
    }

    #[test]
    fn delete_removes_model() {
        let (dir, mut server) = (tempfile::tempdir().unwrap(), mockito::Server::new());
        let bytes = payload();
        server.mock("GET", "/m.bin").with_body(&bytes).create();
        let store = store(&dir, format!("{}/m.bin", server.url()), &bytes);
        store
            .download("tiny", &Recorder::default(), &AtomicBool::new(false))
            .unwrap();
        store.delete("tiny").unwrap();
        assert_eq!(store.state("tiny"), ModelState::NotDownloaded);
        assert_eq!(store.path("tiny"), None);
    }

    /// Downloads the real small model from Hugging Face in chunked Range requests.
    #[test]
    #[ignore]
    fn real_download_small_model() {
        let dir = tempfile::tempdir().unwrap();
        let store = ModelStore::new(dir.path());
        let started = std::time::Instant::now();
        store
            .download(
                "whisper-small",
                &Recorder::default(),
                &AtomicBool::new(false),
            )
            .unwrap();
        eprintln!("downloaded in {:?}", started.elapsed());
        assert_eq!(store.state("whisper-small"), ModelState::Downloaded);
    }

    #[test]
    fn unknown_model_is_not_found() {
        let dir = tempfile::tempdir().unwrap();
        let store = ModelStore::new(dir.path());
        let err = store
            .download("nope", &Recorder::default(), &AtomicBool::new(false))
            .unwrap_err();
        assert_eq!(err, PennelError::NotFound);
    }
}
