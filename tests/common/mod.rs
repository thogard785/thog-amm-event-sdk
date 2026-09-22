#![allow(dead_code)]
use alloy_sol_types::{sol, SolCall, SolValue};
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
    let mut data = Vec::new();
    flate2::read::GzDecoder::new(&include_bytes!("contract.bin.gz")[..])
        .read_to_end(&mut data)
        .unwrap();
    Fixture::abi_decode_validate(&data).unwrap()
}
pub fn page(bytes: &Bytes) -> abi::PoolData {
    abi::getPoolDataCall::abi_decode_returns_validate(bytes).unwrap()
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
    PoolModel::new(PoolState::from_pages(proxy, h, vec![p]).unwrap()).unwrap()
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
    async fn call(
        &self,
        _to: Address,
        data: Bytes,
        block: thogamm_model::rpc::CallBlock,
    ) -> thogamm_model::Result<Bytes> {
        use thogamm_model::rpc::CallBlock;
        let mut c = self.0.lock().unwrap();
        c.calls.push((block, data.clone()));
        if c.fail_calls {
            return Err(thogamm_model::Error::Rpc("injected read failure".into()));
        }
        let hash = match block {
            CallBlock::Hash(hash) => hash,
            CallBlock::Number(number) => c.headers[&number].hash,
            _ => c.headers[&c.head].hash,
        };
        let p = c.states[&hash].clone();
        let call = abi::getPoolDataCall::abi_decode_validate(&data).unwrap();
        assert_eq!((call.startTokenIndex, call.stopTokenIndex), (0, 64));
        Ok(abi::getPoolDataCall::abi_encode_returns(&p).into())
    }
}
/// Same contract snapshot as model(), with no invented block hash for a latest call.
pub fn polled_model(proxy: Address, bytes: &Bytes) -> PoolModel {
    PoolModel::from_snapshot(proxy, page(bytes), None).unwrap()
}
pub fn model_header(model: &PoolModel) -> BlockHeader {
    model.state().block.clone().try_into().unwrap()
}
