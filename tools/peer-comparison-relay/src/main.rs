use std::io::Write;
use std::process::ExitCode;
use std::time::Duration;

use pennel_peer_comparison_relay::{MAX_HEADER_BYTES, Relay, RelayLimits, RelayOptions};
use tokio::io::{AsyncReadExt, BufReader};

fn options() -> Result<RelayOptions, ()> {
    let mut bind = None;
    let mut advertise_host = None;
    let mut peers = Vec::new();
    let mut candidate = None;
    let mut run_id = None;
    let mut ttl = None;
    let mut rate = None;
    let mut arguments = std::env::args().skip(1);
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--bind" if bind.is_none() => {
                bind = Some(arguments.next().ok_or(())?.parse().map_err(|_| ())?)
            }
            "--advertise-host" if advertise_host.is_none() => {
                advertise_host = Some(arguments.next().ok_or(())?)
            }
            "--allow" if peers.len() < 2 => peers.push(arguments.next().ok_or(())?),
            "--candidate" if candidate.is_none() => candidate = Some(arguments.next().ok_or(())?),
            "--run-id" if run_id.is_none() => run_id = Some(arguments.next().ok_or(())?),
            "--ttl-seconds" if ttl.is_none() => {
                ttl = Some(arguments.next().ok_or(())?.parse().map_err(|_| ())?)
            }
            "--rate-bytes-per-second" if rate.is_none() => {
                rate = Some(arguments.next().ok_or(())?.parse().map_err(|_| ())?)
            }
            _ => return Err(()),
        }
    }
    Ok(RelayOptions {
        bind: bind.ok_or(())?,
        advertise_host,
        peers: peers.try_into().map_err(|_| ())?,
        candidate: candidate.ok_or(())?,
        run_id: run_id.ok_or(())?,
        ticket_ttl: Duration::from_secs(ttl.unwrap_or(1800)),
        limits: RelayLimits {
            rate_bytes_per_second: rate.unwrap_or(0),
            ..RelayLimits::default()
        },
    })
}

fn emit(value: &impl serde::Serialize) -> Result<(), ()> {
    let mut output = std::io::stdout().lock();
    serde_json::to_writer(&mut output, value).map_err(|_| ())?;
    output.write_all(b"\n").map_err(|_| ())?;
    output.flush().map_err(|_| ())
}

async fn command(reader: &mut BufReader<tokio::io::Stdin>) -> Result<Option<String>, ()> {
    let mut bytes = Vec::new();
    loop {
        let byte = match reader.read_u8().await {
            Ok(byte) => byte,
            Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
            Err(_) => return Err(()),
        };
        if bytes.len() >= MAX_HEADER_BYTES {
            return Err(());
        }
        bytes.push(byte);
        if byte == b'\n' {
            break;
        }
    }
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Command {
        command: String,
    }
    serde_json::from_slice::<Command>(&bytes)
        .map(|command| Some(command.command))
        .map_err(|_| ())
}

#[tokio::main]
async fn main() -> ExitCode {
    let Ok(options) = options() else {
        eprintln!(
            "Usage: pennel-peer-comparison-relay --bind IP:PORT --allow PEER_ID --allow PEER_ID --candidate quinn|native --run-id ID [--advertise-host HOST] [--ttl-seconds N] [--rate-bytes-per-second N]"
        );
        return ExitCode::FAILURE;
    };
    let Ok(relay) = Relay::bind(options).await else {
        eprintln!("relay_start_failed");
        return ExitCode::FAILURE;
    };
    let ready = relay.ready();
    if ready.as_ref().map_err(|_| ()).and_then(emit).is_err() {
        relay.shutdown().await;
        return ExitCode::FAILURE;
    }
    let mut input = BufReader::new(tokio::io::stdin());
    let outcome = loop {
        tokio::select! {
            signal = tokio::signal::ctrl_c() => break signal.map_err(|_| ()),
            command = command(&mut input) => match command {
                Ok(Some(command)) if command == "stats" => {
                    if emit(&serde_json::json!({"event":"stats", "result": relay.stats()})).is_err() { break Err(()); }
                }
                Ok(Some(command)) if command == "revoke" => {
                    relay.revoke();
                    if emit(&serde_json::json!({"event":"revoked"})).is_err() { break Err(()); }
                }
                Ok(Some(command)) if command == "pause" => {
                    relay.pause();
                    if emit(&serde_json::json!({"event":"paused","result":{"paused":true}})).is_err() { break Err(()); }
                }
                Ok(Some(command)) if command == "resume" => {
                    relay.resume();
                    if emit(&serde_json::json!({"event":"resumed","result":{"paused":false}})).is_err() { break Err(()); }
                }
                Ok(Some(command)) if command == "shutdown" => break Ok(()),
                Ok(None) => break Ok(()),
                _ => break Err(()),
            }
        }
    };
    relay.shutdown().await;
    if outcome.is_ok() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
