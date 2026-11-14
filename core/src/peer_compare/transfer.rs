use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::PeerSyncError;

const MIN_BLOCK_SIZE: u32 = 128 * 1024;
const MAX_BLOCK_SIZE: u32 = 1024 * 1024;
const MAX_BLOCKS: usize = 16_384;
const MAX_FILE_BYTES: u64 = 16 * 1024 * 1024 * 1024;
const MAX_MANIFEST_BYTES: u64 = 2 * 1024 * 1024;
const MANIFEST_FILE: &str = "manifest.json";

type Result<T> = std::result::Result<T, PeerSyncError>;

/// An immutable original-byte description. It deliberately contains no filesystem paths.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct TransferManifest {
    version: u32,
    size: u64,
    block_size: u32,
    sha256: String,
    blocks: Vec<String>,
    revision: String,
}

impl<'de> Deserialize<'de> for TransferManifest {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Fields {
            version: u32,
            size: u64,
            block_size: u32,
            sha256: String,
            blocks: Vec<String>,
            revision: String,
        }
        let fields = Fields::deserialize(deserializer)?;
        let manifest = Self {
            version: fields.version,
            size: fields.size,
            block_size: fields.block_size,
            sha256: fields.sha256,
            blocks: fields.blocks,
            revision: fields.revision,
        };
        manifest.validate().map_err(serde::de::Error::custom)?;
        Ok(manifest)
    }
}

impl TransferManifest {
    pub fn from_path(path: &Path, block_size: u32) -> Result<Self> {
        validate_block_size(block_size)?;
        let mut file = open_regular(path)?;
        let before = file.metadata().map_err(|_| PeerSyncError::Storage)?;
        let size = before.len();
        validate_size(size, block_size)?;
        let mut whole = Sha256::new();
        let mut blocks = Vec::new();
        let mut buffer = vec![0; block_size as usize];
        let mut read = 0_u64;
        loop {
            let length = read_fill(&mut file, &mut buffer)?;
            if length == 0 {
                break;
            }
            read = read
                .checked_add(length as u64)
                .ok_or(PeerSyncError::LimitExceeded)?;
            if read > size || blocks.len() >= MAX_BLOCKS {
                return Err(PeerSyncError::SourceChanged);
            }
            whole.update(&buffer[..length]);
            blocks.push(hash(&buffer[..length]));
        }
        if read != size || !unchanged(&before, &file, path)? {
            return Err(PeerSyncError::SourceChanged);
        }
        let mut manifest = Self {
            version: 1,
            size,
            block_size,
            sha256: hex(&whole.finalize()),
            blocks,
            revision: String::new(),
        };
        manifest.revision = manifest.calculate_revision();
        manifest.validate()?;
        Ok(manifest)
    }

    pub fn validate(&self) -> Result<()> {
        validate_block_size(self.block_size)?;
        validate_size(self.size, self.block_size)?;
        if self.version != 1
            || self.blocks.len() != block_count(self.size, self.block_size)
            || !valid_hash(&self.sha256)
            || !valid_hash(&self.revision)
            || self.blocks.iter().any(|digest| !valid_hash(digest))
            || self.revision != self.calculate_revision()
            || (self.size == 0 && self.sha256 != hash(&[]))
        {
            return Err(PeerSyncError::InvalidManifest);
        }
        Ok(())
    }

    pub fn size(&self) -> u64 {
        self.size
    }

    pub fn block_size(&self) -> u32 {
        self.block_size
    }

    pub fn block_count(&self) -> u32 {
        self.blocks.len() as u32
    }

    pub fn revision(&self) -> &str {
        &self.revision
    }

    pub fn sha256(&self) -> &str {
        &self.sha256
    }

    pub fn block_len(&self, index: u32) -> Result<usize> {
        if index as usize >= self.blocks.len() {
            return Err(PeerSyncError::InvalidBlock);
        }
        let offset = u64::from(index) * u64::from(self.block_size);
        Ok((self.size - offset).min(u64::from(self.block_size)) as usize)
    }

    pub fn read_block(&self, path: &Path, index: u32) -> Result<Vec<u8>> {
        let length = self.block_len(index)?;
        let mut file = open_regular(path)?;
        let before = file.metadata().map_err(|_| PeerSyncError::Storage)?;
        if before.len() != self.size {
            return Err(PeerSyncError::SourceChanged);
        }
        file.seek(SeekFrom::Start(
            u64::from(index) * u64::from(self.block_size),
        ))
        .map_err(|_| PeerSyncError::Storage)?;
        let mut bytes = vec![0; length];
        file.read_exact(&mut bytes)
            .map_err(|_| PeerSyncError::SourceChanged)?;
        if hash(&bytes) != self.blocks[index as usize] || !unchanged(&before, &file, path)? {
            return Err(PeerSyncError::SourceChanged);
        }
        Ok(bytes)
    }

    fn calculate_revision(&self) -> String {
        let mut digest = Sha256::new();
        digest.update(b"pennel-peer-transfer-v1\0");
        digest.update(self.size.to_le_bytes());
        digest.update(self.block_size.to_le_bytes());
        digest.update(self.sha256.as_bytes());
        digest.update((self.blocks.len() as u64).to_le_bytes());
        for block in &self.blocks {
            digest.update(block.as_bytes());
        }
        hex(&digest.finalize())
    }
}

/// Receiver-owned private staging, with every receipt reconstructed from verified on-disk bytes.
pub struct PartialTransfer {
    manifest: TransferManifest,
    directory: PathBuf,
    directory_handle: File,
    received: Vec<bool>,
}

impl PartialTransfer {
    pub fn open(manifest: TransferManifest, staging_path: &Path) -> Result<Self> {
        manifest.validate()?;
        let path = checked_absolute(staging_path)?;
        let parent = path.parent().ok_or(PeerSyncError::InvalidConfig)?;
        require_directory(parent)?;
        match fs::symlink_metadata(&path) {
            Ok(metadata) => {
                if !metadata.is_dir() || is_link(&metadata) {
                    return Err(PeerSyncError::Storage);
                }
                require_private(&metadata)?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let mut builder = fs::DirBuilder::new();
                #[cfg(unix)]
                {
                    use std::os::unix::fs::DirBuilderExt;
                    builder.mode(0o700);
                }
                builder.create(&path).map_err(|_| PeerSyncError::Storage)?;
                sync_directory(parent)?;
            }
            Err(_) => return Err(PeerSyncError::Storage),
        }
        require_directory(&path)?;
        let directory_handle = open_directory(&path)?;
        let manifest_path = path.join(MANIFEST_FILE);
        match fs::symlink_metadata(&manifest_path) {
            Ok(_) => {
                let mut file = open_regular(&manifest_path)?;
                if file.metadata().map_err(|_| PeerSyncError::Storage)?.len() > MAX_MANIFEST_BYTES {
                    return Err(PeerSyncError::InvalidManifest);
                }
                let mut bytes = Vec::new();
                Read::by_ref(&mut file)
                    .take(MAX_MANIFEST_BYTES + 1)
                    .read_to_end(&mut bytes)
                    .map_err(|_| PeerSyncError::Storage)?;
                if bytes.len() as u64 > MAX_MANIFEST_BYTES {
                    return Err(PeerSyncError::InvalidManifest);
                }
                let existing: TransferManifest =
                    serde_json::from_slice(&bytes).map_err(|_| PeerSyncError::InvalidManifest)?;
                existing.validate()?;
                if existing != manifest {
                    return Err(PeerSyncError::InvalidManifest);
                }
                recover_initialization(&path, &directory_handle, &manifest, false)?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                recover_initialization(&path, &directory_handle, &manifest, true)?;
                publish_manifest(&path, &directory_handle, &manifest)?;
            }
            Err(_) => return Err(PeerSyncError::Storage),
        }
        let mut transfer = Self {
            received: vec![false; manifest.blocks.len()],
            manifest,
            directory: path,
            directory_handle,
        };
        for index in 0..transfer.manifest.block_count() {
            transfer.received[index as usize] = transfer.verified_block(index)?.is_some();
        }
        Ok(transfer)
    }

    pub fn manifest(&self) -> &TransferManifest {
        &self.manifest
    }

    pub fn needed_blocks(&self) -> Vec<u32> {
        self.received
            .iter()
            .enumerate()
            .filter_map(|(index, present)| (!present).then_some(index as u32))
            .collect()
    }

    pub fn received_bytes(&self) -> u64 {
        self.received
            .iter()
            .enumerate()
            .filter(|(_, present)| **present)
            .map(|(index, _)| self.manifest.block_len(index as u32).unwrap_or_default() as u64)
            .sum()
    }

    pub fn write_block(&mut self, index: u32, bytes: &[u8]) -> Result<()> {
        self.check_directory()?;
        if bytes.len() != self.manifest.block_len(index)?
            || hash(bytes) != self.manifest.blocks[index as usize]
        {
            return Err(PeerSyncError::InvalidBlock);
        }
        if self.verified_block(index)?.is_some() {
            self.received[index as usize] = true;
            return Ok(());
        }
        let temporary = self
            .directory
            .join(format!(".block-{}.tmp", uuid::Uuid::now_v7()));
        let destination = self.block_path(index);
        let result = (|| {
            let mut file = private_new(&temporary)?;
            file.write_all(bytes)
                .and_then(|_| file.sync_all())
                .map_err(|_| PeerSyncError::Storage)?;
            drop(file);
            self.check_directory()?;
            refuse_link_if_present(&destination)?;
            fs::rename(&temporary, &destination).map_err(|_| PeerSyncError::Storage)?;
            sync_directory(&self.directory)?;
            self.received[index as usize] = true;
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result
    }

    /// Installs only a fully checked file; an existing destination is preserved for the caller.
    pub fn finish(&self, target: &Path) -> Result<()> {
        self.check_directory()?;
        let target = checked_absolute(target)?;
        let parent = target.parent().ok_or(PeerSyncError::InvalidConfig)?;
        require_directory(parent)?;
        if fs::symlink_metadata(&target).is_ok() {
            return Err(PeerSyncError::Storage);
        }
        let temporary = parent.join(format!(".pennel-peer-{}.tmp", uuid::Uuid::now_v7()));
        let result = (|| {
            let mut file = private_new(&temporary)?;
            let mut whole = Sha256::new();
            for index in 0..self.manifest.block_count() {
                let bytes = self
                    .verified_block(index)?
                    .ok_or(PeerSyncError::InvalidBlock)?;
                whole.update(&bytes);
                file.write_all(&bytes).map_err(|_| PeerSyncError::Storage)?;
            }
            if hex(&whole.finalize()) != self.manifest.sha256 {
                return Err(PeerSyncError::InvalidBlock);
            }
            file.sync_all().map_err(|_| PeerSyncError::Storage)?;
            drop(file);
            require_directory(parent)?;
            // An exclusive hard link provides no-clobber publication on every supported OS.
            fs::hard_link(&temporary, &target).map_err(|_| PeerSyncError::Storage)?;
            fs::remove_file(&temporary).map_err(|_| PeerSyncError::Storage)?;
            sync_directory(parent)?;
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result
    }

    fn block_path(&self, index: u32) -> PathBuf {
        self.directory.join(format!("block-{index:08}.part"))
    }

    fn check_directory(&self) -> Result<()> {
        require_directory(&self.directory)?;
        let current = open_directory(&self.directory)?;
        if file_identity(&self.directory_handle)? != file_identity(&current)? {
            return Err(PeerSyncError::Storage);
        }
        Ok(())
    }

    fn verified_block(&self, index: u32) -> Result<Option<Vec<u8>>> {
        self.check_directory()?;
        let expected_len = self.manifest.block_len(index)?;
        let path = self.block_path(index);
        match fs::symlink_metadata(&path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(_) => return Err(PeerSyncError::Storage),
            Ok(metadata) if is_link(&metadata) || !metadata.is_file() => {
                return Err(PeerSyncError::Storage);
            }
            Ok(metadata) if metadata.len() != expected_len as u64 => return Ok(None),
            Ok(_) => {}
        }
        let mut file = open_regular(&path)?;
        let mut bytes = vec![0; expected_len];
        if file.read_exact(&mut bytes).is_err() {
            return Ok(None);
        }
        let mut extra = [0];
        if file.read(&mut extra).map_err(|_| PeerSyncError::Storage)? != 0
            || hash(&bytes) != self.manifest.blocks[index as usize]
        {
            return Ok(None);
        }
        Ok(Some(bytes))
    }
}

fn validate_block_size(size: u32) -> Result<()> {
    if !(MIN_BLOCK_SIZE..=MAX_BLOCK_SIZE).contains(&size) || !size.is_power_of_two() {
        return Err(PeerSyncError::InvalidManifest);
    }
    Ok(())
}

fn recover_initialization(
    directory: &Path,
    handle: &File,
    manifest: &TransferManifest,
    only_initialization_files: bool,
) -> Result<()> {
    let prefix = format!(".manifest-{}-", manifest.revision());
    let mut owned = Vec::new();
    let owner = handle.metadata().map_err(|_| PeerSyncError::Storage)?;
    for (count, entry) in fs::read_dir(directory)
        .map_err(|_| PeerSyncError::Storage)?
        .enumerate()
    {
        if count >= MAX_BLOCKS + 64 {
            return Err(PeerSyncError::LimitExceeded);
        }
        let entry = entry.map_err(|_| PeerSyncError::Storage)?;
        let name = entry.file_name();
        let ours = name
            .to_str()
            .and_then(|name| name.strip_prefix(&prefix))
            .and_then(|name| name.strip_suffix(".tmp"))
            .is_some_and(|token| {
                uuid::Uuid::parse_str(token)
                    .is_ok_and(|id| id.get_version_num() == 7 && id.to_string() == token)
            });
        if !ours {
            if only_initialization_files {
                return Err(PeerSyncError::InvalidManifest);
            }
            continue;
        }
        let path = entry.path();
        let file = open_regular(&path)?;
        let metadata = file.metadata().map_err(|_| PeerSyncError::Storage)?;
        require_private(&metadata)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if owner.uid() != metadata.uid() {
                return Err(PeerSyncError::Storage);
            }
        }
        let _ = &owner;
        if metadata.len() > MAX_MANIFEST_BYTES {
            return Err(PeerSyncError::InvalidManifest);
        }
        owned.push((path, file_identity(&file)?));
    }
    // Validate the complete listing before removing any interrupted publisher's file.
    for (path, identity) in owned {
        remove_owned_temporary(directory, handle, &path, &identity)?;
    }
    Ok(())
}

fn publish_manifest(directory: &Path, handle: &File, manifest: &TransferManifest) -> Result<()> {
    let bytes = serde_json::to_vec(manifest).map_err(|_| PeerSyncError::InvalidManifest)?;
    let temporary = directory.join(format!(
        ".manifest-{}-{}.tmp",
        manifest.revision(),
        uuid::Uuid::now_v7()
    ));
    let mut file = private_new(&temporary)?;
    let identity = file_identity(&file)?;
    let result = (|| {
        file.write_all(&bytes)
            .and_then(|_| file.sync_all())
            .map_err(|_| PeerSyncError::Storage)?;
        ensure_directory_identity(directory, handle)?;
        fs::hard_link(&temporary, directory.join(MANIFEST_FILE))
            .map_err(|_| PeerSyncError::Storage)?;
        sync_directory(directory)
    })();
    drop(file);
    let cleanup = remove_owned_temporary(directory, handle, &temporary, &identity);
    result?;
    cleanup
}

fn remove_owned_temporary(
    directory: &Path,
    handle: &File,
    path: &Path,
    identity: &FileIdentity,
) -> Result<()> {
    ensure_directory_identity(directory, handle)?;
    let file = open_regular(path)?;
    if &file_identity(&file)? != identity {
        return Err(PeerSyncError::Storage);
    }
    drop(file);
    fs::remove_file(path).map_err(|_| PeerSyncError::Storage)?;
    sync_directory(directory)
}

fn ensure_directory_identity(path: &Path, handle: &File) -> Result<()> {
    let current = open_directory(path)?;
    if file_identity(handle)? != file_identity(&current)? {
        return Err(PeerSyncError::Storage);
    }
    Ok(())
}

fn validate_size(size: u64, block_size: u32) -> Result<()> {
    if size > MAX_FILE_BYTES || block_count(size, block_size) > MAX_BLOCKS {
        return Err(PeerSyncError::LimitExceeded);
    }
    Ok(())
}

fn block_count(size: u64, block_size: u32) -> usize {
    size.div_ceil(u64::from(block_size)) as usize
}

fn read_fill(reader: &mut impl Read, bytes: &mut [u8]) -> Result<usize> {
    let mut read = 0;
    while read < bytes.len() {
        match reader.read(&mut bytes[read..]) {
            Ok(0) => break,
            Ok(length) => read += length,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => return Err(PeerSyncError::Storage),
        }
    }
    Ok(read)
}

fn hash(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn valid_hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn checked_absolute(path: &Path) -> Result<PathBuf> {
    let absolute = std::path::absolute(path).map_err(|_| PeerSyncError::Storage)?;
    if absolute
        .components()
        .any(|part| matches!(part, Component::ParentDir))
    {
        return Err(PeerSyncError::Storage);
    }
    let mut current = PathBuf::new();
    for part in absolute.components() {
        current.push(part.as_os_str());
        match fs::symlink_metadata(&current) {
            Ok(metadata) if is_link(&metadata) => return Err(PeerSyncError::Storage),
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err(PeerSyncError::Storage),
        }
    }
    Ok(absolute)
}

fn require_directory(path: &Path) -> Result<()> {
    let path = checked_absolute(path)?;
    let metadata = fs::symlink_metadata(path).map_err(|_| PeerSyncError::Storage)?;
    if !metadata.is_dir() || is_link(&metadata) {
        return Err(PeerSyncError::Storage);
    }
    Ok(())
}

fn require_private(metadata: &fs::Metadata) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err(PeerSyncError::Storage);
        }
    }
    let _ = metadata;
    Ok(())
}

fn is_link(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_type().is_symlink() || metadata.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        metadata.file_type().is_symlink()
    }
}

fn refuse_link_if_present(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if is_link(&metadata) || !metadata.is_file() => Err(PeerSyncError::Storage),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err(PeerSyncError::Storage),
    }
}

fn open_regular(path: &Path) -> Result<File> {
    let path = checked_absolute(path)?;
    let before = fs::symlink_metadata(&path).map_err(|_| PeerSyncError::Storage)?;
    if !before.is_file() || is_link(&before) {
        return Err(PeerSyncError::Storage);
    }
    let mut options = OpenOptions::new();
    options.read(true);
    no_follow(&mut options);
    let file = options.open(&path).map_err(|_| PeerSyncError::Storage)?;
    let opened = file.metadata().map_err(|_| PeerSyncError::Storage)?;
    let named = options.open(&path).map_err(|_| PeerSyncError::Storage)?;
    let after = named.metadata().map_err(|_| PeerSyncError::Storage)?;
    if !opened.is_file()
        || is_link(&opened)
        || !after.is_file()
        || is_link(&after)
        || file_identity(&file)? != file_identity(&named)?
    {
        return Err(PeerSyncError::Storage);
    }
    Ok(file)
}

fn private_new(path: &Path) -> Result<File> {
    let path = checked_absolute(path)?;
    require_directory(path.parent().ok_or(PeerSyncError::Storage)?)?;
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    no_follow(&mut options);
    options.open(path).map_err(|_| PeerSyncError::Storage)
}

fn no_follow(options: &mut OpenOptions) {
    #[cfg(target_os = "macos")]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(0x100); // O_NOFOLLOW on Darwin.
    }
    #[cfg(target_os = "ios")]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(0x100);
    }
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(0x20000); // O_NOFOLLOW on Linux.
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(0x00200000); // FILE_FLAG_OPEN_REPARSE_POINT.
    }
    let _ = options;
}

#[derive(PartialEq, Eq)]
struct FileIdentity {
    volume: u64,
    index: u64,
}

fn file_identity(file: &File) -> Result<FileIdentity> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let metadata = file.metadata().map_err(|_| PeerSyncError::Storage)?;
        Ok(FileIdentity {
            volume: metadata.dev(),
            index: metadata.ino(),
        })
    }
    #[cfg(windows)]
    {
        use std::ffi::c_void;
        use std::mem::MaybeUninit;
        use std::os::windows::io::AsRawHandle;
        #[repr(C)]
        struct FileTime {
            low: u32,
            high: u32,
        }
        #[repr(C)]
        struct ByHandleFileInformation {
            attributes: u32,
            created: FileTime,
            accessed: FileTime,
            written: FileTime,
            volume: u32,
            size_high: u32,
            size_low: u32,
            links: u32,
            index_high: u32,
            index_low: u32,
        }
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn GetFileInformationByHandle(
                handle: *mut c_void,
                information: *mut ByHandleFileInformation,
            ) -> i32;
        }
        let mut information = MaybeUninit::<ByHandleFileInformation>::uninit();
        // The live File owns the HANDLE and Win32 fills this exact documented C structure.
        if unsafe { GetFileInformationByHandle(file.as_raw_handle(), information.as_mut_ptr()) }
            == 0
        {
            return Err(PeerSyncError::Storage);
        }
        let information = unsafe { information.assume_init() };
        Ok(FileIdentity {
            volume: u64::from(information.volume),
            index: (u64::from(information.index_high) << 32) | u64::from(information.index_low),
        })
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = file;
        Err(PeerSyncError::InvalidConfig)
    }
}

fn open_directory(path: &Path) -> Result<File> {
    require_directory(path)?;
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        // Open the directory itself and prevent its removal while a transfer owns its handle.
        options
            .custom_flags(0x02000000 | 0x00200000)
            .share_mode(0x1 | 0x2);
    }
    #[cfg(not(windows))]
    no_follow(&mut options);
    let directory = options.open(path).map_err(|_| PeerSyncError::Storage)?;
    let metadata = directory.metadata().map_err(|_| PeerSyncError::Storage)?;
    if !metadata.is_dir() || is_link(&metadata) {
        return Err(PeerSyncError::Storage);
    }
    Ok(directory)
}

fn unchanged(before: &fs::Metadata, file: &File, path: &Path) -> Result<bool> {
    let after = file.metadata().map_err(|_| PeerSyncError::Storage)?;
    let named = open_regular(path).map_err(|_| PeerSyncError::SourceChanged)?;
    Ok(file_identity(file)? == file_identity(&named)?
        && before.len() == after.len()
        && before.modified().ok() == after.modified().ok())
}

fn sync_directory(path: &Path) -> Result<()> {
    #[cfg(unix)]
    File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(|_| PeerSyncError::Storage)?;
    let _ = path;
    Ok(())
}
