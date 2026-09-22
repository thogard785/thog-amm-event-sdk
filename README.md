# thog-amm-event-sdk

[![Rust SDK](https://github.com/thogard785/thog-amm-event-sdk/actions/workflows/ci.yml/badge.svg)](https://github.com/thogard785/thog-amm-event-sdk/actions/workflows/ci.yml)

Rust SDK for deterministic ThogAMM quotes from **pushed Monad events**.
Initialization uses five subscriptions and one snapshot call. After startup,
updates perform **zero read RPCs** and publish complete finalized models.


Both SDKs share the exact integer `thogamm-model` engine. It models all 64 token
slots, actual balances, portfolio risk, pair spreads, inventory limits, pauses,
staleness and execution friction. Quotes return raw `U256` token amounts.
For the other update mechanism, see the [snapshot/polling SDK](https://github.com/thogard785/thog-amm-poll-sdk).

**Requires a compatible schema-6 ThogAMM deployment.** A published SDK does not
upgrade the live pool. Configure a verified proxy; see [deployment requirements](docs/DEPLOYMENT.md).
This library does not sign or submit transactions.

## Install

Use Rust **1.88 or newer** and pin the release in your application's `Cargo.toml`:

```toml
[dependencies]
event-driven-sdk = { git = "https://github.com/thogard785/thog-amm-event-sdk", tag = "v0.1.0" }
tokio = { version = "1.48", features = ["macros", "rt-multi-thread"] }
```

The GitHub repository is named `thog-amm-event-sdk`; the Rust crate remains `event-driven-sdk` and
imports as `event_driven_sdk`. Git installation resolves the workspace package.
Packages are distributed through GitHub, not crates.io. Commit your application's
lockfile for reproducible dependencies. No private repository access is needed.

The canonical model is in [thog-amm-poll-sdk](https://github.com/thogard785/thog-amm-poll-sdk/tree/v0.1.0/thogamm-model).
The event SDK pins that public release instead of duplicating pricing code.
Using both SDKs at matching tags resolves one shared model crate and compatible
`PoolModel`, amount and error types.

## Quickstart

Clone this repository and set the following environment variables. Substitute
your verified pool/token addresses and transaction context; the table does not
advertise any default proxy as currently deployed and compatible.

| Variable | Required value |
| --- | --- |
| `THOGAMM_HTTP_RPC` | Monad HTTP endpoint, used only for the single startup snapshot |
| `THOGAMM_WS_RPC` | Monad WebSocket endpoint supporting the ordering and commitments in the transport guide |
| `THOGAMM_PROXY` | Verified schema-6 pool proxy address |
| `TOKEN_IN`, `TOKEN_OUT` | Listed ERC-20 token addresses |
| `AMOUNT_IN` | Integer sell amount in raw token units; 1,000,000 is one token only when decimals = 6 |
| `EFFECTIVE_GAS_PRICE_WEI` | Intended execution's effective transaction gas price, not its fee cap |
| `FAST_LANE_HOT` | `true` or `false`, based on the actual execution context |

```sh
git clone https://github.com/thogard785/thog-amm-event-sdk.git
cd thog-amm-event-sdk
cargo run --locked -p event-driven-sdk --example execution-quotes
```

The example quotes the initial image, then each published update. It holds the
supplied execution context fixed for demonstration; a router must supply the
correct gas price and warmth for each intended transaction. Each update replaces
the model atomically; unavailable quotes are reported with their source block.
The program does not send a swap. The complete executable example is
[`execution-quotes.rs`](event-driven-sdk/examples/execution-quotes.rs).

```rust
use event_driven_sdk::{Config, ExecutionContext, EventDrivenSdk, U256};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let sell = std::env::var("TOKEN_IN")?.parse()?;
    let buy = std::env::var("TOKEN_OUT")?.parse()?;
    let amount: U256 = std::env::var("AMOUNT_IN")?.parse()?;
    let context = ExecutionContext {
        gas_price: std::env::var("EFFECTIVE_GAS_PRICE_WEI")?.parse()?,
        fast_lane_hot: std::env::var("FAST_LANE_HOT")?.parse()?,
    };
    let mut sdk = EventDrivenSdk::connect(
        std::env::var("THOGAMM_HTTP_RPC")?,
        std::env::var("THOGAMM_WS_RPC")?,
        std::env::var("THOGAMM_PROXY")?.parse()?,
        Config::default(),
    )
    .await?;

    loop {
        let model = sdk.model();
        match model.quote_execution_exact_input(sell, buy, amount, &context) {
            Ok(quote) => println!(
                "block={} amount_out={} last_posted_block={}",
                model.state().block.number,
                quote.amount_out,
                quote.last_posted_block
            ),
            Err(error) => eprintln!("block={} unavailable={error}", model.state().block.number),
        }
        sdk.next_update().await?;
    }
}
```

## Local quote surfaces

| API | Purpose |
| --- | --- |
| `quote_exact_input` | Match the read-only maker quote |
| `quote_execution_exact_input` | Include execution friction and settlement exposure checks |
| `quote_exact_output` | Solve the ERC-7815 Buy input with exact contract rounding |
| `marginal_price` | Rational raw output/input price |
| `limits` | Directional sell/buy bounds |
| `prepare` | Reuse decoded pair math across an amount ladder |
| `at_block` | Project aging/base-fee context while retaining source-state provenance |

These methods are synchronous and perform no network I/O. Clone the immutable
model to share a consistent snapshot across workers. Prepare each direction once
per snapshot, then reuse it for many amounts. Quoting does not mutate balances or
simulate successive fills; books share liquidity and portfolio risk.

## Update lifecycle

`EventDrivenSdk::connect` registers four log filters and `monadNewHeads` on one
socket, waits for an observed proposal to finalize, and makes one hash-pinned
state call. The reader is then dropped. `next_update()` consumes only pushed
notifications, applies complete finalized ancestry atomically, and returns an
`Update` with its block identity and the number of blocks applied.

The provider must preserve Monad's per-connection header/log/commitment ordering.
Ordinary Ethereum `newHeads` and logs alone do not prove a block's logs are complete.
State is finalized, so it can trail the newest proposal. New token listings arrive
through metadata/balance events without a reseed or resubscription.

For a caller-owned stream and finalized seed, `EventSynchronizer::from_model`
performs no I/O; feed `on_head` and `on_log` in their original connection order.
Disconnects, missing history, upgrades and malformed/accounting events are explicit
errors. There is no automatic reconnect, catch-up query or snapshot fallback.
See the [transport guide](docs/TRANSPORT.md) for initialization, failure handling
and event-accounting requirements.

## Documentation

- [Transport lifecycle and RPC contract](docs/TRANSPORT.md)
- [Quote API, units, execution context and shared liquidity](docs/QUOTING.md)
- [Settlement encoding and router responsibilities](docs/SETTLEMENT.md)
- [Deployment and upgrade compatibility](docs/DEPLOYMENT.md)
- [Troubleshooting](docs/TROUBLESHOOTING.md)
- [CPU performance and benchmark methodology](docs/PERFORMANCE.md)
- [Source and fixture provenance](docs/PROVENANCE.md)
- [Release/version policy](docs/RELEASES.md), [changelog](CHANGELOG.md), and [contributing](CONTRIBUTING.md)

Generate API documentation locally with `cargo doc --locked --workspace --no-deps --open`.
Run all tests with `cargo test --locked --workspace --all-targets`; no live RPC
access or funded wallet is needed by the tests. CI also runs release tests,
formatting, Clippy, documentation builds, and fixture/link checks.

## License

[MIT](LICENSE). Commercial use, modification and redistribution are permitted
subject to retaining the license and copyright notice. Third-party dependencies
retain their respective licenses. Repository issues are for SDK source support;
coordinate production operation and upgrades through your deployment's contact.
