use num_derive::FromPrimitive;
use num_traits::FromPrimitive;
use rsnano_core::{
    utils::{TEST_ENDPOINT_1, TEST_ENDPOINT_2},
    PublicKey,
};
use rsnano_nullable_clock::Timestamp;
use std::{
    fmt::{Debug, Display},
    net::{Ipv6Addr, SocketAddrV6},
    sync::{
        atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering},
        Mutex,
    },
    time::Duration,
};

use crate::utils::{ipv4_address_or_ipv6_subnet, map_address_to_subnetwork};

#[derive(PartialEq, Eq, PartialOrd, Ord, Copy, Clone, Hash)]
pub struct ChannelId(usize);

impl ChannelId {
    pub const LOOPBACK: Self = Self(0);
    pub const MIN: Self = Self(usize::MIN);
    pub const MAX: Self = Self(usize::MAX);

    pub fn as_usize(&self) -> usize {
        self.0
    }
}

impl Display for ChannelId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(&self.0, f)
    }
}

impl Debug for ChannelId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(&self.0, f)
    }
}

impl From<usize> for ChannelId {
    fn from(value: usize) -> Self {
        Self(value)
    }
}

#[derive(PartialEq, Eq, Clone, Copy, FromPrimitive, Debug)]
pub enum ChannelDirection {
    /// Socket was created by accepting an incoming connection
    Inbound,
    /// Socket was created by initiating an outgoing connection
    Outbound,
}

#[derive(FromPrimitive, Copy, Clone, Debug)]
pub enum TrafficType {
    Generic,
    /** For bootstrap (asc_pull_ack, asc_pull_req) traffic */
    Bootstrap,
}

#[derive(PartialEq, Eq, Clone, Copy, Debug, FromPrimitive)]
pub enum ChannelMode {
    /// No messages have been exchanged yet, so the mode is undefined
    Undefined,
    /// Only serve bootstrap requests
    Bootstrap,
    /// serve realtime traffic (votes, new blocks,...)
    Realtime,
}

impl ChannelMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            ChannelMode::Undefined => "undefined",
            ChannelMode::Bootstrap => "bootstrap",
            ChannelMode::Realtime => "realtime",
        }
    }
}

/// Default timeout in seconds
const DEFAULT_TIMEOUT: u64 = 120;

pub struct ChannelInfo {
    channel_id: ChannelId,
    local_addr: SocketAddrV6,
    peer_addr: SocketAddrV6,
    data: Mutex<ChannelInfoData>,
    protocol_version: AtomicU8,
    direction: ChannelDirection,

    /// the timestamp (in seconds since epoch) of the last time there was successful activity on the socket
    last_activity: AtomicU64,
    last_bootstrap_attempt: AtomicU64,
    last_packet_received: AtomicU64,
    last_packet_sent: AtomicU64,

    /// Duration in seconds of inactivity that causes a socket timeout
    /// activity is any successful connect, send or receive event
    timeout_seconds: AtomicU64,

    /// Flag that is set when cleanup decides to close the socket due to timeout.
    /// NOTE: Currently used by tcp_server::timeout() but I suspect that this and tcp_server::timeout() are not needed.
    timed_out: AtomicBool,

    /// Set by close() - completion handlers must check this. This is more reliable than checking
    /// error codes as the OS may have already completed the async operation.
    closed: AtomicBool,

    socket_type: AtomicU8,
}

impl ChannelInfo {
    pub fn new(
        channel_id: ChannelId,
        local_addr: SocketAddrV6,
        peer_addr: SocketAddrV6,
        direction: ChannelDirection,
        protocol_version: u8,
        now: Timestamp,
    ) -> Self {
        Self {
            channel_id,
            local_addr,
            peer_addr,
            // TODO set protocol version to 0
            protocol_version: AtomicU8::new(protocol_version),
            direction,
            last_activity: AtomicU64::new(now.into()),
            last_bootstrap_attempt: AtomicU64::new(0),
            last_packet_received: AtomicU64::new(now.into()),
            last_packet_sent: AtomicU64::new(now.into()),
            timeout_seconds: AtomicU64::new(DEFAULT_TIMEOUT),
            timed_out: AtomicBool::new(false),
            socket_type: AtomicU8::new(ChannelMode::Undefined as u8),
            closed: AtomicBool::new(false),
            data: Mutex::new(ChannelInfoData {
                node_id: None,
                is_queue_full_impl: None,
                peering_addr: if direction == ChannelDirection::Outbound {
                    Some(peer_addr)
                } else {
                    None
                },
            }),
        }
    }

    pub fn new_test_instance() -> Self {
        Self::new(
            ChannelId::from(42),
            TEST_ENDPOINT_1,
            TEST_ENDPOINT_2,
            ChannelDirection::Outbound,
            u8::MAX,
            Timestamp::new_test_instance(),
        )
    }

    pub fn set_queue_full_query(&self, query: Box<dyn Fn(TrafficType) -> bool + Send>) {
        self.data.lock().unwrap().is_queue_full_impl = Some(query);
    }

    pub fn channel_id(&self) -> ChannelId {
        self.channel_id
    }

    pub fn node_id(&self) -> Option<PublicKey> {
        self.data.lock().unwrap().node_id
    }

    pub fn direction(&self) -> ChannelDirection {
        self.direction
    }

    pub fn local_addr(&self) -> SocketAddrV6 {
        self.local_addr
    }

    /// The address that we are connected to. If this is an incoming channel, then
    /// the peer_addr uses an ephemeral port
    pub fn peer_addr(&self) -> SocketAddrV6 {
        self.peer_addr
    }

    /// The address where the peer accepts incoming connections. In case of an outbound
    /// channel, the peer_addr and peering_addr are the same
    pub fn peering_addr(&self) -> Option<SocketAddrV6> {
        self.data.lock().unwrap().peering_addr.clone()
    }

    pub fn peering_addr_or_peer_addr(&self) -> SocketAddrV6 {
        self.data
            .lock()
            .unwrap()
            .peering_addr
            .clone()
            .unwrap_or(self.peer_addr())
    }

    pub fn ipv4_address_or_ipv6_subnet(&self) -> Ipv6Addr {
        ipv4_address_or_ipv6_subnet(&self.peer_addr().ip())
    }

    pub fn subnetwork(&self) -> Ipv6Addr {
        map_address_to_subnetwork(self.peer_addr().ip())
    }

    pub fn protocol_version(&self) -> u8 {
        self.protocol_version.load(Ordering::Relaxed)
    }

    // TODO make private and set via NetworkInfo
    pub fn set_protocol_version(&self, version: u8) {
        self.protocol_version.store(version, Ordering::Relaxed);
    }

    pub fn last_activity(&self) -> Timestamp {
        self.last_activity.load(Ordering::Relaxed).into()
    }

    pub fn set_last_activity(&self, now: Timestamp) {
        self.last_activity.store(now.into(), Ordering::Relaxed);
    }

    pub fn timeout(&self) -> Duration {
        Duration::from_secs(self.timeout_seconds.load(Ordering::Relaxed))
    }

    pub fn set_timeout(&self, value: Duration) {
        self.timeout_seconds
            .store(value.as_secs(), Ordering::Relaxed)
    }

    pub fn timed_out(&self) -> bool {
        self.timed_out.load(Ordering::Relaxed)
    }

    pub fn set_timed_out(&self, value: bool) {
        self.timed_out.store(value, Ordering::Relaxed)
    }

    pub fn is_alive(&self) -> bool {
        !self.is_closed()
    }

    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Relaxed)
    }

    pub fn close(&self) {
        self.closed.store(true, Ordering::Relaxed);
        self.set_timeout(Duration::ZERO);
    }

    pub fn set_node_id(&self, node_id: PublicKey) {
        self.data.lock().unwrap().node_id = Some(node_id);
    }

    pub fn set_peering_addr(&self, peering_addr: SocketAddrV6) {
        self.data.lock().unwrap().peering_addr = Some(peering_addr);
    }

    pub fn mode(&self) -> ChannelMode {
        FromPrimitive::from_u8(self.socket_type.load(Ordering::SeqCst)).unwrap()
    }

    pub fn set_mode(&self, mode: ChannelMode) {
        self.socket_type.store(mode as u8, Ordering::SeqCst);
    }

    pub fn last_bootstrap_attempt(&self) -> Timestamp {
        self.last_bootstrap_attempt.load(Ordering::Relaxed).into()
    }

    pub fn set_last_bootstrap_attempt(&self, now: Timestamp) {
        self.last_bootstrap_attempt
            .store(now.into(), Ordering::Relaxed);
    }

    pub fn last_packet_received(&self) -> Timestamp {
        self.last_packet_received.load(Ordering::Relaxed).into()
    }

    pub fn set_last_packet_received(&self, now: Timestamp) {
        self.last_packet_received
            .store(now.into(), Ordering::Relaxed);
    }

    pub fn last_packet_sent(&self) -> Timestamp {
        self.last_packet_sent.load(Ordering::Relaxed).into()
    }

    pub fn set_last_packet_sent(&self, now: Timestamp) {
        self.last_packet_sent.store(now.into(), Ordering::Relaxed);
    }

    pub fn is_queue_full(&self, traffic_type: TrafficType) -> bool {
        let guard = self.data.lock().unwrap();
        match &guard.is_queue_full_impl {
            Some(cb) => cb(traffic_type),
            None => false,
        }
    }
}

struct ChannelInfoData {
    node_id: Option<PublicKey>,
    peering_addr: Option<SocketAddrV6>,
    is_queue_full_impl: Option<Box<dyn Fn(TrafficType) -> bool + Send>>,
}
