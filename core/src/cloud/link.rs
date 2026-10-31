//! One-use device connection. The service relays PAKE and sealed messages; the private
//! connection password and library key never form part of an HTTP request in plaintext.

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use base64::Engine;
use base64::engine::general_purpose::{STANDARD as B64, URL_SAFE_NO_PAD};
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use hkdf::Hkdf;
use hmac::{Hmac, Mac};
use qrcodegen::{QrCode, QrCodeEcc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use spake2::{Ed25519Group, Identity, Password, Spake2};
use zeroize::{Zeroize, Zeroizing};

use super::account::Session;
use super::crypto::VaultKey;
use super::http::{agent, error_from, send_json};
use super::{VAULT_KEY_ACCOUNT, VAULT_OWNER_ACCOUNT, store_verified};
use crate::{PennelEngine, PennelError};

mod provisioning;

pub(crate) use provisioning::has_pending;

const PROTOCOL: &str = "pennel-device-link-v1";
const MAX_ENVELOPE: usize = 8_192;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, uniffi::Enum)]
#[serde(rename_all = "snake_case")]
pub enum CloudLinkRole {
    NewDevice,
    TrustedDevice,
}

impl CloudLinkRole {
    fn wire(self) -> &'static str {
        match self {
            Self::NewDevice => "new",
            Self::TrustedDevice => "trusted",
        }
    }
    fn peer(self) -> Self {
        match self {
            Self::NewDevice => Self::TrustedDevice,
            Self::TrustedDevice => Self::NewDevice,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, uniffi::Enum)]
#[serde(rename_all = "snake_case")]
pub enum CloudLinkPhase {
    Idle,
    WaitingPeer,
    Confirming,
    AwaitingApproval,
    WaitingApproval,
    Provisioning,
    Complete,
    Cancelled,
    Expired,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
pub struct CloudLinkAvailability {
    pub available: bool,
    pub reason_code: Option<String>,
}

impl CloudLinkAvailability {
    fn unavailable(reason: &str) -> Self {
        Self {
            available: false,
            reason_code: Some(reason.into()),
        }
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
pub struct CloudLinkView {
    pub phase: CloudLinkPhase,
    pub role: Option<CloudLinkRole>,
    pub request_id: Option<String>,
    pub connection_code: Option<String>,
    pub qr_payload: Option<String>,
    pub qr_svg: Option<String>,
    pub verification_code: Option<String>,
    pub peer_name: Option<String>,
    pub peer_platform: Option<String>,
    pub account_email: Option<String>,
    pub expires_at_ms: Option<i64>,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
}

impl std::fmt::Debug for CloudLinkView {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CloudLinkView")
            .field("phase", &self.phase)
            .field("role", &self.role)
            .finish_non_exhaustive()
    }
}

impl Default for CloudLinkView {
    fn default() -> Self {
        Self {
            phase: CloudLinkPhase::Idle,
            role: None,
            request_id: None,
            connection_code: None,
            qr_payload: None,
            qr_svg: None,
            verification_code: None,
            peer_name: None,
            peer_platform: None,
            account_email: None,
            expires_at_ms: None,
            error_code: None,
            error_message: None,
        }
    }
}

#[derive(Default)]
pub(crate) struct LinkFlow {
    pub(super) generation: u64,
    in_flight: bool,
    active: Option<LinkSession>,
    remote: Option<Remote>,
    view: CloudLinkView,
}

struct BusyGuard<'a> {
    engine: &'a PennelEngine,
    generation: u64,
}
impl Drop for BusyGuard<'_> {
    fn drop(&mut self) {
        let mut flow = self.engine.cloud_link.lock().unwrap();
        if flow.generation == self.generation {
            flow.in_flight = false;
        }
    }
}

#[derive(Clone)]
struct Remote {
    id: String,
    capability: Zeroizing<String>,
    role: CloudLinkRole,
}

struct LinkSession {
    remote: Remote,
    password: Zeroizing<String>,
    expires_at_ms: i64,
    auth_generation: u64,
    flow_generation: u64,
    new_identity: String,
    context: Option<Context>,
    exchange: Option<Spake2<Ed25519Group>>,
    pake: Option<String>,
    keys: Option<Keys>,
    confirmation: Option<String>,
    peer_confirmed: bool,
    grant: Option<Zeroizing<String>>,
    envelope: Option<String>,
    provisioning: bool,
    pake_sent: bool,
    confirmation_sent: bool,
    envelope_sent: bool,
    view: CloudLinkView,
}

#[derive(Clone)]
struct Context {
    bytes: Vec<u8>,
    hash: String,
    account: String,
    approver: String,
    generation: u32,
    new_id: String,
    existing_new_device: Option<String>,
    email: String,
}

struct Keys {
    seal: Zeroizing<[u8; 32]>,
    new_confirm: Zeroizing<[u8; 32]>,
    trusted_confirm: Zeroizing<[u8; 32]>,
    verifier: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Transfer {
    version: u8,
    grant: String,
    vault_key: String,
    account_id: String,
    generation: u32,
    provisional_id: String,
    transcript_hash: String,
}

impl Drop for Transfer {
    fn drop(&mut self) {
        self.grant.zeroize();
        self.vault_key.zeroize();
    }
}

impl Transfer {
    fn valid_for(&self, context: &Context) -> bool {
        self.version == 1
            && self.account_id == context.account
            && self.generation == context.generation
            && self.provisional_id == context.new_id
            && self.transcript_hash == context.hash
            && self.grant.len() == 43
            && URL_SAFE_NO_PAD
                .decode(&self.grant)
                .is_ok_and(|bytes| bytes.len() == 32 && URL_SAFE_NO_PAD.encode(bytes) == self.grant)
            && VaultKey::from_b64(&self.vault_key).is_some()
    }
}

fn failure(code: &str, message: &str) -> PennelError {
    PennelError::Cloud {
        code: code.into(),
        message: message.into(),
    }
}
fn invalid() -> PennelError {
    failure(
        "pairing_invalid",
        "The connection could not be verified. Start a new connection.",
    )
}

fn provisioning_storage(error: &PennelError) -> bool {
    matches!(error, PennelError::Storage { .. })
        || matches!(error,PennelError::Cloud {code,..} if code == "pairing_storage")
}
fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}
fn random_token() -> String {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).expect("operating system randomness");
    URL_SAFE_NO_PAD.encode(bytes)
}
fn password() -> String {
    let mut value = [0u8; 8];
    getrandom::fill(&mut value).expect("operating system randomness");
    let number = u64::from_be_bytes(value);
    // Rejection avoids bias in the private eight-digit code.
    if number >= u64::MAX - (u64::MAX % 100_000_000) {
        return password();
    }
    format!("{:08}", number % 100_000_000)
}
fn field<'a>(value: &'a Value, key: &str) -> Result<&'a str, PennelError> {
    value.get(key).and_then(Value::as_str).ok_or_else(invalid)
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|v| format!("{v:02x}")).collect()
}

fn parse_code(input: &str) -> Result<(String, Zeroizing<String>), PennelError> {
    if input.len() > 128 {
        return Err(invalid());
    }
    let raw = input.trim();
    let canonical = if let Some(rest) = raw.strip_prefix("pennel-link:v1:") {
        let (locator, password) = rest.split_once(':').ok_or_else(invalid)?;
        format!("{locator}{password}")
    } else {
        raw.to_owned()
    };
    let chars: String = canonical
        .chars()
        .filter(|c| *c != '-' && !c.is_whitespace())
        .map(|c| c.to_ascii_uppercase())
        .collect();
    if !chars.is_ascii()
        || chars.len() != 16
        || !chars[..8].bytes().all(|c| c.is_ascii_alphanumeric())
        || !chars[8..].bytes().all(|c| c.is_ascii_digit())
    {
        return Err(failure(
            "pairing_code",
            "Enter the connection code shown on your other device.",
        ));
    }
    Ok((
        format!("{}-{}", &chars[..4], &chars[4..8]),
        Zeroizing::new(chars[8..].into()),
    ))
}

fn qr_svg(payload: &str) -> Result<String, PennelError> {
    let qr = QrCode::encode_text(payload, QrCodeEcc::Medium).map_err(|_| invalid())?;
    let size = qr.size();
    let mut path = String::new();
    for y in 0..size {
        for x in 0..size {
            if qr.get_module(x, y) {
                path.push_str(&format!("M{},{}h1v1h-1z", x + 4, y + 4));
            }
        }
    }
    Ok(format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {} {}\"><rect width=\"100%\" height=\"100%\" fill=\"white\"/><path d=\"{path}\" fill=\"black\"/></svg>",
        size + 8,
        size + 8
    ))
}

impl Context {
    fn from_snapshot(snapshot: &Value, base_url: &str) -> Result<Option<Self>, PennelError> {
        let Some(new) = snapshot.get("new_device").filter(|v| !v.is_null()) else {
            return Ok(None);
        };
        let Some(trusted) = snapshot.get("trusted_device").filter(|v| !v.is_null()) else {
            return Ok(None);
        };
        let generation = trusted
            .get("vault_generation")
            .and_then(Value::as_u64)
            .and_then(|n| u32::try_from(n).ok())
            .ok_or_else(invalid)?;
        let account = field(trusted, "user_id")?.to_string();
        let approver = field(trusted, "device_id")?.to_string();
        let new_id = field(new, "provisional_id")?.to_string();
        for id in [&account, &approver, &new_id] {
            uuid::Uuid::parse_str(id).map_err(|_| invalid())?;
        }
        let existing_account = new.get("user_id").and_then(Value::as_str).unwrap_or("");
        let existing_device = new.get("device_id").and_then(Value::as_str).unwrap_or("");
        if !existing_account.is_empty() && existing_account != account {
            return Err(invalid());
        }
        let bytes = serde_json::to_vec(&json!([
            PROTOCOL,
            base_url.trim_end_matches('/'),
            field(snapshot, "request_id")?,
            new_id,
            field(new, "device_name")?,
            field(new, "platform")?,
            account,
            approver,
            generation,
            field(trusted, "device_name")?,
            field(trusted, "platform")?,
            existing_account,
            existing_device
        ]))
        .map_err(|_| invalid())?;
        Ok(Some(Self {
            hash: hex(&Sha256::digest(&bytes)),
            bytes,
            account,
            approver,
            generation,
            new_id,
            existing_new_device: (!existing_device.is_empty()).then(|| existing_device.to_string()),
            email: trusted
                .get("account_email")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
        }))
    }
}

impl Keys {
    fn new(shared: &[u8], context: &Context) -> Result<Self, PennelError> {
        let hk = Hkdf::<Sha256>::new(Some(&Sha256::digest(&context.bytes)), shared);
        let derive = |label: &str| -> Result<Zeroizing<[u8; 32]>, PennelError> {
            let mut out = Zeroizing::new([0u8; 32]);
            hk.expand(format!("{PROTOCOL}/{label}").as_bytes(), out.as_mut())
                .map_err(|_| invalid())?;
            Ok(out)
        };
        let verifier_key = derive("display-verifier")?;
        let verifier = format!(
            "{:08}",
            u32::from_be_bytes(verifier_key[..4].try_into().expect("four bytes")) % 100_000_000
        );
        Ok(Self {
            seal: derive("transfer-envelope")?,
            new_confirm: derive("new-confirmation")?,
            trusted_confirm: derive("trusted-confirmation")?,
            verifier,
        })
    }
    fn confirm(&self, role: CloudLinkRole, context: &Context) -> String {
        let key = if role == CloudLinkRole::NewDevice {
            &self.new_confirm
        } else {
            &self.trusted_confirm
        };
        let mut mac = <Hmac<Sha256> as KeyInit>::new_from_slice(key.as_ref()).expect("HMAC key");
        mac.update(&context.bytes);
        B64.encode(mac.finalize().into_bytes())
    }
    fn verify(
        &self,
        role: CloudLinkRole,
        context: &Context,
        confirmation: &str,
    ) -> Result<(), PennelError> {
        let bytes = B64.decode(confirmation).map_err(|_| invalid())?;
        if bytes.len() != 32 {
            return Err(invalid());
        }
        let key = if role == CloudLinkRole::NewDevice {
            &self.new_confirm
        } else {
            &self.trusted_confirm
        };
        let mut mac = <Hmac<Sha256> as KeyInit>::new_from_slice(key.as_ref()).expect("HMAC key");
        mac.update(&context.bytes);
        mac.verify_slice(&bytes).map_err(|_| {
            failure(
                "pairing_code",
                "The connection codes do not match. Start a new connection.",
            )
        })
    }
    fn seal(&self, context: &Context, transfer: &Transfer) -> Result<String, PennelError> {
        let plain = Zeroizing::new(serde_json::to_vec(transfer).map_err(|_| invalid())?);
        let mut nonce = [0u8; 24];
        getrandom::fill(&mut nonce).map_err(PennelError::storage)?;
        let encrypted = XChaCha20Poly1305::new_from_slice(self.seal.as_ref())
            .map_err(|_| invalid())?
            .encrypt(
                &XNonce::from(nonce),
                Payload {
                    msg: plain.as_ref(),
                    aad: &context.bytes,
                },
            )
            .map_err(|_| invalid())?;
        let mut framed = vec![1];
        framed.extend_from_slice(&nonce);
        framed.extend(encrypted);
        Ok(B64.encode(framed))
    }
    fn open(&self, context: &Context, encoded: &str) -> Result<Transfer, PennelError> {
        if encoded.len() > MAX_ENVELOPE {
            return Err(invalid());
        }
        let framed = B64.decode(encoded).map_err(|_| invalid())?;
        if framed.len() < 1 + 24 + 16 || framed[0] != 1 {
            return Err(invalid());
        }
        let plain = Zeroizing::new(
            XChaCha20Poly1305::new_from_slice(self.seal.as_ref())
                .map_err(|_| invalid())?
                .decrypt(
                    &XNonce::try_from(&framed[1..25]).map_err(|_| invalid())?,
                    Payload {
                        msg: &framed[25..],
                        aad: &context.bytes,
                    },
                )
                .map_err(|_| invalid())?,
        );
        let transfer: Transfer = serde_json::from_slice(&plain).map_err(|_| invalid())?;
        if !transfer.valid_for(context) {
            return Err(invalid());
        }
        Ok(transfer)
    }
}

struct Client {
    session: Arc<Session>,
    http: ureq::Agent,
}
impl Client {
    fn new(session: Arc<Session>) -> Self {
        Self {
            session,
            http: agent(10),
        }
    }

    fn availability(&self) -> Result<CloudLinkAvailability, PennelError> {
        let response = match send_json(
            &self.http,
            "GET",
            &format!("{}/v1/device-links/capabilities", self.session.base_url()),
            None,
            None,
        ) {
            Err(PennelError::InvalidResponse { .. }) => {
                return Ok(CloudLinkAvailability::unavailable("pairing_incompatible"));
            }
            other => other?,
        };
        if matches!(response.0, 404 | 405)
            && matches!(
                response.1.get("error").and_then(Value::as_str),
                None | Some("" | "not_found" | "method_not_allowed")
            )
        {
            return Ok(CloudLinkAvailability::unavailable("pairing_unavailable"));
        }
        if !(200..300).contains(&response.0) {
            return Err(link_http_error(response.0, &response.1));
        }
        let supported = response.1.get("supported").and_then(Value::as_bool);
        if response.0 != 200
            || supported.is_none()
            || response.1.get("protocol").and_then(Value::as_str) != Some(PROTOCOL)
        {
            return Ok(CloudLinkAvailability::unavailable("pairing_incompatible"));
        }
        Ok(if supported == Some(true) {
            CloudLinkAvailability {
                available: true,
                reason_code: None,
            }
        } else {
            CloudLinkAvailability::unavailable("pairing_unavailable")
        })
    }

    fn call(
        &self,
        action: &str,
        remote: Option<&Remote>,
        body: &Value,
        authenticated: bool,
    ) -> Result<Value, PennelError> {
        let path = remote.map_or_else(
            || "/v1/device-links".to_string(),
            |r| format!("/v1/device-links/{}/{action}", r.id),
        );
        let url = format!("{}{path}", self.session.base_url());
        let token = if authenticated {
            Some(self.session.access_token()?)
        } else {
            None
        };
        let mut response = send_json(&self.http, "POST", &url, token.as_deref(), Some(body))?;
        if response.0 == 401 && authenticated {
            let token = self.session.refreshed_token()?;
            response = send_json(&self.http, "POST", &url, Some(&token), Some(body))?;
        }
        if !(200..300).contains(&response.0) {
            return Err(link_http_error(response.0, &response.1));
        }
        Ok(response.1)
    }
    fn poll(&self, remote: &Remote) -> Result<Value, PennelError> {
        self.call(
            "poll",
            Some(remote),
            &json!({"participant_token":remote.capability.as_str()}),
            false,
        )
    }
    fn message(
        &self,
        remote: &Remote,
        phase: &str,
        data: &str,
        context: &Context,
    ) -> Result<(), PennelError> {
        self.call("message",Some(remote),&json!({"participant_token":remote.capability.as_str(),"phase":phase,"data":data,"transcript_hash":context.hash}),false).map(drop)
    }
    fn cancel(&self, remote: &Remote) {
        let _ = self.call(
            "cancel",
            Some(remote),
            &json!({"participant_token":remote.capability.as_str()}),
            false,
        );
    }
}

fn link_http_error(status: u16, body: &Value) -> PennelError {
    match error_from(status, body) {
        PennelError::Cloud { code, message } if code == "unknown" || code.is_empty() => {
            PennelError::Cloud {
                code: format!("http_{status}"),
                message,
            }
        }
        error => error,
    }
}

fn link_error_code(error: &PennelError) -> String {
    match error {
        PennelError::Cloud { code, .. } => code.clone(),
        PennelError::Cancelled => "cancelled".into(),
        PennelError::Storage { .. } => "storage".into(),
        PennelError::Network { .. } => "network".into(),
        PennelError::Http { status, .. } => format!("http_{status}"),
        PennelError::NotSignedIn => "not_signed_in".into(),
        PennelError::VaultLocked => "vault_locked".into(),
        _ => "error".into(),
    }
}

fn retryable_link_error(error: &PennelError) -> bool {
    match error {
        PennelError::Network { .. } => true,
        PennelError::Http { status, .. } => matches!(status, 408 | 429 | 500..=599),
        PennelError::Cloud { code, .. } => {
            matches!(
                code.as_str(),
                "internal" | "pairing_retry" | "rate_limited" | "too_many" | "slow_down"
            ) || code
                .strip_prefix("http_")
                .and_then(|status| status.parse::<u16>().ok())
                .is_some_and(|status| matches!(status, 408 | 429 | 500..=599))
        }
        _ => false,
    }
}

impl LinkSession {
    fn check_live(&self, engine: &PennelEngine, client: &Client) -> Result<(), PennelError> {
        if engine.cloud_link.lock().unwrap().generation != self.flow_generation
            || client.session.generation() != self.auth_generation
        {
            return Err(PennelError::Cancelled);
        }
        Ok(())
    }

    fn poll(&mut self, engine: &PennelEngine, client: &Client) -> Result<(), PennelError> {
        if self.expires_at_ms <= now_ms() {
            return Err(failure(
                "pairing_expired",
                "This connection code expired. Start a new connection.",
            ));
        }
        self.check_live(engine, client)?;
        let snapshot = client.poll(&self.remote)?;
        self.check_live(engine, client)?;
        match field(&snapshot, "status")? {
            "cancelled" => return Err(PennelError::Cancelled),
            "expired" => {
                return Err(failure(
                    "pairing_expired",
                    "This connection code expired. Start a new connection.",
                ));
            }
            "failed" => return Err(invalid()),
            "complete" => {
                if self.remote.role == CloudLinkRole::NewDevice {
                    let mut receipt = provisioning::load(engine)?.ok_or_else(invalid)?;
                    provisioning::advance(engine, client, &mut receipt, Some(&snapshot), || {
                        self.check_live(engine, client)
                    })?;
                } else if !self.peer_confirmed || self.envelope.is_none() {
                    return Err(invalid());
                }
                self.view.phase = CloudLinkPhase::Complete;
                self.clear_display();
                return Ok(());
            }
            _ => {}
        }
        let Some(context) = Context::from_snapshot(&snapshot, client.session.base_url())? else {
            self.view.phase = CloudLinkPhase::WaitingPeer;
            return Ok(());
        };
        if self
            .context
            .as_ref()
            .is_some_and(|old| old.bytes != context.bytes)
        {
            return Err(invalid());
        }
        if self.remote.role == CloudLinkRole::NewDevice && context.new_id != self.new_identity {
            return Err(invalid());
        }
        if self.context.is_none() {
            let a_id = [b"new:".as_slice(), &context.bytes].concat();
            let b_id = [b"trusted:".as_slice(), &context.bytes].concat();
            let (exchange, outbound) = match self.remote.role {
                CloudLinkRole::NewDevice => Spake2::<Ed25519Group>::start_a(
                    &Password::new(self.password.as_bytes()),
                    &Identity::new(&a_id),
                    &Identity::new(&b_id),
                ),
                CloudLinkRole::TrustedDevice => Spake2::<Ed25519Group>::start_b(
                    &Password::new(self.password.as_bytes()),
                    &Identity::new(&a_id),
                    &Identity::new(&b_id),
                ),
            };
            self.exchange = Some(exchange);
            self.pake = Some(B64.encode(outbound));
            self.context = Some(context.clone());
        }
        let peer = if self.remote.role == CloudLinkRole::NewDevice {
            &snapshot["trusted_device"]
        } else {
            &snapshot["new_device"]
        };
        self.view.peer_name = Some(field(peer, "device_name")?.to_string());
        self.view.peer_platform = Some(field(peer, "platform")?.to_string());
        self.view.account_email = Some(context.email.clone());
        self.view.phase = CloudLinkPhase::Confirming;
        if !self.pake_sent {
            client.message(
                &self.remote,
                "pake",
                self.pake.as_deref().ok_or_else(invalid)?,
                &context,
            )?;
            self.pake_sent = true;
        }
        let peer_message = &snapshot["messages"][self.remote.role.peer().wire()];
        if self.keys.is_none() {
            let Some(pake) = peer_message.get("pake").and_then(Value::as_str) else {
                return Ok(());
            };
            let bytes = B64.decode(pake).map_err(|_| invalid())?;
            if bytes.len() != 33 {
                return Err(invalid());
            }
            let shared = Zeroizing::new(
                self.exchange
                    .take()
                    .ok_or_else(invalid)?
                    .finish(&bytes)
                    .map_err(|_| invalid())?,
            );
            self.keys = Some(Keys::new(&shared, &context)?);
            self.password.zeroize();
        }
        let keys = self.keys.as_ref().ok_or_else(invalid)?;
        self.view.verification_code = Some(keys.verifier.clone());
        let confirmation = self
            .confirmation
            .get_or_insert_with(|| keys.confirm(self.remote.role, &context));
        if !self.confirmation_sent {
            client.message(&self.remote, "confirmation", confirmation, &context)?;
            self.confirmation_sent = true;
        }
        let Some(peer_confirmation) = peer_message.get("confirmation").and_then(Value::as_str)
        else {
            return Ok(());
        };
        keys.verify(self.remote.role.peer(), &context, peer_confirmation)?;
        self.peer_confirmed = true;
        self.clear_display();
        if self.remote.role == CloudLinkRole::TrustedDevice {
            if !self.envelope_sent
                && let Some(envelope) = &self.envelope
            {
                client.message(&self.remote, "envelope", envelope, &context)?;
                self.envelope_sent = true;
            }
            self.view.phase = if self.envelope.is_some() {
                CloudLinkPhase::Provisioning
            } else {
                CloudLinkPhase::AwaitingApproval
            };
            return Ok(());
        }
        self.view.phase = CloudLinkPhase::WaitingApproval;
        let Some(envelope) = snapshot["messages"].get("envelope").and_then(Value::as_str) else {
            return Ok(());
        };
        if self.envelope.as_deref().is_some_and(|old| old != envelope) {
            return Err(invalid());
        }
        let transfer = self
            .keys
            .as_ref()
            .ok_or_else(invalid)?
            .open(&context, envelope)?;
        self.envelope = Some(envelope.to_owned());
        self.provision(engine, client, &snapshot, &context, &transfer)
    }

    fn approve(&mut self, engine: &PennelEngine, client: &Client) -> Result<(), PennelError> {
        self.poll(engine, client)?;
        if self.remote.role != CloudLinkRole::TrustedDevice || !self.peer_confirmed {
            return Err(invalid());
        }
        let context = self.context.clone().ok_or_else(invalid)?;
        let (account, device) = client.session.identity()?;
        if account != context.account || device != context.approver {
            return Err(PennelError::Cancelled);
        }
        let setup = engine.cloud_sync_setup()?;
        if !setup.can_approve_devices || setup.generation != Some(context.generation) {
            return Err(PennelError::VaultLocked);
        }
        let key = engine.vault_key().ok_or(PennelError::VaultLocked)?;
        self.check_live(engine, client)?;
        if self.grant.is_none() {
            self.grant = Some(Zeroizing::new(random_token()));
        }
        let grant = self.grant.as_ref().ok_or_else(invalid)?.clone();
        client.call(
            "approve",
            Some(&self.remote),
            &json!({"participant_token":self.remote.capability.as_str(),
            "transcript_hash":context.hash,"grant_token":grant.as_str()}),
            true,
        )?;
        self.check_live(engine, client)?;
        if self.envelope.is_none() {
            let transfer = Transfer {
                version: 1,
                grant: grant.to_string(),
                vault_key: key.to_b64(),
                account_id: account,
                generation: context.generation,
                provisional_id: context.new_id.clone(),
                transcript_hash: context.hash.clone(),
            };
            self.envelope = Some(
                self.keys
                    .as_ref()
                    .ok_or_else(invalid)?
                    .seal(&context, &transfer)?,
            );
        }
        if !self.envelope_sent {
            client.message(
                &self.remote,
                "envelope",
                self.envelope.as_deref().ok_or_else(invalid)?,
                &context,
            )?;
            self.envelope_sent = true;
        }
        self.view.phase = CloudLinkPhase::Provisioning;
        Ok(())
    }

    fn provision(
        &mut self,
        engine: &PennelEngine,
        client: &Client,
        snapshot: &Value,
        context: &Context,
        transfer: &Transfer,
    ) -> Result<(), PennelError> {
        self.check_live(engine, client)?;
        self.view.phase = CloudLinkPhase::Provisioning;
        if let Some(existing) = &context.existing_new_device {
            let (account, device) = client.session.identity()?;
            if account != context.account || &device != existing {
                return Err(PennelError::Cancelled);
            }
        } else if client.session.signed_in() {
            return Err(PennelError::Cancelled);
        }
        let mut receipt = provisioning::create(
            engine,
            client,
            &self.remote,
            context,
            transfer,
            self.expires_at_ms,
        )?;
        self.provisioning = true;
        if provisioning::advance(engine, client, &mut receipt, Some(snapshot), || {
            self.check_live(engine, client)
        })? {
            self.view.phase = CloudLinkPhase::Complete;
        }
        Ok(())
    }

    fn rollback(&mut self, engine: &PennelEngine) {
        if let Ok(Some(mut receipt)) = provisioning::load(engine)
            && receipt.request_id == self.remote.id
            && let Ok(session) = engine.cloud_session()
            && provisioning::cancel(engine, &Client::new(session), &mut receipt).unwrap_or(false)
        {
            self.view.phase = CloudLinkPhase::Complete;
        }
    }

    fn clear_display(&mut self) {
        self.view.connection_code = None;
        self.view.qr_payload = None;
        self.view.qr_svg = None;
    }
}

#[uniffi::export]
impl PennelEngine {
    pub fn cloud_link_availability(&self) -> Result<CloudLinkAvailability, PennelError> {
        Client::new(self.cloud_session()?).availability()
    }

    pub fn cloud_link_start(
        &self,
        role: CloudLinkRole,
        device_name: String,
        platform: String,
    ) -> Result<CloudLinkView, PennelError> {
        self.begin_link(role, None, device_name, platform)
    }
    pub fn cloud_link_join(
        &self,
        code: String,
        role: CloudLinkRole,
        device_name: String,
        platform: String,
    ) -> Result<CloudLinkView, PennelError> {
        self.begin_link(role, Some(code), device_name, platform)
    }
    pub fn cloud_link_poll(&self) -> Result<CloudLinkView, PennelError> {
        if self.cloud_link.lock().unwrap().active.is_none() && has_pending(self) {
            return self.resume_link_provisioning();
        }
        self.link_operation(false)
    }
    pub fn cloud_link_approve(&self) -> Result<CloudLinkView, PennelError> {
        self.link_operation(true)
    }
    pub fn cloud_link_view(&self) -> CloudLinkView {
        if has_pending(self) {
            return match provisioning::load(self) {
                Ok(Some(receipt)) => provisioning::view(&receipt),
                _ => CloudLinkView {
                    phase: CloudLinkPhase::Provisioning,
                    error_code: Some("pairing_storage".into()),
                    error_message: Some("Unlock this device to finish connecting.".into()),
                    ..CloudLinkView::default()
                },
            };
        }
        self.cloud_link.lock().unwrap().view.clone()
    }
    pub fn cloud_link_cancel(&self) -> Result<CloudLinkView, PennelError> {
        let (remote, active, view, generation) = {
            let mut flow = self.cloud_link.lock().unwrap();
            flow.generation = flow.generation.wrapping_add(1);
            flow.in_flight = true;
            let active = flow.active.take();
            let remote = flow.remote.take();
            flow.view = CloudLinkView {
                phase: CloudLinkPhase::Cancelled,
                ..CloudLinkView::default()
            };
            (remote, active, flow.view.clone(), flow.generation)
        };
        let _busy = BusyGuard {
            engine: self,
            generation,
        };
        drop(active);
        if let Some(mut receipt) = provisioning::load(self)? {
            let client = Client::new(self.cloud_session()?);
            match provisioning::cancel(self, &client, &mut receipt) {
                Ok(true) => {
                    let view = CloudLinkView {
                        phase: CloudLinkPhase::Complete,
                        ..CloudLinkView::default()
                    };
                    self.cloud_link.lock().unwrap().view = view.clone();
                    self.cloud_link.lock().unwrap().in_flight = false;
                    return Ok(view);
                }
                Err(PennelError::Cancelled) if !has_pending(self) => {
                    self.cloud_link.lock().unwrap().in_flight = false;
                    return Ok(view);
                }
                Err(error) => {
                    let mut pending = provisioning::view(&receipt);
                    pending.error_message = Some(error.to_string());
                    self.cloud_link.lock().unwrap().in_flight = false;
                    return Ok(pending);
                }
                _ => {}
            }
        }
        if let Some(remote) = remote {
            Client::new(self.cloud_session()?).cancel(&remote);
        }
        self.cloud_link.lock().unwrap().in_flight = false;
        Ok(view)
    }
}

impl PennelEngine {
    fn resume_link_provisioning(&self) -> Result<CloudLinkView, PennelError> {
        let mut receipt = provisioning::load(self)?.ok_or_else(invalid)?;
        let generation = {
            let mut flow = self.cloud_link.lock().unwrap();
            if flow.in_flight {
                return Ok(provisioning::view(&receipt));
            }
            flow.in_flight = true;
            flow.generation
        };
        let client = Client::new(self.cloud_session()?);
        let auth_generation = client.session.generation();
        let _busy = BusyGuard {
            engine: self,
            generation,
        };
        let result = provisioning::advance(self, &client, &mut receipt, None, || {
            if self.cloud_link.lock().unwrap().generation != generation
                || client.session.generation() != auth_generation
            {
                Err(PennelError::Cancelled)
            } else {
                Ok(())
            }
        });
        let mut view = provisioning::view(&receipt);
        match result {
            Ok(true) => view.phase = CloudLinkPhase::Complete,
            Err(error) => {
                if !has_pending(self) {
                    view.phase = if error == PennelError::Cancelled {
                        CloudLinkPhase::Cancelled
                    } else {
                        CloudLinkPhase::Failed
                    };
                }
                view.error_code = Some(if provisioning_storage(&error) {
                    "pairing_storage".into()
                } else {
                    link_error_code(&error)
                });
                view.error_message = Some(error.to_string());
            }
            _ => {}
        }
        let mut flow = self.cloud_link.lock().unwrap();
        if flow.generation == generation {
            flow.in_flight = false;
            flow.view = view.clone();
        }
        Ok(view)
    }

    fn begin_link(
        &self,
        role: CloudLinkRole,
        code: Option<String>,
        device_name: String,
        platform: String,
    ) -> Result<CloudLinkView, PennelError> {
        self.ensure_cloud_identity_mutable()?;
        self.cloud_link_cancel()?;
        if has_pending(self) {
            return Err(failure(
                "pairing_pending",
                "Finish the current device connection first.",
            ));
        }
        let generation = {
            let mut flow = self.cloud_link.lock().unwrap();
            flow.generation = flow.generation.wrapping_add(1);
            flow.in_flight = true;
            flow.generation
        };
        let result = self.create_link(role, code, device_name, platform, generation);
        let mut flow = self.cloud_link.lock().unwrap();
        if flow.generation != generation {
            drop(flow);
            if let Ok(active) = result {
                Client::new(self.cloud_session()?).cancel(&active.remote);
            }
            return Err(PennelError::Cancelled);
        }
        flow.in_flight = false;
        match result {
            Ok(active) => {
                flow.view = active.view.clone();
                flow.remote = Some(active.remote.clone());
                flow.active = Some(active);
                Ok(flow.view.clone())
            }
            Err(error) => {
                flow.view = CloudLinkView {
                    phase: CloudLinkPhase::Failed,
                    role: Some(role),
                    error_code: Some(link_error_code(&error)),
                    error_message: Some(error.to_string()),
                    ..CloudLinkView::default()
                };
                Err(error)
            }
        }
    }

    fn create_link(
        &self,
        role: CloudLinkRole,
        code: Option<String>,
        device_name: String,
        platform: String,
        flow_generation: u64,
    ) -> Result<LinkSession, PennelError> {
        if device_name.trim().is_empty()
            || device_name.chars().count() > 80
            || !["ios", "macos", "windows", "linux", "android"].contains(&platform.as_str())
        {
            return Err(invalid());
        }
        let session = self.cloud_session()?;
        let auth_generation = session.generation();
        let client = Client::new(session.clone());
        let check_live = || {
            if self.cloud_link.lock().unwrap().generation != flow_generation
                || session.generation() != auth_generation
            {
                Err(PennelError::Cancelled)
            } else {
                Ok(())
            }
        };
        let availability = client.availability()?;
        check_live()?;
        if !availability.available {
            let code = availability
                .reason_code
                .as_deref()
                .unwrap_or("pairing_unavailable");
            return Err(failure(
                code,
                if code == "pairing_incompatible" {
                    "Update Pennel or use email and your recovery code to connect."
                } else {
                    "Device connection is not available on this service. Use email and your recovery code."
                },
            ));
        }
        let identity = uuid::Uuid::now_v7().to_string();
        let capability = Zeroizing::new(random_token());
        let (id, password) = match code {
            Some(code) => {
                let (id, password) = parse_code(&code)?;
                (Some(id), password)
            }
            None => (None, Zeroizing::new(password())),
        };
        let generation = if role == CloudLinkRole::TrustedDevice {
            let setup = self.cloud_sync_setup()?;
            if !setup.can_approve_devices {
                return Err(PennelError::VaultLocked);
            }
            setup.generation
        } else {
            None
        };
        let body = json!({"role":role.wire(),"device_name":device_name.trim(),"platform":platform,"provisional_id":identity,
            "vault_generation":generation,"participant_token":capability.as_str()});
        let temporary = id.as_ref().map(|id| Remote {
            id: id.clone(),
            capability: capability.clone(),
            role,
        });
        check_live()?;
        let response = client.call("join", temporary.as_ref(), &body, session.signed_in())?;
        if field(&response, "protocol")? != PROTOCOL
            || field(&response, "participant_token")? != capability.as_str()
        {
            return Err(invalid());
        }
        let request_id = field(&response, "request_id")?.to_string();
        if id.as_ref().is_some_and(|expected| expected != &request_id) {
            return Err(invalid());
        }
        if request_id.len() != 9
            || !request_id.bytes().enumerate().all(|(i, c)| {
                if i == 4 {
                    c == b'-'
                } else {
                    c.is_ascii_alphanumeric()
                }
            })
        {
            return Err(invalid());
        }
        let expires_at_ms = response
            .get("expires_at_ms")
            .and_then(Value::as_i64)
            .filter(|t| *t > now_ms() && *t <= now_ms() + 660_000)
            .ok_or_else(invalid)?;
        let compact_password = password.as_str();
        let connection_code = format!(
            "{request_id}-{}-{}",
            &compact_password[..4],
            &compact_password[4..]
        );
        let qr_payload = format!("pennel-link:v1:{request_id}:{}", password.as_str());
        let view = CloudLinkView {
            phase: CloudLinkPhase::WaitingPeer,
            role: Some(role),
            request_id: Some(request_id.clone()),
            connection_code: Some(connection_code),
            qr_svg: Some(qr_svg(&qr_payload)?),
            qr_payload: Some(qr_payload),
            expires_at_ms: Some(expires_at_ms),
            ..CloudLinkView::default()
        };
        let remote = Remote {
            id: request_id,
            capability,
            role,
        };
        let active = LinkSession {
            remote: remote.clone(),
            password,
            expires_at_ms,
            auth_generation,
            flow_generation,
            new_identity: identity,
            context: None,
            exchange: None,
            pake: None,
            keys: None,
            confirmation: None,
            peer_confirmed: false,
            grant: None,
            envelope: None,
            provisioning: false,
            pake_sent: false,
            confirmation_sent: false,
            envelope_sent: false,
            view,
        };
        Ok(active)
    }

    fn link_operation(&self, approve: bool) -> Result<CloudLinkView, PennelError> {
        let (generation, mut active) = {
            let mut flow = self.cloud_link.lock().unwrap();
            if flow.in_flight {
                return Ok(flow.view.clone());
            }
            let Some(active) = flow.active.take() else {
                return Ok(flow.view.clone());
            };
            if matches!(
                active.view.phase,
                CloudLinkPhase::Complete
                    | CloudLinkPhase::Cancelled
                    | CloudLinkPhase::Expired
                    | CloudLinkPhase::Failed
            ) {
                flow.active = Some(active);
                return Ok(flow.view.clone());
            }
            flow.in_flight = true;
            (flow.generation, active)
        };
        let client = Client::new(self.cloud_session()?);
        let result = if approve {
            active.approve(self, &client)
        } else {
            active.poll(self, &client)
        };
        let fatal = result.as_ref().err().is_some_and(|e| {
            !(retryable_link_error(e) || (has_pending(self) && provisioning_storage(e)))
        });
        if let Err(error) = &result {
            active.view.error_message = Some(error.to_string());
            active.view.error_code = Some(link_error_code(error));
            if fatal {
                active.rollback(self);
                client.cancel(&active.remote);
                active.clear_display();
                if active.view.phase != CloudLinkPhase::Complete {
                    active.view.phase = match error {
                        PennelError::Cancelled => CloudLinkPhase::Cancelled,
                        PennelError::Cloud { code, .. } if code == "pairing_expired" => {
                            CloudLinkPhase::Expired
                        }
                        _ => CloudLinkPhase::Failed,
                    };
                }
            }
        } else {
            active.view.error_code = None;
            active.view.error_message = None;
        }
        let mut flow = self.cloud_link.lock().unwrap();
        if flow.generation != generation {
            drop(flow);
            active.rollback(self);
            client.cancel(&active.remote);
            return Err(PennelError::Cancelled);
        }
        flow.in_flight = false;
        flow.view = active.view.clone();
        let view = flow.view.clone();
        if !fatal && active.view.phase != CloudLinkPhase::Complete {
            flow.active = Some(active);
        } else {
            flow.remote = None;
        }
        Ok(view)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(super) fn context() -> Context {
        let snapshot = json!({"request_id":"ABCD-EFGH",
            "new_device":{"device_name":"Phone","platform":"ios","provisional_id":"00000000-0000-4000-8000-000000000001"},
            "trusted_device":{"user_id":"00000000-0000-4000-8000-000000000002","device_id":"00000000-0000-4000-8000-000000000003",
                "vault_generation":1,"device_name":"Computer","platform":"macos","account_email":"fixture@example.test"}});
        Context::from_snapshot(&snapshot, "https://example.test")
            .unwrap()
            .unwrap()
    }

    fn handshake(
        a_password: &str,
        b_password: &str,
        a_context: &Context,
        b_context: &Context,
    ) -> (Keys, Keys) {
        let aid = [b"new:".as_slice(), &a_context.bytes].concat();
        let bid = [b"trusted:".as_slice(), &a_context.bytes].concat();
        let (a, ap) = Spake2::<Ed25519Group>::start_a(
            &Password::new(a_password.as_bytes()),
            &Identity::new(&aid),
            &Identity::new(&bid),
        );
        let aid = [b"new:".as_slice(), &b_context.bytes].concat();
        let bid = [b"trusted:".as_slice(), &b_context.bytes].concat();
        let (b, bp) = Spake2::<Ed25519Group>::start_b(
            &Password::new(b_password.as_bytes()),
            &Identity::new(&aid),
            &Identity::new(&bid),
        );
        let a = Zeroizing::new(a.finish(&bp).unwrap());
        let b = Zeroizing::new(b.finish(&ap).unwrap());
        (
            Keys::new(&a, a_context).unwrap(),
            Keys::new(&b, b_context).unwrap(),
        )
    }

    #[test]
    fn matching_codes_confirm_both_roles_and_transfer_the_same_key() {
        let context = context();
        let (a, b) = handshake("12345678", "12345678", &context, &context);
        a.verify(
            CloudLinkRole::TrustedDevice,
            &context,
            &b.confirm(CloudLinkRole::TrustedDevice, &context),
        )
        .unwrap();
        b.verify(
            CloudLinkRole::NewDevice,
            &context,
            &a.confirm(CloudLinkRole::NewDevice, &context),
        )
        .unwrap();
        assert_eq!(a.verifier, b.verifier);
        let key = VaultKey::generate();
        let transfer = Transfer {
            version: 1,
            grant: random_token(),
            vault_key: key.to_b64(),
            account_id: context.account.clone(),
            generation: context.generation,
            provisional_id: context.new_id.clone(),
            transcript_hash: context.hash.clone(),
        };
        let sealed = b.seal(&context, &transfer).unwrap();
        let opened = a.open(&context, &sealed).unwrap();
        assert_eq!(opened.vault_key, key.to_b64());
        assert_ne!(sealed, transfer.vault_key);
    }

    #[test]
    fn wrong_code_and_changed_context_fail_mutual_confirmation() {
        let context = context();
        let (a, b) = handshake("12345678", "87654321", &context, &context);
        assert!(
            a.verify(
                CloudLinkRole::TrustedDevice,
                &context,
                &b.confirm(CloudLinkRole::TrustedDevice, &context)
            )
            .is_err()
        );
        let mut changed = context.clone();
        changed.bytes.push(1);
        changed.hash = hex(&Sha256::digest(&changed.bytes));
        let (a, b) = handshake("12345678", "12345678", &context, &changed);
        assert!(
            a.verify(
                CloudLinkRole::TrustedDevice,
                &context,
                &b.confirm(CloudLinkRole::TrustedDevice, &changed)
            )
            .is_err()
        );
    }

    #[test]
    fn connection_codes_are_local_and_not_recovery_codes_or_arbitrary_urls() {
        let (id, password) = parse_code("ABCD-EFGH-1234-5678").unwrap();
        assert_eq!(id, "ABCD-EFGH");
        assert_eq!(password.as_str(), "12345678");
        assert_eq!(
            parse_code("pennel-link:v1:ABCD-EFGH:12345678").unwrap().0,
            id
        );
        for wrong in [
            "https://example.test/link#ABCD-EFGH-1234-5678",
            "ABCD-EFGH-1234-5678-9012-3456",
            "éééééééé",
            "ABCD-EFGH-password",
        ] {
            assert!(parse_code(wrong).is_err());
        }
    }

    #[test]
    fn reflected_confirmation_and_modified_envelopes_are_rejected() {
        let context = context();
        let (a, b) = handshake("12345678", "12345678", &context, &context);
        assert!(
            a.verify(
                CloudLinkRole::TrustedDevice,
                &context,
                &a.confirm(CloudLinkRole::NewDevice, &context)
            )
            .is_err()
        );
        let key = VaultKey::generate();
        let transfer = Transfer {
            version: 1,
            grant: random_token(),
            vault_key: key.to_b64(),
            account_id: context.account.clone(),
            generation: 1,
            provisional_id: context.new_id.clone(),
            transcript_hash: context.hash.clone(),
        };
        let sealed = b.seal(&context, &transfer).unwrap();
        let mut bytes = B64.decode(&sealed).unwrap();
        let last = bytes.len() - 1;
        bytes[last] ^= 1;
        assert!(a.open(&context, &B64.encode(bytes)).is_err());
        assert!(a.open(&context, &"A".repeat(MAX_ENVELOPE + 1)).is_err());
        for (vault_key, grant) in [
            ("bad key".into(), random_token()),
            (key.to_b64(), "!".repeat(43)),
        ] {
            let malformed = Transfer {
                version: 1,
                grant,
                vault_key,
                account_id: context.account.clone(),
                generation: context.generation,
                provisional_id: context.new_id.clone(),
                transcript_hash: context.hash.clone(),
            };
            assert!(
                a.open(&context, &b.seal(&context, &malformed).unwrap())
                    .is_err()
            );
        }
    }

    #[test]
    fn cancellation_during_create_cannot_reactivate_the_connection() {
        let mut server = mockito::Server::new();
        let _capabilities = capabilities(&mut server);
        let (entered, observed) = std::sync::mpsc::channel();
        let (release, blocked) = std::sync::mpsc::channel();
        let blocked = Arc::new(std::sync::Mutex::new(blocked));
        let create = server.mock("POST","/v1/device-links").with_body_from_request(move |request| {
            let body: Value = serde_json::from_slice(request.body().unwrap()).unwrap();
            entered.send(()).unwrap(); blocked.lock().unwrap().recv().unwrap();
            serde_json::to_vec(&json!({"protocol":PROTOCOL,"request_id":"ABCD-EFGH","participant_token":body["participant_token"],"expires_at_ms":now_ms()+600_000})).unwrap()
        }).expect(1).create();
        let cancel = server
            .mock("POST", "/v1/device-links/ABCD-EFGH/cancel")
            .with_body("{}")
            .expect(1)
            .create();
        let directory = tempfile::tempdir().unwrap();
        let engine = PennelEngine::new(directory.path().to_string_lossy().into_owned()).unwrap();
        let secrets = Arc::new(crate::cloud::tests::MemorySecrets::default());
        engine.set_secret_store(secrets.clone()).unwrap();
        engine.use_cloud_session(Session::new(server.url(), secrets));
        let starting = {
            let engine = engine.clone();
            std::thread::spawn(move || {
                engine.cloud_link_start(CloudLinkRole::NewDevice, "Fixture".into(), "ios".into())
            })
        };
        observed
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        assert_eq!(
            engine.cloud_link_cancel().unwrap().phase,
            CloudLinkPhase::Cancelled
        );
        release.send(()).unwrap();
        assert_eq!(starting.join().unwrap(), Err(PennelError::Cancelled));
        assert_eq!(engine.cloud_link_view().phase, CloudLinkPhase::Cancelled);
        create.assert();
        cancel.assert();
    }

    pub(super) fn engine_for(
        server: &mockito::ServerGuard,
    ) -> (tempfile::TempDir, Arc<PennelEngine>) {
        let directory = tempfile::tempdir().unwrap();
        let engine = PennelEngine::new(directory.path().to_string_lossy().into_owned()).unwrap();
        let secrets = Arc::new(crate::cloud::tests::MemorySecrets::default());
        engine.set_secret_store(secrets.clone()).unwrap();
        engine.use_cloud_session(Session::new(server.url(), secrets));
        (directory, engine)
    }

    fn capabilities(server: &mut mockito::ServerGuard) -> mockito::Mock {
        server
            .mock("GET", "/v1/device-links/capabilities")
            .with_body(json!({"supported":true,"protocol":PROTOCOL,"expires_in":600}).to_string())
            .create()
    }

    #[test]
    fn capabilities_classify_disabled_missing_and_incompatible_services() {
        for (status, body, reason) in [
            (
                200,
                json!({"supported":true,"protocol":PROTOCOL}).to_string(),
                None,
            ),
            (
                200,
                json!({"supported":false,"protocol":PROTOCOL}).to_string(),
                Some("pairing_unavailable"),
            ),
            (404, String::new(), Some("pairing_unavailable")),
            (
                404,
                "<html>Not found</html>".into(),
                Some("pairing_unavailable"),
            ),
            (405, String::new(), Some("pairing_unavailable")),
            (
                404,
                r#"{"error":"not_found"}"#.into(),
                Some("pairing_unavailable"),
            ),
            (
                200,
                "<html>Gateway</html>".into(),
                Some("pairing_incompatible"),
            ),
            (200, "{}".into(), Some("pairing_incompatible")),
            (
                200,
                json!({"supported":"yes","protocol":PROTOCOL}).to_string(),
                Some("pairing_incompatible"),
            ),
            (
                200,
                json!({"supported":true,"protocol":"future-v2"}).to_string(),
                Some("pairing_incompatible"),
            ),
        ] {
            let mut server = mockito::Server::new();
            let request = server
                .mock("GET", "/v1/device-links/capabilities")
                .match_header("authorization", mockito::Matcher::Missing)
                .match_header(
                    "user-agent",
                    concat!("pennel-core/", env!("CARGO_PKG_VERSION")),
                )
                .with_status(status)
                .with_body(body)
                .expect(1)
                .create();
            let (_directory, engine) = engine_for(&server);
            let availability = engine.cloud_link_availability().unwrap();
            assert_eq!(availability.available, reason.is_none(), "status {status}");
            assert_eq!(
                availability.reason_code.as_deref(),
                reason,
                "status {status}"
            );
            request.assert();
        }
    }

    #[test]
    fn capabilities_preserve_explicit_errors_and_status_for_service_failures() {
        for (status, body, code) in [
            (403, "<html>Denied</html>", "http_403"),
            (503, "<html>Unavailable</html>", "http_503"),
            (429, r#"{"error":"rate_limited"}"#, "rate_limited"),
            (404, r#"{"error":"pairing_unknown"}"#, "pairing_unknown"),
        ] {
            let mut server = mockito::Server::new();
            let request = server
                .mock("GET", "/v1/device-links/capabilities")
                .with_status(status)
                .with_body(body)
                .expect(1)
                .create();
            let (_directory, engine) = engine_for(&server);
            assert!(
                matches!(engine.cloud_link_availability(), Err(PennelError::Cloud { code: observed, .. }) if observed == code)
            );
            request.assert();
        }
    }

    #[test]
    fn unavailable_admission_preserves_role_and_error_without_posting() {
        for (code, role, reason) in [
            (None, CloudLinkRole::NewDevice, "pairing_unavailable"),
            (None, CloudLinkRole::TrustedDevice, "pairing_unavailable"),
            (
                Some("ABCD-EFGH-1234-5678"),
                CloudLinkRole::NewDevice,
                "pairing_incompatible",
            ),
            (
                Some("ABCD-EFGH-1234-5678"),
                CloudLinkRole::TrustedDevice,
                "pairing_incompatible",
            ),
        ] {
            let mut server = mockito::Server::new();
            let body = if reason == "pairing_unavailable" {
                json!({"supported":false,"protocol":PROTOCOL})
            } else {
                json!({"supported":true,"protocol":"future"})
            };
            let probe = server
                .mock("GET", "/v1/device-links/capabilities")
                .with_body(body.to_string())
                .expect(1)
                .create();
            let admission = server
                .mock("POST", mockito::Matcher::Any)
                .expect(0)
                .create();
            let (_directory, engine) = engine_for(&server);
            let result = match code {
                Some(code) => {
                    engine.cloud_link_join(code.into(), role, "Fixture".into(), "ios".into())
                }
                None => engine.cloud_link_start(role, "Fixture".into(), "ios".into()),
            };
            assert!(matches!(result, Err(PennelError::Cloud { code, .. }) if code == reason));
            let restored = engine.cloud_link_view();
            assert_eq!(restored.phase, CloudLinkPhase::Failed);
            assert_eq!(restored.role, Some(role));
            assert_eq!(restored.error_code.as_deref(), Some(reason));
            assert!(restored.connection_code.is_none());
            probe.assert();
            admission.assert();
        }
    }

    #[test]
    fn capability_reads_leave_pending_view_and_credentials_unchanged() {
        let mut server = mockito::Server::new();
        let probe = capabilities(&mut server);
        let (_directory, engine) = engine_for(&server);
        let store = engine.secret_store().unwrap();
        for key in [
            super::super::REFRESH_ACCOUNT,
            VAULT_KEY_ACCOUNT,
            VAULT_OWNER_ACCOUNT,
        ] {
            store.set(key.into(), Some(format!("fixture-{key}")));
        }
        let view = CloudLinkView {
            phase: CloudLinkPhase::WaitingApproval,
            role: Some(CloudLinkRole::NewDevice),
            request_id: Some("ABCD-EFGH".into()),
            ..CloudLinkView::default()
        };
        engine.cloud_link.lock().unwrap().view = view.clone();
        assert!(engine.cloud_link_availability().unwrap().available);
        assert!(engine.cloud_link_view() == view);
        for key in [
            super::super::REFRESH_ACCOUNT,
            VAULT_KEY_ACCOUNT,
            VAULT_OWNER_ACCOUNT,
        ] {
            assert_eq!(store.get(key.into()), Some(format!("fixture-{key}")));
        }
        probe.assert();
    }

    #[test]
    fn cancellation_or_account_change_during_capability_check_never_posts() {
        for change_account in [false, true] {
            let mut server = mockito::Server::new();
            let (entered, observed) = std::sync::mpsc::channel();
            let (release, blocked) = std::sync::mpsc::channel();
            let blocked = Arc::new(std::sync::Mutex::new(blocked));
            let probe = server
                .mock("GET", "/v1/device-links/capabilities")
                .with_body_from_request(move |_| {
                    entered.send(()).unwrap();
                    blocked.lock().unwrap().recv().unwrap();
                    serde_json::to_vec(&json!({"supported":true,"protocol":PROTOCOL})).unwrap()
                })
                .expect(1)
                .create();
            let admission = server
                .mock("POST", mockito::Matcher::Any)
                .expect(0)
                .create();
            let (_directory, engine) = engine_for(&server);
            let starting = {
                let engine = engine.clone();
                std::thread::spawn(move || {
                    engine.cloud_link_start(
                        CloudLinkRole::NewDevice,
                        "Fixture".into(),
                        "ios".into(),
                    )
                })
            };
            observed
                .recv_timeout(std::time::Duration::from_secs(5))
                .unwrap();
            if change_account {
                engine.cloud_session().unwrap().cancel_login();
            } else {
                engine.cloud_link_cancel().unwrap();
            }
            release.send(()).unwrap();
            assert_eq!(starting.join().unwrap(), Err(PennelError::Cancelled));
            assert_ne!(engine.cloud_link_view().phase, CloudLinkPhase::WaitingPeer);
            probe.assert();
            admission.assert();
        }
    }

    #[test]
    fn an_offline_capability_service_is_retryable_instead_of_disabled() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        drop(listener);
        let directory = tempfile::tempdir().unwrap();
        let engine = PennelEngine::new(directory.path().to_string_lossy().into_owned()).unwrap();
        let secrets = Arc::new(crate::cloud::tests::MemorySecrets::default());
        engine.set_secret_store(secrets.clone()).unwrap();
        engine.use_cloud_session(Session::new(format!("http://{address}"), secrets));
        assert!(matches!(
            engine.cloud_link_availability(),
            Err(PennelError::Network { .. })
        ));
        assert_eq!(engine.cloud_link_view().phase, CloudLinkPhase::Idle);
    }

    #[test]
    fn existing_connections_keep_polling_and_cancelling_when_admission_is_disabled() {
        let mut server = mockito::Server::new();
        let initial_probe = capabilities(&mut server);
        let create = server
            .mock("POST", "/v1/device-links")
            .with_body_from_request(|request| {
                let body: Value = serde_json::from_slice(request.body().unwrap()).unwrap();
                serde_json::to_vec(&json!({"protocol":PROTOCOL,"request_id":"ABCD-EFGH",
                "participant_token":body["participant_token"],"expires_at_ms":now_ms()+600_000}))
                .unwrap()
            })
            .expect(1)
            .create();
        let (_directory, engine) = engine_for(&server);
        assert_eq!(
            engine
                .cloud_link_start(CloudLinkRole::NewDevice, "Fixture".into(), "ios".into())
                .unwrap()
                .phase,
            CloudLinkPhase::WaitingPeer
        );
        initial_probe.assert();
        initial_probe.remove();
        let disabled_probe = server
            .mock("GET", "/v1/device-links/capabilities")
            .with_body(json!({"supported":false,"protocol":PROTOCOL}).to_string())
            .expect(1)
            .create();
        assert!(!engine.cloud_link_availability().unwrap().available);
        let poll = server
            .mock("POST", "/v1/device-links/ABCD-EFGH/poll")
            .with_body(r#"{"status":"waiting_peer"}"#)
            .expect(1)
            .create();
        let cancel = server
            .mock("POST", "/v1/device-links/ABCD-EFGH/cancel")
            .with_body("{}")
            .expect(1)
            .create();
        assert_eq!(
            engine.cloud_link_poll().unwrap().phase,
            CloudLinkPhase::WaitingPeer
        );
        assert_eq!(
            engine.cloud_link_cancel().unwrap().phase,
            CloudLinkPhase::Cancelled
        );
        create.assert();
        poll.assert();
        cancel.assert();
        disabled_probe.assert();
    }

    #[test]
    fn a_temporary_poll_failure_keeps_the_same_pending_connection_for_retry() {
        let mut server = mockito::Server::new();
        let _probe = capabilities(&mut server);
        let _create = server
            .mock("POST", "/v1/device-links")
            .with_body_from_request(|request| {
                let body: Value = serde_json::from_slice(request.body().unwrap()).unwrap();
                serde_json::to_vec(&json!({"protocol":PROTOCOL,"request_id":"ABCD-EFGH",
                "participant_token":body["participant_token"],"expires_at_ms":now_ms()+600_000}))
                .unwrap()
            })
            .create();
        let (_directory, engine) = engine_for(&server);
        engine
            .cloud_link_start(CloudLinkRole::NewDevice, "Fixture".into(), "ios".into())
            .unwrap();
        for (status, body, code) in [
            (503, "<html>Gateway</html>", "http_503"),
            (500, r#"{"error":"internal"}"#, "internal"),
        ] {
            let temporary = server
                .mock("POST", "/v1/device-links/ABCD-EFGH/poll")
                .with_status(status)
                .with_body(body)
                .expect(1)
                .create();
            let view = engine.cloud_link_poll().unwrap();
            assert_eq!(view.phase, CloudLinkPhase::WaitingPeer);
            assert_eq!(view.error_code.as_deref(), Some(code));
            temporary.assert();
            temporary.remove();
        }
        let poll = server
            .mock("POST", "/v1/device-links/ABCD-EFGH/poll")
            .with_body(r#"{"status":"waiting_peer"}"#)
            .expect(1)
            .create();
        let view = engine.cloud_link_poll().unwrap();
        assert_eq!(view.phase, CloudLinkPhase::WaitingPeer);
        assert!(view.error_code.is_none());
        poll.assert();
    }
}
