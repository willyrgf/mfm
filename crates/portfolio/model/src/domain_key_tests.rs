use super::HoldingsDomainKey;
use serde_json::json;

#[test]
fn stable_domain_keys_validate_deserialized_values() {
    let key: HoldingsDomainKey =
        serde_json::from_value(json!({"holdings_key": "wallet_main"})).expect("valid key");
    assert_eq!(key.as_str(), "wallet_main");
}
