use alloy::{primitives::Address, signers::local::PrivateKeySigner};
use anyhow::Result;
use rengine_types::{
    MarketType, PrivateExchangePollingConfig, PublicExchangePollingConfig, Symbol,
};
use rengine_utils::duration_serde;
use serde::{de::Error as DeError, Deserialize, Serialize};
use std::time::Duration;

#[derive(Clone, Debug, Deserialize)]
pub struct HyperLiquidAccountExecutionConfig {
    #[serde(deserialize_with = "parse_signer")]
    pub signer: PrivateKeySigner,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct HyperLiquidPrivateConfig {
    #[serde(with = "duration_serde")]
    pub max_response_duration: Duration,
    pub account_address: Address,
    #[serde(skip_serializing)]
    pub trading_account: HyperLiquidAccountExecutionConfig,
    #[serde(default)]
    pub private_exchange_polling_config: PrivateExchangePollingConfig,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct MarketConfig {
    pub symbol: Symbol,
    pub market_type: MarketType,
}

#[derive(Clone, Debug, Deserialize, Serialize, Default)]
pub struct HyperLiquidPublicConfig {
    pub markets: Vec<MarketConfig>,
    #[serde(default)]
    pub public_exchange_polling_config: PublicExchangePollingConfig,
}

fn parse_signer<'de, D>(deserializer: D) -> Result<PrivateKeySigner, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let s: String = Deserialize::deserialize(deserializer)?;
    s.parse().map_err(DeError::custom)
}

#[cfg(test)]
mod test {
    use crate::hyperliquid::HyperLiquidPrivateConfig;

    #[test]
    fn deserialize_hyperliquid_config() {
        let toml_str = r#"
max_response_duration = "1s"

account_address = "4838B106FCe9647Bdf1E7877BF73cE8B0BAD5f97"

[trading_account]
signer = "0xabcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789"
"#;

        let str: HyperLiquidPrivateConfig =
            toml::from_str(toml_str).expect("should deserialize HyperLiquidConfig");

        println!("{str:?}");
    }
}
