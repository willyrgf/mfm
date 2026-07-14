use super::*;

pub(super) fn contract_context_json() -> serde_json::Value {
    let material = contract_artifact_material();
    json!({
        "lifecycle_key": "reth-dev-contract",
        "network": {
            "network_id": "reth-dev",
            "expected_chain_id": 31337,
            "chain_fingerprint": null,
            "finality_or_observation_policy": null
        },
        "contract_profile": {
            "profile_id": "reth-dev-contract-profile",
            "artifact_digest": material.evidence.digest.to_string(),
            "artifact_ref": material.reference,
            "interface_digest": null,
            "creation_bytecode_digest": null,
            "deployed_code_hash": null,
            "selector_event_policy_digest": null
        }
    })
}

pub(super) fn deployer_signer_json(expected_signer_address: &str) -> serde_json::Value {
    json!({
        "signer_ref": "deployer",
        "expected_signer_address": expected_signer_address
    })
}

pub(super) fn deploy_entry_config_json(expected_signer_address: &str) -> serde_json::Value {
    json!({
        "context": contract_context_json(),
        "deploy": {
            "signer": deployer_signer_json(expected_signer_address),
            "transaction": {
                "style": "eip1559",
                "max_fee_per_gas": "11",
                "max_priority_fee_per_gas": "3"
            }
        }
    })
}

pub(super) fn validate_entry_config_json() -> serde_json::Value {
    json!({
        "context": contract_context_json(),
        "import_configured": {
            "kind": "adopt_external_address",
            "adoption": {
                "address": "0x000000000000000000000000000000000000dead",
                "provenance_label": "test-configured",
                "evidence_policy": {
                    "require_code": false,
                    "allow_external_claimed_configured": true
                }
            }
        },
        "validate": validate_action_json()
    })
}

pub(super) fn lifecycle_entry_config_json(expected_signer_address: &str) -> serde_json::Value {
    json!({
        "context": contract_context_json(),
        "deploy": {
            "signer": deployer_signer_json(expected_signer_address),
            "transaction": {
                "style": "eip1559",
                "max_fee_per_gas": "11",
                "max_priority_fee_per_gas": "3"
            }
        },
        "configure": {
        "signer": deployer_signer_json(expected_signer_address),
            "calls": [
                {
                    "function": "configure",
                    "args": []
                },
                {
                    "function": "configure",
                    "args": []
                }
            ]
        },
        "validate": validate_action_json()
    })
}

pub(super) fn validate_action_json() -> serde_json::Value {
    json!({
        "read_assertions": [
            {
                "function": "ready",
                "expected": {
                    "json_text": "true"
                }
            }
        ]
    })
}

pub(super) fn artifact_json() -> serde_json::Value {
    json!({
        "abi": {
            "json_text": json!([
                {
                    "type": "constructor",
                    "inputs": []
                },
                {
                    "type": "function",
                    "name": "configure",
                    "inputs": [],
                    "outputs": [],
                    "stateMutability": "nonpayable"
                },
                {
                    "type": "function",
                    "name": "ready",
                    "inputs": [],
                    "outputs": [
                        {
                            "name": "",
                            "type": "bool"
                        }
                    ],
                    "stateMutability": "view"
                },
                {
                    "type": "event",
                    "name": "Configured",
                    "inputs": [],
                    "anonymous": false
                }
            ]).to_string()
        },
        "bytecode": {
            "json_text": json!({"object": "0x6000"}).to_string()
        }
    })
}
