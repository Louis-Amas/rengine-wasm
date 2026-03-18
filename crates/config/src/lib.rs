use alloy::primitives::Address;
use anyhow::Result;
use binance_perp::{BinancePerpConfig, BinancePerpPublicConfig};
use binance_spot::{BinanceSpotConfig, BinanceSpotPublicConfig};
use hyperliquid::hyperliquid::{HyperLiquidPrivateConfig, HyperLiquidPublicConfig};
use rengine_types::{Account, EvmAccount, MappingInner, Venue};
use rengine_utils::duration_serde;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, fs, path::Path, time::Duration};

#[derive(Deserialize, Debug, Serialize)]
pub struct ClickHouseConfig {
    pub url: String,
    pub db_name: String,
    pub user: String,
    pub password: String,
    #[serde(with = "duration_serde")]
    pub flush_interval: Duration,
    #[serde(with = "duration_serde")]
    pub metrics_interval: Duration,
    #[serde(default)]
    pub save_public_trade: bool,
}

#[derive(Deserialize, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DbConfig {
    pub duck_db_path: String,
    pub clickhouse: Option<ClickHouseConfig>,
}

impl Default for DbConfig {
    fn default() -> Self {
        Self {
            duck_db_path: ":memory:".to_string(),
            clickhouse: Default::default(),
        }
    }
}

#[derive(Deserialize, Debug, Serialize, Clone)]
pub struct EvmExecutionConfig {
    pub private_key: String,
}

#[derive(Deserialize, Debug, Serialize)]
#[serde(tag = "type")]
#[serde(rename_all = "snake_case")]
#[allow(clippy::large_enum_variant)]
pub enum ExchangeConfig {
    Hyperliquid(HyperLiquidPrivateConfig),
    BinancePerp(BinancePerpConfig),
    BinanceSpot(BinanceSpotConfig),
}

#[derive(Deserialize, Debug, Serialize)]
#[serde(tag = "type")]
#[serde(rename_all = "snake_case")]
pub enum ReaderExchangeConfig {
    Hyperliquid(HyperLiquidPublicConfig),
    BinanceSpot(BinanceSpotPublicConfig),
    BinancePerp(BinancePerpPublicConfig),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct EvmReaderConfig {
    pub ws_url: String,
    pub http_url: String,
    #[serde(with = "duration_serde")]
    pub idle_timeout: Duration,
    pub multicall_address: Address,
    pub chain_id: u64,
    #[serde(default = "default_tx_timeout", with = "duration_serde")]
    pub tx_timeout: Duration,
    #[serde(default = "default_tx_poll_interval", with = "duration_serde")]
    pub tx_poll_interval: Duration,
}

#[derive(Default, Debug, Deserialize, Serialize)]
pub struct Config {
    #[serde(default)]
    pub db: DbConfig,
    #[serde(default)]
    pub exchanges: HashMap<Account, ExchangeConfig>,
    #[serde(default)]
    pub evm_executors: HashMap<EvmAccount, EvmExecutionConfig>,
    #[serde(default)]
    pub readers: HashMap<Venue, ReaderExchangeConfig>,
    #[serde(default)]
    pub evm_readers: HashMap<Venue, EvmReaderConfig>,
    #[serde(flatten)]
    pub mappings: MappingInner,
    #[serde(default = "default_http_api_port")]
    pub http_api_port: u16,
}

const fn default_http_api_port() -> u16 {
    3000
}

const fn default_tx_timeout() -> Duration {
    Duration::from_secs(60)
}

const fn default_tx_poll_interval() -> Duration {
    Duration::from_secs(1)
}

impl Config {
    pub fn config_from_file<P: AsRef<Path>>(path: P) -> Result<Self> {
        let toml_str = fs::read_to_string(path)?;
        let config = toml::from_str::<Self>(&toml_str)?;
        Ok(config)
    }
}

#[cfg(test)]
mod test {
    use super::{Config, ExchangeConfig};
    use crate::ReaderExchangeConfig;
    use alloy::primitives::address;
    use hyperliquid::hyperliquid::{HyperLiquidPrivateConfig, MarketConfig};
    use rengine_types::{Account, MarketType};
    use std::time::Duration;

    #[test]
    fn test_read_config_from_toml() {
        let mut config = Config::default();
        let account = Account {
            venue: "hyperliquid".into(),
            account_id: "account".into(),
            market_type: MarketType::Spot,
        };

        let reader =
            ReaderExchangeConfig::Hyperliquid(hyperliquid::hyperliquid::HyperLiquidPublicConfig {
                markets: vec![
                    MarketConfig {
                        symbol: "eth".into(),
                        market_type: MarketType::Perp,
                    },
                    MarketConfig {
                        symbol: "usdc".into(),
                        market_type: MarketType::Perp,
                    },
                ],
                public_exchange_polling_config: Default::default(),
            });

        let writer = ExchangeConfig::Hyperliquid(HyperLiquidPrivateConfig {
            max_response_duration: Duration::from_secs(1),
            account_address: address!("0x0000000000000000000000000000000000000000"),
            trading_account: hyperliquid::hyperliquid::HyperLiquidAccountExecutionConfig {
                signer: "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80"
                    .parse()
                    .unwrap(),
            },
            private_exchange_polling_config: Default::default(),
        });

        config.readers.insert(account.venue.clone(), reader);
        config.exchanges.insert(account, writer);

        let result = toml::to_string(&config).unwrap();

        println!("{result}");

        let toml = r#"
[db]
duckDbPath = ":memory:"

[exchanges."hyperliquid|spot|account"]
type = "hyperliquid"
max_response_duration = "1s"
account_address = "0000000000000000000000000000000000000000"

[exchanges."hyperliquid|spot|account".trading_account]
signer = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80"

[readers.hyperliquid]
type = "hyperliquid"

[[readers.hyperliquid.markets]]
symbol = "eth"
market_type = "perp"

[[readers.hyperliquid.markets]]
symbol = "usdc"
market_type = "perp"

[instrument_mapping."hyperliquid"."ETH"]
base = "eth"
quote = "usd"
marketType = "perp"

[token_mapping."hyperliquid"]
"ETH" = "eth"
        "#;

        let config: Config = toml::from_str(toml).expect("Failed to parse TOML");

        let account = Account {
            venue: "hyperliquid".into(),
            account_id: "account".into(),
            market_type: MarketType::Spot,
        };

        let exchange = config
            .exchanges
            .get(&account)
            .expect("Missing hyperliquid config");

        match &exchange {
            ExchangeConfig::Hyperliquid(h) => {
                assert_eq!(
                    h.trading_account.signer,
                    "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80"
                        .parse()
                        .unwrap()
                );
            }
            _ => panic!("Unexpected config type"),
        }
    }
}
