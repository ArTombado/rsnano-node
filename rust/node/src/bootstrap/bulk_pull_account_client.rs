use crate::{
    config::NodeConfig,
    stats::{DetailType, Direction, StatType, Stats},
    transport::{BufferDropPolicy, TrafficType},
};

use super::{
    BootstrapAttemptWallet, BootstrapClient, BootstrapConnections, BootstrapConnectionsExt,
    BootstrapInitiator, BootstrapInitiatorExt,
};
use rsnano_core::{
    utils::{BufferReader, Deserialize, FixedSizeSerialize},
    Account, Amount, BlockHash,
};
use rsnano_ledger::Ledger;
use rsnano_messages::{BulkPullAccount, BulkPullAccountFlags, Message};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};
use tracing::{debug, trace};

pub struct BulkPullAccountClient {
    connection: Arc<BootstrapClient>,
    attempt: Arc<BootstrapAttemptWallet>,
    account: Account,
    config: NodeConfig,
    stats: Arc<Stats>,
    pull_blocks: AtomicU64,
    connections: Arc<BootstrapConnections>,
    ledger: Arc<Ledger>,
    bootstrap_initiator: Arc<BootstrapInitiator>,
}

impl BulkPullAccountClient {
    pub fn new(
        connection: Arc<BootstrapClient>,
        attempt: Arc<BootstrapAttemptWallet>,
        account: Account,
        config: NodeConfig,
        stats: Arc<Stats>,
        connections: Arc<BootstrapConnections>,
        ledger: Arc<Ledger>,
        bootstrap_initiator: Arc<BootstrapInitiator>,
    ) -> Self {
        attempt.attempt.condition.notify_all();
        Self {
            connection,
            attempt,
            account,
            config,
            stats,
            pull_blocks: AtomicU64::new(0),
            connections,
            ledger,
            bootstrap_initiator,
        }
    }
}

impl Drop for BulkPullAccountClient {
    fn drop(&mut self) {
        self.attempt.attempt.pull_finished();
    }
}

pub trait BulkPullAccountClientExt {
    fn request(&self);
    fn receive_pending(&self);
}

impl BulkPullAccountClientExt for Arc<BulkPullAccountClient> {
    fn request(&self) {
        let req = Message::BulkPullAccount(BulkPullAccount {
            account: self.account,
            minimum_amount: self.config.receive_minimum,
            flags: BulkPullAccountFlags::PendingHashAndAmount,
        });

        trace!(
            account = self.account.encode_account(),
            connection = self.connection.channel_string(),
            "requesting pending"
        );

        if self.attempt.attempt.should_log() {
            debug!("Accounts in pull queue: {}", self.attempt.wallet_size());
        }

        let self_l = Arc::clone(self);
        self.connection.send(
            &req,
            Some(Box::new(move |ec, _size| {
                if ec.is_ok() {
                    self_l.receive_pending();
                } else {
                    debug!(
                        "Error starting bulk pull request to: {} ({:?})",
                        self_l.connection.channel_string(),
                        ec
                    );
                    self_l.stats.inc_dir(
                        StatType::Bootstrap,
                        DetailType::BulkPullErrorStartingRequest,
                        Direction::In,
                    );

                    self_l.attempt.requeue_pending(self_l.account);
                }
            })),
            BufferDropPolicy::NoLimiterDrop,
            TrafficType::Generic,
        );
    }

    fn receive_pending(&self) {
        let this_l = Arc::clone(self);
        let size_l = BlockHash::serialized_size() + Amount::serialized_size();
        self.connection.read_async(
            size_l,
            Box::new(move |ec, size| {
                // An issue with asio is that sometimes, instead of reporting a bad file descriptor during disconnect,
                // we simply get a size of 0.
                if size == size_l {
                    if ec.is_ok() {
                        let buf = this_l.connection.receive_buffer();
                        let mut reader = BufferReader::new(&buf);
                        let pending = BlockHash::deserialize(&mut reader).unwrap();
                        let balance = Amount::deserialize(&mut reader).unwrap();
                        if this_l.pull_blocks.load(Ordering::SeqCst) == 0 || !pending.is_zero() {
                            if this_l.pull_blocks.load(Ordering::SeqCst) == 0
                                || balance >= this_l.config.receive_minimum
                            {
                                this_l.pull_blocks.fetch_add(1, Ordering::SeqCst);
                                {
                                    if !pending.is_zero() {
                                        if !this_l.ledger.any().block_exists_or_pruned(
                                            &this_l.ledger.read_txn(),
                                            &pending,
                                        ) {
                                            this_l.bootstrap_initiator.bootstrap_lazy(
                                                pending.into(),
                                                false,
                                                "".to_string(),
                                            );
                                        }
                                    }
                                }
                                this_l.receive_pending();
                            } else {
                                this_l.attempt.requeue_pending(this_l.account);
                            }
                        } else {
                            this_l.connections.pool_connection(
                                Arc::clone(&this_l.connection),
                                false,
                                false,
                            );
                        }
                    } else {
                        debug!("Error while receiving bulk pull account frontier: {:?}", ec);
                        this_l.attempt.requeue_pending(this_l.account);
                    }
                } else {
                    debug!("Invalid size: Expected {}, got: {}", size_l, size);
                    this_l.attempt.requeue_pending(this_l.account);
                }
            }),
        );
    }
}
