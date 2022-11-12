use num_bigint;
use serde::{self, Deserialize, Serialize};
use std::convert::{TryFrom, TryInto};
use std::fmt::{self, Display, Formatter};
use std::ops::{Add, BitAnd, BitOr, Div, Mul, Rem, Sub};
use std::str::FromStr;
use thiserror::Error;
use web3::types::*;

pub use num_bigint::Sign as BigIntSign;

// TODO: add BigDecimalError

/// All operations on `BigDecimal` return a normalized value.
// Caveat: The exponent is currently an i64 and may overflow. See
// https://github.com/akubera/bigdecimal-rs/issues/54.
// Using `#[serde(from = "BigDecimal"]` makes sure deserialization calls `BigDecimal::new()`.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(from = "bigdecimal::BigDecimal")]
pub struct BigDecimal(bigdecimal::BigDecimal);

impl From<bigdecimal::BigDecimal> for BigDecimal {
    fn from(big_decimal: bigdecimal::BigDecimal) -> Self {
        BigDecimal(big_decimal)
    }
}

impl BigDecimal {
    /// These are the limits of IEEE-754 decimal128, a format we may want to switch to. See
    /// https://en.wikipedia.org/wiki/Decimal128_floating-point_format.
    pub const MIN_EXP: i32 = -6143;
    pub const MAX_EXP: i32 = 6144;
    pub const MAX_SIGNFICANT_DIGITS: i32 = 34;

    pub fn new(digits: BigInt, exp: i64) -> Self {
        // bigdecimal uses `scale` as the opposite of the power of ten, so negate `exp`.
        Self::from(bigdecimal::BigDecimal::new(digits.0, -exp))
    }

    pub fn with_scale(&self, new_scale: i64) -> BigDecimal {
        Self::from(self.0.with_scale(new_scale))
    }

    pub fn from_unsigned_u256(n: &U256, scale: i64) -> Self {
        let digits = BigInt::from_unsigned_u256(n);
        Self::from(bigdecimal::BigDecimal::new(digits.0, scale))
    }

    pub fn from_signed_u256(n: &U256, scale: i64) -> Self {
        let digits = BigInt::from_signed_u256(n);
        Self::from(bigdecimal::BigDecimal::new(digits.0, scale))
    }

    pub fn parse_bytes(bytes: &[u8]) -> Option<Self> {
        bigdecimal::BigDecimal::parse_bytes(bytes, 10).map(Self)
    }

    pub fn zero() -> BigDecimal {
        use bigdecimal::Zero;

        BigDecimal(bigdecimal::BigDecimal::zero())
    }

    pub fn as_bigint_and_exponent(&self) -> (num_bigint::BigInt, i64) {
        self.0.as_bigint_and_exponent()
    }

    pub fn abs(&self) -> BigDecimal {
        BigDecimal::from(self.0.abs())
    }

    pub fn to_unsigned_u256(&self) -> U256 {
        let (bigint, _) = self.as_bigint_and_exponent();
        BigInt::from(bigint).to_unsigned_u256()
    }

    pub fn to_f64(&self) -> Option<f64> {
        use bigdecimal::ToPrimitive;
        self.normalized().0.to_f64()
    }

    pub fn digits(&self) -> u64 {
        self.0.digits()
    }

    // Copy-pasted from `bigdecimal::BigDecimal::normalize`. We can use the upstream version once it
    // is included in a released version supported by Diesel.
    #[must_use]
    pub fn normalized(&self) -> BigDecimal {
        if self == &BigDecimal::zero() {
            return BigDecimal::zero();
        }

        // Round to the maximum significant digits.
        let big_decimal = self.0.with_prec(Self::MAX_SIGNFICANT_DIGITS as u64);

        let (bigint, exp) = big_decimal.as_bigint_and_exponent();
        let (sign, mut digits) = bigint.to_radix_be(10);
        let trailing_count = digits.iter().rev().take_while(|i| **i == 0).count();
        digits.truncate(digits.len() - trailing_count);
        let int_val = num_bigint::BigInt::from_radix_be(sign, &digits, 10).unwrap();
        let scale = exp - trailing_count as i64;

        BigDecimal(bigdecimal::BigDecimal::new(int_val, scale))
    }
}

impl Display for BigDecimal {
    fn fmt(&self, f: &mut Formatter) -> Result<(), fmt::Error> {
        self.0.fmt(f)
    }
}

impl fmt::Debug for BigDecimal {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "BigDecimal({})", self.0)
    }
}

impl FromStr for BigDecimal {
    type Err = <bigdecimal::BigDecimal as FromStr>::Err;

    fn from_str(s: &str) -> Result<BigDecimal, Self::Err> {
        Ok(Self::from(bigdecimal::BigDecimal::from_str(s)?))
    }
}

impl From<i32> for BigDecimal {
    fn from(n: i32) -> Self {
        Self::from(bigdecimal::BigDecimal::from(n))
    }
}

impl From<i64> for BigDecimal {
    fn from(n: i64) -> Self {
        Self::from(bigdecimal::BigDecimal::from(n))
    }
}

impl From<u64> for BigDecimal {
    fn from(n: u64) -> Self {
        Self::from(bigdecimal::BigDecimal::from(n))
    }
}

impl TryFrom<f64> for BigDecimal {
    type Error = anyhow::Error;
    fn try_from(n: f64) -> Result<Self, Self::Error> {
        Ok(Self::from(bigdecimal::BigDecimal::try_from(n).map_err(
            |e| anyhow::anyhow!("from failed to f64, error: {}", e),
        )?))
    }
}

impl Add for BigDecimal {
    type Output = Self;

    fn add(self, other: Self) -> Self {
        Self::from(self.0.add(other.0))
    }
}

impl Sub for BigDecimal {
    type Output = Self;

    fn sub(self, other: Self) -> Self {
        Self::from(self.0.sub(other.0))
    }
}

impl Mul for BigDecimal {
    type Output = Self;

    fn mul(self, other: Self) -> Self {
        Self::from(self.0.mul(other.0))
    }
}

impl Div for BigDecimal {
    type Output = Self;

    fn div(self, other: Self) -> Self {
        if other == BigDecimal::from(0) {
            panic!("Cannot divide by zero-valued `BigDecimal`!")
        }

        Self::from(self.0.div(other.0))
    }
}

impl bigdecimal::ToPrimitive for BigDecimal {
    fn to_i64(&self) -> Option<i64> {
        self.0.to_i64()
    }
    fn to_u64(&self) -> Option<u64> {
        self.0.to_u64()
    }
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct BigInt(num_bigint::BigInt);

#[derive(Error, Debug)]
pub enum BigIntOutOfRangeError {
    #[error("Cannot convert negative BigInt into type")]
    Negative,
    #[error("BigInt value is too large for type")]
    Overflow,
}

impl<'a> TryFrom<&'a BigInt> for u64 {
    type Error = BigIntOutOfRangeError;
    fn try_from(value: &'a BigInt) -> Result<u64, BigIntOutOfRangeError> {
        let (sign, bytes) = value.to_bytes_le();

        if sign == num_bigint::Sign::Minus {
            return Err(BigIntOutOfRangeError::Negative);
        }

        if bytes.len() > 8 {
            return Err(BigIntOutOfRangeError::Overflow);
        }

        // Replace this with u64::from_le_bytes when stabilized
        let mut n = 0u64;
        let mut shift_dist = 0;
        for b in bytes {
            n |= (b as u64) << shift_dist;
            shift_dist += 8;
        }
        Ok(n)
    }
}

impl TryFrom<BigInt> for u64 {
    type Error = BigIntOutOfRangeError;
    fn try_from(value: BigInt) -> Result<u64, BigIntOutOfRangeError> {
        (&value).try_into()
    }
}

impl fmt::Debug for BigInt {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "BigInt({})", self)
    }
}

impl BigInt {
    pub fn from_unsigned_bytes_le(bytes: &[u8]) -> Self {
        BigInt(num_bigint::BigInt::from_bytes_le(
            num_bigint::Sign::Plus,
            bytes,
        ))
    }

    pub fn from_signed_bytes_le(bytes: &[u8]) -> Self {
        BigInt(num_bigint::BigInt::from_signed_bytes_le(bytes))
    }

    pub fn from_signed_bytes_be(bytes: &[u8]) -> Self {
        BigInt(num_bigint::BigInt::from_signed_bytes_be(bytes))
    }

    pub fn to_bytes_le(&self) -> (BigIntSign, Vec<u8>) {
        self.0.to_bytes_le()
    }

    pub fn to_bytes_be(&self) -> (BigIntSign, Vec<u8>) {
        self.0.to_bytes_be()
    }

    pub fn to_signed_bytes_le(&self) -> Vec<u8> {
        self.0.to_signed_bytes_le()
    }

    pub fn from_unsigned_u256(n: &U256) -> Self {
        let mut bytes: [u8; 32] = [0; 32];
        n.to_little_endian(&mut bytes);
        BigInt::from_unsigned_bytes_le(&bytes)
    }

    pub fn from_signed_u256(n: &U256) -> Self {
        let mut bytes: [u8; 32] = [0; 32];
        n.to_little_endian(&mut bytes);
        BigInt::from_signed_bytes_le(&bytes)
    }

    pub fn to_signed_u256(&self) -> U256 {
        let bytes = self.to_signed_bytes_le();
        if self < &BigInt::from(0) {
            assert!(
                bytes.len() <= 32,
                "BigInt value does not fit into signed U256"
            );
            let mut i_bytes: [u8; 32] = [255; 32];
            i_bytes[..bytes.len()].copy_from_slice(&bytes);
            U256::from_little_endian(&i_bytes)
        } else {
            U256::from_little_endian(&bytes)
        }
    }

    pub fn to_unsigned_u256(&self) -> U256 {
        let (sign, bytes) = self.to_bytes_le();
        assert!(
            sign == BigIntSign::NoSign || sign == BigIntSign::Plus,
            "negative value encountered for U256: {}",
            self
        );
        U256::from_little_endian(&bytes)
    }

    pub fn pow(self, exponent: u8) -> Self {
        use num_traits::pow::Pow;

        BigInt(self.0.pow(&exponent))
    }

    pub fn bits(&self) -> usize {
        self.0.bits().try_into().unwrap()
    }
}

impl Display for BigInt {
    fn fmt(&self, f: &mut Formatter) -> Result<(), fmt::Error> {
        self.0.fmt(f)
    }
}

impl From<num_bigint::BigInt> for BigInt {
    fn from(big_int: num_bigint::BigInt) -> BigInt {
        BigInt(big_int)
    }
}

impl From<i32> for BigInt {
    fn from(i: i32) -> BigInt {
        BigInt(i.into())
    }
}

impl From<u64> for BigInt {
    fn from(i: u64) -> BigInt {
        BigInt(i.into())
    }
}

impl From<u8> for BigInt {
    fn from(i: u8) -> BigInt {
        BigInt(i.into())
    }
}

impl From<i64> for BigInt {
    fn from(i: i64) -> BigInt {
        BigInt(i.into())
    }
}

impl From<U64> for BigInt {
    /// This implementation assumes that U64 represents an unsigned U64,
    /// and not a signed U64 (aka int64 in Solidity). Right now, this is
    /// all we need (for block numbers). If it ever becomes necessary to
    /// handle signed U64s, we should add the same
    /// `{to,from}_{signed,unsigned}_u64` methods that we have for U64.
    fn from(n: U64) -> BigInt {
        BigInt::from(n.as_u64())
    }
}

impl From<U128> for BigInt {
    /// This implementation assumes that U128 represents an unsigned U128,
    /// and not a signed U128 (aka int128 in Solidity). Right now, this is
    /// all we need (for block numbers). If it ever becomes necessary to
    /// handle signed U128s, we should add the same
    /// `{to,from}_{signed,unsigned}_u128` methods that we have for U256.
    fn from(n: U128) -> BigInt {
        let mut bytes: [u8; 16] = [0; 16];
        n.to_little_endian(&mut bytes);
        BigInt::from_unsigned_bytes_le(&bytes)
    }
}

impl FromStr for BigInt {
    type Err = <num_bigint::BigInt as FromStr>::Err;

    fn from_str(s: &str) -> Result<BigInt, Self::Err> {
        num_bigint::BigInt::from_str(s).map(BigInt)
    }
}

impl Serialize for BigInt {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.to_string().serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for BigInt {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        use serde::de::Error;

        let decimal_string = <String>::deserialize(deserializer)?;
        BigInt::from_str(&decimal_string).map_err(D::Error::custom)
    }
}

impl Add for BigInt {
    type Output = BigInt;

    fn add(self, other: BigInt) -> BigInt {
        BigInt(self.0.add(other.0))
    }
}

impl Sub for BigInt {
    type Output = BigInt;

    fn sub(self, other: BigInt) -> BigInt {
        BigInt(self.0.sub(other.0))
    }
}

impl Mul for BigInt {
    type Output = BigInt;

    fn mul(self, other: BigInt) -> BigInt {
        BigInt(self.0.mul(other.0))
    }
}

impl Div for BigInt {
    type Output = BigInt;

    fn div(self, other: BigInt) -> BigInt {
        if other == BigInt::from(0) {
            panic!("Cannot divide by zero-valued `BigInt`!")
        }

        BigInt(self.0.div(other.0))
    }
}

impl Rem for BigInt {
    type Output = BigInt;

    fn rem(self, other: BigInt) -> BigInt {
        BigInt(self.0.rem(other.0))
    }
}

impl BitOr for BigInt {
    type Output = Self;

    fn bitor(self, other: Self) -> Self {
        Self::from(self.0.bitor(other.0))
    }
}

impl BitAnd for BigInt {
    type Output = Self;

    fn bitand(self, other: Self) -> Self {
        Self::from(self.0.bitand(other.0))
    }
}

#[cfg(test)]
mod test {
    use web3::types::U256;

    use super::{BigDecimal, BigInt};

    #[test]
    fn f64_bigdecimal() {
        let test_cases = vec![
            (1.354, BigDecimal::try_from(1.354).unwrap(), "1.354"),
            (11.0, BigDecimal::try_from(11).unwrap(), "11"),
            (0.0, BigDecimal::try_from(0).unwrap(), "0"),
            (0.33, BigDecimal::try_from(0.33).unwrap(), "0.33"),
            (-0.33, BigDecimal::try_from(-0.33).unwrap(), "-0.33"),
        ];

        for (fnumber, fbigdecimal, snumber) in test_cases {
            assert_eq!(fnumber, fbigdecimal.to_f64().unwrap());
            assert_eq!(fbigdecimal.normalized().to_string().as_str(), snumber);
        }
    }

    #[test]
    #[warn(clippy::unnecessary_cast)]
    fn u256_bigdecimal() {
        let test_cases = vec![
            (
                U256::from(132397000000000000000_u128),
                BigDecimal::from_unsigned_u256(&U256::from(132397000000000000000_u128), 18),
                "132.397",
            ),
            (
                U256::from(9300002397000000000000000_u128),
                BigDecimal::from_unsigned_u256(&U256::from(9300002397000000000000000_u128), 18),
                "9300002.397",
            ),
        ];

        for (n, nbigdecimal, snumber) in test_cases {
            assert_eq!(n, nbigdecimal.to_unsigned_u256());
            assert_eq!(nbigdecimal.normalized().to_string().as_str(), snumber);
        }
    }

    #[test]
    fn f64_u256() {
        let test_cases = vec![
            (
                132.397,
                BigDecimal::try_from(132.397).unwrap(),
                U256::from(132397000000000000000_u128),
                18,
            ),
            (
                11.0,
                BigDecimal::try_from(11).unwrap(),
                U256::from(11000000000000000000_u128),
                18,
            ),
            (
                0.33,
                BigDecimal::try_from(0.33).unwrap(),
                U256::from(330000000000000000_u128),
                18,
            ),
        ];

        for (fnumber, fbigdecimal, nu256, decimal) in test_cases {
            let bigdecimal_from_u256 = BigDecimal::from_unsigned_u256(&nu256, decimal);
            assert_eq!(fbigdecimal.with_scale(decimal).to_unsigned_u256(), nu256);
            assert_eq!(bigdecimal_from_u256.to_f64().unwrap(), fnumber);
        }
    }

    #[test]
    fn normalize() {
        let vals = vec![
            (
                BigDecimal::new(BigInt::from(10), -2),
                BigDecimal(bigdecimal::BigDecimal::new(1.into(), 1)),
                "0.1",
            ),
            (
                BigDecimal::new(BigInt::from(132400), 4),
                BigDecimal(bigdecimal::BigDecimal::new(1324.into(), -6)),
                "1324000000",
            ),
            (
                BigDecimal::new(BigInt::from(1_900_000), -3),
                BigDecimal(bigdecimal::BigDecimal::new(19.into(), -2)),
                "1900",
            ),
            (BigDecimal::new(0.into(), 3), BigDecimal::zero(), "0"),
            (BigDecimal::new(0.into(), -5), BigDecimal::zero(), "0"),
        ];

        for (not_normalized, normalized, string) in vals {
            assert_eq!(not_normalized.normalized(), normalized);
            assert_eq!(not_normalized.normalized().to_string(), string);
            assert_eq!(normalized.to_string(), string);
        }
    }

    #[test]
    fn fmt_debug() {
        let bi = BigInt::from(-17);
        let bd = BigDecimal::new(bi.clone(), -2);
        assert_eq!("BigInt(-17)", format!("{:?}", bi));
        assert_eq!("BigDecimal(-0.17)", format!("{:?}", bd));
    }
}
