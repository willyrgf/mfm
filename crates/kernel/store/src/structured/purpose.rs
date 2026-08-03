//! Sealed purpose-specific run history readers.

use mfm_ids::RunId;

use super::backend::{StructuredHistoryBackend, StructuredRunHistoryReader};
use super::fold::VerifiedStructuredRun;
use super::Result;

macro_rules! purpose_reader {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        pub struct $name<B: StructuredHistoryBackend> {
            reader: StructuredRunHistoryReader<B>,
        }

        impl<B: StructuredHistoryBackend> $name<B> {
            pub(super) fn new(reader: StructuredRunHistoryReader<B>) -> Self {
                Self { reader }
            }

            /// Returns the immutable qualified store identity.
            pub fn store_identity(&self) -> &super::StructuredStoreIdentity {
                self.reader.store_identity()
            }

            /// Probes backend readability without requiring an existing application run.
            pub async fn check_ready(&self) -> Result<()> {
                self.reader.check_ready().await
            }

            /// Loads and callback-free verifies one recorded run for this purpose only.
            pub async fn load(&self, run_id: &RunId) -> Result<VerifiedStructuredRun> {
                self.reader.load_verified(run_id).await
            }
        }
    };
}

purpose_reader!(
    /// Target-bound public-read projection authority.
    PublicRunReader
);
purpose_reader!(
    /// Target-bound transition-trace projection authority.
    TraceRunReader
);
purpose_reader!(
    /// Target-bound access-audit projection authority.
    AuditRunReader
);
purpose_reader!(
    /// Target-bound recorded-replay projection authority.
    ReplayRunReader
);
purpose_reader!(
    /// Target-bound export projection authority.
    ExportRunReader
);
