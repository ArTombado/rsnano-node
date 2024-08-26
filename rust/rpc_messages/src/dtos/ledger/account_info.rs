use rsnano_core::{Account, BlockHash};
use serde::{Deserialize, Serialize};

#[derive(PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct AccountInfoDto {
    pub frontier: BlockHash,
    pub open_block: BlockHash,
    pub representative_block: BlockHash,
    pub balance: u128,
    pub confirmed_balance: Option<u128>,
    pub modified_timestamp: u64,
    pub block_count: u64,
    pub account_version: u8,
    pub confirmation_height: Option<u64>,
    pub confirmation_height_frontier: Option<BlockHash>,
    pub confirmed_height: Option<u64>,
    pub confirmed_frontier: Option<BlockHash>,
    pub representative: Option<Account>,
    pub confirmed_representative: Option<Account>,
    pub weight: Option<u128>,
    pub pending: Option<u128>,
    pub receivable: Option<u128>,
    pub confirmed_pending: Option<u128>,
    pub confirmed_receivable: Option<u128>,
}

impl AccountInfoDto {
    pub fn new(
        frontier: BlockHash,
        open_block: BlockHash,
        representative_block: BlockHash,
        balance: u128,
        modified_timestamp: u64,
        block_count: u64,
        account_version: u8,
    ) -> Self {
        AccountInfoDto {
            frontier,
            open_block,
            representative_block,
            balance,
            modified_timestamp,
            block_count,
            account_version,
            confirmed_balance: None,
            confirmation_height: None,
            confirmation_height_frontier: None,
            confirmed_height: None,
            confirmed_frontier: None,
            representative: None,
            confirmed_representative: None,
            weight: None,
            pending: None,
            receivable: None,
            confirmed_pending: None,
            confirmed_receivable: None,
        }
    }

    pub fn set_confirmed_balance(&mut self, balance: u128) {
        self.confirmed_balance = Some(balance);
    }

    pub fn set_confirmation_height(&mut self, height: u64) {
        self.confirmation_height = Some(height);
    }

    pub fn set_confirmation_height_frontier(&mut self, frontier: BlockHash) {
        self.confirmation_height_frontier = Some(frontier);
    }

    pub fn set_confirmed_height(&mut self, height: u64) {
        self.confirmed_height = Some(height);
    }

    pub fn set_confirmed_frontier(&mut self, frontier: BlockHash) {
        self.confirmed_frontier = Some(frontier);
    }

    pub fn set_representative(&mut self, representative: Account) {
        self.representative = Some(representative);
    }

    pub fn set_confirmed_representative(&mut self, representative: Account) {
        self.confirmed_representative = Some(representative);
    }

    pub fn set_weight(&mut self, weight: u128) {
        self.weight = Some(weight);
    }

    pub fn set_pending(&mut self, pending: u128) {
        self.pending = Some(pending);
    }

    pub fn set_receivable(&mut self, receivable: u128) {
        self.receivable = Some(receivable);
    }

    pub fn set_confirmed_pending(&mut self, pending: u128) {
        self.confirmed_pending = Some(pending);
    }

    pub fn set_confirmed_receivable(&mut self, receivable: u128) {
        self.confirmed_receivable = Some(receivable);
    }
}
