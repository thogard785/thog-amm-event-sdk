# Event transport


```rust
use event_driven_sdk::{Address, Config, EventDrivenSdk, U256};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let proxy: Address = std::env::var("THOGAMM_PROXY")?.parse()?;
    let mut sdk = EventDrivenSdk::connect(
        std::env::var("THOGAMM_HTTP_RPC")?,
        std::env::var("THOGAMM_WS_RPC")?,
        proxy,
        Config::default(),
    ).await?;
    loop {
        let update = sdk.next_update().await?;
        let sell = sdk.model().state().tokens[0].token;
        let buy = sdk.model().state().tokens[4].token;
        let quote = sdk.model().quote_exact_input(sell, buy, U256::from(1_000_000))?;
        println!("finalized block={} output={}", update.block.number, quote.amount_out);
    }
}
```

The client establishes four log filters (proxy events, outgoing transfers,
incoming transfers, WMON wrap/unwrap) and then `monadNewHeads` on **one socket**.
Transfers are filtered by the proxy's indexed address, so newly listed tokens
are covered without resubscribing. `Config.wmon` defaults to the implementation's
immutable Monad mainnet WMON and is checked against token 3 in the seed.

Initialization waits for a proposal observed on that connection to finalize,
then calls `getPoolData(0, 64)` once at that finalized hash using EIP-1898
`{blockHash, requireCanonical: true}`. Notifications received during initialization
are retained. The returned context must match the subscribed header. The reader
is then dropped: the running `EventDrivenSdk` has **no HTTP client or RPC reader**.
An application that already has a finalized seed and a continuous subscription
stream can use `EventSynchronizer::from_model()` with no initialization call,
then feed `on_head`/`on_log` in their socket order.

Updates publish **complete finalized blocks**, including empty blocks. This
choice matters: a normal `newHeads` notification does not mean all of its logs
have arrived. [Monad streams headers before logs, serializing each block's
subscription notifications before starting the next block/commitment notification](https://github.com/category-labs/monad-bft/blob/master/monad-rpc/src/websocket/handler.rs).
The subsequent commitment header closes that block's log frame.
[`monadNewHeads` supplies Proposed/Voted/Finalized/Verified notifications](https://docs.monad.xyz/reference/json-rpc/overview#websocket-subscriptions),
so the SDK knows when the full proposed log set is complete without requesting
anything. It orders and deduplicates overlapping log filters, applies complete
finalized ancestry atomically, and discards abandoned proposals. There is no
sleep-based completeness guess. Providers must preserve this Monad per-connection
ordering and supply all commitment notifications; a generic Ethereum subscription
endpoint is not sufficient. Finalized state can trail the latest proposed state;
`state().block` always identifies the actual quoted source.

The reducer covers legacy price/risk/inventory checkpoints (including V1-only
updates, the split XAUt0 price, and V4 risk state), `MakerStorageUpdated`, pause,
ERC-20 transfers, WMON deposit/withdrawal and absolute WMON balance checkpoints.
Swap summary events do not count transfers twice. The schema-6 `MakerTokenAdded`
event appends both token/category metadata and the token's absolute pool balance.
Prefunding is therefore included without a balance read; later transfers in the
same block apply normally. New listings need no snapshot or SDK restart.

Transport failure, missing ancestry, malformed events, inconsistent balance
accounting, or a contract upgrade is returned explicitly. The last published model
retains its block stamp, and the failed stream is not reused. There is **no automatic
reconnect, catch-up query, retry, or snapshot fallback**. A disconnected subscription
cannot reconstruct messages it never received. After fixing the cause, explicitly
initialize a new client (one startup snapshot) or supply a persisted complete
seed/replayed stream. An upgrade requires verification of a compatible quote
model as well as state. Event tracking requires token balance changes to be
observable through Transfer or the supported WMON events; silent rebases cannot
be inferred from logs. Fee-on-transfer settlement is not modeled as an ordinary
ERC-20 transfer.


## Caller-owned stream

The public reducer accepts decoded `BlockHeader`, `CommitState` and `RpcLog`
values. Use `model::events::log_filters(proxy, wmon)` to obtain the same four
filters, register them before `monadNewHeads`, and preserve one connection's
notification order. Feed every commitment stage, not only finalization.

A seed must include a real, known finalized block hash. A plain latest snapshot
has no current block hash and is not sufficient. You must already possess that
identity from your admitted stream; neither the reducer nor quote engine fetches
it. Do not independently multiplex unrelated sockets and assume their ordering
proves log completeness. The provider's serialization is part of the protocol.

Cloning a published `PoolModel` gives workers a stable image. `Update.blocks` may
exceed one when a finalization commits several observed ancestors. Empty blocks
still advance the quote context. The reducer retains its last valid model on an
error, but its block stamp does not advance and the failed stream cannot continue.

The subscription `Config.wmon` defaults to Monad mainnet WMON at registry index 3.
The seed must match it. WMON deposit/withdrawal accounting assumes the supported
WETH9-style events; silent rebases or tokens with incompatible balance-event
semantics require a different event model. The [polling SDK](https://github.com/thogard785/thog-amm-poll-sdk) obtains
absolute balances from each state call, but does not make unsupported settlement
token behavior compatible with the contract.
