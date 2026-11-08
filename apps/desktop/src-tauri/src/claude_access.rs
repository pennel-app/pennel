//! Claude-toegang: keeps a long poll to Pennel Cloud open while signed in and switched on, and
//! answers each relayed tool call from this computer's meetings. Never logs questions or answers.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use pennel_core::{PennelError, RelayAnswer, RelayRequest};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::diagnostics::Log;
use crate::state::Shared;

const LOG: Log = Log::new("claude");
const FIRST_BACKOFF: Duration = Duration::from_secs(2);
const MAX_BACKOFF: Duration = Duration::from_secs(60);
/// While idle the runner still looks again now and then, in case a change came without a poke.
const IDLE_RECHECK: Duration = Duration::from_secs(30);
/// The server allows four calls in flight per account; more would only queue here.
const MAX_WORKERS: usize = 4;
pub const STATUS_EVENT: &str = "claude-access-changed";

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ClaudeStatus {
    Off,
    Connecting,
    Connected { last_request_ms: Option<i64> },
    Error { message: String },
}

/// 2 s, doubling, capped at 60 s.
pub fn backoff(failures: u32) -> Duration {
    let factor = 1u32 << failures.saturating_sub(1).min(5);
    (FIRST_BACKOFF * factor).min(MAX_BACKOFF)
}

pub fn should_run(signed_in: bool, enabled: bool) -> bool {
    signed_in && enabled
}

pub trait Relay: Send + Sync + 'static {
    fn active(&self) -> bool;
    fn generation(&self) -> u64 {
        0
    }
    fn permitted(&self, _request: &RelayRequest) -> bool {
        self.active()
    }
    fn revalidate(&self, request: &RelayRequest, answer: RelayAnswer) -> RelayAnswer {
        if self.permitted(request) {
            answer
        } else {
            disabled()
        }
    }
    fn poll(&self) -> Result<Vec<RelayRequest>, PennelError>;
    fn answer(&self, request: &RelayRequest) -> RelayAnswer;
    fn respond(&self, id: &str, answer: &RelayAnswer) -> Result<(), PennelError>;
}

#[derive(Default)]
struct Signals {
    poked: bool,
    stop: bool,
}

struct Inner {
    signals: Mutex<Signals>,
    wake: Condvar,
    status: Mutex<ClaudeStatus>,
    workers: AtomicUsize,
}

impl Inner {
    /// Sleeps until poked, stopped or `timeout`; true when the runner must end.
    fn sleep(&self, timeout: Duration) -> bool {
        let mut signals = self.signals.lock().unwrap();
        if !signals.poked && !signals.stop {
            signals = self.wake.wait_timeout(signals, timeout).unwrap().0;
        }
        signals.poked = false;
        signals.stop
    }

    fn stopped(&self) -> bool {
        self.signals.lock().unwrap().stop
    }
}

pub struct ClaudeAccessRunner {
    inner: Arc<Inner>,
    thread: Mutex<Option<JoinHandle<()>>>,
}

impl ClaudeAccessRunner {
    pub fn start(shared: Arc<Shared>, app: AppHandle) -> Self {
        Self::with_relay(Arc::new(EngineRelay(shared)), move |status| {
            let _ = app.emit(STATUS_EVENT, status);
        })
    }

    fn with_relay(
        relay: Arc<dyn Relay>,
        on_status: impl Fn(ClaudeStatus) + Send + Sync + 'static,
    ) -> Self {
        let inner = Arc::new(Inner {
            signals: Mutex::new(Signals::default()),
            wake: Condvar::new(),
            status: Mutex::new(ClaudeStatus::Off),
            workers: AtomicUsize::new(0),
        });
        let on_status = Arc::new(on_status);
        let worker = inner.clone();
        let thread = std::thread::spawn(move || run(worker, relay, on_status));
        Self {
            inner,
            thread: Mutex::new(Some(thread)),
        }
    }

    pub fn status(&self) -> ClaudeStatus {
        self.inner.status.lock().unwrap().clone()
    }

    pub fn poke(&self) {
        self.inner.signals.lock().unwrap().poked = true;
        self.inner.wake.notify_all();
    }

    /// Returns at once; a poll in flight ends on its own and its requests are not answered.
    pub fn stop(&self) {
        self.inner.signals.lock().unwrap().stop = true;
        self.inner.wake.notify_all();
        drop(self.thread.lock().unwrap().take());
    }
}

fn set_status(inner: &Inner, on_status: &(dyn Fn(ClaudeStatus) + Send + Sync), next: ClaudeStatus) {
    let mut status = inner.status.lock().unwrap();
    if *status != next {
        *status = next.clone();
        drop(status);
        on_status(next);
    }
}

fn last_request(inner: &Inner) -> Option<i64> {
    match &*inner.status.lock().unwrap() {
        ClaudeStatus::Connected { last_request_ms } => *last_request_ms,
        _ => None,
    }
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn run(
    inner: Arc<Inner>,
    relay: Arc<dyn Relay>,
    on_status: Arc<dyn Fn(ClaudeStatus) + Send + Sync>,
) {
    let mut failures = 0u32;
    loop {
        if inner.stopped() {
            return;
        }
        if !relay.active() {
            failures = 0;
            set_status(&inner, on_status.as_ref(), ClaudeStatus::Off);
            if inner.sleep(IDLE_RECHECK) {
                return;
            }
            continue;
        }
        if !matches!(
            *inner.status.lock().unwrap(),
            ClaudeStatus::Connected { .. }
        ) {
            set_status(&inner, on_status.as_ref(), ClaudeStatus::Connecting);
        }
        let generation = relay.generation();
        match relay.poll() {
            Ok(requests) => {
                failures = 0;
                if inner.stopped() {
                    return;
                }
                if generation != relay.generation() {
                    continue;
                }
                let last = if requests.is_empty() {
                    last_request(&inner)
                } else {
                    Some(now_ms())
                };
                set_status(
                    &inner,
                    on_status.as_ref(),
                    ClaudeStatus::Connected {
                        last_request_ms: last,
                    },
                );
                for request in requests {
                    dispatch(&inner, &relay, request, generation);
                }
            }
            Err(PennelError::NotSignedIn) => {
                set_status(&inner, on_status.as_ref(), ClaudeStatus::Off);
                if inner.sleep(IDLE_RECHECK) {
                    return;
                }
            }
            Err(error) => {
                failures += 1;
                LOG.error(format!("relay unreachable (attempt {failures})"));
                set_status(
                    &inner,
                    on_status.as_ref(),
                    ClaudeStatus::Error {
                        message: error.to_string(),
                    },
                );
                if inner.sleep(backoff(failures)) {
                    return;
                }
            }
        }
    }
}

fn disabled() -> RelayAnswer {
    RelayAnswer::Failed {
        code: "unavailable".into(),
        message: "Access is disabled or the requested profile is no longer current.".into(),
    }
}

fn dispatch(inner: &Arc<Inner>, relay: &Arc<dyn Relay>, request: RelayRequest, generation: u64) {
    if inner.workers.fetch_add(1, Ordering::SeqCst) >= MAX_WORKERS {
        inner.workers.fetch_sub(1, Ordering::SeqCst);
        let busy = RelayAnswer::Failed {
            code: "unavailable".into(),
            message: "This computer is still processing other questions. Try again shortly.".into(),
        };
        let _ = relay.respond(&request.id, &busy);
        return;
    }
    let inner = inner.clone();
    let relay = relay.clone();
    std::thread::spawn(move || {
        let started = std::time::Instant::now();
        let answer = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            if !inner.stopped() && relay.generation() == generation && relay.permitted(&request) {
                relay.answer(&request)
            } else {
                disabled()
            }
        }))
        .unwrap_or_else(|_| RelayAnswer::Failed {
            code: "failed".into(),
            message: "Pennel could not answer this question.".into(),
        });
        let mut answer = if inner.stopped() || relay.generation() != generation {
            disabled()
        } else {
            relay.revalidate(&request, answer)
        };
        if inner.stopped() || relay.generation() != generation {
            answer = disabled();
        }
        let outcome = match &answer {
            RelayAnswer::Text { .. } => "ok",
            RelayAnswer::Failed { code, .. } => code.as_str(),
        };
        let sent = relay.respond(&request.id, &answer).is_ok();
        LOG.info(format!(
            "{} {outcome} in {} ms{}",
            request.tool,
            started.elapsed().as_millis(),
            if sent { "" } else { ", response not delivered" }
        ));
        inner.workers.fetch_sub(1, Ordering::SeqCst);
    });
}

struct EngineRelay(Arc<Shared>);

impl EngineRelay {
    fn capabilities(&self) -> Vec<String> {
        let mut capabilities = Vec::new();
        if self.0.settings().claude_access {
            capabilities.extend(
                [
                    "search_meetings",
                    "list_recent_meetings",
                    "get_meeting",
                    "ask_meetings",
                ]
                .map(str::to_owned),
            );
        }
        if self.0.engine.writing_profile_sharing_enabled() {
            capabilities.push("get_writing_profile".into());
        }
        capabilities
    }
}

impl Relay for EngineRelay {
    fn active(&self) -> bool {
        should_run(
            self.0.engine.cloud_signed_in(),
            self.0.settings().claude_access || self.0.engine.writing_profile_sharing_enabled(),
        )
    }

    fn generation(&self) -> u64 {
        self.0.engine.cloud_account_epoch()
    }

    fn permitted(&self, request: &RelayRequest) -> bool {
        self.0.engine.cloud_signed_in()
            && if request.tool == "get_writing_profile" {
                self.0.engine.writing_profile_sharing_enabled()
            } else {
                self.0.settings().claude_access
            }
    }

    fn revalidate(&self, request: &RelayRequest, answer: RelayAnswer) -> RelayAnswer {
        if !self.permitted(request) {
            return disabled();
        }
        if request.tool == "get_writing_profile" {
            // A source may have been deleted while the first snapshot was being prepared.
            let current = self.answer(request);
            if current != answer {
                return disabled();
            }
        }
        answer
    }

    fn poll(&self) -> Result<Vec<RelayRequest>, PennelError> {
        self.0.engine.cloud_relay_poll_with_capabilities(
            &format!("pennel-desktop/{}", self.0.version),
            &self.capabilities(),
        )
    }

    fn answer(&self, request: &RelayRequest) -> RelayAnswer {
        if !self.permitted(request) {
            return disabled();
        }
        if request.tool == "get_writing_profile" {
            let Some(language) = crate::writing_profile::profile_language(&request.arguments_json)
            else {
                return RelayAnswer::Failed {
                    code: "invalid_arguments".into(),
                    message: "Supply only a valid output language.".into(),
                };
            };
            return match self.0.engine.writing_profile_shared(&language) {
                Ok(profile) => RelayAnswer::Text {
                    text: profile.to_string(),
                },
                Err(_) => disabled(),
            };
        }
        self.0.engine.claude_access_answer(
            request.tool.clone(),
            request.arguments_json.clone(),
            self.0.keys(),
        )
    }

    fn respond(&self, id: &str, answer: &RelayAnswer) -> Result<(), PennelError> {
        self.0.engine.cloud_relay_respond(id.into(), answer.clone())
    }
}

pub fn poke(app: &AppHandle) {
    if let Some(runner) = app.try_state::<ClaudeAccessRunner>() {
        runner.poke();
    }
}

#[derive(Serialize)]
pub struct ClaudeAccessView {
    enabled: bool,
    status: ClaudeStatus,
    mcp_url: String,
}

fn view(app: &AppHandle, shared: &Shared) -> ClaudeAccessView {
    let status = app
        .try_state::<ClaudeAccessRunner>()
        .map_or(ClaudeStatus::Off, |runner| runner.status());
    ClaudeAccessView {
        enabled: shared.settings().claude_access,
        status,
        mcp_url: shared.engine.cloud_mcp_url(),
    }
}

#[tauri::command]
pub fn claude_access(app: AppHandle, shared: State<'_, Arc<Shared>>) -> ClaudeAccessView {
    view(&app, &shared)
}

#[tauri::command]
pub async fn set_claude_access(
    enabled: bool,
    app: AppHandle,
    shared: State<'_, Arc<Shared>>,
) -> Result<ClaudeAccessView, crate::cloud::CloudError> {
    let engine = shared.engine.clone();
    tauri::async_runtime::spawn_blocking(move || engine.set_claude_access(enabled))
        .await
        .map_err(crate::cloud::CloudError::other)?
        .map_err(crate::cloud::CloudError::from)?;
    poke(&app);
    let _ = app.emit("settings-changed", ());
    Ok(view(&app, &shared))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicBool, AtomicU64};
    use std::time::Instant;

    fn wait_until(mut condition: impl FnMut() -> bool) {
        let start = Instant::now();
        while !condition() {
            assert!(start.elapsed() < Duration::from_secs(5), "timed out");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[derive(Default)]
    struct Fake {
        active: AtomicBool,
        polls: Mutex<VecDeque<Result<Vec<RelayRequest>, PennelError>>>,
        poll_count: AtomicUsize,
        answered: Mutex<Vec<(String, RelayAnswer)>>,
        off_after_poll: AtomicBool,
        epoch: AtomicU64,
        change_identity_after_poll: AtomicBool,
        off_after_answer: AtomicBool,
        change_identity_after_answer: AtomicBool,
    }

    impl Relay for Fake {
        fn generation(&self) -> u64 {
            self.epoch.load(Ordering::SeqCst)
        }
        fn active(&self) -> bool {
            self.active.load(Ordering::SeqCst)
        }
        fn poll(&self) -> Result<Vec<RelayRequest>, PennelError> {
            self.poll_count.fetch_add(1, Ordering::SeqCst);
            if self.off_after_poll.load(Ordering::SeqCst) {
                self.active.store(false, Ordering::SeqCst);
            }
            if self
                .change_identity_after_poll
                .swap(false, Ordering::SeqCst)
            {
                self.epoch.fetch_add(1, Ordering::SeqCst);
            }
            let next = self.polls.lock().unwrap().pop_front();
            next.unwrap_or_else(|| {
                std::thread::sleep(Duration::from_millis(5));
                Ok(Vec::new())
            })
        }
        fn answer(&self, request: &RelayRequest) -> RelayAnswer {
            if self.off_after_answer.load(Ordering::SeqCst) {
                self.active.store(false, Ordering::SeqCst);
            }
            if self
                .change_identity_after_answer
                .swap(false, Ordering::SeqCst)
            {
                self.epoch.fetch_add(1, Ordering::SeqCst);
            }
            RelayAnswer::Text {
                text: format!("antwoord op {}", request.tool),
            }
        }
        fn respond(&self, id: &str, answer: &RelayAnswer) -> Result<(), PennelError> {
            self.answered
                .lock()
                .unwrap()
                .push((id.to_owned(), answer.clone()));
            Ok(())
        }
    }

    fn request(id: &str) -> RelayRequest {
        RelayRequest {
            id: id.into(),
            tool: "list_recent_meetings".into(),
            arguments_json: "{}".into(),
            timeout_ms: 20_000,
        }
    }

    fn runner(fake: &Arc<Fake>) -> (ClaudeAccessRunner, Arc<Mutex<Vec<ClaudeStatus>>>) {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let sink = seen.clone();
        let runner = ClaudeAccessRunner::with_relay(fake.clone(), move |status| {
            sink.lock().unwrap().push(status)
        });
        (runner, seen)
    }

    #[test]
    fn runs_only_signed_in_and_switched_on() {
        assert!(should_run(true, true));
        assert!(!should_run(true, false));
        assert!(!should_run(false, true));
    }

    #[test]
    fn production_profile_relay_preserves_independent_meeting_permissions() {
        let directory = tempfile::tempdir().unwrap();
        let shared = Shared::testing_production(directory.path().to_path_buf());
        shared
            .secrets
            .set("pennel-cloud-refresh", "synthetic-session")
            .unwrap();
        shared
            .engine
            .set_secret_store(shared.secrets.clone())
            .unwrap();
        let relay = EngineRelay(shared.clone());
        let profile = RelayRequest {
            tool: "get_writing_profile".into(),
            arguments_json: r#"{"language":"en"}"#.into(),
            ..request("profile")
        };
        assert!(!relay.active());
        assert!(!relay.permitted(&profile));
        assert!(relay.capabilities().is_empty());

        shared.engine.set_writing_profile_sharing(true).unwrap();
        assert!(relay.active());
        assert!(relay.permitted(&profile));
        assert!(!relay.permitted(&request("meetings")));
        assert_eq!(relay.capabilities(), ["get_writing_profile"]);
        assert!(
            matches!(relay.answer(&profile), RelayAnswer::Text { text } if text.contains("insufficient_evidence"))
        );

        shared.engine.set_writing_profile_sharing(false).unwrap();
        let mut settings = shared.settings();
        settings.claude_access = true;
        shared.engine.update_settings(settings).unwrap();
        assert!(relay.active());
        assert!(!relay.permitted(&profile));
        assert!(relay.permitted(&request("meetings")));
        assert_eq!(relay.capabilities().len(), 4);
        assert!(
            !relay
                .capabilities()
                .iter()
                .any(|tool| tool == "get_writing_profile")
        );
        assert!(matches!(relay.answer(&profile), RelayAnswer::Failed { .. }));
    }

    #[test]
    fn backoff_doubles_from_two_seconds_to_a_minute() {
        let secs: Vec<u64> = (1..=8).map(|n| backoff(n).as_secs()).collect();
        assert_eq!(secs, [2, 4, 8, 16, 32, 60, 60, 60]);
        assert_eq!(backoff(u32::MAX), MAX_BACKOFF);
    }

    #[test]
    fn idle_runner_never_polls() {
        let fake = Arc::new(Fake::default());
        let (runner, _) = runner(&fake);
        std::thread::sleep(Duration::from_millis(50));
        assert_eq!(fake.poll_count.load(Ordering::SeqCst), 0);
        assert_eq!(runner.status(), ClaudeStatus::Off);
        runner.stop();
    }

    #[test]
    fn answers_relayed_requests_once_switched_on() {
        let fake = Arc::new(Fake::default());
        fake.polls
            .lock()
            .unwrap()
            .push_back(Ok(vec![request("a"), request("b")]));
        let (runner, _) = runner(&fake);
        fake.active.store(true, Ordering::SeqCst);
        runner.poke();
        wait_until(|| fake.answered.lock().unwrap().len() == 2);
        let mut ids: Vec<String> = fake
            .answered
            .lock()
            .unwrap()
            .iter()
            .map(|(id, _)| id.clone())
            .collect();
        ids.sort();
        assert_eq!(ids, ["a", "b"]);
        assert!(matches!(
            runner.status(),
            ClaudeStatus::Connected {
                last_request_ms: Some(_)
            }
        ));
        runner.stop();
    }

    #[test]
    fn switching_off_stops_polling() {
        let fake = Arc::new(Fake::default());
        fake.active.store(true, Ordering::SeqCst);
        let (runner, seen) = runner(&fake);
        wait_until(|| fake.poll_count.load(Ordering::SeqCst) > 2);
        fake.active.store(false, Ordering::SeqCst);
        runner.poke();
        wait_until(|| runner.status() == ClaudeStatus::Off);
        let polls = fake.poll_count.load(Ordering::SeqCst);
        std::thread::sleep(Duration::from_millis(50));
        assert!(fake.poll_count.load(Ordering::SeqCst) <= polls + 1);
        assert!(seen.lock().unwrap().contains(&ClaudeStatus::Connecting));
        runner.stop();
    }

    #[test]
    fn a_request_arriving_after_switch_off_is_refused_not_run() {
        let fake = Arc::new(Fake::default());
        fake.polls
            .lock()
            .unwrap()
            .push_back(Ok(vec![request("late")]));
        fake.off_after_poll.store(true, Ordering::SeqCst);
        fake.active.store(true, Ordering::SeqCst);
        let (runner, _) = runner(&fake);
        wait_until(|| !fake.answered.lock().unwrap().is_empty());
        let (id, answer) = fake.answered.lock().unwrap()[0].clone();
        assert_eq!(id, "late");
        assert!(matches!(answer, RelayAnswer::Failed { code, .. } if code == "unavailable"));
        runner.stop();
    }

    #[test]
    fn disabling_or_switching_identity_during_answer_withholds_content() {
        for switch_identity in [false, true] {
            let fake = Arc::new(Fake::default());
            fake.active.store(true, Ordering::SeqCst);
            fake.off_after_answer
                .store(!switch_identity, Ordering::SeqCst);
            fake.change_identity_after_answer
                .store(switch_identity, Ordering::SeqCst);
            fake.polls
                .lock()
                .unwrap()
                .push_back(Ok(vec![request("inflight")]));
            let (runner, _) = runner(&fake);
            wait_until(|| !fake.answered.lock().unwrap().is_empty());
            assert!(
                matches!(&fake.answered.lock().unwrap()[0].1, RelayAnswer::Failed {code, ..} if code == "unavailable")
            );
            runner.stop();
        }
    }

    #[test]
    fn account_change_during_poll_discards_old_requests_and_runner_recovers() {
        let fake = Arc::new(Fake::default());
        fake.active.store(true, Ordering::SeqCst);
        fake.change_identity_after_poll
            .store(true, Ordering::SeqCst);
        fake.polls
            .lock()
            .unwrap()
            .push_back(Ok(vec![request("old-account")]));
        let (runner, _) = runner(&fake);
        wait_until(|| fake.poll_count.load(Ordering::SeqCst) > 1);
        assert!(fake.answered.lock().unwrap().is_empty());
        fake.polls
            .lock()
            .unwrap()
            .push_back(Ok(vec![request("new-account")]));
        wait_until(|| !fake.answered.lock().unwrap().is_empty());
        assert_eq!(fake.answered.lock().unwrap()[0].0, "new-account");
        runner.stop();
    }

    #[test]
    fn signed_out_goes_idle_without_backoff_errors() {
        let fake = Arc::new(Fake::default());
        fake.active.store(true, Ordering::SeqCst);
        fake.polls
            .lock()
            .unwrap()
            .push_back(Err(PennelError::NotSignedIn));
        let (runner, seen) = runner(&fake);
        wait_until(|| fake.poll_count.load(Ordering::SeqCst) >= 1);
        std::thread::sleep(Duration::from_millis(50));
        assert_eq!(runner.status(), ClaudeStatus::Off);
        assert_eq!(fake.poll_count.load(Ordering::SeqCst), 1);
        assert!(
            !seen
                .lock()
                .unwrap()
                .iter()
                .any(|s| matches!(s, ClaudeStatus::Error { .. }))
        );
        runner.stop();
    }

    #[test]
    fn network_errors_show_and_back_off() {
        let fake = Arc::new(Fake::default());
        fake.active.store(true, Ordering::SeqCst);
        fake.polls
            .lock()
            .unwrap()
            .push_back(Err(PennelError::Network {
                message: "offline".into(),
            }));
        let (runner, _) = runner(&fake);
        wait_until(|| matches!(runner.status(), ClaudeStatus::Error { .. }));
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(fake.poll_count.load(Ordering::SeqCst), 1, "waits 2 s");
        runner.poke();
        wait_until(|| matches!(runner.status(), ClaudeStatus::Connected { .. }));
        runner.stop();
    }

    #[test]
    fn stop_returns_promptly() {
        let fake = Arc::new(Fake::default());
        let (runner, _) = runner(&fake);
        let start = Instant::now();
        runner.stop();
        assert!(start.elapsed() < Duration::from_millis(500));
    }

    #[test]
    fn status_serialises_with_a_kind() {
        let json = serde_json::to_value(ClaudeStatus::Connected {
            last_request_ms: Some(5),
        })
        .unwrap();
        assert_eq!(json["kind"], "connected");
        assert_eq!(json["last_request_ms"], 5);
        let error = serde_json::to_value(ClaudeStatus::Error {
            message: "x".into(),
        })
        .unwrap();
        assert_eq!(error["kind"], "error");
    }
}
