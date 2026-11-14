use std::sync::Arc;
use std::time::Duration;

use pennel_peer_comparison_relay::{Relay, RelayLimits, RelayOptions};
use rustls::pki_types::{CertificateDer, ServerName};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::{sleep, timeout};
use tokio_rustls::TlsConnector;
use tokio_rustls::client::TlsStream;

const A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

fn options() -> RelayOptions {
    RelayOptions {
        bind: "127.0.0.1:0".parse().unwrap(),
        advertise_host: None,
        peers: [A.into(), B.into()],
        candidate: "quinn".into(),
        run_id: "relay-test".into(),
        ticket_ttl: Duration::from_secs(30),
        limits: RelayLimits::default(),
    }
}

async fn connect(relay: &Relay, ticket: &str) -> TlsStream<TcpStream> {
    let mut stream = tls_connect(relay).await;
    let header = serde_json::json!({"ticket": ticket}).to_string() + "\n";
    stream.write_all(header.as_bytes()).await.unwrap();
    stream
}

async fn tls_connect(relay: &Relay) -> TlsStream<TcpStream> {
    let mut roots = rustls::RootCertStore::empty();
    roots
        .add(CertificateDer::from(relay.certificate_der().to_vec()))
        .unwrap();
    let config = rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_protocol_versions(&[&rustls::version::TLS13])
    .unwrap()
    .with_root_certificates(roots)
    .with_no_client_auth();
    TlsConnector::from(Arc::new(config))
        .connect(
            ServerName::try_from("pennel-peer-relay.qa").unwrap(),
            TcpStream::connect(relay.addr()).await.unwrap(),
        )
        .await
        .unwrap()
}

async fn line(stream: &mut TlsStream<TcpStream>) -> String {
    let mut bytes = Vec::new();
    while let Ok(byte) = timeout(Duration::from_secs(2), stream.read_u8())
        .await
        .unwrap()
    {
        bytes.push(byte);
        if byte == b'\n' {
            break;
        }
        assert!(bytes.len() < 1024);
    }
    String::from_utf8(bytes).unwrap()
}

async fn pair(relay: &Relay) -> (TlsStream<TcpStream>, TlsStream<TcpStream>) {
    let mut left = connect(relay, &relay.client_config(A).unwrap().ticket).await;
    let mut right = connect(relay, &relay.client_config(B).unwrap().ticket).await;
    assert_eq!(line(&mut left).await, "{\"ready\":true}\n");
    assert_eq!(line(&mut right).await, "{\"ready\":true}\n");
    (left, right)
}

#[tokio::test]
async fn encrypted_pair_waits_for_both_peers_and_copies_original_bytes() {
    let relay = Relay::bind(options()).await.unwrap();
    let mut left = connect(&relay, &relay.client_config(A).unwrap().ticket).await;
    assert!(
        timeout(Duration::from_millis(50), left.read_u8())
            .await
            .is_err()
    );
    let mut right = connect(&relay, &relay.client_config(B).unwrap().ticket).await;
    assert_eq!(line(&mut left).await, "{\"ready\":true}\n");
    assert_eq!(line(&mut right).await, "{\"ready\":true}\n");
    let fixture: Vec<u8> = (0..160_000).map(|i| (i % 251) as u8).collect();
    let expected = fixture.clone();
    let sent = tokio::spawn(async move {
        left.write_all(&fixture).await.unwrap();
        let mut reply = [0; 4];
        left.read_exact(&mut reply).await.unwrap();
        assert_eq!(&reply, b"done");
        left
    });
    let mut received = vec![0; expected.len()];
    right.read_exact(&mut received).await.unwrap();
    assert_eq!(received, expected);
    right.write_all(b"done").await.unwrap();
    drop(sent.await.unwrap());
    drop(right);
    relay.shutdown().await;
    assert_eq!(relay.stats().active_pairs, 0);
}

#[tokio::test]
async fn duplicate_role_is_rejected_without_displacing_waiting_peer() {
    let relay = Relay::bind(options()).await.unwrap();
    let mut first = connect(&relay, &relay.client_config(A).unwrap().ticket).await;
    let mut duplicate = connect(&relay, &relay.client_config(A).unwrap().ticket).await;
    assert_eq!(
        line(&mut duplicate).await,
        "{\"ready\":false,\"error\":\"busy\"}\n"
    );
    let mut second = connect(&relay, &relay.client_config(B).unwrap().ticket).await;
    assert_eq!(line(&mut first).await, "{\"ready\":true}\n");
    assert_eq!(line(&mut second).await, "{\"ready\":true}\n");
    first.write_all(b"original").await.unwrap();
    let mut data = [0; 8];
    second.read_exact(&mut data).await.unwrap();
    assert_eq!(&data, b"original");
    relay.shutdown().await;
}

#[tokio::test]
async fn forged_and_other_run_tickets_are_rejected() {
    let relay = Relay::bind(options()).await.unwrap();
    let mut other_options = options();
    other_options.run_id = "other-run".into();
    let other = Relay::bind(other_options).await.unwrap();
    let ticket = other.client_config(A).unwrap().ticket;
    let mut stream = connect(&relay, &ticket).await;
    assert_eq!(
        line(&mut stream).await,
        "{\"ready\":false,\"error\":\"unauthorized\"}\n"
    );
    let mut ticket = relay.client_config(A).unwrap().ticket;
    ticket.replace_range(0..1, if ticket.starts_with('A') { "B" } else { "A" });
    let mut stream = connect(&relay, &ticket).await;
    assert_eq!(
        line(&mut stream).await,
        "{\"ready\":false,\"error\":\"unauthorized\"}\n"
    );
    relay.shutdown().await;
    other.shutdown().await;
}

#[tokio::test]
async fn pre_tls_setup_permits_cover_slow_connections_and_are_released() {
    let mut settings = options();
    settings.limits.max_setup = 1;
    settings.limits.handshake_timeout = Duration::from_millis(100);
    let relay = Relay::bind(settings).await.unwrap();
    let stalled = TcpStream::connect(relay.addr()).await.unwrap();
    sleep(Duration::from_millis(30)).await;
    assert_eq!(relay.stats().setup_connections, 1);
    let mut refused = TcpStream::connect(relay.addr()).await.unwrap();
    let mut byte = [0; 1];
    assert_eq!(
        timeout(Duration::from_secs(1), refused.read(&mut byte))
            .await
            .unwrap()
            .unwrap(),
        0
    );
    sleep(Duration::from_millis(120)).await;
    assert_eq!(relay.stats().setup_connections, 0);
    drop(stalled);
    relay.shutdown().await;
}

#[tokio::test]
async fn revocation_closes_waiting_and_active_connections() {
    let relay = Relay::bind(options()).await.unwrap();
    let (mut left, mut right) = pair(&relay).await;
    relay.revoke();
    let mut buf = [0; 1];
    for stream in [&mut left, &mut right] {
        assert!(
            timeout(Duration::from_secs(1), stream.read(&mut buf))
                .await
                .unwrap()
                .unwrap_or(0)
                == 0
        );
    }
    let mut rejected = connect(&relay, &relay.client_config(A).unwrap().ticket).await;
    assert_eq!(
        line(&mut rejected).await,
        "{\"ready\":false,\"error\":\"unauthorized\"}\n"
    );
    relay.shutdown().await;
}

#[tokio::test]
async fn ticket_expiry_closes_an_active_pair() {
    let mut settings = options();
    settings.ticket_ttl = Duration::from_secs(2);
    let relay = Relay::bind(settings).await.unwrap();
    let (mut left, mut right) = pair(&relay).await;
    sleep(Duration::from_millis(2100)).await;
    let mut buf = [0; 1];
    for stream in [&mut left, &mut right] {
        assert!(
            timeout(Duration::from_secs(1), stream.read(&mut buf))
                .await
                .unwrap()
                .unwrap_or(0)
                == 0
        );
    }
    relay.shutdown().await;
}

#[tokio::test]
async fn disconnect_closes_the_pair_and_tickets_can_resume_sequentially() {
    let relay = Relay::bind(options()).await.unwrap();
    let (left, mut right) = pair(&relay).await;
    drop(left);
    let mut buf = [0; 1];
    assert_eq!(
        timeout(Duration::from_secs(1), right.read(&mut buf))
            .await
            .unwrap()
            .unwrap_or(0),
        0
    );
    drop(right);
    sleep(Duration::from_millis(20)).await;
    let (mut left, mut right) = pair(&relay).await;
    left.write_all(b"resume").await.unwrap();
    let mut buf = [0; 6];
    right.read_exact(&mut buf).await.unwrap();
    assert_eq!(&buf, b"resume");
    relay.shutdown().await;
}

#[tokio::test]
async fn immediate_shutdown_observes_a_signal_sent_before_serve_starts() {
    let relay = Relay::bind(options()).await.unwrap();
    assert!(
        timeout(Duration::from_millis(200), relay.shutdown())
            .await
            .is_ok()
    );
    assert_eq!(relay.stats().setup_connections, 0);
}

#[tokio::test]
async fn oversized_and_malformed_headers_do_not_take_a_pair_slot() {
    let relay = Relay::bind(options()).await.unwrap();
    let mut malformed = tls_connect(&relay).await;
    malformed.write_all(b"{\"ticket\":12}\n").await.unwrap();
    assert_eq!(
        line(&mut malformed).await,
        "{\"ready\":false,\"error\":\"unauthorized\"}\n"
    );
    let mut oversized = tls_connect(&relay).await;
    oversized
        .write_all(&vec![b'x'; 16 * 1024 + 1])
        .await
        .unwrap();
    assert_eq!(
        line(&mut oversized).await,
        "{\"ready\":false,\"error\":\"unauthorized\"}\n"
    );
    assert_eq!(relay.stats().waiting_connections, 0);
    let (left, right) = pair(&relay).await;
    drop((left, right));
    relay.shutdown().await;
}

#[tokio::test]
async fn shutdown_closes_waiting_peers_without_a_match() {
    let relay = Relay::bind(options()).await.unwrap();
    let mut waiting = connect(&relay, &relay.client_config(A).unwrap().ticket).await;
    sleep(Duration::from_millis(10)).await;
    assert_eq!(relay.stats().waiting_connections, 1);
    timeout(Duration::from_millis(200), relay.shutdown())
        .await
        .unwrap();
    let mut byte = [0; 1];
    assert_eq!(
        timeout(Duration::from_millis(200), waiting.read(&mut byte))
            .await
            .unwrap()
            .unwrap_or(0),
        0
    );
    assert_eq!(relay.stats().waiting_connections, 0);
    assert_eq!(relay.stats().setup_connections, 0);
}

#[tokio::test]
async fn configured_pacing_creates_a_real_slow_consumer_window() {
    let mut settings = options();
    settings.limits.rate_bytes_per_second = 64 * 1024;
    let relay = Relay::bind(settings).await.unwrap();
    let (mut left, mut right) = pair(&relay).await;
    let sender = tokio::spawn(async move {
        let _ = left.write_all(&vec![0x5a; 128 * 1024]).await;
        left
    });
    let mut first = [0; 1024];
    right.read_exact(&mut first).await.unwrap();
    assert_eq!(first, [0x5a; 1024]);
    let mut rest = vec![0; 128 * 1024 - first.len()];
    assert!(
        timeout(Duration::from_millis(100), right.read_exact(&mut rest))
            .await
            .is_err()
    );
    timeout(Duration::from_millis(200), relay.shutdown())
        .await
        .unwrap();
    drop(sender.await.unwrap());
}

#[tokio::test]
async fn expired_ticket_cannot_start_a_new_pair() {
    let mut settings = options();
    settings.ticket_ttl = Duration::from_secs(2);
    let relay = Relay::bind(settings).await.unwrap();
    let ticket = relay.client_config(A).unwrap().ticket;
    sleep(Duration::from_millis(2100)).await;
    let mut stream = connect(&relay, &ticket).await;
    assert_eq!(
        line(&mut stream).await,
        "{\"ready\":false,\"error\":\"unauthorized\"}\n"
    );
    relay.shutdown().await;
}

#[tokio::test]
async fn restarted_relay_rejects_old_tickets_even_for_same_pair() {
    let relay = Relay::bind(options()).await.unwrap();
    let old_ticket = relay.client_config(A).unwrap().ticket;
    relay.shutdown().await;
    let restarted = Relay::bind(options()).await.unwrap();
    let mut stream = connect(&restarted, &old_ticket).await;
    assert_eq!(
        line(&mut stream).await,
        "{\"ready\":false,\"error\":\"unauthorized\"}\n"
    );
    let (left, right) = pair(&restarted).await;
    drop((left, right));
    restarted.shutdown().await;
}

#[tokio::test]
async fn disconnected_waiting_peer_releases_its_role_for_reconnection() {
    let relay = Relay::bind(options()).await.unwrap();
    let first = connect(&relay, &relay.client_config(A).unwrap().ticket).await;
    sleep(Duration::from_millis(20)).await;
    assert_eq!(relay.stats().waiting_connections, 1);
    drop(first);
    sleep(Duration::from_millis(20)).await;
    assert_eq!(relay.stats().waiting_connections, 0);
    let (left, right) = pair(&relay).await;
    drop((left, right));
    relay.shutdown().await;
}

#[tokio::test]
async fn one_setup_permit_can_pair_two_authenticated_peers() {
    let mut settings = options();
    settings.limits.max_setup = 1;
    let relay = Relay::bind(settings).await.unwrap();
    let mut first = connect(&relay, &relay.client_config(A).unwrap().ticket).await;
    sleep(Duration::from_millis(20)).await;
    assert_eq!(relay.stats().waiting_connections, 1);
    assert_eq!(relay.stats().setup_connections, 0);
    let mut second = connect(&relay, &relay.client_config(B).unwrap().ticket).await;
    assert_eq!(line(&mut first).await, "{\"ready\":true}\n");
    assert_eq!(line(&mut second).await, "{\"ready\":true}\n");
    relay.shutdown().await;
}

#[tokio::test]
async fn pause_after_pairing_stalls_bounded_writes_and_resume_preserves_bytes() {
    let relay = Relay::bind(options()).await.unwrap();
    let (mut left, mut right) = pair(&relay).await;
    left.write_all(b"stage").await.unwrap();
    let mut staged = [0; 5];
    right.read_exact(&mut staged).await.unwrap();
    assert_eq!(&staged, b"stage");
    relay.pause();
    assert!(relay.stats().paused);
    let staged_bytes = relay.stats().forwarded_bytes;
    let fixture: Vec<u8> = (0..160_000).map(|i| (i % 251) as u8).collect();
    let expected = fixture.clone();
    let sender = tokio::spawn(async move {
        left.write_all(&fixture).await.unwrap();
        left
    });
    let mut probe = [0; 1];
    assert!(
        timeout(Duration::from_millis(100), right.read(&mut probe))
            .await
            .is_err()
    );
    assert_eq!(relay.stats().forwarded_bytes, staged_bytes);
    relay.resume();
    assert!(!relay.stats().paused);
    let mut received = vec![0; expected.len()];
    timeout(Duration::from_secs(2), right.read_exact(&mut received))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(received, expected);
    drop(sender.await.unwrap());
    relay.shutdown().await;
}

#[tokio::test]
async fn revocation_and_shutdown_interrupt_paused_forwarding() {
    for revoke in [true, false] {
        let relay = Relay::bind(options()).await.unwrap();
        let (mut left, mut right) = pair(&relay).await;
        relay.pause();
        left.write_all(b"blocked").await.unwrap();
        let mut byte = [0; 1];
        assert!(
            timeout(Duration::from_millis(30), right.read(&mut byte))
                .await
                .is_err()
        );
        if revoke {
            relay.revoke();
        } else {
            timeout(Duration::from_millis(200), relay.shutdown())
                .await
                .unwrap();
        }
        assert_eq!(
            timeout(Duration::from_millis(200), right.read(&mut byte))
                .await
                .unwrap()
                .unwrap_or(0),
            0
        );
        timeout(Duration::from_millis(200), relay.shutdown())
            .await
            .unwrap();
        assert_eq!(relay.stats().active_pairs, 0);
    }
}

#[tokio::test]
async fn ticket_expiry_closes_a_paused_pair() {
    let mut settings = options();
    settings.ticket_ttl = Duration::from_secs(2);
    let relay = Relay::bind(settings).await.unwrap();
    let (mut left, mut right) = pair(&relay).await;
    relay.pause();
    left.write_all(b"blocked").await.unwrap();
    sleep(Duration::from_millis(2100)).await;
    let mut byte = [0; 1];
    assert_eq!(
        timeout(Duration::from_millis(200), right.read(&mut byte))
            .await
            .unwrap()
            .unwrap_or(0),
        0
    );
    timeout(Duration::from_millis(200), relay.shutdown())
        .await
        .unwrap();
    assert_eq!(relay.stats().active_pairs, 0);
}
