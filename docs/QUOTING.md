# Deterministic quoting


All token amounts are unsigned 256-bit **raw base units**. Decimals come from
the snapshot. There is no floating-point conversion in pricing.

| Operation | API | Semantics |
| --- | --- | --- |
| Compare with the contract's read-only maker quote | `quote_exact_input` | Output balance checked; execution friction and settlement exposure checks excluded |
| Price an executable exact-input leg | `quote_execution_exact_input` | Adds transaction friction and signed exposure headroom checks |
| Solve input for an exact output | `quote_exact_output` | Exact ERC-7815 Buy integer solver, including friction and inventory constraints |
| Marginal price | `marginal_price` | Rational raw output/input units; matches the read-only price surface |
| Capacity | `limits` | Live sell/buy bounds, not a reservation or a promise every interior amount is executable |

For settlement pricing, use the execution method and provide the **effective**
transaction gas price, not the maximum fee cap. Friction widens output once by
10 bps when `tx.gasprice > 2 * block.basefee` or the FastLane AuctionHandler is
already warm at the check. Exactly twice base fee does not trigger it. Account
warmth is transaction-local and cannot be learned from the pool state or logs;
the adapter must know its transaction context, including earlier calls and any
access list. Neither trigger is silently guessed by the SDK.

Quotes default to the observed snapshot block. To evaluate the same prices and
inventory at a later landing block, use `model.at_block(number, base_fee)?`,
then prepare a pair from that projected model. Aging/spread widening and stale
price rules advance; the source snapshot identity is preserved. This does not
predict intervening trades, price updates, or the next block's base fee.

Repeated amounts are **alternative trades against the same initial state**.
Quotes do not mutate balances or exposures. ThogAMM has shared balances and
portfolio risk across pairs; pair A's liquidity must not be counted again as an
independent book for pair B. For a route that revisits the same ThogAMM instance,
the aggregate route needs sequential state/execution simulation. The SDK does
not currently expose a hypothetical `apply_fill` API. Independent quotes must
not be represented as a simulation of a sequence of fills.

The SDK returns explicit unavailable/limit/arithmetic errors. Preserve the
snapshot block in diagnostics and discard a prepared pair when selecting a new
snapshot or projected context. Paused/stale state is a valid snapshot that can
reject quotes; it does not trigger hidden retries or conservative price buffers.


## Reuse and concurrency

`PoolModel` is immutable. Clone it to give a worker an owned `Arc` handle to the
same image; successful quote calls allocate no heap memory and take no quote
lock. Prepare one direction once for an amount ladder:

```rust
let snapshot = sdk.model().clone();
let pair = snapshot.prepare(sell_token, buy_token)?;
for amount in raw_amounts {
    match pair.quote_execution_exact_input(amount, &context) {
        Ok(quote) => { /* consume quote.amount_out and quote.last_posted_block */ }
        Err(error) => { /* this amount is unavailable in the attached context */ }
    }
}
```

`PoolModel` and `PreparedPair` are `Send + Sync`. A prepared pair borrows its
model; keep the owning model alive and prepare again for a new snapshot or block
projection. An old model remains internally consistent after an SDK update.

The direct address-based methods prepare on each invocation. Prepared methods
have the same semantics and omit the sell/buy arguments. Token addresses and
decimals are available in `model.state().tokens`; `max_index()` is local and
requires no call. Do not assume token positions identify a desired trading pair.
