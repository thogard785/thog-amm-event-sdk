# Changelog

## 0.1.0 — 2026-09-21

- Initial public `thog-amm-event-sdk` release under the MIT license.
- Rust 1.88, schema-6 state, all 64 token slots and deterministic integer quoting.
- Exact-input/exact-output, marginal price and limit surfaces; shared portfolio
  risk, balances, staleness, pauses and explicit execution context.
- Prepared pairs and immutable shared snapshots for amount ladders/concurrency.
- Complete installation, transport, settlement and compatibility documentation.
- Synthetic Solidity regression fixtures, local transport tests and public CI.

Publishing this SDK does not deploy a compatible pool. See
[deployment requirements](docs/DEPLOYMENT.md) before connecting a live integration.
