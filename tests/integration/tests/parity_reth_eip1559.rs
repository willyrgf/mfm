#![cfg(feature = "parity-tests")]
#![allow(clippy::disallowed_methods)]

use std::future;
use std::str::FromStr;
use std::time::Duration;

use alloy_primitives::{keccak256, Address, PrimitiveSignature, U256};
use k256::ecdsa::SigningKey;
use k256::elliptic_curve::rand_core::OsRng;
use mfm_evm::{
    EvmNetworkBinding, EvmTransactionAction, EvmTransactionConfig, EvmTransactionIntent,
    EvmTransactionSession, EvmUnsignedTransaction, EVM_EIP1559_TRANSACTION_TYPE,
};
use mfm_ids::LocalPublicId;
use mfm_signing::{
    DeterministicSigningProvider, PublicSigningIdentity, SignatureBytes, SignerRef, SigningFuture,
    SigningProvider, SigningRequest, SigningResult, SECP256K1_RFC6979_LOW_S_PROFILE_ID,
};
use mfm_transports_evm::EvmJsonRpcTransport;
use serde_json::{json, Value};

const RETH_HTTP_URL_ENV: &str = "MFM_RETH_PARITY_HTTP_URL";

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reth_estimates_and_submits_the_same_type_two_description() {
    let rpc_url = std::env::var(RETH_HTTP_URL_ENV).expect("managed Reth HTTP URL");
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .expect("parity RPC client");
    let chain_id = rpc_quantity(&client, &rpc_url, "eth_chainId", json!([])).await;
    let chain_id = u64::try_from(chain_id).expect("Reth chain id fits u64");
    let accounts = rpc_call(&client, &rpc_url, "eth_accounts", json!([]))
        .await
        .as_array()
        .expect("Reth accounts")
        .iter()
        .map(|account| {
            Address::from_str(account.as_str().expect("account address")).expect("valid account")
        })
        .collect::<Vec<_>>();
    assert!(
        accounts.len() >= 2,
        "managed Reth must expose funded dev accounts"
    );

    let signing_key = SigningKey::random(&mut OsRng);
    let sender = signing_key_address(&signing_key);
    fund_ephemeral_sender(&client, &rpc_url, chain_id, accounts[0], sender).await;

    let transport = EvmJsonRpcTransport::new().expect("transport");
    let session = transport
        .bind(
            EvmNetworkBinding::new(
                LocalPublicId::new("reth-parity").expect("network id"),
                chain_id,
            )
            .expect("network binding"),
            LocalPublicId::new("managed-reth").expect("source ref"),
            rpc_url,
            None,
        )
        .await
        .expect("checked Reth session");
    let signer_ref = SignerRef::new("ephemeral-parity-signer").expect("signer ref");
    let intent = EvmTransactionIntent::from_config(
        &EvmTransactionConfig::new(
            "reth-parity",
            chain_id,
            sender,
            signer_ref.clone(),
            Vec::new(),
        )
        .expect("transaction config"),
        &EvmTransactionAction::call(accounts[1], [], U256::from(1)).expect("native transfer"),
    )
    .expect("transaction intent");
    let nonce = session.pending_nonce(sender).await.expect("pending nonce");
    let fees = session.fee_inputs().await.expect("fee inputs");
    let estimate = intent
        .transaction_estimate(nonce, &fees)
        .expect("admitted estimate");
    let gas_limit = session.estimate_gas(&estimate).await.expect("gas estimate");
    let unsigned =
        EvmUnsignedTransaction::from_estimate(&estimate, gas_limit).expect("unsigned transaction");
    let signer = EphemeralSigner {
        signing_key,
        sender,
    };
    let signed = mfm_evm::sign_eip1559(
        &unsigned.to_signing_envelope().expect("signing envelope"),
        signer_ref,
        sender,
        &signer,
    )
    .await
    .expect("signed transaction");
    let transaction_hash = session
        .submit_raw_transaction(signed.bytes(), signed.transaction_hash())
        .await
        .expect("submitted transaction");
    assert_eq!(transaction_hash, signed.transaction_hash());

    let observed = wait_for_transaction(&session, transaction_hash).await;
    assert_eq!(estimate.transaction_type(), EVM_EIP1559_TRANSACTION_TYPE);
    assert_eq!(observed.chain_id, estimate.chain_id());
    assert_eq!(observed.nonce, estimate.nonce());
    assert_eq!(observed.from, estimate.from());
    assert_eq!(observed.to, estimate.to());
    assert_eq!(observed.value, estimate.value());
    assert_eq!(observed.input, *estimate.input());
    assert_eq!(observed.access_list, *estimate.access_list());
    assert_eq!(observed.max_fee_per_gas, estimate.max_fee_per_gas());
    assert_eq!(
        observed.max_priority_fee_per_gas,
        estimate.max_priority_fee_per_gas()
    );
    assert_eq!(observed.gas_limit, gas_limit);
}

struct EphemeralSigner {
    signing_key: SigningKey,
    sender: Address,
}

impl SigningProvider for EphemeralSigner {
    fn sign<'a>(&'a self, request: &'a SigningRequest) -> SigningFuture<'a> {
        let result = self
            .signing_key
            .sign_prehash_recoverable(request.digest().as_bytes())
            .map_err(mfm_signing::SigningError::redacted_provider_failure)
            .and_then(|(signature, recovery_id)| {
                if signature.normalize_s().is_some() {
                    return Err(mfm_signing::SigningError::redacted_provider_failure(
                        "non-canonical signature",
                    ));
                }
                let signature = PrimitiveSignature::from_signature_and_parity(
                    signature,
                    recovery_id.is_y_odd(),
                );
                let identity = PublicSigningIdentity::new(
                    request.algorithm().clone(),
                    None,
                    Some(format!("{:?}", self.sender)),
                )?;
                SigningResult::for_request(
                    request,
                    identity,
                    SignatureBytes::new(signature.as_bytes().to_vec())?,
                )
            });
        Box::pin(future::ready(result))
    }
}

impl DeterministicSigningProvider for EphemeralSigner {
    fn implementation_id(&self) -> &'static str {
        "mfm.test.ephemeral-reth-signer"
    }

    fn deterministic_profile_id(&self) -> &'static str {
        SECP256K1_RFC6979_LOW_S_PROFILE_ID
    }
}

async fn fund_ephemeral_sender(
    client: &reqwest::Client,
    rpc_url: &str,
    chain_id: u64,
    funding_account: Address,
    sender: Address,
) {
    let nonce = rpc_quantity(
        client,
        rpc_url,
        "eth_getTransactionCount",
        json!([format!("{funding_account:#x}"), "pending"]),
    )
    .await;
    let gas_price = rpc_quantity(client, rpc_url, "eth_gasPrice", json!([])).await;
    let priority = rpc_quantity(client, rpc_url, "eth_maxPriorityFeePerGas", json!([])).await;
    let transaction_hash = rpc_call(
        client,
        rpc_url,
        "eth_sendTransaction",
        json!([{
            "type": "0x2",
            "chainId": rpc_quantity_string(U256::from(chain_id)),
            "nonce": rpc_quantity_string(nonce),
            "from": format!("{funding_account:#x}"),
            "to": format!("{sender:#x}"),
            "value": "0xde0b6b3a7640000",
            "gas": "0x5208",
            "maxFeePerGas": rpc_quantity_string(gas_price),
            "maxPriorityFeePerGas": rpc_quantity_string(priority),
        }]),
    )
    .await;
    let transaction_hash = transaction_hash
        .as_str()
        .expect("funding transaction hash")
        .to_owned();
    for _ in 0..100 {
        if !rpc_call(
            client,
            rpc_url,
            "eth_getTransactionReceipt",
            json!([transaction_hash]),
        )
        .await
        .is_null()
        {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("managed Reth did not mine the funding transaction");
}

async fn wait_for_transaction(
    session: &impl EvmTransactionSession,
    transaction_hash: alloy_primitives::B256,
) -> mfm_evm::EvmObservedTransaction {
    for _ in 0..100 {
        if let Some(transaction) = session
            .transaction_by_hash(transaction_hash)
            .await
            .expect("transaction lookup")
        {
            return transaction;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("managed Reth did not expose the submitted transaction");
}

async fn rpc_quantity(
    client: &reqwest::Client,
    rpc_url: &str,
    method: &'static str,
    params: Value,
) -> U256 {
    let value = rpc_call(client, rpc_url, method, params).await;
    let raw = value.as_str().expect("RPC quantity");
    U256::from_str_radix(raw.strip_prefix("0x").expect("quantity prefix"), 16)
        .expect("valid RPC quantity")
}

async fn rpc_call(
    client: &reqwest::Client,
    rpc_url: &str,
    method: &'static str,
    params: Value,
) -> Value {
    let response = client
        .post(rpc_url)
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": method,
            "params": params,
        }))
        .send()
        .await
        .expect("RPC request")
        .error_for_status()
        .expect("RPC HTTP status")
        .json::<Value>()
        .await
        .expect("RPC response");
    assert!(response.get("error").is_none(), "RPC {method} failed");
    response.get("result").cloned().expect("RPC result")
}

fn signing_key_address(signing_key: &SigningKey) -> Address {
    let encoded = signing_key.verifying_key().to_encoded_point(false);
    let digest = keccak256(&encoded.as_bytes()[1..]);
    Address::from_slice(&digest.as_slice()[12..])
}

fn rpc_quantity_string(value: U256) -> String {
    format!("0x{value:x}")
}
