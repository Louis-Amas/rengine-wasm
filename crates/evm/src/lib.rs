pub mod executor;
use crate::{types::NewHead, ws::EvmWebsocketClient};
use alloy::{
    dyn_abi::SolType,
    eips::BlockId,
    hex,
    primitives::{Address, Bytes},
    providers::{
        bindings::IMulticall3::{aggregate3Call, Result as MultiCallResult},
        Provider, RootProvider,
    },
    rpc::types::TransactionRequest,
    sol_types::{sol_data::Array, SolCall, SolEvent},
};
use anyhow::{anyhow, bail, Context, Result};
use chrono::Utc;
use evm_types::{erc20::ERC20Mock::Transfer, Call3};
use executor::PendingTx;
use rengine_config::EvmReaderConfig;
use rengine_interfaces::db::{AnalyticRepository, EvmLogsRepository, MultiCallRepository};
use rengine_metrics::counters::increment_counter;
use rengine_non_wasm_types::{send_changes, ChangesTx};
use rengine_types::{
    db::{EvmLogsDb, EvmTxDb, MultiCallDb, Record},
    evm::MulticallPluginConfig,
    MultiCallId, State, Venue,
};
use serde_json::json;
use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::{
    sync::{
        broadcast,
        mpsc::{channel, Receiver, Sender},
        watch, RwLock as TokioRwLock,
    },
    task::JoinHandle,
    time::interval,
};
use tracing::info;
use wasm_runtime::{
    evm_logs::EvmLogsRuntime, multicall::MulticallRuntime, Multicall, Runtime, RwLock,
};

pub mod logs;
pub mod test_utils;
mod types;
mod ws;

struct MulticallReaderWrapper {
    config: MulticallPluginConfig,
    calls: Vec<Call3>,
    reader: Multicall,
    last_triggerd_block: Option<u64>,
    state: Vec<u8>,
}

type Readers = Arc<TokioRwLock<HashMap<MultiCallId, MulticallReaderWrapper>>>;

#[derive(Clone)]
pub struct EvmReader {
    pub provider: RootProvider,
    pub ws_url: String,
    pub chain_id: u64,
    pub venue: String,
    multicall_readers: Readers,
    log_readers: Arc<TokioRwLock<HashMap<String, JoinHandle<()>>>>,
    wasm_runtime: Arc<TokioRwLock<Runtime>>,

    multicall_address: Address,
    changes_tx: ChangesTx,

    tx_timeout: Duration,
    tx_poll_interval: Duration,
    pub analytic_repo: Option<Arc<dyn AnalyticRepository>>,
}

pub struct EvmReaderHandler {
    pub reader: EvmReader,
    venue: Venue,
    multicall_repo: Arc<dyn MultiCallRepository>,
    evm_logs_repo: Arc<dyn EvmLogsRepository>,
    handles: Vec<JoinHandle<()>>,
    pub pending_tx_tx: Sender<PendingTx>,
}

// --- tiny plan entry to remember which results belong to which reader ---
#[derive(Debug, Clone)]
struct PlanEntry {
    id: MultiCallId,
    start: usize,
    len: usize,
}

type DispatchPlan = Vec<PlanEntry>;

impl EvmReaderHandler {
    pub async fn try_new(
        venue: Venue,
        config: EvmReaderConfig,
        state: Arc<RwLock<State>>,
        multicall_repo: Arc<dyn MultiCallRepository>,
        evm_logs_repo: Arc<dyn EvmLogsRepository>,
        analytic_repo: Option<Arc<dyn AnalyticRepository>>,
        changes_tx: ChangesTx,
    ) -> Result<Self> {
        const MAX_CAPACITY_BROADCAST: usize = 10;

        let (reconnect_tx, reconnect_rx) = broadcast::channel(MAX_CAPACITY_BROADCAST);

        let provider = RootProvider::new_http(config.http_url.parse()?);

        let latest_block: alloy::rpc::types::Block =
            provider
                .get_block(BlockId::latest())
                .await?
                .with_context(|| anyhow!("missing latest block {config:?}"))?;

        let (latest_block_tx, latest_block_rx) = watch::channel(latest_block.header.into());

        let ws_client = EvmWebsocketClient::new(reconnect_tx, latest_block_tx, config.idle_timeout);
        let ws_url = config.ws_url.clone();
        tokio::spawn(async move {
            ws_client.run(ws_url).await;
        });

        let (pending_tx_tx, pending_tx_rx) = channel(100);

        let reader = EvmReader {
            provider,
            ws_url: config.ws_url,
            chain_id: config.chain_id,
            venue: venue.to_string(),
            multicall_readers: Default::default(),
            log_readers: Default::default(),
            wasm_runtime: Arc::new(TokioRwLock::new(Runtime::new(state)?)),
            multicall_address: config.multicall_address,
            changes_tx,
            tx_timeout: config.tx_timeout,
            tx_poll_interval: config.tx_poll_interval,
            analytic_repo,
        };

        let reader_clone = reader.clone();
        let handle = tokio::spawn(async move {
            reader_clone
                .run(latest_block_rx, reconnect_rx, pending_tx_rx)
                .await;
        });

        let handler = Self {
            reader,
            venue,
            multicall_repo,
            evm_logs_repo,
            handles: vec![handle],
            pending_tx_tx,
        };

        handler.load_multicall().await?;
        handler.load_evm_logs().await?;

        Ok(handler)
    }

    async fn load_multicall(&self) -> Result<()> {
        let multicalls = self
            .multicall_repo
            .list_multicall(self.venue.clone())
            .await?;

        for call in multicalls {
            self.add_multicall_reader(call.name.into(), call.wasm)
                .await?;
        }

        Ok(())
    }

    async fn load_evm_logs(&self) -> Result<()> {
        let logs = self.evm_logs_repo.list_evm_logs(self.venue.clone()).await?;

        for log in logs {
            self.add_log_reader(log.name, log.wasm).await?;
        }

        Ok(())
    }

    pub async fn add_multicall_reader(&self, id: MultiCallId, wasm: Vec<u8>) -> Result<()> {
        info!("adding multicall reader with id {id}");
        let mut guard = self.reader.wasm_runtime.write().await;
        let multicall = guard.instantiate_multicall_reader(&wasm)?;
        let state = guard.execute_multicall_reader_init(&multicall)?;

        let config = guard.execute_multicall_reader_config(&multicall)?;
        let calls = guard.execute_multicall_reader_requests(&multicall)?;
        drop(guard);

        info!(?config, ?calls, "decoded multicall config");

        let wrapper = MulticallReaderWrapper {
            config,
            calls,
            reader: multicall,
            last_triggerd_block: None,
            state,
        };

        if self
            .reader
            .multicall_readers
            .write()
            .await
            .insert(id.clone(), wrapper)
            .is_some()
        {
            tracing::info!("replace existing multicall reader {id:?}");

            // FIXME: This could cause race condition
            self.multicall_repo
                .remove_multicall_reader(self.venue.clone(), id.clone())
                .await?;
        }

        self.multicall_repo
            .add_multicall_reader(MultiCallDb {
                venue: self.venue.to_string(),
                name: id.to_string(),
                wasm,
            })
            .await?;

        Ok(())
    }

    pub async fn add_log_reader(&self, id: String, wasm: Vec<u8>) -> Result<()> {
        info!("adding log reader with id {id}");

        let state = {
            let guard = self.reader.wasm_runtime.read().await;
            guard.store.data().inner.state.clone()
        };

        let mut runtime = Runtime::new(state)?;
        let evm_logs = runtime.instantiate_evm_logs(&wasm)?;

        let log_reader = logs::LogReader::new(evm_logs, runtime)?;
        let ws_url = self.reader.ws_url.clone();
        let (tx, mut rx) = channel(100);

        let changes_tx = self.reader.changes_tx.clone();

        let handle = tokio::spawn(async move {
            let reader_task = logs::run_log_reader(ws_url, log_reader, tx);
            let forwarder_task = async {
                while let Some(actions) = rx.recv().await {
                    send_changes(&changes_tx, actions);
                }
                Ok::<(), anyhow::Error>(())
            };

            tokio::select! {
                res = reader_task => {
                    if let Err(e) = res {
                        tracing::error!("log reader error: {e:?}");
                    }
                }
                _ = forwarder_task => {}
            }
        });

        self.reader
            .log_readers
            .write()
            .await
            .insert(id.clone(), handle);

        self.evm_logs_repo
            .add_evm_logs(EvmLogsDb {
                venue: self.venue.to_string(),
                name: id,
                wasm,
            })
            .await?;

        Ok(())
    }

    pub async fn remove_multicall_reader(&self, id: MultiCallId) -> Result<()> {
        info!("removing multicall reader with id {id}");
        let mut readers = self.reader.multicall_readers.write().await;
        let existed = readers.remove(&id).is_some();
        drop(readers);

        if existed {
            // Remove from persistent storage as well
            self.multicall_repo
                .remove_multicall_reader(self.venue.clone(), id.clone())
                .await?;
            info!("removed multicall reader {id}");
        } else {
            tracing::warn!(reader_id = ?id, "attempted to remove non-existent multicall reader");
        }

        Ok(())
    }

    pub async fn remove_log_reader(&self, id: String) -> Result<()> {
        info!("removing log reader with id {id}");
        let mut readers = self.reader.log_readers.write().await;
        if let Some(handle) = readers.remove(&id) {
            handle.abort();

            self.evm_logs_repo
                .remove_evm_logs(self.venue.clone(), id.clone())
                .await?;
            info!("removed log reader {id}");
        } else {
            tracing::warn!(reader_id = ?id, "attempted to remove non-existent log reader");
        }
        Ok(())
    }

    pub fn stop(&self) {
        for h in &self.handles {
            h.abort();
        }
    }
}

impl EvmReader {
    async fn handle_reconnect(recv: Result<(), broadcast::error::RecvError>) -> bool {
        match recv {
            Ok(_) => {
                // Handle reconnect signal (placeholder).
                // If you need to act on reconnect, do it here.
                true
            }
            Err(_) => {
                tracing::warn!("reconnect rx channel is closed");
                false
            }
        }
    }

    // Build the on-chain call AND a plan that says where each reader's results live in the flat array.
    async fn aggregate_multicalls_with_plan(
        &self,
        trigger_block_number: u64,
    ) -> (aggregate3Call, DispatchPlan) {
        let mut readers = self.multicall_readers.write().await;

        let mut calls: Vec<Call3> = Vec::new();
        let mut plan: DispatchPlan = Vec::new();

        for (id, wrapper) in readers.iter_mut() {
            // skip empty readers
            if wrapper.calls.is_empty() {
                continue;
            }

            // Respect the reader's configured "every" interval: only trigger if enough blocks
            // have passed since the last time this reader was triggered.
            //
            // If `every_x_block` is zero (or 1) we treat it as "every block" (always trigger).
            // Only update last_triggerd_block when we actually schedule the reader.
            let every = wrapper.config.every_x_block;
            let should_trigger = match wrapper.last_triggerd_block {
                None => true,
                Some(last) => {
                    if every <= 1 {
                        true
                    } else {
                        trigger_block_number.saturating_sub(last) >= every
                    }
                }
            };

            if !should_trigger {
                continue;
            }

            let start = calls.len();
            calls.extend(wrapper.calls.iter().cloned());
            let len = calls.len() - start;

            wrapper.last_triggerd_block = Some(trigger_block_number);

            plan.push(PlanEntry {
                id: id.clone(),
                start,
                len,
            });
        }

        (aggregate3Call { calls }, plan)
    }

    async fn do_call(&self, call: aggregate3Call) -> Result<Vec<MultiCallResult>> {
        let bytes = Bytes::from(call.abi_encode());
        let tx_request = TransactionRequest::default()
            .to(self.multicall_address)
            .input(bytes.into());

        let res = self.provider.call(tx_request).await?;
        <Array<MultiCallResult> as SolType>::abi_decode(&res).map_err(Into::into)
    }

    // Map results back to each reader using the plan, and let the WASM reader handle its slice.
    async fn dispatch_results(
        &self,
        results: Vec<MultiCallResult>,
        plan: &DispatchPlan,
    ) -> Result<()> {
        let expected: usize = plan.iter().map(|e| e.len).sum();
        if expected != results.len() {
            bail!(
                "multicall results mismatch: got {}, expected {}",
                results.len(),
                expected
            );
        }

        // We’ll need both the runtime (to execute the reader) and the wrappers (to get the reader instances)
        let mut runtime = self.wasm_runtime.write().await;
        let mut readers = self.multicall_readers.write().await;

        let mut changes = vec![];
        for entry in plan {
            if let Some(wrapper) = readers.get_mut(&entry.id) {
                let slice = &results[entry.start..entry.start + entry.len];

                let (new_state, result) = runtime.execute_multicall_reader_handle(
                    &wrapper.reader,
                    &wrapper.state,
                    slice,
                )?;
                wrapper.state = new_state;
                changes.extend(result);
            } else {
                // Reader disappeared between aggregate and dispatch: just log.
                tracing::warn!(reader_id = ?entry.id, "multicall reader missing during dispatch");
            }
        }

        send_changes(&self.changes_tx, changes);

        Ok(())
    }

    async fn handle_latest_block_change(
        &self,
        latest_block_number: u64,
        changed: Result<(), watch::error::RecvError>,
    ) -> bool {
        match changed {
            Ok(()) => {
                increment_counter(format!("{}|evm", self.venue));
                let (call, plan) = self
                    .aggregate_multicalls_with_plan(latest_block_number)
                    .await;

                if call.calls.is_empty() {
                    return true;
                }

                match self.do_call(call).await {
                    Ok(results) => {
                        increment_counter(format!("{}|evm", self.venue));
                        if let Err(err) = self.dispatch_results(results, &plan).await {
                            tracing::error!(?err, "error dispatching multicall results");
                        }
                    }
                    Err(err) => {
                        increment_counter(format!("{}|evm", self.venue));
                        tracing::error!(?err, "error when doing rpc calls");
                    }
                }

                true
            }
            Err(_) => {
                tracing::error!("latest block watch channel failed");
                false
            }
        }
    }

    pub async fn run(
        self,
        mut latest_block_rx: watch::Receiver<NewHead>,
        mut reconnect_rx: broadcast::Receiver<()>,
        mut pending_tx_rx: Receiver<PendingTx>,
    ) {
        loop {
            tokio::select! {
                // Reconnect signal received
                recv = reconnect_rx.recv() => {
                    if !Self::handle_reconnect(recv).await {
                        break;
                    }
                }

                // New head available
                changed = latest_block_rx.changed() => {
                    let latest_block_number = {
                        let latest_block = latest_block_rx.borrow_and_update();
                        latest_block.number
                    };

                    if !self.handle_latest_block_change(latest_block_number, changed).await {
                        break;
                    }
                }

                Some(tx) = pending_tx_rx.recv() => {
                    let provider = self.provider.clone();
                    let timeout = self.tx_timeout;
                    let poll_interval = self.tx_poll_interval;
                    let analytic_repo = self.analytic_repo.clone();
                    let _chain_id = self.chain_id;
                    tokio::spawn(async move {
                        let start = Instant::now();
                        let mut interval = interval(poll_interval);

                        loop {
                            interval.tick().await;
                            if start.elapsed() > timeout {
                                let _ = tx.result_sender.send(Err(anyhow::anyhow!("tx timeout")));
                                break;
                            }

                            match provider.get_transaction_receipt(tx.hash).await {
                                Ok(Some(receipt)) => {
                                    let error = if receipt.status() { None } else { Some("tx reverted".to_string()) };

                                    let mut transfers = vec![];
                                    for log in receipt.inner.logs() {
                                        if let Ok(transfer) = Transfer::decode_raw_log(log.topics(), &log.data().data) {
                                            transfers.push(json!({
                                                "from": transfer.from.to_string(),
                                                "to": transfer.to.to_string(),
                                                "value": transfer.value.to_string(),
                                                "token": log.address().to_string()
                                            }));
                                        }
                                    }

                                    let transfers_json = if transfers.is_empty() {
                                        None
                                    } else {
                                        Some(serde_json::json!({ "data": transfers }).to_string())
                                    };

                                    let record = EvmTxDb {
                                        created_at: Utc::now(),
                                        tx_hash: tx.hash.to_string(),
                                        block_number: receipt.block_number.unwrap_or_default(),
                                        nonce: tx.nonce,
                                        from: tx.from.to_string(),
                                        to: tx.to.map(|t| t.to_string()).unwrap_or_default(),
                                        value: tx.value.to_string(),
                                        gas_limit: tx.gas_limit,
                                        gas_price: Some(receipt.effective_gas_price.to_string()),
                                        max_fee_per_gas: tx.max_fee_per_gas.map(|v| v.to_string()),
                                        max_priority_fee_per_gas: tx.max_priority_fee_per_gas.map(|v| v.to_string()),
                                        data: hex::encode(&tx.data),
                                        error,
                                        transfers: transfers_json,
                                    };

                                    if let Some(repo) = &analytic_repo {
                                        let _ = repo.batch_insert(vec![Record::EvmTx(record)]).await;
                                    }

                                    if receipt.status() {
                                         let _ = tx.result_sender.send(Ok(()));
                                    } else {
                                         let _ = tx.result_sender.send(Err(anyhow::anyhow!("tx reverted")));
                                    }
                                    break;
                                }
                                Ok(None) => continue,
                                Err(_) => {
                                    continue;
                                }
                            }
                        }
                    });
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::{deploy_erc20_mock, deploy_multicall3};
    use alloy::{
        node_bindings::Anvil,
        primitives::B256,
        providers::{ext::AnvilApi, ProviderBuilder},
        signers::local::PrivateKeySigner,
    };
    use rengine_interfaces::db::{
        MockAnalyticRepository, MockEvmLogsRepository, MockMultiCallRepository,
    };
    use rengine_types::Timestamp;
    use rengine_utils::init_logging;
    use std::{sync::Arc, time::Duration};
    use tokio::sync::{broadcast, mpsc::channel, watch};

    /// Sanity test for the WebSocket client: it should stream `newHeads` from Anvil.
    #[tokio::test]
    async fn evm_ws_streams_new_heads_from_anvil() {
        // 1) Launch a local Anvil
        let anvil = Anvil::new().spawn();
        let ws_url = anvil.ws_endpoint();
        let http_url = anvil.endpoint();

        // 2) Wire up broadcast (reconnect) + watch (latest block)
        let (reconnect_tx, _reconnect_rx) = broadcast::channel::<()>(8);

        let init_head = NewHead {
            number: 0,
            timestamp: Timestamp::now(),
            hash: B256::ZERO,
        };
        let (latest_block_tx, mut latest_block_rx) = watch::channel(init_head);

        // 3) Start WS client
        let client = EvmWebsocketClient::new(
            reconnect_tx.clone(),
            latest_block_tx,
            Duration::from_secs(10),
        );
        let ws_task = tokio::spawn(async move {
            let _ = client.run(ws_url).await;
        });

        // 4) Give it a moment to subscribe
        tokio::time::sleep(Duration::from_millis(250)).await;

        // 5) Mine a few blocks to trigger newHeads
        let http = ProviderBuilder::new().connect_http(http_url.parse().unwrap());
        http.anvil_mine(Some(3), None).await.unwrap();

        // 6) Observe a head > 0
        let observed = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                latest_block_rx.changed().await.unwrap();
                let head = latest_block_rx.borrow().clone();
                if head.number > 0 {
                    break head;
                }
            }
        })
        .await
        .expect("timed out waiting for newHeads");

        assert!(observed.number >= 1, "expected mined block number >= 1");
        assert_ne!(observed.hash, B256::ZERO, "expected non-zero block hash");

        // 7) Cleanup
        ws_task.abort();
    }

    const TEST_WASM_READER_BYTES: &[u8] =
        include_bytes!("../../../evm_multicalls_wasm/test_multicall.cwasm");

    #[tokio::test]
    async fn evm_reader_handler_reacts_to_new_heads() {
        init_logging();
        // Anvil node
        let anvil = Anvil::new().spawn();
        let ws_url = anvil.ws_endpoint();
        let http_url = anvil.endpoint();

        let signer: PrivateKeySigner = anvil.keys()[0].clone().into();

        let http = ProviderBuilder::new()
            .wallet(signer)
            .connect_http(anvil.endpoint().parse().unwrap());

        let multicall_address = deploy_multicall3(&http).await.unwrap();
        let _erc20_mock_address = deploy_erc20_mock(&http).await.unwrap();

        // Minimal config; using Address::ZERO as the multicall address is fine —
        // eth_call to a non-contract returns empty bytes on Anvil.
        let cfg = EvmReaderConfig {
            ws_url,
            http_url,
            idle_timeout: Duration::from_secs(10),
            multicall_address,
            chain_id: 31337,
            tx_timeout: Duration::from_secs(60),
            tx_poll_interval: Duration::from_secs(1),
        };

        // State for WASM runtime (assumes Default)
        let state = Arc::new(RwLock::new(State::default()));

        let (tx, mut rx) = channel(10);

        let mut mock_repo = MockMultiCallRepository::default();
        mock_repo
            .expect_list_multicall()
            .withf(|venue| venue == "test")
            .returning(|_| Box::pin(async { Ok(vec![]) }));
        mock_repo
            .expect_add_multicall_reader()
            .withf(|_| true)
            .times(2)
            .returning(|_| Box::pin(async { Ok(()) }));
        let repo = Arc::new(mock_repo);
        let analytic_repo = Arc::new(MockAnalyticRepository::default());
        let mut mock_evm_logs_repo = MockEvmLogsRepository::default();
        mock_evm_logs_repo
            .expect_list_evm_logs()
            .returning(|_| Box::pin(async { Ok(vec![]) }));
        let evm_logs_repo = Arc::new(mock_evm_logs_repo);

        // Start the handler (spawns WS + reader loops)
        let handler = EvmReaderHandler::try_new(
            "test".into(),
            cfg,
            state,
            repo,
            evm_logs_repo,
            Some(analytic_repo),
            tx,
        )
        .await
        .expect("failed to init EvmReaderHandler");

        tokio::time::sleep(Duration::from_millis(20)).await;

        // Initial aggregate call set should be empty (no readers added)
        let (agg, _plan) = handler.reader.aggregate_multicalls_with_plan(0).await;

        assert!(agg.calls.is_empty(), "no multicall readers registered yet");

        handler
            .add_multicall_reader("test-reader".into(), TEST_WASM_READER_BYTES.to_vec())
            .await
            .unwrap();

        handler
            .add_multicall_reader("test-reader2".into(), TEST_WASM_READER_BYTES.to_vec())
            .await
            .unwrap();

        http.anvil_mine(None, None).await.unwrap();

        let result = rx.recv().await.unwrap();

        assert_eq!(result.len(), 2);

        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}
