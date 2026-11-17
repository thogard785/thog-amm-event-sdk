//! End to end over a real WebSocket: subscribe, read once, follow from logs.
mod common;
use common::{node::Node, *};
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::time::Duration;
use thogamm_events::{thogamm::Error, Subscriber};
use tokio::{net::TcpListener, sync::mpsc};
use tokio_tungstenite::{accept_async, tungstenite::Message};

/// A WebSocket server that acknowledges the five subscriptions, then sends
/// whatever notifications the test hands it. `Value::Null` makes it stall, a
/// string is sent as it is instead of as JSON, and dropping the sender closes
/// the connection.
async fn server() -> (
    String,
    mpsc::UnboundedSender<Value>,
    tokio::task::JoinHandle<Vec<Value>>,
) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}", listener.local_addr().unwrap());
    let (send, mut notifications) = mpsc::unbounded_channel::<Value>();
    let task = tokio::spawn(async move {
        let (connection, _) = listener.accept().await.unwrap();
        let mut socket = accept_async(connection).await.unwrap();
        let mut subscriptions = Vec::new();
        for index in 0..5 {
            let Message::Text(text) = socket.next().await.unwrap().unwrap() else {
                panic!()
            };
            let request: Value = serde_json::from_str(&text).unwrap();
            assert_eq!(request["method"], "eth_subscribe");
            subscriptions.push(request["params"].clone());
            let reply =
                json!({ "jsonrpc": "2.0", "id": request["id"], "result": format!("0x{index}") });
            socket
                .send(Message::Text(reply.to_string().into()))
                .await
                .unwrap();
        }
        while let Some(notification) = notifications.recv().await {
            if notification.is_null() {
                tokio::time::sleep(Duration::from_secs(3600)).await;
            }
            let text = match &notification {
                Value::String(raw) => raw.clone(),
                json => json.to_string(),
            };
            socket.send(Message::Text(text.into())).await.unwrap();
        }
        subscriptions
    });
    (url, send, task)
}

fn head(block: &Value, state: &str) -> Value {
    let mut header = block["header"].clone();
    header["commitState"] = json!(state);
    json!({ "jsonrpc": "2.0", "method": "eth_subscription", "params": { "subscription": "0x4", "result": header } })
}

fn log(log: &Value) -> Value {
    json!({ "jsonrpc": "2.0", "method": "eth_subscription", "params": { "subscription": "0x0", "result": log } })
}

#[tokio::test]
async fn reads_once_then_follows_finalized_blocks_from_logs_alone() {
    let f = fixture("events-stored-multipliers.json.gz");
    let blocks = f["blocks"].as_array().unwrap();
    let pool = address(&f["proxy"]);
    let node = Node::new(pool, &blocks[0]);
    let (url, send, server) = server().await;

    // The first finalized block seen as a proposal is where the snapshot is read.
    send.send(head(&blocks[0], "Proposed")).unwrap();
    send.send(head(&blocks[0], "Finalized")).unwrap();
    let mut subscriber = Subscriber::with_rpc(&node, &url, pool).await.unwrap();
    assert_eq!(
        subscriber.pool().snapshot().block.number,
        blocks[0]["header"]["number"]
            .as_str()
            .map(|n| u64::from_str_radix(&n[2..], 16).unwrap())
            .unwrap()
    );
    assert_eq!(subscriber.pool().tokens().len(), 17);
    assert_eq!(node.requests(), 2);

    for block in &blocks[1..=10] {
        send.send(head(block, "Proposed")).unwrap();
        for entry in block["logs"].as_array().unwrap() {
            send.send(log(entry)).unwrap();
        }
        send.send(head(block, "Voted")).unwrap();
        send.send(head(block, "Finalized")).unwrap();
        let followed = subscriber.next_update().await.unwrap().snapshot().clone();
        let mut read = snapshot(pool, block);
        read.block.hash = followed.block.hash;
        assert_eq!(followed.block, read.block);
        assert_eq!(followed.balances, read.balances);
        assert_eq!(followed.xstocks, read.xstocks);
        assert_eq!(followed.words[..120], read.words[..120]);
    }
    assert_eq!(node.requests(), 2, "following reads nothing over HTTP");

    // A connection that goes quiet is an error, not a pool that silently ages.
    subscriber.set_stall_timeout(Duration::from_millis(100));
    send.send(Value::Null).unwrap();
    assert!(matches!(
        subscriber.next_update().await,
        Err(Error::Transport(_))
    ));
    assert!(matches!(
        subscriber.next_update().await,
        Err(Error::Discontinuous)
    ));

    drop(send);
    server.abort();
    let subscriptions = server.await;
    assert!(subscriptions.is_err(), "the server was stopped mid-stall");
}

/// A message that cannot be read might have been a log, and a closed
/// connection misses every block after it. Either ends the subscription.
#[tokio::test]
async fn a_lost_message_ends_the_subscription() {
    let f = fixture("events-stored-multipliers.json.gz");
    let blocks = f["blocks"].as_array().unwrap();
    let pool = address(&f["proxy"]);
    for unreadable in [true, false] {
        let node = Node::new(pool, &blocks[0]);
        let (url, send, _server) = server().await;
        send.send(head(&blocks[0], "Proposed")).unwrap();
        send.send(head(&blocks[0], "Finalized")).unwrap();
        let mut subscriber = Subscriber::with_rpc(&node, &url, pool).await.unwrap();

        if unreadable {
            // The next block arrives whole, except for one unreadable message.
            send.send(head(&blocks[1], "Proposed")).unwrap();
            send.send(json!("{ not JSON")).unwrap();
            send.send(head(&blocks[1], "Voted")).unwrap();
            send.send(head(&blocks[1], "Finalized")).unwrap();
            assert!(matches!(subscriber.next_update().await, Err(Error::Rpc(_))));
        } else {
            drop(send);
            assert!(matches!(
                subscriber.next_update().await,
                Err(Error::Transport(_))
            ));
        }
        assert!(matches!(
            subscriber.next_update().await,
            Err(Error::Discontinuous)
        ));
        assert_eq!(
            subscriber.pool().snapshot().block.number,
            snapshot(pool, &blocks[0]).block.number
        );
    }
}

#[tokio::test]
async fn subscribes_to_logs_before_heads() {
    let f = fixture("events-stored-multipliers.json.gz");
    let blocks = f["blocks"].as_array().unwrap();
    let pool = address(&f["proxy"]);
    let node = Node::new(pool, &blocks[0]);
    let (url, send, server) = server().await;
    send.send(head(&blocks[0], "Proposed")).unwrap();
    send.send(head(&blocks[0], "Finalized")).unwrap();
    let subscriber = Subscriber::with_rpc(&node, &url, pool).await.unwrap();
    drop(send);
    let subscriptions = server.await.unwrap();
    drop(subscriber);
    assert_eq!(subscriptions.len(), 5);
    for subscription in &subscriptions[..4] {
        assert_eq!(subscription[0], "logs");
    }
    assert_eq!(subscriptions[0][1]["address"], json!(pool));
    assert_eq!(subscriptions[4], json!(["monadNewHeads"]));
}
