//! Reusable EVM runtime states grouped by execution intent.
//!
//! Use [`read`] for deterministic read-only RPC queries that normalize results into context. Use
//! [`write`] for contract deployment, post-deploy configuration, and validation flows that depend
//! on prepared artifacts and assertions from [`crate::dcv`].

/// Reusable EVM contract-set deployment states.
pub mod contract_set;
/// Reusable EVM oracle and valuation-source helpers.
pub mod price;
/// Reusable EVM read/query states.
pub mod read;
/// Reusable rpc.control source preparation helpers.
pub mod rpc_control;
/// Reusable EVM deploy/configure/validate states.
pub mod write;
