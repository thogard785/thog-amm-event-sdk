# ThogAMM event SDK

Quote [ThogAMM](https://github.com/thogard785/thog-amm-integration-guide) in
your own process, kept current by a WebSocket subscription. The SDK reads the
pool once over HTTP, then follows it block by block from logs, with no further
reads.

- Network: Monad mainnet, chain ID 143
- Pool: `0x80c74517BCC2D67fFE02D3ED886796272F647210`
- Supports implementation `0xf83c81c62c4cc1942003983919c03f5fd06d3eb4`
- Needs a Monad HTTP endpoint and a Monad WebSocket endpoint that serves the
  [`monadNewHeads`](https://docs.monad.xyz/reference/json-rpc/overview#websocket-subscriptions)
  subscription
- Rust 1.88 or later. Before Rust 1.90, your project also needs Cargo's
  `resolver = "3"`, which the 2024 edition uses by default

New to ThogAMM? Read the
[integration guide](https://github.com/thogard785/thog-amm-integration-guide/blob/main/THOGAMM_AGGREGATOR_INTEGRATION.md)
first. Quoting is the same as in the
[polling SDK](https://github.com/thogard785/thog-amm-poll-sdk#what-you-get),
which is simpler to run if one HTTP request per refresh suits you.

## Use it

```toml
[dependencies]
thogamm-events = { git = "https://github.com/thogard785/thog-amm-event-sdk", tag = "v0.7.1" }
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
```

```rust
use thogamm_events::{Subscriber, TxContext, U256};

let mut subscriber = Subscriber::connect(http_url, ws_url, pool_address).await?;
loop {
    // Price the swap in the block it should land in.
    let read = subscriber.pool();
    let pool = read.at_block(read.block_number() + 2, read.base_fee())?;
    if let Ok(pair) = pool.pair(usdc, weth) {
        let quote = pair.maker_quote_exact_input(amount_in)?; // makerQuoteExactInput
        let swap = pair.maker_swap_exact_input(amount_in, tx)?; // makerSwapExactInput
    }
    subscriber.next_update().await?; // the next finalized block
}
```

[examples/subscribe.rs](examples/subscribe.rs) is a complete program that
reconnects when the subscription ends:

```sh
MONAD_RPC=https://rpc.monad.xyz MONAD_WS=wss://rpc.monad.xyz \
TOKEN_IN=0x754704Bc059F8C67012fEd69BC8A327a5aafb603 \
TOKEN_OUT=0xEE8c0E9f1BFFb4Eb878d8f15f368A02a35481242 \
AMOUNT_IN=100000000 cargo run --example subscribe
```

## How it stays correct

`subscriber.pool()` is the pool as of the latest **finalized** block the SDK
has followed. The chain's head is a block or two ahead of it, so evaluate
quotes at the block you expect to trade in with `pool.at_block(..)`.

Following from logs is exact only if no log is missed, so the SDK is strict:

- It applies a block only once it is finalized and all of its logs have
  arrived. Proposed blocks that are never finalized change nothing.
- It stops at the first sign of a gap: a closed or silent connection (10
  seconds by default, `set_stall_timeout`), a block it did not see proposed, a
  log that does not fit the state. `next_update()` then returns an error, and
  keeps returning one.
- It stops when a token is listed or retired (`Error::RegistryChanged`) and
  when the pool is upgraded (`Error::ContractUpgraded`).

After an error, connect again; that reads a fresh snapshot. After
`ContractUpgraded`, first check that this release supports the new
implementation.

The SDK relies on the order in which a Monad node delivers one connection's
notifications: a block's head, then all of that block's logs, then the next
head. Use one connection to one node, not a load balancer that can interleave
several.

## Your own subscription

If you already run a Monad subscription, use `Follower` and feed it yourself:

```rust
let mut follower = Follower::new(snapshot)?;   // a finalized block, hash known
follower.on_head(header, CommitState::Proposed)?;
follower.on_log(log)?;
if follower.on_head(header, CommitState::Finalized)? {
    let pool = follower.pool();                // advanced
}
```

Subscribe to `thogamm::events::log_filters(pool_address)` and to
`monadNewHeads`, on one connection, heads last. Feed every notification in the
order it arrives, including `Voted` and `Verified` heads.

## Tests

```sh
cargo test --release
```

The tests follow 50 recorded blocks of the deployed pool through every commit
state and compare the result with the state read from the chain at each block,
over a real WebSocket as well as directly. They cover reordered and duplicated
logs, repeated and abandoned proposals, gaps, stalls, upgrades and listing
changes. The pricing arithmetic is tested in the
[polling SDK](https://github.com/thogard785/thog-amm-poll-sdk#tests), whose
`thogamm` crate this SDK uses.

One more test follows the deployed pool for 60 blocks and compares with the
chain:

```sh
MONAD_RPC=https://rpc.monad.xyz MONAD_WS=wss://rpc.monad.xyz \
cargo test --release --test live -- --ignored --nocapture
```

[MIT license](LICENSE).
