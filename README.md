# ThogAMM event SDK

A Rust SDK for aggregators integrating ThogAMM on Monad. Load pool state once,
then keep an in-memory quote model up to date through event subscriptions.
**Updates after initialization make zero read RPC calls.**

It supports the deployed schema-7 registry and policy events. A listing or
retirement returns `Error::RegistryChanged`: reconnect to obtain the complete
replacement state, since those events omit policy rows required for pricing.

- **Network:** Monad mainnet, chain ID `143`
- **ThogAMM pool:** `0x80c74517BCC2D67fFE02D3ED886796272F647210`
- **Connection:** a Monad HTTP RPC URL for initialization and a WebSocket URL
  supporting `logs` and `monadNewHeads` subscriptions

## Install

```toml
[dependencies]
event-driven-sdk = { git = "https://github.com/thogard785/thog-amm-event-sdk", tag = "v0.3.0" }
```

Requires Rust 1.88 or later and Tokio for subscriptions. Both Cargo resolvers 2 and 3 are supported. Commit your application's
`Cargo.lock`. The shared model dependency is pinned to an immutable Git revision.
Import the crate as `event_driven_sdk`.

## Connect once, then quote locally

Pass your Monad RPC URLs, the ThogAMM pool address above, the two token addresses,
an input amount and your transaction's effective gas price in wei:

```rust
use event_driven_sdk::{Address, Config, Error, EventDrivenSdk, Result, U256};
use std::time::Duration;

async fn follow_quotes(http_rpc: &str, ws_rpc: &str, pool: Address,
    token_in: Address, token_out: Address, amount_in: U256, gas_price: U256) -> Result<()> {
    let mut delay = Duration::from_millis(250);
    loop {
        let mut sdk = match EventDrivenSdk::connect(http_rpc, ws_rpc, pool, Config::default()).await {
            Ok(sdk) => sdk,
            Err(error @ (Error::Transport(_) | Error::Rpc(_))) => {
                eprintln!("connect failed: {error}; retrying in {delay:?}");
                tokio::time::sleep(delay).await;
                delay = (delay * 2).min(Duration::from_secs(8));
                continue;
            }
            Err(error) => return Err(error),
        };
        loop {
            match sdk.next_update().await {
                Ok(update) => {
                    delay = Duration::from_millis(250);
                    let model = sdk.model()?;
                    match model.quote_execution_exact_input(token_in, token_out, amount_in, gas_price) {
                        Ok(q) => println!("block={} timestamp={} amount_out={}", update.block.number, update.block.timestamp, q.amount_out),
                        Err(error) => eprintln!("quote unavailable: {error}"),
                    }
                }
                Err(error @ (Error::Transport(_) | Error::Rpc(_) | Error::Discontinuous
                    | Error::RegistryChanged | Error::ContractUpgraded | Error::BalanceCheckpointMismatch { .. })) => {
                    eprintln!("subscription failed: {error}; reconnecting in {delay:?}");
                    break;
                }
                Err(error) => return Err(error),
            }
        }
        tokio::time::sleep(delay).await;
        delay = (delay * 2).min(Duration::from_secs(8));
    }
}
```

`connect()` opens subscriptions and loads the initial state with one
`getPoolData(0, 32)` call. `next_update()` consumes subscription messages and
publishes a complete finalized snapshot by default, including blocks without trades.
There are no polling, log-history, balance or token-discovery reads while running.

For earlier complete state, set `Config { publish_at: PublishAt::Voted,
..Config::default() }` (import `event_driven_sdk::PublishAt`). This publishes
when the voted header closes a fully received proposal's logs. It never publishes
an incomplete proposal. A voted ancestry conflict is reported and requires a new
snapshot. Finalized mode remains the default. The provider ordering is defined
by Monad's [WebSocket handler](https://github.com/category-labs/monad-bft/blob/4f40dca998738bcd2a504b4c86b26475afb72c12/monad-rpc/src/websocket/handler.rs).

Finalized state may trail the latest voted block. Use
`model.state().block.number` and `.hash` to identify the quoted state;
`model.state().tokens` provides listed token addresses and decimals. Keep calling
`next_update()` to process incoming messages. Quote methods are synchronous and
make no network calls. `Update.block.timestamp` is the chain timestamp (Unix
seconds); `Update.received_at` is the local publication time. Compare the block
number with your head and use the timestamp to identify a queued old update.
`Config::update_timeout` defaults to 2 seconds without a complete new block;
the client sends idle WebSocket pings, but pongs do not extend that deadline.

## Quote trade sizes

Amounts are `U256` values in raw token units: one token with six decimals is
`1_000_000`. `gas_price` is the intended transaction's effective price in wei;
for EIP-1559, use `min(maxFeePerGas, baseFee + maxPriorityFeePerGas)`.
Execution applies one 10 bp output reduction if that price exceeds twice the
quote block base fee or the AuctionHandler is already warm. The convenience
methods below assume a cold AuctionHandler. Use
`quote_execution_exact_input_with_context(..., gas_price, auction_handler_warm)`
and `quote_exact_output_with_context(...)` to specify account warmth explicitly.
`quote_exact_input(...)` matches `makerQuoteExactInput` with its strict 90%
receiving-inventory caps and no execution widening. Execution checks full caps.

| Method on `PoolModel` | Result |
| --- | --- |
| `quote_execution_exact_input(token_in, token_out, amount_in, gas_price)` | Output amount, including final full-cap checks; cold AuctionHandler |
| `quote_exact_output(token_in, token_out, amount_out, gas_price)` | Required input for an exact output amount |
| `limits(token_in, token_out)` | Directional input and output bounds |
| `marginal_price(token_in, token_out, amount_in)` | A fraction expressing raw output units per raw input unit |

For multiple amounts on the same pair, prepare it once per snapshot:

```rust
let pair = model.prepare(token_in, token_out)?;
for amount_in in amounts_in {
    let quote = pair.quote_execution_exact_input(amount_in, gas_price)?;
    println!("{} -> {}", quote.amount_in, quote.amount_out);
}
```

A prepared pair has the same quote methods, without the two token arguments.
Preparation and successful quotes allocate no memory. Clone a model to share
its immutable snapshot across workers, and prepare a new pair after an update.
Quotes use the contract's integer rounding and reject paused trading, stale
prices, disabled directions and insufficient capacity.

Quotes apply to the snapshot's block. `model.at_block(number, base_fee)` evaluates
price aging for a later landing block, assuming no intervening trades or price
updates. Pairs share balances and portfolio risk; independent quotes do not
simulate successive fills. For swap encoding and settlement, see the shared
[transaction guide](https://github.com/thogard785/thog-amm-poll-sdk#build-a-swap-transaction).

## Handle connection errors

The example reports every failure and uses capped exponential reconnect backoff,
including failures during initial connection. A successful update resets the
delay. Unsupported formats and malformed snapshots stop it for investigation.

If `next_update()` reports a disconnect or missing events, create a new client
with `connect()` to load current state and resume subscriptions. The SDK reports
these errors instead of silently making recovery reads. `sdk.model()` returns an error after a failed or cancelled update. A previously
cloned model keeps its original block stamp. Quote errors such as a stale price or unavailable
pair do not stop the subscription; keep processing updates.

If the registry changes, reconnect and rebuild all prepared pairs. An upgrade
also requires a complete post-upgrade snapshot: reconnect validates its schema,
wire and model versions. A same-version upgrade can resume with this SDK; only
an unsupported format requires a matching release. The upgrade event alone
cannot establish that the previous state is still complete.

Event completeness assumes the supported tokens emit their ordinary balance
movements and native MON follows the executor’s checkpointed paths. Forced
native transfers or tokens that change balances without events require account
state updates or a fresh atomic snapshot; log subscriptions alone cannot prove
those changes. The SDK does not perform hidden recovery reads.

## Use your own event stream

`EventSynchronizer::from_model()` accepts a finalized snapshot with a known block
hash and performs no I/O. Pass `BlockHeader`, `CommitState` and `RpcLog` values to
`on_head` and `on_log`. `from_model_with_commitment(model, PublishAt::Voted)`
selects earlier publication for your own stream.
`model::events::log_filters(pool, wmon)` provides the filters.

Use one ordered Monad connection and pass every header commitment stage and log
in received order. The provider must finish a block's logs before its next header
notification. Standard Ethereum `newHeads` alone does not provide the completeness
information this reducer needs. The built-in client handles these subscriptions.

## Run the example

Set `THOGAMM_HTTP_RPC`, `THOGAMM_WS_RPC`, `THOGAMM_PROXY` (the pool address above),
`TOKEN_IN`, `TOKEN_OUT`, `AMOUNT_IN` and `EFFECTIVE_GAS_PRICE_WEI`, then run:

```sh
cargo run --locked --example subscribe
```

[examples/subscribe.rs](examples/subscribe.rs) processes updates and prints local
quotes. For integrations that already schedule their own state reads, the
[polling SDK](https://github.com/thogard785/thog-amm-poll-sdk) offers the same quote
model with one `eth_call` per refresh. Both SDKs are [MIT licensed](LICENSE).

## Schema 7 and verification

Version **0.3.0** supports schema 7 / wire 7 / model 1 and up to 32 active
tokens. It replaces the incompatible schema-6 model. State includes all 126
words, all registered ERC-20 balances, native MON, lending exposure offsets,
separate bid/ask widths, volume charges, wrapper policies and inventory caps.

Volume cost is `ceil(rate * notionalUSDWad² / 1e27)` and remains payable for
risk-reducing trades, same-category trades and zero lambda. Sliding output
refunds participate in the nonlinear output solve with exact integer rounding.

The [integration guide](https://github.com/thogard785/thog-amm-integration-guide)
contains the full ABI, state layout, math, settlement rules and reproducible
fixtures. Tests cover **9,240 contract comparisons**: 2,016 mainnet read-only
queries, 1,680 execution/exact-output calls on a local Monad fork, and 5,544
fork boundary cases. All 5,362 successful results match exactly; all 3,878
rejected cases also reject locally. Event replay matches 200 consecutive
mainnet snapshots, including every word, ERC-20 balance and native balance.
Separate mainnet fixtures cover lending exposure and pause/unpause transitions.
Run `cargo test --locked` (or `cargo test --workspace --locked` in the polling
repository). Fork tests do not broadcast transactions.
