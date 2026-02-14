use crate::commands::result::{CommandError, CommandOutput, CommandResult};
use crate::commands::CommandContext;
use crate::presentation::output::handle_command_result;
use crate::support::evm_tx_signing::{
    parse_address, parse_data_hex, parse_u128_quantity, sign_eip1559_transaction,
    write_raw_transaction_file, Eip1559TxToSign,
};
use crate::support::keystore_manager::KeystoreManager;
use clap::Args;
use serde::Serialize;
use std::fmt;
use std::path::PathBuf;

#[derive(Args)]
pub struct TxSignArgs {
    /// Key ID (UUID) to use for signing
    #[arg(long)]
    pub id: Option<String>,

    /// Key label to use for signing
    #[arg(long)]
    pub by_label: Option<String>,

    /// Destination address (0x-prefixed)
    #[arg(long)]
    pub to: String,

    /// Transfer value in wei (decimal or 0x-prefixed hex)
    #[arg(long)]
    pub value_wei: String,

    /// EVM chain id
    #[arg(long)]
    pub chain_id: u64,

    /// Sender nonce
    #[arg(long)]
    pub nonce: u64,

    /// Max fee per gas (wei, decimal or 0x-prefixed hex)
    #[arg(long)]
    pub max_fee_per_gas: String,

    /// Max priority fee per gas (wei, decimal or 0x-prefixed hex)
    #[arg(long)]
    pub max_priority_fee_per_gas: String,

    /// Gas limit
    #[arg(long)]
    pub gas_limit: u64,

    /// Output file path where the signed raw transaction hex will be written
    #[arg(long)]
    pub out: PathBuf,

    /// Transaction calldata (0x-prefixed hex), defaults to empty calldata
    #[arg(long, default_value = "0x")]
    pub data: String,

    /// Keystore file path
    #[arg(long)]
    pub keystore: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TxSignResponse {
    from: String,
    to: String,
    nonce: u64,
    chain_id: u64,
    tx_type: String,
    payload_hash: String,
    out_path: String,
}

impl fmt::Display for TxSignResponse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Signed EIP-1559 tx (type {}) from {} to {} and wrote payload to {}",
            self.tx_type, self.from, self.to, self.out_path
        )
    }
}

pub async fn execute(ctx: &CommandContext, args: &TxSignArgs) -> ! {
    let result = execute_internal(ctx, args).await;
    handle_command_result(result, &ctx.output_format);
}

async fn execute_internal(
    _ctx: &CommandContext,
    args: &TxSignArgs,
) -> CommandResult<TxSignResponse> {
    let manager = KeystoreManager::new(args.keystore.clone());
    let mut keystore = manager
        .get_unlocked_keystore()
        .await
        .map_err(|e| CommandError::new("KeystoreError", e.to_string()))?;

    let key_id =
        KeystoreManager::resolve_key_id(&keystore, args.id.as_deref(), args.by_label.as_deref())?;
    let to = parse_address(&args.to, "to")?;
    let value_wei = parse_u128_quantity(&args.value_wei, "value-wei")?;
    let max_fee_per_gas = parse_u128_quantity(&args.max_fee_per_gas, "max-fee-per-gas")?;
    let max_priority_fee_per_gas =
        parse_u128_quantity(&args.max_priority_fee_per_gas, "max-priority-fee-per-gas")?;
    let data = parse_data_hex(&args.data)?;

    let tx = Eip1559TxToSign {
        to,
        value_wei,
        chain_id: args.chain_id,
        nonce: args.nonce,
        max_fee_per_gas,
        max_priority_fee_per_gas,
        gas_limit: args.gas_limit,
        data,
    };

    let signed = sign_eip1559_transaction(&mut keystore, key_id, &tx)?;
    write_raw_transaction_file(&args.out, &signed.raw_tx_hex)?;

    Ok(CommandOutput::new(TxSignResponse {
        from: signed.from,
        to: format!("{to:?}"),
        nonce: args.nonce,
        chain_id: args.chain_id,
        tx_type: "0x2".to_string(),
        payload_hash: signed.payload_hash,
        out_path: args.out.display().to_string(),
    }))
}
