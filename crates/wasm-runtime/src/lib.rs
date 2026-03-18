pub mod evm_logs;
pub mod multicall;
pub mod runtime;
pub mod strategy;
#[cfg(test)]
mod tests;
mod types;

pub use crate::{evm_logs::EvmLogs, multicall::Multicall, runtime::Runtime};
pub use parking_lot::RwLock;

#[cfg(test)]
mod test {
    use super::Runtime;
    use crate::{multicall::MulticallRuntime, strategy::StrategyRuntime};
    use alloy::hex::FromHex;
    use parking_lot::RwLock;
    use rengine_types::{Action, ExecutionRequest, OrderActions, Side, State};
    use rust_decimal_macros::dec;
    use std::sync::Arc;

    const STRATEGY_BYTES: &[u8] = include_bytes!("../../../strategies-wasm/simple_strategy.cwasm");

    const TEST_STRATEGY_BYTES: &[u8] =
        include_bytes!("../../../strategies-wasm/test_strategy_state.cwasm");

    #[test]
    fn wasm_strategy() {
        let state: Arc<RwLock<State>> = Default::default();
        let mut runtime = Runtime::new(state).unwrap();

        let strategy = runtime.instantiate_strategy(STRATEGY_BYTES).unwrap();
        runtime.execute(&strategy, &[], None).unwrap();
    }

    #[test]
    fn wasm_strategy_state() {
        let state: Arc<RwLock<State>> = Default::default();
        let mut runtime = Runtime::new(state.clone()).unwrap();
        let strategy = runtime.instantiate_strategy(TEST_STRATEGY_BYTES).unwrap();

        state.write().indicators.insert("test".into(), dec!(1));
        state
            .write()
            .balances
            .insert("venue|spot|account|eth".parse().unwrap(), dec!(15));

        let ExecutionRequest::Orderbook(OrderActions::BulkPost((venue, orders, _))) = runtime
            .execute(&strategy, &[], None)
            .unwrap()
            .1
            .requests
            .into_iter()
            .next()
            .unwrap()
        else {
            panic!("wrong result");
        };

        assert_eq!("venue|spot|test", venue.to_string());

        let (symbol, order) = orders.into_iter().next().unwrap();

        assert_eq!("eth", symbol.to_string());
        assert_eq!(Side::Ask, order.side);
        assert_eq!(dec!(1), order.price);
        assert_eq!(dec!(15), order.size);
    }

    const TEST_MULTICALL_BYTES: &[u8] =
        include_bytes!("../../../evm_multicalls_wasm/test_multicall.cwasm");

    #[test]
    fn wasm_multicall_test() {
        let state: Arc<RwLock<State>> = Default::default();
        let mut runtime = Runtime::new(state.clone()).unwrap();

        let multicall = runtime
            .instantiate_multicall_reader(TEST_MULTICALL_BYTES)
            .unwrap();

        let config = runtime.execute_multicall_reader_config(&multicall).unwrap();

        assert_eq!(config.every_x_block, 1);

        let calls = runtime
            .execute_multicall_reader_requests(&multicall)
            .unwrap();

        assert_eq!(calls.len(), 2);

        use evm_types::Result as MulticallResult;

        let results = vec![MulticallResult {
            success: true,
            returnData: alloy::primitives::Bytes::from_hex(
                "00000000000000000000000000000000000000000000003635c9adc5dea00000",
            )
            .unwrap(),
        }];

        let result = runtime
            .execute_multicall_reader_handle(&multicall, &[], &results)
            .unwrap();

        assert_eq!(result.1.len(), 1);

        let Action::SetIndicator(key, value) = result.1.into_iter().next().unwrap() else {
            panic!("not set indicator");
        };

        assert_eq!(key, "test");
        assert_eq!(value, dec!(1000));
    }
}
