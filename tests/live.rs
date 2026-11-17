//! Follow the deployed pool from logs, then compare with the chain.
//!
//! Needs Monad endpoints, so it only runs when asked:
//!
//!     MONAD_RPC=https://rpc.monad.xyz MONAD_WS=wss://rpc.monad.xyz \
//!         cargo test --release --test live -- --ignored --nocapture
use thogamm_events::{
    thogamm::rpc::{read_snapshot, BlockId},
    HttpRpc, Subscriber,
};

#[tokio::test]
#[ignore = "needs MONAD_RPC and MONAD_WS"]
async fn state_followed_from_logs_equals_state_read_from_the_chain() {
    let http = std::env::var("MONAD_RPC").expect("set MONAD_RPC");
    let ws = std::env::var("MONAD_WS").expect("set MONAD_WS");
    let pool_address = "0x80c74517BCC2D67fFE02D3ED886796272F647210"
        .parse()
        .unwrap();
    let rpc = HttpRpc::new(&http).unwrap();
    let mut subscriber = Subscriber::connect(&http, &ws, pool_address).await.unwrap();
    let start = subscriber.pool().snapshot().block.number;
    let mut compared = 0;
    for update in 1..=60 {
        let followed = subscriber.next_update().await.unwrap().snapshot().clone();
        if update % 20 != 0 {
            continue;
        }
        // A finalized block is the only block at its number.
        let read = read_snapshot(
            &rpc,
            pool_address,
            BlockId::Number(followed.block.number),
            &followed.tokens,
        )
        .await
        .unwrap();
        assert_eq!(followed.block.parent_hash, read.block.parent_hash);
        assert_eq!(
            followed.balances, read.balances,
            "balances at {}",
            read.block.number
        );
        assert_eq!(followed.native_balance, read.native_balance);
        assert_eq!(followed.xstocks, read.xstocks);
        // Words 122..=127 are not pricing inputs and are not carried by events.
        assert_eq!(
            followed.words[..120],
            read.words[..120],
            "storage at {}",
            read.block.number
        );
        compared += 1;
    }
    let end = subscriber.pool().snapshot().block.number;
    println!("followed blocks {start}..={end} from logs; {compared} comparisons with the chain were equal");
}
