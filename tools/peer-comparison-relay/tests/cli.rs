use std::io::{BufRead, BufReader, Write};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

struct OwnedChild(Child);
impl Drop for OwnedChild {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn cli_emits_client_configs_and_accepts_scoped_control_commands() {
    let peers = ["a".repeat(64), "b".repeat(64)];
    let mut child = OwnedChild(
        Command::new(env!("CARGO_BIN_EXE_pennel-peer-comparison-relay"))
            .args([
                "--bind",
                "127.0.0.1:0",
                "--allow",
                &peers[0],
                "--allow",
                &peers[1],
                "--candidate",
                "native",
                "--run-id",
                "cli-test",
                "--rate-bytes-per-second",
                "1048576",
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let mut input = child.0.stdin.take().unwrap();
    let mut output = BufReader::new(child.0.stdout.take().unwrap());
    let (send, receive) = mpsc::channel();
    let reader = std::thread::spawn(move || {
        loop {
            let mut line = String::new();
            match output.read_line(&mut line) {
                Ok(0) | Err(_) => break,
                _ => {
                    if send.send(line).is_err() {
                        break;
                    }
                }
            }
        }
    });
    let next = || {
        let line = receive.recv_timeout(Duration::from_secs(3)).unwrap();
        serde_json::from_str::<serde_json::Value>(&line).unwrap()
    };
    let ready = next();
    assert_eq!(ready["event"], "ready");
    assert_eq!(ready["candidate"], "native");
    assert_eq!(ready["run_id"], "cli-test");
    assert_eq!(ready["rate_bytes_per_second"], 1_048_576);
    assert_eq!(ready["payload_storage"], false);
    assert!(ready["port"].as_u64().unwrap() != 0);
    for peer in peers {
        let config = &ready["clients"][peer];
        assert!(config["ticket"].as_str().unwrap().contains('.'));
        assert_eq!(config["server_name"], "pennel-peer-relay.qa");
        assert!(!config["server_cert_der_b64"].as_str().unwrap().is_empty());
    }
    input.write_all(b"{\"command\":\"stats\"}\n").unwrap();
    let stats = next();
    assert_eq!(stats["event"], "stats");
    assert_eq!(stats["result"]["active_pairs"], 0);
    assert_eq!(stats["result"]["paused"], false);
    input.write_all(b"{\"command\":\"pause\"}\n").unwrap();
    let paused = next();
    assert_eq!(paused["event"], "paused");
    assert_eq!(paused["result"]["paused"], true);
    input.write_all(b"{\"command\":\"resume\"}\n").unwrap();
    let resumed = next();
    assert_eq!(resumed["event"], "resumed");
    assert_eq!(resumed["result"]["paused"], false);
    input.write_all(b"{\"command\":\"revoke\"}\n").unwrap();
    assert_eq!(next()["event"], "revoked");
    input.write_all(b"{\"command\":\"shutdown\"}\n").unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        if let Some(status) = child.0.try_wait().unwrap() {
            assert!(status.success());
            break;
        }
        assert!(Instant::now() < deadline, "owned relay did not shut down");
        std::thread::sleep(Duration::from_millis(10));
    }
    reader.join().unwrap();
}
