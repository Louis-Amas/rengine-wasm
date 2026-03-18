use anyhow::Result;
use rengine_interfaces::db::TradesRepository;
use rengine_types::{db::TradeDb, Account, BalanceKey, Side, Timestamp, Trade, TradeId};
use rust_decimal::Decimal;
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};
use tokio::task;
use tracing::{debug, error, info};

fn add_delta(map: &mut HashMap<BalanceKey, Decimal>, key: BalanceKey, delta: Decimal) {
    *map.entry(key).or_insert(Decimal::ZERO) += delta;
}

#[derive(Default, Debug, Clone)]
struct State {
    seen_trade: HashSet<TradeId>,
    latest_emitted: HashMap<BalanceKey, Timestamp>,
    deltas: HashMap<BalanceKey, Decimal>, // cumulative trade-driven deltas
    initial: HashMap<BalanceKey, Decimal>, // initial balances snapshot
}

/// Accumulates spot exposure deltas per (account, symbol) from trades,
/// handling base/quote legs and fees. You can feed it new trades over time.
/// Now supports seeding initial balances and computing exposure = initial + deltas.
pub(crate) struct TradesExposure {
    trades_repo: Arc<dyn TradesRepository>,
    state: State,
}

impl TradesExposure {
    pub(crate) fn new(trades_repo: Arc<dyn TradesRepository>) -> Self {
        Self {
            trades_repo,
            state: <_>::default(),
        }
    }

    pub(crate) async fn load_account_exposure(
        &mut self,
        account: Account,
    ) -> Result<HashMap<BalanceKey, Decimal>> {
        let exposures = self.trades_repo.load_exposures(account.clone()).await?;

        for exposure in exposures {
            let base_key = BalanceKey {
                account: account.clone(),
                symbol: exposure.base.clone(),
            };
            let quote_key = BalanceKey {
                account: account.clone(),
                symbol: exposure.quote.clone(),
            };

            self.state
                .initial
                .insert(base_key.clone(), exposure.base_exposure);
            self.state
                .initial
                .insert(quote_key.clone(), exposure.quote_exposure);

            self.state
                .latest_emitted
                .insert(base_key, exposure.latest_emitted_at);
            self.state
                .latest_emitted
                .insert(quote_key, exposure.latest_emitted_at);
        }

        info!(?self.state,  "Initialize exposure for account {account}");

        Ok(self.state.initial.clone())
    }

    pub(crate) fn apply_trades<'a, I>(
        &mut self,
        trades: I,
    ) -> (HashMap<BalanceKey, Decimal>, Vec<TradeDb>)
    where
        I: IntoIterator<Item = &'a Trade>,
    {
        let mut batch: HashMap<BalanceKey, Decimal> = HashMap::new();

        // FIXME: ensure it's sorted before ?
        let mut trades_vec: Vec<&Trade> = trades.into_iter().collect();
        trades_vec.sort_by_key(|t| t.emitted_at);
        let trades = trades_vec;

        let mut new_trades = vec![];
        let mut updated_keys = HashSet::new();

        for trade in trades {
            if self.state.seen_trade.contains(&trade.trade_id) {
                debug!(?trade, "already handled trade");
                continue;
            }
            self.state.seen_trade.insert(trade.trade_id);

            let base_key = BalanceKey {
                account: trade.account.clone(),
                symbol: trade.base.clone(),
            };
            let quote_key = BalanceKey {
                account: trade.account.clone(),
                symbol: trade.quote.clone(),
            };

            let base_latest_emitted = self
                .state
                .latest_emitted
                .entry(base_key.clone())
                .or_default();

            if *base_latest_emitted >= trade.emitted_at {
                continue;
            }
            *base_latest_emitted = trade.emitted_at;

            let quote_latest_emitted = self
                .state
                .latest_emitted
                .entry(quote_key.clone())
                .or_default();

            if *quote_latest_emitted >= trade.emitted_at {
                continue;
            }
            info!(?trade, "handle trade");
            *quote_latest_emitted = trade.emitted_at;

            let notional = trade.price * trade.size;

            match trade.side {
                Side::Bid => {
                    // Buy base, pay quote
                    add_delta(&mut batch, base_key.clone(), trade.size);
                    add_delta(&mut batch, quote_key.clone(), -notional);
                }
                Side::Ask => {
                    // Sell base, receive quote
                    add_delta(&mut batch, base_key.clone(), -trade.size);
                    add_delta(&mut batch, quote_key.clone(), notional);
                }
            }

            if trade.fee.is_sign_positive() && trade.fee > Decimal::ZERO {
                if trade.fee_symbol == trade.base {
                    add_delta(&mut batch, base_key.clone(), -trade.fee);
                } else if trade.fee_symbol == trade.quote {
                    add_delta(&mut batch, quote_key.clone(), -trade.fee);
                } else {
                    let fee_key = BalanceKey {
                        account: trade.account.clone(),
                        symbol: trade.fee_symbol.clone(),
                    };
                    add_delta(&mut batch, fee_key.clone(), -trade.fee);
                    updated_keys.insert(fee_key);
                }
            }

            updated_keys.insert(base_key);
            updated_keys.insert(quote_key);
            new_trades.push(TradeDb::from_trade(trade.clone()));
        }

        if new_trades.is_empty() {
            return <_>::default();
        }

        // Recompute exposure only for modified keys
        let mut exposure_after = HashMap::new();
        for key in &updated_keys {
            // Start from initial (default 0)
            let mut total = *self.state.initial.get(key).unwrap_or(&Decimal::ZERO);

            // Add past cumulative deltas (default 0)
            total += *self.state.deltas.get(key).unwrap_or(&Decimal::ZERO);

            // Add current batch delta (default 0)
            total += *batch.get(key).unwrap_or(&Decimal::ZERO);

            // Update exposure_after snapshot
            exposure_after.insert(key.clone(), total);

            // Persist this batch delta into cumulative state
            if let Some(batch_delta) = batch.get(key) {
                add_delta(&mut self.state.deltas, key.clone(), *batch_delta);
            }
        }

        let repo = self.trades_repo.clone();

        // FIXME: improve writing to local db
        let new_trades_cloned = new_trades.clone();
        task::spawn(async move {
            if let Err(err) = repo.record_trades(new_trades_cloned).await {
                error!(?err, "couldn't record trades");
            }
        });

        (exposure_after, new_trades)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{DateTime, Duration, TimeZone, Utc};
    use rengine_interfaces::db::MockTradesRepository;
    use rengine_types::{db::Exposure, Account, MarketType};
    use rust_decimal_macros::dec;
    use std::sync::Arc;

    fn acc() -> Account {
        Account {
            venue: "binance".into(),
            market_type: MarketType::Spot,
            account_id: "hotwallet".into(),
        }
    }

    fn key(account: &Account, sym: &str) -> BalanceKey {
        BalanceKey {
            account: account.clone(),
            symbol: sym.into(),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn mk_trade(
        t: DateTime<Utc>,
        trade_id: i64,
        side: Side,
        price: Decimal,
        size: Decimal,
        fee: Decimal,
        fee_sym: &str,
        market_type: MarketType,
        base: &str,
        quote: &str,
    ) -> Trade {
        Trade {
            emitted_at: t.into(),
            received_at: t.into(),
            order_id: 1,
            trade_id,
            account: acc(),
            base: base.into(),
            quote: quote.into(),
            side,
            market_type,
            price,
            size,
            fee,
            fee_symbol: fee_sym.into(),
        }
    }

    #[tokio::test]
    async fn buy_then_sell_with_quote_fee_updates_exposure() {
        let account = acc();
        let account2 = Account {
            venue: "hyperliquid".into(),
            market_type: MarketType::Perp,
            account_id: "hotwallet".into(),
        };
        let eth = key(&account, "eth");
        let usdc = key(&account, "usdc");

        let t0 = Utc.with_ymd_and_hms(2045, 1, 1, 0, 0, 0).unwrap();
        let t1 = t0 + Duration::seconds(1);
        let t2 = t1 + Duration::seconds(1);

        let mut repo = MockTradesRepository::default();
        let account_clone = account.clone();
        let base_sym = eth.clone().symbol;
        let quote_sym = usdc.clone().symbol;
        repo.expect_load_exposures().return_once(move |_| {
            Box::pin(async move {
                Ok(vec![
                    Exposure {
                        account: account_clone,
                        base: base_sym.clone(),
                        quote: quote_sym.clone(),
                        base_exposure: -dec!(10),
                        quote_exposure: -dec!(5000),
                        at: Utc::now().into(),
                        latest_emitted_at: Utc::now().into(),
                    },
                    Exposure {
                        account: account2,
                        base: base_sym,
                        quote: quote_sym,
                        base_exposure: dec!(10),
                        quote_exposure: dec!(5000),
                        at: Utc::now().into(),
                        latest_emitted_at: Utc::now().into(),
                    },
                ])
            })
        });
        let trades_repo = Arc::new(repo);

        let mut exp = TradesExposure::new(trades_repo);

        exp.load_account_exposure(account.clone()).await.unwrap();

        // Buy 2 ETH @ 1000 USDC
        let buy = mk_trade(
            t1,
            2,
            Side::Bid,
            dec!(1000),
            dec!(2),
            dec!(0),
            "usdc",
            MarketType::Spot,
            "eth",
            "usdc",
        );
        let (after_buy, _trades) = exp.apply_trades([&buy]);
        assert_eq!(after_buy[&eth], dec!(12));
        assert_eq!(after_buy[&usdc], dec!(3000));

        // Sell 1 ETH @ 1100 USDC, fee 0.1 USDC
        let sell = mk_trade(
            t2,
            3,
            Side::Ask,
            dec!(1100),
            dec!(1),
            dec!(0.1),
            "usdc",
            MarketType::Spot,
            "eth",
            "usdc",
        );
        let (after_sell, _trades) = exp.apply_trades([&sell]);
        assert_eq!(after_sell[&eth], dec!(11));
        assert_eq!(after_sell[&usdc], dec!(4099.9));
    }

    #[tokio::test]
    async fn base_fee_and_third_token_fee_are_accounted() {
        let account = acc();
        let eth = key(&account, "eth");
        let usdc = key(&account, "usdc");
        let bnb = key(&account, "bnb");

        let t0 = Utc.with_ymd_and_hms(2045, 1, 1, 0, 0, 0).unwrap();
        let t1 = t0 + Duration::seconds(1);
        let t2 = t1 + Duration::seconds(1);

        let trades_repo = Arc::new(MockTradesRepository::default());

        let mut exp = TradesExposure::new(trades_repo);

        // Buy 1 ETH @ 1000 USDC, fee 0.01 ETH
        let buy = mk_trade(
            t1,
            1,
            Side::Bid,
            dec!(1000),
            dec!(1),
            dec!(0.01),
            "eth",
            MarketType::Spot,
            "eth",
            "usdc",
        );
        let (after_1, _trades) = exp.apply_trades([&buy]);
        assert_eq!(after_1[&eth], dec!(0.99));
        assert_eq!(after_1[&usdc], dec!(-1000));

        // Sell 1 ETH @ 1000 USDC, fee 0.5 BNB
        let sell = mk_trade(
            t2,
            2,
            Side::Ask,
            dec!(1000),
            dec!(1),
            dec!(0.5),
            "bnb",
            MarketType::Spot,
            "eth",
            "usdc",
        );
        let (after_2, _trades) = exp.apply_trades([&sell]);
        assert_eq!(after_2[&eth], dec!(-0.01));
        assert_eq!(after_2[&usdc], dec!(0));
        assert_eq!(after_2[&bnb], dec!(-0.5));
    }

    #[tokio::test]
    async fn duplicate_trade_id_is_ignored() {
        let account = acc();
        let eth = key(&account, "eth");

        let t0 = Utc.with_ymd_and_hms(2045, 1, 1, 0, 0, 0).unwrap();

        let mut repo = MockTradesRepository::default();
        let account_clone = account.clone();
        let base_sym = eth.clone().symbol;
        repo.expect_load_exposures().return_once(move |_| {
            Box::pin(async move {
                Ok(vec![Exposure {
                    account: account_clone,
                    base: base_sym,
                    quote: "usdc".into(),
                    base_exposure: dec!(5),
                    quote_exposure: dec!(0),
                    at: Utc::now().into(),
                    latest_emitted_at: Utc::now().into(),
                }])
            })
        });

        let mut exp = TradesExposure::new(Arc::new(repo));
        exp.load_account_exposure(account.clone()).await.unwrap();

        let trade = mk_trade(
            t0,
            2,
            Side::Bid,
            dec!(1000),
            dec!(1),
            dec!(0),
            "usdc",
            MarketType::Spot,
            "eth",
            "usdc",
        );
        let (result, _trades) = exp.apply_trades([&trade]);
        assert_eq!(result[&eth], dec!(6));
        let (after_dup, _trades) = exp.apply_trades([&trade]);

        // Second application should not change anything
        assert!(after_dup.is_empty());
    }
}
