use super::*;

const INDEX: &str = "pennel-cloud-link-pending-v1";

#[derive(Serialize, Deserialize)]
pub(super) struct Receipt {
    version: u8,
    origin: String,
    pub(super) request_id: String,
    participant_token: String,
    grant: String,
    account: String,
    existing_device: Option<String>,
    vault_key: String,
    generation: u32,
    transcript_hash: String,
    previous_key: Option<String>,
    previous_owner: Option<String>,
    previous_sync: bool,
    previous_generation: Option<u32>,
    expires_at_ms: i64,
    account_email: String,
    local_prepared: bool,
    redeem_attempted: bool,
    credentials: Option<Credentials>,
    acknowledged: bool,
    cancel_requested: bool,
    #[serde(skip)]
    runtime_generation: Option<u64>,
}

impl Drop for Receipt {
    fn drop(&mut self) {
        self.participant_token.zeroize();
        self.grant.zeroize();
        self.vault_key.zeroize();
        self.previous_key.zeroize();
    }
}

#[derive(Serialize, Deserialize)]
struct Credentials {
    access_token: String,
    refresh_token: String,
    expires_in: u64,
    user_id: String,
    device_id: String,
}

impl Drop for Credentials {
    fn drop(&mut self) {
        self.access_token.zeroize();
        self.refresh_token.zeroize();
    }
}

fn entry(account: &str) -> String {
    format!("pennel-cloud-link-staging-{account}")
}

pub(crate) fn has_pending(engine: &PennelEngine) -> bool {
    engine
        .secret_store()
        .and_then(|store| store.get(INDEX.into()))
        .is_some()
}

pub(super) fn load(engine: &PennelEngine) -> Result<Option<Receipt>, PennelError> {
    let Some(store) = engine.secret_store() else {
        return Ok(None);
    };
    let Some(account) = store.get(INDEX.into()) else {
        return Ok(None);
    };
    uuid::Uuid::parse_str(&account).map_err(|_| invalid())?;
    let encoded = Zeroizing::new(store.get(entry(&account)).ok_or_else(|| {
        failure(
            "pairing_storage",
            "Unlock this device to finish connecting.",
        )
    })?);
    let receipt: Receipt = serde_json::from_str(&encoded).map_err(|_| invalid())?;
    if receipt.version != 1
        || receipt.account != account
        || receipt.participant_token.len() != 43
        || receipt.grant.len() != 43
        || receipt.origin != engine.cloud_session()?.base_url()
        || VaultKey::from_b64(&receipt.vault_key).is_none()
    {
        return Err(invalid());
    }
    Ok(Some(receipt))
}

fn save(engine: &PennelEngine, receipt: &Receipt) -> Result<(), PennelError> {
    let _journal = engine.link_journal_gate.lock().unwrap();
    if receipt
        .runtime_generation
        .is_some_and(|generation| engine.cloud_link.lock().unwrap().generation != generation)
    {
        return Err(PennelError::Cancelled);
    }
    if let Some(previous) = load(engine)?
        && previous.request_id == receipt.request_id
        && previous.cancel_requested
        && !receipt.cancel_requested
    {
        return Err(PennelError::Cancelled);
    }
    let store = engine.secret_store().ok_or(PennelError::NotSignedIn)?;
    let encoded = Zeroizing::new(serde_json::to_string(receipt).map_err(PennelError::storage)?);
    store_verified(store.as_ref(), &entry(&receipt.account), &encoded)?;
    store_verified(store.as_ref(), INDEX, &receipt.account)
}

fn clear(engine: &PennelEngine, receipt: &Receipt) -> Result<(), PennelError> {
    let _journal = engine.link_journal_gate.lock().unwrap();
    if receipt
        .runtime_generation
        .is_some_and(|generation| engine.cloud_link.lock().unwrap().generation != generation)
    {
        return Err(PennelError::Cancelled);
    }
    let store = engine.secret_store().ok_or(PennelError::NotSignedIn)?;
    if let Some(current) = load(engine)?
        && current.request_id != receipt.request_id
    {
        return Err(PennelError::Cancelled);
    }
    // All active state is durable before removing this barrier. A leftover private
    // receipt remains protected by the OS store if its best-effort deletion fails.
    store.set(INDEX.into(), None);
    if store.get(INDEX.into()).is_some() {
        return Err(failure(
            "pairing_storage",
            "Unlock this device to finish connecting.",
        ));
    }
    store.set(entry(&receipt.account), None);
    Ok(())
}

pub(super) fn view(receipt: &Receipt) -> CloudLinkView {
    CloudLinkView {
        phase: CloudLinkPhase::Provisioning,
        role: Some(CloudLinkRole::NewDevice),
        request_id: Some(receipt.request_id.clone()),
        account_email: Some(receipt.account_email.clone()),
        expires_at_ms: Some(receipt.expires_at_ms),
        ..CloudLinkView::default()
    }
}

pub(super) fn create(
    engine: &PennelEngine,
    client: &Client,
    remote: &Remote,
    context: &Context,
    transfer: &Transfer,
    expires: i64,
) -> Result<Receipt, PennelError> {
    if !transfer.valid_for(context) {
        return Err(invalid());
    }
    if let Some(receipt) = load(engine)? {
        if receipt.request_id != remote.id {
            return Err(failure(
                "pairing_pending",
                "Finish the current device connection first.",
            ));
        }
        return Ok(receipt);
    }
    let store = engine.secret_store().ok_or(PennelError::NotSignedIn)?;
    let receipt = Receipt {
        version: 1,
        origin: client.session.base_url().to_string(),
        request_id: remote.id.clone(),
        participant_token: remote.capability.to_string(),
        grant: transfer.grant.clone(),
        account: context.account.clone(),
        existing_device: context.existing_new_device.clone(),
        vault_key: transfer.vault_key.clone(),
        generation: context.generation,
        transcript_hash: context.hash.clone(),
        previous_key: store.get(VAULT_KEY_ACCOUNT.into()),
        previous_owner: store.get(VAULT_OWNER_ACCOUNT.into()),
        previous_sync: engine.settings().cloud_sync,
        previous_generation: engine.with_sync_state(|state| state.generation())?,
        expires_at_ms: expires,
        account_email: context.email.clone(),
        local_prepared: false,
        redeem_attempted: false,
        credentials: None,
        acknowledged: false,
        cancel_requested: false,
        runtime_generation: Some(engine.cloud_link.lock().unwrap().generation),
    };
    save(engine, &receipt)?;
    Ok(receipt)
}

fn remote(receipt: &Receipt) -> Remote {
    Remote {
        id: receipt.request_id.clone(),
        capability: Zeroizing::new(receipt.participant_token.clone()),
        role: CloudLinkRole::NewDevice,
    }
}

fn restore(engine: &PennelEngine, receipt: &Receipt) -> Result<(), PennelError> {
    let _speaker = crate::speaker_jobs::speaker_identity_guard();
    let _sync = engine.sync_gate.lock().unwrap();
    let _identity = engine.inference_capture_gate();
    let store = engine.secret_store().ok_or(PennelError::NotSignedIn)?;
    if store.get(VAULT_KEY_ACCOUNT.into()).as_deref() == Some(receipt.vault_key.as_str()) {
        match &receipt.previous_key {
            Some(key) => store_verified(store.as_ref(), VAULT_KEY_ACCOUNT, key)?,
            None => {
                store.set(VAULT_KEY_ACCOUNT.into(), None);
                if store.get(VAULT_KEY_ACCOUNT.into()).is_some() {
                    return Err(failure(
                        "pairing_storage",
                        "Unlock this device to cancel connecting.",
                    ));
                }
            }
        }
        match &receipt.previous_owner {
            Some(owner) => store_verified(store.as_ref(), VAULT_OWNER_ACCOUNT, owner)?,
            None => store.set(VAULT_OWNER_ACCOUNT.into(), None),
        }
        if let Some(key) = receipt.previous_key.as_deref().and_then(VaultKey::from_b64) {
            engine.with_sync_state(|state| {
                state.use_vault(&super::super::crypto::vault_check(&key))?;
                if let Some(generation) = receipt.previous_generation {
                    state.set_generation(generation)?;
                }
                Ok(())
            })?;
        }
        let mut settings = engine.settings();
        settings.cloud_sync = receipt.previous_sync && receipt.previous_key.is_some();
        engine.update_settings_locked(settings)?;
    }
    clear(engine, receipt)
}

fn commit(
    engine: &PennelEngine,
    client: &Client,
    receipt: &Receipt,
    expected_auth: u64,
    live: &impl Fn() -> Result<(), PennelError>,
) -> Result<(), PennelError> {
    let _speaker = crate::speaker_jobs::speaker_identity_guard();
    let _sync = engine.sync_gate.lock().unwrap();
    let _identity = engine.inference_capture_gate();
    live()?;
    engine.ensure_cloud_identity_mutable()?;
    let store = engine.secret_store().ok_or(PennelError::NotSignedIn)?;
    if !receipt.local_prepared
        || store.get(VAULT_KEY_ACCOUNT.into()).as_deref() != Some(receipt.vault_key.as_str())
    {
        return Err(failure(
            "pairing_storage",
            "Unlock this device to finish connecting.",
        ));
    }
    let expected_device = if let Some(device) = &receipt.existing_device {
        device.as_str()
    } else {
        receipt
            .credentials
            .as_ref()
            .ok_or_else(invalid)?
            .device_id
            .as_str()
    };
    if client.session.signed_in() {
        let (account, device) = client.session.identity()?;
        if account != receipt.account || device != expected_device {
            return Err(PennelError::Cancelled);
        }
    } else {
        let credentials = receipt.credentials.as_ref().ok_or(PennelError::Cancelled)?;
        let body = json!({"access_token":credentials.access_token,"refresh_token":credentials.refresh_token,"expires_in":credentials.expires_in});
        client.session.install_link_tokens(expected_auth, &body)?;
    }
    let key = VaultKey::from_b64(&receipt.vault_key).ok_or_else(invalid)?;
    let vault = engine.signed_in_sync_api()?.vault()?;
    if vault.generation != receipt.generation
        || vault
            .kdf
            .as_ref()
            .and_then(|k| k.key_check.as_deref())
            .is_some_and(|check| check != super::super::crypto::vault_check(&key))
    {
        let mut settings = engine.settings();
        settings.cloud_sync = false;
        engine.update_settings_locked(settings)?;
        clear(engine, receipt)?;
        return Err(PennelError::VaultLocked);
    }
    if vault.kdf.as_ref().is_some_and(|k| k.key_check.is_none()) {
        store_verified(
            store.as_ref(),
            super::super::VAULT_VALIDATION_ACCOUNT,
            &super::super::setup::validation_marker(&key, &vault),
        )?;
    }
    let mut settings = engine.settings();
    settings.cloud_sync = true;
    live()?;
    engine.update_settings_locked(settings)?;
    clear(engine, receipt)
}

fn prove_acknowledged(client: &Client, receipt: &Receipt) -> Result<bool, PennelError> {
    if client.session.signed_in() {
        let (account, device) = client.session.identity()?;
        let expected_device = receipt.existing_device.as_deref().or_else(|| {
            receipt
                .credentials
                .as_ref()
                .map(|credentials| credentials.device_id.as_str())
        });
        return Ok(account == receipt.account
            && expected_device == Some(device.as_str())
            && (receipt.existing_device.is_none() || receipt.acknowledged));
    }
    if let Some(existing) = &receipt.existing_device {
        if !receipt.acknowledged {
            return Ok(false);
        }
        return client
            .session
            .identity()
            .map(|(account, device)| account == receipt.account && device == *existing);
    }
    let Some(credentials) = receipt.credentials.as_ref() else {
        return Ok(false);
    };
    let url = format!("{}/v1/whoami", client.session.base_url());
    let (status, body) = send_json(
        &client.http,
        "GET",
        &url,
        Some(&credentials.access_token),
        None,
    )?;
    if status == 401 {
        // A proof-only request must not spend the receipt's refresh token: losing
        // that response could make recovery replay it and revoke this device.
        return Ok(false);
    }
    if status != 200 {
        return Err(link_http_error(status, &body));
    }
    Ok(field(&body, "user_id")? == receipt.account
        && field(&body, "device_id")? == credentials.device_id)
}

pub(super) fn advance(
    engine: &PennelEngine,
    client: &Client,
    receipt: &mut Receipt,
    snapshot: Option<&Value>,
    live: impl Fn() -> Result<(), PennelError>,
) -> Result<bool, PennelError> {
    let expected_auth = client.session.generation();
    if receipt.runtime_generation.is_none() {
        receipt.runtime_generation = Some(engine.cloud_link.lock().unwrap().generation);
    }
    if receipt.acknowledged && receipt.local_prepared {
        live()?;
        commit(engine, client, receipt, expected_auth, &live)?;
        return Ok(true);
    }
    let remote = remote(receipt);
    let loaded;
    let snapshot = match snapshot {
        Some(value) => value,
        None => {
            loaded = match client.poll(&remote) {
                Ok(value) => value,
                Err(PennelError::Cloud { code, .. }) if code == "pairing_unknown" => {
                    if prove_acknowledged(client, receipt)? {
                        receipt.acknowledged = true;
                        live()?;
                        commit(engine, client, receipt, expected_auth, &live)?;
                        return Ok(true);
                    }
                    restore(engine, receipt)?;
                    return Err(failure(
                        "pairing_expired",
                        "This device connection expired. Start a new connection.",
                    ));
                }
                Err(error) => return Err(error),
            };
            &loaded
        }
    };
    let status = field(snapshot, "status")?;
    if status == "complete" {
        if snapshot["transcript_hash"].as_str() != Some(receipt.transcript_hash.as_str()) {
            return Err(invalid());
        }
        receipt.acknowledged = true;
        live()?;
        commit(engine, client, receipt, expected_auth, &live)?;
        return Ok(true);
    }
    if receipt.cancel_requested || matches!(status, "cancelled" | "expired" | "failed") {
        if !matches!(status, "cancelled" | "expired" | "failed") {
            client.call(
                "cancel",
                Some(&remote),
                &json!({"participant_token":receipt.participant_token}),
                false,
            )?;
        }
        restore(engine, receipt)?;
        return Err(PennelError::Cancelled);
    }
    live()?;
    if let Some(existing) = &receipt.existing_device {
        let (account, device) = client.session.identity()?;
        if account != receipt.account || device != *existing {
            return Err(PennelError::Cancelled);
        }
    } else if client.session.signed_in() {
        return Err(PennelError::Cancelled);
    }
    if !receipt.local_prepared {
        let _speaker = crate::speaker_jobs::speaker_identity_guard();
        let _sync = engine.sync_gate.lock().unwrap();
        let _identity = engine.inference_capture_gate();
        live()?;
        engine.ensure_cloud_identity_mutable()?;
        let mut settings = engine.settings();
        settings.cloud_sync = false;
        engine.update_settings_locked(settings)?;
        let store = engine.secret_store().ok_or(PennelError::NotSignedIn)?;
        store_verified(store.as_ref(), VAULT_KEY_ACCOUNT, &receipt.vault_key)?;
        store_verified(store.as_ref(), VAULT_OWNER_ACCOUNT, &receipt.account)?;
        let key = VaultKey::from_b64(&receipt.vault_key).ok_or_else(invalid)?;
        engine.with_sync_state(|state| {
            state.use_vault(&super::super::crypto::vault_check(&key))?;
            state.set_generation(receipt.generation)
        })?;
        receipt.local_prepared = true;
        save(engine, receipt)?;
    }
    if receipt.existing_device.is_none() && receipt.credentials.is_none() {
        if receipt.redeem_attempted && status == "provisioning" {
            receipt.cancel_requested = true;
            save(engine, receipt)?;
            client.call(
                "cancel",
                Some(&remote),
                &json!({"participant_token":receipt.participant_token}),
                false,
            )?;
            restore(engine, receipt)?;
            return Err(failure(
                "pairing_restart",
                "The connection was interrupted before access could be saved. Start a new connection.",
            ));
        }
        live()?;
        receipt.redeem_attempted = true;
        save(engine, receipt)?;
        let body = client.call(
            "redeem",
            Some(&remote),
            &json!({"participant_token":receipt.participant_token,
            "grant":receipt.grant,"transcript_hash":receipt.transcript_hash}),
            false,
        )?;
        let credentials = Credentials {
            access_token: field(&body, "access_token")?.to_string(),
            refresh_token: field(&body, "refresh_token")?.to_string(),
            expires_in: body["expires_in"]
                .as_u64()
                .filter(|n| (1..=86_400).contains(n))
                .ok_or_else(invalid)?,
            user_id: field(&body, "user_id")?.to_string(),
            device_id: field(&body, "device_id")?.to_string(),
        };
        if credentials.user_id != receipt.account
            || uuid::Uuid::parse_str(&credentials.device_id).is_err()
        {
            return Err(invalid());
        }
        receipt.credentials = Some(credentials);
        save(engine, receipt)?;
    }
    live()?;
    client.call(
        "ack",
        Some(&remote),
        &json!({"participant_token":receipt.participant_token,"grant":receipt.grant}),
        false,
    )?;
    receipt.acknowledged = true;
    // The prior durable receipt is enough to recover a lost acknowledgment response.
    let _ = save(engine, receipt);
    live()?;
    commit(engine, client, receipt, expected_auth, &live)?;
    Ok(true)
}

pub(super) fn cancel(
    engine: &PennelEngine,
    client: &Client,
    receipt: &mut Receipt,
) -> Result<bool, PennelError> {
    receipt.runtime_generation = Some(engine.cloud_link.lock().unwrap().generation);
    receipt.cancel_requested = true;
    save(engine, receipt)?;
    advance(engine, client, receipt, None, || Ok(()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cloud::sync::api::{SyncApi, fake::FakeApi};
    use crate::cloud::tests::MemorySecrets;
    use crate::cloud::{REFRESH_ACCOUNT, SecretStore};
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicBool, Ordering};

    #[derive(Default)]
    struct FaultStore {
        inner: MemorySecrets,
        fail_refresh: AtomicBool,
    }
    impl SecretStore for FaultStore {
        fn get(&self, account: String) -> Option<String> {
            self.inner.get(account)
        }
        fn set(&self, account: String, value: Option<String>) {
            if account != REFRESH_ACCOUNT || !self.fail_refresh.load(Ordering::SeqCst) {
                self.inner.set(account, value);
            }
        }
    }

    fn fixture(
        url: &str,
    ) -> (
        tempfile::TempDir,
        Arc<PennelEngine>,
        Arc<FaultStore>,
        Arc<FakeApi>,
        Receipt,
    ) {
        let directory = tempfile::tempdir().unwrap();
        let engine = PennelEngine::new(directory.path().to_string_lossy().into_owned()).unwrap();
        let store = Arc::new(FaultStore::default());
        engine.set_secret_store(store.clone()).unwrap();
        engine.use_cloud_session(Session::new(url.into(), store.clone()));
        let api = Arc::new(FakeApi::new());
        engine.use_sync_api(api.clone());
        let key = VaultKey::generate();
        let code = super::super::super::crypto::new_recovery_code();
        let (wrapped, kdf) = super::super::super::crypto::wrap(&key, &code).unwrap();
        api.put_vault(1, &wrapped, &kdf, true).unwrap();
        let receipt = Receipt {
            version: 1,
            origin: url.into(),
            request_id: "ABCD-EFGH".into(),
            participant_token: random_token(),
            grant: random_token(),
            account: "00000000-0000-4000-8000-000000000002".into(),
            existing_device: None,
            vault_key: key.to_b64(),
            generation: 1,
            transcript_hash: "a".repeat(64),
            previous_key: None,
            previous_owner: None,
            previous_sync: false,
            previous_generation: None,
            expires_at_ms: now_ms() + 600_000,
            account_email: "fixture@example.test".into(),
            local_prepared: true,
            redeem_attempted: true,
            credentials: Some(Credentials {
                access_token: "fixture-access".into(),
                refresh_token: "fixture-refresh".into(),
                expires_in: 900,
                user_id: "00000000-0000-4000-8000-000000000002".into(),
                device_id: "00000000-0000-4000-8000-000000000004".into(),
            }),
            acknowledged: false,
            cancel_requested: false,
            runtime_generation: None,
        };
        save(&engine, &receipt).unwrap();
        store_verified(store.as_ref(), VAULT_KEY_ACCOUNT, &receipt.vault_key).unwrap();
        store_verified(store.as_ref(), VAULT_OWNER_ACCOUNT, &receipt.account).unwrap();
        engine
            .with_sync_state(|state| {
                state.use_vault(&super::super::super::crypto::vault_check(&key))?;
                state.set_generation(1)
            })
            .unwrap();
        (directory, engine, store, api, receipt)
    }

    #[test]
    fn concurrent_account_reads_cannot_use_or_clear_provisional_credentials() {
        let mut server = mockito::Server::new();
        let (_directory, engine, store, _api, mut receipt) = fixture(&server.url());
        let (entered, observed) = std::sync::mpsc::channel();
        let (release, blocked) = std::sync::mpsc::channel();
        let blocked = Arc::new(Mutex::new(blocked));
        let ack = server
            .mock("POST", "/v1/device-links/ABCD-EFGH/ack")
            .with_body_from_request(move |_| {
                entered.send(()).unwrap();
                blocked.lock().unwrap().recv().unwrap();
                br#"{"status":"complete"}"#.to_vec()
            })
            .expect(1)
            .create();
        let no_refresh = server.mock("POST", "/v1/auth/refresh").expect(0).create();
        let finishing = {
            let engine = engine.clone();
            std::thread::spawn(move || {
                let client = Client::new(engine.cloud_session().unwrap());
                advance(
                    &engine,
                    &client,
                    &mut receipt,
                    Some(&json!({"status":"provisioning"})),
                    || Ok(()),
                )
            })
        };
        observed
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        assert!(!engine.cloud_signed_in());
        assert!(store.get(REFRESH_ACCOUNT.into()).is_none());
        assert_eq!(engine.cloud_account(), Err(PennelError::NotSignedIn));
        assert_eq!(engine.cloud_sync_last().status, crate::SyncStatus::Pending);
        release.send(()).unwrap();
        assert_eq!(finishing.join().unwrap(), Ok(true));
        assert!(engine.cloud_signed_in());
        assert!(!has_pending(&engine));
        ack.assert();
        no_refresh.assert();
    }

    #[test]
    fn an_acknowledged_connection_survives_failed_public_storage_and_a_process_restart() {
        let mut server = mockito::Server::new();
        let (directory, engine, store, api, mut receipt) = fixture(&server.url());
        let ack = server
            .mock("POST", "/v1/device-links/ABCD-EFGH/ack")
            .with_body("{}")
            .expect(1)
            .create();
        store.fail_refresh.store(true, Ordering::SeqCst);
        let client = Client::new(engine.cloud_session().unwrap());
        assert!(matches!(
            advance(
                &engine,
                &client,
                &mut receipt,
                Some(&json!({"status":"provisioning"})),
                || Ok(())
            ),
            Err(PennelError::Storage { .. })
        ));
        assert!(!engine.cloud_signed_in());
        assert!(has_pending(&engine));
        assert!(load(&engine).unwrap().unwrap().credentials.is_some());
        drop(client);
        drop(engine);
        store.fail_refresh.store(false, Ordering::SeqCst);
        let engine = PennelEngine::new(directory.path().to_string_lossy().into_owned()).unwrap();
        engine.set_secret_store(store.clone()).unwrap();
        engine.use_cloud_session(Session::new(server.url(), store));
        engine.use_sync_api(api);
        let poll = server
            .mock("POST", "/v1/device-links/ABCD-EFGH/poll")
            .with_body(json!({"status":"complete","transcript_hash":"a".repeat(64)}).to_string())
            .expect(0)
            .create();
        assert_eq!(engine.cloud_link_view().phase, CloudLinkPhase::Provisioning);
        assert_eq!(
            engine.cloud_link_poll().unwrap().phase,
            CloudLinkPhase::Complete
        );
        assert!(engine.cloud_signed_in());
        assert!(engine.settings().cloud_sync);
        assert!(!has_pending(&engine));
        ack.assert();
        poll.assert();
    }

    #[test]
    fn a_lost_ack_response_recovers_after_the_server_receipt_was_purged() {
        let mut server = mockito::Server::new();
        let (_directory, engine, _store, _api, mut receipt) = fixture(&server.url());
        receipt.expires_at_ms = now_ms() - 90_000_000;
        save(&engine, &receipt).unwrap();
        let poll = server
            .mock("POST", "/v1/device-links/ABCD-EFGH/poll")
            .with_status(404)
            .with_body(r#"{"error":"pairing_unknown"}"#)
            .expect(1)
            .create();
        let identity=server.mock("GET","/v1/whoami").match_header("authorization","Bearer fixture-access")
            .with_body(json!({"user_id":receipt.account,"device_id":receipt.credentials.as_ref().unwrap().device_id}).to_string()).expect(1).create();
        assert!(!receipt.acknowledged);
        assert!(!engine.cloud_signed_in());
        assert_eq!(
            engine.cloud_link_poll().unwrap().phase,
            CloudLinkPhase::Complete
        );
        assert!(engine.cloud_signed_in());
        assert!(!has_pending(&engine));
        poll.assert();
        identity.assert();
    }

    #[test]
    fn expired_staged_access_does_not_rotate_a_refresh_token_for_acknowledgment_proof() {
        let mut server = mockito::Server::new();
        let (_directory, engine, store, _api, mut receipt) = fixture(&server.url());
        receipt.expires_at_ms = now_ms() - 90_000_000;
        save(&engine, &receipt).unwrap();
        let poll = server
            .mock("POST", "/v1/device-links/ABCD-EFGH/poll")
            .with_status(404)
            .with_body(r#"{"error":"pairing_unknown"}"#)
            .expect(1)
            .create();
        let identity = server
            .mock("GET", "/v1/whoami")
            .match_header("authorization", "Bearer fixture-access")
            .with_status(401)
            .expect(1)
            .create();
        let refresh = server
            .mock("POST", "/v1/auth/refresh")
            .with_status(503)
            .expect(0)
            .create();
        let view = engine.cloud_link_poll().unwrap();
        assert_eq!(view.phase, CloudLinkPhase::Failed);
        assert_eq!(view.error_code.as_deref(), Some("pairing_expired"));
        assert!(!has_pending(&engine));
        assert!(!engine.cloud_signed_in());
        assert!(!engine.settings().cloud_sync);
        assert!(store.get(VAULT_KEY_ACCOUNT.into()).is_none());
        poll.assert();
        identity.assert();
        refresh.assert();
    }

    #[test]
    fn transient_acknowledgment_proof_failures_preserve_the_durable_receipt() {
        for status in [403, 429, 502, 503] {
            let mut server = mockito::Server::new();
            let (_directory, engine, store, _api, receipt) = fixture(&server.url());
            let original = store.get(entry(&receipt.account)).unwrap();
            let poll = server
                .mock("POST", "/v1/device-links/ABCD-EFGH/poll")
                .with_status(404)
                .with_body(r#"{"error":"pairing_unknown"}"#)
                .expect(1)
                .create();
            let identity = server
                .mock("GET", "/v1/whoami")
                .with_status(status)
                .with_body("<html>Service unavailable</html>")
                .expect(1)
                .create();
            let refresh = server.mock("POST", "/v1/auth/refresh").expect(0).create();
            let view = engine.cloud_link_poll().unwrap();
            assert_eq!(view.phase, CloudLinkPhase::Provisioning);
            assert_eq!(
                view.error_code.as_deref(),
                Some(format!("http_{status}").as_str())
            );
            assert!(has_pending(&engine));
            assert_eq!(
                store.get(entry(&receipt.account)).as_deref(),
                Some(original.as_str())
            );
            assert_eq!(
                load(&engine)
                    .unwrap()
                    .unwrap()
                    .credentials
                    .as_ref()
                    .unwrap()
                    .refresh_token,
                "fixture-refresh"
            );
            assert!(!engine.cloud_signed_in());
            poll.assert();
            identity.assert();
            refresh.assert();
        }
    }

    #[test]
    fn malformed_transfer_never_creates_a_receipt_or_changes_active_access() {
        let server = mockito::Server::new();
        let (_directory, engine) = super::super::tests::engine_for(&server);
        let client = Client::new(engine.cloud_session().unwrap());
        let context = super::super::tests::context();
        let remote = Remote {
            id: "ABCD-EFGH".into(),
            capability: Zeroizing::new(random_token()),
            role: CloudLinkRole::NewDevice,
        };
        let store = engine.secret_store().unwrap();
        let prior_key = VaultKey::generate().to_b64();
        store.set(VAULT_KEY_ACCOUNT.into(), Some(prior_key.clone()));
        store.set(VAULT_OWNER_ACCOUNT.into(), Some("prior-owner".into()));
        for (key, grant) in [
            ("bad key".into(), random_token()),
            (VaultKey::generate().to_b64(), "!".repeat(43)),
        ] {
            let transfer = Transfer {
                version: 1,
                grant,
                vault_key: key,
                account_id: context.account.clone(),
                generation: context.generation,
                provisional_id: context.new_id.clone(),
                transcript_hash: context.hash.clone(),
            };
            assert!(
                create(
                    &engine,
                    &client,
                    &remote,
                    &context,
                    &transfer,
                    now_ms() + 600_000
                )
                .is_err()
            );
            assert!(!has_pending(&engine));
            assert_eq!(
                store.get(VAULT_KEY_ACCOUNT.into()).as_deref(),
                Some(prior_key.as_str())
            );
            assert_eq!(
                store.get(VAULT_OWNER_ACCOUNT.into()).as_deref(),
                Some("prior-owner")
            );
            assert!(!engine.settings().cloud_sync);
        }
    }

    #[test]
    fn known_ack_uses_current_public_credentials_without_replaying_a_spent_refresh() {
        let mut server = mockito::Server::new();
        let (_directory, engine, store, _api, mut receipt) = fixture(&server.url());
        receipt.acknowledged = true;
        save(&engine, &receipt).unwrap();
        let session = engine.cloud_session().unwrap();
        session.install_link_tokens(session.generation(),&json!({"access_token":"rotated-access","refresh_token":"rotated-refresh","expires_in":900})).unwrap();
        let identity=server.mock("GET","/v1/whoami").match_header("authorization","Bearer rotated-access")
            .with_body(json!({"user_id":receipt.account,"device_id":receipt.credentials.as_ref().unwrap().device_id}).to_string()).expect(1).create();
        let no_poll = server
            .mock("POST", "/v1/device-links/ABCD-EFGH/poll")
            .expect(0)
            .create();
        let no_refresh = server.mock("POST", "/v1/auth/refresh").expect(0).create();
        assert_eq!(
            engine.cloud_link_poll().unwrap().phase,
            CloudLinkPhase::Complete
        );
        assert_eq!(
            store.get(REFRESH_ACCOUNT.into()).as_deref(),
            Some("rotated-refresh")
        );
        identity.assert();
        no_poll.assert();
        no_refresh.assert();
    }

    #[test]
    fn stale_writes_cannot_remove_a_persisted_cancel_or_recreate_a_cleared_receipt() {
        let server = mockito::Server::new();
        let (_directory, engine, _store, _api, mut old) = fixture(&server.url());
        old.runtime_generation = Some(0);
        save(&engine, &old).unwrap();
        engine.cloud_link.lock().unwrap().generation = 1;
        let mut cancelled = load(&engine).unwrap().unwrap();
        cancelled.runtime_generation = Some(1);
        cancelled.cancel_requested = true;
        save(&engine, &cancelled).unwrap();
        assert_eq!(save(&engine, &old), Err(PennelError::Cancelled));
        assert!(load(&engine).unwrap().unwrap().cancel_requested);
        clear(&engine, &cancelled).unwrap();
        assert_eq!(save(&engine, &old), Err(PennelError::Cancelled));
        assert!(!has_pending(&engine));
    }

    #[test]
    fn an_identity_change_while_waiting_for_commit_gates_cannot_publish_credentials() {
        let server = mockito::Server::new();
        let (_directory, engine, store, _api, mut receipt) = fixture(&server.url());
        receipt.acknowledged = true;
        save(&engine, &receipt).unwrap();
        let gate = engine.sync_gate.lock().unwrap();
        let (entered, observed) = std::sync::mpsc::channel();
        let finishing = {
            let engine = engine.clone();
            std::thread::spawn(move || {
                let client = Client::new(engine.cloud_session().unwrap());
                let expected = client.session.generation();
                let observed = AtomicBool::new(false);
                advance(&engine, &client, &mut receipt, None, || {
                    if !observed.swap(true, Ordering::SeqCst) {
                        entered.send(()).unwrap();
                    }
                    if client.session.generation() != expected {
                        Err(PennelError::Cancelled)
                    } else {
                        Ok(())
                    }
                })
            })
        };
        observed
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        engine.cloud_login_cancel();
        drop(gate);
        assert_eq!(finishing.join().unwrap(), Err(PennelError::Cancelled));
        assert!(store.get(REFRESH_ACCOUNT.into()).is_none());
        assert!(!engine.settings().cloud_sync);
    }

    #[test]
    fn an_identity_change_while_waiting_to_prepare_cannot_write_a_staged_key() {
        let mut server = mockito::Server::new();
        let (_directory, engine, store, _api, mut receipt) = fixture(&server.url());
        receipt.local_prepared = false;
        save(&engine, &receipt).unwrap();
        store.set(VAULT_KEY_ACCOUNT.into(), None);
        let no_ack = server
            .mock("POST", "/v1/device-links/ABCD-EFGH/ack")
            .expect(0)
            .create();
        let gate = engine.sync_gate.lock().unwrap();
        let (entered, observed) = std::sync::mpsc::channel();
        let preparing = {
            let engine = engine.clone();
            std::thread::spawn(move || {
                let client = Client::new(engine.cloud_session().unwrap());
                let expected = client.session.generation();
                let called = AtomicBool::new(false);
                advance(
                    &engine,
                    &client,
                    &mut receipt,
                    Some(&json!({"status":"approved"})),
                    || {
                        if !called.swap(true, Ordering::SeqCst) {
                            entered.send(()).unwrap();
                        }
                        if client.session.generation() != expected {
                            Err(PennelError::Cancelled)
                        } else {
                            Ok(())
                        }
                    },
                )
            })
        };
        observed
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        engine.cloud_login_cancel();
        drop(gate);
        assert_eq!(preparing.join().unwrap(), Err(PennelError::Cancelled));
        assert!(store.get(VAULT_KEY_ACCOUNT.into()).is_none());
        no_ack.assert();
    }
}
