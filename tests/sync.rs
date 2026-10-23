#[path = "common/mod.rs"]
mod support;
use alloy_sol_types::SolEvent;
use event_driven_sdk::{CommitState as C, Config, EventDrivenSdk, EventSynchronizer};
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use thogamm_model::{abi, BlockHeader, Error, PoolModel, RpcLog, U256};
use tokio::net::TcpListener;
use tokio_tungstenite::{accept_async, tungstenite::Message};

#[test]
fn subscription_logs_reproduce_two_hundred_deployed_checkpoints() {
    let f = support::fixture();
    let seed = support::model(f.proxy, f.baseFee, &f.events[0].beforeState);
    let mut sdk = EventSynchronizer::from_model(seed).unwrap();
    for e in &f.events {
        let expected = support::model(f.proxy, f.baseFee, &e.afterState);
        let h = support::model_header(&expected);
        let previous = sdk.model().unwrap().state().clone();
        assert!(sdk.on_head(h.clone(), C::Proposed).unwrap().is_none());
        // Separate filters are delivered in groups, not global log order.
        for log in support::logs(e, f.baseFee).into_iter().rev() {
            sdk.on_log(log.clone()).unwrap();
            sdk.on_log(log).unwrap(); // Overlapping filters must deduplicate.
        }
        assert_eq!(sdk.model().unwrap().state(), &previous);
        assert!(sdk.on_head(h.clone(), C::Voted).unwrap().is_none());
        assert_eq!(sdk.model().unwrap().state(), &previous);
        let update = sdk.on_head(h.clone(), C::Finalized).unwrap().unwrap();
        assert_eq!(update.blocks, 1);
        assert_eq!(sdk.model().unwrap().state(), expected.state());
        assert!(sdk.on_head(h, C::Verified).unwrap().is_none());
    }
    assert_eq!(sdk.model().unwrap().max_index(), 7);
    assert!(!sdk.model().unwrap().state().balances[0].is_zero());
}
#[test]
fn repeated_proposal_frames_do_not_apply_balance_deltas_twice() {
    let f = support::fixture();
    let e = &f.events[4];
    let expected = support::model(f.proxy, f.baseFee, &e.afterState);
    let header = support::model_header(&expected);
    let mut sdk =
        EventSynchronizer::from_model(support::model(f.proxy, f.baseFee, &e.beforeState)).unwrap();
    for _ in 0..2 {
        sdk.on_head(header.clone(), C::Proposed).unwrap();
        for log in support::logs(e, f.baseFee) {
            sdk.on_log(log).unwrap();
        }
    }
    sdk.on_head(header, C::Finalized).unwrap().unwrap();
    assert_eq!(sdk.model().unwrap().state(), expected.state());
}

#[test]
fn skipped_intermediate_commitments_and_multiple_pending_blocks_need_no_reads() {
    let f = support::fixture();
    let mut sdk =
        EventSynchronizer::from_model(support::model(f.proxy, f.baseFee, &f.events[0].beforeState))
            .unwrap();
    for e in f.events.iter().take(3) {
        let h = support::model_header(&support::model(f.proxy, f.baseFee, &e.afterState));
        sdk.on_head(h, C::Proposed).unwrap();
        for log in support::logs(e, f.baseFee) {
            sdk.on_log(log).unwrap();
        }
    }
    let expected = support::model(f.proxy, f.baseFee, &f.events[2].afterState);
    let update = sdk
        .on_head(support::model_header(&expected), C::Finalized)
        .unwrap()
        .unwrap();
    assert_eq!(update.blocks, 3);
    assert_eq!(sdk.model().unwrap().state(), expected.state());
}
#[test]
fn abandoned_proposals_never_change_the_finalized_model() {
    let f = support::fixture();
    let seed = support::model(f.proxy, f.baseFee, &f.events[0].beforeState);
    let n = seed.state().block.number + 1;
    let mut sdk = EventSynchronizer::from_model(seed.clone()).unwrap();
    let mut orphan = support::header(n, f.baseFee);
    orphan.hash = support::hash(n, 1);
    sdk.on_head(orphan.clone(), C::Proposed).unwrap();
    let mut logs = support::logs(&f.events[0], f.baseFee);
    for log in &mut logs {
        log.block_hash = orphan.hash;
        sdk.on_log(log.clone()).unwrap();
    }
    let canonical = support::header(n, f.baseFee);
    sdk.on_head(canonical.clone(), C::Proposed).unwrap();
    // The winning block has no pool logs.
    sdk.on_head(canonical.clone(), C::Finalized)
        .unwrap()
        .unwrap();
    let mut expected = seed.into_state();
    expected.block = canonical.into();
    assert_eq!(sdk.model().unwrap().state(), &expected);
}
#[test]
fn missing_proposal_is_reported_without_inventing_an_empty_block() {
    let f = support::fixture();
    let seed = support::model(f.proxy, f.baseFee, &f.events[0].beforeState);
    let previous = seed.state().clone();
    let mut sdk = EventSynchronizer::from_model(seed).unwrap();
    let h = support::header(previous.block.number + 1, f.baseFee);
    assert!(matches!(
        sdk.on_head(h.clone(), C::Finalized),
        Err(Error::Discontinuous)
    ));
    assert!(matches!(sdk.model(), Err(Error::SnapshotUnavailable)));
    assert!(sdk.on_head(h, C::Proposed).is_err());
}
#[test]
fn malformed_logs_and_balance_mismatches_are_atomic_errors_without_reseed() {
    let f = support::fixture();
    for arithmetic in [false, true] {
        let seed = support::model(f.proxy, f.baseFee, &f.events[0].beforeState);
        let previous = seed.state().clone();
        let mut sdk = EventSynchronizer::from_model(seed).unwrap();
        let h = support::header(previous.block.number + 1, f.baseFee);
        sdk.on_head(h.clone(), C::Proposed).unwrap();
        let event = abi::Transfer {
            from: f.proxy,
            to: thogamm_model::Address::ZERO,
            value: U256::MAX,
        }
        .encode_log_data();
        sdk.on_log(RpcLog {
            address: previous.tokens[0].token,
            topics: event.topics().to_vec(),
            data: if arithmetic {
                event.data
            } else {
                vec![1].into()
            },
            block_hash: h.hash,
            block_number: h.number,
            transaction_index: 0,
            log_index: 0,
            removed: false,
        })
        .unwrap();
        let err = sdk.on_head(h, C::Finalized).unwrap_err();
        assert!(if arithmetic {
            matches!(err, Error::Arithmetic)
        } else {
            matches!(err, Error::Abi(_))
        });
        assert!(matches!(sdk.model(), Err(Error::SnapshotUnavailable)));
    }
}
#[test]
fn upgrade_is_explicit_and_requires_a_compatible_model() {
    let f = support::fixture();
    let seed = support::model(f.proxy, f.baseFee, &f.events[0].beforeState);
    let h = support::header(seed.state().block.number + 1, f.baseFee);
    let mut sdk = EventSynchronizer::from_model(seed).unwrap();
    sdk.on_head(h.clone(), C::Proposed).unwrap();
    let event = abi::Upgraded {
        implementation: f.proxy,
    }
    .encode_log_data();
    sdk.on_log(RpcLog {
        address: f.proxy,
        topics: event.topics().to_vec(),
        data: event.data,
        block_hash: h.hash,
        block_number: h.number,
        transaction_index: 0,
        log_index: 0,
        removed: false,
    })
    .unwrap();
    assert!(matches!(
        sdk.on_head(h, C::Finalized),
        Err(Error::ContractUpgraded)
    ));
}
fn head(h: &BlockHeader, state: &str) -> Value {
    let mut payload = serde_json::to_value(h).unwrap();
    payload["commitState"] = json!(state);
    json!({"jsonrpc":"2.0","method":"eth_subscription","params":{"subscription":"sub5","result":payload}})
}
fn log(l: &RpcLog) -> Value {
    json!({"jsonrpc":"2.0","method":"eth_subscription","params":{"subscription":"sub1","result":l}})
}
#[tokio::test]
async fn websocket_updates_issue_zero_rpc_reads_and_only_one_startup_snapshot() {
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        let f = support::fixture();
        let rpc = support::MockRpc::from_events(&f);
        let seed = support::model(f.proxy, f.baseFee, &f.events[0].beforeState);
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("ws://{}", listener.local_addr().unwrap());
        let seed_head = support::model_header(&seed);
        let frames: Vec<_> = f
            .events
            .iter()
            .map(|e| {
                (
                    support::model_header(&support::model(f.proxy, f.baseFee, &e.afterState)),
                    support::logs(e, f.baseFee),
                )
            })
            .collect();
        let (send, receive) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = accept_async(stream).await.unwrap();
            for i in 1..=5 {
                let req: Value =
                    serde_json::from_str(socket.next().await.unwrap().unwrap().to_text().unwrap())
                        .unwrap();
                assert_eq!(req["method"], "eth_subscribe");
                assert_eq!(
                    req["params"][0],
                    if i == 5 { "monadNewHeads" } else { "logs" }
                );
                socket
                    .send(Message::Text(
                        json!({"jsonrpc":"2.0","id":i,"result":format!("sub{i}")})
                            .to_string()
                            .into(),
                    ))
                    .await
                    .unwrap();
            }
            for state in ["Proposed", "Finalized"] {
                socket
                    .send(Message::Text(head(&seed_head, state).to_string().into()))
                    .await
                    .unwrap();
            }
            receive.await.unwrap();
            socket
                .send(Message::Ping(vec![1, 2, 3].into()))
                .await
                .unwrap();
            for (h, logs) in frames {
                socket
                    .send(Message::Text(head(&h, "Proposed").to_string().into()))
                    .await
                    .unwrap();
                for l in logs.into_iter().rev() {
                    socket
                        .send(Message::Text(log(&l).to_string().into()))
                        .await
                        .unwrap();
                }
                socket
                    .send(Message::Text(head(&h, "Finalized").to_string().into()))
                    .await
                    .unwrap();
            }
            // Any request after the five setup subscriptions is a test failure.
            let message = socket.next().await.unwrap().unwrap();
            assert!(matches!(message, Message::Pong(_)));
            socket.close(None).await.unwrap();
        });
        let mut sdk = EventDrivenSdk::with_reader(
            rpc.clone(),
            url,
            f.proxy,
            Config {
                wmon: seed.state().tokens[3].token,
                ..Config::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(rpc.call_count(), 1);
        // The running SDK cannot use even this available reader: force all reads to fail.
        rpc.0.lock().unwrap().fail_calls = true;
        send.send(()).unwrap();
        for e in &f.events {
            sdk.next_update().await.unwrap();
            let expected: PoolModel = support::model(f.proxy, f.baseFee, &e.afterState);
            assert_eq!(sdk.model().unwrap().state(), expected.state());
            assert_eq!(rpc.call_count(), 1);
        }
        assert!(sdk.next_update().await.is_err());
        assert_eq!(rpc.call_count(), 1);
        server.await.unwrap();
    })
    .await
    .unwrap();
}

#[test]
fn registry_retirement_requires_an_explicit_complete_snapshot() {
    let f = support::fixture();
    let seed = support::model(f.proxy, f.baseFee, &f.events[0].beforeState);
    let before = seed.state().clone();
    let h = support::header(before.block.number + 1, f.baseFee);
    let mut sdk = EventSynchronizer::from_model(seed).unwrap();
    sdk.on_head(h.clone(), C::Proposed).unwrap();
    let event = abi::MakerTokenRetired {
        retiredIndex: 1,
        token: before.tokens[1].token,
        movedFrom: 7,
        generation: 2,
    }
    .encode_log_data();
    sdk.on_log(RpcLog {
        address: f.proxy,
        topics: event.topics().to_vec(),
        data: event.data,
        block_hash: h.hash,
        block_number: h.number,
        transaction_index: 0,
        log_index: 0,
        removed: false,
    })
    .unwrap();
    assert!(matches!(
        sdk.on_head(h, C::Finalized),
        Err(Error::RegistryChanged)
    ));
    assert!(matches!(sdk.model(), Err(Error::SnapshotUnavailable)));
}

#[tokio::test]
async fn a_live_socket_without_new_blocks_times_out_even_when_pongs_arrive() {
    use std::time::Duration;
    let f = support::fixture();
    let rpc = support::MockRpc::from_events(&f);
    let seed = support::model(f.proxy, f.baseFee, &f.events[0].beforeState);
    let seed_head = support::model_header(&seed);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}", listener.local_addr().unwrap());
    let (done, finish) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = accept_async(stream).await.unwrap();
        for i in 1..=5 {
            let req: Value =
                serde_json::from_str(socket.next().await.unwrap().unwrap().to_text().unwrap())
                    .unwrap();
            assert_eq!(req["method"], "eth_subscribe");
            socket
                .send(Message::Text(
                    json!({"jsonrpc":"2.0","id":i,"result":format!("sub{i}")})
                        .to_string()
                        .into(),
                ))
                .await
                .unwrap();
        }
        for stage in ["Proposed", "Finalized"] {
            socket
                .send(Message::Text(head(&seed_head, stage).to_string().into()))
                .await
                .unwrap();
        }
        // No state messages follow. Keep the transport alive by answering the client's ping.
        let Message::Ping(payload) = socket.next().await.unwrap().unwrap() else {
            panic!("expected client ping")
        };
        socket.send(Message::Pong(payload)).await.unwrap();
        finish.await.unwrap();
    });
    let mut sdk = EventDrivenSdk::with_reader(
        rpc.clone(),
        url,
        f.proxy,
        Config {
            update_timeout: Duration::from_millis(1200),
            ..Config::default()
        },
    )
    .await
    .unwrap();
    let historical = sdk.model().unwrap().clone();
    // Cancellation also invalidates live access; no subsequent reader can mistake it for a fresh update.
    assert!(
        tokio::time::timeout(Duration::from_millis(10), sdk.next_update())
            .await
            .is_err()
    );
    assert!(matches!(sdk.model(), Err(Error::SnapshotUnavailable)));
    assert!(matches!(sdk.next_update().await, Err(Error::Transport(_))));
    assert!(matches!(sdk.model(), Err(Error::SnapshotUnavailable)));
    assert_eq!(historical.state(), seed.state());
    assert_eq!(rpc.call_count(), 1);
    done.send(()).unwrap();
    server.await.unwrap();
}

#[test]
fn updates_expose_the_block_time_independently_of_local_publication_time() {
    let f = support::fixture();
    let e = &f.events[0];
    let mut sdk =
        EventSynchronizer::from_model(support::model(f.proxy, f.baseFee, &e.beforeState)).unwrap();
    let mut h = support::model_header(&support::model(f.proxy, f.baseFee, &e.afterState));
    h.timestamp = U256::from(1_700_000_000);
    sdk.on_head(h.clone(), C::Proposed).unwrap();
    for l in support::logs(e, f.baseFee) {
        sdk.on_log(l).unwrap();
    }
    let before = std::time::SystemTime::now();
    let update = sdk.on_head(h, C::Finalized).unwrap().unwrap();
    assert_eq!(update.block.timestamp, U256::from(1_700_000_000));
    assert!(update.received_at >= before);
}
