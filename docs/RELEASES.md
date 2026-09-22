# Releases and versioning

SDKs are distributed as **GitHub source repositories and tagged releases**.
The Rust packages are not published to crates.io. Install with the exact Git tag
shown in the README and commit the consuming application's Cargo.lock.

`v0.1.0` is the initial schema-6 release. The pre-1.0 API can evolve between
releases; do not follow an unpinned branch for production quoting. A pool upgrade
that changes pricing can require a new SDK even when the tuple's schema number
does not change. Release notes must identify supported contract behavior.

## Maintainer release procedure

1. Review source changes, the contract compatibility statement and fixture origin.
2. Run all checks in CONTRIBUTING.md and verify the snapshot/event request counts.
3. Release the canonical model and polling SDK first, with an immutable version tag.
4. Update the event SDK's `thogamm-model` Git tag and Cargo.lock, then run event tests.
5. Release the event SDK with its supported model revision stated in release notes.
6. Verify a consumer can use both Git dependencies with one shared model type.
7. Notify integrators through the operational channel agreed for their deployment.

Do not retarget published tags or overwrite releases to hide an incompatible
change. New listing support within the existing schema does not require a new
SDK release; schema/pricing semantics changes can. The repository's issue tracker
is for SDK source issues and does not establish a production support SLA.
