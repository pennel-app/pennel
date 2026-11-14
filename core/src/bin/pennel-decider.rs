fn main() {
    let mut arguments = std::env::args().collect::<Vec<_>>();
    if arguments.get(1).map(String::as_str) != Some("--pennel-decider-worker") {
        arguments.insert(1, "--pennel-decider-worker".into());
    }
    std::process::exit(pennel_core::managed_local_decider_worker_main(arguments).unwrap_or(2));
}
