use crate::{http::HttpClient, hyperliquid::MarketConfig};
use anyhow::Result;
use async_trait::async_trait;
use futures::future::join_all;
use rengine_interfaces::PublicExchangeReader;
use rengine_non_wasm_types::{send_changes, ChangesTx};
use rengine_types::{Action, Level, Mapping, MarketType, TopBookUpdate, Venue, VenueBookKey};
use tokio::sync::broadcast::Receiver;
use tracing::error;

pub struct HyperLiquidPublicReader {
    venue: Venue,
    client: HttpClient,
    mapping: Mapping,
    markets: Vec<MarketConfig>,
    changes_tx: ChangesTx,
}

impl HyperLiquidPublicReader {
    pub fn new(
        venue: Venue,
        changes_tx: ChangesTx,
        markets: Vec<MarketConfig>,
        mapping: Mapping,
    ) -> Self {
        Self {
            venue,
            client: HttpClient::default(),
            markets,
            mapping,
            changes_tx,
        }
    }

    pub(crate) async fn run_on_reconnect(self, mut reconnect: Receiver<()>) -> Result<()> {
        loop {
            let _ = reconnect.recv().await;

            let _ = self.fetch_book().await;

            let _ = self.fetch_funding().await;
        }

        #[allow(unreachable_code)]
        Ok(())
    }
}

#[async_trait]
impl PublicExchangeReader for HyperLiquidPublicReader {
    async fn fetch_market_specs(&self) -> Result<()> {
        let mut actions = Vec::new();

        // Fetch perp market specs
        match self.client.all_perp_market_specs().await {
            Ok(specs) => {
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
            }
            Err(err) => error!("couldn't fetch perp market specs {err:?}"),
        }

        // Fetch spot market specs
        match self.client.all_spot_market_specs().await {
            Ok(specs) => {
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
            }
            Err(err) => error!("couldn't fetch spot market specs {err:?}"),
        }

        send_changes(&self.changes_tx, actions);

        Ok(())
    }

    async fn fetch_book(&self) -> Result<()> {
        let futures: Vec<_> = self
            .markets
            .iter()
            .map(|market| self.client.l2_snapshot(&market.symbol))
            .collect();

        let actions: Vec<_> = join_all(futures)
            .await
            .into_iter()
            .zip(&self.markets)
            .filter_map(|(result, market)| match result {
                Ok(snapshot) => {
                    let bids = snapshot.levels.bids;
                    let top_bid = bids.first().map(|o| Level {
                        size: o.sz,
                        price: o.px,
                    })?;

                    let asks = snapshot.levels.asks;
                    let top_ask = asks.first().map(|o| Level {
                        size: o.sz,
                        price: o.px,
                    })?;

                    let key = VenueBookKey {
                        venue: self.venue.clone(),
                        instrument: market.symbol.clone(),
                    };

                    let top_book_update = TopBookUpdate { top_bid, top_ask };

                    Some(Action::SetTopBook(key, top_book_update))
                }
                Err(err) => {
                    error!("couldn't fetch book {err:?}");
                    None
                }
            })
            .collect();

        send_changes(&self.changes_tx, actions);

        Ok(())
    }

    async fn fetch_funding(&self) -> Result<()> {
        let markets = self
            .markets
            .iter()
            .filter(|market| market.market_type == MarketType::Perp);

        let futures: Vec<_> = markets
            .clone()
            .map(|market| self.client.funding_rate(&market.symbol))
            .collect();

        let actions: Vec<_> = join_all(futures)
            .await
            .into_iter()
            .zip(markets)
            .filter_map(|(result, market)| match result {
                Ok(funding) => {
                    let symbol = self.mapping.map_symbol(&self.venue, &market.symbol).ok()?;
                    let key = format!("{}-funding-{}", self.venue, symbol).as_str().into();

                    Some(Action::SetIndicator(key, funding.funding_rate))
                }
                Err(err) => {
                    error!("couldn't fetch funding {err:?}");
                    None
                }
            })
            .collect();

        send_changes(&self.changes_tx, actions);

        Ok(())
    }
}
