use crate::public::types::{BinanceAggTrade, BinanceBookTicker, BinanceSpotPublicMessage};
use anyhow::Result;
use rengine_non_wasm_types::{send_changes, ChangesTx, TopBookRegistry};
use rengine_types::{
    Action, Level, PublicTrade, PublicTrades, Side, TopBookUpdate, Venue, VenueBookKey,
};
use std::sync::Arc;
use tokio::sync::mpsc::UnboundedReceiver;
use tracing::warn;

pub struct BinanceSpotPublicStreamer {
    venue: Venue,
    changes_tx: ChangesTx,
    registry: Arc<TopBookRegistry>,
}

impl BinanceSpotPublicStreamer {
    pub const fn new(venue: Venue, changes_tx: ChangesTx, registry: Arc<TopBookRegistry>) -> Self {
        Self {
            venue,
            changes_tx,
            registry,
        }
    }

    fn handle_book_ticker(&self, ticker: BinanceBookTicker) -> Result<()> {
        let top_bid = Level {
            price: ticker.bid_px,
            size: ticker.best_bid_qty,
        };
        let top_ask = Level {
            price: ticker.ask_px,
            size: ticker.best_ask_qty,
        };

        let upd = TopBookUpdate { top_bid, top_ask };
        let key = VenueBookKey {
            venue: self.venue.clone(),
            instrument: ticker.symbol.into(),
        };

        let sender = self.registry.get_sender(key);
        let _ = sender.send(upd);
        Ok(())
    }

    fn handle_agg_trade(&self, trade: BinanceAggTrade) -> Result<()> {
        let key = VenueBookKey {
            venue: self.venue.clone(),
            instrument: trade.symbol.clone().into(),
        };

        // is_buyer_maker: true means the buyer is the maker, so the trade was a sell (taker sold)
        // is_buyer_maker: false means the buyer is the taker, so the trade was a buy (taker bought)
        let side = if trade.is_buyer_maker {
            Side::Ask
        } else {
            Side::Bid
        };

        let public_trade = PublicTrade {
            price: trade.price,
            size: trade.quantity,
            side,
            time: trade.trade_time,
            trade_id: trade.aggregate_trade_id.to_string(),
            book_key: key.clone(),
        };

        let action = Action::SetTradeFlow(
            key,
            PublicTrades {
                data: vec![public_trade],
            },
        );
        send_changes(&self.changes_tx, vec![action]);

        Ok(())
    }

    pub async fn handle_connection(
        self,
        mut receiver: UnboundedReceiver<BinanceSpotPublicMessage>,
    ) -> Result<()> {
        while let Some(msg) = receiver.recv().await {
            match msg {
                BinanceSpotPublicMessage::AggTrade(trade) => {
                    if let Err(err) = self.handle_agg_trade(trade) {
                        warn!(?err, "Failed to handle aggregate trade");
                    }
                }
                BinanceSpotPublicMessage::BookTicker(ticker) => {
                    if let Err(err) = self.handle_book_ticker(ticker) {
                        warn!(?err, "Failed to handle book ticker");
                    }
                }
                BinanceSpotPublicMessage::DepthUpdate(_depth) => {
                    // Handle depth update if needed
                }
                BinanceSpotPublicMessage::SubscriptionResponse(_) => {
                    // Ignore subscription response
                }
            }
        }
        Ok(())
    }
}
