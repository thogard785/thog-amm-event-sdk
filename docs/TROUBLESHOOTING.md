# Troubleshooting

| Symptom | Meaning and action |
| --- | --- |
| Fresh Git install selects a dependency requiring Rust newer than 1.88 | Configure the consuming application with `rust-version = "1.88"` and resolver 3 as in the README. The SDK's lockfile does not control another application's dependency resolution. Review existing lock entries if they were selected under another policy. |
| `eth_call` reverts or returns empty bytes | Verify the chain, proxy, schema-6 implementation and `(0,64)` interface. Publication of the SDK does not upgrade the contract. |
| `UnsupportedSchema` / invalid image | Use the SDK release matching the implementation; pass the complete raw ABI return bytes, not a JSON envelope or a partial page. |
| Execution friction differs from settlement | Supply the effective gas price for the landing block and the actual FastLane warmth at the contract's check. Preserve the nonzero simulation gas price on the snapshot call. |
| Snapshot base fee is zero unexpectedly | Retain `gasPrice: "0x1"` in state-read requests. On Monad, omitting fee fields can zero the simulated BASEFEE. Do not add a header query to compensate. |
| `ProviderBehind` | A polling response reported an older block. That invocation has made one call and retains the old model. Resolve the provider's consistency issue; any later update is an explicit new call. |
| `Unavailable` / `LimitExceeded` | The contract would reject that quote surface/size in the attached context. Inspect the source block, amounts, sides, staleness, balances and exposure bounds. |
| `Arithmetic` | Solidity-equivalent arithmetic could not produce a valid quote, or event balance accounting diverged. Diagnose the matching contract operation/token event instead of adding slop. |
| Event startup times out | The provider must support all four log filters and `monadNewHeads` on one connection, including proposal/finality notifications and EIP-1898 startup calls. |
| `Discontinuous`, ABI or event accounting error | Keep the old image's block stamp; the stream is failed. Repair the root cause, then explicitly initialize a new client or replay a complete persisted stream. No automatic recovery read occurs. |
| `ContractUpgraded` | Verify the new implementation and use a compatible model before explicit reinitialization. |
| A saved model still gives old quotes | Snapshots are immutable. Acquire the newly published model and prepare its pair again. |

Polling's `refresh()` has no timer; `next_update()` waits its configured interval
and then makes one call, including unchanged state and errors. A constructor's
initial read is separate from a later refresh. The event constructor makes one
startup read and its operating client has no HTTP reader.

When reporting an issue, include the SDK tag/commit, implementation revision,
block number/hash, quote surface, raw amounts, context and the exact error.
Remove RPC credentials and wallet keys. Synthetic reproduction data is preferred.
Do not expose a private provider URL in logs or a public issue.
