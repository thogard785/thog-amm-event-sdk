# ThogAMM event SDK

Keep an in-memory ThogAMM quote model current through Monad subscriptions.
Version **0.6.1** supports **schema 8 / wire 8 / model 3** with pool-stored
xStock multipliers and the expanded 17-token registry. The nine unwrapped
xStocks are SPYx, QQQx, EWYx, NVDAx, TSLAx, SMHx, SPCXx, MUx and SKHYx, all
with 18 decimals. [Token addresses](https://github.com/thogard785/thog-amm-integration-guide/blob/main/THOGAMM_AGGREGATOR_INTEGRATION.md#listed-tokens).

Quote calculations are synchronous and make no network calls. The polling SDK
reads each multiplier from the pool's `storedMultiplier(token)`; the event
SDK applies `MakerMultiplierUpdated` after its initial snapshot. Issuer changes
alone do not change the model's stored values.

- Monad mainnet, chain ID **143**.
- Pool: `0x80c74517BCC2D67fFE02D3ED886796272F647210`.
- HTTP RPC plus a WebSocket endpoint supporting `logs` and `monadNewHeads`.
- Rust **1.88+**, Cargo resolver **3**, Tokio. Commit your application's lockfile.

Use this release with implementation
`0xb108a1daf3c9ba52778b97a15ebdfa3f6f59b506`. SDK 0.5.0 uses issuer
multipliers and is incompatible; 0.6.0 already supports stored multipliers.
The state format remains 8/8/3, so verify implementation compatibility after
an upgrade and load a fresh snapshot.

## Install and subscribe

```toml
[dependencies]
event-driven-sdk = { git = "https://github.com/thogard785/thog-amm-event-sdk", tag = "v0.6.1" }
```

```rust
use event_driven_sdk::{Address, Config, EventDrivenSdk, Result, U256};

async fn follow_quotes(
    http: &str, ws: &str, pool: Address, token_in: Address, token_out: Address,
    amount_in: U256, effective_gas_price: U256,
) -> Result<()> {
    let mut sdk = EventDrivenSdk::connect(http, ws, pool, Config::default()).await?;
    loop {
        match sdk.model().quote_execution_exact_input(
            token_in, token_out, amount_in, effective_gas_price,
        ) {
            Ok(q) => println!("{}", q.amount_out),
            Err(e) => eprintln!("quote unavailable: {e}"),
        }
        sdk.next_update().await?;
    }
}
```

`connect()` subscribes first, then loads pool state, physical shares and pool-stored
multipliers at one finalized block hash. `next_update()` replays complete
finalized blocks, including `MakerMultiplierUpdated`, without RPC reads.
Use `model.state().block.number` and `.hash` to identify the published state;
it can trail the proposed head. Keep consuming updates even when a quote rejects.
Active listings are dynamic; share accounting uses the contract's
[nine compiled xStock addresses](https://github.com/thogard785/thog-amm-poll-sdk/blob/v0.6.1/model/src/shares.rs).
Listing another of those nine needs no model change; extending the contract's
xStock catalog requires a matching SDK release.

## Quotes and settlement

The model is shared with the [polling SDK](https://github.com/thogard785/thog-amm-poll-sdk#quote-methods).
It provides `quote_exact_input`, `quote_execution_exact_input`,
`quote_exact_output`, `limits` and `marginal_price`. For several sizes, use
`model.prepare(token_in, token_out)?` and quote the resulting pair. All amounts
are raw `U256` token units. Rebuild prepared pairs after each update.

Execution methods model an ordinary external caller/recipient without extra
fees. Pass the transaction's effective gas price. Defaults assume a cold
AuctionHandler; the `*_with_context` variants accept explicit account warmth.
Read-only quotes use 90% cap thresholds; execution uses full settlement caps.
Execution output also depends on the block base fee and whether prices were
updated in the execution block. For EIP-1559, effective gas price is
`min(maxFeePerGas, baseFee + maxPriorityFeePerGas)`. The SDK includes these
adjustments and price-age widening; see the
[exact rules](https://github.com/thogard785/thog-amm-integration-guide/blob/main/REFERENCE.md#exact-output-marginal-prices-and-settlement).
Simulate the full settlement route with its actual fee and account-warmth context.

`Quote.amount_in` is the amount passed to Thog. Execution `amount_out` is guaranteed
recipient credit at the quoted state/context: compare `received >= amount_out`,
not equality. For Buy it equals the target; actual xStock credit may be higher.
Read-only quotes omit execution friction. Measure funding received by your router
before quoting/passing input; see the [funding rules](https://github.com/thogard785/thog-amm-integration-guide/blob/main/THOGAMM_AGGREGATOR_INTEGRATION.md#funding-through-an-aggregator).
Share floors, xStock dust and direct/Buy rounding are included. For calldata and
slippage protection, see the [settlement guide](https://github.com/thogard785/thog-amm-integration-guide/blob/main/THOGAMM_AGGREGATOR_INTEGRATION.md#quotes-limits-and-settlement).

`at_block` changes age and execution context without fetching new inventory, policy or
stored multipliers. Independent quotes do not simulate successive fills.

## Errors and custom streams

A disconnect, missing block/log or malformed publication
returns an error and preserves the last published snapshot. Reconnect explicitly
with `connect()`. A registry change returns `Error::RegistryChanged`; reconnect
and rebuild prepared pairs. On `ContractUpgraded` or an unsupported format,
verify compatibility before using the matching release.

`EventSynchronizer::from_model` accepts a snapshot with a known finalized hash.
Feed `on_log` and `on_head(header, commitment)` in socket order. Complete
finalized ancestry is applied atomically, with the stored multiplier updates
from its logs. Proposed/Voted/Verified heads do not publish model changes. The
reducer performs no I/O; `pending_headers` exposes ancestry for custom consumers.

`model::events::log_filters(pool, wmon)` covers ordinary transfers, xStock
`TransferShares`, WMON and pool checkpoints. Use one ordered Monad connection,
including all commitment stages: each subsequent header closes the previous
log frame. Generic Ethereum `newHeads` does not establish this completeness.
Forced native transfers require account-state data or a fresh snapshot.

## Example and verification

[examples/subscribe.rs](examples/subscribe.rs) reads `THOGAMM_HTTP_RPC`,
`THOGAMM_WS_RPC`, `THOGAMM_PROXY`, `TOKEN_IN`, `TOKEN_OUT`, `AMOUNT_IN` and
`EFFECTIVE_GAS_PRICE_WEI`:

```sh
cargo run --locked --example subscribe
cargo test --locked
```

Tests compare all 272 directed pairs in the deployed 17-token registry against
contract results and replay finalized blocks against independently read
snapshots. They also cover subscription ordering, reorgs, stored-multiplier
updates, invalid values and no RPC reads after initialization, alongside the
historical quote and settlement corpus.
[Fixtures and reproduction instructions](https://github.com/thogard785/thog-amm-integration-guide#verification).

When upgrading from 0.5.0, remove multiplier-map arguments from `on_head`,
`apply_block` and `apply_owned_block`. `EventDrivenSdk` no longer retains a reader
or has a reader type parameter. `with_reader` still accepts a custom reader for
the initial snapshot. Reconnect after the contract upgrade with this release.
[MIT license](LICENSE).
