//! Keep a ThogAMM pool current from a Monad WebSocket subscription.
//!
//! [`Subscriber`] reads the pool once, then follows it block by block from
//! logs alone. [`Follower`] is the same logic without the socket, for callers
//! that already run a Monad subscription.
//!
//! Quoting is in the [`thogamm`] crate, re-exported here.
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::{
    collections::{btree_map::Entry, BTreeMap, VecDeque},
    time::Duration,
};
pub use thogamm::{
    self,
    events::{BlockHeader, Log},
    rpc::{EthCall, HttpRpc},
    Address, Error, Pair, Pool, Quote, Rejection, Result, Swap, TxContext, B256, U256,
};
use thogamm::{
    events::{apply_block, log_filters},
    rpc::{read_snapshot, BlockId},
    Snapshot,
};
use tokio::net::TcpStream;
use tokio_tungstenite::{connect_async, tungstenite::Message, MaybeTlsStream, WebSocketStream};

/// The stage a Monad block has reached, as `monadNewHeads` reports it in
/// `commitState`. <https://docs.monad.xyz/reference/websockets>
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommitState {
    Proposed,
    Voted,
    Finalized,
    Verified,
}

struct PendingBlock {
    header: BlockHeader,
    logs: BTreeMap<(u64, u64), Log>,
    /// Set once the next head arrives: every log of this block has been seen.
    complete: bool,
}

/// Follows the pool through finalized blocks, given heads and logs in the
/// order one Monad WebSocket connection delivers them.
///
/// A Monad node sends a block's head, then all of that block's logs, before
/// the next head. That ordering is what tells the follower a block's logs are
/// complete; an Ethereum-style `newHeads` subscription does not give it.
/// Proposed blocks are held back, and applied only once finalized.
pub struct Follower {
    pool: Pool,
    pending: BTreeMap<B256, PendingBlock>,
    /// The proposed block whose logs are arriving now.
    receiving: Option<B256>,
    failed: bool,
}

impl Follower {
    /// Start from a snapshot of a finalized block whose hash is known.
    pub fn new(snapshot: Snapshot) -> Result<Self> {
        if snapshot.block.hash.is_none() {
            return Err(Error::InvalidSnapshot(
                "following needs the snapshot's block hash".into(),
            ));
        }
        Ok(Self {
            pool: Pool::new(snapshot)?,
            pending: BTreeMap::new(),
            receiving: None,
            failed: false,
        })
    }

    /// The pool as of the latest finalized block that was followed.
    pub fn pool(&self) -> &Pool {
        &self.pool
    }

    /// Feed one log. After an error the follower stays failed: start again
    /// from a fresh snapshot.
    pub fn on_log(&mut self, log: Log) -> Result<()> {
        self.guard(|follower| follower.add_log(log))
    }

    /// Feed one head. Returns true when the pool advanced.
    pub fn on_head(&mut self, header: BlockHeader, state: CommitState) -> Result<bool> {
        self.guard(|follower| follower.add_head(header, state))
    }

    fn guard<T>(&mut self, step: impl FnOnce(&mut Self) -> Result<T>) -> Result<T> {
        if self.failed {
            return Err(Error::Discontinuous);
        }
        let result = step(self);
        self.failed = result.is_err();
        result
    }

    fn add_log(&mut self, log: Log) -> Result<()> {
        if log.removed {
            return Err(Error::Discontinuous);
        }
        if log.block_number <= self.pool.snapshot().block.number {
            return Ok(());
        }
        let block = self
            .receiving
            .filter(|hash| *hash == log.block_hash)
            .and_then(|hash| self.pending.get_mut(&hash))
            .filter(|block| !block.complete && block.header.number == log.block_number)
            .ok_or(Error::Discontinuous)?;
        // The four filters overlap, so the same log can arrive more than once.
        match block.logs.entry((log.transaction_index, log.log_index)) {
            Entry::Occupied(seen) if seen.get() != &log => Err(Error::Discontinuous),
            Entry::Occupied(_) => Ok(()),
            Entry::Vacant(slot) => {
                slot.insert(log);
                Ok(())
            }
        }
    }

    fn add_head(&mut self, header: BlockHeader, state: CommitState) -> Result<bool> {
        // A new head means the block that was receiving logs has them all.
        if let Some(hash) = self.receiving.take() {
            self.pending
                .get_mut(&hash)
                .ok_or(Error::Discontinuous)?
                .complete = true;
        }
        let current = &self.pool.snapshot().block;
        if header.number <= current.number {
            let replaced = state == CommitState::Finalized
                && header.number == current.number
                && Some(header.hash) != current.hash;
            return if replaced {
                Err(Error::Discontinuous)
            } else {
                Ok(false)
            };
        }
        match state {
            CommitState::Proposed => {
                let block = self
                    .pending
                    .entry(header.hash)
                    .or_insert_with(|| PendingBlock {
                        header: header.clone(),
                        logs: BTreeMap::new(),
                        complete: false,
                    });
                if block.header != header {
                    return Err(Error::Discontinuous);
                }
                // A proposal can be sent again; its logs follow again too.
                block.complete = false;
                self.receiving = Some(header.hash);
                Ok(false)
            }
            CommitState::Voted | CommitState::Verified => Ok(false),
            CommitState::Finalized => {
                self.finalize(&header)?;
                Ok(true)
            }
        }
    }

    /// Apply every block from the pool's block up to `header`, or none.
    fn finalize(&mut self, header: &BlockHeader) -> Result<()> {
        let current = &self.pool.snapshot().block;
        let mut chain = Vec::new();
        let mut hash = header.hash;
        for number in (current.number + 1..=header.number).rev() {
            let block = self
                .pending
                .get(&hash)
                .filter(|block| block.complete && block.header.number == number)
                .ok_or(Error::Discontinuous)?;
            chain.push(block);
            hash = block.header.parent_hash;
        }
        if Some(hash) != current.hash || chain.first().is_some_and(|tip| tip.header != *header) {
            return Err(Error::Discontinuous);
        }
        let mut snapshot = self.pool.snapshot().clone();
        for block in chain.into_iter().rev() {
            snapshot = apply_block(snapshot, &block.header, block.logs.values())?;
        }
        self.pool = Pool::new(snapshot)?;
        self.pending
            .retain(|_, block| block.header.number > header.number);
        Ok(())
    }
}

type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

#[derive(Clone, Copy)]
enum Feed {
    Logs,
    Heads,
}

/// How long [`Subscriber::next_update`] waits for any message before giving
/// up on the connection. Monad produces a block about every 0.4 seconds.
pub const DEFAULT_STALL_TIMEOUT: Duration = Duration::from_secs(10);

/// A ThogAMM pool read once over HTTP, then followed over a WebSocket.
pub struct Subscriber {
    follower: Follower,
    socket: Socket,
    feeds: BTreeMap<String, Feed>,
    /// Notifications that arrived before the first snapshot was read.
    backlog: VecDeque<Value>,
    stall_timeout: Duration,
}

impl Subscriber {
    /// Subscribe through a Monad WebSocket endpoint, then read the pool at
    /// `pool_address` through a Monad HTTP endpoint.
    pub async fn connect(
        http_url: impl Into<String>,
        ws_url: impl AsRef<str>,
        pool_address: Address,
    ) -> Result<Self> {
        Self::with_rpc(&HttpRpc::new(http_url)?, ws_url, pool_address).await
    }

    /// As [`Subscriber::connect`], reading the first snapshot through your own RPC client.
    pub async fn with_rpc(
        rpc: &impl EthCall,
        ws_url: impl AsRef<str>,
        pool_address: Address,
    ) -> Result<Self> {
        let timeout = DEFAULT_STALL_TIMEOUT;
        // Subscribe first, so that no block between the snapshot and the
        // first notification can be missed.
        let (mut socket, feeds) = deadline(timeout, subscribe(ws_url.as_ref(), pool_address))
            .await
            .ok_or_else(|| Error::Transport("subscribing timed out".into()))??;
        let mut backlog = VecDeque::new();
        let start = deadline(
            timeout,
            first_finalized_head(&mut socket, &feeds, &mut backlog),
        )
        .await
        .ok_or_else(|| Error::Transport("no finalized block arrived".into()))??;
        // A finalized block is the only block at its number.
        let mut snapshot =
            read_snapshot(rpc, pool_address, BlockId::Number(start.number), &[]).await?;
        if snapshot.block.parent_hash != start.parent_hash
            || snapshot.block.base_fee != start.base_fee_per_gas
        {
            return Err(Error::Rpc(
                "the HTTP and WebSocket endpoints disagree about the block".into(),
            ));
        }
        snapshot.block.hash = Some(start.hash);
        Ok(Self {
            follower: Follower::new(snapshot)?,
            socket,
            feeds,
            backlog,
            stall_timeout: timeout,
        })
    }

    /// Change how long [`Subscriber::next_update`] tolerates a silent connection.
    pub fn set_stall_timeout(&mut self, timeout: Duration) {
        self.stall_timeout = timeout;
    }

    /// The pool as of the latest finalized block. Its block is
    /// `pool().snapshot().block.number`; the chain's head is a block or two
    /// ahead. Before quoting, move the pool to the block you expect to trade
    /// in with [`Pool::at_block`].
    pub fn pool(&self) -> &Pool {
        self.follower.pool()
    }

    /// Wait until the pool has advanced to a newer finalized block.
    ///
    /// Any error ends the subscription: a missed block cannot be recovered
    /// from logs. Connect again to continue.
    pub async fn next_update(&mut self) -> Result<&Pool> {
        if self.follower.failed {
            return Err(Error::Discontinuous);
        }
        loop {
            let message = match self.backlog.pop_front() {
                Some(message) => message,
                None => deadline(self.stall_timeout, next_message(&mut self.socket))
                    .await
                    .ok_or_else(|| {
                        self.follower.failed = true;
                        Error::Transport("the subscription went silent".into())
                    })??,
            };
            let advanced = match notification(&message, &self.feeds) {
                Ok((Feed::Logs, log)) => parse_log(log)
                    .and_then(|log| self.follower.on_log(log))
                    .map(|()| false),
                Ok((Feed::Heads, head)) => parse_head(head)
                    .and_then(|(header, state)| self.follower.on_head(header, state)),
                Err(error) => Err(error),
            };
            match advanced {
                Ok(true) => return Ok(self.follower.pool()),
                Ok(false) => {}
                Err(error) => {
                    self.follower.failed = true;
                    return Err(error);
                }
            }
        }
    }
}

async fn deadline<T>(limit: Duration, future: impl std::future::Future<Output = T>) -> Option<T> {
    tokio::time::timeout(limit, future).await.ok()
}

/// Open the four log subscriptions, then `monadNewHeads`. Heads come last so
/// that every head seen is followed by all of its block's logs.
async fn subscribe(url: &str, pool: Address) -> Result<(Socket, BTreeMap<String, Feed>)> {
    let (mut socket, _) = connect_async(url)
        .await
        .map_err(|e| Error::Transport(e.to_string()))?;
    let mut requests: Vec<(Feed, Value)> = log_filters(pool)
        .into_iter()
        .map(|filter| (Feed::Logs, json!(["logs", filter])))
        .collect();
    requests.push((Feed::Heads, json!(["monadNewHeads"])));
    let mut feeds = BTreeMap::new();
    for (id, (feed, params)) in requests.into_iter().enumerate() {
        let request =
            json!({ "jsonrpc": "2.0", "id": id, "method": "eth_subscribe", "params": params });
        socket
            .send(Message::Text(request.to_string().into()))
            .await
            .map_err(|e| Error::Transport(e.to_string()))?;
        // Notifications of earlier subscriptions may arrive before this
        // reply. They describe blocks older than the first snapshot.
        let reply = loop {
            let message = next_message(&mut socket).await?;
            if message["method"] != "eth_subscription" {
                break message;
            }
        };
        match reply["result"].as_str() {
            Some(subscription) if reply["id"] == json!(id) => {
                feeds.insert(subscription.to_owned(), feed);
            }
            _ => return Err(Error::Rpc(format!("eth_subscribe failed: {reply}"))),
        }
    }
    Ok((socket, feeds))
}

/// Wait for a finalized head that was first seen as a proposal on this
/// connection, keeping every notification for replay after the snapshot.
async fn first_finalized_head(
    socket: &mut Socket,
    feeds: &BTreeMap<String, Feed>,
    backlog: &mut VecDeque<Value>,
) -> Result<BlockHeader> {
    let mut first_proposal = None;
    loop {
        let message = next_message(socket).await?;
        let head = match notification(&message, feeds)? {
            (Feed::Heads, head) => Some(parse_head(head)?),
            (Feed::Logs, _) => None,
        };
        backlog.push_back(message);
        if let Some((header, state)) = head {
            if state == CommitState::Proposed && first_proposal.is_none() {
                first_proposal = Some(header.number);
            }
            if state == CommitState::Finalized && first_proposal.is_some_and(|n| header.number >= n)
            {
                return Ok(header);
            }
        }
    }
}

fn notification<'a>(
    message: &'a Value,
    feeds: &BTreeMap<String, Feed>,
) -> Result<(Feed, &'a Value)> {
    let feed = message["params"]["subscription"]
        .as_str()
        .and_then(|id| feeds.get(id))
        .filter(|_| message["method"] == "eth_subscription")
        .ok_or_else(|| Error::Rpc(format!("unexpected WebSocket message: {message}")))?;
    Ok((*feed, &message["params"]["result"]))
}

fn parse_log(log: &Value) -> Result<Log> {
    serde_json::from_value(log.clone()).map_err(|e| Error::Rpc(e.to_string()))
}

fn parse_head(head: &Value) -> Result<(BlockHeader, CommitState)> {
    let state = match head["commitState"].as_str() {
        Some("Proposed") => CommitState::Proposed,
        Some("Voted") => CommitState::Voted,
        Some("Finalized") => CommitState::Finalized,
        Some("Verified") => CommitState::Verified,
        _ => return Err(Error::Rpc("head without a Monad commitState".into())),
    };
    let header = serde_json::from_value(head.clone()).map_err(|e| Error::Rpc(e.to_string()))?;
    Ok((header, state))
}

async fn next_message(socket: &mut Socket) -> Result<Value> {
    let transport = |e: tokio_tungstenite::tungstenite::Error| Error::Transport(e.to_string());
    loop {
        match socket.next().await {
            Some(Ok(Message::Text(text))) => {
                return serde_json::from_str(&text).map_err(|e| Error::Rpc(e.to_string()))
            }
            Some(Ok(Message::Binary(bytes))) => {
                return serde_json::from_slice(&bytes).map_err(|e| Error::Rpc(e.to_string()))
            }
            Some(Ok(Message::Ping(payload))) => socket
                .send(Message::Pong(payload))
                .await
                .map_err(transport)?,
            Some(Ok(Message::Close(_))) | None => {
                return Err(Error::Transport("the subscription closed".into()))
            }
            Some(Ok(_)) => {}
            Some(Err(error)) => return Err(transport(error)),
        }
    }
}
