use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use crate::{DecisionRequest, DecisionResult, InferenceCancellation, PennelError};

use super::{
    ManagedLocalDeciderRuntime, ManagedLocalDeciderRuntimeStatus, VerifiedLocalDeciderArtifact,
};

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
mod mac;

/// A trusted app-bundle runtime. No PATH lookup or user-provided service URL is used.
pub struct BundledLocalDeciderRuntime {
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    inner: mac::Runtime,
}

impl BundledLocalDeciderRuntime {
    /// The Rust host must supply its own verified app resources and worker executable.
    /// This constructor verifies the component manifest/hash without loading model weights.
    pub fn new(bundle: PathBuf, executable: PathBuf) -> Result<Arc<Self>, PennelError> {
        #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
        {
            Ok(Arc::new(Self {
                inner: mac::Runtime::new(bundle, executable)?,
            }))
        }
        #[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
        {
            let _ = (bundle, executable);
            Err(super::invalid(
                "The local Decider runtime is unavailable on this device.",
            ))
        }
    }

    pub fn bundle_path(&self) -> &Path {
        #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
        {
            self.inner.bundle_path()
        }
        #[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
        {
            Path::new("")
        }
    }
}

impl ManagedLocalDeciderRuntime for BundledLocalDeciderRuntime {
    fn status(&self) -> ManagedLocalDeciderRuntimeStatus {
        #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
        {
            self.inner.status()
        }
        #[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
        {
            ManagedLocalDeciderRuntimeStatus {
                available: false,
                revision: None,
                reason_code: Some("unsupported_device".into()),
            }
        }
    }

    fn decide(
        &self,
        model: Arc<VerifiedLocalDeciderArtifact>,
        request_id: String,
        request: DecisionRequest,
        cancellation: Arc<InferenceCancellation>,
        deadline: Instant,
    ) -> Result<DecisionResult, PennelError> {
        #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
        {
            self.inner
                .decide(model, request_id, request, cancellation, deadline)
        }
        #[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
        {
            let _ = (model, request_id, request, cancellation, deadline);
            Err(super::invalid(
                "The local Decider runtime is unavailable on this device.",
            ))
        }
    }

    fn unload(&self) {
        #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
        self.inner.unload();
    }
}

/// Run before a desktop host initializes its GUI/single-instance plugins.
/// `None` means ordinary application arguments; no model or library is inspected.
pub fn managed_local_decider_worker_main(arguments: Vec<String>) -> Option<i32> {
    if arguments.get(1).map(String::as_str) != Some("--pennel-decider-worker") {
        return None;
    }
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        Some(mac::worker_main(&arguments[2..]))
    }
    #[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
    {
        Some(2)
    }
}
