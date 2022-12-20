use std::sync::{Arc, RwLock};

use rsnano_core::{
    utils::seconds_since_epoch, Account, AccountInfo, Amount, Block, BlockEnum, BlockHash,
    BlockSubType, BlockType, ChangeBlock, Epoch, OpenBlock, PendingInfo, PendingKey, ReceiveBlock,
    SendBlock, StateBlock,
};
use rsnano_store_traits::WriteTransaction;

use super::Ledger;

pub(crate) struct BlockRollbackPerformer<'a> {
    ledger: &'a Ledger,
    pub txn: &'a mut dyn WriteTransaction,
    pub rolled_back: &'a mut Vec<Arc<RwLock<BlockEnum>>>,
}

impl<'a> BlockRollbackPerformer<'a> {
    pub(crate) fn new(
        ledger: &'a Ledger,
        txn: &'a mut dyn WriteTransaction,
        list: &'a mut Vec<Arc<RwLock<BlockEnum>>>,
    ) -> Self {
        Self {
            ledger,
            txn,
            rolled_back: list,
        }
    }

    pub(crate) fn roll_back(&mut self, block: &BlockEnum) -> anyhow::Result<()> {
        match block {
            BlockEnum::LegacySend(send) => self.rollback_legacy_send(block, send),
            BlockEnum::LegacyReceive(receive) => self.rollback_legacy_receive(block, receive),
            BlockEnum::LegacyOpen(open) => self.rollback_legacy_open(block, open),
            BlockEnum::LegacyChange(change) => self.rollback_legacy_change(block, change),
            BlockEnum::State(state) => self.rollback_state_block(block, state),
        }
    }

    pub(crate) fn rollback_legacy_send(
        &mut self,
        block: &BlockEnum,
        send: &SendBlock,
    ) -> anyhow::Result<()> {
        let pending_info =
            self.roll_back_destination_account_until_send_block_is_unreceived(send)?;

        let account = &pending_info.source;
        let current_account_info = self.load_account(account)?;
        self.delete_pending(send);

        self.roll_back_send_in_representative_cache(
            &current_account_info.representative,
            &pending_info.amount,
        );

        self.do_roll_back(
            block,
            &current_account_info,
            account,
            &Account::zero(),
            &pending_info.amount,
            None,
        );

        self.ledger.observer.block_rolled_back(BlockSubType::Send);
        Ok(())
    }

    pub(crate) fn rollback_legacy_receive(
        &mut self,
        block: &BlockEnum,
        receive: &ReceiveBlock,
    ) -> anyhow::Result<()> {
        let amount = self.ledger.amount(self.txn.txn(), &block.hash()).unwrap();
        let account = self.ledger.account(self.txn.txn(), &block.hash()).unwrap();
        // Pending account entry can be incorrect if source block was pruned. But it's not affecting correct ledger processing
        let linked_account = self.get_source_account(receive);
        let current_account_info = self.load_account(&account)?;

        self.roll_back_receive_in_representative_cache(
            &current_account_info.representative,
            amount,
        );

        self.do_roll_back(
            block,
            &current_account_info,
            &account,
            &linked_account,
            &amount,
            None,
        );

        self.ledger
            .observer
            .block_rolled_back(BlockSubType::Receive);
        Ok(())
    }

    pub(crate) fn rollback_legacy_open(
        &mut self,
        block: &BlockEnum,
        open: &OpenBlock,
    ) -> anyhow::Result<()> {
        let current_account_info = AccountInfo::default();

        let amount = self.ledger.amount(self.txn.txn(), &block.hash()).unwrap();
        let account = self.ledger.account(self.txn.txn(), &block.hash()).unwrap();
        // Pending account entry can be incorrect if source block was pruned. But it's not affecting correct ledger processing
        let linked_account = self
            .ledger
            .account(self.txn.txn(), &open.mandatory_source())
            .unwrap_or_default();

        self.roll_back_receive_in_representative_cache(&open.hashables.representative, amount);

        self.do_roll_back(
            block,
            &current_account_info,
            &account,
            &linked_account,
            &amount,
            None,
        );

        self.ledger.observer.block_rolled_back(BlockSubType::Open);
        Ok(())
    }

    pub(crate) fn rollback_legacy_change(
        &mut self,
        block: &BlockEnum,
        change: &ChangeBlock,
    ) -> anyhow::Result<()> {
        let amount = Amount::zero();
        let account = self.ledger.account(self.txn.txn(), &change.hash()).unwrap();

        let linked_account = Account::zero();
        let current_account_info = self.load_account(&account)?;

        let previous_representative = self.get_previous_representative(block)?.unwrap();

        self.roll_back_change_in_representative_cache(change, previous_representative);

        self.do_roll_back(
            block,
            &current_account_info,
            &account,
            &linked_account,
            &amount,
            Some(previous_representative),
        );

        self.ledger.observer.block_rolled_back(BlockSubType::Change);
        Ok(())
    }

    pub(crate) fn rollback_state_block(
        &mut self,
        block: &BlockEnum,
        state: &StateBlock,
    ) -> anyhow::Result<()> {
        let previous_rep = self.get_previous_representative(block)?;

        let previous_balance = self.ledger.balance(self.txn.txn(), &block.previous());
        let is_send = state.balance() < previous_balance;
        if let Some(previous_rep) = previous_rep {
            self.ledger.cache.rep_weights.representation_add_dual(
                previous_rep,
                previous_balance,
                state.mandatory_representative(),
                Amount::zero().wrapping_sub(state.balance()),
            );
        } else {
            // Add in amount delta only
            self.ledger.cache.rep_weights.representation_add(
                state.mandatory_representative(),
                Amount::zero().wrapping_sub(state.balance()),
            );
        }

        let (mut error, account_info) = match self
            .ledger
            .store
            .account()
            .get(self.txn.txn(), &state.account())
        {
            Some(info) => (false, info),
            None => (true, AccountInfo::default()),
        };

        if is_send {
            let key = PendingKey::new(state.link().into(), state.hash());
            while !error && !self.ledger.store.pending().exists(self.txn.txn(), &key) {
                let latest = self
                    .ledger
                    .latest(self.txn.txn(), &state.link().into())
                    .unwrap();
                match self.ledger.rollback(self.txn, &latest) {
                    Ok(mut list) => self.rolled_back.append(&mut list),
                    Err(_) => error = true,
                };
            }
            self.ledger.store.pending().del(self.txn, &key);
            self.ledger.observer.block_rolled_back(BlockSubType::Send);
        } else if !state.link().is_zero() && !self.ledger.is_epoch_link(&state.link()) {
            // Pending account entry can be incorrect if source block was pruned. But it's not affecting correct ledger processing
            let source_account = self
                .ledger
                .account(self.txn.txn(), &state.link().into())
                .unwrap_or_default();
            let pending_info = PendingInfo::new(
                source_account,
                state.balance() - previous_balance,
                state.sideband().unwrap().source_epoch,
            );
            self.ledger.store.pending().put(
                self.txn,
                &PendingKey::new(state.account(), state.link().into()),
                &pending_info,
            );
            self.ledger
                .observer
                .block_rolled_back(BlockSubType::Receive);
        }
        assert!(!error);
        let previous_version = self
            .ledger
            .store
            .block()
            .version(self.txn.txn(), &state.previous());

        let new_info = AccountInfo {
            head: state.previous(),
            representative: previous_rep.unwrap_or_default(),
            open_block: account_info.open_block,
            balance: previous_balance,
            modified: seconds_since_epoch(),
            block_count: account_info.block_count - 1,
            epoch: previous_version,
        };

        self.ledger
            .update_account(self.txn, &state.account(), &account_info, &new_info);

        match self
            .ledger
            .store
            .block()
            .get(self.txn.txn(), &state.previous())
        {
            Some(previous) => {
                self.ledger
                    .store
                    .block()
                    .successor_clear(self.txn, &state.previous());
                match previous.block_type() {
                    BlockType::Invalid | BlockType::NotABlock => unreachable!(),
                    BlockType::LegacySend
                    | BlockType::LegacyReceive
                    | BlockType::LegacyOpen
                    | BlockType::LegacyChange => {
                        self.ledger.store.frontier().put(
                            self.txn,
                            &state.previous(),
                            &state.account(),
                        );
                    }
                    BlockType::State => {}
                }
            }
            None => {
                self.ledger.observer.block_rolled_back(BlockSubType::Open);
            }
        }

        self.ledger.store.block().del(self.txn, &state.hash());
        Ok(())
    }

    /*************************************************************
     * Helper Functions
     *************************************************************/

    fn load_pending_info_for_send_block(&self, block: &SendBlock) -> Option<PendingInfo> {
        self.ledger
            .store
            .pending()
            .get(self.txn.txn(), &block.pending_key())
    }

    fn roll_back_destination_account_until_send_block_is_unreceived(
        &mut self,
        block: &SendBlock,
    ) -> anyhow::Result<PendingInfo> {
        loop {
            if let Some(info) = self.load_pending_info_for_send_block(block) {
                return Ok(info);
            }

            self.recurse_roll_back(&self.latest_block_for_destination(block)?)?;
        }
    }

    fn recurse_roll_back(&mut self, block_hash: &BlockHash) -> anyhow::Result<()> {
        let mut rolled_back = self.ledger.rollback(self.txn, block_hash)?;
        self.rolled_back.append(&mut rolled_back);
        Ok(())
    }

    fn latest_block_for_destination(&self, block: &SendBlock) -> anyhow::Result<BlockHash> {
        self.ledger
            .latest(self.txn.txn(), &block.hashables.destination)
            .ok_or_else(|| anyhow!("no latest block found"))
    }

    fn get_source_account(&self, block: &ReceiveBlock) -> rsnano_core::PublicKey {
        self.ledger
            .account(self.txn.txn(), &block.mandatory_source())
            .unwrap_or_default()
    }

    fn roll_back_send_in_representative_cache(&self, representative: &Account, amount: &Amount) {
        self.ledger
            .cache
            .rep_weights
            .representation_add(*representative, *amount);
    }

    fn roll_back_receive_in_representative_cache(&self, representative: &Account, amount: Amount) {
        self.ledger
            .cache
            .rep_weights
            .representation_add(*representative, Amount::zero().wrapping_sub(amount));
    }

    fn roll_back_change_in_representative_cache(
        &self,
        change: &ChangeBlock,
        previous_representative: Account,
    ) {
        let previous_balance = self.ledger.balance(self.txn.txn(), &change.previous());

        self.ledger.cache.rep_weights.representation_add_dual(
            change.mandatory_representative(),
            Amount::zero().wrapping_sub(previous_balance),
            previous_representative,
            previous_balance,
        );
    }

    fn do_roll_back(
        &mut self,
        block: &BlockEnum,
        current_account_info: &AccountInfo,
        account: &Account,
        linked_account: &Account,
        amount: &Amount,
        previous_representative: Option<Account>,
    ) {
        let previous_account_info =
            self.previous_account_info(block, current_account_info, previous_representative);

        self.ledger.update_account(
            self.txn,
            account,
            current_account_info,
            &previous_account_info,
        );

        self.ledger.store.block().del(self.txn, &block.hash());

        let receive_source_block = match block {
            BlockEnum::LegacyReceive(receive) => Some(receive.mandatory_source()),
            BlockEnum::LegacyOpen(open) => Some(open.mandatory_source()),
            _ => None,
        };
        if let Some(source) = receive_source_block {
            self.ledger.store.pending().put(
                self.txn,
                &PendingKey::new(*account, source),
                &PendingInfo::new(*linked_account, *amount, Epoch::Epoch0),
            );
        }

        self.ledger.store.frontier().del(self.txn, &block.hash());

        if !block.previous().is_zero() {
            self.ledger
                .store
                .frontier()
                .put(self.txn, &block.previous(), account);

            self.ledger
                .store
                .block()
                .successor_clear(self.txn, &block.previous());
        }
    }

    fn delete_pending(&mut self, block: &SendBlock) {
        self.ledger
            .store
            .pending()
            .del(self.txn, &block.pending_key());
    }

    fn previous_account_info(
        &self,
        block: &BlockEnum,
        current_info: &AccountInfo,
        previous_rep: Option<Account>,
    ) -> AccountInfo {
        if block.block_type() == BlockType::LegacyOpen {
            Default::default()
        } else {
            AccountInfo {
                head: block.previous(),
                representative: previous_rep.unwrap_or(current_info.representative),
                open_block: current_info.open_block,
                balance: self.ledger.balance(self.txn.txn(), &block.previous()),
                modified: seconds_since_epoch(),
                block_count: current_info.block_count - 1,
                epoch: Epoch::Epoch0,
            }
        }
    }

    fn load_account(&self, account: &Account) -> anyhow::Result<AccountInfo> {
        self.ledger
            .store
            .account()
            .get(self.txn.txn(), account)
            .ok_or_else(|| anyhow!("account not found"))
    }

    fn load_block(&self, block_hash: &BlockHash) -> anyhow::Result<BlockEnum> {
        self.ledger
            .store
            .block()
            .get(self.txn.txn(), block_hash)
            .ok_or_else(|| anyhow!("block not found"))
    }

    fn get_previous_representative(&self, block: &BlockEnum) -> anyhow::Result<Option<Account>> {
        let rep_block_hash = if !block.previous().is_zero() {
            self.ledger
                .representative_block_hash(self.txn.txn(), &block.previous())
        } else {
            BlockHash::zero()
        };

        let previous_rep = if !rep_block_hash.is_zero() {
            let rep_block = self.load_block(&rep_block_hash)?;
            Some(rep_block.representative().unwrap_or_default())
        } else {
            None
        };
        Ok(previous_rep)
    }
}
