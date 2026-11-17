//! Fixtures recorded from the contract, shared by every test.
//!
//! Each fixture holds snapshots (the return data of `getPoolData`, plus each
//! xStock's shares and multiplier) and, per snapshot, calls with the result
//! the contract gave. Tests compare this crate's result with the contract's.
#![allow(dead_code)]
use serde_json::Value;
use std::{collections::BTreeMap, io::Read};
use thogamm::{
    abi::{self, SolCall},
    layout, Address, Bytes, Pool, Snapshot, XStock, B256, U256,
};

pub fn fixture(name: &str) -> Value {
    serde_json::from_slice(&gunzip(name)).unwrap()
}

pub fn gunzip(name: &str) -> Vec<u8> {
    let path = format!(
        "{}/fixtures/{name}",
        env!("CARGO_MANIFEST_DIR")
            .trim_end_matches("/model")
            .to_owned()
            + "/tests"
    );
    let mut bytes = Vec::new();
    flate2::read::GzDecoder::new(
        std::fs::File::open(&path).unwrap_or_else(|e| panic!("{path}: {e}")),
    )
    .read_to_end(&mut bytes)
    .unwrap();
    bytes
}

pub fn address(value: &Value) -> Address {
    value.as_str().unwrap().parse().unwrap()
}
pub fn uint(value: &Value) -> U256 {
    value.as_str().unwrap().parse().unwrap()
}
pub fn bytes(value: &Value) -> Bytes {
    value.as_str().unwrap_or("0x").parse().unwrap()
}

/// The pool data of one recorded snapshot. Captures made before pricing model
/// 3 differ from it only in how execution is surcharged, so they are relabeled
/// and reused for everything else.
pub fn pool_data(entry: &Value) -> abi::PoolData {
    let mut data =
        abi::getPoolDataCall::abi_decode_returns_validate(&bytes(&entry["snapshot"])).unwrap();
    let format = data
        .words
        .iter_mut()
        .find(|w| w.slot == layout::FORMAT.slot)
        .unwrap();
    let model = U256::from(0xff) << (layout::FORMAT.offset + 16);
    format.value = (format.value & !model) | (U256::from(3) << (layout::FORMAT.offset + 16));
    data
}

pub fn xstocks(entry: &Value) -> BTreeMap<Address, XStock> {
    serde_json::from_value(entry["shareBalances"].clone()).unwrap()
}

pub fn snapshot(pool: Address, entry: &Value) -> Snapshot {
    snapshot_from(pool, pool_data(entry), &xstocks(entry))
}

pub fn snapshot_from(
    pool: Address,
    data: abi::PoolData,
    xstocks: &BTreeMap<Address, XStock>,
) -> Snapshot {
    Snapshot::from_pool_data(pool, data, |token| xstocks.get(&token).cloned()).unwrap()
}

pub fn pool(pool: Address, entry: &Value) -> Pool {
    Pool::new(snapshot(pool, entry)).unwrap()
}

/// The 17-token pool as deployed, read at one block.
pub fn deployed() -> (Value, Pool) {
    let f = fixture("deployed-stored-multipliers.json.gz");
    let pool = pool(address(&f["proxy"]), &f["snapshots"][0]);
    (f, pool)
}

/// A deterministic stand-in for a block hash.
pub fn hash(number: u64) -> B256 {
    let mut bytes = [0u8; 32];
    bytes[24..].copy_from_slice(&number.to_be_bytes());
    B256::from(bytes)
}

/// A node that answers the snapshot call from one recorded snapshot.
pub mod node {
    use super::*;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    };
    use thogamm::{
        rpc::{BlockId, EthCall, MULTICALL3},
        Error, Result, WMON,
    };

    #[derive(Clone)]
    pub struct Node {
        pub pool: Address,
        pub state: Arc<Mutex<(abi::PoolData, BTreeMap<Address, XStock>)>>,
        pub requests: Arc<AtomicUsize>,
        pub fail: Arc<Mutex<bool>>,
    }

    impl Node {
        pub fn new(pool: Address, entry: &Value) -> Self {
            Self {
                pool,
                state: Arc::new(Mutex::new((pool_data(entry), xstocks(entry)))),
                requests: Arc::default(),
                fail: Arc::default(),
            }
        }
        pub fn requests(&self) -> usize {
            self.requests.load(Ordering::Relaxed)
        }
        pub fn set(&self, data: abi::PoolData) {
            self.state.lock().unwrap().0 = data;
        }
        pub fn data(&self) -> abi::PoolData {
            self.state.lock().unwrap().0.clone()
        }
    }

    impl EthCall for Node {
        async fn eth_call(&self, to: Address, data: Bytes, block: BlockId) -> Result<Bytes> {
            self.requests.fetch_add(1, Ordering::Relaxed);
            if *self.fail.lock().unwrap() {
                return Err(Error::Rpc("node unavailable".into()));
            }
            assert_eq!(to, MULTICALL3);
            let (data_now, xstocks) = self.state.lock().unwrap().clone();
            if let BlockId::Number(number) = block {
                assert_eq!(
                    U256::from(number),
                    data_now.blockNumber,
                    "this node holds one block"
                );
            }
            let calls = abi::aggregate3Call::abi_decode_validate(&data)
                .unwrap()
                .calls;
            let results = calls
                .iter()
                .map(|call| {
                    let selector: [u8; 4] = call.callData[..4].try_into().unwrap();
                    let (success, output) = if selector == abi::getPoolDataCall::SELECTOR {
                        assert_eq!(call.target, self.pool);
                        (true, abi::getPoolDataCall::abi_encode_returns(&data_now))
                    } else if selector == abi::storedMultiplierCall::SELECTOR {
                        assert_eq!(
                            call.target, self.pool,
                            "the multiplier is read from the pool"
                        );
                        let token = abi::storedMultiplierCall::abi_decode_validate(&call.callData)
                            .unwrap()
                            .token;
                        let multiplier = xstocks.get(&token).map_or(U256::ZERO, |x| x.multiplier);
                        (
                            true,
                            abi::storedMultiplierCall::abi_encode_returns(&multiplier),
                        )
                    } else if selector == abi::sharesOfCall::SELECTOR {
                        assert!(call.allowFailure, "ordinary tokens do not have sharesOf");
                        match xstocks.get(&call.target) {
                            Some(xstock) => {
                                (true, abi::sharesOfCall::abi_encode_returns(&xstock.shares))
                            }
                            // WMON's fallback accepts any call and returns nothing.
                            None if call.target == WMON => (true, Vec::new()),
                            None => (false, Vec::new()),
                        }
                    } else {
                        panic!("unexpected call {}", call.callData)
                    };
                    abi::CallResult {
                        success,
                        returnData: output.into(),
                    }
                })
                .collect::<Vec<_>>();
            Ok(abi::aggregate3Call::abi_encode_returns(&results).into())
        }
    }
}
