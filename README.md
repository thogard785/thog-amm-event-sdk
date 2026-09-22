# ThogAMM event SDK

Maintain a deterministic in-memory ThogAMM model from Monad subscriptions.
Initialization makes one state call; operating updates make **zero read RPCs**.
Requires a compatible **schema-6** deployment and Monad's ordered commitment stream.

## Install

```toml
[dependencies]
event-driven-sdk = { git = "https://github.com/thogard785/thog-amm-event-sdk", tag = "v0.1.1" }
```

Requires Rust 1.88+. In your application, declare `rust-version = "1.88"` and
use Cargo resolver `"3"` (at the workspace root for a workspace). Commit your
application's lockfile. The crate imports as `event_driven_sdk` and is distributed
through GitHub. Its shared quote engine is pinned to the matching release of
[thog-amm-poll-sdk](https://github.com/thogard785/thog-amm-poll-sdk).

## Subscribe and quote

```rust
use event_driven_sdk::{Config, EventDrivenSdk};

async fn follow(
    http_rpc: &str,
    ws_rpc: &str,
    proxy: event_driven_sdk::Address,
) -> event_driven_sdk::Result<()> {
    let mut sdk = EventDrivenSdk::connect(http_rpc, ws_rpc, proxy, Config::default()).await?;
    // sdk.model() is ready to quote after initialization.
    loop {
        let update = sdk.next_update().await?;
        let model = sdk.model();
        // Prepare directions and quote amounts against this immutable model.
        println!("finalized block={} tokens={}", update.block.number, model.state().tokens.len());
    }
}
```

Initialization registers four log filters and `monadNewHeads` on one socket,
waits for an observed proposal to finalize, and makes one hash-pinned
`getPoolData(0,64)` call. Notifications received during startup are retained.
The seed must match the subscribed header and `Config.wmon`, which defaults to
Monad mainnet WMON. The HTTP reader is then dropped.

`next_update()` consumes pushed messages, orders/deduplicates overlapping filters,
and publishes complete finalized ancestry atomically, including empty blocks.
New listings carry metadata and absolute balances; they need no additional reads
or subscriptions. Finalized state can trail the latest proposal. Every model
retains its actual block number, hash and base fee.

The provider must serialize a block's complete log notifications before the next
header/commitment notification on that connection. Feed every commitment stage.
Ordinary Ethereum `newHeads` plus logs does not establish completeness, and
independently ordered sockets cannot be combined to infer it.

## Quoting

All quotes are synchronous local integer arithmetic over an immutable snapshot.
Amounts are raw token units in `U256`; token addresses and decimals are in the
model. Clone it to share a snapshot across workers, then call
`model.prepare(sell, buy)` once and reuse the pair for multiple amounts.

- `quote_exact_input` matches the read-only maker quote.
- `quote_execution_exact_input` adds execution friction and exposure checks.
- `quote_exact_output` solves the ERC-7815 Buy input.
- `marginal_price` and `limits` expose price and directional capacity.

Execution methods require `ExecutionContext { gas_price, fast_lane_hot }`:
effective transaction gas price, not a fee cap, and actual transaction-local
FastLane warmth. Pauses, stale prices and capacity limits produce errors.
`at_block(number, base_fee)` projects aging without predicting intervening state.
Balances and risk are shared across pairs; independent quotes do not simulate
successive fills. Prepare again when selecting another snapshot or projection.

See the shared model's [quote and settlement instructions](https://github.com/thogard785/thog-amm-poll-sdk#quoting)
for method semantics, direct swap encoding, funding and transaction constraints.

## Caller-owned streams and failures

`EventSynchronizer::from_model()` takes a validated finalized seed with a known
block hash and performs no I/O. Supply `BlockHeader`, `CommitState` and `RpcLog`
values through `on_head` and `on_log` in connection order. The stream must cover
every successor. `model::events::log_filters(proxy, wmon)` supplies the log filters.

Disconnects, missing history, malformed logs, inconsistent balances and upgrades
return explicit errors and fail the stream. The last valid model keeps its old
block stamp. There is no automatic reconnect, catch-up query or recovery snapshot.
After resolving the cause, explicitly initialize another client or replay a
complete persisted stream. `ContractUpgraded` requires a compatible model first.

Balance changes must be observable through ERC-20 Transfer or the supported
WETH9-style WMON events. Silent rebases cannot be inferred. With a custom startup
`ChainReader`, preserve Monad's real BASEFEE using a nonzero simulation gas price
(the built-in reader uses one wei). Startup requires EIP-1898 hash-pinned calls.

## Example and tests

Set `THOGAMM_HTTP_RPC`, `THOGAMM_WS_RPC`, `THOGAMM_PROXY`, `TOKEN_IN`, `TOKEN_OUT`,
`AMOUNT_IN`, `EFFECTIVE_GAS_PRICE_WEI` and `FAST_LANE_HOT` (`true`/`false`):

```sh
cargo run --locked --example subscribe
cargo test --locked --release --all-targets
cargo clippy --locked --all-targets -- -D warnings
```

The example prints execution quotes with the supplied context; it submits no
transactions. Tests use synthetic Solidity fixtures and a local WebSocket server
and verify state transitions and zero operating read calls. No live network is
required by the tests. Confirm contract/provider compatibility before routing.

[MIT license](LICENSE).
