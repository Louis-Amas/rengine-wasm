use alloy::primitives::U256;
#[cfg(feature = "providers")]
pub use alloy::providers::bindings::IMulticall3::{Call3, Result};
use rust_decimal::Decimal;
use std::str::FromStr;

pub fn u256_to_decimal(value: U256) -> Decimal {
    u256_to_decimal_with_scale(value, 18)
}

pub fn u256_to_decimal_with_scale(value: U256, scale: u32) -> Decimal {
    if let Ok(val) = i128::try_from(value) {
        return Decimal::from_i128_with_scale(val, scale);
    }

    let s = value.to_string();
    let scale_usize = scale as usize;
    if s.len() <= scale_usize {
        let zeros = "0".repeat(scale_usize - s.len());
        Decimal::from_str(&format!("0.{}{}", zeros, s)).unwrap_or_default()
    } else {
        let split_idx = s.len() - scale_usize;
        let (int_part, frac_part) = s.split_at(split_idx);
        Decimal::from_str(&format!("{}.{}", int_part, frac_part)).unwrap_or_default()
    }
}

#[cfg(feature = "providers")]
pub mod erc20;

use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Debug, Clone, Serialize, Deserialize, BorshSerialize, BorshDeserialize)]
pub struct LogSubscription {
    pub address: [u8; 20],
    pub topics: Vec<[u8; 32]>,
    #[borsh(
        serialize_with = "rengine_types::borsh_option_duration::serialize",
        deserialize_with = "rengine_types::borsh_option_duration::deserialize"
    )]
    pub timeout: Option<Duration>,
}

use alloy::sol;

sol! {
    #[derive(Debug, PartialEq, Eq)]
    struct EvmTxRequest {
        address to;
        uint256 value;
        bytes data;
    }
}

#[cfg(test)]
mod test {
    #[cfg(feature = "providers")]
    use alloy::{
        providers::bindings::IMulticall3::Call3,
        sol_types::{sol_data::Array, SolValue},
    };
    use rust_decimal::Decimal;

    #[test]
    #[cfg(feature = "providers")]
    fn serialize_alloy_struct_to_abi() {
        let call = Call3 {
            target: <_>::default(),
            callData: <_>::default(),
            allowFailure: true,
        };

        let abi = call.abi_encode();
        let decoded = Call3::abi_decode(&abi).unwrap();

        assert!(decoded.allowFailure);
    }

    #[test]
    #[cfg(feature = "providers")]
    fn serialize_vec_of_alloy_struct_to_abi() {
        use alloy::sol_types::SolType;

        let calls = vec![
            Call3 {
                target: <_>::default(),
                callData: <_>::default(),
                allowFailure: true,
            },
            Call3 {
                target: [1u8; 20].into(),
                callData: vec![0xde, 0xad, 0xbe, 0xef].into(),
                allowFailure: false,
            },
        ];

        let abi = <Array<Call3> as SolType>::abi_encode(&calls);

        let decoded: Vec<Call3> = <Array<Call3> as SolType>::abi_decode(&abi).unwrap();

        assert_eq!(decoded.len(), calls.len());
        for (original, decoded_call) in calls.iter().zip(decoded.iter()) {
            assert_eq!(original.target, decoded_call.target);
            assert_eq!(original.callData, decoded_call.callData);
            assert_eq!(original.allowFailure, decoded_call.allowFailure);
        }
    }

    #[test]
    fn test_u256_to_decimal_with_scale() {
        use crate::u256_to_decimal_with_scale;
        use alloy::primitives::U256;
        use std::str::FromStr;

        let val = U256::from(1500000000000000000u64); // 1.5 * 10^18
        let decimal = u256_to_decimal_with_scale(val, 18);
        assert_eq!(decimal, Decimal::from_str("1.5").unwrap());

        // Test case 2: Standard 18 decimals, value < 1
        let val = U256::from(500000000000000000u64); // 0.5 * 10^18
        let decimal = u256_to_decimal_with_scale(val, 18);
        assert_eq!(decimal, Decimal::from_str("0.5").unwrap());

        // Test case 3: 6 decimals (USDC like), value > 1
        let val = U256::from(1500000u64); // 1.5 * 10^6
        let decimal = u256_to_decimal_with_scale(val, 6);
        assert_eq!(decimal, Decimal::from_str("1.5").unwrap());

        // Test case 4: 6 decimals, small value
        let val = U256::from(1u64); // 0.000001
        let decimal = u256_to_decimal_with_scale(val, 6);
        assert_eq!(decimal, Decimal::from_str("0.000001").unwrap());

        // Test case 5: 0 value
        let val = U256::ZERO;
        let decimal = u256_to_decimal_with_scale(val, 18);
        assert_eq!(decimal, Decimal::ZERO);

        // Test case 6: Large value (fits in i128)
        let val = U256::from(1234567890123456789u64);
        let decimal = u256_to_decimal_with_scale(val, 18);
        assert_eq!(decimal, Decimal::from_str("1.234567890123456789").unwrap());

        // Test case 7: Very large value (exceeds i128, fallback to string parsing)
        // 2^128 is approx 3.4e38. Let's use something larger.
        // 10^40
        let val = U256::from(10u64).pow(U256::from(40));
        // With scale 18, this should be 10^22
        let decimal = u256_to_decimal_with_scale(val, 18);
        assert_eq!(
            decimal,
            Decimal::from_str("10000000000000000000000").unwrap()
        );
    }
}
