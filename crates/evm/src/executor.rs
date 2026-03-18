use alloy::{
    hex,
    network::EthereumWallet,
    primitives::{Bytes, B256},
    providers::{Provider, ProviderBuilder, RootProvider},
    rpc::types::TransactionRequest,
    signers::local::PrivateKeySigner,
    sol_types::SolType,
};
use anyhow::{anyhow, Result};
use chrono::Utc;
use evm_types::EvmTxRequest;
use rengine_metrics::counters::increment_counter;
use rengine_types::{
    db::{EvmTxDb, Record},
    Venue,
};
use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    Arc,
};
use tokio::sync::{mpsc::Sender, oneshot};
use tracing::warn;

#[derive(Debug)]
pub struct PendingTx {
    pub hash: B256,
    pub nonce: u64,
    pub from: alloy::primitives::Address,
    pub to: Option<alloy::primitives::Address>,
    pub value: alloy::primitives::U256,
    pub gas_limit: u64,
    pub max_fee_per_gas: Option<u128>,
    pub max_priority_fee_per_gas: Option<u128>,
    pub data: Bytes,
    pub result_sender: oneshot::Sender<Result<()>>,
}

#[derive(Debug, Default)]
pub struct NonceManager {
    pub nonce: AtomicU64,
    pub initialized: AtomicBool,
}

impl NonceManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get_and_increment(&self) -> u64 {
        self.nonce.fetch_add(1, Ordering::SeqCst)
    }

    pub fn reset(&self, nonce: u64) {
        self.nonce.store(nonce, Ordering::SeqCst);
    }

    pub fn set_initialized(&self) {
        self.initialized.store(true, Ordering::SeqCst);
    }

    pub fn is_initialized(&self) -> bool {
        self.initialized.load(Ordering::SeqCst)
    }
}

pub struct EvmExecutor {
    pub provider: Arc<dyn Provider + Send + Sync>,
    pub nonce_manager: Arc<NonceManager>,
    pub pending_tx_tx: Sender<PendingTx>,
    pub from: alloy::primitives::Address,
    pub chain_id: u64,
    pub venue: Venue,
}

impl EvmExecutor {
    pub async fn new(
        rpc_url: String,
        private_key: String,
        nonce_manager: Arc<NonceManager>,
        pending_tx_tx: Sender<PendingTx>,
        chain_id: u64,
        venue: Venue,
    ) -> Result<Self> {
        let signer: PrivateKeySigner = private_key
            .parse()
            .map_err(|e| anyhow!("failed to parse private key: {e}"))?;
        let from = signer.address();
        let wallet = EthereumWallet::from(signer);

        let provider = ProviderBuilder::new().wallet(wallet).connect_http(
            rpc_url
                .parse()
                .map_err(|e| anyhow!("failed to parse rpc url: {e}"))?,
        );

        if !nonce_manager.is_initialized() {
            let nonce = provider
                .get_transaction_count(from)
                .await
                .map_err(|e| anyhow!("failed to fetch nonce: {e}"))?;
            nonce_manager.reset(nonce);
            nonce_manager.set_initialized();
        }

        Ok(Self {
            provider: Arc::new(provider),
            nonce_manager,
            pending_tx_tx,
            from,
            chain_id,
            venue,
        })
    }

    pub async fn new_with_provider(
        provider: RootProvider,
        private_key: String,
        pending_tx_tx: Sender<PendingTx>,
        chain_id: u64,
        venue: Venue,
    ) -> Result<Self> {
        let signer: PrivateKeySigner = private_key
            .parse()
            .map_err(|e| anyhow!("failed to parse private key: {e}"))?;
        let from = signer.address();
        let wallet = EthereumWallet::from(signer);

        let provider = ProviderBuilder::new()
            .wallet(wallet)
            .connect_provider(provider);

        let nonce_manager = Arc::new(NonceManager::new());
        let nonce = provider
            .get_transaction_count(from)
            .await
            .map_err(|e| anyhow!("failed to fetch nonce: {e}"))?;
        nonce_manager.reset(nonce);
        nonce_manager.set_initialized();

        Ok(Self {
            provider: Arc::new(provider),
            nonce_manager,
            pending_tx_tx,
            from,
            chain_id,
            venue,
        })
    }

    pub async fn execute_evm_tx(&self, tx: Vec<u8>) -> Result<Option<Record>> {
        let req = EvmTxRequest::abi_decode(&tx)
            .map_err(|e| anyhow!("failed to decode evm tx request: {e}"))?;

        let mut tx_req = TransactionRequest::default()
            .to(req.to)
            .value(req.value)
            .input(req.data.clone().into());

        tx_req.chain_id = Some(self.chain_id);

        // Estimate fees
        let fees = self
            .provider
            .estimate_eip1559_fees()
            .await
            .map_err(|e| anyhow!("failed to estimate fees: {e}"))?;
        tx_req.max_fee_per_gas = Some(fees.max_fee_per_gas);
        tx_req.max_priority_fee_per_gas = Some(fees.max_priority_fee_per_gas);

        // Estimate gas without nonce to avoid "Nonce too high" error
        let gas_limit = match self.provider.estimate_gas(tx_req.clone()).await {
            Ok(gas) => gas,
            Err(e) => {
                tracing::error!("failed to estimate gas: {e}");
                return Ok(Some(Record::EvmTx(EvmTxDb {
                    tx_hash: String::new(),
                    block_number: 0,
                    nonce: 0,
                    from: self.from.to_string(),
                    to: req.to.to_string(),
                    value: req.value.to_string(),
                    gas_limit: 0,
                    gas_price: None,
                    max_fee_per_gas: Some(fees.max_fee_per_gas.to_string()),
                    max_priority_fee_per_gas: Some(fees.max_priority_fee_per_gas.to_string()),
                    data: hex::encode(&req.data),
                    transfers: None,
                    error: Some(format!("failed to estimate gas: {e}")),
                    created_at: Utc::now(),
                })));
            }
        };

        let nonce = self.nonce_manager.get_and_increment();

        tx_req = tx_req.nonce(nonce);
        tx_req.gas = Some(gas_limit);

        // Send transaction
        let pending = match self.provider.send_transaction(tx_req.clone()).await {
            Ok(pending) => {
                increment_counter(format!("{}|evm", self.venue));
                pending
            }
            Err(e) => {
                increment_counter(format!("{}|evm", self.venue));
                tracing::error!("failed to send transaction: {e}");
                // If transaction fails, we should reset the nonce to the current on-chain value
                // This handles cases like "Nonce too high" or other submission errors
                if let Ok(current_nonce) = self.provider.get_transaction_count(self.from).await {
                    self.nonce_manager.reset(current_nonce);
                }
                return Ok(Some(Record::EvmTx(EvmTxDb {
                    tx_hash: String::new(),
                    block_number: 0,
                    nonce,
                    from: self.from.to_string(),
                    to: req.to.to_string(),
                    value: req.value.to_string(),
                    gas_limit,
                    gas_price: None,
                    max_fee_per_gas: Some(fees.max_fee_per_gas.to_string()),
                    max_priority_fee_per_gas: Some(fees.max_priority_fee_per_gas.to_string()),
                    data: hex::encode(&req.data),
                    transfers: None,
                    error: Some(e.to_string()),
                    created_at: Utc::now(),
                })));
            }
        };
        let hash = *pending.tx_hash();

        let (tx_sender, tx_receiver) = oneshot::channel();

        // Notify reader to monitor
        self.pending_tx_tx
            .send(PendingTx {
                hash,
                nonce,
                from: self.from,
                to: Some(req.to),
                value: req.value,
                gas_limit,
                max_fee_per_gas: Some(fees.max_fee_per_gas),
                max_priority_fee_per_gas: Some(fees.max_priority_fee_per_gas),
                data: req.data.clone(),
                result_sender: tx_sender,
            })
            .await
            .map_err(|_| anyhow!("failed to send pending tx to monitor"))?;

        // Spawn a task to wait for the result and handle nonce reset
        let provider = self.provider.clone();
        let from = self.from;
        let nonce_manager = self.nonce_manager.clone();
        let venue = self.venue.clone();
        tokio::spawn(async move {
            match tx_receiver.await {
                Ok(Ok(())) => {
                    increment_counter(format!("{}|evm", venue));
                }
                Ok(Err(e)) => {
                    increment_counter(format!("{}|evm", venue));
                    warn!("transaction failed, resetting nonce: {e}");
                    if let Ok(nonce) = provider.get_transaction_count(from).await {
                        nonce_manager.reset(nonce);
                    }
                }
                Err(e) => {
                    increment_counter(format!("{}|evm", venue));
                    warn!("transaction monitor channel closed, resetting nonce: {e}");
                    if let Ok(nonce) = provider.get_transaction_count(from).await {
                        nonce_manager.reset(nonce);
                    }
                }
            }
        });
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        test_utils::{deploy_erc20_mock, deploy_multicall3},
        EvmReaderHandler,
    };
    use alloy::{
        node_bindings::Anvil,
        providers::ProviderBuilder,
        signers::local::PrivateKeySigner,
        sol_types::{SolCall, SolValue},
    };
    use evm_types::erc20::ERC20Mock;
    use rengine_config::EvmReaderConfig;
    use rengine_interfaces::db::{
        MockAnalyticRepository, MockEvmLogsRepository, MockMultiCallRepository,
    };
    use rengine_types::State;
    use std::time::Duration;
    use tokio::sync::mpsc::channel;
    use wasm_runtime::RwLock;

    #[tokio::test]
    async fn test_failed_tx_resets_nonce() {
        let anvil = Anvil::new().spawn();
        let rpc_url = anvil.endpoint();
        let private_key = alloy::hex::encode(anvil.keys()[0].to_bytes());
        let signer: PrivateKeySigner = private_key.parse().unwrap();
        let from = signer.address();

        let provider = ProviderBuilder::new()
            .wallet(EthereumWallet::from(signer))
            .connect_http(rpc_url.parse().unwrap());

        let multicall_address = deploy_multicall3(&provider).await.unwrap();

        let cfg = EvmReaderConfig {
            ws_url: anvil.ws_endpoint(),
            http_url: anvil.endpoint(),
            idle_timeout: Duration::from_secs(10),
            multicall_address,
            chain_id: 31337,
            tx_timeout: Duration::from_secs(60),
            tx_poll_interval: Duration::from_millis(100),
        };

        let state = Arc::new(RwLock::new(State::default()));
        let (changes_tx, _changes_rx) = channel(100);
        let mut mock_repo = MockMultiCallRepository::default();
        mock_repo
            .expect_list_multicall()
            .returning(|_| Box::pin(async { Ok(vec![]) }));
        let repo = Arc::new(mock_repo);
        let analytic_repo = Arc::new(MockAnalyticRepository::default());
        let mut mock_evm_logs_repo = MockEvmLogsRepository::default();
        mock_evm_logs_repo
            .expect_list_evm_logs()
            .returning(|_| Box::pin(async { Ok(vec![]) }));
        let evm_logs_repo = Arc::new(mock_evm_logs_repo);

        let handler = EvmReaderHandler::try_new(
            "test".into(),
            cfg,
            state,
            repo,
            evm_logs_repo,
            Some(analytic_repo),
            changes_tx,
        )
        .await
        .expect("failed to init EvmReaderHandler");

        let tx_sender = handler.pending_tx_tx.clone();
        let chain_id = 31337;
        let nonce_manager = Arc::new(NonceManager::new());
        let executor = EvmExecutor::new(
            rpc_url,
            private_key,
            nonce_manager.clone(),
            tx_sender,
            chain_id,
            "test".into(),
        )
        .await
        .unwrap();

        // Create a call that will revert (invalid contract call)
        let invalid_address = alloy::primitives::Address::from([0xFF; 20]);
        let req = EvmTxRequest {
            to: invalid_address,
            value: alloy::primitives::U256::ZERO,
            data: Bytes::from(vec![0xde, 0xad, 0xbe, 0xef]), // likely to revert
        };
        let encoded_req = req.abi_encode();

        // Execute and expect error
        let _ = executor.execute_evm_tx(encoded_req).await;

        // Wait a bit for the spawned task to reset nonce
        tokio::time::sleep(Duration::from_millis(200)).await;

        // Nonce should be reset to the current value on chain
        let reset_nonce = nonce_manager.nonce.load(Ordering::SeqCst);
        let chain_nonce = provider.get_transaction_count(from).await.unwrap();
        assert_eq!(
            reset_nonce, chain_nonce,
            "Nonce should be reset after failed tx"
        );
    }

    #[tokio::test]
    async fn test_execute_evm_tx() {
        let anvil = Anvil::new().spawn();
        let rpc_url = anvil.endpoint();
        let private_key = alloy::hex::encode(anvil.keys()[0].to_bytes());
        let signer: PrivateKeySigner = private_key.parse().unwrap();
        let from = signer.address();

        let provider = ProviderBuilder::new()
            .wallet(EthereumWallet::from(signer))
            .connect_http(rpc_url.parse().unwrap());

        let token = deploy_erc20_mock(&provider).await.unwrap();
        let multicall_address = deploy_multicall3(&provider).await.unwrap();

        // Setup Reader
        let cfg = EvmReaderConfig {
            ws_url: anvil.ws_endpoint(),
            http_url: anvil.endpoint(),
            idle_timeout: Duration::from_secs(10),
            multicall_address,
            chain_id: 31337,
            tx_timeout: Duration::from_secs(60),
            tx_poll_interval: Duration::from_millis(100),
        };

        let state = Arc::new(RwLock::new(State::default()));
        let (changes_tx, _changes_rx) = channel(100);
        let mut mock_repo = MockMultiCallRepository::default();
        mock_repo
            .expect_list_multicall()
            .returning(|_| Box::pin(async { Ok(vec![]) }));
        let repo = Arc::new(mock_repo);
        let analytic_repo = Arc::new(MockAnalyticRepository::default());
        let mut mock_evm_logs_repo = MockEvmLogsRepository::default();
        mock_evm_logs_repo
            .expect_list_evm_logs()
            .returning(|_| Box::pin(async { Ok(vec![]) }));
        let evm_logs_repo = Arc::new(mock_evm_logs_repo);

        let handler = EvmReaderHandler::try_new(
            "test".into(),
            cfg,
            state,
            repo,
            evm_logs_repo,
            Some(analytic_repo),
            changes_tx,
        )
        .await
        .expect("failed to init EvmReaderHandler");

        let tx_sender = handler.pending_tx_tx.clone();

        let chain_id = 31337;
        let nonce_manager = Arc::new(NonceManager::new());
        let executor = EvmExecutor::new(
            rpc_url,
            private_key,
            nonce_manager,
            tx_sender,
            chain_id,
            "test".into(),
        )
        .await
        .unwrap();

        // Create an approve call
        let spender = alloy::primitives::Address::from([0x01; 20]);
        let amount = alloy::primitives::U256::from(1000);
        let approve_call = ERC20Mock::approveCall { spender, amount };
        let data = approve_call.abi_encode();

        let req = EvmTxRequest {
            to: token,
            value: alloy::primitives::U256::ZERO,
            data: Bytes::from(data),
        };
        let encoded_req = req.abi_encode();

        executor.execute_evm_tx(encoded_req).await.unwrap();

        // Verify allowance
        let allowance_call = ERC20Mock::allowanceCall {
            owner: from,
            spender,
        };
        let allowance_data = allowance_call.abi_encode();
        let tx_req = TransactionRequest::default()
            .to(token)
            .input(Bytes::from(allowance_data).into());

        let res = provider.call(tx_req).await.unwrap();
        let allowance = ERC20Mock::allowanceCall::abi_decode_returns(&res).unwrap();
        assert_eq!(allowance, amount);
    }

    #[tokio::test]
    async fn test_strategy_execution() {
        let anvil = Anvil::new().spawn();
        let rpc_url = anvil.endpoint();
        let private_key = alloy::hex::encode(anvil.keys()[0].to_bytes());
        let signer: PrivateKeySigner = private_key.parse().unwrap();
        let from = signer.address();

        let provider = ProviderBuilder::new()
            .wallet(EthereumWallet::from(signer))
            .connect_http(rpc_url.parse().unwrap());

        let token = deploy_erc20_mock(&provider).await.unwrap();
        let multicall_address = deploy_multicall3(&provider).await.unwrap();

        // Setup Reader
        let cfg = EvmReaderConfig {
            ws_url: anvil.ws_endpoint(),
            http_url: anvil.endpoint(),
            idle_timeout: Duration::from_secs(10),
            multicall_address,
            chain_id: 31337,
            tx_timeout: Duration::from_secs(60),
            tx_poll_interval: Duration::from_millis(100),
        };

        let state = Arc::new(RwLock::new(State::default()));
        let (changes_tx, _changes_rx) = channel(100);
        let mut mock_repo = MockMultiCallRepository::default();
        mock_repo
            .expect_list_multicall()
            .returning(|_| Box::pin(async { Ok(vec![]) }));
        let repo = Arc::new(mock_repo);
        let mut mock_analytic = MockAnalyticRepository::default();
        mock_analytic
            .expect_batch_insert()
            .returning(|_| Box::pin(async { Ok(()) }));
        let analytic_repo = Arc::new(mock_analytic);
        let mut mock_evm_logs_repo = MockEvmLogsRepository::default();
        mock_evm_logs_repo
            .expect_list_evm_logs()
            .returning(|_| Box::pin(async { Ok(vec![]) }));
        let evm_logs_repo = Arc::new(mock_evm_logs_repo);

        let handler = EvmReaderHandler::try_new(
            "test".into(),
            cfg,
            state,
            repo,
            evm_logs_repo,
            Some(analytic_repo),
            changes_tx,
        )
        .await
        .expect("failed to init EvmReaderHandler");

        let tx_sender = handler.pending_tx_tx.clone();

        let chain_id = 31337;
        let nonce_manager = Arc::new(NonceManager::new());
        let executor = EvmExecutor::new(
            rpc_url,
            private_key,
            nonce_manager,
            tx_sender,
            chain_id,
            "test".into(),
        )
        .await
        .unwrap();

        // Load the WASM strategy
        let wasm_bytes = include_bytes!("../../../strategies-wasm/test_strategy_evm.cwasm");

        // Initialize runtime
        use rengine_types::{ExecutionRequest, State};
        use std::sync::Arc;
        use wasm_runtime::{strategy::StrategyRuntime, Runtime, RwLock};

        let state = Arc::new(RwLock::new(State::default()));
        let mut runtime = Runtime::new(state).unwrap();

        let instance = runtime.instantiate_strategy(wasm_bytes).unwrap();

        // Execute strategy
        let (_, requests_with_logs) = runtime.execute(&instance, &[], None).unwrap();
        let requests = requests_with_logs.requests;
        assert_eq!(requests.len(), 1);

        let (account, data) = match &requests[0] {
            ExecutionRequest::EvmTx((acc, data)) => (acc, &data.0),
            _ => panic!("expected EvmTx"),
        };

        assert_eq!(account.venue, "venue");

        // The strategy returns EvmTxRequest encoded bytes
        let mut req = <EvmTxRequest as SolValue>::abi_decode(data).unwrap();

        // Override the destination to the actual deployed token for this test
        req.to = token;

        let encoded_req = req.abi_encode();

        let res = executor.execute_evm_tx(encoded_req).await.unwrap();
        if let Some(record) = res {
            panic!("Execution failed: {:?}", record);
        }

        // Wait for transaction to be mined
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;

        // Verify allowance
        // The strategy approves 1000 to Address::repeat_byte(0x1)
        let spender = alloy::primitives::Address::repeat_byte(0x1);
        let amount = alloy::primitives::U256::from(1000);

        let allowance_call = ERC20Mock::allowanceCall {
            owner: from,
            spender,
        };
        let allowance_data = allowance_call.abi_encode();
        let tx_req = TransactionRequest::default()
            .to(token)
            .input(Bytes::from(allowance_data).into());

        let res = provider.call(tx_req).await.unwrap();
        let allowance = ERC20Mock::allowanceCall::abi_decode_returns(&res).unwrap();

        if allowance != amount {
            panic!(
                "Allowance: {}, Expected: {}, Token: {:?}, Spender: {:?}, Owner: {:?}, ReqTo: {:?}",
                allowance, amount, token, spender, from, req.to
            );
        }
        assert_eq!(allowance, amount);
    }
}
