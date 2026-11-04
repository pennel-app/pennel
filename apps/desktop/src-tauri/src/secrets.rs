//! Platform credential storage. macOS uses the app-scoped Data Protection Keychain.
use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use pennel_core::{ApiKeys, ProviderAccount, SecretStore};
use serde::{Deserialize, Serialize};

use crate::diagnostics::Log;

const LOG: Log = Log::new("secrets");

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CredentialState {
    #[default]
    Ready,
    MigrationRequired,
    Locked,
    Denied,
    Unavailable,
}

impl CredentialState {
    pub fn message(self) -> &'static str {
        match self {
            Self::Ready => "",
            Self::MigrationRequired => {
                "Update saved access in Settings before using this connection."
            }
            Self::Locked => "Unlock your Mac and retry saved access in Settings.",
            Self::Denied => "Saved access was not approved. Retry in Settings when you are ready.",
            Self::Unavailable => {
                "Saved access is unavailable. Retry in Settings or check this app's installation."
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct CredentialStoreStatus {
    pub status: CredentialState,
    pub pending: usize,
}

// This bridge response stays native; only CredentialStoreStatus goes to the webview.
#[cfg(target_os = "macos")]
#[derive(Deserialize)]
struct NativeReply {
    #[serde(flatten)]
    status: CredentialStoreStatus,
    value: Option<String>,
    #[serde(default)]
    present: bool,
}

pub struct Secrets {
    service: &'static str,
    cache: Mutex<HashMap<String, Option<String>>>,
    providers: Mutex<Vec<String>>,
    revision: AtomicU64,
    // A failed Option-returning core read must not authorize later writes in that worker.
    read_failures: Mutex<HashMap<std::thread::ThreadId, CredentialState>>,
    in_memory: bool,
    #[cfg(test)]
    fail_writes: std::sync::atomic::AtomicBool,
    #[cfg(test)]
    pub(crate) access_failure: Mutex<Option<CredentialState>>,
}

impl Default for Secrets {
    fn default() -> Self {
        Self::new(true)
    }
}

impl Secrets {
    pub fn for_capture() -> Self {
        Self {
            service: "dev.pennel.Pennel.capture.preview",
            in_memory: true,
            ..Self::new(true)
        }
    }

    pub fn new(preview: bool) -> Self {
        let service = match (cfg!(target_os = "macos"), preview) {
            (true, false) => "dev.pennel.Pennel",
            (true, true) => "dev.pennel.Pennel.preview",
            (false, false) => "app.pennel.desktop",
            (false, true) => "app.pennel.desktop.preview",
        };
        Self {
            service,
            cache: Mutex::new(HashMap::new()),
            providers: Mutex::new(Vec::new()),
            revision: AtomicU64::new(0),
            read_failures: Mutex::new(HashMap::new()),
            in_memory: false,
            #[cfg(test)]
            fail_writes: std::sync::atomic::AtomicBool::new(false),
            #[cfg(test)]
            access_failure: Mutex::new(None),
        }
    }

    #[cfg(test)]
    pub fn in_memory() -> Self {
        Self {
            in_memory: true,
            ..Self::new(true)
        }
    }

    #[cfg(test)]
    pub fn in_memory_canonical() -> Self {
        Self {
            in_memory: true,
            ..Self::new(false)
        }
    }

    pub fn configure_providers(&self, providers: &[ProviderAccount]) {
        *self.providers.lock().unwrap() = providers.iter().map(|p| p.id.clone()).collect();
    }

    fn record_failure(&self, state: CredentialState) -> String {
        self.read_failures
            .lock()
            .unwrap()
            .insert(std::thread::current().id(), state);
        state.message().into()
    }

    fn check_test_access(&self) -> Result<(), String> {
        #[cfg(test)]
        if let Some(state) = *self.access_failure.lock().unwrap() {
            return Err(self.record_failure(state));
        }
        Ok(())
    }

    #[cfg(not(target_os = "macos"))]
    fn entry(&self, account: &str) -> Result<keyring::Entry, String> {
        keyring::Entry::new(self.service, account)
            .map_err(|_| "Saved access is unavailable.".into())
    }

    #[cfg(target_os = "macos")]
    fn native(
        &self,
        operation: &str,
        account: Option<&str>,
        value: Option<&str>,
    ) -> Result<NativeReply, String> {
        let request = serde_json::json!({
            "operation": operation,
            "service": self.service,
            "account": account,
            "value": value,
            "providers": self.providers.lock().unwrap().clone(),
        });
        let reply = crate::macos::credential_store(&request)
            .and_then(|reply| {
                serde_json::from_value(reply).map_err(|_| "Invalid credential response".to_string())
            })
            .map_err(|_| self.record_failure(CredentialState::Unavailable))?;
        Ok(reply)
    }

    pub fn status(&self) -> CredentialStoreStatus {
        #[cfg(test)]
        if let Some(status) = *self.access_failure.lock().unwrap() {
            return CredentialStoreStatus {
                status,
                pending: usize::from(status == CredentialState::MigrationRequired),
            };
        }
        if self.in_memory {
            return CredentialStoreStatus::default();
        }
        #[cfg(target_os = "macos")]
        {
            self.native("status", None, None)
                .map(|reply| reply.status)
                .unwrap_or(CredentialStoreStatus {
                    status: CredentialState::Unavailable,
                    pending: 0,
                })
        }
        #[cfg(not(target_os = "macos"))]
        CredentialStoreStatus::default()
    }

    pub fn get_checked(&self, account: &str) -> Result<Option<String>, String> {
        self.check_test_access()?;
        if account.is_empty() {
            return Ok(None);
        }
        if self.in_memory {
            return Ok(self.cache.lock().unwrap().get(account).cloned().flatten());
        }
        #[cfg(target_os = "macos")]
        {
            let reply = self.native("get", Some(account), None)?;
            if reply.status.status != CredentialState::Ready {
                return Err(self.record_failure(reply.status.status));
            }
            Ok(reply.value.filter(|v| !v.is_empty()))
        }
        #[cfg(not(target_os = "macos"))]
        {
            if let Some(value) = self.cache.lock().unwrap().get(account) {
                return Ok(value.clone());
            }
            let value = match self.entry(account)?.get_password() {
                Ok(value) => Some(value).filter(|v| !v.is_empty()),
                Err(keyring::Error::NoEntry) => None,
                Err(_) => return Err(self.record_failure(CredentialState::Unavailable)),
            };
            self.cache
                .lock()
                .unwrap()
                .insert(account.into(), value.clone());
            Ok(value)
        }
    }

    pub fn get(&self, account: &str) -> Option<String> {
        self.get_checked(account).unwrap_or_else(|_| {
            LOG.error("credential store read paused");
            None
        })
    }

    /// Presence uses noninteractive metadata, including pending legacy credentials.
    pub fn has(&self, account: &str) -> Result<bool, String> {
        self.check_test_access()?;
        if account.is_empty() {
            return Ok(false);
        }
        if self.in_memory {
            return Ok(self
                .cache
                .lock()
                .unwrap()
                .get(account)
                .is_some_and(Option::is_some));
        }
        #[cfg(target_os = "macos")]
        {
            let reply = self.native("has", Some(account), None)?;
            if reply.status.status != CredentialState::Ready {
                return Err(self.record_failure(reply.status.status));
            }
            Ok(reply.present)
        }
        #[cfg(not(target_os = "macos"))]
        {
            Ok(self.get_checked(account)?.is_some())
        }
    }

    pub fn set(&self, account: &str, value: &str) -> Result<(), String> {
        self.check_test_access()?;
        if account.is_empty() {
            return Err("no account".into());
        }
        let value = value.trim();
        #[cfg(test)]
        if self.fail_writes.load(Ordering::SeqCst) {
            return Err("simulated".into());
        }
        if !self.in_memory {
            #[cfg(target_os = "macos")]
            {
                let reply =
                    self.native("set", Some(account), (!value.is_empty()).then_some(value))?;
                if reply.status.status != CredentialState::Ready {
                    return Err(self.record_failure(reply.status.status));
                }
            }
            #[cfg(not(target_os = "macos"))]
            {
                let entry = self.entry(account)?;
                if value.is_empty() {
                    match entry.delete_credential() {
                        Ok(()) | Err(keyring::Error::NoEntry) => {}
                        Err(_) => return Err(self.record_failure(CredentialState::Unavailable)),
                    }
                } else {
                    entry
                        .set_password(value)
                        .map_err(|_| self.record_failure(CredentialState::Unavailable))?;
                }
            }
        }
        self.cache
            .lock()
            .unwrap()
            .insert(account.into(), (!value.is_empty()).then(|| value.into()));
        self.revision.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    pub fn revision(&self) -> u64 {
        self.revision.load(Ordering::SeqCst)
    }

    pub fn keys_checked(&self, providers: &[ProviderAccount]) -> Result<ApiKeys, String> {
        let mut keys = ApiKeys {
            by_provider: HashMap::new(),
        };
        for provider in providers
            .iter()
            .filter(|p| pennel_core::requires_key(p.kind))
        {
            if let Some(key) = self.get_checked(&provider.id)? {
                keys.by_provider.insert(provider.id.clone(), key);
            }
        }
        Ok(keys)
    }

    pub fn ensure_cloud_access(&self) -> Result<(), String> {
        self.read_failures
            .lock()
            .unwrap()
            .remove(&std::thread::current().id());
        self.check_test_access()?;
        #[cfg(target_os = "macos")]
        if !self.in_memory {
            // Includes dynamic pending device-link receipts as well as account/vault secrets.
            let reply = self.native("cloud_status", None, None)?;
            if reply.status.status != CredentialState::Ready {
                return Err(self.record_failure(reply.status.status));
            }
        }
        Ok(())
    }

    pub fn migrate_saved(&self, providers: &[ProviderAccount]) -> Result<(), String> {
        self.configure_providers(providers);
        if !self.in_memory {
            #[cfg(target_os = "macos")]
            {
                let reply = self.native("migrate", None, None)?;
                if reply.status.status != CredentialState::Ready {
                    return Err(self.record_failure(reply.status.status));
                }
            }
        }
        if !self.in_memory {
            self.cache.lock().unwrap().clear();
        }
        self.read_failures
            .lock()
            .unwrap()
            .remove(&std::thread::current().id());
        self.revision.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    #[cfg(any(target_os = "macos", test))]
    pub fn migrate_legacy(&self, providers: &[ProviderAccount]) -> Result<(), String> {
        if self.service.ends_with(".preview") {
            return Err("The preview does not import saved API keys.".into());
        }
        self.configure_providers(providers);
        // Startup only detects pending access. The explicit settings action performs import.
        let status = self.status();
        if status.status != CredentialState::Ready {
            return Err(status.status.message().into());
        }
        Ok(())
    }
}

impl SecretStore for Secrets {
    fn get(&self, account: String) -> Option<String> {
        Secrets::get(self, &account)
    }

    fn set(&self, account: String, value: Option<String>) {
        if self
            .read_failures
            .lock()
            .unwrap()
            .contains_key(&std::thread::current().id())
        {
            LOG.error("credential write paused after unavailable read");
            return;
        }
        if Secrets::set(self, &account, value.as_deref().unwrap_or("")).is_err() {
            LOG.error("credential store write failed");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn inaccessible_credentials_are_not_missing_or_cached_as_missing() {
        let secrets = Secrets::in_memory();
        secrets.set("provider", "saved").unwrap();
        for status in [
            CredentialState::MigrationRequired,
            CredentialState::Locked,
            CredentialState::Denied,
            CredentialState::Unavailable,
        ] {
            *secrets.access_failure.lock().unwrap() = Some(status);
            assert!(secrets.get_checked("provider").is_err());
            assert!(secrets.has("provider").is_err());
            assert!(secrets.set("provider", "replacement").is_err());
            assert!(secrets.ensure_cloud_access().is_err());
        }
        *secrets.access_failure.lock().unwrap() = None;
        assert_eq!(
            secrets.get_checked("provider").unwrap().as_deref(),
            Some("saved")
        );
        assert!(secrets.get_checked("absent").unwrap().is_none());
        assert!(secrets.ensure_cloud_access().is_ok());
    }

    #[test]
    fn credential_status_contains_no_saved_values() {
        let secrets = Secrets::in_memory();
        secrets.set("provider", "synthetic-secret").unwrap();
        let rendered = serde_json::to_value(secrets.status()).unwrap();
        assert_eq!(rendered, serde_json::json!({"status":"ready","pending":0}));
    }

    #[test]
    fn core_read_failure_blocks_writes_until_successful_worker_readmission() {
        let secrets = Secrets::in_memory();
        secrets
            .set("pennel-cloud-vault-key", "retained-key")
            .unwrap();
        let store: &dyn SecretStore = &secrets;
        *secrets.access_failure.lock().unwrap() = Some(CredentialState::Locked);
        assert!(store.get("pennel-cloud-vault-key".into()).is_none());
        *secrets.access_failure.lock().unwrap() = None;
        store.set(
            "pennel-cloud-vault-key".into(),
            Some("unwanted-replacement".into()),
        );
        assert_eq!(
            secrets
                .get_checked("pennel-cloud-vault-key")
                .unwrap()
                .as_deref(),
            Some("retained-key")
        );
        secrets.ensure_cloud_access().unwrap();
        store.set(
            "pennel-cloud-vault-key".into(),
            Some("explicit-new-value".into()),
        );
        assert_eq!(
            secrets
                .get_checked("pennel-cloud-vault-key")
                .unwrap()
                .as_deref(),
            Some("explicit-new-value")
        );
    }

    #[test]
    fn migration_does_not_readmit_a_different_failed_worker() {
        let secrets = Arc::new(Secrets::in_memory());
        secrets.set("pennel-cloud-vault-key", "retained").unwrap();
        let boundary = Arc::new(std::sync::Barrier::new(2));
        let worker = {
            let secrets = secrets.clone();
            let boundary = boundary.clone();
            std::thread::spawn(move || {
                *secrets.access_failure.lock().unwrap() = Some(CredentialState::Locked);
                let store: &dyn SecretStore = secrets.as_ref();
                assert!(store.get("pennel-cloud-vault-key".into()).is_none());
                boundary.wait();
                boundary.wait();
                store.set("pennel-cloud-vault-key".into(), Some("unwanted".into()));
            })
        };
        boundary.wait();
        *secrets.access_failure.lock().unwrap() = None;
        secrets.migrate_saved(&[]).unwrap();
        boundary.wait();
        worker.join().unwrap();
        assert_eq!(
            secrets
                .get_checked("pennel-cloud-vault-key")
                .unwrap()
                .as_deref(),
            Some("retained")
        );
    }

    #[test]
    fn native_capture_never_uses_installed_preview_credentials() {
        let capture = Secrets::for_capture();
        assert_ne!(capture.service, Secrets::new(true).service);
        assert_ne!(capture.service, Secrets::new(false).service);
        assert!(capture.service.ends_with(".preview"));
        assert!(capture.migrate_legacy(&[]).is_err());
    }

    #[test]
    fn verification_revision_changes_only_after_successful_credential_writes() {
        let secrets = Secrets::in_memory();
        let initial = secrets.revision();
        secrets.set("provider", "first synthetic key").unwrap();
        let first = secrets.revision();
        assert!(first > initial);
        secrets
            .set("provider", "replacement synthetic key")
            .unwrap();
        let replacement = secrets.revision();
        assert!(replacement > first);
        secrets.fail_writes.store(true, Ordering::SeqCst);
        assert!(secrets.set("provider", "rejected synthetic key").is_err());
        assert_eq!(secrets.revision(), replacement);
    }

    #[test]
    fn presence_reports_saved_and_removed_keys_without_exposing_values() {
        let secrets = Secrets::in_memory();
        assert!(!secrets.has("synthetic").unwrap());
        secrets.set("synthetic", "synthetic-secret").unwrap();
        assert!(secrets.has("synthetic").unwrap());
        secrets.set("synthetic", "").unwrap();
        assert!(!secrets.has("synthetic").unwrap());
    }

    #[test]
    fn secrets_impl_roundtrip() {
        let secrets = Secrets::in_memory();
        let store: &dyn SecretStore = &secrets;
        store.set("pennel-cloud-refresh".into(), Some("token".into()));
        assert_eq!(
            store.get("pennel-cloud-refresh".into()).as_deref(),
            Some("token")
        );
        store.set("pennel-cloud-refresh".into(), None);
        assert_eq!(store.get("pennel-cloud-refresh".into()), None);
    }

    #[test]
    fn shared_testing_secrets_are_a_store() {
        let dir = tempfile::tempdir().unwrap();
        let shared = crate::state::Shared::testing(dir.path().to_path_buf());
        let store: Arc<dyn SecretStore> = shared.secrets.clone();
        store.set("a".into(), Some("b".into()));
        assert_eq!(shared.secrets.get("a").as_deref(), Some("b"));
        store.set("a".into(), None);
        assert_eq!(shared.secrets.get("a"), None);
    }

    #[test]
    fn failed_write_keeps_the_old_value_and_does_not_panic() {
        let secrets = Secrets::in_memory();
        secrets.set("acct", "old").unwrap();
        secrets
            .fail_writes
            .store(true, std::sync::atomic::Ordering::SeqCst);
        assert!(secrets.set("acct", "new").is_err());
        let store: &dyn SecretStore = &secrets;
        store.set("acct".into(), Some("new".into()));
        store.set("acct".into(), None);
        assert_eq!(secrets.get("acct").as_deref(), Some("old"));
    }
}
