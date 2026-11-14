//! A single-pair, in-memory relay for the explicitly enabled transport experiment.
//! It understands admission tickets, never the inner encrypted peer protocol.

use std::collections::BTreeMap;
use std::io;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::Engine;
use base64::engine::general_purpose::{STANDARD as B64, URL_SAFE_NO_PAD as URL64};
use hmac::{Hmac, KeyInit, Mac};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{OwnedSemaphorePermit, Semaphore, oneshot, watch};
use tokio::task::{JoinHandle, JoinSet};
use tokio::time::{Instant, sleep_until, timeout};
use tokio_rustls::TlsAcceptor;
use tokio_rustls::server::TlsStream;
use zeroize::Zeroizing;

pub const RELAY_SERVER_NAME: &str = "pennel-peer-relay.qa";
pub const MAX_HEADER_BYTES: usize = 16 * 1024;
pub const COPY_BUFFER_BYTES: usize = 64 * 1024;
const MAX_TICKET_BYTES: usize = 4 * 1024;
const MAX_TTL: Duration = Duration::from_secs(2 * 60 * 60);
const READY: &[u8] = b"{\"ready\":true}\n";
const BUSY: &[u8] = b"{\"ready\":false,\"error\":\"busy\"}\n";
const UNAUTHORIZED: &[u8] = b"{\"ready\":false,\"error\":\"unauthorized\"}\n";

#[derive(Clone, Debug)]
pub struct RelayLimits {
    pub max_setup: usize,
    pub handshake_timeout: Duration,
    pub pair_timeout: Duration,
    /// Zero disables pacing; buffering remains bounded in either mode.
    pub rate_bytes_per_second: u64,
}

impl Default for RelayLimits {
    fn default() -> Self {
        Self {
            max_setup: 16,
            handshake_timeout: Duration::from_secs(10),
            pair_timeout: Duration::from_secs(30),
            rate_bytes_per_second: 0,
        }
    }
}

#[derive(Clone, Debug)]
pub struct RelayOptions {
    pub bind: SocketAddr,
    pub advertise_host: Option<String>,
    pub peers: [String; 2],
    pub candidate: String,
    pub run_id: String,
    pub ticket_ttl: Duration,
    pub limits: RelayLimits,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RelayClientConfig {
    pub host: String,
    pub port: u16,
    pub server_name: String,
    pub server_cert_der_b64: String,
    pub ticket: String,
}

#[derive(Serialize)]
pub struct RelayReady {
    pub event: &'static str,
    pub host: String,
    pub port: u16,
    pub server_name: &'static str,
    pub server_cert_der_b64: String,
    pub candidate: String,
    pub run_id: String,
    pub expires_unix: u64,
    pub max_setup: usize,
    pub max_clients: usize,
    pub buffer_bytes_per_direction: usize,
    pub rate_bytes_per_second: u64,
    pub payload_storage: bool,
    pub clients: BTreeMap<String, RelayClientConfig>,
}

#[derive(Clone, Copy, Debug, Default, Serialize)]
pub struct RelayStats {
    pub paused: bool,
    pub setup_connections: usize,
    pub peak_setup_connections: usize,
    pub waiting_connections: usize,
    pub active_pairs: usize,
    pub rejected_connections: u64,
    pub forwarded_bytes: u64,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Claims {
    version: u8,
    pair_id: String,
    candidate: String,
    subject: String,
    recipient: String,
    role: u8,
    expires_unix: u64,
    nonce: String,
}

struct Authority {
    key: Zeroizing<[u8; 32]>,
    claims: [Claims; 2],
}

impl Authority {
    fn new(options: &RelayOptions) -> io::Result<Self> {
        let mut key = Zeroizing::new([0; 32]);
        getrandom::fill(key.as_mut()).map_err(|_| error("relay_random_unavailable"))?;
        let expires_unix = unix_now()?
            .checked_add(options.ticket_ttl.as_secs())
            .ok_or_else(|| error("relay_invalid_expiry"))?;
        let mut claims = Vec::new();
        for role in 0..2 {
            let mut nonce = [0; 24];
            getrandom::fill(&mut nonce).map_err(|_| error("relay_random_unavailable"))?;
            claims.push(Claims {
                version: 1,
                pair_id: options.run_id.clone(),
                candidate: options.candidate.clone(),
                subject: options.peers[role].clone(),
                recipient: options.peers[1 - role].clone(),
                role: role as u8,
                expires_unix,
                nonce: URL64.encode(nonce),
            });
        }
        Ok(Self {
            key,
            claims: claims
                .try_into()
                .map_err(|_| error("relay_invalid_peers"))?,
        })
    }

    fn issue(&self, role: usize) -> io::Result<String> {
        let bytes = serde_json::to_vec(&self.claims[role])
            .map_err(|_| error("relay_ticket_encoding_failed"))?;
        let mut mac = Hmac::<Sha256>::new_from_slice(self.key.as_ref())
            .map_err(|_| error("relay_ticket_encoding_failed"))?;
        mac.update(&bytes);
        Ok(format!(
            "{}.{}",
            URL64.encode(&bytes),
            URL64.encode(mac.finalize().into_bytes())
        ))
    }

    fn verify(&self, ticket: &str) -> Option<usize> {
        if ticket.len() > MAX_TICKET_BYTES {
            return None;
        }
        let (encoded, signature) = ticket.split_once('.')?;
        let bytes = URL64.decode(encoded).ok()?;
        let signature = URL64.decode(signature).ok()?;
        if bytes.len() > MAX_TICKET_BYTES || signature.len() != 32 {
            return None;
        }
        let mut mac = Hmac::<Sha256>::new_from_slice(self.key.as_ref()).ok()?;
        mac.update(&bytes);
        mac.verify_slice(&signature).ok()?;
        let claims: Claims = serde_json::from_slice(&bytes).ok()?;
        let role = usize::from(claims.role);
        let expected = self.claims.get(role)?;
        (claims == *expected && claims.expires_unix > unix_now().ok()?).then_some(role)
    }
}

struct Waiting {
    role: usize,
    generation: u64,
    matched: oneshot::Sender<MatchedPeer>,
}

struct MatchedPeer {
    stream: Box<TlsStream<TcpStream>>,
    _guard: PairGuard,
}

#[derive(Default)]
struct PairState {
    waiting: Option<Waiting>,
    active: bool,
    generation: u64,
}

struct Shared {
    authority: Authority,
    limits: RelayLimits,
    expires: Instant,
    pair: Mutex<PairState>,
    revoked: AtomicBool,
    paused: watch::Sender<bool>,
    cancel_sessions: watch::Sender<bool>,
    shutdown: watch::Sender<bool>,
    setup: AtomicUsize,
    peak_setup: AtomicUsize,
    rejected: AtomicU64,
    forwarded: AtomicU64,
}

impl Shared {
    fn sessions_unavailable(&self) -> bool {
        self.revoked.load(Ordering::Acquire)
            || *self.shutdown.borrow()
            || Instant::now() >= self.expires
    }

    fn stats(&self) -> RelayStats {
        let pair = self
            .pair
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        RelayStats {
            paused: *self.paused.borrow(),
            setup_connections: self.setup.load(Ordering::Relaxed),
            peak_setup_connections: self.peak_setup.load(Ordering::Relaxed),
            waiting_connections: usize::from(pair.waiting.is_some()),
            active_pairs: usize::from(pair.active),
            rejected_connections: self.rejected.load(Ordering::Relaxed),
            forwarded_bytes: self.forwarded.load(Ordering::Relaxed),
        }
    }
}

struct SetupGuard {
    shared: Arc<Shared>,
    _permit: OwnedSemaphorePermit,
}

impl SetupGuard {
    fn new(shared: Arc<Shared>, permit: OwnedSemaphorePermit) -> Self {
        let current = shared.setup.fetch_add(1, Ordering::Relaxed) + 1;
        shared.peak_setup.fetch_max(current, Ordering::Relaxed);
        Self {
            shared,
            _permit: permit,
        }
    }
}

impl Drop for SetupGuard {
    fn drop(&mut self) {
        self.shared.setup.fetch_sub(1, Ordering::Relaxed);
    }
}

struct PairGuard(Arc<Shared>);

impl Drop for PairGuard {
    fn drop(&mut self) {
        self.0
            .pair
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .active = false;
    }
}

pub struct Relay {
    addr: SocketAddr,
    host: String,
    certificate: Vec<u8>,
    options: RelayOptions,
    shared: Arc<Shared>,
    task: Mutex<Option<JoinHandle<()>>>,
}

impl Relay {
    pub async fn bind(options: RelayOptions) -> io::Result<Self> {
        validate(&options)?;
        let (certificate, config) = tls_config(&options)?;
        let listener = TcpListener::bind(options.bind).await?;
        let addr = listener.local_addr()?;
        let host = options
            .advertise_host
            .clone()
            .unwrap_or_else(|| addr.ip().to_string());
        let (shutdown, _) = watch::channel(false);
        let (cancel_sessions, _) = watch::channel(false);
        let (paused, _) = watch::channel(false);
        let authority = Authority::new(&options)?;
        let expires_at = UNIX_EPOCH + Duration::from_secs(authority.claims[0].expires_unix);
        let remaining = expires_at
            .duration_since(SystemTime::now())
            .map_err(|_| error("relay_invalid_expiry"))?;
        let shared = Arc::new(Shared {
            authority,
            limits: options.limits.clone(),
            expires: Instant::now() + remaining,
            pair: Mutex::new(PairState::default()),
            revoked: AtomicBool::new(false),
            paused,
            cancel_sessions,
            shutdown,
            setup: AtomicUsize::new(0),
            peak_setup: AtomicUsize::new(0),
            rejected: AtomicU64::new(0),
            forwarded: AtomicU64::new(0),
        });
        let task = tokio::spawn(serve(
            listener,
            TlsAcceptor::from(Arc::new(config)),
            shared.clone(),
        ));
        Ok(Self {
            addr,
            host,
            certificate,
            options,
            shared,
            task: Mutex::new(Some(task)),
        })
    }

    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    pub fn certificate_der(&self) -> &[u8] {
        &self.certificate
    }

    pub fn client_config(&self, peer: &str) -> io::Result<RelayClientConfig> {
        let role = self
            .options
            .peers
            .iter()
            .position(|id| id == peer)
            .ok_or_else(|| error("relay_unknown_peer"))?;
        Ok(RelayClientConfig {
            host: self.host.clone(),
            port: self.addr.port(),
            server_name: RELAY_SERVER_NAME.into(),
            server_cert_der_b64: B64.encode(&self.certificate),
            ticket: self.shared.authority.issue(role)?,
        })
    }

    /// Contains bearer admission tickets. Persist only in the caller's private QA control file.
    pub fn ready(&self) -> io::Result<RelayReady> {
        let clients = self
            .options
            .peers
            .iter()
            .map(|peer| Ok((peer.clone(), self.client_config(peer)?)))
            .collect::<io::Result<BTreeMap<_, _>>>()?;
        Ok(RelayReady {
            event: "ready",
            host: self.host.clone(),
            port: self.addr.port(),
            server_name: RELAY_SERVER_NAME,
            server_cert_der_b64: B64.encode(&self.certificate),
            candidate: self.options.candidate.clone(),
            run_id: self.options.run_id.clone(),
            expires_unix: self.shared.authority.claims[0].expires_unix,
            max_setup: self.options.limits.max_setup,
            max_clients: 2,
            buffer_bytes_per_direction: COPY_BUFFER_BYTES,
            rate_bytes_per_second: self.options.limits.rate_bytes_per_second,
            payload_storage: false,
            clients,
        })
    }

    pub fn stats(&self) -> RelayStats {
        self.shared.stats()
    }

    /// Gates future bounded writes. Bytes already accepted by the socket may still drain.
    pub fn pause(&self) {
        self.shared.paused.send_replace(true);
    }

    pub fn resume(&self) {
        self.shared.paused.send_replace(false);
    }

    pub fn revoke(&self) {
        self.shared.revoked.store(true, Ordering::Release);
        self.shared.cancel_sessions.send_replace(true);
    }

    pub async fn shutdown(&self) {
        self.shared.shutdown.send_replace(true);
        self.shared.cancel_sessions.send_replace(true);
        let task = self
            .task
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .take();
        if let Some(task) = task {
            let _ = task.await;
        }
    }
}

impl Drop for Relay {
    fn drop(&mut self) {
        self.shared.shutdown.send_replace(true);
        self.shared.cancel_sessions.send_replace(true);
        if let Some(task) = self
            .task
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .take()
        {
            task.abort();
        }
    }
}

fn validate(options: &RelayOptions) -> io::Result<()> {
    let valid_id = |id: &str| {
        id.len() == 64
            && id
                .bytes()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
    };
    if options.peers[0] == options.peers[1]
        || !options.peers.iter().all(|id| valid_id(id))
        || !matches!(options.candidate.as_str(), "quinn" | "native")
        || options.run_id.is_empty()
        || options.run_id.len() > 80
        || !options
            .run_id
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_'))
        || options.ticket_ttl.as_secs() == 0
        || options.ticket_ttl > MAX_TTL
        || options.limits.max_setup == 0
        || options.limits.max_setup > 16
        || options.limits.handshake_timeout.is_zero()
        || options.limits.handshake_timeout > Duration::from_secs(10)
        || options.limits.pair_timeout.is_zero()
        || options.limits.pair_timeout > Duration::from_secs(30)
        || (options.bind.ip().is_unspecified() && options.advertise_host.is_none())
        || options.advertise_host.as_ref().is_some_and(|host| {
            host.is_empty()
                || host.len() > 253
                || host
                    .chars()
                    .any(|c| c.is_whitespace() || c.is_control() || matches!(c, '/' | '\\' | '@'))
        })
    {
        return Err(error("relay_invalid_configuration"));
    }
    Ok(())
}

fn tls_config(options: &RelayOptions) -> io::Result<(Vec<u8>, rustls::ServerConfig)> {
    let key = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256)
        .map_err(|_| error("relay_identity_failed"))?;
    let mut names = vec![
        RELAY_SERVER_NAME.to_owned(),
        "localhost".to_owned(),
        "127.0.0.1".to_owned(),
    ];
    if !options.bind.ip().is_unspecified() {
        names.push(options.bind.ip().to_string());
    }
    if let Some(host) = &options.advertise_host {
        names.push(host.clone());
    }
    let mut parameters =
        rcgen::CertificateParams::new(names).map_err(|_| error("relay_identity_failed"))?;
    parameters.extended_key_usages = vec![rcgen::ExtendedKeyUsagePurpose::ServerAuth];
    parameters.not_before = (SystemTime::now() - Duration::from_secs(300)).into();
    parameters.not_after = (SystemTime::now() + Duration::from_secs(24 * 60 * 60)).into();
    let certificate = parameters
        .self_signed(&key)
        .map_err(|_| error("relay_identity_failed"))?
        .der()
        .to_vec();
    let private_key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key.serialize_der()));
    let mut config = rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_protocol_versions(&[&rustls::version::TLS13])
    .map_err(|_| error("relay_tls_failed"))?
    .with_no_client_auth()
    .with_single_cert(vec![CertificateDer::from(certificate.clone())], private_key)
    .map_err(|_| error("relay_tls_failed"))?;
    config.max_early_data_size = 0;
    config.send_tls13_tickets = 0;
    config.session_storage = Arc::new(rustls::server::NoServerSessionStorage {});
    Ok((certificate, config))
}

async fn serve(listener: TcpListener, acceptor: TlsAcceptor, shared: Arc<Shared>) {
    let semaphore = Arc::new(Semaphore::new(shared.limits.max_setup));
    let mut stop = shared.shutdown.subscribe();
    let mut tasks = JoinSet::new();
    loop {
        tokio::select! {
            biased;
            _ = cancelled(&mut stop) => break,
            _ = tasks.join_next(), if !tasks.is_empty() => {},
            accepted = listener.accept() => {
                let Ok((socket, _)) = accepted else { break; };
                // The permit is acquired before allocating a TLS handshake.
                let Ok(permit) = semaphore.clone().try_acquire_owned() else {
                    shared.rejected.fetch_add(1, Ordering::Relaxed);
                    drop(socket);
                    continue;
                };
                let guard = SetupGuard::new(shared.clone(), permit);
                tasks.spawn(handle(socket, acceptor.clone(), shared.clone(), guard));
            }
        }
    }
    tasks.abort_all();
    while tasks.join_next().await.is_some() {}
    let mut pair = shared
        .pair
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    pair.waiting.take();
    pair.active = false;
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Admission {
    ticket: String,
}

async fn read_admission(stream: &mut TlsStream<TcpStream>) -> io::Result<String> {
    let mut bytes = Vec::with_capacity(256);
    loop {
        let byte = stream.read_u8().await?;
        if bytes.len() >= MAX_HEADER_BYTES {
            return Err(error("relay_header_too_large"));
        }
        bytes.push(byte);
        if byte == b'\n' {
            break;
        }
    }
    let request: Admission =
        serde_json::from_slice(&bytes).map_err(|_| error("relay_invalid_admission"))?;
    Ok(request.ticket)
}

enum AdmissionOutcome {
    Rejected(Box<TlsStream<TcpStream>>, &'static [u8]),
    Waiting(
        Box<TlsStream<TcpStream>>,
        oneshot::Receiver<MatchedPeer>,
        u64,
    ),
    Matched(Box<TlsStream<TcpStream>>, oneshot::Sender<MatchedPeer>),
}

async fn handle(socket: TcpStream, acceptor: TlsAcceptor, shared: Arc<Shared>, guard: SetupGuard) {
    let admission = timeout(shared.limits.handshake_timeout, async {
        let mut stream = acceptor.accept(socket).await?;
        let ticket = read_admission(&mut stream).await;
        Ok::<_, io::Error>((Box::new(stream), ticket))
    })
    .await;
    let Ok(Ok((mut stream, ticket))) = admission else {
        shared.rejected.fetch_add(1, Ordering::Relaxed);
        return;
    };
    let role = ticket
        .ok()
        .and_then(|ticket| shared.authority.verify(&ticket));
    let Some(role) = role.filter(|_| !shared.sessions_unavailable()) else {
        shared.rejected.fetch_add(1, Ordering::Relaxed);
        let _ = timeout(
            shared.limits.handshake_timeout,
            stream.write_all(UNAUTHORIZED),
        )
        .await;
        return;
    };
    let outcome = {
        let mut pair = shared
            .pair
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if shared.sessions_unavailable() {
            AdmissionOutcome::Rejected(stream, UNAUTHORIZED)
        } else if pair.active
            || pair
                .waiting
                .as_ref()
                .is_some_and(|waiting| waiting.role == role)
        {
            AdmissionOutcome::Rejected(stream, BUSY)
        } else if let Some(waiting) = pair.waiting.take() {
            pair.active = true;
            AdmissionOutcome::Matched(stream, waiting.matched)
        } else {
            pair.generation = pair.generation.wrapping_add(1);
            let generation = pair.generation;
            let (matched, receiver) = oneshot::channel();
            pair.waiting = Some(Waiting {
                role,
                generation,
                matched,
            });
            AdmissionOutcome::Waiting(stream, receiver, generation)
        }
    };
    match outcome {
        AdmissionOutcome::Rejected(mut stream, reply) => {
            shared.rejected.fetch_add(1, Ordering::Relaxed);
            let _ = timeout(shared.limits.handshake_timeout, stream.write_all(reply)).await;
        }
        AdmissionOutcome::Waiting(mut stream, receiver, generation) => {
            // Authenticated waiting has its own single slot; it is no longer a TLS setup.
            drop(guard);
            let mut cancel = shared.cancel_sessions.subscribe();
            let peer = tokio::select! {
                biased;
                _ = cancelled(&mut cancel) => None,
                _ = sleep_until(shared.expires) => None,
                // Clients must await ready before sending their inner protocol. This also
                // observes EOF immediately instead of pinning a disconnected waiting role.
                _ = stream.read_u8() => None,
                result = timeout(shared.limits.pair_timeout, receiver) => result.ok().and_then(Result::ok),
            };
            {
                let mut pair = shared
                    .pair
                    .lock()
                    .unwrap_or_else(|poison| poison.into_inner());
                if pair
                    .waiting
                    .as_ref()
                    .is_some_and(|waiting| waiting.generation == generation)
                {
                    pair.waiting.take();
                }
            }
            if let Some(peer) = peer {
                run_pair(stream, peer.stream, shared.clone()).await;
            }
        }
        AdmissionOutcome::Matched(stream, matched) => {
            drop(guard);
            let _ = matched.send(MatchedPeer {
                stream,
                _guard: PairGuard(shared.clone()),
            });
        }
    }
}

async fn run_pair(
    mut left: Box<TlsStream<TcpStream>>,
    mut right: Box<TlsStream<TcpStream>>,
    shared: Arc<Shared>,
) {
    let mut cancel = shared.cancel_sessions.subscribe();
    let ready = tokio::select! {
        biased;
        _ = cancelled(&mut cancel) => return,
        _ = sleep_until(shared.expires) => return,
        result = timeout(shared.limits.handshake_timeout, async {
            left.write_all(READY).await?;
            left.flush().await?;
            right.write_all(READY).await?;
            right.flush().await
        }) => result,
    };
    if !matches!(ready, Ok(Ok(()))) || shared.sessions_unavailable() {
        return;
    }
    let (left_read, left_write) = tokio::io::split(left);
    let (right_read, right_write) = tokio::io::split(right);
    tokio::select! {
        biased;
        _ = cancelled(&mut cancel) => {},
        _ = sleep_until(shared.expires) => {},
        _ = pump(left_read, right_write, shared.clone()) => {},
        _ = pump(right_read, left_write, shared.clone()) => {},
    }
    // The qualification protocol exchanges its final receipt before closing. Ending
    // either direction closes the pair; this relay does not support half-close/offline queues.
}

async fn pump<R, W>(mut reader: R, mut writer: W, shared: Arc<Shared>) -> io::Result<()>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let mut buffer = vec![0; COPY_BUFFER_BYTES];
    let mut next_write = Instant::now();
    let mut paused = shared.paused.subscribe();
    loop {
        if shared.sessions_unavailable() {
            return Err(error("relay_session_closed"));
        }
        let length = reader.read(&mut buffer).await?;
        if length == 0 {
            return Ok(());
        }
        let mut offset = 0;
        while offset < length {
            sleep_until(next_write).await;
            wait_unpaused(&shared, &mut paused).await?;
            let wrote = tokio::select! {
                biased;
                changed = paused.changed() => {
                    changed.map_err(|_| error("relay_session_closed"))?;
                    continue;
                },
                result = writer.write(&buffer[offset..length]) => result?,
            };
            if wrote == 0 {
                return Err(error("relay_write_zero"));
            }
            offset += wrote;
            shared.forwarded.fetch_add(wrote as u64, Ordering::Relaxed);
            if shared.limits.rate_bytes_per_second != 0 {
                next_write = Instant::now()
                    + Duration::from_secs_f64(
                        wrote as f64 / shared.limits.rate_bytes_per_second as f64,
                    );
            }
        }
        loop {
            wait_unpaused(&shared, &mut paused).await?;
            tokio::select! {
                biased;
                changed = paused.changed() => changed.map_err(|_| error("relay_session_closed"))?,
                result = writer.flush() => { result?; break; },
            }
        }
    }
}

async fn wait_unpaused(shared: &Shared, paused: &mut watch::Receiver<bool>) -> io::Result<()> {
    let mut cancel = shared.cancel_sessions.subscribe();
    loop {
        if shared.sessions_unavailable() {
            return Err(error("relay_session_closed"));
        }
        if !*paused.borrow_and_update() {
            return Ok(());
        }
        tokio::select! {
            biased;
            _ = cancelled(&mut cancel) => return Err(error("relay_session_closed")),
            _ = sleep_until(shared.expires) => return Err(error("relay_session_closed")),
            changed = paused.changed() => changed.map_err(|_| error("relay_session_closed"))?,
        }
    }
}

async fn cancelled(receiver: &mut watch::Receiver<bool>) {
    loop {
        if *receiver.borrow_and_update() {
            return;
        }
        if receiver.changed().await.is_err() {
            return;
        }
    }
}

fn unix_now() -> io::Result<u64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .map_err(|_| error("relay_invalid_clock"))
}

fn error(code: &str) -> io::Error {
    io::Error::other(code)
}
