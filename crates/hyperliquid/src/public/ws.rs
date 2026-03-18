use crate::{
    types::{HyperliquidTrades, L2Book},
    ws::{ExtraData, Message},
};
use anyhow::Result;
use rengine_metrics::latencies::{record_latency, LatencyId};
use rengine_non_wasm_types::{send_changes, ChangesTx, TopBookRegistry};
use rengine_types::{Action, Level, PublicTrade, PublicTrades, TopBookUpdate, Venue, VenueBookKey};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::mpsc::UnboundedReceiver;
use tracing::{error, warn};

pub(crate) struct HyperLiquidPublicStreamer {
    venue: Venue,
    changes_tx: ChangesTx,
    registry: Arc<TopBookRegistry>,
}

impl HyperLiquidPublicStreamer {
    pub(crate) const fn new(
        venue: Venue,
        changes_tx: ChangesTx,
        registry: Arc<TopBookRegistry>,
    ) -> Self {
        Self {
            venue,
            changes_tx,
            registry,
        }
    }

    fn handle_book_message(&self, l2: L2Book) -> Result<()> {
        let mut levels = l2.data.levels.into_iter();
        let bids = levels.next().ok_or_else(|| anyhow::anyhow!("no bids"))?;
        let asks = levels.next().ok_or_else(|| anyhow::anyhow!("no asks"))?;
        let top_bid = bids
            .first()
            .map(|b| Level {
                price: b.px,
                size: b.sz,
            })
            .ok_or_else(|| anyhow::anyhow!("empty bids"))?;
        let top_ask = asks
            .first()
            .map(|a| Level {
                price: a.px,
                size: a.sz,
            })
            .ok_or_else(|| anyhow::anyhow!("empty asks"))?;
        let upd = TopBookUpdate { top_bid, top_ask };
        let key = VenueBookKey {
            venue: self.venue.clone(),
            instrument: l2.data.coin,
        };

        let sender = self.registry.get_sender(key.clone());
        if let Err(e) = sender.send(upd) {
            warn!("Failed to send TopBookUpdate for {:?}: {:?}", key, e);
        }
        Ok(())
    }

    fn handle_trades_message(&self, trades: HyperliquidTrades) -> Result<()> {
        if let Some(first) = trades.data.first() {
            let key = VenueBookKey {
                venue: self.venue.clone(),
                instrument: first.coin.clone(),
            };

            let data: Vec<PublicTrade> = trades
                .data
                .into_iter()
                .map(|trade| PublicTrade {
                    price: trade.px,
                    size: trade.sz,
                    side: trade.side.into(),
                    time: trade.time.timestamp_millis() as u64,
                    trade_id: trade.tid.to_string(),
                    book_key: key.clone(),
                })
                .collect();

            let action = Action::SetTradeFlow(key, PublicTrades { data });

            send_changes(&self.changes_tx, vec![action]);
        }

        Ok(())
    }

    pub(crate) async fn handle_connection(
        self,
        mut receiver: UnboundedReceiver<(Message, Option<ExtraData>)>,
    ) -> Result<()> {
        let mut book_instant = Instant::now();
        while let Some((msg, _)) = receiver.recv().await {
            match msg {
                Message::L2Book(book) => {
                    if let Err(err) = self.handle_book_message(book) {
                        warn!(?err);
                    }

                    if book_instant.elapsed() > Duration::from_millis(11) {
                        record_latency(LatencyId::HyperliquidMarketUpdateWs, book_instant);
                    }
                    book_instant = Instant::now();
                }
                Message::Trades(trades) => {
                    if let Err(err) = self.handle_trades_message(trades) {
                        warn!(?err);
                    }
                }

                Message::Unknown | Message::Pong | Message::SubscriptionResponse => {}
                _ => {
                    error!("couldn't handle message {:?}", msg);
                }
            }
        }

        Ok(())
    }
}
