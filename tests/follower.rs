//! The follower applies finalized blocks, whole and in order, and nothing else.
mod common;
use common::*;
use serde_json::Value;
use thogamm_events::{
    thogamm::{abi, abi::SolEvent, Address, Snapshot},
    BlockHeader, CommitState as C, Error, Follower, Log, B256,
};

struct Recorded {
    f: Value,
}

impl Recorded {
    fn new() -> Self {
        Self {
            f: fixture("events-stored-multipliers.json.gz"),
        }
    }
    fn blocks(&self) -> &[Value] {
        self.f["blocks"].as_array().unwrap()
    }
    fn header(&self, index: usize) -> BlockHeader {
        serde_json::from_value(self.blocks()[index]["header"].clone()).unwrap()
    }
    fn logs(&self, index: usize) -> Vec<Log> {
        serde_json::from_value(self.blocks()[index]["logs"].clone()).unwrap()
    }
    /// The state read from the chain at a block.
    fn read(&self, index: usize) -> Snapshot {
        let mut snapshot = snapshot(address(&self.f["proxy"]), &self.blocks()[index]);
        snapshot.block.hash = Some(self.header(index).hash);
        snapshot
    }
    fn follower(&self) -> Follower {
        Follower::new(self.read(0)).unwrap()
    }
    fn propose(&self, follower: &mut Follower, index: usize) {
        assert!(!follower.on_head(self.header(index), C::Proposed).unwrap());
        for log in self.logs(index) {
            follower.on_log(log).unwrap();
        }
    }
}

/// Storage words 122..=127 are not pricing inputs and are not carried by events.
fn assert_follows(follower: &Follower, read: &Snapshot) {
    let followed = follower.pool().snapshot();
    assert_eq!(followed.block, read.block);
    assert_eq!(followed.balances, read.balances);
    assert_eq!(followed.native_balance, read.native_balance);
    assert_eq!(followed.xstocks, read.xstocks);
    assert_eq!(followed.words[..120], read.words[..120]);
}

#[test]
fn follows_fifty_recorded_blocks_through_every_commit_state() {
    let recorded = Recorded::new();
    let mut follower = recorded.follower();
    for index in 1..recorded.blocks().len() {
        let header = recorded.header(index);
        let before = follower.pool().snapshot().clone();
        assert!(!follower.on_head(header.clone(), C::Proposed).unwrap());
        // The log filters overlap and are delivered filter by filter, so logs
        // arrive out of order and more than once.
        for log in recorded.logs(index).into_iter().rev() {
            follower.on_log(log.clone()).unwrap();
            follower.on_log(log).unwrap();
        }
        assert!(!follower.on_head(header.clone(), C::Voted).unwrap());
        assert_eq!(
            follower.pool().snapshot(),
            &before,
            "nothing changes before finality"
        );
        assert!(follower.on_head(header.clone(), C::Finalized).unwrap());
        assert_follows(&follower, &recorded.read(index));
        assert!(!follower.on_head(header, C::Verified).unwrap());
    }
}

#[test]
fn replays_antx_transfers_into_an_empty_holding() {
    let recorded = Recorded {
        f: fixture("events-18.json.gz"),
    };
    let antx: Address = "0x909369940f22dc1c143ac78753a33f41597138b1"
        .parse()
        .unwrap();
    let before = recorded.read(0);
    let index = before.tokens.iter().position(|&t| t == antx).unwrap();
    assert!(before.xstocks[index].as_ref().unwrap().shares.is_zero());

    let mut follower = recorded.follower();
    recorded.propose(&mut follower, 1);
    assert!(follower.on_head(recorded.header(1), C::Finalized).unwrap());
    let after = recorded.read(1);
    assert!(!after.xstocks[index].as_ref().unwrap().shares.is_zero());
    assert_follows(&follower, &after);
}

#[test]
fn several_blocks_can_finalize_at_once() {
    let recorded = Recorded::new();
    let mut follower = recorded.follower();
    for index in 1..=4 {
        recorded.propose(&mut follower, index);
    }
    assert!(follower.on_head(recorded.header(3), C::Finalized).unwrap());
    assert_follows(&follower, &recorded.read(3));
    assert!(follower.on_head(recorded.header(4), C::Finalized).unwrap());
    assert_follows(&follower, &recorded.read(4));
}

#[test]
fn a_proposal_sent_twice_counts_its_logs_once() {
    let recorded = Recorded::new();
    let mut follower = recorded.follower();
    recorded.propose(&mut follower, 1);
    recorded.propose(&mut follower, 1);
    assert!(follower.on_head(recorded.header(1), C::Finalized).unwrap());
    assert_follows(&follower, &recorded.read(1));
}

#[test]
fn a_proposal_that_is_never_finalized_changes_nothing() {
    let recorded = Recorded::new();
    let mut follower = recorded.follower();
    // A competing block at the same height, carrying the next block's logs.
    let mut abandoned = recorded.header(1);
    abandoned.hash = B256::repeat_byte(0xab);
    assert!(!follower.on_head(abandoned.clone(), C::Proposed).unwrap());
    for mut log in recorded.logs(2) {
        log.block_hash = abandoned.hash;
        log.block_number = abandoned.number;
        follower.on_log(log).unwrap();
    }
    recorded.propose(&mut follower, 1);
    assert!(follower.on_head(recorded.header(1), C::Finalized).unwrap());
    assert_follows(&follower, &recorded.read(1));
}

#[test]
fn a_gap_is_an_error_and_the_follower_stays_stopped() {
    let recorded = Recorded::new();
    let mut follower = recorded.follower();
    let before = follower.pool().snapshot().clone();
    // Block 1 was never proposed on this connection.
    recorded.propose(&mut follower, 2);
    assert!(matches!(
        follower.on_head(recorded.header(2), C::Finalized),
        Err(Error::Discontinuous)
    ));
    assert_eq!(follower.pool().snapshot(), &before);
    assert!(matches!(
        follower.on_head(recorded.header(1), C::Proposed),
        Err(Error::Discontinuous)
    ));
    let any_log = (1..)
        .find_map(|index| recorded.logs(index).into_iter().next())
        .unwrap();
    assert!(matches!(
        follower.on_log(any_log),
        Err(Error::Discontinuous)
    ));
}

#[test]
fn a_block_whose_logs_may_be_incomplete_is_not_applied() {
    let recorded = Recorded::new();
    let mut follower = recorded.follower();
    // A log for a block that is not the one currently being received.
    let with_logs = (1..)
        .find(|index| !recorded.logs(*index).is_empty())
        .unwrap();
    for index in 1..=with_logs + 1 {
        recorded.propose(&mut follower, index);
    }
    let late = recorded.logs(with_logs).remove(0);
    assert!(matches!(follower.on_log(late), Err(Error::Discontinuous)));
}

#[test]
fn an_upgrade_or_listing_change_stops_the_follower() {
    let recorded = Recorded::new();
    let pool = address(&recorded.f["proxy"]);
    for (topic, expected) in [
        (abi::Upgraded::SIGNATURE_HASH, "upgraded"),
        (abi::MakerTokenRetired::SIGNATURE_HASH, "token list"),
    ] {
        let mut follower = recorded.follower();
        let header = recorded.header(1);
        follower.on_head(header.clone(), C::Proposed).unwrap();
        follower
            .on_log(Log {
                address: pool,
                topics: vec![topic, B256::ZERO],
                data: Default::default(),
                block_hash: header.hash,
                block_number: header.number,
                transaction_index: 0,
                log_index: 0,
                removed: false,
            })
            .unwrap();
        let error = follower.on_head(header, C::Finalized).unwrap_err();
        assert!(
            matches!(error, Error::ContractUpgraded | Error::RegistryChanged),
            "{error}"
        );
        assert!(error.to_string().contains(expected), "{error}");
    }
}

#[test]
fn following_needs_a_known_starting_block_hash() {
    let recorded = Recorded::new();
    let mut snapshot = recorded.read(0);
    snapshot.block.hash = None;
    assert!(Follower::new(snapshot).is_err());
}
