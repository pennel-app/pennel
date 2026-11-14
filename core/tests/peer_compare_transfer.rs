#![cfg(feature = "peer-transport-qa")]

use std::fs;

use pennel_core::peer_compare::{PartialTransfer, TransferManifest};

#[test]
fn original_bytes_survive_verified_transfer() {
    let dir = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(dir.path()).unwrap();
    let source = root.join("source.m4a");
    let original: Vec<u8> = (0..300_000).map(|index| (index % 251) as u8).collect();
    fs::write(&source, &original).unwrap();
    let manifest = TransferManifest::from_path(&source, 128 * 1024).unwrap();
    let mut transfer = PartialTransfer::open(manifest.clone(), &root.join("partial")).unwrap();

    for index in transfer.needed_blocks() {
        transfer
            .write_block(index, &manifest.read_block(&source, index).unwrap())
            .unwrap();
    }
    let destination = root.join("received.m4a");
    transfer.finish(&destination).unwrap();

    assert_eq!(fs::read(destination).unwrap(), original);
}

#[test]
fn malformed_manifest_never_performs_unchecked_block_arithmetic() {
    let dir = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(dir.path()).unwrap();
    let source = root.join("source");
    fs::write(&source, vec![7; 300_000]).unwrap();
    let mut encoded =
        serde_json::to_value(TransferManifest::from_path(&source, 128 * 1024).unwrap()).unwrap();
    encoded["size"] = 1.into();
    assert!(serde_json::from_value::<TransferManifest>(encoded).is_err());
}

#[test]
fn restart_requests_only_unconfirmed_original_blocks() {
    let dir = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(dir.path()).unwrap();
    let source = root.join("source");
    fs::write(&source, vec![4; 300_000]).unwrap();
    let manifest = TransferManifest::from_path(&source, 128 * 1024).unwrap();
    let staging = root.join("partial");
    let mut first = PartialTransfer::open(manifest.clone(), &staging).unwrap();
    first
        .write_block(0, &manifest.read_block(&source, 0).unwrap())
        .unwrap();
    drop(first);

    let resumed = PartialTransfer::open(manifest, &staging).unwrap();

    assert_eq!(resumed.needed_blocks(), vec![1, 2]);
    assert_eq!(resumed.received_bytes(), 128 * 1024);
}

#[test]
fn corruption_and_missing_parts_are_requested_again_after_restart() {
    let dir = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(dir.path()).unwrap();
    let source = root.join("source");
    fs::write(&source, vec![4; 300_000]).unwrap();
    let manifest = TransferManifest::from_path(&source, 128 * 1024).unwrap();
    let staging = root.join("partial");
    let mut first = PartialTransfer::open(manifest.clone(), &staging).unwrap();
    for index in first.needed_blocks() {
        first
            .write_block(index, &manifest.read_block(&source, index).unwrap())
            .unwrap();
    }
    drop(first);
    fs::write(staging.join("block-00000000.part"), vec![8; 128 * 1024]).unwrap();
    fs::remove_file(staging.join("block-00000001.part")).unwrap();

    let resumed = PartialTransfer::open(manifest, &staging).unwrap();

    assert_eq!(resumed.needed_blocks(), vec![0, 1]);
    assert_eq!(resumed.received_bytes(), 37_856);
}

#[test]
fn received_data_requires_exact_index_length_and_digest() {
    use pennel_core::peer_compare::PeerSyncError;
    let dir = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(dir.path()).unwrap();
    let source = root.join("source");
    fs::write(&source, vec![4; 300_000]).unwrap();
    let manifest = TransferManifest::from_path(&source, 128 * 1024).unwrap();
    let mut transfer = PartialTransfer::open(manifest, &root.join("partial")).unwrap();

    assert_eq!(
        transfer.write_block(0, &vec![9; 128 * 1024]),
        Err(PeerSyncError::InvalidBlock)
    );
    assert_eq!(
        transfer.write_block(0, &vec![4; 128 * 1024 + 1]),
        Err(PeerSyncError::InvalidBlock)
    );
    assert_eq!(
        transfer.write_block(u32::MAX, &[4]),
        Err(PeerSyncError::InvalidBlock)
    );
    assert_eq!(transfer.received_bytes(), 0);
    assert_eq!(
        transfer.finish(&root.join("received")),
        Err(PeerSyncError::InvalidBlock)
    );
    assert!(!root.join("received").exists());
}

#[test]
fn a_different_revision_cannot_reuse_existing_receipts() {
    let dir = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(dir.path()).unwrap();
    let source = root.join("source");
    fs::write(&source, vec![4; 300_000]).unwrap();
    let original = TransferManifest::from_path(&source, 128 * 1024).unwrap();
    let staging = root.join("partial");
    let mut transfer = PartialTransfer::open(original.clone(), &staging).unwrap();
    transfer
        .write_block(0, &original.read_block(&source, 0).unwrap())
        .unwrap();
    drop(transfer);
    fs::write(&source, vec![5; 300_000]).unwrap();
    let other = TransferManifest::from_path(&source, 128 * 1024).unwrap();

    assert!(matches!(
        PartialTransfer::open(other, &staging),
        Err(pennel_core::peer_compare::PeerSyncError::InvalidManifest)
    ));
    assert_eq!(
        PartialTransfer::open(original, &staging)
            .unwrap()
            .needed_blocks(),
        vec![1, 2]
    );
}

#[test]
fn changed_source_is_not_sent_under_the_old_revision() {
    let dir = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(dir.path()).unwrap();
    let source = root.join("source");
    fs::write(&source, vec![4; 300_000]).unwrap();
    let manifest = TransferManifest::from_path(&source, 128 * 1024).unwrap();
    fs::write(&source, vec![5; 300_000]).unwrap();

    assert_eq!(
        manifest.read_block(&source, 0),
        Err(pennel_core::peer_compare::PeerSyncError::SourceChanged)
    );
}

#[test]
fn existing_destination_is_preserved_for_conflict_handling() {
    let dir = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(dir.path()).unwrap();
    let source = root.join("source");
    fs::write(&source, b"original").unwrap();
    let manifest = TransferManifest::from_path(&source, 128 * 1024).unwrap();
    let mut transfer = PartialTransfer::open(manifest.clone(), &root.join("partial")).unwrap();
    transfer
        .write_block(0, &manifest.read_block(&source, 0).unwrap())
        .unwrap();
    let target = root.join("target");
    fs::write(&target, b"local edit").unwrap();

    assert_eq!(
        transfer.finish(&target),
        Err(pennel_core::peer_compare::PeerSyncError::Storage)
    );
    assert_eq!(fs::read(target).unwrap(), b"local edit");
}

#[cfg(unix)]
#[test]
fn symbolic_paths_and_substituted_staging_are_refused() {
    use std::os::unix::fs::symlink;
    let dir = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(dir.path()).unwrap();
    let source = root.join("source");
    fs::write(&source, b"original").unwrap();
    symlink(&source, root.join("linked-source")).unwrap();
    assert!(TransferManifest::from_path(&root.join("linked-source"), 128 * 1024).is_err());
    let manifest = TransferManifest::from_path(&source, 128 * 1024).unwrap();
    let staging = root.join("partial");
    let mut transfer = PartialTransfer::open(manifest.clone(), &staging).unwrap();
    fs::rename(&staging, root.join("old-partial")).unwrap();
    fs::create_dir(&staging).unwrap();

    assert!(
        transfer
            .write_block(0, &manifest.read_block(&source, 0).unwrap())
            .is_err()
    );
    fs::remove_dir(&staging).unwrap();
    symlink(root.join("old-partial"), &staging).unwrap();
    assert!(PartialTransfer::open(manifest, &staging).is_err());
}

#[test]
fn interrupted_manifest_initialization_resumes_without_deleting_the_stage() {
    use std::io::Write;
    let dir = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(dir.path()).unwrap();
    let source = root.join("source");
    fs::write(&source, b"original bytes after interrupted initialization").unwrap();
    let manifest = TransferManifest::from_path(&source, 128 * 1024).unwrap();
    let staging = root.join("partial");
    let mut directory = fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        directory.mode(0o700);
    }
    directory.create(&staging).unwrap();
    let interrupted = staging.join(format!(
        ".manifest-{}-{}.tmp",
        manifest.revision(),
        uuid::Uuid::now_v7()
    ));
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut temporary = options.open(&interrupted).unwrap();
    temporary.write_all(b"{\"version\":").unwrap();
    temporary.sync_all().unwrap();
    drop(temporary);

    let mut resumed = PartialTransfer::open(manifest.clone(), &staging).unwrap();
    assert_eq!(resumed.needed_blocks(), vec![0]);
    resumed
        .write_block(0, &manifest.read_block(&source, 0).unwrap())
        .unwrap();
    let target = root.join("target");
    resumed.finish(&target).unwrap();
    assert_eq!(
        fs::read(target).unwrap(),
        b"original bytes after interrupted initialization"
    );
    assert!(!interrupted.exists());
}

#[test]
fn initialization_never_overwrites_a_corrupt_published_manifest_or_unknown_file() {
    let dir = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(dir.path()).unwrap();
    let source = root.join("source");
    fs::write(&source, b"original").unwrap();
    let manifest = TransferManifest::from_path(&source, 128 * 1024).unwrap();
    let staging = root.join("partial");
    drop(PartialTransfer::open(manifest.clone(), &staging).unwrap());
    fs::write(
        staging.join("manifest.json"),
        b"truncated published manifest",
    )
    .unwrap();
    assert!(matches!(
        PartialTransfer::open(manifest.clone(), &staging),
        Err(pennel_core::peer_compare::PeerSyncError::InvalidManifest)
    ));
    assert_eq!(
        fs::read(staging.join("manifest.json")).unwrap(),
        b"truncated published manifest"
    );

    fs::remove_file(staging.join("manifest.json")).unwrap();
    fs::write(staging.join("keep-me"), b"unrelated owned data").unwrap();
    assert!(matches!(
        PartialTransfer::open(manifest, &staging),
        Err(pennel_core::peer_compare::PeerSyncError::InvalidManifest)
    ));
    assert_eq!(
        fs::read(staging.join("keep-me")).unwrap(),
        b"unrelated owned data"
    );
}
