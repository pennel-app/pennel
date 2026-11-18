use std::{
    fmt,
    io::{self, IoSliceMut},
    net::{Ipv4Addr, SocketAddr},
    pin::Pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    task::{Context, Poll},
};

use base64::{Engine, engine::general_purpose::STANDARD as B64};
use quinn::{
    AsyncUdpSocket, UdpPoller,
    udp::{RecvMeta, Transmit},
};
use rustls::pki_types::{CertificateDer, ServerName};
use serde::Deserialize;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
    sync::{Notify, mpsc},
};

use super::{PeerCompareError, STEP_TIMEOUT, identity::provider};

const MTU: usize = 1200;
const PACKETS: usize = 32;

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RelayConfig {
    pub(crate) host: String,
    pub(crate) port: u16,
    pub(crate) server_cert_der_b64: String,
    pub(crate) ticket: String,
    #[serde(default = "server_name")]
    pub(crate) server_name: String,
}
fn server_name() -> String {
    "pennel-peer-relay.qa".into()
}

impl RelayConfig {
    pub(crate) fn parse(json: &str) -> Result<Self, PeerCompareError> {
        if json.len() > 24 * 1024 {
            return Err(PeerCompareError::LimitExceeded);
        }
        let config: Self =
            serde_json::from_str(json).map_err(|_| PeerCompareError::InvalidConfig)?;
        if config.host.is_empty()
            || config.host.len() > 253
            || config.port == 0
            || config.server_name != server_name()
            || config.ticket.is_empty()
            || config.ticket.len() > 12 * 1024
            || config.server_cert_der_b64.len() > 8192
        {
            return Err(PeerCompareError::InvalidConfig);
        }
        Ok(config)
    }

    async fn connect(
        &self,
    ) -> Result<tokio_rustls::client::TlsStream<TcpStream>, PeerCompareError> {
        let cert = B64
            .decode(&self.server_cert_der_b64)
            .map_err(|_| PeerCompareError::InvalidConfig)?;
        let mut roots = rustls::RootCertStore::empty();
        roots
            .add(CertificateDer::from(cert))
            .map_err(|_| PeerCompareError::InvalidConfig)?;
        let mut config = rustls::ClientConfig::builder_with_provider(provider())
            .with_protocol_versions(&[&rustls::version::TLS13])
            .map_err(|_| PeerCompareError::InvalidConfig)?
            .with_root_certificates(roots)
            .with_no_client_auth();
        config.enable_early_data = false;
        config.resumption = rustls::client::Resumption::disabled();
        let connector = tokio_rustls::TlsConnector::from(Arc::new(config));
        let name = ServerName::try_from(self.server_name.clone())
            .map_err(|_| PeerCompareError::InvalidConfig)?;
        let mut stream = tokio::time::timeout(STEP_TIMEOUT, async {
            let tcp = TcpStream::connect((self.host.as_str(), self.port)).await?;
            tcp.set_nodelay(true)?;
            connector.connect(name, tcp).await
        })
        .await
        .map_err(|_| PeerCompareError::Transport)?
        .map_err(|_| PeerCompareError::Unauthorized)?;
        let mut header = serde_json::to_vec(&serde_json::json!({"ticket": self.ticket}))
            .map_err(|_| PeerCompareError::InvalidConfig)?;
        header.push(b'\n');
        if header.len() > 16 * 1024 {
            return Err(PeerCompareError::LimitExceeded);
        }
        tokio::time::timeout(STEP_TIMEOUT, async {
            stream.write_all(&header).await?;
            let mut response = Vec::new();
            loop {
                let byte = stream.read_u8().await?;
                response.push(byte);
                if byte == b'\n' {
                    break;
                }
                if response.len() > 128 {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "relay response exceeds limit",
                    ));
                }
            }
            let ready: serde_json::Value = serde_json::from_slice(&response).map_err(|_| {
                io::Error::new(io::ErrorKind::InvalidData, "invalid relay response")
            })?;
            if ready != serde_json::json!({"ready": true}) {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "relay refused",
                ));
            }
            Ok::<(), io::Error>(())
        })
        .await
        .map_err(|_| PeerCompareError::Transport)?
        .map_err(|_| PeerCompareError::Unauthorized)?;
        Ok(stream)
    }
}

pub(crate) struct RelaySocket {
    outgoing: mpsc::Sender<Vec<u8>>,
    incoming: Mutex<mpsc::Receiver<Vec<u8>>>,
    writable: Arc<Notify>,
    closed: Arc<AtomicBool>,
    close: Arc<Notify>,
    tasks: Mutex<Vec<tokio::task::AbortHandle>>,
}
impl fmt::Debug for RelaySocket {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RelaySocket")
            .field("mtu", &MTU)
            .field("queue_packets", &PACKETS)
            .finish()
    }
}
impl Drop for RelaySocket {
    fn drop(&mut self) {
        self.close();
    }
}

impl RelaySocket {
    pub(crate) async fn connect(config: RelayConfig) -> Result<Arc<Self>, PeerCompareError> {
        let stream = config.connect().await?;
        let (mut reader, mut writer) = tokio::io::split(stream);
        let (outgoing, mut output) = mpsc::channel::<Vec<u8>>(PACKETS);
        let (input, incoming) = mpsc::channel::<Vec<u8>>(PACKETS);
        let writable = Arc::new(Notify::new());
        let closed = Arc::new(AtomicBool::new(false));
        let close = Arc::new(Notify::new());
        let socket = Arc::new(Self {
            outgoing,
            incoming: Mutex::new(incoming),
            writable: writable.clone(),
            closed: closed.clone(),
            close: close.clone(),
            tasks: Mutex::new(Vec::new()),
        });
        let writer_closed = closed.clone();
        let writer_close = close.clone();
        let writer_writable = writable.clone();
        let writer_task = tokio::spawn(async move {
            loop {
                let bytes = tokio::select! { _ = writer_close.notified() => break, next = output.recv() => match next { Some(bytes) => bytes, None => break } };
                writer_writable.notify_waiters();
                let sent = tokio::time::timeout(STEP_TIMEOUT, async {
                    writer.write_u16(bytes.len() as u16).await?;
                    writer.write_all(&bytes).await
                })
                .await;
                if !matches!(sent, Ok(Ok(()))) {
                    break;
                }
            }
            writer_closed.store(true, Ordering::Release);
            writer_close.notify_waiters();
            writer_writable.notify_waiters();
            let _ = writer.shutdown().await;
        });
        let reader_task = tokio::spawn(async move {
            loop {
                let packet = tokio::select! {
                    _ = close.notified() => break,
                    result = tokio::time::timeout(STEP_TIMEOUT, async {
                        let length = reader.read_u16().await? as usize;
                        if length == 0 || length > MTU { return Err(io::Error::new(io::ErrorKind::InvalidData, "invalid relay datagram")); }
                        let mut packet = vec![0; length];
                        reader.read_exact(&mut packet).await?;
                        Ok::<_, io::Error>(packet)
                    }) => match result { Ok(Ok(packet)) => packet, _ => break }
                };
                if input.send(packet).await.is_err() {
                    break;
                }
            }
            closed.store(true, Ordering::Release);
            close.notify_waiters();
            writable.notify_waiters();
        });
        socket
            .tasks
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .extend([writer_task.abort_handle(), reader_task.abort_handle()]);
        Ok(socket)
    }

    pub(crate) fn close(&self) {
        self.closed.store(true, Ordering::Release);
        self.incoming
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .close();
        self.close.notify_waiters();
        self.writable.notify_waiters();
        for task in self.tasks.lock().unwrap_or_else(|p| p.into_inner()).iter() {
            task.abort();
        }
    }
}

impl AsyncUdpSocket for RelaySocket {
    fn create_io_poller(self: Arc<Self>) -> Pin<Box<dyn UdpPoller>> {
        Box::pin(RelayPoller {
            socket: self,
            waiting: None,
        })
    }
    fn try_send(&self, transmit: &Transmit<'_>) -> io::Result<()> {
        if self.closed.load(Ordering::Acquire) {
            return Err(io::ErrorKind::BrokenPipe.into());
        }
        if transmit.contents.is_empty()
            || transmit.contents.len() > MTU
            || transmit.segment_size.is_some()
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "QA relay MTU exceeded",
            ));
        }
        self.outgoing
            .try_send(transmit.contents.to_vec())
            .map_err(|error| match error {
                mpsc::error::TrySendError::Full(_) => io::ErrorKind::WouldBlock.into(),
                mpsc::error::TrySendError::Closed(_) => io::ErrorKind::BrokenPipe.into(),
            })
    }
    fn poll_recv(
        &self,
        cx: &mut Context<'_>,
        bufs: &mut [IoSliceMut<'_>],
        meta: &mut [RecvMeta],
    ) -> Poll<io::Result<usize>> {
        if self.closed.load(Ordering::Acquire) {
            return Poll::Ready(Err(io::ErrorKind::BrokenPipe.into()));
        }
        if bufs.is_empty() || meta.is_empty() {
            return Poll::Ready(Err(io::ErrorKind::InvalidInput.into()));
        }
        let mut receiver = self.incoming.lock().unwrap_or_else(|p| p.into_inner());
        match receiver.poll_recv(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(None) => Poll::Ready(Err(io::ErrorKind::BrokenPipe.into())),
            Poll::Ready(Some(packet)) => {
                if packet.len() > bufs[0].len() {
                    return Poll::Ready(Err(io::ErrorKind::InvalidData.into()));
                }
                bufs[0][..packet.len()].copy_from_slice(&packet);
                meta[0] = RecvMeta {
                    addr: SocketAddr::from((Ipv4Addr::LOCALHOST, 2)),
                    len: packet.len(),
                    stride: packet.len(),
                    ecn: None,
                    dst_ip: Some(Ipv4Addr::LOCALHOST.into()),
                };
                Poll::Ready(Ok(1))
            }
        }
    }
    fn local_addr(&self) -> io::Result<SocketAddr> {
        Ok(SocketAddr::from((Ipv4Addr::LOCALHOST, 1)))
    }
    fn may_fragment(&self) -> bool {
        false
    }
}

struct RelayPoller {
    socket: Arc<RelaySocket>,
    waiting: Option<Pin<Box<dyn std::future::Future<Output = ()> + Send + Sync>>>,
}
impl fmt::Debug for RelayPoller {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RelayPoller").finish()
    }
}
impl UdpPoller for RelayPoller {
    fn poll_writable(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        loop {
            if self.socket.closed.load(Ordering::Acquire) {
                return Poll::Ready(Err(io::ErrorKind::BrokenPipe.into()));
            }
            if self.waiting.is_none() {
                self.waiting = Some(Box::pin(self.socket.writable.clone().notified_owned()));
            }
            let readiness = self.waiting.as_mut().unwrap().as_mut().poll(cx);
            if self.socket.outgoing.capacity() > 0 {
                self.waiting = None;
                return Poll::Ready(Ok(()));
            }
            if readiness.is_pending() {
                return Poll::Pending;
            }
            self.waiting = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn closing_a_retained_relay_socket_disconnects_tls_promptly() {
        let key = rcgen::KeyPair::generate().unwrap();
        let cert = rcgen::CertificateParams::new(vec![server_name()])
            .unwrap()
            .self_signed(&key)
            .unwrap();
        let config = rustls::ServerConfig::builder_with_provider(provider())
            .with_protocol_versions(&[&rustls::version::TLS13])
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(
                vec![cert.der().clone()],
                rustls::pki_types::PrivatePkcs8KeyDer::from(key.serialize_der()).into(),
            )
            .unwrap();
        let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(config));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let closed = tokio::spawn(async move {
            let (tcp, _) = listener.accept().await.unwrap();
            let mut stream = acceptor.accept(tcp).await.unwrap();
            loop {
                if stream.read_u8().await.unwrap() == b'\n' {
                    break;
                }
            }
            stream.write_all(b"{\"ready\":true}\n").await.unwrap();
            let mut byte = [0];
            stream.read(&mut byte).await
        });
        let socket = RelaySocket::connect(RelayConfig {
            host: address.ip().to_string(),
            port: address.port(),
            server_cert_der_b64: B64.encode(cert.der()),
            ticket: "scoped-lifecycle-fixture".into(),
            server_name: server_name(),
        })
        .await
        .unwrap();
        socket.close();
        let result = tokio::time::timeout(std::time::Duration::from_secs(1), closed)
            .await
            .unwrap()
            .unwrap();
        assert!(!matches!(result, Ok(count) if count != 0));
        // Keep the closed socket owned until after the peer has observed disconnect.
        assert!(socket.closed.load(Ordering::Acquire));
        drop(socket);
    }
}
