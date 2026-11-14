use std::{
    future::Future,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use super::{Cancellation, PartialTransfer, PeerCompareError, STEP_TIMEOUT, TransferManifest};

const MAX_MANIFEST_BYTES: usize = 2 * 1024 * 1024;
const MAX_BLOCK_BYTES: usize = 1024 * 1024;
const END: u32 = u32::MAX;
const MAGIC: &[u8] = b"PENNEL-PEER-COMPARISON/1\0";

pub(crate) trait ByteChannel: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> ByteChannel for T {}
pub(crate) type Channel = Box<dyn ByteChannel>;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
pub struct PeerCompareReport {
    pub complete: bool,
    pub bytes_sent: u64,
    pub bytes_received: u64,
    pub blocks_transferred: u32,
    pub total_bytes: u64,
    pub sha256: String,
}

impl PeerCompareReport {
    fn pending(manifest: &TransferManifest) -> Self {
        Self {
            complete: false,
            bytes_sent: 0,
            bytes_received: 0,
            blocks_transferred: 0,
            total_bytes: manifest.size(),
            sha256: manifest.sha256().to_owned(),
        }
    }
}

async fn step<T>(
    cancel: &Cancellation,
    future: impl Future<Output = std::io::Result<T>>,
) -> Result<T, PeerCompareError> {
    cancel.check()?;
    tokio::select! {
        biased;
        _ = cancel.wait() => Err(PeerCompareError::Cancelled),
        result = tokio::time::timeout(STEP_TIMEOUT, future) => {
            let result = result.map_err(|_| PeerCompareError::Transport)?.map_err(|_| PeerCompareError::Transport)?;
            cancel.check()?;
            Ok(result)
        }
    }
}

async fn disk<T: Send + 'static>(
    cancel: &Cancellation,
    operation: impl FnOnce() -> Result<T, PeerCompareError> + Send + 'static,
) -> Result<T, PeerCompareError> {
    cancel.check()?;
    let check = cancel.clone();
    let task = tokio::task::spawn_blocking(move || check.with_active(operation));
    let result = task.await.map_err(|_| PeerCompareError::Storage)??;
    cancel.check()?;
    Ok(result)
}

pub(crate) async fn send(
    mut channel: Channel,
    manifest: TransferManifest,
    source: PathBuf,
    cancel: Cancellation,
) -> Result<PeerCompareReport, PeerCompareError> {
    manifest.validate()?;
    if step(&cancel, channel.read_u8()).await? != 1 {
        return Err(PeerCompareError::InvalidProtocol);
    }
    let encoded = serde_json::to_vec(&manifest).map_err(|_| PeerCompareError::InvalidManifest)?;
    if encoded.len() > MAX_MANIFEST_BYTES {
        return Err(PeerCompareError::LimitExceeded);
    }
    step(&cancel, channel.write_all(MAGIC)).await?;
    step(&cancel, channel.write_u32(encoded.len() as u32)).await?;
    step(&cancel, channel.write_all(&encoded)).await?;
    let mut report = PeerCompareReport::pending(&manifest);
    let mut requested = vec![false; manifest.block_count() as usize];
    loop {
        let index = step(&cancel, channel.read_u32()).await?;
        if index == END {
            let complete = step(&cancel, channel.read_u8()).await?;
            if complete > 1 {
                return Err(PeerCompareError::InvalidProtocol);
            }
            step(&cancel, channel.write_u8(1)).await?;
            step(&cancel, channel.flush()).await?;
            // The final receipt proves that the receiver observed our acknowledgement.
            if step(&cancel, channel.read_u8()).await? != 2 {
                return Err(PeerCompareError::InvalidProtocol);
            }
            step(&cancel, channel.shutdown()).await?;
            report.complete = complete == 1;
            return Ok(report);
        }
        if index >= manifest.block_count() || requested[index as usize] {
            return Err(PeerCompareError::InvalidBlock);
        }
        requested[index as usize] = true;
        let reading = manifest.clone();
        let path = source.clone();
        let bytes = disk(&cancel, move || reading.read_block(&path, index)).await?;
        step(&cancel, channel.write_u32(bytes.len() as u32)).await?;
        step(&cancel, channel.write_all(&bytes)).await?;
        report.bytes_sent += bytes.len() as u64;
        report.blocks_transferred += 1;
    }
}

pub(crate) async fn receive(
    mut channel: Channel,
    staging: PathBuf,
    target: PathBuf,
    max_blocks: Option<u32>,
    cancel: Cancellation,
) -> Result<PeerCompareReport, PeerCompareError> {
    step(&cancel, channel.write_u8(1)).await?;
    let mut magic = vec![0; MAGIC.len()];
    step(&cancel, channel.read_exact(&mut magic)).await?;
    if magic != MAGIC {
        return Err(PeerCompareError::InvalidProtocol);
    }
    let length = step(&cancel, channel.read_u32()).await? as usize;
    if length == 0 || length > MAX_MANIFEST_BYTES {
        return Err(PeerCompareError::LimitExceeded);
    }
    let mut encoded = vec![0; length];
    step(&cancel, channel.read_exact(&mut encoded)).await?;
    let manifest: TransferManifest =
        serde_json::from_slice(&encoded).map_err(|_| PeerCompareError::InvalidManifest)?;
    manifest.validate()?;
    let mut report = PeerCompareReport::pending(&manifest);
    let expected = manifest.clone();
    let check_target = target.clone();
    let existing = disk(&cancel, move || {
        if !check_target
            .try_exists()
            .map_err(|_| PeerCompareError::Storage)?
        {
            return Ok(false);
        }
        if TransferManifest::from_path(&check_target, expected.block_size())? != expected {
            return Err(PeerCompareError::Storage);
        }
        Ok(true)
    })
    .await?;
    if existing {
        report.complete = true;
        finish(&mut channel, true, &cancel).await?;
        return Ok(report);
    }
    let initial = manifest.clone();
    let mut partial = disk(&cancel, move || PartialTransfer::open(initial, &staging)).await?;
    for index in partial.needed_blocks() {
        if max_blocks.is_some_and(|limit| report.blocks_transferred >= limit) {
            break;
        }
        step(&cancel, channel.write_u32(index)).await?;
        let length = step(&cancel, channel.read_u32()).await? as usize;
        if length == 0 || length > MAX_BLOCK_BYTES || length != manifest.block_len(index)? {
            return Err(PeerCompareError::LimitExceeded);
        }
        let mut bytes = vec![0; length];
        step(&cancel, channel.read_exact(&mut bytes)).await?;
        partial = disk(&cancel, move || {
            partial.write_block(index, &bytes)?;
            Ok(partial)
        })
        .await?;
        report.bytes_received += length as u64;
        report.blocks_transferred += 1;
    }
    if partial.needed_blocks().is_empty() {
        disk(&cancel, move || partial.finish(&target)).await?;
        report.complete = true;
    }
    finish(&mut channel, report.complete, &cancel).await?;
    Ok(report)
}

async fn finish(
    channel: &mut Channel,
    complete: bool,
    cancel: &Cancellation,
) -> Result<(), PeerCompareError> {
    step(cancel, channel.write_u32(END)).await?;
    step(cancel, channel.write_u8(u8::from(complete))).await?;
    step(cancel, channel.flush()).await?;
    if step(cancel, channel.read_u8()).await? != 1 {
        return Err(PeerCompareError::InvalidProtocol);
    }
    step(cancel, channel.write_u8(2)).await?;
    step(cancel, channel.shutdown()).await
}

pub(crate) fn fixture_path(path: String) -> Result<PathBuf, PeerCompareError> {
    if path.len() > 4096 || path.contains('\0') || !Path::new(&path).is_absolute() {
        return Err(PeerCompareError::InvalidConfig);
    }
    Ok(PathBuf::from(path))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn shared_wire_resumes_and_repeats_without_content() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let source = root.join("source");
        std::fs::write(&source, vec![7; 300_000]).unwrap();
        let manifest = TransferManifest::from_path(&source, 128 * 1024).unwrap();
        for (limit, expected, complete) in [
            (Some(1), 131_072, false),
            (None, 168_928, true),
            (None, 0, true),
        ] {
            let (a, b) = tokio::io::duplex(64 * 1024);
            let sent = tokio::spawn(send(
                Box::new(a),
                manifest.clone(),
                source.clone(),
                Cancellation::default(),
            ));
            let received = receive(
                Box::new(b),
                root.join("partial"),
                root.join("target"),
                limit,
                Cancellation::default(),
            )
            .await
            .unwrap();
            assert_eq!(received.bytes_received, expected);
            assert_eq!(received.complete, complete);
            assert_eq!(sent.await.unwrap().unwrap().bytes_sent, expected);
        }
        assert_eq!(
            std::fs::read(root.join("target")).unwrap(),
            vec![7; 300_000]
        );
    }
    #[tokio::test]
    async fn cancellation_unblocks_stalled_read_without_publication() {
        let dir = tempfile::tempdir().unwrap();
        let (mut a, b) = tokio::io::duplex(64);
        let token = Cancellation::default();
        let cancelled = token.clone();
        let task = tokio::spawn(receive(
            Box::new(b),
            dir.path().join("stage"),
            dir.path().join("target"),
            None,
            token,
        ));
        assert_eq!(a.read_u8().await.unwrap(), 1);
        let began = std::time::Instant::now();
        cancelled.cancel();
        let result = tokio::time::timeout(std::time::Duration::from_millis(250), task)
            .await
            .expect("stalled network cancellation must not wait for the step watchdog")
            .unwrap();
        eprintln!(
            "peer_compare_stalled_read_cancel_ms={:.3}",
            began.elapsed().as_secs_f64() * 1000.0
        );
        assert!(
            began.elapsed() <= std::time::Duration::from_millis(250),
            "the complete stalled-read cancel operation exceeded its deadline"
        );
        assert_eq!(result, Err(PeerCompareError::Cancelled));
        assert!(!dir.path().join("target").exists());
    }
    #[tokio::test]
    async fn oversized_manifest_is_rejected_before_allocation() {
        let dir = tempfile::tempdir().unwrap();
        let (mut a, b) = tokio::io::duplex(64);
        let task = tokio::spawn(async move {
            a.read_u8().await.unwrap();
            a.write_all(MAGIC).await.unwrap();
            a.write_u32((MAX_MANIFEST_BYTES + 1) as u32).await.unwrap();
        });
        let result = receive(
            Box::new(b),
            dir.path().join("stage"),
            dir.path().join("target"),
            None,
            Cancellation::default(),
        )
        .await;
        assert_eq!(result, Err(PeerCompareError::LimitExceeded));
        task.await.unwrap();
    }

    #[tokio::test]
    async fn truncated_wire_headers_never_create_staging_or_complete() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let source = root.join("source");
        std::fs::write(&source, b"synthetic header fixture").unwrap();
        let manifest = TransferManifest::from_path(&source, 128 * 1024).unwrap();
        let encoded = serde_json::to_vec(&manifest).unwrap();
        let mut truncated_length = MAGIC.to_vec();
        truncated_length.extend_from_slice(&[0, 1]);
        let mut truncated_manifest = MAGIC.to_vec();
        truncated_manifest.extend_from_slice(&(encoded.len() as u32).to_be_bytes());
        truncated_manifest.extend_from_slice(&encoded[..encoded.len() / 2]);
        for (index, bytes) in [
            MAGIC[..MAGIC.len() / 2].to_vec(),
            truncated_length,
            truncated_manifest,
        ]
        .into_iter()
        .enumerate()
        {
            let (mut sender, receiver) = tokio::io::duplex(64 * 1024);
            let malicious = tokio::spawn(async move {
                assert_eq!(sender.read_u8().await.unwrap(), 1);
                sender.write_all(&bytes).await.unwrap();
                sender.shutdown().await.unwrap();
                let mut unexpected = Vec::new();
                sender.read_to_end(&mut unexpected).await.unwrap();
                unexpected
            });
            let staging = root.join(format!("stage-{index}"));
            let target = root.join(format!("target-{index}"));
            let result = tokio::time::timeout(
                std::time::Duration::from_secs(2),
                receive(
                    Box::new(receiver),
                    staging.clone(),
                    target.clone(),
                    None,
                    Cancellation::default(),
                ),
            )
            .await
            .unwrap();
            assert_eq!(result, Err(PeerCompareError::Transport));
            assert!(!staging.exists());
            assert!(!target.exists());
            assert!(malicious.await.unwrap().is_empty());
        }
    }

    #[tokio::test]
    async fn invalid_magic_and_malformed_manifest_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let mut invalid_magic = MAGIC.to_vec();
        invalid_magic[0] ^= 1;
        let mut malformed = MAGIC.to_vec();
        malformed.extend_from_slice(&2u32.to_be_bytes());
        malformed.extend_from_slice(b"{]");
        for (index, (bytes, expected)) in [
            (invalid_magic, PeerCompareError::InvalidProtocol),
            (malformed, PeerCompareError::InvalidManifest),
        ]
        .into_iter()
        .enumerate()
        {
            let (mut sender, receiver) = tokio::io::duplex(64 * 1024);
            let malicious = tokio::spawn(async move {
                assert_eq!(sender.read_u8().await.unwrap(), 1);
                sender.write_all(&bytes).await.unwrap();
                sender.shutdown().await.unwrap();
            });
            let staging = root.join(format!("stage-{index}"));
            let target = root.join(format!("target-{index}"));
            assert_eq!(
                receive(
                    Box::new(receiver),
                    staging.clone(),
                    target.clone(),
                    None,
                    Cancellation::default(),
                )
                .await,
                Err(expected)
            );
            assert!(!staging.exists());
            assert!(!target.exists());
            malicious.await.unwrap();
        }
    }

    async fn write_manifest(sender: &mut tokio::io::DuplexStream, manifest: &TransferManifest) {
        assert_eq!(sender.read_u8().await.unwrap(), 1);
        let encoded = serde_json::to_vec(manifest).unwrap();
        sender.write_all(MAGIC).await.unwrap();
        sender.write_u32(encoded.len() as u32).await.unwrap();
        sender.write_all(&encoded).await.unwrap();
    }

    #[tokio::test]
    async fn truncated_block_preserves_only_verified_receipts() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let source = root.join("source");
        std::fs::write(&source, vec![0x37; 128 * 1024 + 17]).unwrap();
        let manifest = TransferManifest::from_path(&source, 128 * 1024).unwrap();
        let sending = manifest.clone();
        let (mut sender, receiver) = tokio::io::duplex(64 * 1024);
        let malicious = tokio::spawn(async move {
            write_manifest(&mut sender, &sending).await;
            assert_eq!(sender.read_u32().await.unwrap(), 0);
            let first = sending.read_block(&source, 0).unwrap();
            sender.write_u32(first.len() as u32).await.unwrap();
            sender.write_all(&first).await.unwrap();
            assert_eq!(sender.read_u32().await.unwrap(), 1);
            let second = sending.read_block(&source, 1).unwrap();
            sender.write_u32(second.len() as u32).await.unwrap();
            sender.write_all(&second[..second.len() / 2]).await.unwrap();
            sender.shutdown().await.unwrap();
            let mut unexpected = Vec::new();
            sender.read_to_end(&mut unexpected).await.unwrap();
            unexpected
        });
        let staging = root.join("partial");
        let target = root.join("target");
        assert_eq!(
            receive(
                Box::new(receiver),
                staging.clone(),
                target.clone(),
                None,
                Cancellation::default(),
            )
            .await,
            Err(PeerCompareError::Transport)
        );
        assert!(!target.exists());
        assert!(malicious.await.unwrap().is_empty());
        let resumed = PartialTransfer::open(manifest, &staging).unwrap();
        assert_eq!(resumed.received_bytes(), 128 * 1024);
        assert_eq!(resumed.needed_blocks(), vec![1]);
    }

    #[tokio::test]
    async fn publication_failure_never_sends_a_completion_receipt() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let source = root.join("source");
        let original = b"verified synthetic publication fixture";
        std::fs::write(&source, original).unwrap();
        let manifest = TransferManifest::from_path(&source, 128 * 1024).unwrap();
        let sending = manifest.clone();
        let target = root.join("target");
        let obstructed = target.clone();
        let (mut sender, receiver) = tokio::io::duplex(64 * 1024);
        let malicious = tokio::spawn(async move {
            write_manifest(&mut sender, &sending).await;
            assert_eq!(sender.read_u32().await.unwrap(), 0);
            // A real competing filesystem change after the receiver checked absence.
            std::fs::create_dir(&obstructed).unwrap();
            std::fs::write(obstructed.join("keep-me"), b"unrelated local data").unwrap();
            let bytes = sending.read_block(&source, 0).unwrap();
            sender.write_u32(bytes.len() as u32).await.unwrap();
            sender.write_all(&bytes).await.unwrap();
            let mut unexpected = Vec::new();
            sender.read_to_end(&mut unexpected).await.unwrap();
            unexpected
        });
        let staging = root.join("partial");
        assert_eq!(
            receive(
                Box::new(receiver),
                staging.clone(),
                target.clone(),
                None,
                Cancellation::default(),
            )
            .await,
            Err(PeerCompareError::Storage)
        );
        assert!(malicious.await.unwrap().is_empty());
        assert!(target.is_dir());
        assert_eq!(
            std::fs::read(target.join("keep-me")).unwrap(),
            b"unrelated local data"
        );
        let resumed = PartialTransfer::open(manifest, &staging).unwrap();
        assert!(resumed.needed_blocks().is_empty());
        let recovered = root.join("recovered");
        resumed.finish(&recovered).unwrap();
        assert_eq!(std::fs::read(recovered).unwrap(), original);
    }
}
