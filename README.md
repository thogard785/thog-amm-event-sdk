# ThogAMM event SDK

Keep an in-memory ThogAMM quote model current through Monad subscriptions.
Version **0.5.0** supports **schema 8 / wire 8 / model 3** and all 13 listed tokens,
including **SPYx, QQQx, EWYx, NVDAx and TSLAx**. These are unwrapped xStocks with
18 decimals; [token addresses](https://github.com/thogard785/thog-amm-integration-guide/blob/main/THOGAMM_AGGREGATOR_INTEGRATION.md#listed-tokens).

Pool/share movements come from events. **Issuer multipliers are read at every
published block**, because a scheduled rebase can activate without a log. Quote
calculations themselves are synchronous and make no network calls.

- Monad mainnet, chain ID **143**.
- Pool: `0x80c74517BCC2D67fFE02D3ED886796272F647210`.
- HTTP RPC plus a WebSocket endpoint supporting `logs` and `monadNewHeads`.
- Rust **1.88+**, Cargo resolver **3**, Tokio. Commit your application's lockfile.

Model 3 was activated at Monad block **117177785** in
[the proxy upgrade transaction](https://monadscan.com/tx/0x962e87a568d98fba303d7077f6b51242599c07446799b87f117cbd14065214d2).
Use **v0.5.0** for model 3; v0.4.1 is retained for historical model-2 snapshots.
The SDK rejects mismatched model identities. Verify the selected snapshot's
identity and commit your application's `Cargo.lock`.

The coordinated upgrade pauses both maker liquidity and taker execution while
the shared operator backend is stopped, upgraded and restarted. The operator
must confirm the new risk configuration and fresh valuations before unpausing;
the migration preserves the previous widening. Reconnect using this matching SDK
and obtain a complete snapshot after the upgrade. See
[upgrade availability](https://github.com/thogard785/thog-amm-integration-guide/blob/main/THOGAMM_AGGREGATOR_INTEGRATION.md#upgrade-availability).

## Install and subscribe

```toml
[dependencies]
event-driven-sdk = { git = "https://github.com/thogard785/thog-amm-event-sdk", tag = "v0.5.0" }
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
Active listings are dynamic; share accounting uses the contract's
[nine compiled xStock addresses](https://github.com/thogard785/thog-amm-poll-sdk/blob/v0.4.1/model/src/shares.rs).
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
Execution applies a dynamic output reduction. Evaluate the following rows in
order using the transaction's **effective** gas price and the current block's
base fee; every gas-price comparison is strictly greater-than.

| First matching condition | Raw defense | Defense after a valuation update earlier in this block |
| --- | ---: | ---: |
| FastLane AuctionHandler is already warm | 12 bp | 2 bp |
| `gasPrice > 2.5 * baseFee` | 12 bp | 2 bp |
| `gasPrice > 2 * baseFee` | 8 bp | 1 bp |
| `gasPrice > 1.5 * baseFee` | 6 bp | 0.5 bp |
| Otherwise | 0 bp | 0 bp |

The same-block column applies exactly when the latest accepted valuation's
`postedBlock` equals the execution block: `max(rawDefense / 4 - 1 bp, 0)`.
A risk, wrapper, cap or inventory update alone does not qualify. The raw defense
applies when the latest valuation is from an earlier block. Warmth and gas-price
triggers do not stack. With zero base fee, any positive gas price selects 12 bp;
a cold zero-price transaction selects zero. Fractional thresholds are exact,
including odd base fees; implementations compare differences to avoid overflow.

The configured age widening is **0.5 bp per elapsed block** without a valuation
update (`widen_per_block_bps: 0.50`, encoded as 50 in 0.01 bp units). It adds to
the pair's spread and affects both read-only and execution quotes. Execution
defense is a separate multiplicative output reduction, not another age term.

The SDK's default execution methods assume a cold AuctionHandler; the
`*_with_context` methods accept explicit warmth. For EIP-1559, effective gas price
is `min(maxFeePerGas, baseFee + maxPriorityFeePerGas)`. `at_block(number, base_fee)`
sets both age and the block/base-fee context used by this policy; it cannot predict
an intervening valuation publication, trade or inventory change.

`Quote.amount_in` is the amount passed to Thog. Execution `amount_out` is guaranteed
recipient credit at the quoted state/context: compare `received >= amount_out`,
not equality. For Buy it equals the target; actual xStock credit may be higher.
Read-only quotes omit execution friction. Measure funding received by your router
before quoting/passing input; see the [funding example](https://github.com/thogard785/thog-amm-integration-guide/blob/main/THOGAMM_AGGREGATOR_INTEGRATION.md#funding-through-an-aggregator).
Share floors, xStock dust and direct/Buy rounding are included. For calldata and
slippage protection, see the [settlement guide](https://github.com/thogard785/thog-amm-integration-guide/blob/main/THOGAMM_AGGREGATOR_INTEGRATION.md#quotes-limits-and-settlement).

`at_block` changes age and execution context without fetching new inventory, policy or
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

Tests check 32,760 execution cases against the newly compiled model-3 contract
on a local Monad fork, plus 25,896 historical read-only cases and 200 blocks of
event replay with a test-only model identity migration. Historical expected
results remain unchanged and do not validate the new defense. Tests also cover
subscription ordering, reorgs and log-free issuer activations.
[Fixtures and reproduction instructions](https://github.com/thogard785/thog-amm-integration-guide#verification).
Upgrade from 0.4.1 together with the contract migration; model-2 and schema-7
snapshots are incompatible. Reconnect after the upgrade with this matching SDK.
[MIT license](LICENSE).
