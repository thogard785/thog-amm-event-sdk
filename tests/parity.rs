use alloy_sol_types::{sol, SolCall, SolValue};
use std::io::Read;
use thogamm_model::{abi, Address, Bytes, PoolModel, U256};
sol! { struct ContractFraction { uint256 numerator; uint256 denominator; } }
#[test]
fn all_deployed_snapshots_match_exactly() {
    let mut bytes = Vec::new();
    flate2::read::GzDecoder::new(&include_bytes!("common/deployed-v7.json.gz")[..])
        .read_to_end(&mut bytes)
        .unwrap();
    let f: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let proxy: Address = f["proxy"].as_str().unwrap().parse().unwrap();
    let mut checked = 0;
    let mut success = 0;
    let mut volume = 0;
    let mut refunds = 0;
    for s in f["snapshots"].as_array().unwrap() {
        let encoded: Bytes = s["snapshot"].as_str().unwrap().parse().unwrap();
        let page = abi::getPoolDataCall::abi_decode_returns_validate(&encoded).unwrap();
        let model = PoolModel::from_snapshot(proxy, page, Some(thogamm_model::B256::ZERO)).unwrap();
        let sdk = event_driven_sdk::EventSynchronizer::from_model(model).unwrap();
        let model = sdk.model().unwrap();
        for (i, c) in s["cases"].as_array().unwrap().iter().enumerate() {
            let a = model.state().tokens[c["inputIndex"].as_u64().unwrap() as usize].token;
            let b = model.state().tokens[c["outputIndex"].as_u64().unwrap() as usize].token;
            let amount: U256 = c["amount"].as_str().unwrap().parse().unwrap();
            let got = match c["method"].as_str().unwrap() {
                "quote" => model.quote_exact_input(a, b, amount).map(|q| {
                    volume += usize::from(!q.volume_cost_usd_wad.is_zero());
                    refunds += usize::from(q.wrapper_cost_usd_wad.is_negative());
                    (q.amount_out, U256::from(q.last_posted_block)).abi_encode()
                }),
                "price" => model.marginal_price(a, b, amount).map(|p| {
                    vec![ContractFraction {
                        numerator: p.numerator,
                        denominator: p.denominator,
                    }]
                    .abi_encode()
                }),
                "limits" => model.limits(a, b).map(|l| vec![l.sell, l.buy].abi_encode()),
                _ => panic!("unknown case"),
            };
            assert_eq!(
                got.is_ok(),
                c["success"].as_bool().unwrap(),
                "block {} case {i} {}->{} amount {amount}: {got:?}; contract {c}",
                s["blockNumber"],
                c["inputIndex"],
                c["outputIndex"]
            );
            if let Ok(got) = got {
                let expected: Bytes = c["result"].as_str().unwrap().parse().unwrap();
                assert_eq!(
                    got,
                    expected.as_ref(),
                    "block {} case {i} {}",
                    s["blockNumber"],
                    c["method"]
                );
                success += 1;
            }
            checked += 1;
        }
    }
    assert_eq!(checked, 2016);
    assert_eq!(success, 1308);
    assert!(volume > 0);
    assert!(refunds > 0);
    println!("{checked} deployed cases: {success} exact results, {} reverts, {volume} volume charges, {refunds} net wrapper refunds",checked-success);
}

#[test]
fn replay_two_hundred_deployed_blocks_matches_every_snapshot() {
    let mut bytes = Vec::new();
    flate2::read::GzDecoder::new(&include_bytes!("common/events-v7.json.gz")[..])
        .read_to_end(&mut bytes)
        .unwrap();
    let f: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let proxy: Address = f["proxy"].as_str().unwrap().parse().unwrap();
    let blocks = f["blocks"].as_array().unwrap();
    let decode = |b: &serde_json::Value| {
        let data: Bytes = b["snapshot"].as_str().unwrap().parse().unwrap();
        let h: thogamm_model::BlockHeader = serde_json::from_value(b["header"].clone()).unwrap();
        PoolModel::from_snapshot(
            proxy,
            abi::getPoolDataCall::abi_decode_returns_validate(&data).unwrap(),
            Some(h.hash),
        )
        .unwrap()
    };
    let mut sdk = event_driven_sdk::EventSynchronizer::from_model(decode(&blocks[0])).unwrap();
    let mut count = 0;
    for b in &blocks[1..] {
        let logs: Vec<thogamm_model::RpcLog> = serde_json::from_value(b["logs"].clone()).unwrap();
        count += logs.len();
        let h: thogamm_model::BlockHeader = serde_json::from_value(b["header"].clone()).unwrap();
        sdk.on_head(h.clone(), event_driven_sdk::CommitState::Proposed)
            .unwrap();
        for log in logs {
            sdk.on_log(log).unwrap();
        }
        sdk.on_head(h, event_driven_sdk::CommitState::Finalized)
            .unwrap();
        let state = sdk.model().unwrap().state();
        let expected = decode(b).into_state();
        assert_eq!(state.block, expected.block);
        assert_eq!(
            state.balances, expected.balances,
            "balances at {}",
            state.block.number
        );
        assert_eq!(
            state.native_balance, expected.native_balance,
            "native at {}",
            state.block.number
        );
        for (slot, value) in &state.words {
            assert_eq!(
                *value,
                expected.word(*slot).unwrap(),
                "slot {slot} at {}",
                state.block.number
            );
        }
        assert_eq!(state, &expected);
    }
    assert_eq!(blocks.len(), 201);
    assert_eq!(count, 204);
}

#[test]
fn execution_and_exact_output_match_deployed_bytecode_on_monad_fork() {
    let mut bytes = Vec::new();
    flate2::read::GzDecoder::new(&include_bytes!("common/execution-v7.json.gz")[..])
        .read_to_end(&mut bytes)
        .unwrap();
    let f: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let proxy: Address = f["proxy"].as_str().unwrap().parse().unwrap();
    let s = &f["snapshots"][0];
    let encoded: Bytes = s["snapshot"].as_str().unwrap().parse().unwrap();
    let model = PoolModel::from_snapshot(
        proxy,
        abi::getPoolDataCall::abi_decode_returns_validate(&encoded).unwrap(),
        Some(thogamm_model::B256::ZERO),
    )
    .unwrap();
    let sdk = event_driven_sdk::EventSynchronizer::from_model(model).unwrap();
    let model = sdk.model().unwrap();
    let mut success = 0;
    for (i, c) in s["cases"].as_array().unwrap().iter().enumerate() {
        let a = model.state().tokens[c["inputIndex"].as_u64().unwrap() as usize].token;
        let b = model.state().tokens[c["outputIndex"].as_u64().unwrap() as usize].token;
        let amount: U256 = c["amount"].as_str().unwrap().parse().unwrap();
        let gas: U256 = c["gasPrice"].as_str().unwrap().parse().unwrap();
        let warm = c["auctionHandlerWarm"].as_bool().unwrap();
        let got = if c["method"] == "execution" {
            model
                .quote_execution_exact_input_with_context(a, b, amount, gas, warm)
                .map(|q| q.amount_out)
        } else {
            model
                .quote_exact_output_with_context(a, b, amount, gas, warm)
                .map(|q| q.amount_in)
        };
        assert_eq!(
            got.is_ok(),
            c["success"].as_bool().unwrap(),
            "case {i}: {got:?}; contract {c}"
        );
        if let Ok(got) = got {
            let result: Bytes = c["result"].as_str().unwrap().parse().unwrap();
            let expected = U256::from_be_slice(&result[..32]);
            assert_eq!(got, expected, "case {i}: {c}");
            success += 1;
        }
    }
    println!("1680 execution cases; {success} exact successes");
}
#[test]
fn boundary_states_match_deployed_bytecode() {
    let mut bytes = Vec::new();
    flate2::read::GzDecoder::new(&include_bytes!("common/boundaries-v7.json.gz")[..])
        .read_to_end(&mut bytes)
        .unwrap();
    let f: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let proxy: Address = f["proxy"].as_str().unwrap().parse().unwrap();
    let mut checked = 0;
    let mut success = 0;
    let mut volume = 0;
    let mut refunds = 0;
    for s in f["snapshots"].as_array().unwrap() {
        let encoded: Bytes = s["snapshot"].as_str().unwrap().parse().unwrap();
        let page = abi::getPoolDataCall::abi_decode_returns_validate(&encoded).unwrap();
        let model = PoolModel::from_snapshot(proxy, page, Some(thogamm_model::B256::ZERO)).unwrap();
        let sdk = event_driven_sdk::EventSynchronizer::from_model(model).unwrap();
        let model = sdk.model().unwrap();
        for (i, c) in s["cases"].as_array().unwrap().iter().enumerate() {
            let a = model.state().tokens[c["inputIndex"].as_u64().unwrap() as usize].token;
            let b = model.state().tokens[c["outputIndex"].as_u64().unwrap() as usize].token;
            let amount: U256 = c["amount"].as_str().unwrap().parse().unwrap();
            let got = match c["method"].as_str().unwrap() {
                "quote" => model.quote_exact_input(a, b, amount).map(|q| {
                    volume += usize::from(!q.volume_cost_usd_wad.is_zero());
                    refunds += usize::from(q.wrapper_cost_usd_wad.is_negative());
                    (q.amount_out, U256::from(q.last_posted_block)).abi_encode()
                }),
                "price" => model.marginal_price(a, b, amount).map(|p| {
                    vec![ContractFraction {
                        numerator: p.numerator,
                        denominator: p.denominator,
                    }]
                    .abi_encode()
                }),
                "limits" => model.limits(a, b).map(|l| vec![l.sell, l.buy].abi_encode()),
                _ => panic!("unknown case"),
            };
            assert_eq!(
                got.is_ok(),
                c["success"].as_bool().unwrap(),
                "block {} case {i} {}->{} amount {amount}: {got:?}; contract {c}",
                s["blockNumber"],
                c["inputIndex"],
                c["outputIndex"]
            );
            if let Ok(got) = got {
                let expected: Bytes = c["result"].as_str().unwrap().parse().unwrap();
                assert_eq!(
                    got,
                    expected.as_ref(),
                    "block {} case {i} {}",
                    s["blockNumber"],
                    c["method"]
                );
                success += 1;
            }
            checked += 1;
        }
    }
    assert_eq!(checked, 5544);
    assert!(success > 2000);
    assert!(volume > 0);
    assert!(refunds > 0);
    println!("{checked} deployed cases: {success} exact results, {} reverts, {volume} volume charges, {refunds} net wrapper refunds",checked-success);
}

#[test]
fn mainnet_lending_pause_and_unpause_match_fresh_snapshots() {
    use alloy_sol_types::SolEvent;
    use thogamm_model::{BlockHeader, RpcLog};
    let mut bytes = Vec::new();
    flate2::read::GzDecoder::new(&include_bytes!("common/events-lending-pause-v7.json.gz")[..])
        .read_to_end(&mut bytes)
        .unwrap();
    let f: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let mut saw = [false; 3];
    for capture in f["captures"].as_array().unwrap() {
        let proxy: Address = capture["proxy"].as_str().unwrap().parse().unwrap();
        let decode = |b: &serde_json::Value| {
            let data: Bytes = b["snapshot"].as_str().unwrap().parse().unwrap();
            let h: BlockHeader = serde_json::from_value(b["header"].clone()).unwrap();
            PoolModel::from_snapshot(
                proxy,
                abi::getPoolDataCall::abi_decode_returns_validate(&data).unwrap(),
                Some(h.hash),
            )
            .unwrap()
        };
        let blocks = capture["blocks"].as_array().unwrap();
        let mut synchronizer =
            event_driven_sdk::EventSynchronizer::from_model(decode(&blocks[0])).unwrap();
        for b in &blocks[1..] {
            let logs: Vec<RpcLog> = serde_json::from_value(b["logs"].clone()).unwrap();
            for l in &logs {
                if l.topics[0] == abi::MakerLendingExposureUpdated::SIGNATURE_HASH {
                    saw[0] = true;
                }
                if l.topics[0] == abi::Paused::SIGNATURE_HASH {
                    let e = abi::Paused::decode_raw_log_validate(l.topics.iter().copied(), &l.data)
                        .unwrap();
                    saw[if e.paused { 1 } else { 2 }] = true;
                }
            }
            let h: BlockHeader = serde_json::from_value(b["header"].clone()).unwrap();
            synchronizer
                .on_head(h.clone(), event_driven_sdk::CommitState::Proposed)
                .unwrap();
            for log in logs {
                synchronizer.on_log(log).unwrap();
            }
            synchronizer
                .on_head(h, event_driven_sdk::CommitState::Finalized)
                .unwrap()
                .unwrap();
            assert_eq!(
                synchronizer.model().unwrap().state(),
                decode(b).state(),
                "{}",
                capture["name"]
            );
        }
    }
    assert_eq!(saw, [true; 3]);
}
