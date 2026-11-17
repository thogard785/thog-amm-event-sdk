mod common;
use alloy_sol_types::{sol, SolCall, SolValue};
use common as support;
use std::io::Read;
use thogamm_model::{abi, Address, Bytes, PoolModel, U256};
sol! { struct ContractFraction { uint256 numerator; uint256 denominator; } }
#[test]
fn historical_read_only_snapshots_match_after_model_identity_migration() {
    let mut bytes = Vec::new();
    flate2::read::GzDecoder::new(&include_bytes!("common/deployed-v8.json.gz")[..])
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
        let page = support::historical_page(
            abi::getPoolDataCall::abi_decode_returns_validate(&encoded).unwrap(),
        );
        let model = PoolModel::from_snapshot(
            proxy,
            page,
            Some(thogamm_model::B256::ZERO),
            serde_json::from_value(s["shareBalances"].clone()).unwrap(),
        )
        .unwrap();
        let sdk = event_driven_sdk::EventSynchronizer::from_model(model).unwrap();
        let model = sdk.model();
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
    assert_eq!(checked, 1872);
    assert_eq!(success, 954);
    assert!(volume > 0);
    assert!(refunds > 0);
    println!("{checked} deployed cases: {success} exact results, {} reverts, {volume} volume charges, {refunds} net wrapper refunds",checked-success);
}

#[test]
fn replay_two_hundred_deployed_blocks_matches_every_snapshot() {
    let mut bytes = Vec::new();
    flate2::read::GzDecoder::new(&include_bytes!("common/events-v8.json.gz")[..])
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
            support::historical_page(
                abi::getPoolDataCall::abi_decode_returns_validate(&data).unwrap(),
            ),
            Some(h.hash),
            serde_json::from_value(b["shareBalances"].clone()).unwrap(),
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
        sdk.on_head(h.clone(), event_driven_sdk::CommitState::Finalized)
            .unwrap();
        let state = sdk.model().state();
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
    assert_eq!(count, 220);
}

#[test]
fn execution_and_exact_output_match_model3_contract_on_monad_fork() {
    let mut bytes = Vec::new();
    flate2::read::GzDecoder::new(&include_bytes!("common/execution-model3.json.gz")[..])
        .read_to_end(&mut bytes)
        .unwrap();
    let f: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let proxy: Address = f["proxy"].as_str().unwrap().parse().unwrap();
    let mut success = 0;
    let mut checked = 0;
    for s in f["snapshots"].as_array().unwrap() {
        let encoded: Bytes = s["snapshot"].as_str().unwrap().parse().unwrap();
        let model = PoolModel::from_snapshot(
            proxy,
            abi::getPoolDataCall::abi_decode_returns_validate(&encoded).unwrap(),
            Some(thogamm_model::B256::ZERO),
            serde_json::from_value(s["shareBalances"].clone()).unwrap(),
        )
        .unwrap();
        let sdk = event_driven_sdk::EventSynchronizer::from_model(model).unwrap();
        let model = sdk.model();
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
        checked += s["cases"].as_array().unwrap().len();
    }
    assert_eq!(checked, 32760);
    assert!(success > 0);
    println!("{checked} execution cases; {success} exact successes");
}
#[test]
fn historical_read_only_boundary_states_match_after_model_identity_migration() {
    let mut bytes = Vec::new();
    flate2::read::GzDecoder::new(&include_bytes!("common/boundaries-v8.json.gz")[..])
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
        let page = support::historical_page(
            abi::getPoolDataCall::abi_decode_returns_validate(&encoded).unwrap(),
        );
        let model = PoolModel::from_snapshot(
            proxy,
            page,
            Some(thogamm_model::B256::ZERO),
            serde_json::from_value(s["shareBalances"].clone()).unwrap(),
        )
        .unwrap();
        let sdk = event_driven_sdk::EventSynchronizer::from_model(model).unwrap();
        let model = sdk.model();
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
    assert_eq!(checked, 24024);
    assert!(success > 2000);
    assert!(volume > 0);
    assert!(refunds > 0);
    println!("{checked} deployed cases: {success} exact results, {} reverts, {volume} volume charges, {refunds} net wrapper refunds",checked-success);
}

#[test]
fn deployed_seventeen_token_quotes_use_stored_multipliers() {
    let mut bytes = Vec::new();
    flate2::read::GzDecoder::new(&include_bytes!("common/deployed-stored-multipliers.json.gz")[..])
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
        assert_eq!(page.tokens.len(), 17);
        let model = PoolModel::from_snapshot(
            proxy,
            page,
            Some(s["blockHash"].as_str().unwrap().parse().unwrap()),
            serde_json::from_value(s["shareBalances"].clone()).unwrap(),
        )
        .unwrap();
        let sdk = event_driven_sdk::EventSynchronizer::from_model(model).unwrap();
        let model = sdk.model();
        assert_eq!(model.state().share_balances.len(), 9);
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
    assert_eq!(checked, 17 * 16 * 12);
    assert_eq!(success, 2033);
    println!("{checked} deployed cases: {success} exact results, {} reverts, {volume} volume charges, {refunds} net wrapper refunds",checked-success);
}

#[test]
fn replay_fifty_blocks_with_seventeen_tokens_matches_every_quote_field() {
    let mut bytes = Vec::new();
    flate2::read::GzDecoder::new(&include_bytes!("common/events-stored-multipliers.json.gz")[..])
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
            serde_json::from_value(b["shareBalances"].clone()).unwrap(),
        )
        .unwrap()
    };
    let mut sdk = event_driven_sdk::EventSynchronizer::from_model(decode(&blocks[0])).unwrap();
    let mut count = 0;
    let mut perpetual_exposure_updates = 0;
    for b in &blocks[1..] {
        let logs: Vec<thogamm_model::RpcLog> = serde_json::from_value(b["logs"].clone()).unwrap();
        count += logs.len();
        use alloy_sol_types::SolEvent;
        for log in &logs {
            if log.topics.first() == Some(&abi::MakerLendingExposureUpdated::SIGNATURE_HASH) {
                let event = abi::MakerLendingExposureUpdated::decode_raw_log_validate(
                    log.topics.iter().copied(),
                    &log.data,
                )
                .unwrap();
                perpetual_exposure_updates += usize::from(event.venue == 2);
            }
        }
        let h: thogamm_model::BlockHeader = serde_json::from_value(b["header"].clone()).unwrap();
        sdk.on_head(h.clone(), event_driven_sdk::CommitState::Proposed)
            .unwrap();
        for log in logs {
            sdk.on_log(log).unwrap();
        }
        sdk.on_head(h.clone(), event_driven_sdk::CommitState::Finalized)
            .unwrap();
        let state = sdk.model().state();
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
        assert_quote_state_eq(state, &expected);
    }
    assert_eq!(blocks.len(), 51);
    assert_eq!(count, 505);
    assert_eq!(perpetual_exposure_updates, 1);
}

// Perpetual account configuration and positions (122..127) are included in the
// raw snapshot but are not inputs to maker quotes or maintained by event replay.
// Its quote effect arrives separately as MakerLendingExposureUpdated. Compare
// every quote field and also check that the differing operational words do not
// affect any pair's quotes; do not rewrite the replayed or captured source state.
fn assert_quote_state_eq(actual: &thogamm_model::PoolState, expected: &thogamm_model::PoolState) {
    let actual_model = PoolModel::new(actual.clone()).unwrap();
    let expected_model = PoolModel::new(expected.clone()).unwrap();
    for input in &expected.tokens {
        for output in &expected.tokens {
            if input.token == output.token {
                continue;
            }
            for amount in [U256::ONE, thogamm_model::math::pow10(input.decimals)] {
                assert_eq!(
                    actual_model
                        .quote_exact_input(input.token, output.token, amount)
                        .ok(),
                    expected_model
                        .quote_exact_input(input.token, output.token, amount)
                        .ok(),
                    "quote at {}: {} -> {} amount {amount}",
                    expected.block.number,
                    input.token,
                    output.token,
                );
            }
        }
    }
    let mut actual_quote_state = actual.clone();
    let mut expected_quote_state = expected.clone();
    actual_quote_state.words.retain(|slot, _| *slot < 122);
    expected_quote_state.words.retain(|slot, _| *slot < 122);
    assert_eq!(actual_quote_state, expected_quote_state);
}
