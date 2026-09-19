//! Product configuration wire; shared balance contracts own execution admission.
use std::num::NonZeroU64;

use mfm_chain::balance::{BalanceRequest, BalanceSource, DecimalScale};
use mfm_chain::{BalanceTarget, LedgerIdentity};
use mfm_evm::{EvmAddress, EvmBalanceLedger, EvmBalanceTarget};
use mfm_program_derive::MfmValue;
use mfm_values::Object;
use serde::{Deserialize, Serialize};

use super::EvmPortfolioClientError;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
pub(super) struct PortfolioBalanceSourceConfig {
    pub(super) source_id: String,
    pub(super) chain_id: NonZeroU64,
    pub(super) address: EvmAddress,
    pub(super) token: Option<EvmAddress>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, MfmValue)]
#[serde(deny_unknown_fields)]
pub(super) struct PortfolioBalanceRequestConfig {
    sources: Vec<PortfolioBalanceSourceConfig>,
    decimals: DecimalScale,
}
impl PortfolioBalanceRequestConfig {
    pub(super) fn new(
        sources: Vec<PortfolioBalanceSourceConfig>,
        decimals: DecimalScale,
    ) -> Result<Self, EvmPortfolioClientError> {
        let value = Self { sources, decimals };
        value.to_request()?;
        Ok(value)
    }
    pub(super) fn sources(&self) -> &[PortfolioBalanceSourceConfig] {
        &self.sources
    }
    pub(super) fn to_request(&self) -> Result<BalanceRequest, EvmPortfolioClientError> {
        let mut sources = Vec::with_capacity(self.sources.len());
        for source in &self.sources {
            let ledger =
                LedgerIdentity::new(Object::from_value(&EvmBalanceLedger::new(source.chain_id))?);
            let native = Object::from_value(&EvmBalanceTarget::new(
                source.address.clone(),
                source.token.clone(),
            ))?;
            sources.push(BalanceSource::new(
                source.source_id.clone(),
                BalanceTarget::new(ledger, native),
            )?);
        }
        Ok(BalanceRequest::new(sources, self.decimals)?)
    }
}
impl<'de> Deserialize<'de> for PortfolioBalanceRequestConfig {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            sources: Vec<PortfolioBalanceSourceConfig>,
            decimals: DecimalScale,
        }
        let wire = Wire::deserialize(deserializer)?;
        Self::new(wire.sources, wire.decimals).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configuration_wire_roundtrips_and_converts_to_exact_shared_targets() {
        let wire = serde_json::json!({
            "sources": [
                {"source_id": "native", "chain_id": 1,
                 "address": "0x1111111111111111111111111111111111111111", "token": null},
                {"source_id": "token", "chain_id": 1,
                 "address": "0x1111111111111111111111111111111111111111",
                 "token": "0x2222222222222222222222222222222222222222"}
            ],
            "decimals": 6
        });
        let config: PortfolioBalanceRequestConfig = serde_json::from_value(wire.clone()).unwrap();
        assert_eq!(serde_json::to_value(&config).unwrap(), wire);
        let request = config.to_request().unwrap();
        assert_eq!(request.decimals().get(), 6);
        for (index, source) in request.sources().iter().enumerate() {
            let expected = &config.sources()[index];
            assert_eq!(source.source_id(), expected.source_id);
            assert_eq!(
                source
                    .target()
                    .ledger()
                    .native()
                    .decode::<EvmBalanceLedger>()
                    .unwrap()
                    .chain_id(),
                expected.chain_id
            );
            let target = source
                .target()
                .native()
                .decode::<EvmBalanceTarget>()
                .unwrap();
            assert_eq!(target.account(), &expected.address);
            assert_eq!(target.token(), expected.token.as_ref());
        }
        let restored = Object::from_value(&config)
            .unwrap()
            .decode::<PortfolioBalanceRequestConfig>()
            .unwrap();
        assert_eq!(restored, config);

        for (field, value) in [
            ("source_id", serde_json::json!("native")),
            ("chain_id", serde_json::json!(2)),
            ("address", serde_json::json!("invalid")),
            ("extra", serde_json::json!(true)),
        ] {
            let mut changed = wire.clone();
            changed["sources"][1][field] = value;
            assert!(serde_json::from_value::<PortfolioBalanceRequestConfig>(changed).is_err());
        }
        let mut changed = wire;
        changed["decimals"] = serde_json::json!(31);
        assert!(serde_json::from_value::<PortfolioBalanceRequestConfig>(changed).is_err());
        let mut duplicate = config.sources().to_vec();
        duplicate[1].source_id = duplicate[0].source_id.clone();
        let error = PortfolioBalanceRequestConfig::new(duplicate, config.decimals).unwrap_err();
        assert!(matches!(
            error,
            EvmPortfolioClientError::Request(mfm_chain::balance::BalanceRequestError::Duplicate {
                first: 0,
                duplicate: 1
            })
        ));
        assert!(std::error::Error::source(&error).is_some());
    }
}
