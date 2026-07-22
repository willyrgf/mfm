mod catalog;
mod dto;
mod query;
mod ref_id;
mod selector;
mod service;
#[cfg(any(test, feature = "test-support"))]
mod test_support;

pub use catalog::{FactCatalogService, FactPublicRefResolver};
pub use dto::{
    PublicFactDescriptorRef, PublicFactDescriptorSummary, PublicFactExplain,
    PublicFactFieldSummary, PublicFactFieldValue, PublicFactKindSummary, PublicFactOrderingSummary,
    PublicFactOrderingTermSummary, PublicFactQueryPage, PublicFactRef, PublicFactScalarValue,
};
pub use query::{PublicFactPredicate, PublicFactQueryRequest, PublicFactShapeSelector};
pub use ref_id::PublicFactRefId;
pub use selector::{
    is_public_fact_query_parameter_error, parse_public_fact_predicates, PublicFactQuerySelector,
};
pub use service::FactPublicQueryService;

#[cfg(any(test, feature = "test-support"))]
pub use test_support::{
    assert_public_fact_json_redacts_private_tokens_for_test, PublicFactFixtureForTest,
};

#[cfg(test)]
pub(crate) use ref_id::public_ref_id;
