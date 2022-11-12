use std::ops::Mul;
use web3::types::U256;

use super::scalar::BigDecimal;

//TODO: add test to all functions

pub fn to_percent(value: f64) -> f64 {
    value / 100.0
}

pub fn f64_to_u256(value: f64, decimals: u8) -> U256 {
    f64_to_bigdecimal(value, decimals).to_unsigned_u256()
}

pub fn f64_to_bigdecimal(value: f64, decimals: u8) -> BigDecimal {
    BigDecimal::try_from(value)
        .unwrap()
        .with_scale(decimals.into())
}

pub fn percent_to_bigdecimal(percent: f64, decimals: u8) -> BigDecimal {
    f64_to_bigdecimal(to_percent(percent), decimals)
}

pub fn percent_to_u256(percent: f64, decimals: u8) -> U256 {
    f64_to_u256(to_percent(percent), decimals)
}

pub fn get_slippage_amount(amount: U256, slippage: f64, decimals: u8) -> U256 {
    let amount_bd = BigDecimal::from_unsigned_u256(&amount, decimals.into());
    let slippage_bd = percent_to_bigdecimal(slippage, decimals);

    amount_bd
        .mul(slippage_bd)
        .with_scale(decimals.into())
        .to_unsigned_u256()
}

mod test {
    #[test]
    fn get_slippage_amount_test() {
        use crate::utils::{math::get_slippage_amount, scalar::BigDecimal};
        use web3::types::U256;

        struct TestCase {
            amount: U256,
            slippage: f64,
            decimals: u8,
            expected: U256,
        }

        let test_cases = vec![
            TestCase {
                amount: BigDecimal::from(12).with_scale(18).to_unsigned_u256(),
                slippage: 2.0,
                decimals: 18,
                expected: BigDecimal::try_from(12.0 * 0.02)
                    .unwrap()
                    .with_scale(18)
                    .to_unsigned_u256(),
            },
            TestCase {
                amount: BigDecimal::from(12).with_scale(6).to_unsigned_u256(),
                slippage: 1.5,
                decimals: 6,
                expected: U256::from(180000_u32),
            },
            TestCase {
                amount: U256::from(153987924_u32),
                slippage: 4.0,
                decimals: 6,
                expected: U256::from(6159516_u32),
            },
            TestCase {
                amount: BigDecimal::from(12).with_scale(18).to_unsigned_u256(),
                slippage: 0.5,
                decimals: 18,
                expected: BigDecimal::try_from(12.0 * 0.005)
                    .unwrap()
                    .with_scale(18)
                    .to_unsigned_u256(),
            },
            TestCase {
                amount: BigDecimal::try_from(13.33)
                    .unwrap()
                    .with_scale(18)
                    .to_unsigned_u256(),
                slippage: 0.3,
                decimals: 18,
                expected: BigDecimal::try_from(13.33 * 0.003)
                    .unwrap()
                    .with_scale(18)
                    .to_unsigned_u256(),
            },
            TestCase {
                amount: BigDecimal::try_from(13.33)
                    .unwrap()
                    .with_scale(6)
                    .to_unsigned_u256(),
                slippage: 0.3,
                decimals: 6,
                expected: BigDecimal::try_from(13.33 * 0.003)
                    .unwrap()
                    .with_scale(6)
                    .to_unsigned_u256(),
            },
        ];

        for test_case in test_cases {
            let result =
                get_slippage_amount(test_case.amount, test_case.slippage, test_case.decimals);
            assert_eq!(test_case.expected, result);
        }
    }
}
