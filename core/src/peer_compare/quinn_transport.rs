use std::{
    collections::HashSet,
    io,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    pin::Pin,
    sync::{Arc, Mutex},
    task::{Context, Poll},
};

use quinn::{
    Endpoint, EndpointConfig, TransportConfig,
    crypto::rustls::{QuicClientConfig, QuicServerConfig},
};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

use super::{
    ALPN, PeerCompareError, PeerCompareIdentity, STEP_TIMEOUT,
    identity::{PeerVerifier, fingerprint, provider, valid_peer},
    relay::{RelayConfig, RelaySocket},
    wire::Channel,
};

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Address {
    pub(crate) identity_id: String,
    pub(crate) host: String,
    pub(crate) port: u16,
    pub(crate) route: String,
}
impl Address {
    pub(crate) fn parse(json: &str) -> Result<Self, PeerCompareError> {
        if json.len() > 16 * 1024 {
            return Err(PeerCompareError::LimitExceeded);
        }
        let address: Self =
            serde_json::from_str(json).map_err(|_| PeerCompareError::InvalidConfig)?;
        let ip: IpAddr = address
            .host
            .parse()
            .map_err(|_| PeerCompareError::InvalidConfig)?;
        if !valid_peer(&address.identity_id)
            || address.port == 0
            || ip.is_unspecified()
            || ip.is_multicast()
            || !matches!(address.route.as_str(), "direct" | "forced_relay")
        {
            return Err(PeerCompareError::InvalidConfig);
        }
        Ok(address)
    }
    fn socket(&self) -> Result<SocketAddr, PeerCompareError> {
        Ok(SocketAddr::new(
            self.host
                .parse()
                .map_err(|_| PeerCompareError::InvalidConfig)?,
            self.port,
        ))
    }
}

pub(crate) struct QuinnTransport {
    endpoint: Endpoint,
    identity: PeerCompareIdentity,
    pins: Arc<Mutex<HashSet<String>>>,
    connections: Mutex<Vec<(String, quinn::Connection)>>,
    route: &'static str,
    relay_socket: Option<Arc<RelaySocket>>,
}

impl QuinnTransport {
    pub(crate) async fn bind(
        identity: PeerCompareIdentity,
        bind: SocketAddr,
        relay: Option<RelayConfig>,
    ) -> Result<Arc<Self>, PeerCompareError> {
        let pins = Arc::new(Mutex::new(HashSet::new()));
        let (cert, key) = identity.parts()?;
        let mut tls = rustls::ServerConfig::builder_with_provider(provider())
            .with_protocol_versions(&[&rustls::version::TLS13])
            .map_err(|_| PeerCompareError::InvalidConfig)?
            .with_client_cert_verifier(Arc::new(PeerVerifier { pins: pins.clone() }))
            .with_single_cert(vec![cert], key)
            .map_err(|_| PeerCompareError::Storage)?;
        tls.alpn_protocols = vec![ALPN.to_vec()];
        tls.max_early_data_size = 0;
        tls.send_tls13_tickets = 0;
        let crypto =
            QuicServerConfig::try_from(tls).map_err(|_| PeerCompareError::InvalidConfig)?;
        let mut server = quinn::ServerConfig::with_crypto(Arc::new(crypto));
        server
            .max_incoming(16)
            .incoming_buffer_size(64 * 1024)
            .incoming_buffer_size_total(256 * 1024);
        server.transport_config(transport_config());
        let route = if relay.is_some() {
            "forced_relay"
        } else {
            "direct"
        };
        let mut relay_socket = None;
        let endpoint = if let Some(config) = relay {
            let socket = RelaySocket::connect(config).await?;
            relay_socket = Some(socket.clone());
            Endpoint::new_with_abstract_socket(
                EndpointConfig::default(),
                Some(server),
                socket,
                Arc::new(quinn::TokioRuntime),
            )
        } else {
            let socket =
                std::net::UdpSocket::bind(bind).map_err(|_| PeerCompareError::Transport)?;
            Endpoint::new(
                EndpointConfig::default(),
                Some(server),
                socket,
                Arc::new(quinn::TokioRuntime),
            )
        }
        .map_err(|_| PeerCompareError::Transport)?;
        Ok(Arc::new(Self {
            endpoint,
            identity,
            pins,
            connections: Mutex::new(Vec::new()),
            route,
            relay_socket,
        }))
    }

    pub(crate) fn id(&self) -> String {
        self.identity.identity_id.clone()
    }
    pub(crate) fn address_json(&self) -> Result<String, PeerCompareError> {
        let address = if self.route == "forced_relay" {
            SocketAddr::from((Ipv4Addr::LOCALHOST, 2))
        } else {
            self.endpoint
                .local_addr()
                .map_err(|_| PeerCompareError::Transport)?
        };
        serde_json::to_string(&Address {
            identity_id: self.id(),
            host: address.ip().to_string(),
            port: address.port(),
            route: self.route.into(),
        })
        .map_err(|_| PeerCompareError::InvalidConfig)
    }
    pub(crate) fn approve(&self, id: &str) -> Result<bool, PeerCompareError> {
        if !valid_peer(id) || id == self.identity.identity_id {
            return Err(PeerCompareError::InvalidConfig);
        }
        let mut pins = self.pins.lock().unwrap_or_else(|p| p.into_inner());
        if !pins.contains(id) && pins.len() >= 16 {
            return Err(PeerCompareError::LimitExceeded);
        }
        Ok(pins.insert(id.to_owned()))
    }
    pub(crate) fn revoke(&self, id: &str) -> Result<bool, PeerCompareError> {
        if !valid_peer(id) {
            return Err(PeerCompareError::InvalidConfig);
        }
        let removed = self
            .pins
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(id);
        let mut connections = self.connections.lock().unwrap_or_else(|p| p.into_inner());
        connections.retain(|(peer, connection)| {
            if peer == id {
                connection.close(403u32.into(), b"peer revoked");
                false
            } else {
                connection.close_reason().is_none()
            }
        });
        Ok(removed)
    }
    pub(crate) fn close(&self) {
        self.endpoint.close(0u32.into(), b"qualification closed");
        if let Some(socket) = &self.relay_socket {
            socket.close();
        }
    }

    fn track(
        &self,
        connection: quinn::Connection,
        expected: Option<&str>,
    ) -> Result<quinn::Connection, PeerCompareError> {
        let identity = connection
            .peer_identity()
            .ok_or(PeerCompareError::Unauthorized)?;
        let certificates = identity
            .downcast::<Vec<rustls::pki_types::CertificateDer<'static>>>()
            .map_err(|_| PeerCompareError::Unauthorized)?;
        let peer = certificates
            .first()
            .map(|cert| fingerprint(cert.as_ref()))
            .ok_or(PeerCompareError::Unauthorized)?;
        let pins = self.pins.lock().unwrap_or_else(|p| p.into_inner());
        if !pins.contains(&peer) || expected.is_some_and(|expected| expected != peer) {
            connection.close(403u32.into(), b"peer not approved");
            return Err(PeerCompareError::Unauthorized);
        }
        let mut connections = self.connections.lock().unwrap_or_else(|p| p.into_inner());
        connections.retain(|(_, connection)| connection.close_reason().is_none());
        if connections.len() >= 16 || connections.iter().filter(|(id, _)| id == &peer).count() >= 2
        {
            connection.close(429u32.into(), b"connection limit");
            return Err(PeerCompareError::LimitExceeded);
        }
        connections.push((peer, connection.clone()));
        Ok(connection)
    }

    pub(crate) async fn connect(&self, encoded: &str) -> Result<Channel, PeerCompareError> {
        let address = Address::parse(encoded)?;
        if address.route != self.route {
            return Err(PeerCompareError::InvalidConfig);
        }
        if !self
            .pins
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .contains(&address.identity_id)
        {
            return Err(PeerCompareError::Unauthorized);
        }
        let client = client_config(&self.identity, &address.identity_id)?;
        let connecting = self
            .endpoint
            .connect_with(client, address.socket()?, "pennel-peer.qa")
            .map_err(|_| PeerCompareError::Transport)?;
        let connection = tokio::time::timeout(STEP_TIMEOUT, connecting)
            .await
            .map_err(|_| PeerCompareError::Transport)?
            .map_err(|_| PeerCompareError::Unauthorized)?;
        let connection = self.track(connection, Some(&address.identity_id))?;
        let (send, recv) = tokio::time::timeout(STEP_TIMEOUT, connection.open_bi())
            .await
            .map_err(|_| PeerCompareError::Transport)?
            .map_err(|_| PeerCompareError::Transport)?;
        Ok(Box::new(QuinnChannel {
            send,
            recv,
            connection,
            shutdown: None,
        }))
    }

    pub(crate) async fn accept(&self) -> Result<Channel, PeerCompareError> {
        loop {
            let incoming = self
                .endpoint
                .accept()
                .await
                .ok_or(PeerCompareError::Closed)?;
            let Ok(Ok(connection)) = tokio::time::timeout(STEP_TIMEOUT, incoming).await else {
                continue;
            };
            let Ok(connection) = self.track(connection, None) else {
                continue;
            };
            let (send, recv) = tokio::time::timeout(STEP_TIMEOUT, connection.accept_bi())
                .await
                .map_err(|_| PeerCompareError::Transport)?
                .map_err(|_| PeerCompareError::Transport)?;
            return Ok(Box::new(QuinnChannel {
                send,
                recv,
                connection,
                shutdown: None,
            }));
        }
    }
}

fn client_config(
    identity: &PeerCompareIdentity,
    peer: &str,
) -> Result<quinn::ClientConfig, PeerCompareError> {
    let (cert, key) = identity.parts()?;
    let pins = Arc::new(Mutex::new(HashSet::from([peer.to_owned()])));
    let mut tls = rustls::ClientConfig::builder_with_provider(provider())
        .with_protocol_versions(&[&rustls::version::TLS13])
        .map_err(|_| PeerCompareError::InvalidConfig)?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(PeerVerifier { pins }))
        .with_client_auth_cert(vec![cert], key)
        .map_err(|_| PeerCompareError::Storage)?;
    tls.alpn_protocols = vec![ALPN.to_vec()];
    tls.enable_early_data = false;
    tls.resumption = rustls::client::Resumption::disabled();
    let crypto = QuicClientConfig::try_from(tls).map_err(|_| PeerCompareError::InvalidConfig)?;
    let mut config = quinn::ClientConfig::new(Arc::new(crypto));
    config.transport_config(transport_config());
    Ok(config)
}

pub(crate) fn transport_config() -> Arc<TransportConfig> {
    let mut transport = TransportConfig::default();
    transport
        .max_concurrent_bidi_streams(2u32.into())
        .max_concurrent_uni_streams(0u32.into())
        .stream_receive_window((2 * 1024 * 1024u32).into())
        .receive_window((4 * 1024 * 1024u32).into())
        .send_window(4 * 1024 * 1024)
        .initial_mtu(1200)
        .min_mtu(1200)
        .mtu_discovery_config(None)
        .enable_segmentation_offload(false)
        .max_idle_timeout(Some(
            quinn::IdleTimeout::try_from(STEP_TIMEOUT).expect("fixed timeout"),
        ));
    Arc::new(transport)
}

struct QuinnChannel {
    send: quinn::SendStream,
    recv: quinn::RecvStream,
    connection: quinn::Connection,
    shutdown: Option<Pin<Box<dyn std::future::Future<Output = io::Result<()>> + Send>>>,
}
impl Drop for QuinnChannel {
    fn drop(&mut self) {
        self.connection
            .close(0u32.into(), b"qualification finished");
    }
}
impl AsyncRead for QuinnChannel {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        AsyncRead::poll_read(Pin::new(&mut self.recv), cx, buf)
    }
}
impl AsyncWrite for QuinnChannel {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<io::Result<usize>> {
        AsyncWrite::poll_write(Pin::new(&mut self.send), cx, bytes)
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        AsyncWrite::poll_flush(Pin::new(&mut self.send), cx)
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        if self.shutdown.is_none() {
            if let Err(error) = self.send.finish() {
                return Poll::Ready(Err(error.into()));
            }
            let stopped = self.send.stopped();
            self.shutdown = Some(Box::pin(async move {
                match stopped.await {
                    Ok(None) => Ok(()),
                    _ => Err(io::Error::new(
                        io::ErrorKind::BrokenPipe,
                        "QA stream stopped before receipt",
                    )),
                }
            }));
        }
        self.shutdown.as_mut().unwrap().as_mut().poll(cx)
    }
}

#[cfg(test)]
mod tests {
    use super::super::generate_peer_compare_identity;
    use super::*;

    #[tokio::test]
    async fn real_quic_attempts_are_bounded_before_any_tls_acceptance() {
        let server = QuinnTransport::bind(
            generate_peer_compare_identity().unwrap(),
            "127.0.0.1:0".parse().unwrap(),
            None,
        )
        .await
        .unwrap();
        let client = QuinnTransport::bind(
            generate_peer_compare_identity().unwrap(),
            "127.0.0.1:0".parse().unwrap(),
            None,
        )
        .await
        .unwrap();
        let config = client_config(&client.identity, &server.id()).unwrap();
        let address = server.endpoint.local_addr().unwrap();
        let mut attempts = Vec::new();
        let mut pending = Vec::new();
        for _ in 0..16 {
            attempts.push(
                client
                    .endpoint
                    .connect_with(config.clone(), address, "pennel-peer.qa")
                    .unwrap(),
            );
            // Hold the public Incoming objects: no peer identity or TLS accept has run.
            let incoming =
                tokio::time::timeout(std::time::Duration::from_secs(2), server.endpoint.accept())
                    .await
                    .unwrap()
                    .unwrap();
            pending.push(incoming);
        }
        let excess = client
            .endpoint
            .connect_with(config.clone(), address, "pennel-peer.qa")
            .unwrap();
        attempts.push(excess);
        // The locked quinn-proto drops saturated Initial packets rather than allocating
        // state to produce a refusal. This assertion observes admission, not client errors.
        assert!(
            tokio::time::timeout(
                std::time::Duration::from_millis(250),
                server.endpoint.accept()
            )
            .await
            .is_err()
        );
        drop(pending.pop());
        attempts.push(
            client
                .endpoint
                .connect_with(config, address, "pennel-peer.qa")
                .unwrap(),
        );
        let admitted =
            tokio::time::timeout(std::time::Duration::from_secs(2), server.endpoint.accept())
                .await
                .unwrap()
                .unwrap();
        drop(admitted);
        drop(pending);
        drop(attempts);
        server.close();
        client.close();
    }
}
