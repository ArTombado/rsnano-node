use crate::{
    config::NodeConfig,
    consensus::VoteBroadcaster,
    stats::Stats,
    transport::{
        ChannelEnum, ChannelId, ChannelInProc, InboundCallback, Network, OutboundBandwidthLimiter,
    },
    utils::AsyncRuntime,
    wallets::Wallets,
    NetworkParams,
};

use super::{vote_generator::VoteGenerator, LocalVoteHistory};
use rsnano_core::{utils::ContainerInfoComponent, BlockEnum, BlockHash, PublicKey, Root};
use rsnano_ledger::Ledger;
use std::{
    net::{Ipv6Addr, SocketAddrV6},
    sync::Arc,
    time::{Duration, SystemTime},
};

pub struct VoteGenerators {
    non_final_vote_generator: VoteGenerator,
    final_vote_generator: VoteGenerator,
}

pub(crate) fn create_loopback_channel(
    node_id: PublicKey,
    network: &Network,
    stats: Arc<Stats>,
    network_params: &NetworkParams,
    inbound: InboundCallback,
    runtime: &Arc<AsyncRuntime>,
) -> Arc<ChannelEnum> {
    let local_endpoint = SocketAddrV6::new(Ipv6Addr::LOCALHOST, network.port(), 0, 0);
    Arc::new(ChannelEnum::InProc(ChannelInProc::new(
        ChannelId::LOOPBACK,
        SystemTime::now(),
        network_params.network.clone(),
        network.publish_filter.clone(),
        stats,
        Arc::new(OutboundBandwidthLimiter::default()),
        Arc::clone(&inbound),
        Arc::clone(&inbound),
        runtime,
        local_endpoint,
        local_endpoint,
        node_id,
        node_id,
    )))
}

impl VoteGenerators {
    pub(crate) fn new(
        ledger: Arc<Ledger>,
        wallets: Arc<Wallets>,
        history: Arc<LocalVoteHistory>,
        stats: Arc<Stats>,
        config: &NodeConfig,
        network_params: &NetworkParams,
        vote_broadcaster: Arc<VoteBroadcaster>,
    ) -> Self {
        let non_final_vote_generator = VoteGenerator::new(
            ledger.clone(),
            wallets.clone(),
            history.clone(),
            false, //none-final
            stats.clone(),
            Duration::from_secs(network_params.voting.delay_s as u64),
            Duration::from_millis(config.vote_generator_delay_ms as u64),
            config.vote_generator_threshold as usize,
            vote_broadcaster.clone(),
        );

        let final_vote_generator = VoteGenerator::new(
            ledger,
            wallets,
            history,
            true, //final
            stats,
            Duration::from_secs(network_params.voting.delay_s as u64),
            Duration::from_millis(config.vote_generator_delay_ms as u64),
            config.vote_generator_threshold as usize,
            vote_broadcaster,
        );

        Self {
            non_final_vote_generator,
            final_vote_generator,
        }
    }

    pub fn start(&self) {
        self.non_final_vote_generator.start();
        self.final_vote_generator.start();
    }

    pub fn stop(&self) {
        self.non_final_vote_generator.stop();
        self.final_vote_generator.stop();
    }

    pub(crate) fn generate_final_vote(&self, root: &Root, hash: &BlockHash) {
        self.final_vote_generator.add(root, hash);
    }

    pub(crate) fn generate_final_votes(
        &self,
        blocks: &[Arc<BlockEnum>],
        channel: Arc<ChannelEnum>,
    ) -> usize {
        self.final_vote_generator.generate(blocks, channel)
    }

    pub fn generate_non_final_vote(&self, root: &Root, hash: &BlockHash) {
        self.non_final_vote_generator.add(root, hash);
    }

    pub fn generate_non_final_votes(
        &self,
        blocks: &[Arc<BlockEnum>],
        channel: Arc<ChannelEnum>,
    ) -> usize {
        self.non_final_vote_generator.generate(blocks, channel)
    }

    pub(crate) fn collect_container_info(&self, name: impl Into<String>) -> ContainerInfoComponent {
        ContainerInfoComponent::Composite(
            name.into(),
            vec![
                self.non_final_vote_generator
                    .collect_container_info("non_final"),
                self.final_vote_generator.collect_container_info("final"),
            ],
        )
    }
}
