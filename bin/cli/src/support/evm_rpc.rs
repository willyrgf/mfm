use crate::commands::result::CommandError;
use chrono::Utc;
use reqwest::Url;
use serde_json::Value;
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct RpcRawTxSubmission {
    pub tx_hash: String,
    pub rpc_url_host: String,
    pub submitted_at: String,
}

pub fn parse_rpc_url(raw: &str) -> Result<Url, CommandError> {
    let parsed = Url::parse(raw).map_err(|_| {
        CommandError::new(
            "InvalidRpcUrl",
            "rpc url must be a valid absolute URL (for example: http://127.0.0.1:8545)",
        )
    })?;
    if parsed.host_str().is_none() {
        return Err(CommandError::new(
            "InvalidRpcUrl",
            "rpc url must include a host",
        ));
    }
    Ok(parsed)
}

pub fn validate_raw_transaction_hex(raw_tx_hex: &str) -> Result<(), CommandError> {
    let value = raw_tx_hex.trim();
    if !value.starts_with("0x") {
        return Err(CommandError::new(
            "InvalidRawTransaction",
            "raw transaction must be 0x-prefixed hex",
        ));
    }
    if value.len() <= 2 || !value.len().is_multiple_of(2) {
        return Err(CommandError::new(
            "InvalidRawTransaction",
            "raw transaction must have an even number of hex characters",
        ));
    }
    if !value[2..].chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(CommandError::new(
            "InvalidRawTransaction",
            "raw transaction must contain only hex characters",
        ));
    }
    Ok(())
}

pub async fn send_raw_transaction(
    rpc_url: &str,
    raw_tx_hex: &str,
) -> Result<RpcRawTxSubmission, CommandError> {
    validate_raw_transaction_hex(raw_tx_hex)?;
    let parsed_url = parse_rpc_url(rpc_url)?;
    let host = url_host_with_port(&parsed_url);

    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "eth_sendRawTransaction",
        "params": [raw_tx_hex],
    });

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|_| CommandError::new("RpcClientInitFailed", "failed to initialize RPC client"))?;

    let response = client
        .post(parsed_url.clone())
        .json(&body)
        .send()
        .await
        .map_err(|_| CommandError::new("RpcRequestFailed", "failed to send rpc request"))?;

    if !response.status().is_success() {
        return Err(CommandError::new(
            "RpcHttpStatus",
            format!(
                "rpc endpoint returned HTTP status {}",
                response.status().as_u16()
            ),
        ));
    }

    let payload: Value = response
        .json()
        .await
        .map_err(|_| CommandError::new("RpcInvalidResponse", "rpc response was not valid JSON"))?;

    if let Some(error) = payload.get("error") {
        let code = error
            .get("code")
            .and_then(|c| c.as_i64())
            .map(|c| c.to_string())
            .unwrap_or_else(|| "unknown".to_string());
        let message = error
            .get("message")
            .and_then(|m| m.as_str())
            .unwrap_or("rpc returned an error");
        return Err(CommandError::new(
            "RpcError",
            format!("RPC error {code}: {message}"),
        ));
    }

    let tx_hash = payload
        .get("result")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            CommandError::new(
                "RpcInvalidResponse",
                "eth_sendRawTransaction returned a non-string result",
            )
        })?
        .to_string();

    validate_tx_hash(&tx_hash)?;

    Ok(RpcRawTxSubmission {
        tx_hash,
        rpc_url_host: host,
        submitted_at: Utc::now().to_rfc3339(),
    })
}

fn url_host_with_port(url: &Url) -> String {
    let host = url.host_str().unwrap_or_default();
    match url.port() {
        Some(port) => format!("{host}:{port}"),
        None => host.to_string(),
    }
}

fn validate_tx_hash(tx_hash: &str) -> Result<(), CommandError> {
    if tx_hash.len() != 66 || !tx_hash.starts_with("0x") {
        return Err(CommandError::new(
            "RpcInvalidResponse",
            "eth_sendRawTransaction returned an invalid tx hash shape",
        ));
    }
    if !tx_hash[2..].chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(CommandError::new(
            "RpcInvalidResponse",
            "eth_sendRawTransaction returned a non-hex tx hash",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_raw_transaction_hex_rejects_non_hex() {
        let err = validate_raw_transaction_hex("0x00zz").expect_err("non-hex should fail");
        assert_eq!(err.code, "InvalidRawTransaction");
    }

    #[test]
    fn parse_rpc_url_requires_host() {
        let err = parse_rpc_url("http:///").expect_err("hostless url should fail");
        assert_eq!(err.code, "InvalidRpcUrl");
    }
}
