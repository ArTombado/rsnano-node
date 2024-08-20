use super::{Channel, ChannelDirection, ChannelId, ChannelMode};
use rsnano_core::PublicKey;
use std::{
    collections::{BTreeMap, HashMap},
    hash::Hash,
    net::{Ipv6Addr, SocketAddrV6},
    sync::Arc,
    time::SystemTime,
};

/// Keeps track of all connected channels
#[derive(Default)]
pub struct ChannelContainer {
    by_channel_id: HashMap<ChannelId, Arc<Channel>>,
    by_endpoint: HashMap<SocketAddrV6, Vec<ChannelId>>,
    sequential: Vec<ChannelId>,
    by_bootstrap_attempt: BTreeMap<SystemTime, Vec<ChannelId>>,
    by_network_version: BTreeMap<u8, Vec<ChannelId>>,
    by_ip_address: HashMap<Ipv6Addr, Vec<ChannelId>>,
    by_subnet: HashMap<Ipv6Addr, Vec<ChannelId>>,
}

impl ChannelContainer {
    pub fn insert(&mut self, channel: Arc<Channel>) -> bool {
        let id = channel.channel_id();
        if self.by_channel_id.contains_key(&id) {
            panic!("Channel already in collection!");
        }

        self.sequential.push(id);
        self.by_bootstrap_attempt
            .entry(channel.info.last_bootstrap_attempt())
            .or_default()
            .push(id);
        self.by_network_version
            .entry(channel.info.protocol_version())
            .or_default()
            .push(id);
        self.by_ip_address
            .entry(channel.ipv4_address_or_ipv6_subnet())
            .or_default()
            .push(id);
        self.by_subnet
            .entry(channel.subnetwork())
            .or_default()
            .push(id);
        self.by_endpoint
            .entry(channel.info.peer_addr())
            .or_default()
            .push(id);
        self.by_channel_id.insert(id, channel);
        true
    }

    pub fn iter(&self) -> impl Iterator<Item = &Arc<Channel>> {
        self.by_channel_id.values().filter(|c| c.info.is_alive())
    }

    pub fn iter_by_last_bootstrap_attempt(&self) -> impl Iterator<Item = &Arc<Channel>> {
        self.by_bootstrap_attempt
            .iter()
            .flat_map(|(_, ids)| ids.iter().map(|id| self.by_channel_id.get(id).unwrap()))
            .filter(|c| c.info.is_alive())
    }

    pub fn count_by_mode(&self, mode: ChannelMode) -> usize {
        self.by_channel_id
            .values()
            .filter(|c| c.info.mode() == mode && c.info.is_alive())
            .count()
    }

    pub(crate) fn remove_by_id(&mut self, id: ChannelId) -> Option<Arc<Channel>> {
        if let Some(channel) = self.by_channel_id.remove(&id) {
            self.sequential.retain(|x| *x != id); // todo: linear search is slow?

            remove_from_btree(
                &mut self.by_bootstrap_attempt,
                &channel.info.last_bootstrap_attempt(),
                id,
            );
            remove_from_btree(
                &mut self.by_network_version,
                &channel.info.protocol_version(),
                id,
            );
            remove_from_hashmap(&mut self.by_endpoint, &channel.info.peer_addr(), id);
            remove_from_hashmap(
                &mut self.by_ip_address,
                &channel.ipv4_address_or_ipv6_subnet(),
                id,
            );
            remove_from_hashmap(&mut self.by_subnet, &channel.subnetwork(), id);
            Some(channel)
        } else {
            None
        }
    }

    pub fn get_by_id(&self, id: ChannelId) -> Option<&Arc<Channel>> {
        self.by_channel_id.get(&id)
    }

    pub fn get_by_node_id(&self, node_id: &PublicKey) -> Option<&Arc<Channel>> {
        self.by_channel_id
            .values()
            .filter(|c| c.info.node_id() == Some(*node_id) && c.info.is_alive())
            .next()
    }

    pub fn set_protocol_version(&mut self, channel_id: ChannelId, protocol_version: u8) {
        if let Some(channel) = self.by_channel_id.get(&channel_id) {
            let old_version = channel.info.protocol_version();
            channel.info.set_protocol_version(protocol_version);
            if old_version == protocol_version {
                return;
            }
            remove_from_btree(&mut self.by_network_version, &old_version, channel_id);
            self.by_network_version
                .entry(protocol_version)
                .or_default()
                .push(channel_id);
        }
    }

    pub fn set_last_bootstrap_attempt(&mut self, channel_id: ChannelId, attempt_time: SystemTime) {
        if let Some(channel) = self.by_channel_id.get(&channel_id) {
            let old_time = channel.info.last_bootstrap_attempt();
            channel.info.set_last_bootstrap_attempt(attempt_time);
            remove_from_btree(&mut self.by_bootstrap_attempt, &old_time, channel_id);
            self.by_bootstrap_attempt
                .entry(attempt_time)
                .or_default()
                .push(channel_id);
        }
    }

    pub fn count_by_direction(&self, direction: ChannelDirection) -> usize {
        self.by_channel_id
            .values()
            .filter(|c| c.info.direction() == direction && c.info.is_alive())
            .count()
    }

    pub fn clear(&mut self) {
        self.by_endpoint.clear();
        self.sequential.clear();
        self.by_bootstrap_attempt.clear();
        self.by_network_version.clear();
        self.by_ip_address.clear();
        self.by_subnet.clear();
        self.by_channel_id.clear();
    }
}

fn remove_from_hashmap<K>(tree: &mut HashMap<K, Vec<ChannelId>>, key: &K, id: ChannelId)
where
    K: Ord + Hash,
{
    let channel_ids = tree.get_mut(key).unwrap();
    if channel_ids.len() > 1 {
        channel_ids.retain(|x| *x != id);
    } else {
        tree.remove(key);
    }
}

fn remove_from_btree<K: Ord>(tree: &mut BTreeMap<K, Vec<ChannelId>>, key: &K, id: ChannelId) {
    let channel_ids = tree.get_mut(key).unwrap();
    if channel_ids.len() > 1 {
        channel_ids.retain(|x| *x != id);
    } else {
        tree.remove(key);
    }
}
