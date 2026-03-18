use crate::{
    keys::{Instrument, Symbol, Venue},
    primitive::MarketType,
    state::StateUpdateKey,
};
use anyhow::{anyhow, Result};
use borsh::{BorshDeserialize, BorshSerialize};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
    time::Duration,
};

/// Market specification containing contract size, precision, and trading limits.
/// This information is fetched from exchange APIs and can be used by strategies
/// to properly size orders and format prices.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, BorshSerialize, BorshDeserialize)]
#[serde(rename_all = "camelCase")]
pub struct MarketSpec {
    /// The symbol this spec applies to
    pub symbol: Symbol,
    /// Number of decimal places for size/quantity
    pub size_decimals: u32,
    /// Minimum order size
    pub min_size: Decimal,
    /// Size increment (tick size for quantity)
    pub size_increment: Decimal,
    /// Number of decimal places for price
    pub price_decimals: u32,
    /// Minimum price
    pub min_price: Decimal,
    /// Price increment (tick size for price)
    pub price_increment: Decimal,
    /// Contract size (for perp markets, typically 1.0)
    pub contract_size: Decimal,
    /// Market type (spot or perp)
    pub market_type: MarketType,
    /// Maximum leverage (for perp markets)
    pub max_leverage: Option<u32>,
    /// Minimum notional value (price * size)
    pub min_notional: Option<Decimal>,
}

impl MarketSpec {
    /// Create a new `MarketSpec` with the given parameters
    pub fn new(
        symbol: Symbol,
        size_decimals: u32,
        price_decimals: u32,
        market_type: MarketType,
    ) -> Self {
        let size_increment = Decimal::new(1, size_decimals);
        let price_increment = Decimal::new(1, price_decimals);

        Self {
            symbol,
            size_decimals,
            min_size: size_increment,
            size_increment,
            price_decimals,
            min_price: price_increment,
            price_increment,
            contract_size: Decimal::ONE,
            market_type,
            max_leverage: None,
            min_notional: None,
        }
    }

    /// Round a size value to the correct precision for this market
    pub fn round_size(&self, size: Decimal) -> Decimal {
        size.round_dp(self.size_decimals)
    }

    /// Round a price value to the correct precision for this market
    pub fn round_price(&self, price: Decimal) -> Decimal {
        price.round_dp(self.price_decimals)
    }

    /// Round size down to the nearest valid increment
    pub fn floor_size(&self, size: Decimal) -> Decimal {
        (size / self.size_increment).floor() * self.size_increment
    }

    /// Round price down to the nearest valid increment
    pub fn floor_price(&self, price: Decimal) -> Decimal {
        (price / self.price_increment).floor() * self.price_increment
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstrumentDetails {
    pub base: Symbol,
    pub quote: Symbol,
    pub market_type: MarketType,
}

impl InstrumentDetails {
    pub fn key(&self) -> Instrument {
        format!("{}/{}-{}", self.base, self.quote, self.market_type).into()
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MappingInner {
    instrument_mapping: HashMap<Venue, HashMap<Instrument, InstrumentDetails>>,
    token_mapping: HashMap<Venue, HashMap<Symbol, Symbol>>,

    #[serde(skip)] // not in config files; derived at runtime
    reverse_instrument_mapping: HashMap<Venue, HashMap<Instrument, Instrument>>,
}

impl MappingInner {
    pub fn build_reverse_mapping(
        instrument_mapping: &HashMap<Venue, HashMap<Instrument, InstrumentDetails>>,
    ) -> HashMap<Venue, HashMap<Instrument, Instrument>> {
        let mut rev = HashMap::new();

        for (venue, instruments) in instrument_mapping {
            let mut inner = HashMap::new();
            for (instrument, details) in instruments {
                inner.insert(details.key(), instrument.clone());
            }
            rev.insert(venue.clone(), inner);
        }

        rev
    }
}

impl Default for MappingInner {
    fn default() -> Self {
        // create the details for ETH/USDC spot
        let details_spot = InstrumentDetails {
            base: Symbol::from("eth"),
            quote: Symbol::from("usdc"),
            market_type: MarketType::Spot,
        };
        let inst = details_spot.key();

        let mut inner_map = HashMap::new();
        inner_map.insert(inst, details_spot);

        let details_perp = InstrumentDetails {
            base: Symbol::from("eth"),
            quote: Symbol::from("usdc"),
            market_type: MarketType::Perp,
        };
        let inst = details_perp.key();
        inner_map.insert(inst, details_perp);

        let mut mapping = HashMap::new();
        mapping.insert("test".into(), inner_map);

        let mut token_mapping = HashMap::new();
        let mut inner_map_token = HashMap::new();
        token_mapping.insert("ETH".into(), "eth".into());

        inner_map_token.insert("test".into(), token_mapping);

        Self {
            token_mapping: inner_map_token,

            reverse_instrument_mapping: Self::build_reverse_mapping(&mapping),
            instrument_mapping: mapping,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Mapping {
    inner: Arc<MappingInner>,
}

impl Mapping {
    pub fn new(mut mapping: MappingInner) -> Self {
        mapping.reverse_instrument_mapping =
            MappingInner::build_reverse_mapping(&mapping.instrument_mapping);
        Self {
            inner: Arc::new(mapping),
        }
    }

    pub fn instruments(&self, venue: &Venue) -> Option<&HashMap<Instrument, InstrumentDetails>> {
        self.inner.instrument_mapping.get(venue)
    }

    pub fn map_instrument(
        &self,
        venue: &Venue,
        instrument: &Instrument,
    ) -> Result<InstrumentDetails> {
        self.inner
            .instrument_mapping
            .get(venue)
            .ok_or_else(|| anyhow!("missing venue {venue:?}"))?
            .get(instrument)
            .ok_or_else(|| anyhow!("missing instrument {instrument:?}"))
            .cloned()
    }

    pub fn map_symbol(&self, venue: &Venue, symbol: &Symbol) -> Result<&Symbol> {
        self.inner
            .token_mapping
            .get(venue)
            .ok_or_else(|| anyhow!("map_symbol: no mapping found for venue {venue}"))?
            .get(symbol)
            .ok_or_else(|| {
                anyhow!("map_symbol: no mapping found for symbol {symbol} in venue {venue}")
            })
    }

    pub fn reverse_map_instrument(&self, venue: &Venue, key: &Instrument) -> Result<Instrument> {
        self.inner
            .reverse_instrument_mapping
            .get(venue)
            .ok_or_else(|| anyhow!("missing venue {venue:?}"))?
            .get(key)
            .ok_or_else(|| anyhow!("missing reverse key {key:?}"))
            .cloned()
    }
}

#[derive(Default, Debug, Clone, Serialize, Deserialize)]
pub struct PrivateExchangePollingConfig {
    pub fetch_open_orders_interval: Option<Duration>,
    pub fetch_positions_interval: Option<Duration>,
    pub fetch_balances_interval: Option<Duration>,
    pub fetch_trades_interval: Option<Duration>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublicExchangePollingConfig {
    pub fetch_book_interval: Option<Duration>,
    pub fetch_funding_interval: Option<Duration>,
}

impl Default for PublicExchangePollingConfig {
    fn default() -> Self {
        Self {
            fetch_book_interval: Default::default(),
            fetch_funding_interval: Some(Duration::from_secs(60 * 5)),
        }
    }
}

#[derive(BorshSerialize, BorshDeserialize, Debug, Clone, Serialize, Deserialize)]
pub struct StrategyConfiguration {
    pub triggers_keys: HashSet<StateUpdateKey>,
    #[borsh(
        serialize_with = "crate::serialization::borsh_option_duration::serialize",
        deserialize_with = "crate::serialization::borsh_option_duration::deserialize"
    )]
    pub cooldown: Option<Duration>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deserialize_instrument_mapping_from_toml() {
        let toml_str = r#"
[instrument_mapping."hyperliquid"."eth"]
base = "eth"
quote = "usd"
marketType = "spot"

[token_mapping."hyperliquid"]
"ETH" = "eth"
"#;

        let mapping: MappingInner =
            toml::from_str(toml_str).expect("should deserialize InstrumentMapping from TOML");
        let mapping = Mapping::new(mapping);

        let details = mapping
            .map_instrument(&"hyperliquid".into(), &"eth".into())
            .expect("mapping should contain our key");

        assert_eq!(details.base.as_str(), "eth");
        assert_eq!(details.quote.as_str(), "usd");
        assert_eq!(details.market_type, MarketType::Spot);
    }
}
