#![allow(dead_code)]
use alloy_sol_types::{sol, SolCall};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};
use thogamm_model::{abi, Address, BlockHeader, Bytes, PoolModel, PoolState, RpcLog, B256, U256};

sol! {
    struct Sample { uint8 kind; uint8 inputIndex; uint8 outputIndex; uint256 amount; uint256 gasPrice; bool fastLaneHot; bool success; bytes result; }
    struct SampleSet { bytes snapshot; Sample[] quotes; }
    struct CapturedLog { bytes32[] topics; bytes data; address emitter; }
    struct EventSet { bytes beforeState; bytes afterState; CapturedLog[] logs; }
    struct Fixture { address proxy; uint256 baseFee; SampleSet[] samples; EventSet[] events; bytes[] singleTokenPages; }
    struct ContractFraction { uint256 numerator; uint256 denominator; }
    struct Trade { uint256 calculatedAmount; uint256 gasUsed; ContractFraction marginalPriceAfterTrade; }
}
pub fn fixture() -> Fixture {
    use std::io::Read;
    let read = |data: &[u8]| {
        let mut bytes = Vec::new();
        flate2::read::GzDecoder::new(data)
            .read_to_end(&mut bytes)
            .unwrap();
        serde_json::from_slice::<serde_json::Value>(&bytes).unwrap()
    };
    let quotes = read(include_bytes!("deployed-v8.json.gz"));
    let events = read(include_bytes!("events-v8.json.gz"));
    let normalize = |encoded: &serde_json::Value| {
        let bytes: Bytes = encoded.as_str().unwrap().parse().unwrap();
        let mut p = page(&bytes);
        p.parentBlockHash = hash(p.blockNumber.to::<u64>() - 1, 0);
        Bytes::from(abi::getPoolDataCall::abi_encode_returns(&p))
    };
    let blocks = events["blocks"].as_array().unwrap();
    Fixture {
        proxy: quotes["proxy"].as_str().unwrap().parse().unwrap(),
        baseFee: quotes["snapshots"][0]["baseFeePerGas"]
            .as_str()
            .unwrap()
            .parse()
            .unwrap(),
        samples: quotes["snapshots"]
            .as_array()
            .unwrap()
            .iter()
            .map(|b| SampleSet {
                snapshot: normalize(&b["snapshot"]),
                quotes: b["cases"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|c| Sample {
                        kind: match c["method"].as_str().unwrap() {
                            "quote" => 0,
                            "price" => 1,
                            "limits" => 2,
                            _ => unreachable!(),
                        },
                        inputIndex: c["inputIndex"].as_u64().unwrap() as u8,
                        outputIndex: c["outputIndex"].as_u64().unwrap() as u8,
                        amount: c["amount"].as_str().unwrap().parse().unwrap(),
                        gasPrice: U256::ZERO,
                        fastLaneHot: false,
                        success: c["success"].as_bool().unwrap(),
                        result: c["result"].as_str().unwrap_or("0x").parse().unwrap(),
                    })
                    .collect(),
            })
            .collect(),
        events: blocks
            .windows(2)
            .map(|pair| EventSet {
                beforeState: normalize(&pair[0]["snapshot"]),
                afterState: normalize(&pair[1]["snapshot"]),
                logs: pair[1]["logs"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|l| CapturedLog {
                        emitter: l["address"].as_str().unwrap().parse().unwrap(),
                        data: l["data"].as_str().unwrap().parse().unwrap(),
                        topics: serde_json::from_value(l["topics"].clone()).unwrap(),
                    })
                    .collect(),
            })
            .collect(),
        singleTokenPages: Vec::new(),
    }
}
pub fn page(bytes: &Bytes) -> abi::PoolData {
    historical_page(abi::getPoolDataCall::abi_decode_returns_validate(bytes).unwrap())
}
/// Reuse immutable model-2 captures for unchanged read-only and event semantics.
/// This test-only migration changes the model byte, never expected quote results.
/// Execution-policy tests use separately captured model-3 contract outputs.
pub fn historical_page(mut p: abi::PoolData) -> abi::PoolData {
    let word = p.words.iter_mut().find(|w| w.slot == 8).unwrap();
    let version = (word.value >> 176usize) & U256::from(255);
    assert!(version == U256::from(2) || version == U256::from(3));
    word.value = (word.value & !(U256::from(255) << 176usize)) | (U256::from(3) << 176usize);
    p
}
pub fn hash(number: u64, branch: u8) -> B256 {
    let mut bytes = [0u8; 32];
    bytes[0] = branch;
    bytes[24..].copy_from_slice(&number.to_be_bytes());
    B256::from(bytes)
}
pub fn header(number: u64, base_fee: U256) -> BlockHeader {
    BlockHeader {
        number,
        hash: hash(number, 0),
        parent_hash: hash(number - 1, 0),
        base_fee_per_gas: base_fee,
    }
}
pub fn model(proxy: Address, base_fee: U256, bytes: &Bytes) -> PoolModel {
    let p = page(bytes);
    let h = header(p.blockNumber.to::<u64>(), base_fee);
    PoolModel::new(PoolState::from_pages(proxy, h, vec![p.clone()], shares_for(&p)).unwrap())
        .unwrap()
}
pub fn logs(event: &EventSet, base_fee: U256) -> Vec<RpcLog> {
    let p = page(&event.afterState);
    let h = header(p.blockNumber.to::<u64>(), base_fee);
    event
        .logs
        .iter()
        .enumerate()
        .map(|(i, l)| RpcLog {
            address: l.emitter,
            topics: l.topics.clone(),
            data: l.data.clone(),
            block_hash: h.hash,
            block_number: h.number,
            transaction_index: 0,
            log_index: i as u64,
            removed: false,
        })
        .collect()
}
#[derive(Default)]
pub struct Chain {
    pub head: u64,
    pub headers: BTreeMap<u64, BlockHeader>,
    pub states: BTreeMap<B256, abi::PoolData>,
    pub calls: Vec<(thogamm_model::rpc::CallBlock, Bytes)>,
    pub fail_calls: bool,
}
#[derive(Clone, Default)]
pub struct MockRpc(pub Arc<Mutex<Chain>>);
impl MockRpc {
    pub fn insert(&self, p: abi::PoolData, base_fee: U256) {
        let h = header(p.blockNumber.to::<u64>(), base_fee);
        let mut c = self.0.lock().unwrap();
        c.head = h.number;
        c.states.insert(h.hash, p);
        c.headers.insert(h.number, h);
    }
    pub fn from_events(f: &Fixture) -> Self {
        let rpc = Self::default();
        rpc.insert(page(&f.events[0].beforeState), f.baseFee);
        for event in &f.events {
            rpc.insert(page(&event.afterState), f.baseFee);
        }
        rpc.0.lock().unwrap().head = page(&f.events[0].beforeState).blockNumber.to::<u64>();
        rpc
    }
    pub fn call_count(&self) -> usize {
        self.0.lock().unwrap().calls.len()
    }
}
#[async_trait::async_trait]
impl thogamm_model::rpc::ChainReader for MockRpc {
    async fn header(
        &self,
        block: thogamm_model::rpc::CallBlock,
    ) -> thogamm_model::Result<BlockHeader> {
        use thogamm_model::rpc::CallBlock;
        let c = self.0.lock().unwrap();
        let h = match block {
            CallBlock::Hash(hash) => c.headers.values().find(|h| h.hash == hash).unwrap(),
            CallBlock::Number(number) => &c.headers[&number],
            _ => &c.headers[&c.head],
        };
        Ok(h.clone())
    }
    async fn calls(
        &self,
        calls: Vec<(Address, Bytes)>,
        block: thogamm_model::rpc::CallBlock,
    ) -> thogamm_model::Result<Vec<Bytes>> {
        use thogamm_model::rpc::CallBlock;
        let mut c = self.0.lock().unwrap();
        let hash = match block {
            CallBlock::Hash(hash) => hash,
            _ => panic!("all state reads must be hash-pinned"),
        };
        let p = c.states[&hash].clone();
        let shares = shares_for(&p);
        calls
            .into_iter()
            .map(|(to, data)| {
                c.calls.push((block, data.clone()));
                if c.fail_calls {
                    return Err(thogamm_model::Error::Rpc("injected read failure".into()));
                }
                let bytes = if data.starts_with(&abi::getPoolDataCall::SELECTOR) {
                    let call = abi::getPoolDataCall::abi_decode_validate(&data).unwrap();
                    assert_eq!((call.startTokenIndex, call.stopTokenIndex), (0, 28));
                    abi::getPoolDataCall::abi_encode_returns(&p)
                } else if data.starts_with(&abi::sharesOfCall::SELECTOR) {
                    abi::sharesOfCall::abi_encode_returns(&shares[&to].shares)
                } else if data.starts_with(&abi::getCurrentMultiplierCall::SELECTOR) {
                    abi::getCurrentMultiplierCall::abi_encode_returns(
                        &abi::getCurrentMultiplierReturn {
                            multiplier: shares[&to].multiplier,
                            activation: U256::ZERO,
                            period: U256::ZERO,
                        },
                    )
                } else {
                    panic!("unexpected calldata {data}")
                };
                Ok(bytes.into())
            })
            .collect()
    }
}
pub fn share_images() -> &'static BTreeMap<u64, thogamm_model::ShareBalances> {
    static IMAGES: std::sync::OnceLock<BTreeMap<u64, thogamm_model::ShareBalances>> =
        std::sync::OnceLock::new();
    IMAGES.get_or_init(|| {
        use std::io::Read;
        let mut images = BTreeMap::new();
        for data in [
            include_bytes!("events-v8.json.gz").as_slice(),
            include_bytes!("deployed-v8.json.gz").as_slice(),
        ] {
            let mut bytes = Vec::new();
            flate2::read::GzDecoder::new(data)
                .read_to_end(&mut bytes)
                .unwrap();
            let f: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
            for b in f
                .get("blocks")
                .or_else(|| f.get("snapshots"))
                .unwrap()
                .as_array()
                .unwrap()
            {
                let encoded: Bytes = b["snapshot"].as_str().unwrap().parse().unwrap();
                images.insert(
                    page(&encoded).blockNumber.to::<u64>(),
                    serde_json::from_value(b["shareBalances"].clone()).unwrap(),
                );
            }
        }
        images
    })
}
pub fn shares_for(p: &abi::PoolData) -> thogamm_model::ShareBalances {
    // Synthetic publication fixtures advance only block context, retaining the seed holdings.
    let candidates = share_images();
    let valid = |shares: &&thogamm_model::ShareBalances| {
        p.tokens.iter().enumerate().all(|(i, t)| {
            shares.get(&t.token).is_none_or(|b| {
                thogamm_model::math::mul_div(
                    b.shares,
                    b.multiplier,
                    thogamm_model::math::pow10(18),
                    false,
                )
                .unwrap()
                    == p.portfolioBalances[i]
            })
        })
    };
    candidates
        .get(&p.blockNumber.to::<u64>())
        .filter(valid)
        .or_else(|| candidates.values().find(valid))
        .expect("fixture issuer state not found")
        .clone()
}
pub fn multipliers_for(m: &PoolModel) -> thogamm_model::Multipliers {
    m.state()
        .share_balances
        .iter()
        .map(|(a, b)| (*a, b.multiplier))
        .collect()
}
pub fn multiplier_blocks() -> BTreeMap<B256, thogamm_model::Multipliers> {
    share_images()
        .iter()
        .map(|(n, shares)| {
            (
                hash(*n, 0),
                shares.iter().map(|(a, b)| (*a, b.multiplier)).collect(),
            )
        })
        .collect()
}
/// Expected snapshot for the mock reader’s deterministic header.
pub fn polled_model(proxy: Address, bytes: &Bytes) -> PoolModel {
    PoolModel::from_snapshot(
        proxy,
        page(bytes),
        Some(hash(page(bytes).blockNumber.to::<u64>(), 0)),
        shares_for(&page(bytes)),
    )
    .unwrap()
}
pub fn model_header(model: &PoolModel) -> BlockHeader {
    model.state().block.clone().try_into().unwrap()
}
