use std::{
    net::SocketAddr,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Mutex, Weak,
    },
    time::{SystemTime, UNIX_EPOCH},
};

use async_trait::async_trait;
use rsnano_core::Account;
use tokio::task::spawn_blocking;

use crate::{
    config::NetworkConstants,
    messages::{Message, MessageSerializer},
    stats::{DetailType, Direction, StatType, Stats},
    utils::{AsyncRuntime, BlockUniquer, ErrorCode},
    voting::VoteUniquer,
};

use super::{
    message_deserializer::{AsyncBufferReader, AsyncMessageDeserializer},
    BandwidthLimitType, BufferDropPolicy, Channel, ChannelEnum, DeserializedMessage, NetworkFilter,
    OutboundBandwidthLimiter, ParseStatus, TrafficType, WriteCallback,
};

pub struct InProcChannelData {
    last_bootstrap_attempt: SystemTime,
    last_packet_received: SystemTime,
    last_packet_sent: SystemTime,
    node_id: Option<Account>,
}

pub type InboundCallback = Arc<dyn Fn(DeserializedMessage, Arc<ChannelEnum>) + Send + Sync>;

pub struct ChannelInProc {
    channel_id: usize,
    temporary: AtomicBool,
    channel_mutex: Mutex<InProcChannelData>,
    network_constants: NetworkConstants,
    network_filter: Arc<NetworkFilter>,
    stats: Arc<Stats>,
    limiter: Arc<OutboundBandwidthLimiter>,
    source_inbound: InboundCallback,
    destination_inbound: InboundCallback,
    async_rt: Weak<AsyncRuntime>,
    pub source_endpoint: SocketAddr,
    pub destination_endpoint: SocketAddr,
    source_node_id: Account,
    destination_node_id: Account,
}

impl ChannelInProc {
    pub fn new(
        channel_id: usize,
        now: SystemTime,
        network_constants: NetworkConstants,
        network_filter: Arc<NetworkFilter>,
        stats: Arc<Stats>,
        limiter: Arc<OutboundBandwidthLimiter>,
        source_inbound: InboundCallback,
        destination_inbound: InboundCallback,
        async_rt: &Arc<AsyncRuntime>,
        source_endpoint: SocketAddr,
        destination_endpoint: SocketAddr,
        source_node_id: Account,
        destination_node_id: Account,
    ) -> Self {
        Self {
            channel_id,
            temporary: AtomicBool::new(false),
            channel_mutex: Mutex::new(InProcChannelData {
                last_bootstrap_attempt: UNIX_EPOCH,
                last_packet_received: now,
                last_packet_sent: now,
                node_id: Some(source_node_id),
            }),
            network_constants,
            network_filter,
            stats,
            limiter,
            source_inbound,
            destination_inbound,
            async_rt: Arc::downgrade(async_rt),
            source_endpoint,
            destination_endpoint,
            source_node_id,
            destination_node_id,
        }
    }

    pub fn send_new(
        &self,
        message: &Message,
        callback: Option<WriteCallback>,
        drop_policy: BufferDropPolicy,
        traffic_type: TrafficType,
    ) {
        let mut serializer = MessageSerializer::new(self.network_constants.protocol_info());
        let buffer = serializer.serialize(message).unwrap();
        let buffer = Arc::new(Vec::from(buffer)); // TODO don't copy buffer
        let detail = DetailType::from(message);
        let is_droppable_by_limiter = drop_policy == BufferDropPolicy::Limiter;
        let should_pass = self
            .limiter
            .should_pass(buffer.len(), BandwidthLimitType::from(traffic_type));

        if !is_droppable_by_limiter || should_pass {
            self.send_buffer_2(&buffer, callback, drop_policy, traffic_type);
            self.stats.inc(StatType::Message, detail, Direction::Out);
        } else {
            if let Some(cb) = callback {
                if let Some(async_rt) = self.async_rt.upgrade() {
                    async_rt.post(Box::new(move || {
                        cb(ErrorCode::not_supported(), 0);
                    }))
                }
            }

            self.stats.inc(StatType::Drop, detail, Direction::Out);
        }
    }

    pub fn send_buffer_2(
        &self,
        buffer_a: &Arc<Vec<u8>>,
        callback_a: Option<WriteCallback>,
        _policy_a: BufferDropPolicy,
        _traffic_type: TrafficType,
    ) {
        let stats = self.stats.clone();
        let network_constants = self.network_constants.clone();
        let limiter = self.limiter.clone();
        let source_inbound = self.source_inbound.clone();
        let destination_inbound = self.destination_inbound.clone();
        let source_endpoint = self.source_endpoint;
        let destination_endpoint = self.destination_endpoint;
        let source_node_id = self.source_node_id;
        let destination_node_id = self.destination_node_id;
        let async_rt = self.async_rt.clone();

        let callback_wrapper = Box::new(move |ec: ErrorCode, msg: Option<DeserializedMessage>| {
            if ec.is_err() {
                return;
            }
            let Some(async_rt) = async_rt.upgrade() else {
                return;
            };
            let Some(msg) = msg else {
                return;
            };
            let filter = Arc::new(NetworkFilter::new(100000));
            // we create a temporary channel for the reply path, in case the receiver of the message wants to reply
            let remote_channel = Arc::new(ChannelEnum::InProc(ChannelInProc::new(
                1,
                SystemTime::now(),
                network_constants.clone(),
                filter,
                stats.clone(),
                limiter,
                source_inbound,
                destination_inbound.clone(),
                &async_rt,
                source_endpoint,
                destination_endpoint,
                source_node_id,
                destination_node_id,
            )));

            // process message
            {
                stats.inc(
                    StatType::Message,
                    msg.message.message_type().into(),
                    Direction::In,
                );

                destination_inbound(msg, remote_channel);
            }
        });

        self.send_buffer_impl(buffer_a, callback_wrapper);

        if let Some(cb) = callback_a {
            let buffer_size = buffer_a.len();
            if let Some(async_rt) = self.async_rt.upgrade() {
                async_rt.post(Box::new(move || {
                    cb(ErrorCode::new(), buffer_size);
                }));
            }
        }
    }

    fn send_buffer_impl(
        &self,
        buffer: &[u8],
        callback_msg: Box<dyn FnOnce(ErrorCode, Option<DeserializedMessage>) + Send>,
    ) {
        if let Some(rt) = self.async_rt.upgrade() {
            let message_deserializer = Arc::new(AsyncMessageDeserializer::new(
                self.network_constants.clone(),
                self.network_filter.clone(),
                Arc::new(BlockUniquer::new()),
                Arc::new(VoteUniquer::new()),
                VecBufferReader::new(buffer.to_vec()),
            ));

            rt.tokio.spawn(async move {
                let result = message_deserializer.read().await;
                spawn_blocking(move || match result {
                    Ok(msg) => callback_msg(ErrorCode::new(), Some(msg)),
                    Err(ParseStatus::DuplicatePublishMessage) => {
                        callback_msg(ErrorCode::new(), None)
                    }
                    Err(ParseStatus::InsufficientWork) => callback_msg(ErrorCode::new(), None),
                    Err(_) => callback_msg(ErrorCode::fault(), None),
                });
            });
        }
    }

    pub fn network_version(&self) -> u8 {
        self.network_constants.protocol_version
    }
}

struct VecBufferReader {
    buffer: Vec<u8>,
    position: AtomicUsize,
}

impl VecBufferReader {
    fn new(buffer: Vec<u8>) -> Self {
        Self {
            buffer,
            position: AtomicUsize::new(0),
        }
    }
}

#[async_trait]
impl AsyncBufferReader for VecBufferReader {
    async fn read(&self, buffer: Arc<Mutex<Vec<u8>>>, count: usize) -> anyhow::Result<()> {
        let pos = self.position.load(Ordering::SeqCst);
        if count > self.buffer.len() - pos {
            bail!("no more data to read");
        }
        let mut guard = buffer.lock().unwrap();
        guard[..count].copy_from_slice(&self.buffer[pos..pos + count]);
        self.position.fetch_add(count, Ordering::SeqCst);
        Ok(())
    }
}

impl Channel for ChannelInProc {
    fn is_temporary(&self) -> bool {
        self.temporary.load(Ordering::SeqCst)
    }

    fn set_temporary(&self, temporary: bool) {
        self.temporary
            .store(temporary, std::sync::atomic::Ordering::SeqCst);
    }

    fn get_last_bootstrap_attempt(&self) -> SystemTime {
        self.channel_mutex.lock().unwrap().last_bootstrap_attempt
    }

    fn set_last_bootstrap_attempt(&self, time: SystemTime) {
        self.channel_mutex.lock().unwrap().last_bootstrap_attempt = time;
    }

    fn get_last_packet_received(&self) -> SystemTime {
        self.channel_mutex.lock().unwrap().last_packet_received
    }

    fn set_last_packet_received(&self, instant: SystemTime) {
        self.channel_mutex.lock().unwrap().last_packet_received = instant;
    }

    fn get_last_packet_sent(&self) -> SystemTime {
        self.channel_mutex.lock().unwrap().last_packet_sent
    }

    fn set_last_packet_sent(&self, instant: SystemTime) {
        self.channel_mutex.lock().unwrap().last_packet_sent = instant;
    }

    fn get_node_id(&self) -> Option<Account> {
        self.channel_mutex.lock().unwrap().node_id
    }

    fn set_node_id(&self, id: Account) {
        self.channel_mutex.lock().unwrap().node_id = Some(id);
    }

    fn is_alive(&self) -> bool {
        true
    }

    fn channel_id(&self) -> usize {
        self.channel_id
    }

    fn get_type(&self) -> super::TransportType {
        super::TransportType::Loopback
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn empty_vec() {
        let reader = VecBufferReader::new(Vec::new());
        let buffer = Arc::new(Mutex::new(vec![0u8; 3]));
        let result = reader.read(buffer, 1).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn read_one_byte() {
        let reader = VecBufferReader::new(vec![42]);
        let buffer = Arc::new(Mutex::new(vec![0u8; 1]));
        let result = reader.read(Arc::clone(&buffer), 1).await;
        assert!(result.is_ok());
        let guard = buffer.lock().unwrap();
        assert_eq!(guard[0], 42);
    }

    #[tokio::test]
    async fn multiple_reads() {
        let reader = VecBufferReader::new(vec![1, 2, 3, 4, 5]);
        let buffer = Arc::new(Mutex::new(vec![0u8; 2]));
        reader.read(Arc::clone(&buffer), 1).await.unwrap();
        {
            let guard = buffer.lock().unwrap();
            assert_eq!(guard[0], 1);
        }
        reader.read(Arc::clone(&buffer), 2).await.unwrap();
        {
            let guard = buffer.lock().unwrap();
            assert_eq!(guard[0], 2);
            assert_eq!(guard[1], 3);
        }
        reader.read(Arc::clone(&buffer), 2).await.unwrap();
        {
            let guard = buffer.lock().unwrap();
            assert_eq!(guard[0], 4);
            assert_eq!(guard[1], 5);
        }
        assert!(reader.read(Arc::clone(&buffer), 1).await.is_err());
    }
}
