# Contributing

Use Rust 1.88 or newer. Run from the repository root:

```sh
python3 scripts/check_docs.py
cargo fmt --all -- --check
cargo test --locked --workspace --all-targets
cargo test --locked --release --workspace --all-targets
cargo clippy --locked --workspace --all-targets -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --locked --workspace --no-deps
```

Tests use synthetic committed Solidity fixtures and local HTTP/WebSocket servers.
They require no mainnet account, API key, private contract repository or funded
wallet. First-time dependency downloads use Cargo's normal registry/Git access.
The fixture exporter itself requires the separate contract development tree;
see [provenance](docs/PROVENANCE.md) for what public CI does and does not reproduce.

Quote correctness must match the target contract, including rejected inputs,
rounding, overflow and transaction context. Fix the root mismatch; do not mask it
with local buffers, silent reseeds, catch-up requests or no-send filters. Preserve
the one-read polling and zero-operating-read event contracts in transport tests.

The shared `thogamm-model` is maintained in the polling repository. Propose math,
ABI and core-state changes there. The event repository pins a tagged model release
instead of keeping another copy. Coordinate contract behavior, fixtures and model
version updates; do not silently change pricing behind an existing tag.

Keep changes focused, include the relevant parity/transport evidence, and update
the integration documentation when public behavior changes. Do not commit keys,
provider credentials, production data dumps or build artifacts. Contributions
are made under the repository's [MIT license](LICENSE).
