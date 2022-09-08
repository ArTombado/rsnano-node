use crate::{Block, BlockEnum, BlockHash};

use super::{Transaction, WriteTransaction};

pub trait BlockStore {
    fn put(&self, txn: &dyn WriteTransaction, hash: &BlockHash, block: &dyn Block);
    fn exists(&self, txn: &dyn Transaction, hash: &BlockHash) -> bool;
    fn successor(&self, txn: &dyn Transaction, hash: &BlockHash) -> BlockHash;
    fn successor_clear(&self, txn: &dyn WriteTransaction, hash: &BlockHash);
    fn get(&self, txn: &dyn Transaction, hash: &BlockHash) -> Option<BlockEnum>;
}
