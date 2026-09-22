# Source and fixture provenance

The initial public release is extracted from the ThogAMM SDK implementation at
source revision `50cf4d4ea9fda202ee41e087b636e6bc86543464` (2026-09-21).
The release changes packaging and documentation; quote and reducer algorithms
are preserved. Only SDK code, synthetic test fixtures, and SDK documentation
are included. This repository has a fresh public history.

## Shared model

The canonical `thogamm-model` crate lives in
[state-poll-sdk](https://github.com/thogard785/thog-amm-poll-sdk/tree/v0.1.0/thogamm-model).
The event SDK depends on that exact tagged Git source. Both SDKs therefore
resolve the same model crate when their matching release tags are used together.
The committed Cargo lockfile records the resolved source revision.

## Solidity-generated test data

`fixtures/contract.bin.gz` is a deterministic, locally generated ABI fixture.
It contains synthetic token balances and prices, mock contract addresses, quote
results and captured event transitions. No live credentials or account keys
are embedded. The decompressed payload is 3,757,824 bytes.

- Compressed SHA-256: `981e0f2c05b4b9a5f2dcbae24750a699c735b5b60d7f9461a2463e317c88c881`
- ABI payload SHA-256: `18e4099e459f9692ce1fa22bb4b86672568fa7b77c920c0d8100ab7d5febaf90`
- Generator toolchain: Foundry 1.7.1, Solidity 0.8.28.
- Coverage: 5,120 quote cases, 11 event stages, and 64 actual single-token exports.

The original exporter executed the full Solidity implementation and its test
harness locally; it broadcast no transaction. That contract development tree is
not included here. Public CI validates the committed fixture against the Rust
model and verifies its checksum; it does not claim to regenerate Solidity without
the contract sources. Quote cases include valid values and expected reverts.
The fixture is a regression corpus, not an independent security audit or proof
of compatibility with every future contract implementation.

To inspect the ABI schema used to decode this fixture, read `fixtures/support.rs`.
The supported public contract ABI is in the canonical model's `src/abi.rs`.
