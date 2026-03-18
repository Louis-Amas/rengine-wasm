use anyhow::Result;
use rengine_types::{
    db::{EvmLogsDb, Exposure, MultiCallDb, Record, StrategyDb, TradeDb, TransformerDb},
    Account, MultiCallId, Venue,
};

#[async_trait::async_trait]
#[mockall::automock]
pub trait StrategyRepository: Send + Sync {
    async fn add_strategy(&self, strategy: StrategyDb) -> Result<()>;
    async fn set_enable(&self, name: &str, enabled: bool) -> Result<()>;
    async fn list_strategies(&self) -> Result<Vec<StrategyDb>>;
}

#[async_trait::async_trait]
#[mockall::automock]
pub trait TransformerRepository: Send + Sync {
    async fn add_transformer(&self, transformer: TransformerDb) -> Result<()>;
    async fn set_transformer_enable(&self, name: &str, enabled: bool) -> Result<()>;
    async fn list_transformers(&self) -> Result<Vec<TransformerDb>>;
}

#[async_trait::async_trait]
#[mockall::automock]
pub trait MultiCallRepository: Send + Sync {
    async fn add_multicall_reader(&self, multicall: MultiCallDb) -> Result<()>;
    async fn remove_multicall_reader(&self, venue: Venue, name: MultiCallId) -> Result<()>;
    async fn list_multicall(&self, venue: Venue) -> Result<Vec<MultiCallDb>>;
}

#[async_trait::async_trait]
#[mockall::automock]
pub trait TradesRepository: Send + Sync {
    async fn record_trades(&self, trades: Vec<TradeDb>) -> Result<()>;
    async fn list_trades(&self, account: Account) -> Result<Vec<TradeDb>>;
    async fn load_exposures(&self, account: Account) -> Result<Vec<Exposure>>;
}

#[async_trait::async_trait]
#[mockall::automock]
pub trait AnalyticRepository: Send + Sync {
    async fn batch_insert(&self, records: Vec<Record>) -> Result<()>;
}

#[async_trait::async_trait]
#[mockall::automock]
pub trait EvmLogsRepository: Send + Sync {
    async fn add_evm_logs(&self, evm_logs: EvmLogsDb) -> Result<()>;
    async fn remove_evm_logs(&self, venue: Venue, name: String) -> Result<()>;
    async fn list_evm_logs(&self, venue: Venue) -> Result<Vec<EvmLogsDb>>;
}
