# ThogAMM event SDK

A Rust SDK for aggregators integrating ThogAMM on Monad. Load pool state once,
then keep an in-memory quote model up to date through event subscriptions.
**Updates after initialization make zero read RPC calls.**

It works with the current ThogAMM deployment and automatically supports all
current and future tokens listed on ThogAMM. New listings, balances and pricing
updates arrive through the subscribed events.

- **Network:** Monad mainnet, chain ID `143`
- **ThogAMM pool:** `0x80c74517BCC2D67fFE02D3ED886796272F647210`
- **Connection:** a Monad HTTP RPC URL for initialization and a WebSocket URL
  supporting `logs` and `monadNewHeads` subscriptions

## Install

```toml
[dependencies]
event-driven-sdk = { git = "https://github.com/thogard785/thog-amm-event-sdk", tag = "v0.2.0" }
```

Requires Rust 1.88 or later and Tokio for subscriptions. Set your application's
`rust-version` and use Cargo resolver `"3"` at the package or workspace root so
dependencies respect that Rust version. Commit your application's `Cargo.lock`.
Import the crate as `event_driven_sdk`.

## Connect once, then quote locally

Pass your Monad RPC URLs, the ThogAMM pool address above, the two token addresses,
an input amount and your transaction's effective gas price in wei:

```rust
use event_driven_sdk::{Address, Config, EventDrivenSdk, Result, U256};

async fn follow_quotes(
    http_rpc: &str,
    ws_rpc: &str,
    pool: Address,
    token_in: Address,
    token_out: Address,
    amount_in: U256,
    gas_price: U256,
) -> Result<()> {
    let mut sdk = EventDrivenSdk::connect(http_rpc, ws_rpc, pool, Config::default()).await?;
    loop {
        let model = sdk.model();
        match model.quote_execution_exact_input(token_in, token_out, amount_in, gas_price) {
            Ok(quote) => println!("amount_out={}", quote.amount_out),
            Err(error) => eprintln!("quote unavailable: {error}"),
        }
        sdk.next_update().await?;
    }
}
```

`connect()` opens subscriptions and loads the initial state with one
`getPoolData(0, 64)` call. `next_update()` consumes subscription messages and
publishes a complete finalized snapshot, including blocks without trades.
There are no polling, log-history, balance or token-discovery reads while running.

Finalized state may trail the latest proposed block. Use
`model.state().block.number` and `.hash` to identify the quoted state;
`model.state().tokens` provides listed token addresses and decimals. Keep calling
`next_update()` to process incoming messages. Quote methods are synchronous and
make no network calls.

## Quote trade sizes

Amounts are `U256` values in raw token units: one token with six decimals is
`1_000_000`. `gas_price` is the intended transaction's effective price in wei;
for EIP-1559, use `min(maxFeePerGas, baseFee + maxPriorityFeePerGas)`.
The model includes additional spread if that price exceeds twice the block base fee.

| Method on `PoolModel` | Result |
| --- | --- |
| `quote_execution_exact_input(token_in, token_out, amount_in, gas_price)` | Output amount, including balance and exposure checks |
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

If `next_update()` reports a disconnect or missing events, create a new client
with `connect()` to load current state and resume subscriptions. The SDK reports
these errors instead of silently making recovery reads. The last valid snapshot
keeps its original block stamp. Quote errors such as a stale price or unavailable
pair do not stop the subscription; keep processing updates.

If the SDK reports an unsupported state format or a contract upgrade, use the
SDK release supplied by ThogAMM for that update.

## Use your own event stream

`EventSynchronizer::from_model()` accepts a finalized snapshot with a known block
hash and performs no I/O. Pass `BlockHeader`, `CommitState` and `RpcLog` values to
`on_head` and `on_log`. `model::events::log_filters(pool, wmon)` provides the filters.

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
