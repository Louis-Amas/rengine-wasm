use rengine_types::{Action, Level, TopBookUpdate, VenueBookKey};
use rust_decimal::Decimal;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};
use tokio::sync::{
    mpsc::{self, error::TrySendError, UnboundedReceiver, UnboundedSender},
    watch,
};
use tracing::error;

pub type ChangesTx = mpsc::Sender<Vec<Action>>;

pub fn send_changes(tx: &ChangesTx, actions: Vec<Action>) {
    if let Err(TrySendError::Full(_msg)) = tx.try_send(actions) {
        error!("couldn't send action channel full");
    }
}

pub struct TopBookRegistry {
    senders: Mutex<HashMap<VenueBookKey, watch::Sender<TopBookUpdate>>>,
    register_tx: UnboundedSender<(VenueBookKey, watch::Receiver<TopBookUpdate>)>,
}

impl TopBookRegistry {
    pub fn new() -> (
        Arc<Self>,
        UnboundedReceiver<(VenueBookKey, watch::Receiver<TopBookUpdate>)>,
    ) {
        let (register_tx, register_rx) = mpsc::unbounded_channel();
        (
            Arc::new(Self {
                senders: Mutex::new(HashMap::new()),
                register_tx,
            }),
            register_rx,
        )
    }

    pub fn get_sender(&self, key: VenueBookKey) -> watch::Sender<TopBookUpdate> {
        let mut senders = self.senders.lock().unwrap();
        if let Some(sender) = senders.get(&key) {
            sender.clone()
        } else {
            let default_update = TopBookUpdate {
                top_bid: Level {
                    price: Decimal::ZERO,
                    size: Decimal::ZERO,
                },
                top_ask: Level {
                    price: Decimal::ZERO,
                    size: Decimal::ZERO,
                },
            };
            let (tx, rx) = watch::channel(default_update);
            let _ = self.register_tx.send((key.clone(), rx));
            senders.insert(key, tx.clone());
            tx
        }
    }
}
