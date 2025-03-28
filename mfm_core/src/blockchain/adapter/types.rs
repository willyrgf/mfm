use alloy_primitives::{
    Address as AlloyAddress, Bytes as AlloyBytes, B256 as AlloyB256, U256 as AlloyU256,
};
use hex;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;
use std::str::FromStr;
use thiserror::Error;

/// Error type for adapter
#[derive(Error, Debug)]
pub enum AdapterError {
    /// Invalid hex character
    #[error("Invalid hex character: {0}")]
    InvalidHexCharacter(char),

    /// Invalid hex length
    #[error("Invalid length: expected {expected}, got {actual}")]
    InvalidLength { expected: usize, actual: usize },

    /// Generic parse error
    #[error("Parse error: {0}")]
    ParseError(String),
}

/// Ethereum address representation
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Address(pub AlloyAddress);

impl Address {
    /// Returns a zero address
    pub fn zero() -> Self {
        Self(AlloyAddress::ZERO)
    }
}

impl Default for Address {
    fn default() -> Self {
        Self::zero()
    }
}

impl FromStr for Address {
    type Err = AdapterError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        AlloyAddress::from_str(s)
            .map(Address)
            .map_err(|e| AdapterError::ParseError(e.to_string()))
    }
}

impl From<AlloyAddress> for Address {
    fn from(address: AlloyAddress) -> Self {
        Address(address)
    }
}

impl From<Address> for AlloyAddress {
    fn from(address: Address) -> Self {
        address.0
    }
}

impl AsRef<[u8]> for Address {
    fn as_ref(&self) -> &[u8] {
        self.0.as_ref()
    }
}

impl fmt::Display for Address {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:#x}", self.0)
    }
}

// Implement serde Serialize for Address
impl Serialize for Address {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&format!("{:#x}", self.0))
    }
}

// Implement serde Deserialize for Address
impl<'de> Deserialize<'de> for Address {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        use serde::de::Error;
        let s = String::deserialize(deserializer)?;
        Address::from_str(&s).map_err(Error::custom)
    }
}

/// Byte array representation for contract data and calldata
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bytes(pub Vec<u8>);

impl From<AlloyBytes> for Bytes {
    fn from(bytes: AlloyBytes) -> Self {
        Bytes(bytes.to_vec())
    }
}

impl From<Bytes> for AlloyBytes {
    fn from(bytes: Bytes) -> Self {
        AlloyBytes::from(bytes.0)
    }
}

impl From<Vec<u8>> for Bytes {
    fn from(bytes: Vec<u8>) -> Self {
        Self(bytes)
    }
}

impl From<&[u8]> for Bytes {
    fn from(bytes: &[u8]) -> Self {
        Self(bytes.to_vec())
    }
}

impl fmt::Display for Bytes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "0x{}", hex::encode(&self.0))
    }
}

impl AsRef<[u8]> for Bytes {
    fn as_ref(&self) -> &[u8] {
        self.0.as_ref()
    }
}

/// 256-bit unsigned integer representation
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct U256(pub AlloyU256);

impl U256 {
    /// Returns a zero U256
    pub fn zero() -> Self {
        Self(AlloyU256::ZERO)
    }

    pub fn as_u128(&self) -> u128 {
        self.0.try_into().unwrap_or(u128::MAX)
    }

    /// Convert ETH units to Wei (U256)
    pub fn from_eth_units(eth_amount: f64) -> Self {
        // 1 ETH = 10^18 wei
        let wei_value = eth_amount * 1_000_000_000_000_000_000.0;
        Self(AlloyU256::from_str(&wei_value.to_string()).unwrap_or_default())
    }
}

impl FromStr for U256 {
    type Err = AdapterError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        AlloyU256::from_str(s)
            .map(U256)
            .map_err(|e| AdapterError::ParseError(e.to_string()))
    }
}

impl fmt::Display for U256 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

// Implement serde Serialize for U256
impl Serialize for U256 {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.0.to_string())
    }
}

// Implement serde Deserialize for U256
impl<'de> Deserialize<'de> for U256 {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        use serde::de::Error;
        let s = String::deserialize(deserializer)?;
        U256::from_str(&s).map_err(Error::custom)
    }
}

// derive from u64
impl From<u64> for U256 {
    fn from(value: u64) -> Self {
        Self(AlloyU256::from(value))
    }
}

// derive from alloy U256
impl From<AlloyU256> for U256 {
    fn from(value: AlloyU256) -> Self {
        Self(value)
    }
}

impl From<U256> for AlloyU256 {
    fn from(value: U256) -> Self {
        value.0
    }
}

/// 32-byte array representation for hashes and private keys
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct B256(pub AlloyB256);

impl B256 {
    /// Returns a zero B256
    pub fn zero() -> Self {
        Self(AlloyB256::ZERO)
    }
}

impl Default for B256 {
    fn default() -> Self {
        Self::zero()
    }
}

impl FromStr for B256 {
    type Err = AdapterError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        AlloyB256::from_str(s)
            .map(B256)
            .map_err(|e| AdapterError::ParseError(e.to_string()))
    }
}

impl From<AlloyB256> for B256 {
    fn from(value: AlloyB256) -> Self {
        Self(value)
    }
}

impl From<B256> for AlloyB256 {
    fn from(value: B256) -> Self {
        value.0
    }
}

impl AsRef<[u8]> for B256 {
    fn as_ref(&self) -> &[u8] {
        self.0.as_ref()
    }
}

impl fmt::Display for B256 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:#x}", self.0)
    }
}

// Implement serde Serialize for B256
impl Serialize for B256 {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&format!("{:#x}", self.0))
    }
}

// Implement serde Deserialize for B256
impl<'de> Deserialize<'de> for B256 {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        use serde::de::Error;
        let s = String::deserialize(deserializer)?;
        B256::from_str(&s).map_err(Error::custom)
    }
}

// Helper for creating addresses in tests or code
#[macro_export]
macro_rules! address {
    ($addr:expr) => {
        alloy_primitives::address!($addr)
    };
}

// Helper for creating bytes in tests or code
#[macro_export]
macro_rules! bytes {
    ($bytes:expr) => {
        alloy_primitives::bytes!($bytes)
    };
}

// Helper for creating b256 values in tests or code
#[macro_export]
macro_rules! b256 {
    ($b256:expr) => {
        alloy_primitives::b256!($b256)
    };
}
