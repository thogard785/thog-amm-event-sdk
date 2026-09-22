//! Subscription-only ThogAMM updates. Startup takes one snapshot; the running
//! client owns no RPC reader. Proposed logs become an atomic model when Monad's
//! ordered commitment stream finalizes their block.
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::collections::{btree_map::Entry, BTreeMap, VecDeque};
pub use thogamm_model::{self as model, Address, Error, ExecutionContext, PoolModel, Result, U256};
use thogamm_model::{
    events::{apply_owned_block, log_filters},
    rpc::{snapshot, CallBlock, ChainReader, HttpRpc},
    BlockHeader, RpcLog, B256,
};
use tokio::net::TcpStream;
use tokio_tungstenite::{connect_async, tungstenite::Message, MaybeTlsStream, WebSocketStream};

type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;
#[derive(Clone, Debug)]
pub struct Config {
    /// Immutable legacy registry token 3; must match the seed image.
    pub wmon: Address,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            wmon: alloy_primitives::address!("3bd359c1119da7da1d913d1c4d2b7c461115433a"),
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommitState {
    Proposed,
    Voted,
    Finalized,
    Verified,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Update {
    pub block: BlockHeader,
    pub blocks: u64,
}
struct PendingBlock {
    header: BlockHeader,
    logs: BTreeMap<(u64, u64), RpcLog>,
    complete: bool,
}

/// An RPC-free reducer for one ordered Monad connection: four `logs` filters
/// plus `monadNewHeads`. A subsequent commitment header closes the prior log
/// frame. Only finalized ancestry is published; abandoned proposals are discarded.
/// Feed notifications in socket order, including Voted/Verified heads.
///
/// The source must use Monad's header-before-logs, whole-block serialization
/// (monad-rpc/src/websocket/handler.rs::handle_notification). Generic Ethereum
/// newHeads/logs streams do not provide this completion boundary.
pub struct EventSynchronizer {
    model: PoolModel,
    pending: BTreeMap<B256, PendingBlock>,
    active: Option<B256>,
    failed: bool,
}
impl EventSynchronizer {
    /// The caller supplies a finalized seed and a continuous subscription stream
    /// covering its successors. This constructor performs no I/O.
    pub fn from_model(model: PoolModel) -> Result<Self> {
        if model.state().block.hash.is_none() {
            return Err(Error::InvalidData(
                "event seed requires a known finalized block hash".into(),
            ));
        }
        model.state().validate()?;
        Ok(Self {
            model,
            pending: BTreeMap::new(),
            active: None,
            failed: false,
        })
    }
    pub fn model(&self) -> &PoolModel {
        &self.model
    }
    pub fn on_log(&mut self, log: RpcLog) -> Result<()> {
        if self.failed {
            return Err(Error::Discontinuous);
        }
        let result = self.apply_log(log);
        if result.is_err() {
            self.failed = true;
        }
        result
    }
    fn apply_log(&mut self, log: RpcLog) -> Result<()> {
        if log.removed {
            return Err(Error::Discontinuous);
        }
        if log.block_number <= self.model.state().block.number {
            return Ok(());
        }
        if self.active != Some(log.block_hash) {
            return Err(Error::Discontinuous);
        }
        let block = self
            .pending
            .get_mut(&log.block_hash)
            .ok_or(Error::Discontinuous)?;
        if block.complete || block.header.number != log.block_number {
            return Err(Error::Discontinuous);
        }
        let position = (log.transaction_index, log.log_index);
        match block.logs.entry(position) {
            Entry::Occupied(previous) if previous.get() != &log => {
                return Err(Error::InvalidData("conflicting subscription logs".into()));
            }
            Entry::Occupied(_) => {}
            Entry::Vacant(entry) => {
                entry.insert(log);
            }
        }
        Ok(())
    }
    pub fn on_head(
        &mut self,
        header: BlockHeader,
        commitment: CommitState,
    ) -> Result<Option<Update>> {
        if self.failed {
            return Err(Error::Discontinuous);
        }
        let result = self.apply_head(header, commitment);
        if result.is_err() {
            self.failed = true;
        }
        result
    }
    fn apply_head(
        &mut self,
        header: BlockHeader,
        commitment: CommitState,
    ) -> Result<Option<Update>> {
        // Monad serializes every prior block's complete set of log filters before
        // starting this header notification on the same WebSocket connection.
        if let Some(hash) = self.active.take() {
            self.pending
                .get_mut(&hash)
                .ok_or(Error::Discontinuous)?
                .complete = true;
        }
        let current = &self.model.state().block;
        if header.number <= current.number {
            if commitment == CommitState::Finalized
                && header.number == current.number
                && Some(header.hash) != current.hash
            {
                return Err(Error::Discontinuous);
            }
            return Ok(None);
        }
        if commitment == CommitState::Proposed {
            self.active = Some(header.hash);
            let block = self
                .pending
                .entry(header.hash)
                .or_insert_with(|| PendingBlock {
                    header: header.clone(),
                    logs: BTreeMap::new(),
                    complete: false,
                });
            if block.header != header {
                return Err(Error::InvalidData("conflicting proposal header".into()));
            }
            // A repeated frame retains its logs; position-based deduplication
            // prevents replaying transfer deltas twice.
            block.complete = false;
            return Ok(None);
        }
        if commitment != CommitState::Finalized {
            return Ok(None);
        }
        let mut path = Vec::new();
        let mut hash = header.hash;
        let mut number = header.number;
        while number > current.number {
            let block = self.pending.get(&hash).ok_or(Error::Discontinuous)?;
            if !block.complete || block.header.number != number {
                return Err(Error::Discontinuous);
            }
            if number == header.number && block.header != header {
                return Err(Error::Discontinuous);
            }
            path.push(hash);
            hash = block.header.parent_hash;
            number -= 1;
        }
        if Some(hash) != current.hash {
            return Err(Error::Discontinuous);
        }
        let mut next = self.model.state().clone();
        for hash in path.iter().rev() {
            let block = &self.pending[hash];
            next = apply_owned_block(next, block.header.clone(), block.logs.values())?;
        }
        let update = Update {
            blocks: header.number - current.number,
            block: header.clone(),
        };
        self.model = PoolModel::new(next)?;
        self.pending
            .retain(|_, block| block.header.number > header.number);
        Ok(Some(update))
    }
}

#[derive(Clone, Copy)]
enum Subscription {
    Logs,
    Heads,
}
type Subscriptions = BTreeMap<String, Subscription>;

/// After construction this type holds only a WebSocket, subscription messages,
/// and local state. There is no HTTP client or path that can issue a read RPC.
pub struct EventDrivenSdk {
    synchronizer: EventSynchronizer,
    socket: Socket,
    subscriptions: Subscriptions,
    queued: VecDeque<Value>,
}
impl EventDrivenSdk {
    /// Creates subscriptions first, waits for an observed proposal to finalize,
    /// then makes exactly one hash-pinned getPoolData(0,64) call to seed the model.
    pub async fn connect(
        http_url: impl Into<String>,
        ws_url: impl AsRef<str>,
        proxy: Address,
        config: Config,
    ) -> Result<Self> {
        Self::with_reader(HttpRpc::new(http_url)?, ws_url, proxy, config).await
    }
    /// The reader is used once during initialization and is not retained.
    pub async fn with_reader<R: ChainReader>(
        reader: R,
        ws_url: impl AsRef<str>,
        proxy: Address,
        config: Config,
    ) -> Result<Self> {
        let (mut socket, subscriptions, mut queued) = tokio::time::timeout(
            std::time::Duration::from_secs(15),
            subscribe(ws_url.as_ref(), proxy, config.wmon),
        )
        .await
        .map_err(|_| Error::Transport("subscription handshake timed out".into()))??;
        let seed_header = tokio::time::timeout(std::time::Duration::from_secs(15), async {
            let mut min_seed = None;
            let mut cursor = 0;
            loop {
                if cursor == queued.len() {
                    queued.push_back(read_json(&mut socket).await?);
                }
                let value = &queued[cursor];
                cursor += 1;
                if let Some((Subscription::Heads, payload)) = notification(value, &subscriptions)? {
                    let (header, commitment) = parse_head(payload)?;
                    if commitment == CommitState::Proposed && min_seed.is_none() {
                        min_seed = Some(header.number);
                    }
                    if commitment == CommitState::Finalized
                        && min_seed.is_some_and(|n| header.number >= n)
                    {
                        return Ok::<_, Error>(header);
                    }
                }
            }
        })
        .await
        .map_err(|_| Error::Transport("no finalized subscription checkpoint received".into()))??;
        let model = snapshot(&reader, proxy, CallBlock::Hash(seed_header.hash)).await?;
        if model.state().block != seed_header.into() || model.state().tokens[3].token != config.wmon
        {
            return Err(Error::InvalidData(
                "seed disagrees with subscription context or WMON configuration".into(),
            ));
        }
        Ok(Self {
            synchronizer: EventSynchronizer::from_model(model)?,
            socket,
            subscriptions,
            queued,
        })
    }
    pub fn model(&self) -> &PoolModel {
        self.synchronizer.model()
    }
    /// Reads pushed messages only. A lost connection, missing block, incompatible
    /// upgrade or reducer error is reported; none triggers a hidden snapshot/retry.
    pub async fn next_update(&mut self) -> Result<Update> {
        if self.synchronizer.failed {
            return Err(Error::Discontinuous);
        }
        let result = self.read_update().await;
        if result.is_err() {
            self.synchronizer.failed = true;
        }
        result
    }
    async fn read_update(&mut self) -> Result<Update> {
        loop {
            let value = match self.queued.pop_front() {
                Some(value) => value,
                None => read_json(&mut self.socket).await?,
            };
            if let Some((kind, payload)) = notification(&value, &self.subscriptions)? {
                match kind {
                    Subscription::Logs => self.synchronizer.on_log(
                        serde_json::from_value(payload.clone())
                            .map_err(|e| Error::Rpc(e.to_string()))?,
                    )?,
                    Subscription::Heads => {
                        let (header, commitment) = parse_head(payload)?;
                        if let Some(update) = self.synchronizer.on_head(header, commitment)? {
                            return Ok(update);
                        }
                    }
                }
            }
        }
    }
}
fn parse_head(payload: &Value) -> Result<(BlockHeader, CommitState)> {
    let header = serde_json::from_value(payload.clone()).map_err(|e| Error::Rpc(e.to_string()))?;
    let commitment = match payload["commitState"].as_str() {
        Some("Proposed") => CommitState::Proposed,
        Some("Voted") => CommitState::Voted,
        Some("Finalized") => CommitState::Finalized,
        Some("Verified") => CommitState::Verified,
        _ => return Err(Error::Rpc("expected Monad commitState".into())),
    };
    Ok((header, commitment))
}
fn notification<'a>(
    value: &'a Value,
    subscriptions: &Subscriptions,
) -> Result<Option<(Subscription, &'a Value)>> {
    if let Some(error) = value.get("error") {
        return Err(Error::Rpc(error.to_string()));
    }
    if value["method"] != "eth_subscription" {
        return Err(Error::Rpc("unexpected WebSocket message".into()));
    }
    let id = value["params"]["subscription"]
        .as_str()
        .ok_or_else(|| Error::Rpc("missing subscription ID".into()))?;
    let kind = *subscriptions
        .get(id)
        .ok_or_else(|| Error::Rpc("unknown subscription".into()))?;
    Ok(Some((kind, &value["params"]["result"])))
}
async fn subscribe(
    url: &str,
    proxy: Address,
    wmon: Address,
) -> Result<(Socket, Subscriptions, VecDeque<Value>)> {
    let (mut socket, _) = connect_async(url)
        .await
        .map_err(|e| Error::Transport(e.to_string()))?;
    let mut requests: Vec<Value> = log_filters(proxy, wmon)
        .into_iter()
        .map(|filter| json!(["logs", filter]))
        .collect();
    // Register the head stream last, after all log filters are active. This is
    // essential for using later headers as complete-log boundaries.
    requests.push(json!(["monadNewHeads"]));
    let mut subscriptions = BTreeMap::new();
    let mut queued = VecDeque::new();
    for (i, params) in requests.into_iter().enumerate() {
        let kind = if i == 4 {
            Subscription::Heads
        } else {
            Subscription::Logs
        };
        socket
            .send(Message::Text(
                json!({"jsonrpc":"2.0","id":i+1,"method":"eth_subscribe","params":params})
                    .to_string()
                    .into(),
            ))
            .await
            .map_err(|e| Error::Transport(e.to_string()))?;
        loop {
            let value = read_json(&mut socket).await?;
            if value["method"] == "eth_subscription" {
                queued.push_back(value);
                continue;
            }
            if value["id"] != json!(i + 1) {
                return Err(Error::Rpc("unexpected subscription response".into()));
            }
            if let Some(error) = value.get("error") {
                return Err(Error::Rpc(error.to_string()));
            }
            let id = value["result"]
                .as_str()
                .ok_or_else(|| Error::Rpc("missing subscription ID".into()))?;
            if subscriptions.insert(id.to_owned(), kind).is_some() {
                return Err(Error::Rpc("duplicate subscription ID".into()));
            }
            break;
        }
    }
    Ok((socket, subscriptions, queued))
}
async fn read_json(socket: &mut Socket) -> Result<Value> {
    loop {
        match socket.next().await {
            Some(Ok(Message::Text(text))) => {
                return serde_json::from_str(&text).map_err(|e| Error::Rpc(e.to_string()))
            }
            Some(Ok(Message::Binary(bytes))) => {
                return serde_json::from_slice(&bytes).map_err(|e| Error::Rpc(e.to_string()))
            }
            Some(Ok(Message::Ping(data))) => socket
                .send(Message::Pong(data))
                .await
                .map_err(|e| Error::Transport(e.to_string()))?,
            Some(Ok(Message::Pong(_))) => {}
            Some(Ok(Message::Close(_))) | None => {
                return Err(Error::Transport(
                    "subscription closed; explicitly initialize a new client from complete state"
                        .into(),
                ))
            }
            Some(Err(error)) => return Err(Error::Transport(error.to_string())),
            Some(Ok(_)) => {}
        }
    }
}
