use std::ops::{Div, Mul};

use web3::types::U256;

use super::scalar::{BigDecimal, BigInt};

//TODO: add test to all functions

pub fn transform_percent(value: f64) -> f64 {
    value / 100.0
}

pub fn exp10(decimals: u8) -> BigDecimal {
    BigDecimal::new(BigInt::from(10).pow(decimals), decimals.into())
}

pub fn transform_slippage_u256(slippage: f64, decimals: u8) -> U256 {
    transform_slippage_bigdecimal(slippage, decimals).to_unsigned_u256()
}

pub fn transform_slippage_bigdecimal(slippage: f64, decimals: u8) -> BigDecimal {
    BigDecimal::try_from(transform_percent(slippage))
        .unwrap()
        .with_scale(decimals.into())
}

pub fn get_slippage_amount(amount: U256, slippage: f64, decimals: u8) -> U256 {
    let slippage_bd = transform_slippage_bigdecimal(slippage, decimals);

    let amount_bd = BigDecimal::from_unsigned_u256(&amount, decimals.into());

    let powed_decimals = exp10(decimals);

    amount_bd
        .mul(slippage_bd)
        .div(powed_decimals)
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
        ];

        for test_case in test_cases {
            let result =
                get_slippage_amount(test_case.amount, test_case.slippage, test_case.decimals);
            assert_eq!(test_case.expected, result);
        }
    }
}
