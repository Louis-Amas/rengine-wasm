use anyhow::Result;
use rengine_types::{
    Account, ExecutionRequest, ExecutionType, Instrument, OrderActions, OrderInfo, Side,
    StrategyConfiguration, TimeInForce, Venue,
};
use strategy_api::{bindings::export, get_balance, get_indicator, impl_guest_from_plugin, Plugin};

struct MyPlugin;

#[derive(Default, borsh::BorshSerialize, borsh::BorshDeserialize)]
pub struct State {
    pub counter: u64,
}

impl Plugin for MyPlugin {
    type State = State;

    fn init() -> StrategyConfiguration {
        StrategyConfiguration {
            triggers_keys: <_>::default(),
            cooldown: None,
        }
    }

    fn execute(mut state: Self::State) -> Result<(Self::State, Vec<ExecutionRequest>), String> {
        state.counter += 1;
        let indicator = get_indicator("test")?;

        let venue: Venue = "venue".into();
        let account = Account {
            venue,
            account_id: "test".into(),
            market_type: rengine_types::MarketType::Spot,
        };
        let instrument: Instrument = "eth".into();
        let balance = get_balance("venue|spot|account|eth").unwrap();
        let order = OrderInfo {
            side: Side::Ask,
            size: balance,
            price: indicator,
            tif: TimeInForce::PostOnly,
            client_order_id: Some(state.counter.to_string().into()),
            order_type: Default::default(),
        };
        let order_action =
            OrderActions::BulkPost((account, vec![(instrument, order)], ExecutionType::Managed));

        Ok((state, vec![ExecutionRequest::Orderbook(order_action)]))
    }
}

impl_guest_from_plugin!(MyPlugin, "test_strategy_state");

export!(MyPlugin with_types_in strategy_api::bindings);
