use crate::public::types::{BinanceDepth, BinanceInstrumentsPayload, SymbolFilter};
use anyhow::Result;
use async_trait::async_trait;
use rengine_interfaces::PublicExchangeReader;
use rengine_non_wasm_types::{send_changes, ChangesTx};
use rengine_types::{Action, Mapping, MarketSpec, MarketType, Symbol, Venue, VenueBookKey};
use rengine_utils::http::RequestExt;
use reqwest::Client;
use rust_decimal::Decimal;
use tracing::error;

pub const BINANCE_SPOT_DEFAULT_HTTP: &str = "https://api.binance.com";

#[derive(Clone)]
pub struct HttpClient {
    client: Client,
    base_url: String,
}

impl Default for HttpClient {
    fn default() -> Self {
        Self {
            client: Client::new(),
            base_url: BINANCE_SPOT_DEFAULT_HTTP.to_string(),
        }
    }
}

impl HttpClient {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            client: Client::new(),
            base_url: base_url.into(),
        }
    }

    pub const fn client(&self) -> &Client {
        &self.client
    }

    pub fn get(&self, path: &str) -> reqwest::RequestBuilder {
        let url = format!("{}{}", self.base_url, path);
        self.client.get(url)
    }

    pub async fn exchange_info(&self) -> Result<BinanceInstrumentsPayload> {
        let url = format!("{}/api/v3/exchangeInfo", self.base_url);
        self.client
            .get(url)
            .query(&[("permissions", "SPOT"), ("symbolStatus", "TRADING")])
            .send_ok("binance_spot_exchange_info")
            .await?
            .json()
            .await
            .map_err(Into::into)
    }

    pub async fn depth(&self, symbol: &str, limit: u32) -> Result<BinanceDepth> {
        let url = format!("{}/api/v3/depth", self.base_url);
        self.client
            .get(url)
            .query(&[("symbol", symbol), ("limit", &limit.to_string())])
            .send_ok("binance_spot_depth")
            .await?
            .json()
            .await
            .map_err(Into::into)
    }

    pub async fn all_market_specs(&self) -> Result<Vec<(Symbol, MarketSpec)>> {
        let info = self.exchange_info().await?;
        let mut specs = Vec::new();

        for sym in info.symbols {
            let mut price_tick_size = None;
            let mut qty_step_size = None;
            let mut min_notional = None;

            for filter in sym.filters {
                match filter {
                    SymbolFilter::PriceFilter { tick_size } => price_tick_size = Some(tick_size),
                    SymbolFilter::LotSize { step_size } => qty_step_size = Some(step_size),
                    SymbolFilter::Notional { min_notional: m } => min_notional = Some(m),
                    _ => {}
                }
            }

            if let (Some(price_tick_size), Some(qty_step_size), Some(min_notional)) =
                (price_tick_size, qty_step_size, min_notional)
            {
                // Calculate decimals from tick_size/step_size
                let price_decimals = price_tick_size.normalize().scale();
                let size_decimals = qty_step_size.normalize().scale();

                let spec = MarketSpec {
                    symbol: sym.base_asset.clone().into(), // Note: MarketSpec symbol usually refers to the pair, but here we might need to adjust.
                    // Hyperliquid uses the coin name for perps, but for spot it uses pair?
                    // Let's check hyperliquid implementation again.
                    // Hyperliquid spot: symbol: market.name.clone() (e.g. "ETH/USDC")
                    // Binance symbols are like "ETHUSDC".
                    // I should probably construct the pair name or use the binance symbol.
                    // For now I'll use the binance symbol as the key, but maybe I should format it as Base/Quote if that's the convention.
                    // But rengine-types Symbol is SharedStr.
                    size_decimals,
                    min_size: qty_step_size,
                    size_increment: qty_step_size,
                    price_decimals,
                    min_price: price_tick_size,
                    price_increment: price_tick_size,
                    contract_size: Decimal::ONE,
                    market_type: MarketType::Spot,
                    max_leverage: None,
                    min_notional: Some(min_notional),
                };

                // Construct symbol name. Hyperliquid uses "ETH/USDC". Binance has "ETH" and "USDC" as base/quote.
                // I'll use "BASE/QUOTE" format to match Hyperliquid spot if possible, or just the symbol if that's what's expected.
                // In hyperliquid/src/http.rs: market_spec_from_spot uses market.name.
                let symbol_name = format!("{}{}", sym.base_asset, sym.quote_asset);

                specs.push((symbol_name.into(), spec));
            }
        }

        Ok(specs)
    }
}

pub struct BinanceSpotPublicReader {
    pub client: HttpClient,
    pub venue: Venue,
    pub changes_tx: ChangesTx,
    pub mapping: Mapping,
}

#[async_trait]
impl PublicExchangeReader for BinanceSpotPublicReader {
    async fn fetch_book(&self) -> Result<()> {
        // Not supported
        Ok(())
    }

    async fn fetch_funding(&self) -> Result<()> {
        // Not supported
        Ok(())
    }

    async fn fetch_market_specs(&self) -> Result<()> {
        match self.client.all_market_specs().await {
            Ok(specs) => {
                let mut actions = Vec::new();
                for (symbol, mut spec) in specs {
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
