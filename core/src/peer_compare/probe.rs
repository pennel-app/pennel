use std::{
    collections::HashSet,
    future::Future,
    net::SocketAddr,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
};

use tokio::{
    runtime::{Builder, Runtime},
    task::JoinHandle,
};
use zeroize::Zeroizing;

use super::{
    Cancellation, PeerCompareError, PeerCompareIdentity, PeerCompareNativeTransport,
    PeerCompareReport, TransferManifest, generate_peer_compare_identity,
    identity::valid_peer,
    native,
    quinn_transport::{Address, QuinnTransport},
    relay::RelayConfig,
    wire::{self, Channel, fixture_path},
};

#[uniffi::export(with_foreign)]
pub trait PeerCompareIdentityStore: Send + Sync {
    fn load(&self, account: String) -> Result<Option<String>, PeerCompareError>;
    fn save(&self, account: String, value: String) -> Result<(), PeerCompareError>;
}

enum Transport {
    Quinn(Arc<QuinnTransport>),
    Native(
        Arc<dyn PeerCompareNativeTransport>,
        Arc<Mutex<HashSet<String>>>,
    ),
}
impl Transport {
    fn id(&self) -> String {
        match self {
            Self::Quinn(t) => t.id(),
            Self::Native(t, _) => t.identity_id(),
        }
    }
    fn close(&self) {
        match self {
            Self::Quinn(t) => t.close(),
            Self::Native(t, _) => t.close(),
        }
    }
    fn address(&self) -> Result<String, PeerCompareError> {
        match self {
            Self::Quinn(t) => t.address_json(),
            Self::Native(t, _) => t.address_json(),
        }
    }
    fn approve(&self, id: String) -> Result<bool, PeerCompareError> {
        match self {
            Self::Quinn(t) => t.approve(&id),
            Self::Native(t, _) => t.approve_peer(id),
        }
    }
    fn revoke(&self, id: String) -> Result<bool, PeerCompareError> {
        match self {
            Self::Quinn(t) => t.revoke(&id),
            Self::Native(t, _) => t.revoke_peer(id),
        }
    }
    async fn accept(&self) -> Result<Channel, PeerCompareError> {
        match self {
            Self::Quinn(t) => t.accept().await,
            Self::Native(t, approved) => {
                let transport = t.clone();
                let channel = tokio::task::spawn_blocking(move || transport.accept())
                    .await
                    .map_err(|_| PeerCompareError::Transport)??;
                if !approved
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .contains(&channel.peer_id())
                {
                    channel.close();
                    return Err(PeerCompareError::Unauthorized);
                }
                Ok(native::stream(channel))
            }
        }
    }
    async fn connect(&self, address: String) -> Result<Channel, PeerCompareError> {
        match self {
            Self::Quinn(t) => t.connect(&address).await,
            Self::Native(t, approved) => {
                let expected = Address::parse(&address)?.identity_id;
                let check = expected.clone();
                let transport = t.clone();
                let channel =
                    tokio::task::spawn_blocking(move || transport.connect(address, expected))
                        .await
                        .map_err(|_| PeerCompareError::Transport)??;
                if channel.peer_id() != check
                    || !approved
                        .lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .contains(&check)
                {
                    channel.close();
                    return Err(PeerCompareError::Unauthorized);
                }
                Ok(native::stream(channel))
            }
        }
    }
}

/// Explicit fixture-only QA facade; it never opens a meeting library or shipping sync.
#[derive(uniffi::Object)]
pub struct PeerComparisonProbe {
    runtime: Option<Runtime>,
    transport: Arc<Transport>,
    cancelled: Cancellation,
    closed: AtomicBool,
    offer: Mutex<Option<JoinHandle<()>>>,
    offered: Arc<Mutex<Option<(TransferManifest, PathBuf)>>>,
    operation: Mutex<()>,
    approved: Arc<Mutex<HashSet<String>>>,
}

#[uniffi::export]
impl PeerComparisonProbe {
    #[uniffi::constructor]
    pub fn new_quinn(
        identity_store: Arc<dyn PeerCompareIdentityStore>,
        identity_account: String,
        bind_address: String,
        relay_config_json: Option<String>,
    ) -> Result<Arc<Self>, PeerCompareError> {
        let identity = stored_identity(identity_store.as_ref(), &identity_account)?;
        let bind: SocketAddr = bind_address
            .parse()
            .map_err(|_| PeerCompareError::InvalidConfig)?;
        if bind.ip().is_multicast() {
            return Err(PeerCompareError::InvalidConfig);
        }
        let relay = relay_config_json
            .as_deref()
            .map(RelayConfig::parse)
            .transpose()?;
        let runtime = runtime()?;
        let transport = on_runtime(&runtime, async move {
            QuinnTransport::bind(identity, bind, relay).await
        });
        match transport {
            Ok(transport) => Ok(Self::from_transport(runtime, Transport::Quinn(transport))),
            Err(error) => {
                runtime.shutdown_background();
                Err(error)
            }
        }
    }

    #[uniffi::constructor]
    pub fn new_native(
        transport: Arc<dyn PeerCompareNativeTransport>,
    ) -> Result<Arc<Self>, PeerCompareError> {
        if !valid_peer(&transport.identity_id()) {
            return Err(PeerCompareError::InvalidConfig);
        }
        let address = Address::parse(&transport.address_json()?)?;
        if address.identity_id != transport.identity_id() {
            return Err(PeerCompareError::InvalidConfig);
        }
        Ok(Self::from_transport(
            runtime()?,
            Transport::Native(transport, Arc::new(Mutex::new(HashSet::new()))),
        ))
    }

    pub fn identity_id(&self) -> String {
        self.transport.id()
    }
    pub fn candidate(&self) -> String {
        match self.transport.as_ref() {
            Transport::Quinn(_) => "quinn",
            Transport::Native(_, _) => "native",
        }
        .into()
    }
    pub fn route(&self) -> Result<String, PeerCompareError> {
        Ok(Address::parse(&self.address_json()?)?.route)
    }
    pub fn address_json(&self) -> Result<String, PeerCompareError> {
        self.ensure_open()?;
        self.transport.address()
    }
    pub fn approve_peer(&self, id: String) -> Result<bool, PeerCompareError> {
        self.ensure_open()?;
        if !valid_peer(&id) || id == self.identity_id() {
            return Err(PeerCompareError::InvalidConfig);
        }
        let mut approved = self.approved.lock().unwrap_or_else(|p| p.into_inner());
        if !approved.contains(&id) && approved.len() >= 16 {
            return Err(PeerCompareError::LimitExceeded);
        }
        let added = self.transport.approve(id.clone())?;
        approved.insert(id);
        Ok(added)
    }
    pub fn revoke_peer(&self, id: String) -> Result<bool, PeerCompareError> {
        self.ensure_open()?;
        self.approved
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(&id);
        self.transport.revoke(id)
    }

    pub fn offer_file(&self, path: String, block_size: u32) -> Result<String, PeerCompareError> {
        let _operation = self
            .operation
            .try_lock()
            .map_err(|_| PeerCompareError::InvalidConfig)?;
        self.ensure_open()?;
        let path = fixture_path(path)?;
        let reading = path.clone();
        let cancel = self.cancelled.clone();
        let manifest = self.run(async move {
            tokio::task::spawn_blocking(move || {
                cancel.check()?;
                let manifest = TransferManifest::from_path(&reading, block_size)?;
                cancel.check()?;
                Ok(manifest)
            })
            .await
            .map_err(|_| PeerCompareError::Storage)?
        })?;
        let encoded =
            serde_json::to_string(&manifest).map_err(|_| PeerCompareError::InvalidManifest)?;
        let mut offered = self.offer.lock().unwrap_or_else(|p| p.into_inner());
        *self.offered.lock().unwrap_or_else(|p| p.into_inner()) = Some((manifest, path));
        if offered.is_some() {
            return Ok(encoded);
        }
        let current = self.offered.clone();
        let transport = self.transport.clone();
        let token = self.cancelled.clone();
        *offered = Some(self.runtime.as_ref().ok_or(PeerCompareError::Closed)?.spawn(async move {
            loop {
                let channel = tokio::select! { biased; _ = token.wait() => break, channel = transport.accept() => channel };
                match channel {
                    Ok(channel) => {
                        let Some((manifest, path)) = current.lock().unwrap_or_else(|p| p.into_inner()).clone() else { break; };
                        let _ = wire::send(channel, manifest, path, token.clone()).await;
                    }
                    Err(PeerCompareError::Closed | PeerCompareError::Cancelled) => break,
                    Err(_) => tokio::time::sleep(std::time::Duration::from_millis(25)).await,
                }
            }
        }));
        Ok(encoded)
    }

    pub fn receive_file(
        &self,
        address_json: String,
        staging: String,
        target: String,
        max_blocks: Option<u32>,
    ) -> Result<PeerCompareReport, PeerCompareError> {
        let _operation = self
            .operation
            .try_lock()
            .map_err(|_| PeerCompareError::InvalidConfig)?;
        self.ensure_open()?;
        let address = Address::parse(&address_json)?;
        if !self
            .approved
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .contains(&address.identity_id)
        {
            return Err(PeerCompareError::Unauthorized);
        }
        let staging = fixture_path(staging)?;
        let target = fixture_path(target)?;
        let transport = self.transport.clone();
        let token = self.cancelled.clone();
        self.run(async move {
            let channel = tokio::select! { biased; _ = token.wait() => return Err(PeerCompareError::Cancelled), channel = transport.connect(address_json) => channel? };
            wire::receive(channel, staging, target, max_blocks, token).await
        })
    }

    /// Cancellation ends this isolated probe and unblocks native reads and accepts.
    pub fn cancel(&self) {
        self.cancelled.cancel();
        self.transport.close();
    }
    pub fn close(&self) {
        self.closed.store(true, Ordering::Release);
        self.cancelled.cancel();
        self.transport.close();
        if let Some(task) = self.offer.lock().unwrap_or_else(|p| p.into_inner()).take() {
            task.abort();
        }
    }
}

impl PeerComparisonProbe {
    fn from_transport(runtime: Runtime, transport: Transport) -> Arc<Self> {
        let approved = match &transport {
            Transport::Native(_, approved) => approved.clone(),
            Transport::Quinn(_) => Arc::new(Mutex::new(HashSet::new())),
        };
        Arc::new(Self {
            runtime: Some(runtime),
            transport: Arc::new(transport),
            cancelled: Cancellation::default(),
            closed: AtomicBool::new(false),
            offer: Mutex::new(None),
            offered: Arc::new(Mutex::new(None)),
            operation: Mutex::new(()),
            approved,
        })
    }
    fn ensure_open(&self) -> Result<(), PeerCompareError> {
        if self.closed.load(Ordering::Acquire) {
            return Err(PeerCompareError::Closed);
        }
        self.cancelled.check()
    }
    fn run<T: Send + 'static>(
        &self,
        future: impl Future<Output = Result<T, PeerCompareError>> + Send + 'static,
    ) -> Result<T, PeerCompareError> {
        self.ensure_open()?;
        let result = on_runtime(
            self.runtime.as_ref().ok_or(PeerCompareError::Closed)?,
            future,
        );
        self.ensure_open()?;
        result
    }
}
impl Drop for PeerComparisonProbe {
    fn drop(&mut self) {
        self.close();
        if let Some(runtime) = self.runtime.take() {
            runtime.shutdown_background();
        }
    }
}

fn runtime() -> Result<Runtime, PeerCompareError> {
    Builder::new_multi_thread()
        .worker_threads(2)
        .max_blocking_threads(8)
        .thread_name("pennel-peer-compare-qa")
        .enable_all()
        .build()
        .map_err(|_| PeerCompareError::Transport)
}
fn on_runtime<T: Send + 'static>(
    runtime: &Runtime,
    future: impl Future<Output = Result<T, PeerCompareError>> + Send + 'static,
) -> Result<T, PeerCompareError> {
    let (sender, receiver) = mpsc::sync_channel(1);
    runtime.spawn(async move {
        let _ = sender.send(future.await);
    });
    receiver.recv().map_err(|_| PeerCompareError::Closed)?
}

fn stored_identity(
    store: &dyn PeerCompareIdentityStore,
    account: &str,
) -> Result<PeerCompareIdentity, PeerCompareError> {
    if !account.starts_with("peer-probe-")
        || account.len() <= 11
        || account.len() > 80
        || !account
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
    {
        return Err(PeerCompareError::InvalidConfig);
    }
    let encoded = match store.load(account.to_owned())? {
        Some(value) => Zeroizing::new(value),
        None => {
            let identity = generate_peer_compare_identity()?;
            let encoded = Zeroizing::new(
                serde_json::to_string(&identity).map_err(|_| PeerCompareError::Storage)?,
            );
            store.save(account.to_owned(), encoded.to_string())?;
            let readback = Zeroizing::new(store.load(account.to_owned())?);
            if readback.as_deref() != Some(encoded.as_str()) {
                return Err(PeerCompareError::Storage);
            }
            encoded
        }
    };
    if encoded.len() > 24 * 1024 {
        return Err(PeerCompareError::Storage);
    }
    let identity: PeerCompareIdentity =
        serde_json::from_str(&encoded).map_err(|_| PeerCompareError::Storage)?;
    identity.parts()?;
    Ok(identity)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[derive(Default)]
    struct Store(Mutex<std::collections::HashMap<String, String>>);
    impl PeerCompareIdentityStore for Store {
        fn load(&self, account: String) -> Result<Option<String>, PeerCompareError> {
            Ok(self.0.lock().unwrap().get(&account).cloned())
        }
        fn save(&self, account: String, value: String) -> Result<(), PeerCompareError> {
            self.0.lock().unwrap().insert(account, value);
            Ok(())
        }
    }
    #[test]
    fn direct_quinn_authenticates_resumes_and_rejects_unapproved_peers() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let source = root.join("source");
        std::fs::write(&source, vec![23; 300_000]).unwrap();
        let store = Arc::new(Store::default());
        let a = PeerComparisonProbe::new_quinn(
            store.clone(),
            "peer-probe-a".into(),
            "127.0.0.1:0".into(),
            None,
        )
        .unwrap();
        let b = PeerComparisonProbe::new_quinn(
            store,
            "peer-probe-b".into(),
            "127.0.0.1:0".into(),
            None,
        )
        .unwrap();
        let address = a.address_json().unwrap();
        assert_eq!(
            b.receive_file(
                address.clone(),
                root.join("stage").display().to_string(),
                root.join("target").display().to_string(),
                None
            ),
            Err(PeerCompareError::Unauthorized)
        );
        a.approve_peer(b.identity_id()).unwrap();
        b.approve_peer(a.identity_id()).unwrap();
        a.offer_file(source.display().to_string(), 128 * 1024)
            .unwrap();
        for (max, bytes, complete) in [
            (Some(1), 131_072, false),
            (None, 168_928, true),
            (None, 0, true),
        ] {
            let report = b
                .receive_file(
                    address.clone(),
                    root.join("stage").display().to_string(),
                    root.join("target").display().to_string(),
                    max,
                )
                .unwrap();
            assert_eq!(report.bytes_received, bytes);
            assert_eq!(report.complete, complete);
        }
        b.revoke_peer(a.identity_id()).unwrap();
        assert_eq!(
            b.receive_file(
                address,
                root.join("stage").display().to_string(),
                root.join("target").display().to_string(),
                None
            ),
            Err(PeerCompareError::Unauthorized)
        );
        assert_eq!(
            std::fs::read(root.join("target")).unwrap(),
            vec![23; 300_000]
        );
        a.close();
        b.close();
    }

    #[test]
    fn a_pinned_different_identity_rejects_the_actual_tls_server() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let source = root.join("source");
        std::fs::write(&source, b"synthetic").unwrap();
        let store = Arc::new(Store::default());
        let a = PeerComparisonProbe::new_quinn(
            store.clone(),
            "peer-probe-a".into(),
            "127.0.0.1:0".into(),
            None,
        )
        .unwrap();
        let b = PeerComparisonProbe::new_quinn(
            store,
            "peer-probe-b".into(),
            "127.0.0.1:0".into(),
            None,
        )
        .unwrap();
        a.approve_peer(b.identity_id()).unwrap();
        let false_pin = generate_peer_compare_identity().unwrap().identity_id;
        b.approve_peer(false_pin.clone()).unwrap();
        a.offer_file(source.display().to_string(), 128 * 1024)
            .unwrap();
        let mut address: serde_json::Value =
            serde_json::from_str(&a.address_json().unwrap()).unwrap();
        address["identity_id"] = false_pin.into();
        assert_eq!(
            b.receive_file(
                address.to_string(),
                root.join("stage").display().to_string(),
                root.join("target").display().to_string(),
                None
            ),
            Err(PeerCompareError::Unauthorized)
        );
        assert!(!root.join("target").exists());
        a.close();
        b.close();
    }

    #[test]
    fn failed_keychain_reads_never_create_an_identity() {
        struct Failed;
        impl PeerCompareIdentityStore for Failed {
            fn load(&self, _: String) -> Result<Option<String>, PeerCompareError> {
                Err(PeerCompareError::Storage)
            }
            fn save(&self, _: String, _: String) -> Result<(), PeerCompareError> {
                panic!("must not replace unavailable credentials")
            }
        }
        assert!(matches!(
            PeerComparisonProbe::new_quinn(
                Arc::new(Failed),
                "peer-probe-a".into(),
                "127.0.0.1:0".into(),
                None
            ),
            Err(PeerCompareError::Storage)
        ));
    }
}
