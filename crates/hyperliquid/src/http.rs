use crate::types::{AssetPosition, HyperLiquidSide, Trade};
use alloy::primitives::Address;
use anyhow::{anyhow, Result};
use chrono::{serde::ts_milliseconds, DateTime, Utc};
use rengine_types::{ClientOrderId, MarketSpec, MarketType, SharedStr, Symbol};
use rengine_utils::http::RequestExt;
use reqwest::{Client, Method};
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use serde::{
    de::{self, SeqAccess, Visitor},
    Deserialize, Deserializer, Serialize,
};
use std::fmt;

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OpenOrdersResponse {
    pub coin: SharedStr,
    pub limit_px: Decimal,
    pub oid: u64,
    pub side: HyperLiquidSide,
    pub sz: Decimal,
    pub cloid: Option<ClientOrderId>,
    // pub timestamp: u64,
}

// #[derive(Deserialize, Debug)]
// #[serde(rename_all = "camelCase")]
// pub(crate) struct MarginSummary {
//     pub account_value: Decimal,
//     pub total_ntl_pos: Decimal,
//     pub total_raw_usd: Decimal,
//     pub total_margin_used: Decimal,
// }

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UserStateResponse {
    pub asset_positions: Vec<AssetPosition>,
    // pub margin_summary: MarginSummary,
    pub withdrawable: Decimal,
}

#[derive(Deserialize, Debug)]
pub(crate) struct Level {
    // pub n: u64,
    pub px: Decimal,
    pub sz: Decimal,
}

#[derive(Debug)]
pub(crate) struct L2Book {
    pub(crate) bids: Vec<Level>,
    pub(crate) asks: Vec<Level>,
}

impl<'de> Deserialize<'de> for L2Book {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct L2BookVisitor;

        impl<'de> Visitor<'de> for L2BookVisitor {
            type Value = L2Book;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a two-element array with bids and asks")
            }

            fn visit_seq<V>(self, mut seq: V) -> Result<L2Book, V::Error>
            where
                V: SeqAccess<'de>,
            {
                let bids: Vec<Level> = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(0, &self))?;
                let asks: Vec<Level> = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                Ok(L2Book { bids, asks })
            }
        }

        deserializer.deserialize_seq(L2BookVisitor)
    }
}

#[derive(Deserialize, Debug)]
pub(crate) struct L2SnapshotResponse {
    // pub(crate) coin: String,
    pub(crate) levels: L2Book,
    // pub(crate) time: u64,
}

#[derive(Deserialize, Debug)]
pub(crate) struct Balance {
    pub(crate) coin: Symbol,
    pub(crate) hold: Decimal,
    pub(crate) total: Decimal,
}

#[derive(Deserialize, Debug)]
pub(crate) struct BalancesResponse {
    pub(crate) balances: Vec<Balance>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FundingRateEntry {
    pub(crate) coin: Symbol,
    pub(crate) funding_rate: Decimal,
    pub(crate) premium: Decimal,
    #[serde(with = "ts_milliseconds")]
    pub(crate) time: DateTime<Utc>,
}

/// Universe entry for perp markets from Meta endpoint
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PerpUniverseEntry {
    pub name: Symbol,
    pub sz_decimals: u32,
    #[serde(default)]
    pub max_leverage: Option<u32>,
    #[serde(default)]
    pub only_isolated: Option<bool>,
}

/// Asset context for perp markets from Meta endpoint
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub(crate) struct PerpAssetCtx {
    pub day_ntl_vlm: Decimal,
    pub funding: Decimal,
    pub impact_pxs: Option<Vec<Decimal>>,
    pub mark_px: Decimal,
    pub mid_px: Option<Decimal>,
    pub open_interest: Decimal,
    pub oracle_px: Decimal,
    pub premium: Option<Decimal>,
    pub prev_day_px: Decimal,
}

/// Response from Meta endpoint for perp markets
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct PerpMetaResponse {
    pub universe: Vec<PerpUniverseEntry>,
}

/// Combined perp meta and asset contexts
#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(dead_code)]
pub(crate) struct PerpMetaAndAssetCtxs(pub PerpMetaResponse, pub Vec<PerpAssetCtx>);

/// EVM contract details for a spot token
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct EvmContract {
    pub address: String,
    pub evm_extra_wei_decimals: i32,
}

/// Token entry in spot universe
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SpotToken {
    pub name: Symbol,
    pub sz_decimals: u32,
    pub wei_decimals: u32,
    pub index: u32,
    pub token_id: String,
    pub is_canonical: bool,
    #[serde(default)]
    pub evm_contract: Option<EvmContract>,
    #[serde(default)]
    pub full_name: Option<String>,
}

/// Universe entry for spot markets from `SpotMeta` endpoint
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SpotUniverseEntry {
    pub name: Symbol,
    pub tokens: Vec<u32>,
    pub index: u32,
    pub is_canonical: bool,
}

/// Response from `SpotMeta` endpoint
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct SpotMetaResponse {
    pub tokens: Vec<SpotToken>,
    pub universe: Vec<SpotUniverseEntry>,
}

/// Asset context for spot markets from `SpotMetaAndAssetCtxs` endpoint
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SpotAssetCtx {
    pub day_ntl_vlm: Decimal,
    pub mark_px: Decimal,
    pub mid_px: Option<Decimal>,

    pub prev_day_px: Decimal,
    pub circulating_supply: Decimal,
}

/// Combined spot meta and asset contexts
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct SpotMetaAndAssetCtxsResponse(pub SpotMetaResponse, pub Vec<SpotAssetCtx>);

/// Create a `MarketSpec` from a Hyperliquid perp universe entry
fn market_spec_from_perp(entry: &PerpUniverseEntry) -> MarketSpec {
    let size_decimals = entry.sz_decimals;
    let size_increment = Decimal::new(1, size_decimals);
    // Hyperliquid perp prices use 6 - sz_decimals decimal places for precision
    let price_decimals = 6u32.saturating_sub(size_decimals);
    let price_increment = Decimal::new(1, price_decimals);

    MarketSpec {
        symbol: entry.name.clone(),
        size_decimals,
        min_size: size_increment,
        size_increment,
        price_decimals,
        min_price: price_increment,
        price_increment,
        contract_size: Decimal::ONE,
        market_type: MarketType::Perp,
        max_leverage: entry.max_leverage,
        min_notional: Some(dec!(10)),
    }
}

/// Create a `MarketSpec` from a Hyperliquid spot universe entry and token
fn market_spec_from_spot(market: &SpotUniverseEntry, token: &SpotToken) -> MarketSpec {
    let size_decimals = token.sz_decimals;
    let size_increment = Decimal::new(1, size_decimals);
    // Hyperliquid spot prices use 8 - sz_decimals decimal places for precision
    let price_decimals = 8u32.saturating_sub(size_decimals);
    let price_increment = Decimal::new(1, price_decimals);

    MarketSpec {
        symbol: market.name.clone(),
        size_decimals,
        min_size: size_increment,
        size_increment,
        price_decimals,
        min_price: price_increment,
        price_increment,
        contract_size: Decimal::ONE,
        market_type: MarketType::Spot,
        max_leverage: None,
        min_notional: Some(dec!(10)),
    }
}

use strum::AsRefStr;

#[derive(Deserialize, Serialize, Debug, Clone, AsRefStr)]
#[serde(tag = "type")]
#[serde(rename_all = "camelCase")]
#[strum(serialize_all = "snake_case")]
pub(crate) enum InfoRequest {
    #[serde(rename = "clearinghouseState")]
    UserState {
        user: Address,
    },
    #[serde(rename = "batchClearinghouseStates")]
    UserStates {
        users: Vec<Address>,
    },
    #[serde(rename = "spotClearinghouseState")]
    UserTokenBalances {
        user: Address,
    },
    UserFees {
        user: Address,
    },
    OpenOrders {
        user: Address,
    },
    OrderStatus {
        user: Address,
        oid: u64,
    },
    Meta,
    SpotMeta,
    SpotMetaAndAssetCtxs,
    AllMids,
    UserFills {
        user: Address,
    },
    #[serde(rename_all = "camelCase")]
    FundingHistory {
        coin: Symbol,
        start_time: u64,
        end_time: Option<u64>,
    },
    #[serde(rename_all = "camelCase")]
    UserFunding {
        user: Address,
        start_time: u64,
        end_time: Option<u64>,
    },
    L2Book {
        coin: String,
    },
    RecentTrades {
        coin: String,
    },
}

impl InfoRequest {
    fn label(&self) -> String {
        format!("hyperliquid_{}", self.as_ref())
    }
}

const URL: &str = "https://api.hyperliquid.xyz";

#[derive(Default)]
pub(crate) struct HttpClient {
    client: Client,
}

impl HttpClient {
    async fn send_info_request<T: for<'a> Deserialize<'a>>(
        &self,
        info_request: InfoRequest,
    ) -> Result<T> {
        let url = format!("{URL}/info");
        let label = info_request.label();
        self.client
            .request(Method::POST, url)
            .json(&info_request)
            .send_ok(&label)
            .await?
            .json::<T>()
            .await
            .map_err(Into::into)
    }

    pub(crate) async fn open_orders(&self, address: Address) -> Result<Vec<OpenOrdersResponse>> {
        let input = InfoRequest::OpenOrders { user: address };
        self.send_info_request(input).await
    }

    pub(crate) async fn user_state(&self, address: Address) -> Result<UserStateResponse> {
        let input = InfoRequest::UserState { user: address };
        self.send_info_request(input).await
    }

    pub(crate) async fn l2_snapshot(&self, coin: &Symbol) -> Result<L2SnapshotResponse> {
        let input = InfoRequest::L2Book {
            coin: coin.to_string(),
        };
        self.send_info_request(input).await
    }

    pub(crate) async fn user_token_balances(&self, address: Address) -> Result<BalancesResponse> {
        let input = InfoRequest::UserTokenBalances { user: address };

        self.send_info_request(input).await
    }

    pub(crate) async fn trades(&self, address: Address) -> Result<Vec<Trade>> {
        let input = InfoRequest::UserFills { user: address };

        let trades = self.send_info_request::<Vec<Trade>>(input).await?;

        Ok(trades)
    }

    pub(crate) async fn funding_rate(&self, symbol: &Symbol) -> Result<FundingRateEntry> {
        let start_time = (Utc::now() - chrono::Duration::hours(1) - chrono::Duration::minutes(10))
            .timestamp_millis();

        let input = InfoRequest::FundingHistory {
            coin: symbol.clone(),
            start_time: start_time as u64,
            end_time: None,
        };

        let fundings = self
            .send_info_request::<Vec<FundingRateEntry>>(input)
            .await?;

        fundings
            .into_iter()
            .next_back()
            .ok_or_else(|| anyhow!("couldn't get funding data for {}", symbol))
    }

    /// Fetch perpetual market metadata including contract size decimals
    pub(crate) async fn perp_meta(&self) -> Result<PerpMetaResponse> {
        let input = InfoRequest::Meta;
        self.send_info_request(input).await
    }

    /// Fetch spot market metadata including token decimals
    pub(crate) async fn spot_meta(&self) -> Result<SpotMetaResponse> {
        let input = InfoRequest::SpotMeta;
        self.send_info_request(input).await
    }

    /// Fetch spot market metadata with asset contexts (includes mark price, etc.)
    #[allow(dead_code)]
    pub(crate) async fn spot_meta_and_asset_ctxs(&self) -> Result<SpotMetaAndAssetCtxsResponse> {
        let input = InfoRequest::SpotMetaAndAssetCtxs;
        self.send_info_request(input).await
    }

    /// Get market specifications for a perp market
    #[allow(dead_code)]
    pub(crate) async fn perp_market_spec(&self, symbol: &Symbol) -> Result<MarketSpec> {
        let meta = self.perp_meta().await?;

        let entry = meta
            .universe
            .iter()
            .find(|e| &e.name == symbol)
            .ok_or_else(|| anyhow!("couldn't find perp market spec for {}", symbol))?;

        Ok(market_spec_from_perp(entry))
    }

    /// Get market specifications for a spot market
    #[allow(dead_code)]
    pub(crate) async fn spot_market_spec(&self, symbol: &Symbol) -> Result<MarketSpec> {
        let meta = self.spot_meta().await?;

        let market = meta
            .universe
            .iter()
            .find(|e| &e.name == symbol)
            .ok_or_else(|| anyhow!("couldn't find spot market spec for {}", symbol))?;

        // Get the base token (first token in the pair)
        let base_token_index = *market
            .tokens
            .first()
            .ok_or_else(|| anyhow!("spot market {} has no tokens", symbol))?;

        let token = meta
            .tokens
            .iter()
            .find(|t| t.index == base_token_index)
            .ok_or_else(|| anyhow!("couldn't find token for spot market {}", symbol))?;

        Ok(market_spec_from_spot(market, token))
    }

    /// Get all perp market specifications
    pub(crate) async fn all_perp_market_specs(&self) -> Result<Vec<(Symbol, MarketSpec)>> {
        let meta = self.perp_meta().await?;

        Ok(meta
            .universe
            .iter()
            .map(|entry| (entry.name.clone(), market_spec_from_perp(entry)))
            .collect())
    }

    /// Get all spot market specifications
    pub(crate) async fn all_spot_market_specs(&self) -> Result<Vec<(Symbol, MarketSpec)>> {
        let meta = self.spot_meta().await?;

        Ok(meta
            .universe
            .iter()
            .filter_map(|market| {
                if market.tokens.len() < 2 {
                    return None;
                }
                let base_token_index = market.tokens[0];
                let base_token = meta.tokens.iter().find(|t| t.index == base_token_index)?;

                Some((
                    market.name.clone(),
                    market_spec_from_spot(market, base_token),
                ))
            })
            .collect())
    }
}

#[cfg(test)]
mod test {
    use super::{
        market_spec_from_perp, market_spec_from_spot, HttpClient, L2Book, PerpUniverseEntry,
        SpotToken, SpotUniverseEntry,
    };
    use alloy::primitives::address;

    #[test]
    fn test_deseriliaze_l2_book() {
        let l2json = r#"
[
  [
    {
      "px": "19900",
      "sz": "1",
      "n": 1
    },
    {
      "px": "19800",
      "sz": "2",
      "n": 2
    },
    {
      "px": "19700",
      "sz": "3",
      "n": 3
    }
  ],
  [
    {
      "px": "20100",
      "sz": "1",
      "n": 1
    },
    {
      "px": "20200",
      "sz": "2",
      "n": 2
    },
    {
      "px": "20300",
      "sz": "3",
      "n": 3
    }
  ]
]"#;

        let book = serde_json::from_str::<L2Book>(l2json).unwrap();

        println!("{book:?}");
    }

    #[tokio::test]
    #[ignore]
    async fn test_http_client_open_order() {
        let client = HttpClient::default();

        let addr = address!("0xa6F1eF0733FC462627f280053040f5f47D7fA0c6");
        let result = client.open_orders(addr).await.unwrap();

        println!("{result:?}");
    }

    #[tokio::test]
    #[ignore]
    async fn test_http_client_user_state() {
        let client = HttpClient::default();

        let addr = address!("0xa6F1eF0733FC462627f280053040f5f47D7fA0c6");
        let result = client.user_state(addr).await.unwrap();

        println!("{result:?}");
    }

    #[tokio::test]
    #[ignore]
    async fn test_http_client_l2_snapshot() {
        let client = HttpClient::default();

        let coin = "ETH".into();
        let result = client.l2_snapshot(&coin).await.unwrap();

        println!("{result:?}");
    }

    #[tokio::test]
    #[ignore]
    async fn test_http_client_user_balances() {
        let client = HttpClient::default();

        let addr = address!("0xa6F1eF0733FC462627f280053040f5f47D7fA0c6");
        let result = client.user_token_balances(addr).await.unwrap();

        println!("{result:?}");
    }

    #[tokio::test]
    #[ignore]
    async fn test_http_client_user_trades() {
        let client = HttpClient::default();

        let addr = address!("0xa6F1eF0733FC462627f280053040f5f47D7fA0c6");
        let result = client.trades(addr).await.unwrap();

        println!("{result:?}");
    }

    #[tokio::test]
    #[ignore]
    async fn test_http_client_funding_rate() {
        let client = HttpClient::default();

        let coin = "ETH".into();
        let result = client.funding_rate(&coin).await.unwrap();

        println!("{result:?}");
    }

    #[tokio::test]
    #[ignore]
    async fn test_http_client_perp_meta() {
        let client = HttpClient::default();
        let result = client.perp_meta().await.unwrap();

        println!("Perp markets: {} total", result.universe.len());
        for market in result.universe.iter().take(5) {
            println!(
                "  {} - sz_decimals: {}, max_leverage: {:?}",
                market.name, market.sz_decimals, market.max_leverage
            );
        }
    }

    #[tokio::test]
    #[ignore]
    async fn test_http_client_spot_meta() {
        let client = HttpClient::default();
        let result = client.spot_meta().await.unwrap();

        println!("Spot markets: {} total", result.universe.len());
        println!("Tokens: {} total", result.tokens.len());
        for market in result.universe.iter().take(5) {
            println!("  {} - tokens: {:?}", market.name, market.tokens);
        }
        for token in result.tokens.iter().take(5) {
            println!(
                "  Token {} - sz_decimals: {}, wei_decimals: {}",
                token.name, token.sz_decimals, token.wei_decimals
            );
        }
    }

    #[tokio::test]
    #[ignore]
    async fn test_http_client_perp_market_spec() {
        let client = HttpClient::default();
        let spec = client.perp_market_spec(&"BTC".into()).await.unwrap();

        println!("BTC perp spec: {:?}", spec);
        assert_eq!(spec.symbol.as_str(), "BTC");
        assert_eq!(spec.market_type, rengine_types::MarketType::Perp);
    }

    #[test]
    fn test_market_spec_from_perp() {
        use rengine_types::MarketType;
        use rust_decimal_macros::dec;

        let entry = PerpUniverseEntry {
            name: "ETH".into(),
            sz_decimals: 3,
            max_leverage: Some(50),
            only_isolated: None,
        };

        let spec = market_spec_from_perp(&entry);

        assert_eq!(spec.symbol.as_str(), "ETH");
        assert_eq!(spec.size_decimals, 3);
        assert_eq!(spec.min_size, dec!(0.001));
        assert_eq!(spec.size_increment, dec!(0.001));
        assert_eq!(spec.price_decimals, 3);
        assert_eq!(spec.market_type, MarketType::Perp);
        assert_eq!(spec.max_leverage, Some(50));
    }

    #[test]
    fn test_market_spec_from_spot() {
        use rengine_types::MarketType;
        use rust_decimal_macros::dec;

        let market = SpotUniverseEntry {
            name: "ETH/USDC".into(),
            tokens: vec![1, 2],
            index: 0,
            is_canonical: true,
        };

        let token = SpotToken {
            name: "ETH".into(),
            sz_decimals: 4,
            wei_decimals: 18,
            index: 1,
            token_id: "0x1".into(),
            is_canonical: true,
            evm_contract: None,
            full_name: None,
        };

        let spec = market_spec_from_spot(&market, &token);

        assert_eq!(spec.symbol.as_str(), "ETH/USDC");
        assert_eq!(spec.size_decimals, 4);
        assert_eq!(spec.min_size, dec!(0.0001));
        assert_eq!(spec.size_increment, dec!(0.0001));
        assert_eq!(spec.price_decimals, 4);
        assert_eq!(spec.market_type, MarketType::Spot);
        assert_eq!(spec.max_leverage, None);
    }
}
