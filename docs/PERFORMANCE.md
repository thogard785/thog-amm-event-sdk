# Quote performance

Measured 2026-09-21 on an Apple M1 Max, Rust 1.88.0 release profile with overflow
checks enabled. Both SDKs use the same shared model. These are **CPU timings**,
excluding network, JSON framing, EVM execution, finality and route construction.
They are sampled workloads, not worst-case latency bounds or mainnet guarantees.

| Operation | 8 tokens / 5 categories | 16 tokens / 9 categories | 64 tokens / 40 categories |
| --- | ---: | ---: | ---: |
| Direct exact input including preparation | 0.640 µs | 0.934 µs | 7.393 µs |
| Prepared exact input, preparation excluded | 0.173 µs | 0.220 µs | 0.256 µs |
| Direct exact output including preparation | 0.934 µs | 2.655 µs | 11.722 µs |
| Prepared exact output, preparation excluded | 0.467 µs | 1.921 µs | 3.905 µs |
| Preparation plus 16 exact-input amounts | 3.289 µs | 3.620 µs | 9.614 µs |
| ABI decode and model construction | 2.027 µs | 4.069 µs | 21.576 µs |

Preparation and successful quotes allocate zero heap objects. `PoolModel::clone`
and `at_block` share the immutable image through `Arc`, approximately 9.5 ns with
zero allocations in this fixture. Raw `PoolState::clone` still copies; error
formatting and update construction are not claimed to be allocation-free.

The event fixture covers 11 stages, growing from 16 to 18 tokens. Applying one
block averages 2.25 µs. Constructing a synchronizer and replaying all 11 finalized
blocks takes 51.3 µs with 247 allocations. That includes seed validation, supplied
log ingestion, commitment processing and publication; WebSocket decoding is excluded.

## Reproduce

The benchmark lives in the event repository and exercises the canonical model
from the pinned polling release as well as the event reducer:

```sh
git clone https://github.com/thogard785/thog-amm-event-sdk.git
cd thog-amm-event-sdk
git checkout v0.1.0
cargo bench --locked -p event-driven-sdk --bench performance
```

The harness cycles successful Solidity fixture cases in worlds 0, 1 and 3,
excluding paused/stale rejection timings. It reports the median of seven batch
means after warm-up, plus minimum and allocations per operation. The 64-token
world has 40 categories, nonzero signed inventory, covariance, pair spreads,
different decimals and nonlinear exact-output solves.

The 8/16-token ladders use indices 1→4 and input 1,000,000 divided by 1..16;
the 64-token ladder uses indices 2→37 and input 97,000,000 divided by 1..16.
Uncompressed snapshot ABI images are 5,312 / 10,624 / 49,408 bytes. JSON hex
approximately doubles that payload. Network/provider latency is additional.

Prepare only the directions your routing search needs, reuse them across amounts,
and acquire a new prepared pair when selecting another snapshot or projected block.
Exact-output timing varies with the nonlinear solve. CI verifies exact values and
transport behavior; CPU benchmarks are manual measurements, not a noisy CI gate.
