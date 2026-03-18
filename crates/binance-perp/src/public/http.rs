use crate::public::types::{
    BinanceDepth, BinanceFundingRate, BinanceInstrumentsPayload, ContractStatus, ContractType,
    FundingInfo, SymbolFilter,
};
use anyhow::Result;
use async_trait::async_trait;
use rengine_interfaces::PublicExchangeReader;
use rengine_non_wasm_types::{send_changes, ChangesTx};
use rengine_types::{Action, Mapping, MarketSpec, MarketType, Symbol, Venue, VenueBookKey};
use rengine_utils::http::RequestExt;
use reqwest::Client;
use rust_decimal::Decimal;
use std::collections::HashMap;
use tracing::error;

pub const BINANCE_FUTURE_DEFAULT_HTTP: &str = "https://fapi.binance.com";

#[derive(Clone)]
pub struct HttpClient {
    client: Client,
    base_url: String,
    funding_info: HashMap<String, FundingInfo>,
}

impl Default for HttpClient {
    fn default() -> Self {
        Self {
            client: Client::new(),
            base_url: BINANCE_FUTURE_DEFAULT_HTTP.to_string(),
            funding_info: HashMap::new(),
        }
    }
}

impl HttpClient {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            client: Client::new(),
            base_url: base_url.into(),
            funding_info: HashMap::new(),
        }
    }

    pub async fn fetch_funding_info(&mut self) -> Result<()> {
        let url = format!("{}/fapi/v1/fundingInfo", self.base_url);
        let info: Vec<FundingInfo> = self
            .client
            .get(url)
            .query(&[("limit", "1000")])
            .send_ok("binance_perp_funding_info")
            .await?
            .json()
            .await?;

        self.funding_info = info
            .into_iter()
            .map(|info| (info.symbol.clone(), info))
            .collect();

        Ok(())
    }

    pub async fn exchange_info(&self) -> Result<BinanceInstrumentsPayload> {
        let url = format!("{}/fapi/v1/exchangeInfo", self.base_url);
        self.client
            .get(url)
            .send_ok("binance_perp_exchange_info")
            .await?
            .json()
            .await
            .map_err(Into::into)
    }

    pub async fn depth(&self, symbol: &str, limit: u32) -> Result<BinanceDepth> {
        let url = format!("{}/fapi/v1/depth", self.base_url);
        self.client
            .get(url)
            .query(&[("symbol", symbol), ("limit", &limit.to_string())])
            .send_ok("binance_perp_depth")
            .await?
            .json()
            .await
            .map_err(Into::into)
    }

    pub async fn funding_rates(&self) -> Result<Vec<BinanceFundingRate>> {
        let url = format!("{}/fapi/v1/fundingRate", self.base_url);
        self.client
            .get(url)
            .query(&[("limit", "1000")])
            .send_ok("binance_perp_funding_rates")
            .await?
            .json()
            .await
            .map_err(Into::into)
    }

    pub async fn all_market_specs(&self) -> Result<Vec<(Symbol, MarketSpec)>> {
        let info = self.exchange_info().await?;
        let mut specs = Vec::new();

        for sym in info.symbols {
            if sym.contract_type != ContractType::Perpetual || sym.status != ContractStatus::Trading
            {
                continue;
            }

            let mut price_tick_size = None;
            let mut qty_step_size = None;
            let mut min_notional = None;

            for filter in sym.filters {
                match filter {
                    SymbolFilter::PriceFilter { tick_size } => price_tick_size = Some(tick_size),
                    SymbolFilter::LotSize { step_size } => qty_step_size = Some(step_size),
                    SymbolFilter::MinNotional { notional } => min_notional = Some(notional),
                    _ => {}
                }
            }

            if let (Some(price_tick_size), Some(qty_step_size), Some(min_notional)) =
                (price_tick_size, qty_step_size, min_notional)
            {
                let price_decimals: u32 = price_tick_size.normalize().scale();
                let size_decimals: u32 = qty_step_size.normalize().scale();

                let spec = MarketSpec {
                    symbol: sym.base_asset.clone().into(), // Using coin name as symbol for perps, similar to Hyperliquid
                    size_decimals,
                    min_size: qty_step_size,
                    size_increment: qty_step_size,
                    price_decimals,
                    min_price: price_tick_size,
                    price_increment: price_tick_size,
                    contract_size: Decimal::ONE,
                    market_type: MarketType::Perp,
                    max_leverage: None, // Binance API doesn't return max leverage in exchangeInfo easily without auth?
                    min_notional: Some(min_notional),
                };
                specs.push((sym.base_asset.into(), spec));
            }
        }

        Ok(specs)
    }
}

pub struct BinancePerpPublicReader {
    pub client: HttpClient,
    pub venue: Venue,
    pub changes_tx: ChangesTx,
    pub mapping: Mapping,
}

#[async_trait]
impl PublicExchangeReader for BinancePerpPublicReader {
    async fn fetch_book(&self) -> Result<()> {
        Ok(())
    }

    async fn fetch_funding(&self) -> Result<()> {
        Ok(())
    }

    async fn fetch_market_specs(&self) -> Result<()> {
        match self.client.all_market_specs().await {
            Ok(specs) => {
                let mut actions = Vec::new();
                for (symbol, mut spec) in specs {
                    let symbol = format!("{}USDT", symbol).into();
                    if let Ok(details) = self.mapping.map_instrument(&self.venue, &symbol) {
                        let mapped_symbol = details.key();
                        let key = VenueBookKey {
                            venue: self.venue.clone(),
                            instrument: mapped_symbol.clone(),
                        };
                        spec.symbol = mapped_symbol.clone();
                        actions.push(Action::SetMarketSpec(key, spec));
                    }
                }
                send_changes(&self.changes_tx, actions);
            }
            Err(err) => error!("couldn't fetch market specs {err:?}"),
        }
        Ok(())
    }
}
