use parking_lot::RwLock;
use rengine_types::{PublicTrade, State, VenueBookKey};
use std::{collections::HashMap, sync::Arc};

pub struct WasmStateWrapper {
    pub state: Arc<RwLock<State>>,
    pub logs: Vec<String>,
    /// Aggregated trade flows passed to transformers for this execution cycle
    pub aggregated_trade_flows: HashMap<VenueBookKey, Vec<PublicTrade>>,
}

#[macro_export]
macro_rules! impl_imports_for {
    ($trait_name:ident,$struct_name:ident) => {
        use std::str::FromStr;
        impl $trait_name for $struct_name {
            fn indicator(&mut self, key: String) -> Result<Vec<u8>, String> {
                let state = self.inner.state.read();

                let value = state
                    .indicators
                    .get(key.as_str())
                    .ok_or_else(|| format!("missing indicator {key}"))?;

                borsh::to_vec(value).map_err(|err| err.to_string())
            }

            fn balance(&mut self, key: String) -> Result<Vec<u8>, String> {
                let key =
                    rengine_types::BalanceKey::from_str(&key).map_err(|err| err.to_string())?;

                let state = self.inner.state.read();

                let value = state
                    .balances
                    .get(&key)
                    .ok_or_else(|| format!("missing balance key {key}"))?;

                borsh::to_vec(value).map_err(|err| err.to_string())
            }

            fn book(&mut self, key: String) -> Result<Vec<u8>, String> {
                let venue_key = rengine_types::VenueBookKey::from_str(&key)?;
                let state = self.inner.state.read();

                let book = state
                    .book
                    .get(&venue_key)
                    .ok_or_else(|| format!("missing book for venue {venue_key:?}"))?;

                borsh::to_vec(book).map_err(|err| err.to_string())
            }

            fn open_orders(&mut self, key: String) -> Result<Vec<u8>, String> {
                let key = rengine_types::BookKey::from_str(&key).map_err(|err| err.to_string())?;
                let state = self.inner.state.read();

                // FIXME: use constant map here
                let empty_venue_orders = std::collections::HashMap::new();
                let orders = state.open_orders.get(&key).unwrap_or(&empty_venue_orders);

                borsh::to_vec(orders).map_err(|err| err.to_string())
            }

            fn perp_positions(&mut self, key: String) -> Result<Vec<u8>, String> {
                let key = rengine_types::BookKey::from_str(&key).map_err(|err| err.to_string())?;

                let state = self.inner.state.read();

                let position = state
                    .positions
                    .get(&key)
                    .ok_or_else(|| format!("missing position key {key}"))?;

                borsh::to_vec(position).map_err(|err| err.to_string())
            }

            fn spot_exposure(&mut self, key: String) -> Result<Vec<u8>, String> {
                let key =
                    rengine_types::BalanceKey::from_str(&key).map_err(|err| err.to_string())?;

                let state = self.inner.state.read();

                let value = state
                    .spot_exposures
                    .get(&key)
                    .ok_or_else(|| format!("missing spot exposure  {key}"))?;

                borsh::to_vec(value).map_err(|err| err.to_string())
            }

            fn trade_flow(&mut self) -> Result<Vec<u8>, String> {
                // Return all aggregated trade flows as HashMap<VenueBookKey, Vec<PublicTrade>>
                borsh::to_vec(&self.inner.aggregated_trade_flows).map_err(|err| err.to_string())
            }

            fn market_spec(&mut self, key: String) -> Result<Vec<u8>, String> {
                let key =
                    rengine_types::VenueBookKey::from_str(&key).map_err(|err| err.to_string())?;

                let state = self.inner.state.read();

                let value = state
                    .market_specs
                    .get(&key)
                    .ok_or_else(|| format!("missing market spec for {key}"))?;

                borsh::to_vec(value).map_err(|err| err.to_string())
            }

            fn trace(&mut self, value: String) {
                self.inner.logs.push(value);
            }

            fn record_latency(&mut self, key: String, value: u64) {
                rengine_metrics::latencies::record_latency_nanos(key, value);
            }
        }
    };
}
