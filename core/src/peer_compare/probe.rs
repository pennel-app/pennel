use std::{
    collections::HashMap,
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

#[derive(Default)]
struct PeerApprovals {
    peers: Mutex<HashMap<String, Cancellation>>,
    stopped: Cancellation,
}

impl PeerApprovals {
    fn for_peer(&self, id: &str) -> Result<Cancellation, PeerCompareError> {
        self.stopped.check()?;
        self.peers
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(id)
            .cloned()
            .ok_or(PeerCompareError::Unauthorized)
    }

    fn approve(
        &self,
        id: &str,
        update_transport: impl FnOnce() -> Result<bool, PeerCompareError>,
    ) -> Result<bool, PeerCompareError> {
        let mut peers = self.peers.lock().unwrap_or_else(|p| p.into_inner());
        self.stopped.check()?;
        if !peers.contains_key(id) && peers.len() >= 16 {
            return Err(PeerCompareError::LimitExceeded);
        }
        let added = update_transport()?;
        peers.entry(id.to_owned()).or_default();
        Ok(added)
    }

    fn revoke(
        &self,
        id: &str,
        update_transport: impl FnOnce() -> Result<bool, PeerCompareError>,
    ) -> Result<bool, PeerCompareError> {
        let mut peers = self.peers.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(approval) = peers.remove(id) {
            // Wait for any disk operation already holding this peer's approval.
            approval.cancel();
        }
        // Re-approval must wait until the transport has retired the old channels.
        update_transport()
    }

    fn stop(&self) {
        self.stopped.cancel();
        let peers = self.peers.lock().unwrap_or_else(|p| p.into_inner());
        for approval in peers.values() {
            approval.cancel();
        }
    }
}

enum Transport {
    Quinn(Arc<QuinnTransport>),
    Native(Arc<dyn PeerCompareNativeTransport>),
}
impl Transport {
    fn id(&self) -> String {
        match self {
            Self::Quinn(t) => t.id(),
            Self::Native(t) => t.identity_id(),
        }
    }
    fn close(&self) {
        match self {
            Self::Quinn(t) => t.close(),
            Self::Native(t) => t.close(),
        }
    }
    fn address(&self) -> Result<String, PeerCompareError> {
        match self {
            Self::Quinn(t) => t.address_json(),
            Self::Native(t) => t.address_json(),
        }
    }
    fn approve(&self, id: &str) -> Result<bool, PeerCompareError> {
        match self {
            Self::Quinn(t) => t.approve(id),
            Self::Native(t) => t.approve_peer(id.to_owned()),
        }
    }
    fn revoke(&self, id: &str) -> Result<bool, PeerCompareError> {
        match self {
            Self::Quinn(t) => t.revoke(id),
            Self::Native(t) => t.revoke_peer(id.to_owned()),
        }
    }
    async fn accept(
        &self,
        approvals: &Arc<PeerApprovals>,
    ) -> Result<(Channel, Cancellation), PeerCompareError> {
        match self {
            Self::Quinn(t) => t.accept(|id| approvals.for_peer(id)).await,
            Self::Native(t) => {
                let transport = t.clone();
                let approvals = approvals.clone();
                tokio::task::spawn_blocking(move || {
                    let channel = transport.accept()?;
                    match approvals.for_peer(&channel.peer_id()) {
                        Ok(approval) => Ok((native::stream(channel), approval)),
                        Err(error) => {
                            channel.close();
                            Err(error)
                        }
                    }
                })
                .await
                .map_err(|_| PeerCompareError::Transport)?
            }
        }
    }
    async fn connect(
        &self,
        address: String,
        expected: String,
        approval: Cancellation,
    ) -> Result<Channel, PeerCompareError> {
        match self {
            Self::Quinn(t) => t.connect(&address).await,
            Self::Native(t) => {
                let transport = t.clone();
                tokio::task::spawn_blocking(move || {
                    let channel = transport.connect(address, expected.clone())?;
                    if channel.peer_id() != expected {
                        channel.close();
                        return Err(PeerCompareError::Unauthorized);
                    }
                    if let Err(error) = approval.check() {
                        channel.close();
                        return Err(error);
                    }
                    Ok(native::stream(channel))
                })
                .await
                .map_err(|_| PeerCompareError::Transport)?
            }
        }
    }
}

/// Explicit fixture-only QA facade; it never opens a meeting library or shipping sync.
#[derive(uniffi::Object)]
pub struct PeerComparisonProbe {
    runtime: Option<Runtime>,
    transport: Arc<Transport>,
    approvals: Arc<PeerApprovals>,
    closed: AtomicBool,
    offer: Mutex<Option<JoinHandle<()>>>,
    offered: Arc<Mutex<Option<(TransferManifest, PathBuf)>>>,
    operation: Mutex<()>,
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
            Transport::Native(transport),
        ))
    }

    pub fn identity_id(&self) -> String {
        self.transport.id()
    }
    pub fn candidate(&self) -> String {
        match self.transport.as_ref() {
            Transport::Quinn(_) => "quinn",
            Transport::Native(_) => "native",
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
        self.approvals.approve(&id, || self.transport.approve(&id))
    }
    pub fn revoke_peer(&self, id: String) -> Result<bool, PeerCompareError> {
        self.ensure_open()?;
        self.approvals.revoke(&id, || self.transport.revoke(&id))
    }

    pub fn offer_file(&self, path: String, block_size: u32) -> Result<String, PeerCompareError> {
        let _operation = self
            .operation
            .try_lock()
            .map_err(|_| PeerCompareError::InvalidConfig)?;
        self.ensure_open()?;
        let path = fixture_path(path)?;
        let reading = path.clone();
        let cancel = self.approvals.stopped.clone();
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
        let approvals = self.approvals.clone();
        *offered = Some(self.runtime.as_ref().ok_or(PeerCompareError::Closed)?.spawn(async move {
            loop {
                let channel = tokio::select! { biased; _ = approvals.stopped.wait() => break, channel = transport.accept(&approvals) => channel };
                match channel {
                    Ok((channel, approval)) => {
                        let Some((manifest, path)) = current.lock().unwrap_or_else(|p| p.into_inner()).clone() else { break; };
                        let _ = wire::send(channel, manifest, path, approval).await;
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
        let approval = self.approvals.for_peer(&address.identity_id)?;
        let staging = fixture_path(staging)?;
        let target = fixture_path(target)?;
        let transport = self.transport.clone();
        self.run(async move {
            let channel = tokio::select! {
                biased;
                _ = approval.wait() => return Err(PeerCompareError::Cancelled),
                channel = transport.connect(address_json, address.identity_id, approval.clone()) => channel?,
            };
            wire::receive(channel, staging, target, max_blocks, approval).await
        })
    }

    /// Cancellation ends this isolated probe and unblocks native reads and accepts.
    pub fn cancel(&self) {
        self.approvals.stop();
        self.transport.close();
    }
    pub fn close(&self) {
        self.closed.store(true, Ordering::Release);
        self.cancel();
        if let Some(task) = self.offer.lock().unwrap_or_else(|p| p.into_inner()).take() {
            task.abort();
        }
    }
}

impl PeerComparisonProbe {
    fn from_transport(runtime: Runtime, transport: Transport) -> Arc<Self> {
        Arc::new(Self {
            runtime: Some(runtime),
            transport: Arc::new(transport),
            approvals: Arc::default(),
            closed: AtomicBool::new(false),
            offer: Mutex::new(None),
            offered: Arc::new(Mutex::new(None)),
            operation: Mutex::new(()),
        })
    }
    fn ensure_open(&self) -> Result<(), PeerCompareError> {
        if self.closed.load(Ordering::Acquire) {
            return Err(PeerCompareError::Closed);
        }
        self.approvals.stopped.check()
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
    use super::super::{PartialTransfer, PeerCompareNativeChannel};
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream, ReadHalf, WriteHalf};

    struct Checkpoint {
        first_block: PathBuf,
        reached: mpsc::Sender<()>,
        resume: mpsc::Receiver<()>,
    }

    struct MemoryChannel {
        peer: String,
        reader: Mutex<ReadHalf<DuplexStream>>,
        writer: Mutex<WriteHalf<DuplexStream>>,
        checkpoint: Mutex<Option<Checkpoint>>,
        closed: Cancellation,
    }

    impl PeerCompareNativeChannel for MemoryChannel {
        fn peer_id(&self) -> String {
            self.peer.clone()
        }
        fn read(&self, max: u32) -> Result<Vec<u8>, PeerCompareError> {
            let mut bytes = vec![0; max as usize];
            let count = tokio::runtime::Handle::current()
                .block_on(self.reader.lock().unwrap().read(&mut bytes))
                .map_err(|_| PeerCompareError::Transport)?;
            bytes.truncate(count);
            Ok(bytes)
        }
        fn write(&self, bytes: Vec<u8>) -> Result<u32, PeerCompareError> {
            let checkpoint = self
                .checkpoint
                .lock()
                .unwrap()
                .take_if(|p| p.first_block.is_file());
            if let Some(checkpoint) = checkpoint {
                checkpoint.reached.send(()).unwrap();
                checkpoint
                    .resume
                    .recv_timeout(std::time::Duration::from_secs(10))
                    .unwrap();
            }
            tokio::runtime::Handle::current()
                .block_on(self.writer.lock().unwrap().write_all(&bytes))
                .map_err(|_| PeerCompareError::Transport)?;
            Ok(bytes.len() as u32)
        }
        fn finish_write(&self) -> Result<(), PeerCompareError> {
            tokio::runtime::Handle::current()
                .block_on(self.writer.lock().unwrap().shutdown())
                .map_err(|_| PeerCompareError::Transport)
        }
        fn close(&self) {
            self.closed.cancel();
        }
    }

    struct MemoryTransport {
        source: PathBuf,
        manifest: TransferManifest,
        checkpoint: Mutex<Option<Checkpoint>>,
        approved: Mutex<std::collections::HashSet<String>>,
    }

    fn fixture_address(id: String) -> String {
        serde_json::to_string(&Address {
            identity_id: id,
            host: "127.0.0.1".into(),
            port: 1,
            route: "direct".into(),
        })
        .unwrap()
    }

    impl PeerCompareNativeTransport for MemoryTransport {
        fn identity_id(&self) -> String {
            "a".repeat(64)
        }
        fn address_json(&self) -> Result<String, PeerCompareError> {
            Ok(fixture_address(self.identity_id()))
        }
        fn connect(
            &self,
            _: String,
            peer: String,
        ) -> Result<Arc<dyn PeerCompareNativeChannel>, PeerCompareError> {
            if !self.approved.lock().unwrap().contains(&peer) {
                return Err(PeerCompareError::Unauthorized);
            }
            let (sender, receiver) = tokio::io::duplex(64 * 1024);
            let (reader, writer) = tokio::io::split(receiver);
            let closed = Cancellation::default();
            tokio::spawn(wire::send(
                Box::new(sender),
                self.manifest.clone(),
                self.source.clone(),
                closed.clone(),
            ));
            Ok(Arc::new(MemoryChannel {
                peer,
                reader: Mutex::new(reader),
                writer: Mutex::new(writer),
                checkpoint: Mutex::new(self.checkpoint.lock().unwrap().take()),
                closed,
            }))
        }
        fn accept(&self) -> Result<Arc<dyn PeerCompareNativeChannel>, PeerCompareError> {
            Err(PeerCompareError::Closed)
        }
        fn approve_peer(&self, id: String) -> Result<bool, PeerCompareError> {
            Ok(self.approved.lock().unwrap().insert(id))
        }
        fn revoke_peer(&self, id: String) -> Result<bool, PeerCompareError> {
            // Admission ends immediately; the test controls when in-flight native I/O resumes.
            Ok(self.approved.lock().unwrap().remove(&id))
        }
        fn close(&self) {}
    }

    #[test]
    fn receive_file_revokes_at_a_verified_checkpoint_and_keeps_other_peers_usable() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let source = root.join("source");
        let stage = root.join("stage");
        let target = root.join("target");
        let block_size = 128 * 1024;
        let bytes = vec![0x31; block_size as usize + 17];
        std::fs::write(&source, &bytes).unwrap();
        let manifest = TransferManifest::from_path(&source, block_size).unwrap();
        let (reached, checkpoint) = mpsc::channel();
        let (resume, paused) = mpsc::channel();
        let probe = PeerComparisonProbe::new_native(Arc::new(MemoryTransport {
            source,
            manifest: manifest.clone(),
            approved: Mutex::default(),
            checkpoint: Mutex::new(Some(Checkpoint {
                first_block: stage.join("block-00000000.part"),
                reached,
                resume: paused,
            })),
        }))
        .unwrap();
        let first = "b".repeat(64);
        let other = "c".repeat(64);
        probe.approve_peer(first.clone()).unwrap();
        probe.approve_peer(other.clone()).unwrap();

        std::thread::scope(|threads| {
            let receiving = threads.spawn(|| {
                probe.receive_file(
                    fixture_address(first.clone()),
                    stage.display().to_string(),
                    target.display().to_string(),
                    None,
                )
            });
            checkpoint
                .recv_timeout(std::time::Duration::from_secs(10))
                .unwrap();
            assert!(probe.revoke_peer(first.clone()).unwrap());
            assert!(probe.approve_peer(first.clone()).unwrap());
            resume.send(()).unwrap();
            assert_eq!(receiving.join().unwrap(), Err(PeerCompareError::Cancelled));
        });
        assert!(!target.exists());
        assert_eq!(
            PartialTransfer::open(manifest, &stage)
                .unwrap()
                .needed_blocks(),
            [1]
        );

        let received = probe
            .receive_file(
                fixture_address(other),
                stage.display().to_string(),
                target.display().to_string(),
                None,
            )
            .unwrap();
        assert!(received.complete);
        assert_eq!(received.bytes_received, 17);
        assert_eq!(std::fs::read(target).unwrap(), bytes);
        probe.close();
    }

    #[tokio::test]
    async fn revocation_fences_prepared_publication_across_reapproval() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let source = root.join("source");
        let target = root.join("target");
        let bytes = b"synthetic publication fixture";
        std::fs::write(&source, bytes).unwrap();
        let manifest = TransferManifest::from_path(&source, 128 * 1024).unwrap();
        let mut partial = PartialTransfer::open(manifest, &root.join("stage")).unwrap();
        partial.write_block(0, bytes).unwrap();

        let approvals = PeerApprovals::default();
        approvals.approve("first", || Ok(true)).unwrap();
        approvals.approve("second", || Ok(true)).unwrap();
        let in_flight = approvals.for_peer("first").unwrap();
        let unaffected = approvals.for_peer("second").unwrap();
        assert!(!approvals.approve("first", || Ok(false)).unwrap());
        approvals.revoke("first", || Ok(true)).unwrap();
        assert!(matches!(
            approvals.for_peer("first"),
            Err(PeerCompareError::Unauthorized)
        ));
        approvals.approve("first", || Ok(true)).unwrap();
        let fresh = approvals.for_peer("first").unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(1), in_flight.wait())
            .await
            .expect("revocation must release a waiting transfer");
        assert_eq!(
            in_flight.with_active(|| partial.finish(&target)),
            Err(PeerCompareError::Cancelled)
        );
        assert!(!target.exists());
        assert!(unaffected.check().is_ok());
        fresh.with_active(|| partial.finish(&target)).unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), bytes);

        approvals.stop();
        assert_eq!(fresh.check(), Err(PeerCompareError::Cancelled));
        assert_eq!(unaffected.check(), Err(PeerCompareError::Cancelled));
        assert_eq!(
            approvals.approve("third", || Ok(true)),
            Err(PeerCompareError::Cancelled)
        );
    }

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
