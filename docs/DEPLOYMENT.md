# Deployment and compatibility

## Supported implementation

Version 0.1.0 models ThogAMM **schema 6**, for up to 64 token slots. It supports
Monad and the pricing/storage semantics captured by the included Solidity
fixtures. An ABI-compatible upgrade can still change pricing semantics; schema
decoding alone does not establish compatibility with an arbitrary implementation.

Configure `THOGAMM_PROXY` from a verified deployment handoff. There is deliberately
no default pool address. A public SDK release does not deploy or upgrade a pool.
The original configured mainnet proxy rejected the required state read at block
106,930,619, so that historical address is not advertised here as ready to use.

The implementation must expose `getPoolData(uint8,uint8)` and accept `(0,64)`
as the complete currently listed registry. The returned schema-6 tuple includes
block number, base fee, parent hash, metadata, actual balances, all category risk
dependencies and sorted physical storage words. The Rust ABI is the decoding
source of truth. No preliminary `maxIndex()`, `decimals()` or balance read is
needed. Registry additions are included in the next snapshot or listing event.

## Contract behavior reflected by the model

- Books share token balances and portfolio risk; swaps can affect other pairs.
- Pauses, disabled sides and stale prices can make quotes unavailable.
- Risk-anchor freshness can affect quotes even for other token pairs.
- Execution friction depends on effective transaction gas price and transaction-
  local FastLane account warmth. Neither is guessed from historical logs.
- Native coin wrapping, fee-on-transfer tokens and silent rebases are not modeled
  as ordinary ERC-20 settlement.
- The SDK does not submit transactions, manage signing keys or estimate route gas.

## Before enabling a route

1. Confirm chain, proxy, implementation revision and the schema-6 ABI.
2. Read a complete image at a known block and compare representative local quotes
   with that implementation, including unavailable amounts and boundary values.
3. Exercise the actual router adapter: funding, allowance, recipient, minimum
   output, block deadline and effective gas price must match the intended trade.
4. Measure gas through that adapter on the target chain. MIP-8 storage pages and
   warm/cold transaction context affect cost; CPU quote time is not a gas estimate.
5. Pin SDK releases and agree on an upgrade notification contact/channel. A public
   GitHub release is a code distribution, not an operational service commitment.

The event transport additionally requires Monad's ordered commitment stream and
observable balance changes. Read its transport guide before selecting a provider.
The polling transport requires one `eth_call` per update and no subscriptions.
Both keep source block identity in the model so the caller can evaluate freshness.

## Upgrades

An `Upgraded` event makes the event client return `ContractUpgraded`; it does not
silently fetch a replacement model. Polling rejects an unsupported schema, but a
same-schema semantic upgrade must be managed by the integration's version policy.
Validate the new implementation, adopt its matching SDK revision, and explicitly
initialize a compatible state. An incompatible ABI or price model is an error to
resolve at its source, not a reason to add price buffers or hidden retries.
