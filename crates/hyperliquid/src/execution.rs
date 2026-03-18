use crate::{
    assets::ASSETS_TO_IDS,
    types::Tif,
    ws::{ExtraData, Payload, PostRequest, RequestType, WsHyperliquidMessage},
};
use alloy::{
    primitives::{address, keccak256, Address, B256, U256},
    signers::{local::PrivateKeySigner, Signer},
    sol,
    sol_types::{eip712_domain, Eip712Domain, SolStruct},
};
use anyhow::{anyhow, Result};
use async_trait::async_trait;
use chrono::Utc;
use const_hex::Buffer;
use rengine_interfaces::ExchangeExecution;
use rengine_types::{Mapping, OrderInfo, OrderReference, Side, Symbol, TimeInForce, Timestamp};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize, Serializer};
use std::{
    fmt,
    sync::atomic::{AtomicI64, AtomicU64, Ordering},
    time::{Duration, Instant},
};
use tokio::sync::mpsc::UnboundedSender;
use tracing::error;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct Limit {
    pub(crate) tif: Tif,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Trigger {
    pub(crate) is_market: bool,
    pub(crate) trigger_px: String,
    pub(crate) tpsl: String,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) enum Order {
    Limit(Limit),
    Trigger(Trigger),
}

#[derive(Deserialize, Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OrderRequest {
    #[serde(rename = "a", alias = "asset")]
    pub(crate) asset: u32,
    #[serde(rename = "b", alias = "isBuy")]
    pub(crate) is_buy: bool,
    #[serde(
        rename = "p",
        alias = "limitPx",
        serialize_with = "serialize_decimal_trimmed"
    )]
    pub(crate) limit_px: Decimal,
    #[serde(
        rename = "s",
        alias = "sz",
        serialize_with = "serialize_decimal_trimmed"
    )]
    pub(crate) sz: Decimal,
    #[serde(rename = "r", alias = "reduceOnly", default)]
    pub(crate) reduce_only: bool,
    #[serde(rename = "t", alias = "orderType")]
    pub(crate) order_type: Order,
    #[serde(rename = "c", alias = "cloid", skip_serializing_if = "Option::is_none")]
    pub(crate) cloid: Option<String>,
}

fn serialize_decimal_trimmed<S>(d: &Decimal, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    let trimmed = d.normalize().to_string();

    serializer.serialize_str(&trimmed)
}

fn asset_to_id(asset: &Symbol) -> Result<u32> {
    ASSETS_TO_IDS
        .get(asset)
        .copied()
        .ok_or_else(|| anyhow!("missing asset {asset}"))
}

impl OrderRequest {
    fn from_order_info(asset: Symbol, order: OrderInfo) -> Result<Self> {
        let id = asset_to_id(&asset)?;

        let is_buy = order.side == Side::Bid;

        let (reduce_only, tif) = match order.tif {
            TimeInForce::ReduceOnly => (true, Tif::Gtc),
            other => (false, other.into()),
        };

        let order = Self {
            asset: id,
            is_buy,
            limit_px: order.price,
            sz: order.size,
            reduce_only,
            order_type: Order::Limit(Limit { tif }),
            cloid: order.client_order_id.map(|c| c.to_string()),
        };

        Ok(order)
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BulkOrder {
    pub(crate) orders: Vec<OrderRequest>,
    pub(crate) grouping: String,
    // #[serde(default, skip_serializing_if = "Option::is_none")]
    // pub(crate) builder: Option<BuilderInfo>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub(crate) struct CancelRequest {
    #[serde(rename = "a", alias = "asset")]
    pub asset: u32,
    #[serde(rename = "o", alias = "oid")]
    pub oid: u64,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BulkCancel {
    pub cancels: Vec<CancelRequest>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub(crate) struct CancelByCloidRequest {
    pub asset: u32,
    pub cloid: String,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BulkCancelByCloid {
    pub cancels: Vec<CancelByCloidRequest>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "type")]
#[serde(rename_all = "camelCase")]
pub(crate) enum Actions {
    // UsdSend(UsdSend),
    // UpdateLeverage(UpdateLeverage),
    // UpdateIsolatedMargin(UpdateIsolatedMargin),
    Order(BulkOrder),
    Cancel(BulkCancel),
    #[serde(rename = "cancelByCloid")]
    CancelByCloid(BulkCancelByCloid),
    // BatchModify(BulkModify),
    // ApproveAgent(ApproveAgent),
    // Withdraw3(Withdraw3),
    // SpotUser(SpotUser),
    // VaultTransfer(VaultTransfer),
    // SpotSend(SpotSend),
    // SetReferrer(SetReferrer),
    // ApproveBuilderFee(ApproveBuilderFee),
}

impl Actions {
    fn hash(&self, timestamp: u64, _: Option<Address>) -> Result<B256> {
        let mut bytes = rmp_serde::to_vec_named(self)?;
        bytes.extend(timestamp.to_be_bytes());
        bytes.push(0);

        Ok(keccak256(bytes))
    }
}

sol! {
    #[derive(Debug, serde::Serialize)]
    struct Agent {
        string source;
        bytes32 connectionId;
    }
}

#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct Signature {
    pub r: U256,
    pub s: U256,
    pub v: u64,
}

impl From<&Signature> for [u8; 65] {
    fn from(src: &Signature) -> [u8; 65] {
        let mut sig = [0u8; 65];
        sig[..32].copy_from_slice(&src.r.to_be_bytes::<32>());
        sig[32..64].copy_from_slice(&src.s.to_be_bytes::<32>());
        // TODO: What if we try to serialize a signature where
        // the `v` is not normalized?

        // The u64 to u8 cast is safe because `sig.v` can only ever be 27 or 28
        // here. Regarding EIP-155, the modification to `v` happens during tx
        // creation only _after_ the transaction is signed using
        // `ethers_signers::to_eip155_v`.

        sig[64] = src.v as u8;
        sig
    }
}

impl fmt::Display for Signature {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(Buffer::<65, false>::new().format(&self.into()))
    }
}

pub(crate) async fn sign_l1_action(
    wallet: &PrivateKeySigner,
    connection_id: B256,
) -> Result<Signature> {
    let source = "a".to_string();

    let payload = Agent {
        source,
        connectionId: connection_id,
    };

    let domain = eip712_domain! {
        name: "Exchange",
        version: "1",
        chain_id: 1337,
        verifying_contract: address!("0x0000000000000000000000000000000000000000"),
    };

    sign_typed_data(&payload, wallet, &domain).await
}

async fn sign_typed_data<T: SolStruct>(
    payload: &T,
    wallet: &PrivateKeySigner,
    domain: &Eip712Domain,
) -> Result<Signature> {
    let hash = payload.eip712_signing_hash(domain);

    sign_hash(hash, wallet).await
}

async fn sign_hash(hash: B256, wallet: &PrivateKeySigner) -> Result<Signature> {
    let sig = wallet.sign_hash(&hash).await?;

    let r = U256::from_be_bytes(sig.r().to_be_bytes::<32>());
    let s = U256::from_be_bytes(sig.s().to_be_bytes::<32>());
    let v = if sig.v() { 28 } else { 27 };

    let signature = Signature { r, s, v };

    Ok(signature)
}

pub struct HyperLiquidPerp {
    key: PrivateKeySigner,
    outcoming_message_tx: UnboundedSender<(WsHyperliquidMessage, Option<ExtraData>)>,
    ws_id: AtomicU64,
    nonce_gen: NonceGen,
    pub instrument_mapping: Mapping,
    max_response_duration: Duration,
}

impl HyperLiquidPerp {
    pub(crate) fn new(
        key: PrivateKeySigner,
        instrument_mapping: Mapping,
        outcoming_message_tx: UnboundedSender<(WsHyperliquidMessage, Option<ExtraData>)>,
        max_response_duration: Duration,
    ) -> Self {
        Self {
            key,
            outcoming_message_tx,
            ws_id: Default::default(),
            nonce_gen: Default::default(),
            instrument_mapping,
            max_response_duration,
        }
    }

    async fn send_action(&self, action: Actions, extra_data: Option<ExtraData>) -> Result<()> {
        let now = Utc::now().timestamp_millis();
        let nonce = self.nonce_gen.nonce(now) as u64;

        let connection_id = action.hash(nonce, None)?;
        let signature = sign_l1_action(&self.key, connection_id).await?;

        let payload = Payload {
            action,
            nonce,
            signature,
        };
        let request = PostRequest {
            request_type: RequestType::Action,
            payload,
        };
        // Unique ws message id
        let id = self.ws_id.fetch_add(1, Ordering::Relaxed);
        let ws_message = WsHyperliquidMessage::Post { id, request };

        if let Err(err) = self.outcoming_message_tx.send((ws_message, extra_data)) {
            error!(?err, "[hyperliquid] failure sending message");
        }

        Ok(())
    }
}

#[async_trait]
impl ExchangeExecution for HyperLiquidPerp {
    async fn post_orders(&self, orders: Vec<(Symbol, OrderInfo)>) -> Result<()> {
        let orders_cloned = orders.clone();
        let requests: Result<Vec<_>> = orders
            .into_iter()
            .map(|(asset, order)| OrderRequest::from_order_info(asset, order))
            .collect();

        let action = Actions::Order(BulkOrder {
            orders: requests?,
            grouping: "na".to_string(),
        });
        let extra_data = ExtraData::Orders(Timestamp::now(), orders_cloned, Instant::now());

        self.send_action(action, Some(extra_data)).await
    }

    async fn cancel_orders(&self, cancels: Vec<(Symbol, OrderReference)>) -> Result<()> {
        let mut oid_cancels = Vec::new();
        let mut cloid_cancels = Vec::new();

        for (asset, reference) in cancels {
            match reference {
                OrderReference::ExternalOrderId(oid) => oid_cancels.push((asset, oid)),
                OrderReference::ClientOrderId(cloid) => cloid_cancels.push((asset, cloid)),
            }
        }

        if !oid_cancels.is_empty() {
            let cancels_cloned: Vec<(Symbol, OrderReference)> = oid_cancels
                .iter()
                .map(|(a, o)| (a.clone(), OrderReference::ExternalOrderId(o.clone())))
                .collect();

            let requests: Result<Vec<_>> = oid_cancels
                .into_iter()
                .map(|(asset, oid)| {
                    Ok(CancelRequest {
                        asset: asset_to_id(&asset)?,
                        oid: oid.parse()?,
                    })
                })
                .collect();

            let action = Actions::Cancel(BulkCancel { cancels: requests? });
            let extra_data = ExtraData::Cancel(Timestamp::now(), cancels_cloned, Instant::now());
            self.send_action(action, Some(extra_data)).await?;
        }

        if !cloid_cancels.is_empty() {
            let cancels_cloned: Vec<(Symbol, OrderReference)> = cloid_cancels
                .iter()
                .map(|(a, c)| (a.clone(), OrderReference::ClientOrderId(c.clone())))
                .collect();

            let requests: Result<Vec<_>> = cloid_cancels
                .into_iter()
                .map(|(asset, cloid)| {
                    Ok(CancelByCloidRequest {
                        asset: asset_to_id(&asset)?,
                        cloid: cloid.to_string(),
                    })
                })
                .collect();

            let action = Actions::CancelByCloid(BulkCancelByCloid { cancels: requests? });
            let extra_data = ExtraData::Cancel(Timestamp::now(), cancels_cloned, Instant::now());
            self.send_action(action, Some(extra_data)).await?;
        }

        Ok(())
    }

    fn max_response_duration(&self) -> Duration {
        self.max_response_duration
    }
}

#[derive(Debug, Default)]
pub(crate) struct NonceGen {
    last: AtomicI64,
}

impl NonceGen {
    /// Generate a monotonic unique nonce based on the given timestamp in milliseconds.
    ///
    /// - If `now_ms` is greater than the last nonce, it returns `now_ms`.
    /// - Otherwise, it increments from the last known nonce to ensure monotonicity.
    pub(crate) fn nonce(&self, now_ms: i64) -> i64 {
        let mut prev = self.last.load(Ordering::Relaxed);
        loop {
            let next = if now_ms > prev { now_ms } else { prev + 1 };
            match self
                .last
                .compare_exchange(prev, next, Ordering::Relaxed, Ordering::Relaxed)
            {
                Ok(_) => return next,
                Err(p) => prev = p,
            }
        }
    }
}

#[cfg(test)]
mod test {
    use crate::{
        execution::{sign_l1_action, Actions, BulkOrder, Limit, Order, OrderRequest, Tif},
        ws::{Payload, PostRequest, RequestType, WsHyperliquidMessage},
    };
    use alloy::{primitives::B256, signers::local::PrivateKeySigner};
    use anyhow::Result;
    use chrono::Utc;
    use rust_decimal_macros::dec;
    use std::str::FromStr;

    fn get_wallet() -> Result<PrivateKeySigner> {
        "e908f86dbb4d55ac876378565aafeabc187f6690f046459397b17d9b9a19688e"
            .parse()
            .map_err(Into::into)
    }

    fn get_hotwallet() -> Result<PrivateKeySigner> {
        todo!();
    }

    #[tokio::test]
    async fn test_sign_l1_action() -> Result<()> {
        let wallet = get_wallet()?;
        let connection_id =
            B256::from_str("0xde6c4037798a4434ca03cd05f00e3b803126221375cd1e7eaaaf041768be06eb")?;

        let expected_mainnet_sig = "fa8a41f6a3fa728206df80801a83bcbfbab08649cd34d9c0bfba7c7b2f99340f53a00226604567b98a1492803190d65a201d6805e5831b7044f17fd530aec7841c";

        let signature_to_string = sign_l1_action(&wallet, connection_id).await?.to_string();

        assert_eq!(signature_to_string, expected_mainnet_sig);

        Ok(())
    }

    #[ignore]
    #[tokio::test]
    async fn test_order_serialize_and_signature() -> Result<()> {
        let wallet = get_hotwallet()?;

        let order = OrderRequest {
            asset: 1,
            is_buy: true,
            limit_px: dec!(1800),
            sz: dec!(0.0061),
            reduce_only: false,
            order_type: Order::Limit(Limit { tif: Tif::Gtc }),
            cloid: None,
        };

        let order_str = serde_json::to_string(&order).unwrap();
        println!("{order_str}");

        let action = Actions::Order(BulkOrder {
            orders: vec![order],
            grouping: "na".to_string(),
        });

        let action_str = serde_json::to_string(&action).unwrap();
        println!("{action_str}");

        let nonce = Utc::now().timestamp_millis() as u64;
        let connection_id = action.hash(nonce, None)?;

        let signature = sign_l1_action(&wallet, connection_id).await?;

        let payload = Payload {
            action,
            nonce,
            signature,
        };

        let request = PostRequest {
            request_type: RequestType::Action,
            payload,
        };
        let ws_message = WsHyperliquidMessage::Post { id: 1, request };
        let ws_message_str = serde_json::to_string(&ws_message).unwrap();

        println!("{ws_message_str}");

        Ok(())
    }
}
