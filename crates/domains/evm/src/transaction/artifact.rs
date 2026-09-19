use super::*;

// solc 0.8.33, --evm-version cancun, no optimizer, original managed fixture source path.
// See docs/dsl-phase-a.md for the reproducible compilation command; bytecode is not committed.
const INITCODE_SHA256: &str = "c7aed441e0afa86de84779ac168d27e4d8a29148b6834565d5930ef7c8ae855d";
pub(super) const CONFIGURE_SELECTOR: [u8; 4] = [0x1e, 0xb2, 0x5e, 0x0a];

/// Rejection of an unsupported or oversized scalar-contract artifact.
#[derive(Debug, Serialize, thiserror::Error)]
#[serde(rename_all = "snake_case")]
pub enum ScalarArtifactError {
    /// Native initcode exceeds its supported size boundary.
    #[error("scalar artifact size: {0}")]
    Size(#[from] mfm_values::SizeLimitExceeded),
    /// Initcode does not identify the maintained compiler output and ABI.
    #[error("unsupported scalar artifact")]
    Unsupported {
        /// Expected SHA-256 digest of the maintained compiler output.
        expected_sha256: &'static str,
        /// Exact digest of the supplied public initcode; its bytes are not copied into diagnostics.
        actual: mfm_ids::ContentDigest,
    },
}

/// Exact maintained scalar-contract artifact, qualified by initcode identity and fixed ABI selectors.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "scalar-contract-artifact",
    version = "1",
    schema = "mfm.evm-scalar-contract-artifact"
)]
pub struct EvmScalarContractArtifact {
    #[mfm(minimum_bytes = 0, maximum_bytes = 49152)]
    initcode: CanonicalBytes,
}
impl EvmScalarContractArtifact {
    /// Accepts only the maintained first-party fixture's exact pinned compiler output.
    pub fn new(initcode: Vec<u8>) -> Result<Self, ScalarArtifactError> {
        mfm_values::SizeLimitExceeded::check(initcode.len() as u64, MAX_EVM_INITCODE_BYTES as u64)?;
        let actual = mfm_canonical::raw_content_digest(&initcode);
        if actual.digest().to_string() != INITCODE_SHA256 {
            return Err(ScalarArtifactError::Unsupported {
                expected_sha256: INITCODE_SHA256,
                actual,
            });
        }
        Ok(Self {
            initcode: CanonicalBytes::new(initcode),
        })
    }
    /// Complete admitted creation bytes.
    pub fn initcode(&self) -> &[u8] {
        self.initcode.as_bytes()
    }
}
impl<'de> Deserialize<'de> for EvmScalarContractArtifact {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            initcode: CanonicalBytes,
        }
        Self::new(Wire::deserialize(deserializer)?.initcode.into_bytes())
            .map_err(serde::de::Error::custom)
    }
}

pub(super) fn configure_calldata(value: &mfm_chain::transaction::ConfigurationValue) -> Vec<u8> {
    // The checked input fits 256 bits, so each decimal step fits after propagating its carry.
    let mut word = [0_u8; 32];
    for digit in value.to_string().bytes() {
        let mut carry = u16::from(digit - b'0');
        for byte in word.iter_mut().rev() {
            let product = u16::from(*byte) * 10 + carry;
            *byte = product as u8;
            carry = product >> 8;
        }
    }
    let mut calldata = Vec::with_capacity(36);
    calldata.extend_from_slice(&CONFIGURE_SELECTOR);
    calldata.extend_from_slice(&word);
    calldata
}
