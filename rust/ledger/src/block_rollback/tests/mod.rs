use rsnano_core::{Account, BlockHash, ConfirmationHeightInfo, TestAccountChain};

use crate::ledger_constants::LEDGER_CONSTANTS_STUB;

use super::rollback_planner::{RollbackInstructions, RollbackPlanner, RollbackStep};

mod rollback_tests;

pub(crate) struct RollbackTest<'a> {
    planner: RollbackPlanner<'a>,
}

impl<'a> RollbackTest<'a> {
    pub const SECONDS_SINCE_EPOCH: u64 = 1234;
    pub fn for_chain(chain: &'a TestAccountChain) -> Self {
        Self {
            planner: create_test_rollback_planner(chain),
        }
    }

    pub fn with_linked_account(mut self, account: impl Into<Account>) -> Self {
        self.planner.linked_account = account.into();
        self
    }

    pub fn assert_rollback_succeeds(self) -> RollbackInstructions {
        let result = self
            .planner
            .roll_back_head_block()
            .expect("rollback should succeed");
        let RollbackStep::RollBackBlock(instructions) = result else { panic!("expected RollBackBlock") };
        instructions
    }

    pub fn assert_dependency_rollback(self) -> BlockHash {
        let result = self
            .planner
            .roll_back_head_block()
            .expect("rollback should succeed");
        let RollbackStep::RequestDependencyRollback(dependency_hash) = result else { panic!("expected dependency rollback") };
        dependency_hash
    }
}

fn create_test_rollback_planner<'a>(chain: &'a TestAccountChain) -> RollbackPlanner<'a> {
    RollbackPlanner {
        epochs: &LEDGER_CONSTANTS_STUB.epochs,
        head_block: chain.latest_block(),
        account: chain.account(),
        current_account_info: chain.account_info(),
        previous_representative: chain.representative_at_height(chain.height() - 1),
        previous: chain.try_get_block(chain.height() - 1).cloned(),
        linked_account: Account::zero(),
        pending_receive: None,
        latest_block_for_destination: None,
        confirmation_height: ConfirmationHeightInfo {
            height: 0,
            frontier: BlockHash::zero(),
        },
        seconds_since_epoch: RollbackTest::SECONDS_SINCE_EPOCH,
    }
}
