use crate::commands::result::CommandError;
use alloy_primitives::{keccak256, Address, PrimitiveSignature, B256};
use mfm_op_keystore::Keystore;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;
use uuid::Uuid;

#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

#[derive(Debug, Clone)]
pub struct Eip1559TxToSign {
    pub to: Address,
    pub value_wei: u128,
    pub chain_id: u64,
    pub nonce: u64,
    pub max_fee_per_gas: u128,
    pub max_priority_fee_per_gas: u128,
    pub gas_limit: u64,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct SignedEip1559Tx {
    pub from: String,
    pub payload_hash: String,
    pub raw_tx_hex: String,
}

pub fn parse_address(raw: &str, field_name: &str) -> Result<Address, CommandError> {
    raw.parse::<Address>().map_err(|_| {
        CommandError::new(
            "InvalidAddress",
            format!("{field_name} must be a valid 0x-prefixed Ethereum address"),
        )
    })
}

pub fn parse_u128_quantity(raw: &str, field_name: &str) -> Result<u128, CommandError> {
    let value = raw.trim();
    if value.is_empty() {
        return Err(CommandError::new(
            "InvalidQuantity",
            format!("{field_name} cannot be empty"),
        ));
    }

    if let Some(hex) = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
    {
        if hex.is_empty() {
            return Err(CommandError::new(
                "InvalidQuantity",
                format!("{field_name} hex quantity is empty"),
            ));
        }
        return u128::from_str_radix(hex, 16).map_err(|_| {
            CommandError::new(
                "InvalidQuantity",
                format!("{field_name} must be a valid u128 decimal or hex value"),
            )
        });
    }

    value.parse::<u128>().map_err(|_| {
        CommandError::new(
            "InvalidQuantity",
            format!("{field_name} must be a valid u128 decimal or hex value"),
        )
    })
}

pub fn parse_data_hex(raw: &str) -> Result<Vec<u8>, CommandError> {
    let value = raw.trim();
    let normalized = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
        .ok_or_else(|| CommandError::new("InvalidData", "data must be 0x-prefixed hex"))?;
    if normalized.is_empty() {
        return Ok(Vec::new());
    }
    if normalized.len() % 2 != 0 {
        return Err(CommandError::new(
            "InvalidData",
            "data must contain an even number of hex characters",
        ));
    }
    hex::decode(normalized).map_err(|_| CommandError::new("InvalidData", "data must be valid hex"))
}

pub fn sign_eip1559_transaction(
    keystore: &mut Keystore,
    key_id: Uuid,
    tx: &Eip1559TxToSign,
) -> Result<SignedEip1559Tx, CommandError> {
    if tx.max_priority_fee_per_gas > tx.max_fee_per_gas {
        return Err(CommandError::new(
            "InvalidFeeConfig",
            "max-priority-fee-per-gas must be <= max-fee-per-gas",
        ));
    }

    let secure_key = keystore
        .get_private_key(key_id)
        .map_err(|e| CommandError::new("KeystoreError", e.to_string()))?;
    let from_address = secure_key
        .ethereum_address()
        .map_err(|e| CommandError::new("SigningError", e.to_string()))?;

    let unsigned = encode_eip1559_unsigned_payload(tx);
    let mut preimage = vec![0x02];
    preimage.extend_from_slice(&unsigned);

    let hash: B256 = keccak256(&preimage);
    let mut hash_bytes = [0u8; 32];
    hash_bytes.copy_from_slice(hash.as_slice());

    let signature = secure_key
        .sign_hash(&hash_bytes)
        .map_err(|e| CommandError::new("SigningError", e.to_string()))?;
    let signed_sig = derive_signature_with_matching_recovery_id(&signature, &hash, from_address)?;

    let signed = encode_eip1559_signed_payload(tx, signed_sig);
    let mut raw = vec![0x02];
    raw.extend_from_slice(&signed);

    Ok(SignedEip1559Tx {
        from: format!("{from_address:?}"),
        payload_hash: format!("0x{}", hex::encode(hash.as_slice())),
        raw_tx_hex: format!("0x{}", hex::encode(raw)),
    })
}

pub fn write_raw_transaction_file(path: &Path, raw_tx_hex: &str) -> Result<(), CommandError> {
    let mut options = OpenOptions::new();
    options.create(true).truncate(true).write(true);

    #[cfg(unix)]
    {
        options.mode(0o600);
    }

    let mut file = options.open(path).map_err(|e| {
        CommandError::new(
            "FileWriteError",
            format!("Failed to open output file '{}': {e}", path.display()),
        )
    })?;

    file.write_all(raw_tx_hex.as_bytes()).map_err(|e| {
        CommandError::new(
            "FileWriteError",
            format!(
                "Failed to write signed transaction file '{}': {e}",
                path.display()
            ),
        )
    })?;

    #[cfg(unix)]
    {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).map_err(|e| {
            CommandError::new(
                "FileWriteError",
                format!(
                    "Failed to set output file permissions '{}': {e}",
                    path.display()
                ),
            )
        })?;
    }

    Ok(())
}

fn derive_signature_with_matching_recovery_id(
    signature: &k256::ecdsa::Signature,
    hash: &B256,
    expected_from: Address,
) -> Result<PrimitiveSignature, CommandError> {
    for parity in [false, true] {
        let candidate = PrimitiveSignature::from_signature_and_parity(*signature, parity);
        if let Ok(recovered) = candidate.recover_address_from_prehash(hash) {
            if recovered == expected_from {
                return Ok(candidate);
            }
        }
    }

    Err(CommandError::new(
        "SigningError",
        "Failed to derive recovery id for signed transaction",
    ))
}

fn encode_eip1559_unsigned_payload(tx: &Eip1559TxToSign) -> Vec<u8> {
    rlp_encode_list(&[
        rlp_encode_bytes(&u64_to_min_be(tx.chain_id)),
        rlp_encode_bytes(&u64_to_min_be(tx.nonce)),
        rlp_encode_bytes(&u128_to_min_be(tx.max_priority_fee_per_gas)),
        rlp_encode_bytes(&u128_to_min_be(tx.max_fee_per_gas)),
        rlp_encode_bytes(&u64_to_min_be(tx.gas_limit)),
        rlp_encode_bytes(tx.to.as_slice()),
        rlp_encode_bytes(&u128_to_min_be(tx.value_wei)),
        rlp_encode_bytes(&tx.data),
        rlp_encode_list(&[]),
    ])
}

fn encode_eip1559_signed_payload(tx: &Eip1559TxToSign, sig: PrimitiveSignature) -> Vec<u8> {
    let parity = if sig.v() { 1u8 } else { 0u8 };
    let r = trim_leading_zero_bytes(&sig.r().to_be_bytes::<32>());
    let s = trim_leading_zero_bytes(&sig.s().to_be_bytes::<32>());

    rlp_encode_list(&[
        rlp_encode_bytes(&u64_to_min_be(tx.chain_id)),
        rlp_encode_bytes(&u64_to_min_be(tx.nonce)),
        rlp_encode_bytes(&u128_to_min_be(tx.max_priority_fee_per_gas)),
        rlp_encode_bytes(&u128_to_min_be(tx.max_fee_per_gas)),
        rlp_encode_bytes(&u64_to_min_be(tx.gas_limit)),
        rlp_encode_bytes(tx.to.as_slice()),
        rlp_encode_bytes(&u128_to_min_be(tx.value_wei)),
        rlp_encode_bytes(&tx.data),
        rlp_encode_list(&[]),
        rlp_encode_bytes(&[parity]),
        rlp_encode_bytes(&r),
        rlp_encode_bytes(&s),
    ])
}

fn trim_leading_zero_bytes(bytes: &[u8]) -> Vec<u8> {
    let mut idx = 0usize;
    while idx < bytes.len() && bytes[idx] == 0 {
        idx += 1;
    }
    bytes[idx..].to_vec()
}

fn u64_to_min_be(mut value: u64) -> Vec<u8> {
    if value == 0 {
        return Vec::new();
    }

    let mut out = Vec::new();
    while value > 0 {
        out.push((value & 0xff) as u8);
        value >>= 8;
    }
    out.reverse();
    out
}

fn u128_to_min_be(mut value: u128) -> Vec<u8> {
    if value == 0 {
        return Vec::new();
    }

    let mut out = Vec::new();
    while value > 0 {
        out.push((value & 0xff) as u8);
        value >>= 8;
    }
    out.reverse();
    out
}

fn rlp_encode_bytes(bytes: &[u8]) -> Vec<u8> {
    if bytes.len() == 1 && bytes[0] < 0x80 {
        return vec![bytes[0]];
    }

    let mut out = Vec::new();
    if bytes.len() <= 55 {
        out.push(0x80 + bytes.len() as u8);
        out.extend_from_slice(bytes);
        return out;
    }

    let len_bytes = usize_to_min_be(bytes.len());
    out.push(0xb7 + len_bytes.len() as u8);
    out.extend_from_slice(&len_bytes);
    out.extend_from_slice(bytes);
    out
}

fn rlp_encode_list(items: &[Vec<u8>]) -> Vec<u8> {
    let mut payload = Vec::new();
    for item in items {
        payload.extend_from_slice(item);
    }

    let mut out = Vec::new();
    if payload.len() <= 55 {
        out.push(0xc0 + payload.len() as u8);
        out.extend_from_slice(&payload);
        return out;
    }

    let len_bytes = usize_to_min_be(payload.len());
    out.push(0xf7 + len_bytes.len() as u8);
    out.extend_from_slice(&len_bytes);
    out.extend_from_slice(&payload);
    out
}

fn usize_to_min_be(mut value: usize) -> Vec<u8> {
    if value == 0 {
        return vec![0];
    }

    let mut out = Vec::new();
    while value > 0 {
        out.push((value & 0xff) as u8);
        value >>= 8;
    }
    out.reverse();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_u128_quantity_supports_decimal_and_hex() {
        assert_eq!(parse_u128_quantity("42", "value").expect("decimal"), 42);
        assert_eq!(parse_u128_quantity("0x2a", "value").expect("hex"), 42);
    }

    #[test]
    fn parse_data_hex_rejects_non_prefixed_input() {
        let err = parse_data_hex("1234").expect_err("missing prefix should fail");
        assert_eq!(err.code, "InvalidData");
    }

    #[test]
    fn rlp_encode_empty_list_is_c0() {
        assert_eq!(rlp_encode_list(&[]), vec![0xc0]);
    }
}
