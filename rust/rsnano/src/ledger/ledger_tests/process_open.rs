use crate::{
    core::{Amount, Block, BlockEnum},
    ledger::ledger_tests::LedgerWithOpenBlock,
    DEV_CONSTANTS, DEV_GENESIS_ACCOUNT,
};

#[test]
fn updates_sideband() {
    let ctx = LedgerWithOpenBlock::new();
    let sideband = ctx.open_block.sideband().unwrap();
    assert_eq!(sideband.account, ctx.receiver_account);
    assert_eq!(sideband.balance, ctx.amount_sent);
    assert_eq!(sideband.height, 1);
}

#[test]
fn saves_block() {
    let ctx = LedgerWithOpenBlock::new();

    let loaded_open = ctx
        .ledger()
        .store
        .block()
        .get(ctx.txn.txn(), &ctx.open_block.hash())
        .unwrap();

    let BlockEnum::Open(loaded_open) = loaded_open else{panic!("not an open block")};
    assert_eq!(loaded_open, ctx.open_block);
    assert_eq!(
        loaded_open.sideband().unwrap(),
        ctx.open_block.sideband().unwrap()
    );
}

#[test]
fn updates_block_amount() {
    let ctx = LedgerWithOpenBlock::new();
    assert_eq!(
        ctx.ledger().amount(ctx.txn.txn(), &ctx.open_block.hash()),
        Some(ctx.amount_sent)
    );
    assert_eq!(
        ctx.ledger()
            .store
            .block()
            .account_calculated(&ctx.open_block),
        ctx.receiver_account
    );
}

#[test]
fn updates_frontier_store() {
    let ctx = LedgerWithOpenBlock::new();
    assert_eq!(
        ctx.ledger()
            .store
            .frontier()
            .get(ctx.txn.txn(), &ctx.open_block.hash()),
        ctx.receiver_account
    );
}

#[test]
fn updates_account_balance() {
    let ctx = LedgerWithOpenBlock::new();
    assert_eq!(
        ctx.ledger()
            .account_balance(ctx.txn.txn(), &ctx.receiver_account, false),
        ctx.amount_sent
    );
}

#[test]
fn updates_account_receivable() {
    let ctx = LedgerWithOpenBlock::new();
    assert_eq!(
        ctx.ledger()
            .account_receivable(ctx.txn.txn(), &ctx.receiver_account, false),
        Amount::zero()
    );
}

#[test]
fn updates_vote_weight() {
    let ctx = LedgerWithOpenBlock::new();
    assert_eq!(
        ctx.ledger().weight(&DEV_GENESIS_ACCOUNT),
        DEV_CONSTANTS.genesis_amount - ctx.amount_sent
    );
    assert_eq!(ctx.ledger().weight(&ctx.receiver_account), ctx.amount_sent);
}

#[test]
fn updates_sender_account_info() {
    let ctx = LedgerWithOpenBlock::new();
    let sender_info = ctx
        .ledger()
        .store
        .account()
        .get(ctx.txn.txn(), &DEV_GENESIS_ACCOUNT)
        .unwrap();
    assert_eq!(sender_info.head, ctx.send_block.hash());
}

#[test]
fn updates_receiver_account_info() {
    let ctx = LedgerWithOpenBlock::new();
    let receiver_info = ctx
        .ledger()
        .store
        .account()
        .get(ctx.txn.txn(), &ctx.receiver_account)
        .unwrap();
    assert_eq!(receiver_info.head, ctx.open_block.hash());
}
