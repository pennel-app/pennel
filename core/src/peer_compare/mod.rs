//! Isolated transport qualification. This module is absent from shipping builds.

mod transfer;
pub use transfer::{PartialTransfer, TransferManifest};
mod identity;
mod native;
mod probe;
mod quinn_transport;
mod relay;
mod wire;

pub use identity::{PeerCompareIdentity, generate_peer_compare_identity};
pub use native::{PeerCompareNativeChannel, PeerCompareNativeTransport};
pub use probe::{PeerCompareIdentityStore, PeerComparisonProbe};
pub use wire::PeerCompareReport;

pub const ALPN: &[u8] = b"pennel/peer-transport-qa/1";
pub(crate) const STEP_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

#[derive(Clone, Default)]
pub(crate) struct Cancellation {
    cancelled: std::sync::Arc<std::sync::Mutex<bool>>,
    changed: std::sync::Arc<tokio::sync::Notify>,
}

impl Cancellation {
    pub(crate) fn cancel(&self) {
        *self.cancelled.lock().unwrap_or_else(|p| p.into_inner()) = true;
        self.changed.notify_waiters();
    }

    pub(crate) fn check(&self) -> Result<(), PeerCompareError> {
        self.with_active(|| Ok(()))
    }

    pub(crate) fn with_active<T>(
        &self,
        operation: impl FnOnce() -> Result<T, PeerCompareError>,
    ) -> Result<T, PeerCompareError> {
        let guard = self.cancelled.lock().unwrap_or_else(|p| p.into_inner());
        if *guard {
            return Err(PeerCompareError::Cancelled);
        }
        operation()
    }

    pub(crate) async fn wait(&self) {
        loop {
            let notified = self.changed.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if self.check().is_err() {
                return;
            }
            notified.await;
        }
    }
}

#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq, uniffi::Error)]
pub enum PeerCompareError {
    #[error("The device connection failed.")]
    Transport,
    #[error("This device is not approved for the transfer.")]
    Unauthorized,
    #[error("The private transfer configuration is invalid.")]
    InvalidConfig,
    #[error("The device connection is closed.")]
    Closed,
    #[error("The transfer manifest is invalid.")]
    InvalidManifest,
    #[error("The source changed during the transfer.")]
    SourceChanged,
    #[error("A received block failed verification.")]
    InvalidBlock,
    #[error("The transfer could not be saved.")]
    Storage,
    #[error("The transfer was cancelled.")]
    Cancelled,
    #[error("The peer sent an unsupported transfer message.")]
    InvalidProtocol,
    #[error("The transfer exceeds its resource limit.")]
    LimitExceeded,
}

// Preserve the retained file engine byte-for-byte, including its error spelling.
pub type PeerSyncError = PeerCompareError;
