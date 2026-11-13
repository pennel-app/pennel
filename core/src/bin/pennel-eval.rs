use std::io::{BufRead, Read, Write};

enum Execution {
    Offline,
    Cloud(Box<pennel_core::evaluation::CloudReviewClient>),
    Local(Box<pennel_core::evaluation::LocalReviewClient>),
}

fn run(mut execution: Execution) -> std::io::Result<()> {
    let mut input = std::io::stdin().lock();
    let mut output = std::io::stdout().lock();
    loop {
        let mut line = String::new();
        let count = input
            .by_ref()
            .take(pennel_core::evaluation::MAX_REQUEST_BYTES as u64 + 1)
            .read_line(&mut line)?;
        if count == 0 {
            return Ok(());
        }
        if line.len() > pennel_core::evaluation::MAX_REQUEST_BYTES {
            writeln!(output, "{{\"error\":\"request_too_large\"}}")?;
            return Ok(());
        }
        let result = serde_json::from_str(&line)
            .map_err(|_| "invalid_json")
            .and_then(|value| match &mut execution {
                Execution::Cloud(client) => client.evaluate(value),
                Execution::Local(client) => client.evaluate(value),
                Execution::Offline => pennel_core::evaluation::evaluate(value),
            })
            .unwrap_or_else(|code| serde_json::json!({"error":code}));
        serde_json::to_writer(&mut output, &result)?;
        writeln!(output)?;
        output.flush()?;
    }
}

fn local_arguments(
    arguments: &[String],
) -> Result<(std::path::PathBuf, std::path::PathBuf, std::path::PathBuf), ()> {
    if arguments.len() != 7
        || arguments[0] != "--execute-local-review"
        || arguments[1] != "--runtime-bundle"
        || arguments[3] != "--worker"
        || arguments[5] != "--model"
        || [2, 4, 6]
            .iter()
            .any(|index| arguments[*index].is_empty() || arguments[*index].starts_with('-'))
    {
        return Err(());
    }
    Ok((
        arguments[2].clone().into(),
        arguments[4].clone().into(),
        arguments[6].clone().into(),
    ))
}

fn session_argument(arguments: &[String]) -> Result<Option<i32>, ()> {
    if arguments.is_empty() {
        return Ok(None);
    }
    if arguments.len() == 3
        && arguments[0] == "--execute-cloud-review"
        && arguments[1] == "--session-fd"
    {
        return arguments[2]
            .parse::<i32>()
            .ok()
            .filter(|fd| *fd >= 3)
            .map(Some)
            .ok_or(());
    }
    Err(())
}

#[cfg(unix)]
fn session_from_fd(fd: i32) -> Result<zeroize::Zeroizing<Vec<u8>>, ()> {
    use std::os::fd::FromRawFd;
    use std::os::unix::fs::{FileTypeExt, MetadataExt};
    let file = unsafe { std::fs::File::from_raw_fd(fd) };
    let metadata = file.metadata().map_err(|_| ())?;
    if !metadata.file_type().is_fifo() || metadata.nlink() != 0 {
        return Err(());
    }
    #[cfg(target_os = "linux")]
    if std::fs::read_link(format!("/proc/self/fd/{fd}")).map_err(|_| ())?
        != std::path::PathBuf::from(format!("pipe:[{}]", metadata.ino()))
    {
        return Err(());
    }
    let (send, receive) = std::sync::mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let mut secret = zeroize::Zeroizing::new(Vec::new());
        let result = file
            .take(64 * 1024 + 1)
            .read_to_end(&mut secret)
            .map_err(|_| ())
            .and_then(|_| {
                if secret.is_empty() || secret.len() > 64 * 1024 {
                    Err(())
                } else {
                    Ok(secret)
                }
            });
        let _ = send.send(result);
    });
    receive
        .recv_timeout(std::time::Duration::from_secs(15))
        .map_err(|_| ())?
}

#[cfg(not(unix))]
fn session_from_fd(_fd: i32) -> Result<zeroize::Zeroizing<Vec<u8>>, ()> {
    Err(())
}

fn start() -> Result<(), ()> {
    let arguments = std::env::args().skip(1).collect::<Vec<_>>();
    if arguments.first().map(String::as_str) == Some("--execute-local-review") {
        let (bundle, worker, model) = local_arguments(&arguments)?;
        let local = pennel_core::evaluation::LocalReviewClient::new(bundle, worker, model)
            .map_err(|_| ())?;
        return run(Execution::Local(Box::new(local))).map_err(|_| ());
    }
    let execution = match session_argument(&arguments)? {
        None => Execution::Offline,
        Some(fd) => {
            let secret = session_from_fd(fd)?;
            Execution::Cloud(Box::new(
                pennel_core::evaluation::CloudReviewClient::from_session_json(&secret)
                    .map_err(|_| ())?,
            ))
        }
    };
    run(execution).map_err(|_| ())
}

fn main() {
    if start().is_err() {
        eprintln!("Evaluation bridge input/output failure.");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn execution_and_an_inherited_descriptor_are_both_explicit() {
        assert_eq!(session_argument(&[]), Ok(None));
        let args = |values: &[&str]| {
            values
                .iter()
                .map(|value| (*value).to_owned())
                .collect::<Vec<_>>()
        };
        assert_eq!(
            session_argument(&args(&["--execute-cloud-review", "--session-fd", "3"])),
            Ok(Some(3))
        );
        for values in [
            vec!["--session-fd", "3"],
            vec!["--execute-cloud-review", "--session-fd", "0"],
            vec!["--execute-cloud-review", "--session-fd", "not-a-descriptor"],
        ] {
            assert!(session_argument(&args(&values)).is_err());
        }
    }

    #[test]
    fn local_execution_requires_every_owned_path_and_cannot_mix_cloud_credentials() {
        let args = |values: &[&str]| {
            values
                .iter()
                .map(|value| (*value).to_owned())
                .collect::<Vec<_>>()
        };
        assert!(
            local_arguments(&args(&[
                "--execute-local-review",
                "--runtime-bundle",
                "/owned/runtime",
                "--worker",
                "/owned/worker",
                "--model",
                "/owned/model.gguf"
            ]))
            .is_ok()
        );
        for value in [
            args(&["--execute-local-review"]),
            args(&[
                "--runtime-bundle",
                "/owned/runtime",
                "--worker",
                "/owned/worker",
                "--model",
                "/owned/model.gguf",
            ]),
            args(&[
                "--execute-local-review",
                "--runtime-bundle",
                "/owned/runtime",
                "--worker",
                "/owned/worker",
                "--session-fd",
                "3",
            ]),
        ] {
            assert!(local_arguments(&value).is_err());
            assert!(session_argument(&value).is_err());
        }
    }

    #[cfg(unix)]
    #[test]
    fn session_descriptor_rejects_regular_files_without_reading_their_contents() {
        use std::os::fd::IntoRawFd;
        let file = std::fs::File::open(std::env::current_exe().unwrap()).unwrap();
        assert!(session_from_fd(file.into_raw_fd()).is_err());
    }
}
