use crate::command_handler::RpcCommandHandler;
use rsnano_rpc_messages::BlockCountResponse;

impl RpcCommandHandler {
    pub(crate) fn block_count(&self) -> BlockCountResponse {
        let count = self.node.ledger.block_count();
        let unchecked = self.node.unchecked.len() as u64;
        let cemented = self.node.ledger.cemented_count();
        let mut block_count = BlockCountResponse {
            count: count.into(),
            unchecked: unchecked.into(),
            cemented: cemented.into(),
            full: None,
            pruned: None,
        };

        if self.node.flags.enable_pruning {
            let full = self.node.ledger.block_count() - self.node.ledger.pruned_count();
            let pruned = self.node.ledger.pruned_count();

            block_count.full = Some(full.into());
            block_count.pruned = Some(pruned.into())
        }

        block_count
    }
}
