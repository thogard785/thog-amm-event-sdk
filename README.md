# ThogAMM event SDK

Keep an in-memory ThogAMM quote model current through Monad subscriptions.
Version **0.4.0** supports **schema 8 / wire 8 / model 2** and all 13 listed tokens,
including **SPYx, QQQx, EWYx, NVDAx and TSLAx**. These are unwrapped xStocks with
18 decimals; [token addresses](https://github.com/thogard785/thog-amm-integration-guide/blob/main/THOGAMM_AGGREGATOR_INTEGRATION.md#listed-tokens).

Pool/share movements come from events. **Issuer multipliers are read at every
published block**, because a scheduled rebase can activate without a log. Quote
calculations themselves are synchronous and make no network calls.

- Monad mainnet, chain ID **143**.
- Pool: `0x80c74517BCC2D67fFE02D3ED886796272F647210`.
- HTTP RPC plus a WebSocket endpoint supporting `logs` and `monadNewHeads`.
- Rust **1.88+**, Cargo resolver **3**, Tokio. Commit your application's lockfile.

## Install and subscribe

```toml
[dependencies]
event-driven-sdk = { git = "https://github.com/thogard785/thog-amm-event-sdk", tag = "v0.4.0" }
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

`connect()` subscribes first, then loads a finalized pool/issuer snapshot at one
block hash. `next_update()` replays complete finalized blocks and reads current
multipliers in a single batch of five calls per block with today's registry.
It does not poll `getPoolData` or physical balances after startup. Use
`model.state().block.number` and `.hash` to identify the published state; it can
trail the proposed head. Keep consuming updates even when a quote rejects.

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
Share floors, xStock dust and direct/Buy rounding are included. For calldata and
slippage protection, see the [settlement guide](https://github.com/thogard785/thog-amm-integration-guide/blob/main/THOGAMM_AGGREGATOR_INTEGRATION.md#quotes-limits-and-settlement).

`at_block` ages an existing price without fetching new inventory, policy or
issuer state. Independent quotes do not simulate successive fills.

## Errors and custom streams

A disconnect, missing block/log, failed issuer read or malformed publication
returns an error and preserves the last published snapshot. Reconnect explicitly
with `connect()`. A registry change returns `Error::RegistryChanged`; reconnect
and rebuild prepared pairs. On `ContractUpgraded` or an unsupported format,
verify compatibility before using the matching release.

`EventSynchronizer::from_model` accepts a snapshot with a known finalized hash.
Feed `on_log` and `on_head(header, commitment, &multipliers_by_block_hash)` in
socket order. Before a finalized head, call `pending_headers(&header)` and supply
hash-pinned issuer multipliers for every returned block. The helper
`model::rpc::read_multipliers` can fetch them. Proposed/Voted/Verified heads need
no multiplier input. The reducer itself performs no I/O.

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

Tests match **39,936 contract cases** and 200 consecutive mainnet snapshots,
plus subscription ordering, reorgs and log-free issuer activations.
[Fixtures and reproduction instructions](https://github.com/thogard785/thog-amm-integration-guide#verification).
Upgrade from 0.3 with the new reader/issuer inputs; schema-7 state is incompatible.
[MIT license](LICENSE).
