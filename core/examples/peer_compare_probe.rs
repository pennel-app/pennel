//! JSON-line host driver for scoped synthetic fixtures. Keys never enter stdout.
use std::{
    fs::{self, OpenOptions},
    io::{self, BufRead, Write},
    path::PathBuf,
    sync::Arc,
};

use pennel_core::peer_compare::{PeerCompareError, PeerCompareIdentityStore, PeerComparisonProbe};
use serde_json::{Value, json};

const MAX_CONTROL: usize = 32 * 1024;

struct FileIdentityStore {
    path: PathBuf,
    account: String,
}
impl PeerCompareIdentityStore for FileIdentityStore {
    fn load(&self, account: String) -> Result<Option<String>, PeerCompareError> {
        if account != self.account {
            return Err(PeerCompareError::InvalidConfig);
        }
        let metadata = match fs::symlink_metadata(&self.path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(_) => return Err(PeerCompareError::Storage),
        };
        if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > 24 * 1024 {
            return Err(PeerCompareError::Storage);
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if metadata.permissions().mode() & 0o077 != 0 {
                return Err(PeerCompareError::Storage);
            }
        }
        fs::read_to_string(&self.path)
            .map(Some)
            .map_err(|_| PeerCompareError::Storage)
    }
    fn save(&self, account: String, value: String) -> Result<(), PeerCompareError> {
        if account != self.account || value.len() > 24 * 1024 {
            return Err(PeerCompareError::InvalidConfig);
        }
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options
            .open(&self.path)
            .map_err(|_| PeerCompareError::Storage)?;
        file.write_all(value.as_bytes())
            .and_then(|_| file.sync_all())
            .map_err(|_| PeerCompareError::Storage)
    }
}

fn code(error: &PeerCompareError) -> &'static str {
    match error {
        PeerCompareError::Transport => "transport",
        PeerCompareError::Unauthorized => "unauthorized",
        PeerCompareError::InvalidConfig => "invalid_config",
        PeerCompareError::Closed => "closed",
        PeerCompareError::InvalidManifest => "invalid_manifest",
        PeerCompareError::SourceChanged => "source_changed",
        PeerCompareError::InvalidBlock => "invalid_block",
        PeerCompareError::Storage => "storage",
        PeerCompareError::Cancelled => "cancelled",
        PeerCompareError::InvalidProtocol => "invalid_protocol",
        PeerCompareError::LimitExceeded => "limit_exceeded",
    }
}
fn emit(value: Value) -> io::Result<()> {
    let mut out = io::stdout().lock();
    serde_json::to_writer(&mut out, &value)?;
    out.write_all(b"\n")?;
    out.flush()
}
fn string(value: &Value, key: &str) -> Result<String, PeerCompareError> {
    value[key]
        .as_str()
        .map(str::to_owned)
        .ok_or(PeerCompareError::InvalidConfig)
}
fn optional_u32(value: &Value, key: &str) -> Result<Option<u32>, PeerCompareError> {
    if value[key].is_null() {
        return Ok(None);
    }
    value[key]
        .as_u64()
        .and_then(|n| u32::try_from(n).ok())
        .map(Some)
        .ok_or(PeerCompareError::InvalidConfig)
}

fn read_control(reader: &mut impl BufRead) -> io::Result<Option<Vec<u8>>> {
    let mut line = Vec::new();
    loop {
        let buffer = reader.fill_buf()?;
        if buffer.is_empty() {
            return if line.is_empty() {
                Ok(None)
            } else {
                Err(io::ErrorKind::UnexpectedEof.into())
            };
        }
        let count = buffer
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(buffer.len(), |position| position + 1);
        if line.len() + count > MAX_CONTROL {
            return Err(io::ErrorKind::InvalidData.into());
        }
        let ended = buffer[count - 1] == b'\n';
        line.extend_from_slice(&buffer[..count]);
        reader.consume(count);
        if ended {
            return Ok(Some(line));
        }
    }
}

fn probe() -> Result<Arc<PeerComparisonProbe>, PeerCompareError> {
    let mut identity = None;
    let mut run_id = None;
    let mut bind = "127.0.0.1:0".to_owned();
    let mut relay = None;
    let mut args = std::env::args().skip(1);
    while let Some(argument) = args.next() {
        let value = args.next().ok_or(PeerCompareError::InvalidConfig)?;
        match argument.as_str() {
            "--identity-file" if identity.is_none() => identity = Some(PathBuf::from(value)),
            "--run-id" if run_id.is_none() => run_id = Some(value),
            "--bind" => bind = value,
            "--candidate" if value == "quinn" => {}
            "--relay-config" if relay.is_none() => {
                let metadata =
                    fs::symlink_metadata(&value).map_err(|_| PeerCompareError::Storage)?;
                if !metadata.is_file()
                    || metadata.file_type().is_symlink()
                    || metadata.len() > MAX_CONTROL as u64
                {
                    return Err(PeerCompareError::InvalidConfig);
                }
                relay = Some(fs::read_to_string(value).map_err(|_| PeerCompareError::Storage)?);
            }
            _ => return Err(PeerCompareError::InvalidConfig),
        }
    }
    let path = identity.ok_or(PeerCompareError::InvalidConfig)?;
    if !path.is_absolute() {
        return Err(PeerCompareError::InvalidConfig);
    }
    let account = run_id.ok_or(PeerCompareError::InvalidConfig)?;
    PeerComparisonProbe::new_quinn(
        Arc::new(FileIdentityStore {
            path,
            account: account.clone(),
        }),
        account,
        bind,
        relay,
    )
}

fn command(probe: &PeerComparisonProbe, input: Value) -> Result<Value, PeerCompareError> {
    match input["command"]
        .as_str()
        .ok_or(PeerCompareError::InvalidConfig)?
    {
        "identity" => Ok(json!(probe.identity_id())),
        "address" => Ok(json!(probe.address_json()?)),
        "approve" => Ok(json!(probe.approve_peer(string(&input, "peer_id")?)?)),
        "revoke" => Ok(json!(probe.revoke_peer(string(&input, "peer_id")?)?)),
        "offer" => {
            let encoded = probe.offer_file(
                string(&input, "path")?,
                optional_u32(&input, "block_size")?.unwrap_or(1024 * 1024),
            )?;
            serde_json::from_str(&encoded).map_err(|_| PeerCompareError::InvalidManifest)
        }
        "receive" => {
            let address = match &input["address_json"] {
                Value::String(address) => address.clone(),
                Value::Object(_) => input["address_json"].to_string(),
                _ => return Err(PeerCompareError::InvalidConfig),
            };
            let report = probe.receive_file(
                address,
                string(&input, "staging")?,
                string(&input, "target")?,
                optional_u32(&input, "max_blocks")?,
            )?;
            serde_json::to_value(report).map_err(|_| PeerCompareError::InvalidProtocol)
        }
        "cancel" => {
            probe.cancel();
            Ok(json!(true))
        }
        "close" => {
            probe.close();
            Ok(json!(true))
        }
        _ => Err(PeerCompareError::InvalidConfig),
    }
}

fn main() {
    let probe = match probe() {
        Ok(probe) => probe,
        Err(error) => {
            let _ = emit(json!({"ok":false,"error":code(&error)}));
            std::process::exit(1);
        }
    };
    if emit(json!({"ok":true,"event":"ready","result":{
        "identity_id":probe.identity_id(), "address_json":probe.address_json().ok(),
        "candidate":probe.candidate(), "route":probe.route().ok()
    }}))
    .is_err()
    {
        probe.close();
        return;
    }
    let mut input = io::stdin().lock();
    while let Ok(Some(line)) = read_control(&mut input) {
        let parsed = serde_json::from_slice::<Value>(&line);
        let closing = parsed
            .as_ref()
            .is_ok_and(|value| value["command"] == "close");
        let result = parsed
            .map_err(|_| PeerCompareError::InvalidConfig)
            .and_then(|value| command(&probe, value));
        let response = match result {
            Ok(result) => json!({"ok":true,"result":result}),
            Err(error) => json!({"ok":false,"error":code(&error)}),
        };
        if emit(response).is_err() || closing {
            break;
        }
    }
    probe.close();
}
