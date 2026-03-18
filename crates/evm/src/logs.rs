use crate::ws::EthWsMessage;
use alloy::{hex, rpc::types::Log};
use anyhow::Result;
use evm_types::LogSubscription;
use frunk::{hlist, HCons, HNil};
use frunk_ws::{
    engine::{bind_stream, run_ws_loop},
    handler::to_handler,
    handlers::logging::{check_last_msg_timeout, update_last_msg, LastMsg},
    types::{ConnectHandler, ContextState, HandlerOutcome, WsStream},
};
use futures::{future::BoxFuture, FutureExt, SinkExt};
use rengine_types::Action;
use serde_json::json;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message as WsMessage;
use wasm_runtime::{evm_logs::EvmLogsRuntime, EvmLogs, Runtime};

pub struct LogReader {
    pub reader: EvmLogs,
    pub runtime: Runtime,
    pub subscription: LogSubscription,
    pub state: Vec<u8>,
}

pub struct LogReaderState {
    pub log_reader: LogReader,
    pub action_tx: mpsc::Sender<Vec<Action>>,
}

pub type WsState = HCons<LogReaderState, HCons<LastMsg, HCons<ContextState, HNil>>>;

impl LogReader {
    pub fn new(reader: EvmLogs, mut runtime: Runtime) -> Result<Self> {
        let (state, subscription_bytes) = runtime.execute_evm_logs_init(&reader)?;
        let subscription: LogSubscription = borsh::from_slice(&subscription_bytes)?;
        Ok(Self {
            reader,
            runtime,
            subscription,
            state,
        })
    }

    pub fn handle_log(&mut self, log: &Log) -> Result<Vec<Action>> {
        let log_bytes = serde_json::to_vec(log)?;
        let (new_state, action_bytes) =
            self.runtime
                .execute_evm_logs_handle(&self.reader, &self.state, &log_bytes)?;
        self.state = new_state;
        let actions: Vec<Action> = borsh::from_slice(&action_bytes)?;
        Ok(actions)
    }
}

fn subscribe_handler<'a>(ws: &'a mut WsStream, state: &'a mut WsState) -> BoxFuture<'a, ()> {
    async move {
        let log_state: &mut LogReaderState = state.get_mut();
        let address_str = format!(
            "0x{}",
            hex::encode(log_state.log_reader.subscription.address)
        );
        let topics_json: Vec<String> = log_state
            .log_reader
            .subscription
            .topics
            .iter()
            .map(|t| format!("0x{}", hex::encode(t)))
            .collect();

        let params = json!([
            "logs",
            {
                "address": address_str,
                "topics": topics_json
            }
        ]);

        let request = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "eth_subscribe",
            "params": params
        });

        if let Err(e) = ws.send(WsMessage::Text(request.to_string())).await {
            tracing::error!("Failed to subscribe: {:?}", e);
        }
    }
    .boxed()
}

fn log_handler<'a>(
    _ws: &'a mut WsStream,
    state: &'a mut WsState,
    msg: &'a WsMessage,
) -> BoxFuture<'a, Result<HandlerOutcome>> {
    async move {
        if let WsMessage::Text(text) = msg {
            if let Ok(EthWsMessage::Subscription { params }) =
                serde_json::from_str::<EthWsMessage<Log>>(text)
            {
                let log = params.result;
                let log_state: &mut LogReaderState = state.get_mut();
                match log_state.log_reader.handle_log(&log) {
                    Ok(actions) => {
                        if !actions.is_empty() {
                            if let Err(e) = log_state.action_tx.send(actions).await {
                                tracing::error!("Failed to send actions: {:?}", e);
                                return Ok(HandlerOutcome::Stop);
                            }
                        }
                    }
                    Err(e) => tracing::error!("Error handling log: {:?}", e),
                }
            }
        }
        Ok(HandlerOutcome::Continue)
    }
    .boxed()
}

pub async fn run_log_reader(
    url: String,
    log_reader: LogReader,
    action_tx: mpsc::Sender<Vec<Action>>,
) -> Result<()> {
    let state = hlist![
        LogReaderState {
            log_reader,
            action_tx
        },
        LastMsg::default(),
        ContextState::new("EvmLogs")
    ];

    let on_connect: Vec<ConnectHandler<WsState>> = vec![Box::new(subscribe_handler)];

    let handlers = hlist![to_handler(log_handler), to_handler(update_last_msg)];

    let log_state: &LogReaderState = state.get();
    let timeout = log_state
        .log_reader
        .subscription
        .timeout
        .unwrap_or(Duration::from_secs(600));

    let watchdog_stream =
        tokio_stream::wrappers::IntervalStream::new(tokio::time::interval(Duration::from_secs(5)));
    let action_watchdog = bind_stream(watchdog_stream, move |ws, state: &mut WsState, _| {
        check_last_msg_timeout(ws, state, timeout)
    });

    run_ws_loop(url, state, on_connect, handlers, vec![action_watchdog]).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::{deploy_erc20_mock, deploy_multicall3};
    use alloy::{
        node_bindings::Anvil,
        primitives::{address, Address, U256},
        providers::{Provider, ProviderBuilder},
        signers::local::PrivateKeySigner,
        sol_types::SolCall,
    };
    use evm_types::erc20::ERC20Mock;
    use rengine_types::State;
    use std::{sync::Arc, time::Duration};

    const TEST_LOGS_WASM: &[u8] = include_bytes!("../../../evm_logs_wasm/test_logs.cwasm");
    const EXPECTED_ADDRESS: Address = address!("e7f1725E7734CE288F8367e1Bb143E90bb3F0512");

    #[tokio::test]
    async fn test_log_reader_integration() {
        // 1. Setup Anvil
        let anvil = Anvil::new().spawn();
        let ws_url = anvil.ws_endpoint();
        let http_url = anvil.endpoint();

        let signer: PrivateKeySigner = anvil.keys()[0].clone().into();
        let provider = ProviderBuilder::new()
            .wallet(signer)
            .connect_http(http_url.parse().unwrap());

        // 2. Deploy contracts to match expected address
        // Nonce 0: Multicall3
        let _ = deploy_multicall3(&provider).await.unwrap();
        // Nonce 1: MockERC20
        let erc20_addr = deploy_erc20_mock(&provider).await.unwrap();

        assert_eq!(
            erc20_addr, EXPECTED_ADDRESS,
            "Deployed address must match the one hardcoded in WASM"
        );

        // 3. Setup Runtime and LogReader
        // State uses parking_lot::RwLock (from wasm_runtime)
        let state = Arc::new(wasm_runtime::RwLock::new(State::default()));
        // Runtime uses tokio::sync::RwLock (local import)
        let mut runtime = Runtime::new(state).unwrap();
        let evm_logs = runtime.instantiate_evm_logs(TEST_LOGS_WASM).unwrap();

        let log_reader = LogReader::new(evm_logs, runtime).unwrap();

        // 4. Run LogReader
        let (tx, mut rx) = mpsc::channel(10);
        let handle = tokio::spawn(run_log_reader(ws_url, log_reader, tx));

        // Give it time to subscribe
        tokio::time::sleep(Duration::from_millis(500)).await;

        // 5. Trigger Event
        // Transfer to 0x02... to trigger the logic in WASM
        let target = Address::from([0x02; 20]);
        let amount = U256::from(1337);

        let call = ERC20Mock::transferCall { to: target, amount };

        let _ = provider
            .send_transaction(
                alloy::rpc::types::TransactionRequest::default()
                    .to(erc20_addr)
                    .input(call.abi_encode().into()),
            )
            .await
            .unwrap()
            .get_receipt()
            .await
            .unwrap();

        // 6. Assert Action
        let actions = tokio::time::timeout(Duration::from_secs(5), rx.recv())
            .await
            .expect("Timed out waiting for log action")
            .expect("Channel closed");

        assert!(!actions.is_empty());
        match &actions[0] {
            Action::SetIndicator(key, value) => {
                assert_eq!(key, "cumulative_transfer_volume");
                assert!(value.is_sign_positive());
            }
            _ => panic!("Unexpected action"),
        }

        handle.abort();
    }
}
