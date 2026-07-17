use std::fmt;
use std::path::PathBuf;
use std::str::FromStr;

use alloy_primitives::{Address, Bytes, TxKind, U256};
use clap::Args;
use serde::Serialize;

use crate::commands::result::{CommandOutput, CommandResult, PublicError};
use crate::commands::CommandContext;
use crate::presentation::output::handle_command_result;
use crate::support::output_file::{self, CreateNewFileError};

/// Arguments for `mfm keystore tx-sign`.
#[derive(Args)]
pub(crate) struct TxSignArgs {
    /// Exact signer ref from the runtime configuration
    #[arg(long)]
    pub signer_ref: String,

    /// Expected sender address (0x-prefixed)
    #[arg(long = "from")]
    pub expected_from: String,

    /// Destination address (0x-prefixed)
    #[arg(long)]
    pub to: String,

    /// Transfer value in wei (decimal or 0x-prefixed hex)
    #[arg(long)]
    pub value_wei: String,

    /// EVM chain id (decimal or 0x-prefixed hex)
    #[arg(long)]
    pub chain_id: String,

    /// Sender nonce (decimal or 0x-prefixed hex)
    #[arg(long)]
    pub nonce: String,

    /// Max fee per gas in wei (decimal or 0x-prefixed hex)
    #[arg(long)]
    pub max_fee_per_gas: String,

    /// Max priority fee per gas in wei (decimal or 0x-prefixed hex)
    #[arg(long)]
    pub max_priority_fee_per_gas: String,

    /// Gas limit (decimal or 0x-prefixed hex)
    #[arg(long)]
    pub gas_limit: String,

    /// Explicit bearer-output file for the signed raw transaction hex
    #[arg(long)]
    pub out: PathBuf,

    /// Replace an existing regular output file
    #[arg(long)]
    pub overwrite: bool,

    /// Transaction calldata (0x-prefixed hex), defaults to empty calldata
    #[arg(long, default_value = "0x")]
    pub data: String,

    /// Runtime configuration file (default: $MFM_RUNTIME_CONFIG_FILE)
    #[arg(long)]
    pub runtime_config: Option<PathBuf>,
}

/// Response returned after writing one signed transaction bearer.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct TxSignResponse {
    from: String,
    to: String,
    nonce: String,
    chain_id: String,
    signing_digest: String,
    transaction_hash: String,
}

impl fmt::Display for TxSignResponse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Signed EIP-1559 transaction {} from {} to {} and wrote bearer output locally",
            self.transaction_hash, self.from, self.to
        )
    }
}

/// Executes the signing command and terminates the process.
pub(crate) async fn execute(ctx: &CommandContext, args: &TxSignArgs) -> ! {
    let result = execute_internal(args).await;
    handle_command_result(result, &ctx.output_format);
}

async fn execute_internal(args: &TxSignArgs) -> CommandResult<TxSignResponse> {
    let signer_ref = mfm_app::SignerRef::new(&args.signer_ref)
        .map_err(|_| PublicError::bad_request("invalid_signer_ref", "Signer ref is invalid"))?;
    let expected_from = parse_address(&args.expected_from, "from")?;
    let to = parse_address(&args.to, "to")?;
    let chain_id = parse_quantity(&args.chain_id, "chain-id")?;
    let nonce = parse_quantity(&args.nonce, "nonce")?;
    let max_priority_fee_per_gas =
        parse_quantity(&args.max_priority_fee_per_gas, "max-priority-fee-per-gas")?;
    let max_fee_per_gas = parse_quantity(&args.max_fee_per_gas, "max-fee-per-gas")?;
    let gas_limit = parse_quantity(&args.gas_limit, "gas-limit")?;
    let value = parse_quantity(&args.value_wei, "value-wei")?;
    let data = parse_data(&args.data)?;
    let envelope = mfm_app::UnsignedEip1559Envelope::new(
        chain_id,
        nonce,
        max_priority_fee_per_gas,
        max_fee_per_gas,
        gas_limit,
        TxKind::Call(to),
        value,
        Default::default(),
        data,
    )
    .map_err(|_| {
        PublicError::bad_request(
            "invalid_eip1559_transaction",
            "EIP-1559 transaction inputs are invalid",
        )
    })?;
    let signing_digest = envelope.signing_digest();
    let signed = mfm_app::sign_eip1559_transaction(
        args.runtime_config.as_deref(),
        signer_ref,
        expected_from,
        &envelope,
    )
    .await?;
    let transaction_hash = signed.transaction_hash();
    let output_path = args.out.clone();
    let overwrite = args.overwrite;
    tokio::task::spawn_blocking(move || {
        let raw_transaction_hex = format!("0x{}", hex::encode(signed.bytes()));
        output_file::publish_bearer_atomic(&output_path, raw_transaction_hex.as_bytes(), overwrite)
            .map_err(public_output_error)
    })
    .await
    .map_err(|_| {
        PublicError::internal(
            "file_write_error",
            "Signed transaction output worker failed",
        )
    })??;

    Ok(CommandOutput::new(TxSignResponse {
        from: format!("{expected_from:?}"),
        to: format!("{to:?}"),
        nonce: nonce.to_string(),
        chain_id: chain_id.to_string(),
        signing_digest: format!("{signing_digest:?}"),
        transaction_hash: format!("{transaction_hash:?}"),
    }))
}

fn parse_address(raw: &str, field: &'static str) -> Result<Address, PublicError> {
    if raw.len() != 42
        || !raw.starts_with("0x")
        || !raw.as_bytes()[2..].iter().all(u8::is_ascii_hexdigit)
    {
        return Err(invalid_input(
            field,
            "must be a 0x-prefixed 20-byte address",
        ));
    }
    Address::from_str(raw)
        .map_err(|_| invalid_input(field, "must be a 0x-prefixed 20-byte address"))
}

fn parse_quantity(raw: &str, field: &'static str) -> Result<U256, PublicError> {
    let valid = match raw.strip_prefix("0x") {
        Some(hex) => !hex.is_empty() && hex.as_bytes().iter().all(u8::is_ascii_hexdigit),
        None => !raw.is_empty() && raw.as_bytes().iter().all(u8::is_ascii_digit),
    };
    if !valid {
        return Err(invalid_input(
            field,
            "must be an unsigned decimal or 0x-prefixed hexadecimal quantity",
        ));
    }
    U256::from_str(raw).map_err(|_| {
        invalid_input(
            field,
            "must fit the canonical unsigned 256-bit quantity range",
        )
    })
}

fn parse_data(raw: &str) -> Result<Bytes, PublicError> {
    let Some(hex) = raw.strip_prefix("0x") else {
        return Err(invalid_input(
            "data",
            "must be 0x-prefixed hexadecimal bytes",
        ));
    };
    if hex.len() % 2 != 0 || !hex.as_bytes().iter().all(u8::is_ascii_hexdigit) {
        return Err(invalid_input(
            "data",
            "must be 0x-prefixed hexadecimal bytes",
        ));
    }
    hex::decode(hex)
        .map(Bytes::from)
        .map_err(|_| invalid_input("data", "must be 0x-prefixed hexadecimal bytes"))
}

fn invalid_input(field: &'static str, requirement: &'static str) -> PublicError {
    PublicError::bad_request(
        format!("invalid_{}", field.replace('-', "_")),
        format!("{field} {requirement}"),
    )
}

fn public_output_error(error: CreateNewFileError) -> PublicError {
    match error {
        CreateNewFileError::TargetExists => PublicError::bad_request(
            "file_write_error",
            "Output file already exists; pass --overwrite to replace it",
        ),
        CreateNewFileError::InvalidPath => PublicError::bad_request(
            "file_write_error",
            "Signed transaction output path is unsafe or invalid",
        ),
        CreateNewFileError::WriteFailed => PublicError::internal(
            "file_write_error",
            "Failed to publish signed transaction output",
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parsers_accept_only_documented_canonical_cli_forms() {
        assert_eq!(
            parse_quantity("42", "nonce").expect("decimal"),
            U256::from(42)
        );
        assert_eq!(
            parse_quantity("0x2a", "nonce").expect("hex"),
            U256::from(42)
        );
        for invalid in ["", "0x", "0X2a", "2_a", "-1", " 1"] {
            assert!(parse_quantity(invalid, "nonce").is_err(), "{invalid}");
        }

        assert!(parse_address("0x1111111111111111111111111111111111111111", "to").is_ok());
        assert!(parse_address("1111111111111111111111111111111111111111", "to").is_err());
        assert_eq!(parse_data("0x").expect("empty data"), Bytes::new());
        assert!(parse_data("0x0").is_err());
        assert!(parse_data("00").is_err());
    }
}
