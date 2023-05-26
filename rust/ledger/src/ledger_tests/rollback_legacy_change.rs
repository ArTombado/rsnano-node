use rsnano_core::Amount;
use rsnano_store_traits::FrontierStore;

use crate::{
    ledger_constants::LEDGER_CONSTANTS_STUB, ledger_tests::LedgerContext, DEV_GENESIS_ACCOUNT,
    DEV_GENESIS_HASH,
};

#[test]
fn update_frontier_store() {
    let ctx = LedgerContext::empty();
    let mut txn = ctx.ledger.rw_txn();
    let genesis = ctx.genesis_block_factory();

    let mut change = genesis.legacy_change(&txn).build();
    ctx.ledger.process(&mut txn, &mut change).unwrap();

    ctx.ledger.rollback(&mut txn, &change.hash()).unwrap();

    let frontier = &ctx.ledger.store.frontier;
    assert_eq!(frontier.get(&txn, &change.hash()), None);
    assert_eq!(
        frontier.get(&txn, &DEV_GENESIS_HASH),
        Some(*DEV_GENESIS_ACCOUNT)
    );
}

#[test]
fn update_account_info() {
    let ctx = LedgerContext::empty();
    let mut txn = ctx.ledger.rw_txn();
    let genesis = ctx.genesis_block_factory();

    let mut change = genesis.legacy_change(&txn).build();
    ctx.ledger.process(&mut txn, &mut change).unwrap();

    ctx.ledger.rollback(&mut txn, &change.hash()).unwrap();

    let account_info = ctx.ledger.account_info(&txn, &DEV_GENESIS_ACCOUNT).unwrap();

    assert_eq!(account_info.head, *DEV_GENESIS_HASH);
    assert_eq!(account_info.balance, LEDGER_CONSTANTS_STUB.genesis_amount);
    assert_eq!(account_info.block_count, 1);
    assert_eq!(account_info.representative, *DEV_GENESIS_ACCOUNT);
}

#[test]
fn update_vote_weight() {
    let ctx = LedgerContext::empty();
    let mut txn = ctx.ledger.rw_txn();
    let genesis = ctx.genesis_block_factory();

    let mut change = genesis.legacy_change(&txn).build();
    ctx.ledger.process(&mut txn, &mut change).unwrap();

    ctx.ledger.rollback(&mut txn, &change.hash()).unwrap();

    assert_eq!(
        ctx.ledger.weight(&DEV_GENESIS_ACCOUNT),
        LEDGER_CONSTANTS_STUB.genesis_amount
    );
    assert_eq!(
        ctx.ledger.weight(&change.representative().unwrap()),
        Amount::zero(),
    );
}

#[test]
fn rollback_dependent_blocks_too() {
    let ctx = LedgerContext::empty();
    let mut txn = ctx.ledger.rw_txn();
    let genesis = ctx.genesis_block_factory();

    let mut change = genesis.legacy_change(&txn).build();
    ctx.ledger.process(&mut txn, &mut change).unwrap();

    let mut send = genesis.legacy_send(&txn).build();
    ctx.ledger.process(&mut txn, &mut send).unwrap();

    ctx.ledger.rollback(&mut txn, &change.hash()).unwrap();

    assert_eq!(ctx.ledger.store.block.get(&txn, &send.hash()), None);

    assert_eq!(
        ctx.ledger.weight(&DEV_GENESIS_ACCOUNT),
        LEDGER_CONSTANTS_STUB.genesis_amount
    );
}
