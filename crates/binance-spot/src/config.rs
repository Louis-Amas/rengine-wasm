use anyhow::{anyhow, Result};
use rengine_types::PublicExchangePollingConfig;
use serde::{Deserialize, Serialize};
use std::env;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BinanceSpotConfig {
    pub api_key: String,
    pub secret_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BinanceSpotPublicConfig {
    #[serde(default)]
    pub public_exchange_polling_config: PublicExchangePollingConfig,
}

impl BinanceSpotConfig {
    pub fn get_credentials(&self) -> Result<(String, String)> {
        let api_key = if self.api_key.starts_with('$') {
            env::var(&self.api_key[1..]).map_err(|_| anyhow!("missing env var {}", self.api_key))?
        } else {
            self.api_key.clone()
        };

        let secret_key = if self.secret_key.starts_with('$') {
            env::var(&self.secret_key[1..])
                .map_err(|_| anyhow!("missing env var {}", self.secret_key))?
        } else {
            self.secret_key.clone()
        };

        Ok((api_key, secret_key))
    }
}
